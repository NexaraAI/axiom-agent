use std::collections::BTreeMap;

use super::schema::*;
use super::{AgentWorkMode, CURRENT_CONFIG_VERSION};

pub(super) fn default_plan_approval() -> bool {
    true
}

pub(super) fn default_learn_skills() -> bool {
    true
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
                work_mode: AgentWorkMode::Build,
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
                plan_approval: default_plan_approval(),
                learn_skills: default_learn_skills(),
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
                variant: default_variant(),
                variant_models: default_variant_models(),
                thinking: None,
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
            gateway: GatewayConfig::default(),
            mcp: McpConfig::default(),
        }
    }
}

pub(super) fn default_registry_url() -> String {
    "https://raw.githubusercontent.com/NexaraAI/axiom-skills/main/registry.json".to_string()
}

pub(super) fn default_agent_loop_enabled() -> bool {
    true
}

pub(super) fn default_ui_color() -> bool {
    true
}

pub(super) fn default_ui_theme() -> String {
    "axiom".to_string()
}

pub(super) fn default_policy_mode() -> String {
    "velocity".to_string()
}

pub(super) fn default_policy_filesystem_read() -> String {
    "allow".to_string()
}

pub(super) fn default_policy_ask() -> String {
    "ask".to_string()
}

pub(super) fn default_network_https_only() -> bool {
    true
}

pub(super) fn default_agent_max_iterations() -> u32 {
    12
}

pub(super) fn default_agent_max_tool_iterations() -> u32 {
    20
}

pub(super) fn default_agent_max_tokens() -> u32 {
    200_000
}

pub(super) fn default_agent_max_cost_usd() -> f64 {
    1.0
}

pub(super) fn default_agent_max_wall_seconds() -> u64 {
    1800
}

pub(super) fn default_agent_max_consecutive_tool_errors() -> u32 {
    3
}

pub(super) fn default_registry_cache_ttl_hours() -> u64 {
    24
}

pub(super) fn default_fallback_to_bundled_registry() -> bool {
    true
}

pub(super) fn default_update_channel() -> String {
    "stable".to_string()
}

pub(super) fn default_update_policy() -> String {
    "notify".to_string()
}

pub(super) fn default_update_release_repo() -> String {
    "https://github.com/NexaraAI/axiom-agent".to_string()
}

pub(super) fn default_update_check_interval_hours() -> u64 {
    24
}

pub(super) fn default_update_backup_previous_binary() -> bool {
    true
}

pub(super) fn default_update_verify_checksums() -> bool {
    true
}

pub fn default_variant() -> String {
    "Default".to_string()
}

