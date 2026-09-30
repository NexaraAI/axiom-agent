mod agent_loop;
mod events;
mod hooks;
mod join_all;
mod observation;
mod subagent;
mod tool_dispatch;
mod turn;

#[cfg(test)]
mod observation_tests;
#[cfg(test)]
mod tests;

use axiom_engine::{
    ExternalToolSource, InstalledSkill, SideEffectDecision, SideEffectPolicy, SkillApproval,
    SkillExecutionContext,
};
use axiom_llm::{ChatMessage, ChatToolDefinition, LlmProvider};

use crate::{AgentCaps, CancellationToken, TodoList, UsageLedger, UsagePricing};

pub use events::{
    AgentCapKind, AgentTransition, AgentTransitionKind, GiveUpReason, StreamObserver,
    ToolExecutionEvent, ToolExecutionStatus, TransitionCheckpoint, TransitionObserver,
    TurnCompletion, TurnResult,
};

pub struct AgentLoop<'a> {
    provider: &'a dyn LlmProvider,
    model: String,
    caps: AgentCaps,
    system_messages: Vec<ChatMessage>,
    history: Vec<ChatMessage>,
    installed_skills: &'a [InstalledSkill],
    execution_context: SkillExecutionContext,
    approval: &'a mut dyn SkillApproval,
    allow_tools: bool,
    todo: TodoList,
    temperature: Option<f32>,
    max_response_tokens: Option<u32>,
    tool_definitions: Vec<ChatToolDefinition>,
    pricing: UsagePricing,
    streaming: bool,
    cancellation: CancellationToken,
    transition_observer: Option<&'a mut dyn TransitionObserver>,
    stream_observer: Option<&'a mut dyn StreamObserver>,
    side_effect_policy: SideEffectPolicy,
    provider_options: Option<std::collections::BTreeMap<String, serde_json::Value>>,
    subagent_depth: u32,
    /// Nesting depth while running a manifest hook. Non-zero means hooks must
    /// not fire again, which is what makes hook recursion impossible.
    hook_depth: u32,
    /// Extra tools owned by another component (for example a connected MCP
    /// server). Calls are routed through the same policy and approval hooks.
    external_tools: Option<&'a dyn ExternalToolSource>,
}

#[derive(Debug, Default)]
struct TurnProgress {
    partial: String,
    history_delta: Vec<ChatMessage>,
    tool_events: Vec<ToolExecutionEvent>,
    ledger: UsageLedger,
    context_tokens_estimate: u64,
    compacted_messages: usize,
    todo_updates: u32,
    transitions: Vec<AgentTransition>,
    policy_decisions: Vec<SideEffectDecision>,
}

impl TurnProgress {
    fn complete(self, content: String, iterations: u32, todo: TodoList) -> TurnCompletion {
        TurnCompletion {
            content,
            history_delta: self.history_delta,
            tool_events: self.tool_events,
            iterations,
            ledger: self.ledger,
            context_tokens_estimate: self.context_tokens_estimate,
            compacted_messages: self.compacted_messages,
            todo,
            todo_updates: self.todo_updates,
            transitions: self.transitions,
            policy_decisions: self.policy_decisions,
        }
    }
}

fn native_tool_name(skill_id: &str) -> String {
    format!("axiom_{}", skill_id.replace('.', "_"))
}
