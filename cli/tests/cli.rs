use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn run(arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_facetra-render"))
        .args(arguments)
        .output()
        .unwrap()
}

fn workspace() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("charts")).unwrap();
    fs::create_dir_all(root.path().join("data")).unwrap();
    fs::write(
        root.path().join("data/sales.csv"),
        "label,value\nA,12\nB,18\nC,9\n",
    )
    .unwrap();
    fs::write(
        root.path().join("charts/sample.yaml"),
        r##"metadata:
  id: sample
  title: Sample
sources:
  sales:
    source: duckdb
    file: data/sales.csv
dataset:
  id: data
  sources:
    - { id: raw, ref: "#/sources/sales" }
  sql: SELECT label, value FROM raw
chart:
  dataset: data
  renderer: echarts
  option:
    xAxis: { type: category, data: { from: data, field: label } }
    yAxis: { type: value }
    series:
      - { type: bar, data: { from: data, field: value } }
"##,
    )
    .unwrap();
    root
}

fn path(root: &Path) -> &str {
    root.to_str().unwrap()
}

#[test]
fn inspects_and_validates_a_workspace_without_an_app() {
    let root = workspace();
    let inspect = run(&["--json", "workspace", "inspect", path(root.path())]);
    assert!(
        inspect.status.success(),
        "{}",
        String::from_utf8_lossy(&inspect.stderr)
    );
    let value: Value = serde_json::from_slice(&inspect.stdout).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["result"]["resources"][0]["id"], "sample");

    let validate = run(&["--json", "workspace", "validate", path(root.path())]);
    assert!(
        validate.status.success(),
        "{}",
        String::from_utf8_lossy(&validate.stderr)
    );
    let value: Value = serde_json::from_slice(&validate.stdout).unwrap();
    assert_eq!(value["result"]["valid"], true);
}

#[test]
fn compiles_the_final_payload_without_a_mac_app() {
    let root = workspace();
    let output = run(&[
        "--json",
        "chart",
        "payload",
        "--workspace",
        path(root.path()),
        "--id",
        "sample",
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], true);
    assert_eq!(value["result"]["renderer"], "echarts");
    assert_eq!(
        value["result"]["payload"]["option"]["series"][0]["data"][1],
        "18"
    );
}

#[test]
fn refuses_to_overwrite_an_existing_png() {
    let root = workspace();
    let out = root.path().join("chart.png");
    fs::write(&out, b"existing").unwrap();
    let output = run(&[
        "--json",
        "chart",
        "render",
        "--workspace",
        path(root.path()),
        "--id",
        "sample",
        "--out",
        out.to_str().unwrap(),
    ]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(fs::read(&out).unwrap(), b"existing");
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["ok"], false);
}

#[test]
fn renders_a_png_when_explicitly_enabled() {
    if std::env::var("FACETRA_TEST_RENDER").as_deref() != Ok("1") {
        return;
    }
    let root = workspace();
    let out = root.path().join("chart.png");
    let renderer_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../renderers");
    let output = run(&[
        "--json",
        "chart",
        "render",
        "--workspace",
        path(root.path()),
        "--id",
        "sample",
        "--out",
        out.to_str().unwrap(),
        "--width",
        "640",
        "--height",
        "360",
        "--renderer-dir",
        renderer_dir.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = fs::read(&out).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert!(bytes.len() > 1_000);
}
