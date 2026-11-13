//! Behavioral promises that must survive implementation and dependency changes.
use facetra_contract::{self as contract, models::*, resolution};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn chart(id: &str) -> Value {
    json!({"metadata":{"id":id,"title":"Stable v1"},
        "sources":{"input":{"source":"duckdb","file":"data/input.csv"}},
        "dataset":{"id":"data","sources":[{"id":"raw","ref":"#/sources/input"}]},
        "chart":{"dataset":"data","renderer":"value","value":"value"}})
}

#[test]
fn http_records_path_uses_the_default_json_format() {
    for format in [None, Some(json!(null)), Some(json!("json"))] {
        let mut value = chart("http_defaults");
        value["sources"]["input"] = json!({"source":"http", "url":"https://example.com/data",
            "recordsPath":["data","items"]});
        if let Some(format) = format {
            value["sources"]["input"]["format"] = format;
        }
        let docs = contract::documents(&[value.to_string()]).unwrap();
        contract::validate_chart(&docs["http_defaults"], &docs).unwrap();
        value["sources"]["input"]["format"] = json!("csv");
        assert!(contract::documents(&[value.to_string()]).is_err());
    }
}

#[test]
fn source_reuse_requires_only_acquisition_parameters_but_dataset_reuse_is_complete() {
    let mut provider = chart("provider");
    provider["params"] = json!([
        {"name":"unrelated","type":"integer"},
        {"name":"region","type":"string","default":"target"},
        {"name":"scale","type":"number","default":2}
    ]);
    provider["dataset"]["sql"] = json!("SELECT $unrelated FROM raw");
    let mut consumer = chart("consumer");
    consumer["params"] = json!([
        {"name":"region","type":"string","default":"caller"},
        {"name":"scale","type":"integer","default":3}
    ]);
    // Exercise both request-template and SQL dependency discovery, through a
    // local alias as well as a direct external source reference.
    for source in [
        json!({"source":"http","url":"https://example.com/data",
            "params":{"region":"{{ params.region }}","scale":"{{ params.scale }}"}}),
        json!({"source":"sqlite","file_path":"input.sqlite",
            "sql":"SELECT $region AS region, $scale AS scale"}),
    ] {
        provider["sources"]["input"] = source;
        for alias in [false, true] {
            for explicit in [false, true] {
                let mut reference = json!({"ref":"provider.yaml#/sources/input"});
                if explicit {
                    reference["with"] =
                        json!({"region":{"param":"region"},"scale":{"param":"scale"}});
                }
                if alias {
                    consumer["sources"]["input"] = reference;
                    consumer["dataset"]["sources"][0] = json!({"id":"raw","ref":"#/sources/input"});
                } else {
                    consumer["sources"]["input"] = chart("consumer")["sources"]["input"].clone();
                    reference["id"] = json!("raw");
                    consumer["dataset"]["sources"][0] = reference;
                }
                let docs =
                    contract::documents(&[provider.to_string(), consumer.to_string()]).unwrap();
                let owner = &docs["consumer"];
                contract::validate_chart(owner, &docs).unwrap();
                let resolved =
                    resolution::resolve(owner, &docs, &owner.parameters(&BTreeMap::new()).unwrap())
                        .unwrap();
                let params = &resolved.sources["raw"].params;
                assert_eq!(params["region"], if explicit { "caller" } else { "target" });
                assert_eq!(
                    params["scale"].as_f64(),
                    Some(if explicit { 3.0 } else { 2.0 })
                );
                assert!(params["scale"].is_f64());
                assert!(!params.contains_key("unrelated"));
            }
        }
    }
    consumer["dataset"] = json!({"id":"data","ref":"provider.yaml#/dataset"});
    consumer["sources"] = json!({});
    let docs = contract::documents(&[provider.to_string(), consumer.to_string()]).unwrap();
    let owner = &docs["consumer"];
    assert_eq!(
        contract::validate_chart(owner, &docs).unwrap_err().code,
        "binding.required"
    );
    assert!(
        resolution::resolve(owner, &docs, &owner.parameters(&BTreeMap::new()).unwrap()).is_err()
    );
}

