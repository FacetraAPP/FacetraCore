use serde::{Deserialize, Serialize};

/// Values, query text and credentials are never included in diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: String,
    /// JSON Pointer in the authored resource (empty for the root).
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
}
impl Diagnostic {
    pub fn new(code: &str, path: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            path: path.into(),
            resource: None,
            line: None,
            column: None,
        }
    }
    pub fn resource(mut self, id: &str) -> Self {
        self.resource = Some(id.into());
        self
    }
}
impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} at {}", self.code, self.path)
    }
}
impl std::error::Error for Diagnostic {}
