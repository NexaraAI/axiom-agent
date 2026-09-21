use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{anyhow, bail, Result};
use axiom_core::AxiomConfig;
use axiom_proof::{ProofMode, ProofRecorder};
use axiom_upd::{
    build_update_check, current_platform_asset, detect_installation_mode,
    download_release_asset_bytes, download_release_checksum_bytes, find_asset, find_checksum_asset,
    github_releases_api_url, install_staged_update, parse_releases_json, rollback_update,
    run_binary_version_check, stage_verified_update, validate_release_asset_url,
    GitHubReleaseClient, InstallOutcome, InstallationMode, ReleaseChannel, ReleaseMetadata,
    StageUpdateRequest, UpdateDirs, UpdateKind, UpdatePolicy, UpdateState, UpdateStatus,
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

pub(crate) async fn install() -> Result<()> {
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

    if mode == InstallationMode::NpmGlobal {
        println!("Axiom was installed via npm. Running npm update...");
        match run_npm_global_update(Some(&binary_path)) {
            Ok(()) => {
                println!("Axiom updated successfully via npm. Please restart your session.");
                return Ok(());
            }
            Err(e) => bail!("{e}"),
        }
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

    let release_repo = context.release_repo.as_deref().ok_or_else(|| {
        anyhow!(
            "local dev release metadata is check-only; update installation requires an exact https://github.com/<owner>/<repo> release repository"
        )
    })?;

    if !context.check.install_allowed_without_confirmation
        && !chat::confirm("Install this Axiom update?", false)?
    {
        bail!("update cancelled")
    }

    println!("Downloading {}", context.check.asset_name);
    let asset_bytes = download_release_asset_bytes(
        release_repo,
        &context.check.release.tag_name,
        &context.check.asset_name,
        &context.asset_url,
    )
    .await?;
    let checksum_bytes = download_release_checksum_bytes(
        release_repo,
        &context.check.release.tag_name,
        &context.checksum_url,
    )
    .await?;
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
            let credential_env_names = crate::credentials::credential_environment_names(&config)?;
            match run_binary_version_check(&binary_path, &latest_version, &credential_env_names) {
                Ok(output) => println!("Post-install version check: {output}"),
                Err(error) => {
                    let rollback = rollback_update(&dirs);
                    let failure = match rollback {
                        Ok(outcome) => format!(
                            "{error}; rollback restored {} to {}",
                            outcome.restored_from.display(),
                            outcome.restored_to.display()
                        ),
                        Err(rollback_error) => {
                            format!("{error}; rollback also failed: {rollback_error}")
                        }
                    };
                    config.update.last_update_error = Some(failure.clone());
                    config.save_to_path(&config_path)?;
                    record_update_proof(
                        &config_path,
                        &config,
                        "axiom update install",
                        "update failed exact version verification".to_string(),
                        Some(failure.clone()),
                        false,
                    );
                    return Err(anyhow!(failure));
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
    release_repo: Option<String>,
    policy: UpdatePolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ReleaseSource {
    GitHub,
    Local(PathBuf),
}

#[derive(Debug, Clone)]
struct LoadedReleases {
    releases: Vec<ReleaseMetadata>,
    release_repo: Option<String>,
}

async fn check_context(config: &AxiomConfig) -> Result<CheckContext> {
    let channel = ReleaseChannel::parse(&config.update.channel)?;
    let policy = UpdatePolicy::parse(&config.update.policy)?;
    let loaded = load_releases(&config.update.release_repo, channel).await?;
    let platform = current_platform_asset()?;
    let check = build_update_check(
        current_version(),
        &loaded.releases,
        channel,
        policy,
        &platform,
    )?;
    let asset = find_asset(&check.release, &check.asset_name)?;
    let checksum = find_checksum_asset(&check.release)?;
    if let Some(repo) = loaded.release_repo.as_deref() {
        validate_release_asset_url(
            repo,
            &check.release.tag_name,
            &check.asset_name,
            &asset.browser_download_url,
        )?;
        validate_release_asset_url(
            repo,
            &check.release.tag_name,
            &check.checksum_asset_name,
            &checksum.browser_download_url,
        )?;
    }
    Ok(CheckContext {
        asset_url: asset.browser_download_url.clone(),
        checksum_url: checksum.browser_download_url.clone(),
        release_repo: loaded.release_repo,
        check,
        policy,
    })
}

async fn load_releases(release_repo: &str, channel: ReleaseChannel) -> Result<LoadedReleases> {
    match resolve_release_source(release_repo, channel)? {
        ReleaseSource::GitHub => Ok(LoadedReleases {
            releases: GitHubReleaseClient::new(release_repo)
                .fetch_releases()
                .await?,
            release_repo: Some(release_repo.to_string()),
        }),
        ReleaseSource::Local(manifest) => {
            let content = fs::read_to_string(&manifest)?;
            Ok(LoadedReleases {
                releases: parse_releases_json(&content)?,
                release_repo: None,
            })
        }
    }
}

fn resolve_release_source(release_repo: &str, channel: ReleaseChannel) -> Result<ReleaseSource> {
    match github_releases_api_url(release_repo) {
        Ok(_) => return Ok(ReleaseSource::GitHub),
        Err(error) if channel != ReleaseChannel::Dev => return Err(error.into()),
        Err(_) => {}
    }

    let path = PathBuf::from(release_repo);
    if !path.is_absolute() {
        bail!(
            "dev release metadata must be an explicit absolute file or directory path, or an exact https://github.com/<owner>/<repo> URL"
        );
    }
    let manifest = if path.is_dir() {
        path.join("releases.json")
    } else {
        path
    };
    if !manifest.is_file() {
        bail!(
            "dev release metadata file does not exist or is not a regular file: {}",
            manifest.display()
        );
    }
    Ok(ReleaseSource::Local(fs::canonicalize(manifest)?))
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

pub(crate) fn npm_package_version_in(package_dir: &Path) -> Option<String> {
    let raw = fs::read_to_string(package_dir.join("package.json")).ok()?;
    let value = serde_json::from_str::<serde_json::Value>(&raw).ok()?;
    value.get("version")?.as_str().map(str::to_string)
}

/// Extract the semantic version from `--version` output such as
/// `axiom 1.2.3`, mirroring the updater's binary version check parsing.
fn version_from_shim_output(output: &str) -> Result<String, String> {
    let trimmed = output.trim();
    let reported = trimmed
        .split_whitespace()
        .last()
        .ok_or_else(|| "shim --version produced no output".to_string())?;
    let parsed = axiom_upd::parse_version(reported).map_err(|_| {
        format!("shim --version output did not end with a semantic version: {trimmed:?}")
    })?;
    Ok(parsed.to_string())
}

/// Run the npm shim (`node bin/axiom.js --version`) and return the version it
/// reports. Exercising the shim (rather than the binary directly) validates
/// the exact entrypoint users launch and lets the shim's first-run download
/// repair a blocked postinstall during the check itself.
fn npm_shim_version_output(package_dir: &Path) -> Result<String, String> {
    let shim = package_dir.join("bin").join("axiom.js");
    if !shim.exists() {
        return Err(
            "npm package shim is missing (bin/axiom.js); package install is broken".to_string(),
        );
    }
    let node = if cfg!(windows) { "node.exe" } else { "node" };
    let output = std::process::Command::new(node)
        .arg(&shim)
        .arg("--version")
        .output()
        .map_err(|error| format!("failed to launch the shim version check: {error}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let code = output
            .status
            .code()
            .map(|value| value.to_string())
            .unwrap_or_else(|| "signal".to_string());
        return Err(format!(
            "shim --version exited with status {code}: {}",
            stderr.trim()
        ));
    }
    version_from_shim_output(&String::from_utf8_lossy(&output.stdout))
}

/// Verify a freshly installed npm package can actually run: the shim must
/// report exactly the version the package declares. A mismatch means the
/// vendored binary is stale (a blocked or failed postinstall left the old
/// binary in place) or broken, and the update must not be reported as a
/// success.
fn verify_npm_install(package_dir: &Path, expected_version: &str) -> Result<String, String> {
    let declared = npm_package_version_in(package_dir)
        .ok_or_else(|| "installed package manifest (package.json) is unreadable".to_string())?;
    if declared != expected_version {
        return Err(format!(
            "package manifest reports v{declared} but v{expected_version} was expected"
        ));
    }
    let reported = npm_shim_version_output(package_dir)?;
    if reported != declared {
        return Err(format!(
            "vendored binary reports v{reported} but the npm package is v{declared} — the binary is stale (postinstall was blocked or failed)"
        ));
    }
    Ok(reported)
}

fn npm_rollback_args(previous_version: &str) -> Vec<String> {
    vec![
        "install".to_string(),
        "-g".to_string(),
        format!("axiom-agent@{previous_version}"),
    ]
}

fn npm_rollback(previous_version: &str) -> Result<(), String> {
    let npm_cmd = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let status = std::process::Command::new(npm_cmd)
        .args(npm_rollback_args(previous_version))
        .status()
        .map_err(|error| format!("rollback npm invocation failed: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "npm rollback to axiom-agent@{previous_version} failed; restore manually with: npm install -g axiom-agent@{previous_version}"
        ))
    }
}

pub(crate) fn run_npm_global_update(binary_path: Option<&Path>) -> Result<(), String> {
    let npm_cmd = if cfg!(windows) { "npm.cmd" } else { "npm" };
    let package_dir = binary_path.and_then(find_axiom_package_dir);
    // Capture the currently installed version BEFORE the update so a failed
    // post-install verification can restore it.
    let previous_version = package_dir.as_deref().and_then(npm_package_version_in);

    // On Windows, moving a running executable outside the npm package folder prevents
    // npm from failing with EBUSY during directory replacement.
    #[cfg(windows)]
    let staged_backup = if let Some(bin) = binary_path {
        move_running_binary_for_npm_update(bin)
    } else {
        None
    };

    // Try npm install with --allow-scripts first (for modern npm 12+), fallback to standard
    let status = std::process::Command::new(npm_cmd)
        .args([
            "install",
            "-g",
            "axiom-agent@latest",
            "--allow-scripts=axiom-agent",
        ])
        .status();

    let success = match status {
        Ok(s) if s.success() => true,
        _ => {
            // Fallback without --allow-scripts for older npm versions
            std::process::Command::new(npm_cmd)
                .args(["install", "-g", "axiom-agent@latest"])
                .status()
                .is_ok_and(|s| s.success())
        }
    };

    if !success {
        #[cfg(windows)]
        if let Some((ref original_path, ref backup_path)) = staged_backup {
            let _ = std::fs::rename(backup_path, original_path);
        }
        return Err("npm update failed to install the latest package".to_string());
    }

    // Repair a blocked postinstall before verifying: without the vendored
    // binary the shim would download on first launch anyway, but doing it
    // here keeps the update self-contained.
    if let Some(bin) = binary_path {
        if !bin.exists() {
            if let Some(pkg_dir) = find_axiom_package_dir(bin) {
                let postinstall = pkg_dir.join("scripts").join("postinstall.js");
                if postinstall.exists() {
                    let _ = std::process::Command::new("node")
                        .arg(&postinstall)
                        .status();
                }
            }
        }
    }

    // Verify the new install actually runs. A package whose npm metadata
    // advanced while its vendored binary is stale (blocked postinstall) or
    // broken must roll back instead of being reported as a success.
    let verification = match &package_dir {
        Some(dir) => {
            let expected = npm_package_version_in(dir).ok_or_else(|| {
                "updated package manifest (package.json) is unreadable".to_string()
            })?;
            verify_npm_install(dir, &expected)
        }
        // Standalone layouts outside an npm package tree cannot be verified
        // through the shim; keep the legacy behavior for them.
        None => Ok(String::new()),
    };

    match verification {
        Ok(_) => {
            #[cfg(windows)]
            if let Some((_, ref backup_path)) = staged_backup {
                let _ = std::fs::remove_file(backup_path);
            }
            Ok(())
        }
        Err(verification_error) => {
            let mut failure = format!("update verification failed: {verification_error}");
            match previous_version {
                Some(previous) => {
                    failure.push_str(&format!("; rolling back to axiom-agent@{previous}"));
                    if let Err(rollback_error) = npm_rollback(&previous) {
                        failure.push_str(&format!(" — {rollback_error}"));
                        return Err(failure);
                    }
                    // Restore the known-good running binary after the rollback
                    // (the backup lives outside the package dir npm replaced).
                    #[cfg(windows)]
                    if let Some((ref original_path, ref backup_path)) = staged_backup {
                        let _ = std::fs::remove_file(original_path);
                        let _ = std::fs::rename(backup_path, original_path);
                    }
                    if let Some(dir) = &package_dir {
                        if let Err(rollback_verify_error) = verify_npm_install(dir, &previous) {
                            failure.push_str(&format!(
                                " — rolled-back package also failed verification ({rollback_verify_error}); restore manually with: npm install -g axiom-agent@{previous}"
                            ));
                            return Err(failure);
                        }
                    }
                    failure.push_str(" — previous version restored");
                    Err(failure)
                }
                None => {
                    failure.push_str(
                        " — could not determine the previous version to roll back to; reinstall manually with: npm install -g axiom-agent@latest",
                    );
                    Err(failure)
                }
            }
        }
    }
}

#[cfg(windows)]
fn move_running_binary_for_npm_update(binary_path: &Path) -> Option<(PathBuf, PathBuf)> {
    if !binary_path.exists() {
        return None;
    }
    let mut current = binary_path.parent();
    while let Some(dir) = current {
        if dir
            .file_name()
            .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("axiom-agent"))
        {
            if let Some(parent) = dir.parent() {
                let backup_name = format!(".axiom-running-{}.bak", std::process::id());
                let backup_path = parent.join(backup_name);
                if let Ok(entries) = std::fs::read_dir(parent) {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if name.starts_with(".axiom-running-") && name.ends_with(".bak") {
                            let _ = std::fs::remove_file(entry.path());
                        }
                    }
                }
                if std::fs::rename(binary_path, &backup_path).is_ok() {
                    return Some((binary_path.to_path_buf(), backup_path));
                }
            }
            break;
        }
        current = dir.parent();
    }
    None
}

pub(crate) fn find_axiom_package_dir(binary_path: &Path) -> Option<PathBuf> {
    let mut current = binary_path.parent();
    while let Some(dir) = current {
        if dir
            .file_name()
            .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("axiom-agent"))
        {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_from_shim_output_parses_and_rejects_garbage() {
        assert_eq!(
            version_from_shim_output("axiom 1.2.3\n").expect("plain output parses"),
            "1.2.3"
        );
        assert_eq!(
            version_from_shim_output("  axiom-agent 1.0.19  ").expect("padded output parses"),
            "1.0.19"
        );
        assert!(version_from_shim_output("").is_err());
        assert!(version_from_shim_output("axiom unknown").is_err());
    }

    #[test]
    fn npm_rollback_pins_previous_version() {
        assert_eq!(
            npm_rollback_args("1.0.18"),
            vec![
                "install".to_string(),
                "-g".to_string(),
                "axiom-agent@1.0.18".to_string()
            ]
        );
    }

    #[test]
    fn verify_npm_install_detects_stale_vendored_binary() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("axiom-verify-{}-{nanos}", std::process::id()));
        let bin_dir = dir.join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin dir");
        fs::write(
            dir.join("package.json"),
            r#"{"name":"axiom-agent","version":"2.0.0"}"#,
        )
        .expect("write manifest");

        // A shim reporting the OLD version for the NEW package simulates the
        // blocked-postinstall failure: npm metadata advanced, vendored binary
        // did not. Verification must catch it.
        fs::write(bin_dir.join("axiom.js"), r#"console.log("axiom 1.0.18");"#)
            .expect("write stale shim");
        let error = verify_npm_install(&dir, "2.0.0").expect_err("stale vendored binary must fail");
        assert!(error.contains("stale"), "unexpected error: {error}");

        // A healthy shim reporting the declared version passes.
        fs::write(bin_dir.join("axiom.js"), r#"console.log("axiom 2.0.0");"#)
            .expect("write healthy shim");
        assert_eq!(
            verify_npm_install(&dir, "2.0.0").expect("healthy install verifies"),
            "2.0.0"
        );

        // A shim that exits non-zero (broken binary) fails with its stderr.
        fs::write(
            bin_dir.join("axiom.js"),
            r#"console.error("boom"); process.exit(3);"#,
        )
        .expect("write broken shim");
        let error = verify_npm_install(&dir, "2.0.0").expect_err("broken install must fail");
        assert!(error.contains("status 3"), "unexpected error: {error}");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn github_release_source_is_validated_before_any_path_interpretation() {
        assert_eq!(
            resolve_release_source(
                "https://github.com/NexaraAI/axiom-agent",
                ReleaseChannel::Stable
            )
            .expect("exact GitHub URL"),
            ReleaseSource::GitHub
        );
        assert!(resolve_release_source(
            "https:/github.com/NexaraAI/axiom-agent",
            ReleaseChannel::Stable
        )
        .is_err());
    }

    #[test]
    fn local_release_metadata_is_absolute_dev_only() {
        let root = std::env::temp_dir().join(format!(
            "axiom-update-source-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("temp directory");
        let manifest = root.join("releases.json");
        fs::write(&manifest, "[]").expect("fixture");

        assert!(resolve_release_source(
            manifest.to_str().expect("UTF-8 path"),
            ReleaseChannel::Stable
        )
        .is_err());
        assert!(resolve_release_source("releases.json", ReleaseChannel::Dev).is_err());
        assert!(matches!(
            resolve_release_source(
                manifest.to_str().expect("UTF-8 path"),
                ReleaseChannel::Dev
            )
            .expect("absolute dev fixture"),
            ReleaseSource::Local(path) if path == fs::canonicalize(&manifest).expect("canonical")
        ));

        fs::remove_dir_all(root).expect("clean fixture");
    }
}
