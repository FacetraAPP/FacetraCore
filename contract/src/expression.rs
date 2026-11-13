//! A versioned, expression-only Rhai subset shared by validation and execution.
//! No modules, user functions, statements, I/O, dynamic eval or output callbacks.
use crate::Result;
use rhai::{Dynamic, Engine, OptimizationLevel, Scope, AST};
use serde_json::Value;
use std::collections::BTreeMap;

pub struct Expression {
    engine: Engine,
    ast: AST,
}
impl Expression {
    pub fn compile(text: &str) -> Result<Self> {
        if text.trim().is_empty() || text.len() > 4096 {
            return Err("HTTP expression must contain 1–4096 bytes".into());
        }
        let mut engine = Engine::new();
        engine.set_optimization_level(OptimizationLevel::None);
        engine.set_max_operations(20_000);
        engine.set_max_expr_depths(32);
        engine.set_max_string_size(1024 * 1024);
        engine.set_max_array_size(100_000);
        engine.set_max_map_size(1024);
        engine.set_fail_on_invalid_map_property(true);
        engine.register_fn(
            "url_join",
            |base: &str, path: &str| -> std::result::Result<String, Box<rhai::EvalAltResult>> {
                url::Url::parse(base)
                    .and_then(|u| u.join(path))
                    .map(|u| u.to_string())
                    .map_err(|_| "Invalid URL".into())
            },
        );
        for symbol in [
            "eval", "print", "debug", "exit", "throw", "import", "export",
        ] {
            engine.disable_symbol(symbol);
        }
        // Never include expression text, bound data or engine diagnostics in ABI errors.
        let ast = engine
            .compile_expression(text)
            .map_err(|_| "Invalid HTTP expression")?;
        Ok(Self { engine, ast })
    }
    pub fn evaluate(&self, variables: &BTreeMap<String, Value>) -> Result<Value> {
        if serde_json::to_vec(variables)
            .map_err(|_| "Invalid HTTP expression input")?
            .len()
            > 8 * 1024 * 1024
        {
            return Err("HTTP expression input exceeds 8 MiB".into());
        }
        let mut scope = Scope::new();
        for (name, value) in variables {
            scope.push_constant_dynamic(
                name.clone(),
                rhai::serde::to_dynamic(value).map_err(|_| "Invalid HTTP expression input")?,
            );
        }
        let value: Dynamic = self
            .engine
            .eval_ast_with_scope(&mut scope, &self.ast)
            .map_err(|_| "HTTP expression evaluation failed")?;
        finite(&value)?;
        let value: Value = rhai::serde::from_dynamic(&value)
            .map_err(|_| "HTTP expression must return JSON-compatible data")?;
        check_value(&value, 0, &mut 0)?;
        if serde_json::to_vec(&value)
            .map_err(|_| "Invalid HTTP expression result")?
            .len()
            > 8 * 1024 * 1024
        {
            return Err("HTTP expression result exceeds 8 MiB".into());
        }
        Ok(value)
    }
}
fn finite(value: &Dynamic) -> Result<()> {
    if value.is::<rhai::FLOAT>() && !value.clone_cast::<rhai::FLOAT>().is_finite() {
        return Err("HTTP expression returned a nonfinite number".into());
    }
    if value.is_array() {
        for v in value.clone_cast::<rhai::Array>() {
            finite(&v)?;
        }
    }
    if value.is_map() {
        for v in value.clone_cast::<rhai::Map>().values() {
            finite(v)?;
        }
    }
    Ok(())
}
fn check_value(value: &Value, depth: usize, nodes: &mut usize) -> Result<()> {
    *nodes += 1;
    if depth > 64 || *nodes > 1_000_000 || value.as_f64().is_some_and(|n| !n.is_finite()) {
        return Err("HTTP expression result exceeds structural limits".into());
    }
    match value {
        Value::Array(a) => {
            for v in a {
                check_value(v, depth + 1, nodes)?;
            }
        }
        Value::Object(o) => {
            for v in o.values() {
                check_value(v, depth + 1, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn expressions_preserve_types_and_compute_state_without_mutating_inputs() {
        let vars = BTreeMap::from([
            ("page".into(), json!({"offset":10})),
            ("records".into(), json!([{"id":1},{"id":2}])),
        ]);
        let expr = Expression::compile("if records.len > 0 { #{offset: page.offset + records.len, ok: true, cursor: ()} } else { () }").unwrap();
        assert_eq!(
            expr.evaluate(&vars).unwrap(),
            json!({"offset":12,"ok":true,"cursor":null})
        );
        assert_eq!(vars["page"]["offset"], 10);
        assert_eq!(
            Expression::compile("()").unwrap().evaluate(&vars).unwrap(),
            Value::Null
        );
    }
    #[test]
    fn expressions_reject_statements_io_eval_and_resource_exhaustion() {
        for text in [
            "loop {}",
            "while true {}",
            "let x = 1",
            "page.x = 2",
            "eval(\"1\")",
            "print(\"secret\")",
            "import \"file\"",
            "|x| x + 1",
        ] {
            assert!(Expression::compile(text).is_err(), "{text}");
        }
        for text in [
            "missing.value",
            "#{a: 1}.absent",
            "1 / 0",
            "\"x\" * 2000000",
        ] {
            assert!(
                Expression::compile(text)
                    .unwrap()
                    .evaluate(&BTreeMap::new())
                    .is_err(),
                "{text}"
            );
        }
        assert!(Expression::compile(&"(".repeat(4097)).is_err());
        assert!(Expression::compile(&("(".repeat(100) + "1" + ")".repeat(100).as_str())).is_err());
    }
}
