use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Document {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub i18n: Option<MessageCatalog>,
    pub metadata: Metadata,
    #[serde(default)]
    pub params: Vec<Parameter>,
    #[serde(default)]
    pub sources: BTreeMap<String, Source>,
    pub dataset: Dataset,
    pub chart: Chart,
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<LocalizedText>,
    #[serde(rename = "type")]
    #[schemars(with = "ParameterType")]
    pub kind: String,
    #[schemars(with = "Option<ScalarValue>")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
    #[serde(default = "yes")]
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minimum: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub maximum: Option<f64>,
    /// Preferred title/control block width in native points; omitted means automatic.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<ParameterOptions>,
    #[serde(default, rename = "advanced")]
    pub _advanced: bool,
}
fn yes() -> bool {
    true
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ParameterOptions {
    Values(#[schemars(with = "Vec<ScalarValue>")] Vec<Value>),
    Source(OptionsSource),
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OptionsSource {
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default = "option_value_column")]
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[schemars(with = "Option<BTreeMap<String, BindingValue>>")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with: Option<BTreeMap<String, Value>>,
}
fn option_value_column() -> String {
    "value".into()
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum SourceDisplay {
    Df,
    Json,
    Text,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Source {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<SourceDisplay>,
    #[serde(rename = "ref")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with: Option<BTreeMap<String, Value>>,
    #[serde(flatten)]
    pub provider: BTreeMap<String, Value>,
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Dataset {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<Input>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sql: Option<String>,
    #[serde(rename = "ref")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[schemars(with = "Option<BTreeMap<String, BindingValue>>")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with: Option<BTreeMap<String, Value>>,
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub id: String,
    #[serde(rename = "ref")]
    pub reference: String,
    #[schemars(with = "Option<BTreeMap<String, BindingValue>>")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub with: Option<BTreeMap<String, Value>>,
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Chart {
    pub dataset: String,
    #[schemars(with = "Renderer")]
    pub renderer: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scripts: Option<BTreeMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Metadata {
    pub id: String,
    pub title: LocalizedText,
    #[serde(default, rename = "notes")]
    pub _notes: LocalizedText,
    #[serde(default, rename = "tags")]
    pub _tags: Vec<String>,
}

/// Authored text is retained unchanged; hosts resolve references for display.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(untagged)]
pub enum LocalizedText {
    Literal(String),
    Reference(MessageReference),
}
impl Default for LocalizedText {
    fn default() -> Self {
        Self::Literal(String::new())
    }
}
impl LocalizedText {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Literal(text) => text.is_empty(),
            Self::Reference(reference) => reference.i18n.is_empty(),
        }
    }
    pub fn is_blank(&self) -> bool {
        match self {
            Self::Literal(text) => text.trim().is_empty(),
            Self::Reference(reference) => reference.i18n.trim().is_empty(),
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct MessageReference {
    pub i18n: String,
}

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MessageCatalog {
    pub default_locale: String,
    pub messages: BTreeMap<String, BTreeMap<String, String>>,
}

#[derive(JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ParameterType {
    String,
    Integer,
    Number,
    Boolean,
    Date,
}
#[derive(JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Renderer {
    Value,
    Echarts,
    Vela,
    Html,
    #[serde(rename = "vega_lite")]
    VegaLite,
}

#[derive(JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DataFormat {
    Csv,
    Json,
    Html,
    Xml,
}

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HttpSpec {
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    #[schemars(with = "BTreeMap<String, ScalarValue>")]
    pub params: BTreeMap<String, Value>,
    #[serde(
        default,
        deserialize_with = "present_json",
        skip_serializing_if = "Option::is_none"
    )]
    pub json: Option<Value>,
    #[schemars(with = "Option<BTreeMap<String, ScalarValue>>")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<BTreeMap<String, Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<DataFormat>")]
    pub format: Option<String>,
    #[serde(default, rename = "recordsPath")]
    pub records_path: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pagination: Option<Pagination>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub foreach: Option<ForEach>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extract: Option<Extraction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limits: Option<HttpLimits>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<HttpRetry>,
}

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pagination {
    pub initial: BTreeMap<String, Value>,
    /// Strict Rhai expressions. Unit () ends iteration; a map advances it.
    pub next: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub records: Option<String>,
    #[serde(default = "default_pages")]
    pub max_pages: usize,
}
pub fn default_pages() -> usize {
    100
}

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ForEach {
    pub source: String,
    #[serde(default = "default_items")]
    pub max_items: usize,
    #[serde(default)]
    pub include_item: bool,
}
pub fn default_items() -> usize {
    1000
}

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Extraction {
    pub rows: String,
    pub fields: BTreeMap<String, ExtractField>,
    /// Document-level fields (e.g. an HTML next link), available as response.extracted.
    #[serde(default)]
    pub context: BTreeMap<String, ExtractField>,
    #[serde(default)]
    pub namespaces: BTreeMap<String, String>,
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExtractField {
    pub selector: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribute: Option<String>,
    #[serde(default)]
    pub all: bool,
    #[serde(default = "yes")]
    pub required: bool,
    #[serde(default)]
    pub json: bool,
    #[serde(default)]
    pub resolve_url: bool,
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HttpLimits {
    #[serde(default = "default_requests")]
    pub max_requests: usize,
    #[serde(default = "default_bytes")]
    pub max_bytes: usize,
    #[serde(default = "default_rows")]
    pub max_rows: usize,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}
pub fn default_requests() -> usize {
    1000
}
pub fn default_bytes() -> usize {
    100 * 1024 * 1024
}
pub fn default_rows() -> usize {
    100_000
}
pub fn default_timeout() -> u64 {
    60_000
}
impl Default for HttpLimits {
    fn default() -> Self {
        Self {
            max_requests: default_requests(),
            max_bytes: default_bytes(),
            max_rows: default_rows(),
            timeout_ms: default_timeout(),
        }
    }
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HttpRetry {
    #[serde(default = "default_attempts")]
    pub max_attempts: usize,
    #[serde(default = "default_delay")]
    pub delay_ms: u64,
    /// Explicit assertion that replaying this POST is safe.
    #[serde(default)]
    pub retry_post: bool,
}
pub fn default_attempts() -> usize {
    3
}
pub fn default_delay() -> u64 {
    250
}
fn present_json<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<Value>, D::Error> {
    Value::deserialize(d).map(Some)
}

// Provider fields remain a map in the execution projection. Its schema comes
// from typed variants, also used to check authored source fields.
#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum SourceShape {
    Reference(ReferenceSource),
    Concrete(ConcreteSource),
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReferenceSource {
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(rename = "with")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<BTreeMap<String, BindingValue>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display: Option<SourceDisplay>,
}
#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum BindingValue {
    Parameter(ParameterReference),
    Literal(ScalarValue),
}
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ParameterReference {
    pub param: String,
}
#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum ScalarValue {
    String(String),
    Integer(i64),
    Number(f64),
    Boolean(bool),
}
#[derive(Deserialize, JsonSchema)]
#[serde(tag = "source", rename_all = "lowercase", deny_unknown_fields)]
pub enum ConcreteSource {
    Duckdb {
        file: String,
        depends_on: Option<Vec<String>>,
        display: Option<SourceDisplay>,
    },
    Http {
        depends_on: Option<Vec<String>>,
        #[serde(flatten)]
        spec: Box<HttpSpec>,
        display: Option<SourceDisplay>,
    },
    Database {
        depends_on: Option<Vec<String>>,
        connection: String,
        sql: String,
        display: Option<SourceDisplay>,
    },
    Sqlite {
        depends_on: Option<Vec<String>>,
        file_path: String,
        sql: String,
        display: Option<SourceDisplay>,
    },
}
impl JsonSchema for Source {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Source".into()
    }
    fn json_schema(generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        SourceShape::json_schema(generator)
    }
}

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Dashboard {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub i18n: Option<MessageCatalog>,
    pub version: u32,
    pub kind: String,
    pub metadata: Metadata,
    #[serde(default)]
    pub params: Vec<Parameter>,
    pub tabs: Vec<Tab>,
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Tab {
    pub id: String,
    pub title: LocalizedText,
    pub panels: Vec<Panel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<Grid>,
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Grid {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default = "columns")]
    pub columns: u32,
    #[serde(default = "row_height")]
    pub row_height: f64,
    #[serde(default = "gap")]
    pub gap: f64,
    #[serde(default = "min_width")]
    pub min_width: f64,
}
fn columns() -> u32 {
    12
}
fn row_height() -> f64 {
    40.0
}
fn gap() -> f64 {
    12.0
}
fn min_width() -> f64 {
    1200.0
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Panel {
    pub id: String,
    pub chart: String,
    #[serde(default = "height")]
    pub height: f64,
    #[serde(default)]
    #[schemars(with = "BTreeMap<String, BindingValue>")]
    pub params: BTreeMap<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub row_span: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responsive: Option<Responsive>,
}
fn height() -> f64 {
    240.0
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Responsive {
    pub compact: Compact,
}
#[derive(Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Compact {
    pub card_height: f64,
}
