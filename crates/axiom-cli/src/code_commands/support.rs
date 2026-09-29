use std::{
    io::{self, Write},
    path::PathBuf,
    process::Command,
};

use anyhow::Result;
use axiom_coder::{PreparedPatch, ProjectScanSummary, TestCommand, WorkspaceCheckpoint};
use axiom_core::{run_command_bounded, SECRET_GIT_PATHSPEC_EXCLUSIONS};
use axiom_engine::{ApprovalRequest, SkillApproval};
use axiom_proof::{new_approval, CheckpointProof, CommandProof, ProofRecorder, TestProof};

use super::CommandRunResult;
use crate::chat;

pub(super) struct CoderPolicyApprover<'a> {
    pub(super) proof: &'a mut ProofRecorder,
}

impl SkillApproval for CoderPolicyApprover<'_> {
    fn approve(&mut self, request: &ApprovalRequest) -> bool {
        println!(
            "Axiom policy approval required [{}]: {}",
            request.risk_level, request.message
        );
        let approved = chat::confirm("Approve this side effect?", false).unwrap_or(false);
        self.proof.record_approval(new_approval(
            format!("policy:{}", request.skill_id),
            request.risk_level.clone(),
            request.message.clone(),
            if approved { "approved" } else { "denied" },
        ));
        approved
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InteractiveResult {
    Continue,
    Exit,
    NotCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ApplyChoice {
    Apply,
    Edit,
    Cancel,
}

pub(super) fn prompt_apply_choice() -> Result<ApplyChoice> {
    println!();
    println!("╭────────────────────────────────────────────────────────────");
    println!("│  Plan Decision:");
    println!("│  [1] Apply changes now       (Enter / '1' / 'y' / 'a')");
    println!("│  [2] Revise / edit plan      ('2' / 'e' / 'edit')");
    println!("│  [3] Cancel                  ('3' / 'c' / 'n' / 'cancel')");
    println!("╰────────────────────────────────────────────────────────────");
    loop {
        print!("Choice [1-3] (Default: 1 - Apply): ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let cleaned = crate::chat::clean_pasted_input(&input);
        match cleaned.trim().to_ascii_lowercase().as_str() {
            "" | "1" | "y" | "yes" | "a" | "apply" => return Ok(ApplyChoice::Apply),
            "2" | "e" | "edit" | "r" | "revise" => return Ok(ApplyChoice::Edit),
            "3" | "c" | "cancel" | "n" | "no" | "q" | "quit" => return Ok(ApplyChoice::Cancel),
            _ => println!("Please select 1 (Apply), 2 (Edit), or 3 (Cancel)."),
        }
    }
}

pub(super) fn prompt_plan_revision() -> Result<String> {
    print!("Describe desired plan changes (or press Enter to keep plan): ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let cleaned = crate::chat::clean_pasted_input(&input);
    Ok(cleaned.trim().to_string())
}

pub(super) fn print_scan_summary(scan: &ProjectScanSummary) {
    println!("Workspace: {}", scan.root);
    println!("Project type: {}", scan.project_type);
    println!("Files scanned: {}", scan.files.len());
    if scan.ignored.is_empty() {
        println!("Ignored: none");
    } else {
        println!("Ignored: {}", scan.ignored.join(", "));
    }

    if scan.likely_test_commands.is_empty() {
        println!("Likely test commands: none");
    } else {
        println!("Likely test commands:");
        for command in &scan.likely_test_commands {
            println!(
                "- {} ({})",
                display_test_command(&command.command, command.working_directory.as_deref()),
                command.reason
            );
        }
    }
}

pub(super) fn print_plan(plan: &str) {
    println!("Plan:");
    println!("{plan}");
}

pub(super) fn choose_test_command(commands: &[TestCommand]) -> Result<&TestCommand> {
    if commands.len() == 1 {
        return Ok(&commands[0]);
    }

    println!("Multiple test commands detected:");
    for (index, command) in commands.iter().enumerate() {
        println!(
            "{}: {} ({})",
            index + 1,
            display_test_command(&command.command, command.working_directory.as_deref()),
            command.reason
        );
    }

    loop {
        print!("Choose test command number: ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let cleaned = crate::chat::clean_pasted_input(&input);
        if let Ok(index) = cleaned.trim().parse::<usize>() {
            if let Some(command) = commands.get(index.saturating_sub(1)) {
                return Ok(command);
            }
        }
        println!("Enter a number from 1 to {}.", commands.len());
    }
}

fn display_test_command(command: &str, working_directory: Option<&str>) -> String {
    match working_directory {
        Some(directory) => format!("`{command}` in `{directory}`"),
        None => format!("`{command}`"),
    }
}

pub(super) fn command_proof(
    command: &str,
    cwd: PathBuf,
    result: &CommandRunResult,
    approved: bool,
) -> CommandProof {
    CommandProof {
        event_id: axiom_proof::trace::new_event_id("command"),
        command: command.to_string(),
        cwd: cwd.display().to_string(),
        allowed: is_safe_test_command(command),
        approved,
        exit_code: result.exit_code,
        stdout_summary: Some(result.stdout.clone()),
        stderr_summary: Some(result.stderr.clone()),
    }
}

pub(super) fn test_proof(
    command: &str,
    ran: bool,
    approved: bool,
    result: &CommandRunResult,
) -> TestProof {
    TestProof {
        event_id: axiom_proof::trace::new_event_id("test"),
        detected_command: command.to_string(),
        ran,
        approved,
        exit_code: result.exit_code,
        passed: result.exit_code.map(|code| code == 0),
        output_summary: Some(format!(
            "{}{}",
            result.stdout,
            if result.stderr.is_empty() {
                String::new()
            } else {
                format!("\nstderr:\n{}", result.stderr)
            }
        )),
    }
}

pub(super) fn checkpoint_proof(
    checkpoint: &WorkspaceCheckpoint,
    restored: bool,
    reason: &str,
) -> CheckpointProof {
    CheckpointProof {
        event_id: axiom_proof::trace::new_event_id("checkpoint"),
        checkpoint_id: checkpoint.id.clone(),
        path: checkpoint.root().display().to_string(),
        files: checkpoint
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect(),
        restored,
        reason: reason.to_string(),
    }
}

pub(super) fn patch_scope(patch: &PreparedPatch) -> (usize, u64) {
    let bytes = patch.files.iter().fold(0_u64, |total, file| {
        total.saturating_add(u64::try_from(file.content.len()).unwrap_or(u64::MAX))
    });
    (patch.files.len(), bytes)
}

pub(super) fn is_safe_test_command(command: &str) -> bool {
    let parts = command.split_whitespace().collect::<Vec<_>>();
    matches!(
        parts.as_slice(),
        ["cargo", "test", ..]
            | ["npm", "test", ..]
            | ["pnpm", "test", ..]
            | ["yarn", "test", ..]
            | ["python", "-m", "pytest", ..]
            | ["pytest", ..]
            | ["go", "test", ..]
            | ["mvn", "test", ..]
            | ["gradle", "test", ..]
            | ["deno", "test", ..]
            | ["bun", "test", ..]
    )
}

pub(super) fn floor_char_boundary(value: &str, requested: usize) -> usize {
    let mut boundary = requested.min(value.len());
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

pub(super) fn bounded_test_output(result: &CommandRunResult, max_chars: usize) -> String {
    let combined = if result.stderr.is_empty() {
        result.stdout.clone()
    } else {
        format!("{}\nstderr:\n{}", result.stdout, result.stderr)
    };
    let mut chars = combined.chars();
    let bounded = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{bounded}\n...[test output truncated]")
    } else {
        bounded
    }
}

pub(super) fn print_bounded_lines(content: &str, max_lines: usize) {
    let mut lines = content.lines();
    for line in lines.by_ref().take(max_lines) {
        println!("+ {line}");
    }
    if lines.next().is_some() {
        println!("... [new-file preview truncated; full content remains in the patch proof]");
    }
}

pub(super) fn print_interactive_help() {
    println!("Commands:");
    println!("!help");
    println!("!exit");
    println!("!scan");
    println!("!plan TASK");
    println!("!apply TASK");
    println!("!checkpoints");
    println!("!restore CHECKPOINT_ID");
    println!("!diff");
    println!("!test");
    println!("!explain");
    println!("!model current");
    println!("!model use MODEL");
    println!("!provider current");
    println!("!provider list");
    println!("!provider use PROVIDER");
    println!("!skills");
    println!("!clear");
}

pub(super) fn git_diff_message(
    workspace: &std::path::Path,
    credential_env_names: &[String],
) -> Result<String> {
    if !workspace.join(".git").exists() {
        return Ok("No git repository detected. Git integration is optional for now.".to_string());
    }

    let mut command = hardened_git_diff_command(workspace, credential_env_names);
    const MAX_GIT_DIFF_BYTES: usize = 2 * 1024 * 1024;
    const MAX_GIT_ERROR_BYTES: usize = 64 * 1024;
    let output = run_command_bounded(&mut command, MAX_GIT_DIFF_BYTES, MAX_GIT_ERROR_BYTES)?;
    if !output.status.success() {
        return Ok(format!(
            "git diff failed: {}",
            retained_git_text(&output.stderr, output.stderr_truncated, "git stderr").trim()
        ));
    }

    let diff = retained_git_text(&output.stdout, output.stdout_truncated, "git diff");
    if diff.trim().is_empty() {
        Ok("No git diff.".to_string())
    } else {
        Ok(diff)
    }
}

fn retained_git_text(bytes: &[u8], truncated: bool, label: &str) -> String {
    let mut text = String::from_utf8_lossy(bytes).to_string();
    if truncated {
        text.push_str(&format!("\n...[{label} truncated]"));
    }
    text
}

pub(super) fn hardened_git_diff_command(
    workspace: &std::path::Path,
    credential_env_names: &[String],
) -> Command {
    let mut command = Command::new("git");
    command
        .arg("--no-pager")
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-C")
        .arg(workspace)
        .arg("diff")
        .arg("--no-ext-diff")
        .arg("--no-textconv")
        .arg("--")
        .arg(".")
        .args(SECRET_GIT_PATHSPEC_EXCLUSIONS);
    crate::credentials::scrub_credential_names(&mut command, credential_env_names);
    command
}
