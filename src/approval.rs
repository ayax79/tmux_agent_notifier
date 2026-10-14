//! Detects tool approval prompts, which agents like Cursor show without
//! firing a hook: `preToolUse` starts a detached watcher that polls the pane
//! and raises `attention` once the prompt is on screen.

use std::env;
use std::os::unix::process::CommandExt;
use std::process::{self, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::agent::Transition;
use crate::pane;
use crate::state::State;
use crate::tmux::{self, PANE_STATE, PANE_WATCH};

const POLL: Duration = Duration::from_millis(1000);
const MAX_WATCH: Duration = Duration::from_secs(60 * 60);
const PROMPT_MARKERS: [&str; 2] = ["Skip (esc or n)", "Reject & propose changes"];
const DEFAULT_MESSAGE: &str = "waiting for tool approval";

/// Starts a watcher for `pane`, replacing any previous one.
///
/// # Errors
///
/// Returns an error if tmux cannot be updated or the watcher cannot be spawned.
pub fn spawn(pane: &str, label: &str) -> Result<()> {
    let token = format!("{}-{}", process::id(), pane::now_nanos());
    tmux::run(["set", "-pq", "-t", pane, PANE_WATCH, &token])?;
    let exe = env::current_exe().context("failed to locate tan binary")?;
    Command::new(exe)
        .args([
            "watch-approval",
            "--pane",
            pane,
            "--label",
            label,
            "--token",
            &token,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .context("failed to spawn approval watcher")?;
    Ok(())
}

/// Polls `pane` while the agent is running and this watcher is current,
/// raising `attention` when an approval prompt appears.
///
/// # Errors
///
/// Returns an error if tmux cannot be queried (e.g. the pane closed).
pub fn watch(pane: &str, label: &str, token: &str) -> Result<()> {
    let deadline = Instant::now() + MAX_WATCH;
    let format = format!("#{{{PANE_WATCH}}}");
    let state = format!("#{{{PANE_STATE}}}");
    while Instant::now() < deadline {
        thread::sleep(POLL);
        let f = tmux::display(Some(pane), &[&format, &state])?;
        if f[0] != token || State::from_option(&f[1]) != Some(State::Running) {
            return Ok(());
        }
        if pane::is_focused(pane)? {
            continue;
        }
        let screen = tmux::run(["capture-pane", "-p", "-t", pane])?;
        if let Some(question) = prompt(&screen) {
            let transition = Transition::Set {
                state: State::Attention,
                message: Some(question),
            };
            return pane::apply(pane, label, &transition);
        }
    }
    Ok(())
}

/// The prompt's question (e.g. "Run this MCP tool?") if `screen` shows one.
fn prompt(screen: &str) -> Option<String> {
    let lines: Vec<&str> = screen.lines().map(str::trim).collect();
    let marker = lines
        .iter()
        .rposition(|l| PROMPT_MARKERS.iter().any(|m| l.contains(m)))?;
    let question = lines[..marker]
        .iter()
        .rev()
        .take(8)
        .find(|l| l.ends_with('?'))
        .map_or(DEFAULT_MESSAGE, |l| l);
    Some(question.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_cursor_mcp_prompt() {
        let screen = r#"
 databricks-mcp-server: execute_sql
   {
     "output_format": "markdown"
   }

 Run this MCP tool?
  → Run (once) (y)
    Allowlist MCP Tool (tab)
    Reject & propose changes (p)
    Skip (esc or n)
"#;
        assert_eq!(prompt(screen).as_deref(), Some("Run this MCP tool?"));
    }

    #[test]
    fn falls_back_without_question() {
        assert_eq!(
            prompt("  Skip (esc or n)\n").as_deref(),
            Some(DEFAULT_MESSAGE)
        );
    }

    #[test]
    fn ignores_ordinary_output() {
        assert_eq!(prompt("Read 2 files\nNow I'll run the query?\n"), None);
    }
}
