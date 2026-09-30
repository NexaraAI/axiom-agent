use axiom_llm::detect_repetition_period;
use serde_json::Value;

use super::events::{HookExecution, HookStatus, ToolExecutionEvent, ToolExecutionStatus};
use crate::caps::{HOOK_OUTPUT_BUDGET_CHARS, TOOL_RESULT_BUDGET_CHARS};

/// The model-facing observation for a finished tool call: the tool result
/// itself, plus whatever manifest hooks reported around it.
pub(super) fn tool_observation(event: &ToolExecutionEvent) -> String {
    let mut observation = tool_observation_body(event);
    if let Some(hooks) = hook_observation(&event.hooks) {
        observation.push_str("\n\n");
        observation.push_str(&hooks);
    }
    observation
}

/// Summarizes the manifest hooks that ran around a tool call, so hook output
/// (for example a lint failure) can steer the next iteration.
fn hook_observation(hooks: &[HookExecution]) -> Option<String> {
    if hooks.is_empty() {
        return None;
    }
    let mut lines = vec!["Manifest hooks:".to_string()];
    for hook in hooks {
        let detail = match hook.status {
            HookStatus::Succeeded => hook.output.as_ref().map(|output| {
                truncate_for_observation(&output.to_string(), HOOK_OUTPUT_BUDGET_CHARS)
            }),
            HookStatus::Failed => hook.error.clone(),
            HookStatus::Unavailable => {
                Some("the hook skill is not executable in this workspace".to_string())
            }
            HookStatus::SkippedReentrancy => {
                Some("skipped: refusing to re-enter the loop".to_string())
            }
            HookStatus::SkippedBudget => {
                Some("skipped: hook budget or wall clock exhausted".to_string())
            }
            HookStatus::SkippedCancelled => Some("skipped: turn cancelled".to_string()),
        };
        match detail.filter(|detail| !detail.trim().is_empty()) {
            Some(detail) => lines.push(format!(
                "- {} [{}] {}",
                hook.hook_id,
                hook.phase.as_str(),
                detail
            )),
            None => lines.push(format!("- {} [{}]", hook.hook_id, hook.phase.as_str())),
        }
    }
    Some(lines.join("\n"))
}

fn truncate_for_observation(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let truncated = text.chars().take(limit).collect::<String>();
    format!("{truncated}… [hook output truncated]")
}

/// Renders a tool payload within the per-result context budget.
///
/// Fetch-style tools return `{status, content_type, text, url}`, and
/// re-serializing that as JSON spends tokens on the envelope and escapes
/// every newline in the body as `\n`. When the payload is such an envelope,
/// present the text directly and keep the provenance in a short header.
fn describe_fetch_envelope(output: &Value) -> Option<String> {
    let object = output.as_object()?;
    // Only unwrap a shape that is unambiguously a single fetched body.
    object.get("text")?.as_str()?;
    let looks_like_envelope = ["status", "content_type", "url", "bytes"]
        .iter()
        .any(|key| object.contains_key(*key));
    if !looks_like_envelope || object.len() > 6 {
        return None;
    }

    let mut header = String::from("```");
    if let Some(status) = object.get("status").and_then(Value::as_i64) {
        header.push_str(&format!(" HTTP {status}"));
    }
    if let Some(content_type) = object.get("content_type").and_then(Value::as_str) {
        header.push_str(&format!(" {content_type}"));
    }
    if let Some(url) = object.get("url").and_then(Value::as_str) {
        header.push_str(&format!(" {url}"));
    }
    Some(header)
}

/// Serializes a tool payload, applying the per-result context budget.
fn budget_result(output: &Value) -> String {
    let rendered = match describe_fetch_envelope(output) {
        Some(_) => output
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        None => output.to_string(),
    };

    let total = rendered.chars().count();
    if total <= TOOL_RESULT_BUDGET_CHARS {
        if describe_fetch_envelope(output).is_some() {
            return rendered;
        }
        return format!("```json\n{rendered}\n```");
    }

    let head: String = rendered.chars().take(TOOL_RESULT_BUDGET_CHARS).collect();
    format!(
        "{head}\n… [truncated: kept {TOOL_RESULT_BUDGET_CHARS} of {total} characters to limit \
         context growth. Re-run the tool with narrower arguments to see more — for example \
         `file.read` with `offset`/`limit`, `code.grep` for the exact symbol, or \
         `github.search` for a narrower query. The full output is saved for this session.]"
    )
}

