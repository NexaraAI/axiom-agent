use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{atomic_write, AxiomError, Result};

pub const CURRENT_CONFIG_VERSION: u32 = 3;

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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmConfig {
    pub active_provider: Option<String>,
    pub active_model: Option<String>,
    #[serde(default)]
    pub provider_models: BTreeMap<String, String>,
    pub stream: bool,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideEffectPolicyConfig {
    #[serde(default = "default_policy_filesystem_read")]
    pub filesystem_read: String,
    #[serde(default = "default_policy_ask")]
    pub filesystem_write: String,
    #[serde(default = "default_policy_ask")]
    pub network: String,
    #[serde(default = "default_policy_ask")]
    pub process: String,
    #[serde(default = "default_policy_ask")]
    pub git: String,
}

/// Network controls for the model-invoked `web.fetch` tool. Provider endpoints
/// are configured separately so local Ollama/LM Studio use remains possible.
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
        Self {
            filesystem_read: default_policy_filesystem_read(),
            filesystem_write: default_policy_ask(),
            network: default_policy_ask(),
            process: default_policy_ask(),
            git: default_policy_ask(),
        }
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
    /// Number of days of dated proof directories to retain automatically.
    /// Zero disables automatic pruning.
    #[serde(default)]
    pub retention_days: u64,
}

impl Default for AxiomConfig {
    fn default() -> Self {
        let mut providers = BTreeMap::new();
        providers.insert("mock".to_string(), ProviderConfig::Mock {});
        providers.insert(
            "cloudflare".to_string(),
            ProviderConfig::CloudflareAiGateway {
                account_id: "YOUR_ACCOUNT_ID".to_string(),
                gateway_id: "default".to_string(),
                api_token_env: "CLOUDFLARE_API_TOKEN".to_string(),
                base_url: "https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/v1"
                    .to_string(),
            },
        );
        providers.insert(
            "local".to_string(),
            ProviderConfig::OpenaiCompatible {
                base_url: "http://localhost:8000/v1".to_string(),
                api_key_env: None,
                models_url: None,
            },
        );

        Self {
            config_version: CURRENT_CONFIG_VERSION,
            agent: AgentConfig {
                name: "Axiom Agent".to_string(),
                channel: "stable".to_string(),
                first_run_completed: false,
                default_workspace: "~/Axiom".to_string(),
                auto_update_policy: "notify".to_string(),
                loop_enabled: default_agent_loop_enabled(),
                max_iterations: default_agent_max_iterations(),
                max_tool_iterations: default_agent_max_tool_iterations(),
                max_tokens: default_agent_max_tokens(),
                max_cost_usd: default_agent_max_cost_usd(),
                session_budget_usd: None,
                monthly_budget_usd: None,
                input_cost_per_million_tokens: None,
                output_cost_per_million_tokens: None,
                max_wall_seconds: default_agent_max_wall_seconds(),
                max_consecutive_tool_errors: default_agent_max_consecutive_tool_errors(),
            },
            llm: LlmConfig {
                active_provider: Some("cloudflare".to_string()),
                active_model: Some("openai/gpt-4.1-mini".to_string()),
                provider_models: BTreeMap::from([
                    ("cloudflare".to_string(), "openai/gpt-4.1-mini".to_string()),
                    ("local".to_string(), "local-model".to_string()),
                    ("mock".to_string(), "mock-model".to_string()),
                ]),
                stream: true,
            },
            providers,
            skills: SkillsConfig {
                auto_update_policy: "notify".to_string(),
                local_dir: "skills".to_string(),
                registry_url: default_registry_url(),
                registry_cache_ttl_hours: default_registry_cache_ttl_hours(),
                allow_untrusted_registries: false,
                fallback_to_bundled_registry: true,
            },
            update: UpdateConfig::default(),
            ui: UiConfig::default(),
            policy: SideEffectPolicyConfig::default(),
            network: NetworkConfig::default(),
            coder: CoderConfig {
                auto_route_from_chat: default_coder_auto_route_from_chat(),
                auto_route_mode: default_coder_auto_route_mode(),
                approval_mode: default_coder_approval_mode(),
                workspace_only: default_coder_workspace_only(),
                allow_shell: default_coder_allow_shell(),
                max_file_read_bytes: default_coder_max_file_read_bytes(),
                max_correction_attempts: default_coder_max_correction_attempts(),
                max_patch_files: default_coder_max_patch_files(),
                max_patch_bytes: default_coder_max_patch_bytes(),
                scope_confirmation_files: default_coder_scope_confirmation_files(),
                scope_confirmation_bytes: default_coder_scope_confirmation_bytes(),
            },
            proof: ProofConfig {
                enabled: default_proof_enabled(),
                default_format: default_proof_default_format(),
                trace_json: default_proof_trace_json(),
                redact_secrets: default_proof_redact_secrets(),
                auto_export_markdown: default_proof_auto_export_markdown(),
                max_capture_chars: default_proof_max_capture_chars(),
                retention_days: default_proof_retention_days(),
            },
        }
    }
}

