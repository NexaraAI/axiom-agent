use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use axiom_core::{atomic_write, AxiomError, Workspace};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::sha256_hex;

static CHECKPOINT_COUNTER: AtomicU64 = AtomicU64::new(0);
const CHECKPOINT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceCheckpoint {
    pub checkpoint_version: u32,
    pub id: String,
    pub created_at_unix_ms: u128,
    pub workspace: String,
    pub files: Vec<CheckpointFile>,
    #[serde(skip)]
    root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointFile {
    pub path: String,
    pub existed: bool,
    pub sha256: Option<String>,
}

#[derive(Debug, Error)]
pub enum CheckpointError {
    #[error("workspace path error: {0}")]
    Workspace(#[from] AxiomError),
    #[error("checkpoint I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("checkpoint manifest error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("checkpoint schema version {found} is unsupported (expected {expected})")]
    UnsupportedVersion { found: u32, expected: u32 },
    #[error("checkpoint belongs to a different workspace: {0}")]
    WorkspaceMismatch(String),
    #[error("checkpoint snapshot is missing or corrupt: {0}")]
    CorruptSnapshot(String),
}

impl WorkspaceCheckpoint {
    pub fn create(
        workspace_root: impl AsRef<Path>,
        checkpoints_root: impl AsRef<Path>,
        paths: &[String],
    ) -> Result<Self, CheckpointError> {
        let workspace = Workspace::new(workspace_root)?;
        let id = new_checkpoint_id();
        let root = checkpoints_root.as_ref().join(&id);
        let snapshots_root = root.join("files");
        fs::create_dir_all(&snapshots_root)?;

        let mut files = Vec::with_capacity(paths.len());
        for path in paths {
            let resolved = workspace.resolve_inside(path)?;
            let relative = resolved.strip_prefix(workspace.root()).map_err(|_| {
                AxiomError::UnsafeWorkspacePath {
                    path: resolved.clone(),
                }
            })?;
            let normalized = relative.to_string_lossy().replace('\\', "/");
            if resolved.exists() {
                let bytes = fs::read(&resolved)?;
                atomic_write(snapshots_root.join(relative), &bytes)?;
                files.push(CheckpointFile {
                    path: normalized,
                    existed: true,
                    sha256: Some(sha256_hex(&bytes)),
                });
            } else {
                files.push(CheckpointFile {
                    path: normalized,
                    existed: false,
                    sha256: None,
                });
            }
        }

        let checkpoint = Self {
            checkpoint_version: CHECKPOINT_VERSION,
            id,
            created_at_unix_ms: now_unix_ms(),
            workspace: workspace.root().display().to_string(),
            files,
            root,
        };
        checkpoint.save_manifest()?;
        Ok(checkpoint)
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, CheckpointError> {
        let root = path.as_ref().to_path_buf();
        let mut checkpoint: Self =
            serde_json::from_str(&fs::read_to_string(root.join("checkpoint.json"))?)?;
        if checkpoint.checkpoint_version != CHECKPOINT_VERSION {
            return Err(CheckpointError::UnsupportedVersion {
                found: checkpoint.checkpoint_version,
                expected: CHECKPOINT_VERSION,
            });
        }
        checkpoint.root = root;
        Ok(checkpoint)
    }

    pub fn restore(&self, workspace_root: impl AsRef<Path>) -> Result<(), CheckpointError> {
        let workspace = Workspace::new(workspace_root)?;
        if workspace.root().display().to_string() != self.workspace {
            return Err(CheckpointError::WorkspaceMismatch(self.workspace.clone()));
        }

        for file in &self.files {
            let target = workspace.resolve_inside(&file.path)?;
            if file.existed {
                let snapshot = self.root.join("files").join(&file.path);
                let bytes = fs::read(&snapshot)
                    .map_err(|_| CheckpointError::CorruptSnapshot(file.path.clone()))?;
                let snapshot_sha256 = sha256_hex(&bytes);
                if file.sha256.as_deref() != Some(snapshot_sha256.as_str()) {
                    return Err(CheckpointError::CorruptSnapshot(file.path.clone()));
                }
                atomic_write(target, &bytes)?;
            } else if target.exists() {
                if !target.is_file() {
                    return Err(CheckpointError::CorruptSnapshot(file.path.clone()));
                }
                fs::remove_file(target)?;
            }
        }
        Ok(())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn save_manifest(&self) -> Result<(), CheckpointError> {
        let content = serde_json::to_vec_pretty(self)?;
        atomic_write(self.root.join("checkpoint.json"), &content)?;
        Ok(())
    }
}

pub fn list_checkpoints(
    checkpoints_root: impl AsRef<Path>,
) -> Result<Vec<WorkspaceCheckpoint>, CheckpointError> {
    let root = checkpoints_root.as_ref();
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut checkpoints = Vec::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() && entry.path().join("checkpoint.json").exists() {
            checkpoints.push(WorkspaceCheckpoint::load(entry.path())?);
        }
    }
    checkpoints.sort_by(|left, right| {
        right
            .created_at_unix_ms
            .cmp(&left.created_at_unix_ms)
            .then_with(|| right.id.cmp(&left.id))
    });
    Ok(checkpoints)
}

fn new_checkpoint_id() -> String {
    let counter = CHECKPOINT_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("checkpoint-{:x}-{counter:04x}", now_unix_ms())
}

fn now_unix_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_modified_and_new_files() {
        let workspace = unique_temp_dir("workspace");
        let storage = unique_temp_dir("storage");
        fs::create_dir_all(&workspace).expect("workspace");
        fs::write(workspace.join("existing.txt"), "before").expect("existing");
        let checkpoint = WorkspaceCheckpoint::create(
            &workspace,
            &storage,
            &["existing.txt".to_string(), "new.txt".to_string()],
        )
        .expect("checkpoint");
        fs::write(workspace.join("existing.txt"), "after").expect("modify");
        fs::write(workspace.join("new.txt"), "created").expect("new");

        checkpoint.restore(&workspace).expect("restore");

        assert_eq!(
            fs::read_to_string(workspace.join("existing.txt")).expect("read"),
            "before"
        );
        assert!(!workspace.join("new.txt").exists());
        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(storage);
    }

    #[test]
    fn lists_newest_checkpoint_first() {
        let workspace = unique_temp_dir("workspace");
        let storage = unique_temp_dir("storage");
        fs::create_dir_all(&workspace).expect("workspace");
        let first = WorkspaceCheckpoint::create(&workspace, &storage, &[]).expect("first");
        let second = WorkspaceCheckpoint::create(&workspace, &storage, &[]).expect("second");

        let listed = list_checkpoints(&storage).expect("list");

        assert_eq!(listed.len(), 2);
        assert!(listed.iter().any(|checkpoint| checkpoint.id == first.id));
        assert_eq!(listed[0].id, second.id);
        let _ = fs::remove_dir_all(workspace);
        let _ = fs::remove_dir_all(storage);
    }

    fn unique_temp_dir(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "axiom-checkpoint-{label}-{:x}",
            now_unix_ms() + u128::from(CHECKPOINT_COUNTER.fetch_add(1, Ordering::Relaxed))
        ))
    }
}
