//! Versioned application ABI. Sessions retain one immutable imported snapshot;
//! cancellation belongs to a call, so cancelling a queued call cannot interrupt
//! a different query on the same session.
use crate::{
    engine::{Engine, Frame},
    render,
    spec::Chart,
    sql, Result, MAX_BYTES, MAX_ROWS,
};
use duckdb::{appender_params_from_iter, params, types::Value as SqlValue, InterruptHandle};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    ffi::{CStr, CString},
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        raw::c_char,
    },
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Input {
    pub file: Option<String>,
    pub csv: Option<String>,
    pub frame: Option<Frame>,
}

/// Hash serialized keys without retaining copies of CSVs, frames or parameters.
pub(crate) fn cache_key(value: &impl Serialize) -> Result<String> {
    use sha2::{Digest, Sha256};
    struct Writer(Sha256);
    impl Write for Writer {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.update(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut writer = Writer(Sha256::new());
    serde_json::to_writer(&mut writer, value).map_err(|_| "Invalid cache identity")?;
    Ok(format!("{:x}", writer.0.finalize()))
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Render {
    chart: Chart,
    #[serde(default)]
    params: BTreeMap<String, Value>,
    #[serde(default)]
    catalog: Option<crate::spec::MessageCatalog>,
    #[serde(default = "english")]
    locale: String,
}
fn english() -> String {
    "en".into()
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
enum Operation {
    Execute {
        #[serde(default)]
        render: Option<Box<Render>>,
        sources: BTreeMap<String, crate::sources::Resolved>,
        context: crate::sources::Context,
        sql: Option<String>,
        #[serde(default)]
        params: BTreeMap<String, Value>,
        #[serde(default)]
        number_params: BTreeSet<String>,
    },
    Source {
        source: crate::sources::Resolved,
        context: crate::sources::Context,
        #[serde(default)]
        display: Option<String>,
    },
    SourcePage {
        source: crate::sources::Resolved,
        context: crate::sources::Context,
        offset: usize,
        limit: usize,
        revision: Option<String>,
    },
    ReleasePreview {
        revision: String,
    },
    DecodeSource {
        bytes: Option<String>,
        frame: Option<Frame>,
        format: String,
        #[serde(default)]
        records_path: Vec<String>,
        #[serde(default)]
        display: Option<String>,
    },
    ValidateSource {
        source: crate::spec::Source,
    },
    CompileSourceSql {
        sql: String,
        params: BTreeMap<String, Value>,
        #[serde(default)]
        number_params: BTreeSet<String>,
        #[serde(default)]
        question_marks: bool,
    },
    SqliteSnapshot {
        path: String,
    },
    ReadSourceBytes {
        directory: String,
        file: String,
    },
    Query {
        #[serde(default)]
        render: Option<Box<Render>>,
        sources: BTreeMap<String, Input>,
        #[serde(default)]
        directory: String,
        #[serde(default)]
        revision: String,
        sql: Option<String>,
        #[serde(default)]
        params: BTreeMap<String, Value>,
        #[serde(default)]
        number_params: BTreeSet<String>,
        #[serde(default = "yes")]
        include_frame: bool,
    },
    RenderFrame {
        #[serde(default)]
        catalog: Option<crate::spec::MessageCatalog>,
        #[serde(default = "english")]
        locale: String,
        frame: Frame,
        chart: Chart,
        #[serde(default)]
        params: BTreeMap<String, Value>,
    },
    ValidateRender {
        #[serde(default)]
        catalog: Option<crate::spec::MessageCatalog>,
        #[serde(default = "english")]
        locale: String,
        chart: Chart,
        parameters: BTreeSet<String>,
    },
    Resolve {
        documents: BTreeMap<String, crate::spec::Document>,
        chart_id: String,
        #[serde(default)]
        params: BTreeMap<String, Value>,
        #[serde(default)]
        number_params: BTreeSet<String>,
    },
    ResolveSource {
        documents: BTreeMap<String, crate::spec::Document>,
        owner_id: String,
        parameters: Vec<crate::spec::Parameter>,
        #[serde(default)]
        params: BTreeMap<String, Value>,
        #[serde(default)]
        number_params: BTreeSet<String>,
        reference: String,
        arguments: Option<BTreeMap<String, Value>>,
    },
    CompileSql {
        sql: String,
        #[serde(default)]
        params: BTreeMap<String, Value>,
        #[serde(default)]
        number_params: BTreeSet<String>,
    },
}
fn yes() -> bool {
    true
}
fn typed_params(
    mut params: BTreeMap<String, Value>,
    numbers: BTreeSet<String>,
) -> Result<BTreeMap<String, Value>> {
    for name in numbers {
        let value = params.get_mut(&name).ok_or("Missing numeric parameter")?;
        let number = value
            .as_f64()
            .filter(|v| v.is_finite())
            .ok_or("Invalid numeric parameter")?;
        *value = Value::from(number);
    }
    Ok(params)
}
#[derive(Deserialize)]
struct Request {
    version: u32,
    #[serde(flatten)]
    operation: Operation,
}

#[derive(Default)]
pub struct Cancellation {
    cancelled: AtomicBool,
    active: Mutex<Option<Arc<InterruptHandle>>>,
    wake: Condvar,
}
impl Cancellation {
    pub(crate) fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Acquire) {
            Err("Cancelled".into())
        } else {
            Ok(())
        }
    }
    fn attach(&self, engine: &Engine) -> Result<()> {
        let mut active = self
            .active
            .lock()
            .map_err(|_| "Cancellation lock poisoned")?;
        *active = Some(engine.connection.interrupt_handle());
        self.check()
    }
    pub(crate) fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
        if let Ok(active) = self.active.lock() {
            if let Some(handle) = active.as_ref() {
                handle.interrupt();
            }
            self.wake.notify_all();
        }
    }
    fn monitored<T>(&self, action: impl FnOnce() -> T) -> T {
        let finished = AtomicBool::new(false);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let mut active = self.active.lock().unwrap_or_else(|e| e.into_inner());
                while !finished.load(Ordering::Acquire) {
                    if self.cancelled.load(Ordering::Acquire) {
                        if let Some(handle) = active.as_ref() {
                            handle.interrupt();
                        }
                        // DuckDB resets its interrupt flag at query entry. Keep
                        // interrupting a cancelled call until the worker exits,
                        // closing the cancellation/query-entry race.
                        active = self
                            .wake
                            .wait_timeout(active, std::time::Duration::from_millis(10))
                            .unwrap_or_else(|e| e.into_inner())
                            .0;
                    } else {
                        active = self.wake.wait(active).unwrap_or_else(|e| e.into_inner());
                    }
                }
            });
            struct Finished<'a> {
                done: &'a AtomicBool,
                cancel: &'a Cancellation,
            }
            impl Drop for Finished<'_> {
                fn drop(&mut self) {
                    let _active = self.cancel.active.lock().unwrap_or_else(|e| e.into_inner());
                    self.done.store(true, Ordering::Release);
                    self.cancel.wake.notify_all();
                }
            }
            let _finished = Finished {
                done: &finished,
                cancel: self,
            };
            action()
        })
    }
}
struct Active<'a>(&'a Cancellation);
impl Drop for Active<'_> {
    fn drop(&mut self) {
        if let Ok(mut active) = self.0.active.lock() {
            *active = None;
        }
    }
}
#[derive(Default)]
struct State {
    snapshot: Option<(String, Engine)>,
    result: Option<(String, Frame)>,
    rendered: Option<(String, String)>,
}
struct PreviewSnapshot {
    revision: String,
    key: String,
    data: Arc<crate::sources::data::SourceData>,
    weight: usize,
}
#[derive(Default)]
pub struct Session {
    state: Mutex<State>,
    sources: crate::sources::SourceCache,
    previews: Mutex<VecDeque<PreviewSnapshot>>,
    preview_sequence: AtomicU64,
}

