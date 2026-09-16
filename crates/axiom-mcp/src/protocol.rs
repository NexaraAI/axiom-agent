use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Protocol revision Axiom advertises first.
pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

/// Revisions this build understands, newest first.
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

pub const JSONRPC_VERSION: &str = "2.0";

pub const METHOD_INITIALIZE: &str = "initialize";
pub const METHOD_INITIALIZED: &str = "notifications/initialized";
pub const METHOD_TOOLS_LIST: &str = "tools/list";
pub const METHOD_TOOLS_CALL: &str = "tools/call";
pub const METHOD_PING: &str = "ping";

pub const JSONRPC_PARSE_ERROR: i64 = -32700;
pub const JSONRPC_INVALID_REQUEST: i64 = -32600;
pub const JSONRPC_METHOD_NOT_FOUND: i64 = -32601;
pub const JSONRPC_INVALID_PARAMS: i64 = -32602;
pub const JSONRPC_INTERNAL_ERROR: i64 = -32603;

/// JSON-RPC allows string or number request identifiers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    String(String),
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Number(value) => write!(formatter, "{value}"),
            Self::String(value) => formatter.write_str(value),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<RequestId>,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(id: Option<RequestId>, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            method: method.into(),
            params,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<RequestId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    pub fn success(id: RequestId, result: Value) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: Some(id),
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Option<RequestId>, code: i64, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data: None,
            }),
        }
    }
}

/// A decoded inbound frame, parsed permissively because a single stream carries
/// requests, notifications, and responses.
#[derive(Debug, Clone, Deserialize)]
pub struct IncomingMessage {
    #[serde(default)]
    pub jsonrpc: Option<String>,
    #[serde(default)]
    pub id: Option<RequestId>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<JsonRpcError>,
}

/// What an inbound frame actually is.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    /// A request that must be answered.
    Request {
        id: RequestId,
        method: String,
        params: Option<Value>,
    },
    /// A fire-and-forget notification that must never be answered.
    Notification {
        method: String,
        params: Option<Value>,
    },
    /// The answer to a request we sent.
    Response {
        id: Option<RequestId>,
        result: Option<Value>,
        error: Option<JsonRpcError>,
    },
}

impl IncomingMessage {
    /// JSON-RPC 2.0 requests and responses must declare their version.
    pub fn has_supported_version(&self) -> bool {
        self.jsonrpc
            .as_deref()
            .is_none_or(|version| version == JSONRPC_VERSION)
    }

    pub fn classify(&self) -> Incoming {
        match (&self.method, &self.id) {
            (Some(method), Some(id)) => Incoming::Request {
                id: id.clone(),
                method: method.clone(),
                params: self.params.clone(),
            },
            (Some(method), None) => Incoming::Notification {
                method: method.clone(),
                params: self.params.clone(),
            },
            (None, _) => Incoming::Response {
                id: self.id.clone(),
                result: self.result.clone(),
                error: self.error.clone(),
            },
        }
    }
}

/// Identity a peer announces during the handshake.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Implementation {
    pub name: String,
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

