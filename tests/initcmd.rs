use reran::initcmd::init_claude_code;
use std::fs;
use tempfile::tempdir;

fn read(p: &std::path::Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
}

#[test]
fn fresh_settings_get_both_hooks() {
    let d = tempdir().unwrap();
    let settings = d.path().join(".claude").join("settings.json");
    init_claude_code(&settings, "/usr/local/bin/reran").unwrap();

    let v = read(&settings);
    let pre = &v["hooks"]["PreToolUse"];
    assert_eq!(pre[0]["matcher"], "Bash");
    assert_eq!(pre[0]["hooks"][0]["type"], "command");
    assert_eq!(
        pre[0]["hooks"][0]["command"],
        "/usr/local/bin/reran hook --event pre"
    );
    let post = &v["hooks"]["PostToolUse"];
    assert_eq!(
        post[0]["hooks"][0]["command"],
        "/usr/local/bin/reran hook --event post"
    );
}

#[test]
fn existing_settings_preserve_unknown_keys() {
    let d = tempdir().unwrap();
    let settings = d.path().join(".claude").join("settings.json");
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(
        &settings,
        r#"{"permissions": {"allow": ["Bash"]}, "model": "opus"}"#,
    )
    .unwrap();

    init_claude_code(&settings, "reran").unwrap();
    let v = read(&settings);
    assert_eq!(v["permissions"]["allow"][0], "Bash");
    assert_eq!(v["model"], "opus");
    assert!(v["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap()
        .contains("reran hook"));
}

#[test]
fn second_run_is_noop() {
    let d = tempdir().unwrap();
    let settings = d.path().join(".claude").join("settings.json");
    init_claude_code(&settings, "reran").unwrap();
    let before = fs::read_to_string(&settings).unwrap();
    init_claude_code(&settings, "reran").unwrap();
    let after = fs::read_to_string(&settings).unwrap();
    assert_eq!(before, after, "idempotent: byte-identical after re-run");
}
