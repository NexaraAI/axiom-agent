use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    time::Duration,
};

use async_trait::async_trait;
use axiom_core::{McpConfig, McpServerConfig};
use axiom_engine::{
    authorize_side_effect, validate_schema_value, ExternalToolDefinition, ExternalToolSource,
    Permission, PolicyAction, SideEffectAuditSink, SideEffectClass, SideEffectPolicy,
    SideEffectRequest, SkillApproval, SkillExecutionContext, SkillExecutionError,
    SkillExecutionResult, ToolRequest,
};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::{
    client::McpClient,
    error::{McpError, Result},
    gate::{classes_for_annotations, parse_side_effect_classes, permissions_for_classes},
    protocol::{CallToolResult, ToolAnnotations, ToolDefinition},
    transport::StdioOptions,
};

const DEFAULT_OUTPUT_SCHEMA: &str = "object";
const MAX_TOOL_ID_SEGMENT: usize = 48;

/// Arguments commonly carrying the object a remote tool will touch, used for
/// audit records.
const TARGET_ARGUMENTS: &[&str] = &[
    "path",
    "url",
    "uri",
    "repo",
    "repository",
    "query",
    "command",
];

/// A remote MCP tool, resolved into Axiom terms.
#[derive(Debug, Clone, PartialEq)]
pub struct McpToolDefinition {
    /// Axiom skill id, e.g. `mcp.github.search_issues`.
    pub id: String,
    pub server: String,
    /// The tool name the server itself expects in `tools/call`.
    pub remote_name: String,
    pub description: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub side_effects: Vec<SideEffectClass>,
    pub permissions: Vec<Permission>,
    pub auto_approve: bool,
    pub annotations: Option<ToolAnnotations>,
}

/// Connected MCP servers, exposed to Axiom as permission-gated tools.
pub struct McpToolSource {
    tools: Vec<McpToolDefinition>,
    clients: BTreeMap<String, Mutex<McpClient>>,
    warnings: Vec<String>,
    instructions: BTreeMap<String, String>,
    server_labels: BTreeMap<String, String>,
}

impl McpToolSource {
    /// Connects every enabled server, degrading gracefully: a server that fails
    /// to start or hand-shake is recorded as a warning and skipped so the rest
    /// of Axiom keeps working.
    pub async fn connect(
        config: &McpConfig,
        resolved_env: &BTreeMap<String, BTreeMap<String, String>>,
    ) -> Self {
        let mut source = Self::empty();
        for server in config.enabled_servers().cloned().collect::<Vec<_>>() {
            match Self::connect_server(&server, config, resolved_env).await {
                Ok((client, tools, warnings)) => {
                    source.attach(&server, client, tools, warnings);
                }
                Err(error) => source.warnings.push(format!(
                    "mcp server `{}` is unavailable: {error}",
                    server.name
                )),
            }
        }
        source
    }

    /// Connects exactly one configured server, ignoring its `enabled` flag so
    /// an operator can test a server before turning it on. Failures are
    /// reported rather than swallowed.
    pub async fn connect_named(
        config: &McpConfig,
        resolved_env: &BTreeMap<String, BTreeMap<String, String>>,
        server_name: &str,
    ) -> Result<Self> {
        let server = config
            .server(server_name)
            .ok_or_else(|| McpError::UnknownServer {
                server: server_name.to_string(),
            })?
            .clone();
        let (client, tools, warnings) = Self::connect_server(&server, config, resolved_env).await?;
        let mut source = Self::empty();
        source.attach(&server, client, tools, warnings);
        Ok(source)
    }

    fn empty() -> Self {
        Self {
            tools: Vec::new(),
            clients: BTreeMap::new(),
            warnings: Vec::new(),
            instructions: BTreeMap::new(),
            server_labels: BTreeMap::new(),
        }
    }

