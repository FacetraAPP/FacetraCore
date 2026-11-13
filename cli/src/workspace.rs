use facetra_contract::{
    self as contract,
    models::{Document, LocalizedText},
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceIssue {
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceInfo {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub path: String,
    pub renderer: String,
    pub sha256: String,
}

#[derive(Clone)]
pub struct Resource {
    pub info: ResourceInfo,
    pub yaml: String,
}

pub struct Workspace {
    pub root: PathBuf,
    charts: Vec<Resource>,
    issues: Vec<WorkspaceIssue>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationReport {
    pub valid: bool,
    pub charts: usize,
    pub dashboards: usize,
    pub issues: Vec<WorkspaceIssue>,
}

impl Workspace {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, String> {
        let root = fs::canonicalize(root.as_ref())
            .map_err(|error| format!("Cannot open workspace: {error}"))?;
        if !root.is_dir() {
            return Err(format!("Workspace is not a directory: {}", root.display()));
        }

        let mut charts = Vec::new();
        let mut issues = Vec::new();
        let charts_root = root.join("charts");
        if charts_root.is_dir() {
            let mut files = Vec::new();
            collect_yaml(&charts_root, &mut files, &mut issues, &root)?;
            files.sort();
            for file in files {
                match load_chart(&root, &file) {
                    Ok(resource) => {
                        if charts
                            .iter()
                            .any(|item: &Resource| item.info.id == resource.info.id)
                        {
                            issues.push(WorkspaceIssue {
                                path: resource.info.path,
                                message: format!("Duplicate chart ID: {}", resource.info.id),
                            });
                        } else {
                            charts.push(resource);
                        }
                    }
                    Err(message) => issues.push(WorkspaceIssue {
                        path: relative_path(&root, &file),
                        message,
                    }),
                }
            }
        }
        charts.sort_by(|a, b| a.info.id.cmp(&b.info.id));
        Ok(Self {
            root,
            charts,
            issues,
        })
    }

    pub fn charts(&self) -> &[Resource] {
        &self.charts
    }

    pub fn issues(&self) -> &[WorkspaceIssue] {
        &self.issues
    }

    pub fn chart(&self, id: &str) -> Option<&Resource> {
        self.charts.iter().find(|chart| chart.info.id == id)
    }

    pub fn documents(&self) -> Vec<String> {
        self.charts.iter().map(|chart| chart.yaml.clone()).collect()
    }

    pub fn overlay(&self, path: &Path) -> Result<Resource, String> {
        let yaml = fs::read_to_string(path)
            .map_err(|error| format!("Cannot read {}: {error}", path.display()))?;
        let value = contract::parse(&yaml).map_err(|error| error.to_string())?;
        let document = contract::chart(&value).map_err(|error| error.to_string())?;
        Ok(Resource {
            info: resource_info(format!("<draft:{}>", path.display()), &document, &yaml),
            yaml,
        })
    }

    pub fn render_documents(
        &self,
        id: &str,
        overlay: Option<&Resource>,
    ) -> Result<(String, Vec<String>, String), String> {
        let mut selected = None;
        let mut documents = Vec::new();
        for chart in &self.charts {
            if chart.info.id == id {
                if overlay.is_some() {
                    continue;
                }
                selected = Some(chart.clone());
            }
            documents.push(chart.yaml.clone());
        }
        if let Some(overlay) = overlay {
            if overlay.info.id != id {
                return Err(format!(
                    "Draft chart ID {} does not match the target ID {id}.",
                    overlay.info.id
                ));
            }
            selected = Some(overlay.clone());
            documents.push(overlay.yaml.clone());
        }
        let chart = selected.ok_or_else(|| format!("Chart not found: {id}"))?;
        Ok((
            chart.info.renderer.clone(),
            documents,
            chart.info.title.clone(),
        ))
    }

    pub fn validate(&self) -> ValidationReport {
        let mut issues = self.issues.clone();
        let documents = match contract::documents(&self.documents()) {
            Ok(documents) => documents,
            Err(message) => {
                issues.push(WorkspaceIssue {
                    path: "charts/".into(),
                    message,
                });
                BTreeMap::new()
            }
        };
        for chart in &self.charts {
            if let Some(document) = documents.get(&chart.info.id) {
                if let Err(error) = contract::validate_chart(document, &documents) {
                    issues.push(WorkspaceIssue {
                        path: chart.info.path.clone(),
                        message: error.to_string(),
                    });
                }
            }
        }
        ValidationReport {
            valid: issues.is_empty(),
            charts: self.charts.len(),
            dashboards: count_dashboards(&self.root),
            issues,
        }
    }
}

pub fn find_root(start: impl AsRef<Path>) -> Result<PathBuf, String> {
    let mut path = fs::canonicalize(start.as_ref())
        .map_err(|error| format!("Cannot resolve workspace path: {error}"))?;
    loop {
        if path.join("charts").is_dir() || path.join("workspace.yaml").is_file() {
            return Ok(path);
        }
        if !path.pop() {
            return Err("Specify --workspace, or run inside a Facetra workspace.".into());
        }
    }
}

fn load_chart(root: &Path, file: &Path) -> Result<Resource, String> {
    let yaml = fs::read_to_string(file)
        .map_err(|error| format!("Cannot read {}: {error}", file.display()))?;
    let value = contract::parse(&yaml).map_err(|error| error.to_string())?;
    let document = contract::chart(&value).map_err(|error| error.to_string())?;
    let relative = relative_path(root, file);
    let info = resource_info(relative, &document, &yaml);
    Ok(Resource { info, yaml })
}

fn resource_info(path: String, document: &Document, yaml: &str) -> ResourceInfo {
    ResourceInfo {
        id: document.metadata.id.clone(),
        title: match &document.metadata.title {
            LocalizedText::Literal(title) => title.clone(),
            LocalizedText::Reference(_) => document.metadata.id.clone(),
        },
        kind: "chart".into(),
        path,
        renderer: document.chart.renderer.clone(),
        sha256: format!("{:x}", Sha256::digest(yaml.as_bytes())),
    }
}

fn collect_yaml(
    directory: &Path,
    files: &mut Vec<PathBuf>,
    issues: &mut Vec<WorkspaceIssue>,
    root: &Path,
) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("Cannot read {}: {error}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|error| error.to_string())?;
        if file_type.is_symlink() {
            issues.push(WorkspaceIssue {
                path: relative_path(root, &path),
                message: "Symlinked resources are not supported.".into(),
            });
        } else if file_type.is_dir() {
            collect_yaml(&path, files, issues, root)?;
        } else if path
            .extension()
            .is_some_and(|extension| extension == "yaml")
        {
            files.push(path);
        }
    }
    Ok(())
}

fn count_dashboards(root: &Path) -> usize {
    let mut files = Vec::new();
    let mut issues = Vec::new();
    if root.join("dashboards").is_dir() {
        let _ = collect_yaml(&root.join("dashboards"), &mut files, &mut issues, root);
    }
    files.len()
}

fn relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_charts_and_rejects_duplicate_ids() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("charts/a")).unwrap();
        let yaml = r#"metadata:
  id: sample
  title: Sample
sources:
  input: {source: duckdb, file: data/input.csv}
dataset: {id: data, sources: [{id: input, ref: '#/sources/input'}]}
chart: {dataset: data, renderer: value, value: amount}
"#;
        fs::write(root.path().join("charts/a/sample.yaml"), yaml).unwrap();
        fs::write(root.path().join("charts/a/other.yaml"), yaml).unwrap();
        let workspace = Workspace::open(root.path()).unwrap();
        assert_eq!(workspace.charts().len(), 1);
        assert_eq!(workspace.issues().len(), 1);
        assert_eq!(workspace.chart("sample").unwrap().info.renderer, "value");
    }

    #[test]
    fn overlays_a_draft_with_the_same_id() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("charts")).unwrap();
        let yaml = r#"metadata:
  id: sample
  title: Sample
sources:
  input: {source: duckdb, file: data/input.csv}
dataset: {id: data, sources: [{id: input, ref: '#/sources/input'}]}
chart: {dataset: data, renderer: value, value: amount}
"#;
        fs::write(root.path().join("charts/sample.yaml"), yaml).unwrap();
        let draft = root.path().join("draft.yaml");
        let mut file = fs::File::create(&draft).unwrap();
        file.write_all(yaml.as_bytes()).unwrap();
        let workspace = Workspace::open(root.path()).unwrap();
        let overlay = workspace.overlay(&draft).unwrap();
        let (renderer, documents, title) = workspace
            .render_documents("sample", Some(&overlay))
            .unwrap();
        assert_eq!(renderer, "value");
        assert_eq!(documents.len(), 1);
        assert_eq!(title, "Sample");
    }
}
