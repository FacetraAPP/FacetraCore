//! A real libgit2 checkout shared by the macOS and iOS applications.
use crate::runtime::Cancellation;
use git2::{
    build::CheckoutBuilder, BranchType, Cred, DiffOptions, ErrorClass, ErrorCode, ObjectType, Oid,
    Repository, RepositoryState, Signature, Status, StatusOptions,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    ffi::{CStr, CString},
    fs,
    os::raw::c_char,
    path::{Component, Path},
    sync::Mutex,
};

// Multiple app windows must not operate on the same index concurrently.
static OPERATIONS: Mutex<()> = Mutex::new(());
const PREVIEW_BYTES: usize = 1024 * 1024;

#[derive(Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Request {
    pub op: String,
    pub directory: String,
    pub prefix: String,
    pub url: String,
    pub branch: String,
    pub token: String,
    pub username: String,
    pub paths: Vec<String>,
    pub expected: String,
    pub area: String,
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    pub read_only: bool,
    pub connection_missing: bool,
    pub bare: bool,
    pub resolution: String,
    pub commit: String,
    pub content: String,
}
#[derive(Debug, Serialize)]
pub struct Failure {
    pub code: String,
    pub detail: String,
}
type Result<T> = std::result::Result<T, Failure>;
fn fail(code: &str) -> Failure {
    Failure {
        code: code.into(),
        detail: String::new(),
    }
}
impl From<git2::Error> for Failure {
    fn from(error: git2::Error) -> Self {
        let message = error.message().to_lowercase();
        let code = if error.code() == ErrorCode::Auth
            || message.contains("status code: 401")
            || message.contains("http 401")
            || message.contains("authentication is required")
        {
            "authentication"
        } else if error.code() == ErrorCode::NotFastForward || message.contains("non-fast-forward")
        {
            "remote_ahead"
        } else if [
            "status code: 403",
            "http 403",
            "write access",
            "permission denied",
            "protected branch",
            "gh006",
            "gh013",
        ]
        .iter()
        .any(|text| message.contains(text))
        {
            "permission"
        } else if error.code() == ErrorCode::Certificate {
            "certificate"
        } else if error.code() == ErrorCode::Timeout
            || matches!(
                error.class(),
                ErrorClass::Net | ErrorClass::Http | ErrorClass::Ssl
            )
        {
            "network"
        } else {
            "git"
        };
        Self {
            code: code.into(),
            detail: error.message().into(),
        }
    }
}
impl From<std::io::Error> for Failure {
    fn from(error: std::io::Error) -> Self {
        Self {
            code: "io".into(),
            detail: error.to_string(),
        }
    }
}
fn check_cancel(cancel: &Cancellation) -> Result<()> {
    cancel.check().map_err(|_| fail("cancelled"))
}

fn callbacks<'a>(request: &'a Request, cancel: &'a Cancellation) -> git2::RemoteCallbacks<'a> {
    let mut callbacks = git2::RemoteCallbacks::new();
    callbacks.credentials(move |url, _, allowed| {
        // Never pass a GitHub token to a different host or a cleartext transport.
        if !url.starts_with("https://github.com/")
            || request.token.is_empty()
            || !allowed.contains(git2::CredentialType::USER_PASS_PLAINTEXT)
        {
            return Err(git2::Error::from_str(
                "GitHub authentication is required. Select a connection with a token.",
            ));
        }
        Cred::userpass_plaintext(
            if request.username.is_empty() {
                "x-access-token"
            } else {
                &request.username
            },
            &request.token,
        )
    });
    callbacks.transfer_progress(move |_| cancel.check().is_ok());
    callbacks.sideband_progress(move |_| cancel.check().is_ok());
    callbacks.push_transfer_progress(move |_, _, _| {});
    callbacks.push_negotiation(move |_| {
        cancel
            .check()
            .map_err(|_| git2::Error::from_str("Cancelled"))
    });
    callbacks.push_update_reference(|_, status| match status {
        Some(status) => Err(git2::Error::from_str(status)),
        None => Ok(()),
    });
    callbacks
}
fn fetch_options<'a>(request: &'a Request, cancel: &'a Cancellation) -> git2::FetchOptions<'a> {
    let mut options = git2::FetchOptions::new();
    options.remote_callbacks(callbacks(request, cancel));
    options
}

