//! Thin wrapper around the `tmux` CLI.

use std::ffi::OsStr;
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Field separator used in `-F` / `display -p` formats.
pub const SEP: char = '\t';

pub const PANE_STATE: &str = "@agent_state";
pub const PANE_LABEL: &str = "@agent_name";
pub const PANE_SINCE: &str = "@agent_since";
pub const PANE_MESSAGE: &str = "@agent_message";
/// Kept separate from `@agent_state` because pane options inherit window options.
pub const WINDOW_STATE: &str = "@agent_window_state";

/// Runs `tmux` with `args` and returns its stdout.
///
/// # Errors
///
/// Returns an error if tmux cannot be spawned or exits unsuccessfully.
pub fn run<I, S>(args: I) -> Result<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new("tmux")
        .args(args)
        .output()
        .context("failed to run tmux")?;
    if !output.status.success() {
        bail!(
            "tmux failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Expands `fields` (joined with [`SEP`]) against `target` and splits the result.
pub fn display(target: Option<&str>, fields: &[&str]) -> Result<Vec<String>> {
    let format = fields.join(&SEP.to_string());
    let mut args = vec!["display-message", "-p"];
    if let Some(target) = target {
        args.extend(["-t", target]);
    }
    args.push(&format);
    let out = run(args)?;
    Ok(split_line(out.trim_end_matches('\n'), fields.len()))
}

/// Runs a `list-*` command and splits each line into `fields.len()` columns.
pub fn list(command: &[&str], fields: &[&str]) -> Result<Vec<Vec<String>>> {
    let format = fields.join(&SEP.to_string());
    let out = run(command.iter().copied().chain(["-F", format.as_str()]))?;
    Ok(out
        .lines()
        .map(|line| split_line(line, fields.len()))
        .collect())
}

fn split_line(line: &str, n: usize) -> Vec<String> {
    let mut cols: Vec<String> = line.splitn(n, SEP).map(str::to_owned).collect();
    cols.resize(n, String::new());
    cols
}

#[derive(Debug, Clone)]
pub struct Client {
    pub name: String,
    pub session_id: String,
    pub focused: bool,
}

pub fn clients() -> Result<Vec<Client>> {
    let rows = list(
        &["list-clients"],
        &["#{client_name}", "#{session_id}", "#{client_flags}"],
    )?;
    Ok(rows
        .into_iter()
        .map(|mut r| Client {
            focused: r[2].split(',').any(|f| f == "focused"),
            session_id: std::mem::take(&mut r[1]),
            name: std::mem::take(&mut r[0]),
        })
        .collect())
}

/// Several tmux commands sent in one invocation, separated by `;`.
#[derive(Debug, Default)]
pub struct Batch {
    args: Vec<String>,
}

impl Batch {
    pub fn push(&mut self, command: &[&str]) {
        if !self.args.is_empty() {
            self.args.push(";".to_owned());
        }
        self.args.extend(command.iter().map(|s| (*s).to_owned()));
    }

    pub fn run(self) -> Result<()> {
        if !self.args.is_empty() {
            run(&self.args)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_line_pads_missing_columns() {
        assert_eq!(split_line("a\tb", 3), vec!["a", "b", ""]);
        assert_eq!(split_line("a\tb\tc\td", 3), vec!["a", "b", "c\td"]);
    }
}
