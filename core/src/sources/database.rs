use super::{Connection, Context, Resolved};
use crate::{
    engine::{Column, Frame},
    runtime::Cancellation,
    Result, MAX_BYTES, MAX_ROWS,
};
use base64::Engine as _;
use futures_util::{pin_mut, StreamExt};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

pub fn compile(
    sql: &str,
    params: &BTreeMap<String, Value>,
    question_marks: bool,
) -> Result<crate::sql::Compiled> {
    crate::sql::compile_dialect(sql, params, question_marks, true)
}
fn failure(_: impl std::fmt::Display) -> String {
    "Database loading failed. Check the connection, SQL, permissions, timeouts and data limits."
        .into()
}
fn option<'a>(c: &'a Connection, key: &str, default: &'a str) -> &'a str {
    c.extra.get(key).and_then(Value::as_str).unwrap_or(default)
}
fn timeout(c: &Connection, key: &str) -> Result<Duration> {
    let value = c
        .extra
        .get(key)
        .map(|v| v.as_u64().ok_or("Invalid database timeout"))
        .transpose()?
        .unwrap_or(30);
    if !(1..=120).contains(&value) {
        return Err("Invalid database timeout".into());
    }
    Ok(Duration::from_secs(value))
}
fn field<'a>(c: &'a Connection, key: &str) -> Result<&'a str> {
    c.fields
        .get(key)
        .map(String::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| format!("Missing connection field: {key}"))
}
async fn deadline<T>(
    duration: Duration,
    f: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    tokio::time::timeout(duration, f).await.map_err(failure)?
}
#[derive(Default)]
struct Rows {
    names: Vec<String>,
    rows: Vec<Vec<Value>>,
    bytes: usize,
}
impl Rows {
    fn append(&mut self, row: Vec<Value>) -> Result<()> {
        self.bytes += row
            .iter()
            .map(|v| super::data::text(v).len())
            .sum::<usize>();
        if row.len() != self.names.len()
            || self.rows.len() >= MAX_ROWS
            || self.bytes as u64 > MAX_BYTES
        {
            return Err("Database source exceeds data limits".into());
        }
        self.rows.push(row);
        Ok(())
    }
    fn frame(mut self, empty_types: Vec<String>) -> Result<Frame> {
        let mut columns = Vec::new();
        for (i, name) in self.names.into_iter().enumerate() {
            let values: Vec<_> = self
                .rows
                .iter()
                .map(|r| &r[i])
                .filter(|v| !v.is_null())
                .collect();
            let kind = if values.is_empty() {
                empty_types
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| "string".into())
            } else if values.iter().all(|v| v.is_i64()) {
                "integer".into()
            } else if values.iter().all(|v| v.is_number()) {
                "number".into()
            } else if values.iter().all(|v| v.is_boolean()) {
                "boolean".into()
            } else {
                "string".into()
            };
            if kind == "string" {
                for row in &mut self.rows {
                    if !row[i].is_null() {
                        row[i] = Value::String(super::data::text(&row[i]));
                    }
                }
            }
            columns.push(Column { name, kind });
        }
        let frame = Frame {
            columns,
            rows: self.rows,
        };
        frame.validate()?;
        Ok(frame)
    }
}
pub async fn load(source: &Resolved, ctx: &Context) -> Result<Frame> {
    let alias = source.spec.provider["connection"]
        .as_str()
        .ok_or("Missing connection")?;
    let c = ctx
        .connections
        .get(alias)
        .ok_or("Missing database connection")?;
    if !["disable", "require", "verify-full"].contains(&option(c, "tls_mode", "verify-full")) {
        return Err("Invalid TLS mode".into());
    }
    let port: u16 = field(c, "port")?.parse().map_err(failure)?;
    if port == 0 {
        return Err("Invalid port".into());
    }
    let sql = source.spec.provider["sql"].as_str().ok_or("Missing SQL")?;
    match c.connector.as_str() {
        "postgres" => postgres(c, port, compile(sql, &source.params, false)?).await,
        "mysql" => mysql(c, port, compile(sql, &source.params, true)?).await,
        _ => Err("Unsupported database connector".into()),
    }
}
struct Driver(tokio::task::JoinHandle<()>);
impl Drop for Driver {
    fn drop(&mut self) {
        self.0.abort();
    }
}
async fn postgres(c: &Connection, port: u16, query: crate::sql::Compiled) -> Result<Frame> {
    use tokio_postgres::{config::SslMode, types::ToSql};
    let mut config = tokio_postgres::Config::new();
    config
        .host(option(c, "server_name", field(c, "host")?))
        .port(port)
        .user(field(c, "login")?)
        .dbname(field(c, "schema")?);
    if let Some(password) = c.fields.get("password") {
        config.password(password);
    }
    config.connect_timeout(timeout(c, "connect_timeout")?);
    let mode = option(c, "tls_mode", "verify-full");
    config.ssl_mode(if mode == "disable" {
        SslMode::Disable
    } else {
        SslMode::Require
    });
    let mut tls = native_tls::TlsConnector::builder();
    if mode == "require" {
        tls.danger_accept_invalid_certs(true)
            .danger_accept_invalid_hostnames(true);
    }
    let tls = postgres_native_tls::MakeTlsConnector::new(tls.build().map_err(failure)?);
    let (client, connection) = deadline(timeout(c, "connect_timeout")?, async {
        if let Some(name) = c.extra.get("server_name").and_then(Value::as_str) {
            let addresses: Vec<_> = tokio::net::lookup_host((field(c, "host")?, port))
                .await
                .map_err(failure)?
                .collect();
            let address = addresses.first().ok_or("Database address unavailable")?;
            let _ = name;
            config.hostaddr(address.ip());
        }
        config.connect(tls).await.map_err(failure)
    })
    .await?;
    let _driver = Driver(tokio::spawn(async move {
        let _ = connection.await;
    }));
    deadline(timeout(c, "query_timeout")?, async {
        client
            .batch_execute(&format!(
                "BEGIN READ ONLY; SET LOCAL statement_timeout = {}",
                timeout(c, "query_timeout")?.as_millis()
            ))
            .await
            .map_err(failure)?;
        let params: Vec<Box<dyn ToSql + Sync + Send>> = query
            .values
            .iter()
            .map(|v| -> Result<Box<dyn ToSql + Sync + Send>> {
                Ok(if let Some(v) = v.as_bool() {
                    Box::new(v)
                } else if let Some(v) = v.as_i64() {
                    Box::new(v)
                } else if let Some(v) = v.as_f64() {
                    Box::new(v)
                } else if let Some(v) = v.as_str() {
                    Box::new(v.to_owned())
                } else {
                    return Err("Invalid SQL parameter".into());
                })
            })
            .collect::<Result<_>>()?;
        // Explicit parameter types retain integer/DOUBLE distinctions and avoid inference from caller literals.
        let types: Vec<_> = query
            .values
            .iter()
            .map(|v| {
                if v.is_boolean() {
                    tokio_postgres::types::Type::BOOL
                } else if v.is_i64() {
                    tokio_postgres::types::Type::INT8
                } else if v.is_number() {
                    tokio_postgres::types::Type::FLOAT8
                } else {
                    tokio_postgres::types::Type::TEXT
                }
            })
            .collect();
        let statement = client
            .prepare_typed(&query.sql, &types)
            .await
            .map_err(failure)?;
        let mut rows = Rows {
            names: statement
                .columns()
                .iter()
                .map(|c| c.name().into())
                .collect(),
            ..Rows::default()
        };
        let stream = client
            .query_raw(
                &statement,
                params.iter().map(|v| v.as_ref() as &(dyn ToSql + Sync)),
            )
            .await
            .map_err(failure)?;
        pin_mut!(stream);
        while let Some(row) = stream.next().await {
            let row = row.map_err(failure)?;
            rows.append(
                (0..row.len())
                    .map(|i| postgres_value(&row, i))
                    .collect::<Result<_>>()?,
            )?;
        }
        let types = statement
            .columns()
            .iter()
            .map(|column| {
                use tokio_postgres::types::Type;
                match *column.type_() {
                    Type::BOOL => "boolean",
                    Type::INT2 | Type::INT4 | Type::INT8 => "integer",
                    Type::FLOAT4 | Type::FLOAT8 => "number",
                    _ => "string",
                }
                .to_owned()
            })
            .collect();
        rows.frame(types)
    })
    .await
}
struct Raw(Vec<u8>);
impl<'a> tokio_postgres::types::FromSql<'a> for Raw {
    fn from_sql(
        _: &tokio_postgres::types::Type,
        raw: &'a [u8],
    ) -> std::result::Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Self(raw.to_vec()))
    }
    fn accepts(_: &tokio_postgres::types::Type) -> bool {
        true
    }
}
fn postgres_value(row: &tokio_postgres::Row, i: usize) -> Result<Value> {
    use tokio_postgres::types::Type;
    let Some(raw) = row.try_get::<_, Option<Raw>>(i).map_err(failure)? else {
        return Ok(Value::Null);
    };
    let t = row.columns()[i].type_();
    Ok(match *t {
        Type::BOOL => Value::from(row.try_get::<_, bool>(i).map_err(failure)?),
        Type::INT2 => Value::from(row.try_get::<_, i16>(i).map_err(failure)?),
        Type::INT4 => Value::from(row.try_get::<_, i32>(i).map_err(failure)?),
        Type::INT8 => Value::from(row.try_get::<_, i64>(i).map_err(failure)?),
        Type::FLOAT4 => finite(row.try_get::<_, f32>(i).map_err(failure)? as f64)?,
        Type::FLOAT8 => finite(row.try_get::<_, f64>(i).map_err(failure)?)?,
        Type::NUMERIC => Value::String(numeric(&raw.0)?),
        Type::BYTEA => Value::String(base64::engine::general_purpose::STANDARD.encode(raw.0)),
        Type::UUID => Value::String(
            uuid::Uuid::from_slice(&raw.0)
                .map_err(failure)?
                .hyphenated()
                .to_string()
                .to_uppercase(),
        ),
        Type::MONEY => {
            let cents = i64::from_be_bytes(raw.0.try_into().map_err(|_| "Invalid money")?);
            let amount = cents.unsigned_abs();
            Value::String(format!(
                "{}{}.{:02}",
                if cents < 0 { "-" } else { "" },
                amount / 100,
                amount % 100
            ))
        }
        Type::DATE => Value::String(
            row.try_get::<_, chrono::NaiveDate>(i)
                .map_err(failure)?
                .format("%Y-%m-%d")
                .to_string(),
        ),
        Type::TIMESTAMP => Value::String(
            row.try_get::<_, chrono::NaiveDateTime>(i)
                .map_err(failure)?
                .format("%Y-%m-%dT%H:%M:%S%.6fZ")
                .to_string(),
        ),
        Type::TIMESTAMPTZ => Value::String(
            row.try_get::<_, chrono::DateTime<chrono::Utc>>(i)
                .map_err(failure)?
                .format("%Y-%m-%dT%H:%M:%S%.6fZ")
                .to_string(),
        ),
        Type::TIME => Value::String(
            row.try_get::<_, chrono::NaiveTime>(i)
                .map_err(failure)?
                .format("%H:%M:%S%.6f")
                .to_string(),
        ),
        Type::JSONB => Value::String(
            String::from_utf8(raw.0.get(1..).ok_or("Invalid JSONB")?.to_vec()).map_err(failure)?,
        ),
        _ => Value::String(String::from_utf8(raw.0).map_err(failure)?),
    })
}
fn finite(n: f64) -> Result<Value> {
    if n.is_finite() {
        Ok(Value::from(n))
    } else {
        Err("Nonfinite database value".into())
    }
}
// PostgreSQL NUMERIC is a base-10000 coefficient plus decimal scale. Preserve scale and arbitrary precision.
fn numeric(bytes: &[u8]) -> Result<String> {
    if bytes.len() < 8 || !bytes.len().is_multiple_of(2) {
        return Err("Invalid decimal".into());
    }
    let word = |i| i16::from_be_bytes([bytes[i], bytes[i + 1]]);
    let count = word(0);
    let weight = word(2);
    let sign = word(4) as u16;
    let scale = word(6);
    if count < 0
        || bytes.len() != 8 + count as usize * 2
        || scale < 0
        || ![0, 0x4000].contains(&sign)
    {
        return Err("Invalid decimal".into());
    }
    let digits: Vec<_> = bytes[8..]
        .chunks_exact(2)
        .map(|b| u16::from_be_bytes([b[0], b[1]]))
        .collect();
    if digits.iter().any(|n| *n >= 10000) {
        return Err("Invalid decimal".into());
    }
    let digit = |place: i32| -> u16 {
        let i = i32::from(weight) - place;
        if i < 0 {
            0
        } else {
            digits.get(i as usize).copied().unwrap_or(0)
        }
    };
    let mut result = if sign == 0x4000 {
        "-".to_owned()
    } else {
        String::new()
    };
    if weight < 0 {
        result.push('0');
    } else {
        for place in (0..=i32::from(weight)).rev() {
            if place == i32::from(weight) {
                result.push_str(&digit(place).to_string());
            } else {
                result.push_str(&format!("{:04}", digit(place)));
            }
        }
    }
    if scale > 0 {
        result.push('.');
        let mut fraction = String::new();
        for i in 1..=((i32::from(scale) + 3) / 4) {
            fraction.push_str(&format!("{:04}", digit(-i)));
        }
        fraction.truncate(scale as usize);
        result.push_str(&fraction);
    }
    Ok(result)
}
async fn mysql(c: &Connection, port: u16, query: crate::sql::Compiled) -> Result<Frame> {
    use mysql_async::{prelude::Queryable, OptsBuilder, Params, SslOpts};
    let mut options = OptsBuilder::default()
        .ip_or_hostname(field(c, "host")?)
        .tcp_port(port)
        .user(Some(field(c, "login")?))
        .pass(c.fields.get("password").cloned())
        .db_name(Some(field(c, "schema")?))
        .prefer_socket(false);
    let mode = option(c, "tls_mode", "verify-full");
    if mode != "disable" {
        let mut tls = SslOpts::default()
            .with_danger_accept_invalid_certs(mode == "require")
            .with_danger_skip_domain_validation(mode == "require");
        if let Some(name) = c.extra.get("server_name").and_then(Value::as_str) {
            tls = tls.with_danger_tls_hostname_override(Some(name.to_owned()));
        }
        options = options.ssl_opts(tls);
    }
    let mut conn = deadline(timeout(c, "connect_timeout")?, async {
        mysql_async::Conn::new(options).await.map_err(failure)
    })
    .await?;
    deadline(timeout(c, "query_timeout")?, async {
        conn.query_drop("START TRANSACTION READ ONLY")
            .await
            .map_err(failure)?;
        let values: Vec<_> = query
            .values
            .iter()
            .map(|v| {
                if let Some(v) = v.as_i64() {
                    mysql_async::Value::Int(v)
                } else if let Some(v) = v.as_f64() {
                    mysql_async::Value::Double(v)
                } else if let Some(v) = v.as_bool() {
                    mysql_async::Value::Int(i64::from(v))
                } else {
                    mysql_async::Value::Bytes(v.as_str().unwrap_or_default().as_bytes().to_vec())
                }
            })
            .collect();
        let mut result = conn
            .exec_iter(query.sql, Params::Positional(values))
            .await
            .map_err(failure)?;
        let columns = result.columns().unwrap_or_default();
        let mut rows = Rows {
            names: columns.iter().map(|c| c.name_str().into_owned()).collect(),
            ..Rows::default()
        };
        while let Some(row) = result.next().await.map_err(failure)? {
            rows.append(
                row.unwrap()
                    .into_iter()
                    .zip(columns.iter())
                    .map(|(v, c)| mysql_value(v, c))
                    .collect::<Result<_>>()?,
            )?;
        }
        let types = columns
            .iter()
            .map(|column| {
                use mysql_async::consts::ColumnType as T;
                match column.column_type() {
                    T::MYSQL_TYPE_TINY
                    | T::MYSQL_TYPE_SHORT
                    | T::MYSQL_TYPE_LONG
                    | T::MYSQL_TYPE_INT24
                    | T::MYSQL_TYPE_LONGLONG
                    | T::MYSQL_TYPE_YEAR
                    | T::MYSQL_TYPE_BIT => "integer",
                    T::MYSQL_TYPE_FLOAT | T::MYSQL_TYPE_DOUBLE => "number",
                    _ => "string",
                }
                .to_owned()
            })
            .collect();
        rows.frame(types)
    })
    .await
}
fn mysql_value(v: mysql_async::Value, column: &mysql_async::Column) -> Result<Value> {
    use mysql_async::{consts::ColumnType, Value as M};
    Ok(match v {
        M::NULL => Value::Null,
        M::Int(n) => n.into(),
        M::UInt(n) => {
            if n <= i64::MAX as u64 {
                Value::from(n)
            } else {
                Value::String(n.to_string())
            }
        }
        M::Float(n) => finite(n as f64)?,
        M::Double(n) => finite(n)?,
        M::Bytes(b) => {
            if column.column_type() == ColumnType::MYSQL_TYPE_BIT && b.len() <= 8 {
                let n = b.iter().fold(0u64, |n, b| (n << 8) | u64::from(*b));
                if n <= i64::MAX as u64 {
                    n.into()
                } else {
                    Value::String(n.to_string())
                }
            } else if column.character_set() == 63
                && matches!(
                    column.column_type(),
                    ColumnType::MYSQL_TYPE_BLOB
                        | ColumnType::MYSQL_TYPE_TINY_BLOB
                        | ColumnType::MYSQL_TYPE_MEDIUM_BLOB
                        | ColumnType::MYSQL_TYPE_LONG_BLOB
                        | ColumnType::MYSQL_TYPE_STRING
                        | ColumnType::MYSQL_TYPE_VAR_STRING
                        | ColumnType::MYSQL_TYPE_VARCHAR
                        | ColumnType::MYSQL_TYPE_GEOMETRY
                )
            {
                Value::String(base64::engine::general_purpose::STANDARD.encode(b))
            } else {
                Value::String(String::from_utf8(b).map_err(failure)?)
            }
        }
        M::Date(y, m, d, h, min, s, us) => {
            Value::String(if column.column_type() == ColumnType::MYSQL_TYPE_DATE {
                format!("{y:04}-{m:02}-{d:02}")
            } else {
                format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}:{s:02}.{us:06}")
            })
        }
        M::Time(negative, days, h, m, s, us) => Value::String(format!(
            "{}{:02}:{m:02}:{s:02}.{us:06}",
            if negative { "-" } else { "" },
            days * 24 + u32::from(h)
        )),
    })
}
pub fn sqlite(source: &Resolved, ctx: &Context, cancel: Arc<Cancellation>) -> Result<Frame> {
    use rusqlite::{types::ValueRef, OpenFlags};
    let path = format!(
        "data/{}",
        source.spec.provider["file_path"]
            .as_str()
            .ok_or("Missing SQLite path")?
    );
    crate::spec::data_path(&path)?;
    let root = std::fs::canonicalize(&ctx.directory).map_err(failure)?;
    let target = root.join(&path);
    let canonical = std::fs::canonicalize(&target).map_err(failure)?;
    if canonical != target || !canonical.starts_with(&root) || !canonical.is_file() {
        return Err("Invalid SQLite file".into());
    }
    let db = rusqlite::Connection::open_with_flags(
        &canonical,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_NOFOLLOW,
    )
    .map_err(failure)?;
    if !(1..=120_000).contains(&ctx.sqlite_timeout_ms) {
        return Err("Invalid SQLite timeout".into());
    }
    let end = Instant::now() + Duration::from_millis(ctx.sqlite_timeout_ms);
    let progress = cancel.clone();
    db.progress_handler(
        1000,
        Some(move || progress.check().is_err() || Instant::now() >= end),
    );
    db.busy_timeout(Duration::from_millis(100))
        .map_err(failure)?;
    let query = compile(
        source.spec.provider["sql"].as_str().ok_or("Missing SQL")?,
        &source.params,
        true,
    )?;
    let mut statement = db.prepare(&query.sql).map_err(failure)?;
    if !statement.readonly() {
        return Err("SQLite source requires a read-only query".into());
    }
    let types = statement
        .columns()
        .iter()
        .map(|column| {
            let declared = column.decl_type().unwrap_or("").to_ascii_uppercase();
            if declared.contains("INT") {
                "integer"
            } else if ["REAL", "FLOA", "DOUB"]
                .iter()
                .any(|part| declared.contains(part))
            {
                "number"
            } else {
                "string"
            }
            .to_owned()
        })
        .collect();
    let mut rows = Rows {
        names: statement
            .column_names()
            .into_iter()
            .map(str::to_owned)
            .collect(),
        ..Rows::default()
    };
    let values: Vec<rusqlite::types::Value> = query
        .values
        .into_iter()
        .map(|v| {
            if let Some(n) = v.as_i64() {
                n.into()
            } else if let Some(n) = v.as_f64() {
                n.into()
            } else if let Some(n) = v.as_bool() {
                i64::from(n).into()
            } else {
                v.as_str().unwrap_or_default().to_owned().into()
            }
        })
        .collect();
    let mut stream = statement
        .query(rusqlite::params_from_iter(values))
        .map_err(failure)?;
    while let Some(row) = stream.next().map_err(|e| {
        if cancel.check().is_err() {
            "Cancelled".into()
        } else {
            failure(e)
        }
    })? {
        cancel.check()?;
        let cells = (0..rows.names.len())
            .map(|i| {
                Ok(match row.get_ref(i).map_err(failure)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(n) => n.into(),
                    ValueRef::Real(n) => finite(n)?,
                    ValueRef::Text(b) => Value::String(String::from_utf8_lossy(b).into_owned()),
                    ValueRef::Blob(b) => {
                        Value::String(base64::engine::general_purpose::STANDARD.encode(b))
                    }
                })
            })
            .collect::<Result<_>>()?;
        rows.append(cells)?;
    }
    rows.frame(types)
}

