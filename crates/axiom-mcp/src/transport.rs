use std::{
    collections::{BTreeMap, VecDeque},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStderr, Command, Stdio},
    sync::{mpsc::SyncSender, Arc, Mutex},
};

use async_trait::async_trait;
use serde_json::Value;

use crate::error::{McpError, Result};

/// Frames are newline-delimited JSON, so a peer must never emit a raw newline
/// inside a message.
pub const DEFAULT_MAX_FRAME_BYTES: usize = 4 * 1024 * 1024;

/// Bounded queue between the worker threads and the async side.
const CHANNEL_CAPACITY: usize = 64;

const SERVER_LOG_LINES: usize = 50;

/// Environment variables forwarded to a spawned MCP server.
///
/// Third-party servers do not inherit Axiom's environment: provider keys and
/// Axiom's own configuration stay out of their process unless the server
/// explicitly declares them through `[[mcp.servers]]`.
const ENV_PASSTHROUGH: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SystemRoot",
    "SystemDrive",
    "COMSPEC",
    "TEMP",
    "TMP",
    "HOME",
    "USERPROFILE",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
    "PROGRAMFILES",
    "USERNAME",
    "USER",
    "LANG",
    "LC_ALL",
    "TZ",
    "NODE_PATH",
    "NPM_CONFIG_PREFIX",
    "PYTHONPATH",
    "VIRTUAL_ENV",
];

/// A bidirectional frame channel to an MCP peer.
#[async_trait]
pub trait FrameTransport: Send {
    async fn send(&mut self, message: &Value) -> Result<()>;

    /// Returns `Ok(None)` once the peer closed the stream cleanly.
    async fn receive(&mut self) -> Result<Option<Value>>;

    /// Releases the transport, terminating a spawned server process if any.
    async fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn server_logs(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Reads one newline-delimited frame, skipping blank keep-alive lines.
///
/// A frame longer than `limit` is rejected instead of being buffered, so a
/// misbehaving peer cannot exhaust memory.
pub fn read_frame_blocking<R: BufRead>(
    reader: &mut R,
    buffer: &mut Vec<u8>,
    limit: usize,
    server: &str,
) -> Result<Option<Value>> {
    loop {
        buffer.clear();
        let read = reader
            .by_ref()
            .take(limit as u64 + 1)
            .read_until(b'\n', buffer)?;
        if read == 0 {
            return Ok(None);
        }
        if buffer.len() > limit {
            return Err(McpError::FrameTooLarge {
                server: server.to_string(),
                limit,
            });
        }
        let line = buffer.trim_ascii();
        if line.is_empty() {
            continue;
        }
        return Ok(Some(serde_json::from_slice(line)?));
    }
}

/// Writes one newline-delimited frame.
pub fn write_frame_blocking<W: Write>(writer: &mut W, message: &Value) -> Result<()> {
    let mut encoded = serde_json::to_vec(message)?;
    encoded.push(b'\n');
    writer.write_all(&encoded)?;
    writer.flush()?;
    Ok(())
}

/// An in-process transport pair, used to embed an MCP peer in the same program
/// and to test both directions without spawning anything.
pub struct ChannelTransport {
    outbound: tokio::sync::mpsc::UnboundedSender<Result<Value>>,
    inbound: tokio::sync::mpsc::UnboundedReceiver<Result<Value>>,
    server: String,
}

impl ChannelTransport {
    pub fn with_server(mut self, server: impl Into<String>) -> Self {
        self.server = server.into();
        self
    }
}

/// Creates two transports wired to each other.
pub fn channel_transport_pair() -> (ChannelTransport, ChannelTransport) {
    let (left_tx, left_rx) = tokio::sync::mpsc::unbounded_channel();
    let (right_tx, right_rx) = tokio::sync::mpsc::unbounded_channel();
    (
        ChannelTransport {
            outbound: left_tx,
            inbound: right_rx,
            server: "in-process".to_string(),
        },
        ChannelTransport {
            outbound: right_tx,
            inbound: left_rx,
            server: "in-process".to_string(),
        },
    )
}

#[async_trait]
impl FrameTransport for ChannelTransport {
    async fn send(&mut self, message: &Value) -> Result<()> {
        self.outbound
            .send(Ok(message.clone()))
            .map_err(|_| McpError::Closed {
                server: self.server.clone(),
            })
    }

    async fn receive(&mut self) -> Result<Option<Value>> {
        match self.inbound.recv().await {
            Some(Ok(frame)) => Ok(Some(frame)),
            Some(Err(error)) => Err(error),
            None => Ok(None),
        }
    }
}

/// How to launch an MCP server process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StdioOptions {
    pub server: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: Option<PathBuf>,
    pub max_frame_bytes: usize,
}

impl StdioOptions {
    pub fn new(server: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            server: server.into(),
            command: command.into(),
            args: Vec::new(),
            env: BTreeMap::new(),
            cwd: None,
            max_frame_bytes: DEFAULT_MAX_FRAME_BYTES,
        }
    }
}

