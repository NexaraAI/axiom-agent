use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{anyhow, Result};
use axiom_engine::{
    builtin_installed_skill, execute_tool_with_policy, extract_tool_request, AllowAllApprover,
    ExecutorRegistry, ExternalToolSource, InstalledSkill, RecordingSideEffectAuditSink,
    SideEffectDecision, SideEffectPolicy, SkillApproval, SkillExecutionContext,
    SkillExecutionError, SkillExecutionResult, SkillHooks, ToolRequest,
};
use axiom_llm::{
    detect_repetition_period, ChatMessage, ChatRequest, ChatStreamUpdate, ChatToolDefinition,
    LlmProvider,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    compact_messages, parse_todo_update, AgentCaps, CancellationToken, TodoList, UsageLedger,
    UsagePricing,
};

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
    fn skipped(hook_id: &str, phase: HookPhase, status: HookStatus) -> Self {
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

/// Manifest hooks a single turn may run before it stops trying more.
const MAX_HOOKS_PER_TURN: u32 = 24;
/// Hooks fire one level deep: a hook never fires hooks of its own.
const MAX_HOOK_DEPTH: u32 = 1;
/// A single hook may not hold the turn open longer than this.
const HOOK_TIMEOUT_SECS: u64 = 30;

/// Skill id of the delegation tool handled directly by the agent loop.
const SUBAGENT_SKILL_ID: &str = "subagent.run";
/// Sub-agents may not spawn further sub-agents.
const MAX_SUBAGENT_DEPTH: u32 = 1;
/// Read-only tool surface available to an isolated sub-agent.
const SUBAGENT_READONLY_SKILLS: [&str; 9] = [
    "code.grep",
    "code.glob",
    "code.list",
    "file.read",
    "file.read_many",
    "project.scan",
    "git.status",
    "git.diff",
    "web.fetch",
];

fn subagent_system_prompt(role: &str) -> String {
    format!(
        "You are a focused Axiom sub-agent acting as: {role}.\n\
You run in an isolated context with READ-ONLY workspace tools; you cannot write files or run mutating commands.\n\
Investigate the assigned task using code.grep, code.glob, code.list, file.read, file.read_many, project.scan, git.status, and git.diff as needed.\n\
Finish with a concise, evidence-based report: key findings, the file paths and line numbers you relied on, and any uncertainty. Do not ask questions and do not stop at narration."
    )
}

/// Per-turn allowance for manifest hooks.
///
/// Hooks are extra tool executions the model never asked for, so they are
/// capped separately from `max_tool_iterations` and share the turn's wall
/// clock. Consuming a slot is explicit so a skipped hook is recorded rather
/// than silently dropped.
struct HookBudget {
    remaining: u32,
    started_at: Instant,
    deadline_secs: u64,
}

impl HookBudget {
    fn new(caps: &AgentCaps, started_at: Instant) -> Self {
        Self {
            remaining: MAX_HOOKS_PER_TURN,
            started_at,
            deadline_secs: caps.max_wall_seconds,
        }
    }

    fn wall_clock_exhausted(&self) -> bool {
        self.started_at.elapsed().as_secs() >= self.deadline_secs
    }

    /// Consumes one hook slot, reporting whether one was available.
    fn take(&mut self) -> bool {
        if self.remaining == 0 {
            false
        } else {
            self.remaining -= 1;
            true
        }
    }
}

/// The hook a phase declares, if any.
fn hook_for_phase(hooks: &SkillHooks, phase: HookPhase) -> Option<&str> {
    match phase {
        HookPhase::Pre => hooks.pre.as_deref(),
        HookPhase::Post => hooks.post.as_deref(),
        HookPhase::OnError => hooks.on_error.as_deref(),
    }
}

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

impl<'a> AgentLoop<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        provider: &'a dyn LlmProvider,
        model: impl Into<String>,
        caps: AgentCaps,
        system_messages: Vec<ChatMessage>,
        history: Vec<ChatMessage>,
        installed_skills: &'a [InstalledSkill],
        execution_context: SkillExecutionContext,
        approval: &'a mut dyn SkillApproval,
    ) -> Self {
        let side_effect_policy =
            SideEffectPolicy::backward_compatible(execution_context.auto_approve_medium_risk);
        let executor_schemas = ExecutorRegistry::with_builtin_executors()
            .descriptors()
            .into_iter()
            .map(|descriptor| (descriptor.id, descriptor.input_schema))
            .collect::<std::collections::BTreeMap<_, _>>();
        let mut tool_definitions: Vec<_> = installed_skills
            .iter()
            .filter(|skill| skill.record.is_executable())
            .filter(|skill| skill.manifest.skill_type == axiom_engine::SkillType::Tool)
            .filter_map(|skill| {
                executor_schemas
                    .get(&skill.manifest.id)
                    .cloned()
                    .map(|input_schema| ChatToolDefinition {
                        name: native_tool_name(&skill.manifest.id),
                        description: skill.manifest.description.clone(),
                        parameters: input_schema,
                    })
            })
            .collect();
        tool_definitions.sort_by(|a, b| a.name.cmp(&b.name));

        Self {
            provider,
            model: model.into(),
            caps,
            system_messages,
            history,
            installed_skills,
            execution_context,
            approval,
            allow_tools: true,
            todo: TodoList::default(),
            temperature: Some(0.7),
            max_response_tokens: None,
            tool_definitions,
            pricing: UsagePricing::default(),
            streaming: false,
            cancellation: CancellationToken::new(),
            transition_observer: None,
            stream_observer: None,
            side_effect_policy,
            provider_options: None,
            subagent_depth: 0,
            hook_depth: 0,
            external_tools: None,
        }
    }

    pub fn with_provider_options(
        mut self,
        options: Option<std::collections::BTreeMap<String, serde_json::Value>>,
    ) -> Self {
        self.provider_options = options;
        self
    }

    pub fn with_subagent_depth(mut self, depth: u32) -> Self {
        self.subagent_depth = depth;
        self
    }

    /// Sets the hook nesting depth. Only nested loops (a sub-agent, or a loop
    /// embedded in another) ever need this; at [`MAX_HOOK_DEPTH`] or above the
    /// loop records each hook as skipped instead of firing it.
    pub fn with_hook_depth(mut self, depth: u32) -> Self {
        self.hook_depth = depth;
        self
    }

    pub fn with_tools_enabled(mut self, enabled: bool) -> Self {
        self.allow_tools = enabled;
        self
    }

    pub fn with_todo_list(mut self, todo: TodoList) -> Self {
        self.todo = todo;
        self
    }

    pub fn with_generation_options(
        mut self,
        temperature: Option<f32>,
        max_response_tokens: Option<u32>,
    ) -> Self {
        self.temperature = temperature;
        self.max_response_tokens = max_response_tokens;
        self
    }

    pub fn with_pricing(mut self, pricing: UsagePricing) -> Self {
        self.pricing = pricing;
        self
    }

    pub fn with_streaming(mut self, enabled: bool) -> Self {
        self.streaming = enabled;
        self
    }

    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }

    pub fn with_transition_observer(mut self, observer: &'a mut dyn TransitionObserver) -> Self {
        self.transition_observer = Some(observer);
        self
    }

    pub fn with_side_effect_policy(mut self, policy: SideEffectPolicy) -> Self {
        self.side_effect_policy = policy;
        self
    }

    pub fn with_stream_observer(mut self, observer: &'a mut dyn StreamObserver) -> Self {
        self.stream_observer = Some(observer);
        self
    }

    /// Advertises tools provided by an external source (such as a connected MCP
    /// server) alongside the built-in ones, and executes them through the same
    /// side-effect policy and approval hooks.
    ///
    /// Definitions that cannot be advertised safely are skipped rather than
    /// failing the turn, and a definition whose name collides with an existing
    /// tool is ignored so built-ins always win.
    pub fn with_external_tools(mut self, source: &'a dyn ExternalToolSource) -> Self {
        for definition in source.definitions() {
            if definition.validate().is_err() {
                continue;
            }
            let name = native_tool_name(&definition.id);
            if self
                .tool_definitions
                .iter()
                .any(|existing| existing.name == name)
            {
                continue;
            }
            self.tool_definitions.push(ChatToolDefinition {
                name,
                description: definition.description.clone(),
                parameters: definition.input_schema.clone(),
            });
        }
        self.tool_definitions
            .sort_by(|left, right| left.name.cmp(&right.name));
        self.external_tools = Some(source);
        self
    }

    /// The manifest hooks declared by the skill behind `skill_id`.
    ///
    /// Installed manifests win over the built-in ones, so an installed skill can
    /// attach hooks to a built-in executor's id. A skill with no manifest — an
    /// MCP tool, or an unknown id — simply has no hooks.
    fn hooks_for_skill(&self, skill_id: &str) -> SkillHooks {
        if let Some(skill) = self
            .installed_skills
            .iter()
            .find(|skill| skill.manifest.id == skill_id)
        {
            return skill.manifest.hooks.clone();
        }
        builtin_installed_skill(skill_id)
            .map(|skill| skill.manifest.hooks)
            .unwrap_or_default()
    }

    /// True when the hook id resolves to something the loop can actually run.
    fn hook_is_available(&self, hook_id: &str) -> bool {
        let installed = self.installed_skills.iter().any(|skill| {
            skill.manifest.id == hook_id
                && skill.record.is_executable()
                && skill.manifest.skill_type == axiom_engine::SkillType::Tool
        });
        installed
            || ExecutorRegistry::with_builtin_executors()
                .get(hook_id)
                .is_some()
    }

    /// Fires the hook `phase` declares for the skill behind `request`.
    ///
    /// Hooks are ordinary permission-gated tools: they run through
    /// [`execute_tool_with_policy`] with the live approval hook, side-effect
    /// policy, and audit sink, and they receive the triggering tool's arguments
    /// (so a post-write hook sees the path that changed).
    ///
    /// Guardrails, all reported rather than silent:
    /// * re-entrancy — a hook never fires hooks, and a skill cannot hook itself;
    /// * budget — hooks share the turn's wall clock and a per-turn allowance;
    /// * availability — a hook id that cannot execute is recorded as skipped;
    /// * failure — a failing hook is recorded and never fails the turn.
    async fn fire_hooks(
        &mut self,
        phase: HookPhase,
        request: &ToolRequest,
        budget: &mut HookBudget,
        progress: &mut TurnProgress,
    ) -> Vec<HookExecution> {
        let hooks = self.hooks_for_skill(&request.skill_id);
        let Some(hook_id) = hook_for_phase(&hooks, phase).map(ToString::to_string) else {
            return Vec::new();
        };
        if hook_id == request.skill_id || self.hook_depth >= MAX_HOOK_DEPTH {
            return vec![HookExecution::skipped(
                &hook_id,
                phase,
                HookStatus::SkippedReentrancy,
            )];
        }
        if self.cancellation.is_cancelled() {
            return vec![HookExecution::skipped(
                &hook_id,
                phase,
                HookStatus::SkippedCancelled,
            )];
        }
        if !self.hook_is_available(&hook_id) {
            return vec![HookExecution::skipped(
                &hook_id,
                phase,
                HookStatus::Unavailable,
            )];
        }
        if budget.wall_clock_exhausted() || !budget.take() {
            return vec![HookExecution::skipped(
                &hook_id,
                phase,
                HookStatus::SkippedBudget,
            )];
        }

        let hook_request = ToolRequest {
            skill_id: hook_id.clone(),
            arguments: request.arguments.clone(),
        };
        let started_at = Instant::now();
        let mut audit = RecordingSideEffectAuditSink::default();
        let timeout = std::time::Duration::from_secs(HOOK_TIMEOUT_SECS);
        self.hook_depth += 1;
        let hook_future = execute_tool_with_policy(
            &hook_request,
            self.installed_skills,
            &self.execution_context,
            &mut *self.approval,
            &self.side_effect_policy,
            &mut audit,
            None,
        );
        let mut execution = tokio::select! {
            result = tokio::time::timeout(timeout, hook_future) => match result {
                Ok(Ok(result)) => HookExecution {
                    hook_id,
                    phase,
                    status: HookStatus::Succeeded,
                    latency_ms: 0,
                    output: Some(result.output),
                    error: None,
                },
                Ok(Err(error)) => HookExecution {
                    hook_id,
                    phase,
                    status: HookStatus::Failed,
                    latency_ms: 0,
                    output: None,
                    error: Some(error.to_string()),
                },
                Err(_) => HookExecution {
                    hook_id,
                    phase,
                    status: HookStatus::Failed,
                    latency_ms: 0,
                    output: None,
                    error: Some(format!("hook timed out after {HOOK_TIMEOUT_SECS}s")),
                },
            },
            _ = self.cancellation.cancelled() => HookExecution::skipped(
                &hook_id,
                phase,
                HookStatus::SkippedCancelled,
            ),
        };
        self.hook_depth -= 1;
        execution.latency_ms = started_at.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        progress.policy_decisions.extend(audit.into_decisions());
        vec![execution]
    }

    /// Runs an isolated, read-only sub-agent turn and returns its structured report.
    async fn run_subagent_tool(
        &self,
        request: &ToolRequest,
    ) -> (
        Result<SkillExecutionResult, SkillExecutionError>,
        UsageLedger,
    ) {
        if self.subagent_depth >= MAX_SUBAGENT_DEPTH {
            return (
                Err(SkillExecutionError::ExecutionFailed {
                    skill_id: SUBAGENT_SKILL_ID.to_string(),
                    message: "nested sub-agents are not supported".to_string(),
                }),
                UsageLedger::default(),
            );
        }
        let role = match request.arguments.get("role").and_then(Value::as_str) {
            Some(role) if !role.trim().is_empty() => role.trim().to_string(),
            _ => {
                return (
                    Err(SkillExecutionError::MissingArgument {
                        skill_id: SUBAGENT_SKILL_ID.to_string(),
                        argument: "role",
                    }),
                    UsageLedger::default(),
                )
            }
        };
        let task = match request.arguments.get("task").and_then(Value::as_str) {
            Some(task) if !task.trim().is_empty() => task.trim().to_string(),
            _ => {
                return (
                    Err(SkillExecutionError::MissingArgument {
                        skill_id: SUBAGENT_SKILL_ID.to_string(),
                        argument: "task",
                    }),
                    UsageLedger::default(),
                )
            }
        };
        let context = request
            .arguments
            .get("context")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        let mut skills: Vec<InstalledSkill> = self
            .installed_skills
            .iter()
            .filter(|skill| SUBAGENT_READONLY_SKILLS.contains(&skill.manifest.id.as_str()))
            .cloned()
            .collect();
        for skill_id in SUBAGENT_READONLY_SKILLS {
            if !skills.iter().any(|skill| skill.manifest.id == skill_id) {
                if let Some(skill) = builtin_installed_skill(skill_id) {
                    skills.push(skill);
                }
            }
        }

        let caps = AgentCaps {
            max_iterations: 6,
            max_tool_iterations: 12,
            max_tokens: self.caps.max_tokens.min(60_000),
            max_cost_usd: (self.caps.max_cost_usd * 0.25).max(0.05),
            max_wall_seconds: self.caps.max_wall_seconds.min(600),
            max_consecutive_tool_errors: 2,
        };
        let mut approver = AllowAllApprover;
        let mut child = AgentLoop::new(
            self.provider,
            self.model.clone(),
            caps,
            vec![ChatMessage {
                role: "system".to_string(),
                content: subagent_system_prompt(&role),
            }],
            Vec::new(),
            &skills,
            self.execution_context.clone(),
            &mut approver,
        )
        .with_tools_enabled(true)
        .with_streaming(false)
        .with_cancellation(self.cancellation.clone())
        .with_subagent_depth(self.subagent_depth + 1)
        .with_generation_options(Some(0.4), None);

        let user_message = if context.trim().is_empty() {
            task.clone()
        } else {
            format!("{task}\n\nContext:\n{context}")
        };
        // Boxed to break the run_turn -> run_subagent_tool -> run_turn future cycle.
        let outcome = Box::pin(child.run_turn(ChatMessage {
            role: "user".to_string(),
            content: user_message,
        }))
        .await;

        let (output, ledger) = match outcome {
            Ok(TurnResult::Done(completion)) => (
                serde_json::json!({
                    "role": role,
                    "task": task,
                    "status": "completed",
                    "summary": completion.content,
                    "iterations": completion.iterations,
                    "tool_calls": completion.tool_events.len(),
                    "total_tokens": completion.ledger.total_tokens,
                }),
                completion.ledger,
            ),
            Ok(TurnResult::GiveUp {
                reason,
                partial,
                completion,
            }) => (
                serde_json::json!({
                    "role": role,
                    "task": task,
                    "status": "incomplete",
                    "summary": partial,
                    "reason": format!("{reason:?}"),
                    "iterations": completion.iterations,
                    "tool_calls": completion.tool_events.len(),
                    "total_tokens": completion.ledger.total_tokens,
                }),
                completion.ledger,
            ),
            Err(error) => (
                serde_json::json!({
                    "role": role,
                    "task": task,
                    "status": "failed",
                    "summary": format!("sub-agent failed: {error}"),
                    "iterations": 0,
                    "tool_calls": 0,
                    "total_tokens": 0,
                }),
                UsageLedger::default(),
            ),
        };

        (
            Ok(SkillExecutionResult {
                skill_id: SUBAGENT_SKILL_ID.to_string(),
                output,
            }),
            ledger,
        )
    }

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

            let request = ChatRequest {
                model: self.model.clone(),
                messages: messages.clone(),
                temperature: self.temperature,
                max_tokens: self.max_response_tokens,
                stream: self.streaming,
                metadata: None,
                provider_options: self.provider_options.clone(),
                tools: if self.allow_tools {
                    self.tool_definitions.clone()
                } else {
                    Vec::new()
                },
                tool_choice: self.allow_tools.then(|| "auto".to_string()),
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
            let assistant_message = ChatMessage {
                role: "assistant".to_string(),
                content: assistant_content.clone(),
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

            let mut tool_requests = if self.allow_tools && !response.tool_calls.is_empty() {
                response
                    .tool_calls
                    .iter()
                    .map(|tool_call| {
                        let skill_id =
                            self.skill_id_for_native_tool(&tool_call.name)
                                .ok_or_else(|| {
                                    anyhow!(
                                        "provider requested unknown Axiom function: {}",
                                        tool_call.name
                                    )
                                })?;
                        Ok::<ToolRequest, anyhow::Error>(ToolRequest {
                            skill_id,
                            arguments: tool_call.arguments.clone(),
                        })
                    })
                    .collect::<Result<Vec<_>>>()?
            } else {
                Vec::new()
            };
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
                    let status = match tool_result {
                        Ok(result) => {
                            consecutive_tool_errors = 0;
                            ToolExecutionStatus::Succeeded(result)
                        }
                        Err(error) => {
                            let err_str = error.to_string();
                            let is_non_fatal = err_str.contains("approval denied")
                                || err_str.contains("timeout")
                                || err_str.contains("timed out")
                                || err_str.contains("cancelled");
                            if !is_non_fatal {
                                consecutive_tool_errors += 1;
                            }
                            ToolExecutionStatus::Failed(err_str)
                        }
                    };
                    let mut hooks = pre_hooks.get(index).cloned().unwrap_or_default();
                    let outcome_phase = match &status {
                        ToolExecutionStatus::Succeeded(_) => HookPhase::Post,
                        ToolExecutionStatus::Failed(_) => HookPhase::OnError,
                    };
                    hooks.extend(
                        self.fire_hooks(outcome_phase, &request, &mut hook_budget, &mut progress)
                            .await,
                    );
                    progress
                        .policy_decisions
                        .extend(policy_audit.into_decisions());
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
                        &mut progress,
                        AgentTransitionKind::ToolCompleted {
                            iteration,
                            tool_sequence,
                            event,
                            observation: observation_message,
                        },
                    )?;
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
                    let status = match tool_result {
                        Some(Ok(result)) => {
                            consecutive_tool_errors = 0;
                            ToolExecutionStatus::Succeeded(result)
                        }
                        Some(Err(error)) => {
                            let err_str = error.to_string();
                            let is_non_fatal = err_str.contains("approval denied")
                                || err_str.contains("timeout")
                                || err_str.contains("timed out")
                                || err_str.contains("cancelled");
                            if !is_non_fatal {
                                consecutive_tool_errors += 1;
                            }
                            ToolExecutionStatus::Failed(err_str)
                        }
                        None => ToolExecutionStatus::Failed(
                            "Tool execution cancelled by user".to_string(),
                        ),
                    };
                    let mut hooks = pre_hooks;
                    let outcome_phase = match &status {
                        ToolExecutionStatus::Succeeded(_) => HookPhase::Post,
                        ToolExecutionStatus::Failed(_) => HookPhase::OnError,
                    };
                    hooks.extend(
                        self.fire_hooks(outcome_phase, &request, &mut hook_budget, &mut progress)
                            .await,
                    );
                    progress
                        .policy_decisions
                        .extend(policy_audit.into_decisions());
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
                        &mut progress,
                        AgentTransitionKind::ToolCompleted {
                            iteration,
                            tool_sequence,
                            event,
                            observation: observation_message,
                        },
                    )?;

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

    fn cost_limit_reached(&self, ledger: &UsageLedger) -> bool {
        let max_cost = self.caps.max_cost_usd;
        if !max_cost.is_finite() || max_cost < 0.0 {
            return false;
        }
        let Some(cost_microusd) = ledger.estimated_cost_microusd(self.pricing) else {
            return false;
        };
        let cap_microusd = (max_cost * 1_000_000.0).round().clamp(0.0, u64::MAX as f64) as u64;
        cost_microusd >= cap_microusd
    }

    fn skill_id_for_native_tool(&self, name: &str) -> Option<String> {
        let cleaned = name.trim();
        let unprefix = cleaned
            .strip_prefix("axiom_")
            .or_else(|| cleaned.strip_prefix("axiom."))
            .unwrap_or(cleaned);

        if let Some(skill_id) = self
            .installed_skills
            .iter()
            .map(|skill| skill.manifest.id.as_str())
            .find(|skill_id| {
                *skill_id == cleaned
                    || native_tool_name(skill_id) == cleaned
                    || *skill_id == unprefix
                    || skill_id.replace('.', "_") == cleaned
                    || skill_id.replace('.', "_") == unprefix
            })
        {
            return Some(skill_id.to_string());
        }

        if let Some(definition) = self
            .external_tools
            .into_iter()
            .flat_map(ExternalToolSource::definitions)
            .find(|definition| {
                definition.id == cleaned
                    || native_tool_name(&definition.id) == cleaned
                    || definition.id == unprefix
                    || definition.id.replace('.', "_") == cleaned
                    || definition.id.replace('.', "_") == unprefix
            })
        {
            return Some(definition.id);
        }

        const CORE_BUILTIN_IDS: &[&str] = &[
            "file.read",
            "file.write",
            "project.scan",
            "web.fetch",
            "shell.powershell.safe",
            "shell.bash.safe",
            "shell.zsh.safe",
            "shell.run",
            "python.run",
            "git.status",
            "git.diff",
            "skill.create",
            "question.ask",
        ];
        CORE_BUILTIN_IDS
            .iter()
            .copied()
            .find(|builtin| {
                *builtin == cleaned
                    || *builtin == unprefix
                    || native_tool_name(builtin) == cleaned
                    || builtin.replace('.', "_") == cleaned
                    || builtin.replace('.', "_") == unprefix
            })
            .map(ToString::to_string)
    }
}

