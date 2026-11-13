use super::*;
use serde_json::json;

#[test]
fn date_templates_preserve_iso_until_explicitly_formatted() {
    let params = BTreeMap::from([("day".into(), json!("2024-02-29"))]);
    let connections = BTreeMap::new();
    assert_eq!(
        template::render("{{ params.day }}", &params, &connections).unwrap(),
        "2024-02-29"
    );
    assert_eq!(
        template::render(
            "{{ params.day | date_format('yyyyMMdd') }}",
            &params,
            &connections
        )
        .unwrap(),
        "20240229"
    );
    assert_eq!(template::references(&serde_json::from_value(json!({"source":"http","url":"https://example.com","params":{"day":"{{ params.day | date_format('yyyyMMdd') }}"}})).unwrap()).unwrap().0,
        BTreeSet::from(["day".into()]));
    for invalid in [
        "{{ params.day | date_format('dd/MM/yyyy') }}",
        "{{ params.day | unknown() }}",
        "{{ connections.market.day | date_format('yyyyMMdd') }}",
        "{{ params.day | date_format('yyyyMMdd') | other }}",
    ] {
        assert!(template::tokens(invalid).is_err());
    }
    for invalid in [
        "20240229",
        "2026-02-29",
        "2026-2-01",
        "2026-08-01T00:00:00Z",
    ] {
        let params = BTreeMap::from([("day".into(), json!(invalid))]);
        assert!(template::render(
            "{{ params.day | date_format('yyyyMMdd') }}",
            &params,
            &connections
        )
        .is_err());
    }
}

#[test]
fn templates_keep_scalar_types_and_escape_only_at_transport() {
    let params = BTreeMap::from([("limit".into(), json!(3)), ("active".into(), json!(true))]);
    let connections = BTreeMap::from([(
        "market".into(),
        Connection {
            fields: BTreeMap::from([("token".into(), "a&b+c".into())]),
            ..Connection::default()
        },
    )]);
    assert_eq!(
        template::render("{{ params.limit }}", &params, &connections).unwrap(),
        3
    );
    assert_eq!(
        template::render("x/{{ params.limit }}", &params, &connections).unwrap(),
        "x/3"
    );
    assert_eq!(
        template::render("{{ connections.market.token }}", &params, &connections).unwrap(),
        "a&b+c"
    );
    assert_eq!(
        template::render_value(
            &json!({"a":["{{ params.active }}",null]}),
            &params,
            &connections
        )
        .unwrap(),
        json!({"a":[true,null]})
    );
    for text in [
        "{{ token }}",
        "{{ connections.x }}",
        "{{ params.x + 1 }}",
        "{{ params.x",
        "{% x %}",
        "}}",
    ] {
        assert!(template::tokens(text).is_err(), "{text}");
    }
}
#[test]
fn json_frames_preserve_null_nested_values_and_large_integers() {
    let bytes = br#"{"items":[{"id":9223372036854775807,"big":18446744073709551615,"n":12.0,"bool":true,"nested":{"x":1}}, {"id":null,"big":"42","n":1.5,"bool":false,"nested":[1,2]}],"outside":"keep"}"#;
    let data = data::SourceData::raw(bytes.to_vec(), "json".into(), vec!["items".into()]).unwrap();
    let cancel = Cancellation::default();
    let frame = data.dataframe(&cancel).unwrap();
    assert_eq!(
        frame
            .columns
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        vec!["big", "bool", "id", "n", "nested"]
    );
    assert_eq!(frame.rows[0][0], "18446744073709551615");
    assert_eq!(frame.rows[0][2], i64::MAX);
    assert_eq!(frame.columns[3].kind, "number");
    assert_eq!(frame.columns[4].kind, "json");
    assert_eq!(frame.rows[1][2], Value::Null);
    let preview = data.preview("json", &cancel).unwrap();
    assert!(preview["text"].as_str().unwrap().contains("outside"));
}
#[test]
fn raw_previews_survive_invalid_table_conversion() {
    let cancel = Cancellation::default();
    for bytes in [b"not JSON".as_slice(), b"[1,true,null]"] {
        let data = data::SourceData::raw(bytes.to_vec(), "json".into(), vec![]).unwrap();
        assert!(data.dataframe(&cancel).is_err());
        assert_eq!(
            data.preview("text", &cancel).unwrap()["text"],
            String::from_utf8_lossy(bytes).as_ref()
        );
        assert!(data.preview("df", &cancel).is_ok());
    }
}

