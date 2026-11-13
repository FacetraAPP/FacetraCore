use crate::{engine::Frame, runtime::Cancellation, spec::Chart, Result};
use chrono::{DateTime, NaiveDate};
use serde::Serialize;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Serialize)]
struct Payload {
    bars: Vec<Bar>,
    timeframe: String,
    #[serde(rename = "showVolume")]
    show_volume: bool,
    indicators: Vec<Indicator>,
}

#[derive(Serialize)]
struct Bar {
    time: f64,
    open: f64,
    high: f64,
    low: f64,
    close: f64,
    volume: Option<f64>,
}

#[derive(Serialize)]
struct Indicator {
    #[serde(rename = "type")]
    kind: String,
    inputs: BTreeMap<String, Value>,
}

fn fail(message: impl Into<String>) -> String {
    format!("Vela: {}", message.into())
}

fn allowed_keys(object: &Map<String, Value>, allowed: &[&str]) -> Result<()> {
    if let Some(key) = object
        .keys()
        .find(|key| !allowed.contains(&key.as_str()))
        .cloned()
    {
        return Err(fail(format!("Unsupported field: {key}.")));
    }
    Ok(())
}

fn text<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| fail(format!("{key} requires a nonempty string.")))
}

fn valid_timeframe(value: &str) -> bool {
    let bytes = value.as_bytes();
    let Some(unit) = bytes.last() else {
        return false;
    };
    if !matches!(unit, b'm' | b'h' | b'd' | b'w') {
        return false;
    }
    let digits = &bytes[..bytes.len() - 1];
    !digits.is_empty()
        && digits.len() <= 6
        && digits[0] != b'0'
        && digits.iter().all(u8::is_ascii_digit)
}

fn has_zone(value: &str) -> bool {
    if value.ends_with('Z') {
        return true;
    }
    let bytes = value.as_bytes();
    bytes.len() >= 6
        && matches!(bytes[bytes.len() - 6], b'+' | b'-')
        && bytes[bytes.len() - 5..bytes.len() - 3]
            .iter()
            .all(u8::is_ascii_digit)
        && bytes[bytes.len() - 3] == b':'
        && bytes[bytes.len() - 2..].iter().all(u8::is_ascii_digit)
}

fn milliseconds(value: &str, field: &str, row: usize) -> Result<f64> {
    if value.len() == 10 {
        let date = NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .map_err(|_| fail(format!("Row {}, invalid ISO time in {field}.", row + 1)))?;
        return Ok(date
            .and_hms_opt(0, 0, 0)
            .expect("midnight is valid")
            .and_utc()
            .timestamp_millis() as f64);
    }
    let mut normalized = value.replace(' ', "T");
    if !has_zone(&normalized) {
        normalized.push('Z');
    }
    let parsed = DateTime::parse_from_rfc3339(&normalized)
        .map_err(|_| fail(format!("Row {}, invalid ISO time in {field}.", row + 1)))?;
    Ok(parsed.timestamp_millis() as f64)
}

fn number(row: &[Value], index: usize, name: &str, row_index: usize) -> Result<f64> {
    row[index]
        .as_f64()
        .filter(|value| value.is_finite() && value.abs() <= 1e100)
        .ok_or_else(|| {
            fail(format!(
                "Row {}, {name}: requires a finite numeric SQL value.",
                row_index + 1
            ))
        })
}

