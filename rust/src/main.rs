use std::collections::HashSet;
use std::fs;
use std::io::{self, BufRead, Write as IoWrite};
use std::process::Command;

use anyhow::{Context, Result};
use serde_json::{json, Value};

const TOOLS_JSON: &str = include_str!("../tools.json");
const MCP_INSTRUCTIONS: &str = include_str!("../../MCP_INSTRUCTIONS.md");

fn bus_dir() -> std::path::PathBuf {
    dirs::home_dir().unwrap_or_default().join(".agent-bus")
}

fn channels_dir() -> std::path::PathBuf {
    bus_dir().join("channels")
}

fn channel_path(channel: &str) -> std::path::PathBuf {
    channels_dir().join(format!("{channel}.json"))
}

fn load_channel(channel: &str) -> Value {
    let _ = fs::create_dir_all(channels_dir());
    let p = channel_path(channel);
    match fs::read_to_string(&p) {
        Ok(s) => {
            let mut data: Value = serde_json::from_str(&s).unwrap_or(json!({}));
            if data.get("agents").is_none() {
                data["agents"] = json!({});
            }
            data
        }
        Err(_) => json!({ "agents": {} }),
    }
}

fn save_channel(channel: &str, data: &Value) {
    let _ = fs::create_dir_all(channels_dir());
    let s = serde_json::to_string_pretty(data).unwrap_or_default() + "\n";
    let _ = fs::write(channel_path(channel), s);
}

fn log_handoff(record: &Value) {
    let log_path = bus_dir().join("history.jsonl");
    let mut entry = record.clone();
    entry["ts"] = json!(iso_now());
    let line = serde_json::to_string(&entry).unwrap_or_default() + "\n";
    let _ = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .and_then(|mut f| f.write_all(line.as_bytes()));
}

fn iso_now() -> String {
    let output = Command::new("date")
        .args(["-u", "+%Y-%m-%dT%H:%M:%SZ"])
        .output();
    match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => String::new(),
    }
}

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

fn send_to_pane(pane: &str, message: &str) -> Result<()> {
    let sanitized: String = message.split_whitespace().collect::<Vec<_>>().join(" ");
    let status = Command::new("tmux")
        .args(["send-keys", "-t", pane, "-l", &sanitized])
        .status()
        .context("failed to run tmux send-keys")?;
    if !status.success() {
        anyhow::bail!("tmux send-keys failed");
    }
    let status = Command::new("tmux")
        .args(["send-keys", "-t", pane, "Enter"])
        .status()
        .context("failed to send Enter")?;
    if !status.success() {
        anyhow::bail!("tmux send-keys Enter failed");
    }
    Ok(())
}

struct AgentState {
    name: Option<String>,
    channel: Option<String>,
}

impl Drop for AgentState {
    fn drop(&mut self) {
        if let (Some(channel), Some(name)) = (&self.channel, &self.name) {
            let mut data = load_channel(channel);
            if let Some(agents) = data["agents"].as_object_mut() {
                agents.remove(name);
            }
            save_channel(channel, &data);
            eprintln!("tmux-agent-bus: unregistered \"{name}\" from channel \"{channel}\"");
        }
    }
}

fn register() -> AgentState {
    let (pane, session) = match detect_pane() {
        Some(v) => v,
        None => {
            return AgentState { name: None, channel: None };
        }
    };

    let agent_type = detect_agent_type();
    let mut data = load_channel(&session);
    let existing: HashSet<String> = data["agents"]
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();

    let mut n = 1u32;
    let name = loop {
        let candidate = format!("{agent_type}-{n}");
        if !existing.contains(&candidate) {
            break candidate;
        }
        n += 1;
    };

    data["agents"][&name] = json!({ "pane": pane, "type": agent_type });
    save_channel(&session, &data);

    let _ = Command::new("tmux")
        .args(["set-option", "-p", "-t", &pane, "@agent-name", &name])
        .status();

    eprintln!("tmux-agent-bus: registered as \"{name}\" on channel \"{session}\" (pane {pane})");

    AgentState {
        name: Some(name),
        channel: Some(session),
    }
}

// --- Tool handlers ---

fn handle_who(state: &AgentState) -> Value {
    let channel = match &state.channel {
        Some(c) => c,
        None => return err_result("Not running inside tmux."),
    };
    let data = load_channel(channel);
    let agents = match data["agents"].as_object() {
        Some(m) if !m.is_empty() => m,
        _ => return ok_result(&format!("No agents on channel \"{channel}\".")),
    };

    let lines: Vec<String> = agents
        .iter()
        .map(|(n, info)| {
            let you = if Some(n) == state.name.as_ref() { " (you)" } else { "" };
            let t = info["type"].as_str().unwrap_or("");
            let p = info["pane"].as_str().unwrap_or("");
            format!("- {n} [{t}]{you} (pane {p})")
        })
        .collect();
    ok_result(&format!("Channel \"{channel}\":\n{}", lines.join("\n")))
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

    let data = load_channel(channel);
    match data["agents"][next]["pane"].as_str() {
        None => {
            let available = available_agents(&data, my_name);
            err_result(&format!("Unknown agent \"{next}\". Available: {available}."))
        }
        Some(pane) => {
            let message = format!("[from {my_name}]: {summary} Request: {request}");
            log_handoff(&json!({
                "type": "signal_done", "channel": channel,
                "from": my_name, "to": next, "summary": summary, "request": request
            }));
            match send_to_pane(pane, &message) {
                Ok(_) => ok_result(&format!("Handed off to {next} (pane {pane}).")),
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

    let data = load_channel(channel);
    match data["agents"][to]["pane"].as_str() {
        None => {
            let available = available_agents(&data, my_name);
            err_result(&format!("Unknown agent \"{to}\". Available: {available}."))
        }
        Some(pane) => {
            let full_message = format!("[message from {my_name}]: {message}");
            log_handoff(&json!({
                "type": "send_message", "channel": channel,
                "from": my_name, "to": to, "message": message
            }));
            match send_to_pane(pane, &full_message) {
                Ok(_) => ok_result(&format!("Message sent to {to} (pane {pane}).")),
                Err(e) => err_result(&format!("Failed to reach {to}: {e}")),
            }
        }
    }
}

fn available_agents(data: &Value, exclude: &str) -> String {
    data["agents"]
        .as_object()
        .map(|m| {
            let names: Vec<&String> = m.keys().filter(|n| n.as_str() != exclude).collect();
            if names.is_empty() {
                "none".to_string()
            } else {
                names.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
            }
        })
        .unwrap_or_else(|| "none".to_string())
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
                let response = json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "protocolVersion": "2025-11-25",
                        "capabilities": { "tools": {} },
                        "serverInfo": {
                            "name": "tmux-agent-bus",
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
