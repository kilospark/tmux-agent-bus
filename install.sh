#!/bin/sh
set -e

REPO="kilospark/tmux-agent-bus"
BINARY="tmux-agent-bus"

# Use INSTALL_DIR if set, otherwise default to /usr/local/bin
if [ -z "$INSTALL_DIR" ]; then
  INSTALL_DIR="/usr/local/bin"
fi

# Detect OS and architecture
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Darwin) PLATFORM="darwin" ;;
  Linux)  PLATFORM="linux" ;;
  *)      echo "Unsupported OS: $OS"; exit 1 ;;
esac

case "$ARCH" in
  arm64|aarch64) ARCH_NAME="arm64" ;;
  x86_64|amd64)  ARCH_NAME="x64" ;;
  *)              echo "Unsupported architecture: $ARCH"; exit 1 ;;
esac

ASSET="${BINARY}-${PLATFORM}-${ARCH_NAME}"

# Get latest release tag if not specified
if [ -z "$VERSION" ]; then
  VERSION="$(curl -fsSL "https://api.github.com/repos/${REPO}/releases/latest" | grep '"tag_name"' | cut -d'"' -f4)"
fi

if [ -z "$VERSION" ]; then
  echo "Failed to determine latest version"
  exit 1
fi

URL="https://github.com/${REPO}/releases/download/${VERSION}/${ASSET}.tar.gz"

echo "Installing ${BINARY} ${VERSION} (${PLATFORM}/${ARCH_NAME})..."

TMPDIR="$(mktemp -d)"
trap 'rm -rf "$TMPDIR"' EXIT

curl -fsSL "$URL" | tar xz -C "$TMPDIR"

mkdir -p "$INSTALL_DIR"

if [ -w "$INSTALL_DIR" ]; then
  mv "$TMPDIR/${ASSET}" "${INSTALL_DIR}/${BINARY}"
elif [ -e /dev/tty ]; then
  echo "Need admin access to install to ${INSTALL_DIR}."
  sudo mv "$TMPDIR/${ASSET}" "${INSTALL_DIR}/${BINARY}" < /dev/tty
else
  INSTALL_DIR="$HOME/.local/bin"
  mkdir -p "$INSTALL_DIR"
  mv "$TMPDIR/${ASSET}" "${INSTALL_DIR}/${BINARY}"
fi

chmod +x "${INSTALL_DIR}/${BINARY}"

echo "Installed ${BINARY} to ${INSTALL_DIR}/${BINARY}"

# Update stale copies in other known locations
for other_dir in /usr/local/bin "$HOME/.local/bin"; do
  if [ "$other_dir" != "$INSTALL_DIR" ]; then
    if [ -x "$other_dir/${BINARY}" ]; then
      if [ -w "$other_dir" ]; then
        cp "${INSTALL_DIR}/${BINARY}" "$other_dir/${BINARY}"
        echo "Updated stale copy at ${other_dir}/${BINARY}"
      elif sudo -n true 2>/dev/null; then
        sudo cp "${INSTALL_DIR}/${BINARY}" "$other_dir/${BINARY}"
        echo "Updated stale copy at ${other_dir}/${BINARY}"
      else
        echo "WARNING: stale copy at ${other_dir}/${BINARY} (update manually or remove)"
      fi
    fi
  fi
done

# Auto-add install dir to PATH in shell rc if needed
case ":$PATH:" in
  *":${INSTALL_DIR}:"*) ;;
  *)
    PATH_LINE="export PATH=\"${INSTALL_DIR}:\$PATH\""
    if [ -f "$HOME/.zshrc" ]; then
      RC_FILE="$HOME/.zshrc"
    elif [ -f "$HOME/.bashrc" ]; then
      RC_FILE="$HOME/.bashrc"
    elif [ -f "$HOME/.bash_profile" ]; then
      RC_FILE="$HOME/.bash_profile"
    else
      RC_FILE=""
    fi
    if [ -n "$RC_FILE" ]; then
      if ! grep -q "${INSTALL_DIR}" "$RC_FILE" 2>/dev/null; then
        echo "" >> "$RC_FILE"
        echo "# Added by tmux-agent-bus installer" >> "$RC_FILE"
        echo "$PATH_LINE" >> "$RC_FILE"
        echo "Added ${INSTALL_DIR} to PATH in ${RC_FILE}"
      fi
    else
      echo "WARNING: ${INSTALL_DIR} is not in your PATH. Add it with:"
      echo "  $PATH_LINE"
    fi
    export PATH="${INSTALL_DIR}:$PATH"
    ;;
esac

# --- Configure MCP clients ---

BINARY_PATH="${INSTALL_DIR}/${BINARY}"
CONFIGURED=""

