use super::*;
use tempfile::TempDir;

#[test]
fn initial_checkout_and_same_commit_branch_switch_invalidate_previews() {
    let temp = TempDir::new().unwrap();
    let directory = temp.path().join("new");
    let initial = run(&request(&directory, "init"), &Cancellation::default()).unwrap();
    assert_eq!(initial["branch"], "main");
    write(&directory, "new.txt", "new\n");
    assert_eq!(
        run(&request(&directory, "init"), &Cancellation::default())
            .unwrap_err()
            .code,
        "destination_not_empty"
    );
    let repo = Repository::open(&directory).unwrap();
    save(&repo, "first");
    let old = run(&request(&directory, "status"), &Cancellation::default()).unwrap();
    let mut input = request(&directory, "create_branch");
    input.branch = "same-commit".into();
    input.expected = old["revision"].as_str().unwrap().into();
    let switched = run(&input, &Cancellation::default()).unwrap();
    assert_ne!(old["revision"], switched["revision"]);
    input.op = "push".into();
    assert_eq!(
        run(&input, &Cancellation::default()).unwrap_err().code,
        "stale"
    );
}

fn write(directory: &Path, path: &str, text: &str) {
    let file = directory.join(path);
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(file, text).unwrap();
}
fn seed(directory: &Path) -> Repository {
    let mut options = git2::RepositoryInitOptions::new();
    options.initial_head("main");
    let repo = Repository::init_opts(directory, &options).unwrap();
    write(directory, "README.md", "base\n");
    write(directory, "team/charts/a.yaml", "base\n");
    save(&repo, "initial");
    repo
}
fn save(repo: &Repository, message: &str) {
    let mut index = repo.index().unwrap();
    index
        .add_all(["*"], git2::IndexAddOption::DEFAULT, None)
        .unwrap();
    index.write().unwrap();
    let tree = repo.find_tree(index.write_tree().unwrap()).unwrap();
    let author = Signature::now("Tester", "tester@example.com").unwrap();
    let parent = repo.head().ok().and_then(|h| h.peel_to_commit().ok());
    repo.commit(
        Some("HEAD"),
        &author,
        &author,
        message,
        &tree,
        &parent.iter().collect::<Vec<_>>(),
    )
    .unwrap();
}
fn request(directory: &Path, op: &str) -> Request {
    Request {
        op: op.into(),
        directory: directory.to_str().unwrap().into(),
        expected: open(directory.to_str().unwrap())
            .ok()
            .map(|r| revision(&r, &changes(&r).unwrap()).unwrap())
            .unwrap_or_default(),
        author_name: "Tester".into(),
        author_email: "tester@example.com".into(),
        ..Default::default()
    }
}
fn call(request: &Request) -> Value {
    run(request, &Cancellation::default()).unwrap()
}
fn remote_fixture() -> (TempDir, Repository, Repository, std::path::PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let source = seed(&temp.path().join("source"));
    let remote = Repository::init_bare(temp.path().join("remote")).unwrap();
    remote.set_head("refs/heads/main").unwrap();
    source
        .remote("origin", remote.path().to_str().unwrap())
        .unwrap()
        .push(&["refs/heads/main:refs/heads/main"], None)
        .unwrap();
    let target = temp.path().join("client");
    let clone = Request {
        url: remote.path().to_str().unwrap().into(),
        ..request(&target, "clone")
    };
    call(&clone);
    (temp, source, remote, target)
}
fn remote_save(source: &Repository, path: &str, text: &str) {
    write(source.workdir().unwrap(), path, text);
    save(source, "remote edit");
    source
        .find_remote("origin")
        .unwrap()
        .push(&["refs/heads/main:refs/heads/main"], None)
        .unwrap();
}

