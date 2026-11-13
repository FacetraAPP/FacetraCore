use crate::{diagnostics::Diagnostic, models::*, template, validation::*};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
type Check<T = ()> = std::result::Result<T, Diagnostic>;
fn invalid(code: &str, path: &str) -> Diagnostic {
    Diagnostic::new(code, path)
}

pub fn validate_chart(doc: &Document, docs: &BTreeMap<String, Document>) -> Check {
    crate::localization::validate(&serde_json::to_value(doc).unwrap())?;
    doc.validate().map_err(|_| invalid("chart.invalid", ""))?;
    let mut graph = Graph {
        docs,
        active: BTreeSet::new(),
        complete: BTreeSet::new(),
        dependency_steps: std::cell::Cell::new(0),
    };
    graph.document(doc)
}

/// Parse file#pointer without consulting the filesystem. File identifiers are
/// metadata IDs, even when a host stores the file in nested library folders.
pub fn target<'a>(
    reference: &'a str,
    owner: &'a str,
    source: bool,
) -> Check<(&'a str, Option<&'a str>)> {
    let (file, pointer) = reference
        .split_once('#')
        .ok_or_else(|| invalid("reference.syntax", ""))?;
    let id = if file.is_empty() {
        owner
    } else {
        let id = file
            .strip_suffix(".yaml")
            .ok_or_else(|| invalid("reference.syntax", ""))?;
        valid_id(id).map_err(|_| invalid("reference.syntax", ""))?;
        id
    };
    let name = if source {
        let name = pointer
            .strip_prefix("/sources/")
            .ok_or_else(|| invalid("reference.syntax", ""))?;
        valid_id(name).map_err(|_| invalid("reference.syntax", ""))?;
        Some(name)
    } else {
        if pointer != "/dataset" {
            return Err(invalid("reference.syntax", ""));
        }
        None
    };
    Ok((id, name))
}
fn bindings(
    owner: &[Parameter],
    target: &[Parameter],
    arguments: Option<&BTreeMap<String, Value>>,
    inherited: bool,
    path: &str,
) -> Check {
    for (name, value) in arguments.into_iter().flatten() {
        let p = target
            .iter()
            .find(|p| &p.name == name)
            .ok_or_else(|| invalid("binding.unknown", path))?;
        if let Some(object) = value.as_object() {
            if object.len() != 1 {
                return Err(invalid("binding.invalid", path));
            }
            let key = object
                .get("param")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("binding.invalid", path))?;
            let from = owner
                .iter()
                .find(|p| p.name == key)
                .ok_or_else(|| invalid("binding.missing", path))?;
            if from.kind != p.kind && !(p.kind == "number" && from.kind == "integer") {
                return Err(invalid("binding.type", path));
            }
            if let Some(default) = &from.default {
                p.validate(default)
                    .map_err(|_| invalid("binding.value", path))?;
            }
        } else {
            p.validate(value)
                .map_err(|_| invalid("binding.value", path))?;
        }
    }
    if !inherited
        && target.iter().any(|p| {
            p.required && p.default.is_none() && arguments.is_none_or(|a| !a.contains_key(&p.name))
        })
    {
        return Err(invalid("binding.required", path));
    }
    Ok(())
}
struct Graph<'a> {
    docs: &'a BTreeMap<String, Document>,
    active: BTreeSet<String>,
    complete: BTreeSet<String>,
    dependency_steps: std::cell::Cell<usize>,
}
impl<'a> Graph<'a> {
    fn document(&mut self, doc: &Document) -> Check {
        self.node(&doc.metadata.id, None)?;
        for name in doc.sources.keys() {
            self.node(&doc.metadata.id, Some(name))?;
        }
        self.options(&doc.params, &doc.metadata.id)
    }
    fn edge(
        &mut self,
        reference: &str,
        owner: &str,
        parameters: &[Parameter],
        args: Option<&BTreeMap<String, Value>>,
        source: bool,
        path: &str,
    ) -> Check {
        let (id, name) = target(reference, owner, source).map_err(|mut e| {
            e.path = path.into();
            e
        })?;
        let doc = self
            .docs
            .get(id)
            .ok_or_else(|| invalid("reference.missing", path).resource(owner))?;
        bindings(parameters, &doc.params, args, id == owner || source, path)?;
        if source && id != owner {
            let origins = doc
                .params
                .iter()
                .map(|p| (p.name.clone(), BTreeSet::from([p.name.clone()])))
                .collect();
            let used = self.dependencies(
                &format!("#/sources/{}", name.unwrap()),
                id,
                &origins,
                None,
                0,
            )?;
            if doc.params.iter().any(|p| {
                used.contains(&p.name)
                    && p.required
                    && p.default.is_none()
                    && args.is_none_or(|a| !a.contains_key(&p.name))
            }) {
                return Err(invalid("binding.required", path));
            }
        }
        self.node(id, name)
    }
    fn node(&mut self, id: &str, source: Option<&str>) -> Check {
        let key = format!(
            "{id}#{}",
            source
                .map(|s| format!("/sources/{s}"))
                .unwrap_or("/dataset".into())
        );
        if self.active.contains(&key) || self.active.len() >= 64 {
            return Err(invalid("reference.cycle", "").resource(id));
        }
        if self.complete.contains(&key) {
            return Ok(());
        }
        let doc = self
            .docs
            .get(id)
            .ok_or_else(|| invalid("reference.missing", ""))?;
        crate::localization::validate(&serde_json::to_value(doc).unwrap())?;
        doc.validate()
            .map_err(|_| invalid("chart.invalid", "").resource(id))?;
        self.active.insert(key.clone());
        if let Some(name) = source {
            let s = doc.sources.get(name).ok_or_else(|| {
                invalid("reference.missing", format!("/sources/{name}").as_str()).resource(id)
            })?;
            for dependency in s.depends_on.iter().flatten() {
                self.edge(
                    &format!("#/sources/{dependency}"),
                    id,
                    &doc.params,
                    None,
                    true,
                    &format!("/sources/{name}/depends_on"),
                )?;
            }
            if let Some(reference) = &s.reference {
                self.edge(
                    reference,
                    id,
                    &doc.params,
                    s.with.as_ref(),
                    true,
                    &format!("/sources/{name}/ref"),
                )?;
            } else {
                let used = if matches!(s.source.as_deref(), Some("database" | "sqlite")) {
                    let placeholders = doc
                        .params
                        .iter()
                        .map(|p| (p.name.clone(), Value::from(0)))
                        .collect();
                    crate::sql::compile_dialect(
                        s.provider["sql"].as_str().unwrap_or(""),
                        &placeholders,
                        false,
                        true,
                    )
                    .map_err(|_| {
                        invalid(
                            "parameter.reference",
                            format!("/sources/{name}/sql").as_str(),
                        )
                    })?
                    .parameters
                } else {
                    template::references(s)
                        .map_err(|_| {
                            invalid("template.invalid", format!("/sources/{name}").as_str())
                        })?
                        .0
                };
                if used
                    .iter()
                    .any(|p| !doc.params.iter().any(|v| &v.name == p))
                {
                    return Err(invalid(
                        "parameter.reference",
                        format!("/sources/{name}").as_str(),
                    ));
                }
            }
        } else if let Some(reference) = &doc.dataset.reference {
            self.edge(
                reference,
                id,
                &doc.params,
                doc.dataset.with.as_ref(),
                false,
                "/dataset/ref",
            )?;
        } else {
            for input in doc.dataset.sources.iter().flatten() {
                self.edge(
                    &input.reference,
                    id,
                    &doc.params,
                    input.with.as_ref(),
                    true,
                    "/dataset/sources",
                )?;
            }
            if let Some(sql) = &doc.dataset.sql {
                let placeholders = doc
                    .params
                    .iter()
                    .map(|p| (p.name.clone(), Value::from(0)))
                    .collect();
                crate::sql::compile(sql, &placeholders)
                    .map_err(|_| invalid("parameter.reference", "/dataset/sql"))?;
            }
        }
        self.active.remove(&key);
        self.complete.insert(key);
        Ok(())
    }
    fn options(&mut self, params: &[Parameter], owner: &str) -> Check {
        for (index, p) in params.iter().enumerate() {
            if let Some(ParameterOptions::Source(options)) = &p.options {
                let path = format!("/params/{index}/options");
                self.edge(
                    &options.reference,
                    owner,
                    params,
                    options.with.as_ref(),
                    true,
                    &path,
                )?;
                let origins = params
                    .iter()
                    .map(|p| (p.name.clone(), BTreeSet::from([p.name.clone()])))
                    .collect();
                let deps = self.dependencies(
                    &options.reference,
                    owner,
                    &origins,
                    options.with.as_ref(),
                    0,
                )?;
                if deps.contains(&p.name) {
                    return Err(invalid("parameter.options_cycle", &path));
                }
            }
        }
        Ok(())
    }
    fn dependencies(
        &self,
        reference: &str,
        owner: &str,
        origins: &BTreeMap<String, BTreeSet<String>>,
        args: Option<&BTreeMap<String, Value>>,
        depth: usize,
    ) -> Check<BTreeSet<String>> {
        let steps = self.dependency_steps.get() + 1;
        self.dependency_steps.set(steps);
        if steps > 4096 {
            return Err(invalid("reference.limit", ""));
        }
        if depth >= 64 {
            return Err(invalid("reference.cycle", ""));
        }
        let (id, name) = target(reference, owner, true)?;
        let doc = self
            .docs
            .get(id)
            .ok_or_else(|| invalid("reference.missing", ""))?;
        let spec = doc
            .sources
            .get(name.unwrap())
            .ok_or_else(|| invalid("reference.missing", ""))?;
        let mut passed = if id == owner {
            origins.clone()
        } else {
            BTreeMap::new()
        };
        for (key, value) in args.into_iter().flatten() {
            passed.insert(
                key.clone(),
                value
                    .get("param")
                    .and_then(Value::as_str)
                    .and_then(|p| origins.get(p))
                    .cloned()
                    .unwrap_or_default(),
            );
        }
        if let Some(reference) = &spec.reference {
            return self.dependencies(reference, id, &passed, spec.with.as_ref(), depth + 1);
        }
        let used = if matches!(spec.source.as_deref(), Some("database" | "sqlite")) {
            let placeholders = doc
                .params
                .iter()
                .map(|p| (p.name.clone(), Value::from(0)))
                .collect();
            crate::sql::compile_dialect(
                spec.provider["sql"].as_str().unwrap_or(""),
                &placeholders,
                false,
                true,
            )
            .map_err(|_| invalid("parameter.reference", ""))?
            .parameters
        } else {
            template::references(spec)
                .map_err(|_| invalid("template.invalid", ""))?
                .0
        };
        let mut dependencies: BTreeSet<String> = used
            .iter()
            .flat_map(|k| passed.get(k).into_iter().flatten().cloned())
            .collect();
        for name in spec.depends_on.iter().flatten() {
            dependencies.extend(self.dependencies(
                &format!("#/sources/{name}"),
                id,
                &passed,
                None,
                depth + 1,
            )?);
        }
        Ok(dependencies)
    }
}

