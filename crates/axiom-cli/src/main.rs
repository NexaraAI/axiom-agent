mod chat;
mod code_commands;
mod cost_commands;
mod credentials;
mod identity;
mod onboarding;
mod proof_commands;
mod side_effects;
mod skill_commands;
mod startup;
mod ui;
mod update_commands;

use std::{path::PathBuf, process::Command};

use anyhow::Result;
use axiom_core::{AxiomConfig, ProviderConfig, Workspace};
use axiom_engine::{load_installed_skills, ExecutorRegistry};
use axiom_upd::{UpdateDirs, UpdateState};
use clap::{Args, Parser, Subcommand};
use startup::StartupRoute;

#[derive(Debug, Parser)]
#[command(name = "axiom", version, about = "Axiom Agent terminal CLI")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Run local environment checks.
    Doctor(DoctorCommand),
    /// Read and write Axiom configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },
    /// Run or update terminal onboarding.
    Onboarding(OnboardingCommand),
    /// Open the terminal chat interface.
    Chat,
    /// Resume a saved chat session.
    Resume { session_id: String },
    /// List saved chat sessions.
    Sessions,
    /// Report recorded model costs and configured persistent budgets.
    Cost,
    /// List, inspect, and switch provider models.
    #[command(alias = "models")]
    Model {
        #[command(subcommand)]
        command: ModelCommands,
    },
    /// Inspect and switch configured providers.
    #[command(alias = "providers")]
    Provider {
        #[command(subcommand)]
        command: ProviderCommands,
    },
    /// Send one non-interactive chat message and exit.
    Run(RunCommand),
    /// Open Axiom Coder mode.
    Code(CodeCommand),
    /// Inspect Axiom proof traces and reports.
    Proof {
        #[command(subcommand)]
        command: ProofCommands,
    },
    /// Inspect and install Axiom skills.
    Skill {
        #[command(subcommand)]
        command: SkillCommands,
    },
    /// Check, install, and manage core Axiom binary updates.
    Update {
        #[command(subcommand)]
        command: UpdateCommands,
    },
}

#[derive(Debug, Subcommand)]
enum ProofCommands {
    /// List proof traces.
    List,
    /// Show the latest proof summary.
    Latest,
    /// Show a proof report.
    Show { proof_id: String },
    /// Export a proof in a chosen format.
    Export {
        proof_id: String,
        #[arg(long, default_value = "markdown")]
        format: String,
    },
    /// Print the proof report path.
    Open { proof_id: String },
    /// Delete proof files older than DAYS.
    Clean {
        #[arg(long = "older-than")]
        older_than: u64,
    },
}

#[derive(Debug, Args)]
struct CodeCommand {
    /// Create a plan and stop before proposing edits.
    #[arg(long)]
    plan_only: bool,
    /// Scan the workspace and print a project summary.
    #[arg(long)]
    scan: bool,
    /// Show current git diff if this workspace is a git repository.
    #[arg(long)]
    diff: bool,
    /// Propose and apply an approved patch for TASK.
    #[arg(long)]
    apply: bool,
    /// Detect and optionally run a safe test command.
    #[arg(long = "test")]
    test: bool,
    /// Explain the project structure.
    #[arg(long)]
    explain: bool,
    /// Coding task.
    #[arg(value_name = "TASK", trailing_var_arg = true)]
    task: Vec<String>,
}

#[derive(Debug, Default, Args)]
struct OnboardingCommand {
    /// Run setup without prompts.
    #[arg(long)]
    non_interactive: bool,
    /// Workspace path for non-interactive setup.
    #[arg(long)]
    workspace: Option<String>,
    /// Preset: mock, groq, openrouter, gemini, github-models, ollama, lm-studio, openai, or cloudflare.
    #[arg(long)]
    provider: Option<String>,
    /// Default model for non-interactive provider setup.
    #[arg(long)]
    model: Option<String>,
    /// Cloudflare account ID (required with --provider cloudflare).
    #[arg(long)]
    account_id: Option<String>,
    /// Registry URL or local registry path for starter skills.
    #[arg(long)]
    registry: Option<String>,
    /// Configure no active provider.
    #[arg(long)]
    skip_provider: bool,
    /// Confirm non-interactive setup.
    #[arg(long)]
    yes: bool,
}

#[derive(Debug, Args)]
struct RunCommand {
    /// User message.
    message: String,
    /// Disable tool execution for this run.
    #[arg(long = "no-tools")]
    no_tools: bool,
    /// Disable proof recording for this run.
    #[arg(long = "no-proof")]
    no_proof: bool,
    /// Override provider for this run without editing config.
    #[arg(long)]
    provider: Option<String>,
    /// Override model for this run without editing config.
    #[arg(long)]
    model: Option<String>,
}

