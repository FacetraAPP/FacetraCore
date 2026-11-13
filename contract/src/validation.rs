use crate::{models::*, Result};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub fn valid_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 80
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
    {
        return Err(format!("Invalid ID: {id}"));
    }
    Ok(())
}
pub fn data_path(path: &str) -> Result<()> {
    let segments: Vec<_> = path.split('/').collect();
    if segments.len() < 2
        || segments[0] != "data"
        || path.contains('\\')
        || segments.iter().any(|s| s.is_empty() || s.starts_with('.'))
    {
        return Err("CSV must be a relative path inside data/".into());
    }
    Ok(())
}
pub(crate) fn scalar(value: &Value) -> bool {
    value.is_string() || value.is_boolean() || value.as_i64().is_some() || value.as_f64().is_some()
}
// JSONDecoder's Value tries Int before Double, accepting an integral 12.0 as
// Int. Mirror that rule for YAML defaults, options and boundary parameters.
pub fn normalize_numbers(value: &mut Value) {
    match value {
        Value::Array(values) => values.iter_mut().for_each(normalize_numbers),
        Value::Object(values) => values.values_mut().for_each(normalize_numbers),
        Value::Number(number) if number.is_f64() => {
            if let Some(n) = number.as_f64() {
                if n.fract() == 0.0 && n >= i64::MIN as f64 && n < 9223372036854775808.0 {
                    *value = Value::from(n as i64);
                }
            }
        }
        _ => {}
    }
}
impl Parameter {
    pub fn validate(&self, value: &Value) -> Result<()> {
        let matches = match self.kind.as_str() {
            "string" | "date" => value.is_string(),
            "boolean" => value.is_boolean(),
            "integer" => value.as_i64().is_some(),
            "number" => value.is_number(),
            _ => false,
        };
        if !matches {
            return Err(format!("Parameter {} requires {}", self.name, self.kind));
        }
        if self.required && value.as_str().is_some_and(|s| s.trim().is_empty()) {
            return Err(format!("Empty required parameter: {}", self.name));
        }
        if let ("date", Some(text)) = (self.kind.as_str(), value.as_str()) {
            if !text.is_empty() || self.required {
                let pattern = "%Y-%m-%d";
                let valid = text.len() == 10
                    && !text.starts_with("0000")
                    && chrono::NaiveDate::parse_from_str(text, pattern)
                        .is_ok_and(|date| date.format(pattern).to_string() == text);
                if !valid {
                    return Err(format!("Invalid date parameter: {}", self.name));
                }
            }
        }
        if let Some(n) = value.as_f64() {
            if !n.is_finite()
                || self.minimum.is_some_and(|m| n < m)
                || self.maximum.is_some_and(|m| n > m)
            {
                return Err(format!("Parameter out of range: {}", self.name));
            }
        }
        if self.options.as_ref().is_some_and(|options| {
            let ParameterOptions::Values(options) = options else {
                return false;
            };
            !options.iter().any(|option| {
                if self.kind == "number" {
                    option.as_f64() == value.as_f64()
                } else {
                    option == value
                }
            })
        }) {
            return Err(format!("Parameter outside options: {}", self.name));
        }
        Ok(())
    }
}
impl Source {
    pub fn validate(&self) -> Result<()> {
        validate_provider(self)
    }
}
impl Document {
    pub fn parameters(&self, inputs: &BTreeMap<String, Value>) -> Result<BTreeMap<String, Value>> {
        parameters(&self.params, inputs)
    }
    pub fn validate(&self) -> Result<()> {
        crate::localization::validate(&serde_json::to_value(self).map_err(|_| "Invalid document")?)
            .map_err(|e| e.to_string())?;
        crate::structure(
            &serde_json::to_value(self).map_err(|_| "Invalid document")?,
            "chart",
        )
        .map_err(|_| "Invalid contract structure")?;
        validate_tags(&self.metadata._tags)?;
        if self.chart.dataset != self.dataset.id {
            return Err("chart.dataset must match dataset.id".into());
        }
        validate_definitions(&self.params)?;
        for source in self.sources.values() {
            source.validate()?;
        }
        let mut names = BTreeSet::new();
        for input in self.dataset.sources.iter().flatten() {
            if !names.insert(input.id.to_lowercase()) {
                return Err("Duplicate input ID".into());
            }
        }
        Ok(())
    }
}

