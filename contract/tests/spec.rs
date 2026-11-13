use facetra_contract::{documents, resolution::*};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
fn document(id: &str) -> Value {
    json!({"params":[{"name":"n","type":"number","default":2,"minimum":1,"maximum":9},{"name":"label","type":"string","default":"owner"},{"name":"flag","type":"boolean","default":true},{"name":"optional","type":"integer","required":false}],
      "sources":{"input":{"source":"duckdb","file":"data/input.csv"}},"dataset":{"id":"points","sources":[{"id":"raw","ref":"#/sources/input"}],"sql":"SELECT $n AS n FROM raw"},
      "metadata":{"id":id,"title":"Fixture"},"chart":{"dataset":"points","renderer":"value","value":"n"}})
}

#[test]
fn metadata_is_required_and_never_part_of_renderer_configuration() {
    let mut valid = document("owner");
    valid["metadata"]["notes"] = json!("Dataset provenance");
    valid["metadata"]["tags"] = json!(["股票", "Macro"]);
    let docs = documents(&[valid.to_string()]).unwrap();
    assert_eq!(
        docs["owner"].metadata.title,
        facetra_contract::models::LocalizedText::Literal("Fixture".into())
    );
    assert_eq!(docs["owner"].chart.dataset, "points");
    for field in ["id", "title", "notes", "tags"] {
        let mut mixed = valid.clone();
        mixed["chart"][field] = mixed["metadata"][field].clone();
        assert!(documents(&[mixed.to_string()]).is_err(), "chart.{field}");
        let mut mixed = valid.clone();
        mixed[field] = mixed["metadata"][field].clone();
        assert!(documents(&[mixed.to_string()]).is_err(), "root {field}");
    }
    for field in ["dataset", "renderer", "params", "folder"] {
        let mut mixed = valid.clone();
        mixed["metadata"][field] = json!("misplaced");
        assert!(documents(&[mixed.to_string()]).is_err(), "metadata.{field}");
    }
    for invalid in [
        Value::Null,
        json!([]),
        json!("metadata"),
        json!({}),
        json!({"id": "owner", "title": 42}),
        json!({"id": "owner", "title": "Fixture", "notes": null}),
        json!({"id": "owner", "title": "Fixture", "tags": [42]}),
    ] {
        let mut doc = valid.clone();
        doc["metadata"] = invalid;
        assert!(documents(&[doc.to_string()]).is_err());
    }
    let mut legacy = valid;
    let metadata = legacy.as_object_mut().unwrap().remove("metadata").unwrap();
    legacy["chart"]
        .as_object_mut()
        .unwrap()
        .extend(metadata.as_object().unwrap().clone());
    assert!(documents(&[legacy.to_string()]).is_err());
}

#[test]
fn date_parameters_validate_iso_calendar_dates_and_keep_string_values() {
    let mut value = document("dates");
    value["params"] = json!([{"name":"day","type":"date","default":"2024-02-29"}]);
    value["dataset"]["sql"] = json!("SELECT $day AS day FROM raw");
    let docs = documents(&[serde_yaml_ng::to_string(&value).unwrap()]).unwrap();
    assert_eq!(
        docs["dates"].parameters(&Default::default()).unwrap()["day"],
        "2024-02-29"
    );
    for invalid in [
        "20240229",
        "2026-02-29",
        "2026-2-01",
        "2026-13-01",
        "0000-01-01",
        "+2026-01-01",
    ] {
        value["params"][0]["default"] = json!(invalid);
        assert!(documents(&[serde_yaml_ng::to_string(&value).unwrap()]).is_err());
    }
}
#[test]
fn parameter_width_is_positive_optional_and_preserves_values() {
    let original = document("widths");
    let expected = documents(&[original.to_string()]).unwrap()["widths"]
        .parameters(&Default::default())
        .unwrap();
    for width in [Value::Null, json!(1), json!(72.5), json!(320)] {
        let mut value = original.clone();
        value["params"][0]["width"] = width.clone();
        let docs = documents(&[value.to_string()]).unwrap();
        let doc = &docs["widths"];
        assert_eq!(doc.params[0].width, width.as_f64());
        assert_eq!(doc.parameters(&Default::default()).unwrap(), expected);
        let encoded = serde_json::to_value(doc).unwrap();
        assert_eq!(encoded["params"][0]["width"].as_f64(), width.as_f64());
        if width.is_null() {
            assert!(encoded["params"][0].get("width").is_none());
        }
    }
    for width in [json!(0), json!(-1), json!("120"), json!(true), json!([120])] {
        let mut value = original.clone();
        value["params"][0]["width"] = width;
        assert!(documents(&[value.to_string()]).is_err());
    }
    let mut params = documents(&[original.to_string()]).unwrap()["widths"]
        .params
        .clone();
    for width in [0.0, -1.0, f64::INFINITY, f64::NAN] {
        params[0].width = Some(width);
        assert!(facetra_contract::validation::validate_definitions(&params).is_err());
    }
}

