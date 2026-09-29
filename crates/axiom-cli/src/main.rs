mod chat;
mod cli_args;
mod code_commands;
mod cost_commands;
mod credentials;
mod doctor;
mod gateway_discord;
mod gateway_runtime;
mod identity;
mod mcp_commands;
mod onboarding;
mod proof_commands;
mod side_effects;
mod skill_commands;
mod startup;
mod ui;
mod update_commands;

use doctor::doctor;
#[cfg(test)]
use doctor::provider_diagnostic;

use anyhow::Result;
use axiom_core::AxiomConfig;
use clap::Parser;
use startup::StartupRoute;

pub(crate) use cli_args::{
    Cli, CodeCommand, Commands, ConfigCommands, GatewayCommands, McpCommands, ModelCommands,
    OnboardingCommand, ProofCommands, ProviderCommands, RunCommand, SkillCommands,
    SkillRegistryCommands, UninstallCommand, UpdateCommands,
};

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
        Some(Commands::Chat(command)) => {
            chat(chat::ChatOptions {
                workspace: command.workspace,
                inline: command.inline,
            })
            .await
        }
        Some(Commands::Resume { session_id }) => chat::resume_terminal_chat(&session_id).await,
        Some(Commands::Sessions) => chat::list_sessions(),
        Some(Commands::Cost) => cost_commands::run(),
        Some(Commands::Model { command }) => model(command).await,
        Some(Commands::Provider { command }) => provider(command).await,
        Some(Commands::Run(command)) => chat::run_one_shot(command).await,
        Some(Commands::Code(command)) => code_commands::run(command).await,
        Some(Commands::Proof { command }) => proof_commands::run(command),
        Some(Commands::Skill { command }) => skill_commands::run(command).await,
        Some(Commands::Update { command }) => update_commands::run(command).await,
        Some(Commands::Gateway { command }) => gateway(command).await,
        Some(Commands::Mcp { command }) => mcp_commands::run(command).await,
        Some(Commands::Uninstall(command)) => uninstall(command),
        None => startup().await,
    }
}

async fn gateway(command: GatewayCommands) -> Result<()> {
    let config_path = AxiomConfig::default_config_path()?;
    match command {
        GatewayCommands::Status => {
            let config = AxiomConfig::load_or_create(&config_path)?;
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
                    "pick at least one: `axiom gateway disable --telegram` and/or `--discord` (see `axiom gateway status`)"
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
                    println!("No --telegram/--discord flag given; starting both bots.");
                    println!(
                        "(To run a single bot, use `axiom gateway run --telegram` or `--discord`.)"
                    );
                    tokio::try_join!(
                        gateway_runtime::run_telegram_gateway(config_path.clone()),
                        gateway_discord::run_discord_gateway(config_path)
                    )
                    .map(|_| ())
                }
            }
        }
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

