use facetra_pipeline::*;
use serde_json::{json, Value};
use std::ffi::{CStr, CString};
struct Session(*mut std::ffi::c_void);
impl Session {
    fn new() -> Self {
        Self(facetra_session_create().cast())
    }
    fn call(&self, mut input: Value) -> Value {
        input["version"] = json!(1);
        let input = CString::new(input.to_string()).unwrap();
        unsafe {
            let output = facetra_session_run(self.0.cast(), input.as_ptr(), std::ptr::null());
            let result = serde_json::from_slice(CStr::from_ptr(output).to_bytes()).unwrap();
            facetra_pipeline_free(output);
            result
        }
    }
    fn ok(&self, input: Value) -> Value {
        let r = self.call(input);
        assert!(r.get("error").is_none(), "{r}");
        r["ok"].clone()
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            facetra_session_destroy(self.0.cast());
        }
    }
}
fn csv() -> Value {
    json!({"spec":{"source":"duckdb","file":"data/input.csv"}})
}
fn context() -> Value {
    json!({"workspace":"test","revision":"0","files":{"data/input.csv":"id,label\n001,first\n002,second\n"}})
}

#[test]
fn renderer_cache_returns_only_final_json_and_keeps_inserted_script_strings_as_data() {
    let session = Session::new();
    let request = json!({"op":"execute", "sources":{"raw":csv()}, "context":context(), "sql":null, "params":{},
        "render":{"chart":{"dataset":"records", "renderer":"echarts", "option":{
            "title":{"text":{"from":"params","name":"title"}},
            "tooltip":{"formatter":"js:(p) => p.name"},
            "series":[{"data":{"from":"records","field":"label"}}]}},
            "params":{"title":"js:this is ordinary data"}, "locale":"zh-Hans"}});
    let first = session.ok(request.clone());
    assert!(first.get("frame").is_none());
    let payload: Value = serde_json::from_str(first["payload_json"].as_str().unwrap()).unwrap();
    assert_eq!(payload["locale"], "ZH");
    assert_eq!(
        payload["option"]["title"]["text"],
        "js:this is ordinary data"
    );
    assert_eq!(
        payload["option"]["series"][0]["data"],
        json!(["first", "second"])
    );
    assert_eq!(payload["scripts"].as_array().unwrap().len(), 1);
    let hit = session.ok(request.clone());
    assert_eq!(hit["result_reused"], true);
    assert_eq!(hit["payload_json"], first["payload_json"]);
    let mut changed = request;
    changed["render"]["params"]["title"] = json!("New title");
    let updated = session.ok(changed);
    assert_eq!(updated["result_reused"], true);
    assert_ne!(updated["payload_json"], hit["payload_json"]);
}

