use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Result};
use axiom_core::AxiomConfig;
use axiom_proof::{ProofMode, ProofRecorder};
use axiom_upd::{
    build_update_check, current_platform_asset, detect_installation_mode, download_url_bytes,
    find_asset, find_checksum_asset, install_staged_update, parse_releases_json, rollback_update,
    run_binary_version_check, stage_verified_update, GitHubReleaseClient, InstallOutcome,
    InstallationMode, ReleaseChannel, ReleaseMetadata, StageUpdateRequest, UpdateDirs, UpdateKind,
    UpdatePolicy, UpdateState, UpdateStatus,
};

use crate::{chat, UpdateCommands};

pub(crate) async fn run(command: UpdateCommands) -> Result<()> {
    match command {
        UpdateCommands::Status => status(),
        UpdateCommands::Check => check().await.map(|_| ()),
        UpdateCommands::Install => install().await,
        UpdateCommands::Rollback => rollback(),
        UpdateCommands::SetChannel { channel } => set_channel(&channel),
        UpdateCommands::SetPolicy { policy } => set_policy(&policy),
    }
}

fn status() -> Result<()> {
    let (config_path, config) = load_config()?;
    let dirs = update_dirs(&config_path)?;
    let state = UpdateState::load(&dirs.state_path)?;
    let binary_path = std::env::current_exe().ok();
    let mode = binary_path
        .as_ref()
        .map(detect_installation_mode)
        .unwrap_or(InstallationMode::Unknown);

    println!("Axiom update status");
    println!("Current version: {}", current_version());
    println!("Channel: {}", config.update.channel);
    println!("Policy: {}", config.update.policy);
    println!("Release repo: {}", config.update.release_repo);
    println!(
        "Last checked: {}",
        config.update.last_checked_at.as_deref().unwrap_or("never")
    );
    println!(
        "Available version: {}",
        config
            .update
            .last_available_version
            .as_deref()
            .or(state.available_version.as_deref())
            .unwrap_or("none")
    );
    println!(
        "Last update error: {}",
        config
            .update
            .last_update_error
            .as_deref()
            .or(state.last_error.as_deref())
            .unwrap_or("none")
    );
    println!("Installation mode: {mode}");
    if let Some(path) = binary_path {
        println!("Binary path: {}", path.display());
    }
    println!("Update state: {}", state.status);
    Ok(())
}

async fn check() -> Result<CheckContext> {
    let (config_path, mut config) = load_config()?;
    let context = check_context(&config).await?;
    let dirs = update_dirs(&config_path)?;
    dirs.create_all()?;

    config.update.last_checked_at = Some(axiom_upd::now_timestamp());
    config.update.last_available_version = context
        .check
        .update_available
        .then(|| context.check.latest_version.to_string());
    config.update.last_update_error = None;
    config.save_to_path(&config_path)?;

    let state = UpdateState {
        current_version: Some(context.check.current_version.to_string()),
        available_version: Some(context.check.latest_version.to_string()),
        downloaded_asset: Some(context.check.asset_name.clone()),
        status: UpdateStatus::Checked,
        release_url: context.check.release.html_url.clone(),
        asset_url: Some(context.asset_url.clone()),
        checksum_url: Some(context.checksum_url.clone()),
        ..Default::default()
    };
    state.save(&dirs.state_path)?;

    print_check_result(&context);
    record_update_proof(
        &config_path,
        &config,
        "axiom update check",
        proof_summary(&context),
        None,
        true,
    );

    Ok(context)
}

