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

/// True when this event's entries already contain a reran hook (any wiring).
fn event_has_reran(v: &Value, event: &str) -> bool {
    v["hooks"][event]
        .as_array()
        .map_or(false, |entries| {
            entries.iter().any(|e| {
                e["hooks"].as_array().map_or(false, |hooks| {
                    hooks.iter().any(|h| {
                        h["command"].as_str().is_some_and(|c| c.contains(RERAN_HOOK_MARK))
                    })
                })
            })
        })
}

/// Wire reran's Bash hooks into a ZCode config.json. ZCode shape differs from
/// Claude Code: hooks live under `hooks.events.*` and require `hooks.enabled: true`
/// (configuration-file hooks are disabled by default — the #1 silent trap).
pub fn init_zcode(config_path: &Path, reran_bin: &str) -> io::Result<()> {
    let mut root = match std::fs::read_to_string(config_path) {
        Ok(text) => serde_json::from_str::<Value>(&text)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
        Err(_) => Value::Object(serde_json::Map::new()),
    };
    if !root.is_object() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "config.json is not a JSON object"));
    }

    let pre_wired = event_has_reran_events(&root, "PreToolUse");
    let post_wired = event_has_reran_events(&root, "PostToolUse");

    let obj = root.as_object_mut().unwrap();
    let hooks = obj
        .entry("hooks")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let hooks_obj = hooks.as_object_mut().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "\"hooks\" is not an object")
    })?;
    hooks_obj.insert("enabled".into(), Value::Bool(true));

    let events = hooks_obj
        .entry("events")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let events_obj = events.as_object_mut().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "\"events\" is not an object")
    })?;

    for (event, wired, command) in [
        ("PreToolUse", pre_wired, format!("{reran_bin} hook --event pre")),
        ("PostToolUse", post_wired, format!("{reran_bin} hook --event post")),
    ] {
        if wired {
            continue;
        }
        let entries = events_obj
            .entry(event)
            .or_insert_with(|| Value::Array(Vec::new()));
        let arr = entries.as_array_mut().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "event list is not an array")
        })?;
        arr.push(hook_entry(&command));
    }

    let text = serde_json::to_string_pretty(&root)?;
    let tmp = config_path.with_extension("json.reran-tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, config_path)
}

/// ZCode config variant: reran entries live under hooks.events.<Event>.
fn event_has_reran_events(v: &Value, event: &str) -> bool {
    v["hooks"]["events"][event]
        .as_array()
        .map_or(false, |entries| {
            entries.iter().any(|e| {
                e["hooks"].as_array().map_or(false, |hooks| {
                    hooks
                        .iter()
                        .any(|h| h["command"].as_str().is_some_and(|c| c.contains(RERAN_HOOK_MARK)))
                })
            })
        })
}

/// Merge reran's Bash hooks into a Claude Code settings.json. Idempotent:
/// a file already wiring `reran hook` is left byte-identical.
pub fn init_claude_code(settings_path: &Path, reran_bin: &str) -> io::Result<()> {
    let mut root = match std::fs::read_to_string(settings_path) {
        Ok(text) => serde_json::from_str::<Value>(&text)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?,
        Err(_) => Value::Object(serde_json::Map::new()),
    };

    if !root.is_object() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "settings.json is not a JSON object",
        ));
    }

    // decide BEFORE taking mutable borrows (F7: complete partial wiring per-event)
    let pre_wired = event_has_reran(&root, "PreToolUse");
    let post_wired = event_has_reran(&root, "PostToolUse");

    let obj = root.as_object_mut().unwrap();
    let hooks = obj
        .entry("hooks")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let hooks_obj = hooks.as_object_mut().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "\"hooks\" is not an object")
    })?;

    for (event, wired, command) in [
        ("PreToolUse", pre_wired, format!("{reran_bin} hook --event pre")),
        ("PostToolUse", post_wired, format!("{reran_bin} hook --event post")),
    ] {
        if wired {
            continue; // this event already wired — never duplicate
        }
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
    // atomic: a crash mid-write must not corrupt the user's settings (F12)
    let tmp = settings_path.with_extension("json.reran-tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, settings_path)
}
