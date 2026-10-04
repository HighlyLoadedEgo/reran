//! M3: `reran explain` — why did this call hit/miss? (doit#329: 7-year unmet demand)
use reran::classify::{classify, Class};
use reran::engine::explain_line;
use reran::store::Store;
use std::path::PathBuf;

fn setup() -> (tempfile::TempDir, tempfile::TempDir, PathBuf) {
    let dbd = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let db = dbd.path().join("t.db");
    (dbd, cwd, db)
}

#[test]
fn explain_reports_miss_no_entry() {
    let (_d, cwdd, db) = setup();
    let store = Store::open(&db).unwrap();
    let line = explain_line(
        &store,
        &["git".into(), "status".into()],
        cwdd.path(),
        "s1",
    );
    assert!(line.contains("miss"), "{line}");
    assert!(line.contains("no entry"), "{line}");
}

#[test]
fn explain_reports_bypass_with_reason() {
    let (_d, cwdd, db) = setup();
    let store = Store::open(&db).unwrap();
    let line = explain_line(
        &store,
        &["git".into(), "commit".into()],
        cwdd.path(),
        "s1",
    );
    assert!(line.contains("bypass"), "{line}");
    assert!(line.contains("write"), "{line}");
}

#[test]
fn explain_reports_unseen_session_miss_and_hit() {
    let (_d, cwdd, db) = setup();
    let store = Store::open(&db).unwrap();
    let cwd = cwdd.path();
    let cmd: Vec<String> = vec!["git".into(), "status".into()];

    // other session recorded; THIS session never saw it
    reran::engine::record(&store, &cmd, cwd, "other", "out", 0);
    let line = explain_line(&store, &cmd, cwd, "s1");
    assert!(line.contains("miss"), "{line}");
    assert!(line.contains("never saw"), "{line}"); // cachebro#11 reason named

    // after this session runs it once → hit
    reran::engine::record(&store, &cmd, cwd, "s1", "out", 0);
    let line = explain_line(&store, &cmd, cwd, "s1");
    assert!(line.contains("hit"), "{line}");
}

#[test]
fn explain_names_fs_epoch_and_key() {
    let (_d, cwdd, db) = setup();
    let store = Store::open(&db).unwrap();
    let line = explain_line(
        &store,
        &["ls".into()],
        cwdd.path(),
        "s1",
    );
    assert!(line.contains("epoch="), "{line}");
    assert!(line.contains("key="), "{line}");
    assert!(line.contains("memoizable"), "{line}");
    assert!(classify(&["ls".into()]) == Class::Memoizable);
}