    async fn connect_server(
        server: &McpServerConfig,
        config: &McpConfig,
        resolved_env: &BTreeMap<String, BTreeMap<String, String>>,
    ) -> Result<(McpClient, Vec<McpToolDefinition>, Vec<String>)> {
        let request_timeout = Duration::from_secs(config.request_timeout_secs.max(1));
        let handshake_timeout = Duration::from_secs(config.connect_timeout_secs.max(1));

        let mut options = StdioOptions::new(server.name.clone(), server.command.clone());
        options.args = server.args.clone();
        options.env = server_env(server, resolved_env);
        options.cwd = server.cwd.as_ref().map(PathBuf::from);
        options.max_frame_bytes = config.max_response_bytes.max(1024);

        let mut client = McpClient::spawn_stdio(&options, request_timeout)?;
        let remote_tools = tokio::time::timeout(handshake_timeout, async {
            client.initialize().await?;
            client.list_tools().await
        })
        .await
        .map_err(|_| McpError::Timeout {
            server: server.name.clone(),
            method: "initialize".to_string(),
            seconds: config.connect_timeout_secs.max(1),
        })??;

        let mut warnings = Vec::new();
        let mut used_ids = BTreeSet::new();
        let mut tools = Vec::new();
        for tool in remote_tools {
            match tool_definition(server, tool, &mut used_ids) {
                Ok(Some(definition)) => tools.push(definition),
                Ok(None) => {}
                Err(message) => warnings.push(format!(
                    "mcp server `{}` published an unusable tool: {message}",
                    server.name
                )),
            }
        }

        Ok((client, tools, warnings))
    }

    fn attach(
        &mut self,
        server: &McpServerConfig,
        client: McpClient,
        tools: Vec<McpToolDefinition>,
        warnings: Vec<String>,
    ) {
        self.warnings.extend(warnings);
        if let Some(instructions) = client.instructions() {
            self.instructions
                .insert(server.name.clone(), instructions.to_string());
        }
        let label = match client.server_info() {
            Some(info) => format!(
                "{} {} ({})",
                info.name,
                info.version,
                client.protocol_version()
            ),
            None => server.name.clone(),
        };
        self.server_labels.insert(server.name.clone(), label);
        self.clients.insert(server.name.clone(), Mutex::new(client));
        self.tools.extend(tools);
    }

    pub fn tools(&self) -> &[McpToolDefinition] {
        &self.tools
    }

    pub fn tool(&self, skill_id: &str) -> Option<&McpToolDefinition> {
        self.tools.iter().find(|tool| tool.id == skill_id)
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn connected_servers(&self) -> Vec<&str> {
        self.clients.keys().map(String::as_str).collect()
    }

    pub fn server_label(&self, server: &str) -> Option<&str> {
        self.server_labels.get(server).map(String::as_str)
    }

    pub fn instructions(&self, server: &str) -> Option<&str> {
        self.instructions.get(server).map(String::as_str)
    }

    /// True when at least one remote tool is usable.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Invokes a remote tool, applying the same side-effect gate as built-ins.
    pub async fn invoke(
        &self,
        skill_id: &str,
        arguments: Value,
        approval: &mut dyn SkillApproval,
        policy: &SideEffectPolicy,
        audit: &mut dyn SideEffectAuditSink,
    ) -> std::result::Result<SkillExecutionResult, SkillExecutionError> {
        let tool = self
            .tool(skill_id)
            .ok_or_else(|| SkillExecutionError::UnsupportedSkill(skill_id.to_string()))?;

        let arguments = if arguments.is_null() {
            json!({})
        } else {
            arguments
        };
        validate_schema_value(&arguments, &tool.input_schema).map_err(|message| {
            SkillExecutionError::SchemaValidation {
                skill_id: skill_id.to_string(),
                direction: "input",
                message,
            }
        })?;

        let relaxed_policy;
        let policy = if tool.auto_approve {
            relaxed_policy = relax_ask_actions(policy, &tool.side_effects);
            &relaxed_policy
        } else {
            policy
        };
        let operation = format!("mcp.{}.{}", tool.server, tool.remote_name);
        authorize_side_effect(
            policy,
            audit,
            approval,
            SideEffectRequest::new(
                &tool.id,
                operation,
                tool.side_effects.clone(),
                audit_target(&arguments),
            ),
        )?;

        let result = self.call_remote(tool, arguments).await.map_err(|error| {
            SkillExecutionError::ExecutionFailed {
                skill_id: skill_id.to_string(),
                message: error.to_string(),
            }
        })?;
        let output = tool_output(tool, &result);
        validate_schema_value(&output, &tool.output_schema).map_err(|message| {
            SkillExecutionError::SchemaValidation {
                skill_id: skill_id.to_string(),
                direction: "output",
                message,
            }
        })?;

        Ok(SkillExecutionResult {
            skill_id: skill_id.to_string(),
            output,
        })
    }

    /// Calls one remote tool without consulting the policy. Prefer [`Self::invoke`].
    pub async fn call_remote(
        &self,
        tool: &McpToolDefinition,
        arguments: Value,
    ) -> Result<CallToolResult> {
        let client = self
            .clients
            .get(&tool.server)
            .ok_or_else(|| McpError::UnknownTool {
                tool: tool.id.clone(),
            })?;
        let mut client = client.lock().await;
        client.call_tool(&tool.remote_name, Some(arguments)).await
    }

    /// Terminates every spawned server process.
    pub async fn shutdown(&mut self) {
        for client in self.clients.values_mut() {
            let _ = client.get_mut().close().await;
        }
        self.clients.clear();
    }
}

#[async_trait(?Send)]
impl ExternalToolSource for McpToolSource {
    fn definitions(&self) -> Vec<ExternalToolDefinition> {
        self.tools
            .iter()
            .map(|tool| {
                ExternalToolDefinition::new(
                    tool.id.clone(),
                    tool.server.clone(),
                    tool.description.clone(),
                    tool.input_schema.clone(),
                    tool.permissions.clone(),
                    tool.side_effects.clone(),
                )
            })
            .collect()
    }

