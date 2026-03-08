#!/usr/bin/env node

import { readFileSync, writeFileSync, mkdirSync, existsSync, appendFileSync } from "fs";
import { execSync } from "child_process";
import { homedir } from "os";
import { join, dirname } from "path";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const INSTRUCTIONS = readFileSync(join(__dirname, "MCP_INSTRUCTIONS.md"), "utf-8");

const BUS_DIR = join(homedir(), ".agent-bus");
const CHANNELS_DIR = join(BUS_DIR, "channels");
const LOG_PATH = join(BUS_DIR, "history.jsonl");

function channelPath(channel) {
  return join(CHANNELS_DIR, `${channel}.json`);
}

function loadChannel(channel) {
  mkdirSync(CHANNELS_DIR, { recursive: true });
  const p = channelPath(channel);
  if (!existsSync(p)) {
    return { agents: {} };
  }
  return JSON.parse(readFileSync(p, "utf-8"));
}

function saveChannel(channel, data) {
  mkdirSync(CHANNELS_DIR, { recursive: true });
  writeFileSync(channelPath(channel), JSON.stringify(data, null, 2) + "\n");
}

// Returns { session, pane } — detected once at startup since process tree is stable
function detectPane() {
  try {
    const paneList = execSync(
      "tmux list-panes -a -F '#{pane_pid} #{session_name}:#{window_index}.#{pane_index} #{session_name}'",
      { timeout: 3000 }
    ).toString().trim().split("\n");

    const paneMap = {};
    for (const line of paneList) {
      const parts = line.split(" ");
      paneMap[parts[0]] = { pane: parts[1], session: parts[2] };
    }

    let pid = process.pid;
    while (pid && pid !== 1) {
      if (paneMap[String(pid)]) {
        return paneMap[String(pid)];
      }
      try {
        pid = parseInt(
          execSync(`ps -o ppid= -p ${pid}`, { timeout: 1000 }).toString().trim()
        );
      } catch {
        break;
      }
    }
  } catch {}
  return null;
}

function sendToPane(pane, message) {
  try {
    const sanitized = message.replace(/\n+/g, " ").trim();
    execSync(`tmux send-keys -t ${JSON.stringify(pane)} -l ${JSON.stringify(sanitized)}`, { timeout: 5000 });
    execSync(`tmux send-keys -t ${JSON.stringify(pane)} Enter`, { timeout: 5000 });
    return { success: true };
  } catch (err) {
    return { success: false, error: err.message };
  }
}

function logHandoff(record) {
  const entry = JSON.stringify({ ts: new Date().toISOString(), ...record });
  appendFileSync(LOG_PATH, entry + "\n");
}

function getAgents(channel) {
  return loadChannel(channel).agents || {};
}

// Detect once at startup — stable for lifetime of this MCP server instance
const myLocation = detectPane();
const myChannel = myLocation?.session || null;
const myPane = myLocation?.pane || null;

if (myChannel) {
  console.error(`agent-bus: detected channel "${myChannel}" pane ${myPane}`);
} else {
  console.error("agent-bus: WARNING — could not detect tmux pane");
}

const server = new McpServer({
  name: "agent-bus",
  version: "0.4.0",
}, {
  instructions: INSTRUCTIONS,
});

server.tool(
  "who",
  "List all agents registered on your channel (tmux session). No parameters needed — channel is auto-detected.",
  {},
  async () => {
    if (!myChannel) {
      return { content: [{ type: "text", text: "Not running inside tmux — cannot detect channel." }], isError: true };
    }
    const agents = getAgents(myChannel);
    const names = Object.keys(agents);
    if (names.length === 0) {
      return { content: [{ type: "text", text: `No agents on channel "${myChannel}" yet. Be the first — call register.` }] };
    }
    const lines = names.map((n) => `- ${n} (pane ${agents[n].pane})`).join("\n");
    return { content: [{ type: "text", text: `Channel "${myChannel}" agents:\n${lines}` }] };
  }
);

