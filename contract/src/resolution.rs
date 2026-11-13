use crate::{models::*, sql, validation::*, Result};
use serde::Serialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Serialize)]
pub struct ResolvedSource {
    pub spec: Source,
    pub params: BTreeMap<String, Value>,
    pub number_params: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub upstream: BTreeMap<String, ResolvedSource>,
}
#[derive(Serialize)]
pub struct Resolved {
    pub sources: BTreeMap<String, ResolvedSource>,
    pub exports: Vec<ResolvedSource>,
    pub sql: Option<String>,
    pub params: BTreeMap<String, Value>,
    pub number_params: Vec<String>,
}
pub fn number_params(params: &BTreeMap<String, Value>) -> Vec<String> {
    params
        .iter()
        .filter(|(_, v)| v.is_f64())
        .map(|(k, _)| k.clone())
        .collect()
}
struct Resolver<'a> {
    docs: &'a BTreeMap<String, Document>,
    partial: bool,
    expanded: std::cell::Cell<usize>,
}
struct ParameterScope<'a> {
    owner: &'a str,
    params: &'a BTreeMap<String, Value>,
    origins: &'a BTreeMap<String, BTreeSet<String>>,
}
impl<'a> Resolver<'a> {
    fn context(
        &self,
        reference: &str,
        owner: &str,
        params: &BTreeMap<String, Value>,
        arguments: Option<&BTreeMap<String, Value>>,
        source: bool,
    ) -> Result<(&'a Document, BTreeMap<String, Value>, Option<String>)> {
        let (id, name) =
            crate::workspace::target(reference, owner, source).map_err(|e| e.to_string())?;
        let name = name.map(str::to_owned);
        let target = self.docs.get(id).ok_or("Referenced chart missing")?;
        let mut passed = if id == owner {
            params.clone()
        } else {
            BTreeMap::new()
        };
        for (name, binding) in arguments.into_iter().flatten() {
            let value = if let Some(object) = binding.as_object() {
                if object.len() != 1 {
                    return Err("with binding accepts only param".into());
                }
                let key = object
                    .get("param")
                    .and_then(Value::as_str)
                    .ok_or("Invalid with binding")?;
                params
                    .get(key)
                    .ok_or("Parameter missing in with binding")?
                    .clone()
            } else {
                if !scalar(binding) {
                    return Err("with literal must be scalar".into());
                }
                binding.clone()
            };
            passed.insert(name.clone(), value);
        }
        // A source exports acquisition, not the target's entire dataset. Its
        // unrelated required parameters may still be awaiting user input.
        let passed = if self.partial || source {
            available_parameters(&target.params, &passed)?
        } else {
            target.parameters(&passed)?
        };
        Ok((target, passed, name))
    }
    fn push(&self, identity: String, stack: &[String]) -> Result<Vec<String>> {
        if stack.len() >= 64 || stack.contains(&identity) {
            return Err("Circular or excessively deep ref".into());
        }
        let mut next = stack.to_vec();
        next.push(identity);
        Ok(next)
    }
    fn source(
        &self,
        reference: &str,
        owner: &str,
        params: &BTreeMap<String, Value>,
        arguments: Option<&BTreeMap<String, Value>>,
        stack: &[String],
    ) -> Result<ResolvedSource> {
        let expanded = self.expanded.get() + 1;
        if expanded > 4096 {
            return Err("Source dependency expansion exceeds 4096 nodes".into());
        }
        self.expanded.set(expanded);
        let (doc, params, name) = self.context(reference, owner, params, arguments, true)?;
        let name = name.ok_or("Missing source name")?;
        let next = self.push(format!("{}#/sources/{name}", doc.metadata.id), stack)?;
        let spec = doc.sources.get(&name).ok_or("Source not found")?;
        spec.validate()?;
        if let Some(reference) = &spec.reference {
            let mut resolved = self.source(
                reference,
                &doc.metadata.id,
                &params,
                spec.with.as_ref(),
                &next,
            )?;
            if spec.display.is_some() {
                resolved.spec.display = spec.display.clone();
            }
            return Ok(resolved);
        }
        let resolved = ResolvedSource {
            spec: spec.clone(),
            upstream: spec
                .depends_on
                .iter()
                .flatten()
                .map(|name| {
                    Ok((
                        name.clone(),
                        self.source(
                            &format!("#/sources/{name}"),
                            &doc.metadata.id,
                            &params,
                            None,
                            &next,
                        )?,
                    ))
                })
                .collect::<Result<_>>()?,
            number_params: number_params(&params),
            params,
        };
        let resolved = if self.partial {
            resolved.normalized()?
        } else {
            resolved
        };
        for p in &doc.params {
            if let Some(value) = resolved.params.get(&p.name) {
                p.validate(value)?;
            }
        }
        Ok(resolved)
    }
    fn dataset(
        &self,
        doc: &'a Document,
        params: &BTreeMap<String, Value>,
        stack: &[String],
    ) -> Result<Resolved> {
        let next = self.push(format!("{}#/dataset", doc.metadata.id), stack)?;
        if let Some(reference) = &doc.dataset.reference {
            let (target, params, _) = self.context(
                reference,
                &doc.metadata.id,
                params,
                doc.dataset.with.as_ref(),
                false,
            )?;
            return self.dataset(target, &params, &next);
        }
        let mut sources = BTreeMap::new();
        for input in doc.dataset.sources.iter().flatten() {
            sources.insert(
                input.id.clone(),
                self.source(
                    &input.reference,
                    &doc.metadata.id,
                    params,
                    input.with.as_ref(),
                    &next,
                )?,
            );
        }
        if let Some(sql) = &doc.dataset.sql {
            sql::compile(sql, params)?;
        }
        Ok(Resolved {
            sources,
            exports: vec![],
            sql: doc.dataset.sql.clone(),
            params: params.clone(),
            number_params: number_params(params),
        })
    }
}
impl Resolver<'_> {
    // Track caller dependencies through renamed with bindings. Target defaults
    // and literals have no caller dependency, even when names happen to match.
    fn source_dependencies(
        &self,
        reference: &str,
        scope: ParameterScope<'_>,
        arguments: Option<&BTreeMap<String, Value>>,
        stack: &[String],
    ) -> Result<BTreeSet<String>> {
        let (doc, params, name) =
            self.context(reference, scope.owner, scope.params, arguments, true)?;
        let name = name.ok_or("Missing source name")?;
        let next = self.push(format!("{}#/sources/{name}", doc.metadata.id), stack)?;
        let spec = doc.sources.get(&name).ok_or("Source not found")?;
        let mut origins = if doc.metadata.id == scope.owner {
            scope.origins.clone()
        } else {
            BTreeMap::new()
        };
        for (name, binding) in arguments.into_iter().flatten() {
            let origin = binding
                .get("param")
                .and_then(Value::as_str)
                .and_then(|key| scope.origins.get(key))
                .cloned()
                .unwrap_or_default();
            origins.insert(name.clone(), origin);
        }
        if let Some(reference) = &spec.reference {
            return self.source_dependencies(
                reference,
                ParameterScope {
                    owner: &doc.metadata.id,
                    params: &params,
                    origins: &origins,
                },
                spec.with.as_ref(),
                &next,
            );
        }
        let source = ResolvedSource {
            spec: spec.clone(),
            upstream: BTreeMap::new(),
            number_params: number_params(&params),
            params: params.clone(),
        }
        .normalized()?;
        let mut dependencies: BTreeSet<String> = source
            .params
            .keys()
            .flat_map(|key| origins.get(key).into_iter().flatten().cloned())
            .collect();
        for name in spec.depends_on.iter().flatten() {
            dependencies.extend(self.source_dependencies(
                &format!("#/sources/{name}"),
                ParameterScope {
                    owner: &doc.metadata.id,
                    params: &params,
                    origins: &origins,
                },
                None,
                &next,
            )?);
        }
        Ok(dependencies)
    }
}

