use serde_json::Value;
use std::io;
use std::path::Path;

const RERAN_HOOK_MARK: &str = "reran hook";

fn hook_entry(command: &str) -> Value {
    serde_json::json!({
        "matcher": "Bash",
        "hooks": [ { "type": "command", "command": command } ]
    })
}

fn already_wired(v: &Value) -> bool {
    v["hooks"]
        .as_object()
        .map(|events| {
            events.values().any(|entries| {
                entries.as_array().map_or(false, |entries| {
                    entries.iter().any(|e| {
                        e["hooks"].as_array().map_or(false, |hooks| {
                            hooks.iter().any(|h| {
                                h["command"].as_str().is_some_and(|c| c.contains(RERAN_HOOK_MARK))
                            })
                        })
                    })
                })
            })
        })
        .unwrap_or(false)
}

/// Merge reran's Bash hooks into a Claude Code settings.json. Idempotent:
/// a file already wiring `reran hook` is left byte-identical.
pub fn init_claude_code(settings_path: &Path, reran_bin: &str) -> io::Result<()> {
    let mut root = match std::fs::read_to_string(settings_path) {
        Ok(text) => serde_json::from_str::<Value>(&text)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
        Err(_) => Value::Object(serde_json::Map::new()),
    };

    if already_wired(&root) {
        return Ok(());
    }
    if !root.is_object() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "settings.json is not a JSON object",
        ));
    }

    let obj = root.as_object_mut().unwrap();
    let hooks = obj
        .entry("hooks")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let hooks_obj = hooks.as_object_mut().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "\"hooks\" is not an object")
    })?;

    for (event, command) in [
        ("PreToolUse", format!("{reran_bin} hook pre --event pre")),
        ("PostToolUse", format!("{reran_bin} hook post --event post")),
    ] {
        let entries = hooks_obj
            .entry(event)
            .or_insert_with(|| Value::Array(Vec::new()));
        let arr = entries.as_array_mut().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "hook event list is not an array")
        })?;
        arr.push(hook_entry(&command));
    }

    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(&root)?;
    std::fs::write(settings_path, text)
}
