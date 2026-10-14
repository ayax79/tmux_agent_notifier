#!/usr/bin/env bash
# Exercises `tan` against a throwaway tmux server on a private socket.
set -euo pipefail

cd "$(dirname "$0")/.."
cargo build -q
TAN=./target/debug/tan
SOCKET=tan-smoke-$$
t() { tmux -L "$SOCKET" -f /dev/null "$@"; }
trap 't kill-server 2>/dev/null || true' EXIT

t new-session -d -s work -x 120 -y 40 'sleep 600'
t new-window -t work 'sleep 600'
t split-window -t work:1 'sleep 600'
export TMUX="$(t display -p '#{socket_path}'),0,0"

p0=$(t list-panes -t work:0 -F '#{pane_id}')
p1=$(t list-panes -t work:1 -F '#{pane_id}' | head -1)
p2=$(t list-panes -t work:1 -F '#{pane_id}' | tail -1)

hook() { TMUX_PANE=$1 "$TAN" hook "$2"; }

echo '== claude: prompt, then permission prompt'
echo '{"hook_event_name":"UserPromptSubmit","prompt":"hi"}' | hook "$p0" claude
echo '{"hook_event_name":"Notification","message":"Claude needs your permission to use Bash","notification_type":"permission_prompt"}' | hook "$p0" claude

echo '== cursor: stop (stdout should be {})'
echo '{"hook_event_name":"stop","status":"completed"}' | hook "$p1" cursor

echo '== goose: stop (stdout should be empty)'
echo '{"event":"Stop","session_id":"x","last_assistant_message":"Done. Tests pass; #{pane_id}"}' | hook "$p2" goose

echo '== list'
"$TAN" list
echo "== status: [$("$TAN" status)]"
echo '== windows'
t list-windows -t work -F '#{window_index} bell=#{window_bell_flag} agent=#{@agent_window_state}'

echo '== clear cursor pane; window 1 should stay waiting (goose)'
"$TAN" clear --pane "$p1"
"$TAN" list
t list-windows -t work -F '#{window_index} agent=#{@agent_window_state}'

echo '== permission granted, claude runs a tool again'
echo '{"hook_event_name":"PostToolUse","tool_name":"Bash"}' | hook "$p0" claude
"$TAN" list --json

echo '== no $TMUX_PANE and invalid JSON both exit 0'
echo '{"hook_event_name":"Stop"}' | env -u TMUX_PANE "$TAN" hook claude && echo ok
echo 'nope' | hook "$p0" cursor && echo ok
