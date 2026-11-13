//! Shared application pipeline and raw-YAML validation entry point.
mod echarts;
mod engine;
pub mod git;
mod render;
mod runtime;
mod sources;
mod spec;
mod sql;
mod vela;

pub use git::facetra_git_run;
pub use runtime::{
    facetra_cancel_create, facetra_cancel_destroy, facetra_cancel_request, facetra_session_create,
    facetra_session_destroy, facetra_session_run,
};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    ffi::{CStr, CString},
    os::raw::c_char,
    time::Instant,
};

type Result<T> = std::result::Result<T, String>;
pub(crate) const MAX_ROWS: usize = 100_000;
pub(crate) const MAX_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub documents: Vec<String>,
    pub chart_id: String,
    #[serde(default)]
    pub locale: Option<String>,
    pub directory: String,
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
    /// Validation defaults to both outputs; callers can request only payload.
    #[serde(default = "include_frame")]
    pub include_frame: bool,
}
fn include_frame() -> bool {
    true
}
#[derive(Serialize)]
pub struct Output {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<engine::Frame>,
    pub render: Value,
    pub duckdb_version: String,
    pub timings_ms: BTreeMap<&'static str, f64>,
}
pub fn run(mut request: Request) -> Result<Output> {
    let start = Instant::now();
    let docs = spec::documents(&request.documents)?;
    let chart = docs.get(&request.chart_id).ok_or("Chart not found")?;
    facetra_contract::validate_chart(chart, &docs).map_err(|e| e.to_string())?;
    for value in request.params.values_mut() {
        spec::normalize_numbers(value);
    }
    let params = chart.parameters(&request.params)?;
    let dataset = spec::resolve(chart, &docs, &params)?;
    let parsed = start.elapsed();
    let mut engine = engine::Engine::new()?;
    engine.import(
        &dataset
            .sources
            .iter()
            .map(|(k, s)| (k.clone(), s.spec.clone()))
            .collect(),
        &request.directory,
    )?;
    let imported = start.elapsed();
    let frame = engine.query(dataset.sql.as_deref(), &dataset.params)?;
    let queried = start.elapsed();
    let cancel = runtime::Cancellation::default();
    let render = render::payload(
        &chart.chart,
        Some(&frame),
        &params,
        params.keys().cloned().collect(),
        &cancel,
        chart.i18n.as_ref(),
        request.locale.as_deref().unwrap_or("en"),
    )?;
    let prepared = start.elapsed();
    Ok(Output {
        frame: request.include_frame.then_some(frame),
        render,
        duckdb_version: engine.version()?,
        timings_ms: BTreeMap::from([
            ("yaml_and_resolve", parsed.as_secs_f64() * 1000.0),
            (
                "open_and_import",
                (imported - parsed).as_secs_f64() * 1000.0,
            ),
            (
                "query_and_frame",
                (queried - imported).as_secs_f64() * 1000.0,
            ),
            (
                "render_prepare",
                (prepared - queried).as_secs_f64() * 1000.0,
            ),
        ]),
    })
}
pub fn run_json(input: &str) -> String {
    let start = Instant::now();
    let result = std::panic::catch_unwind(|| {
        if input.len() > 8 * 1024 * 1024 {
            return Err("Request exceeds 8 MiB".into());
        }
        let request: Request = serde_json::from_str(input).map_err(|e| e.to_string())?;
        let decode_ms = start.elapsed().as_secs_f64() * 1000.0;
        run(request).map(|output| (output, decode_ms))
    });
    match result {
        Ok(Ok((output, decode_ms))) => {
            let start = Instant::now();
            // Serialize directly: json!({"ok": output}) otherwise copies the
            // entire frame and payload into a second serde_json value tree.
            let body = serde_json::to_string(&output).expect("Serializable output");
            let encode_ms = start.elapsed().as_secs_f64() * 1000.0;
            format!("{{\"ok\":{body},\"boundary_ms\":{{\"request_decode\":{decode_ms},\"response_encode\":{encode_ms}}}}}")
        }
        Ok(Err(error)) => serde_json::json!({"error": error}).to_string(),
        Err(_) => serde_json::json!({"error": "Rust pipeline panicked"}).to_string(),
    }
}
/// Runs one synchronous request. Call on a worker thread, not the UI thread.
///
/// # Safety
/// `input` must be null or a valid, NUL-terminated UTF-8 C string for this call.
/// The returned pointer must be freed exactly once with `facetra_pipeline_free`.
#[no_mangle]
pub unsafe extern "C" fn facetra_pipeline_run(input: *const c_char) -> *mut c_char {
    let response = if input.is_null() {
        serde_json::json!({"error":"Null request"}).to_string()
    } else {
        match unsafe { CStr::from_ptr(input) }.to_str() {
            Ok(input) => run_json(input),
            Err(_) => serde_json::json!({"error":"Request must be UTF-8"}).to_string(),
        }
    };
    CString::new(response)
        .expect("JSON has no literal NUL")
        .into_raw()
}
/// # Safety
/// Pass null or an unfreed pointer returned by `facetra_pipeline_run`.
#[no_mangle]
pub unsafe extern "C" fn facetra_pipeline_free(output: *mut c_char) {
    if !output.is_null() {
        drop(unsafe { CString::from_raw(output) });
    }
}

/// Pure contract ABI; shares the existing owned-string allocator with the App.
/// Input and output have the same ownership rules as facetra_pipeline_run.
///
/// # Safety
/// `input` must be null or a valid NUL-terminated UTF-8 string for the call.
/// Free the returned string exactly once with `facetra_pipeline_free`.
#[no_mangle]
pub unsafe extern "C" fn facetra_contract_run(input: *const c_char) -> *mut c_char {
    let output = std::panic::catch_unwind(|| {
        let text = if input.is_null() {
            ""
        } else {
            CStr::from_ptr(input).to_str().unwrap_or("")
        };
        facetra_contract::run_json(text)
    })
    .unwrap_or_else(|_| r#"{"error":{"code":"contract.internal","path":""}}"#.into());
    CString::new(output).unwrap().into_raw()
}
