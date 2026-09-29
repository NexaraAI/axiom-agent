use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use axiom_engine::{
    DenyAllApprover, InstalledSkillRecord, PolicyAction, SkillLifecycleState, SkillManifest,
    TrustLevel,
};
use axiom_llm::{
    ChatResponse, ChatStream, ChatToolCall, LlmError, MockProvider, ModelInfo, TokenUsage,
};
use serde_json::{json, Value};

use axiom_engine::{
    AllowAllApprover, ExecutorRegistry, SkillExecutionContext, SkillExecutionError,
    SkillExecutionResult, SkillHooks, ToolRequest,
};
use axiom_llm::ChatRequest;

use crate::{AgentCaps, CancellationToken, UsagePricing};

use super::*;
use super::{
    events::{HookPhase, HookStatus},
    hooks::{HookBudget, MAX_HOOKS_PER_TURN},
    observation::{sanitize_interrupted_content, tool_observation},
    turn::{is_no_tool_endpoint_error, is_todo_like_tool_name},
};

mod hook_tests;

fn context() -> SkillExecutionContext {
    SkillExecutionContext {
        workspace_root: PathBuf::from("."),
        max_file_read_bytes: 1_000,
        web_timeout_secs: 1,
        max_web_response_bytes: 1_000,
        web_fetch_https_only: true,
        web_fetch_allowed_hosts: Vec::new(),
        web_fetch_denied_hosts: Vec::new(),
        web_fetch_use_system_proxy: false,
        auto_approve_medium_risk: false,
        credential_env_names: Vec::new(),
        skills_dir: None,
    }
}

#[test]
fn native_function_names_are_provider_safe_and_deterministic() {
    assert_eq!(native_tool_name("file.read"), "axiom_file_read");
    assert_eq!(native_tool_name("git.diff"), "axiom_git_diff");
}

#[test]
fn native_tool_definitions_use_registered_executor_input_schemas() {
    let descriptors = ExecutorRegistry::with_builtin_executors().descriptors();
    let installed = descriptors
        .iter()
        .map(|descriptor| installed_tool(&descriptor.id))
        .collect::<Vec<_>>();
    let provider = MockProvider::new("mock");
    let mut approval = DenyAllApprover;
    let agent = AgentLoop::new(
        &provider,
        "mock-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &installed,
        context(),
        &mut approval,
    );

    assert_eq!(agent.tool_definitions.len(), descriptors.len());
    for descriptor in descriptors {
        let definition = agent
            .tool_definitions
            .iter()
            .find(|definition| definition.name == native_tool_name(&descriptor.id))
            .expect("registered executor is advertised");
        assert_eq!(definition.parameters, descriptor.input_schema);
    }
}

#[test]
fn unsupported_installed_tools_are_not_advertised_to_the_provider() {
    let provider = MockProvider::new("mock");
    let installed = [installed_tool("custom.tool")];
    let mut approval = DenyAllApprover;
    let agent = AgentLoop::new(
        &provider,
        "mock-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &installed,
        context(),
        &mut approval,
    );

    assert!(agent.tool_definitions.is_empty());
}

#[tokio::test]
async fn subagent_delegation_runs_an_isolated_turn_and_reports_usage() {
    let provider = MockProvider::new("mock");
    let installed = [installed_tool("file.read"), installed_tool("code.grep")];
    let mut approval = DenyAllApprover;
    let agent = AgentLoop::new(
        &provider,
        "mock-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &installed,
        context(),
        &mut approval,
    );

    let request = ToolRequest {
        skill_id: "subagent.run".to_string(),
        arguments: json!({"role": "Explorer", "task": "Audit token handling"}),
    };
    let (result, ledger) = agent.run_subagent_tool(&request).await;
    let result = result.expect("sub-agent runs through the provider");

    assert_eq!(result.skill_id, "subagent.run");
    assert_eq!(result.output["status"], "completed");
    assert_eq!(result.output["role"], "Explorer");
    assert!(result.output["summary"]
        .as_str()
        .expect("summary")
        .contains("Audit token handling"));
    assert!(ledger.total_tokens > 0);
}

