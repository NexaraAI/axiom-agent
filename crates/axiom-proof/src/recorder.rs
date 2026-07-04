use std::{fs, path::PathBuf};

use thiserror::Error;

use crate::{
    export, report,
    trace::{current_date_path, new_event_id, new_session_id, new_task_id, now_timestamp},
    ApprovalProof, CommandProof, ErrorProof, FileReadProof, FileWriteProof, PatchProof, ProofMode,
    ProofStatus, ProofTrace, SkillCardProof, TestProof, ToolCallProof,
};
use crate::{redact_text, summarize_text};

#[derive(Debug, Clone)]
pub struct ProofSettings {
    pub enabled: bool,
    pub proofs_dir: PathBuf,
    pub trace_json: bool,
    pub auto_export_markdown: bool,
    pub redact_secrets: bool,
    pub max_capture_chars: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProofExportPaths {
    pub json_path: Option<PathBuf>,
    pub markdown_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LensSelectionRecord {
    pub enabled: bool,
    pub selected_skill_ids: Vec<String>,
    pub reason_summary: Option<String>,
    pub selected_cards: Vec<SkillCardProof>,
    pub installed_skill_count: usize,
    pub auto_routed_to_coder: bool,
    pub auto_route_mode: Option<String>,
}

#[derive(Debug, Error)]
pub enum ProofRecorderError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, ProofRecorderError>;

#[derive(Debug, Clone)]
pub struct ProofRecorder {
    settings: ProofSettings,
    trace: Option<ProofTrace>,
}

impl ProofRecorder {
    pub fn noop() -> Self {
        Self {
            settings: ProofSettings {
                enabled: false,
                proofs_dir: PathBuf::new(),
                trace_json: false,
                auto_export_markdown: false,
                redact_secrets: true,
                max_capture_chars: 4_000,
            },
            trace: None,
        }
    }

    pub fn start_trace(
        settings: ProofSettings,
        mode: ProofMode,
        user_prompt: impl Into<String>,
        provider: Option<String>,
        model: Option<String>,
        workspace: Option<String>,
    ) -> Self {
        if !settings.enabled {
            return Self {
                settings,
                trace: None,
            };
        }

        let mut trace = ProofTrace::new(mode, new_session_id(), new_task_id(), user_prompt);
        trace.provider = provider;
        trace.model = model;
        trace.workspace = workspace;
        Self {
            settings,
            trace: Some(trace),
        }
    }

    pub fn from_trace(settings: ProofSettings, trace: ProofTrace) -> Self {
        let enabled = settings.enabled;
        Self {
            settings,
            trace: enabled.then_some(trace),
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.trace.is_some()
    }

    pub fn trace(&self) -> Option<&ProofTrace> {
        self.trace.as_ref()
    }

    pub fn trace_mut(&mut self) -> Option<&mut ProofTrace> {
        self.trace.as_mut()
    }

    pub fn finish_trace(&mut self, summary: impl Into<String>) {
        let settings = self.settings.clone();
        if let Some(trace) = self.trace_mut() {
            trace.status = ProofStatus::Completed;
            trace.ended_at = Some(now_timestamp());
            trace.summary = Some(capture_with_settings(&settings, summary.into()));
        }
    }

    pub fn fail_trace(&mut self, error: impl Into<String>, stage: impl Into<String>) {
        self.record_error("error", error, stage, false);
        if let Some(trace) = self.trace_mut() {
            trace.status = ProofStatus::Failed;
            trace.ended_at = Some(now_timestamp());
        }
    }

    pub fn cancel_trace(&mut self, summary: impl Into<String>) {
        let settings = self.settings.clone();
        if let Some(trace) = self.trace_mut() {
            trace.status = ProofStatus::Cancelled;
            trace.ended_at = Some(now_timestamp());
            trace.summary = Some(capture_with_settings(&settings, summary.into()));
        }
    }

    pub fn record_lens_selection(&mut self, selection: LensSelectionRecord) {
        if let Some(trace) = self.trace_mut() {
            trace.lens.enabled = selection.enabled;
            trace.lens.selected_skill_ids = selection.selected_skill_ids;
            trace.lens.reason_summary = selection.reason_summary;
            trace.lens.auto_routed_to_coder = selection.auto_routed_to_coder;
            trace.lens.auto_route_mode = selection.auto_route_mode;
            trace.skills.selected_cards = selection.selected_cards;
            trace.skills.installed_skill_count = selection.installed_skill_count;
        }
    }

    pub fn record_tool_call(&mut self, mut call: ToolCallProof) {
        let settings = self.settings.clone();
        if let Some(trace) = self.trace_mut() {
            call.arguments_summary = capture_with_settings(&settings, call.arguments_summary);
            call.output_summary = call
                .output_summary
                .map(|value| capture_with_settings(&settings, value));
            call.error = call
                .error
                .map(|value| capture_with_settings(&settings, value));
            trace.skills.executed_skill_ids.push(call.skill_id.clone());
            trace.tool_calls.push(call);
        }
    }

    pub fn record_file_read(&mut self, read: FileReadProof) {
        if let Some(trace) = self.trace_mut() {
            trace.file_reads.push(read);
        }
    }

    pub fn record_file_write(&mut self, write: FileWriteProof) {
        if let Some(trace) = self.trace_mut() {
            trace.file_writes.push(write);
        }
    }

    pub fn record_command(&mut self, mut command: CommandProof) {
        let settings = self.settings.clone();
        if let Some(trace) = self.trace_mut() {
            command.stdout_summary = command
                .stdout_summary
                .map(|value| capture_with_settings(&settings, value));
            command.stderr_summary = command
                .stderr_summary
                .map(|value| capture_with_settings(&settings, value));
            trace.commands.push(command);
        }
    }

    pub fn record_approval(&mut self, approval: ApprovalProof) {
        if let Some(trace) = self.trace_mut() {
            trace.approvals.push(approval);
        }
    }

    pub fn record_patch(&mut self, mut patch: PatchProof) {
        let settings = self.settings.clone();
        if let Some(trace) = self.trace_mut() {
            patch.diff = capture_with_settings(&settings, patch.diff);
            trace.patches.push(patch);
        }
    }

    pub fn record_test(&mut self, mut test: TestProof) {
        let settings = self.settings.clone();
        if let Some(trace) = self.trace_mut() {
            test.output_summary = test
                .output_summary
                .map(|value| capture_with_settings(&settings, value));
            trace.tests.push(test);
        }
    }

    pub fn record_error(
        &mut self,
        error_type: impl Into<String>,
        message: impl Into<String>,
        stage: impl Into<String>,
        recoverable: bool,
    ) {
        let settings = self.settings.clone();
        if let Some(trace) = self.trace_mut() {
            trace.errors.push(ErrorProof {
                event_id: new_event_id("error"),
                error_type: error_type.into(),
                message: capture_with_settings(&settings, message.into()),
                stage: stage.into(),
                recoverable,
            });
        }
    }

    pub fn set_final_response(&mut self, response: impl Into<String>) {
        let settings = self.settings.clone();
        if let Some(trace) = self.trace_mut() {
            trace.final_response = Some(capture_with_settings(&settings, response.into()));
        }
    }

    pub fn export_json(&self) -> Result<Option<PathBuf>> {
        let Some(trace) = self.trace() else {
            return Ok(None);
        };
        if !self.settings.trace_json {
            return Ok(None);
        }
        let dir = self.trace_dir(trace)?;
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.json", trace.task_id));
        fs::write(&path, export::to_json(trace)?)?;
        Ok(Some(path))
    }

    pub fn export_markdown(&self) -> Result<Option<PathBuf>> {
        let Some(trace) = self.trace() else {
            return Ok(None);
        };
        if !self.settings.auto_export_markdown {
            return Ok(None);
        }
        let dir = self.trace_dir(trace)?;
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.md", trace.task_id));
        fs::write(&path, report::markdown_summary(trace))?;
        Ok(Some(path))
    }

    pub fn export(&self) -> Result<ProofExportPaths> {
        Ok(ProofExportPaths {
            json_path: self.export_json()?,
            markdown_path: self.export_markdown()?,
        })
    }

    fn trace_dir(&self, trace: &ProofTrace) -> Result<PathBuf> {
        Ok(self
            .settings
            .proofs_dir
            .join(current_date_path())
            .join(&trace.session_id))
    }
}

pub fn new_tool_call(
    skill_id: impl Into<String>,
    arguments_summary: impl Into<String>,
) -> ToolCallProof {
    ToolCallProof {
        event_id: new_event_id("tool"),
        skill_id: skill_id.into(),
        arguments_summary: arguments_summary.into(),
        success: false,
        started_at: now_timestamp(),
        ended_at: None,
        risk_level: None,
        permission_result: None,
        output_summary: None,
        error: None,
    }
}

pub fn new_approval(
    action: impl Into<String>,
    risk_level: impl Into<String>,
    prompt: impl Into<String>,
    user_decision: impl Into<String>,
) -> ApprovalProof {
    ApprovalProof {
        approval_id: new_event_id("approval"),
        action: action.into(),
        risk_level: risk_level.into(),
        prompt: prompt.into(),
        user_decision: user_decision.into(),
        timestamp: now_timestamp(),
    }
}

fn capture_with_settings(settings: &ProofSettings, value: String) -> String {
    if settings.redact_secrets {
        summarize_text(&value, settings.max_capture_chars)
    } else if value.chars().count() > settings.max_capture_chars {
        let mut summary = value
            .chars()
            .take(settings.max_capture_chars)
            .collect::<String>();
        summary.push_str("...[truncated]");
        summary
    } else {
        value
    }
}

pub fn record_redaction(trace: &mut ProofTrace, note: impl Into<String>) {
    trace.redactions.push(redact_text(&note.into()));
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;
    use crate::{PatchProof, ProofMode};

    #[test]
    fn creates_and_completes_proof_trace() {
        let dir = temp_dir();
        let mut recorder = ProofRecorder::start_trace(
            settings(&dir),
            ProofMode::Chat,
            "hello",
            Some("local".to_string()),
            Some("model".to_string()),
            Some("workspace".to_string()),
        );

        recorder.finish_trace("done");

        let trace = recorder.trace().expect("trace");
        assert_eq!(trace.status, ProofStatus::Completed);
        assert_eq!(trace.provider.as_deref(), Some("local"));
    }

    #[test]
    fn noop_recorder_when_disabled() {
        let mut settings = settings(&temp_dir());
        settings.enabled = false;

        let recorder =
            ProofRecorder::start_trace(settings, ProofMode::Chat, "hello", None, None, None);

        assert!(!recorder.is_enabled());
    }

    #[test]
    fn exports_json_and_markdown() {
        let dir = temp_dir();
        let mut recorder = ProofRecorder::start_trace(
            settings(&dir),
            ProofMode::Coder,
            "fix bug",
            None,
            None,
            None,
        );
        recorder.finish_trace("fixed");

        let paths = recorder.export().expect("export");

        assert!(paths.json_path.expect("json").exists());
        assert!(paths.markdown_path.expect("md").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn records_lens_tool_write_approval_patch_and_test() {
        let dir = temp_dir();
        let mut recorder =
            ProofRecorder::start_trace(settings(&dir), ProofMode::Coder, "task", None, None, None);

        recorder.record_lens_selection(LensSelectionRecord {
            enabled: true,
            selected_skill_ids: vec!["file.write".to_string()],
            reason_summary: Some("selected file write".to_string()),
            selected_cards: vec![SkillCardProof {
                id: "file.write".to_string(),
                summary: "Write files".to_string(),
                risk_level: "medium".to_string(),
            }],
            installed_skill_count: 1,
            auto_routed_to_coder: false,
            auto_route_mode: Some("ask".to_string()),
        });
        recorder.record_tool_call(new_tool_call("file.write", "{\"path\":\"README.md\"}"));
        recorder.record_file_write(FileWriteProof {
            event_id: new_event_id("write"),
            path: "README.md".to_string(),
            bytes_written: Some(5),
            created: false,
            overwrote: true,
            approved: true,
            diff_summary: Some("changed README".to_string()),
        });
        recorder.record_approval(new_approval("write", "medium", "Apply?", "approved"));
        recorder.record_patch(PatchProof {
            event_id: new_event_id("patch"),
            summary: "patch".to_string(),
            changed_files: vec!["README.md".to_string()],
            diff: "API_KEY=secret".to_string(),
            approved: true,
            applied: true,
        });
        recorder.record_test(TestProof {
            event_id: new_event_id("test"),
            detected_command: "cargo test".to_string(),
            ran: true,
            approved: true,
            exit_code: Some(0),
            passed: Some(true),
            output_summary: Some("ok".to_string()),
        });

        let trace = recorder.trace().expect("trace");
        assert_eq!(trace.lens.selected_skill_ids, vec!["file.write"]);
        assert_eq!(trace.tool_calls.len(), 1);
        assert_eq!(trace.file_writes.len(), 1);
        assert!(trace.patches[0].diff.contains("[REDACTED]"));
        assert_eq!(trace.tests.len(), 1);
    }

    fn settings(dir: &std::path::Path) -> ProofSettings {
        ProofSettings {
            enabled: true,
            proofs_dir: dir.to_path_buf(),
            trace_json: true,
            auto_export_markdown: true,
            redact_secrets: true,
            max_capture_chars: 4_000,
        }
    }

    fn temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("axiom-proof-recorder-test-{nanos}"))
    }
}
