use std::{
    path::{Path, PathBuf},
    time::Instant,
};

use anyhow::{anyhow, bail, Result};
use axiom_core::AxiomConfig;
use axiom_engine::{
    apply_skill_update, assess_skill_lifecycle, check_skill_update_statuses, current_axiom_version,
    disable_skill, enable_skill, execute_installed_tool, install_bundle_from_registry_client,
    install_skill_from_registry_client, load_registry_with_cache, mark_update_check_results,
    policy_plan, record_skill_execution_failure, record_skill_execution_success,
    registry_cache_dir, remove_skill, reset_skill_stats, ApprovalRequest, InstalledSkills,
    Platform, RegistryClient, RegistrySource, SkillApproval, SkillAutoUpdatePolicy,
    SkillExecutionContext, SkillExecutionResult, SkillManifest, SkillUpdateStatus, ToolRequest,
    TrustLevel,
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
        SkillCommands::Update {
            check,
            all,
            apply_patches,
            skill_id,
        } => update_skills(check, all, apply_patches, skill_id.as_deref()).await,
        SkillCommands::Health => show_skill_health(),
        SkillCommands::Enable { skill_id } => enable_installed_skill(&skill_id),
        SkillCommands::Disable { skill_id } => disable_installed_skill(&skill_id),
        SkillCommands::ResetStats { skill_id } => reset_installed_skill_stats(&skill_id),
        SkillCommands::Remove { skill_id } => remove_installed_skill(&skill_id),
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
            if let Some(warning) = selection.warning.as_deref() {
                println!("{warning}");
            }
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
            "- {} {} ({enabled}, state: {}, trust: {}, source: {})",
            record.id, record.version, record.state, record.trust_level, record.source
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

    let started_at = Instant::now();
    let result = {
        let mut approval = ProofSkillApprover { proof: &mut proof };
        execute_installed_tool(&request, &installed, &context, &mut approval).await
    };

    match result {
        Ok(result) => {
            let latency_ms = started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
            let _ = record_skill_execution_success(
                skills_dir(&config_path, &config),
                skill_id,
                latency_ms,
            );
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
            let _ = record_skill_execution_failure(
                skills_dir(&config_path, &config),
                skill_id,
                error.to_string(),
            );
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
    let (manifest, resource) = selection.client.fetch_skill_manifest(skill_id).await?;
    let assessment = assess_skill_lifecycle(
        &manifest,
        &selection.location,
        &selection.source_label,
        resource.sha256.as_deref(),
        &current_axiom_version(),
        &Platform::current(),
    );
    confirm_skill_trust_install(&config, &selection, &entry.id, assessment.trust_level)?;

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

async fn update_skills(
    check: bool,
    all: bool,
    apply_patches: bool,
    skill_id: Option<&str>,
) -> Result<()> {
    let modes = [check, all, apply_patches, skill_id.is_some()]
        .into_iter()
        .filter(|enabled| *enabled)
        .count();
    if modes != 1 {
        bail!(
            "use one of `axiom skill update --check`, `axiom skill update SKILL_ID`, `axiom skill update --all`, or `axiom skill update --apply-patches`"
        );
    }

    if check {
        return check_updates().await;
    }
    if let Some(skill_id) = skill_id {
        return update_one_skill(skill_id).await;
    }
    if all {
        return update_all_skills().await;
    }
    update_patch_skills().await
}

async fn check_updates() -> Result<()> {
    let (config_path, config, selection, installed, updates) = load_update_context().await?;
    if installed.skills.is_empty() {
        println!("No skills installed.");
        return Ok(());
    }

    mark_update_check_results(skills_dir(&config_path, &config), &updates)?;
    record_skill_action_proof(
        &config_path,
        &config,
        "axiom skill update --check",
        format!(
            "checked {} installed skills against {} and found {} updates",
            installed.skills.len(),
            selection.location,
            updates.len()
        ),
        None,
        true,
    );

    if let Some(warning) = selection.warning.as_deref() {
        println!("{warning}");
    }
    if updates.is_empty() {
        println!("No skill updates available.");
        return Ok(());
    }

    println!("Available skill updates:");
    print_updates(&updates);
    Ok(())
}

async fn update_one_skill(skill_id: &str) -> Result<()> {
    let (config_path, config, selection, _installed, updates) = load_update_context().await?;
    let Some(update) = updates.iter().find(|update| update.id == skill_id) else {
        println!("No update available for `{skill_id}`.");
        return Ok(());
    };
    print_updates(std::slice::from_ref(update));
    if !update.compatibility.compatible {
        bail!(
            "update for `{skill_id}` is incompatible: {}",
            update.compatibility.reason
        );
    }
    if update.trust_level == TrustLevel::Blocked {
        bail!("update for `{skill_id}` is blocked");
    }
    if !chat::confirm("Apply this skill update?", false)? {
        record_skill_action_proof(
            &config_path,
            &config,
            format!("axiom skill update {skill_id}"),
            "skill update cancelled".to_string(),
            None,
            false,
        );
        bail!("update cancelled")
    }

    match apply_skill_update(
        &selection.client,
        skills_dir(&config_path, &config),
        skill_id,
    )
    .await
    {
        Ok(applied) => {
            let summary = format!(
                "updated {} {} -> {} from {}",
                applied.id, applied.old_version, applied.new_version, applied.registry_source
            );
            record_skill_action_proof(
                &config_path,
                &config,
                format!("axiom skill update {skill_id}"),
                summary.clone(),
                None,
                true,
            );
            println!("{summary}");
            Ok(())
        }
        Err(error) => {
            record_skill_action_proof(
                &config_path,
                &config,
                format!("axiom skill update {skill_id}"),
                format!("failed to update {skill_id}"),
                Some(error.to_string()),
                false,
            );
            Err(error.into())
        }
    }
}

async fn update_all_skills() -> Result<()> {
    let (config_path, config, selection, _installed, updates) = load_update_context().await?;
    let applicable = updates
        .iter()
        .filter(|update| {
            update.compatibility.compatible && update.trust_level != TrustLevel::Blocked
        })
        .cloned()
        .collect::<Vec<_>>();
    if applicable.is_empty() {
        println!("No compatible skill updates available.");
        return Ok(());
    }

    println!("Compatible skill updates:");
    print_updates(&applicable);
    if !chat::confirm("Apply all compatible skill updates?", false)? {
        bail!("update cancelled")
    }

    apply_update_batch(
        &config_path,
        &config,
        &selection,
        &applicable,
        "axiom skill update --all",
    )
    .await
}

async fn update_patch_skills() -> Result<()> {
    let (config_path, config, selection, _installed, updates) = load_update_context().await?;
    let policy = SkillAutoUpdatePolicy::parse(&config.skills.auto_update_policy);
    let plan = policy_plan(policy, &updates);
    if policy != SkillAutoUpdatePolicy::AutoPatch {
        println!(
            "Skill auto update policy is `{policy}`. Set `skills.auto_update_policy = \"auto-patch\"` to apply patch updates automatically."
        );
        return Ok(());
    }
    let patches = updates
        .iter()
        .filter(|update| plan.patch_skill_ids.iter().any(|id| id == &update.id))
        .cloned()
        .collect::<Vec<_>>();
    if patches.is_empty() {
        println!("No compatible patch updates available.");
        return Ok(());
    }

    apply_update_batch(
        &config_path,
        &config,
        &selection,
        &patches,
        "axiom skill update --apply-patches",
    )
    .await
}

async fn apply_update_batch(
    config_path: &Path,
    config: &AxiomConfig,
    selection: &RegistrySelection,
    updates: &[SkillUpdateStatus],
    action: &str,
) -> Result<()> {
    let mut applied = Vec::new();
    let mut failed = Vec::new();
    for update in updates {
        match apply_skill_update(
            &selection.client,
            skills_dir(config_path, config),
            &update.id,
        )
        .await
        {
            Ok(result) => {
                println!(
                    "Updated {} {} -> {} ({})",
                    result.id, result.old_version, result.new_version, result.update_type
                );
                applied.push(result.id);
            }
            Err(error) => {
                println!("Failed to update {}: {error}", update.id);
                failed.push(format!("{}: {error}", update.id));
            }
        }
    }

    let success = failed.is_empty();
    let summary = format!(
        "{action}: applied {} update(s), failed {}",
        applied.len(),
        failed.len()
    );
    record_skill_action_proof(
        config_path,
        config,
        action,
        summary.clone(),
        (!failed.is_empty()).then(|| failed.join("; ")),
        success,
    );
    if success {
        Ok(())
    } else {
        bail!(summary)
    }
}

async fn load_update_context() -> Result<(
    PathBuf,
    AxiomConfig,
    RegistrySelection,
    InstalledSkills,
    Vec<SkillUpdateStatus>,
)> {
    let (config_path, config) = load_config()?;
    let installed = InstalledSkills::load_from_dir(skills_dir(&config_path, &config))?;
    let selection = load_registry_selection(&config, RegistryCommandSource::Configured).await?;
    let updates = check_skill_update_statuses(
        &installed,
        selection.client.index(),
        &selection.location,
        &current_axiom_version(),
        &Platform::current(),
    );

    Ok((config_path, config, selection, installed, updates))
}

fn print_updates(updates: &[SkillUpdateStatus]) {
    for update in updates {
        println!(
            "- {} {} -> {} | state: {} | source: {} | trust: {} | type: {} | compatibility: {}",
            update.id,
            update.current_version,
            update.available_version,
            update.state,
            update.source,
            update.trust_level,
            update.update_type,
            update.compatibility.reason
        );
    }
}

fn show_skill_health() -> Result<()> {
    let (config_path, config) = load_config()?;
    let installed = InstalledSkills::load_from_dir(skills_dir(&config_path, &config))?;
    if installed.skills.is_empty() {
        println!("No skills installed.");
        return Ok(());
    }

    println!("Skill health:");
    for record in installed.skills.values() {
        let enabled = if record.enabled {
            "enabled"
        } else {
            "disabled"
        };
        let last_error = record
            .last_runtime_error
            .as_deref()
            .or(record.last_update_error.as_deref())
            .map(summarize_error)
            .unwrap_or_else(|| "none".to_string());
        println!(
            "- {} | {} | state: {} | trust: {} | success: {} | failure: {} | last used: {} | avg latency: {} ms | last error: {}",
            record.id,
            enabled,
            record.state,
            record.trust_level,
            record.success_count,
            record.failure_count,
            record.last_used_at.as_deref().unwrap_or("never"),
            record
                .average_latency_ms
                .map(|value| value.to_string())
                .unwrap_or_else(|| "n/a".to_string()),
            last_error
        );
        if record.failure_count >= 3 {
            println!("  suggestion: inspect this skill before relying on it again.");
        }
    }
    Ok(())
}

fn enable_installed_skill(skill_id: &str) -> Result<()> {
    let (config_path, config) = load_config()?;
    match enable_skill(skills_dir(&config_path, &config), skill_id) {
        Ok(record) => {
            let summary = format!("enabled skill `{}`", record.id);
            record_skill_action_proof(
                &config_path,
                &config,
                format!("axiom skill enable {skill_id}"),
                summary.clone(),
                None,
                true,
            );
            println!("{summary}");
            Ok(())
        }
        Err(error) => {
            record_skill_action_proof(
                &config_path,
                &config,
                format!("axiom skill enable {skill_id}"),
                format!("failed to enable `{skill_id}`"),
                Some(error.to_string()),
                false,
            );
            Err(error.into())
        }
    }
}

fn disable_installed_skill(skill_id: &str) -> Result<()> {
    let (config_path, config) = load_config()?;
    let record = disable_skill(skills_dir(&config_path, &config), skill_id)?;
    let summary = format!("disabled skill `{}`", record.id);
    record_skill_action_proof(
        &config_path,
        &config,
        format!("axiom skill disable {skill_id}"),
        summary.clone(),
        None,
        true,
    );
    println!("{summary}");
    Ok(())
}

fn reset_installed_skill_stats(skill_id: &str) -> Result<()> {
    let (config_path, config) = load_config()?;
    let record = reset_skill_stats(skills_dir(&config_path, &config), skill_id)?;
    let summary = format!("reset health stats for skill `{}`", record.id);
    record_skill_action_proof(
        &config_path,
        &config,
        format!("axiom skill reset-stats {skill_id}"),
        summary.clone(),
        None,
        true,
    );
    println!("{summary}");
    Ok(())
}

fn remove_installed_skill(skill_id: &str) -> Result<()> {
    let (config_path, config) = load_config()?;
    if !chat::confirm(&format!("Remove installed skill `{skill_id}`?"), false)? {
        bail!("remove cancelled")
    }
    remove_skill(skills_dir(&config_path, &config), skill_id)?;
    let summary = format!("removed skill `{skill_id}`");
    record_skill_action_proof(
        &config_path,
        &config,
        format!("axiom skill remove {skill_id}"),
        summary.clone(),
        None,
        true,
    );
    println!("{summary}");
    Ok(())
}

fn summarize_error(error: &str) -> String {
    let mut summary = error.chars().take(120).collect::<String>();
    if error.chars().count() > 120 {
        summary.push_str("...");
    }
    summary
}

fn record_skill_action_proof(
    config_path: &Path,
    config: &AxiomConfig,
    action: impl Into<String>,
    summary: String,
    error: Option<String>,
    completed: bool,
) {
    let action = action.into();
    let mut proof = ProofRecorder::start_trace(
        crate::proof_commands::settings_from_config(config_path, config),
        ProofMode::Skill,
        action.clone(),
        config.llm.active_provider.clone(),
        config.llm.active_model.clone(),
        Some(config.default_workspace_path().display().to_string()),
    );
    if let Some(error) = error {
        proof.record_error("skill", error, action, true);
        proof.fail_trace(summary, "skill_lifecycle");
    } else if completed {
        proof.set_final_response(summary.clone());
        proof.finish_trace(summary);
    } else {
        proof.cancel_trace(summary);
    }
    let _ = proof.export();
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
    warning: Option<String>,
}

async fn load_registry_selection(
    config: &AxiomConfig,
    source: RegistryCommandSource<'_>,
) -> Result<RegistrySelection> {
    match source {
        RegistryCommandSource::Configured => {
            let config_path = AxiomConfig::default_config_path()?;
            let config_dir = config_path
                .parent()
                .ok_or_else(|| anyhow!("config path has no parent directory"))?;
            let cached = load_registry_with_cache(
                &config.skills.registry_url,
                registry_cache_dir(config_dir),
                onboarding::bundled_registry_path(),
                config.skills.registry_cache_ttl_hours,
                config.skills.fallback_to_bundled_registry,
            )
            .await?;
            Ok(RegistrySelection {
                custom_remote: cached.location != AxiomConfig::default().skills.registry_url
                    && cached.source_label != "bundled"
                    && !matches!(cached.client.source(), RegistrySource::Local(_))
                    && !config.skills.allow_untrusted_registries,
                client: cached.client,
                source_label: cached.source_label,
                location: cached.location,
                warning: cached.warning,
            })
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
                warning: None,
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
        warning: None,
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

fn confirm_skill_trust_install(
    config: &AxiomConfig,
    selection: &RegistrySelection,
    item_id: &str,
    trust_level: TrustLevel,
) -> Result<()> {
    match trust_level {
        TrustLevel::Trusted => Ok(()),
        TrustLevel::Community => {
            println!(
                "Skill `{item_id}` is from a community registry. Review it before relying on it."
            );
            confirm_custom_registry_install(config, selection, item_id, true)
        }
        TrustLevel::Untrusted => {
            println!(
                "Skill `{item_id}` is untrusted. It may have missing checksum, unknown metadata, or an unsupported entrypoint."
            );
            if config.skills.allow_untrusted_registries
                || chat::confirm("Install this untrusted skill?", false)?
            {
                Ok(())
            } else {
                bail!("install cancelled")
            }
        }
        TrustLevel::Blocked => bail!("skill `{item_id}` is blocked and cannot be installed"),
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
