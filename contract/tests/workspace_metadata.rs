use facetra_contract as contract;
use serde_json::{json, Value};

fn validate(document: Value) -> Result<Value, contract::diagnostics::Diagnostic> {
    contract::run(json!({"op":"workspace_metadata", "document":document}))
}

#[test]
fn workspace_titles_and_descriptions_are_fixed_strings() {
    for document in [
        json!({"title":"Workspace"}),
        json!({"title":"固定名称 · Fixed name", "description":"Fixed notes"}),
        json!({"title":"Workspace", "description":""}),
        json!({"title":"Workspace", "description":null}),
    ] {
        validate(document).unwrap();
    }
    for document in [
        json!({}),
        json!({"title":" "}),
        json!({"title":{"i18n":"title"}}),
        json!({"title":"Workspace", "description":{"i18n":"notes"}}),
        json!({"title":"Workspace", "description":123}),
    ] {
        assert!(validate(document).is_err());
    }
}

#[test]
fn workspace_rejects_all_translation_configuration_without_compatibility() {
    for field in ["i18n", "localizations"] {
        for value in [Value::Null, json!({}), json!({"en":{"title":"English"}})] {
            let mut document = json!({"title":"Workspace"});
            document[field] = value;
            let error = validate(document).unwrap_err();
            assert_eq!(error.code, "structure.invalid");
            assert_eq!(error.path, format!("/{field}"));
        }
    }
}
