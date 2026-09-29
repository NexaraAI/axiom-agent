use std::{cell::RefCell, io::IsTerminal, path::PathBuf, rc::Rc};

use anyhow::Result;
use axiom_agent::{
    compact_messages, AgentTransitionKind, ToolExecutionStatus, TransitionCheckpoint,
    TransitionObserver,
};
use axiom_coder::WorkspaceCheckpoint;
use axiom_core::{
    PersistedSession, SessionApproval, SessionCheckpoint, SessionMessage, SessionStore,
    SessionTodoItem, SessionUsage,
};
use axiom_llm::ChatMessage;
use serde_json::Value;

use crate::ui::{out::emitln, Spinner};

use super::{
    format_tool_result_summary, redact_json_value, render_animated_file_write,
    session_todo_status_label,
};

pub(super) struct DurableTransitionWriter {
    pub(super) store: SessionStore,
    pub(super) base: PersistedSession,
    pub(super) max_tokens: u32,
    pub(super) approvals: Rc<RefCell<Vec<SessionApproval>>>,
    pub(super) live_status: bool,
    pub(super) workspace_checkpoint_root: PathBuf,
    pub(super) last_workspace_checkpoint_reference: Option<String>,
    pub(super) created_checkpoints: Vec<WorkspaceCheckpoint>,
    pub(super) tool_spinner: Option<Spinner>,
}

impl Drop for DurableTransitionWriter {
    fn drop(&mut self) {
        if let Some(mut spinner) = self.tool_spinner.take() {
            spinner.stop();
        }
    }
}

