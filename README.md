# tmux-agent-bus

MCP server that lets AI agents (Claude Code, Codex, Copilot) talk to each other across tmux panes.

## How it works

Each agent spawns its own tmux-agent-bus MCP server. On startup, the server:
1. Detects which tmux pane and session it's in
2. Detects the agent type (claude/codex/copilot) from the process tree
3. Auto-registers with a human-readable name (`claude-1`, `codex-1`, etc.)
4. Sets a tmux pane option (`@agent-name`) so you can see who's who in pane borders
5. Cleans up on exit

Agents communicate by calling `signal_done` or `send_message`, which injects text into the target agent's tmux pane via `tmux send-keys`.

## Install

```bash
# Claude Code
claude mcp add -s user agent-bus node /path/to/agent-bus/index.js

# Codex
codex mcp add agent-bus -- node /path/to/agent-bus/index.js
```

## tmux setup

Show agent names in pane borders (uses a custom pane option since Claude Code overwrites `pane_title`):

```bash
tmux set -g pane-border-format " #{@agent-name} | #{pane_title} "
tmux set -g pane-border-status top
```

Add to `~/.tmux.conf` to persist:

```
set -g pane-border-format " #{@agent-name} | #{pane_title} "
set -g pane-border-status top
```

## Tools

| Tool | Description |
|------|-------------|
| `who` | List all agents on your channel (tmux session) |
| `signal_done` | Hand off to another agent with summary and request |
| `send_message` | Send a message without handing off |

## Architecture

- **Channel** = tmux session. Agents in the same session see each other.
- **Registration** = automatic. Name assigned at startup, pane option set.
- **Routing** = `tmux send-keys`. Messages typed into the target pane's stdin.
- **State** = one JSON file per channel at `~/.agent-bus/channels/<session>.json`
- **Cleanup** = agents unregister on exit. No stale entries.

## Files

```
~/.agent-bus/
  channels/
    0.json          # channel registry (one per tmux session)
  history.jsonl     # log of all handoffs and messages
```
