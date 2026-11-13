use super::*;
use mysql_async::{consts::ColumnType as T, Column as MColumn, Value as M};
use serde_json::json;
#[test]
fn mysql_values_preserve_null_decimal_unsigned_blob_and_duration() {
    let column = |kind| MColumn::new(kind).with_character_set(33);
    assert_eq!(
        mysql_value(M::NULL, &column(T::MYSQL_TYPE_VAR_STRING)).unwrap(),
        Value::Null
    );
    let decimal = "12345678901234567890.123456789";
    assert_eq!(
        mysql_value(
            M::Bytes(decimal.as_bytes().to_vec()),
            &column(T::MYSQL_TYPE_NEWDECIMAL)
        )
        .unwrap(),
        decimal
    );
    assert_eq!(
        mysql_value(M::UInt(u64::MAX), &column(T::MYSQL_TYPE_LONGLONG)).unwrap(),
        u64::MAX.to_string()
    );
    assert_eq!(
        mysql_value(
            M::Time(true, 2, 3, 4, 5, 123456),
            &column(T::MYSQL_TYPE_TIME)
        )
        .unwrap(),
        "-51:04:05.123456"
    );
    assert_eq!(
        mysql_value(
            M::Date(2026, 10, 9, 0, 0, 0, 0),
            &column(T::MYSQL_TYPE_DATE)
        )
        .unwrap(),
        "2026-10-09"
    );
    for kind in [
        T::MYSQL_TYPE_BLOB,
        T::MYSQL_TYPE_VARCHAR,
        T::MYSQL_TYPE_VAR_STRING,
    ] {
        assert_eq!(
            mysql_value(
                M::Bytes(vec![0, 255]),
                &MColumn::new(kind).with_character_set(63)
            )
            .unwrap(),
            "AP8="
        );
    }
    assert_eq!(
        mysql_value(M::Int(42), &column(T::MYSQL_TYPE_LONG)).unwrap(),
        42
    );
    assert!(mysql_value(M::Double(f64::INFINITY), &column(T::MYSQL_TYPE_DOUBLE)).is_err());
}
#[test]
fn postgres_decimal_keeps_scale_sign_and_fractional_leading_zeros() {
    fn encoded(weight: i16, sign: i16, scale: i16, digits: &[i16]) -> Vec<u8> {
        [
            vec![digits.len() as i16, weight, sign, scale],
            digits.to_vec(),
        ]
        .concat()
        .into_iter()
        .flat_map(i16::to_be_bytes)
        .collect()
    }
    assert_eq!(numeric(&encoded(0, 0, 2, &[12, 5000])).unwrap(), "12.50");
    assert_eq!(
        numeric(&encoded(-2, 0x4000, 8, &[12])).unwrap(),
        "-0.00000012"
    );
    assert_eq!(
        numeric(&encoded(
            4,
            0,
            9,
            &[1234, 5678, 9012, 3456, 7890, 1234, 5678, 9000]
        ))
        .unwrap(),
        "12345678901234567890.123456789"
    );
    assert_eq!(numeric(&encoded(0, 0, 3, &[])).unwrap(), "0.000");
    assert!(numeric(&encoded(0, 0, 0, &[10000])).is_err());
}
#[test]
fn source_sql_binds_dialects_and_rejects_ambiguous_quotes() {
    let params = BTreeMap::from([("name".into(), json!("'; DELETE FROM orders; --"))]);
    let query = "SELECT $name, $name, '$missing', `$column` /* a /* $ignored */ b */ -- $ignored\n";
    assert!(compile(query, &params, false)
        .unwrap()
        .sql
        .starts_with("SELECT $1, $2"));
    assert!(compile(query, &params, true)
        .unwrap()
        .sql
        .starts_with("SELECT ?, ?"));
    assert_eq!(compile(query, &params, true).unwrap().values.len(), 2);
    for query in [
        "DELETE FROM x",
        "SELECT 1; SELECT 2",
        "SELECT ?",
        "SELECT $$abc$$",
        "SELECT '$x\\'",
        "SELECT $missing",
        "SELECT 1 # mysql comment",
    ] {
        assert!(compile(query, &params, true).is_err(), "{query}");
    }
}
#[test]
fn sqlite_snapshot_includes_wal_and_sources_remain_read_only() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("data")).unwrap();
    let path = root.path().join("data/input.sqlite");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE input (id INTEGER, code TEXT, value REAL, blob BLOB); INSERT INTO input VALUES (1,'001',NULL,x'00ff')").unwrap();
    let bytes = snapshot(path.to_str().unwrap()).unwrap();
    std::fs::write(root.path().join("data/copy.sqlite"), bytes).unwrap();
    let ctx = Context {
        directory: root.path().to_str().unwrap().into(),
        ..Context::default()
    };
    let source = |sql| {
        serde_json::from_value::<Resolved>(json!({"spec":{"source":"sqlite","file_path":"copy.sqlite","sql":sql},"params":{"id":1}})).unwrap()
    };
    let frame = sqlite(
        &source("SELECT * FROM input WHERE id = $id"),
        &ctx,
        Arc::new(Cancellation::default()),
    )
    .unwrap();
    assert_eq!(
        frame.rows,
        vec![vec![json!(1), json!("001"), Value::Null, json!("AP8=")]]
    );
    for query in [
        "DELETE FROM input",
        "WITH selected AS (SELECT 1) DELETE FROM input",
    ] {
        assert!(sqlite(&source(query), &ctx, Arc::new(Cancellation::default())).is_err());
    }
    std::os::unix::fs::symlink(&path, root.path().join("data/link.sqlite")).unwrap();
    let mut escape = source("SELECT * FROM input");
    escape
        .spec
        .provider
        .insert("file_path".into(), json!("link.sqlite"));
    assert!(sqlite(&escape, &ctx, Arc::new(Cancellation::default())).is_err());
}
#[test]
fn rows_normalize_mixed_types_and_preserve_null() {
    let mut rows = Rows {
        names: vec!["a".into(), "b".into()],
        ..Rows::default()
    };
    rows.append(vec![json!(1), Value::Null]).unwrap();
    rows.append(vec![json!("text"), json!(false)]).unwrap();
    let frame = rows.frame(vec![]).unwrap();
    assert_eq!(frame.columns[0].kind, "string");
    assert_eq!(frame.rows[0], vec![json!("1"), Value::Null]);
    assert_eq!(frame.columns[1].kind, "boolean");
}