fn open(directory: &str) -> Result<Repository> {
    let path = Path::new(directory);
    if fs::symlink_metadata(path)?.file_type().is_symlink() {
        return Err(fail("invalid_path"));
    }
    let repo = Repository::open(path).map_err(|_| fail("not_repository"))?;
    if repo.is_bare()
        || repo.workdir().and_then(|p| p.canonicalize().ok()) != Some(path.canonicalize()?)
    {
        return Err(fail("not_repository"));
    }
    Ok(repo)
}
fn changes(repo: &Repository) -> Result<Vec<Value>> {
    let mut options = StatusOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .renames_head_to_index(false)
        .renames_index_to_workdir(false);
    repo.statuses(Some(&mut options))?.iter().map(|entry| {
        let path = entry.path().map_err(|_| fail("invalid_path"))?;
        let status = entry.status();
        let staged = status.intersects(Status::INDEX_NEW | Status::INDEX_MODIFIED | Status::INDEX_DELETED | Status::INDEX_RENAMED | Status::INDEX_TYPECHANGE);
        let unstaged = status.intersects(Status::WT_NEW | Status::WT_MODIFIED | Status::WT_DELETED | Status::WT_RENAMED | Status::WT_TYPECHANGE);
        let kind = if status.is_conflicted() { "conflict" }
            else if status.intersects(Status::WT_DELETED | Status::INDEX_DELETED) { "deleted" }
            else if status.intersects(Status::WT_NEW | Status::INDEX_NEW) { "added" }
            else { "modified" };
        Ok(json!({"path": path, "kind": kind, "staged": staged, "unstaged": unstaged, "conflict": status.is_conflicted()}))
    }).collect()
}
fn revision(repo: &Repository, changes: &[Value]) -> Result<String> {
    let mut bytes = repo
        .head()
        .ok()
        .and_then(|h| h.target())
        .map(|id| id.to_string())
        .unwrap_or_default()
        .into_bytes();
    bytes.extend_from_slice(format!("{:?}", repo.state()).as_bytes());
    if let Ok(head) = repo.head() {
        bytes.extend_from_slice(head.name_bytes());
    }
    // Switching branches at the same commit or changing a remote invalidates an open preview.
    if let Ok(config) = fs::read(repo.path().join("config")) {
        bytes.extend_from_slice(Oid::hash_object(ObjectType::Blob, &config)?.as_bytes());
    }
    // Include the index and exact changed bytes: a stale diff cannot discard newer edits.
    for entry in repo.index()?.iter() {
        bytes.extend_from_slice(&entry.path);
        bytes.extend_from_slice(entry.id.as_bytes());
        bytes.extend_from_slice(&entry.mode.to_le_bytes());
        bytes.extend_from_slice(&entry.flags.to_le_bytes());
    }
    for change in changes {
        let path = change["path"]
            .as_str()
            .ok_or_else(|| fail("invalid_path"))?;
        bytes.extend_from_slice(path.as_bytes());
        let full = repo.workdir().unwrap().join(path);
        match fs::symlink_metadata(&full) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bytes.extend_from_slice(fs::read_link(full)?.as_os_str().as_encoded_bytes())
            }
            Ok(metadata) if metadata.is_file() => {
                use std::os::unix::fs::MetadataExt;
                bytes.extend_from_slice(&metadata.mode().to_le_bytes());
                // Hash incrementally so imported databases do not duplicate their contents in memory.
                use std::io::Read;
                let mut file = fs::File::open(full)?;
                let mut buffer = [0; 65536];
                // Git's blob hash includes length. Hash the individual chunks into a fingerprint.
                bytes.extend_from_slice(&metadata.len().to_le_bytes());
                loop {
                    let n = file.read(&mut buffer)?;
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(
                        Oid::hash_object(ObjectType::Blob, &buffer[..n])?.as_bytes(),
                    );
                }
            }
            Ok(metadata) if metadata.is_dir() => {
                bytes.extend_from_slice(format!("{:?}", metadata.modified()).as_bytes());
                if let Ok(nested) = Repository::open(&full) {
                    if let Ok(head) = nested.head() {
                        if let Some(id) = head.target() {
                            bytes.extend_from_slice(id.as_bytes());
                        }
                    }
                }
            }
            Ok(_) => return Err(fail("invalid_path")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => bytes.push(0),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(Oid::hash_object(ObjectType::Blob, &bytes)?.to_string())
}
fn in_scope(path: &str, prefix: &str) -> bool {
    prefix.is_empty() || path.starts_with(&format!("{prefix}/"))
}
fn status(repo: &Repository, prefix: &str) -> Result<Value> {
    let changes = changes(repo)?;
    let head = repo.head().ok();
    let branch = head
        .as_ref()
        .and_then(|h| h.shorthand().ok())
        .map(String::from)
        .or_else(|| {
            repo.find_reference("HEAD").ok().and_then(|h| {
                h.symbolic_target()
                    .ok()
                    .flatten()
                    .map(|name| name.trim_start_matches("refs/heads/").to_string())
            })
        })
        .unwrap_or_else(|| "HEAD".into());
    let mut ahead = 0;
    let mut behind = 0;
    let upstream = head
        .as_ref()
        .and_then(|h| h.name().ok())
        .and_then(|name| repo.branch_upstream_name(name).ok())
        .and_then(|buf| buf.as_str().ok().map(String::from));
    if let (Some(local), Some(remote)) = (
        head.as_ref().and_then(|h| h.target()),
        upstream.as_ref().and_then(|r| repo.refname_to_id(r).ok()),
    ) {
        (ahead, behind) = repo.graph_ahead_behind(local, remote)?;
    }
    let config = repo.config()?;
    let scoped: Vec<_> = changes
        .iter()
        .filter(|c| in_scope(c["path"].as_str().unwrap_or_default(), prefix))
        .collect();
    let outside_staged = changes
        .iter()
        .filter(|c| {
            !in_scope(c["path"].as_str().unwrap_or_default(), prefix) && c["staged"] == true
        })
        .count();
    Ok(
        json!({"revision": revision(repo, &changes)?, "branch": branch,
        "head": head.as_ref().and_then(|h| h.target()).map(|id| id.to_string()),
        "remote": repo.find_remote("origin").ok().and_then(|r| r.url().ok().map(String::from)),
        "upstream": upstream, "ahead": ahead, "behind": behind, "changes": scoped, "outside_staged": outside_staged,
        "merging": repo.state() == RepositoryState::Merge,
        "author_name": config.get_string("user.name").unwrap_or_default(),
        "author_email": config.get_string("user.email").unwrap_or_default()}),
    )
}
fn expected(repo: &Repository, request: &Request) -> Result<()> {
    if request.expected.is_empty() || revision(repo, &changes(repo)?)? != request.expected {
        return Err(fail("stale"));
    }
    Ok(())
}
fn require_clean(repo: &Repository) -> Result<()> {
    if repo.state() != RepositoryState::Clean {
        return Err(fail("operation_in_progress"));
    }
    if !changes(repo)?.is_empty() {
        return Err(fail("dirty"));
    }
    Ok(())
}
fn valid_paths(repo: &Repository, paths: &[String]) -> Result<()> {
    if paths.is_empty() {
        return Err(fail("no_selection"));
    }
    for path in paths {
        if path.is_empty()
            || Path::new(path)
                .components()
                .any(|c| !matches!(c, Component::Normal(_)))
            || path.split('/').any(|c| c.eq_ignore_ascii_case(".git"))
        {
            return Err(fail("invalid_path"));
        }
        let mut parent = repo.workdir().unwrap().to_path_buf();
        let components: Vec<_> = Path::new(path).components().collect();
        for component in &components[..components.len() - 1] {
            parent.push(component);
            if fs::symlink_metadata(&parent).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(fail("invalid_path"));
            }
        }
    }
    Ok(())
}
fn signature(request: &Request, repo: &Repository) -> Result<Signature<'static>> {
    if !request.author_name.trim().is_empty() && !request.author_email.trim().is_empty() {
        Ok(Signature::now(
            request.author_name.trim(),
            request.author_email.trim(),
        )?)
    } else {
        repo.signature().map_err(|_| fail("author_required"))
    }
}
fn head_tree(repo: &Repository) -> Result<Option<git2::Tree<'_>>> {
    match repo.head() {
        Ok(head) => Ok(Some(head.peel_to_tree()?)),
        Err(e) if e.code() == ErrorCode::UnbornBranch || e.code() == ErrorCode::NotFound => {
            Ok(None)
        }
        Err(e) => Err(e.into()),
    }
}
fn stage(repo: &Repository, paths: &[String]) -> Result<()> {
    valid_paths(repo, paths)?;
    let mut index = repo.index()?;
    for path in paths {
        match fs::symlink_metadata(repo.workdir().unwrap().join(path)) {
            Ok(_) => index.add_path(Path::new(path))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                index.remove_path(Path::new(path))?
            }
            Err(e) => return Err(e.into()),
        }
    }
    index.write()?;
    Ok(())
}
fn diff(repo: &Repository, request: &Request) -> Result<Value> {
    let mut options = DiffOptions::new();
    options
        .include_untracked(true)
        .recurse_untracked_dirs(true)
        .show_untracked_content(true)
        .disable_pathspec_match(true)
        .context_lines(3);
    if !request.paths.is_empty() {
        valid_paths(repo, &request.paths)?;
        for path in &request.paths {
            options.pathspec(path);
        }
    }
    let tree = head_tree(repo)?;
    let diff = if !request.commit.is_empty() {
        let commit = repo.find_commit(Oid::from_str(&request.commit)?)?;
        let old = if commit.parent_count() > 0 {
            Some(commit.parent(0)?.tree()?)
        } else {
            None
        };
        repo.diff_tree_to_tree(old.as_ref(), Some(&commit.tree()?), Some(&mut options))?
    } else if request.area == "staged" {
        repo.diff_tree_to_index(tree.as_ref(), None, Some(&mut options))?
    } else {
        repo.diff_index_to_workdir(None, Some(&mut options))?
    };
    let mut text = Vec::new();
    let mut binary = false;
    let mut truncated = false;
    diff.print(git2::DiffFormat::Patch, |delta, _, line| {
        if !delta
            .new_file()
            .path()
            .or(delta.old_file().path())
            .is_some_and(|p| in_scope(&p.to_string_lossy(), &request.prefix))
        {
            return true;
        }
        if line.origin() == 'B' {
            binary = true;
        }
        if text.len() + line.content().len() > PREVIEW_BYTES {
            truncated = true;
            return true;
        }
        if matches!(line.origin(), '+' | '-' | ' ') {
            text.push(line.origin() as u8);
        }
        text.extend_from_slice(line.content());
        true
    })?;
    let working = if request.paths.len() == 1 && repo.index()?.has_conflicts() {
        let file = repo.workdir().unwrap().join(&request.paths[0]);
        match fs::symlink_metadata(&file) {
            Ok(metadata) if metadata.is_file() && metadata.len() <= PREVIEW_BYTES as u64 => {
                fs::read_to_string(file).ok()
            }
            _ => None,
        }
    } else {
        None
    };
    Ok(
        json!({"patch": String::from_utf8_lossy(&text), "binary": binary, "truncated": truncated, "working": working}),
    )
}
fn commit(repo: &Repository, request: &Request) -> Result<()> {
    if changes(repo)?.iter().any(|c| {
        c["staged"] == true && !in_scope(c["path"].as_str().unwrap_or_default(), &request.prefix)
    }) {
        return Err(fail("outside_staged"));
    }
    if request.message.trim().is_empty() {
        return Err(fail("message_required"));
    }
    if !matches!(
        repo.state(),
        RepositoryState::Clean | RepositoryState::Merge
    ) {
        return Err(fail("operation_in_progress"));
    }
    let mut index = repo.index()?;
    if index.has_conflicts() {
        return Err(fail("conflicts"));
    }
    let tree = repo.find_tree(index.write_tree()?)?;
    let mut parents = Vec::new();
    if let Ok(head) = repo.head() {
        parents.push(head.peel_to_commit()?);
    }
    if repo.state() == RepositoryState::Merge {
        let data = fs::read_to_string(repo.path().join("MERGE_HEAD"))?;
        for oid in data.lines() {
            parents.push(repo.find_commit(Oid::from_str(oid)?)?);
        }
    } else if parents
        .first()
        .is_some_and(|parent| parent.tree_id() == tree.id())
    {
        return Err(fail("nothing_staged"));
    }
    let signature = signature(request, repo)?;
    repo.commit(
        Some("HEAD"),
        &signature,
        &signature,
        request.message.trim(),
        &tree,
        &parents.iter().collect::<Vec<_>>(),
    )?;
    if repo.state() == RepositoryState::Merge {
        repo.cleanup_state()?;
    }
    Ok(())
}
fn upstream(repo: &Repository) -> Result<(String, String, String)> {
    let head = repo.head().map_err(|_| fail("branch_required"))?;
    if !head.is_branch() {
        return Err(fail("branch_required"));
    }
    let name = head.name().map_err(|_| fail("branch_required"))?;
    let branch = head.shorthand().map_err(|_| fail("branch_required"))?;
    let remote = repo
        .branch_upstream_remote(name)
        .ok()
        .and_then(|b| b.as_str().ok().map(String::from))
        .unwrap_or_else(|| "origin".into());
    let merge = repo
        .branch_upstream_merge(name)
        .ok()
        .and_then(|b| b.as_str().ok().map(String::from))
        .unwrap_or_else(|| format!("refs/heads/{branch}"));
    if remote == "." {
        return Err(fail("remote_required"));
    }
    let target = format!(
        "refs/remotes/{remote}/{}",
        merge
            .strip_prefix("refs/heads/")
            .ok_or_else(|| fail("branch_required"))?
    );
    Ok((remote, merge, target))
}
fn fetch(repo: &Repository, request: &Request, cancel: &Cancellation) -> Result<()> {
    let (name, _, _) = upstream(repo)?;
    let mut remote = repo
        .find_remote(&name)
        .map_err(|_| fail("remote_required"))?;
    remote.fetch(
        &[] as &[&str],
        Some(&mut fetch_options(request, cancel)),
        None,
    )?;
    check_cancel(cancel)
}
fn pull(repo: &Repository, request: &Request, cancel: &Cancellation) -> Result<()> {
    require_clean(repo)?;
    fetch(repo, request, cancel)?;
    let (_, _, target) = upstream(repo)?;
    let reference = repo.find_reference(&target)?;
    let annotated = repo.reference_to_annotated_commit(&reference)?;
    let (analysis, _) = repo.merge_analysis(&[&annotated])?;
    if analysis.is_up_to_date() {
        return Ok(());
    }
    if analysis.is_fast_forward() {
        let object = repo.find_object(annotated.id(), None)?;
        // Safe checkout verifies that no external writer dirtied the checkout after fetch.
        require_clean(repo)?;
        repo.checkout_tree(&object, Some(CheckoutBuilder::new().safe()))?;
        repo.head()?.set_target(annotated.id(), "Facetra: pull")?;
    } else if analysis.is_normal() {
        if request.read_only {
            return Err(fail("read_only_diverged"));
        }
        if request.connection_missing {
            return Err(fail("connection_required_merge"));
        }
        let _ = signature(request, repo)?;
        require_clean(repo)?;
        repo.merge(&[&annotated], None, Some(CheckoutBuilder::new().safe()))?;
        if !repo.index()?.has_conflicts() {
            let message = format!("Merge {}", reference.shorthand().unwrap_or("upstream"));
            let merged = Request {
                message,
                author_name: request.author_name.clone(),
                author_email: request.author_email.clone(),
                ..Default::default()
            };
            commit(repo, &merged)?;
        }
    } else {
        return Err(fail("unrelated_history"));
    }
    Ok(())
}
fn resolve(repo: &Repository, request: &Request) -> Result<()> {
    valid_paths(repo, &request.paths)?;
    let index = repo.index()?;
    let conflicts: Vec<_> = index
        .conflicts()?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for path in &request.paths {
        let conflict = conflicts
            .iter()
            .find(|c| {
                [&c.ancestor, &c.our, &c.their]
                    .iter()
                    .any(|e| e.as_ref().is_some_and(|e| e.path == path.as_bytes()))
            })
            .ok_or_else(|| fail("stale"))?;
        if request.resolution == "ours" || request.resolution == "theirs" {
            let entry = if request.resolution == "ours" {
                &conflict.our
            } else {
                &conflict.their
            };
            if let Some(entry) = entry {
                // Symlink conflicts need explicit external resolution; never follow one while writing a blob.
                if entry.mode & 0o170000 != 0o100000 {
                    return Err(fail("invalid_path"));
                }
                let full = repo.workdir().unwrap().join(path);
                if fs::symlink_metadata(&full).is_ok_and(|m| m.file_type().is_symlink()) {
                    fs::remove_file(&full)?;
                }
                if let Some(parent) = full.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&full, repo.find_blob(entry.id)?.content())?;
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(
                    full,
                    fs::Permissions::from_mode(if entry.mode & 0o111 != 0 {
                        0o755
                    } else {
                        0o644
                    }),
                )?;
            } else {
                let full = repo.workdir().unwrap().join(path);
                if fs::symlink_metadata(&full).is_ok() {
                    fs::remove_file(full)?;
                }
            }
        } else if request.resolution == "manual" {
            let full = repo.workdir().unwrap().join(path);
            if fs::symlink_metadata(&full).is_ok_and(|m| m.file_type().is_symlink()) {
                return Err(fail("invalid_path"));
            }
            if let Some(parent) = full.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(full, &request.content)?;
        } else if request.resolution != "working" {
            return Err(fail("invalid_request"));
        }
    }
    stage(repo, &request.paths)
}

