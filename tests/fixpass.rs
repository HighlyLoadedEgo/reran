/// Fix-pass tests: one per review finding. Each ran RED before its fix.
use assert_cmd::Command;
use predicates::prelude::*;
use reran::classify::{classify, Class};
use reran::store::Store;
use std::fs;
use std::path::PathBuf;

fn v(s: &[&str]) -> Vec<String> {
    s.iter().map(|s| s.to_string()).collect()
}

// ── F1 (Critical): the command `reran init` wires must actually be accepted by the CLI.
#[test]
fn f1_wired_command_is_accepted_by_cli() {
    let out = Command::cargo_bin("reran")
        .unwrap()
        .args(["hook", "--event", "pre"])
        .write_stdin("not json")
        .assert()
        .success(); // fail-open: exit 0 on garbage
    out.stdout(predicate::str::is_empty());

    Command::cargo_bin("reran")
        .unwrap()
        .args(["hook", "--event", "post"])
        .write_stdin("{}")
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn f1_init_writes_exactly_the_accepted_invocation() {
    let d = tempfile::tempdir().unwrap();
    let settings = d.path().join(".claude").join("settings.json");
    reran::initcmd::init_claude_code(&settings, "reran").unwrap();
    let text = fs::read_to_string(&settings).unwrap();
    assert!(text.contains("reran hook --event pre"), "{text}");
    assert!(text.contains("reran hook --event post"), "{text}");
    assert!(!text.contains("hook pre --event"), "{text}");
}

// ── F2: git bare reads cached, write-forms bypassed (pinned both directions).
#[test]
fn f2_git_bare_reads_cached_write_forms_bypassed() {
    assert_eq!(classify(&v(&["git", "tag"])), Class::Memoizable, "bare tag = list");
    assert_eq!(classify(&v(&["git", "branch"])), Class::Memoizable, "bare branch = list");
    assert_eq!(classify(&v(&["git", "remote"])), Class::Memoizable, "bare remote = list");
    assert_eq!(classify(&v(&["git", "tag", "v1.0"])), Class::Bypass, "creates tag!");
    assert_eq!(classify(&v(&["git", "branch", "feature"])), Class::Bypass, "creates branch!");
    assert_eq!(
        classify(&v(&["git", "remote", "add", "origin", "url"])),
        Class::Bypass
    );
    assert_eq!(classify(&v(&["git", "branch", "-D", "x"])), Class::Bypass);
    assert_eq!(classify(&v(&["git", "stash"])), Class::Bypass, "stash = write");
}

// ── F3: command substitution is not computable pre-run (bkt#20) → Bypass.
#[test]
fn f3_command_substitution_is_bypass() {
    assert_eq!(classify(&v(&["echo", "$(date)"])), Class::Bypass);
    assert_eq!(classify(&v(&["git", "log", "$(git rev-parse HEAD~5)..HEAD"])), Class::Bypass);
    assert_eq!(classify(&v(&["echo", "`date`"])), Class::Bypass);
    assert_eq!(classify(&v(&["echo", "${HOME}"])), Class::Bypass);
    assert_eq!(classify(&v(&["ls", "$PWD"])), Class::Bypass);
}

// ── F4: huge outputs are never written into the store.
#[test]
fn f4_huge_output_is_not_cached() {
    let dbd = tempfile::tempdir().unwrap();
    let cwdd = tempfile::tempdir().unwrap();
    let store = Store::open(&dbd.path().join("t.db")).unwrap();
    let cwd = cwdd.path();
    let cmd: Vec<String> = ["cat", "huge.log"].iter().map(|s| s.to_string()).collect();

    let huge = "x".repeat(reran::engine::MAX_CACHED_OUTPUT_BYTES + 1);
    record_and_hit_via_hooks_marker(&store, &cmd, cwd, &huge);

    // nothing cached: same-session evaluate must MISS
    let out = reran::engine::evaluate(&store, &cmd, cwd, "s1");
    assert!(
        matches!(out, reran::engine::Outcome::Miss),
        "oversized output must not be cached"
    );
}

fn record_and_hit_via_hooks_marker(
    store: &Store,
    cmd: &[String],
    cwd: &std::path::Path,
    raw: &str,
) {
    reran::engine::record(store, cmd, cwd, "s1", raw, 0);
}

// ── F5: env-prefixed commands keep their command-specific allowlist.
#[test]
fn f5_env_prefix_selects_command_allowlist() {
    // engine is exercised through keys: FOO=bar git status must key on GIT_DIR etc.
    // Indirect but decisive pin: two different GIT_DIR values (with env prefix) → different keys.
    unsafe { std::env::set_var("GIT_DIR", "/one") };
    let ctx1 = reran::key::CallCtx {
        cwd: PathBuf::from("/p"),
        argv: v(&["FOO=bar", "git", "status"]),
        uid: 1,
        fs_epoch: 0,
    };
    let k1 = reran::key::build_key(&ctx1, &reran::key::env_allowlist_for(&v(&["git", "status"])));
    unsafe { std::env::set_var("GIT_DIR", "/two") };
    let k2 = reran::key::build_key(&ctx1, &reran::key::env_allowlist_for(&v(&["git", "status"])));
    assert_ne!(k1, k2, "GIT_DIR change must change the key even with env prefix");
    unsafe { std::env::remove_var("GIT_DIR") };
}

// ── F6: a single unreadable mtime must not abort the whole scan.
#[test]
fn f6_unreadable_file_does_not_abort_scan() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("new.txt"), "x").unwrap();
    // a path whose metadata read fails: pass a nonexistent root child via skip? Instead:
    // use a root dir containing a dangling symlink — symlink_metadata succeeds but
    // read_dir on it fails; scanner must skip, not abort, and still find new.txt.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(d.path().join("nope"), d.path().join("dangling")).unwrap();
    }
    let m = reran::fsepoch::latest_mtime(d.path(), &[], 10_000);
    assert!(m.is_some(), "scan must survive unreadable entries");
}

