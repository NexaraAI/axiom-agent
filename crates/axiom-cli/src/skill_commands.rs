use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Result};
use axiom_core::AxiomConfig;
use axiom_engine::{
    check_skill_updates, execute_installed_tool, install_bundle_from_registry_client,
    install_skill_from_registry_client, ApprovalRequest, InstalledSkills, RegistryClient,
    RegistrySource, SkillApproval, SkillExecutionContext, SkillExecutionResult, SkillManifest,
    ToolRequest,
};
use axiom_proof::{
    new_approval, new_tool_call, FileReadProof, FileWriteProof, ProofMode, ProofRecorder,
};
use serde_json::Value;

use crate::{chat, onboarding, SkillCommands, SkillRegistryCommands};

pub(crate) async fn run(command: SkillCommands) -> Result<()> {
    match command {
        SkillCommands::Registry { command } => registry_command(command).await,
        SkillCommands::List => list_available_skills().await,
        SkillCommands::Search { query } => search_skills(&query).await,
        SkillCommands::Installed => list_installed_skills(),
        SkillCommands::Bundles => list_bundles().await,
        SkillCommands::Info { skill_id } => show_skill_info(&skill_id).await,
        SkillCommands::Run { skill_id, args } => run_skill(&skill_id, args.as_deref()).await,
        SkillCommands::Install {
            skill_id,
            registry,
            from_local_registry,
        } => {
            install_skill(
                &skill_id,
                registry.as_deref(),
                from_local_registry.as_deref(),
            )
            .await
        }
        SkillCommands::InstallBundle {
            bundle_id,
            registry,
            from_local_registry,
        } => {
            install_bundle(
                &bundle_id,
                registry.as_deref(),
                from_local_registry.as_deref(),
            )
            .await
        }
        SkillCommands::Update { check } => check_updates(check).await,
    }
}

async fn registry_command(command: SkillRegistryCommands) -> Result<()> {
    match command {
        SkillRegistryCommands::Current => {
            let (_config_path, config) = load_config()?;
            println!("{}", config.skills.registry_url);
            Ok(())
        }
        SkillRegistryCommands::Set { url } => {
            let (config_path, mut config) = load_config()?;
            warn_if_custom_registry(&url);
            config.skills.registry_url = url;
            config.save_to_path(config_path)?;
            println!("Registry updated.");
            Ok(())
        }
        SkillRegistryCommands::Refresh => {
            let (_config_path, config) = load_config()?;
            let selection =
                load_registry_selection(&config, RegistryCommandSource::Configured).await?;
            println!("Registry: {}", selection.source_label);
            println!("Location: {}", selection.location);
            println!("Skills: {}", selection.client.list_skills().len());
            println!("Bundles: {}", selection.client.list_bundles().len());
            Ok(())
        }
    }
}

async fn list_available_skills() -> Result<()> {
    let (_config_path, config) = load_config()?;
    let selection = load_registry_selection(&config, RegistryCommandSource::Configured).await?;

    println!("Available skills from {} registry:", selection.source_label);
    for entry in selection.client.list_skills() {
        println!("- {} {} ({})", entry.id, entry.version, entry.category);
    }

    Ok(())
}

async fn search_skills(query: &str) -> Result<()> {
    let (_config_path, config) = load_config()?;
    let selection = load_registry_selection(&config, RegistryCommandSource::Configured).await?;
    let matches = selection.client.search_skills(query);

    if matches.is_empty() {
        println!("No skills matched `{query}`.");
        return Ok(());
    }

    println!("Matching skills from {} registry:", selection.source_label);
    for entry in matches {
        println!("- {} {} ({})", entry.id, entry.version, entry.category);
    }

    Ok(())
}

fn list_installed_skills() -> Result<()> {
    let (config_path, config) = load_config()?;
    let installed = InstalledSkills::load_from_dir(skills_dir(&config_path, &config))?;

    if installed.skills.is_empty() {
        println!("No skills installed.");
        return Ok(());
    }

    println!("Installed skills:");
    for record in installed.skills.values() {
        let enabled = if record.enabled {
            "enabled"
        } else {
            "disabled"
        };
        println!(
            "- {} {} ({enabled}, source: {})",
            record.id, record.version, record.source
        );
    }

    Ok(())
}

