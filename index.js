#!/usr/bin/env node

import { readFileSync, writeFileSync, mkdirSync, existsSync, appendFileSync } from "fs";
import { execSync } from "child_process";
import { homedir } from "os";
import { join } from "path";
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { fileURLToPath } from "url";
import { dirname } from "path";

const __dirname = dirname(fileURLToPath(import.meta.url));
const INSTRUCTIONS = readFileSync(join(__dirname, "MCP_INSTRUCTIONS.md"), "utf-8");

const BUS_DIR = join(homedir(), ".agent-bus");
const CONFIG_PATH = join(BUS_DIR, "config.json");
const LOG_PATH = join(BUS_DIR, "history.jsonl");

function loadConfig() {
  mkdirSync(BUS_DIR, { recursive: true });
  if (!existsSync(CONFIG_PATH)) {
    const defaultConfig = {
      agents: {
        claude: { pane: "0:0.0" },
        codex: { pane: "0:0.2" },
      },
    };
    writeFileSync(CONFIG_PATH, JSON.stringify(defaultConfig, null, 2) + "\n");
    console.error(`Created default config at ${CONFIG_PATH} — edit pane IDs to match your tmux layout`);
    return defaultConfig;
  }
  return JSON.parse(readFileSync(CONFIG_PATH, "utf-8"));
}

function sendToPane(pane, message) {
  try {
    // Replace newlines with spaces to avoid TUI input issues
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

const config = loadConfig();

const server = new McpServer({
  name: "agent-bus",
  version: "0.1.0",
}, {
  instructions: INSTRUCTIONS,
});

server.tool(
  "signal_done",
  "Signal that you are done with your task and hand off to another agent. This injects a message into the other agent's tmux pane with your summary and request.",
  {
    next: z.enum(Object.keys(config.agents)).describe("Which agent should go next"),
    summary: z.string().describe("What you just finished"),
    request: z.string().describe("What you need the next agent to do"),
  },
  async ({ next, summary, request }) => {
    const pane = config.agents[next]?.pane;
    if (!pane) {
      return { content: [{ type: "text", text: `Unknown agent: ${next}` }], isError: true };
    }
    const callerName = Object.keys(config.agents).find((k) => k !== next) || "unknown";
    const message = `[from ${callerName}]: ${summary}\n\nRequest: ${request}`;
    const result = sendToPane(pane, message);
    logHandoff({ type: "signal_done", from: callerName, to: next, summary, request });
    if (!result.success) {
      return { content: [{ type: "text", text: `Failed to reach ${next}: ${result.error}` }], isError: true };
    }
    return { content: [{ type: "text", text: `Handed off to ${next}. Message delivered to tmux pane ${pane}.` }] };
  }
);

server.tool(
  "send_message",
  "Send a message to another agent without handing off. Use for mid-task questions or FYIs.",
  {
    to: z.enum(Object.keys(config.agents)).describe("Which agent to message"),
    message: z.string().describe("The message to send"),
  },
  async ({ to, message }) => {
    const pane = config.agents[to]?.pane;
    if (!pane) {
      return { content: [{ type: "text", text: `Unknown agent: ${to}` }], isError: true };
    }
    const callerName = Object.keys(config.agents).find((k) => k !== to) || "unknown";
    const fullMessage = `[message from ${callerName}]: ${message}`;
    const result = sendToPane(pane, fullMessage);
    logHandoff({ type: "send_message", from: callerName, to, message });
    if (!result.success) {
      return { content: [{ type: "text", text: `Failed to reach ${to}: ${result.error}` }], isError: true };
    }
    return { content: [{ type: "text", text: `Message sent to ${to} in tmux pane ${pane}.` }] };
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