    async fn call(
        &self,
        request: &ToolRequest,
        _context: &SkillExecutionContext,
        approval: &mut dyn SkillApproval,
        policy: &SideEffectPolicy,
        audit: &mut dyn SideEffectAuditSink,
    ) -> std::result::Result<SkillExecutionResult, SkillExecutionError> {
        self.invoke(
            &request.skill_id,
            request.arguments.clone(),
            approval,
            policy,
            audit,
        )
        .await
    }
}

/// Merges literal config variables with the secrets the host resolved.
fn server_env(
    server: &McpServerConfig,
    resolved_env: &BTreeMap<String, BTreeMap<String, String>>,
) -> BTreeMap<String, String> {
    let mut env = server.env.clone();
    if let Some(extra) = resolved_env.get(&server.name) {
        for (key, value) in extra {
            env.insert(key.clone(), value.clone());
        }
    }
    for name in &server.env_from_secret {
        if !env.contains_key(name) {
            if let Ok(value) = std::env::var(name) {
                env.insert(name.clone(), value);
            }
        }
    }
    env
}

/// Downgrades `ask` decisions to `allow` for servers the operator marked as
/// auto-approved. `deny` is never overridden.
fn relax_ask_actions(policy: &SideEffectPolicy, classes: &[SideEffectClass]) -> SideEffectPolicy {
    let mut relaxed = policy.clone();
    for class in classes {
        let action = match class {
            SideEffectClass::FilesystemRead => &mut relaxed.filesystem_read,
            SideEffectClass::FilesystemWrite => &mut relaxed.filesystem_write,
            SideEffectClass::Network => &mut relaxed.network,
            SideEffectClass::Process => &mut relaxed.process,
            SideEffectClass::Git => &mut relaxed.git,
        };
        if *action == PolicyAction::Ask {
            *action = PolicyAction::Allow;
        }
    }
    relaxed
}

fn audit_target(arguments: &Value) -> Option<String> {
    let object = arguments.as_object()?;
    for name in TARGET_ARGUMENTS {
        if let Some(value) = object.get(*name).and_then(Value::as_str) {
            if !value.trim().is_empty() {
                return Some(value.to_string());
            }
        }
    }
    None
}

fn tool_output(tool: &McpToolDefinition, result: &CallToolResult) -> Value {
    json!({
        "server": tool.server,
        "tool": tool.remote_name,
        "is_error": result.is_error(),
        "text": result.text(),
        "content": result.content,
        "structured_content": result.structured_content,
    })
}

fn tool_definition(
    server: &McpServerConfig,
    tool: ToolDefinition,
    used_ids: &mut BTreeSet<String>,
) -> std::result::Result<Option<McpToolDefinition>, String> {
    if tool.name.trim().is_empty() {
        return Err("tool name is empty".to_string());
    }
    if !server.is_exposed(&tool.name) {
        return Ok(None);
    }

    let tool_config = server.tool_config(&tool.name);
    let configured_classes = tool_config
        .and_then(|tool| tool.side_effects.as_ref())
        .or(server.side_effects.as_ref());
    let side_effects = match configured_classes {
        Some(classes) => parse_side_effect_classes(classes)?,
        None => classes_for_annotations(tool.annotations.as_ref()),
    };
    if side_effects.is_empty() {
        return Err("tool has no side-effect classification".to_string());
    }

    let input_schema = match tool.input_schema {
        Value::Null => json!({ "type": "object" }),
        schema => schema,
    };
    if input_schema.get("type").and_then(Value::as_str) != Some("object") {
        return Err("input schema must describe an object".to_string());
    }

    let description = tool
        .description
        .clone()
        .or_else(|| tool.title.clone())
        .filter(|description| !description.trim().is_empty())
        .unwrap_or_else(|| {
            format!(
                "MCP tool `{}` provided by server `{}`",
                tool.name, server.name
            )
        });

    Ok(Some(McpToolDefinition {
        id: unique_tool_id(&server.name, &tool.name, used_ids),
        server: server.name.clone(),
        remote_name: tool.name,
        description,
        input_schema,
        output_schema: tool
            .output_schema
            .unwrap_or_else(|| json!({ "type": DEFAULT_OUTPUT_SCHEMA })),
        permissions: permissions_for_classes(&side_effects),
        side_effects,
        auto_approve: server.auto_approve || tool_config.is_some_and(|tool| tool.auto_approve),
        annotations: tool.annotations,
    }))
}

fn unique_tool_id(server: &str, remote_name: &str, used_ids: &mut BTreeSet<String>) -> String {
    let base = format!("mcp.{}.{}", server, normalize_segment(remote_name));
    let mut candidate = base.clone();
    let mut suffix = 2;
    while !used_ids.insert(candidate.clone()) {
        candidate = format!("{base}_{suffix}");
        suffix += 1;
    }
    candidate
}

/// Folds an arbitrary remote tool name into an Axiom skill-id segment.
fn normalize_segment(name: &str) -> String {
    let mut normalized = String::new();
    for character in name.chars() {
        let lowered = character.to_ascii_lowercase();
        let mapped = if lowered.is_ascii_lowercase()
            || lowered.is_ascii_digit()
            || matches!(lowered, '-' | '_')
        {
            lowered
        } else {
            '_'
        };
        // Collapse runs of separators so `Search Issues!` becomes one segment.
        if mapped == '_' && normalized.ends_with('_') {
            continue;
        }
        normalized.push(mapped);
    }
    let trimmed = normalized.trim_matches('_').to_string();
    let trimmed = if trimmed.is_empty() {
        "tool".to_string()
    } else {
        trimmed
    };
    trimmed.chars().take(MAX_TOOL_ID_SEGMENT).collect()
}

#[cfg(test)]
mod tests {
    use axiom_engine::DenyAllApprover;
    use tokio::sync::Mutex;