/// An MCP server launched as a child process over stdio.
///
/// Frames are exchanged through dedicated blocking threads, which keeps the
/// async side non-blocking without pulling in an async process runtime.
pub struct StdioTransport {
    child: Option<Child>,
    outbound: SyncSender<Value>,
    inbound: tokio::sync::mpsc::Receiver<Result<Value>>,
    logs: Arc<Mutex<VecDeque<String>>>,
    server: String,
}

impl StdioTransport {
    pub fn spawn(options: &StdioOptions) -> Result<Self> {
        let (program, arguments) = resolve_command(&options.command, &options.args);
        let mut command = Command::new(&program);
        command
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear();
        for (key, value) in passthrough_environment() {
            command.env(key, value);
        }
        for (key, value) in &options.env {
            command.env(key, value);
        }
        if let Some(cwd) = &options.cwd {
            command.current_dir(cwd);
        }

        let mut child = command.spawn().map_err(|error| McpError::Spawn {
            server: options.server.clone(),
            message: format!("{error} (resolved program: {})", program.display()),
        })?;
        let stdin = child.stdin.take().ok_or_else(|| McpError::Spawn {
            server: options.server.clone(),
            message: "server process exposed no stdin".to_string(),
        })?;
        let stdout = child.stdout.take().ok_or_else(|| McpError::Spawn {
            server: options.server.clone(),
            message: "server process exposed no stdout".to_string(),
        })?;

        let logs = Arc::new(Mutex::new(VecDeque::new()));
        if let Some(stderr) = child.stderr.take() {
            spawn_stderr_logger(stderr, Arc::clone(&logs));
        }

        Ok(Self {
            child: Some(child),
            outbound: spawn_frame_writer(stdin),
            inbound: spawn_frame_reader(
                BufReader::new(stdout),
                options.max_frame_bytes,
                options.server.clone(),
            ),
            logs,
            server: options.server.clone(),
        })
    }

    /// Terminates the server process and reaps it.
    pub fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[async_trait]
impl FrameTransport for StdioTransport {
    async fn send(&mut self, message: &Value) -> Result<()> {
        self.outbound
            .send(message.clone())
            .map_err(|_| McpError::Closed {
                server: self.server.clone(),
            })
    }

    async fn receive(&mut self) -> Result<Option<Value>> {
        match self.inbound.recv().await {
            Some(Ok(frame)) => Ok(Some(frame)),
            Some(Err(error)) => Err(error),
            None => Ok(None),
        }
    }

    async fn close(&mut self) -> Result<()> {
        self.kill();
        Ok(())
    }

    fn server_logs(&self) -> Vec<String> {
        self.logs
            .lock()
            .map(|logs| logs.iter().cloned().collect())
            .unwrap_or_default()
    }
}

impl Drop for StdioTransport {
    fn drop(&mut self) {
        // A dropped `std::process::Child` would keep running, so make sure a
        // server we launched never outlives us.
        self.kill();
    }
}

/// This process's own stdio, which is how `axiom mcp serve` talks to clients.
pub struct StdinStdoutTransport {
    outbound: SyncSender<Value>,
    inbound: tokio::sync::mpsc::Receiver<Result<Value>>,
}

impl StdinStdoutTransport {
    pub fn new(max_frame_bytes: usize) -> Self {
        Self {
            outbound: spawn_frame_writer(std::io::stdout()),
            inbound: spawn_frame_reader(
                BufReader::new(std::io::stdin()),
                max_frame_bytes,
                "stdout".to_string(),
            ),
        }
    }
}

#[async_trait]
impl FrameTransport for StdinStdoutTransport {
    async fn send(&mut self, message: &Value) -> Result<()> {
        self.outbound
            .send(message.clone())
            .map_err(|_| McpError::Closed {
                server: "stdout".to_string(),
            })
    }

