use serde::{Deserialize, Serialize};
use std::fmt;

/// A category of externally observable work performed by a skill executor.
///
/// Git operations carry both [`Process`](Self::Process) and [`Git`](Self::Git)
/// classifications so a policy may constrain all child processes or Git alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectClass {
    FilesystemRead,
    FilesystemWrite,
    Network,
    Process,
    Git,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyAction {
    Allow,
    Ask,
    Deny,
}

impl PolicyAction {
    const fn priority(self) -> u8 {
        match self {
            Self::Allow => 0,
            Self::Ask => 1,
            Self::Deny => 2,
        }
    }
}

/// One side effect about to be performed by an executor.
///
/// Targets must describe the resource without carrying request bodies or
/// credentials. Built-in executors redact URL query strings and never include
/// file contents here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideEffectRequest {
    pub skill_id: String,
    pub operation: String,
    pub classes: Vec<SideEffectClass>,
    pub target: Option<String>,
}

impl SideEffectRequest {
    pub fn new(
        skill_id: impl Into<String>,
        operation: impl Into<String>,
        classes: impl IntoIterator<Item = SideEffectClass>,
        target: Option<String>,
    ) -> Self {
        let mut classes = classes.into_iter().collect::<Vec<_>>();
        classes.sort_unstable();
        classes.dedup();
        Self {
            skill_id: skill_id.into(),
            operation: operation.into(),
            classes,
            target,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MatchedPolicyRule {
    pub class: SideEffectClass,
    pub action: PolicyAction,
}

/// The deterministic policy evaluation before any interactive approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideEffectEvaluation {
    pub request: SideEffectRequest,
    pub action: PolicyAction,
    pub matched_rules: Vec<MatchedPolicyRule>,
    pub reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PolicyOutcome {
    Allowed,
    Denied,
}

/// The final, serializable authorization decision, including ask resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SideEffectDecision {
    pub evaluation: SideEffectEvaluation,
    pub outcome: PolicyOutcome,
}

impl fmt::Display for SideEffectDecision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{} for {} ({})",
            match self.outcome {
                PolicyOutcome::Allowed => "allowed",
                PolicyOutcome::Denied => "denied",
            },
            self.evaluation.request.operation,
            self.evaluation.reason
        )
    }
}

/// Central policy applied to every built-in skill executor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SideEffectPolicy {
    pub filesystem_read: PolicyAction,
    pub filesystem_write: PolicyAction,
    pub network: PolicyAction,
    pub process: PolicyAction,
    pub git: PolicyAction,
}

impl Default for SideEffectPolicy {
    fn default() -> Self {
        Self {
            filesystem_read: PolicyAction::Allow,
            filesystem_write: PolicyAction::Ask,
            network: PolicyAction::Ask,
            process: PolicyAction::Ask,
            git: PolicyAction::Ask,
        }
    }
}

impl SideEffectPolicy {
    /// Reproduces the approvals used before centralized policy enforcement.
    pub fn backward_compatible(auto_approve_medium_risk: bool) -> Self {
        Self {
            filesystem_read: PolicyAction::Allow,
            filesystem_write: PolicyAction::Ask,
            network: if auto_approve_medium_risk {
                PolicyAction::Allow
            } else {
                PolicyAction::Ask
            },
            process: PolicyAction::Allow,
            git: PolicyAction::Allow,
        }
    }

    pub const fn allow_all() -> Self {
        Self {
            filesystem_read: PolicyAction::Allow,
            filesystem_write: PolicyAction::Allow,
            network: PolicyAction::Allow,
            process: PolicyAction::Allow,
            git: PolicyAction::Allow,
        }
    }

    pub const fn deny_all() -> Self {
        Self {
            filesystem_read: PolicyAction::Deny,
            filesystem_write: PolicyAction::Deny,
            network: PolicyAction::Deny,
            process: PolicyAction::Deny,
            git: PolicyAction::Deny,
        }
    }