# Add agent-bus to an MCP config file
# Usage: add_mcp_config <config_file> <client_name>
add_mcp_config() {
  config_file="$1"
  client_name="$2"

  if [ ! -f "$config_file" ]; then
    return
  fi

  if grep -q '"agent-bus"' "$config_file" 2>/dev/null; then
    echo "  $client_name: already configured"
    CONFIGURED="${CONFIGURED}${client_name}, "
    return
  fi

  content="$(cat "$config_file")"

  escaped_path="$(echo "$BINARY_PATH" | sed 's/[\/&]/\\&/g')"

  if echo "$content" | grep -q '"mcpServers"'; then
    updated="$(echo "$content" | sed 's/"mcpServers"[[:space:]]*:[[:space:]]*{/"mcpServers": { "agent-bus": { "command": "'"$escaped_path"'" },/')"
  else
    updated="$(echo "$content" | sed 's/^{/{ "mcpServers": { "agent-bus": { "command": "'"$escaped_path"'" } },/')"
  fi

  echo "$updated" > "$config_file"
  echo "  $client_name: configured"
  CONFIGURED="${CONFIGURED}${client_name}, "
}

echo ""
echo "Configuring MCP clients..."

# Claude Code (uses CLI, not a config file)
if command -v claude >/dev/null 2>&1; then
  if claude mcp get agent-bus >/dev/null 2>&1; then
    echo "  Claude Code: already configured"
    CONFIGURED="${CONFIGURED}Claude Code, "
  else
    claude mcp add -s user agent-bus "$BINARY_PATH" 2>/dev/null && {
      echo "  Claude Code: configured"
      CONFIGURED="${CONFIGURED}Claude Code, "
    } || echo "  Claude Code: failed to configure (try: claude mcp add -s user agent-bus $BINARY_PATH)"
  fi
fi

# Cline (VSCode extension - check both Code and Cursor hosts)
if [ "$PLATFORM" = "darwin" ]; then
  add_mcp_config "$HOME/Library/Application Support/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json" "Cline (VSCode)"
  add_mcp_config "$HOME/Library/Application Support/Cursor/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json" "Cline (Cursor)"
elif [ "$PLATFORM" = "linux" ]; then
  XDG_CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}"
  add_mcp_config "$XDG_CONFIG/Code/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json" "Cline (VSCode)"
  add_mcp_config "$XDG_CONFIG/Cursor/User/globalStorage/saoudrizwan.claude-dev/settings/cline_mcp_settings.json" "Cline (Cursor)"
fi

# macOS config paths
if [ "$PLATFORM" = "darwin" ]; then
  APP_SUPPORT="$HOME/Library/Application Support"

  add_mcp_config "$APP_SUPPORT/Claude/claude_desktop_config.json" "Claude Desktop"
  add_mcp_config "$APP_SUPPORT/ChatGPT/mcp.json" "ChatGPT Desktop"

  # Cursor
  add_mcp_config "$HOME/.cursor/mcp.json" "Cursor"

  # Windsurf
  add_mcp_config "$HOME/.codeium/windsurf/mcp_config.json" "Windsurf"
fi

# Linux config paths
if [ "$PLATFORM" = "linux" ]; then
  XDG_CONFIG="${XDG_CONFIG_HOME:-$HOME/.config}"

  add_mcp_config "$XDG_CONFIG/Claude/claude_desktop_config.json" "Claude Desktop"
  add_mcp_config "$XDG_CONFIG/chatgpt/mcp.json" "ChatGPT Desktop"

  # Cursor
  add_mcp_config "$HOME/.cursor/mcp.json" "Cursor"

  # Windsurf
  add_mcp_config "$HOME/.codeium/windsurf/mcp_config.json" "Windsurf"
fi

# Codex (uses CLI, not a config file)
if command -v codex >/dev/null 2>&1; then
  if codex mcp list 2>/dev/null | grep -q 'agent-bus'; then
    echo "  Codex: already configured"
    CONFIGURED="${CONFIGURED}Codex, "
  else
    codex mcp add agent-bus -- "$BINARY_PATH" 2>/dev/null && {
      echo "  Codex: configured"
      CONFIGURED="${CONFIGURED}Codex, "
    } || echo "  Codex: failed to configure (try: codex mcp add agent-bus -- $BINARY_PATH)"
  fi
fi

if [ -z "$CONFIGURED" ]; then
  echo "  No MCP clients detected. Add manually to your client config:"
  echo ""
  echo '  { "mcpServers": { "agent-bus": { "command": "'"$BINARY_PATH"'" } } }'
else
  echo ""
  echo "Done! Restart your MCP client to start using agent-bus."
fi
