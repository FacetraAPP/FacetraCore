use super::*;

#[test]
fn source_revision_does_not_apply_csv_transport_limits_to_database_files() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("data")).unwrap();
    let path = root.path().join("data/large.sqlite");
    File::create(&path).unwrap().set_len(MAX_BYTES + 1).unwrap();
    let cancel = Cancellation::default();
    assert!(
        source_file_revision(root.path().to_str().unwrap(), "data/large.sqlite", &cancel).is_ok()
    );
    assert!(read_csv(root.path().to_str().unwrap(), "data/large.sqlite", &cancel).is_err());
}

fn request(value: Value) -> Request {
    serde_json::from_value(value).unwrap()
}
#[test]
fn snapshot_reuse_revision_and_external_access() {
    let session = Session::default();
    let base = json!({"version":1,"op":"query","sources":{"raw":{"csv":"code,value\n001,2\n"}},
            "sql":"SELECT code, CAST(value AS BIGINT) * $scale AS value FROM raw","params":{"scale":2}});
    let first = session
        .run(request(base.clone()), &Cancellation::default())
        .unwrap();
    assert_eq!(first["snapshot_reused"], false);
    assert_eq!(first["result_reused"], false);
    assert_eq!(first["frame"]["rows"][0], json!(["001", 4]));
    let mut next = base.clone();
    next["params"]["scale"] = json!(3);
    let second = session
        .run(request(next.clone()), &Cancellation::default())
        .unwrap();
    assert_eq!(second["snapshot_reused"], true);
    assert_eq!(second["result_reused"], false);
    assert_eq!(second["frame"]["rows"][0], json!(["001", 6]));
    assert_eq!(
        session
            .run(request(next.clone()), &Cancellation::default())
            .unwrap()["result_reused"],
        true
    );
    next["revision"] = json!("changed");
    let changed = session
        .run(request(next), &Cancellation::default())
        .unwrap();
    assert_eq!(changed["result_reused"], false);
    assert_eq!(changed["snapshot_reused"], false);
    let mut forbidden = base;
    forbidden["sql"] = json!("SELECT * FROM read_csv('/etc/passwd')");
    assert!(session
        .run(request(forbidden), &Cancellation::default())
        .is_err());
}
#[test]
fn frames_validate_and_preserve_nulls() {
    let session = Session::default();
    let mut input = json!({"version":1,"op":"query","sources":{"native":{"frame":{"columns":[{"name":"date","type":"date"},{"name":"value","type":"number"}],"rows":[["2026-01-01",null]]}}}});
    let output = session
        .run(request(input.clone()), &Cancellation::default())
        .unwrap();
    assert_eq!(output["frame"]["rows"][0], json!(["2026-01-01", null]));
    input["sources"]["native"]["frame"]["rows"] = json!([["2026-01-01"]]);
    assert!(session
        .run(request(input), &Cancellation::default())
        .is_err());
}
#[test]
fn failed_query_discards_snapshot_before_retry() {
    let session = Session::default();
    let base = json!({"version":1,"op":"query","sources":{"raw":{"csv":"value\n2\n"}},
            "sql":"SELECT CAST(value AS BIGINT) AS value FROM raw"});
    session
        .run(request(base.clone()), &Cancellation::default())
        .unwrap();
    let mut invalid = base.clone();
    invalid["sql"] = json!("SELECT missing FROM raw");
    assert!(session
        .run(request(invalid), &Cancellation::default())
        .is_err());
    let recovered = session
        .run(request(base), &Cancellation::default())
        .unwrap();
    assert_eq!(recovered["frame"]["rows"], json!([[2]]));
    assert_eq!(recovered["snapshot_reused"], false);
    assert_eq!(recovered["result_reused"], false);
}
#[test]
fn session_abi_reports_stable_error_codes_and_preserves_messages() {
    let session = Session::default();
    let cancel = Cancellation::default();
    let call = |input: &str| {
        let input = CString::new(input).unwrap();
        let output = unsafe { facetra_session_run(&session, input.as_ptr(), &cancel) };
        let output = unsafe { CString::from_raw(output) };
        serde_json::from_slice::<Value>(output.as_bytes()).unwrap()
    };
    let invalid = call("{");
    assert_eq!(invalid["error_code"], "invalid_request");
    assert!(invalid["error"]
        .as_str()
        .unwrap()
        .starts_with("Invalid request:"));
    let input = json!({"version":1,"op":"query","sources":{},"sql":"SELECT missing FROM absent"})
        .to_string();
    let execution = call(&input);
    assert_eq!(execution["error_code"], "execution");
    assert!(!execution["error"].as_str().unwrap().is_empty());
    cancel.cancel();
    let cancelled = call(&input);
    assert_eq!(cancelled["error_code"], "cancelled");
    assert_eq!(cancelled["error"], "Cancelled");
}

#[test]
fn cancelled_call_does_not_poison_next_call() {
    let session = Session::default();
    let cancel = Cancellation::default();
    cancel.cancel();
    let input = json!({"version":1,"op":"query","sources":{},"sql":"SELECT 1 AS value"});
    assert_eq!(
        session.run(request(input.clone()), &cancel).unwrap_err(),
        "Cancelled"
    );
    assert!(session
        .run(request(input), &Cancellation::default())
        .is_ok());
}
