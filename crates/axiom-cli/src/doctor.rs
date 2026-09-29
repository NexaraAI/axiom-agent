use std::process::Command;

use anyhow::Result;
use axiom_core::{AxiomConfig, ProviderConfig, Workspace};
use axiom_engine::{load_installed_skills, ExecutorRegistry};
use axiom_upd::{UpdateDirs, UpdateState};

use crate::credentials;

pub(crate) fn doctor(json_output: bool) -> Result<()> {
    let config_path = AxiomConfig::default_config_path()?;
    let config_exists = config_path.exists();
    let config = if config_exists {
        AxiomConfig::load_from_path(&config_path)?
    } else {
        AxiomConfig::default()
    };

    let workspace_root = config.default_workspace_path();

    let workspace_result = Workspace::check_existing(&workspace_root);
    let workspace_status = workspace_result
        .as_ref()
        .map(|workspace| format!("ok ({})", workspace.root().display()))
        .unwrap_or_else(|error| format!("error ({error})"));
    let provider = provider_diagnostic(&config);
    let config_dir = config_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let skills_dir = config_dir.join(&config.skills.local_dir);
    let installed_skills = load_installed_skills(&skills_dir).unwrap_or_default();
    let executable_skills = installed_skills
        .iter()
        .filter(|skill| skill.record.is_executable())
        .map(|skill| skill.manifest.id.clone())
        .collect::<Vec<_>>();
    let built_in_executors = ExecutorRegistry::with_builtin_executors().supported_skill_ids();
    let update_dirs = UpdateDirs::new(config_dir);
    let update_state = UpdateState::load(&update_dirs.state_path).unwrap_or_default();
    let mut failed_mandatory_checks = Vec::new();
    if !config_exists {
        failed_mandatory_checks.push("config_missing");
    }
    if config.requires_migration() {
        failed_mandatory_checks.push("config_migration_required");
    }
    if workspace_result.is_err() {
        failed_mandatory_checks.push("workspace_invalid");
    }
    if !provider.status.starts_with("ready") {
        failed_mandatory_checks.push("provider_not_ready");
    }
    if !config.update.verify_checksums {
        failed_mandatory_checks.push("update_checksum_verification_disabled");
    }
    if !config.network.web_fetch_https_only {
        failed_mandatory_checks.push("web_fetch_https_only_disabled");
    }
    let credential_backend = match std::env::consts::OS {
        "windows" => "windows_credential_manager",
        "macos" => "macos_keychain",
        _ => "secret_service",
    };

    if json_output {
        println!(
            "{}",
            serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "config_schema_version": config.config_version,
                "supported_config_schema_version": axiom_core::CURRENT_CONFIG_VERSION,
                "config_migration_required": config.requires_migration(),
                "session_schema_version": axiom_core::CURRENT_SESSION_VERSION,
                "identity_schema_version": axiom_core::CURRENT_IDENTITY_VERSION,
                "proof_schema_version": axiom_proof::CURRENT_TRACE_VERSION,
                "os": std::env::consts::OS,
                "arch": std::env::consts::ARCH,
                "shell": detect_shell(),
                "commands": {
                    "git": command_available("git", &config),
                    "node": command_available("node", &config),
                    "rust": command_available("rustc", &config),
                },
                "config": {
                    "path": config_path,
                    "exists": config_exists,
                },
                "workspace": workspace_status,
                "provider": {
                    "active": provider.active,
                    "model": provider.model,
                    "status": provider.status,
                },
                "credentials": {
                    "backend": credential_backend,
                    "environment_fallback": true,
                },
                "skills": {
                    "installed": installed_skills.len(),
                    "executable": executable_skills,
                    "built_in_executors": built_in_executors,
                    "external_execution": "mcp",
                    "mcp": {
                        "enabled": config.mcp.enabled,
                        "servers_configured": config.mcp.servers.len(),
                        "servers_enabled":
                            config.mcp.servers.iter().filter(|s| s.enabled).count(),
                    },
                },
                "sandbox": {
                    "workspace_path_containment": true,
                    "central_side_effect_policy": true,
                    "external_skill_sandbox_available": false,
                    "external_skills_fail_closed": true,
                    "mcp_tools_policy_gated": true,
                },
                "policy": {
                    "filesystem_read": config.policy.filesystem_read,
                    "filesystem_write": config.policy.filesystem_write,
                    "network": config.policy.network,
                    "process": config.policy.process,
                    "git": config.policy.git,
                },
                "web_fetch_network": {
                    "https_only": config.network.web_fetch_https_only,
                    "allowed_hosts": config.network.web_fetch_allowed_hosts,
                    "denied_hosts": config.network.web_fetch_denied_hosts,
                    "system_proxy": config.network.web_fetch_use_system_proxy,
                    "redirects": "disabled",
                    "private_addresses": "blocked",
                },
                "update_provenance": {
                    "channel": config.update.channel,
                    "verify_checksums": config.update.verify_checksums,
                    "backup_previous_binary": config.update.backup_previous_binary,
                    "state": update_state.status.to_string(),
                    "checksum": update_state.checksum,
                    "release_url": update_state.release_url,
                },
                "mandatory_checks": {
                    "passed": failed_mandatory_checks.is_empty(),
                    "failed": failed_mandatory_checks,
                },
            })
        );
        return Ok(());
    }

    println!("Axiom doctor");
    println!("version: {}", env!("CARGO_PKG_VERSION"));
    println!("os: {}", std::env::consts::OS);
    println!("arch: {}", std::env::consts::ARCH);
    println!("shell: {}", detect_shell());
    println!("git: {}", command_available("git", &config));
    println!("node: {}", command_available("node", &config));
    println!("rust: {}", command_available("rustc", &config));
    println!(
        "config: {} ({})",
        config_path.display(),
        if config_exists { "exists" } else { "missing" }
    );
    println!("workspace: {workspace_status}");
    println!("provider: {}", provider.active);
    println!("model: {}", provider.model);
    println!("provider status: {}", provider.status);
    println!(
        "credential backend: {credential_backend} (env, then OS keychain, then private local file)"
    );
    println!(
        "telegram gateway: {}",
        doctor_gateway_status("telegram", config.gateway.telegram_bot_token_env.as_deref())
    );
    println!(
        "discord gateway: {}",
        doctor_gateway_status("discord", config.gateway.discord_bot_token_env.as_deref())
    );
    println!("executable skills: {}", executable_skills.join(", "));
    if config.mcp.enabled {
        println!(
            "external execution: MCP ({} of {} configured server(s) enabled; policy-gated)",
            config.mcp.servers.iter().filter(|s| s.enabled).count(),
            config.mcp.servers.len()
        );
    } else {
        println!("external execution: MCP disabled in config (enable under [mcp])");
    }
    println!(
        "side-effect policy: read={} write={} network={} process={} git={} (also gates MCP tools)",
        config.policy.filesystem_read,
        config.policy.filesystem_write,
        config.policy.network,
        config.policy.process,
        config.policy.git
    );
    println!(
        "web.fetch network: https_only={} allow_hosts={} deny_hosts={} system_proxy={} redirects=disabled private_addresses=blocked",
        config.network.web_fetch_https_only,
        config.network.web_fetch_allowed_hosts.len(),
        config.network.web_fetch_denied_hosts.len(),
        config.network.web_fetch_use_system_proxy,
    );
    println!(
        "update provenance: channel={} checksums={} state={}",
        config.update.channel, config.update.verify_checksums, update_state.status
    );
    println!(
        "config schema: v{}{}",
        config.config_version,
        if config.requires_migration() {
            " (run `axiom config migrate`)"
        } else {
            ""
        }
    );
    if failed_mandatory_checks.is_empty() {
        println!("status: all set! Run `axiom` to start chatting, or `axiom code --help` for coding tasks.");
    } else {
        println!(
            "status: needs attention ({})",
            failed_mandatory_checks.join(", ")
        );
        println!(
            "Next: run `axiom onboarding` to fix setup, or `axiom doctor --json` for details."
        );
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderDiagnostic {
    pub(crate) active: String,
    pub(crate) model: String,
    pub(crate) status: String,
}

pub(crate) fn provider_diagnostic(config: &AxiomConfig) -> ProviderDiagnostic {
    let active = config
        .llm
        .active_provider
        .clone()
        .unwrap_or_else(|| "not configured".to_string());
    let model = config
        .llm
        .active_model
        .clone()
        .unwrap_or_else(|| "not configured".to_string());
    let status = match config.llm.active_provider.as_deref() {
        None => "not configured".to_string(),
        Some(provider_name) if config.llm.active_model.as_deref().is_none_or(str::is_empty) => {
            format!("model is not configured for {provider_name}")
        }
        Some(provider_name) => match config.providers.get(provider_name) {
            None => format!("active provider entry is missing: {provider_name}"),
            Some(ProviderConfig::Mock {}) => "ready (offline mock)".to_string(),
            Some(ProviderConfig::CloudflareAiGateway {
                account_id,
                gateway_id,
                api_token_env,
                base_url,
            }) => {
                if account_id.trim().is_empty() || account_id == "YOUR_ACCOUNT_ID" {
                    "Cloudflare account_id is not configured".to_string()
                } else if gateway_id.trim().is_empty() || base_url.trim().is_empty() {
                    "Cloudflare gateway endpoint is incomplete".to_string()
                } else if let Err(error) =
                    axiom_llm::validate_provider_endpoint("base_url", base_url, false)
                {
                    format!("provider endpoint is invalid: {error}")
                } else {
                    authentication_status(api_token_env)
                }
            }
            Some(ProviderConfig::OpenaiCompatible {
                base_url,
                api_key_env,
                models_url,
            }) => {
                if base_url.trim().is_empty() {
                    "provider base_url is empty".to_string()
                } else if let Err(error) =
                    axiom_llm::validate_provider_endpoint("base_url", base_url, true)
                {
                    format!("provider endpoint is invalid: {error}")
                } else if let Some(models_url) = models_url {
                    if let Err(error) =
                        axiom_llm::validate_provider_endpoint("models_url", models_url, true)
                    {
                        format!("provider model catalog is invalid: {error}")
                    } else if let Some(api_key_env) = api_key_env {
                        authentication_status(api_key_env)
                    } else {
                        "ready (authentication not required)".to_string()
                    }
                } else if let Some(api_key_env) = api_key_env {
                    authentication_status(api_key_env)
                } else {
                    "ready (authentication not required)".to_string()
                }
            }
        },
    };

    ProviderDiagnostic {
        active,
        model,
        status,
    }
}

fn authentication_status(environment_variable: &str) -> String {
    if let Err(error) = axiom_llm::validate_credential_env_name(environment_variable) {
        return format!("credential configuration is invalid: {error}");
    }
    if std::env::var(environment_variable).is_ok_and(|value| !value.trim().is_empty()) {
        return format!("ready ({environment_variable} is set)");
    }
    match credentials::resolve_credential(environment_variable) {
        Ok(Some(_)) => format!("ready ({environment_variable} is in the OS credential manager)"),
        Ok(None) => format!("missing credential: {environment_variable}"),
        Err(error) => format!("credential unavailable for {environment_variable}: {error}"),
    }
}

fn detect_shell() -> String {
    std::env::var("SHELL")
        .or_else(|_| std::env::var("COMSPEC"))
        .unwrap_or_else(|_| "unknown".to_string())
}

fn command_available(program: &str, config: &AxiomConfig) -> &'static str {
    let mut command = Command::new(program);
    command.arg("--version");
    if credentials::scrub_provider_credentials(&mut command, config).is_err() {
        return "not checked (invalid credential configuration)";
    }
    match command.output() {
        Ok(output) if output.status.success() => "available",
        Ok(_) => "found but returned an error",
        Err(_) => "not found",
    }
}

fn doctor_gateway_status(platform: &str, token_env: Option<&str>) -> String {
    match token_env {
        None => "not configured".to_string(),
        Some(var) => match credentials::resolve_credential(var) {
            Ok(Some(_)) if platform == "telegram" => {
                "token saved (run `axiom gateway run --telegram`)".to_string()
            }
            Ok(Some(_)) => "token saved".to_string(),
            Ok(None) => format!("token named but MISSING: {var}"),
            Err(error) => format!("token unreadable ({error})"),
        },
    }
}