async fn list_bundles() -> Result<()> {
    let (_config_path, config) = load_config()?;
    let selection = load_registry_selection(&config, RegistryCommandSource::Configured).await?;

    println!(
        "Available bundles from {} registry:",
        selection.source_label
    );
    for bundle in selection.client.list_bundles() {
        println!(
            "- {} ({}, platform: {})",
            bundle.id, bundle.name, bundle.platform
        );
    }

    Ok(())
}

async fn show_skill_info(skill_id: &str) -> Result<()> {
    let (config_path, config) = load_config()?;
    let skills_dir = skills_dir(&config_path, &config);
    let installed_manifest = skills_dir.join(skill_id).join("skill.toml");
    let manifest = if installed_manifest.exists() {
        SkillManifest::from_path(installed_manifest)?
    } else {
        let selection = load_registry_selection(&config, RegistryCommandSource::Configured).await?;
        selection
            .client
            .fetch_skill_manifest(skill_id)
            .await
            .map(|(manifest, _resource)| manifest)
            .map_err(|_| anyhow!("skill is not installed or available: {skill_id}"))?
    };
    let card = manifest.to_skill_card();

    println!("{} ({})", manifest.name, manifest.id);
    println!("version: {}", manifest.version);
    println!("category: {}", manifest.category);
    println!("type: {:?}", manifest.skill_type);
    println!("risk: {}", manifest.risk_level);
    println!("entrypoint: {}", manifest.entrypoint);
    println!("summary: {}", card.summary);
    println!("input: {}", card.input_contract);
    println!("output: {}", card.output_contract);

    Ok(())
}

async fn run_skill(skill_id: &str, args: Option<&str>) -> Result<()> {
    let (config_path, config) = load_config()?;
    let arguments = match args {
        Some(raw) => serde_json::from_str::<Value>(raw)
            .map_err(|error| anyhow!("failed to parse --args as JSON: {error}"))?,
        None => Value::Object(Default::default()),
    };
    let mut proof = ProofRecorder::start_trace(
        crate::proof_commands::settings_from_config(&config_path, &config),
        ProofMode::Skill,
        format!("axiom skill run {skill_id}"),
        config.llm.active_provider.clone(),
        config.llm.active_model.clone(),
        Some(config.default_workspace_path().display().to_string()),
    );
    let installed = axiom_engine::load_installed_skills(skills_dir(&config_path, &config))?;
    let request = ToolRequest {
        skill_id: skill_id.to_string(),
        arguments,
    };
    let context = SkillExecutionContext {
        workspace_root: config.default_workspace_path(),
        max_file_read_bytes: config.coder.max_file_read_bytes,
        web_timeout_secs: 20,
        max_web_response_bytes: 1_000_000,
        auto_approve_medium_risk: config.coder.approval_mode == "trusted",
    };
    let mut tool_call = new_tool_call(&request.skill_id, request.arguments.to_string());
    if let Some(skill) = installed.iter().find(|skill| skill.manifest.id == skill_id) {
        tool_call.risk_level = Some(skill.manifest.risk_level.to_string());
    }

    let result = {
        let mut approval = ProofSkillApprover { proof: &mut proof };
        execute_installed_tool(&request, &installed, &context, &mut approval).await
    };

    match result {
        Ok(result) => {
            tool_call.success = true;
            tool_call.ended_at = Some(axiom_proof::trace::now_timestamp());
            tool_call.output_summary = Some(result.output.to_string());
            tool_call.permission_result = Some("approved".to_string());
            record_skill_output_files(&mut proof, &result);
            proof.record_tool_call(tool_call);
            proof.set_final_response(result.output.to_string());
            proof.finish_trace(format!("skill `{skill_id}` executed"));
            let _ = proof.export();
            println!("{}", serde_json::to_string_pretty(&result.output)?);
            Ok(())
        }
        Err(error) => {
            tool_call.error = Some(error.to_string());
            tool_call.ended_at = Some(axiom_proof::trace::now_timestamp());
            tool_call.permission_result = Some("denied or failed".to_string());
            proof.record_tool_call(tool_call);
            proof.record_error("skill", error.to_string(), "skill_run", true);
            proof.fail_trace(format!("skill `{skill_id}` failed"), "skill_run");
            let _ = proof.export();
            Err(error.into())
        }
    }
}