async fn provider(command: ProviderCommands) -> Result<()> {
    let config_path = AxiomConfig::default_config_path()?;
    match command {
        ProviderCommands::Current => {
            let config = AxiomConfig::load_or_create(&config_path)?;
            println!(
                "provider: {}",
                config
                    .llm
                    .active_provider
                    .as_deref()
                    .unwrap_or("not configured")
            );
        }
        ProviderCommands::List => {
            let config = AxiomConfig::load_or_create(&config_path)?;
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
            let mut session = chat::ChatSession::load(&config_path)?;
            let provider = session.set_provider(provider)?;
            if let Some(model) = model {
                session.set_model(model)?;
            }
            println!(
                "Provider switched to {provider} with model {}.",
                session.active_model().unwrap_or("not configured")
            );
        }
        ProviderCommands::Add {
            name,
            model,
            api_key,
            base_url,
            activate,
        } => {
            let mut config = AxiomConfig::load_or_create(&config_path)?;
            if let Some(provider_name) = name {
                if let Some(preset) = onboarding::provider_preset(&provider_name) {
                    let key_env = preset.api_key_env;
                    if let Some(key) = api_key {
                        if let Some(env_name) = key_env {
                            credentials::store_credential(env_name, &key)?;
                        }
                    }
                    let chosen_model = model
                        .or_else(|| preset.default_model.map(str::to_string))
                        .unwrap_or_else(|| "default".to_string());
                    let setup = onboarding::ProviderSetup::OpenAiCompatible {
                        provider_name: preset.id.to_string(),
                        base_url: base_url.unwrap_or_else(|| preset.base_url.to_string()),
                        api_key_env: key_env.map(str::to_string),
                        models_url: preset.models_url.map(str::to_string),
                        default_model: chosen_model.clone(),
                    };
                    let previous_provider = config.llm.active_provider.clone();
                    let previous_model = config.llm.active_model.clone();
                    onboarding::apply_provider_setup(&mut config, &setup);
                    if !activate && previous_provider.is_some() {
                        config.llm.active_provider = previous_provider;
                        config.llm.active_model = previous_model;
                    } else if activate {
                        config.llm.active_provider = Some(preset.id.to_string());
                        config.llm.active_model = Some(chosen_model);
                    }
                    if activate {
                        config.agent.first_run_completed = config.llm.active_provider.is_some()
                            && config.llm.active_model.is_some();
                    }
                    config.save_to_path(&config_path)?;
                    println!("Provider '{}' configured successfully!", preset.id);
                } else {
                    let b_url = base_url.ok_or_else(|| {
                        anyhow::anyhow!("--base-url required for custom provider")
                    })?;
                    let key_env = format!(
                        "{}_API_KEY",
                        provider_name.to_ascii_uppercase().replace('-', "_")
                    );
                    if let Some(key) = api_key {
                        credentials::store_credential(&key_env, &key)?;
                    }
                    let chosen_model = model.unwrap_or_else(|| "default".to_string());
                    let setup = onboarding::ProviderSetup::OpenAiCompatible {
                        provider_name: provider_name.clone(),
                        base_url: b_url,
                        api_key_env: Some(key_env),
                        models_url: None,
                        default_model: chosen_model.clone(),
                    };
                    let previous_provider = config.llm.active_provider.clone();
                    let previous_model = config.llm.active_model.clone();
                    onboarding::apply_provider_setup(&mut config, &setup);
                    if !activate && previous_provider.is_some() {
                        config.llm.active_provider = previous_provider;
                        config.llm.active_model = previous_model;
                    } else if activate {
                        config.llm.active_provider = Some(provider_name.clone());
                        config.llm.active_model = Some(chosen_model);
                    }
                    if activate {
                        config.agent.first_run_completed = config.llm.active_provider.is_some()
                            && config.llm.active_model.is_some();
                    }
                    config.save_to_path(&config_path)?;
                    println!("Custom provider '{provider_name}' configured successfully!");
                }
            } else {
                let setup = onboarding::prompt_preset_setup("openrouter").await?;
                onboarding::apply_provider_setup(&mut config, &setup);
                config.agent.first_run_completed =
                    config.llm.active_provider.is_some() && config.llm.active_model.is_some();
                config.save_to_path(&config_path)?;
                println!("Provider added successfully!");
            }
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
        ModelCommands::Use {
            model,
            provider,
            force,
        } => {
            if let Some(provider) = provider {
                session.set_provider(provider)?;
            }
            match session.resolve_and_switch_model(&model, force).await? {
                chat::ModelSwitchOutcome::Switched { model }
                | chat::ModelSwitchOutcome::ForceSwitched { model }
                | chat::ModelSwitchOutcome::ResolvedAndSwitched {
                    resolved: model, ..
                }
                | chat::ModelSwitchOutcome::CatalogUnreachable { model } => {
                    println!(
                        "Model switched to {model} for {}.",
                        session.active_provider().unwrap_or("active provider")
                    );
                }
                outcome @ (chat::ModelSwitchOutcome::Ambiguous { .. }
                | chat::ModelSwitchOutcome::NotFound { .. }) => {
                    anyhow::bail!("{}", outcome.display_message());
                }
            }
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
        ConfigCommands::Path => {
            println!("{}", path.display());
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
            onboarding::run_onboarding_command(OnboardingCommand::default()).await?;
            if startup::route_for_config_path(&config_path)? == StartupRoute::Chat {
                chat::run_terminal_chat(chat::ChatOptions::default()).await
            } else {
                println!();
                println!("You're almost there — provider setup is still incomplete.");
                println!("Run `axiom onboarding` when you're ready, or `axiom doctor` to see what's missing.");
                Ok(())
            }
        }
        StartupRoute::Chat => chat(chat::ChatOptions::default()).await,
    }
}

async fn run_onboarding_then_doctor(command: OnboardingCommand) -> Result<()> {
    onboarding::run_onboarding_command(command).await?;
    doctor(false)
}

async fn chat(options: chat::ChatOptions) -> Result<()> {
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

    chat::run_terminal_chat(options).await
}

#[cfg(test)]
mod tests {
    use axiom_core::ProviderConfig;

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

    #[tokio::test]
    async fn provider_command_succeeds_when_config_file_does_not_exist() {
        let dir = std::env::temp_dir().join(format!(
            "axiom-provider-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_nanos()
        ));
        let config_path = dir.join("nested").join("config.toml");
        assert!(!config_path.exists());

        let provider_name = "LMStudio".to_string();
        let chosen_model = "prism-ml/bonsai-27b".to_string();
        let b_url = "https://subaka.ddns.net/v1/chat/completions".to_string();
        let key_env = format!(
            "{}_API_KEY",
            provider_name.to_ascii_uppercase().replace('-', "_")
        );
        let setup = onboarding::ProviderSetup::OpenAiCompatible {
            provider_name: provider_name.clone(),
            base_url: b_url,
            api_key_env: Some(key_env),
            models_url: None,
            default_model: chosen_model.clone(),
        };

        let mut config =
            AxiomConfig::load_or_create(&config_path).expect("load_or_create succeeds");
        onboarding::apply_provider_setup(&mut config, &setup);
        config.llm.active_provider = Some(provider_name.clone());
        config.llm.active_model = Some(chosen_model.clone());
        config.agent.first_run_completed =
            config.llm.active_provider.is_some() && config.llm.active_model.is_some();
        config
            .save_to_path(&config_path)
            .expect("save_to_path succeeds");

        assert!(config_path.exists());
        let reloaded = AxiomConfig::load_from_path(&config_path).expect("load saved config");
        assert_eq!(reloaded.llm.active_provider.as_deref(), Some("LMStudio"));
        assert_eq!(
            reloaded.llm.active_model.as_deref(),
            Some("prism-ml/bonsai-27b")
        );
        assert!(reloaded.agent.first_run_completed);

        let non_existent_session_config = dir.join("another_nested").join("config.toml");
        let session = chat::ChatSession::load(&non_existent_session_config)
            .expect("chat session loads or creates");
        assert!(non_existent_session_config.exists());
        assert_eq!(session.config_path, non_existent_session_config);

        let _ = std::fs::remove_dir_all(dir);
    }
}
