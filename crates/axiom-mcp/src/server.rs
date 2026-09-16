use std::collections::BTreeSet;

use axiom_engine::{
    builtin_installed_skill, execute_installed_tool_with_policy, ExecutorRegistry, InstalledSkill,
    SideEffectClass, SideEffectPolicy, SkillApproval, SkillExecutionContext, SkillExecutionError,
    ToolRequest,
};
use serde_json::{json, Value};

use crate::{
    error::Result,
    protocol::{
        CallToolParams, CallToolResult, ContentBlock, Implementation, IncomingMessage,
        JsonRpcResponse, ListToolsParams, ListToolsResult, RequestId, ToolDefinition,
        JSONRPC_INVALID_REQUEST, JSONRPC_METHOD_NOT_FOUND, JSONRPC_PARSE_ERROR, METHOD_INITIALIZE,
        METHOD_PING, METHOD_TOOLS_CALL, METHOD_TOOLS_LIST,
    },
    transport::FrameTransport,
};

/// Tools that make no sense outside Axiom's own interactive session.
const UNEXPOSED_SKILLS: &[&str] = &["question.ask", "subagent.run", "skill.create"];

/// How `axiom mcp serve` behaves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServerOptions {
    pub name: String,
    pub version: String,
    /// Approve `ask` decisions instead of failing them. Deny is never relaxed.
    pub auto_approve: bool,
    /// Expose only tools that cannot mutate anything outside the workspace.
    pub read_only: bool,
    /// Explicit allow list of Axiom skill ids. Empty means "all supported".
    pub allow_tools: Vec<String>,
    pub deny_tools: Vec<String>,
}

impl Default for McpServerOptions {
    fn default() -> Self {
        Self {
            name: "axiom".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            auto_approve: false,
            read_only: false,
            allow_tools: Vec::new(),
            deny_tools: Vec::new(),
        }
    }
}

/// One Axiom tool as advertised over MCP.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerTool {
    pub skill_id: String,
    pub exposed_name: String,
    pub description: String,
    pub input_schema: Value,
    pub side_effects: Vec<SideEffectClass>,
}

/// An MCP server exposing Axiom's built-in tools to MCP clients.
pub struct McpServer {
    options: McpServerOptions,
    tools: Vec<ServerTool>,
    context: SkillExecutionContext,
    installed_skills: Vec<InstalledSkill>,
    policy: SideEffectPolicy,
}

impl McpServer {
    pub fn new(
        options: McpServerOptions,
        context: SkillExecutionContext,
        installed_skills: Vec<InstalledSkill>,
        policy: SideEffectPolicy,
    ) -> Self {
        let tools = advertised_tools(&options, &installed_skills);
        Self {
            options,
            tools,
            context,
            installed_skills,
            policy,
        }
    }

    pub fn tools(&self) -> &[ServerTool] {
        &self.tools
    }

    /// Serves requests until the peer closes the stream.
    pub async fn serve<T: FrameTransport>(&mut self, transport: &mut T) -> Result<()> {
        while let Some(frame) = transport.receive().await? {
            let incoming: IncomingMessage = match serde_json::from_value(frame) {
                Ok(incoming) => incoming,
                Err(error) => {
                    let response =
                        JsonRpcResponse::error(None, JSONRPC_PARSE_ERROR, error.to_string());
                    transport.send(&serde_json::to_value(response)?).await?;
                    continue;
                }
            };
            if !incoming.has_supported_version() {
                let response = JsonRpcResponse::error(
                    incoming.id.clone(),
                    JSONRPC_INVALID_REQUEST,
                    "axiom mcp serve only speaks JSON-RPC 2.0",
                );
                transport.send(&serde_json::to_value(response)?).await?;
                continue;
            }
            let response = match incoming.classify() {
                // Notifications are never answered.
                crate::protocol::Incoming::Notification { .. } => continue,
                // We never send requests, so a response is unsolicited.
                crate::protocol::Incoming::Response { .. } => continue,
                crate::protocol::Incoming::Request { id, method, params } => {
                    self.dispatch(id, &method, params).await
                }
            };
            transport.send(&serde_json::to_value(response)?).await?;
        }
        Ok(())
    }

