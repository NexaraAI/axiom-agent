use std::{fs, path::PathBuf};

use axiom_core::atomic_write;
use thiserror::Error;

use crate::{
    export, report,
    trace::{current_date_path, new_event_id, new_session_id, new_task_id, now_timestamp},
    AgentRuntimeProof, ApprovalProof, CheckpointProof, CommandProof, ErrorProof, FileReadProof,
    FileWriteProof, PatchProof, ProofMode, ProofStatus, ProofTrace, SkillCardProof, TestProof,
    ToolCallProof,
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
    /// Number of calendar-day proof directories to retain. Zero disables
    /// automatic pruning.
    pub retention_days: u64,
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
                retention_days: 30,
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

        // Proofs are durable artifacts. The original request must pass through
        // the same bounded redaction path as every other captured value before
        // it can reach JSON or Markdown exports.
        let user_prompt = capture_with_settings(&settings, user_prompt.into());
        let mut trace = ProofTrace::new(mode, new_session_id(), new_task_id(), user_prompt);
        trace.provider = provider.map(|value| capture_with_settings(&settings, value));
        trace.model = model.map(|value| capture_with_settings(&settings, value));
        trace.workspace = workspace.map(|value| capture_with_settings(&settings, value));
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

    pub fn record_agent_runtime(&mut self, runtime: AgentRuntimeProof) {
        if let Some(trace) = self.trace_mut() {
            trace.agent_runtime = Some(runtime);
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

    pub fn record_policy_decision(&mut self, decision: crate::PolicyDecisionProof) {
        if let Some(trace) = self.trace_mut() {
            trace.policy_decisions.push(decision);
        }
    }

    pub fn record_patch(&mut self, mut patch: PatchProof) {
        let settings = self.settings.clone();
        if let Some(trace) = self.trace_mut() {
            patch.diff = capture_with_settings(&settings, patch.diff);
            trace.patches.push(patch);
        }
    }

    pub fn record_checkpoint(&mut self, checkpoint: CheckpointProof) {
        if let Some(trace) = self.trace_mut() {
            trace.checkpoints.push(checkpoint);
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
        self.prune_expired_proofs()?;
        let trace = export::redacted_trace(trace)?;
        let dir = self.trace_dir(&trace)?;
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.json", trace.task_id));
        atomic_write(&path, export::to_json(&trace)?.as_bytes())?;
        Ok(Some(path))
    }

    pub fn export_markdown(&self) -> Result<Option<PathBuf>> {
        let Some(trace) = self.trace() else {
            return Ok(None);
        };
        if !self.settings.auto_export_markdown {
            return Ok(None);
        }
        self.prune_expired_proofs()?;
        let trace = export::redacted_trace(trace)?;
        let dir = self.trace_dir(&trace)?;
        fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{}.md", trace.task_id));
        atomic_write(&path, report::markdown_summary(&trace)?.as_bytes())?;
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

    fn prune_expired_proofs(&self) -> Result<()> {
        if self.settings.retention_days == 0 || !self.settings.proofs_dir.exists() {
            return Ok(());
        }
        // Never let routine retention turn a configured symlink/junction into
        // a deletion capability outside the proof directory. A user may still
        // explicitly choose a custom export destination, but automatic pruning
        // fails closed when the root or a dated child resolves elsewhere.
        let root_metadata = fs::symlink_metadata(&self.settings.proofs_dir)?;
        if is_link_or_reparse_point(&root_metadata) || !root_metadata.is_dir() {
            return Ok(());
        }
        let canonical_root = fs::canonicalize(&self.settings.proofs_dir)?;
        let Some(today) = parse_date_path(&current_date_path()) else {
            return Ok(());
        };
        let retention_days = i64::try_from(self.settings.retention_days).unwrap_or(i64::MAX);
        let cutoff = today.saturating_sub(retention_days);

        for entry in fs::read_dir(&self.settings.proofs_dir)? {
            let entry = entry?;
            let entry_path = entry.path();
            let entry_metadata = fs::symlink_metadata(&entry_path)?;
            if is_link_or_reparse_point(&entry_metadata) || !entry_metadata.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Some(date) = parse_date_path(name) else {
                continue;
            };
            if date < cutoff {
                let path = entry_path;
                let Ok(canonical_entry) = fs::canonicalize(&path) else {
                    continue;
                };
                if canonical_entry.parent() != Some(canonical_root.as_path()) {
                    continue;
                }
                // Re-check immediately before removal to guard against a
                // replaced direct child on filesystems with symlink support.
                let metadata = fs::symlink_metadata(&path)?;
                if is_link_or_reparse_point(&metadata) || !metadata.is_dir() {
                    continue;
                }
                fs::remove_dir_all(path)?;
            }
        }
        Ok(())
    }
}

fn is_link_or_reparse_point(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        // Directory junctions are reparse points but are not guaranteed to be
        // reported as symbolic links by every Windows filesystem backend.
        metadata.file_attributes() & 0x0000_0400 != 0
    }
    #[cfg(not(windows))]
    {
        false
    }
}

fn parse_date_path(value: &str) -> Option<i64> {
    let mut components = value.split('-');
    let year = components.next()?.parse::<i32>().ok()?;
    let month = components.next()?.parse::<u32>().ok()?;
    let day = components.next()?.parse::<u32>().ok()?;
    if components.next().is_some() || !(1..=12).contains(&month) {
        return None;
    }
    let days_in_month = match month {
        2 if is_leap_year(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=days_in_month).contains(&day) {
        return None;
    }

    let adjusted_year = year - i32::from(month <= 2);
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let adjusted_month = month as i32 + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * adjusted_month + 2) / 5 + day as i32 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some((era * 146_097 + day_of_era - 719_468) as i64)
}

fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
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
    // `redact_secrets` remains in config for compatibility, but durable proof
    // artifacts never permit secret redaction to be disabled.
    let _legacy_redaction_preference = settings.redact_secrets;
    summarize_text(&value, settings.max_capture_chars)
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
    fn user_prompt_is_always_redacted_and_bounded_in_json_and_markdown() {
        let dir = temp_dir();
        let secret = ["opaque", "prompt", "boundary", "1234567890"].join("-");
        crate::register_secret_for_redaction(&secret);
        let mut proof_settings = settings(&dir);
        proof_settings.redact_secrets = false;
        proof_settings.max_capture_chars = 48;
        let prompt = format!("use {secret} then {}", "x".repeat(200));
        let recorder =
            ProofRecorder::start_trace(proof_settings, ProofMode::Chat, prompt, None, None, None);

        let trace = recorder.trace().expect("trace");
        assert!(!trace.user_prompt.contains(&secret));
        assert!(trace.user_prompt.contains("[REDACTED]"));
        assert!(trace.user_prompt.ends_with("...[truncated]"));
        let json = crate::export::to_json(trace).expect("JSON");
        let markdown = crate::report::markdown_summary(trace).expect("Markdown");
        assert!(!json.contains(&secret));
        assert!(!markdown.contains(&secret));
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
        let credential_name = ["API", "KEY"].join("_");
        recorder.record_patch(PatchProof {
            event_id: new_event_id("patch"),
            summary: "patch".to_string(),
            changed_files: vec!["README.md".to_string()],
            diff: format!("{credential_name}={}", ["test", "value"].join("-")),
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

    #[test]
    fn persistence_prunes_only_expired_date_directories() {
        let dir = temp_dir();
        let expired = dir.join("1970-01-01");
        let malformed = dir.join("operator-notes");
        fs::create_dir_all(expired.join("session-old")).expect("create expired proof");
        fs::create_dir_all(&malformed).expect("create unrelated directory");
        fs::write(expired.join("session-old").join("proof.json"), b"old")
            .expect("write expired proof");
        fs::write(malformed.join("keep.txt"), b"keep").expect("write unrelated file");

        let mut proof_settings = settings(&dir);
        proof_settings.retention_days = 30;
        let recorder =
            ProofRecorder::start_trace(proof_settings, ProofMode::Chat, "task", None, None, None);
        let exported = recorder
            .export_json()
            .expect("export with retention")
            .expect("JSON path");

        assert!(!expired.exists());
        assert!(malformed.join("keep.txt").exists());
        assert!(exported.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn zero_retention_disables_automatic_pruning() {
        let dir = temp_dir();
        let expired = dir.join("1970-01-01").join("session-old");
        fs::create_dir_all(&expired).expect("create old proof directory");
        fs::write(expired.join("proof.json"), b"old").expect("write old proof");

        let mut proof_settings = settings(&dir);
        proof_settings.retention_days = 0;
        let recorder =
            ProofRecorder::start_trace(proof_settings, ProofMode::Chat, "task", None, None, None);
        recorder.export_json().expect("export without pruning");

        assert!(expired.exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn retention_never_follows_symlinked_proof_roots_or_date_directories() {
        use std::os::unix::fs::symlink;

        let root = temp_dir();
        let outside = temp_dir();
        let outside_expired = outside.join("1970-01-01").join("session-old");
        fs::create_dir_all(&outside_expired).expect("create outside proof");
        fs::write(outside_expired.join("proof.json"), b"old").expect("write outside proof");

        let linked_root = root.join("linked-root");
        fs::create_dir_all(&root).expect("create root");
        symlink(&outside, &linked_root).expect("link root");
        let mut root_settings = settings(&linked_root);
        root_settings.retention_days = 30;
        let root_recorder =
            ProofRecorder::start_trace(root_settings, ProofMode::Chat, "task", None, None, None);
        root_recorder
            .prune_expired_proofs()
            .expect("skip linked root");
        assert!(outside_expired.exists());

        let direct_root = root.join("direct-root");
        fs::create_dir_all(&direct_root).expect("create direct root");
        symlink(outside.join("1970-01-01"), direct_root.join("1970-01-01"))
            .expect("link dated child");
        let child_recorder = ProofRecorder::start_trace(
            settings(&direct_root),
            ProofMode::Chat,
            "task",
            None,
            None,
            None,
        );
        child_recorder
            .prune_expired_proofs()
            .expect("skip linked child");
        assert!(outside_expired.exists());

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }

    #[cfg(windows)]
    #[test]
    fn retention_never_follows_junctioned_proof_roots_or_date_directories() {
        let root = temp_dir();
        let outside = temp_dir();
        let outside_expired = outside.join("1970-01-01").join("session-old");
        fs::create_dir_all(&outside_expired).expect("create outside proof");
        fs::write(outside_expired.join("proof.json"), b"old").expect("write outside proof");
        fs::create_dir_all(&root).expect("create root");

        let linked_root = root.join("linked-root");
        create_junction(&linked_root, &outside);
        let root_recorder = ProofRecorder::start_trace(
            settings(&linked_root),
            ProofMode::Chat,
            "task",
            None,
            None,
            None,
        );
        root_recorder
            .prune_expired_proofs()
            .expect("skip junctioned root");
        assert!(outside_expired.exists());
        fs::remove_dir(&linked_root).expect("remove root junction");

        let direct_root = root.join("direct-root");
        fs::create_dir_all(&direct_root).expect("create direct root");
        let linked_date = direct_root.join("1970-01-01");
        create_junction(&linked_date, &outside.join("1970-01-01"));
        let child_recorder = ProofRecorder::start_trace(
            settings(&direct_root),
            ProofMode::Chat,
            "task",
            None,
            None,
            None,
        );
        child_recorder
            .prune_expired_proofs()
            .expect("skip junctioned date directory");
        assert!(outside_expired.exists());
        fs::remove_dir(&linked_date).expect("remove date junction");

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(outside);
    }

    #[cfg(windows)]
    fn create_junction(link: &std::path::Path, target: &std::path::Path) {
        let link_display = link.display().to_string();
        let target = target.display().to_string();
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J", &link_display, &target])
            .status()
            .expect("launch mklink");
        assert!(status.success(), "create junction {link_display}");
    }

    fn settings(dir: &std::path::Path) -> ProofSettings {
        ProofSettings {
            enabled: true,
            proofs_dir: dir.to_path_buf(),
            trace_json: true,
            auto_export_markdown: true,
            redact_secrets: true,
            max_capture_chars: 4_000,
            retention_days: 30,
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