#[test]
fn json_scalar_lists_become_typed_value_columns_including_empty_lists() {
    let cancel = Cancellation::default();
    for (bytes, kind, rows) in [
        ("[]", "string", vec![]),
        (
            "[\"A\",null,\"B\"]",
            "string",
            vec![vec![json!("A")], vec![Value::Null], vec![json!("B")]],
        ),
        ("[1,2.5]", "number", vec![vec![json!(1)], vec![json!(2.5)]]),
        (
            "[true,false]",
            "boolean",
            vec![vec![json!(true)], vec![json!(false)]],
        ),
    ] {
        let data = data::SourceData::raw(bytes.as_bytes().to_vec(), "json".into(), vec![]).unwrap();
        let frame = data.dataframe(&cancel).unwrap();
        assert_eq!(frame.columns[0].name, "value");
        assert_eq!(frame.columns[0].kind, kind);
        assert_eq!(frame.rows, rows);
    }
}
#[test]
fn csv_preserves_strings_quotes_nulls_and_original_text() {
    let raw = b"code,label\n001,\"a,b\"\n002,\n";
    let data = data::SourceData::raw(raw.to_vec(), "csv".into(), vec![]).unwrap();
    let cancel = Cancellation::default();
    assert_eq!(
        data.dataframe(&cancel).unwrap().rows,
        vec![
            vec![json!("001"), json!("a,b")],
            vec![json!("002"), Value::Null]
        ]
    );
    assert_eq!(
        data.preview("text", &cancel).unwrap()["text"],
        String::from_utf8_lossy(raw).as_ref()
    );
}
#[test]
fn source_validation_rejects_invalid_provider_fields_and_templates() {
    for value in [
        json!({"source":"http","url":"http://example.com"}),
        json!({"source":"http","url":"https://u:p@example.com"}),
        json!({"source":"http","url":"https://example.com","method":"get","json":{}}),
        json!({"source":"healthkit","metric":"sleep"}),
        json!({"source":"healthkit","metric":"step_count","days":366}),
        json!({"source":"sqlite","file_path":"../x","sql":"SELECT 1"}),
        json!({"source":"database","connection":"x"}),
        json!({"source":"http","url":"{{ token }}"}),
    ] {
        let source: Source = serde_json::from_value(value.clone()).unwrap();
        assert!(source.validate().is_err(), "{value}");
    }
}
#[test]
fn sql_only_parameters_do_not_change_source_identity() {
    let source: Resolved = serde_json::from_value(json!({"spec":{"source":"http","url":"https://example.com/{{ params.id }}"},"params":{"id":"A","scale":2}})).unwrap();
    let normalized = source.normalized().unwrap();
    assert_eq!(
        normalized.params,
        BTreeMap::from([("id".into(), json!("A"))])
    );
}

