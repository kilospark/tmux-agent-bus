use std::io::{self, BufRead, Write as IoWrite};
use std::process::Command;

use anyhow::{Context, Result};
use serde_json::{json, Value};

const TOOLS_JSON: &str = include_str!("../tools.json");
const MCP_INSTRUCTIONS: &str = include_str!("../MCP_INSTRUCTIONS.md");

// --- Agent info from live tmux state ---

struct PaneAgent {
    name: String,
    pane: String,
    session: String,
}

/// Query tmux for all panes that have @agent-name set.
/// Returns live agent info — no stale entries possible.
fn list_agents() -> Vec<PaneAgent> {
    let output = Command::new("tmux")
        .args([
            "list-panes", "-a", "-F",
            "#{@agent-name}\t#{session_name}:#{window_index}.#{pane_index}\t#{session_name}",
        ])
        .output();

    let output = match output {
        Ok(o) if o.status.success() => o,
        _ => return vec![],
    };

    let text = String::from_utf8_lossy(&output.stdout);
    text.trim()
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.len() == 3 && !parts[0].is_empty() {
                Some(PaneAgent {
                    name: parts[0].to_string(),
                    pane: parts[1].to_string(),
                    session: parts[2].to_string(),
                })
            } else {
                None
            }
        })
        .collect()
}

/// Find the pane for a given agent name within a specific channel (session).
fn find_agent(name: &str, channel: &str) -> Option<String> {
    list_agents()
        .into_iter()
        .find(|a| a.name == name && a.session == channel)
        .map(|a| a.pane)
}

/// List agents on a given session (channel), excluding self.
fn agents_on_channel(session: &str, exclude: &str) -> Vec<PaneAgent> {
    list_agents()
        .into_iter()
        .filter(|a| a.session == session && a.name != exclude)
        .collect()
}

// --- Process tree walking ---

/// Walk process tree to find which tmux pane we're in.
/// Returns (pane_id, session_name).
fn detect_pane() -> Option<(String, String)> {
    let output = Command::new("tmux")
        .args([
            "list-panes", "-a", "-F",
            "#{pane_pid} #{session_name}:#{window_index}.#{pane_index} #{session_name}",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    let mut pane_map = std::collections::HashMap::new();
    for line in text.trim().lines() {
        let parts: Vec<&str> = line.splitn(3, ' ').collect();
        if parts.len() == 3 {
            pane_map.insert(parts[0].to_string(), (parts[1].to_string(), parts[2].to_string()));
        }
    }

    let mut pid = std::process::id();
    loop {
        if pid <= 1 {
            break;
        }
        if let Some((pane, session)) = pane_map.get(&pid.to_string()) {
            return Some((pane.clone(), session.clone()));
        }
        match parent_pid(pid) {
            Some(ppid) if ppid != pid => pid = ppid,
            _ => break,
        }
    }
    None
}

/// Walk process tree to detect agent type (claude, codex, copilot).
fn detect_agent_type() -> String {
    let mut pid = std::process::id();
    loop {
        if pid <= 1 {
            break;
        }
        if let Ok(output) = Command::new("ps").args(["-o", "comm=", "-p", &pid.to_string()]).output() {
            let comm = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let name = comm.rsplit('/').next().unwrap_or("").to_lowercase();
            if name == "claude" || name.starts_with("claude-") {
                return "claude".into();
            }
            if name.starts_with("codex") {
                return "codex".into();
            }
            if name.starts_with("copilot") {
                return "copilot".into();
            }
            if name == "agent" {
                return "agent".into();
            }
        }
        match parent_pid(pid) {
            Some(ppid) if ppid != pid => pid = ppid,
            _ => break,
        }
    }
    "unknown".into()
}

fn parent_pid(pid: u32) -> Option<u32> {
    Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .trim()
                .parse::<u32>()
                .ok()
        })
}

// --- Message sending ---

