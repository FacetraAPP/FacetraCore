mod render;
mod workspace;

use clap::{Args, Parser, Subcommand};
use render::RenderOptions;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use workspace::Workspace;

#[derive(Debug, Parser)]
#[command(
    name = "facetra-render",
    version,
    about = "Render Facetra chart YAML without the Mac app"
)]
struct Cli {
    /// Emit one JSON object. Errors are also JSON.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Inspect or validate a read-only workspace.
    Workspace {
        #[command(subcommand)]
        command: WorkspaceCommand,
    },
    /// Render or compile one chart.
    Chart {
        #[command(subcommand)]
        command: ChartCommand,
    },
    /// Render all charts in a workspace.
    Batch(BatchArgs),
}

#[derive(Debug, Subcommand)]
enum WorkspaceCommand {
    /// List chart resources and workspace issues.
    Inspect {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
    /// Validate the complete chart contract.
    Validate {
        #[arg(default_value = ".")]
        path: PathBuf,
    },
}

#[derive(Debug, Subcommand)]
enum ChartCommand {
    /// Compile the final renderer payload without producing an image.
    Payload(ChartTarget),
    /// Compile the final payload and capture it as a PNG.
    Render(ChartRenderArgs),
}

#[derive(Clone, Debug, Args)]
struct ChartTarget {
    /// Workspace root, or a descendant directory.
    #[arg(long, default_value = ".")]
    workspace: PathBuf,
    /// Select a chart by metadata ID.
    #[arg(long, conflicts_with = "file")]
    id: Option<String>,
    /// Read a draft YAML file. Its metadata ID selects the chart.
    #[arg(long)]
    file: Option<PathBuf>,
    /// JSON object containing scalar parameter values.
    #[arg(long, conflicts_with = "params_file")]
    params: Option<String>,
    #[arg(long)]
    params_file: Option<PathBuf>,
    /// Locale used for resource-localized text and chart labels.
    #[arg(long, default_value = "en")]
    locale: String,
    /// Override the renderer document title used as its accessibility label.
    #[arg(long)]
    title: Option<String>,
}

#[derive(Debug, Args)]
struct ChartRenderArgs {
    #[command(flatten)]
    target: ChartTarget,
    #[arg(long)]
    out: PathBuf,
    #[arg(long, default_value_t = 1200, value_parser = clap::value_parser!(u16).range(100..=4096))]
    width: u16,
    #[arg(long, default_value_t = 700, value_parser = clap::value_parser!(u16).range(100..=4096))]
    height: u16,
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=3))]
    scale: u8,
    #[arg(long)]
    renderer_dir: Option<PathBuf>,
    #[arg(long)]
    force: bool,
}

#[derive(Debug, Args)]
struct BatchArgs {
    #[arg(long, default_value = ".")]
    workspace: PathBuf,
    #[arg(long)]
    out_dir: PathBuf,
    #[arg(long, default_value_t = 1200, value_parser = clap::value_parser!(u16).range(100..=4096))]
    width: u16,
    #[arg(long, default_value_t = 700, value_parser = clap::value_parser!(u16).range(100..=4096))]
    height: u16,
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=3))]
    scale: u8,
    #[arg(long, default_value = "en")]
    locale: String,
    #[arg(long)]
    renderer_dir: Option<PathBuf>,
    #[arg(long)]
    force: bool,
}

struct Failure {
    message: String,
    exit_code: i32,
}

impl Failure {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 2,
        }
    }

    fn runtime(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            exit_code: 1,
        }
    }
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Self::runtime(message)
    }
}

fn main() {
    let cli = Cli::parse();
    match execute(&cli) {
        Ok((value, exit_code)) => {
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string(&json!({"ok": exit_code == 0, "result": value})).unwrap()
                );
            } else {
                print_human(&value);
            }
            std::process::exit(exit_code);
        }
        Err(failure) => {
            if cli.json {
                println!(
                    "{}",
                    serde_json::to_string(&json!({
                        "ok": false,
                        "error": {"message": failure.message},
                    }))
                    .unwrap()
                );
            } else {
                eprintln!("{}", failure.message);
            }
            std::process::exit(failure.exit_code);
        }
    }
}

fn execute(cli: &Cli) -> Result<(Value, i32), Failure> {
    match &cli.command {
        Command::Workspace { command } => match command {
            WorkspaceCommand::Inspect { path } => {
                let root = workspace::find_root(path)?;
                let workspace = Workspace::open(&root)?;
                let resources = workspace
                    .charts()
                    .iter()
                    .map(|chart| &chart.info)
                    .collect::<Vec<_>>();
                Ok((
                    json!({
                        "workspace": workspace.root,
                        "resources": resources,
                        "issues": workspace.issues(),
                    }),
                    0,
                ))
            }
            WorkspaceCommand::Validate { path } => {
                let root = workspace::find_root(path)?;
                let workspace = Workspace::open(&root)?;
                let report = workspace.validate();
                let code = if report.valid { 0 } else { 1 };
                Ok((serde_json::to_value(report).unwrap(), code))
            }
        },
        Command::Chart { command } => match command {
            ChartCommand::Payload(target) => {
                let (workspace, options) = prepare(target)?;
                let rendered = render::render(&workspace, &options)?;
                Ok((serde_json::to_value(rendered).unwrap(), 0))
            }
            ChartCommand::Render(arguments) => {
                let (workspace, options) = prepare(&arguments.target)?;
                let rendered = render::render(&workspace, &options)?;
                let renderer_dir = render::default_renderer_dir(arguments.renderer_dir.as_deref())?;
                render::ensure_output(&arguments.out, arguments.force)?;
                let screenshot = render::screenshot(
                    &rendered,
                    &arguments.out,
                    arguments.width,
                    arguments.height,
                    arguments.scale,
                    &renderer_dir,
                    arguments.force,
                )?;
                Ok((
                    json!({
                        "id": rendered.id,
                        "title": rendered.title,
                        "renderer": rendered.renderer,
                        "width": arguments.width,
                        "height": arguments.height,
                        "scale": arguments.scale,
                        "png": screenshot,
                    }),
                    0,
                ))
            }
        },
        Command::Batch(arguments) => batch(arguments),
    }
}

