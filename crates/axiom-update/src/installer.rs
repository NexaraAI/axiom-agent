use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{
    now_timestamp, UpdateDirs, UpdateError, UpdateState, UpdateStatus, CHECKSUM_FILE_NAME,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallationMode {
    CargoDev,
    NpmGlobal,
    Standalone,
    Unknown,
}

impl std::fmt::Display for InstallationMode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::CargoDev => "cargo-dev",
            Self::NpmGlobal => "npm-global",
            Self::Standalone => "standalone",
            Self::Unknown => "unknown",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedUpdate {
    pub downloaded_asset_path: PathBuf,
    pub staged_binary_path: PathBuf,
    pub checksum_path: PathBuf,
    pub checksum: String,
}

#[derive(Debug, Clone)]
pub struct StageUpdateRequest<'a> {
    pub dirs: &'a UpdateDirs,
    pub asset_name: &'a str,
    pub asset_bytes: &'a [u8],
    pub checksums: &'a str,
    pub current_version: &'a str,
    pub available_version: &'a str,
    pub asset_url: Option<String>,
    pub checksum_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallOutcome {
    Installed { backup_path: Option<PathBuf> },
    PendingRestart { staged_binary_path: PathBuf },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RollbackOutcome {
    pub restored_from: PathBuf,
    pub restored_to: PathBuf,
}

pub fn detect_installation_mode(binary_path: impl AsRef<Path>) -> InstallationMode {
    let normalized = binary_path
        .as_ref()
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_ascii_lowercase())
        .collect::<Vec<_>>();

    let has_target = normalized.iter().any(|part| part == "target");
    let has_cargo_profile = normalized
        .iter()
        .any(|part| part == "debug" || part == "release");
    if has_target && has_cargo_profile {
        return InstallationMode::CargoDev;
    }

    let joined = binary_path
        .as_ref()
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();
    if joined.contains("/vendor/bin/") || joined.contains("/node_modules/axiom-agent/") {
        return InstallationMode::NpmGlobal;
    }

    if binary_path.as_ref().file_name().is_some() {
        InstallationMode::Standalone
    } else {
        InstallationMode::Unknown
    }
}

pub fn ensure_install_allowed(mode: InstallationMode) -> Result<(), UpdateError> {
    if mode == InstallationMode::CargoDev {
        Err(UpdateError::InstallBlocked(
            "Axiom is running from a Cargo build. Build or install a release binary to test updater installation.".to_string(),
        ))
    } else {
        Ok(())
    }
}

pub fn stage_verified_update(request: StageUpdateRequest<'_>) -> Result<StagedUpdate, UpdateError> {
    let StageUpdateRequest {
        dirs,
        asset_name,
        asset_bytes,
        checksums,
        current_version,
        available_version,
        asset_url,
        checksum_url,
    } = request;
    dirs.create_all()?;
    let checksum = crate::verify_asset_from_sums(asset_name, asset_bytes, checksums)?;
    let downloaded_asset_path = dirs.downloads.join(asset_name);
    let staged_binary_path = dirs.staged.join(asset_name);
    let checksum_path = dirs.downloads.join(CHECKSUM_FILE_NAME);

    fs::write(&downloaded_asset_path, asset_bytes)?;
    fs::write(&staged_binary_path, asset_bytes)?;
    fs::write(&checksum_path, checksums)?;
    set_executable_permissions(&staged_binary_path)?;

    let state = UpdateState {
        current_version: Some(current_version.to_string()),
        available_version: Some(available_version.to_string()),
        downloaded_asset: Some(asset_name.to_string()),
        checksum: Some(checksum.clone()),
        downloaded_at: Some(now_timestamp()),
        status: UpdateStatus::Staged,
        asset_url,
        checksum_url,
        ..Default::default()
    };
    state.save(&dirs.state_path)?;

    Ok(StagedUpdate {
        downloaded_asset_path,
        staged_binary_path,
        checksum_path,
        checksum,
    })
}

pub fn install_staged_update(
    dirs: &UpdateDirs,
    current_binary_path: impl AsRef<Path>,
    staged_binary_path: impl AsRef<Path>,
    current_version: &str,
    available_version: &str,
    backup_previous_binary: bool,
    mode: InstallationMode,
) -> Result<InstallOutcome, UpdateError> {
    ensure_install_allowed(mode)?;
    dirs.create_all()?;
    let current_binary_path = current_binary_path.as_ref();
    let staged_binary_path = staged_binary_path.as_ref();
    if !staged_binary_path.exists() {
        return Err(UpdateError::MissingStagedBinary(
            staged_binary_path.display().to_string(),
        ));
    }

    let backup_path = if backup_previous_binary && current_binary_path.exists() {
        Some(create_backup(
            current_binary_path,
            &dirs.backups,
            current_version,
        )?)
    } else {
        None
    };

    match replace_binary(staged_binary_path, current_binary_path) {
        Ok(()) => {
            let mut state = UpdateState::load(&dirs.state_path)?;
            state.current_version = Some(current_version.to_string());
            state.available_version = Some(available_version.to_string());
            state.previous_binary_path = Some(current_binary_path.to_path_buf());
            state.backup_path = backup_path.clone();
            state.installed_at = Some(now_timestamp());
            state.status = UpdateStatus::Installed;
            state.last_error = None;
            state.save(&dirs.state_path)?;
            Ok(InstallOutcome::Installed { backup_path })
        }
        Err(error) => {
            let mut state = UpdateState::load(&dirs.state_path)?;
            state.current_version = Some(current_version.to_string());
            state.available_version = Some(available_version.to_string());
            state.previous_binary_path = Some(current_binary_path.to_path_buf());
            state.backup_path = backup_path;
            state.status = UpdateStatus::PendingRestart;
            state.last_error = Some(error.to_string());
            state.save(&dirs.state_path)?;
            Ok(InstallOutcome::PendingRestart {
                staged_binary_path: staged_binary_path.to_path_buf(),
            })
        }
    }
}

pub fn rollback_update(dirs: &UpdateDirs) -> Result<RollbackOutcome, UpdateError> {
    let mut state = UpdateState::load(&dirs.state_path)?;
    let backup_path = state
        .backup_path
        .clone()
        .ok_or(UpdateError::NoRollbackAvailable)?;
    let previous_binary_path = state
        .previous_binary_path
        .clone()
        .ok_or(UpdateError::NoRollbackAvailable)?;
    if !backup_path.exists() {
        return Err(UpdateError::NoRollbackAvailable);
    }

    fs::copy(&backup_path, &previous_binary_path)?;
    set_executable_permissions(&previous_binary_path)?;
    state.status = UpdateStatus::RolledBack;
    state.last_error = None;
    state.save(&dirs.state_path)?;

    Ok(RollbackOutcome {
        restored_from: backup_path,
        restored_to: previous_binary_path,
    })
}

pub fn backup_path_for(
    binary_path: impl AsRef<Path>,
    backups_dir: impl AsRef<Path>,
    version: &str,
) -> PathBuf {
    let binary_path = binary_path.as_ref();
    let file_name = binary_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("axiom");
    backups_dir.as_ref().join(format!("{file_name}-{version}"))
}

pub fn run_binary_version_check(binary_path: impl AsRef<Path>) -> Result<String, UpdateError> {
    let output = Command::new(binary_path.as_ref())
        .arg("--version")
        .output()
        .map_err(|error| UpdateError::PostInstallVerification(error.to_string()))?;
    if !output.status.success() {
        return Err(UpdateError::PostInstallVerification(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn create_backup(
    binary_path: &Path,
    backups_dir: &Path,
    current_version: &str,
) -> Result<PathBuf, UpdateError> {
    fs::create_dir_all(backups_dir)?;
    let backup_path = backup_path_for(binary_path, backups_dir, current_version);
    fs::copy(binary_path, &backup_path)?;
    set_executable_permissions(&backup_path)?;
    Ok(backup_path)
}

fn replace_binary(staged_binary_path: &Path, current_binary_path: &Path) -> std::io::Result<()> {
    fs::copy(staged_binary_path, current_binary_path)?;
    set_executable_permissions(current_binary_path).map_err(std::io::Error::other)
}

fn set_executable_permissions(path: &Path) -> Result<(), UpdateError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions)?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::sha256_hex;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn cargo_dev_mode_blocks_install() {
        let mode = detect_installation_mode("C:/repo/target/debug/axiom.exe");

        assert_eq!(mode, InstallationMode::CargoDev);
        assert!(matches!(
            ensure_install_allowed(mode),
            Err(UpdateError::InstallBlocked(_))
        ));
    }

    #[test]
    fn standalone_mode_allows_staged_install_in_temp_directory() {
        let dir = temp_dir();
        let dirs = UpdateDirs::new(&dir);
        fs::create_dir_all(&dir).expect("temp dir");
        let current = dir.join(if cfg!(windows) { "axiom.exe" } else { "axiom" });
        fs::write(&current, b"old").expect("old binary");
        let digest = sha256_hex(b"new");
        let sums = format!("{digest}  axiom\n");
        let staged = stage_verified_update(StageUpdateRequest {
            dirs: &dirs,
            asset_name: "axiom",
            asset_bytes: b"new",
            checksums: &sums,
            current_version: "0.1.0",
            available_version: "0.1.1",
            asset_url: None,
            checksum_url: None,
        })
        .expect("stage");

        let outcome = install_staged_update(
            &dirs,
            &current,
            &staged.staged_binary_path,
            "0.1.0",
            "0.1.1",
            true,
            InstallationMode::Standalone,
        )
        .expect("install");

        assert!(matches!(outcome, InstallOutcome::Installed { .. }));
        assert_eq!(fs::read(&current).expect("current"), b"new");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn backup_path_creation_is_stable() {
        let path = backup_path_for("C:/bin/axiom.exe", "C:/config/updates/backups", "0.1.0");

        assert!(path.ends_with("axiom.exe-0.1.0"));
    }

    #[test]
    fn rollback_restores_backup_in_temp_directory() {
        let dir = temp_dir();
        let dirs = UpdateDirs::new(&dir);
        dirs.create_all().expect("dirs");
        let current = dir.join("axiom");
        let backup = dirs.backups.join("axiom-0.1.0");
        fs::write(&current, b"new").expect("current");
        fs::write(&backup, b"old").expect("backup");
        let state = UpdateState {
            previous_binary_path: Some(current.clone()),
            backup_path: Some(backup.clone()),
            status: UpdateStatus::Installed,
            ..Default::default()
        };
        state.save(&dirs.state_path).expect("state");

        let outcome = rollback_update(&dirs).expect("rollback");

        assert_eq!(outcome.restored_from, backup);
        assert_eq!(fs::read(&current).expect("current"), b"old");
        let _ = fs::remove_dir_all(dir);
    }

    fn temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("axiom-update-installer-test-{nanos}-{counter}"))
    }
}
