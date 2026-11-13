//! Source execution owns I/O, conversion, raw previews and shared acquisition.
//! Only platform authorization and scoped credentials cross the host boundary.
mod cache;
#[cfg(test)]
mod collection_tests;
pub(crate) mod data;
pub(crate) mod database;
mod extract;
mod http;
pub(crate) mod template;
use crate::{runtime::Cancellation, spec::Source, Result};
pub(crate) use cache::SourceCache;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, OnceLock},
    time::Duration,
};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Resolved {
    pub spec: Source,
    #[serde(default)]
    pub params: BTreeMap<String, Value>,
    #[serde(default)]
    pub number_params: BTreeSet<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub upstream: BTreeMap<String, Resolved>,
}
#[derive(Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Connection {
    #[serde(default)]
    pub connector: String,
    #[serde(default)]
    pub fields: BTreeMap<String, String>,
    #[serde(default)]
    pub extra: BTreeMap<String, Value>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Context {
    #[serde(default)]
    pub workspace: String,
    #[serde(default)]
    pub directory: String,
    #[serde(default)]
    pub revision: String,
    #[serde(default)]
    pub connections: BTreeMap<String, Connection>,
    #[serde(default)]
    pub files: BTreeMap<String, String>,
    // Additional trust anchors are a host capability, never authored YAML.
    #[serde(default)]
    pub http_roots: Vec<String>,
    #[serde(default = "sqlite_timeout")]
    pub sqlite_timeout_ms: u64,
}
fn sqlite_timeout() -> u64 {
    30_000
}
impl Default for Context {
    fn default() -> Self {
        serde_json::from_value(serde_json::json!({})).expect("default source context")
    }
}
impl Resolved {
    pub fn normalized(self) -> Result<Self> {
        self.normalize_graph(0)
    }
    fn normalize_graph(mut self, depth: usize) -> Result<Self> {
        if depth >= 64 {
            return Err("Excessively deep source dependencies".into());
        }
        let declared: BTreeSet<_> = self.spec.depends_on.iter().flatten().cloned().collect();
        if declared != self.upstream.keys().cloned().collect() {
            return Err("Resolved source dependencies do not match depends_on".into());
        }
        self.upstream = self
            .upstream
            .into_iter()
            .map(|(name, source)| Ok((name, source.normalize_graph(depth + 1)?)))
            .collect::<Result<_>>()?;
        self.spec.validate()?;
        for name in &self.number_params {
            let n = self
                .params
                .get(name)
                .and_then(Value::as_f64)
                .filter(|n| n.is_finite())
                .ok_or("Invalid number parameter")?;
            self.params.insert(name.clone(), Value::from(n));
        }
        if matches!(self.spec.source.as_deref(), Some("database" | "sqlite")) {
            let query = database::compile(
                self.spec.provider["sql"].as_str().unwrap(),
                &self.params,
                false,
            )?;
            self.params.retain(|key, _| query.parameters.contains(key));
            self.number_params
                .retain(|key| query.parameters.contains(key));
        } else {
            let (used, _) = template::references(&self.spec)?;
            if !used.iter().all(|name| self.params.contains_key(name)) {
                return Err("Missing source parameter".into());
            }
            self.params.retain(|key, _| used.contains(key));
            self.number_params.retain(|key| used.contains(key));
        }
        Ok(self)
    }
}
pub fn executor() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("source runtime")
    })
}
async fn cancelled(cancel: &Cancellation) {
    while cancel.check().is_ok() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
#[cfg(test)]
fn acquire(source: Resolved, ctx: Context, cancel: Arc<Cancellation>) -> Result<data::SourceData> {
    acquire_with_inputs(source, ctx, cancel, BTreeMap::new(), None)
}
fn acquire_with_inputs(
    source: Resolved,
    ctx: Context,
    cancel: Arc<Cancellation>,
    inputs: BTreeMap<String, Value>,
    items: Option<Vec<Value>>,
) -> Result<data::SourceData> {
    cancel.check()?;
    match source.spec.source.as_deref() {
        Some("duckdb") => {
            let file = source.spec.file.as_deref().ok_or("Missing CSV file")?;
            let bytes = if let Some(csv) = ctx.files.get(file) { csv.as_bytes().to_vec() }
                else { crate::runtime::read_csv(&ctx.directory, file, &cancel)? };
            data::SourceData::raw(bytes, "csv".into(), vec![])
        },
        Some("http" | "database") => executor().block_on(async {
            tokio::select! {
                biased;
                _ = cancelled(&cancel) => Err("Cancelled".into()),
                result = async {
                    if source.spec.source.as_deref() == Some("http") { http::load(&source, &ctx, &inputs, items.as_deref(), &cancel).await }
                    else { data::SourceData::table(database::load(&source, &ctx).await?) }
                } => result,
            }
        }),
        Some("sqlite") => data::SourceData::table(database::sqlite(&source, &ctx, cancel)?),
        _ => Err("Unknown source provider".into()),
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod integration_tests;

#[cfg(test)]
mod dag_tests;
