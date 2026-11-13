use super::*;
use serde_json::json;
fn source(file: &str) -> Resolved {
    serde_json::from_value(json!({"spec":{"source":"duckdb","file":file}})).unwrap()
}
#[test]
fn cache_identity_ignores_display_but_tracks_workspace_revision_and_imports() {
    let cache = SourceCache::default();
    let cancel = Cancellation::default();
    let mut ctx = Context {
        files: [("data/a.csv".into(), "x\n1\n".into())].into(),
        ..Context::default()
    };
    let (first, revision) = cache.load(source("data/a.csv"), &ctx, &cancel).unwrap();
    let mut displayed = source("data/a.csv");
    displayed.spec.display = Some(crate::spec::SourceDisplay::Text);
    let (same, same_revision) = cache.load(displayed, &ctx, &cancel).unwrap();
    assert!(Arc::ptr_eq(&first, &same));
    assert_eq!(revision, same_revision);
    for change in 0..3 {
        match change {
            0 => ctx.workspace = "new workspace".into(),
            1 => ctx.revision = "next".into(),
            _ => {
                ctx.files.insert("data/a.csv".into(), "x\n2\n".into());
            }
        }
        let (next, next_revision) = cache.load(source("data/a.csv"), &ctx, &cancel).unwrap();
        assert!(!Arc::ptr_eq(&first, &next));
        assert_ne!(revision, next_revision);
    }
    cancel.cancel();
    assert!(cache.load(source("data/a.csv"), &ctx, &cancel).is_err());
}
#[test]
fn local_sources_do_not_expire_and_capacity_evicts_completed_entries() {
    let cache = SourceCache::default();
    let cancel = Cancellation::default();
    let mut ctx = Context::default();
    ctx.files.insert("data/0.csv".into(), "x\n0\n".into());
    let (first, _) = cache.load(source("data/0.csv"), &ctx, &cancel).unwrap();
    for entry in cache.entries.lock().unwrap().values() {
        entry.state.lock().unwrap().output.as_mut().unwrap().0 =
            Instant::now() - Duration::from_secs(120);
    }
    let (same, _) = cache.load(source("data/0.csv"), &ctx, &cancel).unwrap();
    assert!(Arc::ptr_eq(&first, &same));
    for i in 1..33 {
        let file = format!("data/{i}.csv");
        ctx.files.insert(file.clone(), format!("x\n{i}\n"));
        cache.load(source(&file), &ctx, &cancel).unwrap();
    }
    assert_eq!(cache.entries.lock().unwrap().len(), 1);
    let (fresh, _) = cache.load(source("data/0.csv"), &ctx, &cancel).unwrap();
    assert!(!Arc::ptr_eq(&first, &fresh));
}
#[test]
fn remote_ttl_refreshes_the_snapshot_and_failure_is_not_cached() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("data")).unwrap();
    let path = root.path().join("data/test.sqlite");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch("CREATE TABLE input (x INTEGER);INSERT INTO input VALUES (1)")
        .unwrap();
    let source: Resolved = serde_json::from_value(
        json!({"spec":{"source":"sqlite","file_path":"test.sqlite","sql":"SELECT x FROM input"}}),
    )
    .unwrap();
    let ctx = Context {
        directory: root.path().to_str().unwrap().into(),
        ..Context::default()
    };
    let cache = SourceCache::default();
    let cancel = Cancellation::default();
    let (_, before) = cache.load(source.clone(), &ctx, &cancel).unwrap();
    db.execute_batch("UPDATE input SET x=2").unwrap();
    for entry in cache.entries.lock().unwrap().values() {
        entry.state.lock().unwrap().output.as_mut().unwrap().0 =
            Instant::now() - Duration::from_secs(60);
    }
    let (next, after) = cache.load(source, &ctx, &cancel).unwrap();
    assert_ne!(before, after);
    assert_eq!(next.dataframe(&cancel).unwrap().rows[0][0], 2);
    let missing = source_for_missing();
    assert!(cache.load(missing.clone(), &ctx, &cancel).is_err());
    std::fs::write(root.path().join("data/missing.csv"), "x\n3\n").unwrap();
    assert!(cache.load(missing, &ctx, &cancel).is_ok());
}
fn source_for_missing() -> Resolved {
    source("data/missing.csv")
}

#[test]
fn upstream_ttl_refresh_invalidates_a_still_fresh_downstream() {
    let server = super::super::integration_tests::Http::new();
    let parent = server.source();
    let source: Resolved = serde_json::from_value(json!({"spec":{"source":"http","url":format!("{}/child",parent.spec.provider["url"].as_str().unwrap()),"depends_on":["parent"]},"upstream":{"parent":parent}})).unwrap();
    let cache = SourceCache::default();
    let cancel = Cancellation::default();
    let ctx = server.context();
    let (_, first) = cache.load(source.clone(), &ctx, &cancel).unwrap();
    // Expire only the ancestor (created first), leaving its child fresh.
    for entry in cache.entries.lock().unwrap().values() {
        if entry.revision == 1 {
            entry.state.lock().unwrap().output.as_mut().unwrap().0 =
                Instant::now() - Duration::from_secs(60);
        }
    }
    let (_, second) = cache.load(source, &ctx, &cancel).unwrap();
    assert_ne!(first, second);
    assert_eq!(server.requests().len(), 4);
}

impl SourceCache {
    pub(crate) fn wait_for_waiters(&self, count: usize) {
        let start = Instant::now();
        loop {
            if self
                .entries
                .lock()
                .unwrap()
                .values()
                .any(|e| e.state.lock().unwrap().waiters >= count)
            {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "waiters did not register"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}
