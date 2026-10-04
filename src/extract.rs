//! Grammar digests for dynamic command output (spec §4.5, M2).
//!
//! Every digest ends with an explicit `exit N` marker. Non-zero exits are
//! prefixed `FAILED (exit N)` — a failure is never phrased as success
//! (rtk#4421 false-green class is THE bug class this module must not reproduce).

use crate::digest::elide;

const MAX_DIGEST: usize = 1_500;

/// Build a digest for raw output. Always returns something: a grammar summary
/// when the command matches one, otherwise head+tail elision.
pub fn extract(argv: &[String], raw: &str, exit_code: i32) -> String {
    let body = grammar_digest(argv, raw).unwrap_or_else(|| generic_digest(raw));
    tag_exit(body, exit_code)
}

/// Entry point used by the engine on hits: same as extract but for stored raw.
/// (Kept separate so a future raw-less entry can still digest.)
pub fn digest_raw(argv: &[String], raw: &str, exit_code: i32) -> String {
    extract(argv, raw, exit_code)
}

fn tag_exit(body: String, exit_code: i32) -> String {
    let mut d = body;
    if d.len() > MAX_DIGEST {
        d.truncate(MAX_DIGEST);
        d.push_str(" …[digest truncated by reran]");
    }
    if exit_code == 0 {
        d.push_str(" · exit 0");
    } else {
        d = format!("FAILED (exit {exit_code}): {d}");
    }
    d
}

fn first_command_word(argv: &[String]) -> &str {
    argv.first().map(String::as_str).unwrap_or("")
}

fn grammar_digest(argv: &[String], raw: &str) -> Option<String> {
    let cmd = first_command_word(argv);
    match cmd {
        "pytest" => Some(pytest(raw)),
        "cargo" if argv.get(1).map(String::as_str) == Some("test") => Some(cargo_test(raw)),
        "go" if argv.get(1).map(String::as_str) == Some("test") => Some(go_test(raw)),
        "vitest" | "jest" => Some(vitest(raw)),
        "tsc" => Some(tsc(raw)),
        "curl" => Some(curl(raw)),
        _ => None,
    }
}

fn lines(raw: &str) -> std::str::Lines<'_> {
    raw.lines()
}

fn pytest(raw: &str) -> String {
    let mut summary = String::new();
    let mut failed: Vec<&str> = Vec::new();
    for l in lines(raw) {
        let t = l.trim();
        if (t.starts_with('=') || t.starts_with("==")) && t.contains("passed")
            || t.contains("no tests ran")
        {
            summary = t.trim_matches('=').trim().to_string();
        }
        if let Some(name) = t.strip_prefix("FAILED ") {
            if failed.len() < 5 {
                failed.push(name.split_whitespace().next().unwrap_or(name));
            }
        }
    }
    if summary.is_empty() && failed.is_empty() {
        // collection error / import error: no summary line exists — never invent green
        let first_err = lines(raw)
            .find(|l| l.contains("Error") || l.contains("error"))
            .unwrap_or("no summary line");
        return format!("pytest: error — {first_err}");
    }
    let mut d = format!("pytest: {summary}");
    if !failed.is_empty() {
        d.push_str("; failed: ");
        d.push_str(&failed.join(", "));
    }
    d
}

fn cargo_test(raw: &str) -> String {
    let mut results: Vec<String> = Vec::new();
    let mut failed: Vec<&str> = Vec::new();
    for l in lines(raw) {
        let t = l.trim();
        if t.starts_with("test result:") {
            results.push(t.trim_start_matches("test result:").trim().to_string());
        }
        if t.ends_with("... FAILED") {
            if failed.len() < 5 {
                failed.push(t.trim_end_matches("... FAILED").trim());
            }
        }
    }
    if results.is_empty() {
        return generic_digest(raw);
    }
    let total_passed: u64 = results.iter().filter_map(|r| grab(r, "passed")).sum();
    let total_failed: u64 = results.iter().filter_map(|r| grab(r, "failed")).sum();
    let mut d = format!("cargo test: {total_passed} passed, {total_failed} failed");
    if !failed.is_empty() {
        d.push_str("; failed: ");
        d.push_str(&failed.join(", "));
    }
    d
}

