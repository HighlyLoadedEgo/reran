use reran::engine::{evaluate, record, Outcome};
use reran::store::Store;
use std::path::PathBuf;
use std::sync::Arc;

/// db lives in its OWN tempdir, separate from the scanned cwd — mirrors production
/// (~/.cache vs project dir): a db inside the scanned tree would churn fs_epoch.
fn setup() -> (tempfile::TempDir, tempfile::TempDir, PathBuf) {
    let db_dir = tempfile::tempdir().unwrap();
    let cwd_dir = tempfile::tempdir().unwrap();
    let db = db_dir.path().join("test.db");
    (db_dir, cwd_dir, db)
}

fn argv(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn same_session_hit_after_record() {
    let (_dbd, cwdd, db) = setup();
    let store = Store::open(&db).unwrap();
    let session = "s1";
    let cwd = cwdd.path().to_path_buf();
    let cmd = argv(&["git", "status"]);

    assert!(matches!(evaluate(&store, &cmd, &cwd, session), Outcome::Miss));

    record(&store, &cmd, &cwd, session, "On branch main\nnothing to commit", 0);

    match evaluate(&store, &cmd, &cwd, session) {
        Outcome::Hit { digest } => assert!(digest.contains("unchanged since turn"), "{digest}"),
        other => panic!("expected Hit, got {other:?}"),
    }
}

#[test]
fn other_session_never_gets_unchanged() {
    let (_dbd, cwdd, db) = setup();
    let store = Store::open(&db).unwrap();
    let cwd = cwdd.path().to_path_buf();
    let cmd = argv(&["git", "status"]);

    record(&store, &cmd, &cwd, "s1", "On branch main", 0);

    // THE trust test (cachebro#11/#8): a session that never saw the output runs fresh.
    assert!(matches!(
        evaluate(&store, &cmd, &cwd, "s2-never-saw-it"),
        Outcome::Miss
    ));
}

#[test]
fn write_command_bumps_epoch_forcing_miss() {
    let (_dbd, cwdd, db) = setup();
    let store = Store::open(&db).unwrap();
    let cwd = cwdd.path().to_path_buf();
    std::fs::write(cwd.join("tracked.txt"), "v1").unwrap();
    let cmd = argv(&["git", "status"]);

    record(&store, &cmd, &cwd, "s1", "clean", 0);
    assert!(matches!(evaluate(&store, &cmd, &cwd, "s1"), Outcome::Hit { .. }));

    // a write-classified command (bypass) touches the fs marker
    let write = argv(&["touch", "tracked.txt"]);
    assert!(matches!(evaluate(&store, &write, &cwd, "s1"), Outcome::Bypass));
    record(&store, &write, &cwd, "s1", "", 0);

    assert!(
        matches!(evaluate(&store, &cmd, &cwd, "s1"), Outcome::Miss),
        "fs epoch bumped by write ⇒ no stale hit"
    );
}

#[test]
fn failed_command_is_never_cached() {
    let (_dbd, cwdd, db) = setup();
    let store = Store::open(&db).unwrap();
    let session = "s1";
    let cwd = cwdd.path().to_path_buf();
    let cmd = argv(&["git", "status"]);

    record(&store, &cmd, &cwd, session, "fatal: not a git repo", 128);
    assert!(
        matches!(evaluate(&store, &cmd, &cwd, session), Outcome::Miss),
        "failures are never served from cache"
    );
    let s = store.stats().unwrap();
    assert!(s.uncached_failures >= 1);
}

#[test]
fn concurrent_evaluate_record_succeeds() {
    let (_dbd, cwdd, db) = setup();
    let cwd = Arc::new(cwdd.path().to_path_buf());
    let cmd: Arc<Vec<String>> = Arc::new(argv(&["git", "log", "-1"]));

    let handles: Vec<_> = (0..2)
        .map(|i| {
            let db = db.clone();
            let cmd = cmd.clone();
            let cwd = cwd.clone();
            std::thread::spawn(move || {
                let store = Store::open(&db).unwrap();
                let session = format!("s{i}");
                let _ = evaluate(&store, &cmd, &cwd, &session);
                record(&store, &cmd, &cwd, &session, "commit abc", 0);
                store.stats().unwrap();
            })
        })
        .collect();
    for h in handles {
        h.join().expect("thread must not panic");
    }
}
