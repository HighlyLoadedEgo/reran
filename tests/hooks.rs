use reran::hooks::{hook_post, hook_pre};
use reran::engine::record;
use reran::store::Store;
use std::path::PathBuf;

/// db in its own tempdir, cwd in another (stable fs_epoch under parallel tests,
/// mirroring production where db sits in ~/.cache outside the scanned project).
fn store() -> (tempfile::TempDir, Store, std::path::PathBuf) {
    let d = tempfile::tempdir().unwrap();
    let s = Store::open(&d.path().join("t.db")).unwrap();
    let cwd = tempfile::tempdir().unwrap();
    (d, s, cwd.path().to_path_buf())
}

fn pre_payload(cmd: &str, session: &str) -> String {
    serde_json::json!({
        "session_id": session,
        "tool_name": "Bash",
        "tool_input": { "command": cmd }
    })
    .to_string()
}

#[test]
fn empty_store_allows() {
    let (_d, s, cwd) = store();
    let out = hook_pre(&pre_payload("git status", "s1"), &cwd, Some(&s));
    assert_eq!(out, "", "no entry ⇒ allow (empty output)");
}

#[test]
fn recorded_entry_denies_with_digest() {
    let (_d, s, cwd) = store();
    let cmd: Vec<String> = ["git", "status"].iter().map(|s| s.to_string()).collect();
    record(&s, &cmd, &cwd, "s1", "On branch main", 0);

    let out = hook_pre(&pre_payload("git status", "s1"), &cwd, Some(&s));
    assert!(out.contains("unchanged since turn"), "{out}");
    assert!(out.contains("permissionDecision"));
    assert!(out.contains("deny"));

    // other session: allow
    let out = hook_pre(&pre_payload("git status", "s2"), &cwd, Some(&s));
    assert_eq!(out, "");
}

#[test]
fn garbage_stdin_fails_open() {
    let (_d, s, cwd) = store();
    for garbage in ["not json", "{}", "{\"session_id\":\"x\"}", "[]", ""] {
        let out = hook_pre(garbage, &cwd, Some(&s));
        assert_eq!(out, "", "garbage {garbage:?} must allow");
        let out = hook_post(garbage, &cwd, Some(&s));
        assert_eq!(out, "");
    }
}

#[test]
fn compound_command_allows() {
    let (_d, s, cwd) = store();
    let out = hook_pre(
        &pre_payload("git status && npm test", "s1"),
        &cwd,
        Some(&s),
    );
    assert_eq!(out, "");
}

#[test]
fn post_without_exit_code_never_caches() {
    let (_d, s, cwd) = store();
    let payload = serde_json::json!({
        "session_id": "s1",
        "tool_name": "Bash",
        "tool_input": { "command": "git status" },
        "tool_response": { "stdout": "On branch main", "stderr": "", "interrupted": false }
    })
    .to_string();
    let out = hook_post(&payload, &cwd, Some(&s));
    assert_eq!(out, "");

    // nothing cached: pre must still allow
    let out = hook_pre(&pre_payload("git status", "s1"), &cwd, Some(&s));
    assert_eq!(out, "", "no confirmable exit ⇒ never cached");
    let stats = s.stats().unwrap();
    // miss was counted by pre? no — only post ran. no_exit_code event recorded:
    // stats has no direct field; assert via events table through another miss+hit path is overkill.
    // The real pin: nothing cached, asserted above.
}

#[test]
fn post_with_explicit_exit_zero_caches() {
    let (_d, s, cwd) = store();
    let payload = serde_json::json!({
        "session_id": "s1",
        "tool_name": "Bash",
        "tool_input": { "command": "git status" },
        "tool_response": { "stdout": "On branch main", "stderr": "", "exit_code": 0 }
    })
    .to_string();
    hook_post(&payload, &cwd, Some(&s));

    let out = hook_pre(&pre_payload("git status", "s1"), &cwd, Some(&s));
    assert!(out.contains("unchanged since turn"), "explicit exit 0 must enable caching: {out}");
}

#[test]
fn post_with_nonzero_exit_never_caches() {
    let (_d, s, cwd) = store();
    let payload = serde_json::json!({
        "session_id": "s1",
        "tool_name": "Bash",
        "tool_input": { "command": "git status" },
        "tool_response": { "stdout": "", "stderr": "fatal:", "exit_code": 128 }
    })
    .to_string();
    hook_post(&payload, &cwd, Some(&s));
    let out = hook_pre(&pre_payload("git status", "s1"), &cwd, Some(&s));
    assert_eq!(out, "");
}