async fn install() -> Result<()> {
    let (config_path, mut config) = load_config()?;
    let binary_path = std::env::current_exe()?;
    let mode = detect_installation_mode(&binary_path);
    if mode == InstallationMode::CargoDev {
        let message = "Axiom is running from a Cargo build, so it will not replace this binary. Update check works, but install is disabled in cargo-dev mode.";
        println!("{message}");
        record_update_proof(
            &config_path,
            &config,
            "axiom update install",
            message.to_string(),
            Some(message.to_string()),
            false,
        );
        return Ok(());
    }

    let context = check_context(&config).await?;
    print_check_result(&context);
    if !context.check.update_available {
        println!("Axiom is up to date.");
        return Ok(());
    }
    if context.check.kind == UpdateKind::Downgrade {
        bail!(
            "downgrades are blocked. Use `axiom update rollback` if you need to restore a backup"
        );
    }

    if context.check.kind == UpdateKind::Major {
        println!(
            "Axiom {} is available. This is a major update. Review the release notes before installing.",
            context.check.latest_version
        );
    }

    if !context.check.install_allowed_without_confirmation
        && !chat::confirm("Install this Axiom update?", false)?
    {
        bail!("update cancelled")
    }

    println!("Downloading {}", context.check.asset_name);
    let asset_bytes = download_url_bytes(&context.asset_url).await?;
    let checksum_bytes = download_url_bytes(&context.checksum_url).await?;
    let checksums = String::from_utf8(checksum_bytes)
        .map_err(|error| anyhow!("checksum file is not valid UTF-8: {error}"))?;

    let dirs = update_dirs(&config_path)?;
    let current_version = context.check.current_version.to_string();
    let latest_version = context.check.latest_version.to_string();
    let staged = stage_verified_update(StageUpdateRequest {
        dirs: &dirs,
        asset_name: &context.check.asset_name,
        asset_bytes: &asset_bytes,
        checksums: &checksums,
        current_version: &current_version,
        available_version: &latest_version,
        asset_url: Some(context.asset_url.clone()),
        checksum_url: Some(context.checksum_url.clone()),
    })?;
    println!("Checksum verified: {}", staged.checksum);

    let outcome = install_staged_update(
        &dirs,
        &binary_path,
        &staged.staged_binary_path,
        &context.check.current_version.to_string(),
        &context.check.latest_version.to_string(),
        config.update.backup_previous_binary,
        mode,
    )?;

    match &outcome {
        InstallOutcome::Installed { backup_path } => {
            match run_binary_version_check(&binary_path) {
                Ok(output) => println!("Post-install version check: {output}"),
                Err(error) => {
                    let _ = rollback_update(&dirs);
                    config.update.last_update_error = Some(error.to_string());
                    config.save_to_path(&config_path)?;
                    record_update_proof(
                        &config_path,
                        &config,
                        "axiom update install",
                        "update failed verification and rollback was attempted".to_string(),
                        Some(error.to_string()),
                        false,
                    );
                    return Err(error.into());
                }
            }
            println!("Axiom updated to {}.", context.check.latest_version);
            if let Some(path) = backup_path {
                println!("Backup: {}", path.display());
            }
        }
        InstallOutcome::PendingRestart { staged_binary_path } => {
            println!(
                "Update has been downloaded and staged. Close Axiom and run `axiom update install` again, or replace the binary with {}.",
                staged_binary_path.display()
            );
        }
    }

    config.update.last_available_version = Some(context.check.latest_version.to_string());
    config.update.last_update_error = None;
    config.save_to_path(&config_path)?;
    record_update_proof(
        &config_path,
        &config,
        "axiom update install",
        format!(
            "installed update {} -> {}",
            context.check.current_version, context.check.latest_version
        ),
        None,
        true,
    );
    Ok(())
}

fn rollback() -> Result<()> {
    let (config_path, config) = load_config()?;
    if !chat::confirm("Restore the previous Axiom binary backup?", false)? {
        bail!("rollback cancelled")
    }
    let dirs = update_dirs(&config_path)?;
    match rollback_update(&dirs) {
        Ok(outcome) => {
            let summary = format!(
                "rolled back {} from {}",
                outcome.restored_to.display(),
                outcome.restored_from.display()
            );
            println!("{summary}");
            record_update_proof(
                &config_path,
                &config,
                "axiom update rollback",
                summary,
                None,
                true,
            );
            Ok(())
        }
        Err(error) => {
            record_update_proof(
                &config_path,
                &config,
                "axiom update rollback",
                "rollback failed".to_string(),
                Some(error.to_string()),
                false,
            );
            Err(error.into())
        }
    }
}

fn set_channel(channel: &str) -> Result<()> {
    let parsed = ReleaseChannel::parse(channel)?;
    let (config_path, mut config) = load_config()?;
    config.update.channel = parsed.to_string();
    config.save_to_path(&config_path)?;
    let summary = format!("update channel set to {parsed}");
    println!("{summary}");
    record_update_proof(
        &config_path,
        &config,
        format!("axiom update set-channel {parsed}"),
        summary,
        None,
        true,
    );
    Ok(())
}

