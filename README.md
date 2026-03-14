# agentbus

MCP server built on [tmux](https://github.com/tmux/tmux) that lets AI agents (Claude Code, Codex, Copilot) talk to each other across tmux panes. Requires tmux.

## How it works

Each agent spawns its own agentbus MCP server. On startup, the server:
1. Detects which tmux pane and session it's in
2. Detects the agent type (claude/codex/copilot) from the process tree
3. Auto-registers with a human-readable name (`claude-1`, `codex-1`, etc.)
4. Sets a tmux pane option (`@agent-name`) — this is the registration
5. Cleans up pane options on exit

Agents communicate by calling `signal_done` or `send_message`, which injects text into the target agent's tmux pane via `tmux send-keys`.

## Install

```bash
curl -fsSL https://agentbus.site/install | sh
```

This downloads a single binary, adds it to your PATH, and auto-configures any detected MCP clients (Claude Code, Codex, Claude Desktop, Cursor, etc.).

### Uninstall

```bash
curl -fsSL https://agentbus.site/uninstall | sh
```

## tmux setup

Pane borders are enabled automatically when an agent registers. The server sets `pane-border-status` and `pane-border-format` on the current window if not already configured.

To set your own format globally, add to `~/.tmux.conf`:

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

Use `"@all"` as the target to broadcast to all agents on the channel.

## Architecture

- **Channel** = tmux session. Agents in the same session see each other.
- **Registration** = tmux pane option (`@agent-name`). No JSON registry — tmux is the source of truth.
- **Routing** = `tmux list-panes` to find panes by `@agent-name`, then `tmux send-keys` to deliver.
- **Cleanup** = pane options cleared on exit. No stale entries possible.