pub fn snapshot(path: &str) -> Result<Vec<u8>> {
    use rusqlite::{
        backup::{Backup, StepResult},
        OpenFlags,
    };
    let source = rusqlite::Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(failure)?;
    let file = tempfile::NamedTempFile::new().map_err(failure)?;
    let mut destination = rusqlite::Connection::open(file.path()).map_err(failure)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    {
        let backup = Backup::new(&source, &mut destination).map_err(failure)?;
        loop {
            if Instant::now() >= deadline {
                return Err("SQLite backup timed out".into());
            }
            match backup.step(256).map_err(failure)? {
                StepResult::Done => break,
                StepResult::Busy | StepResult::Locked => {
                    std::thread::sleep(Duration::from_millis(10))
                }
                _ => {}
            }
            if file.as_file().metadata().map_err(failure)?.len() > MAX_BYTES {
                return Err("SQLite snapshot exceeds 100 MiB".into());
            }
        }
    }
    destination
        .execute_batch("PRAGMA journal_mode=DELETE")
        .map_err(failure)?;
    drop(destination);
    if file.as_file().metadata().map_err(failure)?.len() > MAX_BYTES {
        return Err("SQLite snapshot exceeds 100 MiB".into());
    }
    std::fs::read(file.path()).map_err(failure)
}

#[cfg(test)]
#[path = "tests/database.rs"]
mod tests;
