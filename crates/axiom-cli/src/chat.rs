use std::{
    io::{self, Write},
    path::{Path, PathBuf},
};

use anyhow::{anyhow, Result};
use axiom_core::{AxiomConfig, ProviderConfig};
use axiom_engine::{
    execute_installed_tool, extract_tool_request, load_installed_skills, ApprovalRequest,
    SkillApproval, SkillCard, SkillExecutionContext, SkillExecutionError, SkillExecutionResult,
};
use axiom_lens::{
    auto_route_action, build_skill_context_message, select_relevant_skills, AutoRouteAction,
};
use axiom_llm::{
    ChatMessage, ChatRequest, CloudflareAiGatewayProvider, LlmProvider, OpenAiCompatibleProvider,
};
use axiom_proof::{
    new_approval, new_tool_call, FileReadProof, FileWriteProof, LensSelectionRecord, ProofMode,
    ProofRecorder, SkillCardProof,
};

pub(crate) struct ChatSession {
    config_path: PathBuf,
    config: AxiomConfig,
    history: Vec<ChatMessage>,
    lens_enabled: bool,
}

impl ChatSession {
    pub(crate) fn load(config_path: impl AsRef<Path>) -> Result<Self> {
        let config_path = config_path.as_ref().to_path_buf();
        let config = AxiomConfig::load_from_path(&config_path)?;

        Ok(Self {
            config_path,
            config,
            history: Vec::new(),
            lens_enabled: true,
        })
    }

    pub(crate) fn provider_names(&self) -> Vec<String> {
        self.config.providers.keys().cloned().collect()
    }

    pub(crate) fn active_provider(&self) -> Option<&str> {
        self.config.llm.active_provider.as_deref()
    }

    pub(crate) fn active_model(&self) -> Option<&str> {
        self.config.llm.active_model.as_deref()
    }

    pub(crate) fn workspace_path(&self) -> PathBuf {
        self.config.default_workspace_path()
    }

    #[cfg(test)]
    pub(crate) fn history_len(&self) -> usize {
        self.history.len()
    }

    pub(crate) fn clear_history(&mut self) {
        self.history.clear();
    }

    pub(crate) fn set_lens_enabled(&mut self, enabled: bool) {
        self.lens_enabled = enabled;
    }

    #[cfg(test)]
    pub(crate) fn lens_enabled(&self) -> bool {
        self.lens_enabled
    }

    pub(crate) fn installed_skill_cards(&self) -> Result<Vec<SkillCard>> {
        Ok(load_installed_skills(self.skills_dir())?
            .into_iter()
            .filter(|skill| skill.record.enabled)
            .map(|skill| skill.manifest.to_skill_card())
            .collect())
    }

    pub(crate) fn select_skill_cards(
        &self,
        prompt: &str,
        max_cards: usize,
    ) -> Result<Vec<SkillCard>> {
        if !self.lens_enabled {
            return Ok(Vec::new());
        }

        Ok(select_relevant_skills(
            prompt,
            &load_installed_skills(self.skills_dir())?,
            max_cards,
        ))
    }

    pub(crate) fn set_model(&mut self, model: impl Into<String>) -> Result<String> {
        let model = model.into();
        if model.trim().is_empty() {
            return Err(anyhow!("model name cannot be empty"));
        }

        self.config.llm.active_model = Some(model.clone());
        self.save_config()?;
        Ok(model)
    }

    pub(crate) fn set_provider(&mut self, provider_name: impl Into<String>) -> Result<String> {
        let provider_name = provider_name.into();
        if !self.config.providers.contains_key(&provider_name) {
            return Err(anyhow!("provider is not configured: {provider_name}"));
        }

        self.config.llm.active_provider = Some(provider_name.clone());
        self.save_config()?;
        Ok(provider_name)
    }

    pub(crate) fn set_proof_enabled(&mut self, enabled: bool) -> Result<()> {
        self.config.proof.enabled = enabled;
        self.save_config()
    }