    use super::*;
    use crate::protocol::IncomingMessage;
    use crate::transport::{channel_transport_pair, ChannelTransport, FrameTransport};

    fn annotations(read_only: bool, open_world: bool) -> ToolAnnotations {
        ToolAnnotations {
            read_only_hint: Some(read_only),
            open_world_hint: Some(open_world),
            ..ToolAnnotations::default()
        }
    }

    fn server_config(name: &str) -> McpServerConfig {
        McpServerConfig {
            name: name.to_string(),
            command: "unused".to_string(),
            args: Vec::new(),
            env: BTreeMap::new(),
            env_from_secret: Vec::new(),
            cwd: None,
            enabled: true,
            auto_approve: false,
            side_effects: None,
            allow_tools: Vec::new(),
            deny_tools: Vec::new(),
            tools: Vec::new(),
        }
    }

    fn remote_tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            title: None,
            description: Some(format!("remote {name}")),
            input_schema: json!({"type": "object", "properties": {"q": {"type": "string"}}}),
            output_schema: None,
            annotations: Some(annotations(true, false)),
        }
    }

    #[test]
    fn names_are_normalized_into_skill_ids_and_deduplicated() {
        let server = server_config("github");
        let mut used = BTreeSet::new();

        let first = tool_definition(
            &server,
            ToolDefinition {
                name: "Search Issues!".to_string(),
                ..remote_tool("ignored")
            },
            &mut used,
        )
        .unwrap()
        .unwrap();
        assert_eq!(first.id, "mcp.github.search_issues");

        // A second tool that normalizes to the same segment gets a stable suffix
        // instead of silently shadowing the first.
        let second = tool_definition(&server, remote_tool("search  issues"), &mut used)
            .unwrap()
            .unwrap();
        assert_eq!(second.id, "mcp.github.search_issues_2");
        assert_eq!(second.remote_name, "search  issues");

        let third = tool_definition(&server, remote_tool("!@#$"), &mut used)
            .unwrap()
            .unwrap();
        assert_eq!(third.id, "mcp.github.tool");
    }

    #[test]
    fn server_and_tool_overrides_beat_annotations() {
        let mut server = server_config("demo");
        server.side_effects = Some(vec!["filesystem_read".to_string()]);
        server.deny_tools = vec!["blocked".to_string()];
        server.allow_tools = vec!["allowed".to_string(), "blocked".to_string()];
        server.tools = vec![axiom_core::McpToolConfig {
            name: "allowed".to_string(),
            enabled: true,
            auto_approve: true,
            side_effects: Some(vec!["network".to_string()]),
        }];

        let mut used = BTreeSet::new();
        let allowed = tool_definition(
            &server,
            ToolDefinition {
                name: "allowed".to_string(),
                ..remote_tool("allowed")
            },
            &mut used,
        )
        .unwrap()
        .unwrap();
        assert_eq!(allowed.side_effects, vec![SideEffectClass::Network]);
        assert!(allowed.auto_approve);
        assert_eq!(allowed.permissions, vec![Permission::Network]);

        let blocked = tool_definition(
            &server,
            ToolDefinition {
                name: "blocked".to_string(),
                ..remote_tool("blocked")
            },
            &mut used,
        )
        .unwrap();
        assert!(blocked.is_none(), "deny_tools wins over allow_tools");

        let skipped = tool_definition(
            &server,
            ToolDefinition {
                name: "other".to_string(),
                ..remote_tool("other")
            },
            &mut used,
        )
        .unwrap();
        assert!(skipped.is_none(), "allow_tools is an allowlist when set");
    }

    #[test]
    fn unusable_tool_definitions_are_rejected() {
        let server = server_config("demo");
        let mut used = BTreeSet::new();

        let no_object_schema = tool_definition(
            &server,
            ToolDefinition {
                name: "broken".to_string(),
                title: None,
                description: None,
                input_schema: json!({"type": "string"}),
                output_schema: None,
                annotations: None,
            },
            &mut used,
        );
        assert!(no_object_schema.unwrap_err().contains("object"));
    }

    #[tokio::test]
    async fn invoke_records_a_policy_decision_for_allowed_calls() {
        let source = source_with_fake_server();
        let policy = SideEffectPolicy::allow_all();
        let mut audit = axiom_engine::RecordingSideEffectAuditSink::default();
        let mut approval = DenyAllApprover;

        let result = source
            .invoke(
                "mcp.fake.read_thing",
                json!({}),
                &mut approval,
                &policy,
                &mut audit,
            )
            .await;

        assert!(result.is_ok(), "allow_all policy permits the call");
        assert_eq!(audit.decisions().len(), 1);
    }

    #[tokio::test]
    async fn invoke_surfaces_denials_and_unknown_tools() {
        let source = source_with_fake_server();
        let policy = SideEffectPolicy::strict();
        let mut audit = axiom_engine::RecordingSideEffectAuditSink::default();
        let mut approval = DenyAllApprover;

        let denied = source
            .invoke(
                "mcp.fake.read_thing",
                json!({}),
                &mut approval,
                &policy,
                &mut audit,
            )
            .await
            .expect_err("strict policy asks, and the approver denies");
        assert!(matches!(denied, SkillExecutionError::ApprovalDenied(_)));

        let unknown = source
            .invoke(
                "mcp.fake.missing",
                json!({}),
                &mut approval,
                &policy,
                &mut audit,
            )
            .await
            .expect_err("unknown tool");
        assert!(matches!(unknown, SkillExecutionError::UnsupportedSkill(_)));
    }

    #[tokio::test]
    async fn invoke_validates_arguments_and_returns_remote_output() {
        let source = source_with_fake_server();
        let policy = SideEffectPolicy::allow_all();
        let mut audit = axiom_engine::RecordingSideEffectAuditSink::default();
        let mut approval = DenyAllApprover;

        let invalid = source
            .invoke(
                "mcp.fake.read_thing",
                json!("not an object"),
                &mut approval,
                &policy,
                &mut audit,
            )
            .await
            .expect_err("schema violation");
        assert!(matches!(
            invalid,
            SkillExecutionError::SchemaValidation {
                direction: "input",
                ..
            }
        ));

        let result = source
            .invoke(
                "mcp.fake.read_thing",
                json!({"q": "hello"}),
                &mut approval,
                &policy,
                &mut audit,
            )
            .await
            .expect("call succeeds");
        assert_eq!(result.output["is_error"], json!(false));
        assert_eq!(result.output["text"], json!("remote says hello"));
    }

    #[test]
    fn auto_approved_servers_relax_only_ask_decisions() {
        let classes = vec![SideEffectClass::Network, SideEffectClass::FilesystemWrite];
        let relaxed = relax_ask_actions(&SideEffectPolicy::strict(), &classes);
        assert_eq!(relaxed.network, PolicyAction::Allow);
        assert_eq!(relaxed.filesystem_write, PolicyAction::Allow);
        assert_eq!(relaxed.git, PolicyAction::Ask);

        let mut denying = SideEffectPolicy::strict();
        denying.network = PolicyAction::Deny;
        let relaxed = relax_ask_actions(&denying, &classes);
        assert_eq!(relaxed.network, PolicyAction::Deny);
    }

    #[test]
    fn audit_targets_are_taken_from_common_argument_names() {
        assert_eq!(
            audit_target(&json!({"path": "src/main.rs", "q": "x"})),
            Some("src/main.rs".to_string())
        );
        assert_eq!(audit_target(&json!({"q": "x"})), None);
    }

    /// Builds a source backed by an in-process fake server with one read-only
    /// tool that echoes its `q` argument.
    fn source_with_fake_server() -> McpToolSource {
        let (client_side, server_side) = channel_transport_pair();
        spawn_echo_server(server_side);
        let client = McpClient::new("fake", Box::new(client_side), Duration::from_secs(5));

        McpToolSource {
            tools: vec![McpToolDefinition {
                id: "mcp.fake.read_thing".to_string(),
                server: "fake".to_string(),
                remote_name: "read_thing".to_string(),
                description: "reads a thing".to_string(),
                input_schema: json!({"type": "object"}),
                output_schema: json!({"type": "object"}),
                side_effects: vec![SideEffectClass::FilesystemRead, SideEffectClass::Process],
                permissions: vec![Permission::FileSystemRead, Permission::ShellRun],
                auto_approve: false,
                annotations: Some(annotations(true, false)),
            }],
            clients: BTreeMap::from([("fake".to_string(), Mutex::new(client))]),
            warnings: Vec::new(),
            instructions: BTreeMap::new(),
            server_labels: BTreeMap::new(),
        }
    }

    fn spawn_echo_server(mut transport: ChannelTransport) {
        tokio::spawn(async move {
            while let Ok(Some(frame)) = transport.receive().await {
                let incoming: IncomingMessage = match serde_json::from_value(frame) {
                    Ok(incoming) => incoming,
                    Err(_) => break,
                };
                let crate::protocol::Incoming::Request { id, method, params } = incoming.classify()
                else {
                    continue;
                };
                let response = match method.as_str() {
                    "initialize" => crate::protocol::JsonRpcResponse::success(
                        id,
                        json!({
                            "protocolVersion": "2025-06-18",
                            "capabilities": {},
                            "serverInfo": {"name": "fake", "version": "1.0.0"},
                        }),
                    ),
                    "tools/list" => {
                        crate::protocol::JsonRpcResponse::success(id, json!({"tools": []}))
                    }
                    "tools/call" => {
                        let q = params
                            .as_ref()
                            .and_then(|params| params.get("arguments"))
                            .and_then(|arguments| arguments.get("q"))
                            .and_then(Value::as_str)
                            .unwrap_or("nothing");
                        crate::protocol::JsonRpcResponse::success(
                            id,
                            json!({
                                "content": [{"type": "text", "text": format!("remote says {q}")}],
                                "isError": false,
                            }),
                        )
                    }
                    _ => crate::protocol::JsonRpcResponse::error(
                        Some(id),
                        crate::protocol::JSONRPC_METHOD_NOT_FOUND,
                        "unsupported",
                    ),
                };
                if transport
                    .send(&serde_json::to_value(response).expect("serialize response"))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        });
    }
}
