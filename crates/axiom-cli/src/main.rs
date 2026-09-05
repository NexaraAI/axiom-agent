mod chat;
mod code_commands;
mod cost_commands;
mod credentials;
mod gateway_discord;
mod gateway_runtime;
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
    Doctor(DoctorCommand),

    Config {
        #[command(subcommand)]
        command: ConfigCommands,
    },

    /// Run or update terminal onboarding (`setup` works too).
    #[command(alias = "setup")]
    Onboarding(OnboardingCommand),

    Chat,

    Resume {
        session_id: String,
    },

    Sessions,

    Cost,

    #[command(alias = "models")]
    Model {
        #[command(subcommand)]
        command: ModelCommands,
    },

    #[command(alias = "providers")]
    Provider {
        #[command(subcommand)]
        command: ProviderCommands,
    },

    Run(RunCommand),

    Code(CodeCommand),

    Proof {
        #[command(subcommand)]
        command: ProofCommands,
    },

    Skill {
        #[command(subcommand)]
        command: SkillCommands,
    },

    Update {
        #[command(subcommand)]
        command: UpdateCommands,
    },

    /// Manage the Telegram/Discord messaging gateway (tokens, status).
    Gateway {
        #[command(subcommand)]
        command: GatewayCommands,
    },

    /// Remove Axiom (binary via npm, local data on request).
    Uninstall(UninstallCommand),
}

#[derive(Debug, Subcommand)]
enum ProofCommands {
    List,

    Latest,

    Show {
        proof_id: String,
    },

    Export {
        proof_id: String,
        #[arg(long, default_value = "markdown")]
        format: String,
    },

    Open {
        proof_id: String,
    },

    Clean {
        #[arg(long = "older-than")]
        older_than: u64,
    },
}

#[derive(Debug, Args)]
struct CodeCommand {
    #[arg(long)]
    plan_only: bool,

    #[arg(long)]
    scan: bool,

    #[arg(long)]
    diff: bool,

    #[arg(long)]
    apply: bool,

    #[arg(long = "test")]
    test: bool,

    #[arg(long)]
    explain: bool,

    #[arg(value_name = "TASK", trailing_var_arg = true)]
    task: Vec<String>,
}

#[derive(Debug, Default, Args)]
struct OnboardingCommand {
    #[arg(long)]
    non_interactive: bool,

    #[arg(long)]
    workspace: Option<String>,

    #[arg(long)]
    provider: Option<String>,

    #[arg(long)]
    model: Option<String>,

    #[arg(long)]
    account_id: Option<String>,

    #[arg(long)]
    registry: Option<String>,

    #[arg(long)]
    skip_provider: bool,

    #[arg(long)]
    yes: bool,
}

#[derive(Debug, Args)]
struct RunCommand {
    message: String,

    #[arg(long = "no-tools")]
    no_tools: bool,

    #[arg(long = "no-proof")]
    no_proof: bool,

    #[arg(long)]
    provider: Option<String>,

    #[arg(long)]
    model: Option<String>,
}

#[derive(Debug, Default, Args)]
struct DoctorCommand {
    #[arg(long)]
    json: bool,
}

#[derive(Debug, Subcommand)]
enum ConfigCommands {
    List,

    Migrate,
}

#[derive(Debug, Subcommand)]
enum ModelCommands {
    Current,

    List {
        #[arg(long)]
        provider: Option<String>,

        #[arg(long)]
        filter: Option<String>,
    },

