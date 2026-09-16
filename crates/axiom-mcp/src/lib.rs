//! Model Context Protocol support for Axiom.
//!
//! Axiom speaks MCP in both directions:
//!
//! * [`McpToolSource`] is an MCP **client**. It launches the servers declared in
//!   `[[mcp.servers]]`, lists their tools, and wraps every remote tool as an
//!   ordinary Axiom tool — so each call is validated and then authorized through
//!   the very same [`axiom_engine::SideEffectPolicy`] and approval hooks that
//!   guard Axiom's built-ins. Remote tools are never silently trusted: the
//!   classes derived from the server's own annotations (or an explicit config
//!   override) decide what the policy sees.
//! * [`McpServer`] is an MCP **server**. It exposes Axiom's tool registry to
//!   other MCP clients over stdio, gated by the same policy plus a
//!   non-interactive approval mode (deny-by-default for `ask` decisions).
//!
//! Both directions share the JSON-RPC framing in [`transport`] and the protocol
//! types in [`protocol`].

mod client;
mod error;
mod gate;
mod protocol;
mod server;
mod tools;
mod transport;

pub use client::McpClient;
pub use error::{McpError, Result};
pub use gate::{classes_for_annotations, parse_side_effect_class, permissions_for_classes};
pub use protocol::{
    CallToolResult, ContentBlock, Implementation, InitializeResult, ListToolsResult,
    ToolAnnotations, ToolDefinition, MCP_PROTOCOL_VERSION, SUPPORTED_PROTOCOL_VERSIONS,
};
pub use server::{McpServer, McpServerOptions, ServerTool};
pub use tools::{McpToolDefinition, McpToolSource};
pub use transport::{
    channel_transport_pair, read_frame_blocking, write_frame_blocking, ChannelTransport,
    FrameTransport, StdinStdoutTransport, StdioOptions, StdioTransport, DEFAULT_MAX_FRAME_BYTES,
};
