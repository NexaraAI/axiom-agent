use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

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
    pub tool_calls: Vec<ToolCallProof>,
    pub file_reads: Vec<FileReadProof>,
    pub file_writes: Vec<FileWriteProof>,
    pub commands: Vec<CommandProof>,
    pub approvals: Vec<ApprovalProof>,
    pub patches: Vec<PatchProof>,
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
pub struct PatchProof {
    pub event_id: String,
    pub summary: String,
    pub changed_files: Vec<String>,
    pub diff: String,
    pub approved: bool,
    pub applied: bool,
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
            trace_version: "0.1".to_string(),
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
            tool_calls: Vec::new(),
            file_reads: Vec::new(),
            file_writes: Vec::new(),
            commands: Vec::new(),
            approvals: Vec::new(),
            patches: Vec::new(),
            tests: Vec::new(),
            errors: Vec::new(),
            final_response: None,
            summary: None,
            redactions: Vec::new(),
        }
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
