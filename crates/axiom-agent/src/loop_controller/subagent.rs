use axiom_engine::{
    builtin_installed_skill, AllowAllApprover, InstalledSkill, SkillExecutionError,
    SkillExecutionResult, ToolRequest,
};
use axiom_llm::ChatMessage;
use serde_json::Value;

use crate::{AgentCaps, UsageLedger};

use super::{AgentLoop, TurnResult};

/// Skill id of the delegation tool handled directly by the agent loop.
pub(super) const SUBAGENT_SKILL_ID: &str = "subagent.run";
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

impl<'a> AgentLoop<'a> {
    /// Runs an isolated, read-only sub-agent turn and returns its structured report.
    pub(super) async fn run_subagent_tool(
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
}
