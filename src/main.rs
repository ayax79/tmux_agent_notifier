mod agent;
mod approval;
mod install;
mod notify;
mod pane;
mod query;
mod state;
mod tmux;

use std::env;
use std::io::{self, Read};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::agent::{Agent, Transition};
use crate::state::State;

/// Surface coding agent activity (Claude Code, Cursor, Goose) in tmux.
#[derive(Parser)]
#[command(name = "tan", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Handle an agent hook event (JSON on stdin). Always exits 0.
    Hook { agent: Agent },
    /// Set a pane's state manually, e.g. after a long build.
    Set {
        state: State,
        /// Pane id; defaults to $TMUX_PANE.
        #[arg(long)]
        pane: Option<String>,
        #[arg(long, default_value = "agent")]
        label: String,
        #[arg(long)]
        message: Option<String>,
    },
    /// Clear a pane's waiting/attention state.
    Clear {
        /// Pane id; defaults to $TMUX_PANE.
        #[arg(long)]
        pane: Option<String>,
        /// Also clear the running state.
        #[arg(long)]
        force: bool,
    },
    /// Print a summary for the tmux status line.
    Status,
    /// List panes that have an agent state.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Switch to the agent pane that most needs attention.
    Next {
        /// Client to switch, e.g. "#{client_name}" from a key binding.
        #[arg(long)]
        client: Option<String>,
    },
    /// Register hooks with agents (all agents when none are given).
    Install {
        agents: Vec<Agent>,
        /// Binary path written into the hook commands.
        #[arg(long, default_value = "tan")]
        bin: String,
    },
    /// Remove hooks from agents (all agents when none are given).
    Uninstall { agents: Vec<Agent> },
    /// Internal: poll a pane for an approval prompt.
    #[command(hide = true)]
    WatchApproval {
        #[arg(long)]
        pane: String,
        #[arg(long)]
        label: String,
        #[arg(long)]
        token: String,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Command::Hook { agent } = cli.command {
        // A failing hook can block or spam the agent, so errors only go to stderr.
        hook(agent);
        return ExitCode::SUCCESS;
    }
    match run(cli.command) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("tan: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn hook(agent: Agent) {
    let mut input = String::new();
    let event = io::stdin()
        .read_to_string(&mut input)
        .context("failed to read hook payload")
        .and_then(|_| agent::parse(agent, &input));
    let name = match &event {
        Ok(event) => event.name.as_str(),
        Err(_) => "",
    };
    if let Some(response) = agent.hook_response(name) {
        println!("{response}");
    }
    let result = event.and_then(|event| match env::var("TMUX_PANE") {
        Ok(pane) if !pane.is_empty() => {
            pane::apply(&pane, agent.display_name(), &event.transition)?;
            if agent.may_prompt_after(&event.name) {
                approval::spawn(&pane, agent.display_name())?;
            }
            Ok(())
        }
        _ => Ok(()),
    });
    if let Err(err) = result {
        eprintln!("tan: {err:#}");
    }
}

fn target_pane(pane: Option<String>) -> Result<String> {
    pane.or_else(|| env::var("TMUX_PANE").ok().filter(|p| !p.is_empty()))
        .context("no --pane given and $TMUX_PANE is not set")
}

fn agents_or_all(agents: Vec<Agent>) -> Vec<Agent> {
    if agents.is_empty() {
        Agent::ALL.to_vec()
    } else {
        agents
    }
}

fn run(command: Command) -> Result<()> {
    match command {
        Command::Hook { .. } => unreachable!("handled in main"),
        Command::Set {
            state,
            pane,
            label,
            message,
        } => {
            let transition = Transition::Set { state, message };
            pane::apply(&target_pane(pane)?, &label, &transition)
        }
        Command::Clear { pane, force } => pane::clear(&target_pane(pane)?, force),
        Command::Status => {
            println!("{}", query::status()?);
            Ok(())
        }
        Command::List { json } => {
            let out = query::list(json)?;
            if !out.is_empty() {
                println!("{out}");
            }
            Ok(())
        }
        Command::Next { client } => query::next(client.as_deref()),
        Command::Install { agents, bin } => {
            for agent in agents_or_all(agents) {
                let path = install::install(agent, &bin)?;
                println!("{}: installed hooks in {}", agent.as_str(), path.display());
            }
            Ok(())
        }
        Command::Uninstall { agents } => {
            for agent in agents_or_all(agents) {
                match install::uninstall(agent)? {
                    Some(path) => {
                        println!("{}: removed hooks from {}", agent.as_str(), path.display())
                    }
                    None => println!("{}: nothing to remove", agent.as_str()),
                }
            }
            Ok(())
        }
        Command::WatchApproval { pane, label, token } => approval::watch(&pane, &label, &token),
    }
}
