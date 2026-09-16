use thiserror::Error;

/// Failures that can occur while talking to (or acting as) an MCP server.
#[derive(Debug, Error)]
pub enum McpError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse MCP message: {0}")]
    Json(#[from] serde_json::Error),
    #[error("MCP server `{server}` returned error {code}: {message}")]
    Protocol {
        server: String,
        code: i64,
        message: String,
    },
    #[error("MCP server `{server}` did not answer `{method}` within {seconds}s")]
    Timeout {
        server: String,
        method: String,
        seconds: u64,
    },
    #[error("MCP server `{server}` closed the connection before responding")]
    Closed { server: String },
    #[error("MCP server `{server}` sent a message larger than the {limit}-byte frame limit")]
    FrameTooLarge { server: String, limit: usize },
    #[error("MCP server `{server}` is not configured")]
    UnknownServer { server: String },
    #[error("MCP tool `{tool}` is not provided by any configured server")]
    UnknownTool { tool: String },
    #[error("MCP server `{server}` published an unusable tool definition: {message}")]
    InvalidTool { server: String, message: String },
    #[error("failed to start MCP server `{server}`: {message}")]
    Spawn { server: String, message: String },
    #[error(
        "MCP server `{server}` speaks protocol version {version}, which this Axiom build does not support (it supports {supported})"
    )]
    UnsupportedProtocolVersion {
        server: String,
        version: String,
        supported: String,
    },
    #[error("no MCP servers are enabled; add one under [[mcp.servers]] in the Axiom config")]
    NoServers,
}

pub type Result<T> = std::result::Result<T, McpError>;