    async fn dispatch(
        &self,
        id: RequestId,
        method: &str,
        params: Option<Value>,
    ) -> JsonRpcResponse {
        match method {
            METHOD_INITIALIZE => {
                let requested = params
                    .as_ref()
                    .and_then(|params| params.get("protocolVersion"))
                    .and_then(Value::as_str)
                    .unwrap_or(crate::protocol::MCP_PROTOCOL_VERSION);
                let negotiated = crate::protocol::negotiate_protocol_version(requested);
                let Some(protocol_version) = negotiated else {
                    return JsonRpcResponse::error(
                        Some(id),
                        JSONRPC_INVALID_REQUEST,
                        "no supported MCP protocol version",
                    );
                };
                JsonRpcResponse::success(
                    id,
                    json!({
                        "protocolVersion": protocol_version,
                        "capabilities": {"tools": {"listChanged": false}},
                        "serverInfo": Implementation::new(&self.options.name, &self.options.version),
                        "instructions": self.instructions(),
                    }),
                )
            }
            METHOD_PING => JsonRpcResponse::success(id, json!({})),
            METHOD_TOOLS_LIST => {
                let _params: ListToolsParams = match params.clone() {
                    Some(params) => match serde_json::from_value(params) {
                        Ok(params) => params,
                        Err(error) => {
                            return JsonRpcResponse::error(
                                Some(id),
                                crate::protocol::JSONRPC_INVALID_PARAMS,
                                error.to_string(),
                            )
                        }
                    },
                    None => ListToolsParams::default(),
                };
                JsonRpcResponse::success(
                    id,
                    serde_json::to_value(ListToolsResult {
                        tools: self
                            .tools
                            .iter()
                            .map(|tool| ToolDefinition {
                                name: tool.exposed_name.clone(),
                                title: None,
                                description: Some(tool.description.clone()),
                                input_schema: tool.input_schema.clone(),
                                output_schema: None,
                                annotations: annotations_for(tool),
                            })
                            .collect(),
                        next_cursor: None,
                    })
                    .unwrap_or(Value::Null),
                )
            }
            METHOD_TOOLS_CALL => {
                let params: CallToolParams = match params.clone() {
                    Some(params) => match serde_json::from_value(params) {
                        Ok(params) => params,
                        Err(error) => {
                            return JsonRpcResponse::error(
                                Some(id),
                                crate::protocol::JSONRPC_INVALID_PARAMS,
                                error.to_string(),
                            )
                        }
                    },
                    None => {
                        return JsonRpcResponse::error(
                            Some(id),
                            crate::protocol::JSONRPC_INVALID_PARAMS,
                            "tools/call requires params",
                        )
                    }
                };
                match self.call_tool(&params).await {
                    Ok(result) => JsonRpcResponse::success(
                        id,
                        serde_json::to_value(result).unwrap_or(Value::Null),
                    ),
                    Err(message) => JsonRpcResponse::success(
                        id,
                        serde_json::to_value(CallToolResult {
                            content: vec![ContentBlock::text(message)],
                            structured_content: None,
                            is_error: Some(true),
                        })
                        .unwrap_or(Value::Null),
                    ),
                }
            }
            other => JsonRpcResponse::error(
                Some(id),
                JSONRPC_METHOD_NOT_FOUND,
                format!("axiom mcp serve does not implement `{other}`"),
            ),
        }
    }

    fn instructions(&self) -> String {
        let mut instructions = String::from(
            "Axiom exposes its workspace tools over MCP. Every call is checked against Axiom's \
             side-effect policy; tools that need approval are refused unless the server was \
             started with --approve.",
        );
        if self.options.read_only {
            instructions.push_str(" This server is running in read-only mode.");
        }
        instructions
    }

    async fn call_tool(
        &self,
        params: &CallToolParams,
    ) -> std::result::Result<CallToolResult, String> {
        let tool = self
            .tools
            .iter()
            .find(|tool| {
                tool.exposed_name == params.name
                    || tool.skill_id == params.name
                    || tool.skill_id.replace('.', "_") == params.name
            })
            .ok_or_else(|| format!("unknown Axiom tool `{}`", params.name))?;

        let request = ToolRequest {
            skill_id: tool.skill_id.clone(),
            arguments: params.arguments.clone().unwrap_or_else(|| json!({})),
        };
        let mut approver = PolicyApprover {
            auto_approve: self.options.auto_approve,
        };
        let mut audit = axiom_engine::RecordingSideEffectAuditSink::default();
        let mut skills = self.installed_skills.clone();
        if !skills
            .iter()
            .any(|skill| skill.manifest.id == tool.skill_id)
        {
            if let Some(skill) = builtin_installed_skill(&tool.skill_id) {
                skills.push(skill);
            }
        }

        match execute_installed_tool_with_policy(
            &request,
            &skills,
            &self.context,
            &mut approver,
            &self.policy,
            &mut audit,
        )
        .await
        {
            Ok(result) => Ok(CallToolResult {
                content: vec![ContentBlock::text(result.output.to_string())],
                structured_content: Some(result.output),
                is_error: Some(false),
            }),
            Err(error) => Err(describe_failure(&error, &self.policy)),
        }
    }
}

