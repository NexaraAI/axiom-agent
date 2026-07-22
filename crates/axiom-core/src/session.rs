use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::atomic_write;

pub const CURRENT_SESSION_VERSION: u32 = 2;
pub const CURRENT_IDENTITY_VERSION: u32 = 1;
static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionId(String);

impl SessionId {
    pub fn new(value: impl Into<String>) -> Result<Self, SessionError> {
        let value = value.into();
        if value.is_empty()
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(SessionError::InvalidId(value));
        }
        Ok(Self(value))
    }

    pub fn generate() -> Self {
        let counter = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
        Self(format!("session-{:x}-{counter:04x}", now_unix_ms()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistedSession {
    pub session_version: u32,
    pub id: SessionId,
    pub created_at_unix_ms: u128,
    pub updated_at_unix_ms: u128,
    pub workspace: String,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub lens_enabled: bool,
    #[serde(default)]
    pub history: Vec<SessionMessage>,
    #[serde(default)]
    pub todo_items: Vec<SessionTodoItem>,
    #[serde(default)]
    pub usage: SessionUsage,
    #[serde(default = "default_identity_version")]
    pub identity_version: u32,
    #[serde(default)]
    pub checkpoint: Option<SessionCheckpoint>,
}

impl PersistedSession {
    pub fn new(id: SessionId, workspace: impl Into<String>) -> Self {
        let now = now_unix_ms();
        Self {
            session_version: CURRENT_SESSION_VERSION,
            id,
            created_at_unix_ms: now,
            updated_at_unix_ms: now,
            workspace: workspace.into(),
            provider: None,
            model: None,
            lens_enabled: true,
            history: Vec::new(),
            todo_items: Vec::new(),
            usage: SessionUsage::default(),
            identity_version: CURRENT_IDENTITY_VERSION,
            checkpoint: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCheckpoint {
    pub transition_sequence: u64,
    pub transition: serde_json::Value,
    #[serde(default)]
    pub partial_response: String,
    #[serde(default)]
    pub tool_events: Vec<serde_json::Value>,
    #[serde(default)]
    pub policy_decisions: Vec<serde_json::Value>,
    #[serde(default)]
    pub approvals: Vec<SessionApproval>,
    #[serde(default)]
    pub workspace_checkpoint_reference: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionApproval {
    pub skill_id: String,
    pub risk_level: String,
    pub message: String,
    pub approved: bool,
}

fn default_identity_version() -> u32 {
    CURRENT_IDENTITY_VERSION
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionTodoItem {
    pub title: String,
    pub status: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummary {
    pub id: SessionId,
    pub updated_at_unix_ms: u128,
    pub workspace: String,
    pub message_count: usize,
    pub todo_count: usize,
    pub total_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct SessionStore {
    root: PathBuf,
}

#[derive(Debug, Error)]
pub enum SessionError {
    #[error("session I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("session JSON error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid session id: {0}")]
    InvalidId(String),
    #[error("session not found: {0}")]
    NotFound(String),
    #[error("session schema version {found} is newer than supported version {supported}")]
    UnsupportedVersion { found: u32, supported: u32 },
}

impl SessionStore {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    pub fn save(&self, session: &mut PersistedSession) -> Result<PathBuf, SessionError> {
        if session.session_version > CURRENT_SESSION_VERSION {
            return Err(SessionError::UnsupportedVersion {
                found: session.session_version,
                supported: CURRENT_SESSION_VERSION,
            });
        }
        session.updated_at_unix_ms = now_unix_ms();
        fs::create_dir_all(&self.root)?;
        let path = self.path_for(&session.id);
        atomic_write(&path, &serde_json::to_vec_pretty(session)?)?;
        Ok(path)
    }

    pub fn load(&self, id: &SessionId) -> Result<PersistedSession, SessionError> {
        let path = self.path_for(id);
        if !path.exists() {
            return Err(SessionError::NotFound(id.as_str().to_string()));
        }
        let session: PersistedSession = serde_json::from_slice(&fs::read(path)?)?;
        if session.session_version > CURRENT_SESSION_VERSION {
            return Err(SessionError::UnsupportedVersion {
                found: session.session_version,
                supported: CURRENT_SESSION_VERSION,
            });
        }
        Ok(session)
    }

    pub fn list(&self) -> Result<Vec<SessionSummary>, SessionError> {
        if !self.root.exists() {
            return Ok(Vec::new());
        }
        let mut sessions = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if !entry.file_type()?.is_file()
                || entry.path().extension().and_then(|value| value.to_str()) != Some("json")
            {
                continue;
            }
            let session: PersistedSession = serde_json::from_slice(&fs::read(entry.path())?)?;
            if session.session_version > CURRENT_SESSION_VERSION {
                return Err(SessionError::UnsupportedVersion {
                    found: session.session_version,
                    supported: CURRENT_SESSION_VERSION,
                });
            }
            sessions.push(SessionSummary {
                id: session.id,
                updated_at_unix_ms: session.updated_at_unix_ms,
                workspace: session.workspace,
                message_count: session.history.len(),
                todo_count: session.todo_items.len(),
                total_tokens: session.usage.total_tokens,
            });
        }
        sessions.sort_by(|left, right| {
            right
                .updated_at_unix_ms
                .cmp(&left.updated_at_unix_ms)
                .then_with(|| right.id.as_str().cmp(left.id.as_str()))
        });
        Ok(sessions)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path_for(&self, id: &SessionId) -> PathBuf {
        self.root.join(format!("{}.json", id.as_str()))
    }
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
    fn session_store_round_trips_and_lists_state() {
        let root = unique_temp_dir();
        let store = SessionStore::new(&root);
        let mut session = PersistedSession::new(SessionId::generate(), "C:/workspace");
        session.history.push(SessionMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
        });
        session.todo_items.push(SessionTodoItem {
            title: "Finish".to_string(),
            status: "pending".to_string(),
        });
        session.usage.total_tokens = 42;

        store.save(&mut session).expect("save");
        let restored = store.load(&session.id).expect("load");
        let listed = store.list().expect("list");

        assert_eq!(restored, session);
        assert_eq!(listed[0].id, session.id);
        assert_eq!(listed[0].message_count, 1);
        assert_eq!(listed[0].total_tokens, 42);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_path_like_and_future_session_ids_or_versions() {
        assert!(SessionId::new("../escape").is_err());
        let root = unique_temp_dir();
        let store = SessionStore::new(&root);
        let mut session = PersistedSession::new(SessionId::generate(), "workspace");
        session.session_version = CURRENT_SESSION_VERSION + 1;
        assert!(matches!(
            store.save(&mut session),
            Err(SessionError::UnsupportedVersion { .. })
        ));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn loads_a_v1_session_with_v2_checkpoint_defaults() {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).expect("create root");
        let id = SessionId::new("legacy-v1").expect("id");
        let legacy = serde_json::json!({
            "session_version": 1,
            "id": "legacy-v1",
            "created_at_unix_ms": 1,
            "updated_at_unix_ms": 1,
            "workspace": "workspace",
            "provider": "mock",
            "model": "mock-model",
            "lens_enabled": true,
            "history": [],
            "todo_items": [],
            "usage": {"prompt_tokens": 0, "completion_tokens": 0, "total_tokens": 0}
        });
        fs::write(
            root.join("legacy-v1.json"),
            serde_json::to_vec_pretty(&legacy).expect("json"),
        )
        .expect("write legacy");

        let restored = SessionStore::new(&root).load(&id).expect("load v1");
        assert_eq!(restored.session_version, 1);
        assert_eq!(restored.identity_version, CURRENT_IDENTITY_VERSION);
        assert!(restored.checkpoint.is_none());
        let _ = fs::remove_dir_all(root);
    }

    fn unique_temp_dir() -> PathBuf {
        std::env::temp_dir().join(format!(
            "axiom-session-test-{:x}-{}",
            now_unix_ms(),
            SESSION_COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }
}