pub fn default_variant_models() -> BTreeMap<String, BTreeMap<String, String>> {
    BTreeMap::from([
        (
            "nvidia".to_string(),
            BTreeMap::from([
                (
                    "default".to_string(),
                    "nvidia/nemotron-3.5-lightning-30b-a3b".to_string(),
                ),
                ("low".to_string(), "meta/llama-3.1-8b-instruct".to_string()),
                (
                    "light".to_string(),
                    "meta/llama-3.1-8b-instruct".to_string(),
                ),
                (
                    "medium".to_string(),
                    "nvidia/nemotron-3.5-lightning-30b-a3b".to_string(),
                ),
                (
                    "high".to_string(),
                    "nvidia/nemotron-4-340b-instruct".to_string(),
                ),
                (
                    "xhigh".to_string(),
                    "nvidia/nemotron-4-340b-instruct".to_string(),
                ),
            ]),
        ),
        (
            "groq".to_string(),
            BTreeMap::from([
                ("default".to_string(), "llama-3.3-70b-versatile".to_string()),
                ("low".to_string(), "llama-3.1-8b-instant".to_string()),
                ("light".to_string(), "llama-3.1-8b-instant".to_string()),
                ("medium".to_string(), "llama-3.3-70b-versatile".to_string()),
                (
                    "high".to_string(),
                    "deepseek-r1-distill-llama-70b".to_string(),
                ),
                (
                    "xhigh".to_string(),
                    "deepseek-r1-distill-llama-70b".to_string(),
                ),
            ]),
        ),
        (
            "openrouter".to_string(),
            BTreeMap::from([
                (
                    "default".to_string(),
                    "anthropic/claude-3.7-sonnet".to_string(),
                ),
                (
                    "low".to_string(),
                    "meta-llama/llama-3.3-70b-instruct".to_string(),
                ),
                (
                    "light".to_string(),
                    "meta-llama/llama-3.3-70b-instruct".to_string(),
                ),
                (
                    "medium".to_string(),
                    "anthropic/claude-3.7-sonnet".to_string(),
                ),
                ("high".to_string(), "deepseek/deepseek-r1".to_string()),
                (
                    "xhigh".to_string(),
                    "anthropic/claude-3.7-sonnet:thinking".to_string(),
                ),
            ]),
        ),
        (
            "gemini".to_string(),
            BTreeMap::from([
                ("default".to_string(), "gemini-2.5-flash".to_string()),
                ("low".to_string(), "gemini-2.5-flash".to_string()),
                ("light".to_string(), "gemini-2.5-flash".to_string()),
                ("medium".to_string(), "gemini-2.5-flash".to_string()),
                ("high".to_string(), "gemini-2.5-pro".to_string()),
                ("xhigh".to_string(), "gemini-2.5-pro".to_string()),
            ]),
        ),
        (
            "github-models".to_string(),
            BTreeMap::from([
                ("default".to_string(), "openai/gpt-4.1".to_string()),
                ("low".to_string(), "meta/llama-3.3-70b-instruct".to_string()),
                (
                    "light".to_string(),
                    "meta/llama-3.3-70b-instruct".to_string(),
                ),
                ("medium".to_string(), "openai/gpt-4.1".to_string()),
                ("high".to_string(), "openai/o3-mini".to_string()),
                ("xhigh".to_string(), "openai/o1".to_string()),
            ]),
        ),
        (
            "github".to_string(),
            BTreeMap::from([
                ("default".to_string(), "openai/gpt-4.1".to_string()),
                ("low".to_string(), "meta/llama-3.3-70b-instruct".to_string()),
                (
                    "light".to_string(),
                    "meta/llama-3.3-70b-instruct".to_string(),
                ),
                ("medium".to_string(), "openai/gpt-4.1".to_string()),
                ("high".to_string(), "openai/o3-mini".to_string()),
                ("xhigh".to_string(), "openai/o1".to_string()),
            ]),
        ),
        (
            "lm-studio".to_string(),
            BTreeMap::from([
                ("default".to_string(), "default".to_string()),
                ("low".to_string(), "default".to_string()),
                ("light".to_string(), "default".to_string()),
                ("medium".to_string(), "default".to_string()),
                ("high".to_string(), "default".to_string()),
                ("xhigh".to_string(), "default".to_string()),
            ]),
        ),
        (
            "lmstudio".to_string(),
            BTreeMap::from([
                ("default".to_string(), "default".to_string()),
                ("low".to_string(), "default".to_string()),
                ("light".to_string(), "default".to_string()),
                ("medium".to_string(), "default".to_string()),
                ("high".to_string(), "default".to_string()),
                ("xhigh".to_string(), "default".to_string()),
            ]),
        ),
        (
            "openai".to_string(),
            BTreeMap::from([
                ("default".to_string(), "gpt-4o".to_string()),
                ("low".to_string(), "gpt-4o-mini".to_string()),
                ("light".to_string(), "gpt-4o-mini".to_string()),
                ("medium".to_string(), "gpt-4o".to_string()),
                ("high".to_string(), "o3-mini".to_string()),
                ("xhigh".to_string(), "o3-mini".to_string()),
            ]),
        ),
        (
            "anthropic".to_string(),
            BTreeMap::from([
                (
                    "default".to_string(),
                    "claude-3-7-sonnet-latest".to_string(),
                ),
                ("low".to_string(), "claude-3-5-haiku-latest".to_string()),
                ("light".to_string(), "claude-3-5-haiku-latest".to_string()),
                ("medium".to_string(), "claude-3-7-sonnet-latest".to_string()),
                ("high".to_string(), "claude-3-7-sonnet-latest".to_string()),
                ("xhigh".to_string(), "claude-3-7-sonnet-latest".to_string()),
            ]),
        ),
        (
            "cloudflare".to_string(),
            BTreeMap::from([
                ("default".to_string(), "openai/gpt-4o".to_string()),
                ("low".to_string(), "openai/gpt-4o-mini".to_string()),
                ("light".to_string(), "openai/gpt-4o-mini".to_string()),
                ("medium".to_string(), "openai/gpt-4o".to_string()),
                ("high".to_string(), "openai/o3-mini".to_string()),
                ("xhigh".to_string(), "openai/o3-mini".to_string()),
            ]),
        ),
        (
            "opencode".to_string(),
            BTreeMap::from([
                (
                    "default".to_string(),
                    "nemotron-3.5-lightning-free".to_string(),
                ),
                ("low".to_string(), "mimo-v2.5-free".to_string()),
                ("light".to_string(), "mimo-v2.5-free".to_string()),
                (
                    "medium".to_string(),
                    "nemotron-3.5-lightning-free".to_string(),
                ),
                ("high".to_string(), "nemotron-3-ultra-free".to_string()),
                ("xhigh".to_string(), "nemotron-3-ultra-free".to_string()),
            ]),
        ),
        (
            "zen".to_string(),
            BTreeMap::from([
                (
                    "default".to_string(),
                    "nemotron-3.5-lightning-free".to_string(),
                ),
                ("low".to_string(), "mimo-v2.5-free".to_string()),
                ("light".to_string(), "mimo-v2.5-free".to_string()),
                (
                    "medium".to_string(),
                    "nemotron-3.5-lightning-free".to_string(),
                ),
                ("high".to_string(), "nemotron-3-ultra-free".to_string()),
                ("xhigh".to_string(), "nemotron-3-ultra-free".to_string()),
            ]),
        ),
        (
            "gmicloud".to_string(),
            BTreeMap::from([
                (
                    "default".to_string(),
                    "deepseek-ai/DeepSeek-V4-Pro".to_string(),
                ),
                (
                    "low".to_string(),
                    "meta-llama/Llama-3.1-8B-Instruct".to_string(),
                ),
                (
                    "light".to_string(),
                    "meta-llama/Llama-3.1-8B-Instruct".to_string(),
                ),
                (
                    "medium".to_string(),
                    "meta-llama/Llama-3.3-70B-Instruct".to_string(),
                ),
                (
                    "high".to_string(),
                    "deepseek-ai/DeepSeek-V4-Pro".to_string(),
                ),
                (
                    "xhigh".to_string(),
                    "deepseek-ai/DeepSeek-V4-Pro".to_string(),
                ),
            ]),
        ),
        (
            "gmi".to_string(),
            BTreeMap::from([
                (
                    "default".to_string(),
                    "deepseek-ai/DeepSeek-V4-Pro".to_string(),
                ),
                (
                    "low".to_string(),
                    "meta-llama/Llama-3.1-8B-Instruct".to_string(),
                ),
                (
                    "light".to_string(),
                    "meta-llama/Llama-3.1-8B-Instruct".to_string(),
                ),
                (
                    "medium".to_string(),
                    "meta-llama/Llama-3.3-70B-Instruct".to_string(),
                ),
                (
                    "high".to_string(),
                    "deepseek-ai/DeepSeek-V4-Pro".to_string(),
                ),
                (
                    "xhigh".to_string(),
                    "deepseek-ai/DeepSeek-V4-Pro".to_string(),
                ),
            ]),
        ),
        (
            "ollama_cloud".to_string(),
            BTreeMap::from([
                ("default".to_string(), "llama3.3:70b".to_string()),
                ("low".to_string(), "qwen2.5-coder:32b".to_string()),
                ("light".to_string(), "qwen2.5-coder:32b".to_string()),
                ("medium".to_string(), "llama3.3:70b".to_string()),
                ("high".to_string(), "deepseek-r1:70b".to_string()),
                ("xhigh".to_string(), "deepseek-r1:70b".to_string()),
            ]),
        ),
        (
            "ollama-cloud".to_string(),
            BTreeMap::from([
                ("default".to_string(), "llama3.3:70b".to_string()),
                ("low".to_string(), "qwen2.5-coder:32b".to_string()),
                ("light".to_string(), "qwen2.5-coder:32b".to_string()),
                ("medium".to_string(), "llama3.3:70b".to_string()),
                ("high".to_string(), "deepseek-r1:70b".to_string()),
                ("xhigh".to_string(), "deepseek-r1:70b".to_string()),
            ]),
        ),
        (
            "ollama".to_string(),
            BTreeMap::from([
                ("default".to_string(), "llama3.2".to_string()),
                ("low".to_string(), "llama3.2:1b".to_string()),
                ("light".to_string(), "llama3.2:1b".to_string()),
                ("medium".to_string(), "llama3.2".to_string()),
                ("high".to_string(), "llama3.3:70b".to_string()),
                ("xhigh".to_string(), "llama3.3:70b".to_string()),
            ]),
        ),
        (
            "mock".to_string(),
            BTreeMap::from([
                ("default".to_string(), "mock-model".to_string()),
                ("low".to_string(), "mock-model".to_string()),
                ("light".to_string(), "mock-model".to_string()),
                ("medium".to_string(), "mock-model".to_string()),
                ("high".to_string(), "mock-model".to_string()),
                ("xhigh".to_string(), "mock-model".to_string()),
            ]),
        ),
    ])
}