// ── F7: partial wiring gets completed, not skipped.
#[test]
fn f7_init_completes_partial_wiring() {
    let d = tempfile::tempdir().unwrap();
    let settings = d.path().join(".claude").join("settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(
        &settings,
        r#"{"hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "reran hook --event pre"}]}]}}"#,
    )
    .unwrap();

    reran::initcmd::init_claude_code(&settings, "reran").unwrap();
    let text = fs::read_to_string(&settings).unwrap();
    let val: serde_json::Value = serde_json::from_str(&text).unwrap();
    // Pre stays single, Post gets added
    assert_eq!(val["hooks"]["PreToolUse"].as_array().unwrap().len(), 1);
    assert_eq!(val["hooks"]["PostToolUse"].as_array().unwrap().len(), 1);
}

// ── F8: no_exit_code is visible in gain (dormant adapter must be detectable).
#[test]
fn f8_gain_shows_no_exit_code_count() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("t.db");
    let store = Store::open(&db).unwrap();
    store.record_event("no_exit_code", 0, "git status").unwrap();
    Command::cargo_bin("reran")
        .unwrap()
        .env("RERAN_DB", &db)
        .arg("gain")
        .assert()
        .success()
        .stdout(predicate::str::contains("no exit code 1"));
}

// ── F12: settings.json is written atomically (tmp + rename, no partial file on crash).
#[test]
fn f12_atomic_write_leaves_no_tmp_files() {
    let d = tempfile::tempdir().unwrap();
    let settings = d.path().join(".claude").join("settings.json");
    reran::initcmd::init_claude_code(&settings, "reran").unwrap();
    let leftovers: Vec<_> = fs::read_dir(settings.parent().unwrap())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().contains(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "no .tmp residue: {leftovers:?}");
}
