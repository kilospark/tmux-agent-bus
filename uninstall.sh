#!/bin/sh

REMOVED=""

echo "Uninstalling agent-bus (cleaning up all versions)..."

# --- Remove binaries (current + old "tmux-agent-bus" name) ---

for dir in /usr/local/bin "$HOME/.local/bin"; do
  for bin in tmux-agent-bus agent-bus; do
    if [ -x "$dir/${bin}" ]; then
      if [ -w "$dir" ]; then
        rm "$dir/${bin}"
        echo "Removed $dir/${bin}"
        REMOVED="${REMOVED}${bin}, "
      elif [ -e /dev/tty ] && sudo -v < /dev/tty 2>/dev/null; then
        sudo rm "$dir/${bin}" < /dev/tty
        echo "Removed $dir/${bin}"
        REMOVED="${REMOVED}${bin}, "
      else
        echo "WARNING: cannot remove $dir/${bin} (no write access)"
      fi
    fi
  done
done

# --- Remove PATH entry from shell rc ---

for rc in "$HOME/.zshrc" "$HOME/.bashrc" "$HOME/.bash_profile"; do
  if [ -f "$rc" ]; then
    for marker in "# Added by agent-bus installer" "# Added by tmux-agent-bus installer"; do
      if grep -q "$marker" "$rc" 2>/dev/null; then
        # Installer adds 3 lines: blank line, comment, export PATH=...
        if command -v python3 >/dev/null 2>&1; then
          python3 -c "
import sys
p, m = sys.argv[1], sys.argv[2]
with open(p) as f:
    lines = f.readlines()
out, i = [], 0
while i < len(lines):
    if m in lines[i]:
        if out and out[-1].strip() == '':
            out.pop()
        i += 2
    else:
        out.append(lines[i])
        i += 1
with open(p, 'w') as f:
    f.writelines(out)
" "$rc" "$marker"
        else
          sed -i.bak -e '/^$/N;/\n'"$marker"'/{N;d;}' "$rc" 2>/dev/null || \
            sed -i '' -e '/^[[:space:]]*$/{N;/'"$marker"'/{N;d;}}' "$rc"
          rm -f "${rc}.bak"
        fi
        echo "Removed PATH entry ($marker) from $rc"
        REMOVED="${REMOVED}PATH, "
      fi
    done
  fi
done

# --- Remove MCP client configs ---

remove_mcp_config() {
  config_file="$1"
  client_name="$2"

  if [ ! -f "$config_file" ]; then
    return
  fi

  found=""
  for key in agent-bus tmux-agent-bus; do
    if grep -q "\"$key\"" "$config_file" 2>/dev/null; then
      found="yes"
    fi
  done

  if [ -z "$found" ]; then
    return
  fi

  # Use python3 for safe JSON manipulation
  if command -v python3 >/dev/null 2>&1; then
    python3 -c "
import json, sys, os
p = sys.argv[1]
with open(p) as f:
    data = json.load(f)
if 'mcpServers' in data:
    data['mcpServers'].pop('agent-bus', None)
    data['mcpServers'].pop('tmux-agent-bus', None)
with open(p, 'w') as f:
    json.dump(data, f, indent=2)
    f.write('\n')
" "$config_file" 2>/dev/null && {
      echo "  $client_name: removed"
      REMOVED="${REMOVED}${client_name}, "
      return
    }
  fi

  echo "  $client_name: found but could not remove (edit $config_file manually)"
}

# Claude Code
if command -v claude >/dev/null 2>&1; then
  for name in agent-bus tmux-agent-bus; do
    if claude mcp get "$name" >/dev/null 2>&1; then
      claude mcp remove -s user "$name" 2>/dev/null && {
        echo "  Claude Code: removed \"$name\""
        REMOVED="${REMOVED}Claude Code, "
      } || echo "  Claude Code: failed to remove (try: claude mcp remove $name)"
    fi
  done