async fn install_skill(
    skill_id: &str,
    registry: Option<&str>,
    from_local_registry: Option<&Path>,
) -> Result<()> {
    let (config_path, config) = load_config()?;
    let selection = load_registry_selection(
        &config,
        RegistryCommandSource::from_args(registry, from_local_registry)?,
    )
    .await?;
    let entry = selection
        .client
        .index()
        .skill_entry(skill_id)
        .ok_or_else(|| anyhow!("skill is not in registry: {skill_id}"))?;
    confirm_custom_registry_install(&config, &selection, &entry.id, entry.sha256.is_some())?;

    let record = install_skill_from_registry_client(
        &selection.client,
        skill_id,
        skills_dir(&config_path, &config),
        &selection.source_label,
    )
    .await?;

    println!(
        "Installed {} {} from {} registry.",
        record.id, record.version, record.source
    );
    if !record.enabled {
        println!("Skill installed disabled because its entrypoint is not supported yet.");
    }
    Ok(())
}

async fn install_bundle(
    bundle_id: &str,
    registry: Option<&str>,
    from_local_registry: Option<&Path>,
) -> Result<()> {
    let (config_path, config) = load_config()?;
    let selection = load_registry_selection(
        &config,
        RegistryCommandSource::from_args(registry, from_local_registry)?,
    )
    .await?;
    let entry = selection
        .client
        .index()
        .bundle_entry(bundle_id)
        .ok_or_else(|| anyhow!("bundle is not in registry: {bundle_id}"))?;
    confirm_custom_registry_install(&config, &selection, &entry.id, entry.sha256.is_some())?;

    let records = install_bundle_from_registry_client(
        &selection.client,
        bundle_id,
        skills_dir(&config_path, &config),
        &selection.source_label,
    )
    .await?;

    println!(
        "Installed bundle {bundle_id} from {} registry:",
        selection.source_label
    );
    for record in records {
        let status = if record.enabled {
            "enabled"
        } else {
            "disabled"
        };
        println!("- {} {} ({status})", record.id, record.version);
    }
    Ok(())
}

