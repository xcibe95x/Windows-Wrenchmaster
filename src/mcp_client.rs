//! Minimal MCP client for local stdio servers.
//!
//! This implements the common initialize/initialized and tools/list exchange.
//! It does not execute tools or send prompts to an AI provider.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

const RESPONSE_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_LINE_BYTES: usize = 1_048_576;

#[derive(Clone, Debug)]
pub struct ToolInfo {
    pub name: String,
    pub description: String,
}

#[derive(Debug)]
pub enum Event {
    Connected {
        server_name: String,
        server_version: String,
        tools: Vec<ToolInfo>,
    },
    Disconnected,
    Failed(String),
}

enum Control {
    Disconnect,
}

pub struct Connection {
    pub events: Receiver<Event>,
    control: Sender<Control>,
    worker: Option<JoinHandle<()>>,
}

impl Connection {
    pub fn start(program: String, args: Vec<String>) -> Self {
        let (event_tx, events) = mpsc::channel();
        let (control, control_rx) = mpsc::channel();
        let worker = thread::Builder::new()
            .name("mcp-stdio-client".into())
            .spawn(move || run_server(program, args, event_tx, control_rx))
            .expect("failed to start MCP worker");
        Self {
            events,
            control,
            worker: Some(worker),
        }
    }

    pub fn disconnect(&mut self) {
        let _ = self.control.send(Control::Disconnect);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.disconnect();
    }
}

fn run_server(
    program: String,
    args: Vec<String>,
    events: Sender<Event>,
    control: Receiver<Control>,
) {
    let result = connect_and_list(&program, &args, &control);
    let (mut child, output) = match result {
        Ok((child, output, server_name, server_version, tools)) => {
            let _ = events.send(Event::Connected {
                server_name,
                server_version,
                tools,
            });
            (child, output)
        }
        Err(error) => {
            let _ = events.send(Event::Failed(error));
            return;
        }
    };

    let mut output_closed = false;
    loop {
        match control.recv_timeout(Duration::from_millis(100)) {
            Ok(Control::Disconnect) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {}
        }
        loop {
            match output.try_recv() {
                Ok(_) => {} // Drain notifications so the server cannot block on stdout.
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    output_closed = true;
                    break;
                }
            }
        }
        if output_closed {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    if output_closed {
        let _ = events.send(Event::Failed("MCP server stopped unexpectedly.".into()));
    } else {
        let _ = events.send(Event::Disconnected);
    }
}

fn connect_and_list(
    program: &str,
    args: &[String],
    control: &Receiver<Control>,
) -> Result<
    (
        Child,
        Receiver<Result<String, String>>,
        String,
        String,
        Vec<ToolInfo>,
    ),
    String,
> {
    let mut child = KillOnDrop(Some(
        Command::new(program)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // MCP stdio reserves stdout for protocol messages. Suppress stderr so
            // a noisy server cannot block on an unread pipe.
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("Could not start MCP server: {error}"))?,
    ));
    let stdin = child
        .0
        .as_mut()
        .unwrap()
        .stdin
        .take()
        .ok_or_else(|| "Could not open MCP server input.".to_string())?;
    let stdout = child
        .0
        .as_mut()
        .unwrap()
        .stdout
        .take()
        .ok_or_else(|| "Could not open MCP server output.".to_string())?;
    let (line_tx, line_rx) = mpsc::sync_channel(64);
    thread::Builder::new()
        .name("mcp-stdio-reader".into())
        .spawn(move || read_lines(stdout, line_tx))
        .map_err(|error| format!("Could not read MCP server output: {error}"))?;
    let mut stdin = stdin;

    write_json(
        &mut stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "Windows Wrenchmaster", "version": env!("CARGO_PKG_VERSION")}
            }
        }),
    )?;
    let initialized = receive_response(&line_rx, control, 1)?;
    let result = initialized.get("result").ok_or_else(|| {
        rpc_error(
            &initialized,
            "MCP server did not return an initialize result.",
        )
    })?;
    let server = result.get("serverInfo").cloned().unwrap_or_default();
    let server_name = server
        .get("name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("MCP server")
        .chars()
        .take(120)
        .collect();
    let server_version = server
        .get("version")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown version")
        .chars()
        .take(80)
        .collect();

    write_json(
        &mut stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
    )?;
    write_json(
        &mut stdin,
        &serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
    )?;
    let response = receive_response(&line_rx, control, 2)?;
    let result = response
        .get("result")
        .ok_or_else(|| rpc_error(&response, "MCP server did not return its tool list."))?;
    let tools = result
        .get("tools")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "MCP server returned an invalid tool list.".to_string())?
        .iter()
        .take(256)
        .filter_map(|tool| {
            let name = tool.get("name")?.as_str()?;
            Some(ToolInfo {
                name: name.chars().take(120).collect(),
                description: tool
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("")
                    .chars()
                    .take(500)
                    .collect(),
            })
        })
        .collect();
    Ok((
        child.0.take().unwrap(),
        line_rx,
        server_name,
        server_version,
        tools,
    ))
}

struct KillOnDrop(Option<Child>);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn write_json(stdin: &mut ChildStdin, value: &serde_json::Value) -> Result<(), String> {
    serde_json::to_writer(&mut *stdin, value)
        .map_err(|error| format!("Could not encode MCP message: {error}"))?;
    stdin
        .write_all(b"\n")
        .and_then(|_| stdin.flush())
        .map_err(|error| format!("Could not send MCP message: {error}"))
}

fn receive_response(
    lines: &Receiver<Result<String, String>>,
    control: &Receiver<Control>,
    id: u64,
) -> Result<serde_json::Value, String> {
    let deadline = std::time::Instant::now() + RESPONSE_TIMEOUT;
    loop {
        if matches!(control.try_recv(), Ok(Control::Disconnect)) {
            return Err("Connection cancelled.".into());
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Err("MCP server did not respond within 15 seconds.".into());
        }
        match lines.recv_timeout(remaining.min(Duration::from_millis(200))) {
            Ok(Ok(line)) => {
                let value: serde_json::Value = serde_json::from_str(&line)
                    .map_err(|_| "MCP server wrote invalid JSON to stdout.".to_string())?;
                if value.get("id").and_then(serde_json::Value::as_u64) == Some(id) {
                    if value.get("error").is_some() {
                        return Err(rpc_error(&value, "MCP request failed."));
                    }
                    return Ok(value);
                }
                // Ignore notifications and server requests during startup.
            }
            Ok(Err(error)) => return Err(error),
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                return Err("MCP server closed its output unexpectedly.".into());
            }
        }
    }
}

fn rpc_error(value: &serde_json::Value, fallback: &str) -> String {
    value
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(serde_json::Value::as_str)
        .map(|message| message.chars().take(300).collect())
        .unwrap_or_else(|| fallback.to_string())
}

fn read_lines(stdout: impl std::io::Read, sender: SyncSender<Result<String, String>>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut bytes = Vec::new();
        match reader.read_until(b'\n', &mut bytes) {
            Ok(0) => break,
            Ok(size) if size <= MAX_LINE_BYTES => {
                let line = String::from_utf8_lossy(&bytes).trim().to_string();
                if !line.is_empty() && sender.send(Ok(line)).is_err() {
                    break;
                }
            }
            Ok(_) => {
                let _ = sender.send(Err(
                    "MCP server response exceeded the 1 MiB safety limit.".into()
                ));
                break;
            }
            Err(error) => {
                let _ = sender.send(Err(format!("Could not read MCP server output: {error}")));
                break;
            }
        }
    }
}
