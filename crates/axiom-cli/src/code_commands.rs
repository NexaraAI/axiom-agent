use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
    process::Command,
};

use anyhow::{anyhow, bail, Result};
use axiom_agent::{AgentCaps, AgentLoop, TurnResult, UsageLedger, UsagePricing};
use axiom_coder::{
    build_patch_prompt_with_context, build_plan_prompt, list_checkpoints, parse_axiom_patch,
    prepare_patch, scan_project, sha256_hex, verify_patch_against_plan, verify_prepared_patch,
    AxiomPatch, PatchContextFile, PreparedPatch, ProjectScanSummary, TestCommand,
    WorkspaceCheckpoint,
};
use axiom_core::{
    current_utc_month, now_unix_seconds, run_command_bounded, usd_to_microusd, AxiomConfig,
    CostLedgerEvent, CostLedgerStore, ProviderConfig, Workspace, SECRET_GIT_PATHSPEC_EXCLUSIONS,
};
use axiom_engine::{
    authorize_side_effect, execute_installed_tool_with_policy, load_installed_skills,
    AllowAllApprover, ApprovalRequest, DenyAllApprover, RecordingSideEffectAuditSink,
    SideEffectClass, SideEffectPolicy, SideEffectRequest, SkillApproval, SkillCard,
    SkillExecutionContext, ToolRequest,
};
use axiom_lens::{build_skill_context_message, select_relevant_skills};
use axiom_llm::{
    ChatMessage, CloudflareAiGatewayProvider, LlmProvider, MockProvider, OpenAiCompatibleProvider,
};
use axiom_proof::{
    new_approval, CheckpointProof, CommandProof, FileWriteProof, LensSelectionRecord, PatchProof,
    ProofMode, ProofRecorder, SkillCardProof, TestProof,
};
use serde_json::json;

use crate::{chat, onboarding, startup, CodeCommand};

pub(crate) async fn run(command: CodeCommand) -> Result<()> {
    ensure_onboarding_completed().await?;
    let mut session = CoderSession::load_default()?;

    if command.scan {
        session.print_scan()?;
        return Ok(());
    }
    if command.diff {
        session.print_git_diff()?;
        return Ok(());
    }
    if command.test {
        session.run_detected_tests()?;
        return Ok(());
    }
    if command.explain {
        session.explain_project()?;
        return Ok(());
    }

    let task = command.task.join(" ").trim().to_string();
    if task.is_empty() {
        return session.run_interactive().await;
    }

    if command.apply {
        session.run_apply_task(&task).await
    } else {
        session.run_task_plan(&task, command.plan_only).await
    }
}

pub(crate) async fn run_task_from_chat(task: String) -> Result<()> {
    ensure_onboarding_completed().await?;
    let mut session = CoderSession::load_default()?;
    session.auto_routed_to_coder = true;
    session.run_task_plan(&task, false).await
}

async fn ensure_onboarding_completed() -> Result<()> {
    let config_path = AxiomConfig::default_config_path()?;
    if startup::route_for_config_path(&config_path)? == startup::StartupRoute::Onboarding {
        println!("Onboarding is required before Axiom Coder can start.");
        if chat::confirm("Start onboarding now?", true)? {
            onboarding::run_terminal_onboarding().await?;
            if startup::route_for_config_path(&config_path)? == startup::StartupRoute::Onboarding {
                bail!("onboarding is still incomplete")
            }
        } else {
            bail!("onboarding required")
        }
    }
    Ok(())
}