fi

OS="$(uname -s)"
case "$OS" in
  Darwin) PLATFORM="darwin" ;;
  Linux)  PLATFORM="linux" ;;
  *)      PLATFORM="unknown" ;;
esac

# Cline
if [ "$PLATFORM" = "darwin" ]; then
  remove_mcp_config "$HOME/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json" "Cline (VSCode)"
  remove_mcp_config "$HOME/Library/Application Support/Cursor/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json" "Cline (Cursor)"
elif [ "$PLATFORM" = "linux" ]; then
  XDG_CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}"
  remove_mcp_config "$XDG_CONFIG/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json" "Cline (VSCode)"
  remove_mcp_config "$XDG_CONFIG/Cursor/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json" "Cline (Cursor)"
fi

if [ "$PLATFORM" = "darwin" ]; then
  APP_SUPPORT="$HOME/Library/Application Support"
  remove_mcp_config "$APP_SUPPORT/Claude/claude_desktop_config.json" "Claude Desktop"
  remove_mcp_config "$APP_SUPPORT/ChatGPT/mcp.json" "ChatGPT Desktop"
  remove_mcp_config "$HOME/.cursor/mcp.json" "Cursor / Agent"
  remove_mcp_config "$HOME/.codeium/windsurf/mcp_config.json" "Windsurf"
elif [ "$PLATFORM" = "linux" ]; then
  XDG_CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}"
  remove_mcp_config "$XDG_CONFIG/Claude/claude_desktop_config.json" "Claude Desktop"
  remove_mcp_config "$XDG_CONFIG/chatgpt/mcp.json" "ChatGPT Desktop"
  remove_mcp_config "$HOME/.cursor/mcp.json" "Cursor / Agent"
  remove_mcp_config "$HOME/.codeium/windsurf/mcp_config.json" "Windsurf"
fi

# --- Remove from project-level MCP configs ---

echo ""
echo "Scanning for project-level MCP configs..."
PROJECT_CONFIGS=""

# Known project-level MCP config patterns:
#   .mcp.json              (Claude Code)
#   .cursor/mcp.json       (Cursor)
#   .windsurf/mcp.json     (Windsurf)
#   .vscode/cline_mcp_settings.json (Cline)
PROJECT_CONFIGS="$(find "$HOME" -maxdepth 6 \
  \( -name .mcp.json -o -path '*/.cursor/mcp.json' -o -path '*/.windsurf/mcp.json' -o -path '*/.vscode/cline_mcp_settings.json' \) \
  -not -path '*/node_modules/*' \
  -not -path '*/.git/*' \
  -not -path '*/Library/Application Support/*' \
  2>/dev/null | xargs grep -l '"tmux-agent-bus"\|"agent-bus"' 2>/dev/null || true)"

if [ -n "$PROJECT_CONFIGS" ]; then
  echo "$PROJECT_CONFIGS" | while read -r pconfig; do
    remove_mcp_config "$pconfig" "project ($pconfig)"
  done
else
  echo "  No project-level configs found."
fi

# Codex
if command -v codex >/dev/null 2>&1; then
  for name in agent-bus tmux-agent-bus; do
    if codex mcp list 2>/dev/null | grep -q "$name"; then
      codex mcp remove "$name" 2>/dev/null && {
        echo "  Codex: removed \"$name\""
        REMOVED="${REMOVED}Codex, "
      } || echo "  Codex: failed to remove (try: codex mcp remove $name)"
    fi
  done
fi

# --- Remove channel data ---

if [ -d "$HOME/.agent-bus" ]; then
  rm -rf "$HOME/.agent-bus"
  echo "Removed ~/.agent-bus"
  REMOVED="${REMOVED}data, "
fi

echo ""
if [ -z "$REMOVED" ]; then
  echo "Nothing to uninstall — agent-bus was not found."
else
  echo "Done! agent-bus has been uninstalled."
fi