    async fn receive(&mut self) -> Result<Option<Value>> {
        match self.inbound.recv().await {
            Some(Ok(frame)) => Ok(Some(frame)),
            Some(Err(error)) => Err(error),
            None => Ok(None),
        }
    }
}

fn spawn_frame_reader<R: BufRead + Send + 'static>(
    mut reader: R,
    limit: usize,
    server: String,
) -> tokio::sync::mpsc::Receiver<Result<Value>> {
    let (sender, receiver) = tokio::sync::mpsc::channel(CHANNEL_CAPACITY);
    std::thread::spawn(move || {
        let mut buffer = Vec::new();
        loop {
            match read_frame_blocking(&mut reader, &mut buffer, limit, &server) {
                Ok(Some(frame)) => {
                    if sender.blocking_send(Ok(frame)).is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    let _ = sender.blocking_send(Err(error));
                    break;
                }
            }
        }
    });
    receiver
}

fn spawn_frame_writer<W: Write + Send + 'static>(mut writer: W) -> SyncSender<Value> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(CHANNEL_CAPACITY);
    std::thread::spawn(move || {
        let mut broken = false;
        // Keep draining after a failure so a sender never blocks forever.
        while let Ok(message) = receiver.recv() {
            if broken {
                continue;
            }
            if write_frame_blocking(&mut writer, &message).is_err() {
                broken = true;
            }
        }
        let _ = writer.flush();
    });
    sender
}

fn spawn_stderr_logger(stderr: ChildStderr, logs: Arc<Mutex<VecDeque<String>>>) {
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut line = Vec::new();
        loop {
            line.clear();
            match reader.read_until(b'\n', &mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let text = String::from_utf8_lossy(line.trim_ascii()).to_string();
                    if text.is_empty() {
                        continue;
                    }
                    if let Ok(mut logs) = logs.lock() {
                        if logs.len() == SERVER_LOG_LINES {
                            logs.pop_front();
                        }
                        logs.push_back(text);
                    }
                }
            }
        }
    });
}

fn passthrough_environment() -> Vec<(String, String)> {
    let current = std::env::vars().collect::<BTreeMap<_, _>>();
    let mut forwarded = Vec::new();
    for allowed in ENV_PASSTHROUGH {
        if let Some(value) = lookup_env(&current, allowed) {
            forwarded.push(((*allowed).to_string(), value));
        }
    }
    forwarded
}

fn lookup_env(current: &BTreeMap<String, String>, name: &str) -> Option<String> {
    if let Some(value) = current.get(name) {
        return Some(value.clone());
    }
    current
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.clone())
}

/// Resolves a configured command to something the OS can execute.
///
/// On Windows `npx`, `uvx`, and friends are `.cmd` shims, which
/// `CreateProcess` will not find on its own, so they are located on `PATH` and
/// wrapped in `cmd /C`.
fn resolve_command(command: &str, args: &[String]) -> (PathBuf, Vec<String>) {
    let explicit = command.contains('/') || command.contains('\\');
    let path = if explicit {
        PathBuf::from(command)
    } else {
        match find_on_path(command) {
            Some(found) => found,
            None => return (PathBuf::from(command), args.to_vec()),
        }
    };

    match wrap_windows_script(&path, args) {
        Some(wrapped) => wrapped,
        None => (path, args.to_vec()),
    }
}

/// Script shims need an interpreter on Windows; everywhere else they are
/// ordinary executables.
#[cfg(windows)]
fn wrap_windows_script(path: &Path, args: &[String]) -> Option<(PathBuf, Vec<String>)> {
    let is_script = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cmd")
                || extension.eq_ignore_ascii_case("bat")
                || extension.eq_ignore_ascii_case("ps1")
        });
    if !is_script {
        return None;
    }
    let mut wrapped = vec!["/C".to_string(), path.to_string_lossy().to_string()];
    wrapped.extend(args.iter().cloned());
    Some((PathBuf::from("cmd"), wrapped))
}

