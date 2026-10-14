//! Translates each agent's hook payload into a pane state transition.
//!
//! Claude Code and Cursor send `hook_event_name`; Goose sends `event`.

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::state::State;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Agent {
    Claude,
    Cursor,
    Goose,
}

impl Agent {
    pub const ALL: [Agent; 3] = [Agent::Claude, Agent::Cursor, Agent::Goose];

    pub fn as_str(self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Cursor => "cursor",
            Agent::Goose => "goose",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Agent::Claude => "Claude Code",
            Agent::Cursor => "Cursor",
            Agent::Goose => "Goose",
        }
    }

    /// JSON the agent expects on stdout for `event`, if any.
    pub fn hook_response(self, event: &str) -> Option<&'static str> {
        match (self, event) {
            (Agent::Cursor, "beforeSubmitPrompt") => Some(r#"{"continue":true}"#),
            (Agent::Cursor, _) => Some("{}"),
            // Goose reads stdout as a block/allow decision, so it must stay empty.
            (Agent::Claude | Agent::Goose, _) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Transition {
    Set {
        state: State,
        message: Option<String>,
    },
    Clear,
    Ignore,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookEvent {
    pub name: String,
    pub transition: Transition,
}

#[derive(Debug, Default, Deserialize)]
struct Payload {
    hook_event_name: Option<String>,
    event: Option<String>,
    message: Option<String>,
    notification_type: Option<String>,
    last_assistant_message: Option<String>,
    status: Option<String>,
}

impl Payload {
    fn event_name(&self) -> &str {
        self.hook_event_name
            .as_deref()
            .or(self.event.as_deref())
            .unwrap_or_default()
    }
}

/// Parses a hook payload read from stdin.
///
/// # Errors
///
/// Returns an error when the input is not a JSON object.
pub fn parse(agent: Agent, input: &str) -> Result<HookEvent> {
    let payload: Payload = if input.trim().is_empty() {
        Payload::default()
    } else {
        serde_json::from_str(input).context("hook payload is not valid JSON")?
    };
    let transition = match agent {
        Agent::Claude => claude(&payload),
        Agent::Cursor => cursor(&payload),
        Agent::Goose => goose(&payload),
    };
    Ok(HookEvent {
        name: payload.event_name().to_owned(),
        transition,
    })
}

fn set(state: State, message: Option<&str>) -> Transition {
    Transition::Set {
        state,
        message: message.map(str::to_owned),
    }
}

fn claude(p: &Payload) -> Transition {
    match p.event_name() {
        "UserPromptSubmit" | "PostToolUse" => set(State::Running, None),
        "Stop" => set(State::Waiting, p.last_assistant_message.as_deref()),
        "Notification" => {
            let message = p.message.as_deref();
            let is_permission = p.notification_type.as_deref() == Some("permission_prompt")
                || message.is_some_and(|m| m.to_lowercase().contains("permission"));
            let state = if is_permission {
                State::Attention
            } else {
                State::Waiting
            };
            set(state, message)
        }
        "SessionStart" | "SessionEnd" => Transition::Clear,
        _ => Transition::Ignore,
    }
}

fn cursor(p: &Payload) -> Transition {
    match p.event_name() {
        "beforeSubmitPrompt" | "postToolUse" => set(State::Running, None),
        "stop" => match p.status.as_deref() {
            Some("aborted") => Transition::Clear,
            Some("error") => set(State::Attention, Some("stopped with an error")),
            _ => set(State::Waiting, None),
        },
        "sessionStart" | "sessionEnd" => Transition::Clear,
        _ => Transition::Ignore,
    }
}

fn goose(p: &Payload) -> Transition {
    match p.event_name() {
        "UserPromptSubmit" | "PostToolUse" => set(State::Running, None),
        "Stop" => set(State::Waiting, p.last_assistant_message.as_deref()),
        "SessionStart" | "SessionEnd" => Transition::Clear,
        _ => Transition::Ignore,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transition(agent: Agent, json: &str) -> Transition {
        parse(agent, json).expect("valid payload").transition
    }

    #[test]
    fn claude_permission_notification_needs_attention() {
        let t = transition(
            Agent::Claude,
            r#"{"hook_event_name":"Notification","message":"Claude needs your permission to use Bash","notification_type":"permission_prompt"}"#,
        );
        assert_eq!(
            t,
            set(
                State::Attention,
                Some("Claude needs your permission to use Bash")
            )
        );
    }

    #[test]
    fn claude_idle_notification_is_waiting() {
        let t = transition(
            Agent::Claude,
            r#"{"hook_event_name":"Notification","message":"Claude is waiting for your input","notification_type":"idle_prompt"}"#,
        );
        assert!(matches!(
            t,
            Transition::Set {
                state: State::Waiting,
                ..
            }
        ));
    }

    #[test]
    fn claude_lifecycle() {
        let prompt = r#"{"hook_event_name":"UserPromptSubmit","prompt":"hi"}"#;
        assert_eq!(transition(Agent::Claude, prompt), set(State::Running, None));
        let stop = r#"{"hook_event_name":"Stop","stop_hook_active":false}"#;
        assert_eq!(transition(Agent::Claude, stop), set(State::Waiting, None));
        let end = r#"{"hook_event_name":"SessionEnd"}"#;
        assert_eq!(transition(Agent::Claude, end), Transition::Clear);
    }

    #[test]
    fn cursor_stop_status_variants() {
        let stop = |status: &str| {
            transition(
                Agent::Cursor,
                &format!(r#"{{"hook_event_name":"stop","status":"{status}"}}"#),
            )
        };
        assert_eq!(stop("completed"), set(State::Waiting, None));
        assert_eq!(stop("aborted"), Transition::Clear);
        assert!(matches!(
            stop("error"),
            Transition::Set {
                state: State::Attention,
                ..
            }
        ));
    }

    #[test]
    fn goose_uses_event_field() {
        let t = transition(
            Agent::Goose,
            r#"{"event":"Stop","session_id":"abc","last_assistant_message":"Done."}"#,
        );
        assert_eq!(t, set(State::Waiting, Some("Done.")));
    }

    #[test]
    fn unknown_and_empty_payloads_are_ignored() {
        assert_eq!(
            transition(Agent::Goose, r#"{"event":"BeforeReadFile"}"#),
            Transition::Ignore
        );
        assert_eq!(transition(Agent::Claude, ""), Transition::Ignore);
    }

    #[test]
    fn invalid_json_is_an_error() {
        assert!(parse(Agent::Claude, "not json").is_err());
    }

    #[test]
    fn cursor_prompt_hook_must_continue() {
        assert_eq!(
            Agent::Cursor.hook_response("beforeSubmitPrompt"),
            Some(r#"{"continue":true}"#)
        );
        assert_eq!(Agent::Goose.hook_response("Stop"), None);
    }
}
