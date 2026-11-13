use crate::Result;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

pub struct Compiled {
    pub sql: String,
    pub values: Vec<Value>,
    pub parameters: BTreeSet<String>,
}
pub fn compile(sql: &str, params: &BTreeMap<String, Value>) -> Result<Compiled> {
    compile_dialect(sql, params, false, false)
}
pub fn compile_dialect(
    sql: &str,
    params: &BTreeMap<String, Value>,
    question_marks: bool,
    source: bool,
) -> Result<Compiled> {
    if sql.len() > 100_000 {
        return Err("SQL exceeds 100,000 bytes".into());
    }
    let mut parameters = BTreeSet::new();
    let chars: Vec<_> = sql.chars().collect();
    let (mut result, mut words, mut values, mut i) = (String::new(), String::new(), Vec::new(), 0);
    while i < chars.len() {
        let c = chars[i];
        if c == '\'' || c == '"' || (source && c == '`') {
            result.push(c);
            i += 1;
            let mut closed = false;
            while i < chars.len() {
                result.push(chars[i]);
                if chars[i] == c {
                    if chars.get(i + 1) == Some(&c) {
                        result.push(c);
                        i += 2;
                        continue;
                    }
                    closed = true;
                    i += 1;
                    break;
                }
                if source && chars[i] == '\\' {
                    return Err("Use standard doubled SQL quotes".into());
                }
                i += 1;
            }
            if !closed {
                return Err("Unterminated SQL quote".into());
            }
            words.push(' ');
            continue;
        }
        if c == '-' && chars.get(i + 1) == Some(&'-') {
            while i < chars.len() && chars[i] != '\n' {
                result.push(chars[i]);
                i += 1;
            }
            words.push(' ');
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            let mut depth = 1;
            result.push_str("/*");
            i += 2;
            while i < chars.len() && depth > 0 {
                if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
                    depth += 1;
                    result.push_str("/*");
                    i += 2;
                } else if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    depth -= 1;
                    result.push_str("*/");
                    i += 2;
                } else {
                    result.push(chars[i]);
                    i += 1;
                }
            }
            if depth != 0 {
                return Err("Unterminated SQL comment".into());
            }
            words.push(' ');
            continue;
        }
        if c == ';' || c == '?' || (source && c == '#') {
            return Err("Use one SELECT/WITH and named $parameters".into());
        }
        if c == '$' {
            i += 1;
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            if start == i
                || !(chars[start].is_ascii_alphabetic() || chars[start] == '_')
                || chars.get(i) == Some(&'$')
            {
                return Err("Use $name; dollar quotes are unsupported".into());
            }
            let name: String = chars[start..i].iter().collect();
            let value = params
                .get(&name)
                .ok_or_else(|| format!("Missing SQL parameter: {name}"))?;
            if !(value.is_string()
                || value.is_boolean()
                || value.is_i64()
                || value.is_f64() && value.as_f64().is_some_and(f64::is_finite))
            {
                return Err("SQL parameters must be finite scalars".into());
            }
            parameters.insert(name);
            values.push(value.clone());
            if question_marks {
                result.push('?');
            } else {
                result.push_str(&format!("${}", values.len()));
            }
            words.push(' ');
            continue;
        }
        result.push(c);
        words.push(c);
        i += 1;
    }
    let first: String = words
        .trim()
        .chars()
        .take_while(|c| c.is_alphabetic())
        .collect();
    if !["SELECT", "WITH"].contains(&first.to_uppercase().as_str()) {
        return Err("Transform requires SELECT/WITH".into());
    }
    Ok(Compiled {
        sql: result,
        values,
        parameters,
    })
}
pub fn identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}