#[derive(Debug, Default, Args)]
struct DoctorCommand {
    /// Emit a stable machine-readable report.
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Subcommand)]
enum ConfigCommands {
    /// Print the active TOML configuration.
    List,
    /// Back up and migrate a legacy config to the current schema.
    Migrate,
}

#[derive(Debug, Subcommand)]
enum ModelCommands {
    /// Show the active provider and model.
    Current,
    /// Fetch the provider's model catalog without making an inference request.
    List {
        /// Configured provider to query; defaults to the active provider.
        #[arg(long)]
        provider: Option<String>,
        /// Show only model IDs containing this text.
        #[arg(long)]
        filter: Option<String>,
    },
    /// Persist a model choice, optionally switching provider too.
    Use {
        model: String,
        #[arg(long)]
        provider: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum ProviderCommands {
    /// Show the active provider.
    Current,
    /// Show configured providers and their saved model choices.
    List,
    /// Switch to a configured provider and restore its saved model.
    Use {
        provider: String,
        /// Override the saved model while switching.
        #[arg(long)]
        model: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum SkillCommands {
    /// Manage the configured skills registry.
    Registry {
        #[command(subcommand)]
        command: SkillRegistryCommands,
    },
    /// List skills available in the configured registry.
    List,
    /// Search skills in the configured registry.
    Search { query: String },
    /// Show installed skills.
    Installed,
    /// Show available skill bundles.
    Bundles,
    /// Show information for a skill.
    Info { skill_id: String },
    /// Run an installed tool skill with JSON arguments.
    Run {
        skill_id: String,
        #[arg(long)]
        args: Option<String>,
    },
    /// Install a skill from the configured registry.
    Install {
        skill_id: String,
        #[arg(long)]
        registry: Option<String>,
        #[arg(long = "from-local-registry")]
        from_local_registry: Option<PathBuf>,
    },
    /// Install every skill in a registry bundle.
    InstallBundle {
        bundle_id: String,
        #[arg(long)]
        registry: Option<String>,
        #[arg(long = "from-local-registry")]
        from_local_registry: Option<PathBuf>,
    },
    /// Check or apply skill updates.
    Update {
        #[arg(long)]
        check: bool,
        #[arg(long)]
        all: bool,
        #[arg(long = "apply-patches")]
        apply_patches: bool,
        skill_id: Option<String>,
    },
    /// Show installed skill health and lifecycle status.
    Health,
    /// Enable an installed skill.
    Enable { skill_id: String },
    /// Disable an installed skill.
    Disable { skill_id: String },
    /// Reset runtime health stats for a skill.
    ResetStats { skill_id: String },
    /// Remove an installed skill.
    Remove { skill_id: String },
}

#[derive(Debug, Subcommand)]
enum SkillRegistryCommands {
    /// Print the configured registry URL.
    Current,
    /// Set the configured registry URL.
    Set { url: String },
    /// Load the configured registry and print a summary.
    Refresh,
}

#[derive(Debug, Subcommand)]
enum UpdateCommands {
    /// Show core updater status.
    Status,
    /// Check GitHub Releases or a dev manifest for updates.
    Check,
    /// Download, verify, stage, and install an available update.
    Install,
    /// Restore the previous binary backup.
    Rollback,
    /// Set release channel: stable, nightly, or dev.
    SetChannel { channel: String },
    /// Set update policy: manual, notify, or auto-patch.
    SetPolicy { policy: String },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .without_time()
        .init();

    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Doctor(command)) => doctor(command.json),
        Some(Commands::Config { command }) => config(command),
        Some(Commands::Onboarding(command)) => run_onboarding_then_doctor(command).await,
        Some(Commands::Chat) => chat().await,
        Some(Commands::Resume { session_id }) => chat::resume_terminal_chat(&session_id).await,
        Some(Commands::Sessions) => chat::list_sessions(),
        Some(Commands::Cost) => cost_commands::run(),
        Some(Commands::Model { command }) => model(command).await,
        Some(Commands::Provider { command }) => provider(command),
        Some(Commands::Run(command)) => chat::run_one_shot(command).await,
        Some(Commands::Code(command)) => code_commands::run(command).await,
        Some(Commands::Proof { command }) => proof_commands::run(command),
        Some(Commands::Skill { command }) => skill_commands::run(command).await,
        Some(Commands::Update { command }) => update_commands::run(command).await,
        None => startup().await,
    }
}

fn provider(command: ProviderCommands) -> Result<()> {
    let config_path = AxiomConfig::default_config_path()?;
    let mut session = chat::ChatSession::load(config_path)?;
    match command {
        ProviderCommands::Current => println!(
            "provider: {}",
            session.active_provider().unwrap_or("not configured")
        ),
        ProviderCommands::List => {
            let config = AxiomConfig::load_from_path(AxiomConfig::default_config_path()?)?;
            for provider in config.providers.keys() {
                let marker = if Some(provider.as_str()) == config.llm.active_provider.as_deref() {
                    "*"
                } else {
                    "-"
                };
                let model = config
                    .llm
                    .provider_models
                    .get(provider)
                    .map(String::as_str)
                    .unwrap_or("model not selected");
                println!("{marker} {provider} ({model})");
            }
        }
        ProviderCommands::Use { provider, model } => {
            let provider = session.set_provider(provider)?;
            if let Some(model) = model {
                session.set_model(model)?;
            }
            println!(
                "Provider switched to {provider} with model {}.",
                session.active_model().unwrap_or("not configured")
            );
        }
    }
    Ok(())
}

async fn model(command: ModelCommands) -> Result<()> {
    let config_path = AxiomConfig::default_config_path()?;
    let mut session = chat::ChatSession::load(config_path)?;
    match command {
        ModelCommands::Current => {
            println!(
                "provider: {}",
                session.active_provider().unwrap_or("not configured")
            );
            println!(
                "model: {}",
                session.active_model().unwrap_or("not configured")
            );
        }
        ModelCommands::List { provider, filter } => {
            let provider_name = provider
                .as_deref()
                .or_else(|| session.active_provider())
                .ok_or_else(|| anyhow::anyhow!("no active provider configured"))?;
            let models = session.available_models(provider_name).await?;
            let (visible, total) = chat::models_for_display(&models, filter.as_deref());
            for model in &visible {
                println!("{}", model.id);
            }
            println!(
                "models: {} shown of {total} matching (provider: {provider_name})",
                visible.len()
            );
            if total > visible.len() {
                println!(
                    "Catalog output is capped at {}; use `--filter <text>` to narrow it.",
                    chat::MAX_MODELS_DISPLAYED
                );
            }
        }
        ModelCommands::Use { model, provider } => {
            if let Some(provider) = provider {
                session.set_provider(provider)?;
            }
            let model = session.set_model(model)?;
            println!(
                "Model switched to {model} for {}.",
                session.active_provider().unwrap_or("active provider")
            );
        }
    }
    Ok(())
}

fn config(command: ConfigCommands) -> Result<()> {
    let path = AxiomConfig::default_config_path()?;
    match command {
        ConfigCommands::List => {
            let config = AxiomConfig::load_or_create(&path)?;
            println!("{}", config.to_toml_string()?);
            Ok(())
        }
        ConfigCommands::Migrate => {
            if !path.exists() {
                let config = AxiomConfig::default();
                config.save_to_path(&path)?;
                println!("Created current config schema at {}.", path.display());
                return Ok(());
            }
            let result = AxiomConfig::migrate_file(&path)?;
            if result.migrated {
                let backup_path = result.backup_path.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "config migration completed without reporting its required backup path"
                    )
                })?;
                println!(
                    "Migrated config schema v{} to v{}. Backup: {}",
                    result.from_version,
                    result.to_version,
                    backup_path.display()
                );
            } else {
                println!("Config is already at schema v{}.", result.to_version);
            }
            Ok(())
        }
    }
}