fn prepare(target: &ChartTarget) -> Result<(Workspace, RenderOptions), Failure> {
    if target.id.is_none() == target.file.is_none() {
        return Err(Failure::usage("Specify exactly one of --id or --file."));
    }
    let root = workspace::find_root(&target.workspace)?;
    let workspace = Workspace::open(&root)?;
    let (id, overlay) = if let Some(file) = &target.file {
        let overlay = workspace.overlay(file)?;
        if let Some(id) = &target.id {
            if id != &overlay.info.id {
                return Err(Failure::usage(format!(
                    "Draft chart ID {} does not match --id {id}.",
                    overlay.info.id
                )));
            }
        }
        (overlay.info.id.clone(), Some(overlay))
    } else {
        let id = target.id.clone().unwrap();
        if workspace.chart(&id).is_none() {
            return Err(Failure::runtime(format!("Chart not found: {id}")));
        }
        (id, None)
    };
    let params = read_params(target.params.as_deref(), target.params_file.as_deref())?;
    Ok((
        workspace,
        RenderOptions {
            id,
            overlay,
            params,
            locale: target.locale.clone(),
            title: target.title.clone(),
        },
    ))
}

fn read_params(
    params: Option<&str>,
    params_file: Option<&Path>,
) -> Result<BTreeMap<String, Value>, Failure> {
    let text = if let Some(path) = params_file {
        fs::read_to_string(path)
            .map_err(|error| Failure::usage(format!("Cannot read {}: {error}", path.display())))?
    } else {
        params.unwrap_or("{}").to_owned()
    };
    let value: Value = serde_json::from_str(&text)
        .map_err(|error| Failure::usage(format!("Invalid --params JSON: {error}")))?;
    let object = value
        .as_object()
        .ok_or_else(|| Failure::usage("--params must be a JSON object."))?;
    if object
        .values()
        .any(|value| !(value.is_string() || value.is_boolean() || value.is_number()))
    {
        return Err(Failure::usage(
            "Parameters must be string, number or boolean values.",
        ));
    }
    Ok(object
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect())
}

fn batch(arguments: &BatchArgs) -> Result<(Value, i32), Failure> {
    let root = workspace::find_root(&arguments.workspace)?;
    let workspace = Workspace::open(&root)?;
    let renderer_dir = render::default_renderer_dir(arguments.renderer_dir.as_deref())?;
    let mut rendered = Vec::new();
    let mut failures = Vec::new();
    for chart in workspace.charts() {
        let options = RenderOptions {
            id: chart.info.id.clone(),
            overlay: None,
            params: BTreeMap::new(),
            locale: arguments.locale.clone(),
            title: None,
        };
        match render::render(&workspace, &options) {
            Ok(chart) => rendered.push(chart),
            Err(message) => failures.push(json!({
                "id": chart.info.id,
                "error": message,
            })),
        }
    }
    for chart in &rendered {
        let out = arguments.out_dir.join(format!("{}.png", chart.id));
        if let Err(error) = render::ensure_output(&out, arguments.force) {
            return Err(Failure::runtime(error));
        }
    }
    let outputs = rendered
        .iter()
        .map(|chart| {
            (
                chart.clone(),
                arguments.out_dir.join(format!("{}.png", chart.id)),
            )
        })
        .collect::<Vec<_>>();
    let capture_results = render::capture_batch(
        &outputs,
        arguments.width,
        arguments.height,
        arguments.scale,
        &renderer_dir,
        arguments.force,
    )?;
    let mut resources = Vec::new();
    for ((chart, out), result) in outputs.iter().zip(capture_results) {
        if result.ok {
            let digest = render::file_digest(out)?;
            resources.push(json!({
                "id": chart.id,
                "title": chart.title,
                "renderer": chart.renderer,
                "path": chart_path(out, &arguments.out_dir),
                "png": out,
                "sha256": digest,
            }));
        } else {
            failures.push(json!({
                "id": chart.id,
                "error": result.error.unwrap_or_else(|| "Screenshot failed.".into()),
            }));
        }
    }
    let valid = failures.is_empty();
    Ok((
        json!({
            "workspace": workspace.root,
            "width": arguments.width,
            "height": arguments.height,
            "scale": arguments.scale,
            "locale": arguments.locale,
            "resources": resources,
            "failures": failures,
        }),
        if valid { 0 } else { 1 },
    ))
}

fn chart_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn print_human(value: &Value) {
    println!("{}", serde_json::to_string_pretty(value).unwrap());
}
