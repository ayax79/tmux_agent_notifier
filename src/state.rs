//! Agent states tracked per tmux pane.

use std::fmt;

/// What an agent in a pane is doing. Ordered by urgency so the most urgent
/// pane in a window wins when aggregating.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, clap::ValueEnum, serde::Serialize,
)]
#[serde(rename_all = "lowercase")]
pub enum State {
    /// The agent is working.
    Running,
    /// The agent finished its turn and is waiting for the next prompt.
    Waiting,
    /// The agent is blocked on the user (permission prompt, error).
    Attention,
}

impl State {
    pub const ALL: [State; 3] = [State::Running, State::Waiting, State::Attention];

    pub fn as_str(self) -> &'static str {
        match self {
            State::Running => "running",
            State::Waiting => "waiting",
            State::Attention => "attention",
        }
    }

    /// Parses the value stored in a tmux option; unset or unknown values are `None`.
    pub fn from_option(value: &str) -> Option<State> {
        State::ALL.into_iter().find(|s| s.as_str() == value.trim())
    }

    /// Whether the user should be told about this state.
    pub fn needs_user(self) -> bool {
        matches!(self, State::Waiting | State::Attention)
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_values_round_trip() {
        for state in State::ALL {
            assert_eq!(State::from_option(state.as_str()), Some(state));
        }
        assert_eq!(State::from_option(""), None);
        assert_eq!(State::from_option("bogus"), None);
    }

    #[test]
    fn attention_is_most_urgent() {
        let states = [
            Some(State::Running),
            None,
            Some(State::Attention),
            Some(State::Waiting),
        ];
        assert_eq!(states.into_iter().flatten().max(), Some(State::Attention));
    }
}