pub fn resolve(
    chart: &Document,
    docs: &BTreeMap<String, Document>,
    params: &BTreeMap<String, Value>,
) -> Result<Resolved> {
    let resolver = Resolver {
        docs,
        partial: false,
        expanded: std::cell::Cell::new(0),
    };
    let mut exports = Vec::new();
    for name in chart.sources.keys() {
        exports.push(resolver.source(
            &format!("#/sources/{name}"),
            &chart.metadata.id,
            params,
            None,
            &[],
        )?);
    }
    for parameter in &chart.params {
        if let Some(ParameterOptions::Source(options)) = &parameter.options {
            let resolved = resolve_source(
                docs,
                &chart.metadata.id,
                &chart.params,
                params,
                &options.reference,
                options.with.as_ref(),
            )?;
            if resolved.dependencies.contains(&parameter.name) {
                return Err("Option source depends on its own parameter".into());
            }
            exports.push(resolved.source);
        }
    }
    let mut dataset = resolver.dataset(chart, params, &[])?;
    dataset.exports = exports;
    Ok(dataset)
}

// A source can load choices before unrelated required parameters have values.
// Only acquisition dependencies are validated after source normalization.
fn available_parameters(
    definitions: &[Parameter],
    inputs: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>> {
    if inputs
        .keys()
        .any(|key| !definitions.iter().any(|p| &p.name == key))
    {
        return Err("Undefined parameter".into());
    }
    Ok(definitions
        .iter()
        .filter_map(|p| {
            inputs.get(&p.name).or(p.default.as_ref()).map(|v| {
                let value = if p.kind == "number" {
                    v.as_f64().map(Value::from).unwrap_or_else(|| v.clone())
                } else {
                    v.clone()
                };
                (p.name.clone(), value)
            })
        })
        .collect())
}

#[derive(Serialize)]
pub struct ResolvedOptionsSource {
    #[serde(flatten)]
    pub source: ResolvedSource,
    pub dependencies: BTreeSet<String>,
}

pub fn resolve_source(
    docs: &BTreeMap<String, Document>,
    owner: &str,
    definitions: &[Parameter],
    inputs: &BTreeMap<String, Value>,
    reference: &str,
    arguments: Option<&BTreeMap<String, Value>>,
) -> Result<ResolvedOptionsSource> {
    let params = available_parameters(definitions, inputs)?;
    let resolver = Resolver {
        docs,
        partial: true,
        expanded: std::cell::Cell::new(0),
    };
    let source = resolver.source(reference, owner, &params, arguments, &[])?;
    let origins = definitions
        .iter()
        .map(|p| (p.name.clone(), BTreeSet::from([p.name.clone()])))
        .collect();
    let dependencies = resolver.source_dependencies(
        reference,
        ParameterScope {
            owner,
            params: &params,
            origins: &origins,
        },
        arguments,
        &[],
    )?;
    Ok(ResolvedOptionsSource {
        source,
        dependencies,
    })
}

impl ResolvedSource {
    /// Retain acquisition dependencies only. This inspects templates/SQL and
    /// never opens a file, contacts a server or resolves a credential.
    pub fn normalized(mut self) -> Result<Self> {
        self.upstream = self
            .upstream
            .into_iter()
            .map(|(name, source)| Ok((name, source.normalized()?)))
            .collect::<Result<_>>()?;
        self.spec.validate()?;
        let used = if matches!(self.spec.source.as_deref(), Some("database" | "sqlite")) {
            sql::compile_dialect(
                self.spec.provider["sql"].as_str().ok_or("Missing SQL")?,
                &self.params,
                false,
                true,
            )?
            .parameters
        } else {
            crate::template::references(&self.spec)?.0
        };
        if !used.iter().all(|key| self.params.contains_key(key)) {
            return Err("Missing source parameter".into());
        }
        self.params.retain(|key, _| used.contains(key));
        self.number_params.retain(|key| used.contains(key));
        Ok(self)
    }
}