pub fn validate_dashboard(d: &Dashboard, docs: &BTreeMap<String, Document>) -> Check {
    crate::localization::validate(&serde_json::to_value(d).unwrap())?;
    if docs.contains_key(&d.metadata.id) {
        return Err(invalid("resource.duplicate", "/metadata/id"));
    }
    valid_id(&d.metadata.id).map_err(|_| invalid("dashboard.invalid", "/metadata/id"))?;
    if d.version != 1 || d.kind != "Dashboard" || d.metadata.title.is_blank() || d.tabs.is_empty() {
        return Err(invalid("dashboard.invalid", ""));
    }
    validate_tags(&d.metadata._tags).map_err(|_| invalid("dashboard.invalid", "/metadata/tags"))?;
    validate_definitions(&d.params).map_err(|_| invalid("parameter.invalid", "/params"))?;
    let mut graph = Graph {
        docs,
        active: BTreeSet::new(),
        complete: BTreeSet::new(),
        dependency_steps: std::cell::Cell::new(0),
    };
    graph.options(&d.params, &d.metadata.id)?;
    let mut tabs = BTreeSet::new();
    let mut panels = BTreeSet::new();
    for (t, tab) in d.tabs.iter().enumerate() {
        let path = format!("/tabs/{t}");
        if valid_id(&tab.id).is_err() || !tabs.insert(&tab.id) || tab.title.is_empty() {
            return Err(invalid("dashboard.tab", &path));
        }
        if let Some(grid) = &tab.layout {
            if grid.kind != "grid"
                || !(1..=12).contains(&grid.columns)
                || !grid.row_height.is_finite()
                || !(24.0..=120.0).contains(&grid.row_height)
                || !grid.gap.is_finite()
                || !(0.0..=32.0).contains(&grid.gap)
                || !grid.min_width.is_finite()
                || !(320.0..=2400.0).contains(&grid.min_width)
                || grid.min_width <= f64::from(grid.columns - 1) * grid.gap
            {
                return Err(invalid(
                    "dashboard.layout",
                    format!("{path}/layout").as_str(),
                ));
            }
        }
        for (i, panel) in tab.panels.iter().enumerate() {
            let path = format!("{path}/panels/{i}");
            if valid_id(&panel.id).is_err() || !panels.insert(&panel.id) {
                return Err(invalid("resource.duplicate", &path));
            }
            if !panel.height.is_finite() || !(160.0..=600.0).contains(&panel.height) {
                return Err(invalid("dashboard.layout", &path));
            }
            if let Some(grid) = &tab.layout {
                let rows = panel.row_span.unwrap_or(4);
                if !(1..=grid.columns).contains(&panel.width.unwrap_or(grid.columns))
                    || !(1..=24).contains(&rows)
                    || f64::from(rows) * grid.row_height + f64::from(rows - 1) * grid.gap < 160.0
                {
                    return Err(invalid("dashboard.layout", &path));
                }
            } else if panel.width.is_some()
                || panel.row_span.is_some()
                || panel.responsive.is_some()
            {
                return Err(invalid("dashboard.layout", &path));
            }
            if let Some(r) = &panel.responsive {
                if !r.compact.card_height.is_finite()
                    || !(160.0..=600.0).contains(&r.compact.card_height)
                {
                    return Err(invalid("dashboard.layout", &path));
                }
            }
            let chart = docs
                .get(&panel.chart)
                .ok_or_else(|| invalid("reference.missing", format!("{path}/chart").as_str()))?;
            bindings(
                &d.params,
                &chart.params,
                Some(&panel.params),
                false,
                &format!("{path}/params"),
            )?;
            graph.document(chart)?;
        }
    }
    Ok(())
}
