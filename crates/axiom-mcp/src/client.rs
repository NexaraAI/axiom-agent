use std::{collections::BTreeSet, time::Duration};

use serde_json::Value;

use crate::{
    error::{McpError, Result},
    protocol::{
        notification, CallToolParams, CallToolResult, Implementation, IncomingMessage,
        InitializeParams, InitializeResult, JsonRpcRequest, JsonRpcResponse, ListToolsParams,
        ListToolsResult, RequestId, ToolDefinition, JSONRPC_INTERNAL_ERROR,
        JSONRPC_METHOD_NOT_FOUND, MCP_PROTOCOL_VERSION, METHOD_INITIALIZE, METHOD_INITIALIZED,
        METHOD_PING, METHOD_TOOLS_CALL, METHOD_TOOLS_LIST, SUPPORTED_PROTOCOL_VERSIONS,
    },
    transport::{FrameTransport, StdioOptions, StdioTransport},
};

/// Guards against a server that never stops handing out pagination cursors.
const MAX_LIST_PAGES: usize = 32;

/// A connected MCP server.
///
/// The client owns the transport and speaks the request/response half of the
/// protocol. Server-initiated requests (sampling, roots) are answered with
/// "method not found" rather than silently ignored, so a server that depends on
/// them fails loudly instead of hanging.
pub struct McpClient {
    server: String,
    transport: Box<dyn FrameTransport>,
    next_id: i64,
    request_timeout: Duration,
    server_info: Option<Implementation>,
    protocol_version: String,
    instructions: Option<String>,
    tools_changed: bool,
}

impl McpClient {
    pub fn new(
        server: impl Into<String>,
        transport: Box<dyn FrameTransport>,
        request_timeout: Duration,
    ) -> Self {
        Self {
            server: server.into(),
            transport,
            next_id: 0,
            request_timeout,
            server_info: None,
            protocol_version: MCP_PROTOCOL_VERSION.to_string(),
            instructions: None,
            tools_changed: false,
        }
    }

    /// Launches a server process and wraps its stdio pipes.
    pub fn spawn_stdio(options: &StdioOptions, request_timeout: Duration) -> Result<Self> {
        let transport = StdioTransport::spawn(options)?;
        Ok(Self::new(
            options.server.clone(),
            Box::new(transport),
            request_timeout,
        ))
    }

    pub fn server_name(&self) -> &str {
        &self.server
    }

    pub fn server_info(&self) -> Option<&Implementation> {
        self.server_info.as_ref()
    }

    pub fn protocol_version(&self) -> &str {
        &self.protocol_version
    }

    pub fn instructions(&self) -> Option<&str> {
        self.instructions.as_deref()
    }

    pub fn server_logs(&self) -> Vec<String> {
        self.transport.server_logs()
    }

    /// True once if the server announced that its tool list changed.
    pub fn take_tools_changed(&mut self) -> bool {
        std::mem::take(&mut self.tools_changed)
    }

    /// Performs the initialize handshake and sends `notifications/initialized`.
    pub async fn initialize(&mut self) -> Result<InitializeResult> {
        let params = serde_json::to_value(InitializeParams {
            protocol_version: MCP_PROTOCOL_VERSION.to_string(),
            capabilities: serde_json::json!({}),
            client_info: Implementation::new("axiom", env!("CARGO_PKG_VERSION")),
        })?;
        let raw = self.request(METHOD_INITIALIZE, Some(params)).await?;
        let result: InitializeResult = serde_json::from_value(raw)?;
        if !SUPPORTED_PROTOCOL_VERSIONS.contains(&result.protocol_version.as_str()) {
            return Err(McpError::UnsupportedProtocolVersion {
                server: self.server.clone(),
                version: result.protocol_version.clone(),
                supported: SUPPORTED_PROTOCOL_VERSIONS.join(", "),
            });
        }
        self.protocol_version = result.protocol_version.clone();
        self.server_info = Some(result.server_info.clone());
        self.instructions = result.instructions.clone();
        self.transport
            .send(&notification(METHOD_INITIALIZED, None))
            .await?;
        Ok(result)
    }

