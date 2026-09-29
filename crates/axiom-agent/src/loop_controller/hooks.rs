use std::time::Instant;

use axiom_engine::{
    builtin_installed_skill, execute_tool_with_policy, ExecutorRegistry,
    RecordingSideEffectAuditSink, SkillHooks, ToolRequest,
};

use crate::AgentCaps;

use super::{
    events::{HookExecution, HookPhase, HookStatus},
    AgentLoop, TurnProgress,
};

/// Manifest hooks a single turn may run before it stops trying more.
pub(super) const MAX_HOOKS_PER_TURN: u32 = 24;
/// Hooks fire one level deep: a hook never fires hooks of its own.
const MAX_HOOK_DEPTH: u32 = 1;
/// A single hook may not hold the turn open longer than this.
const HOOK_TIMEOUT_SECS: u64 = 30;

/// Per-turn allowance for manifest hooks.
///
/// Hooks are extra tool executions the model never asked for, so they are
/// capped separately from `max_tool_iterations` and share the turn's wall
/// clock. Consuming a slot is explicit so a skipped hook is recorded rather
/// than silently dropped.
pub(super) struct HookBudget {
    pub(super) remaining: u32,
    pub(super) started_at: Instant,
    pub(super) deadline_secs: u64,
}

impl HookBudget {
    pub(super) fn new(caps: &AgentCaps, started_at: Instant) -> Self {
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

impl<'a> AgentLoop<'a> {
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
    pub(super) async fn fire_hooks(
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
}