#[test]
fn document_validation_rejects_ambiguous_sources_datasets_and_schema() {
    let mutations: Vec<(&str, Value)> = vec![
        ("/metadata/title", json!(" ")),
        ("/chart/dataset", json!("other")),
        ("/chart/renderer", json!("unknown")),
        ("/params", json!([{"name":"n","type":"string","default":2}])),
        (
            "/params",
            json!([{"name":"n","type":"number","minimum":9,"maximum":1}]),
        ),
        (
            "/params",
            json!([{"name":"n","type":"integer","options":[1,2],"default":3}]),
        ),
        (
            "/params",
            json!([{"name":"n","type":"integer"},{"name":"n","type":"integer"}]),
        ),
        ("/dataset/sources", json!([])),
        (
            "/dataset/sources",
            json!([{"id":"raw","ref":"#/sources/input"},{"id":"RAW","ref":"#/sources/input"}]),
        ),
        (
            "/sources/input",
            json!({"ref":"#/sources/input","file":"data/input.csv"}),
        ),
        (
            "/sources/input",
            json!({"source":"duckdb","file":"data/input.csv","with":{"n":1}}),
        ),
        (
            "/sources/input",
            json!({"source":"sqlite","file_path":"../outside.sqlite","sql":"SELECT 1"}),
        ),
    ];
    for (path, value) in mutations {
        let mut doc = document("owner");
        *doc.pointer_mut(path).unwrap() = value;
        assert!(documents(&[doc.to_string()]).is_err(), "{path}");
    }
}

#[test]
fn option_sources_resolve_independently_with_existing_reference_scope() {
    let mut owner = document("owner");
    owner["params"]
        .as_array_mut()
        .unwrap()
        .push(json!({"name":"selection","type":"string",
        "options":{"ref":"#/sources/input","value":"code","label":"name"}}));
    let mut target = document("catalog");
    target["sources"]["input"] = json!({"source":"http","url":"https://example.com",
        "params":{"label":"{{ params.label }}"},"format":"json"});
    let docs = documents(&[owner.to_string(), target.to_string()]).unwrap();
    let owner = &docs["owner"];
    assert!(owner.parameters(&BTreeMap::new()).is_err());
    let local = resolve_source(
        &docs,
        "owner",
        &owner.params,
        &BTreeMap::new(),
        "#/sources/input",
        None,
    )
    .unwrap();
    assert!(local.source.params.is_empty());
    let inputs = BTreeMap::from([("label".into(), json!("caller"))]);
    let external = resolve_source(
        &docs,
        "owner",
        &owner.params,
        &inputs,
        "catalog.yaml#/sources/input",
        None,
    )
    .unwrap();
    assert_eq!(external.source.params["label"], "owner"); // Target default, not caller globals.
    assert!(external.dependencies.is_empty());
    let arguments = BTreeMap::from([("label".into(), json!({"param":"label"}))]);
    let explicit = resolve_source(
        &docs,
        "owner",
        &owner.params,
        &inputs,
        "catalog.yaml#/sources/input",
        Some(&arguments),
    )
    .unwrap();
    assert_eq!(explicit.source.params["label"], "caller");
    assert_eq!(explicit.dependencies, BTreeSet::from(["label".into()]));
}