#[test]
fn sqlite_deadline_cancel_empty_types_and_bound_scalar_values() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("data")).unwrap();
    let db = rusqlite::Connection::open(root.path().join("data/a.sqlite")).unwrap();
    db.execute_batch(
        "CREATE TABLE t (i INTEGER,n REAL,s TEXT);INSERT INTO t VALUES (NULL,NULL,NULL)",
    )
    .unwrap();
    let mut ctx = Context {
        directory: root.path().to_str().unwrap().into(),
        sqlite_timeout_ms: 20,
        ..Context::default()
    };
    let source = |sql: &str| {
        serde_json::from_value::<Resolved>(json!({"spec":{"source":"sqlite","file_path":"a.sqlite","sql":sql},"params":{"i":9223372036854775807i64,"n":1.5,"b":true,"s":"a\u{0}' OR 1=1"}})).unwrap()
    };
    for sql in ["SELECT * FROM t", "SELECT * FROM t WHERE 0"] {
        let frame = sqlite(&source(sql), &ctx, Arc::default()).unwrap();
        assert_eq!(
            frame
                .columns
                .iter()
                .map(|c| c.kind.as_str())
                .collect::<Vec<_>>(),
            vec!["integer", "number", "string"]
        );
    }
    assert_eq!(
        sqlite(
            &source("SELECT $i AS i,$n AS n,$b AS b,$s AS s"),
            &ctx,
            Arc::default()
        )
        .unwrap()
        .rows[0],
        vec![
            json!(i64::MAX),
            json!(1.5),
            json!(1),
            json!("a\u{0}' OR 1=1")
        ]
    );
    let start = Instant::now();
    assert!(sqlite(&source("WITH RECURSIVE r(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM r WHERE x<100000000) SELECT sum(x) FROM r"),&ctx,Arc::default()).is_err());
    assert!(start.elapsed() < Duration::from_secs(2));
    let cancel = Arc::new(Cancellation::default());
    cancel.cancel();
    assert!(sqlite(&source("SELECT * FROM t"), &ctx, cancel).is_err());
    ctx.sqlite_timeout_ms = 0;
    assert!(sqlite(&source("SELECT * FROM t"), &ctx, Arc::default()).is_err());
}
#[test]
fn database_limits_validate_row_width_bytes_count_and_finite_values() {
    let mut rows = Rows {
        names: vec!["x".into()],
        ..Rows::default()
    };
    assert!(rows.append(vec![]).is_err());
    rows.bytes = MAX_BYTES as usize;
    assert!(rows.append(vec![json!("x")]).is_err());
    let mut rows = Rows {
        names: vec!["x".into()],
        rows: vec![vec![Value::Null]; MAX_ROWS],
        ..Rows::default()
    };
    assert!(rows.append(vec![Value::Null]).is_err());
    assert!(finite(f64::NAN).is_err());
    for bytes in [
        vec![],
        vec![0; 9],
        vec![255; 8],
        vec![0, 0, 0, 0, 0xC0, 0, 0, 0],
    ] {
        assert!(numeric(&bytes).is_err());
    }
}
#[test]
fn database_configuration_and_empty_values_fail_before_dialing() {
    let source: Resolved = serde_json::from_value(
        json!({"spec":{"source":"database","connection":"db","sql":"SELECT 1"}}),
    )
    .unwrap();
    assert!(super::super::executor()
        .block_on(load(&source, &Context::default()))
        .is_err());
    let good = Connection {
        connector: "invalid".into(),
        fields: [("port".into(), "5432".into())].into(),
        ..Connection::default()
    };
    for c in [
        good.clone(),
        Connection {
            extra: [("tls_mode".into(), json!("prefer"))].into(),
            ..good.clone()
        },
        Connection {
            fields: [("port".into(), "0".into())].into(),
            ..good.clone()
        },
        Connection {
            fields: BTreeMap::new(),
            ..good.clone()
        },
    ] {
        let ctx = Context {
            connections: [("db".into(), c)].into(),
            ..Context::default()
        };
        assert!(super::super::executor()
            .block_on(load(&source, &ctx))
            .is_err());
    }
    for value in [json!(0), json!(121), json!("1"), Value::Null] {
        let c = Connection {
            extra: [("connect_timeout".into(), value)].into(),
            ..Connection::default()
        };
        assert!(timeout(&c, "connect_timeout").is_err());
    }
    assert_eq!(
        timeout(&Connection::default(), "connect_timeout").unwrap(),
        Duration::from_secs(30)
    );
}
#[test]
fn mysql_small_unsigned_bit_float_datetime_and_invalid_utf8() {
    assert_eq!(
        mysql_value(M::UInt(42), &MColumn::new(T::MYSQL_TYPE_LONGLONG)).unwrap(),
        42
    );
    assert_eq!(
        mysql_value(M::Float(1.5), &MColumn::new(T::MYSQL_TYPE_FLOAT)).unwrap(),
        1.5
    );
    assert_eq!(
        mysql_value(M::Bytes(vec![1, 2]), &MColumn::new(T::MYSQL_TYPE_BIT)).unwrap(),
        258
    );
    assert_eq!(
        mysql_value(M::Bytes(vec![255; 8]), &MColumn::new(T::MYSQL_TYPE_BIT)).unwrap(),
        u64::MAX.to_string()
    );
    assert_eq!(
        mysql_value(
            M::Date(2026, 10, 12, 1, 2, 3, 4),
            &MColumn::new(T::MYSQL_TYPE_DATETIME)
        )
        .unwrap(),
        "2026-10-12 01:02:03.000004"
    );
    assert!(mysql_value(
        M::Bytes(vec![255]),
        &MColumn::new(T::MYSQL_TYPE_VAR_STRING).with_character_set(33)
    )
    .is_err());
}
