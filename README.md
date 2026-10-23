# tmux-agent-notifier

Tells you when a coding agent running in a tmux pane finishes or needs you.
Works with Claude Code, the Cursor CLI, and Goose via their hook systems.
The command is `tan`, short for **t**mux-**a**gent-**n**otifier.

Each agent calls `tan hook <agent>` on lifecycle events. `tan` reads the hook
payload, uses `$TMUX_PANE` to find the pane, and stores the state in tmux
itself (no daemon, no state files):

| State       | Set when                                              |
|-------------|-------------------------------------------------------|
| `running`   | you submit a prompt, or a tool finishes               |
| `waiting`   | the agent ends its turn                               |
| `attention` | a permission prompt, or an error (Cursor)             |

Cursor has no hook for approval prompts, so on `preToolUse` `tan` starts a
short-lived background watcher that polls the pane (`capture-pane`) and raises
`attention` once the "Run this …?" prompt appears.

On entering `waiting`/`attention` while you are *not* looking at the pane,
`tan` rings the pane's bell (tmux flags the window `!`), sends a desktop
notification, and shows a tmux message on other clients. Focusing the pane
clears it.

## Install

Pick one of the methods below, then [configure tmux](#tmux-configuration).

### Nix (FlakeHub)

The flake is published to [FlakeHub](https://flakehub.com/flake/ayax79/tmux_agent_notifier)
as `ayax79/tmux_agent_notifier`. CI also pushes builds to FlakeHub Cache; Nix
uses them if you are logged in to FlakeHub (`determinate-nixd login` or
`fh login`), and otherwise builds `tan` from source.

Install it into your profile:

```sh
nix profile install "https://flakehub.com/f/ayax79/tmux_agent_notifier/0.1"
tan install            # or: tan install claude goose
```

#### home-manager

Add the input with [`fh`](https://github.com/DeterminateSystems/fh):

```sh
fh add ayax79/tmux_agent_notifier
```

or by hand, then enable the module:

```nix
# flake inputs
tmux_agent_notifier.url = "https://flakehub.com/f/ayax79/tmux_agent_notifier/0.1";

# a home-manager module
{ inputs, ... }: {
  imports = [ inputs.tmux_agent_notifier.homeManagerModules.default ];
  programs.tmux-agent-notifier = {
    enable = true;
    # agents.goose.enable = false;
    # tmux.desktop = "osascript";   # "osc" (default) | "osascript" | "off"
    # tmux.statusRight = true;
  };
}
```

Leave out `inputs.nixpkgs.follows`: overriding the flake's nixpkgs changes the
derivation, so the cached build no longer matches and `tan` is built locally.

Activation runs `tan install`, which merges hook entries into
`~/.claude/settings.json` and `~/.cursor/hooks.json` (other hooks are kept) and
writes a Goose plugin to `~/.agents/plugins/tmux-agent-notifier/`. Disabling an
agent removes its hooks.

### Cargo

Requires Rust 1.85 or newer.

```sh
cargo install --git https://github.com/ayax79/tmux_agent_notifier
# or, from a checkout:
cargo install --path .

tan install            # or: tan install claude goose
```

## tmux configuration

The home-manager module adds this when `programs.tmux` is enabled; otherwise add
to `tmux.conf`:

```tmux
set -g focus-events on
set -g allow-passthrough all   # lets background panes send desktop notifications
set-hook -ga pane-focus-in 'run-shell -b "tan clear --pane #{pane_id}"'
bind-key a choose-tree -Zw -f '#{||:#{==:#{@agent_state},waiting},#{==:#{@agent_state},attention}}'
bind-key A run-shell -b 'tan next --client "#{client_name}"'
# set -ag status-right ' #(tan status)'
```

## Usage

| Command                  | Purpose                                              |
|--------------------------|------------------------------------------------------|
| `prefix a`               | tree picker of panes waiting on you                  |
| `prefix A`               | jump to the most urgent, longest-waiting pane        |
| `tan list [--json]`      | panes with agent state                               |
| `tan status`             | status line segment, e.g. `!1 ✓2 …1`                 |
| `tan set waiting --label build` | mark a pane manually (e.g. after `cargo build`) |
| `tan uninstall`          | remove all hooks                                     |

## tmux options

| Option                  | Default | Meaning                                       |
|-------------------------|---------|-----------------------------------------------|
| `@tan_bell`             | `on`    | ring the pane bell                            |
| `@tan_message`          | `on`    | `display-message` on clients elsewhere        |
| `@tan_desktop`          | `osc`   | `osc` (OSC 777 via passthrough), `osascript`, `off` |
| `@tan_icon_<state>`     | `!` `✓` `…` | `tan status` icons; `none` hides a count  |

State lives in pane options `@agent_state`, `@agent_name`, `@agent_since`,
`@agent_message`, and the window option `@agent_window_state` (most urgent pane
in the window), so themes can use them in `window-status-format`.

## Development

```sh
cargo test
./scripts/smoke-test.sh   # end-to-end against a private tmux server
```
