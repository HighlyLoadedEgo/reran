use assert_cmd::Command;
use predicates::prelude::*;
use reran::engine::{evaluate, record, Outcome};
use reran::store::Store;

#[test]
fn gain_prints_honest_stats() {
    // db lives outside the scanned cwd (T7 lesson: db writes must not churn fs_epoch)
    let dbd = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let db = dbd.path().join("t.db");
    let store = Store::open(&db).unwrap();
    let cwd = cwd.path();

    let cmd: Vec<String> = ["git", "status"].iter().map(|s| s.to_string()).collect();
    record(&store, &cmd, cwd, "s1", "On branch main\nstable output long enough to count", 0);

    // the hit: same session, same inputs
    assert!(matches!(evaluate(&store, &cmd, cwd, "s1"), Outcome::Hit { .. }));

    // bypass: evaluate (counts the event) then record (touches the marker)
    let write: Vec<String> = ["touch", "x"].iter().map(|s| s.to_string()).collect();
    assert!(matches!(evaluate(&store, &write, cwd, "s1"), Outcome::Bypass));
    record(&store, &write, cwd, "s1", "", 0);

    // failure: never cached
    record(&store, &cmd, cwd, "s1", "", 1);

    // a miss
    let diff: Vec<String> = ["git", "diff"].iter().map(|s| s.to_string()).collect();
    assert!(matches!(evaluate(&store, &diff, cwd, "s1"), Outcome::Miss));

    Command::cargo_bin("reran")
        .unwrap()
        .env("RERAN_DB", &db)
        .arg("gain")
        .assert()
        .success()
        .stdout(predicate::str::contains("hits 1"))
        .stdout(predicate::str::contains("misses 1"))
        .stdout(predicate::str::contains("bypass 1"))
        .stdout(predicate::str::contains("uncached failures 1"))
        .stdout(predicate::str::contains("Tokens saved:"))
        .stdout(predicate::str::contains("Hit rate:"))
        // rtk-style extras
        .stdout(predicate::str::contains("█"))
        .stdout(predicate::str::contains("By command"))
        .stdout(predicate::str::contains("git status"));
}

#[test]
fn gain_zero_state_no_crash() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("empty.db");
    Store::open(&db).unwrap();
    Command::cargo_bin("reran")
        .unwrap()
        .env("RERAN_DB", &db)
        .arg("gain")
        .assert()
        .success()
        .stdout(predicate::str::contains("hits 0"))
        .stdout(predicate::str::contains("Hit rate:"))
        .stdout(predicate::str::contains("0.0%"));
}

#[test]
fn gain_history_shows_recent_events_with_labels() {
    let dbd = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let db = dbd.path().join("t.db");
    let store = Store::open(&db).unwrap();
    let cwd = cwd.path();

    let cmd: Vec<String> = ["git", "status"].iter().map(|s| s.to_string()).collect();
    record(&store, &cmd, cwd, "s1", "branch state, fairly long output for savings", 0);
    assert!(matches!(evaluate(&store, &cmd, cwd, "s1"), Outcome::Hit { .. }));

    // a miss from the same session so history shows both kinds
    let diff: Vec<String> = ["git", "diff"].iter().map(|s| s.to_string()).collect();
    assert!(matches!(evaluate(&store, &diff, cwd, "s1"), Outcome::Miss));

    Command::cargo_bin("reran")
        .unwrap()
        .env("RERAN_DB", &db)
        .args(["gain", "--history"])
        .assert()
        .success()
        .stdout(predicate::str::contains("git status"))
        .stdout(predicate::str::contains("hit"))
        .stdout(predicate::str::contains("miss"));
}

#[test]
fn event_labels_are_truncated_to_60_chars() {
    let dbd = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    let store = Store::open(&dbd.path().join("t.db")).unwrap();
    let cwd = cwd.path();
    let long_label = format!("cat {}", "a".repeat(200));
    store.record_event("hit", 100, &long_label).unwrap();
    let recent = store.recent_events(5).unwrap();
    assert_eq!(recent.len(), 1);
    assert!(recent[0].2.chars().count() <= 60, "{}", recent[0].2);
}
