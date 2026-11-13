use facetra_pipeline::run_json;
use serde_json::{json, Value};
use std::fs;

fn recipe(sql: &str) -> String {
    format!("sources:\n  input: {{source: duckdb, file: data/input.csv}}\ndataset:\n  id: points\n  sources: [{{id: raw, ref: '#/sources/input'}}]\n  sql: {sql:?}\nmetadata:\n  id: fixture\n  title: Fixture\nchart:\n  dataset: points\n  renderer: echarts\n  option: {{series: [{{data: {{from: points, rows: true}}}}]}}\n")
}
fn call(root: &std::path::Path, yaml: &str) -> Value {
    serde_json::from_str(&run_json(
        &json!({"documents":[yaml],"chart_id":"fixture","directory":root,"params":{}}).to_string(),
    ))
    .unwrap()
}
#[test]
fn controlled_import_locks_down_user_sql_and_preserves_data() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("data")).unwrap();
    fs::write(
        root.path().join("data/input.csv"),
        "code,label,value\r\n001,\"a, b\",2\r\n002,\"a\nline\",\r\n003,js: alert(1),3\r\n",
    )
    .unwrap();
    let output = call(root.path(), &recipe("SELECT * FROM raw"));
    assert_eq!(
        output["ok"]["frame"]["rows"][0],
        json!(["001", "a, b", "2"])
    );
    assert_eq!(
        output["ok"]["frame"]["rows"][1],
        json!(["002", "a\nline", null])
    );
    assert_eq!(output["ok"]["render"]["scripts"], json!([]));
    let settings = call(root.path(), &recipe("SELECT current_setting('enable_external_access') AS external, current_setting('autoload_known_extensions') AS autoload, current_setting('autoinstall_known_extensions') AS install, current_setting('threads') AS threads FROM raw LIMIT 1"));
    assert_eq!(
        settings["ok"]["frame"]["rows"][0],
        json!([false, false, false, 2])
    );
    for sql in [
        "SELECT * FROM read_csv('/etc/passwd')",
        "SELECT * FROM read_csv('https://example.com/a.csv')",
        "SELECT * FROM read_blob('/etc/passwd')",
        "SELECT * FROM range(100001)",
        "SELECT 1; SELECT 2",
        "INSTALL httpfs",
        "SET enable_external_access=true",
    ] {
        assert!(
            call(root.path(), &recipe(sql))["error"].is_string(),
            "{sql}"
        );
    }
    let payload = serde_json::from_str::<Value>(&run_json(&json!({"documents":[recipe("SELECT * FROM raw")],"chart_id":"fixture","directory":root.path(),"include_frame":false}).to_string())).unwrap();
    assert!(payload["ok"].get("frame").is_none());
    assert!(payload["ok"]["render"].is_object());
}
#[test]
fn file_and_row_limits_and_symlink_escape_are_enforced() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("data")).unwrap();
    let path = root.path().join("data/input.csv");
    let file = fs::File::create(&path).unwrap();
    file.set_len(100 * 1024 * 1024 + 1).unwrap();
    assert!(call(root.path(), &recipe("SELECT * FROM raw"))["error"].is_string());
    drop(file);
    fs::write(&path, format!("a,b\n{}", "x,y\n".repeat(100_001))).unwrap();
    assert!(call(root.path(), &recipe("SELECT COUNT(*) AS n FROM raw"))["error"].is_string());
    #[cfg(unix)]
    {
        fs::remove_file(&path).unwrap();
        fs::write(root.path().join("outside.csv"), "a,b\nx,y\n").unwrap();
        std::os::unix::fs::symlink(root.path().join("outside.csv"), &path).unwrap();
        assert!(call(root.path(), &recipe("SELECT * FROM raw"))["error"].is_string());
        fs::remove_file(&path).unwrap();
        fs::remove_dir(root.path().join("data")).unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("input.csv"), "a,b\nx,y\n").unwrap();
        std::os::unix::fs::symlink(outside.path(), root.path().join("data")).unwrap();
        assert!(call(root.path(), &recipe("SELECT * FROM raw"))["error"].is_string());
    }
}
#[test]
fn malformed_requests_and_renderer_fields_return_error_envelopes() {
    for request in [
        "not json".to_owned(),
        "{}".to_owned(),
        " ".repeat(8 * 1024 * 1024 + 1),
    ] {
        assert!(serde_json::from_str::<Value>(&run_json(&request)).unwrap()["error"].is_string());
    }
    let root = tempfile::tempdir().unwrap();
    let yaml = recipe("SELECT 1").replace("renderer: echarts", "renderer: echarts\n  x: null");
    assert!(call(root.path(), &yaml)["error"].is_string());
}

#[test]
fn vela_recipes_preserve_sql_frames_and_reject_other_renderer_fields() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("data")).unwrap();
    fs::write(
        root.path().join("data/input.csv"),
        "time,open,high,low,close\n2026-01-01,10,12,9,11\n",
    )
    .unwrap();
    let base = recipe("SELECT time, CAST(open AS DOUBLE) AS open, CAST(high AS DOUBLE) AS high, CAST(low AS DOUBLE) AS low, CAST(close AS DOUBLE) AS close FROM raw");
    let start = base.find("  renderer: echarts").unwrap();
    let yaml = format!("{}  renderer: vela\n  option: {{time: time, open: open, high: high, low: low, close: close}}\n", &base[..start]);
    let output = call(root.path(), &yaml);
    assert_eq!(
        output["ok"]["frame"]["rows"],
        json!([["2026-01-01", 10.0, 12.0, 9.0, 11.0]])
    );
    assert!(output["ok"]["render"]["bars"].is_array());
    for extra in ["  scripts: {}\n", "  plot: line\n", "  x: null\n"] {
        assert!(call(root.path(), &(yaml.clone() + extra))["error"].is_string());
    }
    assert!(call(
        root.path(),
        &yaml.replace(
            "option: {time: time, open: open, high: high, low: low, close: close}",
            "option: []"
        )
    )["error"]
        .is_string());
}

#[test]
fn legacy_swift_chart_fields_are_rejected_without_compatibility_fallbacks() {
    let root = tempfile::tempdir().unwrap();
    let base = recipe("SELECT * FROM raw");
    let start = base.find("  renderer: echarts").unwrap();
    let prefix = &base[..start];
    for extra in [
        "  renderer: default\n  plot: line\n  x: x\n  y: y\n",
        "  renderer: echarts\n  plot: line\n  x: x\n  y: y\n",
        "  renderer: echarts\n  average: null\n",
        "  renderer: echarts\n  color: null\n",
        "  renderer: echarts\n  x_end: null\n",
    ] {
        let yaml = format!("{prefix}{extra}");
        assert!(call(root.path(), &yaml)["error"].is_string());
    }
}