fn capture_pane(pane: &str) -> String {
    Command::new("tmux")
        .args(["capture-pane", "-t", pane, "-p"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

fn try_send(pane: &str, sanitized: &str) -> Result<bool> {
    let before = capture_pane(pane);

    // Send text first, then Enter separately after a short delay so the
    // target has time to process the content before submission.
    let status = Command::new("tmux")
        .args(["send-keys", "-t", pane, "--", sanitized])
        .status()
        .context("failed to run tmux send-keys (text)")?;
    if !status.success() {
        anyhow::bail!("tmux send-keys failed (text)");
    }

    std::thread::sleep(std::time::Duration::from_millis(200));

    let status = Command::new("tmux")
        .args(["send-keys", "-t", pane, "Enter"])
        .status()
        .context("failed to run tmux send-keys (Enter)")?;
    if !status.success() {
        anyhow::bail!("tmux send-keys failed (Enter)");
    }

    // Poll for ack: wait for pane to show the message was processed
    for _ in 0..8 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let current = capture_pane(pane);
        if current != before {
            return Ok(true);
        }
    }
    Ok(false)
}

fn send_to_pane(pane: &str, message: &str) -> Result<bool> {
    let sanitized: String = message.split_whitespace().collect::<Vec<_>>().join(" ");

    // First attempt
    if try_send(pane, &sanitized)? {
        return Ok(true);
    }

    // Retry once
    eprintln!("agent-bus: first send to {pane} got no ack, retrying...");
    try_send(pane, &sanitized)
}

// --- Registration ---

struct AgentState {
    name: Option<String>,
    pane: Option<String>,
    channel: Option<String>,
}

impl Drop for AgentState {
    fn drop(&mut self) {
        if let Some(pane) = &self.pane {
            // Clear pane option on exit
            let _ = Command::new("tmux")
                .args(["set-option", "-pu", "-t", pane, "@agent-name"])
                .status();
            if let Some(name) = &self.name {
                eprintln!("agent-bus: unregistered \"{name}\"");
            }
        }
    }
}

fn register() -> AgentState {
    let (pane, session) = match detect_pane() {
        Some(v) => v,
        None => {
            return AgentState { name: None, pane: None, channel: None };
        }
    };

    let agent_type = detect_agent_type();

    // Check existing agent names across this session to pick a unique name
    let existing_names: std::collections::HashSet<String> = list_agents()
        .into_iter()
        .filter(|a| a.session == session)
        .map(|a| a.name)
        .collect();

    let mut n = 1u32;
    let name = loop {
        let candidate = format!("{agent_type}-{n}");
        if !existing_names.contains(&candidate) {
            break candidate;
        }
        n += 1;
    };

    // Set pane option — this IS the registration. No JSON file needed.
    let _ = Command::new("tmux")
        .args(["set-option", "-p", "-t", &pane, "@agent-name", &name])
        .status();

    // Enable pane borders for this window if not already on
    let border_status = Command::new("tmux")
        .args(["show-option", "-wv", "-t", &pane, "pane-border-status"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .unwrap_or_default();
    if border_status.trim().is_empty() || border_status.trim() == "off" {
        let _ = Command::new("tmux")
            .args(["set-option", "-w", "-t", &pane, "pane-border-format", " #{@agent-name} | #{pane_title} "])
            .status();
        let _ = Command::new("tmux")
            .args(["set-option", "-w", "-t", &pane, "pane-border-status", "top"])
            .status();
    }

    eprintln!("agent-bus: registered as \"{name}\" on channel \"{session}\" (pane {pane})");

    AgentState {
        name: Some(name),
        pane: Some(pane),
        channel: Some(session),
    }
}

// --- Tool handlers ---

fn handle_who(state: &AgentState) -> Value {
    let channel = match &state.channel {
        Some(c) => c,
        None => return err_result("Not running inside tmux."),
    };
    let my_name = state.name.as_deref().unwrap_or("unknown");
    let agents = list_agents();
    let on_channel: Vec<&PaneAgent> = agents.iter().filter(|a| a.session == *channel).collect();

    if on_channel.is_empty() {
        return ok_result(&format!("No agents on channel \"{channel}\"."));
    }

    let lines: Vec<String> = on_channel
        .iter()
        .map(|a| {
            let you = if a.name == my_name { " (you)" } else { "" };
            format!("- {}{} (pane {})", a.name, you, a.pane)
        })
        .collect();
    ok_result(&format!("Channel \"{channel}\":\n{}\n\nUse \"@all\" to broadcast to all agents.", lines.join("\n")))
}

fn handle_signal_done(state: &AgentState, args: &Value) -> Value {
    let channel = match &state.channel {
        Some(c) => c,
        None => return err_result("Not running inside tmux."),
    };
    let next = args["next"].as_str().unwrap_or_default();
    let summary = args["summary"].as_str().unwrap_or_default();
    let request = args["request"].as_str().unwrap_or_default();
    let my_name = state.name.as_deref().unwrap_or("unknown");
    let message = format!("[from {my_name}]: {summary} Request: {request}");

    if next == "@all" {
        return broadcast(channel, my_name, &message);
    }

    match find_agent(next, channel) {
        None => {
            let available = available_agents(channel, my_name);
            err_result(&format!("Unknown agent \"{next}\". Available: {available}."))
        }
        Some(pane) => {
            match send_to_pane(&pane, &message) {
                Ok(true) => ok_result(&format!("Handed off to {next} (pane {pane}). Ack: message received.")),
                Ok(false) => err_result(&format!("Message sent to {next} (pane {pane}) but no ack — message could not be delivered.")),
                Err(e) => err_result(&format!("Failed to reach {next}: {e}")),
            }
        }
    }
}

fn handle_send_message(state: &AgentState, args: &Value) -> Value {
    let channel = match &state.channel {
        Some(c) => c,
        None => return err_result("Not running inside tmux."),
    };
    let to = args["to"].as_str().unwrap_or_default();
    let message = args["message"].as_str().unwrap_or_default();
    let my_name = state.name.as_deref().unwrap_or("unknown");
    let full_message = format!("[message from {my_name}]: {message}");

    if to == "@all" {
        return broadcast(channel, my_name, &full_message);
    }

    match find_agent(to, channel) {
        None => {
            let available = available_agents(channel, my_name);
            err_result(&format!("Unknown agent \"{to}\". Available: {available}."))
        }
        Some(pane) => {
            match send_to_pane(&pane, &full_message) {
                Ok(true) => ok_result(&format!("Message sent to {to} (pane {pane}). Ack: message received.")),
                Ok(false) => err_result(&format!("Message sent to {to} (pane {pane}) but no ack — message could not be delivered.")),
                Err(e) => err_result(&format!("Failed to reach {to}: {e}")),
            }
        }
    }
}

fn broadcast(channel: &str, my_name: &str, message: &str) -> Value {
    let targets = agents_on_channel(channel, my_name);
    if targets.is_empty() {
        return err_result("No other agents on this channel to broadcast to.");
    }

    let mut delivered = Vec::new();
    let mut failed = Vec::new();

    for agent in &targets {
        match send_to_pane(&agent.pane, message) {
            Ok(true) => delivered.push(agent.name.as_str()),
            Ok(false) | Err(_) => failed.push(agent.name.as_str()),
        }
    }

    let mut result = format!("Broadcast to {} agent(s).", targets.len());
    if !delivered.is_empty() {
        result.push_str(&format!(" Delivered: {}.", delivered.join(", ")));
    }
    if !failed.is_empty() {
        result.push_str(&format!(" Failed: {}.", failed.join(", ")));
    }

    if failed.is_empty() {
        ok_result(&result)
    } else {
        err_result(&result)
    }
}

fn available_agents(channel: &str, exclude: &str) -> String {
    let agents = agents_on_channel(channel, exclude);
    if agents.is_empty() {
        "none".to_string()
    } else {
        agents.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")
    }
}

fn ok_result(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }] })
}