fn clone_repository(request: &Request, cancel: &Cancellation) -> Result<Value> {
    // HTTPS GitHub is the app transport. Local paths remain available to hermetic engine tests.
    if !request.url.starts_with("https://github.com/") && !Path::new(&request.url).is_absolute() {
        return Err(fail("invalid_remote"));
    }
    let destination = Path::new(&request.directory);
    let created = !destination.exists();
    if created {
        fs::create_dir_all(destination)?;
    }
    let result = (|| {
        if fs::symlink_metadata(destination)?.file_type().is_symlink()
            || fs::read_dir(destination)?.next().is_some()
        {
            return Err(fail("destination_not_empty"));
        }
        let staging = tempfile::Builder::new()
            .prefix(".facetra-clone-")
            .tempdir_in(destination)?;
        let mut builder = git2::build::RepoBuilder::new();
        if !request.branch.is_empty() {
            builder.branch(&request.branch);
        }
        builder.fetch_options(fetch_options(request, cancel));
        let repo = builder.clone(&request.url, staging.path())?;
        check_cancel(cancel)?;
        drop(repo);
        let entries: Vec<_> = fs::read_dir(staging.path())?.collect::<std::io::Result<Vec<_>>>()?;
        let mut moved = Vec::new();
        for entry in entries {
            let target = destination.join(entry.file_name());
            if target.exists() {
                for (from, to) in moved.into_iter().rev() {
                    let _ = fs::rename(to, from);
                }
                return Err(fail("destination_not_empty"));
            }
            if let Err(error) = fs::rename(entry.path(), &target) {
                for (from, to) in moved.into_iter().rev() {
                    let _ = fs::rename(to, from);
                }
                return Err(error.into());
            }
            moved.push((entry.path(), target));
        }
        status(&open(&request.directory)?, &request.prefix)
    })();
    if created && result.is_err() {
        let _ = fs::remove_dir(destination);
    }
    result
}