struct CoderSession {
    config_path: PathBuf,
    config: AxiomConfig,
    cost_session_id: String,
    credential_env_names: Vec<String>,
    identity_system_message: String,
    history: Vec<ChatMessage>,
    auto_routed_to_coder: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandRunResult {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

enum PatchApplyOutcome {
    Passed,
    Unverified(String),
    TestsFailed {
        command: String,
        result: CommandRunResult,
    },
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CoderCostBudget {
    month_utc: String,
    remaining_microusd: Option<u64>,
}

impl CoderSession {
    fn load_default() -> Result<Self> {
        let config_path = AxiomConfig::default_config_path()?;
        Self::load_from_path(config_path)
    }

    fn load_from_path(config_path: PathBuf) -> Result<Self> {
        let config = AxiomConfig::load_from_path(&config_path)?;
        let skills_dir = config_path
            .parent()
            .map(|config_dir| config_dir.join(&config.skills.local_dir))
            .unwrap_or_else(|| PathBuf::from(&config.skills.local_dir));
        let installed_skill_ids: Vec<String> = load_installed_skills(skills_dir)
            .unwrap_or_default()
            .into_iter()
            .filter(|skill| skill.record.is_selectable())
            .map(|skill| skill.manifest.id)
            .collect();
        let credential_env_names = crate::credentials::credential_environment_names(&config)?;
        Ok(Self {
            config_path,
            config,
            cost_session_id: axiom_proof::trace::new_session_id(),
            credential_env_names,
            identity_system_message: crate::identity::system_message(
                "Axiom Agent",
                &installed_skill_ids,
            ),
            history: Vec::new(),
            auto_routed_to_coder: false,
        })
    }

    async fn run_interactive(&mut self) -> Result<()> {
        println!("Axiom Coder");
        println!("workspace: {}", self.workspace_path().display());
        println!(
            "provider: {}",
            self.active_provider().unwrap_or("not configured")
        );
        println!("model: {}", self.active_model().unwrap_or("not configured"));
        println!("approval mode: {}", self.config.coder.approval_mode);
        println!("Type !help for coding commands or !exit to leave.");

        loop {
            print!("axiom-code> ");
            io::stdout().flush()?;

            let mut input = String::new();
            let bytes = io::stdin().read_line(&mut input)?;
            if bytes == 0 {
                println!();
                break;
            }

            let trimmed = input.trim();
            if trimmed.is_empty() {
                continue;
            }

            match self.handle_interactive_command(trimmed).await? {
                InteractiveResult::Continue => continue,
                InteractiveResult::Exit => break,
                InteractiveResult::NotCommand => self.send_coder_message(trimmed).await?,
            }
        }

        Ok(())
    }

    async fn handle_interactive_command(&mut self, input: &str) -> Result<InteractiveResult> {
        if !input.starts_with('!') {
            return Ok(InteractiveResult::NotCommand);
        }

        match input {
            "!exit" => Ok(InteractiveResult::Exit),
            "!help" => {
                print_interactive_help();
                Ok(InteractiveResult::Continue)
            }
            "!scan" => {
                self.print_scan()?;
                Ok(InteractiveResult::Continue)
            }
            "!diff" => {
                self.print_git_diff()?;
                Ok(InteractiveResult::Continue)
            }
            "!test" => {
                self.run_detected_tests()?;
                Ok(InteractiveResult::Continue)
            }
            "!explain" => {
                self.explain_project()?;
                Ok(InteractiveResult::Continue)
            }
            "!checkpoints" => {
                self.print_checkpoints()?;
                Ok(InteractiveResult::Continue)
            }
            "!model current" => {
                println!(
                    "Current model: {}",
                    self.active_model().unwrap_or("not configured")
                );
                Ok(InteractiveResult::Continue)
            }
            "!provider current" => {
                println!(
                    "Current provider: {}",
                    self.active_provider().unwrap_or("not configured")
                );
                Ok(InteractiveResult::Continue)
            }
            "!provider list" => {
                let providers = self.provider_names();
                if providers.is_empty() {
                    println!("No providers configured.");
                } else {
                    println!("Configured providers:");
                    for provider in providers {
                        let marker = if Some(provider.as_str()) == self.active_provider() {
                            "*"
                        } else {
                            "-"
                        };
                        println!("{marker} {provider}");
                    }
                }
                Ok(InteractiveResult::Continue)
            }
            "!skills" => {
                let cards = self.installed_skill_cards()?;
                if cards.is_empty() {
                    println!("No enabled skills installed.");
                } else {
                    println!("Installed enabled skills:");
                    for card in cards {
                        println!("- {}: {}", card.id, card.summary);
                    }
                }
                Ok(InteractiveResult::Continue)
            }
            "!clear" => {
                self.history.clear();
                println!("Coder conversation cleared.");
                Ok(InteractiveResult::Continue)
            }
            _ if input.starts_with("!plan ") => {
                let task = input.trim_start_matches("!plan ").trim();
                self.run_task_plan(task, true).await?;
                Ok(InteractiveResult::Continue)
            }
            _ if input.starts_with("!apply ") => {
                let task = input.trim_start_matches("!apply ").trim();
                self.run_apply_task(task).await?;
                Ok(InteractiveResult::Continue)
            }
            _ if input.starts_with("!restore ") => {
                let checkpoint = input.trim_start_matches("!restore ").trim();
                self.restore_checkpoint(checkpoint)?;
                Ok(InteractiveResult::Continue)
            }
            _ if input.starts_with("!model use ") => {
                let model = input.trim_start_matches("!model use ").trim();
                match self.set_model(model) {
                    Ok(model) => println!("Model switched to {model}."),
                    Err(error) => println!("Model switch failed: {error}"),
                }
                Ok(InteractiveResult::Continue)
            }
            _ if input.starts_with("!provider use ") => {
                let provider = input.trim_start_matches("!provider use ").trim();
                match self.set_provider(provider) {
                    Ok(provider) => println!("Provider switched to {provider}."),
                    Err(error) => println!("Provider switch failed: {error}"),
                }
                Ok(InteractiveResult::Continue)
            }
            _ => {
                println!("Unknown command. Type !help for commands.");
                Ok(InteractiveResult::Continue)
            }
        }
    }

    async fn run_task_plan(&mut self, task: &str, plan_only: bool) -> Result<()> {
        let mut proof = self.start_proof_trace(task);
        let scan = self.scan(&mut proof)?;
        print_scan_summary(&scan);
        let mut plan = self.request_plan(task, &scan, Some(&mut proof)).await?;
        print_plan(&plan);

        if plan_only {
            proof.set_final_response(&plan);
            proof.finish_trace("coder plan-only completed without file changes");
            let _ = proof.export();
            return Ok(());
        }

        loop {
            match prompt_apply_choice()? {
                ApplyChoice::Apply => {
                    proof.record_approval(new_approval(
                        "apply_plan",
                        "medium",
                        "Apply changes?",
                        "approved",
                    ));
                    return self
                        .run_apply_task_with_plan(task, &scan, &plan, &mut proof)
                        .await;
                }
                ApplyChoice::Edit => {
                    proof.record_approval(new_approval(
                        "apply_plan",
                        "medium",
                        "Apply changes?",
                        "edit",
                    ));
                    let revision = prompt_plan_revision()?;
                    let revised_task = format!(
                        "{task}\n\nRevise the plan using this user feedback:\n{revision}\n\nPrevious plan:\n{plan}"
                    );
                    plan = self
                        .request_plan(&revised_task, &scan, Some(&mut proof))
                        .await?;
                    print_plan(&plan);
                }
                ApplyChoice::Cancel => {
                    proof.record_approval(new_approval(
                        "apply_plan",
                        "medium",
                        "Apply changes?",
                        "cancelled",
                    ));
                    proof.cancel_trace("coder plan cancelled before patch generation");
                    let _ = proof.export();
                    println!("Cancelled.");
                    return Ok(());
                }
            }
        }
    }

    async fn run_apply_task(&mut self, task: &str) -> Result<()> {
        let mut proof = self.start_proof_trace(task);
        let scan = self.scan(&mut proof)?;
        let plan = self.request_plan(task, &scan, Some(&mut proof)).await?;
        print_plan(&plan);
        self.run_apply_task_with_plan(task, &scan, &plan, &mut proof)
            .await
    }

    async fn run_apply_task_with_plan(
        &mut self,
        task: &str,
        scan: &ProjectScanSummary,
        plan: &str,
        proof: &mut ProofRecorder,
    ) -> Result<()> {
        if !chat::confirm("Request patch from model?", true)? {
            proof.record_approval(new_approval(
                "request_patch",
                "medium",
                "Request patch from model?",
                "denied",
            ));
            proof.cancel_trace("cancelled before patch generation");
            let _ = proof.export();
            println!("Cancelled before patch generation.");
            return Ok(());
        }
        proof.record_approval(new_approval(
            "request_patch",
            "medium",
            "Request patch from model?",
            "approved",
        ));

        let mut patch_text = self.request_patch(task, scan, plan).await?;
        let mut correction_attempt = 0_u32;
        loop {
            let patch = match parse_axiom_patch(&patch_text) {
                Ok(patch) => patch,
                Err(error) => {
                    proof.record_error("patch", error.to_string(), "parse_patch", true);
                    proof.fail_trace("patch parsing failed", "parse_patch");
                    let _ = proof.export();
                    return Err(error.into());
                }
            };
            let plan_review = verify_patch_against_plan(task, plan, &patch);
            if plan_review.no_op_hunks > 0 {
                let message = format!(
                    "patch contains {} no-op hunk(s); regenerate a minimal patch before approval",
                    plan_review.no_op_hunks
                );
                proof.record_error("plan_patch_verification", &message, "review_patch", true);
                proof.fail_trace("plan-to-patch verification failed", "review_patch");
                let _ = proof.export();
                bail!(message);
            }
            if !plan_review.uncovered_paths.is_empty() {
                let paths = plan_review.uncovered_paths.join(", ");
                let prompt = format!(
                    "Patch includes path(s) not named in the approved task or plan: {paths}. Approve expanded scope?"
                );
                let approved = chat::confirm(&prompt, false)?;
                proof.record_approval(new_approval(
                    "expand_plan_patch_scope",
                    "high",
                    &prompt,
                    if approved { "approved" } else { "denied" },
                ));
                if !approved {
                    proof.cancel_trace("patch paths exceeded the approved plan scope");
                    let _ = proof.export();
                    println!("Cancelled: patch exceeded the approved plan scope.");
                    return Ok(());
                }
            }
            let default_test = scan.likely_test_commands.first();
            match self
                .apply_patch_after_confirmation(&patch, default_test, proof)
                .await?
            {
                PatchApplyOutcome::Passed => {
                    self.finish_apply_trace(&patch, proof, "tests passed");
                    return Ok(());
                }
                PatchApplyOutcome::Unverified(reason) => {
                    self.finish_apply_trace(&patch, proof, &reason);
                    return Ok(());
                }
                PatchApplyOutcome::Cancelled => return Ok(()),
                PatchApplyOutcome::TestsFailed { command, result } => {
                    if correction_attempt >= self.config.coder.max_correction_attempts {
                        let message = format!(
                            "tests still fail after {correction_attempt} correction attempts: `{command}` exited {:?}. Recovery checkpoints remain available.",
                            result.exit_code
                        );
                        proof.record_error(
                            "max_corrections_reached",
                            &message,
                            "test_correction",
                            false,
                        );
                        proof.set_final_response(&message);
                        proof.fail_trace("maximum correction attempts reached", "test_correction");
                        let _ = proof.export();
                        bail!(message);
                    }
                    correction_attempt = correction_attempt.saturating_add(1);
                    println!(
                        "Tests failed. Requesting correction {}/{}.",
                        correction_attempt, self.config.coder.max_correction_attempts
                    );
                    patch_text = self
                        .request_correction_patch(
                            task,
                            scan,
                            plan,
                            &patch,
                            &command,
                            &result,
                            correction_attempt,
                        )
                        .await?;
                }
            }
        }
    }

    async fn apply_patch_after_confirmation(
        &mut self,
        patch: &AxiomPatch,
        default_test_command: Option<&TestCommand>,
        proof: &mut ProofRecorder,
    ) -> Result<PatchApplyOutcome> {
        if self.is_manual_approval()
            && !chat::confirm("Read existing files to build diff preview?", true)?
        {
            proof.record_approval(new_approval(
                "read_diff_inputs",
                "low",
                "Read existing files to build diff preview?",
                "denied",
            ));
            proof.cancel_trace("cancelled before diff preview");
            let _ = proof.export();
            bail!("cancelled before diff preview")
        }
        let prepared = prepare_patch(patch, self.workspace_path())?;
        let (patch_files, patch_bytes) = patch_scope(&prepared);
        if patch_files > self.config.coder.max_patch_files
            || patch_bytes > self.config.coder.max_patch_bytes
        {
            let message = format!(
                "patch scope {patch_files} files / {patch_bytes} bytes exceeds configured hard limit {} files / {} bytes",
                self.config.coder.max_patch_files, self.config.coder.max_patch_bytes
            );
            proof.record_error("patch_scope", &message, "review_patch", false);
            proof.fail_trace("patch exceeded configured hard scope", "review_patch");
            let _ = proof.export();
            bail!(message);
        }
        if patch_files > self.config.coder.scope_confirmation_files
            || patch_bytes > self.config.coder.scope_confirmation_bytes
        {
            let prompt = format!(
                "Patch expands to {patch_files} files / {patch_bytes} bytes. Approve this high scope?"
            );
            let approved = chat::confirm(&prompt, false)?;
            proof.record_approval(new_approval(
                "approve_high_patch_scope",
                "high",
                &prompt,
                if approved { "approved" } else { "denied" },
            ));
            if !approved {
                proof.cancel_trace("high patch scope was not approved");
                let _ = proof.export();
                println!("Cancelled before broad patch preview.");
                return Ok(PatchApplyOutcome::Cancelled);
            }
        }
        println!("Patch summary: {}", patch.summary);
        println!("{}", prepared.diff);

        if self.is_manual_approval() && !self.review_patch_units(patch, proof)? {
            proof.cancel_trace("an individual patch unit was not approved");
            let _ = proof.export();
            println!("Cancelled before writing files.");
            return Ok(PatchApplyOutcome::Cancelled);
        }

        if !chat::confirm("Apply these file changes?", false)? {
            proof.record_approval(new_approval(
                "apply_patch",
                "medium",
                "Apply these file changes?",
                "denied",
            ));
            proof.record_patch(PatchProof {
                event_id: axiom_proof::trace::new_event_id("patch"),
                summary: patch.summary.clone(),
                changed_files: patch
                    .changes
                    .iter()
                    .map(|change| change.path.clone())
                    .collect(),
                diff: prepared.diff,
                approved: false,
                applied: false,
            });
            proof.cancel_trace("cancelled before writing files");
            let _ = proof.export();
            println!("Cancelled before writing files.");
            return Ok(PatchApplyOutcome::Cancelled);
        }

        proof.record_approval(new_approval(
            "apply_patch",
            "medium",
            "Apply these file changes?",
            "approved",
        ));
        verify_prepared_patch(&prepared, self.workspace_path())?;
        let checkpoint_paths = prepared
            .files
            .iter()
            .map(|file| file.path.clone())
            .collect::<Vec<_>>();
        let checkpoint = WorkspaceCheckpoint::create(
            self.workspace_path(),
            self.checkpoints_dir(),
            &checkpoint_paths,
        )?;
        proof.record_checkpoint(checkpoint_proof(
            &checkpoint,
            false,
            "created before patch apply",
        ));
        println!("Recovery checkpoint: {}", checkpoint.id);
        verify_prepared_patch(&prepared, self.workspace_path())?;
        if let Err(write_error) = self.write_patch_through_engine(&prepared, proof).await {
            match checkpoint.restore(self.workspace_path()) {
                Ok(()) => {
                    proof.record_checkpoint(checkpoint_proof(
                        &checkpoint,
                        true,
                        "automatic rollback after patch write failure",
                    ));
                    proof.record_error(
                        "patch_write",
                        write_error.to_string(),
                        "apply_patch",
                        false,
                    );
                    proof.fail_trace(
                        "patch write failed and checkpoint was restored",
                        "apply_patch",
                    );
                    let _ = proof.export();
                    return Err(write_error);
                }
                Err(restore_error) => {
                    proof.record_error(
                        "checkpoint_restore",
                        restore_error.to_string(),
                        "apply_patch",
                        false,
                    );
                    proof.fail_trace(
                        "patch write and automatic checkpoint restore failed",
                        "apply_patch",
                    );
                    let _ = proof.export();
                    bail!(
                        "patch write failed: {write_error}; automatic checkpoint restore also failed: {restore_error}"
                    );
                }
            }
        }
        proof.record_patch(PatchProof {
            event_id: axiom_proof::trace::new_event_id("patch"),
            summary: patch.summary.clone(),
            changed_files: patch
                .changes
                .iter()
                .map(|change| change.path.clone())
                .collect(),
            diff: prepared.diff.clone(),
            approved: true,
            applied: true,
        });

        println!("Changed files:");
        for change in &prepared.files {
            println!("- {}", change.path);
        }

        let (command, working_directory) = match prepared.test_command.as_deref() {
            Some(command) => (command, None),
            None => match default_test_command {
                Some(command) => (
                    command.command.as_str(),
                    command.working_directory.as_deref(),
                ),
                None => {
                    return Ok(PatchApplyOutcome::Unverified(
                        "no test command was available".to_string(),
                    ));
                }
            },
        };
        if !is_safe_test_command(command) {
            proof.record_error(
                "unsafe_test_command",
                format!("model proposed blocked test command: {command}"),
                "run_test",
                true,
            );
            return Ok(PatchApplyOutcome::Unverified(format!(
                "test command `{command}` was blocked by policy"
            )));
        }
        let (result, command_cwd) = self.run_command_in(proof, command, working_directory)?;
        proof.record_command(command_proof(command, command_cwd, &result, true));
        proof.record_test(test_proof(command, true, true, &result));
        if result.exit_code == Some(0) {
            Ok(PatchApplyOutcome::Passed)
        } else {
            Ok(PatchApplyOutcome::TestsFailed {
                command: command.to_string(),
                result,
            })
        }
    }

    fn review_patch_units(&self, patch: &AxiomPatch, proof: &mut ProofRecorder) -> Result<bool> {
        let review_units = patch
            .changes
            .iter()
            .map(|change| {
                change
                    .hunks
                    .len()
                    .max(usize::from(change.content.is_some()))
            })
            .sum::<usize>();
        println!("Manual review: {review_units} patch unit(s). Denying one cancels the patch.");

        for change in &patch.changes {
            if let Some(content) = &change.content {
                println!("\nCreate {}:", change.path);
                print_bounded_lines(content, 60);
                let prompt = format!("Approve new-file content for `{}`?", change.path);
                let approved = chat::confirm(&prompt, false)?;
                proof.record_approval(new_approval(
                    format!("review_file:{}", change.path),
                    "medium",
                    &prompt,
                    if approved { "approved" } else { "denied" },
                ));
                if !approved {
                    return Ok(false);
                }
            }
            for (index, hunk) in change.hunks.iter().enumerate() {
                println!(
                    "\n{} hunk {} at original line {}:",
                    change.path,
                    index + 1,
                    hunk.old_start
                );
                for line in &hunk.old_lines {
                    println!("- {line}");
                }
                for line in &hunk.new_lines {
                    println!("+ {line}");
                }
                let prompt = format!("Approve hunk {} for `{}`?", index + 1, change.path);
                let approved = chat::confirm(&prompt, false)?;
                proof.record_approval(new_approval(
                    format!("review_hunk:{}:{}", change.path, index + 1),
                    "medium",
                    &prompt,
                    if approved { "approved" } else { "denied" },
                ));
                if !approved {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    async fn write_patch_through_engine(
        &self,
        patch: &PreparedPatch,
        proof: &mut ProofRecorder,
    ) -> Result<()> {
        let installed_skills = load_installed_skills(self.skills_dir())?;
        let execution_context = self.execution_context();
        let mut approval = AllowAllApprover;
        let policy = self.side_effect_policy()?;

        for change in &patch.files {
            let target = self.workspace_path().join(&change.path);
            let existed = target.exists();
            let request = ToolRequest {
                skill_id: "file.write".to_string(),
                arguments: json!({
                    "path": change.path,
                    "content": change.content,
                }),
            };
            let mut audit = RecordingSideEffectAuditSink::default();
            execute_installed_tool_with_policy(
                &request,
                &installed_skills,
                &execution_context,
                &mut approval,
                &policy,
                &mut audit,
            )
            .await?;
            crate::side_effects::record_audit(proof, audit);
            proof.record_file_write(FileWriteProof {
                event_id: axiom_proof::trace::new_event_id("write"),
                path: change.path.clone(),
                bytes_written: Some(change.content.len() as u64),
                created: !existed,
                overwrote: existed,
                approved: true,
                diff_summary: Some("applied through Axiom Engine file.write".to_string()),
            });
        }

        Ok(())
    }

    async fn request_plan(
        &self,
        task: &str,
        scan: &ProjectScanSummary,
        proof: Option<&mut ProofRecorder>,
    ) -> Result<String> {
        let cards = self.select_skill_cards(task, 5)?;
        if !cards.is_empty() {
            println!(
                "Axiom Lens: selected {}",
                cards
                    .iter()
                    .map(|card| card.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        if let Some(proof) = proof {
            self.record_proof_lens(proof, &cards)?;
        }
        let skill_context = build_skill_context_message(&cards).unwrap_or_default();
        let prompt = build_plan_prompt(task, scan, &skill_context);
        self.llm_chat(vec![ChatMessage {
            role: "user".to_string(),
            content: prompt,
        }])
        .await
    }

    async fn request_patch(
        &self,
        task: &str,
        scan: &ProjectScanSummary,
        plan: &str,
    ) -> Result<String> {
        let context = self.patch_context(scan)?;
        let prompt = build_patch_prompt_with_context(task, scan, plan, &context);
        self.llm_chat(vec![ChatMessage {
            role: "user".to_string(),
            content: prompt,
        }])
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn request_correction_patch(
        &self,
        task: &str,
        scan: &ProjectScanSummary,
        plan: &str,
        previous_patch: &AxiomPatch,
        command: &str,
        result: &CommandRunResult,
        attempt: u32,
    ) -> Result<String> {
        let output = bounded_test_output(result, 8_000);
        let correction_plan = format!(
            "{plan}\n\nCorrection attempt {attempt}. The previous patch `{}` was applied, but `{command}` failed with exit {:?}. Diagnose the failure from the untrusted test output below and return the smallest correction patch against the CURRENT workspace hashes. Do not repeat unrelated changes.\n\n<untrusted-test-output>\n{output}\n</untrusted-test-output>",
            previous_patch.summary, result.exit_code
        );
        let context = self.patch_context(scan)?;
        let prompt = build_patch_prompt_with_context(task, scan, &correction_plan, &context);
        self.llm_chat(vec![ChatMessage {
            role: "user".to_string(),
            content: prompt,
        }])
        .await
    }

    fn finish_apply_trace(&self, patch: &AxiomPatch, proof: &mut ProofRecorder, result: &str) {
        let changed_files = patch
            .changes
            .iter()
            .map(|change| change.path.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        proof.set_final_response(format!(
            "Applied patch. Changed files: {changed_files}. Verification: {result}."
        ));
        proof.finish_trace(format!("coder apply completed: {result}"));
        let _ = proof.export();
    }

    fn patch_context(&self, scan: &ProjectScanSummary) -> Result<Vec<PatchContextFile>> {
        const MAX_CONTEXT_FILES: usize = 12;
        const MAX_CONTEXT_BYTES: usize = 200_000;

        let workspace = Workspace::new(self.workspace_path())?;
        let configured_budget = usize::try_from(self.config.coder.max_file_read_bytes)
            .unwrap_or(usize::MAX)
            .min(MAX_CONTEXT_BYTES);
        let mut remaining = configured_budget;
        let mut context = Vec::new();
        for path in scan.important_files.iter().take(MAX_CONTEXT_FILES) {
            if remaining == 0 {
                break;
            }
            let resolved = workspace.resolve_inside(path)?;
            if !resolved.is_file() {
                continue;
            }
            let bytes = fs::read(&resolved)?;
            let Ok(full_content) = std::str::from_utf8(&bytes) else {
                continue;
            };
            let take = remaining.min(bytes.len());
            let boundary = floor_char_boundary(full_content, take);
            let content = full_content[..boundary].to_string();
            let truncated = boundary < full_content.len();
            remaining = remaining.saturating_sub(boundary);
            context.push(PatchContextFile {
                path: path.clone(),
                sha256: sha256_hex(&bytes),
                content,
                truncated,
            });
        }
        Ok(context)
    }

    fn checkpoints_dir(&self) -> PathBuf {
        self.config_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join("checkpoints")
    }

    fn print_checkpoints(&self) -> Result<()> {
        let checkpoints = list_checkpoints(self.checkpoints_dir())?;
        if checkpoints.is_empty() {
            println!("No recovery checkpoints.");
        } else {
            println!("Recovery checkpoints:");
            for checkpoint in checkpoints {
                println!(
                    "- {} ({} files, workspace {})",
                    checkpoint.id,
                    checkpoint.files.len(),
                    checkpoint.workspace
                );
            }
        }
        Ok(())
    }

    fn restore_checkpoint(&self, checkpoint_id: &str) -> Result<()> {
        if checkpoint_id.is_empty()
            || !checkpoint_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            bail!("invalid checkpoint id")
        }
        let checkpoint = WorkspaceCheckpoint::load(self.checkpoints_dir().join(checkpoint_id))?;
        if !chat::confirm(
            &format!(
                "Restore checkpoint `{}` over {} tracked files?",
                checkpoint.id,
                checkpoint.files.len()
            ),
            false,
        )? {
            println!("Checkpoint restore cancelled.");
            return Ok(());
        }
        let mut proof = self.start_proof_trace(&format!("restore checkpoint {}", checkpoint.id));
        proof.record_approval(new_approval(
            "restore_checkpoint",
            "high",
            format!("Restore checkpoint `{}`?", checkpoint.id),
            "approved",
        ));
        checkpoint.restore(self.workspace_path())?;
        proof.record_checkpoint(checkpoint_proof(
            &checkpoint,
            true,
            "manual restore requested by user",
        ));
        proof.set_final_response(format!("Restored checkpoint {}", checkpoint.id));
        proof.finish_trace("checkpoint restored");
        let _ = proof.export();
        println!("Restored checkpoint {}.", checkpoint.id);
        Ok(())
    }

    async fn send_coder_message(&mut self, input: &str) -> Result<()> {
        let mut proof = self.start_proof_trace(input);
        let scan = self.scan(&mut proof)?;
        let prompt = format!(
            "Answer as Axiom Coder. Workspace: {}\nProject type: {}\nUser: {}",
            scan.root, scan.project_type, input
        );
        let user = ChatMessage {
            role: "user".to_string(),
            content: prompt,
        };
        let mut messages = self.history.clone();
        messages.push(user.clone());
        let response = self.llm_chat(messages).await?;
        self.history.push(user);
        self.history.push(ChatMessage {
            role: "assistant".to_string(),
            content: response.clone(),
        });
        proof.set_final_response(&response);
        proof.finish_trace("coder response completed");
        let _ = proof.export();
        println!("Axiom Coder: {response}");
        Ok(())
    }

    async fn llm_chat(&self, messages: Vec<ChatMessage>) -> Result<String> {
        let model = self
            .active_model()
            .ok_or_else(|| anyhow!("no active model configured. Use `!model use <model>`."))?
            .to_string();
        let provider_name = self
            .active_provider()
            .ok_or_else(|| anyhow!("no active provider configured. Use `!provider use <name>`."))?
            .to_string();
        let cost_budget = self.prepare_model_cost_budget()?;
        let provider = self.build_provider(&provider_name)?;
        let mut history = messages;
        let user_message = history
            .pop()
            .ok_or_else(|| anyhow!("coder model request has no user message"))?;
        if user_message.role != "user" {
            bail!("coder model request must end with a user message")
        }
        let system_messages = vec![ChatMessage {
            role: "system".to_string(),
            content: self.identity_system_message.clone(),
        }];
        let installed_skills = load_installed_skills(self.skills_dir())?;
        let mut approval = DenyAllApprover;
        let mut agent = AgentLoop::new(
            provider.as_ref(),
            model.clone(),
            self.agent_caps(cost_budget.remaining_microusd),
            system_messages,
            history,
            &installed_skills,
            self.execution_context(),
            &mut approval,
        )
        .with_tools_enabled(false)
        .with_generation_options(Some(0.2), None)
        .with_pricing(self.usage_pricing())
        .with_streaming(self.config.llm.stream);

        let (completion, give_up_reason) = match agent.run_turn(user_message).await? {
            TurnResult::Done(completion) => (completion, None),
            TurnResult::GiveUp {
                reason, completion, ..
            } => (completion, Some(reason)),
        };
        self.record_model_cost(
            axiom_proof::trace::new_event_id("coder-cost"),
            &cost_budget.month_utc,
            &provider_name,
            &model,
            &completion.ledger,
        )?;
        if let Some(reason) = give_up_reason {
            bail!(
                "Axiom Coder stopped at the shared runtime boundary ({reason:?}): {}",
                completion.content
            )
        }
        Ok(completion.content)
    }

    fn agent_caps(&self, remaining_microusd: Option<u64>) -> AgentCaps {
        let mut caps = AgentCaps {
            max_iterations: self.config.agent.max_iterations,
            max_tool_iterations: self.config.agent.max_tool_iterations,
            max_tokens: self.config.agent.max_tokens,
            max_cost_usd: self.config.agent.max_cost_usd,
            max_wall_seconds: self.config.agent.max_wall_seconds,
            max_consecutive_tool_errors: self.config.agent.max_consecutive_tool_errors,
        };
        if let Some(remaining) = remaining_microusd {
            caps.max_cost_usd = caps.max_cost_usd.min(remaining as f64 / 1_000_000.0);
        }
        caps
    }

    fn usage_pricing(&self) -> UsagePricing {
        UsagePricing::new(
            self.config.agent.input_cost_per_million_tokens,
            self.config.agent.output_cost_per_million_tokens,
        )
    }

    fn prepare_model_cost_budget(&self) -> Result<CoderCostBudget> {
        let month_utc = current_utc_month();
        let pricing = self.usage_pricing();
        let configured = self.config.agent.session_budget_usd.is_some()
            || self.config.agent.monthly_budget_usd.is_some();
        if !configured || !pricing.is_complete() {
            return Ok(CoderCostBudget {
                month_utc,
                remaining_microusd: None,
            });
        }

        let ledger = self.cost_ledger_store().load()?;
        let status = ledger.budget_status(
            &self.cost_session_id,
            &month_utc,
            self.config
                .agent
                .session_budget_usd
                .and_then(usd_to_microusd),
            self.config
                .agent
                .monthly_budget_usd
                .and_then(usd_to_microusd),
        );
        if status.is_exhausted() {
            let mut exhausted = Vec::new();
            if status
                .session_budget_microusd
                .is_some_and(|budget| status.session_spent_microusd >= budget)
            {
                exhausted.push("session");
            }
            if status
                .monthly_budget_microusd
                .is_some_and(|budget| status.monthly_spent_microusd >= budget)
            {
                exhausted.push("monthly");
            }
            bail!(
                "persistent {} cost budget reached; no Coder provider call was made. Run `axiom cost` for details.",
                exhausted.join(" and ")
            );
        }

        Ok(CoderCostBudget {
            month_utc,
            remaining_microusd: status.remaining_microusd,
        })
    }

    fn record_model_cost(
        &self,
        event_id: String,
        month_utc: &str,
        provider: &str,
        model: &str,
        usage: &UsageLedger,
    ) -> Result<Option<u64>> {
        let Some(cost_microusd) = usage.estimated_cost_microusd(self.usage_pricing()) else {
            return Ok(None);
        };
        self.cost_ledger_store().record(CostLedgerEvent {
            event_id,
            session_id: self.cost_session_id.clone(),
            month_utc: month_utc.to_string(),
            recorded_at_unix_seconds: now_unix_seconds(),
            cost_microusd,
            prompt_tokens: usage.prompt_tokens,
            completion_tokens: usage.completion_tokens,
            provider: provider.to_string(),
            model: model.to_string(),
        })?;
        Ok(Some(cost_microusd))
    }

    fn cost_ledger_store(&self) -> CostLedgerStore {
        CostLedgerStore::new(crate::cost_commands::cost_ledger_path(&self.config_path))
    }

    fn side_effect_policy(&self) -> Result<SideEffectPolicy> {
        crate::side_effects::configured_policy(&self.config)
    }

    fn authorize_coder_side_effect(
        &self,
        proof: &mut ProofRecorder,
        request: SideEffectRequest,
    ) -> Result<()> {
        let operation = request.operation.clone();
        let policy = self.side_effect_policy()?;
        let mut audit = RecordingSideEffectAuditSink::default();
        let result = {
            let mut approval = CoderPolicyApprover { proof };
            authorize_side_effect(&policy, &mut audit, &mut approval, request)
        };
        crate::side_effects::record_audit(proof, audit);

        match result {
            Ok(()) => {
                // Persist the authorization barrier before the side effect.
                let _ = proof.export();
                Ok(())
            }
            Err(error) => {
                proof.record_error(
                    "side_effect_policy",
                    error.to_string(),
                    operation.clone(),
                    false,
                );
                proof.fail_trace("side effect denied by configured policy", operation);
                let _ = proof.export();
                Err(error.into())
            }
        }
    }

    fn print_scan(&self) -> Result<()> {
        let mut proof = self.start_proof_trace("scan workspace");
        let scan = self.scan(&mut proof)?;
        print_scan_summary(&scan);
        proof.set_final_response(format!("Scanned {} workspace files.", scan.files.len()));
        proof.finish_trace("workspace scan completed");
        let _ = proof.export();
        Ok(())
    }

    fn explain_project(&self) -> Result<()> {
        let mut proof = self.start_proof_trace("explain project");
        let scan = self.scan(&mut proof)?;
        print_scan_summary(&scan);
        println!("Important files:");
        for file in &scan.important_files {
            println!("- {file}");
        }
        if scan.important_files.is_empty() {
            println!("- none detected");
        }
        proof.set_final_response(format!(
            "Explained {} project with {} scanned files.",
            scan.project_type,
            scan.files.len()
        ));
        proof.finish_trace("project explanation completed");
        let _ = proof.export();
        Ok(())
    }

    fn scan(&self, proof: &mut ProofRecorder) -> Result<ProjectScanSummary> {
        self.authorize_coder_side_effect(
            proof,
            SideEffectRequest::new(
                "coder.scan",
                "project.scan",
                [SideEffectClass::FilesystemRead],
                Some(self.workspace_path().display().to_string()),
            ),
        )?;
        match scan_project(self.workspace_path(), 4) {
            Ok(scan) => Ok(scan),
            Err(error) => {
                proof.record_error("project_scan", error.to_string(), "project.scan", false);
                proof.fail_trace("workspace scan failed", "project.scan");
                let _ = proof.export();
                Err(error.into())
            }
        }
    }

    fn print_git_diff(&self) -> Result<()> {
        let mut proof = self.start_proof_trace("show git diff");
        self.authorize_coder_side_effect(
            &mut proof,
            SideEffectRequest::new(
                "coder.git",
                "git.diff",
                [
                    SideEffectClass::FilesystemRead,
                    SideEffectClass::Process,
                    SideEffectClass::Git,
                ],
                Some(self.workspace_path().display().to_string()),
            ),
        )?;
        let message = git_diff_message(&self.workspace_path(), &self.credential_env_names)?;
        println!("{message}");
        proof.set_final_response(&message);
        proof.finish_trace("git diff completed");
        let _ = proof.export();
        Ok(())
    }

    fn run_detected_tests(&self) -> Result<()> {
        let mut proof = self.start_proof_trace("run detected tests");
        let scan = self.scan(&mut proof)?;
        if scan.likely_test_commands.is_empty() {
            proof.record_test(TestProof {
                event_id: axiom_proof::trace::new_event_id("test"),
                detected_command: "none".to_string(),
                ran: false,
                approved: false,
                exit_code: None,
                passed: None,
                output_summary: Some("No obvious test command detected.".to_string()),
            });
            proof.finish_trace("no obvious test command detected");
            let _ = proof.export();
            println!("No obvious test command detected.");
            return Ok(());
        }

        let command = choose_test_command(&scan.likely_test_commands)?;
        let (result, command_cwd) = self.run_command_in(
            &mut proof,
            &command.command,
            command.working_directory.as_deref(),
        )?;
        proof.record_command(command_proof(&command.command, command_cwd, &result, true));
        proof.record_test(test_proof(&command.command, true, true, &result));
        proof.finish_trace("test command completed");
        let _ = proof.export();
        Ok(())
    }

    fn run_command_in(
        &self,
        proof: &mut ProofRecorder,
        command: &str,
        working_directory: Option<&str>,
    ) -> Result<(CommandRunResult, PathBuf)> {
        if !is_safe_test_command(command) {
            bail!("refusing to run unsupported command: {command}");
        }
        self.authorize_coder_side_effect(
            proof,
            SideEffectRequest::new(
                "coder.test",
                "test.run",
                [SideEffectClass::Process],
                Some(format!(
                    "{} @ {}",
                    command,
                    working_directory.unwrap_or(".")
                )),
            ),
        )?;

        let workspace = Workspace::new(self.workspace_path())?;
        let command_cwd = match working_directory {
            Some(directory) => workspace.resolve_inside(directory)?,
            None => workspace.root().to_path_buf(),
        };
        if !command_cwd.is_dir() {
            bail!(
                "test working directory does not exist: {}",
                command_cwd.display()
            );
        }

        let parts = command.split_whitespace().collect::<Vec<_>>();
        let Some((program, args)) = parts.split_first() else {
            bail!("empty command")
        };
        let mut child = Command::new(program);
        child.args(args).current_dir(&command_cwd);
        crate::credentials::scrub_credential_names(&mut child, &self.credential_env_names);
        let output = child.output()?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if !output.stdout.is_empty() {
            println!("{stdout}");
        }
        if !output.stderr.is_empty() {
            eprintln!("{stderr}");
        }
        println!("command exit: {}", output.status);
        Ok((
            CommandRunResult {
                exit_code: output.status.code(),
                stdout,
                stderr,
            },
            command_cwd,
        ))
    }

    fn installed_skill_cards(&self) -> Result<Vec<SkillCard>> {
        Ok(load_installed_skills(self.skills_dir())?
            .into_iter()
            .filter(|skill| skill.record.enabled)
            .map(|skill| skill.manifest.to_skill_card())
            .collect())
    }

    fn select_skill_cards(&self, prompt: &str, max_cards: usize) -> Result<Vec<SkillCard>> {
        Ok(select_relevant_skills(
            prompt,
            &load_installed_skills(self.skills_dir())?,
            max_cards,
        ))
    }

    fn provider_names(&self) -> Vec<String> {
        self.config.providers.keys().cloned().collect()
    }

    fn active_provider(&self) -> Option<&str> {
        self.config.llm.active_provider.as_deref()
    }

    fn active_model(&self) -> Option<&str> {
        self.config.llm.active_model.as_deref()
    }

    fn set_model(&mut self, model: impl Into<String>) -> Result<String> {
        let model = model.into();
        if model.trim().is_empty() {
            bail!("model name cannot be empty");
        }
        self.config.llm.active_model = Some(model.clone());
        if let Some(provider) = self.config.llm.active_provider.clone() {
            self.config
                .llm
                .provider_models
                .insert(provider, model.clone());
        }
        self.config.save_to_path(&self.config_path)?;
        Ok(model)
    }

    fn set_provider(&mut self, provider_name: impl Into<String>) -> Result<String> {
        let provider_name = provider_name.into();
        if !self.config.providers.contains_key(&provider_name) {
            bail!("provider is not configured: {provider_name}");
        }
        self.config.llm.active_provider = Some(provider_name.clone());
        if let Some(model) = self.config.llm.provider_models.get(&provider_name) {
            self.config.llm.active_model = Some(model.clone());
        }
        self.config.save_to_path(&self.config_path)?;
        Ok(provider_name)
    }

    fn start_proof_trace(&self, task: &str) -> ProofRecorder {
        ProofRecorder::start_trace(
            crate::proof_commands::settings_from_config(&self.config_path, &self.config),
            ProofMode::Coder,
            task.to_string(),
            self.active_provider().map(ToString::to_string),
            self.active_model().map(ToString::to_string),
            Some(self.workspace_path().display().to_string()),
        )
    }

    fn record_proof_lens(&self, proof: &mut ProofRecorder, cards: &[SkillCard]) -> Result<()> {
        let installed_count = load_installed_skills(self.skills_dir())?.len();
        proof.record_lens_selection(LensSelectionRecord {
            enabled: true,
            selected_skill_ids: cards.iter().map(|card| card.id.clone()).collect(),
            reason_summary: Some("Axiom Lens selected coding-relevant skill cards.".to_string()),
            selected_cards: cards
                .iter()
                .map(|card| SkillCardProof {
                    id: card.id.clone(),
                    summary: card.summary.clone(),
                    risk_level: card.risk_level.to_string(),
                })
                .collect(),
            installed_skill_count: installed_count,
            auto_routed_to_coder: self.auto_routed_to_coder,
            auto_route_mode: Some(self.config.coder.auto_route_mode.clone()),
        });
        Ok(())
    }

    fn build_provider(&self, provider_name: &str) -> Result<Box<dyn LlmProvider>> {
        let provider_config = self
            .config
            .providers
            .get(provider_name)
            .ok_or_else(|| anyhow!("provider is not configured: {provider_name}"))?;

        match provider_config {
            ProviderConfig::Mock {} => Ok(Box::new(MockProvider::new(provider_name))),
            ProviderConfig::CloudflareAiGateway {
                account_id,
                gateway_id,
                api_token_env,
                base_url,
            } => {
                let provider = CloudflareAiGatewayProvider::new(
                    provider_name,
                    account_id,
                    gateway_id,
                    api_token_env,
                    base_url,
                );
                let provider = match crate::credentials::resolve_credential(api_token_env)? {
                    Some(token) => provider.with_api_token(token),
                    None => provider,
                };
                Ok(Box::new(provider))
            }
            ProviderConfig::OpenaiCompatible {
                base_url,
                api_key_env,
                models_url,
            } => {
                let provider =
                    OpenAiCompatibleProvider::new(provider_name, base_url, api_key_env.clone())
                        .with_models_url(models_url.clone());
                let provider = match api_key_env.as_deref() {
                    Some(environment_variable) => {
                        match crate::credentials::resolve_credential(environment_variable)? {
                            Some(api_key) => provider.with_api_key(api_key),
                            None => provider,
                        }
                    }
                    None => provider,
                };
                Ok(Box::new(provider))
            }
        }
    }

    fn workspace_path(&self) -> PathBuf {
        self.config.default_workspace_path()
    }

    fn skills_dir(&self) -> PathBuf {
        self.config_path
            .parent()
            .map(|config_dir| config_dir.join(&self.config.skills.local_dir))
            .unwrap_or_else(|| PathBuf::from(&self.config.skills.local_dir))
    }

    fn execution_context(&self) -> SkillExecutionContext {
        SkillExecutionContext {
            workspace_root: self.workspace_path(),
            max_file_read_bytes: self.config.coder.max_file_read_bytes,
            web_timeout_secs: 20,
            max_web_response_bytes: 1_000_000,
            web_fetch_https_only: self.config.network.web_fetch_https_only,
            web_fetch_allowed_hosts: self.config.network.web_fetch_allowed_hosts.clone(),
            web_fetch_denied_hosts: self.config.network.web_fetch_denied_hosts.clone(),
            web_fetch_use_system_proxy: self.config.network.web_fetch_use_system_proxy,
            auto_approve_medium_risk: false,
            credential_env_names: self.credential_env_names.clone(),
        }
    }

    fn is_manual_approval(&self) -> bool {
        self.config.coder.approval_mode == "manual"
    }
}

struct CoderPolicyApprover<'a> {
    proof: &'a mut ProofRecorder,
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
enum InteractiveResult {
    Continue,
    Exit,
    NotCommand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyChoice {
    Apply,
    Edit,
    Cancel,
}

fn prompt_apply_choice() -> Result<ApplyChoice> {
    loop {
        print!("Apply changes, edit plan, or cancel? [a/e/c]: ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        match input.trim().to_ascii_lowercase().as_str() {
            "a" | "apply" => return Ok(ApplyChoice::Apply),
            "e" | "edit" => return Ok(ApplyChoice::Edit),
            "" | "c" | "cancel" => return Ok(ApplyChoice::Cancel),
            _ => println!("Enter a, e, or c."),
        }
    }
}

fn prompt_plan_revision() -> Result<String> {
    loop {
        print!("Describe the plan changes: ");
        io::stdout().flush()?;
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let revision = input.trim();
        if !revision.is_empty() {
            return Ok(revision.to_string());
        }
        println!("Plan feedback cannot be empty.");
    }
}

fn print_scan_summary(scan: &ProjectScanSummary) {
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

fn print_plan(plan: &str) {
    println!("Plan:");
    println!("{plan}");
}

fn choose_test_command(commands: &[TestCommand]) -> Result<&TestCommand> {
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
        if let Ok(index) = input.trim().parse::<usize>() {
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

fn command_proof(
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

fn test_proof(command: &str, ran: bool, approved: bool, result: &CommandRunResult) -> TestProof {
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

fn checkpoint_proof(
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

fn patch_scope(patch: &PreparedPatch) -> (usize, u64) {
    let bytes = patch.files.iter().fold(0_u64, |total, file| {
        total.saturating_add(u64::try_from(file.content.len()).unwrap_or(u64::MAX))
    });
    (patch.files.len(), bytes)
}

fn is_safe_test_command(command: &str) -> bool {
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

fn floor_char_boundary(value: &str, requested: usize) -> usize {
    let mut boundary = requested.min(value.len());
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

fn bounded_test_output(result: &CommandRunResult, max_chars: usize) -> String {
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

fn print_bounded_lines(content: &str, max_lines: usize) {
    let mut lines = content.lines();
    for line in lines.by_ref().take(max_lines) {
        println!("+ {line}");
    }
    if lines.next().is_some() {
        println!("... [new-file preview truncated; full content remains in the patch proof]");
    }
}

fn print_interactive_help() {
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

fn git_diff_message(
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

fn hardened_git_diff_command(
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

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[test]
    fn safe_test_command_allowlist_is_restricted() {
        assert!(is_safe_test_command("cargo test"));
        assert!(!is_safe_test_command("cargo run"));
    }

    #[test]
    fn patch_scope_counts_files_and_saturating_bytes() {
        let patch = PreparedPatch {
            summary: "scope".to_string(),
            test_command: None,
            files: vec![
                axiom_coder::PreparedFileChange {
                    path: "a.txt".to_string(),
                    content: "abc".to_string(),
                    observed_sha256: None,
                    existed: false,
                },
                axiom_coder::PreparedFileChange {
                    path: "b.txt".to_string(),
                    content: "12345".to_string(),
                    observed_sha256: None,
                    existed: false,
                },
            ],
            diff: String::new(),
        };

        assert_eq!(patch_scope(&patch), (2, 8));
    }

    #[test]
    fn no_git_repo_diff_message_is_stable() {
        let dir = unique_temp_dir();

        let message = git_diff_message(&dir, &[]).expect("diff message");

        assert_eq!(
            message,
            "No git repository detected. Git integration is optional for now."
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn git_diff_disables_external_drivers_and_scrubs_provider_credentials() {
        let key = "AXIOM_TEST_GIT_CHILD_KEY_9917A8CC".to_string();
        let command =
            hardened_git_diff_command(std::path::Path::new("."), std::slice::from_ref(&key));
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(args.iter().any(|arg| arg == "--no-ext-diff"));
        assert!(args.iter().any(|arg| arg == "--no-textconv"));
        assert!(args
            .windows(2)
            .any(|pair| pair == ["-c", "core.fsmonitor=false"]));
        assert!(args
            .iter()
            .any(|arg| arg == ":(exclude,icase,glob)**/.env*"));
        assert!(command
            .get_envs()
            .any(|(name, value)| name == key.as_str() && value.is_none()));
    }

    #[tokio::test]
    async fn exhausted_persistent_budget_blocks_coder_before_provider_call() {
        let dir = unique_temp_dir();
        let session = coder_session_for_cost_test(&dir, Some(0.0));

        let error = session
            .llm_chat(vec![ChatMessage {
                role: "user".to_string(),
                content: "provider must not be called".to_string(),
            }])
            .await
            .expect_err("zero budget must block Coder");

        assert!(error
            .to_string()
            .contains("no Coder provider call was made"));
        assert_eq!(
            session
                .cost_ledger_store()
                .load()
                .expect("empty ledger")
                .events()
                .count(),
            0
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn coder_provider_call_is_recorded_in_shared_cost_ledger() {
        let dir = unique_temp_dir();
        let session = coder_session_for_cost_test(&dir, None);

        let response = session
            .llm_chat(vec![ChatMessage {
                role: "user".to_string(),
                content: "hello from coder cost test".to_string(),
            }])
            .await
            .expect("mock Coder call");
        let ledger = session.cost_ledger_store().load().expect("cost ledger");
        let events = ledger.events().collect::<Vec<_>>();

        assert!(response.contains("Axiom (offline)"));
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].session_id, session.cost_session_id);
        assert_eq!(events[0].provider, "mock");
        assert_eq!(events[0].model, "mock-model");
        assert!(events[0].prompt_tokens > 0);
        assert!(events[0].completion_tokens > 0);
        let _ = fs::remove_dir_all(dir);
    }

    fn coder_session_for_cost_test(
        dir: &std::path::Path,
        session_budget: Option<f64>,
    ) -> CoderSession {
        fs::create_dir_all(dir.join("workspace")).expect("workspace");
        fs::create_dir_all(dir.join("skills")).expect("skills");
        let config_path = dir.join("config.toml");
        let mut config = AxiomConfig::default();
        config.agent.first_run_completed = true;
        config.agent.default_workspace = dir.join("workspace").display().to_string();
        config.agent.session_budget_usd = session_budget;
        config.agent.input_cost_per_million_tokens = Some(1.0);
        config.agent.output_cost_per_million_tokens = Some(1.0);
        config.llm.active_provider = Some("mock".to_string());
        config.llm.active_model = Some("mock-model".to_string());
        config
            .llm
            .provider_models
            .insert("mock".to_string(), "mock-model".to_string());
        config
            .providers
            .insert("mock".to_string(), ProviderConfig::Mock {});
        config.save_to_path(&config_path).expect("save config");
        CoderSession::load_from_path(config_path).expect("load Coder session")
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("axiom-cli-code-test-{nanos}"))
    }
}
