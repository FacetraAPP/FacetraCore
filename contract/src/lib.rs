//! Versioned Chart/Dashboard contract. No I/O, engine or platform dependencies.
pub mod diagnostics;
pub mod expression;
pub mod localization;
pub mod models;
pub mod resolution;
pub mod schema;
pub mod sql;
pub mod template;
pub mod validation;
pub mod workspace;
use diagnostics::Diagnostic;
use models::{Dashboard, Document};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use std::collections::BTreeMap;
pub use workspace::{validate_chart, validate_dashboard};
pub type Result<T> = std::result::Result<T, String>;

/// One YAML document, string keys, finite numbers, duplicate keys rejected.
/// Merge keys are expanded before decoding; source text is never rewritten.
pub fn parse(yaml: &str) -> std::result::Result<Value, Diagnostic> {
    let error = || Diagnostic::new("yaml.invalid", "");
    let mut value: serde_yaml_ng::Value = serde_yaml_ng::from_str(yaml).map_err(|e| {
        let mut diagnostic = error();
        if let Some(location) = e.location() {
            diagnostic.line = Some(location.line());
            diagnostic.column = Some(location.column());
        }
        diagnostic
    })?;
    value.apply_merge().map_err(|_| error())?;
    fn walk(v: &serde_yaml_ng::Value) -> bool {
        use serde_yaml_ng::Value as Y;
        match v {
            Y::Mapping(m) => m.iter().all(|(k, v)| k.is_string() && walk(v)),
            Y::Sequence(a) => a.iter().all(walk),
            Y::Tagged(_) => false,
            Y::Number(n) => n.as_f64().is_some_and(f64::is_finite),
            _ => true,
        }
    }
    if !walk(&value) {
        return Err(error());
    }
    let mut value = serde_json::to_value(value).map_err(|_| error())?;
    if !value.is_object() {
        return Err(Diagnostic::new("structure.invalid", ""));
    }
    validation::normalize_numbers(&mut value);
    Ok(value)
}
pub fn decode<T: DeserializeOwned>(value: &Value) -> std::result::Result<T, Diagnostic> {
    serde_path_to_error::deserialize(value).map_err(|e| {
        let path = e
            .path()
            .iter()
            .map(|p| match p {
                serde_path_to_error::Segment::Seq { index } => index.to_string(),
                serde_path_to_error::Segment::Map { key } => {
                    key.replace('~', "~0").replace('/', "~1")
                }
                serde_path_to_error::Segment::Enum { variant } => variant.clone(),
                _ => String::new(),
            })
            .map(|p| format!("/{p}"))
            .collect::<String>();
        Diagnostic::new("structure.invalid", path)
    })
}
pub fn chart(value: &Value) -> std::result::Result<Document, Diagnostic> {
    structure(value, "chart")?;
    let doc: Document = decode(value)?;
    Ok(doc)
}
pub fn documents(yamls: &[String]) -> Result<BTreeMap<String, Document>> {
    let mut result = BTreeMap::new();
    for yaml in yamls {
        let doc = chart(&parse(yaml).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        doc.validate()?;
        if result.insert(doc.metadata.id.clone(), doc).is_some() {
            return Err("Duplicate chart ID".into());
        }
    }
    Ok(result)
}

/// JSON boundary used by Swift through the existing owned-string Rust ABI.
/// Contract version is independent of both this API and App/CLI IPC versions.
pub fn run(mut request: Value) -> std::result::Result<Value, Diagnostic> {
    validation::normalize_numbers(&mut request);
    match request["op"].as_str().unwrap_or("") {
        "info" => Ok(schema::info()),
        "parse" => parse(request["yaml"].as_str().unwrap_or("")),
        "workspace_metadata" => {
            let document = &request["document"];
            for field in ["i18n", "localizations"] {
                if document.get(field).is_some() {
                    return Err(Diagnostic::new("structure.invalid", format!("/{field}")));
                }
            }
            static SCHEMA: std::sync::OnceLock<jsonschema::Validator> = std::sync::OnceLock::new();
            SCHEMA
                .get_or_init(|| jsonschema::validator_for(&schema::workspace_metadata()).unwrap())
                .validate(document)
                .map_err(schema_diagnostic)?;
            Ok(json!({}))
        }
        "schema" => schema::schema(request["kind"].as_str().unwrap_or(""))
            .ok_or_else(|| Diagnostic::new("kind.unsupported", "/kind")),
        "decode" | "decode_model" => {
            let value = parse(
                request["yaml"]
                    .as_str()
                    .ok_or_else(|| Diagnostic::new("yaml.invalid", ""))?,
            )?;
            let kind = request["kind"]
                .as_str()
                .filter(|v| ["chart", "dashboard"].contains(v))
                .ok_or_else(|| Diagnostic::new("kind.unsupported", "/kind"))?;
            if request["op"] == "decode_model" {
                model_structure(&value, kind)?;
            } else if kind == "chart" {
                chart(&value)?;
            } else {
                structure(&value, kind)?;
                let _: Dashboard = decode(&value)?;
            }
            Ok(value)
        }
        "template_tokens" => {
            let tokens = template::tokens(request["text"].as_str().unwrap_or(""))
                .map_err(|_| Diagnostic::new("template.invalid", ""))?;
            serde_json::to_value(tokens).map_err(|_| Diagnostic::new("contract.internal", ""))
        }
        "template_references" => {
            let source: models::Source = decode(&request["source"])?;
            let (parameters, connections) = template::references(&source)
                .map_err(|_| Diagnostic::new("template.invalid", ""))?;
            Ok(json!({"parameters": parameters, "connections": connections}))
        }
        "source" => {
            let source: models::Source = decode(&request["source"])?;
            source
                .validate()
                .map_err(|_| Diagnostic::new("source.invalid", "/sources"))?;
            Ok(json!({}))
        }
        "target" => {
            let (id, name) = workspace::target(
                request["reference"].as_str().unwrap_or(""),
                request["owner"].as_str().unwrap_or(""),
                request["kind"] == "source",
            )?;
            Ok(json!({"id":id,"name":name}))
        }
        "definitions" => {
            let definitions: Vec<models::Parameter> = decode(&request["definitions"])?;
            validation::validate_definitions(&definitions)
                .map_err(|_| Diagnostic::new("parameter.invalid", "/params"))?;
            Ok(json!({}))
        }
        "parameters" => {
            let definitions: Vec<models::Parameter> = decode(&request["definitions"])?;
            let inputs = decode(&request["inputs"])?;
            validation::parameters(&definitions, &inputs)
                .map(|v| json!(v))
                .map_err(|_| Diagnostic::new("parameter.invalid", "/params"))
        }
        "validate" => {
            let mut docs = BTreeMap::new();
            if let Some(items) = request["documents"].as_array() {
                for item in items {
                    let doc = chart(item)?;
                    if docs.insert(doc.metadata.id.clone(), doc).is_some() {
                        return Err(Diagnostic::new("resource.duplicate", "/metadata/id"));
                    }
                }
            }
            let value = &request["document"];
            if request["kind"] == "chart" {
                let doc = chart(value)?;
                docs.insert(doc.metadata.id.clone(), doc.clone());
                validate_chart(&doc, &docs)?;
            } else if request["kind"] == "dashboard" {
                structure(value, "dashboard")?;
                validate_dashboard(&decode(value)?, &docs)?;
            } else {
                return Err(Diagnostic::new("kind.unsupported", "/kind"));
            }
            Ok(json!({"valid":true,"level":"workspace"}))
        }
        _ => Err(Diagnostic::new("operation.unsupported", "/op")),
    }
}
pub fn run_json(input: &str) -> String {
    let result = if input.len() > 8 * 1024 * 1024 {
        Err(Diagnostic::new("request.too_large", ""))
    } else {
        serde_json::from_str(input)
            .map_err(|_| Diagnostic::new("request.invalid", ""))
            .and_then(run)
    };
    match result {
        Ok(v) => json!({"ok":v}),
        Err(e) => json!({"error":e}),
    }
    .to_string()
}

/// Compile once, then validate the exact same structural schema returned by CLI.
pub fn structure(value: &Value, kind: &str) -> std::result::Result<(), Diagnostic> {
    if !["chart", "dashboard"].contains(&kind) {
        return Err(Diagnostic::new("kind.unsupported", "/kind"));
    }
    use std::sync::OnceLock;
    static CHART: OnceLock<jsonschema::Validator> = OnceLock::new();
    static DASHBOARD: OnceLock<jsonschema::Validator> = OnceLock::new();
    let cache = if kind == "chart" { &CHART } else { &DASHBOARD };
    let validator = cache.get_or_init(|| {
        jsonschema::validator_for(&schema::schema(kind).unwrap()).expect("generated schema")
    });
    validator.validate(value).map_err(schema_diagnostic)
}

fn model_structure(value: &Value, kind: &str) -> std::result::Result<(), Diagnostic> {
    use std::sync::OnceLock;
    static CHART: OnceLock<jsonschema::Validator> = OnceLock::new();
    static DASHBOARD: OnceLock<jsonschema::Validator> = OnceLock::new();
    let cache = if kind == "chart" { &CHART } else { &DASHBOARD };
    let validator = cache.get_or_init(|| {
        jsonschema::validator_for(&schema::model_schema(kind).unwrap())
            .expect("generated model schema")
    });
    validator.validate(value).map_err(schema_diagnostic)
}

fn schema_diagnostic(error: jsonschema::ValidationError<'_>) -> Diagnostic {
    let mut path = error.instance_path.to_string();
    if let jsonschema::error::ValidationErrorKind::Required { property } = &error.kind {
        if let Some(key) = property.as_str() {
            path.push('/');
            path.push_str(&key.replace('~', "~0").replace('/', "~1"));
        }
    }
    Diagnostic::new("structure.invalid", path)
}