#[test]
fn source_reuse_still_rejects_missing_or_invalid_acquisition_parameters() {
    let mut provider = chart("provider");
    provider["params"] = json!([{"name":"count","type":"integer","minimum":1}]);
    provider["sources"]["input"] = json!({"source":"http","url":"https://example.com/data",
        "params":{"count":"{{ params.count }}"}});
    let mut consumer = chart("consumer");
    consumer["dataset"]["sources"][0] = json!({"id":"raw","ref":"provider.yaml#/sources/input"});
    let docs = contract::documents(&[provider.to_string(), consumer.to_string()]).unwrap();
    let owner = &docs["consumer"];
    assert_eq!(
        contract::validate_chart(owner, &docs).unwrap_err().code,
        "binding.required"
    );
    let acquired = resolution::resolve(owner, &docs, &BTreeMap::new())
        .and_then(|mut resolved| resolved.sources.remove("raw").unwrap().normalized());
    assert!(acquired.is_err());

    consumer["params"] = json!([{"name":"n","type":"integer"}]);
    consumer["dataset"]["sources"][0]["with"] = json!({"count":{"param":"n"}});
    let docs = contract::documents(&[provider.to_string(), consumer.to_string()]).unwrap();
    let owner = &docs["consumer"];
    contract::validate_chart(owner, &docs).unwrap();
    assert!(resolution::resolve(owner, &docs, &BTreeMap::from([("n".into(), json!(0))])).is_err());
    assert!(resolution::resolve(owner, &docs, &BTreeMap::from([("n".into(), json!(2))])).is_ok());
}

#[test]
fn typed_defaults_and_nulls_keep_their_meaning() {
    let mut value = chart("defaults");
    value["params"] = json!([{"name":"n","type":"integer"},
        {"name":"optional","type":"string","required":false,"default":null}]);
    let doc = contract::chart(&value).unwrap();
    assert!(doc.metadata._notes.is_empty());
    assert!(doc.metadata._tags.is_empty());
    assert_eq!(doc.chart.renderer, "value");
    assert!(doc.params[0].required);
    assert!(!doc.params[0]._advanced);
    assert!(doc.params[1].default.is_none());
    assert_eq!(
        doc.parameters(&BTreeMap::from([("n".into(), json!(12))]))
            .unwrap(),
        BTreeMap::from([("n".into(), json!(12))])
    );
    let dashboard: Dashboard = contract::decode(&json!({"version":1,"kind":"Dashboard",
        "metadata":{"id":"dashboard","title":"Dashboard"},
        "tabs":[{"id":"main","title":"Main","layout":{"type":"grid"},
            "panels":[{"id":"panel","chart":"defaults"}]}]}))
    .unwrap();
    assert!(dashboard.params.is_empty());
    let tab = &dashboard.tabs[0];
    let grid = tab.layout.as_ref().unwrap();
    assert_eq!(
        (grid.columns, grid.row_height, grid.gap, grid.min_width),
        (12, 40.0, 12.0, 1200.0)
    );
    assert_eq!(tab.panels[0].height, 240.0);
    assert!(tab.panels[0].params.is_empty());
    let spec: HttpSpec =
        contract::decode(&json!({"url":"https://example.com","json":null})).unwrap();
    assert_eq!(spec.json, Some(Value::Null));
    let absent: HttpSpec = contract::decode(&json!({"url":"https://example.com"})).unwrap();
    assert_eq!(absent.json, None);
}

#[test]
fn yaml_scalar_and_merge_semantics_are_stable() {
    let parsed = contract::parse("yes: yes\nno: no\nflag: true\nday: 2024-02-29\nquoted: '12'\nnumber: 12.0\nbase: &base {value: 1}\nmerged: {<<: *base, value: 2}\n").unwrap();
    assert_eq!(parsed["yes"], "yes");
    assert_eq!(parsed["no"], "no");
    assert_eq!(parsed["flag"], true);
    assert_eq!(parsed["day"], "2024-02-29");
    assert_eq!(parsed["quoted"], "12");
    assert_eq!(parsed["number"].as_i64(), Some(12));
    assert_eq!(parsed["merged"]["value"], 2);
    for yaml in [
        "value: 1\nvalue: 2",
        "value: .nan",
        "value: !custom 1",
        "value: 1\n---\nvalue: 2",
        "1: value",
    ] {
        assert_eq!(contract::parse(yaml).unwrap_err().code, "yaml.invalid");
    }
}

#[test]
fn unused_exports_can_wait_for_optional_acquisition_inputs() {
    let mut value = chart("owner");
    value["params"] = json!([{"name":"filter","type":"string","required":false}]);
    value["sources"]["extra"] = json!({"source":"http","url":"https://example.com/data",
        "params":{"filter":"{{ params.filter }}"}});
    let docs = contract::documents(&[value.to_string()]).unwrap();
    let owner = &docs["owner"];
    contract::validate_chart(owner, &docs).unwrap();
    // An exported source is only acquired when selected. The CSV dataset can
    // execute without the optional input required by this separate HTTP source.
    let resolved = resolution::resolve(owner, &docs, &BTreeMap::new()).unwrap();
    assert!(resolved.sources["raw"].params.is_empty());
}