pub(crate) fn compile(
    chart: &Chart,
    frame: Option<&Frame>,
    params: &BTreeMap<String, Value>,
    parameters: &BTreeSet<String>,
    cancel: &Cancellation,
) -> Result<Value> {
    if chart.renderer != "vela" || chart.scripts.is_some() {
        return Err(fail("renderer: vela requires an option object."));
    }
    let option = chart
        .option
        .as_ref()
        .and_then(Value::as_object)
        .ok_or_else(|| fail("renderer: vela requires an option object."))?;
    allowed_keys(
        option,
        &[
            "time",
            "open",
            "high",
            "low",
            "close",
            "volume",
            "time_unit",
            "timeframe",
            "show_volume",
            "indicators",
        ],
    )?;
    let names = ["time", "open", "high", "low", "close"]
        .map(|key| text(option, key))
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    let volume = option
        .get("volume")
        .map(|_| text(option, "volume"))
        .transpose()?;
    let unit = option
        .get("time_unit")
        .map(|_| text(option, "time_unit"))
        .transpose()?
        .unwrap_or("iso8601");
    if !["iso8601", "seconds", "milliseconds"].contains(&unit) {
        return Err(fail("time_unit must be iso8601, seconds or milliseconds."));
    }
    let timeframe = option
        .get("timeframe")
        .map(|_| text(option, "timeframe"))
        .transpose()?
        .unwrap_or("1d");
    if !valid_timeframe(timeframe) {
        return Err(fail(
            "timeframe must be a positive interval such as 15m, 1h or 1d.",
        ));
    }
    let mut show_volume = volume.is_some();
    if let Some(flag) = option.get("show_volume") {
        show_volume = flag
            .as_bool()
            .ok_or_else(|| fail("show_volume must be boolean."))?;
    }
    if show_volume && volume.is_none() {
        return Err(fail("show_volume requires a volume field."));
    }
    let mut indicators = Vec::new();
    if let Some(value) = option.get("indicators") {
        let items = value
            .as_array()
            .filter(|items| items.len() <= 16)
            .ok_or_else(|| fail("indicators must be a list of at most 16 studies."))?;
        for item in items {
            let spec = item
                .as_object()
                .ok_or_else(|| fail("Each indicator requires type and optional inputs."))?;
            allowed_keys(spec, &["type", "inputs"])?;
            let kind = text(spec, "type")?.to_string();
            let mut inputs = BTreeMap::new();
            if let Some(value) = spec.get("inputs") {
                let values = value
                    .as_object()
                    .ok_or_else(|| fail("indicator.inputs must be an object."))?;
                for (key, value) in values {
                    if value.is_boolean() || value.is_number() || value.is_string() {
                        inputs.insert(key.clone(), value.clone());
                        continue;
                    }
                    let binding = value.as_object().ok_or_else(|| {
                        fail("Indicator inputs must be scalar values or parameter bindings.")
                    })?;
                    allowed_keys(binding, &["from", "name"])?;
                    let name = text(binding, "name")?;
                    if binding.get("from").and_then(Value::as_str) != Some("params")
                        || !parameters.contains(name)
                    {
                        return Err(fail(format!(
                            "Indicator input must reference a defined parameter: {name}"
                        )));
                    }
                    if let Some(resolved) = params.get(name) {
                        inputs.insert(key.clone(), resolved.clone());
                    } else if frame.is_some() {
                        return Err(fail(format!("Missing indicator parameter: {name}.")));
                    }
                }
            }
            indicators.push(Indicator { kind, inputs });
        }
    }
    let mut payload = Payload {
        bars: Vec::new(),
        timeframe: timeframe.into(),
        show_volume,
        indicators,
    };
    let Some(frame) = frame else {
        return serde_json::to_value(payload).map_err(|e| e.to_string());
    };
    let index = |name: &str| {
        frame
            .columns
            .iter()
            .position(|column| column.name == name)
            .ok_or_else(|| fail(format!("Data field not found: {name}.")))
    };
    let indices = names
        .iter()
        .map(|name| index(name))
        .collect::<Result<Vec<_>>>()?;
    let volume_index = volume.map(index).transpose()?;
    let mut bars = Vec::with_capacity(frame.rows.len());
    for (row_index, row) in frame.rows.iter().enumerate() {
        cancel.check()?;
        let time = if unit == "iso8601" {
            let raw = row[indices[0]].as_str().ok_or_else(|| {
                fail(format!(
                    "{} requires ISO dates/timestamps; set time_unit for numeric epochs.",
                    names[0]
                ))
            })?;
            milliseconds(raw, names[0], row_index)?
        } else {
            number(row, indices[0], names[0], row_index)?
                * if unit == "seconds" { 1000.0 } else { 1.0 }
        };
        if !time.is_finite() || time.abs() >= 8.64e15 || time.fract() != 0.0 {
            return Err(fail(
                "Time must resolve to a whole, valid epoch millisecond.",
            ));
        }
        let open = number(row, indices[1], names[1], row_index)?;
        let high = number(row, indices[2], names[2], row_index)?;
        let low = number(row, indices[3], names[3], row_index)?;
        let close = number(row, indices[4], names[4], row_index)?;
        if high < open.max(close) || low > open.min(close) || high < low {
            return Err(fail(format!(
                "Row {}: high/low do not contain open/close.",
                row_index + 1
            )));
        }
        let count = volume_index
            .map(|index| number(row, index, "volume", row_index))
            .transpose()?;
        if count.is_some_and(|value| value < 0.0) {
            return Err(fail(format!(
                "Row {}: volume cannot be negative.",
                row_index + 1
            )));
        }
        bars.push(Bar {
            time,
            open,
            high,
            low,
            close,
            volume: count,
        });
    }
    bars.sort_by(|left, right| left.time.total_cmp(&right.time));
    if bars.windows(2).any(|pair| pair[0].time == pair[1].time) {
        return Err(fail(
            "Duplicate candle time. Filter to one symbol and aggregate to one row per interval in dataset.sql.",
        ));
    }
    payload.bars = bars;
    serde_json::to_value(payload).map_err(|e| e.to_string())
}
