use crate::{
    engine::{Column, Engine, Frame},
    runtime::{Cancellation, Input},
    Result, MAX_BYTES, MAX_ROWS,
};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::OnceLock,
};

pub struct SourceData {
    pub bytes: Option<Vec<u8>>,
    pub format: String,
    pub records_path: Vec<String>,
    pub frame: OnceLock<Result<Frame>>,
}
impl SourceData {
    /// Request chains read original JSON, independently of recordsPath/display.
    /// Tabular providers expose an array of records.
    pub fn template_value(&self, cancel: &Cancellation) -> Result<Value> {
        cancel.check()?;
        if self.format == "json" {
            if let Some(bytes) = &self.bytes {
                return serde_json::from_slice(bytes)
                    .map_err(|_| "Invalid upstream JSON response".into());
            }
        }
        let preview = self.preview("json", cancel)?;
        serde_json::from_str(
            preview["text"]
                .as_str()
                .ok_or("Upstream records unavailable")?,
        )
        .map_err(|_| "Invalid upstream records".into())
    }
    pub fn records_value(&self, cancel: &Cancellation) -> Result<Value> {
        cancel.check()?;
        if self.format == "json" {
            if let Some(bytes) = &self.bytes {
                return Ok(Value::Array(json_records(bytes, &self.records_path)?));
            }
        }
        let frame = self.dataframe(cancel)?;
        let records: Vec<Value> = frame
            .rows
            .iter()
            .map(|row| {
                Value::Object(
                    frame
                        .columns
                        .iter()
                        .zip(row)
                        .map(|(c, v)| {
                            let cell = if c.kind == "json" {
                                v.as_str()
                                    .and_then(|s| serde_json::from_str(s).ok())
                                    .unwrap_or_else(|| v.clone())
                            } else {
                                v.clone()
                            };
                            (c.name.clone(), cell)
                        })
                        .collect(),
                )
            })
            .collect();
        Ok(Value::Array(records))
    }
    pub fn raw(bytes: Vec<u8>, format: String, records_path: Vec<String>) -> Result<Self> {
        if !["csv", "json"].contains(&format.as_str()) {
            return Err("Unknown source format".into());
        }
        if bytes.len() as u64 > MAX_BYTES {
            return Err("Source exceeds 100 MiB".into());
        }
        Ok(Self {
            bytes: Some(bytes),
            format,
            records_path,
            frame: OnceLock::new(),
        })
    }
    pub fn table(frame: Frame) -> Result<Self> {
        frame.validate()?;
        Ok(Self {
            bytes: None,
            format: "json".into(),
            records_path: vec![],
            frame: OnceLock::from(Ok(frame)),
        })
    }
    pub fn weight(&self) -> usize {
        self.bytes.as_ref().map_or(0, Vec::len)
            + self
                .frame
                .get()
                .and_then(|r| r.as_ref().ok())
                .map_or(0, |f| {
                    f.rows.iter().flatten().map(|v| text(v).len() + 32).sum()
                })
    }
    pub fn input(&self, cancel: &Cancellation) -> Result<Input> {
        if self.format == "csv" {
            return Ok(Input {
                file: None,
                csv: Some(
                    std::str::from_utf8(self.bytes.as_deref().ok_or("Missing CSV")?)
                        .map_err(|_| "CSV must be UTF-8")?
                        .into(),
                ),
                frame: None,
            });
        }
        Ok(Input {
            file: None,
            csv: None,
            frame: Some(self.dataframe(cancel)?.clone()),
        })
    }
    pub fn dataframe(&self, cancel: &Cancellation) -> Result<&Frame> {
        cancel.check()?;
        // Failures are not memoized: cancellation must not poison later callers.
        if self.frame.get().is_none() {
            let bytes = self.bytes.as_deref().ok_or("Source data unavailable")?;
            let result = if self.format == "csv" {
                let mut engine = Engine::new()?;
                engine.import_inputs(
                    &BTreeMap::from([("raw".into(), self.input(cancel)?)]),
                    "",
                    cancel,
                )?;
                engine.query_cancel(None, &BTreeMap::new(), cancel)?
            } else {
                json_frame(bytes, &self.records_path, cancel)?
            };
            let _ = self.frame.set(Ok(result));
        }
        self.frame.get().unwrap().as_ref().map_err(Clone::clone)
    }
    pub fn preview(&self, display: &str, cancel: &Cancellation) -> Result<Value> {
        cancel.check()?;
        if !["df", "text", "json"].contains(&display) {
            return Err("Unknown source display".into());
        }
        let raw_text = || {
            self.bytes
                .as_ref()
                .map(|b| String::from_utf8_lossy(b).into_owned())
        };
        if display == "text" {
            if let Some(text) = raw_text() {
                return Ok(json!({"kind":"text","text":text}));
            }
        }
        if display == "json" && self.format == "json" {
            if let Some(bytes) = &self.bytes {
                return Ok(match serde_json::from_slice::<Value>(bytes) {
                    Ok(v) => {
                        json!({"kind":"json","text":serde_json::to_string_pretty(&v).map_err(|_| "Invalid JSON")?})
                    }
                    Err(_) => json!({"kind":"text","text":raw_text().unwrap()}),
                });
            }
        }
        let frame = match self.dataframe(cancel) {
            Ok(f) => f,
            Err(e) => {
                cancel.check()?;
                return raw_text()
                    .map(|text| json!({"kind":"text","text":text}))
                    .ok_or(e);
            }
        };
        match display {
            "df" => Ok(json!({"kind":"df","frame":frame})),
            "json" => Ok(
                json!({"kind":"json","text":serde_json::to_string_pretty(&self.records_value(cancel)?).map_err(|_| "Invalid JSON")?}),
            ),
            "text" => {
                fn csv(s: String) -> String {
                    if s.contains([',', '"', '\r', '\n']) {
                        format!("\"{}\"", s.replace('"', "\"\""))
                    } else {
                        s
                    }
                }
                let mut lines = vec![frame
                    .columns
                    .iter()
                    .map(|c| csv(c.name.clone()))
                    .collect::<Vec<_>>()
                    .join(",")];
                lines.extend(frame.rows.iter().map(|row| {
                    row.iter()
                        .map(|v| csv(if v.is_null() { String::new() } else { text(v) }))
                        .collect::<Vec<_>>()
                        .join(",")
                }));
                Ok(json!({"kind":"text","text":lines.join("\n")}))
            }
            _ => Err("Unknown source display".into()),
        }
    }
}
pub fn text(v: &Value) -> String {
    v.as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| v.to_string())
}
pub fn json_frame(bytes: &[u8], path: &[String], cancel: &Cancellation) -> Result<Frame> {
    let records = json_records(bytes, path)?;
    if records.is_empty() {
        return Ok(Frame {
            columns: vec![Column {
                name: "value".into(),
                kind: "string".into(),
            }],
            rows: vec![],
        });
    }
    let names: BTreeSet<_> = records
        .iter()
        .flat_map(|r| r.as_object().unwrap().keys().cloned())
        .collect();
    let mut columns = Vec::new();
    let mut rows = vec![Vec::new(); records.len()];
    for name in names {
        cancel.check()?;
        let nested = records
            .iter()
            .any(|r| r[&name].is_object() || r[&name].is_array());
        let values: Vec<Value> = records
            .iter()
            .map(|r| match &r[&name] {
                Value::Number(n) if n.is_u64() && !n.is_i64() => Value::String(n.to_string()),
                v if v.is_object() || v.is_array() => Value::String(v.to_string()),
                v => v.clone(),
            })
            .collect();
        let cells: Vec<_> = values.iter().filter(|v| !v.is_null()).collect();
        let kind = if !cells.is_empty() && cells.iter().all(|v| v.is_i64()) {
            "integer"
        } else if !cells.is_empty() && cells.iter().all(|v| v.is_number()) {
            "number"
        } else if !cells.is_empty() && cells.iter().all(|v| v.is_boolean()) {
            "boolean"
        } else if nested {
            "json"
        } else {
            "string"
        };
        for (row, v) in rows.iter_mut().zip(values) {
            row.push(if !v.is_null() && ["string", "json"].contains(&kind) {
                Value::String(text(&v))
            } else {
                v
            });
        }
        columns.push(Column {
            name,
            kind: kind.into(),
        });
    }
    let frame = Frame { columns, rows };
    frame.validate()?;
    Ok(frame)
}

fn json_records(bytes: &[u8], path: &[String]) -> Result<Vec<Value>> {
    let mut payload: Value = serde_json::from_slice(bytes).map_err(|_| "Invalid JSON response")?;
    for key in path {
        payload = payload
            .as_object_mut()
            .and_then(|o| o.remove(key))
            .ok_or("recordsPath not found")?;
    }
    let records = match payload {
        Value::Object(o) => vec![Value::Object(o)],
        Value::Array(a) if a.is_empty() => vec![],
        Value::Array(a) if !a.is_empty() && a.iter().all(Value::is_object) => a,
        Value::Array(a)
            if a.iter().all(|v| v.is_null() || v.is_string())
                || a.iter().all(|v| v.is_null() || v.is_number())
                || a.iter().all(|v| v.is_null() || v.is_boolean()) =>
        {
            a.into_iter()
                .map(|v| serde_json::json!({"value": v}))
                .collect()
        }
        _ => return Err("JSON requires an object or a nonempty array of objects".into()),
    };
    if records.len() > MAX_ROWS {
        return Err("Source exceeds 100,000 rows".into());
    }
    Ok(records)
}
