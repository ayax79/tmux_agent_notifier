//! Registers `tan hook <agent>` with each agent's hook configuration.
//!
//! Existing hooks are preserved; entries are recognised as ours by their
//! command, so reinstalling (e.g. with a new binary path) is idempotent.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::agent::Agent;

const BIN_NAME: &str = "tan";
const HOOK_TIMEOUT_SECS: u64 = 10;
const GOOSE_PLUGIN: &str = "tmux-agent-notifier";

const CLAUDE_EVENTS: [&str; 6] = [
    "UserPromptSubmit",
    "Stop",
    "Notification",
    "PostToolUse",
    "SessionStart",
    "SessionEnd",
];
const CURSOR_EVENTS: [&str; 5] = [
    "beforeSubmitPrompt",
    "stop",
    "postToolUse",
    "sessionStart",
    "sessionEnd",
];
const GOOSE_EVENTS: [&str; 5] = [
    "UserPromptSubmit",
    "Stop",
    "PostToolUse",
    "SessionStart",
    "SessionEnd",
];

/// Installs hooks for `agent` that run `bin hook <agent>`; returns the path written.
///
/// # Errors
///
/// Returns an error if the agent's config cannot be read, parsed, or written.
pub fn install(agent: Agent, bin: &str) -> Result<PathBuf> {
    let command = hook_command(bin, agent);
    match agent {
        Agent::Claude => edit_json(&claude_settings()?, |v| merge_claude(v, Some(&command))),
        Agent::Cursor => edit_json(&cursor_hooks()?, |v| merge_cursor(v, Some(&command))),
        Agent::Goose => install_goose(&command),
    }
}

/// Removes our hooks for `agent`; returns the path touched, if any.
///
/// # Errors
///
/// Returns an error if the agent's config cannot be read, parsed, or written.
pub fn uninstall(agent: Agent) -> Result<Option<PathBuf>> {
    let path = match agent {
        Agent::Claude => claude_settings()?,
        Agent::Cursor => cursor_hooks()?,
        Agent::Goose => goose_plugin_dir()?,
    };
    if !path.exists() {
        return Ok(None);
    }
    match agent {
        Agent::Claude => {
            edit_json(&path, |v| merge_claude(v, None))?;
        }
        Agent::Cursor => {
            edit_json(&path, |v| merge_cursor(v, None))?;
        }
        Agent::Goose => fs::remove_dir_all(&path)
            .with_context(|| format!("failed to remove {}", path.display()))?,
    }
    Ok(Some(path))
}

fn home() -> Result<PathBuf> {
    env::var_os("HOME")
        .map(PathBuf::from)
        .context("HOME is not set")
}

fn claude_settings() -> Result<PathBuf> {
    let dir = match env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => home()?.join(".claude"),
    };
    Ok(dir.join("settings.json"))
}

fn cursor_hooks() -> Result<PathBuf> {
    Ok(home()?.join(".cursor").join("hooks.json"))
}

fn goose_plugin_dir() -> Result<PathBuf> {
    Ok(home()?.join(".agents").join("plugins").join(GOOSE_PLUGIN))
}

fn hook_command(bin: &str, agent: Agent) -> String {
    format!("{} hook {}", shell_quote(bin), agent.as_str())
}

fn shell_quote(s: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "/._-+:@,=".contains(c);
    if !s.is_empty() && s.chars().all(safe) {
        s.to_owned()
    } else {
        format!("'{}'", s.replace('\'', r"'\''"))
    }
}

fn is_ours(command: &str) -> bool {
    let mut words = command.trim().rsplitn(3, char::is_whitespace);
    let (Some(agent), Some("hook"), Some(bin)) = (words.next(), words.next(), words.next()) else {
        return false;
    };
    Agent::ALL.iter().any(|a| a.as_str() == agent)
        && Path::new(bin.trim().trim_matches('\''))
            .file_name()
            .is_some_and(|name| name == BIN_NAME)
}

fn command_of(entry: &Value) -> Option<&str> {
    entry.get("command").and_then(Value::as_str)
}

/// Removes our entries from Claude/Goose style `[{matcher?, hooks: [...]}]` groups.
fn remove_from_groups(groups: &mut Value) {
    let Some(groups) = groups.as_array_mut() else {
        return;
    };
    groups.retain_mut(|group| {
        let Some(entries) = group.get_mut("hooks").and_then(Value::as_array_mut) else {
            return true;
        };
        let before = entries.len();
        entries.retain(|e| !command_of(e).is_some_and(is_ours));
        before == 0 || !entries.is_empty()
    });
}

fn hook_group(command: &str) -> Value {
    json!({ "hooks": [{ "type": "command", "command": command, "timeout": HOOK_TIMEOUT_SECS }] })
}

fn object_field<'a>(
    parent: &'a mut serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a mut serde_json::Map<String, Value>> {
    parent
        .entry(key)
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .with_context(|| format!("\"{key}\" is not a JSON object"))
}

fn merge_claude(settings: &mut Value, command: Option<&str>) -> Result<()> {
    let root = settings
        .as_object_mut()
        .context("settings are not a JSON object")?;
    let hooks = object_field(root, "hooks")?;
    for groups in hooks.values_mut() {
        remove_from_groups(groups);
    }
    hooks.retain(|_, groups| !groups.as_array().is_some_and(Vec::is_empty));
    if let Some(command) = command {
        for event in CLAUDE_EVENTS {
            let groups = hooks.entry(event).or_insert_with(|| json!([]));
            groups
                .as_array_mut()
                .with_context(|| format!("hooks.{event} is not an array"))?
                .push(hook_group(command));
        }
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }
    Ok(())
}

