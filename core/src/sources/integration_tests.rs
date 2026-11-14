//! Real transports on loopback; assertions exercise the same acquisition and
//! shared-flight code as the C ABI, without Apple UI or external credentials.
use super::*;
use base64::Engine as _;
use serde_json::json;
use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    time::Instant,
};

struct Server {
    child: Child,
    port: u16,
}
impl Server {
    fn start(script: &str, args: &[&str]) -> Self {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../Tools")
            .join(script);
        let mut child = Command::new("python3")
            .arg(path)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        Self {
            child,
            port: line.trim().parse().expect("fixture port"),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
pub(super) struct Http {
    root: tempfile::TempDir,
    server: Server,
}
impl Http {
    pub(super) fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let status = Command::new("/usr/bin/openssl")
            .args([
                "req",
                "-x509",
                "-newkey",
                "rsa:2048",
                "-nodes",
                "-days",
                "1",
                "-subj",
                "/CN=localhost",
                "-addext",
                "basicConstraints=critical,CA:FALSE",
                "-addext",
                "subjectAltName=IP:127.0.0.1,DNS:localhost",
                "-addext",
                "extendedKeyUsage=serverAuth",
                "-keyout",
                "key.pem",
                "-out",
                "cert.pem",
            ])
            .current_dir(root.path())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::write(
            root.path().join("response.json"),
            r#"{"body":"W3sidmFsdWUiOjJ9XQ=="}"#,
        )
        .unwrap();
        let server = Server::start("test-http-server.py", &[root.path().to_str().unwrap()]);
        Self { root, server }
    }
    pub(super) fn context(&self) -> Context {
        Context {
            directory: self.root.path().to_str().unwrap().into(),
            http_roots: vec![std::fs::read_to_string(self.root.path().join("cert.pem")).unwrap()],
            ..Context::default()
        }
    }
    pub(super) fn source(&self) -> Resolved {
        serde_json::from_value(json!({"spec":{"source":"http","url":format!("https://127.0.0.1:{}/data", self.server.port)}})).unwrap()
    }
    pub(super) fn respond(&self, bytes: &[u8], status: u16, delay: f64, headers: Value) {
        let state = json!({"body":base64::engine::general_purpose::STANDARD.encode(bytes),"status":status,"delay":delay,"headers":headers});
        let path = self.root.path().join("next.json");
        std::fs::write(&path, state.to_string()).unwrap();
        std::fs::rename(path, self.root.path().join("response.json")).unwrap();
    }
    pub(super) fn requests(&self) -> Vec<Value> {
        std::fs::read_to_string(self.root.path().join("requests.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    pub(super) fn routes(&self, routes: Value) {
        std::fs::write(
            self.root.path().join("response.json"),
            json!({"routes":routes}).to_string(),
        )
        .unwrap();
    }
    pub(super) fn wait_requests(&self, count: usize) {
        let start = Instant::now();
        while self.requests().len() < count {
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "HTTP request did not start"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}
#[test]
fn https_requests_bind_credentials_query_form_json_and_preserve_raw_records() {
    let server = Http::new();
    let mut ctx = server.context();
    ctx.connections.insert(
        "api".into(),
        Connection {
            fields: BTreeMap::from([("token".into(), "secret-fixture".into())]),
            ..Connection::default()
        },
    );
    let mut source = server.source();
    source.params = BTreeMap::from([
        ("label".into(), json!("你好 +&=x")),
        ("count".into(), json!(3)),
    ]);
    source.spec.provider.extend(BTreeMap::from([
        ("method".into(), json!("post")),
        (
            "headers".into(),
            json!({"X-Token":"{{ connections.api.token }}", "FTSHARE_API_KEY":"{{ connections.api.token }}"}),
        ),
        ("params".into(), json!({"name":"{{ params.label }}"})),
        (
            "json".into(),
            json!({"n":"{{ params.count }}","text":"{{ params.label }}"}),
        ),
        ("format".into(), json!("json")),
        ("recordsPath".into(), json!(["records"])),
    ]));
    server.respond(
        br#"{"records":[{"n":3,"missing":null}],"meta":"retained"}"#,
        200,
        0.0,
        json!({}),
    );
    let data = acquire(
        source.clone().normalized().unwrap(),
        ctx.clone(),
        Arc::default(),
    )
    .unwrap();
    assert_eq!(
        data.dataframe(&Cancellation::default()).unwrap().rows,
        vec![vec![Value::Null, json!(3)]]
    );
    assert!(
        data.preview("json", &Cancellation::default()).unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("retained")
    );
    let requests = server.requests();
    let request = &requests[0];
    assert_eq!(request["method"], "POST");
    assert_eq!(request["headers"]["x-token"], "secret-fixture");
    assert_eq!(request["headers"]["ftshare_api_key"], "secret-fixture");
    assert!(request["path"].as_str().unwrap().contains("%2B%26%3Dx"));
    let body = base64::engine::general_purpose::STANDARD
        .decode(request["body"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&body).unwrap(),
        json!({"n":3,"text":"你好 +&=x"})
    );
    source.spec.provider.remove("json");
    source
        .spec
        .provider
        .insert("data".into(), json!({"label":"{{ params.label }}"}));
    acquire(source.normalized().unwrap(), ctx, Arc::default()).unwrap();
    let body = base64::engine::general_purpose::STANDARD
        .decode(server.requests()[1]["body"].as_str().unwrap())
        .unwrap();
    assert_eq!(
        url::form_urlencoded::parse(&body).next().unwrap().1,
        "你好 +&=x"
    );
}
#[test]
fn https_accepts_host_pem_bundles_with_crlf_line_endings() {
    let server = Http::new();
    let mut ctx = server.context();
    let pem = &ctx.http_roots[0];
    ctx.http_roots = vec![format!("{pem}\n{pem}").replace('\n', "\r\n")];
    let data = acquire(server.source(), ctx, Arc::default()).unwrap();
    assert_eq!(
        data.dataframe(&Cancellation::default()).unwrap().rows,
        vec![vec![json!(2)]]
    );
    assert_eq!(server.requests().len(), 1);
}
#[test]
fn https_rejects_untrusted_tls_redirects_bad_headers_and_redacts_failures() {
    let server = Http::new();
    assert!(acquire(server.source(), Context::default(), Arc::default()).is_err());
    for location in [
        "http://127.0.0.1/insecure",
        "https://example.invalid/other",
        "/loop",
    ] {
        server.respond(b"", 302, 0.0, json!({"Location":location}));
        assert!(
            acquire(server.source(), server.context(), Arc::default()).is_err(),
            "{location}"
        );
    }
    server.respond(b"secret-fixture", 403, 0.0, json!({}));
    let error = acquire(server.source(), server.context(), Arc::default())
        .err()
        .unwrap();
    assert!(!error.contains("secret-fixture") && !error.contains("127.0.0.1"));
    let mut source = server.source();
    source
        .spec
        .provider
        .insert("headers".into(), json!({"Authorization":"bad\r\nvalue"}));
    assert!(acquire(source, server.context(), Arc::default()).is_err());
    for name in ["bad header", "bad:header", "bad\r\nheader", ""] {
        let mut source = server.source();
        source
            .spec
            .provider
            .insert("headers".into(), json!({name: "secret-fixture"}));
        let error = acquire(source, server.context(), Arc::default())
            .err()
            .unwrap();
        assert_eq!(error, "Invalid HTTP header");
    }
    let requests = server.requests().len();
    for pem in [
        "",
        "invalid PEM",
        "-----BEGIN CERTIFICATE-----\ninvalid\n-----END CERTIFICATE-----",
    ] {
        let mut ctx = server.context();
        ctx.http_roots = vec![pem.into()];
        assert_eq!(
            acquire(server.source(), ctx, Arc::default()).err().unwrap(),
            "Invalid host certificate"
        );
    }
    assert_eq!(server.requests().len(), requests);
}
#[test]
fn cache_shares_fetch_but_not_cancellation_and_retries_failures() {
    let server = Http::new();
    let cache = Arc::new(SourceCache::default());
    server.respond(br#"[{"value":2}]"#, 200, 0.2, json!({}));
    let first_cancel = Arc::new(Cancellation::default());
    let spawn = |cancel: Arc<Cancellation>| {
        let cache = cache.clone();
        let source = server.source();
        let ctx = server.context();
        std::thread::spawn(move || cache.load(source, &ctx, &cancel))
    };
    let first = spawn(first_cancel.clone());
    server.wait_requests(1);
    let second = spawn(Arc::default());
    // Wait for waiter registration, not an arbitrary delay.
    cache.wait_for_waiters(2);
    first_cancel.cancel();
    assert_eq!(first.join().unwrap().err().unwrap(), "Cancelled");
    let (data, revision) = second.join().unwrap().unwrap();
    assert_eq!(
        data.dataframe(&Cancellation::default()).unwrap().rows,
        vec![vec![json!(2)]]
    );
    let (again, same_revision) = cache
        .load(server.source(), &server.context(), &Cancellation::default())
        .unwrap();
    assert!(Arc::ptr_eq(&data, &again));
    assert_eq!(revision, same_revision);
    assert_eq!(server.requests().len(), 1);
    // Revision/credential changes cannot hit a previous acquisition.
    server.respond(b"failed", 500, 0.0, json!({}));
    let mut ctx = server.context();
    ctx.revision = "new credentials".into();
    assert!(cache
        .load(server.source(), &ctx, &Cancellation::default())
        .is_err());
    server.respond(br#"[{"value":9}]"#, 200, 0.0, json!({}));
    let (data, _) = cache
        .load(server.source(), &ctx, &Cancellation::default())
        .unwrap();
    assert_eq!(
        data.dataframe(&Cancellation::default()).unwrap().rows[0][0],
        9
    );
}
#[test]
fn final_cancel_aborts_producer_and_next_call_can_retry() {
    let server = Http::new();
    server.respond(b"[]", 200, 10.0, json!({}));
    let cache = Arc::new(SourceCache::default());
    let cancel = Arc::new(Cancellation::default());
    let (worker_cache, worker_cancel, source, ctx) = (
        cache.clone(),
        cancel.clone(),
        server.source(),
        server.context(),
    );
    let worker = std::thread::spawn(move || worker_cache.load(source, &ctx, &worker_cancel));
    server.wait_requests(1);
    let start = Instant::now();
    cancel.cancel();
    assert_eq!(worker.join().unwrap().err().unwrap(), "Cancelled");
    assert!(start.elapsed() < Duration::from_secs(2));
    server.respond(br#"[{"value":7}]"#, 200, 0.0, json!({}));
    assert!(cache
        .load(server.source(), &server.context(), &Cancellation::default())
        .is_ok());
    assert_eq!(server.requests().len(), 2);
}
fn database_source(server: &Server, connector: &str) -> (Resolved, Context) {
    let source = serde_json::from_value(json!({"spec":{"source":"database","connection":"db","sql":"SELECT $label AS label, 42 AS amount"},"params":{"label":"'; DROP TABLE orders; -- 中文"}})).unwrap();
    let connection = Connection {
        connector: connector.into(),
        fields: BTreeMap::from([
            ("host".into(), "127.0.0.1".into()),
            ("port".into(), server.port.to_string()),
            ("login".into(), "reader".into()),
            ("schema".into(), "fixture".into()),
            ("password".into(), "fixture".into()),
        ]),
        extra: BTreeMap::from([
            ("tls_mode".into(), json!("disable")),
            ("query_timeout".into(), json!(1)),
        ]),
    };
    (
        source,
        Context {
            connections: BTreeMap::from([("db".into(), connection)]),
            ..Context::default()
        },
    )
}
#[test]
fn real_database_drivers_bind_injection_strings_and_decode_binary_rows() {
    for (connector, mode) in [("postgres", ""), ("mysql", ""), ("mysql", "legacy")] {
        let server = Server::start("test-database-server.py", &[connector, mode]);
        let (source, ctx) = database_source(&server, connector);
        let label = source.params["label"].clone();
        let frame = acquire(source.normalized().unwrap(), ctx, Arc::default()).unwrap();
        let frame = frame.dataframe(&Cancellation::default()).unwrap();
        assert_eq!(frame.rows, vec![vec![label, json!(42)]]);
        assert_eq!(frame.columns[1].kind, "integer");
    }
}
#[test]
fn database_deadlines_and_cancellation_close_inflight_queries() {
    for connector in ["postgres", "mysql"] {
        for cancel_query in [false, true] {
            let server = Server::start("test-database-server.py", &[connector, "stall"]);
            let (source, ctx) = database_source(&server, connector);
            let cancel = Arc::new(Cancellation::default());
            let worker_cancel = cancel.clone();
            let start = Instant::now();
            let worker = std::thread::spawn(move || acquire(source, ctx, worker_cancel));
            if cancel_query {
                std::thread::sleep(Duration::from_millis(100));
                cancel.cancel();
            }
            let error = worker.join().unwrap().err().unwrap();
            if cancel_query {
                assert_eq!(error, "Cancelled");
            }
            assert!(start.elapsed() < Duration::from_secs(3));
        }
    }
}
#[test]
fn postgres_wire_types_preserve_precision_dates_json_binary_and_null() {
    let server = Server::start("test-database-server.py", &["postgres", "types"]);
    let (source, ctx) = database_source(&server, "postgres");
    let data = acquire(source, ctx, Arc::default()).unwrap();
    let frame = data.dataframe(&Cancellation::default()).unwrap();
    assert_eq!(
        frame.rows[0],
        vec![
            json!(true),
            json!(-12),
            json!(123456),
            json!(i64::MAX),
            json!(1.5),
            json!(2.5),
            json!("12.50"),
            json!("AP8="),
            json!("2000-01-01"),
            json!("01:01:01.000002"),
            json!("2000-01-01T00:00:01.000002Z"),
            json!("2000-01-01T00:00:01.000002Z"),
            json!("{\"x\":1}"),
            json!("{\"x\":2}"),
            json!("00112233-4455-6677-8899-AABBCCDDEEFF"),
            json!("-1.05"),
            Value::Null
        ]
    );
}
#[test]
fn database_tls_never_downgrades_to_a_plaintext_fixture() {
    for connector in ["postgres", "mysql"] {
        for mode in ["require", "verify-full"] {
            let server = Server::start("test-database-server.py", &[connector]);
            let (source, mut ctx) = database_source(&server, connector);
            let c = ctx.connections.get_mut("db").unwrap();
            c.extra.insert("tls_mode".into(), json!(mode));
            c.extra.insert("server_name".into(), json!("localhost"));
            c.extra.insert("connect_timeout".into(), json!(1));
            assert!(acquire(source, ctx, Arc::default()).is_err());
        }
    }
}
#[test]
fn postgres_retains_declared_types_for_empty_and_all_null_results() {
    for mode in ["empty", "null"] {
        let server = Server::start("test-database-server.py", &["postgres", mode]);
        let (source, ctx) = database_source(&server, "postgres");
        let data = acquire(source, ctx, Arc::default()).unwrap();
        let frame = data.dataframe(&Cancellation::default()).unwrap();
        assert_eq!(frame.columns[1].kind, "integer");
        assert!(frame.rows.iter().flatten().all(Value::is_null));
    }
}
