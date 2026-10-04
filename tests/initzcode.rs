//! `reran init zcode` — wires Bash hooks into ~/.zcode/cli/config.json.
//! ZCode shape: {"hooks": {"enabled": true, "events": {"PreToolUse": [...], ...}}}
use reran::initcmd::init_zcode;
use std::fs;
use tempfile::tempdir;

fn read(p: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn fresh_config_gets_enabled_and_both_hooks() {
    let d = tempdir().unwrap();
    let cfg = d.path().join("config.json");
    init_zcode(&cfg, "reran").unwrap();

    let v = read(&cfg);
    assert_eq!(v["hooks"]["enabled"], true, "config hooks are disabled by default");
    assert_eq!(v["hooks"]["events"]["PreToolUse"][0]["matcher"], "Bash");
    assert_eq!(
        v["hooks"]["events"]["PreToolUse"][0]["hooks"][0]["command"],
        "reran hook --event pre"
    );
    assert_eq!(
        v["hooks"]["events"]["PostToolUse"][0]["hooks"][0]["command"],
        "reran hook --event post"
    );
}

#[test]
fn existing_config_preserved_and_enabled_flipped() {
    let d = tempdir().unwrap();
    let cfg = d.path().join("config.json");
    fs::write(
        &cfg,
        r#"{"hooks": {"enabled": false, "events": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "/usr/local/bin/clef-guard.sh"}]}]}}, "theme": "dark"}"#,
    )
    .unwrap();

    init_zcode(&cfg, "reran").unwrap();
    let v = read(&cfg);
    assert_eq!(v["theme"], "dark");
    assert_eq!(v["hooks"]["enabled"], true, "must flip enabled for hooks to fire");
    // pre-existing guard untouched, reran appended after it
    let pre = v["hooks"]["events"]["PreToolUse"].as_array().unwrap();
    assert_eq!(pre.len(), 2);
    assert_eq!(pre[0]["hooks"][0]["command"], "/usr/local/bin/clef-guard.sh");
    assert!(pre[1]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .contains("reran hook"));
}

#[test]
fn partial_wiring_completed_and_rerun_is_noop() {
    let d = tempdir().unwrap();
    let cfg = d.path().join("config.json");
    fs::write(
        &cfg,
        r#"{"hooks": {"enabled": true, "events": {"PostToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "reran hook --event post"}]}]}}}"#,
    )
    .unwrap();

    init_zcode(&cfg, "reran").unwrap();
    let v = read(&cfg);
    assert_eq!(v["hooks"]["events"]["PostToolUse"].as_array().unwrap().len(), 1);
    assert_eq!(v["hooks"]["events"]["PreToolUse"].as_array().unwrap().len(), 1);

    let before = fs::read_to_string(&cfg).unwrap();
    init_zcode(&cfg, "reran").unwrap();
    assert_eq!(before, fs::read_to_string(&cfg).unwrap(), "idempotent");
}
