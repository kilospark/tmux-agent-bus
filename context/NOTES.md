# agentbus — Development Notes

## History
- Started as Node.js (240 lines), ported to Rust (~300 lines)
- Node.js version preserved at git tag `node.js-eol`
- First Rust release: v0.1.0
- v0.1.1: auto-enable pane borders per-window, sudo fallback in install, uninstall.sh
- v0.2.x: renamed back from "tmux-agent-bus" to "agent-bus", removed history.jsonl logging
- v0.3.x: renamed from "agent-bus" to "agentbus"

## Key Design Decisions

### Why tmux send-keys (not sockets/HTTP)
Agents run as long-lived chat sessions in tmux panes. `tmux send-keys` injects text directly into the pane's stdin — the agent sees it as user input. No protocol needed on the agent side.

### Why @agent-name pane option (not pane title)
Claude Code continuously overwrites `pane_title` with its spinner. Custom pane option `@agent-name` survives this. Display via `pane-border-format`:
```
set -g pane-border-format " #{@agent-name} | #{pane_title} "
```

### Why raw JSON-RPC (no MCP SDK)
Following webact's pattern. The MCP protocol is just newline-delimited JSON-RPC over stdio. Three methods: `initialize`, `tools/list`, `tools/call`. No SDK needed for this.

### Ack mechanism
`tmux send-keys` succeeding only means tmux accepted the command — not that the target agent processed it. Enter key is flaky. Fix:
1. Snapshot target pane via `tmux capture-pane -p`
2. Send message + Enter in single tmux invocation (`;` chains commands)
3. Poll pane for 1.5s checking for content change
4. If no change, retry once
5. If still no change, report delivery failure

### Process tree walking
MCP server is spawned by the agent process. Walk `ps -o ppid=` from our PID up to find:
- A PID matching `tmux list-panes -a` output → gives us pane ID + session
- A process named `claude`/`codex`/`copilot` → gives us agent type

## Known Issues
- Enter key delivery is intermittent — mitigated by ack+retry but not eliminated
- Each agent session restart re-spawns the MCP server → new registration (old one cleaned up by Drop)
- No hot-reload: code changes require agent session restart

## File Layout
```
Cargo.toml              # Rust project, edition 2021
src/main.rs             # Everything: MCP server, registration, tools
tools.json              # Tool definitions (embedded via include_str!)
MCP_INSTRUCTIONS.md     # Agent instructions (embedded via include_str!)
install.sh              # Download binary + configure MCP clients
uninstall.sh            # Full cleanup: binaries, PATH, MCP configs, old Node.js version, data
bump-version.sh         # Semantic version bump in Cargo.toml
.github/workflows/      # Release CI: 4 targets on tag push
www/index.html           # Homepage (Vercel)
vercel.json             # Points Vercel to www/
context/                # This folder — dev notes
```

## Deployment
- **Binary releases**: Push `v*` tag → GitHub Actions builds macOS (arm64/x64) + Linux (x64/arm64)
- **Release process**: Bump version in Cargo.toml, commit, `git tag v<version> && git push origin v<version>`
- **Homepage**: Vercel auto-deploys from main, or `vercel --prod` manually
- **Install**: `curl -fsSL https://agentbus.site/install | sh`
- **Uninstall**: `curl -fsSL https://agentbus.site/uninstall | sh`

## MCP Server Name
- Registered as `agentbus` in all MCP clients
- Uninstall script cleans up old names (`agent-bus`, `tmux-agent-bus`) and current name