impl HttpSpec {
    pub fn validate(&self) -> Result<()> {
        let method = self.method.as_deref().unwrap_or("get").to_ascii_lowercase();
        if self.url.is_empty()
            || !["get", "post"].contains(&method.as_str())
            || (self.json.is_some() && self.data.is_some())
            || (method == "get" && (self.json.is_some() || self.data.is_some()))
            || !["json", "csv", "html", "xml"].contains(&self.format.as_deref().unwrap_or("json"))
            || (!self.records_path.is_empty() && self.format.as_deref().unwrap_or("json") != "json")
            || self
                .params
                .values()
                .chain(self.data.iter().flat_map(|m| m.values()))
                .any(|v| !v.is_string() && !v.is_number() && !v.is_boolean())
        {
            return Err("Invalid HTTP source definition".into());
        }
        if !self.url.contains("{{") {
            url(&self.url)?;
        }
        if let Some(p) = &self.pagination {
            if !(1..=1000).contains(&p.max_pages)
                || !self.records_path.is_empty() && p.records.is_some()
            {
                return Err("Invalid HTTP pagination limits or competing record selectors".into());
            }
            crate::expression::Expression::compile(&p.next)?;
            if let Some(records) = &p.records {
                crate::expression::Expression::compile(records)?;
            }
        }
        if let Some(f) = &self.foreach {
            valid_id(&f.source)?;
            if !(1..=10_000).contains(&f.max_items) {
                return Err("Invalid HTTP foreach limit".into());
            }
        }
        let markup = matches!(self.format.as_deref(), Some("html" | "xml"));
        if markup != self.extract.is_some() {
            return Err("HTML/XML sources require extract; JSON/CSV sources cannot use it".into());
        }
        if let Some(e) = &self.extract {
            if e.rows.is_empty()
                || e.rows.len() > 1024
                || e.fields.is_empty()
                || e.fields.len() > 128
            {
                return Err("Invalid HTTP extraction definition".into());
            }
            if e.context.len() > 128 {
                return Err("Too many extraction context fields".into());
            }
            for (name, field) in e.fields.iter().chain(e.context.iter()) {
                valid_id(name)?;
                if field.selector.is_empty()
                    || field.selector.len() > 1024
                    || field.json && field.resolve_url
                {
                    return Err("Invalid HTTP extraction field".into());
                }
            }
            if e.namespaces.len() > 32
                || self.format.as_deref() == Some("html") && !e.namespaces.is_empty()
            {
                return Err("Invalid XML namespaces".into());
            }
        }
        if let Some(l) = &self.limits {
            if !(1..=10_000).contains(&l.max_requests)
                || !(1..=default_bytes()).contains(&l.max_bytes)
                || !(1..=default_rows()).contains(&l.max_rows)
                || !(1..=300_000).contains(&l.timeout_ms)
            {
                return Err("Invalid HTTP acquisition limits".into());
            }
        }
        if let Some(r) = &self.retry {
            if !(1..=5).contains(&r.max_attempts)
                || r.delay_ms > 10_000
                || method == "post" && !r.retry_post
            {
                return Err("HTTP POST retries require an explicit retry_post assertion".into());
            }
        }
        Ok(())
    }
}

pub fn url(text: &str) -> Result<url::Url> {
    let url = url::Url::parse(text).map_err(|_| "Invalid HTTP URL")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err("HTTP requires HTTPS without user info or fragment".into());
    }
    Ok(url)
}

