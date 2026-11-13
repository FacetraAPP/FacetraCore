use crate::Result;
pub use facetra_contract::template::{references, tokens, Token};
use serde_json::Value;
use std::collections::BTreeMap;

#[cfg(test)]
pub fn render(
    text: &str,
    params: &BTreeMap<String, Value>,
    connections: &BTreeMap<String, super::Connection>,
) -> Result<Value> {
    render_with_sources(text, params, connections, &BTreeMap::new())
}
#[cfg(test)]
pub fn render_with_sources(
    text: &str,
    params: &BTreeMap<String, Value>,
    connections: &BTreeMap<String, super::Connection>,
    sources: &BTreeMap<String, Value>,
) -> Result<Value> {
    render_with_locals(text, params, connections, sources, &BTreeMap::new())
}
pub fn render_with_locals(
    text: &str,
    params: &BTreeMap<String, Value>,
    connections: &BTreeMap<String, super::Connection>,
    sources: &BTreeMap<String, Value>,
    locals: &BTreeMap<String, Value>,
) -> Result<Value> {
    let tokens = tokens(text)?;
    let value = |token: &Token| -> Result<Value> {
        let raw = if token.path[0] == "params" {
            params
                .get(&token.path[1])
                .cloned()
                .ok_or_else(|| format!("Missing request parameter: {}", token.path[1]))?
        } else if token.path[0] == "sources" {
            let mut value = sources
                .get(&token.path[1])
                .ok_or("Missing upstream source result")?;
            for segment in &token.path[2..] {
                value = match value {
                    Value::Object(object) => object.get(segment),
                    Value::Array(array) => segment.parse::<usize>().ok().and_then(|i| array.get(i)),
                    _ => None,
                }
                .ok_or("Upstream source result path not found")?;
            }
            value.clone()
        } else if ["page", "item"].contains(&token.path[0].as_str()) {
            let mut value = locals
                .get(&token.path[0])
                .ok_or("Request local is outside its iteration scope")?;
            for segment in &token.path[1..] {
                value = match value {
                    Value::Object(o) => o.get(segment),
                    Value::Array(a) => segment.parse::<usize>().ok().and_then(|i| a.get(i)),
                    _ => None,
                }
                .ok_or("Request local path not found")?;
            }
            value.clone()
        } else {
            connections
                .get(&token.path[1])
                .and_then(|c| c.fields.get(&token.path[2]))
                .filter(|s| !s.is_empty())
                .map(|s| Value::String(s.clone()))
                .ok_or_else(|| {
                    format!(
                        "Missing connection field: {}.{}",
                        token.path[1], token.path[2]
                    )
                })?
        };
        if let Some(format) = &token.date_format {
            let text = raw
                .as_str()
                .ok_or("Date formatting requires an ISO date string")?;
            let date = chrono::NaiveDate::parse_from_str(text, "%Y-%m-%d")
                .map_err(|_| "Date formatting requires a valid ISO date")?;
            if text.len() != 10
                || text.starts_with("0000")
                || date.format("%Y-%m-%d").to_string() != text
            {
                return Err("Date formatting requires a valid ISO date".into());
            }
            let pattern = if format == "yyyyMMdd" {
                "%Y%m%d"
            } else {
                "%Y-%m-%d"
            };
            return Ok(Value::String(date.format(pattern).to_string()));
        }
        Ok(raw)
    };
    if tokens.len() == 1 && tokens[0].start == 0 && tokens[0].end == text.len() {
        return value(&tokens[0]);
    }
    let mut output = text.to_owned();
    for token in tokens.iter().rev() {
        output.replace_range(token.start..token.end, &super::data::text(&value(token)?));
    }
    Ok(Value::String(output))
}
#[cfg(test)]
pub fn render_value(
    value: &Value,
    params: &BTreeMap<String, Value>,
    connections: &BTreeMap<String, super::Connection>,
) -> Result<Value> {
    render_value_with_sources(value, params, connections, &BTreeMap::new())
}
#[cfg(test)]
pub fn render_value_with_sources(
    value: &Value,
    params: &BTreeMap<String, Value>,
    connections: &BTreeMap<String, super::Connection>,
    sources: &BTreeMap<String, Value>,
) -> Result<Value> {
    render_value_with_locals(value, params, connections, sources, &BTreeMap::new())
}
pub fn render_value_with_locals(
    value: &Value,
    params: &BTreeMap<String, Value>,
    connections: &BTreeMap<String, super::Connection>,
    sources: &BTreeMap<String, Value>,
    locals: &BTreeMap<String, Value>,
) -> Result<Value> {
    Ok(match value {
        Value::String(s) => render_with_locals(s, params, connections, sources, locals)?,
        Value::Array(a) => Value::Array(
            a.iter()
                .map(|v| render_value_with_locals(v, params, connections, sources, locals))
                .collect::<Result<_>>()?,
        ),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| {
                    Ok((
                        k.clone(),
                        render_value_with_locals(v, params, connections, sources, locals)?,
                    ))
                })
                .collect::<Result<_>>()?,
        ),
        v => v.clone(),
    })
}