pub fn run(request: &Request, cancel: &Cancellation) -> Result<Value> {
    let _lock = OPERATIONS.lock().map_err(|_| fail("internal"))?;
    // These global options are only touched under our libgit2 operation lock.
    unsafe {
        git2::opts::set_server_connect_timeout_in_milliseconds(15000)?;
        git2::opts::set_server_timeout_in_milliseconds(30000)?;
    }
    check_cancel(cancel)?;
    if !request.prefix.is_empty() {
        if Path::new(&request.prefix)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
            || request
                .prefix
                .split('/')
                .any(|c| c.eq_ignore_ascii_case(".git") || c.is_empty())
        {
            return Err(fail("invalid_path"));
        }
        if request.paths.iter().any(|p| !in_scope(p, &request.prefix)) {
            return Err(fail("invalid_path"));
        }
    }
    if request.op == "clone" {
        return clone_repository(request, cancel);
    }
    if request.op == "init" {
        if request.read_only {
            return Err(fail("read_only"));
        }
        let path = Path::new(&request.directory);
        if path.exists()
            && (fs::symlink_metadata(path)?.file_type().is_symlink()
                || fs::read_dir(path)?.next().is_some())
        {
            return Err(fail("destination_not_empty"));
        }
        let mut options = git2::RepositoryInitOptions::new();
        options.initial_head("main").bare(request.bare);
        let repo = Repository::init_opts(path, &options)?;
        return if request.bare {
            Ok(json!({"directory": request.directory}))
        } else {
            status(&repo, &request.prefix)
        };
    }
    let repo = open(&request.directory)?;
    match request.op.as_str() {
        "status" => return status(&repo, &request.prefix),
        "diff" => {
            expected(&repo, request)?;
            return diff(&repo, request);
        }
        "history" => {
            let mut walk = repo.revwalk()?;
            if repo.head().is_err() {
                return Ok(json!([]));
            }
            walk.push_head()?;
            walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)?;
            let entries: Vec<_> = walk
                .take(100)
                .map(|id| {
                    let c = repo.find_commit(id?)?;
                    Ok(
                        json!({"id": c.id().to_string(), "message": c.message().unwrap_or_default(),
                    "author": c.author().name().unwrap_or_default(), "time": c.time().seconds()}),
                    )
                })
                .collect::<std::result::Result<_, git2::Error>>()?;
            return Ok(json!(entries));
        }
        "branches" => {
            let branches: Vec<_> = repo.branches(None)?.map(|item| {
                let (b, kind) = item?;
                Ok(json!({"name": b.name()?.unwrap_or_default(), "current": b.is_head(), "remote": kind == BranchType::Remote}))
            }).collect::<std::result::Result<_, git2::Error>>()?;
            return Ok(json!(branches));
        }
        _ => {}
    }
    if request.read_only && !matches!(request.op.as_str(), "fetch" | "pull") {
        return Err(fail("read_only"));
    }
    if request.connection_missing && matches!(request.op.as_str(), "commit" | "push") {
        return Err(fail("connection_required"));
    }
    expected(&repo, request)?;
    match request.op.as_str() {
        "stage" => {
            if repo.index()?.has_conflicts() {
                return Err(fail("conflicts"));
            }
            stage(&repo, &request.paths)?;
        }
        "unstage" => {
            valid_paths(&repo, &request.paths)?;
            if repo.index()?.has_conflicts() {
                return Err(fail("conflicts"));
            }
            if let Ok(head) = repo.head() {
                repo.reset_default(
                    Some(&head.peel(ObjectType::Commit)?),
                    request.paths.iter().map(Path::new),
                )?;
            } else {
                let mut index = repo.index()?;
                for path in &request.paths {
                    index.remove_path(Path::new(path))?;
                }
                index.write()?;
            }
        }
        "discard" => {
            valid_paths(&repo, &request.paths)?;
            if repo.state() != RepositoryState::Clean {
                return Err(fail("operation_in_progress"));
            }
            let index = repo.index()?;
            for path in &request.paths {
                if index.get_path(Path::new(path), 0).is_some() {
                    repo.checkout_index(None, Some(CheckoutBuilder::new().force().path(path)))?;
                } else {
                    let full = repo.workdir().unwrap().join(path);
                    if fs::symlink_metadata(&full).is_ok() {
                        fs::remove_file(full)?;
                    }
                }
            }
        }
        "commit" => commit(&repo, request)?,
        "fetch" => fetch(&repo, request, cancel)?,
        "pull" => pull(&repo, request, cancel)?,
        "set_remote" => {
            if !request.url.starts_with("https://github.com/")
                && !Path::new(&request.url).is_absolute()
            {
                return Err(fail("invalid_remote"));
            }
            if repo.find_remote("origin").is_ok() {
                repo.remote_set_url("origin", &request.url)?;
            } else {
                repo.remote("origin", &request.url)?;
            }
        }
        "push" => {
            if repo.state() != RepositoryState::Clean {
                return Err(fail("operation_in_progress"));
            }
            let (name, merge, target) = upstream(&repo)?;
            let head = repo.head()?;
            let refspec = format!(
                "{}:{merge}",
                head.name().map_err(|_| fail("branch_required"))?
            );
            let mut remote = repo.find_remote(&name)?;
            let mut options = git2::PushOptions::new();
            options.remote_callbacks(callbacks(request, cancel));
            check_cancel(cancel)?;
            remote.push(&[refspec], Some(&mut options))?;
            repo.reference(
                &target,
                head.target().ok_or_else(|| fail("branch_required"))?,
                true,
                "Facetra: push",
            )?;
            let branch = head.shorthand().map_err(|_| fail("branch_required"))?;
            repo.find_branch(branch, BranchType::Local)?
                .set_upstream(Some(&format!(
                    "{name}/{}",
                    merge.trim_start_matches("refs/heads/")
                )))?;
        }
        "resolve" => resolve(&repo, request)?,
        "abort_merge" => {
            if repo.state() != RepositoryState::Merge {
                return Err(fail("operation_in_progress"));
            }
            repo.reset(
                &repo.head()?.peel(ObjectType::Commit)?,
                git2::ResetType::Hard,
                None,
            )?;
            repo.cleanup_state()?;
        }
        "checkout" | "create_branch" => {
            require_clean(&repo)?;
            if request.branch.is_empty() {
                return Err(fail("branch_required"));
            }
            let branch = if request.op == "create_branch" {
                repo.branch(&request.branch, &repo.head()?.peel_to_commit()?, false)?
            } else if let Ok(branch) = repo.find_branch(&request.branch, BranchType::Local) {
                branch
            } else {
                let remote = repo.find_branch(&request.branch, BranchType::Remote)?;
                let name = request
                    .branch
                    .split_once('/')
                    .ok_or_else(|| fail("branch_required"))?
                    .1;
                let mut local = repo.branch(name, &remote.get().peel_to_commit()?, false)?;
                local.set_upstream(Some(&request.branch))?;
                local
            };
            repo.checkout_tree(
                &branch.get().peel(ObjectType::Commit)?,
                Some(CheckoutBuilder::new().safe()),
            )?;
            repo.set_head(branch.get().name().map_err(|_| fail("branch_required"))?)?;
        }
        _ => return Err(fail("invalid_request")),
    }
    status(&repo, &request.prefix)
}

