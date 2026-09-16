use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    Permission, SideEffectAuditSink, SideEffectClass, SideEffectPolicy, SkillApproval,
    SkillExecutionContext, SkillExecutionError, SkillExecutionResult, ToolRequest,
};

/// A tool offered by a source that lives outside the built-in executor
/// registry, such as a connected MCP server.
///
/// Unlike [`crate::ExecutorDescriptor`], this carries a natural-language
/// description because external tools are advertised to the model by the text
/// their provider published, not by Axiom's own metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalToolDefinition {
    /// Axiom skill id, e.g. `mcp.github.search_issues`.
    pub id: String,
    /// Short identifier of the owning source, e.g. the MCP server name.
    pub source: String,
    pub description: String,
    pub input_schema: Value,
    pub permissions: Vec<Permission>,
    pub side_effects: Vec<SideEffectClass>,
}

impl ExternalToolDefinition {
    pub fn new(
        id: impl Into<String>,
        source: impl Into<String>,
        description: impl Into<String>,
        input_schema: Value,
        permissions: Vec<Permission>,
        side_effects: Vec<SideEffectClass>,
    ) -> Self {
        Self {
            id: id.into(),
            source: source.into(),
            description: description.into(),
            input_schema,
            permissions,
            side_effects,
        }
    }

    /// Rejects definitions that cannot be advertised safely to a provider.
    pub fn validate(&self) -> Result<(), String> {
        if self.id.trim().is_empty() {
            return Err("external tool id cannot be empty".to_string());
        }
        if self.source.trim().is_empty() {
            return Err(format!("external tool `{}` has no source", self.id));
        }
        if self.input_schema.get("type").is_none() {
            return Err(format!(
                "external tool `{}` needs a JSON Schema object for its input",
                self.id
            ));
        }
        if self.description.trim().is_empty() {
            return Err(format!(
                "external tool `{}` needs a description so the model can select it",
                self.id
            ));
        }
        Ok(())
    }
}

/// A source of extra tools that Axiom can call next to its built-ins.
///
/// Implementations own their gating: [`Self::call`] receives the active
/// [`SideEffectPolicy`], the approval hook, and the audit sink so external
/// tools are subject to exactly the same rules as built-in executors.
#[async_trait(?Send)]
pub trait ExternalToolSource: Send + Sync {
    /// Definitions advertised to the model and used to resolve tool calls.
    fn definitions(&self) -> Vec<ExternalToolDefinition>;

    async fn call(
        &self,
        request: &ToolRequest,
        context: &SkillExecutionContext,
        approval: &mut dyn SkillApproval,
        policy: &SideEffectPolicy,
        audit: &mut dyn SideEffectAuditSink,
    ) -> Result<SkillExecutionResult, SkillExecutionError>;

    fn handles(&self, skill_id: &str) -> bool {
        self.definitions()
            .iter()
            .any(|definition| definition.id == skill_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_definitions_without_schema_or_description() {
        let missing_schema = ExternalToolDefinition::new(
            "mcp.demo.echo",
            "demo",
            "Echoes input",
            Value::Null,
            Vec::new(),
            Vec::new(),
        );
        assert!(missing_schema.validate().is_err());

        let missing_description = ExternalToolDefinition::new(
            "mcp.demo.echo",
            "demo",
            "",
            serde_json::json!({"type": "object"}),
            Vec::new(),
            Vec::new(),
        );
        assert!(missing_description.validate().is_err());
        assert!(missing_schema
            .validate()
            .unwrap_err()
            .contains("JSON Schema"));
    }

    #[test]
    fn accepts_complete_definitions() {
        let definition = ExternalToolDefinition::new(
            "mcp.demo.echo",
            "demo",
            "Echoes input",
            serde_json::json!({"type": "object"}),
            vec![Permission::Network],
            vec![SideEffectClass::Network],
        );

        definition.validate().expect("definition is valid");
        assert_eq!(definition.source, "demo");
    }
}
