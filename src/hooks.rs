use crate::engine::{evaluate, record, Outcome};
use crate::store::Store;
use std::panic::AssertUnwindSafe;
use std::path::Path;

#[derive(serde::Deserialize)]
struct PrePayload {
    session_id: String,
    tool_input: ToolInput,
    #[serde(default, alias = "toolCallId", alias = "tool_call_id")]
    tool_call_id: Option<String>,
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
    #[serde(default, alias = "toolCallId", alias = "tool_call_id")]
    tool_call_id: Option<String>,
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

/// A rewrite hook (e.g. rtk-bridge, a plugin registered after config hooks)
/// may replace `git status` with `rtk git status` before execution, so pre
/// can see the original while post sees the wrapped form — or both orders.
/// The wrapper prefix is never part of the cache key; bare `rtk` stays whole
/// (unknown command ⇒ bypass semantics).
fn unwrap_rewrite_hook(argv: Vec<String>) -> Vec<String> {
    if argv.len() > 1 && argv[0] == "rtk" {
        argv[1..].to_vec()
    } else {
        argv
    }
}

/// PreToolUse: cache hit ⇒ deny-with-digest (the cached answer IS the reason).
/// ANY failure mode ⇒ "" (allow the command; fail-open, spec §5.6).
pub fn hook_pre(stdin_json: &str, cwd: &Path, store: Option<&Store>) -> String {
    std::panic::catch_unwind(AssertUnwindSafe(|| -> Option<String> {
        let payload: PrePayload = serde_json::from_str(stdin_json).ok()?;
        let argv = unwrap_rewrite_hook(shlex::split(&payload.tool_input.command)?);
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
        // A rewrite plugin runs after config hooks and swaps the executed
        // command (sometimes changing the verb). pre sees the ORIGINAL argv —
        // pin it to this call so post can cache under what the agent asked for.
        if let Some(id) = payload.tool_call_id.as_deref() {
            if !id.is_empty() {
                let _ = store.record_pending(id, &argv);
            }
        }
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
        let payload_argv = shlex::split(&payload.tool_input.command)?;
        let owned;
        let store = match store {
            Some(s) => s,
            None => {
                owned = Store::open(&Store::default_db_path()).ok()?;
                &owned
            }
        };
        // Original argv wins: the toolCallId join recovers what the agent asked
        // for even when a rewrite hook changed the verb; the prefix strip only
        // covers the simple `rtk <same-verb>` form (payload without a call id).
        let argv = match payload.tool_call_id.as_deref().filter(|id| !id.is_empty()) {
            Some(id) => store.take_pending(id).ok().flatten().unwrap_or_else(|| unwrap_rewrite_hook(payload_argv)),
            None => unwrap_rewrite_hook(payload_argv),
        };
        if argv.is_empty() {
            return Some(String::new());
        }
        let output = payload
            .tool_response
            .get("stdout")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let label = argv.join(" ");
        if !clearly_completed(&payload.tool_response) {
            // cancelled / timedOut / status != completed: even exitCode 0 here is
            // NOT a success — caching it would serve interrupted work as done.
            let _ = store.record_event("not_completed", 0, &label);
            return Some(String::new());
        }
        match confirmable_exit(&payload.tool_response) {
            Some(0) => record(store, &argv, cwd, &payload.session_id, output, 0),
            Some(_) => {
                let _ = store.record_event("uncached_failure", 0, &label); // never cached
            }
            None => {
                let _ = store.record_event("no_exit_code", 0, &label);
            }
        }
        Some(String::new())
    }))
    .unwrap_or(None)
    .unwrap_or_default()
}

/// Trust gate: the run must be unambiguously completed. Any cancellation,
/// timeout, or non-completed status → false, regardless of exit code.
fn clearly_completed(tool_response: &serde_json::Value) -> bool {
    for field in ["cancelled", "timedOut", "timed_out", "interrupted", "isInterrupt"] {
        if tool_response.get(field).and_then(|v| v.as_bool()) == Some(true) {
            return false;
        }
    }
    match tool_response.get("status") {
        Some(serde_json::Value::String(s)) => s == "completed",
        None | Some(serde_json::Value::Null) => true, // field absent: no signal
        Some(_) => true,                              // non-string status: not our signal
    }
}

fn confirmable_exit(tool_response: &serde_json::Value) -> Option<i32> {
    // NB: "status" deliberately NOT here — ZCode uses it for the run state string.
    for field in ["exit_code", "exitCode", "code"] {
        if let Some(v) = tool_response.get(field).and_then(|v| v.as_i64()) {
            return Some(v as i32);
        }
    }
    None
}
