use crate::engine::{evaluate, record, Outcome};
use crate::store::Store;
use std::panic::AssertUnwindSafe;
use std::path::Path;

#[derive(serde::Deserialize)]
struct PrePayload {
    session_id: String,
    tool_input: ToolInput,
}

#[derive(serde::Deserialize)]
struct ToolInput {
    command: String,
}

#[derive(serde::Deserialize)]
struct PostPayload {
    session_id: String,
    tool_input: ToolInput,
    #[serde(default)]
    tool_response: serde_json::Value,
}

fn deny_json(digest: &str) -> String {
    serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": digest
        }
    })
    .to_string()
}

/// PreToolUse: cache hit ⇒ deny-with-digest (the cached answer IS the reason).
/// ANY failure mode ⇒ "" (allow the command; fail-open, spec §5.6).
pub fn hook_pre(stdin_json: &str, cwd: &Path, store: Option<&Store>) -> String {
    std::panic::catch_unwind(AssertUnwindSafe(|| -> Option<String> {
        let payload: PrePayload = serde_json::from_str(stdin_json).ok()?;
        let argv = shlex::split(&payload.tool_input.command)?;
        if argv.is_empty() {
            return Some(String::new());
        }
        let owned;
        let store = match store {
            Some(s) => s,
            None => {
                owned = Store::open(&Store::default_db_path()).ok()?;
                &owned
            }
        };
        match evaluate(store, &argv, cwd, &payload.session_id) {
            Outcome::Hit { digest } => Some(deny_json(&digest)),
            _ => Some(String::new()),
        }
    }))
    .unwrap_or(None)
    .unwrap_or_default()
}

/// PostToolUse: record the real output. Caching requires a CONFIRMABLE exit code:
/// payloads carrying explicit exit_code use it; {stdout,stderr}-only shapes (Claude
/// Code today) are recorded as no_exit_code and never cached (spec §5.2 — a silent
/// failure must never become a hit; grep exit 1 = "no matches" must reach the agent).
pub fn hook_post(stdin_json: &str, cwd: &Path, store: Option<&Store>) -> String {
    std::panic::catch_unwind(AssertUnwindSafe(|| -> Option<String> {
        let payload: PostPayload = serde_json::from_str(stdin_json).ok()?;
        let argv = shlex::split(&payload.tool_input.command)?;
        if argv.is_empty() {
            return Some(String::new());
        }
        let owned;
        let store = match store {
            Some(s) => s,
            None => {
                owned = Store::open(&Store::default_db_path()).ok()?;
                &owned
            }
        };
        let output = payload
            .tool_response
            .get("stdout")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        match confirmable_exit(&payload.tool_response) {
            Some(0) => record(store, &argv, cwd, &payload.session_id, output, 0),
            Some(_) => {
                let _ = store.record_event("uncached_failure", 0); // never cached
            }
            None => {
                let _ = store.record_event("no_exit_code", 0);
            }
        }
        Some(String::new())
    }))
    .unwrap_or(None)
    .unwrap_or_default()
}

fn confirmable_exit(tool_response: &serde_json::Value) -> Option<i32> {
    for field in ["exit_code", "exitCode", "code", "status"] {
        if let Some(v) = tool_response.get(field).and_then(|v| v.as_i64()) {
            return Some(v as i32);
        }
    }
    None
}
