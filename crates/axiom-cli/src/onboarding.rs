use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Context, Result};
use axiom_core::{AxiomConfig, ProviderConfig};
use axiom_engine::{
    essential_bundle_id_for_os, install_bundle_from_registry_client, LocalRegistrySource,
    RegistryClient, RegistrySource,
};

const DEFAULT_WORKSPACE: &str = "~/Axiom";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OnboardingPlan {
    pub workspace: String,
    pub provider: ProviderSetup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProviderSetup {
    Cloudflare {
        account_id: String,
        gateway_id: String,
        api_token_env: String,
        default_model: String,
    },
    OpenAiCompatible {
        provider_name: String,
        base_url: String,
        api_key_env: String,
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
    let config_exists = config_path.exists();

    println!("Axiom Agent onboarding");
    println!("config: {}", config_path.display());

    if config_exists {
        let existing = AxiomConfig::load_from_path(&config_path)?;
        println!("Existing config found.");
        println!("This will update your existing Axiom setup if you continue.");
        if !confirm("Update existing setup?", false)? {
            println!("Onboarding skipped. Existing config was left unchanged.");
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
        println!("Created config directory: {}", parent.display());
    }

    let plan = prompt_for_plan()?;
    let result = apply_onboarding_plan(&config_path, &plan).await?;

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
    apply_onboarding_plan_with_registry_url(
        config_path,
        plan,
        None,
        bundled_registry_path(),
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

    let existing_skills = if config_path.exists() {
        Some(AxiomConfig::load_from_path(config_path)?.skills)
    } else {
        None
    };
    let mut config = build_config(plan);
    if let Some(skills) = existing_skills {
        config.skills = skills;
    } else {
        config.skills.local_dir = "skills".to_string();
    }

    let workspace_path = config.default_workspace_path();
    fs::create_dir_all(&workspace_path).with_context(|| {
        format!(
            "failed to create workspace directory {}",
            workspace_path.display()
        )
    })?;

    let skills_dir = config_dir.join(&config.skills.local_dir);
    println!("Installing essential skills for {os}...");
    let registry_url = registry_url_override.unwrap_or_else(|| config.skills.registry_url.clone());
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
    match load_registry_client_from_location(registry_location).await {
        Ok(client) => {
            println!("Registry: {}", client.source_label());
            let records = install_bundle_from_registry_client(
                &client,
                bundle_id,
                skills_dir,
                client.source_label(),
            )
            .await?;
            for record in &records {
                println!("installed: {}", record.id);
            }
            Ok(EssentialSkillInstallResult {
                source: client.source_label().to_string(),
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

pub(crate) fn build_config(plan: &OnboardingPlan) -> AxiomConfig {
    let mut config = AxiomConfig::default();
    config.agent.default_workspace = plan.workspace.clone();
    config.agent.first_run_completed = true;
    config.providers.clear();
    config.llm.active_provider = None;
    config.llm.active_model = None;

    match &plan.provider {
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
        }
        ProviderSetup::OpenAiCompatible {
            provider_name,
            base_url,
            api_key_env,
            default_model,
        } => {
            config.providers.insert(
                provider_name.clone(),
                ProviderConfig::OpenaiCompatible {
                    base_url: base_url.clone(),
                    api_key_env: api_key_env.clone(),
                },
            );
            config.llm.active_provider = Some(provider_name.clone());
            config.llm.active_model = Some(default_model.clone());
        }
        ProviderSetup::Skip => {}
    }

    config
}

fn prompt_for_plan() -> Result<OnboardingPlan> {
    let workspace = prompt_with_default("Workspace path", DEFAULT_WORKSPACE)?;
    let provider = prompt_provider_setup()?;

    Ok(OnboardingPlan {
        workspace,
        provider,
    })
}

fn prompt_provider_setup() -> Result<ProviderSetup> {
    println!();
    println!("Provider setup:");
    println!("1) Cloudflare AI Gateway");
    println!("2) OpenAI compatible");
    println!("3) Local OpenAI compatible endpoint");
    println!("4) Skip for now");

    loop {
        match prompt_with_default("Choose provider setup", "4")?.as_str() {
            "1" => {
                let account_id = prompt_required("Cloudflare account_id")?;
                let gateway_id = prompt_with_default("Cloudflare gateway_id", "default")?;
                let api_token_env =
                    prompt_with_default("API token environment variable", "CLOUDFLARE_API_TOKEN")?;
                let default_model = prompt_required("Default model")?;
                return Ok(ProviderSetup::Cloudflare {
                    account_id,
                    gateway_id,
                    api_token_env,
                    default_model,
                });
            }
            "2" => {
                let provider_name = prompt_required("Provider name")?;
                let base_url = prompt_required("Base URL")?;
                let api_key_env = prompt_required("API key environment variable")?;
                let default_model = prompt_required("Default model")?;
                return Ok(ProviderSetup::OpenAiCompatible {
                    provider_name,
                    base_url,
                    api_key_env,
                    default_model,
                });
            }
            "3" => {
                let provider_name = prompt_with_default("Provider name", "local")?;
                let base_url = prompt_with_default("Base URL", "http://localhost:8000/v1")?;
                let api_key_env =
                    prompt_with_default("API key environment variable", "LOCAL_LLM_API_KEY")?;
                let default_model = prompt_required("Default model")?;
                return Ok(ProviderSetup::OpenAiCompatible {
                    provider_name,
                    base_url,
                    api_key_env,
                    default_model,
                });
            }
            "4" => return Ok(ProviderSetup::Skip),
            _ => println!("Enter 1, 2, 3, or 4."),
        }
    }
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
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

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
    fn skip_provider_onboarding_creates_config_without_active_provider() {
        let plan = OnboardingPlan {
            workspace: "~/Axiom".to_string(),
            provider: ProviderSetup::Skip,
        };

        let config = build_config(&plan);

        assert!(config.agent.first_run_completed);
        assert!(config.providers.is_empty());
        assert!(config.llm.active_provider.is_none());
        assert!(config.llm.active_model.is_none());
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
                api_key_env: "LOCAL_LLM_API_KEY".to_string(),
                default_model: "local-model".to_string(),
            },
        };

        let result = apply_onboarding_plan_with_registry_url(
            &config_path,
            &plan,
            Some(bundled_registry_path().display().to_string()),
            bundled_registry_path(),
            "windows",
        )
        .await
        .expect("apply onboarding");
        let saved = AxiomConfig::load_from_path(&config_path).expect("load saved config");

        assert!(saved.agent.first_run_completed);
        assert!(result.workspace_path.exists());
        assert_eq!(result.installed_skills.len(), 7);
        assert_eq!(result.registry_source, "local");
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
            "windows",
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
        let custom_skills_dir = "custom-skills";
        let mut existing = AxiomConfig::default();
        existing.skills.registry_url = bundled_registry_path().display().to_string();
        existing.skills.local_dir = custom_skills_dir.to_string();
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
            bundled_registry_path(),
            "windows",
        )
        .await
        .expect("apply onboarding");
        let saved = AxiomConfig::load_from_path(&config_path).expect("load saved config");

        assert_eq!(
            saved.skills.registry_url,
            bundled_registry_path().display().to_string()
        );
        assert_eq!(saved.skills.local_dir, custom_skills_dir);
        assert!(dir
            .join(custom_skills_dir)
            .join("file.read")
            .join("skill.toml")
            .exists());
        let _ = fs::remove_dir_all(dir);
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
}