/// # Safety
/// Input is null or valid NUL-terminated UTF-8. Keep cancellation alive for the
/// entire call. Free the returned JSON string once with facetra_pipeline_free.
#[no_mangle]
pub unsafe extern "C" fn facetra_git_run(
    input: *const c_char,
    cancel: *const Cancellation,
) -> *mut c_char {
    let response = std::panic::catch_unwind(|| {
        if input.is_null() {
            return Err(fail("invalid_request"));
        }
        let input = unsafe { CStr::from_ptr(input) }
            .to_str()
            .map_err(|_| fail("invalid_request"))?;
        if input.len() > 8 * 1024 * 1024 {
            return Err(fail("invalid_request"));
        }
        let request: Request = serde_json::from_str(input).map_err(|_| fail("invalid_request"))?;
        let local = Cancellation::default();
        let cancel = unsafe { cancel.as_ref() }.unwrap_or(&local);
        run(&request, cancel).map_err(|mut error| {
            // Transport messages can contain URLs; credentials must not appear in UI/logs.
            if !request.token.is_empty() {
                error.detail = error.detail.replace(&request.token, "[redacted]");
            }
            error
        })
    });
    let json = match response {
        Ok(Ok(value)) => json!({"ok": value}),
        Ok(Err(error)) => json!({"error": error}),
        Err(_) => json!({"error": fail("internal")}),
    };
    CString::new(json.to_string())
        .expect("JSON escapes NUL")
        .into_raw()
}

#[cfg(test)]
#[path = "tests/git.rs"]
mod tests;
