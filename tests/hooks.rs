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

// ── ZCode live-payload format (probe 2026-10-05): exitCode camelCase +
// trust fields. Cancelled/timedOut/not-completed must NEVER cache even
// when exitCode == 0 — a cancelled command is not a success (spec §5.2).
fn zcode_payload(cmd: &str, session: &str, stdout: &str, exit_code: i64, extra: serde_json::Value) -> String {
    let mut resp = serde_json::json!({
        "stdout": stdout,
        "stderr": "",
        "exitCode": exit_code,
        "status": "completed",
        "cancelled": false,
        "timedOut": false
    });
    if let (Some(dst), Some(src)) = (resp.as_object_mut(), extra.as_object()) {
        for (k, v) in src {
            dst.insert(k.clone(), v.clone());
        }
    }
    serde_json::json!({
        "session_id": session,
        "tool_name": "Bash",
        "tool_input": { "command": cmd },
        "tool_response": resp
    })
    .to_string()
}

#[test]
fn zcode_payload_caches_when_completed() {
    let (_d, s, cwd) = store();
    hook_post(&zcode_payload("git status", "s1", "On branch main", 0, serde_json::json!({})), &cwd, Some(&s));
    let out = hook_pre(&pre_payload("git status", "s1"), &cwd, Some(&s));
    assert!(out.contains("unchanged since turn"), "ZCode exitCode 0 must cache: {out}");
}

#[test]
fn zcode_cancelled_is_never_cached() {
    let (_d, s, cwd) = store();
    hook_post(&zcode_payload("git status", "s1", "partial output", 0, serde_json::json!({"cancelled": true})), &cwd, Some(&s));
    let out = hook_pre(&pre_payload("git status", "s1"), &cwd, Some(&s));
    assert_eq!(out, "", "cancelled with exit 0 must not cache: {out}");
}

#[test]
fn zcode_timed_out_is_never_cached() {
    let (_d, s, cwd) = store();
    hook_post(&zcode_payload("git status", "s1", "", 0, serde_json::json!({"timedOut": true})), &cwd, Some(&s));
    let out = hook_pre(&pre_payload("git status", "s1"), &cwd, Some(&s));
    assert_eq!(out, "", "timedOut must not cache: {out}");
}

#[test]
fn zcode_status_not_completed_is_never_cached() {
    let (_d, s, cwd) = store();
    hook_post(&zcode_payload("git status", "s1", "", 0, serde_json::json!({"status": "failed"})), &cwd, Some(&s));
    let out = hook_pre(&pre_payload("git status", "s1"), &cwd, Some(&s));
    assert_eq!(out, "", "status != completed must not cache: {out}");
}

#[test]
fn zcode_nonzero_exitcode_is_never_cached() {
    let (_d, s, cwd) = store();
    hook_post(&zcode_payload("git status", "s1", "fatal:", 128, serde_json::json!({})), &cwd, Some(&s));
    let out = hook_pre(&pre_payload("git status", "s1"), &cwd, Some(&s));
    assert_eq!(out, "");
}

// ── rtk-bridge coexistence: the plugin rewrites tool_input before execution,
// so post sees `rtk git status` while pre saw `git status` (config hooks run
// before plugin hooks). Both hooks must resolve to the SAME key regardless
// of registration order — the wrapper prefix is never part of the cache key.

#[test]
fn rtk_wrapper_in_post_caches_under_original_command() {
    let (_d, s, cwd) = store();
    hook_post(&zcode_payload("rtk git status", "s1", "On branch main", 0, serde_json::json!({})), &cwd, Some(&s));
    let out = hook_pre(&pre_payload("git status", "s1"), &cwd, Some(&s));
    assert!(out.contains("unchanged since turn"), "post with rtk wrapper must cache under original argv: {out}");
}

#[test]
fn rtk_wrapper_in_pre_hits_original_entry() {
    let (_d, s, cwd) = store();
    hook_post(&zcode_payload("git status", "s1", "On branch main", 0, serde_json::json!({})), &cwd, Some(&s));
    let out = hook_pre(&pre_payload("rtk git status", "s1"), &cwd, Some(&s));
    assert!(out.contains("unchanged since turn"), "pre with rtk wrapper must find entry stored under original argv: {out}");
}

#[test]
fn bare_rtk_wrapped_write_form_is_still_bypass() {
    let (_d, s, cwd) = store();
    // `rtk git commit -m x` unwraps to `git commit …` — write form, never cached.
    hook_post(&zcode_payload("rtk git commit -m x", "s1", "done", 0, serde_json::json!({})), &cwd, Some(&s));
    let out = hook_pre(&pre_payload("git commit -m x", "s1"), &cwd, Some(&s));
    assert_eq!(out, "", "unwrapped write command must stay bypass: {out}");
}