impl Implementation {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            title: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InitializeParams {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    #[serde(default)]
    pub capabilities: Value,
    #[serde(rename = "clientInfo")]
    pub client_info: Implementation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InitializeResult {
    #[serde(rename = "protocolVersion")]
    pub protocol_version: String,
    #[serde(default)]
    pub capabilities: Value,
    #[serde(rename = "serverInfo")]
    pub server_info: Implementation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// Hints a server publishes about a tool's behaviour.
///
/// Absent hints take the protocol's defaults (not read-only, destructive,
/// open-world), which is what makes an un-annotated third-party tool gate as a
/// write plus a network call rather than slipping through as a plain read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolAnnotations {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(
        rename = "readOnlyHint",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub read_only_hint: Option<bool>,
    #[serde(
        rename = "destructiveHint",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub destructive_hint: Option<bool>,
    #[serde(
        rename = "idempotentHint",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub idempotent_hint: Option<bool>,
    #[serde(
        rename = "openWorldHint",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub open_world_hint: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
    #[serde(
        rename = "outputSchema",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub output_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ToolAnnotations>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ListToolsParams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ListToolsResult {
    #[serde(default)]
    pub tools: Vec<ToolDefinition>,
    #[serde(
        rename = "nextCursor",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallToolParams {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
}

/// One block of a tool result. Unmodelled fields are preserved so nothing a
/// server sends is silently dropped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContentBlock {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(rename = "mimeType", default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content_type: "text".to_string(),
            text: Some(text.into()),
            data: None,
            mime_type: None,
            uri: None,
            extra: serde_json::Map::new(),
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        self.text.as_deref()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CallToolResult {
    #[serde(default)]
    pub content: Vec<ContentBlock>,
    #[serde(
        rename = "structuredContent",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub structured_content: Option<Value>,
    #[serde(rename = "isError", default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

impl CallToolResult {
    /// Concatenates every textual block, which is what a model can consume.
    pub fn text(&self) -> String {
        self.content
            .iter()
            .filter_map(ContentBlock::as_text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn is_error(&self) -> bool {
        self.is_error.unwrap_or(false)
    }
}

/// Builds a JSON-RPC notification envelope, omitting `params` when there are
/// none (as the protocol requires for `notifications/initialized`).
pub fn notification(method: &str, params: Option<Value>) -> Value {
    match params {
        Some(params) => serde_json::json!({
            "jsonrpc": JSONRPC_VERSION,
            "method": method,
            "params": params,
        }),
        None => serde_json::json!({
            "jsonrpc": JSONRPC_VERSION,
            "method": method,
        }),
    }
}

/// Selects the revision to answer with, preferring the client's choice when
/// this build supports it.
pub fn negotiate_protocol_version(requested: &str) -> Option<String> {
    if SUPPORTED_PROTOCOL_VERSIONS.contains(&requested) {
        return Some(requested.to_string());
    }
    SUPPORTED_PROTOCOL_VERSIONS.first().map(|v| v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn notifications_omit_absent_params() {
        assert_eq!(
            notification(METHOD_INITIALIZED, None),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        );
        assert_eq!(
            notification(METHOD_PING, Some(json!({}))),
            json!({"jsonrpc": "2.0", "method": "ping", "params": {}})
        );
    }

    #[test]
    fn rejects_frames_that_are_not_jsonrpc_2_0() {
        let modern: IncomingMessage =
            serde_json::from_value(json!({"jsonrpc": "2.0", "method": "x"})).expect("parses");
        assert!(modern.has_supported_version());

        let legacy: IncomingMessage =
            serde_json::from_value(json!({"jsonrpc": "1.0", "method": "x"})).expect("parses");
        assert!(!legacy.has_supported_version());
    }

    #[test]
    fn classifies_requests_notifications_and_responses() {
        let request: IncomingMessage =
            serde_json::from_value(json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
                .expect("request parses");
        assert_eq!(
            request.classify(),
            Incoming::Request {
                id: RequestId::Number(1),
                method: "tools/list".to_string(),
                params: None,
            }
        );

        let notification: IncomingMessage = serde_json::from_value(
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        )
        .expect("notification parses");
        assert!(matches!(
            notification.classify(),
            Incoming::Notification { .. }
        ));

        let response: IncomingMessage =
            serde_json::from_value(json!({"jsonrpc": "2.0", "id": "a", "result": {"ok": true}}))
                .expect("response parses");
        match response.classify() {
            Incoming::Response { id, result, error } => {
                assert_eq!(id, Some(RequestId::String("a".to_string())));
                assert_eq!(result, Some(json!({"ok": true})));
                assert!(error.is_none());
            }
            other => panic!("expected response, got {other:?}"),
        }
    }

    #[test]
    fn serializes_tool_definitions_with_protocol_field_names() {
        let definition = ToolDefinition {
            name: "search".to_string(),
            title: None,
            description: Some("Search things".to_string()),
            input_schema: json!({"type": "object"}),
            output_schema: None,
            annotations: Some(ToolAnnotations {
                read_only_hint: Some(true),
                ..ToolAnnotations::default()
            }),
        };
        let encoded = serde_json::to_value(&definition).expect("serialize tool");

        assert_eq!(encoded["inputSchema"], json!({"type": "object"}));
        assert_eq!(encoded["annotations"]["readOnlyHint"], json!(true));
        assert!(encoded.get("outputSchema").is_none());
    }

    #[test]
    fn content_blocks_keep_unknown_fields_and_join_text() {
        let result: CallToolResult = serde_json::from_value(json!({
            "content": [
                {"type": "text", "text": "first"},
                {"type": "image", "data": "AAAA", "mimeType": "image/png", "annotations": {"audience": []}},
            ],
            "isError": false
        }))
        .expect("call result parses");

        assert_eq!(result.text(), "first");
        assert!(!result.is_error());
        assert!(result.content[1].extra.contains_key("annotations"));
    }

    #[test]
    fn negotiates_supported_protocol_versions() {
        assert_eq!(
            negotiate_protocol_version("2024-11-05").as_deref(),
            Some("2024-11-05")
        );
        assert_eq!(
            negotiate_protocol_version("1999-01-01").as_deref(),
            SUPPORTED_PROTOCOL_VERSIONS.first().copied()
        );
    }

    #[test]
    fn error_responses_omit_result() {
        let response = JsonRpcResponse::error(Some(RequestId::Number(7)), -32601, "nope");
        let encoded = serde_json::to_value(&response).expect("serialize");

        assert!(encoded.get("result").is_none());
        assert_eq!(encoded["error"]["code"], json!(-32601));
        assert_eq!(encoded["id"], json!(7));
    }
}