/// Open every relative component with O_NOFOLLOW using directory descriptors.
/// A rename after opening cannot redirect the opened capability. Intermediate
/// directories and the data directory may not be symlinks.
pub(crate) fn open_source_file(directory: &str, path: &str, cancel: &Cancellation) -> Result<File> {
    crate::spec::data_path(path)?;
    let root = File::open(directory).map_err(|_| "Cannot open workspace")?;
    if !root
        .metadata()
        .map_err(|_| "Cannot inspect workspace")?
        .is_dir()
    {
        return Err("Workspace must be a directory".into());
    }
    let segments: Vec<_> = path.split('/').collect();
    let mut handle = root;
    for (index, segment) in segments.iter().enumerate() {
        cancel.check()?;
        let name = CString::new(*segment).map_err(|_| "Invalid file name")?;
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if index + 1 < segments.len() {
                libc::O_DIRECTORY
            } else {
                0
            };
        // SAFETY: handle and name remain live for openat; the new fd is owned below.
        let fd = unsafe { libc::openat(handle.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(
                "Source must be a regular file within workspace data/ without symlinks".into(),
            );
        }
        handle = unsafe { File::from_raw_fd(fd) };
    }
    let metadata = handle
        .metadata()
        .map_err(|_| "Cannot inspect source file")?;
    if !metadata.is_file() {
        return Err("Source must be a regular file".into());
    }
    Ok(handle)
}

pub(crate) fn source_file_revision(
    directory: &str,
    path: &str,
    cancel: &Cancellation,
) -> Result<String> {
    use std::os::unix::fs::MetadataExt;
    let file = open_source_file(directory, path, cancel)?;
    let meta = file.metadata().map_err(|_| "Cannot inspect source file")?;
    Ok(format!(
        "{}:{}:{}:{}:{}:{}:{}",
        meta.dev(),
        meta.ino(),
        meta.len(),
        meta.mtime(),
        meta.mtime_nsec(),
        meta.ctime(),
        meta.ctime_nsec()
    ))
}

pub(crate) fn read_csv(directory: &str, path: &str, cancel: &Cancellation) -> Result<Vec<u8>> {
    let mut handle = open_source_file(directory, path, cancel)?;
    let mut bytes = Vec::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        cancel.check()?;
        let count = handle.read(&mut buffer).map_err(|_| "Cannot read CSV")?;
        if count == 0 {
            break;
        }
        if bytes.len() as u64 + count as u64 > MAX_BYTES {
            return Err("CSV exceeds 100 MiB".into());
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(bytes)
}

impl Frame {
    pub(crate) fn validate(&self) -> Result<()> {
        let mut names = BTreeSet::new();
        if self.columns.is_empty()
            || self.rows.len() > MAX_ROWS
            || self
                .columns
                .iter()
                .any(|c| c.name.is_empty() || !names.insert(c.name.to_lowercase()))
        {
            return Err("Invalid frame columns or row count".into());
        }
        for column in &self.columns {
            column.sql_type()?;
        }
        for row in &self.rows {
            if row.len() != self.columns.len() {
                return Err("Invalid frame row width".into());
            }
            for (cell, column) in row.iter().zip(&self.columns) {
                let valid = cell.is_null()
                    || match column.kind.as_str() {
                        "integer" => cell.as_i64().is_some(),
                        "number" => cell.as_f64().is_some_and(f64::is_finite),
                        "boolean" => cell.is_boolean(),
                        _ => cell.is_string(),
                    };
                if !valid {
                    return Err(format!("Invalid value in field {}", column.name));
                }
            }
        }
        Ok(())
    }
}
impl crate::engine::Column {
    fn sql_type(&self) -> Result<&'static str> {
        match self.kind.as_str() {
            "string" | "json" => Ok("VARCHAR"),
            "integer" => Ok("BIGINT"),
            "number" => Ok("DOUBLE"),
            "boolean" => Ok("BOOLEAN"),
            "date" => Ok("DATE"),
            "timestamp" => Ok("TIMESTAMP"),
            _ => Err("Unknown frame type".into()),
        }
    }
}
impl Engine {
    pub(crate) fn import_inputs(
        &mut self,
        sources: &BTreeMap<String, Input>,
        directory: &str,
        cancel: &Cancellation,
    ) -> Result<()> {
        if sources.len() > 32 {
            return Err("At most 32 sources".into());
        }
        let mut total = 0;
        for (alias, input) in sources {
            cancel.check()?;
            if alias.is_empty() {
                return Err("Empty source alias".into());
            }
            if input.frame.is_some() {
                if input.file.is_some() || input.csv.is_some() {
                    return Err("Choose CSV or frame acquisition".into());
                }
                let frame = input.frame.as_ref().unwrap();
                frame.validate()?;
                let fields = frame
                    .columns
                    .iter()
                    .map(|c| Ok(format!("{} {}", sql::identifier(&c.name), c.sql_type()?)))
                    .collect::<Result<Vec<_>>>()?
                    .join(", ");
                self.connection
                    .execute_batch(&format!(
                        "CREATE TABLE {} ({fields})",
                        sql::identifier(alias)
                    ))
                    .map_err(|e| e.to_string())?;
                let mut appender = self.connection.appender(alias).map_err(|e| e.to_string())?;
                for row in &frame.rows {
                    cancel.check()?;
                    let values: Vec<_> = row
                        .iter()
                        .map(|cell| {
                            if cell.is_null() {
                                SqlValue::Null
                            } else if let Some(v) = cell.as_bool() {
                                SqlValue::Boolean(v)
                            } else if let Some(v) = cell.as_i64() {
                                SqlValue::BigInt(v)
                            } else if let Some(v) = cell.as_f64() {
                                SqlValue::Double(v)
                            } else {
                                SqlValue::Text(cell.as_str().unwrap().into())
                            }
                        })
                        .collect();
                    appender
                        .append_row(appender_params_from_iter(values.iter()))
                        .map_err(|e| e.to_string())?;
                }
                appender.flush().map_err(|e| e.to_string())?;
            } else {
                let bytes = if let Some(csv) = &input.csv {
                    if let Some(file) = &input.file {
                        crate::spec::data_path(file)?;
                    }
                    csv.as_bytes().to_vec()
                } else {
                    read_csv(
                        directory,
                        input.file.as_deref().ok_or("Missing CSV file")?,
                        cancel,
                    )?
                };
                total += bytes.len() as u64;
                if total > MAX_BYTES {
                    return Err("Source CSV snapshots exceed 100 MiB".into());
                }
                std::str::from_utf8(&bytes).map_err(|_| "CSV must be UTF-8")?;
                let mut snapshot = tempfile::Builder::new()
                    .suffix(".csv")
                    .tempfile()
                    .map_err(|e| e.to_string())?;
                snapshot.write_all(&bytes).map_err(|e| e.to_string())?;
                let query = format!("CREATE TABLE {} AS SELECT * FROM read_csv($1, header=true, all_varchar=true) LIMIT {}", sql::identifier(alias), MAX_ROWS + 1);
                self.connection
                    .execute(
                        &query,
                        params![snapshot.path().to_str().ok_or("Invalid snapshot path")?],
                    )
                    .map_err(|e| e.to_string())?;
                let count: i64 = self
                    .connection
                    .query_row(
                        &format!("SELECT COUNT(*) FROM {}", sql::identifier(alias)),
                        [],
                        |r| r.get(0),
                    )
                    .map_err(|e| e.to_string())?;
                if count as usize > MAX_ROWS {
                    return Err("CSV exceeds 100,000 rows".into());
                }
            }
            self.tables.push(alias.clone());
        }
        // This setting is irreversible for the life of the imported snapshot.
        self.connection
            .execute_batch("SET enable_external_access=false")
            .map_err(|e| e.to_string())?;
        cancel.check()
    }
}
impl Session {
    fn run(&self, request: Request, cancel: &Cancellation) -> Result<Value> {
        if request.version != 1 {
            return Err("Unsupported ABI version".into());
        }
        cancel.check()?;
        if matches!(
            &request.operation,
            Operation::Query { .. } | Operation::Execute { .. }
        ) {
            cancel.monitored(|| self.execute(request, cancel))
        } else {
            self.execute(request, cancel)
        }
    }
    fn execute(&self, request: Request, cancel: &Cancellation) -> Result<Value> {
        match request.operation {
            Operation::CompileSourceSql {
                sql,
                params,
                number_params,
                question_marks,
            } => {
                let query = crate::sources::database::compile(
                    &sql,
                    &typed_params(params, number_params)?,
                    question_marks,
                )?;
                let numbers: Vec<_> = query
                    .values
                    .iter()
                    .enumerate()
                    .filter_map(|(i, v)| v.as_number().filter(|n| n.is_f64()).map(|_| i))
                    .collect();
                Ok(json!({"sql": query.sql, "values": query.values, "number_values": numbers}))
            }
            Operation::ReadSourceBytes { directory, file } => {
                use base64::Engine as _;
                Ok(
                    json!({"bytes":base64::engine::general_purpose::STANDARD.encode(read_csv(&directory, &file, cancel)?)}),
                )
            }
            Operation::SqliteSnapshot { path } => {
                use base64::Engine as _;
                Ok(
                    json!({"bytes":base64::engine::general_purpose::STANDARD.encode(crate::sources::database::snapshot(&path)?)}),
                )
            }
            Operation::ValidateSource { source } => {
                source.validate()?;
                Ok(json!({}))
            }
            Operation::DecodeSource {
                bytes,
                frame,
                format,
                records_path,
                display,
            } => {
                use base64::Engine as _;
                let data = if let Some(bytes) = bytes {
                    if bytes.len() > (MAX_BYTES as usize).div_ceil(3) * 4 {
                        return Err("Source exceeds 100 MiB".into());
                    }
                    let bytes = base64::engine::general_purpose::STANDARD
                        .decode(bytes)
                        .map_err(|_| "Invalid source bytes")?;
                    crate::sources::data::SourceData::raw(bytes, format, records_path)?
                } else {
                    crate::sources::data::SourceData::table(frame.ok_or("Missing source data")?)?
                };
                if let Some(display) = display {
                    data.preview(&display, cancel)
                } else {
                    serde_json::to_value(data.dataframe(cancel)?).map_err(|e| e.to_string())
                }
            }
            Operation::ReleasePreview { revision } => {
                self.previews
                    .lock()
                    .map_err(|_| "Preview lock poisoned")?
                    .retain(|p| p.revision != revision);
                Ok(json!({}))
            }
            Operation::SourcePage {
                source,
                context,
                offset,
                limit,
                revision,
            } => {
                if limit == 0 || limit > 1024 {
                    return Err("Preview page size must be 1–1024 rows".into());
                }
                let source = source.normalized()?;
                let key = cache_key(&(
                    &source,
                    &context.workspace,
                    &context.directory,
                    &context.revision,
                    &context.files,
                ))?;
                let (data, revision) = if let Some(revision) = revision {
                    let previews = self.previews.lock().map_err(|_| "Preview lock poisoned")?;
                    let snapshot = previews
                        .iter()
                        .find(|p| p.revision == revision && p.key == key)
                        .ok_or("Source preview expired; refresh the data")?;
                    (snapshot.data.clone(), revision)
                } else {
                    if offset != 0 {
                        return Err("Start a preview at row zero".into());
                    }
                    let (data, source_revision) = self.sources.load(source, &context, cancel)?;
                    let revision = format!(
                        "{source_revision}:{}",
                        self.preview_sequence.fetch_add(1, Ordering::Relaxed)
                    );
                    data.dataframe(cancel)?;
                    let weight = data.weight();
                    let mut previews = self.previews.lock().map_err(|_| "Preview lock poisoned")?;
                    previews.retain(|p| p.revision != revision);
                    while !previews.is_empty()
                        && (previews.len() >= 4
                            || previews
                                .iter()
                                .map(|p| p.weight)
                                .sum::<usize>()
                                .saturating_add(weight)
                                > MAX_BYTES as usize)
                    {
                        previews.pop_front();
                    }
                    previews.push_back(PreviewSnapshot {
                        revision: revision.clone(),
                        key,
                        data: data.clone(),
                        weight,
                    });
                    (data, revision)
                };
                let frame = data.dataframe(cancel)?;
                if offset > frame.rows.len() {
                    return Err("Preview offset is outside the snapshot".into());
                }
                let end = offset.saturating_add(limit).min(frame.rows.len());
                cancel.check()?;
                let page = Frame {
                    columns: frame.columns.clone(),
                    rows: frame.rows[offset..end].to_vec(),
                };
                Ok(
                    json!({"frame": page, "total_rows": frame.rows.len(), "offset": offset,
                    "next_offset": if end < frame.rows.len() { Some(end) } else { None }, "revision": revision}),
                )
            }
            Operation::Source {
                source,
                context,
                display,
            } => {
                let source = source.normalized()?;
                let (data, _) = self.sources.load(source, &context, cancel)?;
                if let Some(display) = display {
                    data.preview(&display, cancel)
                } else {
                    serde_json::to_value(data.dataframe(cancel)?).map_err(|e| e.to_string())
                }
            }
            Operation::Execute {
                render,
                sources,
                context,
                sql,
                params,
                number_params,
            } => {
                if sources.is_empty() || sources.len() > 32 || (sources.len() > 1 && sql.is_none())
                {
                    return Err("Dataset requires 1–32 sources and SQL for multiple sources".into());
                }
                let mut inputs = BTreeMap::new();
                let mut revisions = Vec::new();
                let mut source_bytes = 0u64;
                // Acquisition never holds the DuckDB state lock. Concurrent callers
                // share source producers, each with independent cancellation.
                for (alias, source) in sources {
                    crate::spec::valid_id(&alias)?;
                    let (data, revision) =
                        self.sources.load(source.normalized()?, &context, cancel)?;
                    let input = data.input(cancel)?;
                    source_bytes = source_bytes.saturating_add(data.weight() as u64);
                    if source_bytes > MAX_BYTES {
                        return Err("Dataset sources exceed 100 MiB".into());
                    }
                    inputs.insert(alias, input);
                    revisions.push(revision);
                }
                self.execute(
                    Request {
                        version: 1,
                        operation: Operation::Query {
                            render,
                            sources: inputs,
                            directory: String::new(),
                            revision: serde_json::to_string(&revisions)
                                .map_err(|e| e.to_string())?,
                            sql,
                            params,
                            number_params,
                            include_frame: true,
                        },
                    },
                    cancel,
                )
            }
            Operation::ValidateRender {
                chart,
                parameters,
                catalog,
                locale,
            } => {
                render::payload(
                    &chart,
                    None,
                    &BTreeMap::new(),
                    parameters,
                    cancel,
                    catalog.as_ref(),
                    &locale,
                )?;
                Ok(json!({}))
            }
            Operation::Resolve {
                documents,
                chart_id,
                params,
                number_params,
            } => {
                let params = typed_params(params, number_params)?;
                let chart = documents.get(&chart_id).ok_or("Chart not found")?;
                serde_json::to_value(crate::spec::resolve(
                    chart,
                    &documents,
                    &chart.parameters(&params)?,
                )?)
                .map_err(|e| e.to_string())
            }
            Operation::ResolveSource {
                documents,
                owner_id,
                parameters,
                params,
                number_params,
                reference,
                arguments,
            } => {
                let params = typed_params(params, number_params)?;
                serde_json::to_value(crate::spec::resolve_source(
                    &documents,
                    &owner_id,
                    &parameters,
                    &params,
                    &reference,
                    arguments.as_ref(),
                )?)
                .map_err(|e| e.to_string())
            }
            Operation::CompileSql {
                sql: query,
                params,
                number_params,
            } => {
                let params = typed_params(params, number_params)?;
                let compiled = sql::compile(&query, &params)?;
                let number_values: Vec<_> = compiled
                    .values
                    .iter()
                    .enumerate()
                    .filter(|(_, v)| v.is_f64())
                    .map(|(index, _)| index)
                    .collect();
                Ok(
                    json!({"sql": compiled.sql, "values": compiled.values, "number_values": number_values}),
                )
            }
            Operation::RenderFrame {
                catalog,
                locale,
                frame,
                chart,
                params,
            } => {
                frame.validate()?;
                let payload = render::payload(
                    &chart,
                    Some(&frame),
                    &params,
                    params.keys().cloned().collect(),
                    cancel,
                    catalog.as_ref(),
                    &locale,
                )?;
                Ok(json!({
                    "payload_json": serde_json::to_string(&payload).map_err(|e| e.to_string())?,
                    "row_count": frame.rows.len(),
                    "snapshot_reused": false,
                    "result_reused": false
                }))
            }
            Operation::Query {
                render,
                sources,
                directory,
                revision,
                sql,
                params,
                number_params,
                include_frame,
            } => {
                let params = typed_params(params, number_params)?;
                let key = cache_key(&(&sources, &directory, &revision))?;
                let mut state = self.state.lock().map_err(|_| "Session lock poisoned")?;
                cancel.check()?;
                let _active = Active(cancel);
                let reused = state.snapshot.as_ref().is_some_and(|(k, _)| k == &key);
                if !reused {
                    // Drop the old engine first to keep retained native memory bounded.
                    state.result = None;
                    state.rendered = None;
                    state.snapshot = None;
                    let mut engine = Engine::new()?;
                    cancel.attach(&engine)?;
                    engine.import_inputs(&sources, &directory, cancel)?;
                    state.snapshot = Some((key, engine));
                }
                let result_key = cache_key(&(&sql, &params))?;
                let result_reused = state.result.as_ref().is_some_and(|(k, _)| k == &result_key);
                if !result_reused {
                    let engine = &state.snapshot.as_ref().unwrap().1;
                    cancel.attach(engine)?;
                    let frame = if sql.is_none() && sources.len() == 1 {
                        match sources.values().next().and_then(|s| s.frame.as_ref()) {
                            Some(frame) => Ok(frame.clone()),
                            None => engine.query_cancel(None, &params, cancel),
                        }
                    } else {
                        engine.query_cancel(sql.as_deref(), &params, cancel)
                    };
                    let frame = match frame {
                        Ok(frame) => frame,
                        Err(error) => {
                            // Internal DuckDB errors can invalidate the database.
                            // Detach cancellation before dropping the engine so
                            // a corrected query can import a fresh snapshot.
                            drop(_active);
                            state.result = None;
                            state.snapshot = None;
                            return Err(error);
                        }
                    };
                    cancel.check()?;
                    state.result = Some((result_key, frame));
                    state.rendered = None;
                }
                let frame = &state.result.as_ref().unwrap().1;
                cancel.check()?;
                let row_count = frame.rows.len();
                if let Some(render) = render {
                    let key = cache_key(&render)?;
                    if state.rendered.as_ref().is_none_or(|(old, _)| old != &key) {
                        let payload = render::payload(
                            &render.chart,
                            Some(frame),
                            &render.params,
                            render.params.keys().cloned().collect(),
                            cancel,
                            render.catalog.as_ref(),
                            &render.locale,
                        )?;
                        let payload = serde_json::to_string(&payload).map_err(|e| e.to_string())?;
                        if payload.len() as u64 > MAX_BYTES {
                            return Err("Rendered payload exceeds 100 MiB".into());
                        }
                        state.rendered = Some((key, payload));
                    }
                    cancel.check()?;
                    return Ok(json!({"payload_json": state.rendered.as_ref().unwrap().1,
                        "row_count": row_count, "snapshot_reused": reused, "result_reused": result_reused}));
                }
                cancel.check()?;
                Ok(
                    json!({"frame": if include_frame { Some(frame) } else { None },
                    "row_count": row_count, "snapshot_reused": reused, "result_reused": result_reused}),
                )
            }
        }
    }
}

#[no_mangle]
pub extern "C" fn facetra_session_create() -> *mut Session {
    Box::into_raw(Box::default())
}
#[no_mangle]
pub extern "C" fn facetra_cancel_create() -> *mut Cancellation {
    Box::into_raw(Box::default())
}
/// # Safety
/// Pass a live cancellation pointer. It must outlive every concurrent call.
#[no_mangle]
pub unsafe extern "C" fn facetra_cancel_request(cancel: *const Cancellation) {
    if let Some(cancel) = unsafe { cancel.as_ref() } {
        cancel.cancel();
    }
}
/// # Safety
/// Pass null or an owned session pointer; no calls may be in flight.
#[no_mangle]
pub unsafe extern "C" fn facetra_session_destroy(session: *mut Session) {
    if !session.is_null() {
        drop(unsafe { Box::from_raw(session) });
    }
}
/// # Safety
/// Pass null or an owned cancellation pointer; no calls may be in flight.
#[no_mangle]
pub unsafe extern "C" fn facetra_cancel_destroy(cancel: *mut Cancellation) {
    if !cancel.is_null() {
        drop(unsafe { Box::from_raw(cancel) });
    }
}
/// # Safety
/// Session, UTF-8 NUL-terminated input and cancellation must be live throughout
/// the call. Cancel may be null. Release the returned JSON with pipeline_free.
#[no_mangle]
pub unsafe extern "C" fn facetra_session_run(
    session: *const Session,
    input: *const c_char,
    cancel: *const Cancellation,
) -> *mut c_char {
    let mut failure_code = "invalid_request";
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let session = unsafe { session.as_ref() }.ok_or("Null session")?;
        if input.is_null() {
            return Err("Null request".into());
        }
        let input = unsafe { CStr::from_ptr(input) }
            .to_str()
            .map_err(|_| "Request must be UTF-8")?;
        if input.len() as u64 > MAX_BYTES * 6 {
            return Err("Request exceeds transport limit".into());
        }
        let request = serde_json::from_str(input).map_err(|e| format!("Invalid request: {e}"))?;
        let local = Cancellation::default();
        let cancel = unsafe { cancel.as_ref() }.unwrap_or(&local);
        failure_code = "execution";
        let result = session.run(request, cancel);
        if cancel.check().is_err() {
            if let Ok(output) = &result {
                if let Some(revision) = output.get("revision").and_then(Value::as_str) {
                    if let Ok(mut previews) = session.previews.lock() {
                        previews.retain(|p| p.revision != revision);
                    }
                }
            }
            return Err("Cancelled".into());
        }
        result
    }));
    let response = match result {
        Ok(Ok(output)) => json!({"ok": output}),
        Ok(Err(error)) => {
            if unsafe { cancel.as_ref() }.is_some_and(|c| c.cancelled.load(Ordering::Acquire)) {
                failure_code = "cancelled";
            }
            json!({"error": error, "error_code": failure_code})
        }
        Err(_) => json!({"error": "Rust pipeline panicked", "error_code": "internal"}),
    };
    CString::new(response.to_string())
        .expect("JSON escapes NUL")
        .into_raw()
}

#[cfg(test)]
#[path = "tests/runtime.rs"]
mod tests;