fn grab(s: &str, word: &str) -> Option<u64> {
    let idx = s.find(word)?;
    let num: String = s[..idx]
        .trim_end()
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    num.parse().ok()
}

fn go_test(raw: &str) -> String {
    let mut ok_pkgs: u64 = 0;
    let mut failed: Vec<&str> = Vec::new();
    let mut fail_pkgs: u64 = 0;
    for l in lines(raw) {
        let t = l.trim();
        if t.starts_with("ok\t") || t.starts_with("ok  ") {
            ok_pkgs += 1;
        }
        if t == "FAIL" {
            continue;
        }
        if t.starts_with("FAIL\t") || t.starts_with("FAIL ") {
            fail_pkgs += 1;
        }
        if let Some(name) = t.strip_prefix("--- FAIL: ") {
            if failed.len() < 5 {
                failed.push(name.split_whitespace().next().unwrap_or(name));
            }
        }
    }
    if ok_pkgs == 0 && fail_pkgs == 0 {
        return generic_digest(raw);
    }
    let mut d = format!("go test: {ok_pkgs} ok, {fail_pkgs} failed pkgs");
    if !failed.is_empty() {
        d.push_str("; failed tests: ");
        d.push_str(&failed.join(", "));
    }
    d
}

fn vitest(raw: &str) -> String {
    for l in lines(raw) {
        let t = l.trim();
        if t.starts_with("Tests") && (t.contains("passed") || t.contains("failed")) {
            return format!("vitest: {}", t);
        }
    }
    generic_digest(raw)
}

fn tsc(raw: &str) -> String {
    let err_lines: Vec<&str> = lines(raw).filter(|l| l.contains("error TS")).collect();
    let n = err_lines.len();
    if n == 0 {
        // "Found 0 errors" style success or clean compile
        if let Some(l) = lines(raw).find(|l| l.contains("error")) {
            return format!("tsc: {l}");
        }
        return "tsc: clean (no errors)".to_string();
    }
    let total = lines(raw)
        .rev()
        .find(|l| l.starts_with("Found ") && l.contains("errors"))
        .map(|l| l.to_string())
        .unwrap_or_else(|| format!("Found {n} errors"));
    let first: Vec<&str> = err_lines.iter().take(3).copied().collect();
    format!("tsc: {total} · first: {}", first.join(" | "))
}

fn curl(raw: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(raw.trim()) {
        Ok(v) => match v {
            serde_json::Value::Object(map) => {
                let keys: Vec<String> = map
                    .keys()
                    .take(8)
                    .map(|k| {
                        let t = match map[k] {
                            serde_json::Value::Null => "null".into(),
                            serde_json::Value::Bool(_) => "bool".into(),
                            serde_json::Value::Number(_) => "num".into(),
                            serde_json::Value::String(ref s) => {
                                format!("str \"{}\"", &s.chars().take(24).collect::<String>())
                            }
                            serde_json::Value::Array(ref a) => format!("[{}]", a.len()),
                            serde_json::Value::Object(_) => "{}".into(),
                        };
                        format!("{k}: {t}")
                    })
                    .collect();
                let more = if map.len() > 8 {
                    format!(" +{} more", map.len() - 8)
                } else {
                    String::new()
                };
                format!("JSON object ({} keys): {}{}", map.len(), keys.join(", "), more)
            }
            serde_json::Value::Array(a) => format!("JSON array ({} items)", a.len()),
            _ => generic_digest(raw),
        },
        Err(_) => generic_digest(raw),
    }
}

fn generic_digest(raw: &str) -> String {
    let n = raw.lines().count();
    if n <= crate::digest::HEAD_LINES + crate::digest::TAIL_LINES {
        return raw.trim_end().to_string();
    }
    format!(
        "({n} lines) {}",
        elide(raw, crate::digest::HEAD_LINES, crate::digest::TAIL_LINES)
    )
}