pub(super) fn default_coder_auto_route_from_chat() -> bool {
    true
}

pub(super) fn default_coder_auto_route_mode() -> String {
    "off".to_string()
}

pub(super) fn default_coder_approval_mode() -> String {
    "safe".to_string()
}

pub(super) fn default_coder_workspace_only() -> bool {
    true
}

pub(super) fn default_coder_allow_shell() -> bool {
    true
}

pub(super) fn default_coder_max_file_read_bytes() -> u64 {
    2_000_000
}

pub(super) fn default_coder_max_correction_attempts() -> u32 {
    2
}

pub(super) fn default_coder_max_patch_files() -> usize {
    20
}

pub(super) fn default_coder_max_patch_bytes() -> u64 {
    1_000_000
}

pub(super) fn default_coder_scope_confirmation_files() -> usize {
    5
}

pub(super) fn default_coder_scope_confirmation_bytes() -> u64 {
    200_000
}

pub(super) fn default_proof_enabled() -> bool {
    true
}

pub(super) fn default_proof_default_format() -> String {
    "markdown".to_string()
}

pub(super) fn default_proof_trace_json() -> bool {
    true
}

pub(super) fn default_proof_redact_secrets() -> bool {
    true
}

pub(super) fn default_proof_auto_export_markdown() -> bool {
    true
}

pub(super) fn default_proof_max_capture_chars() -> usize {
    4_000
}

pub(super) fn default_proof_retention_days() -> u64 {
    30
}

pub(super) fn default_mcp_enabled() -> bool {
    true
}

pub(super) fn default_mcp_connect_timeout_secs() -> u64 {
    20
}

pub(super) fn default_mcp_request_timeout_secs() -> u64 {
    60
}

pub(super) fn default_mcp_max_response_bytes() -> usize {
    1_000_000
}

pub(super) fn default_mcp_server_enabled() -> bool {
    true
}

pub(super) fn default_mcp_tool_enabled() -> bool {
    true
}