    pub fn evaluate(&self, request: SideEffectRequest) -> SideEffectEvaluation {
        let matched_rules = request
            .classes
            .iter()
            .copied()
            .map(|class| MatchedPolicyRule {
                class,
                action: self.action_for(class),
            })
            .collect::<Vec<_>>();
        let action = matched_rules
            .iter()
            .map(|rule| rule.action)
            .max_by_key(|action| action.priority())
            // Unknown/unclassified effects fail closed.
            .unwrap_or(PolicyAction::Deny);
        let reason = if matched_rules.is_empty() {
            "side effect has no classification; denied by default".to_string()
        } else {
            matched_rules
                .iter()
                .map(|rule| format!("{:?}={:?}", rule.class, rule.action).to_ascii_lowercase())
                .collect::<Vec<_>>()
                .join(", ")
        };

        SideEffectEvaluation {
            request,
            action,
            matched_rules,
            reason,
        }
    }

    pub const fn action_for(&self, class: SideEffectClass) -> PolicyAction {
        match class {
            SideEffectClass::FilesystemRead => self.filesystem_read,
            SideEffectClass::FilesystemWrite => self.filesystem_write,
            SideEffectClass::Network => self.network,
            SideEffectClass::Process => self.process,
            SideEffectClass::Git => self.git,
        }
    }
}

/// Receives final policy decisions. Implementations should avoid blocking.
pub trait SideEffectAuditSink {
    fn record(&mut self, decision: &SideEffectDecision);
}

#[derive(Debug, Default)]
pub struct NoopSideEffectAuditSink;

impl SideEffectAuditSink for NoopSideEffectAuditSink {
    fn record(&mut self, _decision: &SideEffectDecision) {}
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordingSideEffectAuditSink {
    decisions: Vec<SideEffectDecision>,
}

impl RecordingSideEffectAuditSink {
    pub fn decisions(&self) -> &[SideEffectDecision] {
        &self.decisions
    }

    pub fn into_decisions(self) -> Vec<SideEffectDecision> {
        self.decisions
    }
}

impl SideEffectAuditSink for RecordingSideEffectAuditSink {
    fn record(&mut self, decision: &SideEffectDecision) {
        self.decisions.push(decision.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strictest_matching_rule_wins_for_git_processes() {
        let policy = SideEffectPolicy {
            process: PolicyAction::Ask,
            git: PolicyAction::Deny,
            ..SideEffectPolicy::allow_all()
        };
        let evaluation = policy.evaluate(SideEffectRequest::new(
            "git.diff",
            "git.diff",
            [SideEffectClass::Git, SideEffectClass::Process],
            Some(".".to_string()),
        ));

        assert_eq!(evaluation.action, PolicyAction::Deny);
        assert_eq!(evaluation.matched_rules.len(), 2);
    }

    #[test]
    fn unclassified_side_effects_fail_closed() {
        let evaluation = SideEffectPolicy::allow_all().evaluate(SideEffectRequest::new(
            "unknown",
            "unknown",
            [],
            None,
        ));

        assert_eq!(evaluation.action, PolicyAction::Deny);
        assert!(evaluation.reason.contains("no classification"));
    }

    #[test]
    fn default_policy_deserializes_missing_fields_safely() {
        let policy: SideEffectPolicy =
            serde_json::from_str(r#"{"network":"deny"}"#).expect("policy should deserialize");

        assert_eq!(policy.network, PolicyAction::Deny);
        assert_eq!(policy.filesystem_write, PolicyAction::Ask);
        assert_eq!(policy.filesystem_read, PolicyAction::Allow);
        assert_eq!(policy.process, PolicyAction::Ask);
    }

    #[test]
    fn final_decisions_are_serializable_and_recordable() {
        let evaluation = SideEffectPolicy::default().evaluate(SideEffectRequest::new(
            "file.write",
            "file.write",
            [SideEffectClass::FilesystemWrite],
            Some("README.md".to_string()),
        ));
        let decision = SideEffectDecision {
            evaluation,
            outcome: PolicyOutcome::Allowed,
        };
        let encoded = serde_json::to_string(&decision).expect("serialize decision");
        let decoded: SideEffectDecision =
            serde_json::from_str(&encoded).expect("deserialize decision");
        let mut audit = RecordingSideEffectAuditSink::default();
        audit.record(&decoded);

        assert_eq!(audit.decisions(), &[decision]);
    }

    #[test]
    fn backward_compatible_profile_preserves_network_auto_approval() {
        assert_eq!(
            SideEffectPolicy::backward_compatible(false).network,
            PolicyAction::Ask
        );
        assert_eq!(
            SideEffectPolicy::backward_compatible(true).network,
            PolicyAction::Allow
        );
        assert_eq!(
            SideEffectPolicy::backward_compatible(false).git,
            PolicyAction::Allow
        );
    }
}
