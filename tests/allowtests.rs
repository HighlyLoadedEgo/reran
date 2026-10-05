use reran::classify::Class;
use reran::engine::{evaluate, is_test_command, record, Outcome};
use reran::fsepoch::marker_path;
use reran::store::Store;

fn setup() -> (tempfile::TempDir, Store, std::path::PathBuf) {
    let db_dir = tempfile::tempdir().unwrap();
    let cwd_dir = tempfile::tempdir().unwrap();
    let db = db_dir.path().join("test.db");
    let s = Store::open(&db).unwrap();
    (db_dir, s, cwd_dir.path().to_path_buf())
}

fn allow_flag(store: &Store, cwd: &std::path::Path, on: bool) {
    let path = marker_path(store.cache_dir(), cwd).with_extension("allow-tests");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    if on {
        std::fs::write(&path, b"opt-in").unwrap();
    } else {
        let _ = std::fs::remove_file(&path);
    }
}

#[test]
fn test_commands_are_bypass_by_default() {
    let (_d, s, cwd) = setup();
    assert_eq!(
        evaluate(&s, &["pytest", "-q"].iter().map(|s| s.to_string()).collect::<Vec<_>>(), &cwd, "s1"),
        Outcome::Bypass
    );
}

#[test]
fn opt_in_makes_tests_cachable() {
    let (_d, s, cwd) = setup();
    allow_flag(&s, &cwd, true);
    let argv = ["pytest", "-q"].iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(evaluate(&s, &argv, &cwd, "s1"), Outcome::Miss);
    record(&s, &argv, &cwd, "s1", "43 passed in 1.2s", 0);
    match evaluate(&s, &argv, &cwd, "s1") {
        Outcome::Hit { digest } => assert!(digest.contains("unchanged since turn"), "{digest}"),
        other => panic!("expected hit, got {other:?}"),
    }
    // flag off → immediately bypass again
    allow_flag(&s, &cwd, false);
    assert_eq!(evaluate(&s, &argv, &cwd, "s1"), Outcome::Bypass);
}

#[test]
fn allow_flag_does_not_unlock_non_tests() {
    let (_d, s, cwd) = setup();
    allow_flag(&s, &cwd, true);
    assert_eq!(
        evaluate(&s, &["rm", "-rf", "x"].iter().map(|s| s.to_string()).collect::<Vec<_>>(), &cwd, "s1"),
        Outcome::Bypass
    );
    assert_eq!(
        evaluate(&s, &["sort", "f"].iter().map(|s| s.to_string()).collect::<Vec<_>>(), &cwd, "s1"),
        Outcome::Bypass
    );
}

#[test]
fn test_command_shapes() {
    let v = |s: &[&str]| s.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert!(is_test_command(&v(&["pytest", "-q"])));
    assert!(is_test_command(&v(&["python3", "-m", "pytest", "-q"])));
    assert!(is_test_command(&v(&["uv", "run", "pytest", "tests/"])));
    assert!(is_test_command(&v(&["cargo", "test"])));
    assert!(is_test_command(&v(&["npm", "test"])));
    assert!(is_test_command(&v(&["npx", "vitest", "run"])));
    assert!(!is_test_command(&v(&["uv", "run", "python", "-c", "x"])));
    assert!(!is_test_command(&v(&["cargo", "build"])));
    assert!(!is_test_command(&v(&["grep", "pytest", "f"])));
    // classifier still refuses compounds even when flagged
    assert_eq!(
        reran::classify::classify_ctx(&v(&["pytest", "-q", "&&", "rm", "x"]), None),
        Class::Bypass
    );
}
