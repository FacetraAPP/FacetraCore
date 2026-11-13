use facetra_contract::{self as contract, resolution};
use serde_json::{json, Value};
use std::collections::BTreeMap;

fn chart(id: &str) -> Value {
    json!({"metadata":{"id":id,"title":"Source DAG"},
        "params":[{"name":"region","type":"string","default":"owner"}],
        "sources":{
            "login":{"source":"http","url":"https://example.com/login","params":{"region":"{{ params.region }}"}},
            "items":{"source":"http","depends_on":["login"],"url":"https://example.com/items",
                "headers":{"Authorization":"Bearer {{ sources.login.token }}"}},
            "detail":{"source":"http","depends_on":["items","login"],"url":"https://example.com/detail",
                "params":{"id":"{{ sources.items.0.id }}"}}
        },
        "dataset":{"id":"data","sources":[{"id":"raw","ref":"#/sources/detail"}]},
        "chart":{"dataset":"data","renderer":"value","value":"count"}})
}

#[test]
fn dag_resolves_transitive_and_diamond_dependencies_in_owner_scope() {
    let owner = chart("owner");
    let mut consumer = chart("consumer");
    consumer["params"][0]["default"] = json!("caller");
    consumer["sources"] = json!({"alias":{"ref":"owner.yaml#/sources/detail"}});
    consumer["dataset"]["sources"][0]["ref"] = json!("#/sources/alias");
    for explicit in [false, true] {
        if explicit {
            consumer["sources"]["alias"]["with"] = json!({"region":{"param":"region"}});
        }
        let docs = contract::documents(&[owner.to_string(), consumer.to_string()]).unwrap();
        let doc = &docs["consumer"];
        contract::validate_chart(doc, &docs).unwrap();
        let result =
            resolution::resolve(doc, &docs, &doc.parameters(&BTreeMap::new()).unwrap()).unwrap();
        let detail = &result.sources["raw"];
        assert_eq!(detail.upstream.len(), 2);
        assert_eq!(
            detail.upstream["login"].params["region"],
            if explicit { "caller" } else { "owner" }
        );
        assert_eq!(
            detail.upstream["login"].params,
            detail.upstream["items"].upstream["login"].params
        );
    }
}

#[test]
fn dag_rejects_missing_self_indirect_and_mixed_ref_cycles_before_execution() {
    for (dependencies, alias, code) in [
        (json!(["absent"]), None, "reference.missing"),
        (json!(["detail"]), None, "reference.cycle"),
        (json!(["detail"]), Some("detail"), "reference.cycle"),
        (json!(["alias"]), Some("detail"), "reference.cycle"),
    ] {
        let mut value = chart("cycle");
        value["sources"]["detail"]["depends_on"] = dependencies;
        value["sources"]["detail"]["params"] = json!({});
        if let Some(target) = alias {
            value["sources"]["alias"] = json!({"ref":format!("#/sources/{target}")});
        }
        let docs = contract::documents(&[value.to_string()]).unwrap();
        assert_eq!(
            contract::validate_chart(&docs["cycle"], &docs)
                .unwrap_err()
                .code,
            code
        );
        assert!(resolution::resolve(&docs["cycle"], &docs, &BTreeMap::new()).is_err());
    }
    let mut value = chart("indirect");
    value["sources"]["login"]["depends_on"] = json!(["detail"]);
    let docs = contract::documents(&[value.to_string()]).unwrap();
    assert_eq!(
        contract::validate_chart(&docs["indirect"], &docs)
            .unwrap_err()
            .code,
        "reference.cycle"
    );
}

