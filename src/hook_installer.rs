use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};

/// Location of Claude Code's user settings file.
fn settings_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude").join("settings.json"))
}

/// Current state of the statusLine hook in ~/.claude/settings.json.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallState {
    /// No statusLine entry is present (or the file doesn't exist yet).
    NotConfigured,
    /// statusLine.command is set and references this executable.
    OursInstalled,
    /// statusLine.command is set but points elsewhere.
    OtherInstalled(String),
}

pub fn status() -> InstallState {
    let Some(path) = settings_path() else {
        return InstallState::NotConfigured;
    };
    let Ok(content) = fs::read_to_string(&path) else {
        return InstallState::NotConfigured;
    };
    let Ok(value) = serde_json::from_str::<Value>(&content) else {
        return InstallState::NotConfigured;
    };
    classify(&value, &current_exe_path_lowercase())
}

fn classify(value: &Value, our_exe_lower: &str) -> InstallState {
    let Some(command) = value
        .get("statusLine")
        .and_then(|s| s.get("command"))
        .and_then(|c| c.as_str())
    else {
        return InstallState::NotConfigured;
    };
    if command_references_exe(command, our_exe_lower) {
        InstallState::OursInstalled
    } else {
        InstallState::OtherInstalled(command.to_string())
    }
}

fn command_references_exe(command: &str, our_exe_lower: &str) -> bool {
    if our_exe_lower.is_empty() {
        return false;
    }
    command.to_ascii_lowercase().contains(our_exe_lower)
}

fn current_exe_path_lowercase() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// Build the command line we install into `statusLine.command`. The absolute exe
/// path is wrapped in double quotes so paths with spaces are safe, and embedded
/// backslashes / quotes are escaped to survive JSON round-trip.
fn build_command_line(exe_path: &str) -> String {
    let escaped = escape_for_quoted_arg(exe_path);
    format!("\"{escaped}\" statusline")
}

fn escape_for_quoted_arg(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(ch),
        }
    }
    out
}

/// Attempt to install the statusLine hook pointing at this executable. Returns
/// the backup path on success.
pub fn install() -> Result<PathBuf, String> {
    let path = settings_path().ok_or_else(|| "home directory not found".to_string())?;
    let exe = std::env::current_exe()
        .map_err(|e| format!("cannot resolve exe path: {e}"))?
        .to_string_lossy()
        .into_owned();

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
    }

    let existing = fs::read_to_string(&path).unwrap_or_else(|_| "{}".to_string());
    let mut value: Value =
        serde_json::from_str(&existing).map_err(|e| format!("parse {}: {e}", path.display()))?;
    if !value.is_object() {
        value = Value::Object(Map::new());
    }

    let backup_path = backup_settings(&path)?;
    install_into(&mut value, &exe);

    let serialized = serde_json::to_string_pretty(&value).map_err(|e| format!("serialize: {e}"))?;
    fs::write(&path, serialized).map_err(|e| format!("write {}: {e}", path.display()))?;

    Ok(backup_path)
}

fn install_into(value: &mut Value, exe_path: &str) {
    let obj = value.as_object_mut().expect("caller ensures object");
    let entry = obj
        .entry("statusLine".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !entry.is_object() {
        *entry = Value::Object(Map::new());
    }
    let map = entry.as_object_mut().unwrap();
    map.insert("type".to_string(), Value::String("command".to_string()));
    map.insert(
        "command".to_string(),
        Value::String(build_command_line(exe_path)),
    );
}

/// Remove our statusLine entry if it's present and points at this executable.
pub fn uninstall() -> Result<(), String> {
    let path = settings_path().ok_or_else(|| "home directory not found".to_string())?;
    let Ok(existing) = fs::read_to_string(&path) else {
        return Ok(());
    };
    let mut value: Value =
        serde_json::from_str(&existing).map_err(|e| format!("parse {}: {e}", path.display()))?;

    let exe_lower = current_exe_path_lowercase();
    let remove = matches!(classify(&value, &exe_lower), InstallState::OursInstalled);

    if !remove {
        return Ok(());
    }

    let _backup = backup_settings(&path)?;

    if let Some(obj) = value.as_object_mut() {
        obj.remove("statusLine");
    }

    let serialized = serde_json::to_string_pretty(&value).map_err(|e| format!("serialize: {e}"))?;
    fs::write(&path, serialized).map_err(|e| format!("write {}: {e}", path.display()))?;

    Ok(())
}

fn backup_settings(path: &std::path::Path) -> Result<PathBuf, String> {
    if !path.exists() {
        return Ok(path.with_extension("ccum-backup-0"));
    }
    let unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(format!("settings.json.ccum-backup-{unix}"));
    fs::copy(path, &backup).map_err(|e| format!("backup {}: {e}", backup.display()))?;
    Ok(backup)
}

/// Rendering snippet shown in a "Show snippet" message box so the user can copy
/// the install by hand (e.g. on WSL-only setups).
pub fn snippet() -> String {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "<path to claude-code-usage-monitor.exe>".to_string());
    format!(
        "\"statusLine\": {{\n  \"type\": \"command\",\n  \"command\": \"{}\"\n}}",
        build_command_line(&exe)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classify_not_configured_when_empty_object() {
        let v = json!({});
        assert_eq!(
            classify(&v, "c:\\foo\\bar.exe"),
            InstallState::NotConfigured
        );
    }

    #[test]
    fn classify_ours_when_command_contains_our_path() {
        let v = json!({
            "statusLine": {
                "type": "command",
                "command": "\"C:\\Program Files\\CCUM\\claude-code-usage-monitor.exe\" statusline"
            }
        });
        let state = classify(&v, "c:\\program files\\ccum\\claude-code-usage-monitor.exe");
        assert_eq!(state, InstallState::OursInstalled);
    }

    #[test]
    fn classify_other_when_command_points_elsewhere() {
        let v = json!({
            "statusLine": { "type": "command", "command": "ccusage statusline" }
        });
        let state = classify(&v, "c:\\foo\\ours.exe");
        match state {
            InstallState::OtherInstalled(cmd) => assert_eq!(cmd, "ccusage statusline"),
            _ => panic!("expected OtherInstalled"),
        }
    }

    #[test]
    fn install_into_preserves_unknown_keys() {
        let mut v = json!({
            "permissions": {"allow": ["Bash"]},
            "model": "claude-sonnet-4-6",
        });
        install_into(&mut v, "C:\\a\\b.exe");
        assert_eq!(v.get("permissions").unwrap(), &json!({"allow": ["Bash"]}));
        assert_eq!(v.get("model").unwrap().as_str(), Some("claude-sonnet-4-6"));
        let cmd = v["statusLine"]["command"].as_str().unwrap();
        assert!(cmd.contains("b.exe"));
        assert!(cmd.ends_with("statusline"));
    }

    #[test]
    fn install_into_overwrites_existing_statusline() {
        let mut v = json!({
            "statusLine": {"type": "command", "command": "old"}
        });
        install_into(&mut v, "C:\\new.exe");
        assert!(v["statusLine"]["command"]
            .as_str()
            .unwrap()
            .contains("new.exe"));
    }

    #[test]
    fn build_command_line_quotes_paths_with_spaces() {
        let cmd = build_command_line("C:\\Program Files\\thing.exe");
        assert_eq!(cmd, "\"C:\\\\Program Files\\\\thing.exe\" statusline");
    }
}
