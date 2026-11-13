use facetra_contract::{self as contract, models::Dashboard};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
#[derive(Deserialize, Debug)]
struct Case {
    file: String,
    kind: String,
    structure: bool,
    valid: bool,
    documents: Vec<String>,
    code: Option<String>,
    path: Option<String>,
}
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}
#[test]
fn shared_corpus_agrees_on_schema_models_and_static_diagnostics() {
    let fixtures = root().join("Contracts/charting/v1/fixtures");
    let cases: Vec<Case> =
        serde_json::from_str(&fs::read_to_string(fixtures.join("cases.json")).unwrap()).unwrap();
    for case in cases {
        let yaml = fs::read_to_string(fixtures.join(&case.file)).unwrap();
        let parsed = contract::parse(&yaml);
        let schema: Value = serde_json::from_str(
            &fs::read_to_string(
                root().join(format!("Contracts/charting/v1/{}.schema.json", case.kind)),
            )
            .unwrap(),
        )
        .unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        assert_eq!(
            parsed.as_ref().is_ok_and(|v| validator.is_valid(v)),
            case.structure,
            "schema: {}",
            case.file
        );
        let decoded = contract::run(json!({"op":"decode","kind":case.kind,"yaml":yaml}));
        if case.structure {
            assert!(
                contract::run(json!({"op":"decode_model","kind":case.kind,"yaml":yaml})).is_ok(),
                "editor model: {}",
                case.file
            );
        }
        assert_eq!(
            decoded.is_ok(),
            case.structure,
            "decode: {}: {decoded:?}",
            case.file
        );
        let result = decoded.and_then(|value| {
            let documents = case
                .documents
                .iter()
                .map(|p| contract::parse(&fs::read_to_string(fixtures.join(p)).unwrap()).unwrap())
                .collect::<Vec<_>>();
            contract::run(
                json!({"op":"validate","kind":case.kind,"document":value,"documents":documents}),
            )
        });
        assert_eq!(
            result.is_ok(),
            case.valid,
            "workspace: {}: {result:?}",
            case.file
        );
        if let Some(code) = case.code {
            let diagnostic = result.unwrap_err();
            assert_eq!(diagnostic.code, code, "{}", case.file);
            if let Some(path) = case.path {
                assert_eq!(diagnostic.path, path, "{}", case.file);
            }
        }
    }
}
fn yaml_files(root: &Path, files: &mut Vec<PathBuf>) {
    for item in fs::read_dir(root).unwrap() {
        let path = item.unwrap().path();
        if path.is_dir() {
            yaml_files(&path, files);
        } else if path.extension().is_some_and(|e| e == "yaml") {
            files.push(path);
        }
    }
}
#[test]
fn documentation_and_optional_workspaces_conform() {
    let mut roots = vec![root().join("Docs/examples")];
    if let Some(workspace_root) = std::env::var_os("FACETRA_WORKSPACES_ROOT") {
        for item in fs::read_dir(workspace_root).unwrap() {
            let path = item.unwrap().path();
            if path.join("charts").is_dir() {
                roots.push(path);
            }
        }
    }
    let mut count = 0;
    for root in roots {
        let mut files = Vec::new();
        yaml_files(&root, &mut files);
        let mut charts = BTreeMap::new();
        let mut dashboards = Vec::new();
        for file in files {
            if file.file_name().unwrap() == "workspace.yaml" {
                continue;
            }
            let yaml = fs::read_to_string(&file).unwrap();
            let value =
                contract::parse(&yaml).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            if value.get("chart").is_some() {
                let chart =
                    contract::chart(&value).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
                charts.insert(chart.metadata.id.clone(), chart);
            } else if value["kind"] == "Dashboard" {
                contract::structure(&value, "dashboard")
                    .unwrap_or_else(|e| panic!("{}: {e}", file.display()));
                dashboards.push((file, contract::decode::<Dashboard>(&value).unwrap()));
            }
            count += 1;
        }
        for (id, chart) in &charts {
            contract::validate_chart(chart, &charts).unwrap_or_else(|e| panic!("{id}: {e}"));
        }
        for (file, d) in dashboards {
            contract::validate_dashboard(&d, &charts)
                .unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        }
    }
    assert!(count > 0);
}
#[test]
fn diagnostics_never_include_authored_values() {
    let value = json!({"op":"decode","kind":"chart","yaml":"metadata: {id: private-credential, title: secret-query-value}\nunknown: secret-token"});
    let result = contract::run_json(&value.to_string());
    for secret in ["private-credential", "secret-query-value", "secret-token"] {
        assert!(!result.contains(secret));
    }
}
#[test]
fn resolution_keeps_external_parameter_scope_and_normalizes_numbers() {
    let root = root().join("Contracts/charting/v1/fixtures/references");
    let docs = contract::documents(&[
        fs::read_to_string(root.join("provider.yaml")).unwrap(),
        fs::read_to_string(root.join("consumer.yaml")).unwrap(),
    ])
    .unwrap();
    let owner = &docs["consumer"];
    let result =
        contract::resolution::resolve(owner, &docs, &owner.parameters(&BTreeMap::new()).unwrap())
            .unwrap();
    assert_eq!(result.params, BTreeMap::from([("n".into(), json!(3))]));
    assert!(!result.params.contains_key("count"));
}
