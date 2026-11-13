use crate::{
    spec::Source,
    sql::{self, identifier},
    Result, MAX_ROWS,
};
use duckdb::{params_from_iter, types::Value as SqlValue, Config, Connection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Column {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Frame {
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Value>>,
}
pub(crate) struct Engine {
    pub(crate) connection: Connection,
    pub(crate) tables: Vec<String>,
}
fn error(e: impl std::fmt::Display) -> String {
    e.to_string()
}
impl Engine {
    pub fn new() -> Result<Self> {
        let mut config = Config::default();
        for (key, value) in [
            ("temp_directory", ""),
            ("autoinstall_known_extensions", "false"),
            ("autoload_known_extensions", "false"),
            ("memory_limit", "128MB"),
            ("threads", "2"),
        ] {
            config = config.with(key, value).map_err(error)?;
        }
        Ok(Self {
            connection: Connection::open_in_memory_with_flags(config).map_err(error)?,
            tables: vec![],
        })
    }
    pub fn import(&mut self, sources: &BTreeMap<String, Source>, directory: &str) -> Result<()> {
        let inputs = sources
            .iter()
            .map(|(alias, source)| {
                (
                    alias.clone(),
                    crate::runtime::Input {
                        file: source.file.clone(),
                        csv: None,
                        frame: None,
                    },
                )
            })
            .collect();
        self.import_inputs(&inputs, directory, &crate::runtime::Cancellation::default())
    }
    pub fn version(&self) -> Result<String> {
        self.connection
            .query_row("SELECT version()", [], |row| row.get(0))
            .map_err(error)
    }
    pub fn query(&self, query: Option<&str>, params: &BTreeMap<String, Value>) -> Result<Frame> {
        self.query_cancel(query, params, &crate::runtime::Cancellation::default())
    }
    pub(crate) fn query_cancel(
        &self,
        query: Option<&str>,
        params: &BTreeMap<String, Value>,
        cancel: &crate::runtime::Cancellation,
    ) -> Result<Frame> {
        cancel.check()?;
        let fallback;
        let query = match query {
            Some(sql) => sql,
            None => {
                if self.tables.len() != 1 {
                    return Err("No-SQL dataset requires one table".into());
                }
                fallback = format!("SELECT * FROM {}", identifier(&self.tables[0]));
                &fallback
            }
        };
        let compiled = sql::compile(query, params)?;
        let values: Vec<SqlValue> = compiled
            .values
            .iter()
            .map(|value| {
                if let Some(v) = value.as_bool() {
                    Ok(SqlValue::Boolean(v))
                } else if let Some(v) = value.as_i64() {
                    Ok(SqlValue::BigInt(v))
                } else if let Some(v) = value.as_f64() {
                    Ok(SqlValue::Double(v))
                } else if let Some(v) = value.as_str() {
                    Ok(SqlValue::Text(v.into()))
                } else {
                    Err("SQL parameters must be scalars".to_string())
                }
            })
            .collect::<Result<_>>()?;
        let subquery = format!("({}\n) AS _facetra_result", compiled.sql);
        let mut schema = self
            .connection
            .prepare(&format!("DESCRIBE SELECT * FROM {subquery} LIMIT 0"))
            .map_err(error)?;
        let mut schema_rows = schema
            .query(params_from_iter(values.iter()))
            .map_err(error)?;
        let mut columns = Vec::new();
        let mut projections = Vec::new();
        let mut names = BTreeSet::new();
        while let Some(row) = schema_rows.next().map_err(error)? {
            cancel.check()?;
            let name: String = row.get(0).map_err(error)?;
            let sql_type: String = row.get(1).map_err(error)?;
            if name.is_empty() || !names.insert(name.to_lowercase()) {
                return Err("Invalid or repeated output column".into());
            }
            let field = identifier(&name);
            let (kind, expression) = match sql_type.as_str() {
                "BOOLEAN" => ("boolean", field.clone()),
                "TINYINT" | "SMALLINT" | "INTEGER" | "BIGINT" | "UTINYINT" | "USMALLINT"
                | "UINTEGER" => ("integer", format!("CAST({field} AS BIGINT)")),
                "FLOAT" | "DOUBLE" => ("number", format!("CAST({field} AS DOUBLE)")),
                "DATE" => ("date", format!("CAST({field} AS VARCHAR)")),
                t if t.starts_with("TIMESTAMP") => {
                    ("timestamp", format!("CAST({field} AS VARCHAR)"))
                }
                t if t.ends_with("[]")
                    || t.starts_with("STRUCT(")
                    || t.starts_with("MAP(")
                    || t.starts_with("UNION(") =>
                {
                    ("json", format!("to_json({field})"))
                }
                _ => ("string", format!("CAST({field} AS VARCHAR)")),
            };
            columns.push(Column {
                name,
                kind: kind.into(),
            });
            projections.push(format!("{expression} AS {field}"));
        }
        if columns.is_empty() {
            return Err("Query has no columns".into());
        }
        let mut statement = self
            .connection
            .prepare(&format!(
                "SELECT {} FROM {subquery} LIMIT {}",
                projections.join(", "),
                MAX_ROWS + 1
            ))
            .map_err(error)?;
        let mut result = statement
            .query(params_from_iter(values.iter()))
            .map_err(error)?;
        let mut rows = Vec::new();
        while let Some(row) = result.next().map_err(error)? {
            cancel.check()?;
            if rows.len() >= MAX_ROWS {
                return Err("Transform exceeds 100,000 rows".into());
            }
            let mut cells = Vec::with_capacity(columns.len());
            for (index, column) in columns.iter().enumerate() {
                cells.push(match column.kind.as_str() {
                    "integer" => row
                        .get::<_, Option<i64>>(index)
                        .map_err(error)?
                        .map(Value::from)
                        .unwrap_or(Value::Null),
                    "boolean" => row
                        .get::<_, Option<bool>>(index)
                        .map_err(error)?
                        .map(Value::from)
                        .unwrap_or(Value::Null),
                    "number" => match row.get::<_, Option<f64>>(index).map_err(error)? {
                        None => Value::Null,
                        Some(v) => serde_json::Number::from_f64(v)
                            .map(Value::Number)
                            .ok_or("SQL numeric output must be finite")?,
                    },
                    _ => row
                        .get::<_, Option<String>>(index)
                        .map_err(error)?
                        .map(Value::from)
                        .unwrap_or(Value::Null),
                });
            }
            rows.push(cells);
        }
        Ok(Frame { columns, rows })
    }
}
