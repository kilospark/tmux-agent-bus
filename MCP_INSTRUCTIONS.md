# Agent Bus — Inter-Agent Communication

You are one of multiple AI agents working on the same project in adjacent tmux panes. The agent-bus lets you hand off work and send messages to other agents automatically.

Registration is automatic — you were assigned a unique name at startup (see "Your Identity" below).

## Tools

**`who`** — List all agents on your channel. Call this to discover other agents' names before messaging them.

**`signal_done`** — Hand off to another agent when you're done with a task.
- `next`: agent name to hand off to (call `who` first)
- `summary`: what you just finished
- `request`: what you need the next agent to do

**`send_message`** — Send a message without handing off. For questions or FYIs.
- `to`: agent name to message
- `message`: the message to send

## Broadcast

Use `"@all"` as the target in `signal_done` or `send_message` to broadcast to all other agents on your channel.

## Workflow

1. Call `who` to see other agents on the bus.
2. Do your work.
3. When done, call `signal_done` to hand off — or `send_message` for a question.
4. Do NOT ask the user to relay messages. Use these tools.

## When You Receive a Message

If your input starts with `[from <name>]:`, another agent is handing off to you. Read the request and act on it. When done, call `who` to find them, then `signal_done` to hand back.

## Coordination File

Use `CLAUDE-CODEX-CHAT.md` in the project root for longer discussions and decisions. The bus handles turn-taking; the file handles documentation.
