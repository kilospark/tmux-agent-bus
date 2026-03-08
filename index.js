#!/usr/bin/env node

import { readFileSync, writeFileSync, mkdirSync, existsSync, appendFileSync } from "fs";
import { execSync } from "child_process";
import { randomBytes } from "crypto";
import { homedir } from "os";
import { join, dirname } from "path";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));

const BUS_DIR = join(homedir(), ".agent-bus");
const CHANNELS_DIR = join(BUS_DIR, "channels");
const LOG_PATH = join(BUS_DIR, "history.jsonl");

function channelPath(channel) {
  return join(CHANNELS_DIR, `${channel}.json`);
}

function loadChannel(channel) {
  mkdirSync(CHANNELS_DIR, { recursive: true });
  const p = channelPath(channel);
  if (!existsSync(p)) return { agents: {} };
  const data = JSON.parse(readFileSync(p, "utf-8"));
  if (!data.agents) data.agents = {};
  return data;
}

function saveChannel(channel, data) {
  mkdirSync(CHANNELS_DIR, { recursive: true });
  writeFileSync(channelPath(channel), JSON.stringify(data, null, 2) + "\n");
}

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
      if (paneMap[String(pid)]) return paneMap[String(pid)];
      try {
        pid = parseInt(execSync(`ps -o ppid= -p ${pid}`, { timeout: 1000 }).toString().trim());
      } catch { break; }
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
  return loadChannel(channel).agents;
}

// --- Startup: detect pane and auto-register ---

const myLocation = detectPane();
const myChannel = myLocation?.session || null;
const myPane = myLocation?.pane || null;
let myName = null;

if (myChannel) {
  // Generate a unique name: agent-<random>
  const channelData = loadChannel(myChannel);
  const existing = new Set(Object.keys(channelData.agents));

  // Generate random name, retry on collision (astronomically unlikely)
  do {
    myName = `agent-${randomBytes(3).toString("hex")}`;
  } while (existing.has(myName));

  channelData.agents[myName] = { pane: myPane };
  saveChannel(myChannel, channelData);
  console.error(`agent-bus: registered as "${myName}" on channel "${myChannel}" (pane ${myPane})`);
} else {
  console.error("agent-bus: WARNING — not inside tmux, registration skipped");
}

// Build instructions with this agent's identity baked in
const INSTRUCTIONS = readFileSync(join(__dirname, "MCP_INSTRUCTIONS.md"), "utf-8")
  + `\n\n## Your Identity\n\nYou are registered as **${myName}** on channel **${myChannel}**. Use "${myName}" when others need to message you. You do not need to register — it happened automatically.\n`;

const server = new McpServer({
  name: "agent-bus",
  version: "0.5.0",
}, {
  instructions: INSTRUCTIONS,
});

server.tool(
  "who",
  "List all agents on your channel.",
  {},
  async () => {
    if (!myChannel) {
      return { content: [{ type: "text", text: "Not running inside tmux." }], isError: true };
    }
    const agents = getAgents(myChannel);
    const names = Object.keys(agents);
    if (names.length === 0) {
      return { content: [{ type: "text", text: `No agents on channel "${myChannel}".` }] };
    }
    const lines = names.map((n) => {
      const marker = n === myName ? " (you)" : "";
      return `- ${n}${marker} (pane ${agents[n].pane})`;
    }).join("\n");
    return { content: [{ type: "text", text: `Channel "${myChannel}":\n${lines}` }] };
  }
);

server.tool(
  "signal_done",
  "Hand off to another agent. Call 'who' first to see available agents.",
  {
    next: z.string().describe("Agent name to hand off to (call 'who' to see names)"),
    summary: z.string().describe("What you just finished"),
    request: z.string().describe("What you need the next agent to do"),
  },
  async ({ next, summary, request }) => {
    if (!myChannel) {
      return { content: [{ type: "text", text: "Not running inside tmux." }], isError: true };
    }
    const agents = getAgents(myChannel);
    const pane = agents[next]?.pane;
    if (!pane) {
      const available = Object.keys(agents).filter((n) => n !== myName);
      return { content: [{ type: "text", text: `Unknown agent "${next}". Available: ${available.length ? available.join(", ") : "none"}.` }], isError: true };
    }
    const message = `[from ${myName}]: ${summary} Request: ${request}`;
    const result = sendToPane(pane, message);
    logHandoff({ type: "signal_done", channel: myChannel, from: myName, to: next, summary, request });
    if (!result.success) {
      return { content: [{ type: "text", text: `Failed to reach ${next}: ${result.error}` }], isError: true };
    }
    return { content: [{ type: "text", text: `Handed off to ${next} (pane ${pane}).` }] };
  }
);

server.tool(
  "send_message",
  "Send a message to another agent without handing off.",
  {
    to: z.string().describe("Agent name to message (call 'who' to see names)"),
    message: z.string().describe("The message to send"),
  },
  async ({ to, message }) => {
    if (!myChannel) {
      return { content: [{ type: "text", text: "Not running inside tmux." }], isError: true };
    }
    const agents = getAgents(myChannel);
    const pane = agents[to]?.pane;
    if (!pane) {
      const available = Object.keys(agents).filter((n) => n !== myName);
      return { content: [{ type: "text", text: `Unknown agent "${to}". Available: ${available.length ? available.join(", ") : "none"}.` }], isError: true };
    }
    const fullMessage = `[message from ${myName}]: ${message}`;
    const result = sendToPane(pane, fullMessage);
    logHandoff({ type: "send_message", channel: myChannel, from: myName, to, message });
    if (!result.success) {
      return { content: [{ type: "text", text: `Failed to reach ${to}: ${result.error}` }], isError: true };
    }
    return { content: [{ type: "text", text: `Message sent to ${to} (pane ${pane}).` }] };
  }
);

// Cleanup: remove self from channel on exit
function cleanup() {
  if (myChannel && myName) {
    try {
      const channelData = loadChannel(myChannel);
      delete channelData.agents[myName];
      saveChannel(myChannel, channelData);
      console.error(`agent-bus: unregistered "${myName}" from channel "${myChannel}"`);
    } catch {}
  }
}

process.on("exit", cleanup);
process.on("SIGINT", () => { cleanup(); process.exit(0); });
process.on("SIGTERM", () => { cleanup(); process.exit(0); });

async function main() {
  const transport = new StdioServerTransport();
  await server.connect(transport);
  console.error("agent-bus MCP server running on stdio");
}

main().catch((err) => {
  console.error("Fatal:", err);
  process.exit(1);
});
