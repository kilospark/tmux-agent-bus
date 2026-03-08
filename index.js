#!/usr/bin/env node

import { readFileSync, writeFileSync, mkdirSync, existsSync, appendFileSync } from "fs";
import { execSync } from "child_process";
import { homedir } from "os";
import { join } from "path";

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
    execSync(`tmux send-keys -t ${JSON.stringify(pane)} -l ${JSON.stringify(message)}`, { timeout: 5000 });
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