fn merge_cursor(config: &mut Value, command: Option<&str>) -> Result<()> {
    let root = config
        .as_object_mut()
        .context("hooks.json is not a JSON object")?;
    root.entry("version").or_insert(json!(1));
    let hooks = object_field(root, "hooks")?;
    for entries in hooks.values_mut() {
        if let Some(entries) = entries.as_array_mut() {
            entries.retain(|e| !command_of(e).is_some_and(is_ours));
        }
    }
    hooks.retain(|_, entries| !entries.as_array().is_some_and(Vec::is_empty));
    if let Some(command) = command {
        for event in CURSOR_EVENTS {
            hooks
                .entry(event)
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .with_context(|| format!("hooks.{event} is not an array"))?
                .push(json!({ "command": command, "timeout": HOOK_TIMEOUT_SECS }));
        }
    }
    Ok(())
}

fn goose_hooks(command: &str) -> Value {
    let hooks: serde_json::Map<String, Value> = GOOSE_EVENTS
        .iter()
        .map(|event| ((*event).to_owned(), json!([hook_group(command)])))
        .collect();
    json!({ "hooks": hooks })
}

fn install_goose(command: &str) -> Result<PathBuf> {
    let dir = goose_plugin_dir()?;
    let manifest = json!({
        "name": GOOSE_PLUGIN,
        "version": env!("CARGO_PKG_VERSION"),
        "description": "Surface goose activity in tmux",
    });
    write_json(&dir.join("plugin.json"), &manifest)?;
    write_json(&dir.join("hooks").join("hooks.json"), &goose_hooks(command))?;
    Ok(dir)
}

fn edit_json(path: &Path, edit: impl FnOnce(&mut Value) -> Result<()>) -> Result<PathBuf> {
    let mut value = match fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => json!({}),
        Ok(text) => serde_json::from_str(&text)
            .with_context(|| format!("failed to parse {}", path.display()))?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(err) => return Err(err).with_context(|| format!("failed to read {}", path.display())),
    };
    edit(&mut value).with_context(|| format!("failed to update {}", path.display()))?;
    write_json(path, &value)?;
    Ok(path.to_owned())
}

fn write_json(path: &Path, value: &Value) -> Result<()> {
    if path.is_symlink() {
        bail!(
            "{} is a symlink (managed elsewhere?); refusing to overwrite",
            path.display()
        );
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let mut text = serde_json::to_string_pretty(value)?;
    text.push('\n');
    let tmp = path.with_extension("tan-tmp");
    fs::write(&tmp, text).with_context(|| format!("failed to write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CMD: &str = "/home/me/.nix-profile/bin/tan hook claude";

    #[test]
    fn recognises_our_commands() {
        assert!(is_ours("tan hook claude"));
        assert!(is_ours(
            "/nix/store/abc-tmux-agent-notifier/bin/tan hook goose"
        ));
        assert!(is_ours("'/Users/me/My Tools/tan' hook cursor"));
        assert!(!is_ours("titan hook claude"));
        assert!(!is_ours("tan status"));
        assert!(!is_ours("./hooks/format.sh"));
    }

    #[test]
    fn quotes_paths_with_spaces() {
        assert_eq!(shell_quote("/usr/bin/tan"), "/usr/bin/tan");
        assert_eq!(shell_quote("/My Tools/tan"), "'/My Tools/tan'");
    }

    #[test]
    fn claude_merge_preserves_other_hooks_and_is_idempotent() {
        let mut settings = json!({
            "theme": "dark",
            "hooks": {
                "Stop": [{ "hooks": [{ "type": "command", "command": "say done" }] }]
            }
        });
        merge_claude(&mut settings, Some(CMD)).unwrap();
        merge_claude(&mut settings, Some(CMD)).unwrap();

        assert_eq!(settings["theme"], "dark");
        let stop = settings["hooks"]["Stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[0]["hooks"][0]["command"], "say done");
        assert_eq!(stop[1]["hooks"][0]["command"], CMD);
        for event in CLAUDE_EVENTS {
            assert!(settings["hooks"][event].is_array(), "{event} missing");
        }
    }

    #[test]
    fn claude_uninstall_leaves_only_foreign_hooks() {
        let mut settings = json!({ "theme": "dark" });
        merge_claude(&mut settings, Some(CMD)).unwrap();
        merge_claude(&mut settings, None).unwrap();
        assert_eq!(settings, json!({ "theme": "dark" }));
    }

    #[test]
    fn cursor_merge_replaces_old_binary_path() {
        let mut config = json!({
            "version": 1,
            "hooks": { "stop": [{ "command": "/old/bin/tan hook cursor" }, { "command": "./hooks/audit.sh" }] }
        });
        merge_cursor(&mut config, Some("/new/bin/tan hook cursor")).unwrap();
        let stop = config["hooks"]["stop"].as_array().unwrap();
        assert_eq!(stop.len(), 2);
        assert_eq!(stop[0]["command"], "./hooks/audit.sh");
        assert_eq!(stop[1]["command"], "/new/bin/tan hook cursor");

        merge_cursor(&mut config, None).unwrap();
        assert_eq!(
            config,
            json!({ "version": 1, "hooks": { "stop": [{ "command": "./hooks/audit.sh" }] } })
        );
    }

    #[test]
    fn goose_hooks_cover_lifecycle() {
        let hooks = goose_hooks("tan hook goose");
        for event in GOOSE_EVENTS {
            assert_eq!(
                hooks["hooks"][event][0]["hooks"][0]["command"],
                "tan hook goose"
            );
        }
    }
}
