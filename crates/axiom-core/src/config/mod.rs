use std::{
    fmt, fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{atomic_write, AxiomError, Result};

mod defaults;
mod schema;
mod validate;

pub use defaults::default_variant;
pub use schema::*;
use validate::{
    backup_path_for_migration, ensure_non_negative_finite, expand_home,
    validate_gateway_token_env_name, validate_host_pattern, validate_mcp_name_segment,
    validate_mcp_side_effects,
};
pub use validate::{validate_mode, validate_permission, validate_variant};

pub const CURRENT_CONFIG_VERSION: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentWorkMode {
    Plan,
    #[default]
    Build,
}

impl AgentWorkMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Plan => "plan",
            Self::Build => "build",
        }
    }
}

impl fmt::Display for AgentWorkMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl AxiomConfig {
    pub fn default_config_dir() -> Result<PathBuf> {
        if let Ok(home) = std::env::var("AXIOM_HOME") {
            if !home.trim().is_empty() {
                return Ok(PathBuf::from(home));
            }
        }
        let base = dirs::config_dir().ok_or(AxiomError::MissingConfigDirectory)?;
        Ok(base.join("axiom-agent"))
    }

    pub fn default_config_path() -> Result<PathBuf> {
        Ok(Self::default_config_dir()?.join("config.toml"))
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        config.ensure_supported_version()?;
        config.ensure_valid()?;
        Ok(config)
    }

    pub fn load_or_create(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if path.exists() {
            Self::load_from_path(path)
        } else {
            let config = Self::default();
            config.save_to_path(path)?;
            Ok(config)
        }
    }

