use anyhow::Result;
use axiom_engine::{SideEffectDecision, SkillExecutionResult, ToolRequest};
use axiom_llm::{ChatMessage, ChatStreamUpdate};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{TodoList, UsageLedger};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GiveUpReason {
    MaxIterationsReached,
    MaxToolIterationsReached,
    MaxWallTimeReached,
    MaxTokensReached,
    MaxCostReached,
    ConsecutiveToolErrorsReached,
    Cancelled,
    ProviderFailed(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ToolExecutionStatus {
    Succeeded(SkillExecutionResult),
    Failed(String),
}

/// Which manifest hook (`hooks.pre`, `hooks.post`, `hooks.on_error`) is firing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookPhase {
    /// Declared as `hooks.pre`; runs before the skill executes.
    Pre,
    /// Declared as `hooks.post`; runs after the skill succeeds.
    Post,
    /// Declared as `hooks.on_error`; runs after the skill fails.
    OnError,
}

impl HookPhase {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pre => "pre",
            Self::Post => "post",
            Self::OnError => "on_error",
        }
    }
}

/// How a manifest hook invocation ended.
///
/// Only [`HookStatus::Succeeded`] and [`HookStatus::Failed`] mean the hook
/// actually ran; every other variant records a guard that refused to run it, so
/// a hook which did not fire is visible instead of silently dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookStatus {
    Succeeded,
    Failed,
    /// The hook id is neither an executable installed skill nor a built-in.
    Unavailable,
    /// Firing the hook would re-enter the loop (hook depth, or self reference).
    SkippedReentrancy,
    /// The turn's hook allowance or wall clock is exhausted.
    SkippedBudget,
    SkippedCancelled,
}

/// One manifest hook the loop fired, or deliberately did not fire.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HookExecution {
    pub hook_id: String,
    pub phase: HookPhase,
    pub status: HookStatus,
    pub latency_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl HookExecution {
    pub(super) fn skipped(hook_id: &str, phase: HookPhase, status: HookStatus) -> Self {
        Self {
            hook_id: hook_id.to_string(),
            phase,
            status,
            latency_ms: 0,
            output: None,
            error: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolExecutionEvent {
    pub request: ToolRequest,
    pub latency_ms: u64,
    pub status: ToolExecutionStatus,
    /// Manifest hooks fired around this call, in execution order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<HookExecution>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCapKind {
    Iterations,
    ToolIterations,
    WallTime,
    Tokens,
    Cost,
    ConsecutiveToolErrors,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentTransition {
    pub sequence: u64,
    pub kind: AgentTransitionKind,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentTransitionKind {
    PlanPrepared {
        iteration: u32,
        context_tokens_estimate: u64,
        compacted_messages: usize,
    },
    ProviderRequestPrepared {
        iteration: u32,
        provider: String,
        model: String,
        message_count: usize,
        tool_count: usize,
        streaming: bool,
    },
    ProviderResponseReceived {
        iteration: u32,
        provider: String,
        model: String,
        content: String,
        tool_calls: Vec<axiom_llm::ChatToolCall>,
        usage: Option<axiom_llm::TokenUsage>,
    },
    ProviderFailed {
        iteration: u32,
        provider: String,
        model: String,
        error: String,
    },
    ToolStarted {
        iteration: u32,
        tool_sequence: u32,
        request: ToolRequest,
    },
    ToolCompleted {
        iteration: u32,
        tool_sequence: u32,
        event: ToolExecutionEvent,
        observation: ChatMessage,
    },
    ReflectQueued {
        iteration: u32,
        instruction: ChatMessage,
    },
    CancellationObserved {
        iteration: u32,
    },
    CapReached {
        iteration: u32,
        cap: AgentCapKind,
    },
    Done {
        iteration: u32,
        content: String,
    },
    GiveUp {
        iteration: u32,
        reason: GiveUpReason,
        partial: String,
    },
    ProviderDegradedNoTools {
        iteration: u32,
        provider: String,
        model: String,
        error: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct TransitionCheckpoint {
    pub transition: AgentTransition,
    pub partial: String,
    pub history_delta: Vec<ChatMessage>,
    pub tool_events: Vec<ToolExecutionEvent>,
    pub policy_decisions: Vec<SideEffectDecision>,
    pub ledger: UsageLedger,
    pub context_tokens_estimate: u64,
    pub compacted_messages: usize,
    pub todo: TodoList,
    pub todo_updates: u32,
}

pub trait TransitionObserver {
    fn on_transition(&mut self, checkpoint: &TransitionCheckpoint) -> Result<()>;
}

pub trait StreamObserver {
    fn on_stream_update(&mut self, update: &ChatStreamUpdate);
    fn on_step_started(&mut self) {}
    fn on_step_finished(&mut self) {}
}

impl<F> StreamObserver for F
where
    F: FnMut(&ChatStreamUpdate),
{
    fn on_stream_update(&mut self, update: &ChatStreamUpdate) {
        self(update);
    }
}

impl<F> TransitionObserver for F
where
    F: FnMut(&TransitionCheckpoint) -> Result<()>,
{
    fn on_transition(&mut self, checkpoint: &TransitionCheckpoint) -> Result<()> {
        self(checkpoint)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct TurnCompletion {
    pub content: String,
    pub history_delta: Vec<ChatMessage>,
    pub tool_events: Vec<ToolExecutionEvent>,
    pub iterations: u32,
    pub ledger: UsageLedger,
    pub context_tokens_estimate: u64,
    pub compacted_messages: usize,
    pub todo: TodoList,
    pub todo_updates: u32,
    pub transitions: Vec<AgentTransition>,
    pub policy_decisions: Vec<SideEffectDecision>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TurnResult {
    Done(TurnCompletion),
    GiveUp {
        partial: String,
        reason: GiveUpReason,
        completion: TurnCompletion,
    },
}
