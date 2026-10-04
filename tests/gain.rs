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
        .stdout(predicate::str::contains("tokens saved:"))
        .stdout(predicate::str::contains("hit rate:"));
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
        .stdout(predicate::str::contains("hit rate: 0.0%"));
}