    pub fn save_to_path(&self, path: impl AsRef<Path>) -> Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        atomic_write(path, self.to_toml_string()?.as_bytes())?;
        Ok(())
    }

    pub fn to_toml_string(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    pub fn default_workspace_path(&self) -> PathBuf {
        expand_home(&self.agent.default_workspace)
    }

    pub fn requires_migration(&self) -> bool {
        self.config_version < CURRENT_CONFIG_VERSION
    }

    pub fn migrate_file(path: impl AsRef<Path>) -> Result<ConfigMigrationResult> {
        let path = path.as_ref();
        let mut config = Self::load_from_path(path)?;
        let from_version = config.config_version;
        if !config.requires_migration() {
            return Ok(ConfigMigrationResult {
                from_version,
                to_version: CURRENT_CONFIG_VERSION,
                migrated: false,
                backup_path: None,
            });
        }

        let backup_path = backup_path_for_migration(path, from_version);
        fs::copy(path, &backup_path)?;
        config.config_version = CURRENT_CONFIG_VERSION;
        config.save_to_path(path)?;

        Ok(ConfigMigrationResult {
            from_version,
            to_version: CURRENT_CONFIG_VERSION,
            migrated: true,
            backup_path: Some(backup_path),
        })
    }

    fn ensure_supported_version(&self) -> Result<()> {
        if self.config_version > CURRENT_CONFIG_VERSION {
            return Err(AxiomError::UnsupportedConfigVersion {
                found: self.config_version,
                supported: CURRENT_CONFIG_VERSION,
            });
        }
        Ok(())
    }

    fn ensure_valid(&self) -> Result<()> {
        ensure_non_negative_finite("agent.max_cost_usd", self.agent.max_cost_usd)?;
        if let Some(budget) = self.agent.session_budget_usd {
            ensure_non_negative_finite("agent.session_budget_usd", budget)?;
        }
        if let Some(budget) = self.agent.monthly_budget_usd {
            ensure_non_negative_finite("agent.monthly_budget_usd", budget)?;
        }
        if let Some(rate) = self.agent.input_cost_per_million_tokens {
            ensure_non_negative_finite("agent.input_cost_per_million_tokens", rate)?;
        }
        if let Some(rate) = self.agent.output_cost_per_million_tokens {
            ensure_non_negative_finite("agent.output_cost_per_million_tokens", rate)?;
        }
        if self.agent.input_cost_per_million_tokens.is_some()
            != self.agent.output_cost_per_million_tokens.is_some()
        {
            return Err(AxiomError::InvalidConfig {
                field: "agent pricing",
                message: "input and output token rates must be configured together".to_string(),
            });
        }
        if self.coder.max_patch_files == 0 || self.coder.max_patch_bytes == 0 {
            return Err(AxiomError::InvalidConfig {
                field: "coder patch limits",
                message: "max_patch_files and max_patch_bytes must be greater than zero"
                    .to_string(),
            });
        }
        if self.coder.scope_confirmation_files > self.coder.max_patch_files
            || self.coder.scope_confirmation_bytes > self.coder.max_patch_bytes
        {
            return Err(AxiomError::InvalidConfig {
                field: "coder scope confirmation",
                message: "confirmation thresholds cannot exceed the hard patch limits".to_string(),
            });
        }
        if PermissionMode::parse(&self.policy.mode).is_none() {
            return Err(AxiomError::InvalidConfig {
                field: "policy.mode",
                message: "expected full_machine, velocity, or strict".to_string(),
            });
        }
        for (field, value) in [
            ("policy.filesystem_read", &self.policy.filesystem_read),
            ("policy.filesystem_write", &self.policy.filesystem_write),
            ("policy.network", &self.policy.network),
            ("policy.process", &self.policy.process),
            ("policy.git", &self.policy.git),
        ] {
            if !matches!(value.as_str(), "allow" | "ask" | "deny") {
                return Err(AxiomError::InvalidConfig {
                    field,
                    message: "expected allow, ask, or deny".to_string(),
                });
            }
        }
        if !matches!(
            self.ui.theme.as_str(),
            "axiom" | "blood_red" | "ash" | "high_contrast" | "none"
        ) {
            return Err(AxiomError::InvalidConfig {
                field: "ui.theme",
                message: "expected axiom, blood_red, ash, high_contrast, or none".to_string(),
            });
        }
        for (field, patterns) in [
            (
                "network.web_fetch_allowed_hosts",
                &self.network.web_fetch_allowed_hosts,
            ),
            (
                "network.web_fetch_denied_hosts",
                &self.network.web_fetch_denied_hosts,
            ),
        ] {
            for pattern in patterns {
                validate_host_pattern(field, pattern)?;
            }
        }
        for gateway_variable in [
            self.gateway.telegram_bot_token_env.as_deref(),
            self.gateway.discord_bot_token_env.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            validate_gateway_token_env_name(gateway_variable)?;
        }
        if self.mcp.connect_timeout_secs == 0
            || self.mcp.request_timeout_secs == 0
            || self.mcp.max_response_bytes == 0
        {
            return Err(AxiomError::InvalidConfig {
                field: "mcp limits",
                message: "connect_timeout_secs, request_timeout_secs, and max_response_bytes must be greater than zero"
                    .to_string(),
            });
        }
        let mut mcp_server_names = std::collections::BTreeSet::new();
        for server in &self.mcp.servers {
            validate_mcp_name_segment("mcp.servers.name", &server.name, 32)?;
            if !mcp_server_names.insert(server.name.as_str()) {
                return Err(AxiomError::InvalidConfig {
                    field: "mcp.servers.name",
                    message: format!("duplicate MCP server name `{}`", server.name),
                });
            }
            if server.command.trim().is_empty() {
                return Err(AxiomError::InvalidConfig {
                    field: "mcp.servers.command",
                    message: format!("MCP server `{}` has an empty command", server.name),
                });
            }
            if let Some(classes) = &server.side_effects {
                validate_mcp_side_effects("mcp.servers.side_effects", classes)?;
            }
            for variable in &server.env_from_secret {
                validate_gateway_token_env_name(variable)?;
            }
            let mut tool_names = std::collections::BTreeSet::new();
            for tool in &server.tools {
                if tool.name.trim().is_empty() || tool.name.contains(char::is_control) {
                    return Err(AxiomError::InvalidConfig {
                        field: "mcp.servers.tools.name",
                        message: "MCP tool names cannot be empty or contain control characters"
                            .to_string(),
                    });
                }
                if !tool_names.insert(tool.name.as_str()) {
                    return Err(AxiomError::InvalidConfig {
                        field: "mcp.servers.tools.name",
                        message: format!(
                            "duplicate MCP tool `{}` in server `{}`",
                            tool.name, server.name
                        ),
                    });
                }
                if let Some(classes) = &tool.side_effects {
                    validate_mcp_side_effects("mcp.servers.tools.side_effects", classes)?;
                }
            }
            for tool in server.allow_tools.iter().chain(server.deny_tools.iter()) {
                if tool.trim().is_empty() {
                    return Err(AxiomError::InvalidConfig {
                        field: "mcp.servers.allow_tools",
                        message: "tool names cannot be empty".to_string(),
                    });
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn config_round_trips_through_toml_file() {
        let dir = unique_temp_dir();
        let path = dir.join("config.toml");
        let mut config = AxiomConfig::default();
        config.agent.first_run_completed = true;
        config.llm.active_provider = Some("local".to_string());

        config.save_to_path(&path).expect("save config");
        let loaded = AxiomConfig::load_from_path(&path).expect("load config");

        assert_eq!(loaded, config);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn openai_compatible_auth_is_backward_compatible_and_optional() {
        let authenticated: ProviderConfig = toml::from_str(
            r#"
type = "openai_compatible"
base_url = "https://example.test/v1"
api_key_env = "EXAMPLE_API_KEY"
"#,
        )
        .expect("legacy authenticated provider config");
        let unauthenticated: ProviderConfig = toml::from_str(
            r#"
type = "openai_compatible"
base_url = "http://localhost:11434/v1"
"#,
        )
        .expect("unauthenticated local provider config");

        assert!(matches!(
            authenticated,
            ProviderConfig::OpenaiCompatible {
                api_key_env: Some(ref name),
                ..
            } if name == "EXAMPLE_API_KEY"
        ));
        assert!(matches!(
            unauthenticated,
            ProviderConfig::OpenaiCompatible {
                api_key_env: None,
                ..
            }
        ));
    }

    #[test]
    fn load_or_create_writes_default_config_when_missing() {
        let dir = unique_temp_dir();
        let path = dir.join("config.toml");

        let loaded = AxiomConfig::load_or_create(&path).expect("load or create");

        assert_eq!(loaded, AxiomConfig::default());
        assert!(path.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn new_default_config_enables_thirty_day_proof_retention() {
        assert_eq!(AxiomConfig::default().proof.retention_days, 30);
    }

    #[test]
    fn config_missing_new_coder_route_fields_uses_defaults() {
        let config: AxiomConfig = toml::from_str(
            r#"
[agent]
name = "Axiom Agent"
channel = "stable"
first_run_completed = true
default_workspace = "~/Axiom"
auto_update_policy = "notify"

[llm]
active_provider = "local"
active_model = "local-model"
stream = false

[skills]
auto_update_policy = "notify"
local_dir = "skills"

[coder]
approval_mode = "safe"
workspace_only = true
allow_shell = true
max_file_read_bytes = 2000000

[proof]
enabled = true
format = "json"
"#,
        )
        .expect("parse old config");

        assert!(config.coder.auto_route_from_chat);
        assert_eq!(config.coder.auto_route_mode, "off");
        assert_eq!(config.proof.default_format, "json");
        assert_eq!(config.coder.max_correction_attempts, 2);
        assert!(config.proof.trace_json);
        assert!(config.proof.auto_export_markdown);

        assert_eq!(config.proof.retention_days, 0);
        assert_eq!(config.update.channel, "stable");
        assert_eq!(config.update.policy, "notify");
        assert!(config.ui.color);
        assert!(config.agent.loop_enabled);
        assert_eq!(config.agent.max_iterations, 12);
        assert_eq!(config.agent.input_cost_per_million_tokens, None);
        assert_eq!(config.agent.output_cost_per_million_tokens, None);
        assert_eq!(config.agent.session_budget_usd, None);
        assert_eq!(config.agent.monthly_budget_usd, None);
        assert!(config.network.web_fetch_https_only);
        assert!(config.network.web_fetch_allowed_hosts.is_empty());
        assert!(!config.network.web_fetch_use_system_proxy);
        assert_eq!(config.config_version, 0);
    }

    #[test]
    fn config_llm_effort_and_tier_compatibility() {
        let legacy_config: LlmConfig = toml::from_str(
            r#"
active_provider = "nvidia"
active_model = "nvidia/nemotron-3.5-lightning-30b-a3b"
stream = true
tier = "high"
"#,
        )
        .expect("parse legacy config with tier");

        assert_eq!(legacy_config.active_effort(), "high");
        assert_eq!(legacy_config.active_variant(), "high");

        let modern_config: LlmConfig = toml::from_str(
            r#"
active_provider = "nvidia"
active_model = "nvidia/nemotron-3.5-lightning-30b-a3b"
stream = true
effort = "max"
"#,
        )
        .expect("parse modern config with effort");

        assert_eq!(modern_config.active_effort(), "max");
        assert_eq!(modern_config.active_variant(), "max");

        let variant_config: LlmConfig = toml::from_str(
            r#"
active_provider = "openai"
active_model = "gpt-4o"
stream = true
variant = "high"
"#,
        )
        .expect("parse variant config");

        assert_eq!(variant_config.active_variant(), "high");
        assert_eq!(
            variant_config.model_for_variant("openai", "high"),
            Some("o3-mini")
        );
    }

    #[test]
    fn migration_backs_up_a_legacy_config_and_updates_the_schema_version() {
        let dir = unique_temp_dir();
        let path = dir.join("config.toml");
        fs::create_dir_all(&dir).expect("create config directory");
        fs::write(
            &path,
            r#"
[agent]
name = "Axiom Agent"
channel = "stable"
first_run_completed = true
default_workspace = "~/Axiom"
auto_update_policy = "notify"

[llm]
active_provider = "mock"
active_model = "mock-model"
stream = false

[skills]
auto_update_policy = "notify"
local_dir = "skills"

[coder]
approval_mode = "safe"
workspace_only = true
allow_shell = true
max_file_read_bytes = 2000000

[proof]
enabled = true
format = "json"
"#,
        )
        .expect("write legacy config");

        let result = AxiomConfig::migrate_file(&path).expect("migrate config");
        let migrated = AxiomConfig::load_from_path(&path).expect("load migrated config");

        assert!(result.migrated);
        assert_eq!(result.from_version, 0);
        assert_eq!(migrated.config_version, CURRENT_CONFIG_VERSION);
        assert!(result.backup_path.as_ref().expect("backup path").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_a_config_from_a_newer_schema() {
        let dir = unique_temp_dir();
        let path = dir.join("config.toml");
        let config = AxiomConfig {
            config_version: CURRENT_CONFIG_VERSION + 1,
            ..AxiomConfig::default()
        };
        config.save_to_path(&path).expect("save future config");

        let error = AxiomConfig::load_from_path(&path).expect_err("future version should fail");

        assert!(matches!(error, AxiomError::UnsupportedConfigVersion { .. }));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_partial_or_negative_agent_pricing() {
        let dir = unique_temp_dir();
        let path = dir.join("config.toml");
        let mut config = AxiomConfig::default();
        config.agent.input_cost_per_million_tokens = Some(2.0);
        config.save_to_path(&path).expect("save partial pricing");

        let partial = AxiomConfig::load_from_path(&path).expect_err("partial pricing should fail");
        assert!(matches!(partial, AxiomError::InvalidConfig { .. }));

        config.agent.output_cost_per_million_tokens = Some(-1.0);
        config.save_to_path(&path).expect("save negative pricing");
        let negative = AxiomConfig::load_from_path(&path).expect_err("negative rate should fail");
        assert!(matches!(negative, AxiomError::InvalidConfig { .. }));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_negative_or_non_finite_persistent_cost_budgets() {
        let dir = unique_temp_dir();
        let path = dir.join("config.toml");
        let mut config = AxiomConfig::default();
        config.agent.session_budget_usd = Some(-0.01);
        config.save_to_path(&path).expect("save negative budget");

        let session =
            AxiomConfig::load_from_path(&path).expect_err("negative session budget should fail");
        assert!(matches!(session, AxiomError::InvalidConfig { .. }));

        config.agent.session_budget_usd = None;
        config.agent.monthly_budget_usd = Some(f64::INFINITY);
        config.save_to_path(&path).expect("save infinite budget");
        let monthly =
            AxiomConfig::load_from_path(&path).expect_err("infinite monthly budget should fail");
        assert!(matches!(monthly, AxiomError::InvalidConfig { .. }));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn validates_web_fetch_host_patterns() {
        let dir = unique_temp_dir();
        let path = dir.join("config.toml");
        let mut config = AxiomConfig::default();
        config.network.web_fetch_allowed_hosts = vec!["*.example.com".to_string()];
        config.network.web_fetch_denied_hosts = vec!["blocked.example.com".to_string()];
        config
            .save_to_path(&path)
            .expect("save valid network policy");
        AxiomConfig::load_from_path(&path).expect("valid host patterns");

        config.network.web_fetch_allowed_hosts = vec!["https://example.com".to_string()];
        config
            .save_to_path(&path)
            .expect("save invalid network policy");
        let error = AxiomConfig::load_from_path(&path).expect_err("URL is not a host pattern");
        assert!(matches!(error, AxiomError::InvalidConfig { .. }));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn axiom_home_overrides_default_config_dir() {
        let dir = unique_temp_dir();
        let _guard = EnvVarGuard::set("AXIOM_HOME", dir.as_os_str().to_os_string());

        let config_dir = AxiomConfig::default_config_dir().expect("config dir");
        let config_path = AxiomConfig::default_config_path().expect("config path");

        assert_eq!(config_dir, dir);
        assert!(config_path.ends_with("config.toml"));
    }

    #[test]
    fn permission_modes_configure_expected_presets() {
        let full = SideEffectPolicyConfig::for_mode(PermissionMode::FullMachine);
        assert_eq!(full.mode, "full_machine");
        assert_eq!(full.filesystem_read, "allow");
        assert_eq!(full.filesystem_write, "allow");
        assert_eq!(full.network, "allow");
        assert_eq!(full.process, "allow");
        assert_eq!(full.git, "allow");

        let velocity = SideEffectPolicyConfig::for_mode(PermissionMode::Velocity);
        assert_eq!(velocity.mode, "velocity");
        assert_eq!(velocity.filesystem_read, "allow");
        assert_eq!(velocity.filesystem_write, "allow");
        assert_eq!(velocity.network, "allow");
        assert_eq!(velocity.process, "allow");
        assert_eq!(velocity.git, "ask");

        let strict = SideEffectPolicyConfig::for_mode(PermissionMode::Strict);
        assert_eq!(strict.mode, "strict");
        assert_eq!(strict.filesystem_read, "allow");
        assert_eq!(strict.filesystem_write, "ask");
        assert_eq!(strict.network, "ask");
        assert_eq!(strict.process, "ask");
        assert_eq!(strict.git, "ask");
    }

    #[test]
    fn permission_mode_parsing_and_validation() {
        assert_eq!(
            PermissionMode::parse("full_machine"),
            Some(PermissionMode::FullMachine)
        );
        assert_eq!(
            PermissionMode::parse("unrestricted"),
            Some(PermissionMode::FullMachine)
        );
        assert_eq!(
            PermissionMode::parse("velocity"),
            Some(PermissionMode::Velocity)
        );
        assert_eq!(
            PermissionMode::parse("fast"),
            Some(PermissionMode::Velocity)
        );
        assert_eq!(
            PermissionMode::parse("strict"),
            Some(PermissionMode::Strict)
        );
        assert_eq!(PermissionMode::parse("safe"), Some(PermissionMode::Strict));
        assert_eq!(PermissionMode::parse("unknown_mode"), None);

        let mut config = AxiomConfig::default();
        assert_eq!(config.ui.theme, "axiom");
        assert!(config.ensure_valid().is_ok());
        assert_eq!(config.policy.permission_mode(), PermissionMode::Velocity);
        config.policy.mode = "invalid".to_string();
        assert!(config.ensure_valid().is_err());
    }

    #[test]
    fn thinking_mode_and_new_providers_configuration() {
        let mut config = AxiomConfig::default();
        assert_eq!(config.llm.thinking_display(), "auto");
        assert!(config.llm.is_thinking_enabled());

        config.llm.thinking = Some(true);
        assert_eq!(config.llm.thinking_display(), "on");
        assert!(config.llm.is_thinking_enabled());

        config.llm.thinking = Some(false);
        assert_eq!(config.llm.thinking_display(), "off");
        assert!(!config.llm.is_thinking_enabled());

        assert_eq!(
            config.llm.model_for_variant("opencode", "default"),
            Some("nemotron-3.5-lightning-free")
        );
        assert_eq!(
            config.llm.model_for_variant("zen", "medium"),
            Some("nemotron-3.5-lightning-free")
        );
        assert_eq!(
            config.llm.model_for_variant("gmicloud", "default"),
            Some("deepseek-ai/DeepSeek-V4-Pro")
        );
        assert_eq!(
            config.llm.model_for_variant("gmi", "medium"),
            Some("meta-llama/Llama-3.3-70B-Instruct")
        );
        assert_eq!(
            config.llm.model_for_variant("ollama_cloud", "default"),
            Some("llama3.3:70b")
        );
        assert_eq!(
            config.llm.model_for_variant("ollama_cloud", "low"),
            Some("qwen2.5-coder:32b")
        );
        assert_eq!(
            config.llm.model_for_variant("ollama_cloud", "high"),
            Some("deepseek-r1:70b")
        );
        assert_eq!(
            config.llm.model_for_variant("ollama_cloud", "xhigh"),
            Some("deepseek-r1:70b")
        );
        assert_eq!(
            config.llm.model_for_variant("openai", "xhigh"),
            Some("o3-mini")
        );
        assert_eq!(
            config.llm.model_for_variant("anthropic", "xhigh"),
            Some("claude-3-7-sonnet-latest")
        );
        assert_eq!(
            config.llm.model_for_variant("openrouter", "high"),
            Some("deepseek/deepseek-r1")
        );
        assert_eq!(
            config.llm.model_for_variant("openrouter", "xhigh"),
            Some("anthropic/claude-3.7-sonnet:thinking")
        );
        assert_eq!(
            config.llm.model_for_variant("gemini", "high"),
            Some("gemini-2.5-pro")
        );
        assert_eq!(
            config.llm.model_for_variant("github-models", "high"),
            Some("openai/o3-mini")
        );
        assert_eq!(
            config.llm.model_for_variant("github_models", "xhigh"),
            Some("openai/o1")
        );
        assert_eq!(
            config.llm.model_for_variant("groq", "high"),
            Some("deepseek-r1-distill-llama-70b")
        );
        assert_eq!(
            config.llm.model_for_variant("lm-studio", "default"),
            Some("default")
        );

        config.llm.variant = "xhigh".to_string();
        assert_eq!(config.llm.reasoning_effort_for_variant(), "high");
        assert_eq!(config.llm.thinking_budget_tokens_for_variant(), 8192);
    }

    #[test]
    fn strict_validation_helpers_accept_valid_inputs_and_reject_invalid() {
        assert_eq!(validate_variant("Default").unwrap(), "Default");
        assert_eq!(validate_variant("default").unwrap(), "Default");
        assert_eq!(validate_variant("low").unwrap(), "low");
        assert_eq!(validate_variant("medium").unwrap(), "medium");
        assert_eq!(validate_variant("high").unwrap(), "high");
        assert_eq!(validate_variant("xhigh").unwrap(), "xhigh");
        assert!(validate_variant("max").is_err());
        assert!(validate_variant("light").is_err());
        assert!(validate_variant("unknown").is_err());

        assert_eq!(validate_mode("plan").unwrap(), AgentWorkMode::Plan);
        assert_eq!(validate_mode("build").unwrap(), AgentWorkMode::Build);
        assert_eq!(validate_mode("PLAN").unwrap(), AgentWorkMode::Plan);
        assert_eq!(validate_mode("BUILD").unwrap(), AgentWorkMode::Build);
        assert!(validate_mode("run").is_err());
        assert!(validate_mode("exec").is_err());

        assert_eq!(
            validate_permission("velocity").unwrap(),
            PermissionMode::Velocity
        );
        assert_eq!(
            validate_permission("full_machine").unwrap(),
            PermissionMode::FullMachine
        );
        assert_eq!(
            validate_permission("strict").unwrap(),
            PermissionMode::Strict
        );
        assert!(validate_permission("fast").is_err());
        assert!(validate_permission("safe").is_err());
        assert!(validate_permission("unrestricted").is_err());
        assert!(validate_permission("other").is_err());
    }

    #[test]
    fn agent_work_mode_serialization_and_defaults() {
        assert_eq!(AgentWorkMode::default(), AgentWorkMode::Build);
        assert_eq!(AgentWorkMode::Plan.as_str(), "plan");
        assert_eq!(AgentWorkMode::Build.as_str(), "build");

        let default_config = AxiomConfig::default();
        assert_eq!(default_config.agent.work_mode, AgentWorkMode::Build);

        let serialized = serde_json::to_string(&AgentWorkMode::Plan).expect("serialize plan");
        assert_eq!(serialized, "\"plan\"");
        let deserialized: AgentWorkMode =
            serde_json::from_str("\"plan\"").expect("deserialize plan");
        assert_eq!(deserialized, AgentWorkMode::Plan);

        let serialized_build =
            serde_json::to_string(&AgentWorkMode::Build).expect("serialize build");
        assert_eq!(serialized_build, "\"build\"");
        let deserialized_build: AgentWorkMode =
            serde_json::from_str("\"build\"").expect("deserialize build");
        assert_eq!(deserialized_build, AgentWorkMode::Build);

        let provider_config = ProviderConfig::ollama_cloud(None);
        assert!(matches!(
            provider_config,
            ProviderConfig::OpenaiCompatible {
                ref base_url,
                api_key_env: Some(ref env),
                ..
            } if base_url == "https://api.ollama.com/v1" && env == "OLLAMA_API_KEY"
        ));
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("axiom-core-config-test-{nanos}"))
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: OsString) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            if let Some(previous) = self.previous.as_ref() {
                std::env::set_var(self.key, previous);
            } else {
                std::env::remove_var(self.key);
            }
        }
    }
    /// The provider/variant model table used to be written down twice: once
    /// as a `match` in `LlmConfig::default_variant_model` and once as the
    /// `variant_models` map. The map is consulted first, so the `match` was
    /// only reachable for providers absent from the map, and where both
    /// existed they had already drifted apart in 14 places (`nvidia`
    /// high/xhigh, `ollama` all variants, and the `ollama_cloud` low/medium
    /// arms, which were transposed).
    ///
    /// Pinning the values here means a future edit to the table has to
    /// update this test too, so an accidental swap is visible in review
    /// rather than silently changing which model a variant selects.
    #[test]
    fn variant_model_table_matches_the_pinned_expectations() {
        let map = crate::config::defaults::default_variant_models();
        let expected: &[(&str, &str, &str)] = &[
            ("nvidia", "default", "nvidia/nemotron-3.5-lightning-30b-a3b"),
            ("nvidia", "low", "meta/llama-3.1-8b-instruct"),
            ("nvidia", "high", "nvidia/nemotron-4-340b-instruct"),
            ("nvidia", "xhigh", "nvidia/nemotron-4-340b-instruct"),
            ("groq", "default", "llama-3.3-70b-versatile"),
            ("groq", "low", "llama-3.1-8b-instant"),
            ("groq", "high", "deepseek-r1-distill-llama-70b"),
            ("openrouter", "default", "anthropic/claude-3.7-sonnet"),
            ("openrouter", "high", "deepseek/deepseek-r1"),
            (
                "openrouter",
                "xhigh",
                "anthropic/claude-3.7-sonnet:thinking",
            ),
            ("gemini", "default", "gemini-2.5-flash"),
            ("gemini", "high", "gemini-2.5-pro"),
            ("github-models", "default", "openai/gpt-4.1"),
            ("github-models", "xhigh", "openai/o1"),
            ("openai", "default", "gpt-4o"),
            ("openai", "low", "gpt-4o-mini"),
            ("anthropic", "default", "claude-3-7-sonnet-latest"),
            ("anthropic", "low", "claude-3-5-haiku-latest"),
            ("gmi", "default", "deepseek-ai/DeepSeek-V4-Pro"),
            ("gmi", "medium", "meta-llama/Llama-3.3-70B-Instruct"),
            ("ollama", "default", "llama3.2"),
            ("ollama", "low", "llama3.2:1b"),
            ("ollama_cloud", "low", "qwen2.5-coder:32b"),
            ("ollama_cloud", "medium", "llama3.3:70b"),
            ("mock", "default", "mock-model"),
            ("cloudflare", "default", "openai/gpt-4o"),
            ("lm-studio", "default", "default"),
        ];
        for (provider, variant, model) in expected {
            let actual = map
                .get(*provider)
                .and_then(|variants| variants.get(*variant))
                .map(String::as_str)
                .unwrap_or_else(|| panic!("{provider}/{variant} missing from the table"));
            assert_eq!(actual, *model, "{provider}/{variant}");
        }
    }

    #[test]
    fn model_for_variant_resolves_provider_aliases_and_falls_through_to_none() {
        let llm = LlmConfig {
            active_provider: None,
            active_model: None,
            provider_models: std::collections::BTreeMap::new(),
            stream: true,
            variant: "Default".to_string(),
            variant_models: crate::config::defaults::default_variant_models(),
            thinking: None,
        };
        // Provider keys resolve regardless of case and separator style.
        assert_eq!(
            llm.model_for_variant("openrouter", "high"),
            Some("deepseek/deepseek-r1")
        );
        assert_eq!(
            llm.model_for_variant("OpenRouter", "HIGH"),
            Some("deepseek/deepseek-r1")
        );
        assert_eq!(
            llm.model_for_variant("github_models", "xhigh"),
            Some("openai/o1")
        );
        // An unknown provider or variant has no opinion, rather than a
        // value left over from a different table.
        assert_eq!(llm.model_for_variant("nope", "high"), None);
        assert_eq!(llm.model_for_variant("openrouter", "nope"), None);
    }
}
