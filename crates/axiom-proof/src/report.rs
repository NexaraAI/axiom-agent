use crate::{ProofStatus, ProofTrace};

pub fn markdown_summary(trace: &ProofTrace) -> String {
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

    report
}