#[tokio::test]
async fn subagent_delegation_refuses_nesting_and_requires_role_and_task() {
    let provider = MockProvider::new("mock");
    let mut approval = DenyAllApprover;
    let nested = AgentLoop::new(
        &provider,
        "mock-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &[],
        context(),
        &mut approval,
    )
    .with_subagent_depth(1);
    let (nested_result, _) = nested
        .run_subagent_tool(&ToolRequest {
            skill_id: "subagent.run".to_string(),
            arguments: json!({"role": "Explorer", "task": "x"}),
        })
        .await;
    assert!(matches!(
        nested_result.expect_err("nested sub-agent must fail"),
        SkillExecutionError::ExecutionFailed { .. }
    ));

    let agent = AgentLoop::new(
        &provider,
        "mock-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &[],
        context(),
        &mut approval,
    );
    let (missing, _) = agent
        .run_subagent_tool(&ToolRequest {
            skill_id: "subagent.run".to_string(),
            arguments: json!({"task": "x"}),
        })
        .await;
    assert!(matches!(
        missing.expect_err("missing role must fail"),
        SkillExecutionError::MissingArgument { .. }
    ));
}

#[tokio::test]
async fn finishes_when_the_provider_returns_a_normal_response() {
    let provider = MockProvider::new("mock");
    let mut approval = DenyAllApprover;
    let mut agent = AgentLoop::new(
        &provider,
        "mock-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &[],
        context(),
        &mut approval,
    );

    let result = agent
        .run_turn(ChatMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
        })
        .await
        .expect("turn succeeds");

    let TurnResult::Done(completion) = result else {
        panic!("normal mock response should finish");
    };
    assert_eq!(completion.iterations, 1);
    assert_eq!(completion.content, "Axiom (offline): hello");
    assert_eq!(completion.history_delta.len(), 2);
}

#[tokio::test]
async fn observes_a_tool_failure_then_reflects_to_a_final_answer() {
    let provider = MockProvider::new("mock");
    let mut approval = DenyAllApprover;
    let mut agent = AgentLoop::new(
        &provider,
        "mock-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &[],
        context(),
        &mut approval,
    );

    let result = agent
        .run_turn(ChatMessage {
            role: "user".to_string(),
            content: "read README.md".to_string(),
        })
        .await
        .expect("turn succeeds");

    let TurnResult::Done(completion) = result else {
        panic!("mock provider should reflect after the observation");
    };
    assert_eq!(completion.iterations, 2);
    assert_eq!(completion.tool_events.len(), 1);
    assert!(matches!(
        completion.tool_events[0].status,
        ToolExecutionStatus::Failed(_)
    ));
    assert_eq!(completion.content, "Result verified and summarized.");
}

#[tokio::test]
async fn gives_up_at_the_iteration_cap_after_a_tool_request() {
    let provider = MockProvider::new("mock");
    let mut approval = DenyAllApprover;
    let caps = AgentCaps {
        max_iterations: 1,
        ..AgentCaps::default()
    };
    let mut agent = AgentLoop::new(
        &provider,
        "mock-model",
        caps,
        Vec::new(),
        Vec::new(),
        &[],
        context(),
        &mut approval,
    );

    let result = agent
        .run_turn(ChatMessage {
            role: "user".to_string(),
            content: "read README.md".to_string(),
        })
        .await
        .expect("turn succeeds");

    assert!(matches!(
        result,
        TurnResult::GiveUp {
            reason: GiveUpReason::MaxIterationsReached,
            ..
        }
    ));
}

#[tokio::test]
async fn gives_up_when_configured_pricing_reaches_the_cost_cap() {
    let provider = MockProvider::new("mock");
    let mut approval = DenyAllApprover;
    let caps = AgentCaps {
        max_cost_usd: 0.000_001,
        ..AgentCaps::default()
    };
    let mut agent = AgentLoop::new(
        &provider,
        "mock-model",
        caps,
        Vec::new(),
        Vec::new(),
        &[],
        context(),
        &mut approval,
    )
    .with_pricing(UsagePricing::new(Some(1.0), Some(1.0)));

    let result = agent
        .run_turn(ChatMessage {
            role: "user".to_string(),
            content: "hello".to_string(),
        })
        .await
        .expect("turn succeeds");

    assert!(matches!(
        result,
        TurnResult::GiveUp {
            reason: GiveUpReason::MaxCostReached,
            ..
        }
    ));
}

