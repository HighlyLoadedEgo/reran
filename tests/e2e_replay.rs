//! E2E trust-contract replay: the five invariants a person using reran depends on.
//! If a step fails here, the owning task's code is wrong — fix there, not in this file.
use reran::engine::{evaluate, record, Outcome};
use reran::store::Store;

#[test]
fn trust_contract_replay() {
    let dbd = tempfile::tempdir().unwrap();
    let cwdd = tempfile::tempdir().unwrap();
    let db = dbd.path().join("e2e.db");
    let store = Store::open(&db).unwrap();
    let cwd = cwdd.path().to_path_buf();
    let cmd: Vec<String> = ["git", "status"].iter().map(|s| s.to_string()).collect();

    // 1. s1 records and then hits its own output (long output: savings must be real)
    let long_output: String = (0..100)
        .map(|i| format!(" M file_{i}.rs\n"))
        .collect();
    record(&store, &cmd, &cwd, "s1", &long_output, 0);
    match evaluate(&store, &cmd, &cwd, "s1") {
        Outcome::Hit { digest } => assert!(digest.contains("unchanged since turn"), "{digest}"),
        other => panic!("step 1: expected Hit, got {other:?}"),
    }

    // 2. a session that never saw it runs fresh — never a bare "unchanged"
    assert!(
        matches!(evaluate(&store, &cmd, &cwd, "s2-fresh"), Outcome::Miss),
        "step 2: unseen session must MISS (cachebro#11 invariant)"
    );

    // 3. a write-classified command bumps the fs epoch ⇒ forced miss
    let write: Vec<String> = ["git", "add", "."].iter().map(|s| s.to_string()).collect();
    assert!(matches!(evaluate(&store, &write, &cwd, "s1"), Outcome::Bypass));
    record(&store, &write, &cwd, "s1", "", 0);
    assert!(
        matches!(evaluate(&store, &cmd, &cwd, "s1"), Outcome::Miss),
        "step 3: write must invalidate"
    );

    // 4. a Memoizable command failing with exit 128 is never cached
    record(&store, &cmd, &cwd, "s1", "fatal: not a git repository", 128);
    assert!(
        matches!(evaluate(&store, &cmd, &cwd, "s1"), Outcome::Miss),
        "step 4: failures are never served from cache"
    );

    // 5. honest stats: failure counted, savings only from the earlier real hit
    let stats = store.stats().unwrap();
    assert!(stats.uncached_failures >= 1, "failure must be counted");
    assert!(stats.tokens_saved > 0, "the real hit must show savings");

    // 6. hit wording never carries success vocabulary (false-green tripwire)
    record(&store, &cmd, &cwd, "s3", "all good", 0);
    if let Outcome::Hit { digest } = evaluate(&store, &cmd, &cwd, "s3") {
        assert!(!digest.contains("passed"), "{digest}");
        assert!(!digest.contains(" ok"));
    } else {
        panic!("step 6: expected Hit for same-session record");
    }
}
