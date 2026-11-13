use crate::workspace::{Resource, Workspace};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Clone)]
pub struct RenderOptions {
    pub id: String,
    pub overlay: Option<Resource>,
    pub params: BTreeMap<String, Value>,
    pub locale: String,
    pub title: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderedChart {
    pub id: String,
    pub title: String,
    pub renderer: String,
    pub payload: Value,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Screenshot {
    pub path: String,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct CaptureJob {
    engine: String,
    payload: Value,
    title: String,
    width: u16,
    height: u16,
    scale: u8,
    out: String,
    renderer_dir: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureResult {
    pub ok: bool,
    pub error: Option<String>,
}

pub fn render(workspace: &Workspace, options: &RenderOptions) -> Result<RenderedChart, String> {
    let (renderer, documents, workspace_title) =
        workspace.render_documents(&options.id, options.overlay.as_ref())?;
    let output = facetra_pipeline::run(facetra_pipeline::Request {
        documents,
        chart_id: options.id.clone(),
        locale: Some(options.locale.clone()),
        directory: workspace.root.to_string_lossy().into_owned(),
        params: options.params.clone(),
        include_frame: false,
    })?;
    Ok(RenderedChart {
        id: options.id.clone(),
        title: options.title.clone().unwrap_or(workspace_title),
        renderer,
        payload: output.render,
    })
}

pub fn screenshot(
    rendered: &RenderedChart,
    out: &Path,
    width: u16,
    height: u16,
    scale: u8,
    renderer_dir: &Path,
    force: bool,
) -> Result<Screenshot, String> {
    let jobs = vec![CaptureJob {
        engine: engine(&rendered.renderer)?.into(),
        payload: rendered.payload.clone(),
        title: rendered.title.clone(),
        width,
        height,
        scale,
        out: out.to_string_lossy().into_owned(),
        renderer_dir: renderer_dir.to_string_lossy().into_owned(),
    }];
    let results = capture(&jobs, force)?;
    if let Some(error) = results.first().and_then(|result| result.error.as_ref()) {
        return Err(error.clone());
    }
    Ok(Screenshot {
        path: out.to_string_lossy().into_owned(),
        sha256: file_digest(out)?,
    })
}

pub fn capture_batch(
    rendered: &[(RenderedChart, PathBuf)],
    width: u16,
    height: u16,
    scale: u8,
    renderer_dir: &Path,
    force: bool,
) -> Result<Vec<CaptureResult>, String> {
    let mut jobs = Vec::with_capacity(rendered.len());
    for (chart, out) in rendered {
        jobs.push(CaptureJob {
            engine: engine(&chart.renderer)?.into(),
            payload: chart.payload.clone(),
            title: chart.title.clone(),
            width,
            height,
            scale,
            out: out.to_string_lossy().into_owned(),
            renderer_dir: renderer_dir.to_string_lossy().into_owned(),
        });
    }
    capture(&jobs, force)
}

pub fn file_digest(path: &Path) -> Result<String, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

pub fn default_renderer_dir(explicit: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = explicit {
        return fs::canonicalize(path)
            .map_err(|error| format!("Cannot resolve renderer directory: {error}"));
    }
    if let Some(path) = std::env::var_os("FACETRA_RENDERER_DIR") {
        return fs::canonicalize(path)
            .map_err(|error| format!("Cannot resolve FACETRA_RENDERER_DIR: {error}"));
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            let candidate = parent.join("renderers");
            if candidate.is_dir() {
                return Ok(candidate);
            }
        }
    }
    let development = Path::new(env!("CARGO_MANIFEST_DIR")).join("../renderers");
    fs::canonicalize(&development).map_err(|_| {
        "Cannot find renderers. Pass --renderer-dir or set FACETRA_RENDERER_DIR.".into()
    })
}

pub fn ensure_output(path: &Path, force: bool) -> Result<(), String> {
    if !force && fs::symlink_metadata(path).is_ok() {
        return Err(format!("Output already exists: {}", path.display()));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Cannot create {}: {error}", parent.display()))?;
    }
    Ok(())
}

fn capture(jobs: &[CaptureJob], force: bool) -> Result<Vec<CaptureResult>, String> {
    if jobs.is_empty() {
        return Ok(Vec::new());
    }
    if !force {
        for job in jobs {
            if fs::symlink_metadata(&job.out).is_ok() {
                return Err(format!("Output already exists: {}", job.out));
            }
        }
    }
    for job in jobs {
        if let Some(parent) = Path::new(&job.out).parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("Cannot create {}: {error}", parent.display()))?;
        }
    }
    let renderer_dir = Path::new(&jobs[0].renderer_dir);
    let script = renderer_dir.join("capture.mjs");
    if !script.is_file() {
        return Err(format!(
            "Renderer capture script not found: {}",
            script.display()
        ));
    }
    let node = std::env::var("FACETRA_NODE").unwrap_or_else(|_| "node".into());
    let mut command = Command::new(node);
    command
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if std::env::var_os("NODE_PATH").is_none() {
        let global = Path::new("/opt/homebrew/lib/node_modules");
        if global.is_dir() {
            command.env("NODE_PATH", global);
        }
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("Cannot start Node renderer: {error}"))?;
    let input = serde_json::to_vec(jobs).map_err(|error| error.to_string())?;
    child
        .stdin
        .as_mut()
        .ok_or("Node renderer stdin is unavailable.")?
        .write_all(&input)
        .map_err(|error| format!("Cannot write renderer request: {error}"))?;
    drop(child.stdin.take());
    let output = child
        .wait_with_output()
        .map_err(|error| format!("Node renderer failed: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Invalid Node renderer response: {error}"))
}

fn engine(renderer: &str) -> Result<&'static str, String> {
    match renderer {
        "echarts" => Ok("ECharts"),
        "vela" => Ok("Vela"),
        "vega_lite" => Ok("VegaLite"),
        "html" => Ok("HTML"),
        "value" => Ok("Value"),
        other => Err(format!("Unsupported renderer: {other}")),
    }
}
