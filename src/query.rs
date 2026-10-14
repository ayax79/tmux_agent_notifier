//! Read-side commands: status line summary, listing, and jumping to panes.

use std::cmp::Reverse;

use anyhow::Result;
use serde::Serialize;

use crate::pane::now;
use crate::state::State;
use crate::tmux;

#[derive(Debug, Clone, Serialize)]
pub struct AgentPane {
    pub pane_id: String,
    pub target: String,
    pub state: State,
    pub agent: String,
    pub since: u64,
    pub path: String,
    pub message: String,
}

pub fn agent_panes() -> Result<Vec<AgentPane>> {
    let rows = tmux::list(
        &["list-panes", "-a"],
        &[
            "#{pane_id}",
            "#{session_name}:#{window_index}.#{pane_index}",
            "#{@agent_state}",
            "#{@agent_name}",
            "#{@agent_since}",
            "#{pane_current_path}",
            "#{@agent_message}",
        ],
    )?;
    Ok(rows
        .into_iter()
        .filter_map(|mut r| {
            let state = State::from_option(&r[2])?;
            Some(AgentPane {
                state,
                since: r[4].parse().unwrap_or_default(),
                pane_id: std::mem::take(&mut r[0]),
                target: std::mem::take(&mut r[1]),
                agent: std::mem::take(&mut r[3]),
                path: std::mem::take(&mut r[5]),
                message: std::mem::take(&mut r[6]),
            })
        })
        .collect())
}

/// Status line segment such as `!1 ✓2 …1`. Icons come from `@tan_icon_<state>`;
/// set one to `none` to hide that count.
pub fn status() -> Result<String> {
    let panes = agent_panes()?;
    let icons = tmux::display(
        None,
        &[
            "#{@tan_icon_attention}",
            "#{@tan_icon_waiting}",
            "#{@tan_icon_running}",
        ],
    )?;
    let parts: Vec<String> = [State::Attention, State::Waiting, State::Running]
        .into_iter()
        .zip(icons)
        .filter_map(|(state, icon)| {
            let icon = if icon.is_empty() {
                default_icon(state)
            } else {
                &icon
            };
            let count = panes.iter().filter(|p| p.state == state).count();
            (icon != "none" && count > 0).then(|| format!("{icon}{count}"))
        })
        .collect();
    Ok(parts.join(" "))
}

fn default_icon(state: State) -> &'static str {
    match state {
        State::Attention => "!",
        State::Waiting => "✓",
        State::Running => "…",
    }
}

pub fn list(json: bool) -> Result<String> {
    let panes = agent_panes()?;
    if json {
        return Ok(serde_json::to_string_pretty(&panes)?);
    }
    let now = now();
    let lines: Vec<String> = panes
        .iter()
        .map(|p| {
            format!(
                "{:<20} {:<12} {:<10} {:>5}  {}",
                p.target,
                p.agent,
                p.state,
                age(now.saturating_sub(p.since)),
                p.path
            )
        })
        .collect();
    Ok(lines.join("\n"))
}

fn age(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86400 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86400),
    }
}

/// The pane to visit next: most urgent first, then the one waiting longest.
fn pick_next(panes: &[AgentPane]) -> Option<&AgentPane> {
    panes
        .iter()
        .filter(|p| p.state.needs_user())
        .min_by_key(|p| (Reverse(p.state), p.since))
}

/// Switches `client` to the agent pane that most needs the user.
pub fn next(client: Option<&str>) -> Result<()> {
    let panes = agent_panes()?;
    let mut args = vec!["switch-client"];
    if let Some(client) = client {
        args.extend(["-c", client]);
    }
    match pick_next(&panes) {
        Some(pane) => {
            args.extend(["-t", &pane.pane_id]);
            tmux::run(args)?;
        }
        None => {
            let mut args = vec!["display-message"];
            if let Some(client) = client {
                args.extend(["-c", client]);
            }
            args.push("no agents waiting");
            tmux::run(args)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pane(id: &str, state: State, since: u64) -> AgentPane {
        AgentPane {
            pane_id: id.to_owned(),
            target: String::new(),
            state,
            agent: String::new(),
            since,
            path: String::new(),
            message: String::new(),
        }
    }

    #[test]
    fn next_prefers_attention_then_oldest() {
        let panes = [
            pane("%1", State::Waiting, 10),
            pane("%2", State::Attention, 50),
            pane("%3", State::Attention, 20),
            pane("%4", State::Running, 1),
        ];
        assert_eq!(pick_next(&panes).map(|p| p.pane_id.as_str()), Some("%3"));
    }

    #[test]
    fn next_skips_running() {
        assert!(pick_next(&[pane("%1", State::Running, 1)]).is_none());
    }

    #[test]
    fn age_units() {
        assert_eq!(age(5), "5s");
        assert_eq!(age(125), "2m");
        assert_eq!(age(7200), "2h");
        assert_eq!(age(200_000), "2d");
    }
}
