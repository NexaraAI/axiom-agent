use std::{
    io::{self, Write},
    path::PathBuf,
    process::Command,
};

use anyhow::{anyhow, bail, Result};
use axiom_coder::{
    build_patch_prompt, build_plan_prompt, diff_for_patch, parse_axiom_patch, scan_project,
    AxiomPatch, ProjectScanSummary, TestCommand,
};
use axiom_core::{AxiomConfig, ProviderConfig};
use axiom_engine::{
    execute_installed_tool, load_installed_skills, AllowAllApprover, SkillCard,
    SkillExecutionContext, ToolRequest,
};
use axiom_lens::{build_skill_context_message, select_relevant_skills};
use axiom_llm::{
    ChatMessage, ChatRequest, CloudflareAiGatewayProvider, LlmProvider, OpenAiCompatibleProvider,
};
use axiom_proof::{
    new_approval, CommandProof, FileWriteProof, LensSelectionRecord, PatchProof, ProofMode,
    ProofRecorder, SkillCardProof, TestProof,
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
    history: Vec<ChatMessage>,
    auto_routed_to_coder: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandRunResult {
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl CoderSession {
    fn load_default() -> Result<Self> {
        let config_path = AxiomConfig::default_config_path()?;
        let config = AxiomConfig::load_from_path(&config_path)?;
        Ok(Self {
            config_path,
            config,
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
        let scan = self.scan()?;
        print_scan_summary(&scan);
        let plan = self.request_plan(task, &scan, Some(&mut proof)).await?;
        print_plan(&plan);

        if plan_only {
            proof.set_final_response(&plan);
            proof.finish_trace("coder plan-only completed without file changes");
            let _ = proof.export();
            return Ok(());
        }

        match prompt_apply_choice()? {
            ApplyChoice::Apply => {
                proof.record_approval(new_approval(
                    "apply_plan",
                    "medium",
                    "Apply changes?",
                    "approved",
                ));
                self.run_apply_task_with_plan(task, &scan, &plan, &mut proof)
                    .await
            }
            ApplyChoice::Edit => {
                proof.record_approval(new_approval(
                    "apply_plan",
                    "medium",
                    "Apply changes?",
                    "edit",
                ));
                proof.cancel_trace("plan editing requested; no file changes applied");
                let _ = proof.export();
                println!(
                    "Plan editing is not implemented in v0.1. Start a new !plan with revisions."
                );
                Ok(())
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
                Ok(())
            }
        }
    }

    async fn run_apply_task(&mut self, task: &str) -> Result<()> {
        let mut proof = self.start_proof_trace(task);
        let scan = self.scan()?;
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

        let patch_text = self.request_patch(task, scan, plan).await?;
        let patch = match parse_axiom_patch(&patch_text) {
            Ok(patch) => patch,
            Err(error) => {
                proof.record_error("patch", error.to_string(), "parse_patch", true);
                proof.fail_trace("patch parsing failed", "parse_patch");
                let _ = proof.export();
                return Err(error.into());
            }
        };
        self.apply_patch_after_confirmation(&patch, proof).await
    }

    async fn apply_patch_after_confirmation(
        &mut self,
        patch: &AxiomPatch,
        proof: &mut ProofRecorder,
    ) -> Result<()> {
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
        let preview = diff_for_patch(patch, self.workspace_path())?;
        println!("Patch summary: {}", patch.summary);
        println!("{}", preview.diff);

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
                diff: preview.diff,
                approved: false,
                applied: false,
            });
            proof.cancel_trace("cancelled before writing files");
            let _ = proof.export();
            println!("Cancelled before writing files.");
            return Ok(());
        }

        proof.record_approval(new_approval(
            "apply_patch",
            "medium",
            "Apply these file changes?",
            "approved",
        ));
        self.write_patch_through_engine(patch, proof).await?;
        proof.record_patch(PatchProof {
            event_id: axiom_proof::trace::new_event_id("patch"),
            summary: patch.summary.clone(),
            changed_files: patch
                .changes
                .iter()
                .map(|change| change.path.clone())
                .collect(),
            diff: preview.diff,
            approved: true,
            applied: true,
        });

        println!("Changed files:");
        for change in &patch.changes {
            println!("- {}", change.path);
        }

        if let Some(command) = patch.test_command.as_deref() {
            if is_safe_test_command(command) {
                let approved = chat::confirm(&format!("Run `{command}` now?"), false)?;
                proof.record_approval(new_approval(
                    "run_test",
                    "medium",
                    format!("Run `{command}` now?"),
                    if approved { "approved" } else { "denied" },
                ));
                if approved {
                    let result = self.run_command(command)?;
                    proof.record_command(command_proof(
                        command,
                        self.workspace_path(),
                        &result,
                        true,
                    ));
                    proof.record_test(test_proof(command, true, true, &result));
                } else {
                    proof.record_test(TestProof {
                        event_id: axiom_proof::trace::new_event_id("test"),
                        detected_command: command.to_string(),
                        ran: false,
                        approved: false,
                        exit_code: None,
                        passed: None,
                        output_summary: None,
                    });
                }
            }
        }

        proof.set_final_response(format!(
            "Applied patch. Changed files: {}",
            patch
                .changes
                .iter()
                .map(|change| change.path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        proof.finish_trace("coder apply completed");
        let _ = proof.export();
        Ok(())
    }

    async fn write_patch_through_engine(
        &self,
        patch: &AxiomPatch,
        proof: &mut ProofRecorder,
    ) -> Result<()> {
        let installed_skills = load_installed_skills(self.skills_dir())?;
        let execution_context = self.execution_context();
        let mut approval = AllowAllApprover;

        for change in &patch.changes {
            let target = self.workspace_path().join(&change.path);
            let existed = target.exists();
            let request = ToolRequest {
                skill_id: "file.write".to_string(),
                arguments: json!({
                    "path": change.path,
                    "content": change.content,
                }),
            };
            execute_installed_tool(
                &request,
                &installed_skills,
                &execution_context,
                &mut approval,
            )
            .await?;
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
        let prompt = build_patch_prompt(task, scan, plan);
        self.llm_chat(vec![ChatMessage {
            role: "user".to_string(),
            content: prompt,
        }])
        .await
    }

    async fn send_coder_message(&mut self, input: &str) -> Result<()> {
        let scan = self.scan()?;
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
        let provider = self.build_provider(&provider_name)?;
        let response = provider
            .chat(ChatRequest {
                model,
                messages,
                temperature: Some(0.2),
                max_tokens: None,
                stream: false,
                metadata: None,
                provider_options: None,
            })
            .await?;
        Ok(response.content)
    }

    fn print_scan(&self) -> Result<()> {
        let scan = self.scan()?;
        print_scan_summary(&scan);
        Ok(())
    }

    fn explain_project(&self) -> Result<()> {
        let scan = self.scan()?;
        print_scan_summary(&scan);
        println!("Important files:");
        for file in &scan.important_files {
            println!("- {file}");
        }
        if scan.important_files.is_empty() {
            println!("- none detected");
        }
        Ok(())
    }

    fn scan(&self) -> Result<ProjectScanSummary> {
        if self.is_manual_approval() && !chat::confirm("Scan workspace files?", true)? {
            bail!("scan cancelled")
        }
        Ok(scan_project(self.workspace_path(), 4)?)
    }

    fn print_git_diff(&self) -> Result<()> {
        println!("{}", git_diff_message(&self.workspace_path())?);
        Ok(())
    }

    fn run_detected_tests(&self) -> Result<()> {
        let mut proof = self.start_proof_trace("run detected tests");
        let scan = self.scan()?;
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
        let approved = chat::confirm(&format!("Run `{}`?", command.command), false)?;
        proof.record_approval(new_approval(
            "run_test",
            "medium",
            format!("Run `{}`?", command.command),
            if approved { "approved" } else { "denied" },
        ));
        if !approved {
            proof.record_test(TestProof {
                event_id: axiom_proof::trace::new_event_id("test"),
                detected_command: command.command.clone(),
                ran: false,
                approved: false,
                exit_code: None,
                passed: None,
                output_summary: Some("Test command cancelled.".to_string()),
            });
            proof.cancel_trace("test command cancelled");
            let _ = proof.export();
            println!("Test command cancelled.");
            return Ok(());
        }

        let result = self.run_command(&command.command)?;
        proof.record_command(command_proof(
            &command.command,
            self.workspace_path(),
            &result,
            true,
        ));
        proof.record_test(test_proof(&command.command, true, true, &result));
        proof.finish_trace("test command completed");
        let _ = proof.export();
        Ok(())
    }

    fn run_command(&self, command: &str) -> Result<CommandRunResult> {
        if !is_safe_test_command(command) {
            bail!("refusing to run unsupported command: {command}");
        }

        let parts = command.split_whitespace().collect::<Vec<_>>();
        let Some((program, args)) = parts.split_first() else {
            bail!("empty command")
        };
        let output = Command::new(program)
            .args(args)
            .current_dir(self.workspace_path())
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        if !output.stdout.is_empty() {
            println!("{stdout}");
        }
        if !output.stderr.is_empty() {
            eprintln!("{stderr}");
        }
        println!("command exit: {}", output.status);
        Ok(CommandRunResult {
            exit_code: output.status.code(),
            stdout,
            stderr,
        })
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
        self.config.save_to_path(&self.config_path)?;
        Ok(model)
    }

    fn set_provider(&mut self, provider_name: impl Into<String>) -> Result<String> {
        let provider_name = provider_name.into();
        if !self.config.providers.contains_key(&provider_name) {
            bail!("provider is not configured: {provider_name}");
        }
        self.config.llm.active_provider = Some(provider_name.clone());
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
            ProviderConfig::CloudflareAiGateway {
                account_id,
                gateway_id,
                api_token_env,
                base_url,
            } => Ok(Box::new(CloudflareAiGatewayProvider::new(
                provider_name,
                account_id,
                gateway_id,
                api_token_env,
                base_url,
            ))),
            ProviderConfig::OpenaiCompatible {
                base_url,
                api_key_env,
            } => Ok(Box::new(OpenAiCompatibleProvider::new(
                provider_name,
                base_url,
                api_key_env,
            ))),
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
            auto_approve_medium_risk: false,
        }
    }

    fn is_manual_approval(&self) -> bool {
        self.config.coder.approval_mode == "manual"
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
            println!("- {} ({})", command.command, command.reason);
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
        println!("{}: {} ({})", index + 1, command.command, command.reason);
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

fn is_safe_test_command(command: &str) -> bool {
    matches!(
        command,
        "cargo test" | "npm test" | "pnpm test" | "yarn test" | "python -m pytest" | "pytest"
    )
}

fn print_interactive_help() {
    println!("Commands:");
    println!("!help");
    println!("!exit");
    println!("!scan");
    println!("!plan TASK");
    println!("!apply TASK");
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

fn git_diff_message(workspace: &std::path::Path) -> Result<String> {
    if !workspace.join(".git").exists() {
        return Ok("No git repository detected. Git integration is optional for now.".to_string());
    }

    let output = Command::new("git")
        .arg("-C")
        .arg(workspace)
        .arg("diff")
        .arg("--")
        .output()?;
    if !output.status.success() {
        return Ok(format!(
            "git diff failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let diff = String::from_utf8_lossy(&output.stdout);
    if diff.trim().is_empty() {
        Ok("No git diff.".to_string())
    } else {
        Ok(diff.to_string())
    }
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
    fn no_git_repo_diff_message_is_stable() {
        let dir = unique_temp_dir();

        let message = git_diff_message(&dir).expect("diff message");

        assert_eq!(
            message,
            "No git repository detected. Git integration is optional for now."
        );
        let _ = fs::remove_dir_all(dir);
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("axiom-cli-code-test-{nanos}"))
    }
}