#[test]
fn preview_pages_are_bounded_and_leases_are_independent_and_releasable() {
    let session = Session::new();
    let request =
        json!({"op":"source_page", "source":csv(), "context":context(), "offset":0, "limit":1});
    let first = session.ok(request.clone());
    let second = session.ok(request.clone());
    assert_eq!(first["frame"]["rows"], json!([["001", "first"]]));
    assert_eq!(first["total_rows"], 2);
    assert_eq!(first["next_offset"], 1);
    assert_ne!(first["revision"], second["revision"]);
    session.ok(json!({"op":"release_preview", "revision":first["revision"]}));
    let mut next = request.clone();
    next["offset"] = json!(1);
    next["revision"] = second["revision"].clone();
    let page = session.ok(next.clone());
    assert_eq!(page["frame"]["rows"], json!([["002", "second"]]));
    assert!(page["next_offset"].is_null());
    next["revision"] = first["revision"].clone();
    assert!(session.call(next).get("error").is_some());
    for limit in [0, 1025] {
        let mut invalid = request.clone();
        invalid["limit"] = json!(limit);
        assert!(session.call(invalid).get("error").is_some());
    }
}
#[test]
fn csv_sqlite_join_preserves_null_schema_parameter_types_and_cache_reuse() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("data")).unwrap();
    let db = rusqlite::Connection::open(dir.path().join("data/input.sqlite")).unwrap();
    db.execute_batch("CREATE TABLE input (id TEXT, amount REAL); INSERT INTO input VALUES ('001',12.5),('002',NULL)").unwrap();
    let session = Session::new();
    let mut ctx = context();
    ctx["directory"] = json!(dir.path());
    let mut request = json!({"op":"execute","sources":{"labels":csv(),"measurements":{"spec":{"source":"sqlite","file_path":"input.sqlite","sql":"SELECT id,amount FROM input WHERE $keep"},"params":{"keep":true}}},"context":ctx,
        "sql":"SELECT l.id,l.label,v.amount*$scale AS amount FROM labels l JOIN measurements v ON l.id=v.id ORDER BY l.id","params":{"scale":2},"number_params":["scale"]});
    let first = session.ok(request.clone());
    assert_eq!(
        first["frame"]["rows"],
        json!([["001", "first", 25.0], ["002", "second", null]])
    );
    assert_eq!(
        first["frame"]["columns"][2],
        json!({"name":"amount","type":"number"})
    );
    assert!(first.get("echarts").is_none());
    let hit = session.ok(request.clone());
    assert_eq!(hit["snapshot_reused"], true);
    assert_eq!(hit["result_reused"], true);
    request["params"]["scale"] = json!(4);
    let scaled = session.ok(request.clone());
    assert_eq!(scaled["frame"]["rows"][0][2], 50.0);
    assert_eq!(scaled["snapshot_reused"], true);
    assert_eq!(scaled["result_reused"], false);
    request["context"]["revision"] = json!("1");
    assert_eq!(session.ok(request)["snapshot_reused"], false);
}
#[test]
fn source_preview_decode_validation_compile_and_snapshot_round_trip() {
    use base64::Engine as _;
    let session = Session::new();
    let source = json!({"op":"source","source":csv(),"context":context(),"display":"text"});
    assert_eq!(
        session.ok(source)["text"],
        "id,label\n001,first\n002,second\n"
    );
    let raw =
        base64::engine::general_purpose::STANDARD.encode(br#"{"items":[{"x":true,"y":null}]}"#);
    let decoded = session
        .ok(json!({"op":"decode_source","bytes":raw,"format":"json","records_path":["items"]}));
    assert_eq!(decoded["rows"], json!([[true, null]]));
    assert_eq!(
        session.ok(json!({"op":"decode_source","frame":decoded,"format":"json","display":"df"}))
            ["kind"],
        "df"
    );
    assert_eq!(
        session.ok(json!({"op":"validate_source","source":csv()["spec"]})),
        json!({})
    );
    let compiled=session.ok(json!({"op":"compile_source_sql","sql":"SELECT $n, $n","params":{"n":2},"number_params":["n"],"question_marks":true}));
    assert_eq!(compiled["sql"], "SELECT ?, ?");
    assert_eq!(compiled["number_values"], json!([0, 1]));
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("data")).unwrap();
    std::fs::write(dir.path().join("data/input.csv"), "x\n1\n").unwrap();
    let bytes = session
        .ok(json!({"op":"read_source_bytes","directory":dir.path(),"file":"data/input.csv"}));
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(bytes["bytes"].as_str().unwrap())
            .unwrap(),
        b"x\n1\n"
    );
    let path = dir.path().join("input.sqlite");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE t (x INTEGER);INSERT INTO t VALUES (42)")
        .unwrap();
    let bytes = session.ok(json!({"op":"sqlite_snapshot","path":path}));
    std::fs::write(
        dir.path().join("copy.sqlite"),
        base64::engine::general_purpose::STANDARD
            .decode(bytes["bytes"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap();
    let copy = rusqlite::Connection::open(dir.path().join("copy.sqlite")).unwrap();
    assert_eq!(
        copy.query_row("SELECT x FROM t", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        42
    );
}
#[test]
fn invalid_source_requests_are_errors_and_never_poison_the_session() {
    let session = Session::new();
    for request in [
        json!({"op":"execute","sources":{},"context":context()}),
        json!({"op":"execute","sources":{"a":csv(),"b":csv()},"context":context()}),
        json!({"op":"execute","sources":{"a;drop":csv()},"context":context()}),
        json!({"op":"source","source":{"spec":{"source":"healthkit","metric":"step_count"}},"context":{}}),
        json!({"op":"decode_source","bytes":"???","format":"json"}),
        json!({"op":"decode_source","format":"json"}),
        json!({"op":"compile_source_sql","sql":"SELECT $n","params":{},"number_params":["n"]}),
        json!({"op":"compile_source_sql","sql":"SELECT $n","params":{"n":true},"number_params":["n"]}),
    ] {
        assert!(session.call(request)["error"].is_string());
    }
    assert_eq!(
        session.ok(json!({"op":"execute","sources":{"raw":csv()},"context":context()}))
            ["row_count"],
        2
    );
}
#[test]
fn typed_frame_transport_validates_schema_and_keeps_all_supported_types() {
    let session = Session::new();
    let columns = json!([{"name":"s","type":"string"},{"name":"i","type":"integer"},{"name":"n","type":"number"},{"name":"b","type":"boolean"},{"name":"d","type":"date"},{"name":"t","type":"timestamp"},{"name":"j","type":"json"}]);
    let frame = json!({"columns":columns,"rows":[["001",i64::MAX,1.5,true,"2026-10-12","2026-10-12 12:34:56","{\"x\":1}"],[null,null,null,null,null,null,null]]});
    assert_eq!(
        session.ok(json!({"op":"query","sources":{"raw":{"frame":frame}}}))["frame"],
        frame
    );
    for bad in [
        json!({"columns":[],"rows":[]}),
        json!({"columns":[{"name":"n","type":"number"}],"rows":[[true]]}),
        json!({"columns":[{"name":"n","type":"unknown"}],"rows":[]}),
        json!({"columns":[{"name":"x","type":"string"},{"name":"X","type":"string"}],"rows":[]}),
    ] {
        assert!(
            session.call(json!({"op":"query","sources":{"raw":{"frame":bad}}}))["error"]
                .is_string()
        );
    }
}
#[test]
fn cancelled_and_invalid_c_boundary_calls_have_owned_error_envelopes() {
    unsafe {
        let session = facetra_session_create();
        let cancel = facetra_cancel_create();
        facetra_cancel_request(cancel);
        let request = CString::new(
            json!({"version":1,"op":"execute","sources":{"raw":csv()},"context":context()})
                .to_string(),
        )
        .unwrap();
        let output = facetra_session_run(session, request.as_ptr(), cancel);
        let result: Value = serde_json::from_slice(CStr::from_ptr(output).to_bytes()).unwrap();
        assert_eq!(result["error_code"], "cancelled");
        facetra_pipeline_free(output);
        for (s, input) in [
            (std::ptr::null(), request.as_ptr()),
            (session.cast_const(), std::ptr::null()),
            (session.cast_const(), c"\xff".as_ptr()),
        ] {
            let output = facetra_session_run(s, input, std::ptr::null());
            let r: Value = serde_json::from_slice(CStr::from_ptr(output).to_bytes()).unwrap();
            assert_eq!(r["error_code"], "invalid_request");
            facetra_pipeline_free(output);
        }
        facetra_cancel_destroy(cancel);
        facetra_session_destroy(session);
        facetra_cancel_destroy(std::ptr::null_mut());
        facetra_session_destroy(std::ptr::null_mut());
        facetra_pipeline_free(std::ptr::null_mut());
    }
}
fn document(id: &str) -> Value {
    json!({"params":[{"name":"n","type":"number","default":2,"minimum":1,"maximum":9},{"name":"label","type":"string","default":"owner"},{"name":"flag","type":"boolean","default":true},{"name":"optional","type":"integer","required":false}],
      "sources":{"input":{"source":"duckdb","file":"data/input.csv"}},"dataset":{"id":"points","sources":[{"id":"raw","ref":"#/sources/input"}],"sql":"SELECT $n AS n FROM raw"},
      "metadata":{"id":id,"title":"Fixture"},"chart":{"dataset":"points","renderer":"value","value":"n"}})
}
#[test]
fn source_and_dataset_references_resolve_owned_scopes_display_overrides_and_numbers() {
    let session = Session::new();
    let owner = document("owner");
    let mut child = document("child");
    child["sources"] = json!({"input":{"ref":"owner.yaml#/sources/input","display":"text","with":{"n":{"param":"n"}}}});
    let r=session.ok(json!({"op":"resolve","documents":{"owner":owner,"child":child},"chart_id":"child","params":{"n":3,"label":"caller"},"number_params":["n"]}));
    assert_eq!(r["sources"]["raw"]["params"]["n"], 3.0);
    assert_eq!(r["sources"]["raw"]["params"]["label"], "owner");
    assert_eq!(r["sources"]["raw"]["spec"]["display"], "text");
    assert_eq!(r["number_params"], json!(["n"]));
    let mut child = document("child");
    child["dataset"] = json!({"id":"points","ref":"owner.yaml#/dataset","with":{"n":4}});
    let r=session.ok(json!({"op":"resolve","documents":{"owner":document("owner"),"child":child},"chart_id":"child"}));
    assert_eq!(r["params"]["n"], 4.0);
    assert_eq!(r["sources"]["raw"]["spec"]["file"], "data/input.csv");
}
#[test]
fn resolver_rejects_cycles_broken_refs_undefined_bindings_and_invalid_parameters() {
    let session = Session::new();
    for reference in [
        "#/sources/input",
        "owner.yaml#/sources/absent",
        "missing.yaml#/sources/input",
        "../owner.yaml#/sources/input",
        "owner.json#/sources/input",
        "owner.yaml#dataset",
        "owner.yaml#/dataset",
        "bad",
        "owner.yaml#/sources/../input",
    ] {
        let mut child = document("child");
        child["sources"]["input"] = json!({"ref":reference});
        assert!(session.call(json!({"op":"resolve","documents":{"owner":document("owner"),"child":child},"chart_id":"child"}))["error"].is_string(),"{reference}");
    }
    for binding in [
        json!({"param":"absent"}),
        json!({"param":"n","extra":true}),
        json!({"name":"n"}),
        Value::Null,
        json!([]),
    ] {
        let mut child = document("child");
        child["sources"]["input"] = json!({"ref":"owner.yaml#/sources/input","with":{"n":binding}});
        assert!(session.call(json!({"op":"resolve","documents":{"owner":document("owner"),"child":child},"chart_id":"child"}))["error"].is_string());
    }
    for params in [
        json!({"n":0}),
        json!({"n":10}),
        json!({"n":"2"}),
        json!({"label":" "}),
        json!({"flag":1}),
        json!({"optional":1.5}),
        json!({"missing":true}),
    ] {
        assert!(session.call(json!({"op":"resolve","documents":{"owner":document("owner")},"chart_id":"owner","params":params}))["error"].is_string());
    }
    let mut owner = document("owner");
    owner["dataset"] = json!({"id":"points","ref":"#/dataset"});
    assert!(
        session.call(json!({"op":"resolve","documents":{"owner":owner},"chart_id":"owner"}))
            ["error"]
            .as_str()
            .unwrap()
            .contains("Circular")
    );
}
#[test]
fn renderer_fixture_api_preserves_binding_modes_script_separation_and_colors() {
    let session = Session::new();
    let frame = json!({"columns":[{"name":"name","type":"string"},{"name":"a","type":"number"},{"name":"b","type":"number"}],"rows":[["js: never",2,1],["negative",-2,0],["null",null,null]]});
    let mut chart = json!({"dataset":"points","renderer":"echarts","scripts":{"formatter":"x => x.name"},"option":{
        "rows":{"from":"points","rows":true},"fields":{"from":"points","fields":["name","a"]},"xy":{"from":"points","x":"name","y":"a"},
        "sign":{"from":"points","field":"a","color_by_sign":true},"compare":{"from":"points","field":"a","color_by":["a","b"]},
        "param":{"from":"params","name":"label"},"formatter":{"from":"script","name":"formatter"},"inline":["js: x => x.value",false,3,null]}});
    let response = session
        .ok(json!({"op":"render_frame","frame":frame,"chart":chart,"params":{"label":"title"}}));
    let r: Value = serde_json::from_str(response["payload_json"].as_str().unwrap()).unwrap();
    assert_eq!(r["option"]["rows"][0]["name"], "js: never");
    assert_eq!(r["option"]["fields"][0], json!(["js: never", 2]));
    assert_eq!(r["option"]["sign"][1]["itemStyle"]["color"], "#ef5350");
    assert_eq!(r["option"]["sign"][2], json!({"value":null}));
    assert_eq!(r["scripts"].as_array().unwrap().len(), 2);
    assert!(session
        .ok(json!({"op":"validate_render","chart":chart,"parameters":["label"]}))
        .is_object());
    for binding in [
        json!({"from":"wrong","field":"a"}),
        json!({"from":"points","rows":false}),
        json!({"from":"points","field":"a","rows":true}),
        json!({"from":"points","fields":[]}),
        json!({"from":"points","fields":[1]}),
        json!({"from":"points","field":"absent"}),
        json!({"from":"points","field":"a","color_by_sign":2}),
        json!({"from":"points","rows":true,"color_by_sign":true}),
        json!({"from":"points","field":"name","color_by_sign":true}),
        json!({"from":"points","field":"a","color_by":["a"]}),
        json!({"from":"points","field":"a","extra":true}),
        json!({"from":"params","name":"missing"}),
        json!({"from":"script","name":"missing"}),
        json!("js: "),
    ] {
        chart["option"] = json!({"data":binding});
        assert!(
            session.call(json!({"op":"render_frame","frame":frame,"chart":chart}))["error"]
                .is_string()
        );
    }
}
#[test]
#[ignore = "Run Tools/test-source-databases.py to start isolated databases"]
fn isolated_real_databases_preserve_types_bindings_readonly_and_dataset_results() {
    let session = Session::new();
    for (connector, port, user) in [
        ("postgres", "FACETRA_TEST_PG_PORT", "facetra"),
        ("mysql", "FACETRA_TEST_MYSQL_PORT", "root"),
    ] {
        let context = json!({"workspace":connector,"connections":{"db":{"connector":connector,"fields":{"host":"127.0.0.1","port":std::env::var(port).expect("isolated test database port"),"login":user,"schema":"facetra_source_test","password":""},"extra":{"tls_mode":"disable","query_timeout":2}}}});
        let mut source = json!({"spec":{"source":"database","connection":"db","sql":"SELECT id,category,amount,note FROM orders WHERE category=$category ORDER BY id"},"params":{"category":"books"}});
        let frame = session.ok(json!({"op":"source","source":source,"context":context}));
        assert_eq!(
            frame["rows"],
            json!([
                [1, "books", "12.500000000", "001"],
                [3, "books", "30.250000000", "中文"]
            ]),
            "{connector}"
        );
        let total=session.ok(json!({"op":"execute","sources":{"orders":source},"context":context,"sql":"SELECT sum(cast(amount AS DOUBLE)) AS total,count(*) AS count FROM orders"}));
        assert_eq!(total["frame"]["rows"], json!([[42.75, 2]]), "{connector}");
        source["params"]["category"] = json!("games");
        assert_eq!(
            session.ok(json!({"op":"source","source":source,"context":context}))["rows"],
            json!([[2, "games", null, null]])
        );
        source["params"]["category"] = json!("books' OR 1=1 --");
        assert_eq!(
            session.ok(json!({"op":"source","source":source,"context":context}))["rows"],
            json!([])
        );
        let bound = json!({"spec":{"source":"database","connection":"db","sql":"SELECT $i AS i,$n AS n,$b AS b,$s AS s"},"params":{"i":9223372036854775807i64,"n":2.0,"b":true,"s":"'; DROP TABLE orders; --"},"number_params":["n"]});
        let values = session.ok(json!({"op":"source","source":bound,"context":context}));
        assert_eq!(values["rows"][0][0], i64::MAX);
        assert_eq!(values["rows"][0][1], 2.0);
        assert_eq!(values["rows"][0][3], "'; DROP TABLE orders; --");
        if connector == "postgres" {
            let types = json!({"spec":{"source":"database","connection":"db","sql":"SELECT '00112233-4455-6677-8899-aabbccddeeff'::uuid AS id, '2026-10-12 01:02:03.123456+08'::timestamptz AS utc, '2026-10-12 01:02:03.123456'::timestamp AS local, 12.50::money AS amount"}});
            assert_eq!(
                session.ok(json!({"op":"source","source":types,"context":context}))["rows"],
                json!([[
                    "00112233-4455-6677-8899-AABBCCDDEEFF",
                    "2026-10-11T17:02:03.123456Z",
                    "2026-10-12T01:02:03.123456Z",
                    "12.50"
                ]])
            );
        }
        let write = json!({"spec":{"source":"database","connection":"db","sql":"WITH rows AS (SELECT 1) DELETE FROM orders"}});
        assert!(
            session.call(json!({"op":"source","source":write,"context":context}))["error"]
                .is_string()
        );
        let count = json!({"spec":{"source":"database","connection":"db","sql":"SELECT count(*) AS n FROM orders"}});
        assert_eq!(
            session.ok(json!({"op":"source","source":count,"context":context}))["rows"][0][0],
            3
        );
        let sleepy = if connector == "postgres" {
            "SELECT pg_sleep(5)"
        } else {
            "SELECT SLEEP(5)"
        };
        let start = std::time::Instant::now();
        assert!(session.call(json!({"op":"source","source":{"spec":{"source":"database","connection":"db","sql":sleepy}},"context":context}))["error"].is_string());
        assert!(start.elapsed() < std::time::Duration::from_secs(4));
    }
}