#[test]
fn staged_and_working_diffs_discard_and_selective_commit() {
    let temp = tempfile::tempdir().unwrap();
    let repo = seed(temp.path());
    write(temp.path(), "README.md", "first\n");
    write(temp.path(), "other.yaml", "untracked\n");
    let mut input = request(temp.path(), "diff");
    input.paths = vec!["README.md".into()];
    assert!(call(&input)["patch"].as_str().unwrap().contains("+first"));
    input = request(temp.path(), "stage");
    input.paths = vec!["README.md".into()];
    call(&input);
    write(temp.path(), "README.md", "second\n");
    input = request(temp.path(), "diff");
    input.area = "staged".into();
    input.paths = vec!["README.md".into()];
    let patch = call(&input);
    assert!(patch["patch"].as_str().unwrap().contains("+first"));
    assert!(!patch["patch"].as_str().unwrap().contains("second"));
    input = request(temp.path(), "discard");
    input.paths = vec!["README.md".into(), "other.yaml".into()];
    call(&input);
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "first\n"
    );
    assert!(!temp.path().join("other.yaml").exists());
    write(temp.path(), "team/charts/a.yaml", "keep unstaged\n");
    input = request(temp.path(), "commit");
    input.message = "selected file".into();
    call(&input);
    let tree = repo.head().unwrap().peel_to_tree().unwrap();
    assert_eq!(
        repo.find_blob(tree.get_path(Path::new("team/charts/a.yaml")).unwrap().id())
            .unwrap()
            .content(),
        b"base\n"
    );
    assert_eq!(
        call(&request(temp.path(), "status"))["changes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
#[test]
fn stale_review_and_escape_paths_cannot_discard_files() {
    let temp = tempfile::tempdir().unwrap();
    seed(temp.path());
    write(temp.path(), "README.md", "first\n");
    let mut input = request(temp.path(), "discard");
    input.paths = vec!["README.md".into()];
    write(temp.path(), "README.md", "newer\n");
    assert_eq!(
        run(&input, &Cancellation::default()).unwrap_err().code,
        "stale"
    );
    input = request(temp.path(), "discard");
    input.paths = vec!["../outside".into()];
    assert_eq!(
        run(&input, &Cancellation::default()).unwrap_err().code,
        "invalid_path"
    );
    assert_eq!(
        fs::read_to_string(temp.path().join("README.md")).unwrap(),
        "newer\n"
    );
}
#[test]
fn prefix_hides_other_changes_and_prevents_accidental_commit() {
    let temp = tempfile::tempdir().unwrap();
    let repo = seed(temp.path());
    write(temp.path(), "team/charts/a.yaml", "team\n");
    write(temp.path(), "README.md", "outside\n");
    let mut input = request(temp.path(), "stage");
    input.paths = vec!["README.md".into()];
    call(&input);
    input = request(temp.path(), "status");
    input.prefix = "team".into();
    let state = call(&input);
    assert_eq!(state["outside_staged"], 1);
    assert_eq!(state["changes"].as_array().unwrap().len(), 1);
    input = request(temp.path(), "commit");
    input.prefix = "team".into();
    input.message = "team edit".into();
    assert_eq!(
        run(&input, &Cancellation::default()).unwrap_err().code,
        "outside_staged"
    );
    assert_eq!(
        repo.head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .message()
            .unwrap(),
        "initial"
    );
    input = request(temp.path(), "stage");
    input.prefix = "team".into();
    input.paths = vec!["README.md".into()];
    assert_eq!(
        run(&input, &Cancellation::default()).unwrap_err().code,
        "invalid_path"
    );
}
#[test]
fn clone_refuses_nonempty_destinations_and_failed_clone_cleans_temp() {
    let temp = tempfile::tempdir().unwrap();
    let repo = seed(&temp.path().join("source"));
    let target = temp.path().join("target");
    write(&target, "keep", "keep");
    let mut input = request(&target, "clone");
    input.url = repo.workdir().unwrap().to_str().unwrap().into();
    assert_eq!(
        run(&input, &Cancellation::default()).unwrap_err().code,
        "destination_not_empty"
    );
    assert_eq!(fs::read_to_string(target.join("keep")).unwrap(), "keep");
    let empty = temp.path().join("empty");
    fs::create_dir(&empty).unwrap();
    input.directory = empty.to_str().unwrap().into();
    input.branch = "missing".into();
    assert!(run(&input, &Cancellation::default()).is_err());
    assert_eq!(fs::read_dir(empty).unwrap().count(), 0);
}
#[test]
fn fetch_pull_and_push_operate_on_real_tracking_refs() {
    let (_temp, source, remote, client) = remote_fixture();
    remote_save(&source, "README.md", "remote\n");
    let fetched = call(&request(&client, "fetch"));
    assert_eq!(fetched["behind"], 1);
    call(&request(&client, "pull"));
    assert_eq!(
        fs::read_to_string(client.join("README.md")).unwrap(),
        "remote\n"
    );
    write(&client, "team/charts/a.yaml", "local\n");
    let mut input = request(&client, "stage");
    input.paths = vec!["team/charts/a.yaml".into()];
    call(&input);
    input = request(&client, "commit");
    input.message = "local change".into();
    call(&input);
    let pushed = call(&request(&client, "push"));
    assert_eq!(pushed["ahead"], 0);
    assert_eq!(
        remote.refname_to_id("refs/heads/main").unwrap().to_string(),
        pushed["head"].as_str().unwrap()
    );
}
#[test]
fn dirty_pull_and_rejected_push_preserve_local_and_remote() {
    let (_temp, source, remote, client) = remote_fixture();
    write(&client, "README.md", "local draft\n");
    assert_eq!(
        run(&request(&client, "pull"), &Cancellation::default())
            .unwrap_err()
            .code,
        "dirty"
    );
    let client_repo = open(client.to_str().unwrap()).unwrap();
    save(&client_repo, "local");
    remote_save(&source, "README.md", "remote\n");
    let head = remote.refname_to_id("refs/heads/main").unwrap();
    assert_eq!(
        run(&request(&client, "push"), &Cancellation::default())
            .unwrap_err()
            .code,
        "remote_ahead"
    );
    assert_eq!(remote.refname_to_id("refs/heads/main").unwrap(), head);
    assert_eq!(
        fs::read_to_string(client.join("README.md")).unwrap(),
        "local draft\n"
    );
}
#[test]
fn merge_conflicts_can_resolve_and_commit_two_parents() {
    let (_temp, source, remote, client) = remote_fixture();
    let repo = open(client.to_str().unwrap()).unwrap();
    write(&client, "README.md", "local\n");
    save(&repo, "local");
    remote_save(&source, "README.md", "remote\n");
    let state = call(&request(&client, "pull"));
    assert_eq!(state["merging"], true);
    assert!(open(client.to_str().unwrap())
        .unwrap()
        .index()
        .unwrap()
        .has_conflicts());
    let mut input = request(&client, "resolve");
    input.paths = vec!["README.md".into()];
    input.resolution = "ours".into();
    call(&input);
    assert_eq!(
        fs::read_to_string(client.join("README.md")).unwrap(),
        "local\n"
    );
    assert!(!open(client.to_str().unwrap())
        .unwrap()
        .index()
        .unwrap()
        .has_conflicts());
    input = request(&client, "commit");
    input.message = "resolve merge".into();
    call(&input);
    assert_eq!(
        repo.head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .parent_count(),
        2
    );
    call(&request(&client, "push"));
    assert_eq!(
        remote.refname_to_id("refs/heads/main").unwrap(),
        repo.head().unwrap().target().unwrap()
    );
}
#[test]
fn abort_merge_and_clean_diverged_merge() {
    let (_temp, source, _remote, client) = remote_fixture();
    let repo = open(client.to_str().unwrap()).unwrap();
    write(&client, "README.md", "local\n");
    save(&repo, "local");
    remote_save(&source, "README.md", "remote\n");
    call(&request(&client, "pull"));
    call(&request(&client, "abort_merge"));
    assert_eq!(repo.state(), RepositoryState::Clean);
    assert_eq!(
        fs::read_to_string(client.join("README.md")).unwrap(),
        "local\n"
    );
    let (_temp2, source2, _remote2, client2) = remote_fixture();
    let repo2 = open(client2.to_str().unwrap()).unwrap();
    write(&client2, "team/charts/a.yaml", "local\n");
    save(&repo2, "local");
    remote_save(&source2, "README.md", "remote\n");
    let state = call(&request(&client2, "pull"));
    assert_eq!(state["merging"], false);
    assert_eq!(
        repo2
            .head()
            .unwrap()
            .peel_to_commit()
            .unwrap()
            .parent_count(),
        2
    );
}
#[test]
fn branches_switch_track_and_push_new_branch() {
    let (_temp, _source, remote, client) = remote_fixture();
    let mut input = request(&client, "create_branch");
    input.branch = "feature/charts".into();
    assert_eq!(call(&input)["branch"], "feature/charts");
    call(&request(&client, "push"));
    assert!(remote.find_reference("refs/heads/feature/charts").is_ok());
    input = request(&client, "checkout");
    input.branch = "main".into();
    assert_eq!(call(&input)["branch"], "main");
    assert!(
        call(&request(&client, "branches"))
            .as_array()
            .unwrap()
            .len()
            >= 3
    );
    assert!(call(&request(&client, "history")).as_array().unwrap().len() == 1);
}
#[test]
fn read_only_allows_pull_and_denies_editing() {
    let (_temp, source, _remote, client) = remote_fixture();
    remote_save(&source, "README.md", "updated\n");
    let mut input = request(&client, "pull");
    input.read_only = true;
    call(&input);
    input = request(&client, "stage");
    input.paths = vec!["README.md".into()];
    input.read_only = true;
    assert_eq!(
        run(&input, &Cancellation::default()).unwrap_err().code,
        "read_only"
    );
    assert_eq!(
        fs::read_to_string(client.join("README.md")).unwrap(),
        "updated\n"
    );
}

#[test]
fn missing_connection_keeps_diff_discard_and_fast_forward_pull() {
    let (temp, source, remote, client) = remote_fixture();
    let fresh = temp.path().join("anonymous-clone");
    call(&Request {
        url: remote.path().to_str().unwrap().into(),
        connection_missing: true,
        ..request(&fresh, "clone")
    });
    remote_save(&source, "README.md", "remote update\n");
    write(&client, "README.md", "local draft\n");
    let before = call(&request(&client, "status"))["head"].clone();
    let anonymous = |op: &str| Request {
        connection_missing: true,
        ..request(&client, op)
    };
    let mut input = anonymous("diff");
    input.paths = vec!["README.md".into()];
    assert!(call(&input)["patch"]
        .as_str()
        .unwrap()
        .contains("+local draft"));
    input.op = "stage".into();
    call(&input);
    input = anonymous("diff");
    input.paths = vec!["README.md".into()];
    input.area = "staged".into();
    assert!(call(&input)["patch"]
        .as_str()
        .unwrap()
        .contains("+local draft"));
    for op in ["commit", "push"] {
        assert_eq!(
            run(&anonymous(op), &Cancellation::default())
                .unwrap_err()
                .code,
            "connection_required"
        );
    }
    assert_eq!(
        run(&anonymous("pull"), &Cancellation::default())
            .unwrap_err()
            .code,
        "dirty"
    );
    assert_eq!(call(&anonymous("status"))["head"], before);
    assert_eq!(
        fs::read_to_string(client.join("README.md")).unwrap(),
        "local draft\n"
    );
    input = anonymous("unstage");
    input.paths = vec!["README.md".into()];
    call(&input);
    input = anonymous("discard");
    input.paths = vec!["README.md".into()];
    call(&input);
    assert_eq!(
        fs::read_to_string(client.join("README.md")).unwrap(),
        "base\n"
    );
    call(&anonymous("fetch"));
    call(&anonymous("pull"));
    assert_eq!(
        fs::read_to_string(client.join("README.md")).unwrap(),
        "remote update\n"
    );
    assert!(call(&anonymous("history")).as_array().unwrap().len() >= 2);

    write(&client, "README.md", "keep this edit\n");
    input = anonymous("discard");
    input.paths = vec!["README.md".into()];
    input.read_only = true;
    assert_eq!(
        run(&input, &Cancellation::default()).unwrap_err().code,
        "read_only"
    );
    input.op = "diff".into();
    assert!(call(&input)["patch"]
        .as_str()
        .unwrap()
        .contains("+keep this edit"));
}

#[test]
fn missing_connection_rejects_pull_merge_without_changing_checkout() {
    let (_temp, source, _remote, client) = remote_fixture();
    let repo = open(client.to_str().unwrap()).unwrap();
    write(&client, "local.txt", "local commit\n");
    save(&repo, "local commit");
    remote_save(&source, "remote.txt", "remote commit\n");
    let head = repo.head().unwrap().target().unwrap();
    let input = Request {
        connection_missing: true,
        ..request(&client, "pull")
    };
    assert_eq!(
        run(&input, &Cancellation::default()).unwrap_err().code,
        "connection_required_merge"
    );
    assert_eq!(repo.head().unwrap().target().unwrap(), head);
    assert_eq!(repo.state(), RepositoryState::Clean);
    assert!(!client.join("remote.txt").exists());
    call(&request(&client, "pull"));
    assert!(client.join("remote.txt").exists());
}

#[test]
fn remote_failures_have_actionable_codes() {
    for (code, class, text, expected) in [
        (
            ErrorCode::Auth,
            ErrorClass::Http,
            "credentials rejected",
            "authentication",
        ),
        (
            ErrorCode::GenericError,
            ErrorClass::Http,
            "unexpected http status code: 401",
            "authentication",
        ),
        (
            ErrorCode::GenericError,
            ErrorClass::Http,
            "unexpected http status code: 403",
            "permission",
        ),
        (
            ErrorCode::GenericError,
            ErrorClass::None,
            "GH013: repository rule violation",
            "permission",
        ),
        (
            ErrorCode::NotFastForward,
            ErrorClass::Reference,
            "rejected",
            "remote_ahead",
        ),
        (
            ErrorCode::Certificate,
            ErrorClass::Ssl,
            "certificate failed",
            "certificate",
        ),
        (
            ErrorCode::GenericError,
            ErrorClass::Net,
            "Could not resolve host",
            "network",
        ),
        (
            ErrorCode::GenericError,
            ErrorClass::Index,
            "invalid entry 403abc",
            "git",
        ),
    ] {
        let failure = Failure::from(git2::Error::new(code, class, text));
        assert_eq!(failure.code, expected);
        assert_eq!(failure.detail, text);
    }
}