#[test]
fn malformed_http_definitions_and_explicit_null_bodies_are_rejected() {
    for extra in [
        json!({"method":"put"}),
        json!({"method":"get","json":null}),
        json!({"method":"post","json":null,"data":{}}),
        json!({"params":{"x":null}}),
        json!({"data":{"x":[]}}),
        json!({"format":"xml"}),
        json!({"format":"csv","recordsPath":["items"]}),
        json!({"url":"https://example.com/#secret"}),
        json!({"url":""}),
        json!({"unexpected":true}),
    ] {
        let mut value = json!({"source":"http","url":"https://example.com"});
        value
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        assert!(
            serde_json::from_value::<Source>(value.clone())
                .unwrap()
                .validate()
                .is_err(),
            "{value}"
        );
    }
    let source: Source = serde_json::from_value(
        json!({"source":"http","url":"https://example.com","method":"post","json":null}),
    )
    .unwrap();
    assert!(source.validate().is_ok());
}
#[test]
fn source_parameter_normalization_is_typed_and_reports_missing_values() {
    let source = json!({"spec":{"source":"http","url":"https://example.com/{{ params.n }}"},"params":{"n":2,"unused":true},"number_params":["n"]});
    let result: Resolved = serde_json::from_value(source.clone()).unwrap();
    let result = result.normalized().unwrap();
    assert!(result.params["n"].is_f64());
    assert_eq!(result.params.len(), 1);
    for change in [json!({}), json!({"n":"2"}), json!({"n":null})] {
        let mut invalid = source.clone();
        invalid["params"] = change;
        assert!(serde_json::from_value::<Resolved>(invalid)
            .unwrap()
            .normalized()
            .is_err());
    }
    assert!(template::render("{{ params.absent }}", &BTreeMap::new(), &BTreeMap::new()).is_err());
    assert!(template::render(
        "{{ connections.db.secret }}",
        &BTreeMap::new(),
        &BTreeMap::new()
    )
    .is_err());
    let mut missing = source;
    missing["number_params"] = json!([]);
    missing["params"] = json!({});
    assert!(serde_json::from_value::<Resolved>(missing)
        .unwrap()
        .normalized()
        .is_err());
}
#[test]
fn json_type_union_and_previews_preserve_sparse_mixed_and_nested_values() {
    let cancel = Cancellation::default();
    let data = data::SourceData::raw(br#"[{"a":true,"b":2,"c":null,"d":{"q":"x"}},{"a":false,"b":"2.5","d":3},{"z":"a,\"b\"\n"}]"#.to_vec(),"json".into(),vec![]).unwrap();
    let frame = data.dataframe(&cancel).unwrap();
    assert_eq!(
        frame
            .columns
            .iter()
            .map(|c| c.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["boolean", "string", "string", "json", "string"]
    );
    assert_eq!(
        frame.rows[1],
        vec![
            json!(false),
            json!("2.5"),
            Value::Null,
            json!("3"),
            Value::Null
        ]
    );
    let table = data::SourceData::table(frame.clone()).unwrap();
    let json = table.preview("json", &cancel).unwrap();
    let rows: Value = serde_json::from_str(json["text"].as_str().unwrap()).unwrap();
    assert_eq!(rows[0]["d"], json!({"q":"x"}));
    assert_eq!(rows[1]["d"], 3);
    assert!(table.preview("text", &cancel).unwrap()["text"]
        .as_str()
        .unwrap()
        .contains("\"a,\"\"b\"\"\n\""));
    assert!(table.preview("df", &cancel).unwrap()["frame"]["rows"].is_array());
    assert!(data.preview("unknown", &cancel).is_err());
    assert!(data::SourceData::raw(vec![], "xml".into(), vec![]).is_err());
    let path = data::SourceData::raw(b"{}".to_vec(), "json".into(), vec!["absent".into()]).unwrap();
    assert!(path.dataframe(&cancel).is_err());
    let bad_csv = data::SourceData::raw(vec![255], "csv".into(), vec![]).unwrap();
    assert!(bad_csv.dataframe(&cancel).is_err());
    cancel.cancel();
    assert!(data.dataframe(&cancel).is_err());
    assert!(data.preview("text", &cancel).is_err());
    assert!(data.dataframe(&Cancellation::default()).is_ok());
}
#[test]
fn acquisition_handles_local_files_and_rejects_removed_providers() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("data")).unwrap();
    std::fs::write(root.path().join("data/in.csv"), "code\n001\n").unwrap();
    let ctx = Context {
        directory: root.path().to_str().unwrap().into(),
        ..Context::default()
    };
    let csv: Resolved =
        serde_json::from_value(json!({"spec":{"source":"duckdb","file":"data/in.csv"}})).unwrap();
    let data = acquire(csv, ctx, Arc::default()).unwrap();
    assert_eq!(
        data.dataframe(&Cancellation::default()).unwrap().rows,
        vec![vec![json!("001")]]
    );
    let removed: Resolved =
        serde_json::from_value(json!({"spec":{"source":"healthkit","metric":"step_count"}}))
            .unwrap();
    assert!(removed
        .normalized()
        .err()
        .unwrap()
        .contains("no longer supported"));
    let invalid: Resolved = serde_json::from_value(json!({"spec":{"source":"unknown"}})).unwrap();
    assert!(acquire(invalid, Context::default(), Arc::default()).is_err());
}
#[test]
fn database_source_identity_only_includes_bound_sql_parameters() {
    let source:Resolved=serde_json::from_value(json!({"spec":{"source":"database","connection":"db","sql":"SELECT $category AS value /* $scale */"},"params":{"category":"books","scale":2.0},"number_params":["scale"]})).unwrap();
    let normalized = source.normalized().unwrap();
    assert_eq!(
        normalized.params,
        BTreeMap::from([("category".into(), json!("books"))])
    );
    assert!(normalized.number_params.is_empty());
    for value in [Value::Null, json!([]), json!({}), json!(u64::MAX)] {
        assert!(
            database::compile("SELECT $x", &BTreeMap::from([("x".into(), value)]), true).is_err()
        );
    }
}
