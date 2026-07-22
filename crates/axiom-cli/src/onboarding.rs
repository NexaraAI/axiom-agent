use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Context, Result};
use axiom_core::{atomic_write, AxiomConfig, ProviderConfig};
use axiom_engine::{
    essential_bundle_id_for_os, install_bundle_from_registry_client, LocalRegistrySource,
    RegistryClient, RegistrySource,
};
use axiom_llm::{LlmProvider, ModelInfo, OpenAiCompatibleProvider};

use crate::{ui::Renderer, OnboardingCommand};

const DEFAULT_WORKSPACE: &str = "~/Axiom";
// The registry schema version and the executable version form an immutable
// generation. A later binary therefore never mistakes an incomplete or stale
// starter registry for the assets it was built with.
const EMBEDDED_REGISTRY_DIR: &str = concat!("bundled-registry/v0.1-", env!("CARGO_PKG_VERSION"));
const EMBEDDED_REGISTRY_COMPLETE_MARKER: &str = ".complete";
const EMBEDDED_REGISTRY_MARKER_CONTENTS: &str = concat!(
    "Axiom embedded starter registry ",
    env!("CARGO_PKG_VERSION"),
    "\n"
);

struct EmbeddedRegistryFile {
    relative_path: &'static str,
    contents: &'static [u8],
}

