use std::{collections::BTreeMap, path::PathBuf};

use serde::{Deserialize, Serialize};

use super::defaults::*;
use super::AgentWorkMode;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AxiomConfig {
    #[serde(default)]
    pub config_version: u32,
    pub agent: AgentConfig,
    pub llm: LlmConfig,
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderConfig>,
    pub skills: SkillsConfig,
    #[serde(default)]
    pub update: UpdateConfig,
    #[serde(default)]
    pub ui: UiConfig,
    #[serde(default)]
    pub policy: SideEffectPolicyConfig,
    #[serde(default)]
    pub network: NetworkConfig,
    pub coder: CoderConfig,
    pub proof: ProofConfig,
    #[serde(default)]
    pub gateway: GatewayConfig,
    #[serde(default)]
    pub mcp: McpConfig,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigMigrationResult {
    pub from_version: u32,
    pub to_version: u32,
    pub migrated: bool,
    pub backup_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentConfig {
    pub name: String,
    pub channel: String,
    pub first_run_completed: bool,
    pub default_workspace: String,
    pub auto_update_policy: String,
    #[serde(default)]
    pub work_mode: AgentWorkMode,
    #[serde(default = "default_agent_loop_enabled")]
    pub loop_enabled: bool,
    #[serde(default = "default_agent_max_iterations")]
    pub max_iterations: u32,
    #[serde(default = "default_agent_max_tool_iterations")]
    pub max_tool_iterations: u32,
    #[serde(default = "default_agent_max_tokens")]
    pub max_tokens: u32,
    #[serde(default = "default_agent_max_cost_usd")]
    pub max_cost_usd: f64,
    #[serde(default)]
    pub session_budget_usd: Option<f64>,
    #[serde(default)]
    pub monthly_budget_usd: Option<f64>,
    #[serde(default)]
    pub input_cost_per_million_tokens: Option<f64>,
    #[serde(default)]
    pub output_cost_per_million_tokens: Option<f64>,
    #[serde(default = "default_agent_max_wall_seconds")]
    pub max_wall_seconds: u64,
    #[serde(default = "default_agent_max_consecutive_tool_errors")]
    pub max_consecutive_tool_errors: u32,
    /// In Build mode, agree an implementation plan before anything writes to disk.
    #[serde(default = "default_plan_approval")]
    pub plan_approval: bool,
    /// Offer to keep a completed task's workflow as a reusable skill.
    #[serde(default = "default_learn_skills")]
    pub learn_skills: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmConfig {
    pub active_provider: Option<String>,
    pub active_model: Option<String>,
    #[serde(default)]
    pub provider_models: BTreeMap<String, String>,
    pub stream: bool,
    #[serde(default = "default_variant", alias = "tier", alias = "effort")]
    pub variant: String,
    #[serde(default = "default_variant_models", alias = "tier_models")]
    pub variant_models: BTreeMap<String, BTreeMap<String, String>>,
    #[serde(default)]
    pub thinking: Option<bool>,
}

impl LlmConfig {
    pub fn active_variant(&self) -> &str {
        if !self.variant.is_empty() {
            &self.variant
        } else {
            "Default"
        }
    }

    pub fn active_effort(&self) -> &str {
        self.active_variant()
    }

    pub fn thinking_display(&self) -> &'static str {
        match self.thinking {
            Some(true) => "on",
            Some(false) => "off",
            None => "auto",
        }
    }

    pub fn is_thinking_enabled(&self) -> bool {
        self.thinking.unwrap_or(true)
    }

    pub fn model_for_variant(&self, provider: &str, variant: &str) -> Option<&str> {
        let normalized = variant.to_ascii_lowercase();
        let prov_raw = provider.trim();
        let prov_lower = prov_raw.to_ascii_lowercase();
        let prov_hyphen = prov_lower.replace('_', "-");
        let prov_underscore = prov_lower.replace('-', "_");

        let configured = self
            .variant_models
            .get(prov_raw)
            .or_else(|| self.variant_models.get(&prov_lower))
            .or_else(|| self.variant_models.get(&prov_hyphen))
            .or_else(|| self.variant_models.get(&prov_underscore))
            .and_then(|variants| {
                variants
                    .get(variant)
                    .or_else(|| variants.get(&normalized))
                    .map(String::as_str)
            });

        if configured.is_some() {
            return configured;
        }

        Self::default_variant_model(&prov_lower, &normalized)
    }

    pub fn default_variant_model(provider: &str, variant: &str) -> Option<&'static str> {
        let p = provider.trim().to_ascii_lowercase();
        let v = variant.trim().to_ascii_lowercase();
        match (p.as_str(), v.as_str()) {
            ("opencode" | "zen" | "opencode-zen", "default" | "medium") => {
                Some("nemotron-3.5-lightning-free")
            }
            ("opencode" | "zen" | "opencode-zen", "low" | "light") => Some("mimo-v2.5-free"),
            ("opencode" | "zen" | "opencode-zen", "high" | "xhigh") => {
                Some("nemotron-3-ultra-free")
            }
            ("openrouter", "default" | "medium") => Some("anthropic/claude-3.7-sonnet"),
            ("openrouter", "low" | "light") => Some("meta-llama/llama-3.3-70b-instruct"),
            ("openrouter", "high") => Some("deepseek/deepseek-r1"),
            ("openrouter", "xhigh") => Some("anthropic/claude-3.7-sonnet:thinking"),
            ("gemini", "default" | "low" | "light" | "medium") => Some("gemini-2.5-flash"),
            ("gemini", "high" | "xhigh") => Some("gemini-2.5-pro"),
            ("github-models" | "github", "default" | "medium") => Some("openai/gpt-4.1"),
            ("github-models" | "github", "low" | "light") => Some("meta/llama-3.3-70b-instruct"),
            ("github-models" | "github", "high") => Some("openai/o3-mini"),
            ("github-models" | "github", "xhigh") => Some("openai/o1"),
            ("groq", "default" | "medium") => Some("llama-3.3-70b-versatile"),
            ("groq", "low" | "light") => Some("llama-3.1-8b-instant"),
            ("groq", "high" | "xhigh") => Some("deepseek-r1-distill-llama-70b"),
            ("nvidia" | "nvidia-nim", "default" | "medium") => {
                Some("nvidia/nemotron-3.5-lightning-30b-a3b")
            }
            ("nvidia" | "nvidia-nim", "low" | "light") => Some("meta/llama-3.1-8b-instruct"),
            ("nvidia" | "nvidia-nim", "high" | "xhigh") => Some("deepseek-ai/deepseek-r1"),
            ("openai", "default" | "medium") => Some("gpt-4o"),
            ("openai", "low" | "light") => Some("gpt-4o-mini"),
            ("openai", "high" | "xhigh") => Some("o3-mini"),
            ("anthropic", "default" | "medium" | "high" | "xhigh") => {
                Some("claude-3-7-sonnet-latest")
            }
            ("anthropic", "low" | "light") => Some("claude-3-5-haiku-latest"),
            ("gmicloud", "default" | "high" | "xhigh") => Some("deepseek-ai/DeepSeek-V4-Pro"),
            ("gmicloud", "low" | "light") => Some("meta-llama/Llama-3.1-8B-Instruct"),
            ("gmicloud", "medium") => Some("meta-llama/Llama-3.3-70B-Instruct"),
            ("ollama", _) => Some("llama3.3:70b"),
            ("ollama_cloud" | "ollama-cloud", "low" | "light") => Some("llama3.3:70b"),
            ("ollama_cloud" | "ollama-cloud", "medium" | "default") => Some("qwen2.5-coder:32b"),
            ("ollama_cloud" | "ollama-cloud", "high" | "xhigh") => Some("deepseek-r1:70b"),
            _ => None,
        }
    }

    pub fn model_for_tier(&self, provider: &str, tier: &str) -> Option<&str> {
        self.model_for_variant(provider, tier)
    }

    pub fn parse_variant(variant: &str) -> Option<&'static str> {
        match variant.trim().to_ascii_lowercase().as_str() {
            "default" => Some("Default"),
            "low" | "light" => Some("low"),
            "medium" => Some("medium"),
            "high" | "max" => Some("high"),
            "xhigh" | "x-high" | "extra-high" | "extra_high" => Some("xhigh"),
            _ => None,
        }
    }

    pub fn reasoning_effort_for_variant(&self) -> &'static str {
        let variant = self.active_variant();
        let normalized = variant.to_ascii_lowercase();
        match normalized.as_str() {
            "none" | "default" => "medium",
            "low" | "light" => "low",
            "medium" => "medium",
            "high" | "max" | "xhigh" => "high",
            _ => "medium",
        }
    }

    pub fn thinking_budget_tokens_for_variant(&self) -> u32 {
        let variant = self.active_variant();
        let normalized = variant.to_ascii_lowercase();
        if normalized == "xhigh" {
            8192
        } else {
            2048
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderConfig {
    Mock {},
    CloudflareAiGateway {
        account_id: String,
        gateway_id: String,
        api_token_env: String,
        base_url: String,
    },
    OpenaiCompatible {
        base_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key_env: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        models_url: Option<String>,
    },
}

impl ProviderConfig {
    pub fn ollama_cloud(api_key_env: Option<String>) -> Self {
        Self::OpenaiCompatible {
            base_url: "https://api.ollama.com/v1".to_string(),
            api_key_env: Some(api_key_env.unwrap_or_else(|| "OLLAMA_API_KEY".to_string())),
            models_url: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillsConfig {
    pub auto_update_policy: String,
    pub local_dir: String,
    #[serde(default = "default_registry_url")]
    pub registry_url: String,
    #[serde(default = "default_registry_cache_ttl_hours")]
    pub registry_cache_ttl_hours: u64,
    #[serde(default)]
    pub allow_untrusted_registries: bool,
    #[serde(default = "default_fallback_to_bundled_registry")]
    pub fallback_to_bundled_registry: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateConfig {
    #[serde(default = "default_update_channel")]
    pub channel: String,
    #[serde(default = "default_update_policy")]
    pub policy: String,
    #[serde(default = "default_update_release_repo")]
    pub release_repo: String,
    #[serde(default = "default_update_check_interval_hours")]
    pub check_interval_hours: u64,
    #[serde(default)]
    pub allow_prerelease: bool,
    #[serde(default = "default_update_backup_previous_binary")]
    pub backup_previous_binary: bool,
    #[serde(default = "default_update_verify_checksums")]
    pub verify_checksums: bool,
    #[serde(default)]
    pub last_checked_at: Option<String>,
    #[serde(default)]
    pub last_available_version: Option<String>,
    #[serde(default)]
    pub last_update_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_ui_color")]
    pub color: bool,
    #[serde(default = "default_ui_theme")]
    pub theme: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    FullMachine,
    Velocity,
    Strict,
}

impl PermissionMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "full_machine" | "full" | "unrestricted" | "all" => Some(Self::FullMachine),
            "velocity" | "fast" | "normal" | "balanced" => Some(Self::Velocity),
            "strict" | "safe" | "paranoid" | "lockdown" => Some(Self::Strict),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FullMachine => "full_machine",
            Self::Velocity => "velocity",
            Self::Strict => "strict",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::FullMachine => "unrestricted machine access with auto-approved operations",
            Self::Velocity => {
                "high-speed agentic execution with guardrails for destructive actions"
            }
            Self::Strict => "zero-trust isolation requiring approval for mutations",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideEffectPolicyConfig {
    #[serde(default = "default_policy_mode")]
    pub mode: String,
    #[serde(default = "default_policy_filesystem_read")]
    pub filesystem_read: String,
    #[serde(default = "default_policy_filesystem_read")]
    pub filesystem_write: String,
    #[serde(default = "default_policy_ask")]
    pub network: String,
    #[serde(default = "default_policy_ask")]
    pub process: String,
    #[serde(default = "default_policy_ask")]
    pub git: String,
}

impl SideEffectPolicyConfig {
    pub fn for_mode(mode: PermissionMode) -> Self {
        match mode {
            PermissionMode::FullMachine => Self {
                mode: "full_machine".to_string(),
                filesystem_read: "allow".to_string(),
                filesystem_write: "allow".to_string(),
                network: "allow".to_string(),
                process: "allow".to_string(),
                git: "allow".to_string(),
            },
            PermissionMode::Velocity => Self {
                mode: "velocity".to_string(),
                filesystem_read: "allow".to_string(),
                filesystem_write: "allow".to_string(),
                network: "allow".to_string(),
                process: "allow".to_string(),
                git: "ask".to_string(),
            },
            PermissionMode::Strict => Self {
                mode: "strict".to_string(),
                filesystem_read: "allow".to_string(),
                filesystem_write: "ask".to_string(),
                network: "ask".to_string(),
                process: "ask".to_string(),
                git: "ask".to_string(),
            },
        }
    }

    pub fn permission_mode(&self) -> PermissionMode {
        PermissionMode::parse(&self.mode).unwrap_or(PermissionMode::Velocity)
    }

    pub fn apply_mode(&mut self, mode: PermissionMode) {
        let preset = Self::for_mode(mode);
        self.mode = preset.mode;
        self.filesystem_read = preset.filesystem_read;
        self.filesystem_write = preset.filesystem_write;
        self.network = preset.network;
        self.process = preset.process;
        self.git = preset.git;
    }

    pub fn ensure_mode_consistency(&mut self) {
        if let Some(mode) = PermissionMode::parse(&self.mode) {
            match mode {
                PermissionMode::FullMachine => {
                    self.filesystem_read = "allow".to_string();
                    self.filesystem_write = "allow".to_string();
                    self.network = "allow".to_string();
                    self.process = "allow".to_string();
                    self.git = "allow".to_string();
                }
                PermissionMode::Velocity => {
                    if self.filesystem_read == "ask" {
                        self.filesystem_read = "allow".to_string();
                    }
                    if self.filesystem_write == "ask" {
                        self.filesystem_write = "allow".to_string();
                    }
                    if self.network == "ask" {
                        self.network = "allow".to_string();
                    }
                    if self.process == "ask" {
                        self.process = "allow".to_string();
                    }
                }
                PermissionMode::Strict => {}
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkConfig {
    #[serde(default = "default_network_https_only")]
    pub web_fetch_https_only: bool,
    #[serde(default)]
    pub web_fetch_allowed_hosts: Vec<String>,
    #[serde(default)]
    pub web_fetch_denied_hosts: Vec<String>,
    #[serde(default)]
    pub web_fetch_use_system_proxy: bool,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            web_fetch_https_only: default_network_https_only(),
            web_fetch_allowed_hosts: Vec::new(),
            web_fetch_denied_hosts: Vec::new(),
            web_fetch_use_system_proxy: false,
        }
    }
}

impl Default for SideEffectPolicyConfig {
    fn default() -> Self {
        Self::for_mode(PermissionMode::Velocity)
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            color: default_ui_color(),
            theme: default_ui_theme(),
        }
    }
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            channel: default_update_channel(),
            policy: default_update_policy(),
            release_repo: default_update_release_repo(),
            check_interval_hours: default_update_check_interval_hours(),
            allow_prerelease: false,
            backup_previous_binary: default_update_backup_previous_binary(),
            verify_checksums: default_update_verify_checksums(),
            last_checked_at: None,
            last_available_version: None,
            last_update_error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoderConfig {
    #[serde(default = "default_coder_auto_route_from_chat")]
    pub auto_route_from_chat: bool,
    #[serde(default = "default_coder_auto_route_mode")]
    pub auto_route_mode: String,
    #[serde(default = "default_coder_approval_mode")]
    pub approval_mode: String,
    #[serde(default = "default_coder_workspace_only")]
    pub workspace_only: bool,
    #[serde(default = "default_coder_allow_shell")]
    pub allow_shell: bool,
    #[serde(default = "default_coder_max_file_read_bytes")]
    pub max_file_read_bytes: u64,
    #[serde(default = "default_coder_max_correction_attempts")]
    pub max_correction_attempts: u32,
    #[serde(default = "default_coder_max_patch_files")]
    pub max_patch_files: usize,
    #[serde(default = "default_coder_max_patch_bytes")]
    pub max_patch_bytes: u64,
    #[serde(default = "default_coder_scope_confirmation_files")]
    pub scope_confirmation_files: usize,
    #[serde(default = "default_coder_scope_confirmation_bytes")]
    pub scope_confirmation_bytes: u64,
}

/// Optional messaging-gateway settings (Telegram / Discord bot tokens).
/// Tokens are collected during onboarding and resolved like provider keys
/// (env var, OS keychain, then the private local fallback file). Telegram's
/// runners are live (`axiom gateway run --telegram` / `--discord`). Tokens
/// are resolved like provider keys and are always redacted and scrubbed from
/// child processes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub telegram_bot_token_env: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub telegram_allowed_chat_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub discord_bot_token_env: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub discord_allowed_guild_ids: Vec<String>,
}

/// Model Context Protocol (MCP) integration settings.
///
/// Axiom speaks MCP in both directions: it can act as an MCP *client* (each
/// configured server is wrapped as ordinary, permission-gated Axiom tools) and
/// as an MCP *server* through `axiom mcp serve`. Server definitions are inert
/// until `enabled` is true, so declaring a server never grants access on its
/// own; every call still flows through the side-effect policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(default = "default_mcp_enabled")]
    pub enabled: bool,
    #[serde(default = "default_mcp_connect_timeout_secs")]
    pub connect_timeout_secs: u64,
    #[serde(default = "default_mcp_request_timeout_secs")]
    pub request_timeout_secs: u64,
    #[serde(default = "default_mcp_max_response_bytes")]
    pub max_response_bytes: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub servers: Vec<McpServerConfig>,
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            enabled: default_mcp_enabled(),
            connect_timeout_secs: default_mcp_connect_timeout_secs(),
            request_timeout_secs: default_mcp_request_timeout_secs(),
            max_response_bytes: default_mcp_max_response_bytes(),
            servers: Vec::new(),
        }
    }
}

impl McpConfig {
    /// Servers that should be connected for the current process.
    pub fn enabled_servers(&self) -> impl Iterator<Item = &McpServerConfig> {
        let enabled = self.enabled;
        self.servers
            .iter()
            .filter(move |server| enabled && server.enabled)
    }

    pub fn server(&self, name: &str) -> Option<&McpServerConfig> {
        self.servers.iter().find(|server| server.name == name)
    }
}

/// A single external MCP server, launched over stdio when Axiom connects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Short identifier used in tool names (`mcp.<name>.<tool>`).
    pub name: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Literal environment variables passed to the server process. Values here
    /// are stored in plaintext; prefer `env_from_secret` for credentials.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub env: BTreeMap<String, String>,
    /// Environment variable names resolved from Axiom's credential store (or
    /// the process environment) and forwarded to the server process.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub env_from_secret: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default = "default_mcp_server_enabled")]
    pub enabled: bool,
    /// Automatically approve `ask` policy decisions for this server's tools.
    /// Deny decisions are never overridden.
    #[serde(default)]
    pub auto_approve: bool,
    /// Side-effect classes applied to every tool of this server, overriding the
    /// classes derived from MCP annotations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_effects: Option<Vec<String>>,
    /// Tools exposed to the model. Empty means "all tools the server lists".
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allow_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<McpToolConfig>,
}

impl McpServerConfig {
    pub fn is_exposed(&self, tool_name: &str) -> bool {
        if self.deny_tools.iter().any(|denied| denied == tool_name) {
            return false;
        }
        if self
            .tools
            .iter()
            .any(|tool| tool.name == tool_name && !tool.enabled)
        {
            return false;
        }
        self.allow_tools.is_empty() || self.allow_tools.iter().any(|allowed| allowed == tool_name)
    }

    pub fn tool_config(&self, tool_name: &str) -> Option<&McpToolConfig> {
        self.tools.iter().find(|tool| tool.name == tool_name)
    }
}

/// Per-tool overrides for an MCP server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpToolConfig {
    pub name: String,
    #[serde(default = "default_mcp_tool_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub auto_approve: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side_effects: Option<Vec<String>>,
}

/// Side-effect class names accepted in MCP configuration.
pub const MCP_SIDE_EFFECT_NAMES: &[&str] = &[
    "filesystem_read",
    "filesystem_write",
    "network",
    "process",
    "git",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofConfig {
    #[serde(default = "default_proof_enabled")]
    pub enabled: bool,
    #[serde(default = "default_proof_default_format", alias = "format")]
    pub default_format: String,
    #[serde(default = "default_proof_trace_json")]
    pub trace_json: bool,
    #[serde(default = "default_proof_redact_secrets")]
    pub redact_secrets: bool,
    #[serde(default = "default_proof_auto_export_markdown")]
    pub auto_export_markdown: bool,
    #[serde(default = "default_proof_max_capture_chars")]
    pub max_capture_chars: usize,

    #[serde(default)]
    pub retention_days: u64,
}