    /// Lists every tool the server advertises, following pagination cursors.
    pub async fn list_tools(&mut self) -> Result<Vec<ToolDefinition>> {
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen = BTreeSet::new();
        for _ in 0..MAX_LIST_PAGES {
            let params = serde_json::to_value(ListToolsParams {
                cursor: cursor.clone(),
            })?;
            let raw = self.request(METHOD_TOOLS_LIST, Some(params)).await?;
            let page: ListToolsResult = serde_json::from_value(raw)?;
            tools.extend(page.tools);
            match page.next_cursor {
                Some(next) if seen.insert(next.clone()) => cursor = Some(next),
                _ => return Ok(tools),
            }
        }
        Err(McpError::Protocol {
            server: self.server.clone(),
            code: JSONRPC_INTERNAL_ERROR,
            message: "server kept returning a pagination cursor for tools/list".to_string(),
        })
    }

    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: Option<Value>,
    ) -> Result<CallToolResult> {
        let params = serde_json::to_value(CallToolParams {
            name: name.to_string(),
            arguments,
        })?;
        let raw = self.request(METHOD_TOOLS_CALL, Some(params)).await?;
        Ok(serde_json::from_value(raw)?)
    }

    pub async fn ping(&mut self) -> Result<()> {
        self.request(METHOD_PING, None).await.map(|_| ())
    }

    /// Closes the transport, killing a spawned server process when present.
    pub async fn close(&mut self) -> Result<()> {
        self.transport.close().await
    }

    async fn request(&mut self, method: &str, params: Option<Value>) -> Result<Value> {
        self.next_id = self.next_id.saturating_add(1);
        let id = RequestId::Number(self.next_id);
        let request = JsonRpcRequest::new(Some(id.clone()), method, params);
        self.transport
            .send(&serde_json::to_value(&request)?)
            .await?;

        let timeout = self.request_timeout;
        let server = self.server.clone();
        let awaited_method = method.to_string();
        tokio::time::timeout(timeout, self.await_response(&id))
            .await
            .map_err(|_| McpError::Timeout {
                server,
                method: awaited_method,
                seconds: timeout.as_secs(),
            })?
    }

    async fn await_response(&mut self, expected: &RequestId) -> Result<Value> {
        loop {
            let Some(frame) = self.transport.receive().await? else {
                return Err(McpError::Closed {
                    server: self.server.clone(),
                });
            };
            let incoming: IncomingMessage = serde_json::from_value(frame)?;
            if !incoming.has_supported_version() {
                return Err(McpError::Protocol {
                    server: self.server.clone(),
                    code: crate::protocol::JSONRPC_INVALID_REQUEST,
                    message: format!(
                        "server sent a frame that is not JSON-RPC 2.0: {:?}",
                        incoming.jsonrpc
                    ),
                });
            }
            match incoming.classify() {
                crate::protocol::Incoming::Response { id, result, error } => {
                    if id.as_ref() != Some(expected) {
                        continue;
                    }
                    if let Some(error) = error {
                        return Err(McpError::Protocol {
                            server: self.server.clone(),
                            code: error.code,
                            message: error.message,
                        });
                    }
                    return result.ok_or_else(|| McpError::Protocol {
                        server: self.server.clone(),
                        code: JSONRPC_INTERNAL_ERROR,
                        message: "response carried neither a result nor an error".to_string(),
                    });
                }
                crate::protocol::Incoming::Notification { method, .. } => {
                    self.handle_notification(&method);
                }
                crate::protocol::Incoming::Request { id, method, .. } => {
                    let response = JsonRpcResponse::error(
                        Some(id),
                        JSONRPC_METHOD_NOT_FOUND,
                        format!("axiom does not support the `{method}` request"),
                    );
                    self.transport
                        .send(&serde_json::to_value(&response)?)
                        .await?;
                }
            }
        }
    }

    fn handle_notification(&mut self, method: &str) {
        if method == "notifications/tools/list_changed" {
            self.tools_changed = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use serde_json::json;

    use super::*;
    use crate::transport::{channel_transport_pair, ChannelTransport, FrameTransport};

    /// Drives a scripted MCP server on the far side of an in-process channel,
    /// recording every request it observes.
    fn spawn_fake_server(
        handler: impl Fn(&str, &Value) -> Option<Value> + Send + 'static,
    ) -> (ChannelTransport, Arc<Mutex<Vec<Value>>>) {
        let (client_side, mut server_side) = channel_transport_pair();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&observed);
        tokio::spawn(async move {
            while let Ok(Some(frame)) = server_side.receive().await {
                let incoming: IncomingMessage = match serde_json::from_value(frame.clone()) {
                    Ok(incoming) => incoming,
                    Err(_) => break,
                };
                if let Ok(mut recorder) = recorder.lock() {
                    recorder.push(frame);
                }
                if let crate::protocol::Incoming::Request { id, method, params } =
                    incoming.classify()
                {
                    let response = match handler(&method, &params.unwrap_or(Value::Null)) {
                        Some(result) => JsonRpcResponse::success(id, result),
                        None => JsonRpcResponse::error(
                            Some(id),
                            JSONRPC_METHOD_NOT_FOUND,
                            "unsupported",
                        ),
                    };
                    if server_side
                        .send(&serde_json::to_value(&response).expect("serialize response"))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        });
        (client_side, observed)
    }

    fn fake_client(transport: ChannelTransport, timeout: Duration) -> McpClient {
        McpClient::new("fake", Box::new(transport), timeout)
    }

    #[tokio::test]
    async fn handshake_lists_paginated_tools_and_calls_one() {
        let (client_side, observed) = spawn_fake_server(|method, params| match method {
            METHOD_INITIALIZE => Some(json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {"tools": {"listChanged": true}},
                "serverInfo": {"name": "fake", "version": "1.0.0"},
                "instructions": "be nice",
            })),
            METHOD_TOOLS_LIST => {
                if params.get("cursor").and_then(Value::as_str).is_none() {
                    Some(json!({
                        "tools": [{
                            "name": "first",
                            "description": "first tool",
                            "inputSchema": {"type": "object"},
                            "annotations": {"readOnlyHint": true},
                        }],
                        "nextCursor": "page-2",
                    }))
                } else {
                    Some(json!({
                        "tools": [{
                            "name": "second",
                            "inputSchema": {"type": "object"},
                        }],
                    }))
                }
            }
            METHOD_TOOLS_CALL => Some(json!({
                "content": [{"type": "text", "text": "hello"}],
                "isError": false,
            })),
            _ => None,
        });

        let mut client = fake_client(client_side, Duration::from_secs(5));
        let initialized = client.initialize().await.expect("handshake");
        assert_eq!(initialized.server_info.name, "fake");
        assert_eq!(initialized.instructions.as_deref(), Some("be nice"));

        let tools = client.list_tools().await.expect("list tools");
        assert_eq!(
            tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            vec!["first", "second"]
        );

        let result = client
            .call_tool("first", Some(json!({"q": "x"})))
            .await
            .expect("call tool");
        assert_eq!(result.text(), "hello");

        let requests = observed.lock().unwrap().clone();
        let methods = requests
            .iter()
            .map(|request| request["method"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert_eq!(
            methods,
            vec![
                METHOD_INITIALIZE,
                METHOD_INITIALIZED,
                METHOD_TOOLS_LIST,
                METHOD_TOOLS_LIST,
                METHOD_TOOLS_CALL,
            ]
        );
    }

    #[tokio::test]
    async fn rejects_unsupported_protocol_versions() {
        let (client_side, _observed) = spawn_fake_server(|method, _| {
            (method == METHOD_INITIALIZE).then(|| {
                json!({
                    "protocolVersion": "1999-01-01",
                    "capabilities": {},
                    "serverInfo": {"name": "ancient", "version": "0.0.1"},
                })
            })
        });

        let mut client = fake_client(client_side, Duration::from_secs(5));
        let error = client.initialize().await.expect_err("must reject");
        assert!(matches!(error, McpError::UnsupportedProtocolVersion { .. }));
    }

    #[tokio::test]
    async fn answers_unsupported_server_requests_instead_of_hanging() {
        let (client_side, mut server_side) = channel_transport_pair();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let recorder = Arc::clone(&observed);
        tokio::spawn(async move {
            // Consume initialize and reply.
            let _ = server_side.receive().await;
            server_side
                .send(&json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": {
                        "protocolVersion": "2025-06-18",
                        "capabilities": {},
                        "serverInfo": {"name": "fake", "version": "1.0.0"},
                    },
                }))
                .await
                .expect("initialize reply");
            // Consume the initialized notification.
            let _ = server_side.receive().await;
            // Ask the client for sampling, which it must refuse explicitly.
            server_side
                .send(&json!({
                    "jsonrpc": "2.0",
                    "id": "server-1",
                    "method": "sampling/createMessage",
                    "params": {},
                }))
                .await
                .expect("sampling request");
            // Record the client's refusal. Its ping request may arrive first, so
            // skip anything that is itself a request.
            while let Ok(Some(frame)) = server_side.receive().await {
                if frame.get("method").is_none() {
                    recorder.lock().expect("recorder").push(frame);
                    break;
                }
            }
            // Answer the ping (the second request the client sends).
            server_side
                .send(&json!({"jsonrpc": "2.0", "id": 2, "result": {}}))
                .await
                .expect("ping reply");
        });

        let mut client = fake_client(client_side, Duration::from_secs(5));
        client.initialize().await.expect("handshake");
        client.ping().await.expect("ping survives server request");

        let replies = observed.lock().expect("recorder").clone();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0]["error"]["code"], json!(-32601));
        assert_eq!(replies[0]["id"], json!("server-1"));
    }

    #[tokio::test]
    async fn reports_protocol_errors_and_timeouts() {
        let (client_side, _observed) = spawn_fake_server(|method, _| {
            if method == METHOD_INITIALIZE {
                Some(json!({
                    "protocolVersion": "2025-06-18",
                    "capabilities": {},
                    "serverInfo": {"name": "fake", "version": "1.0.0"},
                }))
            } else {
                None
            }
        });

        let mut client = fake_client(client_side, Duration::from_millis(50));
        client.initialize().await.expect("handshake");
        let error = client.list_tools().await.expect_err("unknown method");
        assert!(matches!(error, McpError::Protocol { code: -32601, .. }));
    }

    #[tokio::test]
    async fn timeouts_are_reported_when_a_server_goes_silent() {
        let (client_side, server_side) = channel_transport_pair();
        // Keep the peer alive so the timeout, not an EOF, is what fires.
        let _silent_peer = server_side;
        let mut client = fake_client(client_side, Duration::from_millis(30));

        let error = client.ping().await.expect_err("server never answers");
        assert!(matches!(error, McpError::Timeout { .. }));
    }

    #[tokio::test]
    async fn closed_transport_is_reported() {
        let (client_side, server_side) = channel_transport_pair();
        drop(server_side);
        let mut client = fake_client(client_side, Duration::from_secs(5));

        let error = client.list_tools().await.expect_err("closed transport");
        assert!(matches!(error, McpError::Closed { .. }));
    }
}
