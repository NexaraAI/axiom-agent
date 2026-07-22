use crate::{export, ProofStatus, ProofTrace};

pub fn markdown_summary(trace: &ProofTrace) -> serde_json::Result<String> {
    let trace = export::redacted_trace(trace)?;
    let mut report = String::new();
    report.push_str("# Axiom Proof Report\n\n");
    report.push_str("## Task\n\n");
    report.push_str(&format!("- Task ID: `{}`\n", trace.task_id));
    report.push_str(&format!("- Session ID: `{}`\n", trace.session_id));
    report.push_str(&format!("- Mode: `{:?}`\n", trace.mode));
    report.push_str(&format!("- Status: `{:?}`\n", trace.status));
    report.push_str(&format!("- Started: `{}`\n", trace.started_at));
    report.push_str(&format!(
        "- Ended: `{}`\n",
        trace.ended_at.as_deref().unwrap_or("not finished")
    ));
    report.push_str(&format!(
        "- Provider: `{}`\n",
        trace.provider.as_deref().unwrap_or("not recorded")
    ));
    report.push_str(&format!(
        "- Model: `{}`\n",
        trace.model.as_deref().unwrap_or("not recorded")
    ));
    report.push_str(&format!(
        "- Workspace: `{}`\n\n",
        trace.workspace.as_deref().unwrap_or("not recorded")
    ));

    report.push_str("## User Request\n\n");
    report.push_str(&trace.user_prompt);
    report.push_str("\n\n");

    report.push_str("## Axiom Lens\n\n");
    report.push_str(&format!("- Enabled: `{}`\n", trace.lens.enabled));
    report.push_str(&format!(
        "- Selected Skills: `{}`\n",
        if trace.lens.selected_skill_ids.is_empty() {
            "none".to_string()
        } else {
            trace.lens.selected_skill_ids.join(", ")
        }
    ));
    report.push_str(&format!(
        "- Auto Routed To Coder: `{}`\n",
        trace.lens.auto_routed_to_coder
    ));
    if let Some(reason) = &trace.lens.reason_summary {
        report.push_str(&format!("- Reason: {reason}\n"));
    }
    report.push('\n');

    if let Some(summary) = &trace.summary {
        report.push_str("## Plan\n\n");
        report.push_str(summary);
        report.push_str("\n\n");
    }

    report.push_str("## Agent Runtime\n\n");
    if let Some(runtime) = &trace.agent_runtime {
        report.push_str(&format!("- Model iterations: `{}`\n", runtime.iterations));
        report.push_str(&format!(
            "- Tool iterations: `{}`\n",
            runtime.tool_iterations
        ));
        report.push_str(&format!(
            "- Provider tokens: `{}` input, `{}` output, `{}` total\n",
            runtime.prompt_tokens, runtime.completion_tokens, runtime.total_tokens
        ));
        report.push_str(&format!(
            "- Context estimate: `{}` tokens\n",
            runtime.context_tokens_estimate
        ));
        report.push_str(&format!(
            "- Compacted messages: `{}`\n",
            runtime.compacted_messages
        ));
        report.push_str(&format!(
            "- Todo state: `{}` updates, `{}` completed, `{}` remaining, `{}` blocked (`{}` total)\n",
            runtime.todo_updates,
            runtime.todo_completed,
            runtime.todo_remaining,
            runtime.todo_blocked,
            runtime.todo_total
        ));
        match runtime.estimated_cost_microusd {
            Some(cost) => report.push_str(&format!(
                "- Estimated cost: `${:.6}`\n\n",
                cost as f64 / 1_000_000.0
            )),
            None => report.push_str("- Estimated cost: `unknown (pricing not configured)`\n\n"),
        }
    } else {
        report.push_str("No agent runtime metrics recorded.\n\n");
    }

    report.push_str("## Actions Taken\n\n");
    report.push_str("### Tool Calls\n\n");
    if trace.tool_calls.is_empty() {
        report.push_str("No tool calls recorded.\n\n");
    } else {
        for call in &trace.tool_calls {
            report.push_str(&format!(
                "- `{}` success={} risk={}\n",
                call.skill_id,
                call.success,
                call.risk_level.as_deref().unwrap_or("unknown")
            ));
        }
        report.push('\n');
    }

    report.push_str("### Files Written\n\n");
    if trace.file_writes.is_empty() {
        report.push_str("No file writes recorded.\n\n");
    } else {
        for write in &trace.file_writes {
            report.push_str(&format!(
                "- `{}` bytes={:?} approved={}\n",
                write.path, write.bytes_written, write.approved
            ));
        }
        report.push('\n');
    }

    report.push_str("### Commands Run\n\n");
    if trace.commands.is_empty() {
        report.push_str("No commands recorded.\n\n");
    } else {
        for command in &trace.commands {
            report.push_str(&format!(
                "- `{}` exit={:?} approved={}\n",
                command.command, command.exit_code, command.approved
            ));
        }
        report.push('\n');
    }

    report.push_str("## Approvals\n\n");
    if trace.approvals.is_empty() {
        report.push_str("No approvals recorded.\n\n");
    } else {
        for approval in &trace.approvals {
            report.push_str(&format!(
                "- `{}` decision={} risk={}\n",
                approval.action, approval.user_decision, approval.risk_level
            ));
        }
        report.push('\n');
    }

    report.push_str("## Side-Effect Policy Decisions\n\n");
    if trace.policy_decisions.is_empty() {
        report.push_str("No side-effect policy decisions recorded.\n\n");
    } else {
        for decision in &trace.policy_decisions {
            report.push_str(&format!(
                "- `{}` operation={} action={} outcome={} target={} reason={}\n",
                decision.skill_id,
                decision.operation,
                decision.action,
                decision.outcome,
                decision.target.as_deref().unwrap_or("none"),
                decision.reason
            ));
        }
        report.push('\n');
    }

    report.push_str("## Patch Summary\n\n");
    if trace.patches.is_empty() {
        report.push_str("No patches recorded.\n\n");
    } else {
        for patch in &trace.patches {
            report.push_str(&format!(
                "- {} files={} approved={} applied={}\n",
                patch.summary,
                patch.changed_files.join(", "),
                patch.approved,
                patch.applied
            ));
        }
        report.push('\n');
    }

    report.push_str("## Test Results\n\n");
    if trace.tests.is_empty() {
        report.push_str("No tests recorded.\n\n");
    } else {
        for test in &trace.tests {
            report.push_str(&format!(
                "- `{}` ran={} passed={:?} exit={:?}\n",
                test.detected_command, test.ran, test.passed, test.exit_code
            ));
        }
        report.push('\n');
    }

    report.push_str("## Recovery Checkpoints\n\n");
    if trace.checkpoints.is_empty() {
        report.push_str("No checkpoints recorded.\n\n");
    } else {
        for checkpoint in &trace.checkpoints {
            report.push_str(&format!(
                "- `{}` restored={} files={} reason={}\n",
                checkpoint.checkpoint_id,
                checkpoint.restored,
                checkpoint.files.join(", "),
                checkpoint.reason
            ));
        }
        report.push('\n');
    }

    report.push_str("## Errors and Recovery\n\n");
    if trace.errors.is_empty() {
        report.push_str("No errors recorded.\n\n");
    } else {
        for error in &trace.errors {
            report.push_str(&format!(
                "- {} at {}: {} recoverable={}\n",
                error.error_type, error.stage, error.message, error.recoverable
            ));
        }
        report.push('\n');
    }

    report.push_str("## Final Response\n\n");
    report.push_str(
        trace
            .final_response
            .as_deref()
            .unwrap_or("No final response recorded."),
    );
    report.push_str("\n\n");

    report.push_str("## Safety Notes\n\n");
    if trace.redactions.is_empty() {
        report.push_str("No redactions were required.\n");
    } else {
        report.push_str("Redactions applied:\n");
        for redaction in &trace.redactions {
            report.push_str(&format!("- {redaction}\n"));
        }
    }

    if trace.status != ProofStatus::Completed {
        report.push_str("\nThis task did not complete successfully.\n");
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use crate::{AgentRuntimeProof, ProofMode};

    use super::*;

    #[test]
    fn report_includes_agent_usage_and_cost() {
        let mut trace = ProofTrace::new(ProofMode::Chat, "session", "task", "hello");
        trace.agent_runtime = Some(AgentRuntimeProof {
            iterations: 2,
            tool_iterations: 1,
            prompt_tokens: 100,
            completion_tokens: 25,
            total_tokens: 125,
            estimated_cost_microusd: Some(350),
            context_tokens_estimate: 90,
            compacted_messages: 3,
            todo_updates: 2,
            todo_total: 3,
            todo_completed: 2,
            todo_remaining: 1,
            todo_blocked: 0,
        });

        let report = markdown_summary(&trace).expect("render report");

        assert!(report.contains("## Agent Runtime"));
        assert!(report.contains("`100` input, `25` output, `125` total"));
        assert!(report.contains("`$0.000350`"));
        assert!(report.contains("Compacted messages: `3`"));
    }
}
