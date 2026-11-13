use crate::{
    echarts,
    engine::Frame,
    spec::{Chart, MessageCatalog},
    Result,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Client-neutral renderer payload. Rendering engines only receive this JSON.
pub(crate) fn payload(
    chart: &Chart,
    frame: Option<&Frame>,
    params: &BTreeMap<String, Value>,
    parameters: BTreeSet<String>,
    cancel: &crate::runtime::Cancellation,
    catalog: Option<&MessageCatalog>,
    locale: &str,
) -> Result<Value> {
    match chart.renderer.as_str() {
        "echarts" | "html" | "vega_lite" => serde_json::to_value(echarts::compile_with_messages(
            chart, frame, params, parameters, cancel, catalog, locale,
        )?)
        .map_err(|e| e.to_string()),
        "vela" => crate::vela::compile(chart, frame, params, &parameters, cancel),
        "value" => value(chart, frame),
        other => Err(format!("Unknown renderer: {other}")),
    }
}

fn value(chart: &Chart, frame: Option<&Frame>) -> Result<Value> {
    let field = chart
        .value
        .as_deref()
        .filter(|field| !field.trim().is_empty())
        .ok_or("Value card requires chart.value")?;
    let Some(frame) = frame else {
        return Ok(json!({"value": null}));
    };
    if frame.rows.len() != 1 {
        return Err("A value card requires the dataset to return exactly one row".into());
    }
    let index = frame
        .columns
        .iter()
        .position(|column| column.name == field)
        .ok_or_else(|| {
            format!("The field specified by chart.value is missing from the data: {field}")
        })?;
    Ok(json!({"value": frame.rows[0][index].clone()}))
}