async fn startup() -> Result<()> {
    let config_path = AxiomConfig::default_config_path()?;
    match startup::route_for_config_path(&config_path)? {
        StartupRoute::Onboarding => {
            println!("Axiom Agent setup is not complete. Starting onboarding.");
            run_onboarding_then_doctor(OnboardingCommand::default()).await?;
            if startup::route_for_config_path(&config_path)? == StartupRoute::Chat {
                chat::run_terminal_chat().await
            } else {
                println!(
                    "Provider setup is still incomplete. Run `axiom onboarding` when you are ready."
                );
                Ok(())
            }
        }
        StartupRoute::Chat => chat().await,
    }
}

async fn run_onboarding_then_doctor(command: OnboardingCommand) -> Result<()> {
    onboarding::run_onboarding_command(command).await?;
    doctor(false)
}

async fn chat() -> Result<()> {
    let config_path = AxiomConfig::default_config_path()?;
    if startup::route_for_config_path(&config_path)? == StartupRoute::Onboarding {
        println!("Onboarding is required before chat can start.");
        if chat::confirm("Start onboarding now?", true)? {
            run_onboarding_then_doctor(OnboardingCommand::default()).await?;
            if startup::route_for_config_path(&config_path)? == StartupRoute::Onboarding {
                println!(
                    "Provider setup is still incomplete. Run `axiom onboarding` when you are ready."
                );
                return Ok(());
            }
        } else {
            println!("Run `axiom onboarding` when you are ready.");
            return Ok(());
        }
    }

    chat::run_terminal_chat().await
}

