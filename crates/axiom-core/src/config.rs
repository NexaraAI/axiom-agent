use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{AxiomError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AxiomConfig {
    pub agent: AgentConfig,
    pub llm: LlmConfig,
    #[serde(default)]
    pub providers: BTreeMap<String, ProviderConfig>,
    pub skills: SkillsConfig,
    #[serde(default)]
    pub update: UpdateConfig,
    pub coder: CoderConfig,
    pub proof: ProofConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentConfig {
    pub name: String,
    pub channel: String,
    pub first_run_completed: bool,
    pub default_workspace: String,
    pub auto_update_policy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmConfig {
    pub active_provider: Option<String>,
    pub active_model: Option<String>,
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
        api_key_env: String,
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
                api_key_env: "LOCAL_LLM_API_KEY".to_string(),
            },
        );

        Self {
            agent: AgentConfig {
                name: "Axiom Agent".to_string(),
                channel: "stable".to_string(),
                first_run_completed: false,
                default_workspace: "~/Axiom".to_string(),
                auto_update_policy: "notify".to_string(),
            },
            llm: LlmConfig {
                active_provider: Some("cloudflare".to_string()),
                active_model: Some("openai/gpt-4.1-mini".to_string()),
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
            coder: CoderConfig {
                auto_route_from_chat: default_coder_auto_route_from_chat(),
                auto_route_mode: default_coder_auto_route_mode(),
                approval_mode: default_coder_approval_mode(),
                workspace_only: default_coder_workspace_only(),
                allow_shell: default_coder_allow_shell(),
                max_file_read_bytes: default_coder_max_file_read_bytes(),
            },
            proof: ProofConfig {
                enabled: default_proof_enabled(),
                default_format: default_proof_default_format(),
                trace_json: default_proof_trace_json(),
                redact_secrets: default_proof_redact_secrets(),
                auto_export_markdown: default_proof_auto_export_markdown(),
                max_capture_chars: default_proof_max_capture_chars(),
            },
        }
    }
}

fn default_registry_url() -> String {
    "https://raw.githubusercontent.com/NexaraAI/axiom-skills/main/registry.json".to_string()
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
        Ok(toml::from_str(&content)?)
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
        fs::write(path, self.to_toml_string()?)?;
        Ok(())
    }

    pub fn to_toml_string(&self) -> Result<String> {
        Ok(toml::to_string_pretty(self)?)
    }

    pub fn default_workspace_path(&self) -> PathBuf {
        expand_home(&self.agent.default_workspace)
    }
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
    fn load_or_create_writes_default_config_when_missing() {
        let dir = unique_temp_dir();
        let path = dir.join("config.toml");

        let loaded = AxiomConfig::load_or_create(&path).expect("load or create");

        assert_eq!(loaded, AxiomConfig::default());
        assert!(path.exists());
        let _ = fs::remove_dir_all(dir);
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
        assert!(config.proof.trace_json);
        assert!(config.proof.auto_export_markdown);
        assert_eq!(config.update.channel, "stable");
        assert_eq!(config.update.policy, "notify");
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