#[tokio::test]
async fn compacts_long_history_before_calling_the_provider() {
    let provider = MockProvider::new("mock");
    let mut approval = DenyAllApprover;
    let caps = AgentCaps {
        max_tokens: 300,
        ..AgentCaps::default()
    };
    let history = (0..30)
        .map(|index| ChatMessage {
            role: if index % 2 == 0 { "user" } else { "assistant" }.to_string(),
            content: format!("history {index}: {}", "detail ".repeat(12)),
        })
        .collect();
    let mut agent = AgentLoop::new(
        &provider,
        "mock-model",
        caps,
        Vec::new(),
        history,
        &[],
        context(),
        &mut approval,
    );

    let result = agent
        .run_turn(ChatMessage {
            role: "user".to_string(),
            content: "finish this".to_string(),
        })
        .await
        .expect("turn succeeds");
    let TurnResult::Done(completion) = result else {
        panic!("compacted context should fit");
    };

    assert!(completion.compacted_messages > 0);
    assert!(completion.context_tokens_estimate <= 300);
    assert!(completion.content.contains("finish this"));
}

#[tokio::test]
async fn streaming_path_accumulates_content_and_usage() {
    let provider = MockProvider::new("mock");
    let mut approval = DenyAllApprover;
    let mut agent = AgentLoop::new(
        &provider,
        "mock-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &[],
        context(),
        &mut approval,
    )
    .with_streaming(true);

    let result = agent
        .run_turn(ChatMessage {
            role: "user".to_string(),
            content: "stream this".to_string(),
        })
        .await
        .expect("streaming turn succeeds");
    let TurnResult::Done(completion) = result else {
        panic!("streaming response should complete");
    };

    assert_eq!(completion.content, "Axiom (offline): stream this");
    assert!(completion.ledger.total_tokens > 0);
}