fn err_result(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": true })
}

fn write_response(stdout: &io::Stdout, response: &Value) -> Result<()> {
    let mut out = stdout.lock();
    serde_json::to_writer(&mut out, response).context("failed writing JSON-RPC response")?;
    out.write_all(b"\n").context("failed writing newline")?;
    out.flush().context("failed flushing stdout")?;
    Ok(())
}

// --- Main ---

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && matches!(args[1].as_str(), "-v" | "-V" | "--version") {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return;
    }

    let state = register();

    if let Err(e) = run_server(&state) {
        eprintln!("Fatal: {e:#}");
    }
    // state drops here -> unregister runs
}

fn run_server(state: &AgentState) -> Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();

    let instructions = format!(
        "{MCP_INSTRUCTIONS}\n\n## Your Identity\n\n\
         You are registered as **{}** on channel **{}**. \
         Use \"{}\" when others need to message you. \
         You do not need to register — it happened automatically.\n",
        state.name.as_deref().unwrap_or("unknown"),
        state.channel.as_deref().unwrap_or("unknown"),
        state.name.as_deref().unwrap_or("unknown"),
    );

    for line in stdin.lock().lines() {
        let line = line.context("failed reading stdin")?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("Invalid JSON-RPC: {e}");
                continue;
            }
        };

        let id = request.get("id").cloned();
        let method = request
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        match method.as_str() {
            "initialize" => {
                // Echo back the client's protocol version for Cursor Agent compat
                let client_protocol = request
                    .pointer("/params/protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or("2024-11-05");
                let response = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": client_protocol,
                        "capabilities": { "tools": {} },
                        "serverInfo": {
                            "name": "agent-bus",
                            "version": env!("CARGO_PKG_VERSION")
                        },
                        "instructions": instructions
                    }
                });
                write_response(&stdout, &response)?;
            }
            "notifications/initialized" => {}
            "tools/list" => {
                let tools: Value =
                    serde_json::from_str(TOOLS_JSON).context("failed parsing tools.json")?;
                let response = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": { "tools": tools }
                });
                write_response(&stdout, &response)?;
            }
            "tools/call" => {
                let params = request.get("params").cloned().unwrap_or(Value::Null);
                let tool_name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

                let result = match tool_name {
                    "who" => handle_who(state),
                    "signal_done" => handle_signal_done(state, &arguments),
                    "send_message" => handle_send_message(state, &arguments),
                    _ => err_result(&format!("Unknown tool: {tool_name}")),
                };

                let response = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": result
                });
                write_response(&stdout, &response)?;
            }
            _ => {
                if let Some(id) = id {
                    let response = json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": {
                            "code": -32601,
                            "message": format!("Method not found: {method}")
                        }
                    });
                    write_response(&stdout, &response)?;
                }
            }
        }
    }

    Ok(())
}