impl TransitionObserver for DurableTransitionWriter {
    fn on_transition(&mut self, checkpoint: &TransitionCheckpoint) -> Result<()> {
        if let AgentTransitionKind::ToolStarted { request, .. } = &checkpoint.transition.kind {
            if request.skill_id == "file.write" || request.skill_id == "file.replace" {
                if let Some(path) = request
                    .arguments
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                {
                    let workspace_checkpoint = WorkspaceCheckpoint::create(
                        &self.base.workspace,
                        &self.workspace_checkpoint_root,
                        &[path.to_string()],
                    )?;
                    self.last_workspace_checkpoint_reference =
                        Some(workspace_checkpoint.root().display().to_string());
                    self.created_checkpoints.push(workspace_checkpoint);
                }
            }
        }
        let mut state = self.base.clone();
        let mut history = state
            .history
            .iter()
            .map(|message| ChatMessage {
                role: message.role.clone(),
                content: message.content.clone(),
            })
            .collect::<Vec<_>>();
        history.extend(checkpoint.history_delta.clone());
        let compacted = compact_messages(&history, 0, self.max_tokens);
        state.history = compacted
            .messages
            .into_iter()
            .map(|message| SessionMessage {
                role: message.role,
                content: axiom_proof::redact_text(&message.content),
            })
            .collect();
        state.todo_items = checkpoint
            .todo
            .items
            .iter()
            .map(|item| SessionTodoItem {
                title: axiom_proof::redact_text(&item.title),
                status: session_todo_status_label(item.status).to_string(),
            })
            .collect();
        state.usage = SessionUsage {
            prompt_tokens: self
                .base
                .usage
                .prompt_tokens
                .saturating_add(checkpoint.ledger.prompt_tokens),
            completion_tokens: self
                .base
                .usage
                .completion_tokens
                .saturating_add(checkpoint.ledger.completion_tokens),
            total_tokens: self
                .base
                .usage
                .total_tokens
                .saturating_add(checkpoint.ledger.total_tokens),
        };
        let transition = redact_json_value(serde_json::to_value(&checkpoint.transition)?);
        let tool_events = checkpoint
            .tool_events
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .map(redact_json_value)
            .collect();
        let policy_decisions = checkpoint
            .policy_decisions
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .map(redact_json_value)
            .collect();
        state.checkpoint = Some(SessionCheckpoint {
            transition_sequence: checkpoint.transition.sequence,
            transition,
            partial_response: axiom_proof::redact_text(&checkpoint.partial),
            tool_events,
            policy_decisions,
            approvals: self
                .approvals
                .borrow()
                .iter()
                .map(|approval| SessionApproval {
                    skill_id: approval.skill_id.clone(),
                    risk_level: approval.risk_level.clone(),
                    message: axiom_proof::redact_text(&approval.message),
                    approved: approval.approved,
                })
                .collect(),
            workspace_checkpoint_reference: self.last_workspace_checkpoint_reference.clone(),
        });
        self.store.save(&mut state)?;
        if self.live_status {
            if let Some(mut spinner) = self.tool_spinner.take() {
                spinner.stop();
            }
            Spinner::clear_line();
            match &checkpoint.transition.kind {
                AgentTransitionKind::ProviderRequestPrepared {
                    iteration,
                    provider,
                    model,
                    ..
                } => {
                    let display_model =
                        model.strip_prefix(&format!("{provider}/")).unwrap_or(model);
                    emitln!(
                        "  ⚡ Axiom: working with {provider}/{display_model} (step {iteration})..."
                    );
                }
                AgentTransitionKind::ToolStarted { request, .. } => {
                    let target = match request.skill_id.as_str() {
                        "file.read" => request
                            .arguments
                            .get("path")
                            .and_then(Value::as_str)
                            .map(|p| format!(" `{p}`")),
                        "file.write" => request
                            .arguments
                            .get("path")
                            .and_then(Value::as_str)
                            .map(|p| format!(" `{p}`")),
                        "file.replace" => request
                            .arguments
                            .get("path")
                            .and_then(Value::as_str)
                            .map(|p| format!(" `{p}`")),
                        "subagent.run" => request
                            .arguments
                            .get("role")
                            .and_then(Value::as_str)
                            .map(|r| format!(" [{r}]")),
                        "shell.powershell.safe"
                        | "shell.bash.safe"
                        | "shell.zsh.safe"
                        | "shell.run" => request
                            .arguments
                            .get("command")
                            .and_then(Value::as_str)
                            .map(|c| {
                                if c.len() > 40 {
                                    format!(" `{}...`", &c[..37])
                                } else {
                                    format!(" `{c}`")
                                }
                            }),
                        "skill.create" => request
                            .arguments
                            .get("id")
                            .and_then(Value::as_str)
                            .map(|id| format!(" `{id}`")),
                        "web.fetch" => request
                            .arguments
                            .get("url")
                            .and_then(Value::as_str)
                            .map(|u| format!(" `{u}`")),
                        "question.ask" => request
                            .arguments
                            .get("question")
                            .and_then(Value::as_str)
                            .map(|q| {
                                if q.len() > 40 {
                                    format!(" `{}...`", &q[..37])
                                } else {
                                    format!(" `{q}`")
                                }
                            }),
                        "test.run" => request
                            .arguments
                            .get("command")
                            .and_then(Value::as_str)
                            .map(|c| format!(" `{c}`")),
                        _ => None,
                    }
                    .unwrap_or_default();
                    if request.skill_id == "file.write" {
                        if let Some(path) = request.arguments.get("path").and_then(Value::as_str) {
                            if let Some(content) =
                                request.arguments.get("content").and_then(Value::as_str)
                            {
                                render_animated_file_write(path, content);
                            }
                        }
                    }
                    if std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none() {
                        self.tool_spinner = Some(Spinner::start(
                            format!("Axiom Tool: executing {}{target}...", request.skill_id),
                            nu_ansi_term::Color::Cyan,
                        ));
                    } else {
                        emitln!("  ⚙ Axiom Tool: executing {}{target}...", request.skill_id);
                    }
                }
                AgentTransitionKind::ToolCompleted { event, .. } => {
                    if let Some(mut spinner) = self.tool_spinner.take() {
                        spinner.stop();
                    }
                    match &event.status {
                        ToolExecutionStatus::Succeeded(result) => {
                            let is_timeout =
                                result.output.get("exit_code").and_then(Value::as_i64) == Some(124);
                            let summary =
                                format_tool_result_summary(&event.request.skill_id, &result.output);
                            if is_timeout {
                                emitln!(
                                    "  ⏱ Axiom Tool: timed out {} → {}",
                                    event.request.skill_id,
                                    summary
                                );
                            } else {
                                emitln!(
                                    "  ✔ Axiom Tool: completed {} → {}",
                                    event.request.skill_id,
                                    summary
                                );
                            }
                        }
                        ToolExecutionStatus::Failed(error) => {
                            emitln!(
                                "  ✖ Axiom Tool: failed {} ({})",
                                event.request.skill_id,
                                error
                            );
                        }
                    }
                }
                AgentTransitionKind::ReflectQueued { .. } => {
                    emitln!("  🔍 Axiom: verifying workspace changes...")
                }
                AgentTransitionKind::ProviderDegradedNoTools {
                    provider, model, ..
                } => {
                    let display_model =
                        model.strip_prefix(&format!("{provider}/")).unwrap_or(model);
                    emitln!(
                        "  ⚠ {provider} has no endpoint for {display_model} that supports tool use — finishing this turn without tools."
                    );
                    emitln!(
                        "    Switch to a tool-capable model with /model <id> or /models to browse."
                    );
                }
                _ => {}
            }
        }
        Ok(())
    }
}