fn set_policy(policy: &str) -> Result<()> {
    let parsed = UpdatePolicy::parse(policy)?;
    let (config_path, mut config) = load_config()?;
    config.update.policy = parsed.to_string();
    config.save_to_path(&config_path)?;
    let summary = format!("update policy set to {parsed}");
    println!("{summary}");
    record_update_proof(
        &config_path,
        &config,
        format!("axiom update set-policy {parsed}"),
        summary,
        None,
        true,
    );
    Ok(())
}

#[derive(Debug, Clone)]
struct CheckContext {
    check: axiom_upd::CoreUpdateCheck,
    asset_url: String,
    checksum_url: String,
    policy: UpdatePolicy,
}

async fn check_context(config: &AxiomConfig) -> Result<CheckContext> {
    let channel = ReleaseChannel::parse(&config.update.channel)?;
    let policy = UpdatePolicy::parse(&config.update.policy)?;
    let releases = load_releases(&config.update.release_repo).await?;
    let platform = current_platform_asset()?;
    let check = build_update_check(current_version(), &releases, channel, policy, &platform)?;
    let asset = find_asset(&check.release, &check.asset_name)?;
    let checksum = find_checksum_asset(&check.release)?;
    Ok(CheckContext {
        asset_url: asset.browser_download_url.clone(),
        checksum_url: checksum.browser_download_url.clone(),
        check,
        policy,
    })
}

async fn load_releases(release_repo: &str) -> Result<Vec<ReleaseMetadata>> {
    let path = PathBuf::from(release_repo);
    if path.exists() {
        let manifest = if path.is_dir() {
            path.join("releases.json")
        } else {
            path
        };
        let content = fs::read_to_string(&manifest)?;
        Ok(parse_releases_json(&content)?)
    } else {
        Ok(GitHubReleaseClient::new(release_repo)
            .fetch_releases()
            .await?)
    }
}

fn print_check_result(context: &CheckContext) {
    if context.check.update_available {
        println!("Axiom update available.");
        println!("Current: {}", context.check.current_version);
        println!("Latest: {}", context.check.latest_version);
        println!("Type: {}", context.check.kind);
        println!("Channel: {}", context.check.channel);
        println!("Policy: {}", context.policy);
        println!(
            "Install without confirmation: {}",
            context.check.install_allowed_without_confirmation
        );
        println!("Asset: {}", context.check.asset_name);
        println!("Run: axiom update install");
    } else {
        println!("Axiom is up to date.");
        println!("Current version: {}", context.check.current_version);
        println!("Channel: {}", context.check.channel);
    }
}

fn proof_summary(context: &CheckContext) -> String {
    format!(
        "current={} available={} type={} channel={} policy={} asset={} checksum_asset={} update_available={}",
        context.check.current_version,
        context.check.latest_version,
        context.check.kind,
        context.check.channel,
        context.policy,
        context.check.asset_name,
        context.check.checksum_asset_name,
        context.check.update_available
    )
}

fn record_update_proof(
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
        ProofMode::Update,
        action.clone(),
        config.llm.active_provider.clone(),
        config.llm.active_model.clone(),
        Some(config.default_workspace_path().display().to_string()),
    );
    if let Some(error) = error {
        proof.record_error("update", error, action, true);
        proof.fail_trace(summary, "core_update");
    } else if completed {
        proof.set_final_response(summary.clone());
        proof.finish_trace(summary);
    } else {
        proof.cancel_trace(summary);
    }
    let _ = proof.export();
}

fn update_dirs(config_path: &Path) -> Result<UpdateDirs> {
    let config_dir = config_path
        .parent()
        .ok_or_else(|| anyhow!("config path has no parent directory"))?;
    Ok(UpdateDirs::new(config_dir))
}

fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn load_config() -> Result<(PathBuf, AxiomConfig)> {
    let config_path = AxiomConfig::default_config_path()?;
    let config = AxiomConfig::load_or_create(&config_path)?;
    Ok((config_path, config))
}