    pub(crate) async fn send_user_message(
        &mut self,
        content: String,
        skill_cards: &[SkillCard],
        approval: &mut dyn SkillApproval,
    ) -> Result<ChatTurnResult> {
        let model = self
            .active_model()
            .ok_or_else(|| anyhow!("no active model configured. Use `!model use <model>`."))?
            .to_string();
        let provider_name = self
            .active_provider()
            .ok_or_else(|| anyhow!("no active provider configured. Use `!provider use <name>`."))?
            .to_string();
        let provider = self.build_provider(&provider_name)?;
        let mut proof = self.start_proof_trace(&content, &provider_name, &model);
        self.record_proof_lens(&mut proof, skill_cards)?;

        let user_message = ChatMessage {
            role: "user".to_string(),
            content,
        };
        let mut messages = Vec::new();
        if let Some(skill_context) = build_skill_context_message(skill_cards) {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: skill_context,
            });
        }
        messages.extend(self.history.clone());
        messages.push(user_message.clone());

        let response_content = match provider_chat(provider.as_ref(), model.clone(), messages).await
        {
            Ok(response) => response,
            Err(error) => {
                proof.record_error("llm", error.to_string(), "chat", true);
                proof.fail_trace("provider call failed", "chat");
                let _ = proof.export();
                return Err(error);
            }
        };
        let mut tool_results = Vec::new();

        let tool_request = match extract_tool_request(&response_content) {
            Ok(tool_request) => Some(tool_request),
            Err(SkillExecutionError::MissingToolBlock) => None,
            Err(error) => {
                proof.record_error(
                    "tool_request",
                    error.to_string(),
                    "parse_tool_request",
                    true,
                );
                proof.fail_trace("invalid tool request", "parse_tool_request");
                let _ = proof.export();
                return Err(anyhow!("invalid Axiom tool request: {error}"));
            }
        };

        if let Some(tool_request) = tool_request {
            let mut tool_call =
                new_tool_call(&tool_request.skill_id, tool_request.arguments.to_string());
            let installed_skills = load_installed_skills(self.skills_dir())?;
            let execution_context = self.execution_context();
            let execution_result = {
                let mut recording_approval = RecordingApprover {
                    inner: approval,
                    proof: &mut proof,
                };
                execute_installed_tool(
                    &tool_request,
                    &installed_skills,
                    &execution_context,
                    &mut recording_approval,
                )
                .await
            };
            let tool_result = match execution_result {
                Ok(result) => result,
                Err(error) => {
                    tool_call.error = Some(error.to_string());
                    proof.record_tool_call(tool_call);
                    proof.record_error("tool", error.to_string(), "execute_tool", true);
                    proof.fail_trace("tool execution failed", "execute_tool");
                    let _ = proof.export();
                    return Err(error.into());
                }
            };
            tool_call.success = true;
            tool_call.ended_at = Some(axiom_proof::trace::now_timestamp());
            tool_call.output_summary = Some(tool_result.output.to_string());
            self.record_tool_output_files(&mut proof, &tool_result);
            proof.record_tool_call(tool_call);
            let tool_result_message = ChatMessage {
                role: "user".to_string(),
                content: format_tool_result_message(&tool_result),
            };
            let final_instruction = ChatMessage {
                role: "user".to_string(),
                content: "Use the Axiom Tool Result to answer the user's original request. Do not request the same tool again unless more data is required.".to_string(),
            };
            let mut follow_up_messages = Vec::new();
            if let Some(skill_context) = build_skill_context_message(skill_cards) {
                follow_up_messages.push(ChatMessage {
                    role: "system".to_string(),
                    content: skill_context,
                });
            }
            follow_up_messages.extend(self.history.clone());
            follow_up_messages.push(user_message.clone());
            follow_up_messages.push(ChatMessage {
                role: "assistant".to_string(),
                content: response_content.clone(),
            });
            follow_up_messages.push(tool_result_message.clone());
            follow_up_messages.push(final_instruction);

            let provider = self.build_provider(&provider_name)?;
            let final_content =
                match provider_chat(provider.as_ref(), model.clone(), follow_up_messages).await {
                    Ok(response) => response,
                    Err(error) => {
                        proof.record_error("llm", error.to_string(), "tool_follow_up", true);
                        proof.fail_trace("provider follow-up failed", "tool_follow_up");
                        let _ = proof.export();
                        return Err(error);
                    }
                };

            self.history.push(user_message);
            self.history.push(ChatMessage {
                role: "assistant".to_string(),
                content: response_content,
            });
            self.history.push(tool_result_message);
            self.history.push(ChatMessage {
                role: "assistant".to_string(),
                content: final_content.clone(),
            });
            tool_results.push(tool_result);
            proof.set_final_response(&final_content);
            proof.finish_trace("chat turn completed with tool execution");
            let _ = proof.export();

            return Ok(ChatTurnResult {
                content: final_content,
                tool_results,
            });
        }

        self.history.push(user_message);
        self.history.push(ChatMessage {
            role: "assistant".to_string(),
            content: response_content.clone(),
        });
        proof.set_final_response(&response_content);
        proof.finish_trace("chat turn completed");
        let _ = proof.export();

        Ok(ChatTurnResult {
            content: response_content,
            tool_results,
        })
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

    fn skills_dir(&self) -> PathBuf {
        self.config_path
            .parent()
            .map(|config_dir| config_dir.join(&self.config.skills.local_dir))
            .unwrap_or_else(|| PathBuf::from(&self.config.skills.local_dir))
    }

    fn execution_context(&self) -> SkillExecutionContext {
        SkillExecutionContext {
            workspace_root: self.config.default_workspace_path(),
            max_file_read_bytes: self.config.coder.max_file_read_bytes,
            web_timeout_secs: 20,
            max_web_response_bytes: 1_000_000,
            auto_approve_medium_risk: self.config.coder.approval_mode == "trusted",
        }
    }

    fn save_config(&self) -> Result<()> {
        self.config.save_to_path(&self.config_path)?;
        Ok(())
    }

    fn auto_route_action(&self, prompt: &str) -> AutoRouteAction {
        if !self.lens_enabled {
            return AutoRouteAction::StayInChat;
        }
        auto_route_action(
            prompt,
            self.config.coder.auto_route_from_chat,
            &self.config.coder.auto_route_mode,
        )
    }

    fn start_proof_trace(&self, prompt: &str, provider_name: &str, model: &str) -> ProofRecorder {
        ProofRecorder::start_trace(
            crate::proof_commands::settings_from_config(&self.config_path, &self.config),
            ProofMode::Chat,
            prompt.to_string(),
            Some(provider_name.to_string()),
            Some(model.to_string()),
            Some(self.workspace_path().display().to_string()),
        )
    }

    fn record_proof_lens(&self, proof: &mut ProofRecorder, cards: &[SkillCard]) -> Result<()> {
        let installed_count = load_installed_skills(self.skills_dir())?.len();
        proof.record_lens_selection(LensSelectionRecord {
            enabled: self.lens_enabled,
            selected_skill_ids: cards.iter().map(|card| card.id.clone()).collect(),
            reason_summary: Some("Axiom Lens selected relevant installed skill cards.".to_string()),
            selected_cards: cards
                .iter()
                .map(|card| SkillCardProof {
                    id: card.id.clone(),
                    summary: card.summary.clone(),
                    risk_level: card.risk_level.to_string(),
                })
                .collect(),
            installed_skill_count: installed_count,
            auto_routed_to_coder: false,
            auto_route_mode: Some(self.config.coder.auto_route_mode.clone()),
        });
        Ok(())
    }

    fn record_tool_output_files(&self, proof: &mut ProofRecorder, result: &SkillExecutionResult) {
        if result.skill_id == "file.read" {
            proof.record_file_read(FileReadProof {
                event_id: axiom_proof::trace::new_event_id("read"),
                path: result.output["path"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string(),
                bytes: result.output["bytes"].as_u64(),
                allowed: true,
                blocked_reason: None,
            });
        }
        if result.skill_id == "file.write" {
            proof.record_file_write(FileWriteProof {
                event_id: axiom_proof::trace::new_event_id("write"),
                path: result.output["path"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_string(),
                bytes_written: result.output["bytes_written"].as_u64(),
                created: result.output["created"].as_bool().unwrap_or(false),
                overwrote: !result.output["created"].as_bool().unwrap_or(false),
                approved: true,
                diff_summary: Some("file.write executed after approval".to_string()),
            });
        }
    }
}

pub(crate) async fn run_terminal_chat() -> Result<()> {
    let config_path = AxiomConfig::default_config_path()?;
    let mut session = ChatSession::load(&config_path)?;

    println!("Axiom chat");
    println!(
        "provider: {}",
        session.active_provider().unwrap_or("not configured")
    );
    println!(
        "model: {}",
        session.active_model().unwrap_or("not configured")
    );
    println!("workspace: {}", session.workspace_path().display());
    println!("Type !help for commands or !exit to leave.");

    loop {
        print!("axiom> ");
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

        match handle_chat_command(&mut session, trimmed)? {
            CommandResult::Continue => continue,
            CommandResult::Exit => break,
            CommandResult::NotCommand => {}
        }

        match session.auto_route_action(trimmed) {
            AutoRouteAction::StayInChat => {}
            AutoRouteAction::Ask => {
                println!("Axiom Lens: this looks like a project coding task.");
                if confirm("Switch to Axiom Coder mode?", true)? {
                    crate::code_commands::run_task_from_chat(trimmed.to_string()).await?;
                    continue;
                }
            }
            AutoRouteAction::Switch => {
                println!("Axiom Lens: switching to Axiom Coder mode for project level coding.");
                crate::code_commands::run_task_from_chat(trimmed.to_string()).await?;
                continue;
            }
        }

        let skill_cards = session.select_skill_cards(trimmed, 5)?;
        if !skill_cards.is_empty() {
            let selected = skill_cards
                .iter()
                .map(|card| card.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            println!("Axiom Lens: selected {selected}");
        }

        let mut approval = TerminalApprover;
        match session
            .send_user_message(trimmed.to_string(), &skill_cards, &mut approval)
            .await
        {
            Ok(turn) => {
                for result in turn.tool_results {
                    println!("Axiom Tool: executed {}", result.skill_id);
                }
                println!("Axiom: {}", turn.content);
            }
            Err(error) => println!("Axiom error: {error}"),
        }
    }

    Ok(())
}

async fn provider_chat(
    provider: &dyn LlmProvider,
    model: String,
    messages: Vec<ChatMessage>,
) -> Result<String> {
    let response = provider
        .chat(ChatRequest {
            model,
            messages,
            temperature: Some(0.7),
            max_tokens: None,
            stream: false,
            metadata: None,
            provider_options: None,
        })
        .await?;

    Ok(response.content)
}

fn format_tool_result_message(result: &SkillExecutionResult) -> String {
    format!(
        "Axiom Tool Result for `{}`:\n```json\n{}\n```",
        result.skill_id, result.output
    )
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ChatTurnResult {
    pub content: String,
    pub tool_results: Vec<SkillExecutionResult>,
}

struct TerminalApprover;

impl SkillApproval for TerminalApprover {
    fn approve(&mut self, request: &ApprovalRequest) -> bool {
        println!(
            "Axiom approval required [{}]: {}",
            request.risk_level, request.message
        );
        confirm("Approve skill execution?", false).unwrap_or(false)
    }
}

struct RecordingApprover<'a, 'b> {
    inner: &'a mut dyn SkillApproval,
    proof: &'b mut ProofRecorder,
}

impl SkillApproval for RecordingApprover<'_, '_> {
    fn approve(&mut self, request: &ApprovalRequest) -> bool {
        let approved = self.inner.approve(request);
        self.proof.record_approval(new_approval(
            format!("skill:{}", request.skill_id),
            request.risk_level.clone(),
            request.message.clone(),
            if approved { "approved" } else { "denied" },
        ));
        approved
    }
}

fn handle_chat_command(session: &mut ChatSession, input: &str) -> Result<CommandResult> {
    if !input.starts_with('!') {
        return Ok(CommandResult::NotCommand);
    }

    match input {
        "!exit" => Ok(CommandResult::Exit),
        "!help" => {
            print_help();
            Ok(CommandResult::Continue)
        }
        "!model current" => {
            println!(
                "Current model: {}",
                session.active_model().unwrap_or("not configured")
            );
            Ok(CommandResult::Continue)
        }
        "!provider current" => {
            println!(
                "Current provider: {}",
                session.active_provider().unwrap_or("not configured")
            );
            Ok(CommandResult::Continue)
        }
        "!provider list" => {
            let providers = session.provider_names();
            if providers.is_empty() {
                println!("No providers configured.");
            } else {
                println!("Configured providers:");
                for provider in providers {
                    let marker = if Some(provider.as_str()) == session.active_provider() {
                        "*"
                    } else {
                        "-"
                    };
                    println!("{marker} {provider}");
                }
            }
            Ok(CommandResult::Continue)
        }
        "!clear" => {
            session.clear_history();
            println!("Conversation cleared.");
            Ok(CommandResult::Continue)
        }
        "!proof on" => {
            session.set_proof_enabled(true)?;
            println!("Proof Mode enabled.");
            Ok(CommandResult::Continue)
        }
        "!proof off" => {
            session.set_proof_enabled(false)?;
            println!("Proof Mode disabled.");
            Ok(CommandResult::Continue)
        }
        "!proof status" => {
            println!(
                "Proof Mode: {}",
                if session.config.proof.enabled {
                    "enabled"
                } else {
                    "disabled"
                }
            );
            Ok(CommandResult::Continue)
        }
        "!proof latest" => {
            match axiom_proof::latest_proof(crate::proof_commands::proofs_dir(
                &session.config_path,
            ))? {
                Some(entry) => {
                    println!(
                        "Latest proof: {}",
                        entry
                            .markdown_path
                            .as_ref()
                            .unwrap_or(&entry.json_path)
                            .display()
                    );
                    println!("{} {} - {}", entry.task_id, entry.status, entry.summary);
                }
                None => println!("No proof traces found."),
            }
            Ok(CommandResult::Continue)
        }
        "!skills" => {
            let cards = session.installed_skill_cards()?;
            if cards.is_empty() {
                println!("No enabled skills installed.");
            } else {
                println!("Installed enabled skills:");
                for card in cards {
                    println!("- {}: {}", card.id, card.summary);
                }
            }
            Ok(CommandResult::Continue)
        }
        "!lens on" => {
            session.set_lens_enabled(true);
            println!("Axiom Lens enabled.");
            Ok(CommandResult::Continue)
        }
        "!lens off" => {
            session.set_lens_enabled(false);
            println!("Axiom Lens disabled.");
            Ok(CommandResult::Continue)
        }
        _ if input.starts_with("!skills selected") => {
            let prompt = input.trim_start_matches("!skills selected").trim();
            if prompt.is_empty() {
                println!("Usage: !skills selected <message>");
            } else {
                let cards = session.select_skill_cards(prompt, 5)?;
                if cards.is_empty() {
                    println!("Axiom Lens: selected no skills.");
                } else {
                    println!("Axiom Lens selected:");
                    for card in cards {
                        println!("- {}: {}", card.id, card.summary);
                    }
                }
            }
            Ok(CommandResult::Continue)
        }
        _ if input.starts_with("!model use ") => {
            let model = input.trim_start_matches("!model use ").trim();
            match session.set_model(model) {
                Ok(model) => println!("Model switched to {model}."),
                Err(error) => println!("Model switch failed: {error}"),
            }
            Ok(CommandResult::Continue)
        }
        _ if input.starts_with("!provider use ") => {
            let provider = input.trim_start_matches("!provider use ").trim();
            match session.set_provider(provider) {
                Ok(provider) => println!("Provider switched to {provider}."),
                Err(error) => println!("Provider switch failed: {error}"),
            }
            Ok(CommandResult::Continue)
        }
        _ => {
            println!("Unknown command. Type !help for commands.");
            Ok(CommandResult::Continue)
        }
    }
}

fn print_help() {
    println!("Commands:");
    println!("!help");
    println!("!exit");
    println!("!model current");
    println!("!model use <model>");
    println!("!provider current");
    println!("!provider list");
    println!("!provider use <name>");
    println!("!clear");
    println!("!proof on");
    println!("!proof off");
    println!("!proof status");
    println!("!proof latest");
    println!("!skills");
    println!("!skills selected <message>");
    println!("!lens on");
    println!("!lens off");
}

pub(crate) fn confirm(label: &str, default: bool) -> Result<bool> {
    let hint = if default { "Y/n" } else { "y/N" };
    loop {
        print!("{label} [{hint}]: ");
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;

        let trimmed = input.trim().to_ascii_lowercase();
        if trimmed.is_empty() {
            return Ok(default);
        }

        match trimmed.as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" => return Ok(false),
            _ => println!("Enter y or n."),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommandResult {
    Continue,
    Exit,
    NotCommand,
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[test]
    fn model_switch_updates_memory_and_config() {
        let dir = unique_temp_dir();
        let config_path = dir.join("config.toml");
        let mut config = AxiomConfig::default();
        config.agent.first_run_completed = true;
        config.save_to_path(&config_path).expect("save config");
        let mut session = ChatSession::load(&config_path).expect("load session");

        session.set_model("new-model").expect("set model");
        let saved = AxiomConfig::load_from_path(&config_path).expect("load saved config");

        assert_eq!(session.active_model(), Some("new-model"));
        assert_eq!(saved.llm.active_model.as_deref(), Some("new-model"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn provider_switch_updates_memory_and_config() {
        let dir = unique_temp_dir();
        let config_path = dir.join("config.toml");
        let mut config = AxiomConfig::default();
        config.agent.first_run_completed = true;
        config.save_to_path(&config_path).expect("save config");
        let mut session = ChatSession::load(&config_path).expect("load session");

        session.set_provider("local").expect("set provider");
        let saved = AxiomConfig::load_from_path(&config_path).expect("load saved config");

        assert_eq!(session.active_provider(), Some("local"));
        assert_eq!(saved.llm.active_provider.as_deref(), Some("local"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn provider_switch_rejects_unknown_provider() {
        let dir = unique_temp_dir();
        let config_path = dir.join("config.toml");
        let mut config = AxiomConfig::default();
        config.agent.first_run_completed = true;
        config.save_to_path(&config_path).expect("save config");
        let mut session = ChatSession::load(&config_path).expect("load session");

        let error = session
            .set_provider("missing")
            .expect_err("missing provider should fail");

        assert!(error.to_string().contains("provider is not configured"));
        assert_eq!(session.active_provider(), Some("cloudflare"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn clear_command_drops_conversation_history() {
        let dir = unique_temp_dir();
        let config_path = dir.join("config.toml");
        let mut config = AxiomConfig::default();
        config.agent.first_run_completed = true;
        config.save_to_path(&config_path).expect("save config");
        let mut session = ChatSession::load(&config_path).expect("load session");
        session.history.push(ChatMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
        });

        handle_chat_command(&mut session, "!clear").expect("clear command");

        assert_eq!(session.history_len(), 0);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn lens_toggle_disables_skill_selection() {
        let dir = unique_temp_dir();
        let config_path = dir.join("config.toml");
        let mut config = AxiomConfig::default();
        config.agent.first_run_completed = true;
        config.save_to_path(&config_path).expect("save config");
        let mut session = ChatSession::load(&config_path).expect("load session");

        session.set_lens_enabled(false);

        assert!(!session.lens_enabled());
        assert!(session
            .select_skill_cards("write python", 5)
            .expect("select cards")
            .is_empty());
        let _ = fs::remove_dir_all(dir);
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("axiom-cli-chat-test-{nanos}"))
    }
}