    Use {
        model: String,
        #[arg(long)]
        provider: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum ProviderCommands {
    Current,

    List,

    Use {
        provider: String,

        #[arg(long)]
        model: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum SkillCommands {
    Registry {
        #[command(subcommand)]
        command: SkillRegistryCommands,
    },

    List,

    Search {
        query: String,
    },

    Installed,

    Bundles,

    Info {
        skill_id: String,
    },

    Run {
        skill_id: String,
        #[arg(long)]
        args: Option<String>,
    },

    Install {
        skill_id: String,
        #[arg(long)]
        registry: Option<String>,
        #[arg(long = "from-local-registry")]
        from_local_registry: Option<PathBuf>,
    },

    InstallBundle {
        bundle_id: String,
        #[arg(long)]
        registry: Option<String>,
        #[arg(long = "from-local-registry")]
        from_local_registry: Option<PathBuf>,
    },

    Update {
        #[arg(long)]
        check: bool,
        #[arg(long)]
        all: bool,
        #[arg(long = "apply-patches")]
        apply_patches: bool,
        skill_id: Option<String>,
    },

    Health,

    Enable {
        skill_id: String,
    },

    Disable {
        skill_id: String,
    },

    ResetStats {
        skill_id: String,
    },

    Remove {
        skill_id: String,
    },
}

#[derive(Debug, Subcommand)]
enum SkillRegistryCommands {
    Current,

    Set { url: String },

    Refresh,
}

#[derive(Debug, Subcommand)]
enum UpdateCommands {
    Status,

    Check,

    Install,

    Rollback,

    SetChannel { channel: String },

    SetPolicy { policy: String },
}

#[derive(Debug, Subcommand)]
enum GatewayCommands {
    /// Show gateway state: saved tokens, active provider/model the bots will use.
    Status,

    /// (Re)run the Telegram/Discord token setup.
    Setup,

    /// Forget saved gateway tokens (keep everything else).
    Disable {
        /// Only forget the Telegram token.
        #[arg(long)]
        telegram: bool,
        /// Only forget the Discord token.
        #[arg(long)]
        discord: bool,
    },

    /// Run the messaging bot (Telegram and Discord).
    Run {
        /// Run the Telegram bot.
        #[arg(long)]
        telegram: bool,
        /// Run the Discord bot.
        #[arg(long)]
        discord: bool,
    },
}

#[derive(Debug, Args)]
struct UninstallCommand {
    /// Also delete local data: config, skills, sessions, proofs, saved keys.
    #[arg(long = "delete-config")]
    delete_config: bool,

    /// Confirm without prompting.
    #[arg(long)]
    yes: bool,
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
        Some(Commands::Gateway { command }) => gateway(command).await,
        Some(Commands::Uninstall(command)) => uninstall(command),
        None => startup().await,
    }
}

async fn gateway(command: GatewayCommands) -> Result<()> {
    let config_path = AxiomConfig::default_config_path()?;
    match command {
        GatewayCommands::Status => {
            let config = AxiomConfig::load_from_path(&config_path)?;
            println!("Messaging gateway (live bots via `gateway run --telegram` / `--discord`):");
            print_gateway_token(
                "telegram",
                config.gateway.telegram_bot_token_env.as_deref(),
                &config.gateway.telegram_allowed_chat_ids,
            );
            print_gateway_token(
                "discord",
                config.gateway.discord_bot_token_env.as_deref(),
                &config.gateway.discord_allowed_guild_ids,
            );
            println!(
                "active provider: {}",
                config
                    .llm
                    .active_provider
                    .as_deref()
                    .unwrap_or("not configured")
            );
            println!(
                "active model: {}",
                config
                    .llm
                    .active_model
                    .as_deref()
                    .unwrap_or("not configured")
            );
            println!("Bots will use the active provider/model above. Change them anytime with:");
            println!("  axiom provider use <name>");
            println!("  axiom model use <id>   (find IDs via: axiom model list --filter <text>)");
            println!("Bot-side commands (supported in bot chats, see docs/GATEWAY.md):");
            println!("  /models [filter]   /model <id>   /provider <name>   /status   /help");
            Ok(())
        }
        GatewayCommands::Setup => {
            if !config_path.exists() {
                println!("No Axiom setup yet — run `axiom onboarding` first, then add messaging.");
                return Ok(());
            }
            let ui = AxiomConfig::load_from_path(&config_path)
                .map(|config| ui::Renderer::from_config(&config))
                .unwrap_or_else(|_| ui::Renderer::for_onboarding());
            onboarding::prompt_gateway_setup(&config_path, &ui).await
        }
        GatewayCommands::Disable { telegram, discord } => {
            if !telegram && !discord {
                return Err(anyhow::anyhow!(
                    "pick at least one: `axiom gateway disable --telegram` and/or `--discord`"
                ));
            }
            let mut config = AxiomConfig::load_from_path(&config_path)?;
            let mut forgotten = Vec::new();
            if telegram {
                if let Some(var) = config.gateway.telegram_bot_token_env.clone() {
                    forgotten.push(var);
                }
                config.gateway.telegram_bot_token_env = None;
                config.gateway.telegram_allowed_chat_ids.clear();
            }
            if discord {
                if let Some(var) = config.gateway.discord_bot_token_env.clone() {
                    forgotten.push(var);
                }
                config.gateway.discord_bot_token_env = None;
                config.gateway.discord_allowed_guild_ids.clear();
            }
            config.save_to_path(&config_path)?;
            for var in &forgotten {
                let _ = credentials::forget_credential(var);
            }
            println!("Gateway tokens forgotten. Provider, model, and chat settings untouched.");
            Ok(())
        }
        GatewayCommands::Run { telegram, discord } => {
            let config_path = AxiomConfig::default_config_path()?;
            match (telegram, discord) {
                (true, false) => gateway_runtime::run_telegram_gateway(config_path).await,
                (false, true) => gateway_discord::run_discord_gateway(config_path).await,
                (true, true) => Err(anyhow::anyhow!(
                    "run one gateway per process: `axiom gateway run --telegram` or `--discord`"
                )),
                (false, false) => {
                    println!("Pick a gateway: `axiom gateway run --telegram` or `--discord`.");
                    Ok(())
                }
            }
        }
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

fn print_gateway_token(platform: &str, token_env: Option<&str>, allowlist: &[String]) {
    match token_env {
        None => println!("{platform}: not configured (run `axiom gateway setup`)"),
        Some(var) => {
            let state = match credentials::resolve_credential(var) {
                Ok(Some(_)) => "token saved".to_string(),
                Ok(None) => "token named but MISSING — re-run `axiom gateway setup`".to_string(),
                Err(error) => format!("token unreadable: {error}"),
            };
            println!("{platform}: {state} (var {var})");
            if allowlist.is_empty() {
                println!("  allowed chats: not restricted yet (anyone with the bot link could talk to it — set IDs in setup)");
            } else {
                println!("  allowed chats: {}", allowlist.join(", "));
            }
        }
    }
}

fn uninstall(command: UninstallCommand) -> Result<()> {
    let config_dir = AxiomConfig::default_config_dir()?;
    if !command.delete_config {
        println!("This removes the installed program. Your data stays where it is.");
        println!("  npm rm -g axiom-agent");
        println!();
        println!("Config, skills, sessions, proofs, and saved keys live in:");
        println!("  {}", config_dir.display());
        println!("To wipe those too:");
        println!("  axiom uninstall --delete-config --yes");
        return Ok(());
    }
    if !command.yes
        && !chat::confirm(
            &format!(
                "Permanently delete {} and everything in it?",
                config_dir.display()
            ),
            false,
        )?
    {
        println!("Cancelled. Nothing was deleted.");
        return Ok(());
    }
    if config_dir.exists() {
        std::fs::remove_dir_all(&config_dir)?;
        println!("Deleted {}.", config_dir.display());
    } else {
        println!(
            "Nothing to delete: {} does not exist.",
            config_dir.display()
        );
    }
    println!("Then remove the program itself with: npm rm -g axiom-agent");
    Ok(())
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
    use std::io::IsTerminal;
    let config_path = AxiomConfig::default_config_path()?;
    match startup::route_for_config_path(&config_path)? {
        StartupRoute::Onboarding => {
            if !std::io::stdin().is_terminal() {
                eprintln!("Welcome to Axiom! Setup isn't complete yet.");
                eprintln!("You're not in an interactive terminal, so I won't start the questionnaire here.");
                eprintln!();
                eprintln!("Next steps (pick one):");
                eprintln!("  1. Run interactively:  axiom onboarding");
                eprintln!("  2. Scripted setup:      axiom onboarding --non-interactive --provider groq --model <model> --workspace ~/Axiom --yes");
                eprintln!("  3. Try offline first:   axiom onboarding --non-interactive --provider mock --workspace ./demo-workspace --yes");
                eprintln!();
                eprintln!("Then run `axiom doctor` to verify, and `axiom` to chat.");
                return Ok(());
            }
            println!("Welcome to Axiom! Let's get you set up (takes ~1 minute).");
            println!("I'll walk you through 3 quick steps: workspace → provider → skills.");
            run_onboarding_then_doctor(OnboardingCommand::default()).await?;
            if startup::route_for_config_path(&config_path)? == StartupRoute::Chat {
                chat::run_terminal_chat().await
            } else {
                println!();
                println!("You're almost there — provider setup is still incomplete.");
                println!("Run `axiom onboarding` when you're ready, or `axiom doctor` to see what's missing.");
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
        use std::io::IsTerminal;
        println!("Welcome! Axiom needs a quick one-time setup before chat.");
        if !std::io::stdin().is_terminal() {
            println!("Non-interactive session detected, so I won't prompt here.");
            println!("Run `axiom onboarding` in a terminal, or use:");
            println!("  axiom onboarding --non-interactive --provider mock --workspace ./demo-workspace --yes");
            return Ok(());
        }
        if chat::confirm("Start the 1-minute setup now?", true)? {
            run_onboarding_then_doctor(OnboardingCommand::default()).await?;
            if startup::route_for_config_path(&config_path)? == StartupRoute::Onboarding {
                println!();
                println!("Setup is still incomplete — no worries, you can resume anytime.");
                println!("Run `axiom onboarding` when you're ready, or `axiom doctor` to see what's missing.");
                return Ok(());
            }
        } else {
            println!("No problem! Run `axiom onboarding` whenever you're ready.");
            println!("Tip: `axiom doctor` shows what's missing.");
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