async fn check_updates(check: bool) -> Result<()> {
    if !check {
        bail!("use `axiom skill update --check` to check for available skill updates");
    }

    let (config_path, config) = load_config()?;
    let installed = InstalledSkills::load_from_dir(skills_dir(&config_path, &config))?;
    if installed.skills.is_empty() {
        println!("No skills installed.");
        return Ok(());
    }

    let selection = load_registry_selection(&config, RegistryCommandSource::Configured).await?;
    let updates = check_skill_updates(&installed, selection.client.index());
    if updates.is_empty() {
        println!("No skill updates available.");
        return Ok(());
    }

    println!("Available skill updates:");
    for update in updates {
        println!(
            "- {} {} -> {}",
            update.id, update.installed_version, update.registry_version
        );
    }
    println!("Automatic skill update installation is not implemented yet.");
    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum RegistryCommandSource<'a> {
    Configured,
    Override(&'a str),
    LocalPath(&'a Path),
}

impl<'a> RegistryCommandSource<'a> {
    fn from_args(registry: Option<&'a str>, from_local_registry: Option<&'a Path>) -> Result<Self> {
        match (registry, from_local_registry) {
            (Some(_), Some(_)) => bail!("use either --registry or --from-local-registry, not both"),
            (Some(location), None) => Ok(Self::Override(location)),
            (None, Some(path)) => Ok(Self::LocalPath(path)),
            (None, None) => Ok(Self::Configured),
        }
    }
}

struct RegistrySelection {
    client: RegistryClient,
    source_label: String,
    location: String,
    custom_remote: bool,
}

async fn load_registry_selection(
    config: &AxiomConfig,
    source: RegistryCommandSource<'_>,
) -> Result<RegistrySelection> {
    match source {
        RegistryCommandSource::Configured => {
            match onboarding::load_registry_client_from_location(&config.skills.registry_url).await
            {
                Ok(client) => Ok(selection_for_client(
                    client,
                    None,
                    &config.skills.registry_url,
                    config,
                )),
                Err(error) if config.skills.fallback_to_bundled_registry => {
                    println!("Configured registry unavailable. Using bundled fallback registry. ({error})");
                    let location = onboarding::bundled_registry_path();
                    let client = RegistryClient::from_local_path(&location)?;
                    Ok(RegistrySelection {
                        client,
                        source_label: "bundled".to_string(),
                        location: location.display().to_string(),
                        custom_remote: false,
                    })
                }
                Err(error) => Err(error),
            }
        }
        RegistryCommandSource::Override(location) => {
            let client = onboarding::load_registry_client_from_location(location).await?;
            Ok(selection_for_client(client, None, location, config))
        }
        RegistryCommandSource::LocalPath(path) => {
            let client = RegistryClient::from_local_path(path)?;
            Ok(RegistrySelection {
                client,
                source_label: "local".to_string(),
                location: path.display().to_string(),
                custom_remote: false,
            })
        }
    }
}

fn selection_for_client(
    client: RegistryClient,
    source_label: Option<&str>,
    requested_location: &str,
    config: &AxiomConfig,
) -> RegistrySelection {
    let source_label = source_label.unwrap_or(client.source_label()).to_string();
    let is_local = matches!(client.source(), RegistrySource::Local(_));
    let custom_remote = !is_local
        && requested_location != AxiomConfig::default().skills.registry_url
        && !config.skills.allow_untrusted_registries;
    let location = client.registry_location();

    RegistrySelection {
        client,
        source_label,
        location,
        custom_remote,
    }
}

fn confirm_custom_registry_install(
    config: &AxiomConfig,
    selection: &RegistrySelection,
    item_id: &str,
    has_checksum: bool,
) -> Result<()> {
    let custom_location = selection.location != AxiomConfig::default().skills.registry_url
        && !matches!(selection.client.source(), RegistrySource::Local(_))
        && selection.source_label != "bundled";

    if !custom_location {
        return Ok(());
    }

    println!("Custom registries can change agent behavior. Only use registries you trust.");
    if !has_checksum {
        println!("Registry entry `{item_id}` does not declare a sha256 checksum.");
    }

    if !selection.custom_remote || config.skills.allow_untrusted_registries {
        return Ok(());
    }

    if chat::confirm("Install from this custom registry?", false)? {
        Ok(())
    } else {
        bail!("install cancelled")
    }
}

fn warn_if_custom_registry(url: &str) {
    let default_url = AxiomConfig::default().skills.registry_url;
    if url != default_url && !PathBuf::from(url).exists() {
        println!("Custom registries can change agent behavior. Only use registries you trust.");
    }
}

fn load_config() -> Result<(PathBuf, AxiomConfig)> {
    let config_path = AxiomConfig::default_config_path()?;
    let config = AxiomConfig::load_or_create(&config_path)?;
    Ok((config_path, config))
}

fn skills_dir(config_path: &Path, config: &AxiomConfig) -> PathBuf {
    config_path
        .parent()
        .map(|config_dir| config_dir.join(&config.skills.local_dir))
        .unwrap_or_else(|| PathBuf::from(&config.skills.local_dir))
}

struct ProofSkillApprover<'a> {
    proof: &'a mut ProofRecorder,
}

impl SkillApproval for ProofSkillApprover<'_> {
    fn approve(&mut self, request: &ApprovalRequest) -> bool {
        println!(
            "Axiom approval required [{}]: {}",
            request.risk_level, request.message
        );
        let approved = chat::confirm("Approve skill execution?", false).unwrap_or(false);
        self.proof.record_approval(new_approval(
            format!("skill:{}", request.skill_id),
            request.risk_level.clone(),
            request.message.clone(),
            if approved { "approved" } else { "denied" },
        ));
        approved
    }
}

fn record_skill_output_files(proof: &mut ProofRecorder, result: &SkillExecutionResult) {
    if result.skill_id == "file.read" {
        proof.record_file_read(FileReadProof {
            event_id: axiom_proof::trace::new_event_id("read"),
            path: result.output["path"]
                .as_str()
                .unwrap_or("unknown")
                .to_string(),
            bytes: result.output["bytes"].as_u64(),
            allowed: true,
            blocked_reason: None,
        });
    }
    if result.skill_id == "file.write" {
        proof.record_file_write(FileWriteProof {
            event_id: axiom_proof::trace::new_event_id("write"),
            path: result.output["path"]
                .as_str()
                .unwrap_or("unknown")
                .to_string(),
            bytes_written: result.output["bytes_written"].as_u64(),
            created: result.output["created"].as_bool().unwrap_or(false),
            overwrote: !result.output["created"].as_bool().unwrap_or(false),
            approved: true,
            diff_summary: Some("file.write executed from `axiom skill run`".to_string()),
        });
    }
}