fn native_tool_name(skill_id: &str) -> String {
    format!("axiom_{}", skill_id.replace('.', "_"))
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

struct SimpleJoinAll<F: std::future::Future> {
    futures: Vec<Option<F>>,
    results: Vec<Option<F::Output>>,
}

impl<F: std::future::Future> SimpleJoinAll<F> {
    fn new(futures: Vec<F>) -> Self {
        let len = futures.len();
        Self {
            futures: futures.into_iter().map(Some).collect(),
            results: (0..len).map(|_| None).collect(),
        }
    }
}

impl<F: std::future::Future + Unpin> std::future::Future for SimpleJoinAll<F>
where
    F::Output: Unpin,
{
    type Output = Vec<F::Output>;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let this = self.get_mut();
        let mut all_done = true;
        let len = this.futures.len();
        for i in 0..len {
            if let Some(fut) = this.futures[i].as_mut() {
                match std::pin::Pin::new(fut).poll(cx) {
                    std::task::Poll::Ready(output) => {
                        this.results[i] = Some(output);
                        this.futures[i] = None;
                    }
                    std::task::Poll::Pending => {
                        all_done = false;
                    }
                }
            }
        }
        if all_done {
            let res = this
                .results
                .iter_mut()
                .map(|opt| opt.take().expect("future completed"))
                .collect();
            std::task::Poll::Ready(res)
        } else {
            std::task::Poll::Pending
        }
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

/// The model-facing observation for a finished tool call: the tool result
/// itself, plus whatever manifest hooks reported around it.
fn tool_observation(event: &ToolExecutionEvent) -> String {
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
            HookStatus::Succeeded => hook
                .output
                .as_ref()
                .map(|output| truncate_for_observation(&output.to_string(), 400)),
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
            format!(
                "Tool `{}` succeeded:\n```json\n{}\n```",
                result.skill_id, result.output
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

fn sanitize_interrupted_content(content: &str) -> String {
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

#[cfg(test)]
mod tests {
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
    use serde_json::json;

    use super::*;

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
        assert_eq!(completion.tool_events.len(), 2);
        assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
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

    #[tokio::test]
    async fn manifest_hooks_fire_around_a_successful_tool_call() {
        let provider = ToolCallProvider::new(vec![(
            "axiom_project_scan".to_string(),
            json!({"path": "."}),
        )]);
        // Hooks receive the triggering tool's arguments, so `code.list` is
        // pointed at the same directory `project.scan` was asked to scan.
        let installed = [installed_tool_with_hooks(
            "project.scan",
            Some("code.list"),
            Some("code.list"),
            None,
        )];
        let mut approval = DenyAllApprover;
        let mut agent = AgentLoop::new(
            &provider,
            "test-model",
            AgentCaps::default(),
            Vec::new(),
            Vec::new(),
            &installed,
            context(),
            &mut approval,
        );

        let completion = done_turn(
            agent
                .run_turn(user_message("read the manifest"))
                .await
                .expect("turn succeeds"),
        );

        let event = completion.tool_events.first().expect("one tool event");
        assert!(matches!(event.status, ToolExecutionStatus::Succeeded(_)));
        let phases = event
            .hooks
            .iter()
            .map(|hook| hook.phase)
            .collect::<Vec<_>>();
        assert_eq!(phases, vec![HookPhase::Pre, HookPhase::Post]);
        for hook in &event.hooks {
            assert_eq!(hook.hook_id, "code.list");
            assert_eq!(hook.status, HookStatus::Succeeded, "{hook:?}");
            assert!(hook.output.is_some(), "{hook:?}");
        }

        let observation = first_tool_observation(&completion);
        assert!(observation.contains("Manifest hooks:"));
        assert!(observation.contains("- code.list [pre]"));
        assert!(observation.contains("- code.list [post]"));
    }

    #[tokio::test]
    async fn on_error_hooks_fire_when_the_tool_fails_and_post_does_not() {
        let provider = ToolCallProvider::new(vec![(
            "axiom_project_scan".to_string(),
            json!({"path": "no-such-directory-axiom-hooks"}),
        )]);
        let installed = [installed_tool_with_hooks(
            "project.scan",
            None,
            Some("code.list"),
            Some("code.list"),
        )];
        let mut approval = DenyAllApprover;
        let mut agent = AgentLoop::new(
            &provider,
            "test-model",
            AgentCaps::default(),
            Vec::new(),
            Vec::new(),
            &installed,
            context(),
            &mut approval,
        );

        let completion = done_turn(
            agent
                .run_turn(user_message("read a missing file"))
                .await
                .expect("turn succeeds"),
        );

        let event = completion.tool_events.first().expect("one tool event");
        assert!(matches!(event.status, ToolExecutionStatus::Failed(_)));
        assert_eq!(event.hooks.len(), 1, "post must not run for a failed tool");
        assert_eq!(event.hooks[0].phase, HookPhase::OnError);
        assert!(
            matches!(
                event.hooks[0].status,
                HookStatus::Succeeded | HookStatus::Failed
            ),
            "the on_error hook actually ran"
        );
        assert!(first_tool_observation(&completion).contains("[on_error]"));
    }

    #[tokio::test]
    async fn manifest_hooks_fire_for_parallel_read_only_batches() {
        let provider = ToolCallProvider::new(vec![
            ("axiom_project_scan".to_string(), json!({"path": "."})),
            ("axiom_project_scan".to_string(), json!({"path": "src"})),
        ]);
        let installed = [installed_tool_with_hooks(
            "project.scan",
            Some("code.list"),
            Some("code.list"),
            None,
        )];
        let mut approval = DenyAllApprover;
        let mut agent = AgentLoop::new(
            &provider,
            "test-model",
            AgentCaps::default(),
            Vec::new(),
            Vec::new(),
            &installed,
            context(),
            &mut approval,
        );

        let completion = done_turn(
            agent
                .run_turn(user_message("read both files"))
                .await
                .expect("turn succeeds"),
        );

        assert_eq!(completion.tool_events.len(), 2);
        for event in &completion.tool_events {
            let phases = event
                .hooks
                .iter()
                .map(|hook| hook.phase)
                .collect::<Vec<_>>();
            assert_eq!(phases, vec![HookPhase::Pre, HookPhase::Post]);
            for hook in &event.hooks {
                assert_eq!(hook.status, HookStatus::Succeeded, "{hook:?}");
            }
        }
    }

    #[tokio::test]
    async fn a_hook_that_cannot_execute_is_recorded_as_unavailable() {
        let provider = ToolCallProvider::new(vec![(
            "axiom_project_scan".to_string(),
            json!({"path": "."}),
        )]);
        let installed = [installed_tool_with_hooks(
            "project.scan",
            None,
            Some("demo.lint"),
            None,
        )];
        let mut approval = DenyAllApprover;
        let mut agent = AgentLoop::new(
            &provider,
            "test-model",
            AgentCaps::default(),
            Vec::new(),
            Vec::new(),
            &installed,
            context(),
            &mut approval,
        );

        let completion = done_turn(
            agent
                .run_turn(user_message("read the manifest"))
                .await
                .expect("turn succeeds"),
        );

        let event = completion.tool_events.first().expect("one tool event");
        assert!(matches!(event.status, ToolExecutionStatus::Succeeded(_)));
        assert_eq!(event.hooks.len(), 1);
        assert_eq!(event.hooks[0].hook_id, "demo.lint");
        assert_eq!(event.hooks[0].status, HookStatus::Unavailable);
        assert!(first_tool_observation(&completion).contains("not executable"));
    }

    #[tokio::test]
    async fn hooks_run_through_the_side_effect_policy_and_approval_gate() {
        let provider = ToolCallProvider::new(vec![(
            "axiom_project_scan".to_string(),
            json!({"path": "."}),
        )]);
        let installed = [installed_tool_with_hooks(
            "project.scan",
            Some("code.list"),
            None,
            None,
        )];
        let mut approval = DenyAllApprover;
        let mut agent = AgentLoop::new(
            &provider,
            "test-model",
            AgentCaps::default(),
            Vec::new(),
            Vec::new(),
            &installed,
            context(),
            &mut approval,
        )
        .with_side_effect_policy(SideEffectPolicy {
            filesystem_read: PolicyAction::Ask,
            ..SideEffectPolicy::deny_all()
        });

        let completion = done_turn(
            agent
                .run_turn(user_message("read the manifest"))
                .await
                .expect("turn succeeds"),
        );

        let event = completion.tool_events.first().expect("one tool event");
        assert_eq!(event.hooks.len(), 1);
        assert_eq!(event.hooks[0].status, HookStatus::Failed);
        assert!(event.hooks[0]
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("approval denied"));
        assert!(matches!(event.status, ToolExecutionStatus::Failed(_)));
        assert!(
            completion
                .policy_decisions
                .iter()
                .any(|decision| decision.evaluation.request.skill_id == "code.list"),
            "the hook's own policy decision is recorded"
        );
    }

    #[tokio::test]
    async fn hooks_skip_on_reentrancy_and_exhausted_budget_instead_of_running() {
        let provider = MockProvider::new("mock");
        let installed = [installed_tool_with_hooks(
            "project.scan",
            Some("code.list"),
            Some("code.list"),
            None,
        )];
        let mut approval = DenyAllApprover;
        let mut agent = AgentLoop::new(
            &provider,
            "test-model",
            AgentCaps::default(),
            Vec::new(),
            Vec::new(),
            &installed,
            context(),
            &mut approval,
        );
        let request = ToolRequest {
            skill_id: "project.scan".to_string(),
            arguments: json!({"path": "."}),
        };
        let mut progress = TurnProgress::default();

        let mut exhausted = HookBudget {
            remaining: 0,
            started_at: Instant::now(),
            deadline_secs: 3600,
        };
        let hooks = agent
            .fire_hooks(HookPhase::Pre, &request, &mut exhausted, &mut progress)
            .await;
        assert_eq!(hooks[0].status, HookStatus::SkippedBudget);
        assert!(hooks[0].output.is_none());

        let mut late = HookBudget {
            remaining: 5,
            started_at: Instant::now() - std::time::Duration::from_secs(60),
            deadline_secs: 1,
        };
        let hooks = agent
            .fire_hooks(HookPhase::Pre, &request, &mut late, &mut progress)
            .await;
        assert_eq!(hooks[0].status, HookStatus::SkippedBudget);
        assert_eq!(late.remaining, 5, "a wall-clock skip spends no allowance");

        let mut nested = AgentLoop::new(
            &provider,
            "test-model",
            AgentCaps::default(),
            Vec::new(),
            Vec::new(),
            &installed,
            context(),
            &mut approval,
        )
        .with_hook_depth(1);
        let mut budget = HookBudget::new(&AgentCaps::default(), Instant::now());
        let hooks = nested
            .fire_hooks(HookPhase::Pre, &request, &mut budget, &mut progress)
            .await;
        assert_eq!(hooks[0].status, HookStatus::SkippedReentrancy);
        assert_eq!(
            budget.remaining, MAX_HOOKS_PER_TURN,
            "a re-entrancy skip spends no allowance"
        );
    }

    #[tokio::test]
    async fn a_skill_cannot_hook_itself() {
        let provider = MockProvider::new("mock");
        let installed = [installed_tool_with_hooks(
            "project.scan",
            Some("project.scan"),
            Some("project.scan"),
            Some("project.scan"),
        )];
        let mut approval = DenyAllApprover;
        let mut agent = AgentLoop::new(
            &provider,
            "test-model",
            AgentCaps::default(),
            Vec::new(),
            Vec::new(),
            &installed,
            context(),
            &mut approval,
        );
        let request = ToolRequest {
            skill_id: "project.scan".to_string(),
            arguments: json!({"path": "."}),
        };
        let mut progress = TurnProgress::default();
        let mut budget = HookBudget::new(&AgentCaps::default(), Instant::now());

        for phase in [HookPhase::Pre, HookPhase::Post, HookPhase::OnError] {
            let hooks = agent
                .fire_hooks(phase, &request, &mut budget, &mut progress)
                .await;
            assert_eq!(hooks.len(), 1);
            assert_eq!(
                hooks[0].status,
                HookStatus::SkippedReentrancy,
                "{phase:?} must refuse to re-enter the loop"
            );
        }
        assert_eq!(budget.remaining, MAX_HOOKS_PER_TURN);
    }

    #[test]
    fn tool_observation_formats_question_ask_as_trusted_user_instruction() {
        let custom_event = ToolExecutionEvent {
            request: ToolRequest {
                skill_id: "question.ask".to_string(),
                arguments: json!({"question": "Where to publish?"}),
            },
            latency_ms: 100,
            status: ToolExecutionStatus::Succeeded(SkillExecutionResult {
                skill_id: "question.ask".to_string(),
                output: json!({
                    "selected": "Modrinth and CurseForge",
                    "is_custom": true,
                    "index": 3
                }),
            }),
            hooks: Vec::new(),
        };

        let obs = tool_observation(&custom_event);
        assert!(obs.contains(
            "The user responded to your clarification question (custom write-in reply):"
        ));
        assert!(obs.contains("Modrinth and CurseForge"));
        assert!(obs.contains("IMPORTANT: Prioritize this user answer above all else"));
    }

    #[test]
    fn tool_observation_formats_timeout_as_autonomous_recovery_directive() {
        let timeout_event = ToolExecutionEvent {
            request: ToolRequest {
                skill_id: "shell.powershell.safe".to_string(),
                arguments: json!({"command": "scp -r user@remote:/data ."}),
            },
            latency_ms: 30000,
            status: ToolExecutionStatus::Succeeded(SkillExecutionResult {
                skill_id: "shell.powershell.safe".to_string(),
                output: json!({
                    "exit_code": 124,
                    "stdout": "Transferred 10 files...",
                    "stderr": "Command timed out after 30s."
                }),
            }),
            hooks: Vec::new(),
        };

        let obs = tool_observation(&timeout_event);
        assert!(obs.contains("timed out after execution limit (exit code 124)"));
        assert!(obs.contains("AUTONOMOUS RECOVERY DIRECTIVE: Do not give up or abandon the task!"));
        assert!(obs.contains("Transferred 10 files..."));
    }

    #[test]
    fn tool_observation_formats_declined_approval_as_recovery_directive() {
        let denied_event = ToolExecutionEvent {
            request: ToolRequest {
                skill_id: "shell.powershell.safe".to_string(),
                arguments: json!({"command": "rm -rf /"}),
            },
            latency_ms: 500,
            status: ToolExecutionStatus::Failed("approval denied by user policy".to_string()),
            hooks: Vec::new(),
        };

        let obs = tool_observation(&denied_event);
        assert!(obs.contains("was declined by user approval"));
        assert!(obs.contains("AUTONOMOUS RECOVERY DIRECTIVE: Do not give up. Select an alternative non-destructive approach"));
    }

    #[test]
    fn sanitize_interrupted_content_truncates_repetitive_and_huge_text() {
        let pattern = "Both servers are launching. Let me verify they're actually up by checking the ports.\n";
        let repetitive_text = format!("{pattern}{pattern}{pattern}");
        let sanitized = sanitize_interrupted_content(&repetitive_text);
        assert_eq!(sanitized, pattern.trim());

        let huge = "A".repeat(3000);
        let sanitized_huge = sanitize_interrupted_content(&huge);
        assert!(sanitized_huge.len() < 1600);
        assert!(sanitized_huge.ends_with("[Output truncated on interruption]"));
    }
}