#[test]
fn dependency_shapes_and_templates_are_strict() {
    for invalid in [
        json!("login"),
        json!(["login", "login"]),
        json!(["owner.yaml#/sources/login"]),
        json!([""]),
        json!([3]),
    ] {
        let mut value = chart("invalid");
        value["sources"]["items"]["depends_on"] = invalid;
        let decoded = contract::documents(&[value.to_string()]);
        assert!(
            decoded.is_err()
                || decoded
                    .is_ok_and(|docs| contract::validate_chart(&docs["invalid"], &docs).is_err())
        );
    }
    for text in ["{{ sources.login.token }}", "{{ sources.unknown }}"] {
        let mut value = chart("undeclared");
        value["sources"]["login"]["headers"] = json!({"X-Test":text});
        let docs = contract::documents(&[value.to_string()]);
        assert!(docs.is_err() || docs.is_ok_and(|docs| contract::validate_chart(&docs["undeclared"], &docs).is_err()));
    }
    for invalid in [
        "{{ sources }}",
        "{{ sources.login..token }}",
        "{{ sources.login[0] }}",
        "{{ sources.login | date_format('yyyyMMdd') }}",
    ] {
        assert!(contract::template::tokens(invalid).is_err());
    }
}

#[test]
fn option_sources_track_transitive_parameters_and_detect_their_own_value() {
    let mut value = chart("options");
    value["params"][0]["options"] = json!({"ref":"#/sources/detail","value":"name"});
    let docs = contract::documents(&[value.to_string()]).unwrap();
    assert_eq!(
        contract::validate_chart(&docs["options"], &docs)
            .unwrap_err()
            .code,
        "parameter.options_cycle"
    );
    let owner = &docs["options"];
    let resolved = resolution::resolve_source(
        &docs,
        "options",
        &owner.params,
        &BTreeMap::new(),
        "#/sources/detail",
        None,
    )
    .unwrap();
    assert!(resolved.dependencies.contains("region"));
    assert_eq!(resolved.source.upstream["login"].params["region"], "owner");
}

#[test]
fn external_sources_require_transitive_inputs_and_reject_invalid_bindings() {
    let mut owner = chart("owner");
    owner["params"][0]
        .as_object_mut()
        .unwrap()
        .remove("default");
    let mut consumer = chart("consumer");
    consumer["sources"] = json!({});
    consumer["dataset"]["sources"][0]["ref"] = json!("owner.yaml#/sources/detail");
    let docs = contract::documents(&[owner.to_string(), consumer.to_string()]).unwrap();
    assert_eq!(
        contract::validate_chart(&docs["consumer"], &docs)
            .unwrap_err()
            .code,
        "binding.required"
    );
    consumer["dataset"]["sources"][0]["with"] = json!({"region":false});
    let docs = contract::documents(&[owner.to_string(), consumer.to_string()]).unwrap();
    assert_eq!(
        contract::validate_chart(&docs["consumer"], &docs)
            .unwrap_err()
            .code,
        "binding.value"
    );
}

#[test]
fn dense_dependency_expansion_fails_with_a_bounded_error() {
    let mut value = chart("dense");
    value["sources"] = json!({});
    for i in 0usize..19 {
        let dependencies: Vec<_> = (i.saturating_sub(2)..i).map(|n| format!("n{n}")).collect();
        value["sources"][format!("n{i}")] =
            json!({"source":"http","url":"https://example.com","depends_on":dependencies});
    }
    value["dataset"]["sources"][0]["ref"] = json!("#/sources/n18");
    let docs = contract::documents(&[value.to_string()]).unwrap();
    let error = resolution::resolve(&docs["dense"], &docs, &BTreeMap::new())
        .err()
        .unwrap();
    assert!(error.contains("4096"));
}

#[test]
fn absent_null_and_empty_dependencies_preserve_independent_sources() {
    for dependencies in [None, Some(Value::Null), Some(json!([]))] {
        let mut value = chart("independent");
        value["sources"] = json!({"detail":{"source":"http","url":"https://example.com"}});
        if let Some(dependencies) = dependencies {
            value["sources"]["detail"]["depends_on"] = dependencies;
        }
        let docs = contract::documents(&[value.to_string()]).unwrap();
        contract::validate_chart(&docs["independent"], &docs).unwrap();
        assert!(
            resolution::resolve(&docs["independent"], &docs, &BTreeMap::new())
                .unwrap()
                .sources["raw"]
                .upstream
                .is_empty()
        );
    }
}
