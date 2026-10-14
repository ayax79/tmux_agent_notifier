//! Applies state transitions to a tmux pane and alerts when it needs the user.

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::agent::Transition;
use crate::notify::{self, Alert, Desktop};
use crate::state::State;
use crate::tmux::{
    self, Batch, Client, PANE_LABEL, PANE_MESSAGE, PANE_SINCE, PANE_STATE, WINDOW_STATE,
};

const MESSAGE_DURATION_MS: &str = "5000";

#[derive(Debug)]
struct PaneInfo {
    tty: String,
    active: bool,
    window_active: bool,
    session_id: String,
    session_name: String,
    window_index: String,
    window_name: String,
    state: Option<State>,
    bell: bool,
    message: bool,
    desktop: Desktop,
}

impl PaneInfo {
    const FIELDS: [&str; 11] = [
        "#{pane_tty}",
        "#{pane_active}",
        "#{window_active}",
        "#{session_id}",
        "#{session_name}",
        "#{window_index}",
        "#{window_name}",
        "#{@agent_state}",
        "#{@tan_bell}",
        "#{@tan_message}",
        "#{@tan_desktop}",
    ];

    fn load(pane: &str) -> Result<PaneInfo> {
        let f = tmux::display(Some(pane), &Self::FIELDS)
            .with_context(|| format!("failed to inspect pane {pane}"))?;
        let on = |v: &str| v.trim() != "off";
        Ok(PaneInfo {
            active: f[1] == "1",
            window_active: f[2] == "1",
            state: State::from_option(&f[7]),
            bell: on(&f[8]),
            message: on(&f[9]),
            desktop: Desktop::from_option(&f[10]),
            tty: f[0].clone(),
            session_id: f[3].clone(),
            session_name: f[4].clone(),
            window_index: f[5].clone(),
            window_name: f[6].clone(),
        })
    }

    fn is_viewed_by(&self, client: &Client) -> bool {
        client.session_id == self.session_id && self.window_active
    }

    /// Visible in a focused terminal, so the user is already looking at it.
    fn is_focused(&self, clients: &[Client]) -> bool {
        self.active && clients.iter().any(|c| c.focused && self.is_viewed_by(c))
    }
}

/// Moves `pane` to the state described by `transition`, alerting on entry
/// into a state that needs the user.
///
/// # Errors
///
/// Returns an error if tmux cannot be queried or updated.
pub fn apply(pane: &str, label: &str, transition: &Transition) -> Result<()> {
    let (target, message) = match transition {
        Transition::Ignore => return Ok(()),
        Transition::Clear => (None, None),
        Transition::Set { state, message } => (Some(*state), message.as_deref()),
    };

    let info = PaneInfo::load(pane)?;
    let clients = tmux::clients()?;
    let target = target.filter(|s| !(s.needs_user() && info.is_focused(&clients)));
    if target == info.state {
        return Ok(());
    }

    let mut batch = Batch::default();
    match target {
        Some(state) => {
            let since = now().to_string();
            batch.push(&["set", "-pq", "-t", pane, PANE_STATE, state.as_str()]);
            batch.push(&["set", "-pq", "-t", pane, PANE_LABEL, &notify::clean(label)]);
            batch.push(&["set", "-pq", "-t", pane, PANE_SINCE, &since]);
            match message {
                Some(m) => batch.push(&["set", "-pq", "-t", pane, PANE_MESSAGE, &notify::clean(m)]),
                None => batch.push(&["set", "-pqu", "-t", pane, PANE_MESSAGE]),
            }
        }
        None => {
            for option in [PANE_STATE, PANE_LABEL, PANE_SINCE, PANE_MESSAGE] {
                batch.push(&["set", "-pqu", "-t", pane, option]);
            }
        }
    }

    match window_state(pane, target)? {
        Some(state) => batch.push(&["set", "-wq", "-t", pane, WINDOW_STATE, state.as_str()]),
        None => batch.push(&["set", "-wqu", "-t", pane, WINDOW_STATE]),
    }

    let alert = target.filter(|s| s.needs_user()).map(|state| {
        let title = format!(
            "{label} · {}:{} {}",
            info.session_name, info.window_index, info.window_name
        );
        let body = message.unwrap_or(match state {
            State::Attention => "needs your attention",
            _ => "finished, waiting for input",
        });
        Alert::new(&title, body)
    });

    if let Some(alert) = alert.as_ref().filter(|_| info.message) {
        let line = alert.tmux_line();
        for client in clients.iter().filter(|c| !info.is_viewed_by(c)) {
            batch.push(&[
                "display-message",
                "-c",
                &client.name,
                "-d",
                MESSAGE_DURATION_MS,
                &line,
            ]);
        }
    }
    for client in &clients {
        batch.push(&["refresh-client", "-S", "-t", &client.name]);
    }
    batch.run()?;

    if let Some(alert) = alert {
        notify::deliver(&info.tty, info.bell, info.desktop, &alert)?;
    }
    Ok(())
}

/// Clears `pane` if it is waiting on the user, or unconditionally with `force`.
///
/// # Errors
///
/// Returns an error if tmux cannot be queried or updated.
pub fn clear(pane: &str, force: bool) -> Result<()> {
    let info = PaneInfo::load(pane)?;
    if info.state.is_some_and(|s| force || s.needs_user()) {
        apply(pane, "", &Transition::Clear)?;
    }
    Ok(())
}

/// Most urgent state across the window's panes once `pane` becomes `target`.
fn window_state(pane: &str, target: Option<State>) -> Result<Option<State>> {
    let panes = tmux::list(
        &["list-panes", "-t", pane],
        &["#{pane_id}", "#{@agent_state}"],
    )?;
    Ok(panes
        .iter()
        .filter(|row| row[0] != pane)
        .filter_map(|row| State::from_option(&row[1]))
        .chain(target)
        .max())
}

/// Whether the user is currently looking at `pane`.
///
/// # Errors
///
/// Returns an error if tmux cannot be queried.
pub fn is_focused(pane: &str) -> Result<bool> {
    Ok(PaneInfo::load(pane)?.is_focused(&tmux::clients()?))
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

pub fn now_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default()
}
