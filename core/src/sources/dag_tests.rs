use super::integration_tests::Http;
use super::*;
use serde_json::json;

fn resolved(spec: Value, upstream: Value) -> Resolved {
    serde_json::from_value::<Resolved>(json!({"spec":spec,"upstream":upstream}))
        .unwrap()
        .normalized()
        .unwrap()
}

#[test]
fn http_dag_passes_raw_json_and_typed_nested_bodies_without_reinterpreting_values() {
    let login = Http::new();
    let detail = Http::new();
    login.respond(
        br#"{"token":"{{ params.secret }}","ids":[7,8],"active":true,"empty":null,"meta":{"x":1}}"#,
        200,
        0.0,
        json!({}),
    );
    detail.respond(br#"[{"value":42}]"#, 200, 0.0, json!({}));
    let mut ctx = detail.context();
    ctx.http_roots.extend(login.context().http_roots);
    let upstream = login.source();
    let source = resolved(
        json!({"source":"http","depends_on":["login"],"url":detail.source().spec.provider["url"],"method":"post",
        "headers":{"X-Token":"{{ sources.login.token }}"},"params":{"id":"{{ sources.login.ids.0 }}"},
        "json":{"ids":"{{ sources.login.ids }}","active":"{{ sources.login.active }}","empty":"{{ sources.login.empty }}","meta":"{{ sources.login.meta }}"}}),
        json!({"login":upstream}),
    );
    let cache = SourceCache::default();
    let cancel = Cancellation::default();
    let (data, first_revision) = cache.load(source.clone(), &ctx, &cancel).unwrap();
    assert_eq!(data.dataframe(&cancel).unwrap().rows[0][0], 42);
    let requests = detail.requests();
    assert_eq!(requests[0]["path"], "/data?id=7");
    assert_eq!(requests[0]["headers"]["x-token"], "{{ params.secret }}");
    use base64::Engine as _;
    let body = base64::engine::general_purpose::STANDARD
        .decode(requests[0]["body"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap(),
        json!({"ids":[7,8],"active":true,"empty":null,"meta":{"x":1}})
    );
    assert_eq!(cache.load(source, &ctx, &cancel).unwrap().1, first_revision);
    assert_eq!(login.requests().len(), 1);
    assert_eq!(detail.requests().len(), 1);
}

#[test]
fn diamond_dag_acquires_shared_ancestor_once_and_honors_order_only_edges() {
    let server = Http::new();
    let ctx = server.context();
    let common = server.source();
    let base = common.spec.provider["url"].as_str().unwrap();
    let left = resolved(
        json!({"source":"http","url":format!("{base}/left"),"depends_on":["common"]}),
        json!({"common":common}),
    );
    let right = resolved(
        json!({"source":"http","url":format!("{base}/right"),"depends_on":["common"]}),
        json!({"common":common}),
    );
    let root = resolved(
        json!({"source":"http","url":format!("{base}/root"),"depends_on":["left","right"]}),
        json!({"left":left,"right":right}),
    );
    SourceCache::default()
        .load(root, &ctx, &Cancellation::default())
        .unwrap();
    let paths: Vec<_> = server
        .requests()
        .iter()
        .map(|r| r["path"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(paths, ["/data", "/data/left", "/data/right", "/data/root"]);
}

#[test]
fn changed_transitive_csv_input_invalidates_downstream_without_a_host_revision_change() {
    let server = Http::new();
    let mut ctx = server.context();
    let input: Resolved =
        serde_json::from_value(json!({"spec":{"source":"duckdb","file":"data/ids.csv"}})).unwrap();
    let source = resolved(
        json!({"source":"http","depends_on":["ids"],"url":server.source().spec.provider["url"],"params":{"id":"{{ sources.ids.0.id }}"}}),
        json!({"ids":input}),
    );
    let cache = SourceCache::default();
    let cancel = Cancellation::default();
    ctx.files
        .insert("data/ids.csv".into(), "id\nfirst\n".into());
    let (_, first) = cache.load(source.clone(), &ctx, &cancel).unwrap();
    ctx.files
        .insert("data/ids.csv".into(), "id\nsecond\n".into());
    let (_, second) = cache.load(source, &ctx, &cancel).unwrap();
    assert_ne!(first, second);
    let paths: Vec<_> = server
        .requests()
        .iter()
        .map(|r| r["path"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(paths, ["/data?id=first", "/data?id=second"]);
}

#[test]
fn upstream_failures_invalid_json_empty_arrays_and_missing_fields_never_send_downstream() {
    for (body, status, path) in [
        ("private body", 503, "token"),
        ("not JSON", 200, "token"),
        ("[]", 200, "0.id"),
        ("{}", 200, "missing"),
        ("null", 200, "token"),
        ("{\"items\":[]}", 200, "items.1.id"),
    ] {
        let server = Http::new();
        server.respond(body.as_bytes(), status, 0.0, json!({}));
        let common = server.source();
        let source = resolved(
            json!({"source":"http","url":format!("{}/child",common.spec.provider["url"].as_str().unwrap()),"depends_on":["parent"],"headers":{"X-Value":format!("{{{{ sources.parent.{path} }}}}")}}),
            json!({"parent":common}),
        );
        let error = SourceCache::default()
            .load(source, &server.context(), &Cancellation::default())
            .err()
            .unwrap();
        assert_eq!(server.requests().len(), 1);
        assert!(!error.contains("private body"));
    }
}

#[test]
fn cancelling_an_upstream_request_stops_the_chain_and_allows_retry() {
    let server = Http::new();
    server.respond(br#"{"token":"value"}"#, 200, 5.0, json!({}));
    let common = server.source();
    let source = resolved(
        json!({"source":"http","url":format!("{}/child",common.spec.provider["url"].as_str().unwrap()),"depends_on":["parent"]}),
        json!({"parent":common}),
    );
    let cache = Arc::new(SourceCache::default());
    let cancel = Arc::new(Cancellation::default());
    let (worker_cache, worker_cancel, ctx, input) = (
        cache.clone(),
        cancel.clone(),
        server.context(),
        source.clone(),
    );
    let worker = std::thread::spawn(move || worker_cache.load(input, &ctx, &worker_cancel));
    server.wait_requests(1);
    cancel.cancel();
    assert!(worker.join().unwrap().is_err());
    assert_eq!(server.requests().len(), 1);
    server.respond(br#"[]"#, 200, 0.0, json!({}));
    cache
        .load(source, &server.context(), &Cancellation::default())
        .unwrap();
    assert_eq!(server.requests().len(), 3);
}

#[test]
fn resolved_graph_requires_exact_declared_dependencies() {
    for upstream in [
        json!({}),
        json!({"extra":{"spec":{"source":"http","url":"https://example.com"}}}),
    ] {
        let source: Resolved = serde_json::from_value(json!({"spec":{"source":"http","url":"https://example.com","depends_on":["parent"]},"upstream":upstream})).unwrap();
        assert!(source.normalized().is_err());
    }
}

#[test]
fn cancelling_one_dag_caller_preserves_the_other_callers_shared_upstream() {
    let server = Http::new();
    server.respond(br#"[]"#, 200, 0.3, json!({}));
    let common = server.source();
    let source = resolved(
        json!({"source":"http","url":format!("{}/child",common.spec.provider["url"].as_str().unwrap()),"depends_on":["parent"]}),
        json!({"parent":common}),
    );
    let cache = Arc::new(SourceCache::default());
    let cancel = Arc::new(Cancellation::default());
    let (first_cache, first_cancel, first_ctx, first_source) = (
        cache.clone(),
        cancel.clone(),
        server.context(),
        source.clone(),
    );
    let first =
        std::thread::spawn(move || first_cache.load(first_source, &first_ctx, &first_cancel));
    server.wait_requests(1);
    let (second_cache, ctx) = (cache.clone(), server.context());
    let second =
        std::thread::spawn(move || second_cache.load(source, &ctx, &Cancellation::default()));
    cache.wait_for_waiters(2);
    cancel.cancel();
    assert!(first.join().unwrap().is_err());
    assert!(second.join().unwrap().is_ok());
    assert_eq!(server.requests().len(), 2);
}

#[test]
fn source_templates_preserve_whole_values_and_reject_missing_paths() {
    let values = BTreeMap::from([(
        "input".into(),
        json!({"nil":null,"empty":[],"object":{"x":true},"numeric":{"0":"key"},"unicode":"你好"}),
    )]);
    for (path, expected) in [
        ("nil", Value::Null),
        ("empty", json!([])),
        ("object", json!({"x":true})),
        ("numeric.0", json!("key")),
        ("unicode", json!("你好")),
    ] {
        assert_eq!(
            template::render_with_sources(
                &format!("{{{{ sources.input.{path} }}}}"),
                &BTreeMap::new(),
                &BTreeMap::new(),
                &values
            )
            .unwrap(),
            expected
        );
    }
    assert_eq!(
        template::render_with_sources(
            "{{ sources.input }}",
            &BTreeMap::new(),
            &BTreeMap::new(),
            &values
        )
        .unwrap(),
        values["input"]
    );
    for invalid in [
        "{{ sources.input.empty.0 }}",
        "{{ sources.input.object.0 }}",
        "{{ sources.absent }}",
    ] {
        assert!(template::render_with_sources(
            invalid,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &values
        )
        .is_err());
    }
}

#[test]
fn sqlite_upstream_preserves_typed_rows_and_blocks_http_on_query_or_lookup_failure() {
    for sql in [
        "SELECT $id AS id",
        "SELECT id FROM missing",
        "SELECT $id AS id WHERE 0",
    ] {
        let server = Http::new();
        let ctx = server.context();
        let directory = std::path::Path::new(&ctx.directory).join("data");
        std::fs::create_dir_all(&directory).unwrap();
        drop(rusqlite::Connection::open(directory.join("input.sqlite")).unwrap());
        let input: Resolved = serde_json::from_value(json!({"spec":{"source":"sqlite","file_path":"input.sqlite","sql":sql},"params":{"id":7,"unused":true}})).unwrap();
        let source = resolved(
            json!({"source":"http","method":"post","url":server.source().spec.provider["url"],"depends_on":["seed"],"json":{"rows":"{{ sources.seed }}","id":"{{ sources.seed.0.id }}"}}),
            json!({"seed":input}),
        );
        let result = SourceCache::default().load(source, &ctx, &Cancellation::default());
        if sql == "SELECT $id AS id" {
            assert!(result.is_ok());
            use base64::Engine as _;
            let requests = server.requests();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(requests[0]["body"].as_str().unwrap())
                .unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&bytes).unwrap(),
                json!({"rows":[{"id":7}],"id":7})
            );
        } else {
            assert!(result.is_err());
            assert!(server.requests().is_empty());
        }
    }
}
