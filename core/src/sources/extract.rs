//! Pure markup extraction. Never executes scripts or loads external resources.
use crate::{runtime::Cancellation, Result, MAX_ROWS};
use facetra_contract::models::{ExtractField, Extraction};
use scraper::{Html, Selector};
use serde_json::{Map, Value};
use sxd_xpath::{Context, Factory};

pub fn validate(spec: &Extraction, format: &str) -> Result<()> {
    if format == "html" {
        Selector::parse(&spec.rows).map_err(|_| "Invalid HTML row selector")?;
        for field in spec.fields.values().chain(spec.context.values()) {
            if field.selector != "." {
                Selector::parse(&field.selector).map_err(|_| "Invalid HTML field selector")?;
            }
        }
    } else {
        for selector in std::iter::once(&spec.rows).chain(
            spec.fields
                .values()
                .chain(spec.context.values())
                .map(|f| &f.selector),
        ) {
            Factory::new()
                .build(selector)
                .map_err(|_| "Invalid XML XPath")?
                .ok_or("Empty XML XPath")?;
        }
    }
    Ok(())
}
fn cell(text: String, field: &ExtractField, base: &str) -> Result<Value> {
    let text = text.trim();
    if field.json {
        return serde_json::from_str(text)
            .map_err(|_| "Invalid embedded JSON")
            .map_err(Into::into);
    }
    if field.resolve_url {
        let url = url::Url::parse(base)
            .and_then(|u| u.join(text))
            .map_err(|_| "Invalid extracted URL")?;
        return Ok(Value::String(url.into()));
    }
    Ok(Value::String(text.into()))
}
fn values(values: Vec<Value>, field: &ExtractField) -> Result<Value> {
    if values.is_empty() && field.required {
        return Err("Required extraction field is missing".into());
    }
    if field.all {
        Ok(Value::Array(values))
    } else {
        Ok(values.into_iter().next().unwrap_or(Value::Null))
    }
}
pub fn records(
    bytes: &[u8],
    format: &str,
    spec: &Extraction,
    base: &str,
    cancel: &Cancellation,
) -> Result<Vec<Value>> {
    if bytes.len() > 8 * 1024 * 1024 {
        return Err("Markup response exceeds 8 MiB".into());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "Markup responses must be UTF-8")?;
    cancel.check()?;
    if format == "html" {
        html(text, spec, base, cancel)
    } else {
        xml(text, spec, base, cancel)
    }
}
pub fn context(
    bytes: &[u8],
    format: &str,
    spec: &Extraction,
    base: &str,
    cancel: &Cancellation,
) -> Result<Value> {
    if spec.context.is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    let mut document = spec.clone();
    document.rows = if format == "html" { "html" } else { "/" }.into();
    document.fields = spec.context.clone();
    Ok(records(bytes, format, &document, base, cancel)?
        .into_iter()
        .next()
        .unwrap_or(Value::Object(Map::new())))
}
fn html(text: &str, spec: &Extraction, base: &str, cancel: &Cancellation) -> Result<Vec<Value>> {
    let document = Html::parse_document(text);
    let rows = Selector::parse(&spec.rows).map_err(|_| "Invalid HTML row selector")?;
    let fields: Vec<_> = spec
        .fields
        .iter()
        .map(|(name, f)| {
            Ok((
                name,
                f,
                if f.selector == "." {
                    None
                } else {
                    Some(Selector::parse(&f.selector).map_err(|_| "Invalid HTML field selector")?)
                },
            ))
        })
        .collect::<Result<_>>()?;
    let mut result = Vec::new();
    for row in document.select(&rows) {
        cancel.check()?;
        if result.len() >= MAX_ROWS {
            return Err("Extraction exceeds 100,000 rows".into());
        }
        let mut object = Map::new();
        for (name, f, selector) in &fields {
            let nodes: Vec<_> = match selector {
                Some(s) => row
                    .select(s)
                    .take(if f.all { MAX_ROWS + 1 } else { 1 })
                    .collect(),
                None => vec![row],
            };
            if nodes.len() > MAX_ROWS {
                return Err("Extraction field exceeds 100,000 values".into());
            }
            let mut cells = Vec::new();
            for node in nodes {
                let text = match &f.attribute {
                    Some(attr) => node.value().attr(attr).map(str::to_owned),
                    None => Some(node.text().collect::<String>()),
                };
                if let Some(text) = text {
                    cells.push(cell(text, f, base)?);
                }
            }
            object.insert((*name).clone(), values(cells, f)?);
        }
        result.push(Value::Object(object));
    }
    Ok(result)
}
fn xml(text: &str, spec: &Extraction, base: &str, cancel: &Cancellation) -> Result<Vec<Value>> {
    // Exclude DTD/entity expansion entirely, including internal entity bombs.
    if text.contains("<!DOCTYPE") || text.contains("<!ENTITY") {
        return Err("XML DTDs and entities are unsupported".into());
    }
    let package = sxd_document::parser::parse(text).map_err(|_| "Invalid XML response")?;
    let document = package.as_document();
    let mut context = Context::new();
    for (prefix, uri) in &spec.namespaces {
        context.set_namespace(prefix, uri);
    }
    let factory = Factory::new();
    let rows = factory
        .build(&spec.rows)
        .map_err(|_| "Invalid XML row XPath")?
        .ok_or("Empty XML row XPath")?;
    let sxd_xpath::Value::Nodeset(nodes) = rows
        .evaluate(&context, document.root())
        .map_err(|_| "XML row XPath failed")?
    else {
        return Err("XML rows XPath must return nodes".into());
    };
    if nodes.size() > MAX_ROWS {
        return Err("Extraction exceeds 100,000 rows".into());
    }
    let fields: Vec<_> = spec
        .fields
        .iter()
        .map(|(name, f)| {
            Ok((
                name,
                f,
                factory
                    .build(&f.selector)
                    .map_err(|_| "Invalid XML field XPath")?
                    .ok_or("Empty XML field XPath")?,
            ))
        })
        .collect::<Result<_>>()?;
    let mut result = Vec::new();
    for row in nodes.document_order() {
        cancel.check()?;
        let mut object = Map::new();
        for (name, f, xpath) in &fields {
            let selected = xpath
                .evaluate(&context, row)
                .map_err(|_| "XML field XPath failed")?;
            let cells = match selected {
                sxd_xpath::Value::Nodeset(nodes) => {
                    if nodes.size() > MAX_ROWS {
                        return Err("Extraction field exceeds 100,000 values".into());
                    }
                    nodes
                        .document_order()
                        .into_iter()
                        .take(if f.all { MAX_ROWS } else { 1 })
                        .filter_map(|node| {
                            let text = if let Some(attr) = &f.attribute {
                                node.element()
                                    .and_then(|e| e.attribute_value(attr.as_str()))
                                    .map(str::to_owned)
                            } else {
                                Some(node.string_value())
                            };
                            text.map(|t| cell(t, f, base))
                        })
                        .collect::<Result<Vec<_>>>()?
                }
                other => vec![cell(other.string(), f, base)?],
            };
            object.insert((*name).clone(), values(cells, f)?);
        }
        result.push(Value::Object(object));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn html_extracts_rows_attributes_entities_relative_links_and_embedded_json() {
        let spec = serde_json::from_value(json!({"rows":"article","fields":{
            "name":{"selector":"h2"},"link":{"selector":"a","attribute":"href","resolve_url":true},
            "meta":{"selector":"script","json":true},"optional":{"selector":".missing","required":false},
            "tags":{"selector":"b","all":true}}})).unwrap();
        let bytes = br#"<article><h2>A &amp; B</h2><a href="../detail">go</a><script type="application/json">{"id":2}</script><b>x</b><b>y</b></article>"#;
        let rows = records(
            bytes,
            "html",
            &spec,
            "https://example.com/list/page",
            &Cancellation::default(),
        )
        .unwrap();
        assert_eq!(rows, json!([{"name":"A & B","link":"https://example.com/detail","meta":{"id":2},"optional":null,"tags":["x","y"]}]).as_array().unwrap().clone());
    }
    #[test]
    fn xml_extracts_namespaced_atom_and_rss_with_strict_missing_fields() {
        let spec = serde_json::from_value(
            json!({"rows":"/a:feed/a:entry","namespaces":{"a":"urn:atom"},"fields":{
            "title":{"selector":"a:title"},"url":{"selector":"a:link/@href","resolve_url":true}}}),
        )
        .unwrap();
        let rows = records(br#"<feed xmlns="urn:atom"><entry><title>A &amp; B</title><link href="/post"/></entry></feed>"#,"xml",&spec,"https://example.com/feed",&Cancellation::default()).unwrap();
        assert_eq!(
            rows[0],
            json!({"title":"A & B","url":"https://example.com/post"})
        );
        assert!(records(
            br#"<feed xmlns="urn:atom"><entry/></feed>"#,
            "xml",
            &spec,
            "https://example.com",
            &Cancellation::default()
        )
        .is_err());
        assert!(records(
            br#"<!DOCTYPE feed [<!ENTITY x "y">]><feed/>"#,
            "xml",
            &spec,
            "https://example.com",
            &Cancellation::default()
        )
        .is_err());
        assert!(records(
            b"<feed>",
            "xml",
            &spec,
            "https://example.com",
            &Cancellation::default()
        )
        .is_err());
    }
}
