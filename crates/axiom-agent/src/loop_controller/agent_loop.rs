use axiom_engine::{
    ExecutorRegistry, ExternalToolSource, InstalledSkill, SideEffectPolicy, SkillApproval,
    SkillExecutionContext,
};
use axiom_llm::{ChatMessage, ChatToolDefinition, LlmProvider};

use crate::{AgentCaps, CancellationToken, TodoList, UsagePricing};

use super::{native_tool_name, AgentLoop, StreamObserver, TransitionObserver};

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
    /// embedded in another) ever need this; at `MAX_HOOK_DEPTH` or above the
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
}
