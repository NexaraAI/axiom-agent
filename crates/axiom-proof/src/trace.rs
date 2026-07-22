use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use std::{error::Error, fmt};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error as ThisError;

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

pub const CURRENT_TRACE_VERSION: &str = "0.2";
pub const SUPPORTED_TRACE_VERSIONS: &[&str] = &["0.1", CURRENT_TRACE_VERSION];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofTraceVersionError {
    found: String,
}

impl ProofTraceVersionError {
    pub fn found(&self) -> &str {
        &self.found
    }
}

impl fmt::Display for ProofTraceVersionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "unsupported proof trace version `{}`; supported versions are {} (current: {})",
            self.found,
            SUPPORTED_TRACE_VERSIONS.join(", "),
            CURRENT_TRACE_VERSION
        )
    }
}

impl Error for ProofTraceVersionError {}

#[derive(Debug, ThisError)]
pub enum ProofTraceLoadError {
    #[error("invalid proof trace JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("proof trace is missing required field `trace_version`")]
    MissingTraceVersion,
    #[error("proof trace field `trace_version` must be a string")]
    InvalidTraceVersionType,
    #[error(transparent)]
    UnsupportedVersion(#[from] ProofTraceVersionError),
}

pub fn validate_trace_version(version: &str) -> Result<(), ProofTraceVersionError> {
    if SUPPORTED_TRACE_VERSIONS.contains(&version) {
        Ok(())
    } else {
        Err(ProofTraceVersionError {
            found: version.to_string(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProofTrace {
    pub trace_version: String,
    pub session_id: String,
    pub task_id: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub mode: ProofMode,
    pub user_prompt: String,
    pub status: ProofStatus,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub workspace: Option<String>,
    pub lens: LensProof,
    pub skills: SkillsProof,
    #[serde(default)]
    pub agent_runtime: Option<AgentRuntimeProof>,
    pub tool_calls: Vec<ToolCallProof>,
    pub file_reads: Vec<FileReadProof>,
    pub file_writes: Vec<FileWriteProof>,
    pub commands: Vec<CommandProof>,
    pub approvals: Vec<ApprovalProof>,
    #[serde(default)]
    pub policy_decisions: Vec<PolicyDecisionProof>,
    pub patches: Vec<PatchProof>,
    #[serde(default)]
    pub checkpoints: Vec<CheckpointProof>,
    pub tests: Vec<TestProof>,
    pub errors: Vec<ErrorProof>,
    pub final_response: Option<String>,
    pub summary: Option<String>,
    pub redactions: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofMode {
    Chat,
    Skill,
    Coder,
    Onboarding,
    Update,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProofStatus {
    Started,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LensProof {
    pub enabled: bool,
    pub selected_skill_ids: Vec<String>,
    pub reason_summary: Option<String>,
    pub auto_routed_to_coder: bool,
    pub auto_route_mode: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillsProof {
    pub installed_skill_count: usize,
    pub selected_cards: Vec<SkillCardProof>,
    pub executed_skill_ids: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRuntimeProof {
    pub iterations: u32,
    pub tool_iterations: u32,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    pub estimated_cost_microusd: Option<u64>,
    pub context_tokens_estimate: u64,
    pub compacted_messages: usize,
    pub todo_updates: u32,
    pub todo_total: usize,
    pub todo_completed: usize,
    pub todo_remaining: usize,
    pub todo_blocked: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillCardProof {
    pub id: String,
    pub summary: String,
    pub risk_level: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallProof {
    pub event_id: String,
    pub skill_id: String,
    pub arguments_summary: String,
    pub success: bool,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub risk_level: Option<String>,
    pub permission_result: Option<String>,
    pub output_summary: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileReadProof {
    pub event_id: String,
    pub path: String,
    pub bytes: Option<u64>,
    pub allowed: bool,
    pub blocked_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileWriteProof {
    pub event_id: String,
    pub path: String,
    pub bytes_written: Option<u64>,
    pub created: bool,
    pub overwrote: bool,
    pub approved: bool,
    pub diff_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandProof {
    pub event_id: String,
    pub command: String,
    pub cwd: String,
    pub allowed: bool,
    pub approved: bool,
    pub exit_code: Option<i32>,
    pub stdout_summary: Option<String>,
    pub stderr_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalProof {
    pub approval_id: String,
    pub action: String,
    pub risk_level: String,
    pub prompt: String,
    pub user_decision: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyDecisionProof {
    pub event_id: String,
    pub skill_id: String,
    pub operation: String,
    pub classes: Vec<String>,
    pub action: String,
    pub outcome: String,
    pub target: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchProof {
    pub event_id: String,
    pub summary: String,
    pub changed_files: Vec<String>,
    pub diff: String,
    pub approved: bool,
    pub applied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointProof {
    pub event_id: String,
    pub checkpoint_id: String,
    pub path: String,
    pub files: Vec<String>,
    pub restored: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestProof {
    pub event_id: String,
    pub detected_command: String,
    pub ran: bool,
    pub approved: bool,
    pub exit_code: Option<i32>,
    pub passed: Option<bool>,
    pub output_summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorProof {
    pub event_id: String,
    pub error_type: String,
    pub message: String,
    pub stage: String,
    pub recoverable: bool,
}

impl ProofTrace {
    pub fn new(
        mode: ProofMode,
        session_id: impl Into<String>,
        task_id: impl Into<String>,
        user_prompt: impl Into<String>,
    ) -> Self {
        Self {
            trace_version: CURRENT_TRACE_VERSION.to_string(),
            session_id: session_id.into(),
            task_id: task_id.into(),
            started_at: now_timestamp(),
            ended_at: None,
            mode,
            user_prompt: user_prompt.into(),
            status: ProofStatus::Started,
            provider: None,
            model: None,
            workspace: None,
            lens: LensProof::default(),
            skills: SkillsProof::default(),
            agent_runtime: None,
            tool_calls: Vec::new(),
            file_reads: Vec::new(),
            file_writes: Vec::new(),
            commands: Vec::new(),
            approvals: Vec::new(),
            policy_decisions: Vec::new(),
            patches: Vec::new(),
            checkpoints: Vec::new(),
            tests: Vec::new(),
            errors: Vec::new(),
            final_response: None,
            summary: None,
            redactions: Vec::new(),
        }
    }

    pub fn validate_version(&self) -> Result<(), ProofTraceVersionError> {
        validate_trace_version(&self.trace_version)
    }

    pub fn from_json_str(input: &str) -> Result<Self, ProofTraceLoadError> {
        let value: Value = serde_json::from_str(input)?;
        let version = value
            .get("trace_version")
            .ok_or(ProofTraceLoadError::MissingTraceVersion)?
            .as_str()
            .ok_or(ProofTraceLoadError::InvalidTraceVersionType)?;
        validate_trace_version(version)?;
        Ok(serde_json::from_value(value)?)
    }
}

pub fn new_session_id() -> String {
    format!("session-{}", next_id_suffix())
}

pub fn new_task_id() -> String {
    format!("task-{}", next_id_suffix())
}

pub fn new_event_id(prefix: &str) -> String {
    format!("{prefix}-{}", next_id_suffix())
}

pub fn now_timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!("unix:{seconds}")
}

pub fn current_date_path() -> String {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| (duration.as_secs() / 86_400) as i64)
        .unwrap_or_default();
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

fn next_id_suffix() -> String {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    let counter = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{millis:x}-{counter:04x}")
}

fn civil_from_days(days_since_unix_epoch: i64) -> (i32, u32, u32) {
    let z = days_since_unix_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if month <= 2 { 1 } else { 0 };
    (year as i32, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_proofs_use_the_current_trace_version() {
        let trace = ProofTrace::new(ProofMode::Chat, "session", "task", "hello");

        assert_eq!(trace.trace_version, CURRENT_TRACE_VERSION);
        trace
            .validate_version()
            .expect("current version is supported");
    }

    #[test]
    fn explicit_loader_accepts_supported_versions_without_rewriting_them() {
        let trace = ProofTrace::new(ProofMode::Chat, "session", "task", "hello");
        let mut value = serde_json::to_value(trace).expect("serialize trace");
        let object = value.as_object_mut().expect("proof object");
        object.remove("agent_runtime");
        object.insert(
            "trace_version".to_string(),
            serde_json::Value::String("0.1".to_string()),
        );

        let restored = ProofTrace::from_json_str(
            &serde_json::to_string(&value).expect("serialize legacy proof"),
        )
        .expect("read legacy proof");
        let reserialized = serde_json::to_value(&restored).expect("reserialize legacy proof");

        assert_eq!(restored.trace_version, "0.1");
        assert_eq!(restored.agent_runtime, None);
        assert_eq!(reserialized["trace_version"], "0.1");
    }

    #[test]
    fn explicit_loader_rejects_unknown_future_and_unsupported_old_versions() {
        for version in ["0.0", "0.3", "1.0"] {
            let trace = ProofTrace::new(ProofMode::Chat, "session", "task", "hello");
            let mut value = serde_json::to_value(trace).expect("serialize trace");
            value["trace_version"] = Value::String(version.to_string());
            let error = ProofTrace::from_json_str(
                &serde_json::to_string(&value).expect("serialize versioned proof"),
            )
            .expect_err("unsupported version is rejected");

            let ProofTraceLoadError::UnsupportedVersion(error) = error else {
                panic!("expected an unsupported-version error");
            };
            assert_eq!(error.found(), version);
            assert_eq!(
                error.to_string(),
                format!(
                    "unsupported proof trace version `{version}`; supported versions are 0.1, 0.2 (current: 0.2)"
                )
            );
        }
    }

    #[test]
    fn explicit_loader_reports_missing_or_non_string_versions_precisely() {
        let trace = ProofTrace::new(ProofMode::Chat, "session", "task", "hello");
        let mut value = serde_json::to_value(trace).expect("serialize trace");
        value
            .as_object_mut()
            .expect("proof object")
            .remove("trace_version");
        assert!(matches!(
            ProofTrace::from_json_str(
                &serde_json::to_string(&value).expect("serialize versionless proof")
            ),
            Err(ProofTraceLoadError::MissingTraceVersion)
        ));

        value["trace_version"] = Value::Number(2.into());
        assert!(matches!(
            ProofTrace::from_json_str(
                &serde_json::to_string(&value).expect("serialize invalid proof")
            ),
            Err(ProofTraceLoadError::InvalidTraceVersionType)
        ));
    }
}