fn doctor(json_output: bool) -> Result<()> {
    let config_path = AxiomConfig::default_config_path()?;
    let config_exists = config_path.exists();
    let config = if config_exists {
        AxiomConfig::load_from_path(&config_path)?
    } else {
        AxiomConfig::default()
    };

    let workspace_root = config.default_workspace_path();
    let workspace_result = Workspace::new(&workspace_root);
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
                    "external_execution": "disabled_v1",
                },
                "sandbox": {
                    "workspace_path_containment": true,
                    "central_side_effect_policy": true,
                    "external_skill_sandbox_available": false,
                    "external_skills_fail_closed": true,
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
    println!("credential backend: {credential_backend} (environment fallback enabled)");
    println!("executable skills: {}", executable_skills.join(", "));
    println!("external skill execution: disabled in v1 (fails closed)");
    println!(
        "side-effect policy: read={} write={} network={} process={} git={}",
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
        println!("status: all local runtime checks passed");
    } else {
        println!(
            "status: mandatory checks need attention ({})",
            failed_mandatory_checks.join(", ")
        );
    }

    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderDiagnostic {
    active: String,
    model: String,
    status: String,
}

fn provider_diagnostic(config: &AxiomConfig) -> ProviderDiagnostic {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_diagnostic_reports_no_auth_local_provider_as_ready() {
        let mut config = AxiomConfig::default();
        config.llm.active_provider = Some("ollama".to_string());
        config.llm.active_model = Some("llama3.2".to_string());
        config.providers.insert(
            "ollama".to_string(),
            ProviderConfig::OpenaiCompatible {
                base_url: "http://localhost:11434/v1".to_string(),
                api_key_env: None,
                models_url: None,
            },
        );

        let diagnostic = provider_diagnostic(&config);

        assert_eq!(diagnostic.active, "ollama");
        assert_eq!(diagnostic.model, "llama3.2");
        assert_eq!(diagnostic.status, "ready (authentication not required)");
    }

    #[test]
    fn provider_diagnostic_names_missing_key_without_exposing_a_value() {
        let environment_variable = "AXIOM_TEST_MISSING_PROVIDER_KEY_D4A1A6";
        std::env::remove_var(environment_variable);
        let mut config = AxiomConfig::default();
        config.llm.active_provider = Some("groq".to_string());
        config.llm.active_model = Some("llama-3.3-70b-versatile".to_string());
        config.providers.insert(
            "groq".to_string(),
            ProviderConfig::OpenaiCompatible {
                base_url: "https://api.groq.com/openai/v1".to_string(),
                api_key_env: Some(environment_variable.to_string()),
                models_url: None,
            },
        );

        let diagnostic = provider_diagnostic(&config);

        assert!(
            diagnostic.status == format!("missing credential: {environment_variable}")
                || diagnostic.status.starts_with(&format!(
                    "credential unavailable for {environment_variable}:"
                )),
            "unexpected diagnostic: {}",
            diagnostic.status
        );
    }

    #[test]
    fn provider_diagnostic_rejects_unsafe_endpoint_and_credential_variable() {
        let mut config = AxiomConfig::default();
        config.llm.active_provider = Some("custom".to_string());
        config.llm.active_model = Some("model".to_string());
        config.providers.insert(
            "custom".to_string(),
            ProviderConfig::OpenaiCompatible {
                base_url: "http://api.example.com/v1".to_string(),
                api_key_env: Some("PATH".to_string()),
                models_url: None,
            },
        );
        assert!(provider_diagnostic(&config)
            .status
            .starts_with("provider endpoint is invalid:"));

        let ProviderConfig::OpenaiCompatible { base_url, .. } =
            config.providers.get_mut("custom").expect("custom provider")
        else {
            panic!("expected custom provider");
        };
        *base_url = "https://api.example.com/v1".to_string();
        assert!(provider_diagnostic(&config)
            .status
            .starts_with("credential configuration is invalid:"));
    }
}
