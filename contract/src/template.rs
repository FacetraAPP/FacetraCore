use crate::Result;
use serde_json::Value;
use std::collections::BTreeSet;

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Token {
    pub start: usize,
    pub end: usize,
    pub path: Vec<String>,
    pub date_format: Option<String>,
}
pub fn tokens(text: &str) -> Result<Vec<Token>> {
    let mut rest = 0;
    let mut result = Vec::new();
    while let Some(start) = text[rest..].find("{{").map(|i| i + rest) {
        let end = text[start + 2..]
            .find("}}")
            .map(|i| i + start + 4)
            .ok_or("Unterminated request template")?;
        let clauses: Vec<_> = text[start + 2..end - 2].split('|').map(str::trim).collect();
        if clauses.len() > 2 {
            return Err("Unsupported request template filter".into());
        }
        let date_format = if clauses.len() == 2 {
            let argument = clauses[1]
                .strip_prefix("date_format(")
                .and_then(|s| s.strip_suffix(')'))
                .map(str::trim)
                .ok_or("Unsupported date format filter")?;
            let quote = argument.chars().next().ok_or("Missing date format")?;
            if !['\'', '"'].contains(&quote) || !argument.ends_with(quote) || argument.len() < 2 {
                return Err("Date format must be quoted".into());
            }
            let format = &argument[1..argument.len() - 1];
            if !["yyyyMMdd", "yyyy-MM-dd"].contains(&format) || !clauses[0].starts_with("params.") {
                return Err("Date format filters require a parameter and supported format".into());
            }
            Some(format.to_owned())
        } else {
            None
        };
        let path: Vec<_> = clauses[0].split('.').map(str::to_owned).collect();
        if !(matches!(
            (path[0].as_str(), path.len()),
            ("params", 2) | ("connections", 3)
        ) || (path[0] == "sources" && path.len() >= 2)
            || (["page", "item"].contains(&path[0].as_str())))
            || path[1..]
                .iter()
                .any(|s| crate::validation::valid_id(s).is_err())
        {
            return Err(
                "Request templates support params.name / connections.id.key / sources.name.path / page.path / item.path"
                    .into(),
            );
        }
        result.push(Token {
            start,
            end,
            path,
            date_format,
        });
        rest = end;
    }
    let mut stripped = text.to_owned();
    for token in result.iter().rev() {
        stripped.replace_range(token.start..token.end, "");
    }
    if stripped.contains("}}") || stripped.contains("{%") {
        return Err("Unsupported request template expression".into());
    }
    Ok(result)
}
pub type References = (BTreeSet<String>, BTreeSet<(String, String)>);
pub fn references(spec: &crate::models::Source) -> Result<References> {
    let mut params = BTreeSet::new();
    let mut connections = BTreeSet::new();
    fn walk(
        v: &Value,
        params: &mut BTreeSet<String>,
        connections: &mut BTreeSet<(String, String)>,
    ) -> Result<()> {
        match v {
            Value::String(s) => {
                for t in tokens(s)? {
                    if t.path[0] == "params" {
                        params.insert(t.path[1].clone());
                    } else if t.path[0] == "connections" {
                        connections.insert((t.path[1].clone(), t.path[2].clone()));
                    }
                }
            }
            Value::Array(a) => {
                for v in a {
                    walk(v, params, connections)?;
                }
            }
            Value::Object(o) => {
                for v in o.values() {
                    walk(v, params, connections)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    for key in ["url", "headers", "params", "json", "data"] {
        if let Some(v) = spec.provider.get(key) {
            walk(v, &mut params, &mut connections)?;
        }
    }
    if let Some(initial) = spec
        .provider
        .get("pagination")
        .and_then(|p| p.get("initial"))
    {
        walk(initial, &mut params, &mut connections)?;
    }
    Ok((params, connections))
}

/// Inspect only request values, so unrelated source metadata cannot add edges.
pub fn source_references(spec: &crate::models::Source) -> Result<BTreeSet<String>> {
    fn walk(value: &Value, names: &mut BTreeSet<String>) -> Result<()> {
        match value {
            Value::String(s) => {
                for token in tokens(s)? {
                    if token.path[0] == "sources" {
                        names.insert(token.path[1].clone());
                    }
                }
            }
            Value::Array(a) => {
                for value in a {
                    walk(value, names)?;
                }
            }
            Value::Object(o) => {
                for value in o.values() {
                    walk(value, names)?;
                }
            }
            _ => {}
        }
        Ok(())
    }
    let mut names = BTreeSet::new();
    for key in ["url", "headers", "params", "json", "data"] {
        if let Some(value) = spec.provider.get(key) {
            walk(value, &mut names)?;
        }
    }
    if let Some(initial) = spec
        .provider
        .get("pagination")
        .and_then(|p| p.get("initial"))
    {
        walk(initial, &mut names)?;
    }
    if let Some(name) = spec
        .provider
        .get("foreach")
        .and_then(|f| f.get("source"))
        .and_then(Value::as_str)
    {
        names.insert(name.into());
    }
    Ok(names)
}
