use super::{integration_tests::Http, *};
use base64::Engine as _;
use serde_json::json;

fn reply(body: Value) -> Value {
    raw(&body.to_string(), json!({}))
}
fn raw(body: &str, headers: Value) -> Value {
    json!({"body":base64::engine::general_purpose::STANDARD.encode(body),"headers":headers})
}
fn source(server: &Http, fields: Value) -> Resolved {
    let mut source = server.source();
    let mut spec = serde_json::to_value(&source.spec).unwrap();
    for (key, value) in fields.as_object().unwrap() {
        spec[key] = value.clone();
    }
    source.spec = serde_json::from_value(spec).unwrap();
    source
}
fn rows(source: Resolved, server: &Http) -> Result<Value> {
    acquire(source, server.context(), Arc::new(Cancellation::default()))?
        .template_value(&Cancellation::default())
}
fn base(server: &Http) -> String {
    server.source().spec.provider["url"]
        .as_str()
        .unwrap()
        .trim_end_matches("/data")
        .into()
}

#[test]
fn pagination_page_handles_short_and_empty_nonterminal_pages() {
    let server = Http::new();
    server.routes(json!({
        "/data?page=1":reply(json!({"items":[{"id":1}],"pages":3})),
        "/data?page=2":reply(json!({"items":[],"pages":3})),
        "/data?page=3":reply(json!({"items":[{"id":3}],"pages":3}))
    }));
    let src = source(
        &server,
        json!({"params":{"page":"{{ page.n }}"},"pagination":{
            "initial":{"n":1},"records":"response.json.items",
            "next":"if page.n < response.json.pages { #{n: page.n + 1} } else { () }","max_pages":3
        }}),
    );
    assert_eq!(rows(src, &server).unwrap(), json!([{"id":1},{"id":3}]));
    assert_eq!(server.requests().len(), 3);
}
#[test]
fn pagination_offset_advances_by_actual_count_until_explicit_empty_page() {
    let server = Http::new();
    server.routes(json!({
        "/data?offset=0&size=2":reply(json!([{"id":1},{"id":2}])),
        "/data?offset=2&size=2":reply(json!([{"id":3}])),
        "/data?offset=3&size=2":reply(json!([]))
    }));
    let src = source(
        &server,
        json!({"params":{"offset":"{{ page.offset }}","size":2},"pagination":{
            "initial":{"offset":0},"next":"if records.len > 0 { #{offset: page.offset + records.len} } else { () }"
        }}),
    );
    assert_eq!(rows(src, &server).unwrap().as_array().unwrap().len(), 3);
    assert_eq!(server.requests().len(), 3);
}
#[test]
fn pagination_cursor_is_opaque_and_query_encoding_omits_initial_null() {
    let server = Http::new();
    let cursor = "a&b+你好";
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("cursor", cursor)
        .finish();
    server.routes(json!({
        "/data":reply(json!({"items":[{"id":1}],"next":cursor})),
        format!("/data?{query}"):reply(json!({"items":[{"id":2}],"next":null}))
    }));
    let src = source(
        &server,
        json!({"params":{"cursor":"{{ page.cursor }}"},"pagination":{
            "initial":{"cursor":null},"records":"response.json.items",
            "next":"if response.json.next != () { #{cursor: response.json.next} } else { () }"
        }}),
    );
    assert_eq!(rows(src, &server).unwrap(), json!([{"id":1},{"id":2}]));
    assert_eq!(server.requests()[0]["path"], "/data");
}
#[test]
fn pagination_graphql_preserves_typed_body_state_and_header_injection() {
    let server = Http::new();
    server.routes(json!({"/data":{"responses":[
        reply(json!({"data":{"nodes":[{"id":1}],"pageInfo":{"hasNextPage":true,"endCursor":"opaque"}}})),
        reply(json!({"data":{"nodes":[{"id":2}],"pageInfo":{"hasNextPage":false,"endCursor":null}}}))
    ]}}));
    let src = source(
        &server,
        json!({"method":"post","headers":{"X-Part":"{{ page.part }}"},
            "json":{"variables":{"after":"{{ page.cursor }}","part":"{{ page.part }}"}},
            "pagination":{"initial":{"cursor":null,"part":1},"records":"response.json.data.nodes",
            "next":"if response.json.data.pageInfo.hasNextPage { #{cursor: response.json.data.pageInfo.endCursor, part: page.part + 1} } else { () }"}
        }),
    );
    assert_eq!(rows(src, &server).unwrap(), json!([{"id":1},{"id":2}]));
    let requests = server.requests();
    let bodies: Vec<Value> = requests
        .iter()
        .map(|r| {
            serde_json::from_slice(
                &base64::engine::general_purpose::STANDARD
                    .decode(r["body"].as_str().unwrap())
                    .unwrap(),
            )
            .unwrap()
        })
        .collect();
    assert_eq!(bodies[0], json!({"variables":{"after":null,"part":1}}));
    assert_eq!(bodies[1], json!({"variables":{"after":"opaque","part":2}}));
    assert_eq!(requests[1]["headers"]["x-part"], "2");
}
#[test]
fn pagination_link_headers_resolve_relative_urls_and_commas() {
    let server = Http::new();
    server.routes(json!({
        "/data":raw("[{\"id\":1}]",json!({"Link":"</data?cursor=a,b>; rel=\"next alternate\"; title=\"comma,title\", </data>; rel=\"prev\""})),
        "/data?cursor=a,b":reply(json!([{"id":2}]))
    }));
    let src = source(
        &server,
        json!({"url":"{{ page.url }}","pagination":{
            "initial":{"url":format!("{}/data",base(&server))},
            "next":"if response.links.contains(\"next\") { #{url: response.links.next} } else { () }"
        }}),
    );
    assert_eq!(rows(src, &server).unwrap(), json!([{"id":1},{"id":2}]));
}
#[test]
fn pagination_compound_time_id_state_works_in_form_body() {
    let server = Http::new();
    server.routes(json!({"/data":{"responses":[
        reply(json!({"items":[{"id":"z"}],"next":{"before":2,"last_id":"x"}})),
        reply(json!({"items":[{"id":"x"}],"next":null}))
    ]}}));
    let src = source(
        &server,
        json!({"method":"post","data":{"before":"{{ page.before }}","last_id":"{{ page.last_id }}"},
        "pagination":{"initial":{"before":3,"last_id":"z"},"records":"response.json.items","next":"response.json.next"}}),
    );
    assert_eq!(rows(src, &server).unwrap(), json!([{"id":"z"},{"id":"x"}]));
    let body = base64::engine::general_purpose::STANDARD
        .decode(server.requests()[1]["body"].as_str().unwrap())
        .unwrap();
    assert_eq!(String::from_utf8(body).unwrap(), "before=2&last_id=x");
}
#[test]
fn html_pagination_uses_document_context_and_row_selectors() {
    let server = Http::new();
    server.routes(json!({
        "/data":raw("<article><b>A &amp; B</b></article><a rel=next href='/two'>Next</a>",json!({})),
        "/two":raw("<article><b>C</b></article>",json!({}))
    }));
    let src = source(
        &server,
        json!({"format":"html","url":"{{ page.url }}",
            "extract":{"rows":"article","fields":{"name":{"selector":"b"}},
                "context":{"next":{"selector":"a[rel=next]","attribute":"href","required":false,"resolve_url":true}}},
            "pagination":{"initial":{"url":format!("{}/data",base(&server))},
                "next":"if response.extracted.next != () { #{url: response.extracted.next} } else { () }"}
        }),
    );
    assert_eq!(
        rows(src, &server).unwrap(),
        json!([{"name":"A & B"},{"name":"C"}])
    );
}
#[test]
fn xml_http_source_extracts_rss_and_atom_namespaces() {
    for (body,extract) in [
        ("<rss><channel><item><title>One</title></item><item><title>Two</title></item></channel></rss>",
            json!({"rows":"/rss/channel/item","fields":{"title":{"selector":"title"}}})),
        ("<feed xmlns='urn:atom'><entry><title>One</title></entry><entry><title>Two</title></entry></feed>",
            json!({"rows":"/a:feed/a:entry","namespaces":{"a":"urn:atom"},"fields":{"title":{"selector":"a:title"}}}))
    ] {
        let server=Http::new(); server.routes(json!({"/data":raw(body,json!({}))}));
        assert_eq!(rows(source(&server,json!({"format":"xml","extract":extract})),&server).unwrap(),json!([{"title":"One"},{"title":"Two"}]));
    }
}
fn fanout(server: &Http) -> Resolved {
    let parent = source(server, json!({"params":{"seed":1},"recordsPath":["items"]}));
    let mut child = source(
        server,
        json!({"depends_on":["list"],"foreach":{"source":"list","include_item":true},
        "params":{"id":"{{ item.id }}","page":"{{ page.n }}"},
        "pagination":{"initial":{"n":1},"next":"if page.n == 1 { #{n: 2} } else { () }"}}),
    );
    child.upstream.insert("list".into(), parent);
    child
}
fn fanout_routes() -> Value {
    json!({
        "/data?seed=1":reply(json!({"items":[{"id":10},{"id":20}],"metadata":"raw retained"})),
        "/data?id=10&page=1":reply(json!([{"value":1}])), "/data?id=10&page=2":reply(json!([{"value":2}])),
        "/data?id=20&page=1":reply(json!([{"value":3}])), "/data?id=20&page=2":reply(json!([{"value":4}]))
    })
}
#[test]
fn foreach_resets_pagination_per_item_preserves_parent_and_shares_cache() {
    let server = Http::new();
    server.routes(fanout_routes());
    let src = fanout(&server).normalized().unwrap();
    let cache = SourceCache::default();
    let cancel = Cancellation::default();
    let first = cache.load(src.clone(), &server.context(), &cancel).unwrap();
    assert_eq!(
        first.0.template_value(&cancel).unwrap(),
        json!([
            {"value":1,"_item":{"id":10}},{"value":2,"_item":{"id":10}},{"value":3,"_item":{"id":20}},{"value":4,"_item":{"id":20}}
        ])
    );
    assert_eq!(
        cache.load(src, &server.context(), &cancel).unwrap().1,
        first.1
    );
    assert_eq!(server.requests().len(), 5);
}
#[test]
fn foreach_and_pagination_share_request_budget_and_never_publish_partial_success() {
    let server = Http::new();
    server.routes(fanout_routes());
    let mut src = fanout(&server);
    src.spec
        .provider
        .insert("limits".into(), json!({"max_requests":3}));
    let cache = SourceCache::default();
    let cancel = Cancellation::default();
    assert!(cache
        .load(src.clone(), &server.context(), &cancel)
        .err()
        .unwrap()
        .contains("request limit"));
    assert_eq!(server.requests().len(), 4); // One upstream request + three child requests.
    src.spec.provider.remove("limits");
    assert_eq!(
        cache
            .load(src, &server.context(), &cancel)
            .unwrap()
            .0
            .template_value(&cancel)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        4
    );
}
#[test]
fn pagination_detects_repeated_states_requests_invalid_results_and_explicit_limits() {
    for (next, extra, error) in [
        ("page", json!({}), "repeated a state"),
        (
            "#{n: page.n + 1}",
            json!({"params":{}}),
            "repeated a request",
        ),
        ("false", json!({}), "object or ()"),
        ("response.json.absent", json!({}), "expression evaluation"),
        (
            "#{n: page.n + 1}",
            json!({"limits":{"max_requests":1}}),
            "request limit",
        ),
        (
            "#{n: page.n + 1}",
            json!({"limits":{"max_rows":1}}),
            "row limit",
        ),
        (
            "#{n: page.n + 1}",
            json!({"limits":{"max_bytes":5}}),
            "byte limit",
        ),
        (
            "#{n: page.n + 1}",
            json!({"pagination":{"initial":{"n":1},"next":"#{n: page.n + 1}","max_pages":1}}),
            "page limit",
        ),
    ] {
        let server = Http::new();
        server.routes(json!({"/data?page=1":reply(json!([{"id":1}])),"/data?page=2":reply(json!([{"id":2}])),"/data":reply(json!([{"id":1}]))}));
        let mut fields =
            json!({"params":{"page":"{{ page.n }}"},"pagination":{"initial":{"n":1},"next":next}});
        for (k, v) in extra.as_object().unwrap() {
            fields[k] = v.clone();
        }
        let err = rows(source(&server, fields), &server).err().unwrap();
        assert!(err.contains(error), "{next}: {err}");
    }
}
#[test]
fn pagination_next_url_cannot_change_origin_or_downgrade_https() {
    for url in ["https://example.invalid/steal", "http://127.0.0.1/steal"] {
        let server = Http::new();
        server.routes(json!({"/data":reply(json!({"items":[],"next":url}))}));
        let src = source(
            &server,
            json!({"url":"{{ page.url }}","pagination":{"initial":{"url":format!("{}/data",base(&server))},"records":"response.json.items","next":"#{url: response.json.next}"}}),
        );
        assert!(rows(src, &server).is_err());
        assert_eq!(server.requests().len(), 1);
    }
}
#[test]
fn retries_replay_same_page_without_duplicate_rows_and_count_failed_response_bytes() {
    let server = Http::new();
    server.routes(json!({
        "/data?page=1":{"responses":[json!({"status":429,"headers":{"Retry-After":"0"}}),reply(json!([{"id":1}]))]},
        "/data?page=2":reply(json!([{"id":2}]))
    }));
    let src = source(
        &server,
        json!({"params":{"page":"{{ page.n }}"},"retry":{"max_attempts":3,"delay_ms":1},
        "pagination":{"initial":{"n":1},"next":"if page.n < 2 { #{n: page.n + 1} } else { () }"}}),
    );
    assert_eq!(rows(src, &server).unwrap(), json!([{"id":1},{"id":2}]));
    assert_eq!(server.requests().len(), 3);
    let server = Http::new();
    server.routes(json!({"/data":{"responses":[
        json!({"status":503,"body":base64::engine::general_purpose::STANDARD.encode("123456")}),reply(json!([]))
    ]}}));
    let src = source(
        &server,
        json!({"retry":{"delay_ms":1},"limits":{"max_bytes":7}}),
    );
    assert!(rows(src, &server).err().unwrap().contains("byte limit"));
}
#[test]
fn cancellation_interrupts_retry_after_and_deadline_covers_whole_acquisition() {
    let server = Http::new();
    server.routes(json!({"/data":json!({"status":429,"headers":{"Retry-After":"30"}})}));
    let src = source(&server, json!({"retry":{"max_attempts":3}}));
    let cancel = Arc::new(Cancellation::default());
    let worker_cancel = cancel.clone();
    let ctx = server.context();
    let worker = std::thread::spawn(move || acquire(src, ctx, worker_cancel));
    server.wait_requests(1);
    let start = std::time::Instant::now();
    cancel.cancel();
    assert_eq!(worker.join().unwrap().err().unwrap(), "Cancelled");
    assert!(start.elapsed() < Duration::from_secs(1));
    let src = source(
        &server,
        json!({"retry":{"max_attempts":3},"limits":{"timeout_ms":50}}),
    );
    assert!(rows(src, &server).err().unwrap().contains("timed out"));
}
#[test]
fn empty_foreach_performs_no_child_requests_and_bad_extractors_fail_before_network() {
    let server = Http::new();
    server.routes(json!({"/data":reply(json!([]))}));
    let mut src = fanout(&server);
    src.upstream
        .get_mut("list")
        .unwrap()
        .spec
        .provider
        .remove("recordsPath");
    src.upstream
        .get_mut("list")
        .unwrap()
        .spec
        .provider
        .remove("params");
    assert_eq!(
        SourceCache::default()
            .load(src, &server.context(), &Cancellation::default())
            .unwrap()
            .0
            .template_value(&Cancellation::default())
            .unwrap(),
        json!([])
    );
    assert_eq!(server.requests().len(), 1);
    let src = source(
        &server,
        json!({"format":"html","extract":{"rows":"[","fields":{"x":{"selector":"b"}}}}),
    );
    assert!(rows(src, &server).is_err());
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn pagination_204_and_missing_or_malformed_json_are_distinct() {
    let server = Http::new();
    server.routes(json!({"/data": {"status":204}}));
    let src = source(
        &server,
        json!({"pagination":{"initial":{},"next":"if response.status == 204 { () } else { #{n: 2} }"}}),
    );
    assert_eq!(rows(src, &server).unwrap(), json!([]));
    let server = Http::new();
    server.routes(json!({"/data":raw("invalid private JSON",json!({}))}));
    let src = source(
        &server,
        json!({"pagination":{"initial":{},"records":"[]","next":"()"}}),
    );
    let error = rows(src, &server).err().unwrap();
    assert_eq!(error, "Invalid JSON response");
    assert!(!error.contains("private"));
}
#[test]
fn interrupted_response_retries_without_appending_and_ordinary_null_templates_are_stable() {
    let server = Http::new();
    let mut truncated = reply(json!([{"id":1}]));
    truncated["truncate"] = json!(3);
    server.routes(json!({"/data":{"responses":[truncated,reply(json!([{"id":1}]))]}}));
    let src = source(
        &server,
        json!({"retry":{"max_attempts":2,"delay_ms":0},"pagination":{"initial":{},"next":"()"}}),
    );
    assert_eq!(rows(src, &server).unwrap(), json!([{"id":1}]));
    assert_eq!(server.requests().len(), 2);
    let server = Http::new();
    server.routes(json!({"/data?token=null":reply(json!([]))}));
    let mut src = source(
        &server,
        json!({"params":{"token":"{{ sources.login.token }}"}}),
    );
    let result = acquire_with_inputs(
        src.clone(),
        server.context(),
        Arc::new(Cancellation::default()),
        BTreeMap::from([("login".into(), json!({"token":null}))]),
        None,
    )
    .unwrap();
    assert_eq!(
        result.template_value(&Cancellation::default()).unwrap(),
        json!([])
    );
    src.spec
        .provider
        .insert("limits".into(), json!({"max_rows":1}));
    server.routes(json!({"/data?token=null":reply(json!([{"id":1},{"id":2}]))}));
    assert!(acquire_with_inputs(
        src,
        server.context(),
        Arc::new(Cancellation::default()),
        BTreeMap::from([("login".into(), json!({"token":null}))]),
        None
    )
    .err()
    .unwrap()
    .contains("row limit"));
}
#[test]
fn foreach_json_records_retain_nested_types_and_literal_numeric_strings() {
    let source = data::SourceData::raw(
        br#"{"items":[{"mixed":{"a":1}},{"mixed":"123"},{"mixed":null}],"token":"raw"}"#.to_vec(),
        "json".into(),
        vec!["items".into()],
    )
    .unwrap();
    assert_eq!(
        source.records_value(&Cancellation::default()).unwrap(),
        json!([{"mixed":{"a":1}},{"mixed":"123"},{"mixed":null}])
    );
    assert_eq!(
        source.template_value(&Cancellation::default()).unwrap()["token"],
        "raw"
    );
}