/// Non-interactive approval policy: `ask` is only granted when the operator
/// explicitly started the server with `--approve`.
struct PolicyApprover {
    auto_approve: bool,
}

impl SkillApproval for PolicyApprover {
    fn approve(&mut self, _request: &axiom_engine::ApprovalRequest) -> bool {
        self.auto_approve
    }
}

fn describe_failure(error: &SkillExecutionError, policy: &SideEffectPolicy) -> String {
    match error {
        SkillExecutionError::SkillBlocked {
            skill_id,
            state,
            trust,
        } => {
            format!("Axiom refused `{skill_id}`: the skill is {state:?} with trust level {trust:?}")
        }
        SkillExecutionError::SideEffectPolicyDenied(decision) => format!(
            "Axiom refused this call: {decision} (policy: {policy:?}). Start the server with \
             --approve for ask-level operations, or adjust `[policy]` in the Axiom config."
        ),
        SkillExecutionError::ApprovalDenied(reason) => format!("Axiom refused this call: {reason}"),
        other => other.to_string(),
    }
}

/// A tool is read-only when it can neither mutate the workspace nor run a
/// process. Reads of the workspace and of the network qualify.
fn is_read_only(classes: &[SideEffectClass]) -> bool {
    classes.iter().all(|class| {
        matches!(
            class,
            SideEffectClass::FilesystemRead | SideEffectClass::Network
        )
    })
}

fn annotations_for(tool: &ServerTool) -> Option<crate::protocol::ToolAnnotations> {
    let read_only = is_read_only(&tool.side_effects);
    Some(crate::protocol::ToolAnnotations {
        title: None,
        read_only_hint: Some(read_only),
        destructive_hint: Some(!read_only),
        idempotent_hint: None,
        open_world_hint: Some(tool.side_effects.contains(&SideEffectClass::Network)),
    })
}

