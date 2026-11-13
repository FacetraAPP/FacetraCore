//! Resource-local message validation. Never interpret SQL, source values or scripts.
use crate::diagnostics::Diagnostic;
use serde_json::Value;
use std::collections::BTreeSet;

type Check = std::result::Result<(), Diagnostic>;
fn pointer(path: &str, key: &str) -> String {
    format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"))
}
fn locale(tag: &str) -> bool {
    let mut parts = tag.split('-');
    parts
        .next()
        .is_some_and(|s| (2..=8).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_alphabetic()))
        && parts.all(|s| (1..=8).contains(&s.len()) && s.bytes().all(|c| c.is_ascii_alphanumeric()))
}
pub fn validate(document: &Value) -> Check {
    let catalog = document.get("i18n").filter(|v| !v.is_null());
    if let Some(catalog) = catalog {
        let default = catalog["defaultLocale"].as_str().unwrap_or("");
        if !locale(default) {
            return Err(Diagnostic::new("i18n.invalid", "/i18n/defaultLocale"));
        }
        for (key, translations) in catalog["messages"].as_object().into_iter().flatten() {
            let path = pointer("/i18n/messages", key);
            if key.is_empty()
                || key.len() > 120
                || !key
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_.-".contains(&c))
            {
                return Err(Diagnostic::new("i18n.invalid", path));
            }
            let mut languages = BTreeSet::new();
            for (language, text) in translations.as_object().into_iter().flatten() {
                if !locale(language)
                    || !languages.insert(language.to_ascii_lowercase())
                    || text.as_str().is_none_or(|s| s.trim().is_empty())
                {
                    return Err(Diagnostic::new("i18n.invalid", pointer(&path, language)));
                }
            }
            if !languages.contains(&default.to_ascii_lowercase()) {
                return Err(Diagnostic::new("i18n.invalid", path));
            }
        }
    }
    fn text(value: &Value, path: &str, catalog: Option<&Value>) -> Check {
        if let Some(object) = value.as_object() {
            let key = object
                .get("i18n")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty());
            if object.len() != 1 || key.is_none() {
                return Err(Diagnostic::new("i18n.invalid", path));
            }
            if catalog.is_none_or(|c| c["messages"].get(key.unwrap()).is_none()) {
                return Err(Diagnostic::new("i18n.missing", path));
            }
        }
        Ok(())
    }
    fn option(value: &Value, path: &str, catalog: Option<&Value>, depth: usize) -> Check {
        if depth >= 64 {
            return Err(Diagnostic::new("i18n.invalid", path));
        }
        match value {
            Value::Object(object) if object.contains_key("i18n") => text(value, path, catalog),
            // Binding payloads are data, and are never recursively interpreted.
            Value::Object(object) if object.contains_key("from") => Ok(()),
            Value::Object(object) => {
                for (key, value) in object {
                    option(value, &pointer(path, key), catalog, depth + 1)?;
                }
                Ok(())
            }
            Value::Array(array) => {
                for (index, value) in array.iter().enumerate() {
                    option(value, &format!("{path}/{index}"), catalog, depth + 1)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
    for key in ["title", "notes"] {
        text(
            &document["metadata"][key],
            &format!("/metadata/{key}"),
            catalog,
        )?;
    }
    for (index, param) in document["params"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        text(&param["label"], &format!("/params/{index}/label"), catalog)?;
    }
    for (index, tab) in document["tabs"]
        .as_array()
        .into_iter()
        .flatten()
        .enumerate()
    {
        text(&tab["title"], &format!("/tabs/{index}/title"), catalog)?;
    }
    option(&document["chart"]["option"], "/chart/option", catalog, 0)
}

impl crate::models::MessageCatalog {
    /// BCP-47 parent fallback; never select a translation merely by map order.
    pub fn text(&self, key: &str, requested: &str) -> Option<&str> {
        let translations = self.messages.get(key)?;
        let mut candidate = requested
            .split('@')
            .next()
            .unwrap_or(requested)
            .replace('_', "-");
        loop {
            if let Some((_, text)) = translations
                .iter()
                .find(|(tag, _)| tag.eq_ignore_ascii_case(&candidate))
            {
                return Some(text);
            }
            match candidate.rsplit_once('-') {
                Some((parent, _)) => candidate = parent.into(),
                None => break,
            }
        }
        translations
            .iter()
            .find(|(tag, _)| tag.eq_ignore_ascii_case(&self.default_locale))
            .map(|(_, text)| text.as_str())
    }
}
