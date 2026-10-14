//! Delivers alerts: terminal bell, desktop notifications, tmux messages.

use std::fs::OpenOptions;
use std::io::Write;
use std::process::Command;

use anyhow::{Context, Result};

const MAX_LEN: usize = 160;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Desktop {
    /// OSC 777 wrapped in tmux passthrough; needs `allow-passthrough all`.
    Osc,
    /// macOS Notification Center via `osascript`.
    Osascript,
    Off,
}

impl Desktop {
    /// Parses `@tan_desktop`; unset means [`Desktop::Osc`].
    pub fn from_option(value: &str) -> Desktop {
        match value.trim() {
            "osascript" => Desktop::Osascript,
            "off" | "none" => Desktop::Off,
            _ => Desktop::Osc,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub title: String,
    pub body: String,
}

impl Alert {
    pub fn new(title: &str, body: &str) -> Alert {
        Alert {
            title: clean(title),
            body: clean(body),
        }
    }

    /// Text for `display-message`, which expands `#` formats.
    pub fn tmux_line(&self) -> String {
        format!("{}: {}", self.title, self.body).replace('#', "##")
    }
}

/// Collapses `text` to a single printable line of bounded length.
pub fn clean(text: &str) -> String {
    let single: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let words: Vec<&str> = single.split_whitespace().collect();
    let joined = words.join(" ");
    if joined.chars().count() <= MAX_LEN {
        joined
    } else {
        let mut cut: String = joined.chars().take(MAX_LEN - 1).collect();
        cut.push('…');
        cut
    }
}

/// Writes the bell and/or OSC notification to the pane's tty so tmux treats
/// them as output from that pane, then runs any out-of-band notifier.
pub fn deliver(pane_tty: &str, bell: bool, desktop: Desktop, alert: &Alert) -> Result<()> {
    let mut bytes = Vec::new();
    if bell {
        bytes.push(0x07);
    }
    if desktop == Desktop::Osc {
        bytes.extend(osc_notification(alert));
    }
    if !bytes.is_empty() {
        OpenOptions::new()
            .write(true)
            .open(pane_tty)
            .and_then(|mut tty| tty.write_all(&bytes))
            .with_context(|| format!("failed to write to {pane_tty}"))?;
    }
    if desktop == Desktop::Osascript {
        osascript(alert)?;
    }
    Ok(())
}

fn osc_notification(alert: &Alert) -> Vec<u8> {
    let inner = format!(
        "\x1b]777;notify;{};{}\x07",
        alert.title.replace(';', ","),
        alert.body.replace(';', ",")
    );
    let mut out = b"\x1bPtmux;".to_vec();
    for byte in inner.bytes() {
        if byte == 0x1b {
            out.push(0x1b);
        }
        out.push(byte);
    }
    out.extend_from_slice(b"\x1b\\");
    out
}

fn osascript(alert: &Alert) -> Result<()> {
    Command::new("osascript")
        .args([
            "-e",
            "on run argv",
            "-e",
            "display notification (item 2 of argv) with title (item 1 of argv)",
            "-e",
            "end run",
            &alert.title,
            &alert.body,
        ])
        .output()
        .context("failed to run osascript")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_flattens_and_truncates() {
        assert_eq!(clean("  done.\n\nran\ttests "), "done. ran tests");
        let long = "x".repeat(500);
        let cleaned = clean(&long);
        assert_eq!(cleaned.chars().count(), MAX_LEN);
        assert!(cleaned.ends_with('…'));
    }

    #[test]
    fn osc_is_wrapped_for_tmux_passthrough() {
        let alert = Alert::new("Claude; api:1", "done");
        let bytes = osc_notification(&alert);
        assert_eq!(
            bytes,
            b"\x1bPtmux;\x1b\x1b]777;notify;Claude, api:1;done\x07\x1b\\".to_vec()
        );
    }

    #[test]
    fn tmux_line_escapes_formats() {
        assert_eq!(Alert::new("a", "#{pane_id}").tmux_line(), "a: ##{pane_id}");
    }

    #[test]
    fn desktop_defaults_to_osc() {
        assert_eq!(Desktop::from_option(""), Desktop::Osc);
        assert_eq!(Desktop::from_option("off"), Desktop::Off);
        assert_eq!(Desktop::from_option("osascript"), Desktop::Osascript);
    }
}