fn tool_observation_body(event: &ToolExecutionEvent) -> String {
    match &event.status {
        ToolExecutionStatus::Succeeded(result) => {
            if result.skill_id == "question.ask" {
                if let Some(selected) = result.output.get("selected").and_then(Value::as_str) {
                    let is_custom = result
                        .output
                        .get("is_custom")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                    let reply_type = if is_custom {
                        "custom write-in reply"
                    } else {
                        "selected choice"
                    };
                    return format!(
                        "The user responded to your clarification question ({reply_type}):\n\"{selected}\"\n\nIMPORTANT: Prioritize this user answer above all else. Address and fulfill this response directly. Do not ask repeated questions or loop."
                    );
                }
            }
            if result.skill_id == "subagent.run" {
                let status = result
                    .output
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let role = result
                    .output
                    .get("role")
                    .and_then(Value::as_str)
                    .unwrap_or("subagent");
                let summary = result
                    .output
                    .get("summary")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                return format!(
                    "Sub-agent [{role}] reported status `{status}`:\n{summary}\n\nUse these findings to continue the original task. Dispatch another sub-agent or use your own tools if more evidence is needed."
                );
            }
            if let Some(124) = result.output.get("exit_code").and_then(Value::as_i64) {
                let stdout = result
                    .output
                    .get("stdout")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let stderr = result
                    .output
                    .get("stderr")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                return format!(
                    "Tool `{}` timed out after execution limit (exit code 124).\nCaptured stdout:\n```\n{stdout}\n```\nCaptured stderr:\n```\n{stderr}\n```\n\nAUTONOMOUS RECOVERY DIRECTIVE: Do not give up or abandon the task! The command took longer than the foreground timeout. Check if partial files were written to disk, check running processes, or adapt your approach (e.g. background the process, run sub-commands, or use compression). Continue your task now.",
                    result.skill_id
                );
            }
            let output = &result.output;
            let header = describe_fetch_envelope(output).unwrap_or_else(|| "```json".to_string());
            format!(
                "Tool `{}` succeeded:\n{}\n\n{}",
                result.skill_id,
                header,
                budget_result(output)
            )
        }
        ToolExecutionStatus::Failed(error) => {
            if error.contains("approval denied") {
                format!(
                    "Tool `{}` was declined by user approval: {error}\nAUTONOMOUS RECOVERY DIRECTIVE: Do not give up. Select an alternative non-destructive approach or explain what was requested.",
                    event.request.skill_id
                )
            } else if error.contains("cancelled")
                || error.contains("timeout")
                || error.contains("timed out")
            {
                format!(
                    "Tool `{}` interrupted or timed out: {error}\nAUTONOMOUS RECOVERY DIRECTIVE: Do not give up or stop. Adapt your approach and continue executing the task autonomously.",
                    event.request.skill_id
                )
            } else {
                format!(
                    "Tool `{}` failed: {error}\nAnalyze the error and take the next necessary step to complete the task.",
                    event.request.skill_id
                )
            }
        }
    }
}

pub(super) fn sanitize_interrupted_content(content: &str) -> String {
    let mut s = content.to_string();
    while let Some(period) = detect_repetition_period(&s) {
        let keep_len = s.len().saturating_sub(period * 2);
        let mut boundary = keep_len;
        while !s.is_char_boundary(boundary) && boundary < s.len() {
            boundary += 1;
        }
        s.truncate(boundary);
    }
    let mut s = s.trim().to_string();
    while let Some(period) = detect_repetition_period(&s) {
        let keep_len = s.len().saturating_sub(period * 2);
        let mut boundary = keep_len;
        while !s.is_char_boundary(boundary) && boundary < s.len() {
            boundary += 1;
        }
        s.truncate(boundary);
        s = s.trim().to_string();
    }
    if s.len() > 1500 {
        let mut boundary = 1500;
        while !s.is_char_boundary(boundary) && boundary > 0 {
            boundary -= 1;
        }
        s.truncate(boundary);
        s.push_str("\n... [Output truncated on interruption]");
    }
    s
}
