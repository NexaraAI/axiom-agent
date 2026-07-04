use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::UpdateError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateDirs {
    pub root: PathBuf,
    pub downloads: PathBuf,
    pub staged: PathBuf,
    pub backups: PathBuf,
    pub state_path: PathBuf,
}

impl UpdateDirs {
    pub fn new(config_dir: impl AsRef<Path>) -> Self {
        let root = config_dir.as_ref().join("updates");
        Self {
            downloads: root.join("downloads"),
            staged: root.join("staged"),
            backups: root.join("backups"),
            state_path: root.join("update-state.json"),
            root,
        }
    }

    pub fn create_all(&self) -> Result<(), UpdateError> {
        fs::create_dir_all(&self.downloads)?;
        fs::create_dir_all(&self.staged)?;
        fs::create_dir_all(&self.backups)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checked,
    Downloaded,
    Staged,
    Installed,
    PendingRestart,
    Failed,
    RolledBack,
}

impl std::fmt::Display for UpdateStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Idle => "idle",
            Self::Checked => "checked",
            Self::Downloaded => "downloaded",
            Self::Staged => "staged",
            Self::Installed => "installed",
            Self::PendingRestart => "pending_restart",
            Self::Failed => "failed",
            Self::RolledBack => "rolled_back",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateState {
    #[serde(default)]
    pub current_version: Option<String>,
    #[serde(default)]
    pub available_version: Option<String>,
    #[serde(default)]
    pub downloaded_asset: Option<String>,
    #[serde(default)]
    pub checksum: Option<String>,
    #[serde(default)]
    pub downloaded_at: Option<String>,
    #[serde(default)]
    pub installed_at: Option<String>,
    #[serde(default)]
    pub previous_binary_path: Option<PathBuf>,
    #[serde(default)]
    pub backup_path: Option<PathBuf>,
    #[serde(default)]
    pub status: UpdateStatus,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub release_url: Option<String>,
    #[serde(default)]
    pub asset_url: Option<String>,
    #[serde(default)]
    pub checksum_url: Option<String>,
}

impl UpdateState {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, UpdateError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let content = fs::read_to_string(path)?;
        serde_json::from_str(&content).map_err(|error| UpdateError::StateJson(error.to_string()))
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), UpdateError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            path,
            serde_json::to_string_pretty(self)
                .map_err(|error| UpdateError::StateJson(error.to_string()))?,
        )?;
        Ok(())
    }

    pub fn mark_failed(&mut self, error: impl Into<String>) {
        self.status = UpdateStatus::Failed;
        self.last_error = Some(error.into());
    }
}

pub fn now_timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!("unix:{seconds}")
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn update_state_json_save_load() {
        let dir = temp_dir();
        let path = dir.join("update-state.json");
        let state = UpdateState {
            current_version: Some("0.1.0".to_string()),
            available_version: Some("0.1.1".to_string()),
            downloaded_asset: Some("axiom".to_string()),
            checksum: Some("abc".to_string()),
            status: UpdateStatus::Checked,
            ..Default::default()
        };

        state.save(&path).expect("save");
        let loaded = UpdateState::load(&path).expect("load");

        assert_eq!(loaded, state);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn update_dirs_are_under_config_dir() {
        let dir = temp_dir();
        let dirs = UpdateDirs::new(&dir);

        assert_eq!(dirs.root, dir.join("updates"));
        assert_eq!(dirs.state_path, dir.join("updates/update-state.json"));
        let _ = fs::remove_dir_all(dir);
    }

    fn temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("axiom-update-state-test-{nanos}-{counter}"))
    }
}
