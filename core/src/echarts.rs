use crate::{engine::Frame, spec::Chart, Result};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Serialize)]
pub struct Script {
    pub path: Vec<String>,
    pub source: String,
}
#[derive(Serialize)]
pub struct Payload {
    pub option: Value,
    pub scripts: Vec<Script>,
    pub locale: String,
}
struct Compiler<'a> {
    chart: &'a Chart,
    catalog: Option<&'a crate::spec::MessageCatalog>,
    locale: &'a str,
    frame: Option<&'a Frame>,
    params: &'a BTreeMap<String, Value>,
    scripts: Vec<Script>,
    parameters: BTreeSet<String>,
    cancel: &'a crate::runtime::Cancellation,
}
fn text<'a>(object: &'a Map<String, Value>, key: &str) -> Result<&'a str> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("Binding requires {key}"))
}
fn keys(object: &Map<String, Value>, allowed: &[&str]) -> Result<()> {
    if let Some(key) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(format!("Unsupported binding key: {key}"));
    }
    Ok(())
}
fn fields(object: &Map<String, Value>, key: &str) -> Result<Vec<String>> {
    let list = object
        .get(key)
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .ok_or_else(|| format!("Binding requires nonempty {key}"))?;
    list.iter()
        .map(|v| {
            v.as_str()
                .filter(|s| !s.is_empty())
                .map(String::from)
                .ok_or("Field must be a string".into())
        })
        .collect()
}
impl Compiler<'_> {
    fn script(&mut self, source: &str, path: &[String]) -> Result<Value> {
        if source.trim().is_empty() {
            return Err("Empty script expression".into());
        }
        self.scripts.push(Script {
            path: path.to_vec(),
            source: source.into(),
        });
        Ok(Value::Null)
    }
    fn index(&self, field: &str) -> Result<usize> {
        self.frame
            .expect("Frame validated before field lookup")
            .columns
            .iter()
            .position(|c| c.name == field)
            .ok_or_else(|| format!("Data field not found: {field}"))
    }
    fn binding(&mut self, object: &Map<String, Value>, path: &[String]) -> Result<Value> {
        let from = text(object, "from")?;
        if from == "script" {
            if self.chart.renderer != "echarts" {
                return Err("Script bindings are only supported by ECharts".into());
            }
            keys(object, &["from", "name"])?;
            let name = text(object, "name")?;
            let source = self
                .chart
                .scripts
                .as_ref()
                .and_then(|s| s.get(name))
                .ok_or("Unknown script")?;
            return self.script(source, path);
        }
        if from == "params" {
            keys(object, &["from", "name"])?;
            let name = text(object, "name")?;
            if !self.parameters.contains(name) {
                return Err(format!("Unknown parameter: {name}"));
            }
            return Ok(self.params.get(name).cloned().unwrap_or(Value::Null));
        }
        if from != self.chart.dataset {
            return Err("Bindings must reference chart.dataset".into());
        }
        keys(
            object,
            &[
                "from",
                "field",
                "fields",
                "x",
                "y",
                "rows",
                "color_by_sign",
                "color_by",
            ],
        )?;
        let row_objects = object.contains_key("rows");
        if row_objects && object["rows"] != Value::Bool(true) {
            return Err("Use rows: true".into());
        }
        let modes = [
            row_objects,
            object.contains_key("field"),
            object.contains_key("fields"),
            object.contains_key("x") || object.contains_key("y"),
        ];
        if modes.into_iter().filter(|m| *m).count() != 1 {
            return Err("Choose one binding mode: field, fields, x/y, rows".into());
        }
        let selected = if row_objects {
            self.frame
                .map(|f| f.columns.iter().map(|c| c.name.clone()).collect())
                .unwrap_or_default()
        } else if object.contains_key("field") {
            vec![text(object, "field")?.into()]
        } else if object.contains_key("fields") {
            fields(object, "fields")?
        } else {
            vec![text(object, "x")?.into(), text(object, "y")?.into()]
        };
        let by_sign = match object.get("color_by_sign") {
            None => false,
            Some(Value::Bool(v)) => *v,
            _ => return Err("color_by_sign must be boolean".into()),
        };
        let color_by = if object.contains_key("color_by") {
            Some(fields(object, "color_by")?)
        } else {
            None
        };
        if color_by.as_ref().is_some_and(|f| f.len() != 2)
            || (by_sign && color_by.is_some())
            || (row_objects && (by_sign || color_by.is_some()))
        {
            return Err("Color binding requires sign or two numeric fields".into());
        }
        let Some(frame) = self.frame else {
            return Ok(Value::Array(vec![]));
        };
        let indices = selected
            .iter()
            .map(|s| self.index(s))
            .collect::<Result<Vec<_>>>()?;
        let color_indices = color_by
            .map(|fields| {
                fields
                    .iter()
                    .map(|s| self.index(s))
                    .collect::<Result<Vec<_>>>()
            })
            .transpose()?;
        let mut values = Vec::with_capacity(frame.rows.len());
        for row in &frame.rows {
            self.cancel.check()?;
            let cells: Vec<_> = indices.iter().map(|i| row[*i].clone()).collect();
            let value = if row_objects {
                Value::Object(selected.iter().cloned().zip(cells).collect())
            } else if object.contains_key("field") {
                cells[0].clone()
            } else {
                Value::Array(cells)
            };
            if by_sign || color_indices.is_some() {
                let compared = color_indices
                    .clone()
                    .unwrap_or_else(|| vec![*indices.last().expect("Nonempty field selection")]);
                let numbers: Vec<Option<f64>> = compared
                    .iter()
                    .map(|i| {
                        if row[*i].is_null() {
                            Ok(None)
                        } else {
                            row[*i]
                                .as_f64()
                                .map(Some)
                                .ok_or("Colors require numeric SQL fields".into())
                        }
                    })
                    .collect::<Result<_>>()?;
                if numbers.iter().any(Option::is_none) {
                    values.push(json!({"value":value}));
                } else {
                    let up = numbers[0].unwrap()
                        >= if numbers.len() == 2 {
                            numbers[1].unwrap()
                        } else {
                            0.0
                        };
                    values.push(json!({"value":value,"itemStyle":{"color":if up {"#26a69a"} else {"#ef5350"}}}));
                }
            } else {
                values.push(value);
            }
        }
        // Return inserted data directly: a CSV cell beginning with js: is data.
        Ok(Value::Array(values))
    }
    fn walk(&mut self, value: &Value, path: &[String], depth: usize) -> Result<Value> {
        self.cancel.check()?;
        if depth >= 64 {
            return Err("ECharts option is too deeply nested".into());
        }
        match value {
            Value::Object(object) if object.contains_key("i18n") => {
                keys(object, &["i18n"])?;
                let key = text(object, "i18n")?;
                let text = self
                    .catalog
                    .and_then(|catalog| catalog.text(key, self.locale))
                    .ok_or("Missing i18n message")?;
                Ok(Value::String(text.into()))
            }
            Value::Object(object)
                if object.contains_key("from")
                    && (self.chart.renderer == "echarts" || object["from"].is_string()) =>
            {
                self.binding(object, path)
            }
            Value::Object(object) => {
                let mut result = Map::new();
                let mut keys: Vec<_> = object.keys().collect();
                keys.sort();
                for key in keys {
                    let mut child = path.to_vec();
                    child.push(key.clone());
                    result.insert(key.clone(), self.walk(&object[key], &child, depth + 1)?);
                }
                Ok(Value::Object(result))
            }
            Value::Array(array) => {
                let mut result = Vec::new();
                for (index, value) in array.iter().enumerate() {
                    let mut child = path.to_vec();
                    child.push(index.to_string());
                    result.push(self.walk(value, &child, depth + 1)?);
                }
                Ok(Value::Array(result))
            }
            Value::String(text) if self.chart.renderer == "echarts" && text.starts_with("js:") => {
                self.script(&text[3..], path)
            }
            _ => Ok(value.clone()),
        }
    }
}
pub(crate) fn compile_with_messages(
    chart: &Chart,
    frame: Option<&Frame>,
    params: &BTreeMap<String, Value>,
    parameters: BTreeSet<String>,
    cancel: &crate::runtime::Cancellation,
    catalog: Option<&crate::spec::MessageCatalog>,
    locale: &str,
) -> Result<Payload> {
    if ["params", "script"].contains(&chart.dataset.as_str()) {
        return Err("Reserved dataset ID".into());
    }
    let option = chart
        .option
        .as_ref()
        .filter(|o| o.as_object().is_some_and(|o| !o.is_empty()))
        .ok_or("ECharts requires nonempty option object")?;
    if chart.renderer != "echarts" && chart.scripts.is_some() {
        return Err("This renderer does not accept chart.scripts".into());
    }
    if chart.renderer == "html" {
        let object = option.as_object().unwrap();
        keys(object, &["html", "css", "data"])?;
        if object
            .get("html")
            .and_then(Value::as_str)
            .is_none_or(|v| v.trim().is_empty())
        {
            return Err("HTML requires nonempty option.html".into());
        }
        for key in ["css"] {
            if object.get(key).is_some_and(|v| !v.is_string()) {
                return Err(format!("HTML option.{key} must be text"));
            }
        }
    }
    if chart.renderer == "vega_lite"
        && ![
            "mark", "layer", "facet", "hconcat", "vconcat", "concat", "repeat",
        ]
        .iter()
        .any(|k| option.get(k).is_some())
    {
        return Err("Vega-Lite requires a mark or a composed view".into());
    }
    for (name, source) in chart.scripts.iter().flatten() {
        let mut chars = name.chars();
        if !chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_' || c == '$')
            || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
            || source.trim().is_empty()
        {
            return Err("Invalid script definition".into());
        }
    }
    let mut compiler = Compiler {
        chart,
        catalog,
        locale,
        frame,
        params,
        scripts: vec![],
        parameters,
        cancel,
    };
    let option = compiler.walk(option, &[], 0)?;
    validate_pie(&option)?;
    Ok(Payload {
        option,
        scripts: compiler.scripts,
        locale: if locale.to_ascii_lowercase().starts_with("zh") {
            "ZH"
        } else {
            "EN"
        }
        .into(),
    })
}

fn validate_pie(option: &Value) -> Result<()> {
    fn value(item: &Value) -> Option<f64> {
        match item {
            Value::Number(number) => number.as_f64(),
            Value::String(text) => text.parse().ok(),
            Value::Object(object) => object.get("value").and_then(value),
            Value::Array(values) => values.last().and_then(value),
            _ => None,
        }
    }
    fn inspect(node: &Value) -> Result<()> {
        match node {
            Value::Object(object) => {
                if object.get("type").and_then(Value::as_str) == Some("pie") {
                    if let Some(Value::Array(items)) = object.get("data") {
                        let mut total = 0.0;
                        let mut seen = false;
                        for item in items {
                            if let Some(number) = value(item) {
                                seen = true;
                                if !number.is_finite() || number < 0.0 {
                                    return Err(
                                        "Pie charts require nonnegative finite values".into()
                                    );
                                }
                                total += number;
                            }
                        }
                        if seen && !(total.is_finite() && total > 0.0) {
                            return Err("Pie charts require at least one positive value".into());
                        }
                    }
                }
                for value in object.values() {
                    inspect(value)?;
                }
            }
            Value::Array(values) => {
                for value in values {
                    inspect(value)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    inspect(option)
}