// Keep the complete offline starter registry inside the executable. On first
// use these immutable assets are materialized under AXIOM_HOME, so a binary
// installed through npm or copied to another machine never depends on the
// repository checkout it was built from.
static EMBEDDED_REGISTRY_FILES: &[EmbeddedRegistryFile] = &[
    EmbeddedRegistryFile {
        relative_path: "registry.json",
        contents: include_bytes!("../../../fixtures/skill-registry/registry.json"),
    },
    EmbeddedRegistryFile {
        relative_path: "bundles/essential.windows.toml",
        contents: include_bytes!("../../../fixtures/skill-registry/bundles/essential.windows.toml"),
    },
    EmbeddedRegistryFile {
        relative_path: "bundles/essential.linux.toml",
        contents: include_bytes!("../../../fixtures/skill-registry/bundles/essential.linux.toml"),
    },
    EmbeddedRegistryFile {
        relative_path: "bundles/essential.macos.toml",
        contents: include_bytes!("../../../fixtures/skill-registry/bundles/essential.macos.toml"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/file.read/skill.toml",
        contents: include_bytes!("../../../fixtures/skill-registry/skills/file.read/skill.toml"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/file.read/README.md",
        contents: include_bytes!("../../../fixtures/skill-registry/skills/file.read/README.md"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/file.write/skill.toml",
        contents: include_bytes!("../../../fixtures/skill-registry/skills/file.write/skill.toml"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/file.write/README.md",
        contents: include_bytes!("../../../fixtures/skill-registry/skills/file.write/README.md"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/project.scan/skill.toml",
        contents: include_bytes!("../../../fixtures/skill-registry/skills/project.scan/skill.toml"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/project.scan/README.md",
        contents: include_bytes!("../../../fixtures/skill-registry/skills/project.scan/README.md"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/web.fetch/skill.toml",
        contents: include_bytes!("../../../fixtures/skill-registry/skills/web.fetch/skill.toml"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/web.fetch/README.md",
        contents: include_bytes!("../../../fixtures/skill-registry/skills/web.fetch/README.md"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/shell.powershell.safe/skill.toml",
        contents: include_bytes!(
            "../../../fixtures/skill-registry/skills/shell.powershell.safe/skill.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/shell.powershell.safe/README.md",
        contents: include_bytes!(
            "../../../fixtures/skill-registry/skills/shell.powershell.safe/README.md"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/shell.bash.safe/skill.toml",
        contents: include_bytes!(
            "../../../fixtures/skill-registry/skills/shell.bash.safe/skill.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/shell.bash.safe/README.md",
        contents: include_bytes!(
            "../../../fixtures/skill-registry/skills/shell.bash.safe/README.md"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/shell.zsh.safe/skill.toml",
        contents: include_bytes!(
            "../../../fixtures/skill-registry/skills/shell.zsh.safe/skill.toml"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/shell.zsh.safe/README.md",
        contents: include_bytes!(
            "../../../fixtures/skill-registry/skills/shell.zsh.safe/README.md"
        ),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/git.status/skill.toml",
        contents: include_bytes!("../../../fixtures/skill-registry/skills/git.status/skill.toml"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/git.status/README.md",
        contents: include_bytes!("../../../fixtures/skill-registry/skills/git.status/README.md"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/git.diff/skill.toml",
        contents: include_bytes!("../../../fixtures/skill-registry/skills/git.diff/skill.toml"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/git.diff/README.md",
        contents: include_bytes!("../../../fixtures/skill-registry/skills/git.diff/README.md"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/python.write/skill.toml",
        contents: include_bytes!("../../../fixtures/skill-registry/skills/python.write/skill.toml"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/python.write/README.md",
        contents: include_bytes!("../../../fixtures/skill-registry/skills/python.write/README.md"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/python.run/skill.toml",
        contents: include_bytes!("../../../fixtures/skill-registry/skills/python.run/skill.toml"),
    },
    EmbeddedRegistryFile {
        relative_path: "skills/python.run/README.md",
        contents: include_bytes!("../../../fixtures/skill-registry/skills/python.run/README.md"),
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProviderPreset {
    id: &'static str,
    base_url: &'static str,
    api_key_env: Option<&'static str>,
    models_url: Option<&'static str>,
    default_model: Option<&'static str>,
    setup_note: &'static str,
}

const PROVIDER_PRESETS: &[ProviderPreset] = &[
    ProviderPreset {
        id: "groq",
        base_url: "https://api.groq.com/openai/v1",
        api_key_env: Some("GROQ_API_KEY"),
        models_url: None,
        default_model: Some("llama-3.3-70b-versatile"),
        setup_note: "Hosted API. A Groq account/key is required; free developer limits may apply.",
    },
    ProviderPreset {
        id: "openrouter",
        base_url: "https://openrouter.ai/api/v1",
        api_key_env: Some("OPENROUTER_API_KEY"),
        models_url: None,
        default_model: Some("openrouter/free"),
        setup_note:
            "Hosted API. The default free router is zero-price but rate and daily limits apply.",
    },
    ProviderPreset {
        id: "gemini",
        base_url: "https://generativelanguage.googleapis.com/v1beta/openai",
        api_key_env: Some("GEMINI_API_KEY"),
        models_url: None,
        default_model: Some("gemini-2.5-flash"),
        setup_note: "Hosted API. Free-tier availability depends on model, account, and region.",
    },
    ProviderPreset {
        id: "github-models",
        base_url: "https://models.github.ai/inference",
        api_key_env: Some("GITHUB_TOKEN"),
        models_url: Some("https://models.github.ai/catalog/models"),
        default_model: Some("openai/gpt-4.1"),
        setup_note: "Hosted preview API with included rate-limited GitHub account usage.",
    },
    ProviderPreset {
        id: "nvidia",
        base_url: "https://integrate.api.nvidia.com/v1",
        api_key_env: Some("NVIDIA_API_KEY"),
        models_url: None,
        default_model: Some("meta/llama-3.3-70b-instruct"),
        setup_note: "Hosted API. NVIDIA NIM microservices. An NVIDIA API key is required.",
    },
    ProviderPreset {
        id: "openai",
        base_url: "https://api.openai.com/v1",
        api_key_env: Some("OPENAI_API_KEY"),
        models_url: None,
        default_model: None,
        setup_note: "Hosted paid API. Select a model available to your OpenAI project.",
    },
    ProviderPreset {
        id: "ollama",
        base_url: "http://localhost:11434/v1",
        api_key_env: None,
        models_url: None,
        default_model: Some("llama3.2"),
        setup_note: "Local and no-key by default. Start Ollama and pull a model first.",
    },
    ProviderPreset {
        id: "lm-studio",
        base_url: "http://localhost:1234/v1",
        api_key_env: None,
        models_url: None,
        default_model: None,
        setup_note: "Local and no-key by default. Start the LM Studio server and load a model.",
    },
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OnboardingPlan {
    pub workspace: String,
    pub provider: ProviderSetup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderSetup {
    Multiple {
        providers: Vec<ProviderSetup>,
    },
    Mock {
        default_model: String,
    },
    Cloudflare {
        account_id: String,
        gateway_id: String,
        api_token_env: String,
        default_model: String,
    },
    OpenAiCompatible {
        provider_name: String,
        base_url: String,
        api_key_env: Option<String>,
        models_url: Option<String>,
        default_model: String,
    },
    Skip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OnboardingResult {
    pub config_path: PathBuf,
    pub workspace_path: PathBuf,
    pub installed_skills: Vec<String>,
    pub registry_source: String,
}

pub(crate) async fn run_terminal_onboarding() -> Result<OnboardingResult> {
    let config_path = AxiomConfig::default_config_path()?;
    let existing_config = if config_path.exists() {
        Some(AxiomConfig::load_from_path(&config_path)?)
    } else {
        None
    };
    // A returning user should see the terminal preferences they already chose,
    // including an opt-out of ANSI color, while a first run keeps the friendly
    // onboarding defaults.
    let ui = existing_config
        .as_ref()
        .map(Renderer::from_config)
        .unwrap_or_else(Renderer::for_onboarding);

    println!("{}", ui.onboarding_banner());
    println!("{}", ui.header("config", config_path.display()));

    if let Some(existing) = existing_config {
        println!("{}", ui.warning("Existing config found."));
        println!(
            "{}",
            ui.plain("This will update your existing Axiom setup if you continue.")
        );
        if !confirm("Update existing setup?", false)? {
            println!(
                "{}",
                ui.plain("Onboarding skipped. Existing config was left unchanged.")
            );
            let workspace_path = existing.default_workspace_path();
            return Ok(OnboardingResult {
                config_path,
                workspace_path,
                installed_skills: Vec::new(),
                registry_source: "unchanged".to_string(),
            });
        }
    } else if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
        println!(
            "{}",
            ui.success(&format!("Created config directory: {}", parent.display()))
        );
    }

    let plan = prompt_for_plan().await?;
    let result = apply_onboarding_plan(&config_path, &plan).await?;

    println!(
        "{}",
        ui.success(&format!("Saved config: {}", result.config_path.display()))
    );
    println!(
        "{}",
        ui.header("workspace", result.workspace_path.display())
    );
    println!(
        "{}",
        ui.success(&format!(
            "Installed starter skills from {} registry: {}",
            result.registry_source,
            result.installed_skills.join(", ")
        ))
    );

    Ok(result)
}

pub(crate) async fn run_onboarding_command(command: OnboardingCommand) -> Result<OnboardingResult> {
    if command.non_interactive {
        run_non_interactive_onboarding(command).await
    } else {
        run_terminal_onboarding().await
    }
}

async fn run_non_interactive_onboarding(command: OnboardingCommand) -> Result<OnboardingResult> {
    if !command.yes {
        bail!("non-interactive onboarding requires --yes");
    }
    let workspace = command
        .workspace
        .ok_or_else(|| anyhow!("non-interactive onboarding requires --workspace"))?;
    if command.skip_provider && command.provider.is_some() {
        bail!("use either --skip-provider or --provider, not both");
    }
    if command.account_id.is_some() && command.provider.as_deref() != Some("cloudflare") {
        bail!("--account-id is only valid with --provider cloudflare");
    }

    let provider = if command.skip_provider {
        ProviderSetup::Skip
    } else {
        let provider = command.provider.as_deref().ok_or_else(|| {
            anyhow!("non-interactive onboarding requires --provider or --skip-provider")
        })?;
        match provider {
            "mock" => ProviderSetup::Mock {
                default_model: command.model.unwrap_or_else(|| "mock-model".to_string()),
            },
            "cloudflare" => ProviderSetup::Cloudflare {
                account_id: command
                    .account_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|account_id| !account_id.is_empty())
                    .map(str::to_string)
                    .ok_or_else(|| {
                        anyhow!("--provider cloudflare requires an explicit --account-id")
                    })?,
                gateway_id: "default".to_string(),
                api_token_env: "CLOUDFLARE_API_TOKEN".to_string(),
                default_model: command
                    .model
                    .ok_or_else(|| anyhow!("--provider cloudflare requires --model"))?,
            },
            other => provider_setup_from_preset(other, command.model)?,
        }
    };

    let plan = OnboardingPlan {
        workspace,
        provider,
    };
    let config_path = AxiomConfig::default_config_path()?;
    let bundled_registry_root = materialize_embedded_registry(&config_path)?;
    let result = apply_onboarding_plan_with_registry_url(
        &config_path,
        &plan,
        command.registry,
        &bundled_registry_root,
        std::env::consts::OS,
    )
    .await?;

    println!("Saved config: {}", result.config_path.display());
    println!("Workspace: {}", result.workspace_path.display());
    println!(
        "Installed starter skills from {} registry: {}",
        result.registry_source,
        result.installed_skills.join(", ")
    );

    Ok(result)
}

pub(crate) async fn apply_onboarding_plan(
    config_path: impl AsRef<Path>,
    plan: &OnboardingPlan,
) -> Result<OnboardingResult> {
    let config_path = config_path.as_ref();
    let bundled_registry_root = materialize_embedded_registry(config_path)?;
    apply_onboarding_plan_with_registry_url(
        config_path,
        plan,
        None,
        &bundled_registry_root,
        std::env::consts::OS,
    )
    .await
}

pub(crate) async fn apply_onboarding_plan_with_registry_url(
    config_path: impl AsRef<Path>,
    plan: &OnboardingPlan,
    registry_url_override: Option<String>,
    fallback_registry_root: impl AsRef<Path>,
    os: &str,
) -> Result<OnboardingResult> {
    let config_path = config_path.as_ref();
    let config_dir = config_path
        .parent()
        .ok_or_else(|| anyhow!("config path has no parent directory"))?;
    fs::create_dir_all(config_dir)?;

    let mut config = if config_path.exists() {
        // Onboarding is also a supported migration path. Migrate first so a
        // successful setup can never leave a legacy schema behind, then mutate
        // only the fields the setup flow owns.
        AxiomConfig::migrate_file(config_path)?;
        configure_onboarding(AxiomConfig::load_from_path(config_path)?, plan, true)
    } else {
        let mut config = build_config(plan);
        config.skills.local_dir = "skills".to_string();
        config
    };

    let workspace_path = config.default_workspace_path();
    fs::create_dir_all(&workspace_path).with_context(|| {
        format!(
            "failed to create workspace directory {}",
            workspace_path.display()
        )
    })?;

    let skills_dir = config_dir.join(&config.skills.local_dir);
    println!("Installing essential skills for {os}...");
    if let Some(registry_url) = registry_url_override.as_ref() {
        // An explicit command-line registry always wins, even if it happens to
        // resemble the old development fixture path.
        config.skills.registry_url = registry_url.clone();
    } else if is_legacy_fixture_registry_location(&config.skills.registry_url) {
        // Older development builds accidentally persisted their source-tree
        // fixture path. It cannot exist for a packaged or relocated binary, so
        // restore the official default and use the embedded registry only as a
        // temporary installation source below.
        config.skills.registry_url = AxiomConfig::default().skills.registry_url;
    }
    // Mock/skip setup must be immediately usable offline, but the internal
    // fallback location is an installation detail rather than the user's
    // configured registry. Keep an explicit/custom registry untouched and
    // persist only an explicit `--registry` override.
    let use_bundled_directly = registry_url_override.is_none()
        && matches!(
            &plan.provider,
            ProviderSetup::Mock { .. } | ProviderSetup::Skip
        )
        && config.skills.registry_url == AxiomConfig::default().skills.registry_url;
    let registry_url = if use_bundled_directly {
        fallback_registry_root.as_ref().display().to_string()
    } else {
        config.skills.registry_url.clone()
    };
    let install_result = install_essential_skills_for_os(
        &skills_dir,
        os,
        &registry_url,
        config.skills.fallback_to_bundled_registry,
        fallback_registry_root,
    )
    .await?;
    let installed_skills = install_result.installed_skill_ids;

    config.save_to_path(config_path)?;

    Ok(OnboardingResult {
        config_path: config_path.to_path_buf(),
        workspace_path,
        installed_skills,
        registry_source: install_result.source,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EssentialSkillInstallResult {
    pub source: String,
    pub installed_skill_ids: Vec<String>,
}

pub(crate) async fn install_essential_skills_for_os(
    skills_dir: impl AsRef<Path>,
    os: &str,
    registry_location: &str,
    fallback_to_bundled_registry: bool,
    fallback_registry_root: impl AsRef<Path>,
) -> Result<EssentialSkillInstallResult> {
    let bundle_id = essential_bundle_id_for_os(os)
        .ok_or_else(|| anyhow!("unsupported platform for essential skill bundle: {os}"))?;
    let direct_bundled = same_existing_path(registry_location, fallback_registry_root.as_ref());
    match load_registry_client_from_location(registry_location).await {
        Ok(client) => {
            let source_label = if direct_bundled {
                "bundled"
            } else {
                client.source_label()
            };
            println!("Registry: {source_label}");
            let records =
                install_bundle_from_registry_client(&client, bundle_id, skills_dir, source_label)
                    .await?;
            for record in &records {
                println!("installed: {}", record.id);
            }
            Ok(EssentialSkillInstallResult {
                source: source_label.to_string(),
                installed_skill_ids: records.into_iter().map(|record| record.id).collect(),
            })
        }
        Err(error) if fallback_to_bundled_registry => {
            println!("Remote registry unavailable. Using bundled fallback registry. ({error})");
            let client = RegistryClient::from_source(RegistrySource::Local(LocalRegistrySource {
                registry_path: fallback_registry_root.as_ref().join("registry.json"),
            }))
            .await?;
            let records =
                install_bundle_from_registry_client(&client, bundle_id, skills_dir, "bundled")
                    .await?;
            for record in &records {
                println!("installed: {}", record.id);
            }
            Ok(EssentialSkillInstallResult {
                source: "bundled".to_string(),
                installed_skill_ids: records.into_iter().map(|record| record.id).collect(),
            })
        }
        Err(error) => Err(error),
    }
}

fn same_existing_path(location: &str, expected: &Path) -> bool {
    let location = PathBuf::from(location);
    location.exists()
        && expected.exists()
        && fs::canonicalize(location).ok() == fs::canonicalize(expected).ok()
}

pub(crate) fn build_config(plan: &OnboardingPlan) -> AxiomConfig {
    configure_onboarding(AxiomConfig::default(), plan, false)
}

fn configure_onboarding(
    mut config: AxiomConfig,
    plan: &OnboardingPlan,
    preserve_existing_provider_on_skip: bool,
) -> AxiomConfig {
    config.agent.default_workspace = plan.workspace.clone();

    // A skip means "leave my provider alone" when onboarding an existing
    // installation. For a fresh config we still clear the illustrative
    // defaults so setup remains visibly incomplete until a provider is chosen.
    if preserve_existing_provider_on_skip && matches!(&plan.provider, ProviderSetup::Skip) {
        return config;
    }

    config.agent.first_run_completed = false;
    config.providers.clear();
    config.llm.active_provider = None;
    config.llm.active_model = None;
    config.llm.provider_models.clear();

    match &plan.provider {
        ProviderSetup::Multiple { providers } => {
            for provider in providers.iter().rev() {
                apply_provider_setup(&mut config, provider);
            }
        }
        provider => apply_provider_setup(&mut config, provider),
    }

    config.agent.first_run_completed = config.llm.active_provider.is_some()
        && config
            .llm
            .active_model
            .as_deref()
            .is_some_and(|model| !model.trim().is_empty());

    config
}

fn apply_provider_setup(config: &mut AxiomConfig, provider: &ProviderSetup) {
    match provider {
        ProviderSetup::Multiple { .. } => {}
        ProviderSetup::Mock { default_model } => {
            config
                .providers
                .insert("mock".to_string(), ProviderConfig::Mock {});
            config.llm.active_provider = Some("mock".to_string());
            config.llm.active_model = Some(default_model.clone());
            config
                .llm
                .provider_models
                .insert("mock".to_string(), default_model.clone());
        }
        ProviderSetup::Cloudflare {
            account_id,
            gateway_id,
            api_token_env,
            default_model,
        } => {
            config.providers.insert(
                "cloudflare".to_string(),
                ProviderConfig::CloudflareAiGateway {
                    account_id: account_id.clone(),
                    gateway_id: gateway_id.clone(),
                    api_token_env: api_token_env.clone(),
                    base_url: "https://api.cloudflare.com/client/v4/accounts/{account_id}/ai/v1"
                        .to_string(),
                },
            );
            config.llm.active_provider = Some("cloudflare".to_string());
            config.llm.active_model = Some(default_model.clone());
            config
                .llm
                .provider_models
                .insert("cloudflare".to_string(), default_model.clone());
        }
        ProviderSetup::OpenAiCompatible {
            provider_name,
            base_url,
            api_key_env,
            models_url,
            default_model,
        } => {
            config.providers.insert(
                provider_name.clone(),
                ProviderConfig::OpenaiCompatible {
                    base_url: base_url.clone(),
                    api_key_env: api_key_env.clone(),
                    models_url: models_url.clone(),
                },
            );
            config.llm.active_provider = Some(provider_name.clone());
            config.llm.active_model = Some(default_model.clone());
            config
                .llm
                .provider_models
                .insert(provider_name.clone(), default_model.clone());
        }
        ProviderSetup::Skip => {}
    }
}

async fn prompt_for_plan() -> Result<OnboardingPlan> {
    let workspace = prompt_with_default("Workspace path", DEFAULT_WORKSPACE)?;
    let provider = prompt_provider_setup().await?;

    Ok(OnboardingPlan {
        workspace,
        provider,
    })
}

async fn prompt_provider_setup() -> Result<ProviderSetup> {
    println!();
    println!("Provider setup:");
    println!("1) Groq (free developer tier, rate limited)");
    println!("2) OpenRouter (free-model router, rate limited)");
    println!("3) Gemini (free tier where available)");
    println!("4) GitHub Models (included free quota, rate limited)");
    println!("5) NVIDIA NIM (hosted microservices API)");
    println!("6) Ollama (local, no API key)");
    println!("7) LM Studio (local, no API key by default)");
    println!("8) OpenAI");
    println!("9) Cloudflare AI Gateway");
    println!("10) Custom OpenAI-compatible endpoint");
    println!("11) Skip for now");

    loop {
        let selection = prompt_with_default(
            "Choose one or two providers (comma-separated; first is active)",
            "2",
        )?;
        let mut choices = selection
            .split(',')
            .map(str::trim)
            .filter(|choice| !choice.is_empty())
            .collect::<Vec<_>>();
        choices.dedup();
        if choices.is_empty() || choices.len() > 2 {
            println!("Choose one provider or two comma-separated providers.");
            continue;
        }
        if choices.contains(&"11") && choices.len() > 1 {
            println!("Skip cannot be combined with another provider.");
            continue;
        }

        let mut providers = Vec::new();
        let mut invalid = false;
        for choice in choices {
            let setup = match choice {
                "1" => prompt_preset_setup("groq").await?,
                "2" => prompt_preset_setup("openrouter").await?,
                "3" => prompt_preset_setup("gemini").await?,
                "4" => prompt_preset_setup("github-models").await?,
                "5" => prompt_preset_setup("nvidia").await?,
                "6" => prompt_preset_setup("ollama").await?,
                "7" => prompt_preset_setup("lm-studio").await?,
                "8" => prompt_preset_setup("openai").await?,
                "9" => {
                    let account_id = prompt_required("Cloudflare account_id")?;
                    let gateway_id = prompt_with_default("Cloudflare gateway_id", "default")?;
                    let api_token_env = prompt_with_default(
                        "API token environment variable",
                        "CLOUDFLARE_API_TOKEN",
                    )?;
                    crate::credentials::prompt_for_credential(&api_token_env)?;
                    let default_model = prompt_required("Default model")?;
                    ProviderSetup::Cloudflare {
                        account_id,
                        gateway_id,
                        api_token_env,
                        default_model,
                    }
                }
                "10" => {
                    let provider_name = prompt_required("Provider name")?;
                    let base_url = prompt_required("Base URL")?;
                    let api_key_env =
                        prompt("API key environment variable (leave blank for none)")?;
                    let api_key_env = non_empty(api_key_env);
                    if let Some(environment_variable) = api_key_env.as_deref() {
                        crate::credentials::prompt_for_credential(environment_variable)?;
                    }
                    let models_url = non_empty(prompt(
                        "Models catalog URL (leave blank to use <base_url>/models)",
                    )?);
                    let default_model = discover_and_choose_model(
                        &provider_name,
                        &base_url,
                        api_key_env.clone(),
                        models_url.clone(),
                        None,
                    )
                    .await?;
                    ProviderSetup::OpenAiCompatible {
                        provider_name,
                        base_url,
                        api_key_env,
                        models_url,
                        default_model,
                    }
                }
                "11" => ProviderSetup::Skip,
                _ => {
                    println!("Enter one or two numbers from 1 through 11.");
                    invalid = true;
                    break;
                }
            };
            providers.push(setup);
        }
        if invalid {
            continue;
        }
        return if providers.len() == 1 {
            Ok(providers.remove(0))
        } else {
            Ok(ProviderSetup::Multiple { providers })
        };
    }
}

async fn prompt_preset_setup(provider: &str) -> Result<ProviderSetup> {
    let preset = provider_preset(provider)
        .ok_or_else(|| anyhow!("provider preset is unavailable: {provider}"))?;
    println!();
    println!("Setting up {}", preset.id);
    println!("{}", preset.setup_note);
    println!("Chat endpoint: {}/chat/completions", preset.base_url);
    println!(
        "Credential: {}",
        preset.api_key_env.unwrap_or("not required")
    );
    println!(
        "Model catalog: {}",
        preset
            .models_url
            .map(str::to_string)
            .unwrap_or_else(|| format!("{}/models", preset.base_url))
    );
    if let Some(environment_variable) = preset.api_key_env {
        crate::credentials::prompt_for_credential(environment_variable)?;
    }
    let default_model = discover_and_choose_model(
        preset.id,
        preset.base_url,
        preset.api_key_env.map(str::to_string),
        preset.models_url.map(str::to_string),
        preset.default_model,
    )
    .await?;
    provider_setup_from_preset(provider, Some(default_model))
}

async fn discover_and_choose_model(
    provider_name: &str,
    base_url: &str,
    api_key_env: Option<String>,
    models_url: Option<String>,
    suggested_model: Option<&str>,
) -> Result<String> {
    println!("Fetching the model catalog (no chat/completion request is made)...");
    let provider = OpenAiCompatibleProvider::new(provider_name, base_url, api_key_env.clone())
        .with_models_url(models_url);
    let provider = match api_key_env.as_deref() {
        Some(environment_variable) => {
            match crate::credentials::resolve_credential(environment_variable) {
                Ok(Some(api_key)) => provider.with_api_key(api_key),
                Ok(None) => provider,
                Err(error) => {
                    println!("Could not read the configured credential: {error}");
                    provider
                }
            }
        }
        None => provider,
    };
    match provider.models().await {
        Ok(models) if !models.is_empty() => choose_model_from_catalog(&models, suggested_model),
        Ok(_) => {
            println!("The provider returned an empty model catalog.");
            prompt_model_without_catalog(suggested_model)
        }
        Err(error) => {
            println!("Could not load models: {error}");
            prompt_model_without_catalog(suggested_model)
        }
    }
}

fn choose_model_from_catalog(
    models: &[ModelInfo],
    suggested_model: Option<&str>,
) -> Result<String> {
    println!("Available models: {}", models.len());
    print_model_matches(models);
    let fallback = suggested_model.or_else(|| models.first().map(|model| model.id.as_str()));
    loop {
        let value = match fallback {
            Some(default) => prompt_with_default("Model ID or search text", default)?,
            None => prompt_required("Model ID or search text")?,
        };
        if models.iter().any(|model| model.id == value) {
            return Ok(value);
        }
        let query = value.to_ascii_lowercase();
        let matches = models
            .iter()
            .filter(|model| model.id.to_ascii_lowercase().contains(&query))
            .cloned()
            .collect::<Vec<_>>();
        match matches.len() {
            0 if confirm(
                &format!("'{value}' is not in the catalog. Use it anyway?"),
                false,
            )? =>
            {
                return Ok(value);
            }
            0 => println!("No matching model. Try another name."),
            1 => {
                println!("Selected {}.", matches[0].id);
                return Ok(matches[0].id.clone());
            }
            _ => print_model_matches(&matches),
        }
    }
}

fn print_model_matches(models: &[ModelInfo]) {
    const PAGE_SIZE: usize = 25;
    for model in models.iter().take(PAGE_SIZE) {
        println!("- {}", model.id);
    }
    if models.len() > PAGE_SIZE {
        println!(
            "... and {} more. Type part of a model name to filter.",
            models.len() - PAGE_SIZE
        );
    }
}

fn prompt_model_without_catalog(suggested_model: Option<&str>) -> Result<String> {
    match suggested_model {
        Some(model) => prompt_with_default("Default model", model),
        None => prompt_required("Default model"),
    }
}

fn provider_setup_from_preset(provider: &str, model: Option<String>) -> Result<ProviderSetup> {
    let preset = provider_preset(provider).ok_or_else(|| {
        anyhow!(
            "unsupported provider: {provider}; choose mock, groq, openrouter, gemini, github-models, nvidia, ollama, lm-studio, openai, openai-compatible, or cloudflare"
        )
    })?;
    let default_model = model
        .or_else(|| preset.default_model.map(str::to_string))
        .ok_or_else(|| anyhow!("--provider {} requires --model", preset.id))?;
    Ok(ProviderSetup::OpenAiCompatible {
        provider_name: preset.id.to_string(),
        base_url: preset.base_url.to_string(),
        api_key_env: preset.api_key_env.map(str::to_string),
        models_url: preset.models_url.map(str::to_string),
        default_model,
    })
}

fn provider_preset(provider: &str) -> Option<ProviderPreset> {
    let provider = match provider {
        "openai-compatible" => "openai",
        "lmstudio" => "lm-studio",
        "nvidia-nim" => "nvidia",
        other => other,
    };
    PROVIDER_PRESETS
        .iter()
        .copied()
        .find(|preset| preset.id == provider)
}

fn non_empty(value: String) -> Option<String> {
    (!value.trim().is_empty()).then_some(value)
}

fn prompt_required(label: &str) -> Result<String> {
    loop {
        let value = prompt(label)?;
        if !value.trim().is_empty() {
            return Ok(value);
        }
        println!("{label} is required.");
    }
}

fn prompt_with_default(label: &str, default: &str) -> Result<String> {
    let value = prompt(&format!("{label} [{default}]"))?;
    if value.trim().is_empty() {
        Ok(default.to_string())
    } else {
        Ok(value)
    }
}

fn confirm(label: &str, default: bool) -> Result<bool> {
    let hint = if default { "Y/n" } else { "y/N" };
    loop {
        let value = prompt(&format!("{label} [{hint}]"))?;
        let trimmed = value.trim().to_ascii_lowercase();
        if trimmed.is_empty() {
            return Ok(default);
        }
        match trimmed.as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("Enter y or n."),
        }
    }
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

pub(crate) fn materialize_embedded_registry(config_path: &Path) -> Result<PathBuf> {
    let config_dir = config_path
        .parent()
        .ok_or_else(|| anyhow!("config path has no parent directory"))?;
    let registry_root = config_dir.join(EMBEDDED_REGISTRY_DIR);
    let completion_marker = registry_root.join(EMBEDDED_REGISTRY_COMPLETE_MARKER);
    let assets_match = || {
        EMBEDDED_REGISTRY_FILES.iter().all(|asset| {
            fs::read(registry_root.join(asset.relative_path))
                .map(|contents| contents == asset.contents)
                .unwrap_or(false)
        })
    };

    if fs::read(&completion_marker)
        .map(|contents| contents == EMBEDDED_REGISTRY_MARKER_CONTENTS.as_bytes())
        .unwrap_or(false)
        && assets_match()
    {
        return Ok(registry_root);
    }

    // A marker is only meaningful if all of this generation's files matched.
    // Remove a stale marker before repair so concurrent callers can never
    // accept an old completion signal while this process fixes assets.
    if let Err(error) = fs::remove_file(&completion_marker) {
        if error.kind() != io::ErrorKind::NotFound {
            return Err(error).with_context(|| {
                format!(
                    "failed to clear incomplete bundled registry marker {}",
                    completion_marker.display()
                )
            });
        }
    }

    for asset in EMBEDDED_REGISTRY_FILES {
        let destination = registry_root.join(asset.relative_path);
        let current_matches = fs::read(&destination)
            .map(|contents| contents == asset.contents)
            .unwrap_or(false);
        if !current_matches {
            atomic_write(&destination, asset.contents).with_context(|| {
                format!(
                    "failed to materialize bundled registry asset {}",
                    destination.display()
                )
            })?;
        }
    }

    // `atomic_write` commits the marker only after every registry resource is
    // present, so readers never treat a partially materialized generation as
    // usable.
    atomic_write(
        &completion_marker,
        EMBEDDED_REGISTRY_MARKER_CONTENTS.as_bytes(),
    )
    .with_context(|| {
        format!(
            "failed to finalize bundled registry marker {}",
            completion_marker.display()
        )
    })?;

    Ok(registry_root)
}

fn is_legacy_fixture_registry_location(location: &str) -> bool {
    let normalized = location.trim().replace('\\', "/");
    if normalized.contains("://") {
        return false;
    }
    let normalized = normalized.trim_end_matches('/').to_ascii_lowercase();
    normalized == "fixtures/skill-registry"
        || normalized.ends_with("/fixtures/skill-registry")
        || normalized.ends_with("/fixtures/skill-registry/registry.json")
}

#[cfg(test)]
pub(crate) fn bundled_registry_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/skill-registry")
}

pub(crate) async fn load_registry_client_from_location(location: &str) -> Result<RegistryClient> {
    let path = PathBuf::from(location);
    if path.exists() {
        Ok(RegistryClient::from_local_path(path)?)
    } else {
        Ok(RegistryClient::from_url(location).await?)
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use axiom_engine::{InstalledSkills, TrustLevel};

    use super::*;

    static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn cloudflare_onboarding_plan_creates_completed_config() {
        let plan = OnboardingPlan {
            workspace: "~/Axiom".to_string(),
            provider: ProviderSetup::Cloudflare {
                account_id: "account".to_string(),
                gateway_id: "default".to_string(),
                api_token_env: "CLOUDFLARE_API_TOKEN".to_string(),
                default_model: "openai/gpt-4.1-mini".to_string(),
            },
        };

        let config = build_config(&plan);

        assert!(config.agent.first_run_completed);
        assert_eq!(config.llm.active_provider.as_deref(), Some("cloudflare"));
        assert_eq!(
            config.llm.active_model.as_deref(),
            Some("openai/gpt-4.1-mini")
        );
        assert!(matches!(
            config.providers.get("cloudflare"),
            Some(ProviderConfig::CloudflareAiGateway { .. })
        ));
    }

    #[test]
    fn skip_provider_onboarding_remains_incomplete_without_active_provider() {
        let plan = OnboardingPlan {
            workspace: "~/Axiom".to_string(),
            provider: ProviderSetup::Skip,
        };

        let config = build_config(&plan);

        assert!(!config.agent.first_run_completed);
        assert!(config.providers.is_empty());
        assert!(config.llm.active_provider.is_none());
        assert!(config.llm.active_model.is_none());
    }

    #[test]
    fn mock_onboarding_plan_creates_mock_provider_config() {
        let plan = OnboardingPlan {
            workspace: "~/Axiom".to_string(),
            provider: ProviderSetup::Mock {
                default_model: "mock-model".to_string(),
            },
        };

        let config = build_config(&plan);

        assert!(config.agent.first_run_completed);
        assert_eq!(config.llm.active_provider.as_deref(), Some("mock"));
        assert_eq!(config.llm.active_model.as_deref(), Some("mock-model"));
        assert!(matches!(
            config.providers.get("mock"),
            Some(ProviderConfig::Mock {})
        ));
    }

    #[test]
    fn multiple_provider_plan_keeps_each_model_and_activates_the_first_choice() {
        let groq = provider_setup_from_preset("groq", Some("groq-model".to_string()))
            .expect("groq preset");
        let ollama = provider_setup_from_preset("ollama", Some("local-model".to_string()))
            .expect("ollama preset");
        let plan = OnboardingPlan {
            workspace: "~/Axiom".to_string(),
            provider: ProviderSetup::Multiple {
                providers: vec![groq, ollama],
            },
        };

        let config = build_config(&plan);

        assert_eq!(config.llm.active_provider.as_deref(), Some("groq"));
        assert_eq!(config.llm.active_model.as_deref(), Some("groq-model"));
        assert_eq!(config.providers.len(), 2);
        assert_eq!(
            config.llm.provider_models.get("groq").map(String::as_str),
            Some("groq-model")
        );
        assert_eq!(
            config.llm.provider_models.get("ollama").map(String::as_str),
            Some("local-model")
        );
    }

    #[test]
    fn hosted_presets_use_official_endpoints_keys_and_defaults() {
        let cases = [
            (
                "groq",
                "https://api.groq.com/openai/v1",
                "GROQ_API_KEY",
                "llama-3.3-70b-versatile",
            ),
            (
                "openrouter",
                "https://openrouter.ai/api/v1",
                "OPENROUTER_API_KEY",
                "openrouter/free",
            ),
            (
                "gemini",
                "https://generativelanguage.googleapis.com/v1beta/openai",
                "GEMINI_API_KEY",
                "gemini-2.5-flash",
            ),
            (
                "github-models",
                "https://models.github.ai/inference",
                "GITHUB_TOKEN",
                "openai/gpt-4.1",
            ),
            (
                "nvidia",
                "https://integrate.api.nvidia.com/v1",
                "NVIDIA_API_KEY",
                "meta/llama-3.3-70b-instruct",
            ),
        ];

        for (provider, base_url, api_key_env, model) in cases {
            let setup = provider_setup_from_preset(provider, None).expect("provider preset");
            assert_eq!(
                setup,
                ProviderSetup::OpenAiCompatible {
                    provider_name: provider.to_string(),
                    base_url: base_url.to_string(),
                    api_key_env: Some(api_key_env.to_string()),
                    models_url: provider_preset(provider)
                        .and_then(|preset| preset.models_url)
                        .map(str::to_string),
                    default_model: model.to_string(),
                }
            );
        }
    }

    #[test]
    fn nvidia_nim_alias_uses_the_canonical_provider_name() {
        let setup = provider_setup_from_preset("nvidia-nim", None).expect("NVIDIA NIM alias");

        assert!(matches!(
            setup,
            ProviderSetup::OpenAiCompatible {
                ref provider_name,
                ref base_url,
                ..
            } if provider_name == "nvidia" && base_url == "https://integrate.api.nvidia.com/v1"
        ));
    }

    #[test]
    fn local_presets_do_not_require_fake_api_keys() {
        let ollama = provider_setup_from_preset("ollama", None).expect("ollama preset");
        assert!(matches!(
            ollama,
            ProviderSetup::OpenAiCompatible {
                api_key_env: None,
                ..
            }
        ));

        let lm_studio = provider_setup_from_preset("lmstudio", Some("local-model".to_string()))
            .expect("LM Studio alias");
        assert!(matches!(
            lm_studio,
            ProviderSetup::OpenAiCompatible {
                ref provider_name,
                api_key_env: None,
                ..
            } if provider_name == "lm-studio"
        ));
    }

    #[test]
    fn presets_without_safe_model_defaults_require_model_selection() {
        let error = provider_setup_from_preset("openai", None)
            .expect_err("OpenAI should require an explicit model");

        assert!(error.to_string().contains("requires --model"));
    }

    #[tokio::test]
    async fn embedded_registry_materializes_every_index_resource_under_config_home() {
        let dir = unique_temp_dir();
        let config_path = dir.join("config.toml");
        let registry_root =
            materialize_embedded_registry(&config_path).expect("materialize embedded registry");
        let client = RegistryClient::from_local_path(&registry_root).expect("load registry");

        assert_eq!(registry_root, dir.join(EMBEDDED_REGISTRY_DIR));
        assert!(
            registry_root
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(env!("CARGO_PKG_VERSION"))),
            "registry generation should include the executable version"
        );
        assert_eq!(
            fs::read(registry_root.join(EMBEDDED_REGISTRY_COMPLETE_MARKER))
                .expect("read completion marker"),
            EMBEDDED_REGISTRY_MARKER_CONTENTS.as_bytes()
        );
        for entry in client.list_skills() {
            client
                .fetch_skill_manifest(&entry.id)
                .await
                .unwrap_or_else(|error| panic!("embedded skill {}: {error}", entry.id));
        }
        for entry in client.list_bundles() {
            client
                .fetch_bundle(&entry.id)
                .await
                .unwrap_or_else(|error| panic!("embedded bundle {}: {error}", entry.id));
        }

        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn onboarding_plan_creates_config_workspace_and_starter_skills() {
        let dir = unique_temp_dir();
        let config_path = dir.join("config.toml");
        let workspace = dir.join("workspace").to_string_lossy().to_string();
        let plan = OnboardingPlan {
            workspace,
            provider: ProviderSetup::OpenAiCompatible {
                provider_name: "local".to_string(),
                base_url: "http://localhost:8000/v1".to_string(),
                api_key_env: Some("LOCAL_LLM_API_KEY".to_string()),
                models_url: None,
                default_model: "local-model".to_string(),
            },
        };

        let result = apply_onboarding_plan_with_registry_url(
            &config_path,
            &plan,
            Some(bundled_registry_path().display().to_string()),
            bundled_registry_path(),
            std::env::consts::OS,
        )
        .await
        .expect("apply onboarding");
        let saved = AxiomConfig::load_from_path(&config_path).expect("load saved config");

        assert!(saved.agent.first_run_completed);
        assert_eq!(
            saved.skills.registry_url,
            bundled_registry_path().display().to_string(),
            "an explicit registry override should remain explicit"
        );
        assert!(result.workspace_path.exists());
        assert_eq!(result.installed_skills.len(), 7);
        assert_eq!(result.registry_source, "bundled");
        assert!(dir
            .join("skills")
            .join("file.read")
            .join("skill.toml")
            .exists());

        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn falls_back_to_bundled_registry_when_primary_local_registry_fails() {
        let dir = unique_temp_dir();
        let skills_dir = dir.join("skills");

        let result = install_essential_skills_for_os(
            &skills_dir,
            std::env::consts::OS,
            &dir.join("missing-registry").display().to_string(),
            true,
            bundled_registry_path(),
        )
        .await
        .expect("fallback install");

        assert_eq!(result.source, "bundled");
        assert!(result
            .installed_skill_ids
            .contains(&"file.read".to_string()));
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn onboarding_preserves_existing_skills_registry_settings() {
        let dir = unique_temp_dir();
        let config_path = dir.join("config.toml");
        let fallback_registry_root =
            materialize_embedded_registry(&config_path).expect("materialize fallback registry");
        let custom_registry_root = dir.join("custom-registry");
        write_embedded_registry_at(&custom_registry_root).expect("write custom registry");
        let custom_skills_dir = "custom-skills";
        let mut existing = AxiomConfig::default();
        existing.skills.registry_url = custom_registry_root.display().to_string();
        existing.skills.local_dir = custom_skills_dir.to_string();
        existing.ui.color = false;
        existing.agent.max_iterations = 99;
        existing
            .save_to_path(&config_path)
            .expect("save existing config");
        let plan = OnboardingPlan {
            workspace: dir.join("workspace").to_string_lossy().to_string(),
            provider: ProviderSetup::Skip,
        };

        apply_onboarding_plan_with_registry_url(
            &config_path,
            &plan,
            None,
            &fallback_registry_root,
            std::env::consts::OS,
        )
        .await
        .expect("apply onboarding");
        let saved = AxiomConfig::load_from_path(&config_path).expect("load saved config");

        assert_eq!(
            saved.skills.registry_url,
            custom_registry_root.display().to_string()
        );
        assert_eq!(saved.skills.local_dir, custom_skills_dir);
        assert!(!saved.ui.color);
        assert_eq!(saved.agent.max_iterations, 99);
        assert!(dir
            .join(custom_skills_dir)
            .join("file.read")
            .join("skill.toml")
            .exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn onboarding_self_heals_legacy_fixture_registry_without_overwriting_custom_choice() {
        let dir = unique_temp_dir();
        let config_path = dir.join("config.toml");
        let fallback_registry_root =
            materialize_embedded_registry(&config_path).expect("materialize fallback registry");
        let mut existing = build_config(&OnboardingPlan {
            workspace: dir.join("old-workspace").to_string_lossy().to_string(),
            provider: ProviderSetup::Mock {
                default_model: "saved-model".to_string(),
            },
        });
        existing.skills.registry_url = bundled_registry_path().display().to_string();
        existing
            .save_to_path(&config_path)
            .expect("save old fixture config");
        let plan = OnboardingPlan {
            workspace: dir.join("workspace").to_string_lossy().to_string(),
            provider: ProviderSetup::Skip,
        };

        let result = apply_onboarding_plan_with_registry_url(
            &config_path,
            &plan,
            None,
            &fallback_registry_root,
            std::env::consts::OS,
        )
        .await
        .expect("apply onboarding");
        let saved = AxiomConfig::load_from_path(&config_path).expect("load saved config");

        assert_eq!(
            saved.skills.registry_url,
            AxiomConfig::default().skills.registry_url,
            "a checkout-only fixture path should be repaired"
        );
        assert_eq!(result.registry_source, "bundled");
        assert_eq!(saved.llm.active_provider.as_deref(), Some("mock"));
        assert_eq!(saved.llm.active_model.as_deref(), Some("saved-model"));

        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn onboarding_skip_preserves_an_existing_provider_and_completion_state() {
        let dir = unique_temp_dir();
        let config_path = dir.join("config.toml");
        let fallback_registry_root =
            materialize_embedded_registry(&config_path).expect("materialize fallback registry");
        let existing = build_config(&OnboardingPlan {
            workspace: dir.join("old-workspace").to_string_lossy().to_string(),
            provider: ProviderSetup::Mock {
                default_model: "saved-model".to_string(),
            },
        });
        let expected_providers = existing.providers.clone();
        let expected_provider_models = existing.llm.provider_models.clone();
        existing
            .save_to_path(&config_path)
            .expect("save existing provider config");
        let plan = OnboardingPlan {
            workspace: dir.join("new-workspace").to_string_lossy().to_string(),
            provider: ProviderSetup::Skip,
        };

        apply_onboarding_plan_with_registry_url(
            &config_path,
            &plan,
            None,
            &fallback_registry_root,
            std::env::consts::OS,
        )
        .await
        .expect("apply onboarding");
        let saved = AxiomConfig::load_from_path(&config_path).expect("load saved config");

        assert_eq!(saved.providers, expected_providers);
        assert_eq!(saved.llm.provider_models, expected_provider_models);
        assert_eq!(saved.llm.active_provider.as_deref(), Some("mock"));
        assert_eq!(saved.llm.active_model.as_deref(), Some("saved-model"));
        assert!(saved.agent.first_run_completed);
        assert_eq!(
            saved.agent.default_workspace,
            dir.join("new-workspace").to_string_lossy()
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn onboarding_migrates_and_preserves_existing_security_controls() {
        let dir = unique_temp_dir();
        let config_path = dir.join("config.toml");
        let mut existing = AxiomConfig {
            config_version: 0,
            ..AxiomConfig::default()
        };
        existing.policy.filesystem_read = "deny".to_string();
        existing.policy.filesystem_write = "deny".to_string();
        existing.policy.network = "deny".to_string();
        existing.network.web_fetch_https_only = false;
        existing.network.web_fetch_allowed_hosts = vec!["api.example.test".to_string()];
        existing.network.web_fetch_denied_hosts = vec!["blocked.example.test".to_string()];
        existing.skills.registry_url = bundled_registry_path().display().to_string();
        existing
            .save_to_path(&config_path)
            .expect("save legacy config");
        let plan = OnboardingPlan {
            workspace: dir.join("workspace").to_string_lossy().to_string(),
            provider: ProviderSetup::Skip,
        };

        apply_onboarding_plan_with_registry_url(
            &config_path,
            &plan,
            None,
            bundled_registry_path(),
            std::env::consts::OS,
        )
        .await
        .expect("apply onboarding");
        let saved = AxiomConfig::load_from_path(&config_path).expect("load saved config");

        assert_eq!(saved.config_version, axiom_core::CURRENT_CONFIG_VERSION);
        assert_eq!(saved.policy.filesystem_read, "deny");
        assert_eq!(saved.policy.filesystem_write, "deny");
        assert_eq!(saved.policy.network, "deny");
        assert!(!saved.network.web_fetch_https_only);
        assert_eq!(
            saved.network.web_fetch_allowed_hosts,
            vec!["api.example.test"]
        );
        assert_eq!(
            saved.network.web_fetch_denied_hosts,
            vec!["blocked.example.test"]
        );
        assert!(dir.join("config.toml.v0.bak").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn non_interactive_onboarding_uses_axiom_home_and_installs_essential_skills() {
        let dir = unique_temp_dir();
        let home = dir.join("home");
        let workspace = dir.join("workspace");
        let _guard = EnvVarGuard::set("AXIOM_HOME", home.as_os_str().to_os_string());

        let result = run_non_interactive_onboarding(OnboardingCommand {
            non_interactive: true,
            workspace: Some(workspace.display().to_string()),
            provider: Some("mock".to_string()),
            model: None,
            account_id: None,
            registry: None,
            skip_provider: false,
            yes: true,
        })
        .await
        .expect("non-interactive onboarding");
        let saved = AxiomConfig::load_from_path(home.join("config.toml")).expect("load config");

        assert_eq!(result.config_path, home.join("config.toml"));
        assert!(workspace.exists());
        assert_eq!(saved.llm.active_provider.as_deref(), Some("mock"));
        assert!(result.installed_skills.contains(&"file.read".to_string()));
        assert!(home.join("skills").join("installed_skills.json").exists());
        assert!(home
            .join(EMBEDDED_REGISTRY_DIR)
            .join("registry.json")
            .exists());
        assert_eq!(
            fs::read(
                home.join(EMBEDDED_REGISTRY_DIR)
                    .join(EMBEDDED_REGISTRY_COMPLETE_MARKER)
            )
            .expect("read completion marker"),
            EMBEDDED_REGISTRY_MARKER_CONTENTS.as_bytes()
        );
        assert_eq!(
            saved.skills.registry_url,
            AxiomConfig::default().skills.registry_url,
            "the materialized fallback must not become the configured registry"
        );
        assert!(!saved.skills.registry_url.contains("bundled-registry"));
        assert!(!saved.skills.registry_url.contains("fixtures"));
        let installed = InstalledSkills::load_from_dir(home.join("skills"))
            .expect("load installed skill records");
        assert!(!installed.skills.is_empty());
        assert!(installed.skills.values().all(|record| {
            record.source == "bundled" && record.trust_level == TrustLevel::Trusted
        }));

        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn non_interactive_onboarding_requires_required_flags() {
        let error = run_non_interactive_onboarding(OnboardingCommand {
            non_interactive: true,
            workspace: None,
            provider: Some("mock".to_string()),
            model: None,
            account_id: None,
            registry: Some(bundled_registry_path().display().to_string()),
            skip_provider: false,
            yes: false,
        })
        .await
        .expect_err("missing --yes should fail");

        assert!(error.to_string().contains("--yes"));
    }

    #[tokio::test]
    async fn non_interactive_cloudflare_requires_explicit_account_id() {
        let error = run_non_interactive_onboarding(OnboardingCommand {
            non_interactive: true,
            workspace: Some("~/Axiom".to_string()),
            provider: Some("cloudflare".to_string()),
            model: Some("model".to_string()),
            account_id: None,
            registry: None,
            skip_provider: false,
            yes: true,
        })
        .await
        .expect_err("missing Cloudflare account ID should fail");

        assert!(error
            .to_string()
            .contains("requires an explicit --account-id"));
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let counter = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "axiom-cli-onboarding-test-{}-{nanos}-{counter}",
            std::process::id()
        ))
    }

    fn write_embedded_registry_at(root: &Path) -> Result<()> {
        for asset in EMBEDDED_REGISTRY_FILES {
            atomic_write(root.join(asset.relative_path), asset.contents)?;
        }
        Ok(())
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
