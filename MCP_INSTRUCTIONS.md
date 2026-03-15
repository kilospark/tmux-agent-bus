# AgentBus — Inter-Agent Communication

You are one of multiple AI agents working on the same project in adjacent tmux panes. agentbus lets you hand off work and send messages to other agents automatically.

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

## Message Types

Use `kind` in `send_message` to indicate intent:
- `"request"` — you expect a response. The recipient will be reminded until they reply.
- `"response"` — you are answering a prior request. Use `reply_to` with the message ID.
- `"fyi"` — informational, no response expected. This is the default.

`signal_done` is always tracked as a handoff (similar to request).

When a tracked message (request/handoff) is sent, the response includes a `msg_id`. The recipient sees this ID in their pending warnings. To resolve it, include `reply_to: "<msg_id>"` in your response.

## Workflow

1. Call `who` to see other agents on the bus.
2. Do your work.
3. When done, call `signal_done` to hand off — or `send_message` for a question.
4. Do NOT ask the user to relay messages. Use these tools.

## When You Receive a Message

If your input starts with `[from <name>]:`, another agent is handing off to you. Read the request and act on it.

**You MUST reply using the bus tools.** When done, call `who` to find them, then `signal_done` or `send_message` to respond. NEVER just output your response as text — the other agent cannot see your text output. The ONLY way to communicate with another agent is through `signal_done` or `send_message`. If you don't use these tools, your response is lost.

If you see a pending message warning in a tool response, you have unanswered requests. Respond to them using `reply_to` with the message ID shown.

## Coordination File

Use `CLAUDE-CODEX-CHAT.md` in the project root for longer discussions and decisions. The bus handles turn-taking; the file handles documentation.
