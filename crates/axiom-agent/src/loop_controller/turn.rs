use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{anyhow, Result};
use axiom_engine::{
    execute_tool_with_policy, extract_tool_request, AllowAllApprover, RecordingSideEffectAuditSink,
    SkillExecutionError, SkillExecutionResult, ToolRequest,
};
use axiom_llm::{ChatMessage, ChatRequest, LlmError};

use crate::{compact_messages, parse_todo_update};

use super::{
    events::{
        AgentCapKind, AgentTransition, AgentTransitionKind, GiveUpReason, HookExecution, HookPhase,
        ToolExecutionEvent, ToolExecutionStatus, TransitionCheckpoint, TurnResult,
    },
    hooks::HookBudget,
    join_all::SimpleJoinAll,
    observation::{sanitize_interrupted_content, tool_observation},
    subagent::SUBAGENT_SKILL_ID,
    AgentLoop, TurnProgress,
};

impl<'a> AgentLoop<'a> {
    pub async fn run_turn(&mut self, user_message: ChatMessage) -> Result<TurnResult> {
        let started_at = Instant::now();
        let mut messages = self.system_messages.clone();
        let todo_message_index = messages.len();
        messages.push(ChatMessage {
            role: "system".to_string(),
            content: self.todo.prompt_context(),
        });
        let protected_prefix_len = messages.len();
        messages.extend(self.history.clone());
        messages.push(user_message.clone());

        let mut progress = TurnProgress {
            history_delta: vec![user_message],
            ..TurnProgress::default()
        };
        let mut consecutive_tool_errors = 0;
        let mut hook_budget = HookBudget::new(&self.caps, started_at);
        let mut tools_degraded = false;

        for iteration in 1..=self.caps.max_iterations {
            if self.cancellation.is_cancelled() {
                return self.give_up(
                    GiveUpReason::Cancelled,
                    iteration.saturating_sub(1),
                    progress,
                );
            }
            if started_at.elapsed().as_secs() >= self.caps.max_wall_seconds {
                return self.give_up(
                    GiveUpReason::MaxWallTimeReached,
                    iteration.saturating_sub(1),
                    progress,
                );
            }
            if self.cost_limit_reached(&progress.ledger) {
                return self.give_up(
                    GiveUpReason::MaxCostReached,
                    iteration.saturating_sub(1),
                    progress,
                );
            }

            let context = compact_messages(&messages, protected_prefix_len, self.caps.max_tokens);
            let compacted_this_iteration = context.compacted_messages;
            progress.compacted_messages = progress
                .compacted_messages
                .saturating_add(compacted_this_iteration);
            progress.context_tokens_estimate = context.estimated_tokens;
            messages = context.messages;
            if progress.context_tokens_estimate > u64::from(self.caps.max_tokens) {
                return self.give_up(
                    GiveUpReason::MaxTokensReached,
                    iteration.saturating_sub(1),
                    progress,
                );
            }
            let context_tokens_estimate = progress.context_tokens_estimate;
            self.record_transition(
                &mut progress,
                AgentTransitionKind::PlanPrepared {
                    iteration,
                    context_tokens_estimate,
                    compacted_messages: compacted_this_iteration,
                },
            )?;

            let effective_allow_tools = self.allow_tools && !tools_degraded;
            let request = ChatRequest {
                model: self.model.clone(),
                messages: messages.clone(),
                temperature: self.temperature,
                max_tokens: self.max_response_tokens,
                stream: self.streaming,
                metadata: None,
                provider_options: self.provider_options.clone(),
                tools: if effective_allow_tools {
                    self.tool_definitions.clone()
                } else {
                    Vec::new()
                },
                tool_choice: effective_allow_tools.then(|| "auto".to_string()),
            };
            self.record_transition(
                &mut progress,
                AgentTransitionKind::ProviderRequestPrepared {
                    iteration,
                    provider: self.provider.provider_name().to_string(),
                    model: request.model.clone(),
                    message_count: request.messages.len(),
                    tool_count: request.tools.len(),
                    streaming: request.stream,
                },
            )?;
            if let Some(observer) = self.stream_observer.as_deref_mut() {
                observer.on_step_started();
            }
            let stream_accumulator = Arc::new(Mutex::new(String::new()));
            let acc_sink = stream_accumulator.clone();
            let provider_call = async {
                if self.streaming {
                    let stream = self.provider.stream_chat(request).await?;
                    if let Some(observer) = self.stream_observer.as_deref_mut() {
                        stream
                            .collect_response_with_observer(
                                self.provider.provider_name(),
                                &self.model,
                                |update| {
                                    if !update.visible_delta.is_empty() {
                                        if let Ok(mut text) = acc_sink.lock() {
                                            text.push_str(&update.visible_delta);
                                        }
                                    }
                                    observer.on_stream_update(&update);
                                },
                            )
                            .await
                    } else {
                        stream
                            .collect_response_with_observer(
                                self.provider.provider_name(),
                                &self.model,
                                |update| {
                                    if !update.visible_delta.is_empty() {
                                        if let Ok(mut text) = acc_sink.lock() {
                                            text.push_str(&update.visible_delta);
                                        }
                                    }
                                },
                            )
                            .await
                    }
                } else {
                    self.provider.chat(request).await
                }
            };
            let provider_result = tokio::select! {
                result = provider_call => result,
                _ = self.cancellation.cancelled() => {
                    let partial_str = stream_accumulator
                        .lock()
                        .map(|guard| guard.clone())
                        .unwrap_or_default();
                    if !partial_str.trim().is_empty() {
                        let cleaned = sanitize_interrupted_content(&partial_str);
                        progress.partial = cleaned.clone();
                        progress.history_delta.push(ChatMessage {
                            role: "assistant".to_string(),
                            content: format!(
                                "{}\n\n[Response interrupted by user]",
                                cleaned.trim_end()
                            ),
                        });
                    }
                    return self.give_up(
                        GiveUpReason::Cancelled,
                        iteration.saturating_sub(1),
                        progress,
                    );
                }
            };
            if let Some(observer) = self.stream_observer.as_deref_mut() {
                observer.on_step_finished();
            }
            let response = match provider_result {
                Ok(response) => response,
                Err(error) => {
                    if !tools_degraded
                        && self.allow_tools
                        && !self.tool_definitions.is_empty()
                        && is_no_tool_endpoint_error(&error)
                    {
                        // The provider routed the request to a model with no
                        // tool-capable endpoint (OpenRouter returns HTTP 404
                        // "No endpoints found that support tool use"). Retry
                        // this iteration once without tools instead of
                        // abandoning the turn: the model can still answer, it
                        // just cannot call tools for the rest of the turn.
                        tools_degraded = true;
                        self.record_transition(
                            &mut progress,
                            AgentTransitionKind::ProviderDegradedNoTools {
                                iteration,
                                provider: self.provider.provider_name().to_string(),
                                model: self.model.clone(),
                                error: error.to_string(),
                            },
                        )?;
                        continue;
                    }
                    self.record_transition(
                        &mut progress,
                        AgentTransitionKind::ProviderFailed {
                            iteration,
                            provider: self.provider.provider_name().to_string(),
                            model: self.model.clone(),
                            error: error.to_string(),
                        },
                    )?;
                    return self.give_up(
                        GiveUpReason::ProviderFailed(error.to_string()),
                        iteration,
                        progress,
                    );
                }
            };
            if let Some(usage) = response.usage.as_ref() {
                progress.ledger.record(Some(usage));
            } else {
                let prompt_tokens =
                    u32::try_from(progress.context_tokens_estimate).unwrap_or(u32::MAX);
                let completion_tokens =
                    u32::try_from(response.content.len().div_ceil(4)).unwrap_or(u32::MAX);
                let total_tokens = prompt_tokens.saturating_add(completion_tokens);
                progress.ledger.record(Some(&axiom_llm::TokenUsage {
                    prompt_tokens,
                    completion_tokens,
                    total_tokens,
                }));
            }
            let mut todo_update_applied = false;
            let mut todo_update_error = None;
            let assistant_content = match parse_todo_update(&response.content) {
                Ok(Some(update)) => {
                    if update.todo != self.todo {
                        progress.todo_updates = progress.todo_updates.saturating_add(1);
                    }
                    self.todo = update.todo;
                    messages[todo_message_index].content = self.todo.prompt_context();
                    todo_update_applied = true;
                    update.visible_content
                }
                Ok(None) => response.content.clone(),
                Err(error) => {
                    todo_update_error = Some(error.to_string());
                    response.content.clone()
                }
            };
            progress.partial = assistant_content.clone();
            let effective_assistant_content =
                if assistant_content.trim().is_empty() && !response.tool_calls.is_empty() {
                    response
                        .tool_calls
                        .iter()
                        .map(|call| {
                            let name = &call.name;
                            let args = &call.arguments;
                            format!("[Invoking tool `{name}` with arguments {args}]")
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                } else if assistant_content.trim().is_empty() {
                    "[No response content]".to_string()
                } else {
                    assistant_content.clone()
                };
            let assistant_message = ChatMessage {
                role: "assistant".to_string(),
                content: effective_assistant_content,
            };
            messages.push(assistant_message.clone());
            progress.history_delta.push(assistant_message);
            self.record_transition(
                &mut progress,
                AgentTransitionKind::ProviderResponseReceived {
                    iteration,
                    provider: response.provider.clone(),
                    model: response.model.clone(),
                    content: assistant_content.clone(),
                    tool_calls: response.tool_calls.clone(),
                    usage: response.usage.clone(),
                },
            )?;

            if self.cost_limit_reached(&progress.ledger) {
                return self.give_up(GiveUpReason::MaxCostReached, iteration, progress);
            }

            if let Some(error) = todo_update_error {
                messages.push(ChatMessage {
                    role: "user".to_string(),
                    content: format!(
                        "The axiom-todo control block was rejected: {error}. Emit one corrected complete todo block and continue the original task."
                    ),
                });
                continue;
            }

            let mut tool_requests = Vec::new();
            let mut unresolved_tool_call: Option<String> = None;
            if self.allow_tools {
                for tool_call in &response.tool_calls {
                    let Some(skill_id) = self.skill_id_for_native_tool(&tool_call.name) else {
                        // Some models hallucinate names or emit their internal
                        // recipient format; aborting the turn strands the user
                        // with no recourse. Feed the mismatch back instead so
                        // the next iteration can correct itself.
                        unresolved_tool_call = Some(tool_call.name.clone());
                        break;
                    };
                    tool_requests.push(ToolRequest {
                        skill_id,
                        arguments: tool_call.arguments.clone(),
                    });
                }
            }
            if let Some(name) = unresolved_tool_call {
                messages.push(ChatMessage {
                    role: "user".to_string(),
                    content: if name.trim().is_empty() {
                        "The last response requested a tool with an empty function name. Retry the tool call using the exact name from the provided tools list (native names look like axiom_file_read), or continue answering without tools.".to_string()
                    } else if is_todo_like_tool_name(&name) {
                        format!(
                            "The requested tool `{name}` does not exist as a function in this session. To maintain a plan or task list, emit it as a fenced control block instead of a tool call: \n```axiom-todo\n{{\"items\":[{{\"title\":\"step\",\"status\":\"pending\"}}]}}\n```\n(valid statuses: pending, in_progress, completed, blocked). Then continue the task with the available tools."
                        )
                    } else {
                        format!(
                            "The requested tool `{name}` is not available in this session. Retry using the exact name of one of the provided tools (native names look like axiom_file_read), or continue answering without tools."
                        )
                    },
                });
                continue;
            }
            if tool_requests.is_empty() {
                match extract_tool_request(&assistant_content) {
                    Ok(request) if self.allow_tools => tool_requests.push(request),
                    Ok(_) | Err(SkillExecutionError::MissingToolBlock) => {}
                    Err(_) => {}
                }
            }

            if tool_requests.is_empty() {
                if todo_update_applied
                    && (self.todo.remaining_count() > 0 || assistant_content.is_empty())
                {
                    messages.push(ChatMessage {
                        role: "user".to_string(),
                        content: if self.todo.remaining_count() > 0 {
                            "Continue with the next pending todo item. Request the tools you need; do not stop at the plan.".to_string()
                        } else {
                            "The todo list is terminal. Provide a concise final answer summarizing the result and any blocked items.".to_string()
                        },
                    });
                    continue;
                }
                if response.stream_truncated {
                    // The provider cut the stream before the finish event, so
                    // this "answer" is a mid-sentence fragment (frequently
                    // after a batch of large tool observations). Salvage the
                    // turn by asking the model to continue where it stopped;
                    // if the retry also truncates, the caps end the turn as
                    // before instead of looping forever.
                    messages.push(ChatMessage {
                        role: "user".to_string(),
                        content: "Your previous response was cut off mid-stream before completion. Continue exactly where you stopped and finish the task or answer; do not repeat content already sent.".to_string(),
                    });
                    continue;
                }
                self.record_transition(
                    &mut progress,
                    AgentTransitionKind::Done {
                        iteration,
                        content: assistant_content.clone(),
                    },
                )?;
                return Ok(TurnResult::Done(progress.complete(
                    assistant_content,
                    iteration,
                    self.todo.clone(),
                )));
            }

            let run_parallel = tool_requests.len() > 1
                && tool_requests.iter().all(|r| is_readonly_tool(&r.skill_id));

            if run_parallel {
                // Pre-hooks run before the batch is dispatched: they borrow the
                // loop mutably, which the parallel futures cannot do.
                let mut pre_hooks = Vec::new();
                for request in &tool_requests {
                    pre_hooks.push(
                        self.fire_hooks(HookPhase::Pre, request, &mut hook_budget, &mut progress)
                            .await,
                    );
                }
                let mut futures = Vec::new();
                for request in &tool_requests {
                    let req = request.clone();
                    let installed = self.installed_skills;
                    let ctx = &self.execution_context;
                    let policy = &self.side_effect_policy;
                    let external = self.external_tools;
                    futures.push(Box::pin(async move {
                        let tool_started_at = Instant::now();
                        let mut policy_audit = RecordingSideEffectAuditSink::default();
                        let mut approver = AllowAllApprover;
                        let tool_future = execute_tool_with_policy(
                            &req,
                            installed,
                            ctx,
                            &mut approver,
                            policy,
                            &mut policy_audit,
                            external,
                        );
                        let res = tool_future.await;
                        (req, tool_started_at, policy_audit, res)
                    }));
                }

                let results = SimpleJoinAll::new(futures).await;
                for (index, (request, tool_started_at, policy_audit, tool_result)) in
                    results.into_iter().enumerate()
                {
                    if self.cancellation.is_cancelled() {
                        return self.give_up(GiveUpReason::Cancelled, iteration, progress);
                    }
                    if progress.tool_events.len() >= self.caps.max_tool_iterations as usize {
                        return self.give_up(
                            GiveUpReason::MaxToolIterationsReached,
                            iteration,
                            progress,
                        );
                    }
                    let tool_sequence = u32::try_from(progress.tool_events.len())
                        .unwrap_or(u32::MAX)
                        .saturating_add(1);
                    self.record_transition(
                        &mut progress,
                        AgentTransitionKind::ToolStarted {
                            iteration,
                            tool_sequence,
                            request: request.clone(),
                        },
                    )?;
                    let pre_hooks = pre_hooks.get(index).cloned().unwrap_or_default();
                    progress
                        .policy_decisions
                        .extend(policy_audit.into_decisions());
                    self.record_tool_completion(
                        &request,
                        Some(tool_result),
                        tool_started_at,
                        pre_hooks,
                        &mut hook_budget,
                        &mut progress,
                        &mut messages,
                        iteration,
                        tool_sequence,
                        &mut consecutive_tool_errors,
                    )
                    .await?;
                }
            } else {
                for request in tool_requests {
                    if self.cancellation.is_cancelled() {
                        return self.give_up(GiveUpReason::Cancelled, iteration, progress);
                    }
                    if progress.tool_events.len() >= self.caps.max_tool_iterations as usize {
                        return self.give_up(
                            GiveUpReason::MaxToolIterationsReached,
                            iteration,
                            progress,
                        );
                    }
                    let tool_sequence = u32::try_from(progress.tool_events.len())
                        .unwrap_or(u32::MAX)
                        .saturating_add(1);
                    self.record_transition(
                        &mut progress,
                        AgentTransitionKind::ToolStarted {
                            iteration,
                            tool_sequence,
                            request: request.clone(),
                        },
                    )?;
                    let pre_hooks = self
                        .fire_hooks(HookPhase::Pre, &request, &mut hook_budget, &mut progress)
                        .await;
                    let tool_started_at = Instant::now();
                    let mut policy_audit = RecordingSideEffectAuditSink::default();
                    let tool_result = if request.skill_id == SUBAGENT_SKILL_ID {
                        let (result, child_ledger) = self.run_subagent_tool(&request).await;
                        progress.ledger.merge(&child_ledger);
                        Some(result)
                    } else {
                        let tool_future = execute_tool_with_policy(
                            &request,
                            self.installed_skills,
                            &self.execution_context,
                            &mut *self.approval,
                            &self.side_effect_policy,
                            &mut policy_audit,
                            self.external_tools,
                        );
                        tokio::select! {
                            res = tool_future => Some(res),
                            _ = self.cancellation.cancelled() => None,
                        }
                    };
                    progress
                        .policy_decisions
                        .extend(policy_audit.into_decisions());
                    self.record_tool_completion(
                        &request,
                        tool_result,
                        tool_started_at,
                        pre_hooks,
                        &mut hook_budget,
                        &mut progress,
                        &mut messages,
                        iteration,
                        tool_sequence,
                        &mut consecutive_tool_errors,
                    )
                    .await?;

                    if self.cancellation.is_cancelled() {
                        return self.give_up(GiveUpReason::Cancelled, iteration, progress);
                    }

                    if consecutive_tool_errors >= self.caps.max_consecutive_tool_errors {
                        return self.give_up(
                            GiveUpReason::ConsecutiveToolErrorsReached,
                            iteration,
                            progress,
                        );
                    }
                }
            }

            let had_question_ask = progress
                .tool_events
                .iter()
                .any(|e| e.request.skill_id == "question.ask");
            let reflection_instruction = if had_question_ask {
                ChatMessage {
                    role: "user".to_string(),
                    content: "The user has provided their choice/reply above. Treat their response as the top-priority instruction and deliver the concrete solution, advice, or action now without asking repeated questions.".to_string(),
                }
            } else {
                ChatMessage {
                    role: "user".to_string(),
                    content: "Reflect on all Axiom Tool Results, update your plan if needed, and either request the next necessary tools or provide the final answer to the original request.".to_string(),
                }
            };
            if let Some(last_message) = messages.last_mut().filter(|m| m.role == "user") {
                last_message.content.push_str("\n\n");
                last_message
                    .content
                    .push_str(&reflection_instruction.content);
                if let Some(last_delta) = progress
                    .history_delta
                    .last_mut()
                    .filter(|m| m.role == "user")
                {
                    last_delta.content = last_message.content.clone();
                }
            } else {
                messages.push(reflection_instruction.clone());
                progress.history_delta.push(reflection_instruction.clone());
            }
            self.record_transition(
                &mut progress,
                AgentTransitionKind::ReflectQueued {
                    iteration,
                    instruction: reflection_instruction,
                },
            )?;
        }

        self.give_up(
            GiveUpReason::MaxIterationsReached,
            self.caps.max_iterations,
            progress,
        )
    }

    fn give_up(
        &mut self,
        reason: GiveUpReason,
        iterations: u32,
        mut progress: TurnProgress,
    ) -> Result<TurnResult> {
        let partial = progress.partial.clone();
        if reason == GiveUpReason::Cancelled {
            if !partial.trim().is_empty() {
                let already_recorded = progress
                    .history_delta
                    .last()
                    .is_some_and(|m| m.role == "assistant");
                if !already_recorded {
                    progress.history_delta.push(ChatMessage {
                        role: "assistant".to_string(),
                        content: format!(
                            "{}\n\n[Response interrupted by user]",
                            partial.trim_end()
                        ),
                    });
                }
            }
            self.record_transition(
                &mut progress,
                AgentTransitionKind::CancellationObserved {
                    iteration: iterations,
                },
            )?;
        } else if let GiveUpReason::ProviderFailed(ref err) = reason {
            let already_recorded = progress
                .history_delta
                .last()
                .is_some_and(|m| m.role == "assistant");
            if !already_recorded {
                let note = if !partial.trim().is_empty() {
                    format!(
                        "{}\n\n[Turn interrupted by error: {err}]",
                        partial.trim_end()
                    )
                } else {
                    format!("[Turn interrupted by error: {err}]")
                };
                progress.history_delta.push(ChatMessage {
                    role: "assistant".to_string(),
                    content: note,
                });
            }
        } else if let Some(cap) = cap_kind(&reason) {
            self.record_transition(
                &mut progress,
                AgentTransitionKind::CapReached {
                    iteration: iterations,
                    cap,
                },
            )?;
        }
        self.record_transition(
            &mut progress,
            AgentTransitionKind::GiveUp {
                iteration: iterations,
                reason: reason.clone(),
                partial: partial.clone(),
            },
        )?;
        Ok(TurnResult::GiveUp {
            partial: partial.clone(),
            reason,
            completion: progress.complete(partial, iterations, self.todo.clone()),
        })
    }

    fn record_transition(
        &mut self,
        progress: &mut TurnProgress,
        kind: AgentTransitionKind,
    ) -> Result<()> {
        let transition = AgentTransition {
            sequence: u64::try_from(progress.transitions.len())
                .unwrap_or(u64::MAX)
                .saturating_add(1),
            kind,
        };
        progress.transitions.push(transition.clone());
        if let Some(observer) = self.transition_observer.as_deref_mut() {
            let checkpoint = TransitionCheckpoint {
                transition,
                partial: progress.partial.clone(),
                history_delta: progress.history_delta.clone(),
                tool_events: progress.tool_events.clone(),
                policy_decisions: progress.policy_decisions.clone(),
                ledger: progress.ledger.clone(),
                context_tokens_estimate: progress.context_tokens_estimate,
                compacted_messages: progress.compacted_messages,
                todo: self.todo.clone(),
                todo_updates: progress.todo_updates,
            };
            observer.on_transition(&checkpoint).map_err(|error| {
                anyhow!(
                    "transition checkpoint {} failed: {error}",
                    checkpoint.transition.sequence
                )
            })?;
        }
        Ok(())
    }

    /// Classifies a tool result, runs the outcome hooks, folds the
    /// observation into the transcript, and records the `ToolCompleted`
    /// transition.
    ///
    /// The parallel and sequential dispatch paths differ only in how they
    /// obtain the raw result and the pre-hooks, so everything after the
    /// call itself lives here to keep the two branches from drifting.
    ///
    /// `tool_result` is `None` when execution was cancelled before the
    /// tool returned, which is reported as a non-fatal failure.
    #[allow(clippy::too_many_arguments)]
    async fn record_tool_completion(
        &mut self,
        request: &ToolRequest,
        tool_result: Option<Result<SkillExecutionResult, SkillExecutionError>>,
        tool_started_at: Instant,
        pre_hooks: Vec<HookExecution>,
        hook_budget: &mut HookBudget,
        progress: &mut TurnProgress,
        messages: &mut Vec<ChatMessage>,
        iteration: u32,
        tool_sequence: u32,
        consecutive_tool_errors: &mut u32,
    ) -> Result<()> {
        let status = match tool_result {
            Some(Ok(result)) => {
                *consecutive_tool_errors = 0;
                ToolExecutionStatus::Succeeded(result)
            }
            Some(Err(error)) => {
                let err_str = error.to_string();
                if !is_non_fatal_tool_error(&err_str) {
                    *consecutive_tool_errors = consecutive_tool_errors.saturating_add(1);
                }
                ToolExecutionStatus::Failed(err_str)
            }
            None => ToolExecutionStatus::Failed("Tool execution cancelled by user".to_string()),
        };
        let outcome_phase = match &status {
            ToolExecutionStatus::Succeeded(_) => HookPhase::Post,
            ToolExecutionStatus::Failed(_) => HookPhase::OnError,
        };
        let mut hooks = pre_hooks;
        hooks.extend(
            self.fire_hooks(outcome_phase, request, hook_budget, progress)
                .await,
        );
        let event = ToolExecutionEvent {
            request: request.clone(),
            latency_ms: tool_started_at
                .elapsed()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
            status,
            hooks,
        };
        let observation = tool_observation(&event);
        let observation_message = ChatMessage {
            role: "user".to_string(),
            content: observation.clone(),
        };
        progress.tool_events.push(event.clone());
        if tool_sequence > 1 && messages.last().is_some_and(|m| m.role == "user") {
            let last = messages.last_mut().expect("last message exists");
            last.content.push_str("\n\n");
            last.content.push_str(&observation);
            if let Some(last_delta) = progress
                .history_delta
                .last_mut()
                .filter(|m| m.role == "user")
            {
                last_delta.content = last.content.clone();
            }
        } else {
            messages.push(observation_message.clone());
            progress.history_delta.push(observation_message.clone());
        }
        self.record_transition(
            progress,
            AgentTransitionKind::ToolCompleted {
                iteration,
                tool_sequence,
                event,
                observation: observation_message,
            },
        )
    }
}

/// True when a provider rejected the request because the routed model has no
/// endpoint that supports tool use (for example OpenRouter's HTTP 404 "No
/// endpoints found that support tool use"). Such a turn can still be answered
/// without tools, so the loop degrades to a tool-free request instead of
/// giving up.
pub(super) fn is_no_tool_endpoint_error(error: &LlmError) -> bool {
    let rendered = error.to_string().to_ascii_lowercase();
    rendered.contains("no endpoints found that support tool use")
        || (rendered.contains("tool") && rendered.contains("endpoint"))
}
/// True when a hallucinated tool name is one of the todo/task-list tool
/// names models carry over from other agent harnesses (GLM in particular
/// emits `todo`). These get a corrective hint pointing at the fenced
/// `axiom-todo` block, which is how plans are tracked in this session.
pub(super) fn is_todo_like_tool_name(name: &str) -> bool {
    const TODO_LIKE: &[&str] = &[
        "todo",
        "todos",
        "todowrite",
        "todo_write",
        "todoupdate",
        "todo_update",
        "update_todos",
        "tasklist",
        "task_list",
        "tasklist_write",
    ];
    let cleaned = name.trim().to_ascii_lowercase();
    let cleaned = cleaned
        .strip_prefix("functions.")
        .or_else(|| cleaned.strip_prefix("tools."))
        .unwrap_or(&cleaned);
    TODO_LIKE.contains(&cleaned)
}

fn cap_kind(reason: &GiveUpReason) -> Option<AgentCapKind> {
    match reason {
        GiveUpReason::MaxIterationsReached => Some(AgentCapKind::Iterations),
        GiveUpReason::MaxToolIterationsReached => Some(AgentCapKind::ToolIterations),
        GiveUpReason::MaxWallTimeReached => Some(AgentCapKind::WallTime),
        GiveUpReason::MaxTokensReached => Some(AgentCapKind::Tokens),
        GiveUpReason::MaxCostReached => Some(AgentCapKind::Cost),
        GiveUpReason::ConsecutiveToolErrorsReached => Some(AgentCapKind::ConsecutiveToolErrors),
        GiveUpReason::Cancelled | GiveUpReason::ProviderFailed(_) => None,
    }
}
fn is_readonly_tool(skill_id: &str) -> bool {
    matches!(
        skill_id,
        "file.read"
            | "file.read_many"
            | "project.scan"
            | "git.status"
            | "git.diff"
            | "web.fetch"
            | "github.search"
            | "code.grep"
            | "code.glob"
            | "code.list"
    )
}

/// A tool failure that should not count toward the consecutive-error budget:
/// the user declined it, or the attempt timed out or was cancelled. These
/// are recoverable, so the loop keeps going instead of giving up.
fn is_non_fatal_tool_error(error: &str) -> bool {
    error.contains("approval denied")
        || error.contains("timeout")
        || error.contains("timed out")
        || error.contains("cancelled")
}