#[tokio::test]
async fn cancelled_turn_gives_up_before_calling_provider() {
    let provider = MultiToolProvider::default();
    let mut approval = DenyAllApprover;
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let mut agent = AgentLoop::new(
        &provider,
        "test-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &[],
        context(),
        &mut approval,
    )
    .with_cancellation(cancellation);

    let result = agent
        .run_turn(ChatMessage {
            role: "user".to_string(),
            content: "cancel me".to_string(),
        })
        .await
        .expect("cancelled turn returns a result");

    assert!(matches!(
        result,
        TurnResult::GiveUp {
            reason: GiveUpReason::Cancelled,
            ..
        }
    ));
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn executes_all_native_tool_calls_from_one_response() {
    let provider = MultiToolProvider::default();
    let mut approval = DenyAllApprover;
    let skills = vec![installed_tool("file.read")];
    let mut agent = AgentLoop::new(
        &provider,
        "test-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &skills,
        context(),
        &mut approval,
    );

    let result = agent
        .run_turn(ChatMessage {
            role: "user".to_string(),
            content: "read both".to_string(),
        })
        .await
        .expect("multi-tool turn succeeds");
    let TurnResult::Done(completion) = result else {
        panic!("provider should finish after tool observations");
    };

    assert_eq!(completion.content, "both tool results received");
    assert!(completion.history_delta[1]
        .content
        .contains("[Invoking tool `axiom_file_read`"));
    assert_eq!(completion.tool_events.len(), 2);
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn namespace_prefixed_native_tool_name_resolves() {
    // GLM-family models sometimes emit their internal recipient format
    // (`functions.axiom_file_read`) instead of the bare wire name.
    let provider = ToolCallProvider::new(vec![(
        "functions.axiom_file_read".to_string(),
        serde_json::json!({}),
    )]);
    let mut approval = DenyAllApprover;
    let skills = vec![installed_tool("file.read")];
    let mut agent = AgentLoop::new(
        &provider,
        "test-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &skills,
        context(),
        &mut approval,
    );

    let result = agent
        .run_turn(ChatMessage {
            role: "user".to_string(),
            content: "read it".to_string(),
        })
        .await
        .expect("turn succeeds");
    let TurnResult::Done(completion) = result else {
        panic!("namespace-prefixed tool call should execute, not abort");
    };

    assert_eq!(completion.content, "hook turn finished");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn unknown_tool_name_gets_corrective_retry_instead_of_abort() {
    let provider = ToolCallProvider::new(vec![(
        "axiom_does_not_exist".to_string(),
        serde_json::json!({}),
    )]);
    let mut approval = DenyAllApprover;
    let skills = vec![installed_tool("file.read")];
    let mut agent = AgentLoop::new(
        &provider,
        "test-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &skills,
        context(),
        &mut approval,
    );

    let result = agent
        .run_turn(ChatMessage {
            role: "user".to_string(),
            content: "do the thing".to_string(),
        })
        .await
        .expect("turn must not be aborted by an unknown tool name");
    let TurnResult::Done(completion) = result else {
        panic!("unknown tool name should trigger a corrective retry, not give up");
    };

    // First request carries the bogus name; the corrective message lets
    // the provider finish with a plain answer on the retry.
    assert_eq!(completion.content, "hook turn finished");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn todo_tool_hallucination_gets_fenced_block_hint() {
    let provider = ToolCallProvider::new(vec![("todo".to_string(), serde_json::json!({}))]);
    let mut approval = DenyAllApprover;
    let skills = vec![installed_tool("file.read")];
    let mut agent = AgentLoop::new(
        &provider,
        "test-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &skills,
        context(),
        &mut approval,
    );

    let result = agent
        .run_turn(ChatMessage {
            role: "user".to_string(),
            content: "plan and go".to_string(),
        })
        .await
        .expect("turn must not die on a todo tool call");
    let TurnResult::Done(completion) = result else {
        panic!("todo hallucination should correct, not give up");
    };

    assert_eq!(completion.content, "hook turn finished");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn truncated_final_answer_continues_the_turn_instead_of_ending() {
    // Provider hands back a salvaged-but-truncated response (no finish
    // event), then a proper answer after the continuation nudge.
    struct TruncatingProvider;
    #[async_trait]
    impl LlmProvider for TruncatingProvider {
        async fn chat(&self, request: ChatRequest) -> axiom_llm::Result<ChatResponse> {
            let call = self.calls();
            if call == 0 {
                Ok(ChatResponse {
                    content:
                        "The initial search shows two distinct GitHub identities with the same br"
                            .to_string(),
                    usage: Some(TokenUsage {
                        prompt_tokens: 10,
                        completion_tokens: 5,
                        total_tokens: 15,
                    }),
                    model: request.model,
                    provider: "truncating".to_string(),
                    raw: None,
                    tool_calls: Vec::new(),
                    stream_truncated: true,
                })
            } else {
                Ok(ChatResponse {
                    content:
                        "Full synthesis: user and org are distinct accounts sharing the brand."
                            .to_string(),
                    usage: Some(TokenUsage {
                        prompt_tokens: 10,
                        completion_tokens: 5,
                        total_tokens: 15,
                    }),
                    model: request.model,
                    provider: "truncating".to_string(),
                    raw: None,
                    tool_calls: Vec::new(),
                    stream_truncated: false,
                })
            }
        }
        async fn stream_chat(&self, _request: ChatRequest) -> axiom_llm::Result<ChatStream> {
            Err(LlmError::NotImplemented("not used"))
        }
        async fn models(&self) -> axiom_llm::Result<Vec<ModelInfo>> {
            Ok(Vec::new())
        }
        fn provider_name(&self) -> &str {
            "truncating"
        }
    }
    impl TruncatingProvider {
        fn calls(&self) -> usize {
            use std::sync::atomic::AtomicUsize;
            static CALLS: AtomicUsize = AtomicUsize::new(0);
            CALLS.fetch_add(1, Ordering::SeqCst)
        }
    }

    let provider = TruncatingProvider;
    let mut approval = DenyAllApprover;
    let skills = vec![installed_tool("file.read")];
    let mut agent = AgentLoop::new(
        &provider,
        "test-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &skills,
        context(),
        &mut approval,
    );

    let result = agent
        .run_turn(ChatMessage {
            role: "user".to_string(),
            content: "research it".to_string(),
        })
        .await
        .expect("turn survives a truncated response");
    let TurnResult::Done(completion) = result else {
        panic!("truncated answer should continue the turn, not end it");
    };

    assert_eq!(
        completion.content,
        "Full synthesis: user and org are distinct accounts sharing the brand."
    );
}

#[test]
fn todo_like_names_are_classified() {
    assert!(is_todo_like_tool_name("todo"));
    assert!(is_todo_like_tool_name("functions.todo"));
    assert!(is_todo_like_tool_name("  TodoWrite "));
    assert!(is_todo_like_tool_name("tasklist"));
    assert!(!is_todo_like_tool_name("axiom_file_read"));
    assert!(!is_todo_like_tool_name(""));
}

#[tokio::test]
async fn checkpoints_tool_completion_before_cancellation_and_next_tool() {
    let provider = MultiToolProvider::default();
    let mut approval = DenyAllApprover;
    let skills = vec![installed_tool("file.read")];
    let cancellation = CancellationToken::new();
    let cancel_from_observer = cancellation.clone();
    let mut checkpoints = Vec::new();
    let mut observer = |checkpoint: &TransitionCheckpoint| {
        checkpoints.push(checkpoint.clone());
        if matches!(
            &checkpoint.transition.kind,
            AgentTransitionKind::ToolCompleted { .. }
        ) {
            cancel_from_observer.cancel();
        }
        Ok(())
    };
    let mut agent = AgentLoop::new(
        &provider,
        "test-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &skills,
        context(),
        &mut approval,
    )
    .with_cancellation(cancellation)
    .with_transition_observer(&mut observer);

    let result = agent
        .run_turn(ChatMessage {
            role: "user".to_string(),
            content: "read both".to_string(),
        })
        .await
        .expect("turn");
    drop(agent);

    let TurnResult::GiveUp {
        reason, completion, ..
    } = result
    else {
        panic!("observer cancellation should stop the turn");
    };
    assert_eq!(reason, GiveUpReason::Cancelled);
    assert_eq!(completion.tool_events.len(), 1);
    assert_eq!(completion.policy_decisions.len(), 1);
    let kinds = checkpoints
        .iter()
        .map(|checkpoint| &checkpoint.transition.kind)
        .collect::<Vec<_>>();
    let completed = kinds
        .iter()
        .position(|kind| matches!(kind, AgentTransitionKind::ToolCompleted { .. }))
        .expect("tool completed transition");
    let cancelled = kinds
        .iter()
        .position(|kind| matches!(kind, AgentTransitionKind::CancellationObserved { .. }))
        .expect("cancel transition");
    assert!(completed < cancelled);
    assert!(checkpoints
        .windows(2)
        .all(|pair| pair[0].transition.sequence + 1 == pair[1].transition.sequence));
}

#[tokio::test]
async fn applies_todo_transitions_and_continues_until_terminal() {
    let provider = TodoProvider::default();
    let mut approval = DenyAllApprover;
    let mut agent = AgentLoop::new(
        &provider,
        "test-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &[],
        context(),
        &mut approval,
    );

    let result = agent
        .run_turn(ChatMessage {
            role: "user".to_string(),
            content: "do the work".to_string(),
        })
        .await
        .expect("todo turn succeeds");
    let TurnResult::Done(completion) = result else {
        panic!("terminal todo should finish");
    };

    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    assert_eq!(completion.todo_updates, 2);
    assert_eq!(completion.todo.completed_count(), 1);
    assert_eq!(completion.todo.remaining_count(), 0);
    assert_eq!(completion.content, "All work completed.");
    assert!(!completion.content.contains("axiom-todo"));
}

#[derive(Default)]
struct MultiToolProvider {
    calls: AtomicUsize,
}

#[derive(Default)]
struct TodoProvider {
    calls: AtomicUsize,
}

#[async_trait]
impl LlmProvider for TodoProvider {
    async fn chat(&self, request: ChatRequest) -> axiom_llm::Result<ChatResponse> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let content = if call == 0 {
            "```axiom-todo\n{\"items\":[{\"title\":\"Do work\",\"status\":\"in_progress\"}]}\n```"
        } else {
            "All work completed.\n```axiom-todo\n{\"items\":[{\"title\":\"Do work\",\"status\":\"completed\"}]}\n```"
        };
        Ok(ChatResponse {
            content: content.to_string(),
            usage: Some(TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            }),
            model: request.model,
            provider: "todo".to_string(),
            raw: None,
            tool_calls: Vec::new(),
            stream_truncated: false,
        })
    }

    async fn stream_chat(&self, _request: ChatRequest) -> axiom_llm::Result<ChatStream> {
        Err(LlmError::NotImplemented("not used in this test"))
    }

    async fn models(&self) -> axiom_llm::Result<Vec<ModelInfo>> {
        Ok(Vec::new())
    }

    fn provider_name(&self) -> &str {
        "todo"
    }
}

#[async_trait]
impl LlmProvider for MultiToolProvider {
    async fn chat(&self, request: ChatRequest) -> axiom_llm::Result<ChatResponse> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let (content, tool_calls) = if call == 0 {
            (
                String::new(),
                vec![
                    ChatToolCall {
                        id: Some("call_1".to_string()),
                        name: "axiom_file_read".to_string(),
                        arguments: json!({ "path": "rust-toolchain.toml" }),
                    },
                    ChatToolCall {
                        id: Some("call_2".to_string()),
                        name: "axiom_file_read".to_string(),
                        arguments: json!({ "path": "Cargo.toml" }),
                    },
                ],
            )
        } else {
            ("both tool results received".to_string(), Vec::new())
        };
        Ok(ChatResponse {
            content,
            usage: Some(TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            }),
            model: request.model,
            provider: "multi".to_string(),
            raw: None,
            tool_calls,
            stream_truncated: false,
        })
    }

    async fn stream_chat(&self, _request: ChatRequest) -> axiom_llm::Result<ChatStream> {
        Err(LlmError::NotImplemented("not used in this test"))
    }

    async fn models(&self) -> axiom_llm::Result<Vec<ModelInfo>> {
        Ok(Vec::new())
    }

    fn provider_name(&self) -> &str {
        "multi"
    }
}

#[tokio::test]
async fn provider_failure_preserves_history_and_gives_up() {
    struct FailingProvider;
    #[async_trait]
    impl LlmProvider for FailingProvider {
        async fn chat(&self, _request: ChatRequest) -> axiom_llm::Result<ChatResponse> {
            Err(LlmError::StreamDisconnected {
                provider: "failing".to_string(),
            })
        }
        async fn stream_chat(&self, _request: ChatRequest) -> axiom_llm::Result<ChatStream> {
            Err(LlmError::StreamDisconnected {
                provider: "failing".to_string(),
            })
        }
        async fn models(&self) -> axiom_llm::Result<Vec<ModelInfo>> {
            Ok(Vec::new())
        }
        fn provider_name(&self) -> &str {
            "failing"
        }
    }

    let provider = FailingProvider;
    let mut approval = DenyAllApprover;
    let skills = Vec::new();
    let mut agent = AgentLoop::new(
        &provider,
        "test-model",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &skills,
        context(),
        &mut approval,
    );

    let result = agent
        .run_turn(ChatMessage {
            role: "user".to_string(),
            content: "do something".to_string(),
        })
        .await
        .expect("turn should gracefully give up instead of failing");

    let TurnResult::GiveUp {
        reason, completion, ..
    } = result
    else {
        panic!("expected GiveUp on provider failure");
    };

    assert!(matches!(reason, GiveUpReason::ProviderFailed(_)));
    assert_eq!(completion.history_delta.len(), 2);
    assert_eq!(completion.history_delta[0].role, "user");
    assert_eq!(completion.history_delta[0].content, "do something");
    assert_eq!(completion.history_delta[1].role, "assistant");
    assert!(completion.history_delta[1]
        .content
        .contains("[Turn interrupted by error:"));
}

#[tokio::test]
async fn no_tool_endpoint_error_degrades_to_tool_free_turn_and_completes() {
    const NO_ENDPOINT_SUMMARY: &str =
        "No endpoints found that support tool use. Try disabling \"axiom_code_glob\".";
    struct NoToolEndpointOnce;
    #[async_trait]
    impl LlmProvider for NoToolEndpointOnce {
        async fn chat(&self, request: ChatRequest) -> axiom_llm::Result<ChatResponse> {
            if request.tools.is_empty() {
                Ok(ChatResponse {
                    content: "answered without tools".to_string(),
                    usage: Some(TokenUsage {
                        prompt_tokens: 1,
                        completion_tokens: 1,
                        total_tokens: 2,
                    }),
                    model: request.model,
                    provider: "openrouter".to_string(),
                    raw: None,
                    tool_calls: Vec::new(),
                    stream_truncated: false,
                })
            } else {
                Err(LlmError::HttpStatus {
                    provider: "openrouter".to_string(),
                    status: 404,
                    body_summary: NO_ENDPOINT_SUMMARY.to_string(),
                })
            }
        }
        async fn stream_chat(&self, mut request: ChatRequest) -> axiom_llm::Result<ChatStream> {
            request.stream = false;
            Ok(ChatStream::from_response(self.chat(request).await?))
        }
        async fn models(&self) -> axiom_llm::Result<Vec<ModelInfo>> {
            Ok(Vec::new())
        }
        fn provider_name(&self) -> &str {
            "openrouter"
        }
    }

    let provider = NoToolEndpointOnce;
    let mut approval = DenyAllApprover;
    let skills = vec![installed_tool("file.read")];
    let mut agent = AgentLoop::new(
        &provider,
        "z-ai/glm-5.2:free",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &skills,
        context(),
        &mut approval,
    );

    let result = agent
        .run_turn(ChatMessage {
            role: "user".to_string(),
            content: "answer me".to_string(),
        })
        .await
        .expect("turn should degrade instead of failing");

    let TurnResult::Done(completion) = result else {
        panic!("expected the turn to complete after degrading to no tools");
    };
    assert!(completion.content.contains("answered without tools"));
    assert!(matches!(
        completion.transitions.first().map(|t| &t.kind),
        Some(AgentTransitionKind::PlanPrepared { .. })
    ));
    assert!(completion.transitions.iter().any(|transition| matches!(
        &transition.kind,
        AgentTransitionKind::ProviderDegradedNoTools { iteration: 1, .. }
    )));
    let prepared_without_tools = completion
        .transitions
        .iter()
        .filter(|transition| {
            matches!(
                &transition.kind,
                AgentTransitionKind::ProviderRequestPrepared { tool_count: 0, .. }
            )
        })
        .count();
    assert_eq!(
        prepared_without_tools, 1,
        "the retried request must be sent without tools"
    );
}

#[tokio::test]
async fn degraded_retry_failure_gives_up_without_looping() {
    struct AlwaysNoToolEndpoint;
    #[async_trait]
    impl LlmProvider for AlwaysNoToolEndpoint {
        async fn chat(&self, _request: ChatRequest) -> axiom_llm::Result<ChatResponse> {
            Err(LlmError::HttpStatus {
                provider: "openrouter".to_string(),
                status: 404,
                body_summary: "No endpoints found that support tool use.".to_string(),
            })
        }
        async fn stream_chat(&self, _request: ChatRequest) -> axiom_llm::Result<ChatStream> {
            Err(LlmError::HttpStatus {
                provider: "openrouter".to_string(),
                status: 404,
                body_summary: "No endpoints found that support tool use.".to_string(),
            })
        }
        async fn models(&self) -> axiom_llm::Result<Vec<ModelInfo>> {
            Ok(Vec::new())
        }
        fn provider_name(&self) -> &str {
            "openrouter"
        }
    }

    let provider = AlwaysNoToolEndpoint;
    let mut approval = DenyAllApprover;
    let skills = vec![installed_tool("file.read")];
    let mut agent = AgentLoop::new(
        &provider,
        "z-ai/glm-5.2:free",
        AgentCaps::default(),
        Vec::new(),
        Vec::new(),
        &skills,
        context(),
        &mut approval,
    );

    let result = agent
        .run_turn(ChatMessage {
            role: "user".to_string(),
            content: "answer me".to_string(),
        })
        .await
        .expect("turn should give up gracefully");

    let TurnResult::GiveUp {
        reason, completion, ..
    } = result
    else {
        panic!("expected GiveUp when the retry also fails");
    };
    assert!(matches!(reason, GiveUpReason::ProviderFailed(_)));
    let prepared = completion
        .transitions
        .iter()
        .filter(|transition| {
            matches!(
                &transition.kind,
                AgentTransitionKind::ProviderRequestPrepared { .. }
            )
        })
        .count();
    assert_eq!(
        prepared, 2,
        "exactly one degraded retry, then give up - no further attempts"
    );
}

#[test]
fn no_tool_endpoint_errors_are_classified_precisely() {
    let hit = LlmError::HttpStatus {
        provider: "openrouter".to_string(),
        status: 404,
        body_summary: "No endpoints found that support tool use.".to_string(),
    };
    assert!(is_no_tool_endpoint_error(&hit));

    let miss = LlmError::HttpStatus {
        provider: "opencode".to_string(),
        status: 403,
        body_summary: "FreeTierError".to_string(),
    };
    assert!(!is_no_tool_endpoint_error(&miss));
}

fn installed_tool(skill_id: &str) -> InstalledSkill {
    let manifest = SkillManifest::parse_toml(&format!(
        r#"
id = "{skill_id}"
name = "Test Tool"
version = "0.1.0"
description = "Test tool."
category = "test"
skill_type = "tool"
risk_level = "low"
permissions = ["file_system_read"]
platforms = ["windows", "linux", "macos"]
entrypoint = "builtin:{skill_id}"
author = "Axiom Agent"
license = "MIT"
min_axiom_version = "0.1.0"
"#
    ))
    .expect("manifest parses");

    InstalledSkill {
        record: InstalledSkillRecord {
            id: skill_id.to_string(),
            version: "0.1.0".parse().expect("version"),
            installed_at: "test".to_string(),
            updated_at: None,
            source: "test".to_string(),
            registry_url: None,
            manifest_url: None,
            checksum: None,
            enabled: true,
            state: SkillLifecycleState::Enabled,
            trust_level: TrustLevel::Trusted,
            last_checked_at: None,
            last_update_error: None,
            last_runtime_error: None,
            success_count: 0,
            failure_count: 0,
            last_used_at: None,
            average_latency_ms: None,
        },
        manifest,
    }
}

/// An installed tool manifest carrying `[hooks]` declarations.
fn installed_tool_with_hooks(
    skill_id: &str,
    pre: Option<&str>,
    post: Option<&str>,
    on_error: Option<&str>,
) -> InstalledSkill {
    let mut skill = installed_tool(skill_id);
    skill.manifest.hooks = SkillHooks {
        pre: pre.map(ToString::to_string),
        post: post.map(ToString::to_string),
        on_error: on_error.map(ToString::to_string),
    };
    skill
}

/// Requests the configured tool calls once, then answers with plain text.
struct ToolCallProvider {
    tool_calls: Vec<(String, Value)>,
    calls: AtomicUsize,
}

impl ToolCallProvider {
    fn new(tool_calls: Vec<(String, Value)>) -> Self {
        Self {
            tool_calls,
            calls: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl LlmProvider for ToolCallProvider {
    async fn chat(&self, request: ChatRequest) -> axiom_llm::Result<ChatResponse> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        let (content, tool_calls) = if call == 0 {
            (
                String::new(),
                self.tool_calls
                    .iter()
                    .enumerate()
                    .map(|(index, (name, arguments))| ChatToolCall {
                        id: Some(format!("call_{index}")),
                        name: name.clone(),
                        arguments: arguments.clone(),
                    })
                    .collect(),
            )
        } else {
            ("hook turn finished".to_string(), Vec::new())
        };
        Ok(ChatResponse {
            content,
            usage: Some(TokenUsage {
                prompt_tokens: 10,
                completion_tokens: 5,
                total_tokens: 15,
            }),
            model: request.model,
            provider: "tool-call".to_string(),
            raw: None,
            tool_calls,
            stream_truncated: false,
        })
    }

    async fn stream_chat(&self, _request: ChatRequest) -> axiom_llm::Result<ChatStream> {
        Err(LlmError::NotImplemented("not used in this test"))
    }

    async fn models(&self) -> axiom_llm::Result<Vec<ModelInfo>> {
        Ok(Vec::new())
    }

    fn provider_name(&self) -> &str {
        "tool-call"
    }
}

fn user_message(content: &str) -> ChatMessage {
    ChatMessage {
        role: "user".to_string(),
        content: content.to_string(),
    }
}

/// The observation the model saw for the first tool call of a turn.
fn first_tool_observation(completion: &TurnCompletion) -> String {
    completion
        .transitions
        .iter()
        .find_map(|transition| match &transition.kind {
            AgentTransitionKind::ToolCompleted { observation, .. } => {
                Some(observation.content.clone())
            }
            _ => None,
        })
        .expect("a tool completion transition is recorded")
}

fn done_turn(result: TurnResult) -> TurnCompletion {
    match result {
        TurnResult::Done(completion) => completion,
        TurnResult::GiveUp { reason, .. } => {
            panic!("expected a completed turn, got {reason:?}")
        }
    }
}