#[cfg(not(windows))]
fn wrap_windows_script(_path: &Path, _args: &[String]) -> Option<(PathBuf, Vec<String>)> {
    None
}

fn find_on_path(command: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        for suffix in candidate_suffixes() {
            let mut candidate = directory.join(command);
            if !suffix.is_empty() {
                candidate.set_extension(suffix);
            }
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn candidate_suffixes() -> &'static [&'static str] {
    #[cfg(windows)]
    {
        &["exe", "cmd", "bat", ""]
    }
    #[cfg(not(windows))]
    {
        &[""]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frames_round_trip_through_framing_helpers() {
        let mut encoded = Vec::new();
        write_frame_blocking(&mut encoded, &json!({"jsonrpc": "2.0", "id": 1})).expect("write");

        let mut reader = BufReader::new(encoded.as_slice());
        let mut buffer = Vec::new();
        let decoded = read_frame_blocking(&mut reader, &mut buffer, 1024, "test")
            .expect("read")
            .expect("frame");

        assert_eq!(decoded, json!({"jsonrpc": "2.0", "id": 1}));
        assert!(read_frame_blocking(&mut reader, &mut buffer, 1024, "test")
            .expect("eof")
            .is_none());
    }

    #[test]
    fn blank_lines_are_skipped() {
        let mut reader = BufReader::new(b"\n\n{\"jsonrpc\":\"2.0\"}\n".as_slice());
        let mut buffer = Vec::new();

        assert_eq!(
            read_frame_blocking(&mut reader, &mut buffer, 1024, "test").expect("read"),
            Some(json!({"jsonrpc": "2.0"}))
        );
    }

    #[test]
    fn oversized_frames_are_rejected() {
        let oversized = vec![b'x'; 4096];
        let mut reader = BufReader::new(oversized.as_slice());
        let mut buffer = Vec::new();

        let error = read_frame_blocking(&mut reader, &mut buffer, 128, "big")
            .expect_err("frame should be rejected");
        assert!(matches!(
            error,
            McpError::FrameTooLarge { ref server, limit: 128 } if server == "big"
        ));
    }

    #[test]
    fn malformed_frames_report_json_errors() {
        let mut reader = BufReader::new(b"{not json}\n".as_slice());
        let mut buffer = Vec::new();

        assert!(matches!(
            read_frame_blocking(&mut reader, &mut buffer, 1024, "test"),
            Err(McpError::Json(_))
        ));
    }

    #[test]
    fn explicit_scripts_are_wrapped_on_windows_only() {
        let (program, args) = resolve_command("./tools/fake-shim.cmd", &["--flag".to_string()]);

        #[cfg(windows)]
        {
            assert_eq!(program.to_string_lossy().to_lowercase(), "cmd");
            assert_eq!(args[0], "/C");
            assert_eq!(args[2], "--flag");
        }
        #[cfg(not(windows))]
        {
            assert_eq!(program, PathBuf::from("./tools/fake-shim.cmd"));
            assert_eq!(args, vec!["--flag".to_string()]);
        }
    }

    #[test]
    fn environment_passthrough_is_an_allowlist() {
        let forwarded = passthrough_environment();
        assert!(forwarded.iter().all(|(key, _)| {
            ENV_PASSTHROUGH
                .iter()
                .any(|allowed| allowed.eq_ignore_ascii_case(key))
        }));
    }

    #[tokio::test]
    async fn channel_transports_deliver_both_ways_and_report_eof() {
        let (mut client, mut server) = channel_transport_pair();

        client
            .send(&json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}))
            .await
            .expect("client send");
        assert_eq!(
            server.receive().await.expect("server receive"),
            Some(json!({"jsonrpc": "2.0", "id": 1, "method": "ping"}))
        );

        server
            .send(&json!({"jsonrpc": "2.0", "id": 1, "result": {}}))
            .await
            .expect("server send");
        assert_eq!(
            client.receive().await.expect("client receive"),
            Some(json!({"jsonrpc": "2.0", "id": 1, "result": {}}))
        );

        drop(server);
        assert!(client.receive().await.expect("eof").is_none());
    }
}