/// Selects the tools this server advertises, honouring the lifecycle state of
/// installed skills so a disabled skill is not offered.
fn advertised_tools(
    options: &McpServerOptions,
    installed_skills: &[InstalledSkill],
) -> Vec<ServerTool> {
    let registry = ExecutorRegistry::with_builtin_executors();
    let mut tools = Vec::new();
    let mut seen = BTreeSet::new();
    for descriptor in registry.descriptors() {
        if UNEXPOSED_SKILLS.contains(&descriptor.id.as_str()) {
            continue;
        }
        if !seen.insert(descriptor.id.clone()) {
            continue;
        }
        if !options.allow_tools.is_empty() && !options.allow_tools.contains(&descriptor.id) {
            continue;
        }
        if options.deny_tools.contains(&descriptor.id) {
            continue;
        }
        if options.read_only && !is_read_only(&descriptor.side_effects) {
            continue;
        }
        if let Some(record) = installed_skills
            .iter()
            .find(|skill| skill.manifest.id == descriptor.id)
        {
            if !record.record.is_executable() {
                continue;
            }
        }

        let (description, side_effects) = match builtin_installed_skill(&descriptor.id) {
            Some(skill) => (
                format!("{}: {}", skill.manifest.name, skill.manifest.description),
                descriptor.side_effects.clone(),
            ),
            None => (descriptor.id.clone(), descriptor.side_effects.clone()),
        };
        let mut side_effects = side_effects;
        side_effects.sort_unstable();
        side_effects.dedup();

        tools.push(ServerTool {
            exposed_name: descriptor.id.replace('.', "_"),
            skill_id: descriptor.id.clone(),
            description,
            input_schema: descriptor.input_schema.clone(),
            side_effects,
        });
    }
    tools.sort_by(|left, right| left.exposed_name.cmp(&right.exposed_name));
    tools
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{channel_transport_pair, ChannelTransport, FrameTransport};
    use axiom_engine::{AllowAllApprover, DenyAllApprover};
    use std::path::PathBuf;

    fn context() -> SkillExecutionContext {
        SkillExecutionContext {
            workspace_root: PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            max_file_read_bytes: 1_000_000,
            web_timeout_secs: 5,
            max_web_response_bytes: 100_000,
            web_fetch_https_only: true,
            web_fetch_allowed_hosts: Vec::new(),
            web_fetch_denied_hosts: Vec::new(),
            web_fetch_use_system_proxy: false,
            auto_approve_medium_risk: false,
            credential_env_names: Vec::new(),
            skills_dir: None,
        }
    }

    fn server(options: McpServerOptions, policy: SideEffectPolicy) -> McpServer {
        McpServer::new(options, context(), Vec::new(), policy)
    }

    /// Serves `server` on a dedicated thread and returns the client side of the
    /// link. `serve` is deliberately not `Send` (tool execution borrows Axiom's
    /// non-`Send` approval and audit hooks), so it gets its own runtime.
    fn start_server(server: McpServer) -> ChannelTransport {
        let (client_side, mut server_side) = channel_transport_pair();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("server runtime");
            runtime.block_on(async move {
                let mut server = server;
                let _ = server.serve(&mut server_side).await;
            });
        });
        client_side
    }

    async fn call(transport: &mut ChannelTransport, id: i64, method: &str, params: Value) -> Value {
        transport
            .send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .await
            .expect("write request");
        transport
            .receive()
            .await
            .expect("read response")
            .expect("response frame")
    }

    #[tokio::test]
    async fn serves_handshake_ping_and_tool_listing() {
        let server = server(McpServerOptions::default(), SideEffectPolicy::strict());
        let mut transport = start_server(server);

        let handshake = call(
            &mut transport,
            1,
            METHOD_INITIALIZE,
            json!({"protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "c", "version": "1"}}),
        )
        .await;
        assert_eq!(handshake["result"]["protocolVersion"], json!("2024-11-05"));
        assert_eq!(handshake["result"]["serverInfo"]["name"], json!("axiom"));

        let ping = call(&mut transport, 2, METHOD_PING, json!({})).await;
        assert_eq!(ping["result"], json!({}));

        let listed = call(&mut transport, 3, METHOD_TOOLS_LIST, json!({})).await;
        let names = listed["result"]["tools"]
            .as_array()
            .expect("tools array")
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_string())
            .collect::<Vec<_>>();
        assert!(names.contains(&"file_read".to_string()));
        assert!(names.contains(&"shell_powershell_safe".to_string()));
        assert!(names.contains(&"git_status".to_string()));
        assert!(!names.contains(&"question_ask".to_string()));
        assert!(!names.contains(&"subagent_run".to_string()));
        assert!(!names.contains(&"skill_create".to_string()));
    }

    #[tokio::test]
    async fn unknown_methods_and_notifications_are_handled() {
        let server = server(McpServerOptions::default(), SideEffectPolicy::strict());
        let mut transport = start_server(server);

        let response = call(&mut transport, 9, "resources/list", json!({})).await;
        assert_eq!(response["error"]["code"], json!(-32601));

        // A notification must not produce a reply.
        transport
            .send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))
            .await
            .expect("write notification");
        let follow_up = call(&mut transport, 10, METHOD_PING, json!({})).await;
        assert_eq!(follow_up["id"], json!(10));
    }

    #[tokio::test]
    async fn read_only_mode_hides_mutating_tools() {
        let options = McpServerOptions {
            read_only: true,
            ..McpServerOptions::default()
        };
        let server = server(options, SideEffectPolicy::strict());
        let names = server
            .tools()
            .iter()
            .map(|tool| tool.skill_id.as_str())
            .collect::<Vec<_>>();

        assert!(names.contains(&"file.read"));
        assert!(names.contains(&"web.fetch"));
        assert!(!names.contains(&"git.diff"), "git spawns a process");
        assert!(!names.contains(&"file.write"));
        assert!(!names.contains(&"shell.run"));
        assert!(!names.contains(&"test.run"));
        assert!(!names.contains(&"python.run"));
    }

    #[tokio::test]
    async fn tool_calls_are_gated_by_the_policy() {
        let server = server(McpServerOptions::default(), SideEffectPolicy::strict());
        let mut transport = start_server(server);

        // `project.scan` is a read, which the strict policy allows.
        let allowed = call(
            &mut transport,
            1,
            METHOD_TOOLS_CALL,
            json!({"name": "project_scan", "arguments": {}}),
        )
        .await;
        assert_eq!(allowed["result"]["isError"], json!(false));

        // `file.write` asks under the strict policy and the server refuses.
        let refused = call(
            &mut transport,
            2,
            METHOD_TOOLS_CALL,
            json!({"name": "file_write", "arguments": {"path": "mcp-refused-test.txt", "content": "x"}}),
        )
        .await;
        assert_eq!(refused["result"]["isError"], json!(true));
        let message = refused["result"]["content"][0]["text"].as_str().unwrap();
        assert!(message.contains("refused"), "unexpected message: {message}");
        assert!(!PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("mcp-refused-test.txt")
            .exists());

        let unknown = call(
            &mut transport,
            3,
            METHOD_TOOLS_CALL,
            json!({"name": "nope", "arguments": {}}),
        )
        .await;
        assert_eq!(unknown["result"]["isError"], json!(true));
    }

    #[tokio::test]
    async fn approve_flag_lets_ask_decisions_through() {
        let options = McpServerOptions {
            auto_approve: true,
            ..McpServerOptions::default()
        };
        let server = server(options, SideEffectPolicy::strict());
        let mut transport = start_server(server);

        let written = call(
            &mut transport,
            1,
            METHOD_TOOLS_CALL,
            json!({"name": "file_write", "arguments": {"path": "mcp-approved-test.txt", "content": "hello"}}),
        )
        .await;

        assert_eq!(written["result"]["isError"], json!(false));
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("mcp-approved-test.txt");
        assert_eq!(
            std::fs::read_to_string(&path).expect("file written"),
            "hello"
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn approvers_are_non_interactive() {
        assert!(
            PolicyApprover { auto_approve: true }.approve(&axiom_engine::ApprovalRequest {
                skill_id: "file.write".to_string(),
                message: "write".to_string(),
                risk_level: "medium".to_string(),
            })
        );
        assert!(!PolicyApprover {
            auto_approve: false
        }
        .approve(&axiom_engine::ApprovalRequest {
            skill_id: "file.write".to_string(),
            message: "write".to_string(),
            risk_level: "medium".to_string(),
        }));

        let mut allow_all = AllowAllApprover;
        assert!(allow_all.approve(&axiom_engine::ApprovalRequest {
            skill_id: "x".to_string(),
            message: "x".to_string(),
            risk_level: "low".to_string(),
        }));
        let mut deny_all = DenyAllApprover;
        assert!(!deny_all.approve(&axiom_engine::ApprovalRequest {
            skill_id: "x".to_string(),
            message: "x".to_string(),
            risk_level: "low".to_string(),
        }));
    }

    #[tokio::test]
    async fn non_jsonrpc_frames_are_rejected() {
        let server = server(McpServerOptions::default(), SideEffectPolicy::strict());
        let mut transport = start_server(server);

        transport
            .send(&json!({"jsonrpc": "1.0", "id": 4, "method": "ping"}))
            .await
            .expect("write legacy frame");
        let response = transport
            .receive()
            .await
            .expect("read response")
            .expect("response frame");

        assert_eq!(response["error"]["code"], json!(-32600));
        assert_eq!(response["id"], json!(4));
    }

    #[test]
    fn advertised_tools_respect_allow_and_deny_lists() {
        let options = McpServerOptions {
            allow_tools: vec!["file.read".to_string(), "git.diff".to_string()],
            deny_tools: vec!["git.diff".to_string()],
            ..McpServerOptions::default()
        };
        let server = server(options, SideEffectPolicy::strict());

        let ids = server
            .tools()
            .iter()
            .map(|tool| tool.skill_id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["file.read"]);
    }

    /// Drives the real client against the real server over an in-process link,
    /// so the handshake, tool listing, and a policy-gated execution are all
    /// exercised together rather than in isolation.
    #[tokio::test]
    async fn client_drives_the_server_end_to_end() {
        use crate::client::McpClient;
        use std::time::Duration;

        let server = server(
            McpServerOptions {
                read_only: true,
                ..McpServerOptions::default()
            },
            SideEffectPolicy::strict(),
        );
        assert!(server
            .tools()
            .iter()
            .any(|tool| tool.skill_id == "file.read"));

        let (client_side, mut server_side) = channel_transport_pair();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("server runtime");
            runtime.block_on(async move {
                let mut server = server;
                let _ = server.serve(&mut server_side).await;
            });
        });

        let mut client =
            McpClient::new("roundtrip", Box::new(client_side), Duration::from_secs(10));
        client.initialize().await.expect("handshake");
        assert_eq!(
            client.protocol_version(),
            crate::protocol::MCP_PROTOCOL_VERSION
        );

        let tools = client.list_tools().await.expect("list tools");
        assert!(tools.iter().any(|tool| tool.name == "file_read"));

        // A read-only tool under the strict policy: reads are allowed without
        // an approval, so the non-interactive approver is never consulted.
        let read = client
            .call_tool("file_read", Some(json!({"path": "src/lib.rs"})))
            .await
            .expect("call file_read");
        assert_eq!(read.is_error, Some(false));
        assert!(read.text().contains("Model Context Protocol"));
    }
}
