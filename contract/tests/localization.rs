use facetra_contract::{self as contract, models::MessageCatalog};
use serde_json::{json, Value};
fn document() -> Value {
    contract::parse(include_str!(
        "../../Contracts/charting/v1/fixtures/valid/localized.yaml"
    ))
    .unwrap()
}
fn validate(value: Value) -> Result<Value, contract::diagnostics::Diagnostic> {
    contract::run(json!({"op":"validate","kind":"chart","document":value,"documents":[]}))
}
#[test]
fn references_are_scoped_to_display_fields_and_owning_resource() {
    let mut value = document();
    value["chart"]["option"]["tooltip"] = json!({"formatter":{"i18n":"nonexistent"}});
    let error = validate(value).unwrap_err();
    assert_eq!(error.code, "i18n.missing");
    assert_eq!(error.path, "/chart/option/tooltip/formatter");
    let mut value = document();
    value["sources"]["remote"] = json!({"source":"http","method":"post","url":"https://example.com","json":{"i18n":"raw request data"}});
    value["chart"]["scripts"] = json!({"tooltip":"() => 'i18n: nonexistent'"});
    validate(value).unwrap();
    let mut value = document();
    value.as_object_mut().unwrap().remove("i18n");
    assert_eq!(validate(value).unwrap_err().code, "i18n.missing");
}
#[test]
fn catalog_requires_unambiguous_locales_and_complete_default_translations() {
    for messages in [
        json!({"chart_title":{"en":"English"}}),
        json!({"chart_title":{"zh-Hans":"中文","ZH-hans":"重复"}}),
    ] {
        let mut value = document();
        value["i18n"]["messages"] = messages;
        assert_eq!(validate(value).unwrap_err().code, "i18n.invalid");
    }
    for reference in [
        json!({"i18n":"chart_title","extra":true}),
        json!({"i18n":12}),
        json!({"i18n":""}),
    ] {
        let mut value = document();
        value["metadata"]["title"] = reference;
        assert!(validate(value).is_err());
    }
    for tag in ["zh_Hans", "", "zh--Hans"] {
        let mut value = document();
        value["i18n"]["defaultLocale"] = json!(tag);
        assert!(validate(value).is_err());
    }
}
#[test]
fn fallback_uses_requested_parents_then_declared_default_and_retains_references() {
    let value = document();
    let catalog: MessageCatalog = serde_json::from_value(value["i18n"].clone()).unwrap();
    assert_eq!(
        catalog.text("chart_title", "en-US"),
        Some("Monthly revenue")
    );
    assert_eq!(catalog.text("chart_title", "ZH-hans-CN"), Some("月度收入"));
    assert_eq!(catalog.text("chart_title", "fr"), Some("月度收入"));
    let doc = contract::chart(&value).unwrap();
    let encoded = serde_json::to_value(doc).unwrap();
    assert_eq!(encoded["metadata"]["title"], json!({"i18n":"chart_title"}));
    assert_eq!(encoded["i18n"], value["i18n"]);
}