pub fn validate_provider(source: &Source) -> Result<()> {
    let mut dependencies = BTreeSet::new();
    for name in source.depends_on.iter().flatten() {
        valid_id(name)?;
        if !dependencies.insert(name) {
            return Err("Duplicate source dependency".into());
        }
    }
    if dependencies.len() > 32 {
        return Err("Too many source dependencies".into());
    }
    if !crate::template::source_references(source)?
        .iter()
        .all(|name| dependencies.contains(name))
    {
        return Err("Source templates require an explicit depends_on entry".into());
    }
    if source.source.as_deref() == Some("healthkit") {
        return Err(
            "HealthKit sources are no longer supported; use CSV, HTTP or a database query".into(),
        );
    }
    // Validate through the same typed source variants used for Schema generation.
    let value = serde_json::to_value(source).map_err(|_| "Invalid source")?;
    let _: SourceShape = serde_json::from_value(value).map_err(|_| "Invalid source fields")?;
    match source.source.as_deref() {
        Some("duckdb") => data_path(source.file.as_deref().ok_or("Missing CSV path")?)?,
        Some("http") => {
            let spec: HttpSpec =
                serde_json::from_value(serde_json::to_value(&source.provider).unwrap())
                    .map_err(|_| "Invalid HTTP source")?;
            spec.validate()?;
            crate::template::references(source)?;
            fn locals(value: &Value, page: bool, item: bool) -> Result<()> {
                match value {
                    Value::String(s) => {
                        for t in crate::template::tokens(s)? {
                            if t.path[0] == "page" && !page || t.path[0] == "item" && !item {
                                return Err(
                                    "HTTP template local is outside its iteration scope".into()
                                );
                            }
                        }
                    }
                    Value::Array(a) => {
                        for v in a {
                            locals(v, page, item)?;
                        }
                    }
                    Value::Object(o) => {
                        for v in o.values() {
                            locals(v, page, item)?;
                        }
                    }
                    _ => {}
                }
                Ok(())
            }
            for key in ["url", "headers", "params", "json", "data"] {
                if let Some(value) = source.provider.get(key) {
                    locals(value, spec.pagination.is_some(), spec.foreach.is_some())?;
                }
            }
            if let Some(p) = &spec.pagination {
                locals(
                    &serde_json::to_value(&p.initial).unwrap(),
                    false,
                    spec.foreach.is_some(),
                )?;
            }
        }
        Some("database" | "sqlite") => {
            if source.source.as_deref() == Some("sqlite") {
                data_path(&format!(
                    "data/{}",
                    source
                        .provider
                        .get("file_path")
                        .and_then(Value::as_str)
                        .ok_or("Missing SQLite path")?
                ))?;
            }
            if source
                .provider
                .get("sql")
                .and_then(Value::as_str)
                .is_none_or(|s| s.trim().is_empty())
            {
                return Err("Database source requires SQL".into());
            }
            if source.source.as_deref() == Some("database") {
                valid_id(
                    source
                        .provider
                        .get("connection")
                        .and_then(Value::as_str)
                        .ok_or("Missing connection")?,
                )?;
            }
        }
        _ => {}
    }
    Ok(())
}

pub fn parameters(
    definitions: &[Parameter],
    inputs: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>> {
    // Reuse the model's parameter resolver without introducing an execution context.
    resolve_parameters(definitions, inputs)
}
fn resolve_parameters(
    definitions: &[Parameter],
    inputs: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>> {
    if inputs
        .keys()
        .any(|key| !definitions.iter().any(|p| &p.name == key))
    {
        return Err("Undefined parameter".into());
    }
    let mut result = BTreeMap::new();
    for p in definitions {
        if let Some(value) = inputs.get(&p.name).or(p.default.as_ref()) {
            p.validate(value)?;
            result.insert(
                p.name.clone(),
                if p.kind == "number" {
                    Value::from(value.as_f64().ok_or("Invalid numeric parameter")?)
                } else {
                    value.clone()
                },
            );
        } else if p.required {
            return Err("Missing required parameter".into());
        }
    }
    Ok(result)
}

pub fn validate_definitions(definitions: &[Parameter]) -> Result<()> {
    let mut names = BTreeSet::new();
    for p in definitions {
        valid_id(&p.name)?;
        if !names.insert(&p.name)
            || p.label.as_ref().is_some_and(|s| s.is_empty())
            || !["string", "integer", "number", "boolean", "date"].contains(&p.kind.as_str())
            || p.minimum.is_some_and(|m| !m.is_finite())
            || p.maximum.is_some_and(|m| !m.is_finite())
            || p.width.is_some_and(|w| !w.is_finite() || w <= 0.0)
            || p.minimum.zip(p.maximum).is_some_and(|(a, b)| a > b)
        {
            return Err("Invalid parameter definition".into());
        }
        if p.kind == "date" && (p.minimum.is_some() || p.maximum.is_some()) {
            return Err("Date parameters cannot have numeric bounds".into());
        }
        if let Some(value) = &p.default {
            p.validate(value)?;
        }
        if let Some(ParameterOptions::Values(values)) = &p.options {
            for value in values {
                p.validate(value)?;
            }
        }
        if let Some(ParameterOptions::Source(options)) = &p.options {
            if options.value.trim().is_empty()
                || options.label.as_ref().is_some_and(|s| s.trim().is_empty())
            {
                return Err("Option columns cannot be empty".into());
            }
        }
    }
    Ok(())
}

pub fn validate_tags(tags: &[String]) -> Result<()> {
    for tag in tags {
        if tag.trim().is_empty()
            || tag.trim().chars().count() > 64
            || tag.chars().any(char::is_control)
        {
            return Err("Invalid metadata tags".into());
        }
    }
    Ok(())
}