server.tool(
  "register",
  "Register this agent with the bus. Auto-detects your tmux session (channel) and pane. Pick a unique name — call 'who' first to see what's taken.",
  {
    name: z.string().describe("Your unique agent name, e.g. 'claude-1', 'codex-alpha'. Must be unique on the channel."),
  },
  async ({ name }) => {
    if (!myChannel) {
      return { content: [{ type: "text", text: "Not running inside tmux — cannot detect channel/pane." }], isError: true };
    }
    const channelData = loadChannel(myChannel);

    if (channelData.agents[name] && channelData.agents[name].pane !== myPane) {
      return { content: [{ type: "text", text: `Name "${name}" is already taken by pane ${channelData.agents[name].pane} on channel "${myChannel}". Pick a different name.` }], isError: true };
    }
    channelData.agents[name] = { pane: myPane };
    saveChannel(myChannel, channelData);
    const others = Object.keys(channelData.agents).filter((k) => k !== name);
    return {
      content: [{ type: "text", text: `Registered as "${name}" on channel "${myChannel}" (pane ${myPane}). Other agents: ${others.length ? others.join(", ") : "none yet"}.` }],
    };
  }
);

server.tool(
  "signal_done",
  "Signal that you are done with your task and hand off to another agent on your channel.",
  {
    from: z.string().describe("Your registered agent name"),
    next: z.string().describe("Which agent should go next"),
    summary: z.string().describe("What you just finished"),
    request: z.string().describe("What you need the next agent to do"),
  },
  async ({ from, next, summary, request }) => {
    if (!myChannel) {
      return { content: [{ type: "text", text: "Not running inside tmux." }], isError: true };
    }
    const agents = getAgents(myChannel);
    const pane = agents[next]?.pane;
    if (!pane) {
      const available = Object.keys(agents);
      return { content: [{ type: "text", text: `Unknown agent "${next}" on channel "${myChannel}". Registered: ${available.length ? available.join(", ") : "none"}.` }], isError: true };
    }
    const message = `[from ${from}]: ${summary} Request: ${request}`;
    const result = sendToPane(pane, message);
    logHandoff({ type: "signal_done", channel: myChannel, from, to: next, summary, request });
    if (!result.success) {
      return { content: [{ type: "text", text: `Failed to reach ${next}: ${result.error}` }], isError: true };
    }
    return { content: [{ type: "text", text: `Handed off to ${next} on channel "${myChannel}" (pane ${pane}).` }] };
  }
);

server.tool(
  "send_message",
  "Send a message to another agent on your channel without handing off. Use for mid-task questions or FYIs.",
  {
    from: z.string().describe("Your registered agent name"),
    to: z.string().describe("Which agent to message"),
    message: z.string().describe("The message to send"),
  },
  async ({ from, to, message }) => {
    if (!myChannel) {
      return { content: [{ type: "text", text: "Not running inside tmux." }], isError: true };
    }
    const agents = getAgents(myChannel);
    const pane = agents[to]?.pane;
    if (!pane) {
      const available = Object.keys(agents);
      return { content: [{ type: "text", text: `Unknown agent "${to}" on channel "${myChannel}". Registered: ${available.length ? available.join(", ") : "none"}.` }], isError: true };
    }
    const fullMessage = `[message from ${from}]: ${message}`;
    const result = sendToPane(pane, fullMessage);
    logHandoff({ type: "send_message", channel: myChannel, from, to, message });
    if (!result.success) {
      return { content: [{ type: "text", text: `Failed to reach ${to}: ${result.error}` }], isError: true };
    }
    return { content: [{ type: "text", text: `Message sent to ${to} on channel "${myChannel}" (pane ${pane}).` }] };
  }
);

async function main() {
  const transport = new StdioServerTransport();
  await server.connect(transport);
  console.error("agent-bus MCP server running on stdio");
}

main().catch((err) => {
  console.error("Fatal:", err);
  process.exit(1);
});