fn default_registry_url() -> String {
    "https://raw.githubusercontent.com/NexaraAI/axiom-skills/main/registry.json".to_string()
}

fn default_agent_loop_enabled() -> bool {
    true
}

fn default_ui_color() -> bool {
    true
}

fn default_ui_theme() -> String {
    "blood_red".to_string()
}

fn default_policy_filesystem_read() -> String {
    "allow".to_string()
}

fn default_policy_ask() -> String {
    "ask".to_string()
}

fn default_network_https_only() -> bool {
    true
}

fn default_agent_max_iterations() -> u32 {
    12
}

fn default_agent_max_tool_iterations() -> u32 {
    20
}

fn default_agent_max_tokens() -> u32 {
    200_000
}

fn default_agent_max_cost_usd() -> f64 {
    1.0
}

fn default_agent_max_wall_seconds() -> u64 {
    300
}

fn default_agent_max_consecutive_tool_errors() -> u32 {
    3
}

fn default_registry_cache_ttl_hours() -> u64 {
    24
}

fn default_fallback_to_bundled_registry() -> bool {
    true
}

fn default_update_channel() -> String {
    "stable".to_string()
}

fn default_update_policy() -> String {
    "notify".to_string()
}

fn default_update_release_repo() -> String {
    "https://github.com/NexaraAI/axiom-agent".to_string()
}

fn default_update_check_interval_hours() -> u64 {
    24
}

fn default_update_backup_previous_binary() -> bool {
    true
}

fn default_update_verify_checksums() -> bool {
    true
}

fn default_coder_auto_route_from_chat() -> bool {
    true
}

fn default_coder_auto_route_mode() -> String {
    "ask".to_string()
}

fn default_coder_approval_mode() -> String {
    "safe".to_string()
}

fn default_coder_workspace_only() -> bool {
    true
}

fn default_coder_allow_shell() -> bool {
    true
}

fn default_coder_max_file_read_bytes() -> u64 {
    2_000_000
}

fn default_coder_max_correction_attempts() -> u32 {
    2
}

fn default_coder_max_patch_files() -> usize {
    20
}

fn default_coder_max_patch_bytes() -> u64 {
    1_000_000
}

fn default_coder_scope_confirmation_files() -> usize {
    5
}

fn default_coder_scope_confirmation_bytes() -> u64 {
    200_000
}

fn default_proof_enabled() -> bool {
    true
}

fn default_proof_default_format() -> String {
    "markdown".to_string()
}

fn default_proof_trace_json() -> bool {
    true
}

fn default_proof_redact_secrets() -> bool {
    true
}

fn default_proof_auto_export_markdown() -> bool {
    true
}

fn default_proof_max_capture_chars() -> usize {
    4_000
}

fn default_proof_retention_days() -> u64 {
    30
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
            "blood_red" | "ash" | "high_contrast" | "none"
        ) {
            return Err(AxiomError::InvalidConfig {
                field: "ui.theme",
                message: "expected blood_red, ash, high_contrast, or none".to_string(),
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
        Ok(())
    }
}

fn validate_host_pattern(field: &'static str, pattern: &str) -> Result<()> {
    let host = pattern.strip_prefix("*.").unwrap_or(pattern);
    let valid = !host.is_empty()
        && host == host.trim()
        && !host
            .chars()
            .any(|character| matches!(character, '/' | ':' | '@'))
        && !host.contains('*')
        && !host.chars().any(char::is_whitespace)
        && !host.starts_with('.')
        && !host.ends_with('.');
    if valid {
        Ok(())
    } else {
        Err(AxiomError::InvalidConfig {
            field,
            message: format!(
                "invalid host pattern `{pattern}`; use an exact hostname or `*.example.com`"
            ),
        })
    }
}

fn ensure_non_negative_finite(field: &'static str, value: f64) -> Result<()> {
    if value.is_finite() && value >= 0.0 {
        Ok(())
    } else {
        Err(AxiomError::InvalidConfig {
            field,
            message: "expected a finite number greater than or equal to zero".to_string(),
        })
    }
}

fn backup_path_for_migration(path: &Path, from_version: u32) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.toml");
    let backup_name = format!("{file_name}.v{from_version}.bak");
    path.with_file_name(backup_name)
}

fn expand_home(path: &str) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    }

    if let Some(rest) = path.strip_prefix("~/") {
        return dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(rest);
    }

    if let Some(rest) = path.strip_prefix("~\\") {
        return dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(rest);
    }

    PathBuf::from(path)
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
        assert_eq!(config.coder.auto_route_mode, "ask");
        assert_eq!(config.proof.default_format, "json");
        assert_eq!(config.coder.max_correction_attempts, 2);
        assert!(config.proof.trace_json);
        assert!(config.proof.auto_export_markdown);
        // A config created before this setting existed must not suddenly start
        // deleting historic proof exports after an upgrade.
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
}
