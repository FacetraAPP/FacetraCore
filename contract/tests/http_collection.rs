use facetra_contract::{self as contract, models::Source};
use serde_json::{json, Value};

fn valid_source(extra: Value) -> Source {
    let mut value = json!({"source":"http","url":"https://example.com"});
    value
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    serde_json::from_value(value).unwrap()
}
#[test]
fn collection_contract_accepts_state_objects_expressions_formats_and_explicit_edges() {
    for extra in [
        json!({"pagination":{"initial":{"n":1},"next":"if page.n < 3 { #{n: page.n + 1} } else { () }"}}),
        json!({"format":"html","extract":{"rows":"article","fields":{"title":{"selector":"h2"}}}}),
        json!({"format":"xml","extract":{"rows":"/a:feed/a:entry","namespaces":{"a":"urn:atom"},"fields":{"title":{"selector":"a:title"}}}}),
        json!({"depends_on":["list"],"foreach":{"source":"list"},"params":{"id":"{{ item.id }}"}}),
        json!({"limits":{"max_requests":20},"retry":{"max_attempts":2}}),
        json!({"method":"post","retry":{"retry_post":true}}),
    ] {
        valid_source(extra).validate().unwrap();
    }
}
#[test]
fn collection_contract_rejects_out_of_scope_locals_invalid_limits_and_unsafe_retry() {
    for extra in [
        json!({"params":{"id":"{{ item.id }}"}}),
        json!({"params":{"n":"{{ page.n }}"}}),
        json!({"foreach":{"source":"list"}}),
        json!({"depends_on":["other"],"foreach":{"source":"list"}}),
        json!({"pagination":{"initial":{},"next":"loop {}"}}),
        json!({"pagination":{"initial":{},"next":"()","max_pages":0}}),
        json!({"pagination":{"initial":{},"next":"()","max_pages":1001}}),
        json!({"pagination":{"initial":{"n":"{{ page.n }}"},"next":"()"}}),
        json!({"pagination":{"initial":{"n":"{{ sources.list.n }}"},"next":"()"}}),
        json!({"pagination":{"initial":{},"next":"()","records":"response.json.items"},"recordsPath":["items"]}),
        json!({"format":"html"}),
        json!({"format":"xml"}),
        json!({"extract":{"rows":"a","fields":{"x":{"selector":"b"}}}}),
        json!({"limits":{"max_requests":0}}),
        json!({"limits":{"max_bytes":104857601}}),
        json!({"retry":{"max_attempts":6}}),
        json!({"method":"post","retry":{}}),
        json!({"format":"html","extract":{"rows":"a","fields":{"x":{"selector":"b","json":true,"resolve_url":true}}}}),
    ] {
        assert!(valid_source(extra.clone()).validate().is_err(), "{extra}");
    }
}
#[test]
fn iteration_initial_parameters_and_connections_are_part_of_dependency_analysis() {
    let source = valid_source(
        json!({"depends_on":["login","list"],"foreach":{"source":"list"},
        "pagination":{"initial":{"n":"{{ params.start }}","url":"{{ connections.api.url }}","token":"{{ sources.login.token }}"},"next":"()"}}),
    );
    source.validate().unwrap();
    let (params, connections) = contract::template::references(&source).unwrap();
    assert!(params.contains("start"));
    assert!(connections.contains(&("api".into(), "url".into())));
    assert_eq!(
        contract::template::source_references(&source)
            .unwrap()
            .into_iter()
            .collect::<Vec<_>>(),
        vec!["list", "login"]
    );
}
