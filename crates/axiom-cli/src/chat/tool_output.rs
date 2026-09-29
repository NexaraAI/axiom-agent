use std::{
    io::{self, IsTerminal},
    path::PathBuf,
};

use crate::ui::Renderer;

/// A tool result too large to have been conveyed by its one-line summary, spilled to
/// disk so `/show` can retrieve it.
pub(crate) struct SavedToolOutput {
    pub(crate) id: String,
    pub(crate) heading: String,
    /// Head of the stored payload, safe to print inline.
    pub(crate) preview: String,
    /// How much of the payload the preview actually shows.
    pub(crate) shown: String,
}

/// Payloads at or below these limits add nothing beyond the tool's own summary line.
///
/// The longest-line limit earns its place because serialised JSON escapes newlines: a
/// 2 KB block of text arrives as a single line, so newline counting alone would wave it
/// through and then wrap it across the whole terminal.
pub(crate) const TOOL_OUTPUT_SPILL_LINES: usize = 40;
pub(crate) const TOOL_OUTPUT_SPILL_CHARS: usize = 2_000;
pub(crate) const TOOL_OUTPUT_SPILL_LONGEST_LINE: usize = 1_200;
/// How much of a spilled payload is printed before pointing at `/show`.
pub(crate) const TOOL_OUTPUT_PREVIEW_LINES: usize = 16;
pub(crate) const TOOL_OUTPUT_PREVIEW_CHARS: usize = 1_600;

/// What to do with a plan the agent just proposed.
pub(crate) enum PlanDecision {
    /// Implement it as written.
    Proceed,
    /// Implement it with extra direction from the user.
    Adjust(String),
    /// Change nothing.
    Cancel,
}

/// Ask the user to agree the plan before it is implemented.
///
/// Returns `Proceed` when there is no terminal to ask in: a scripted run asked for the
/// task, so stalling on an unanswerable prompt would be worse than proceeding.
pub(crate) fn approve_plan(ui: &Renderer) -> PlanDecision {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return PlanDecision::Proceed;
    }
    let options = vec![
        "Approve — implement this plan".to_string(),
        "Cancel — change nothing".to_string(),
    ];
    match crate::ui::interactive_select(
        "Approve this plan? (type a reply to adjust it instead)",
        &options,
        0,
        true,
        ui,
    ) {
        crate::ui::SelectionResult::Selected { index: 0, .. } => PlanDecision::Proceed,
        crate::ui::SelectionResult::Selected { .. } => PlanDecision::Cancel,
        crate::ui::SelectionResult::Custom(reply) => {
            if reply.trim().is_empty() {
                PlanDecision::Cancel
            } else {
                PlanDecision::Adjust(reply)
            }
        }
        crate::ui::SelectionResult::Cancelled => PlanDecision::Cancel,
    }
}

/// Derive a valid, stable skill id from the task text.
pub(crate) fn skill_id_for_task(task: &str) -> String {
    const PREFIX: &str = "learned-";
    const MAX_ID_LEN: usize = 48;
    let mut id = String::from(PREFIX);
    let mut pending_dash = false;
    for character in task.chars() {
        if character.is_ascii_alphanumeric() {
            if pending_dash && id.len() > PREFIX.len() {
                id.push('-');
            }
            pending_dash = false;
            id.push(character.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
        if id.len() >= MAX_ID_LEN {
            break;
        }
    }
    // The loop can overshoot by one when a word boundary adds both a dash and a letter,
    // so clamp here as well. Every character pushed is ASCII, so `truncate` is safe.
    id.truncate(MAX_ID_LEN);
    while id.ends_with('-') {
        id.pop();
    }
    // `PREFIX` ends in a dash, so compare against its stem: a request with nothing
    // sluggable at all strips back to `learned`, which still needs a usable tail.
    if id == PREFIX.trim_end_matches('-') {
        id.push_str("-task");
    }
    id
}

/// Ask whether to keep a completed task's workflow for next time.
pub(crate) fn should_capture_skill(ui: &Renderer, id: &str) -> bool {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return false;
    }
    let options = vec![format!("Save as skill `{id}`"), "Skip this one".to_string()];
    matches!(
        crate::ui::interactive_select(
            "Keep this workflow as a reusable skill?",
            &options,
            0,
            false,
            ui,
        ),
        crate::ui::SelectionResult::Selected { index: 0, .. }
    )
}

/// Expand a leading `~` so `/workspace ~/projects/app` behaves like a shell would.
pub(crate) fn expand_workspace_path(raw: &str) -> PathBuf {
    let trimmed = raw.trim();
    let home = || std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));
    if trimmed == "~" {
        if let Some(home) = home() {
            return PathBuf::from(home);
        }
    }
    if let Some(rest) = trimmed
        .strip_prefix("~/")
        .or_else(|| trimmed.strip_prefix("~\\"))
    {
        if let Some(home) = home() {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(trimmed)
}

/// Prompt used for an automatic verification retry.
pub(crate) fn verification_fix_prompt(command: &str, diagnostics: &str) -> String {
    format!(
        "The verification command `{command}` failed after your last change. Diagnostics:\n\
         ```\n{diagnostics}\n```\n\
         Fix the root cause so `{command}` passes. Change the code rather than the test \
         unless the test itself is wrong. Do not ask for confirmation — this is an \
         automatic fix pass."
    )
}

pub(crate) fn valid_output_id(id: &str) -> bool {
    id.strip_prefix("out-").is_some_and(|suffix| {
        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}

pub(crate) fn bounded_output_preview(content: &str, max_lines: usize, max_chars: usize) -> String {
    let by_lines = content
        .lines()
        .take(max_lines)
        .collect::<Vec<_>>()
        .join("\n");
    by_lines.chars().take(max_chars).collect()
}
