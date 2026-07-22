use anyhow::{anyhow, Result};
use axiom_core::AxiomConfig;
use axiom_engine::{
    PolicyAction, RecordingSideEffectAuditSink, SideEffectDecision, SideEffectPolicy,
};
use axiom_proof::{PolicyDecisionProof, ProofRecorder};

pub(crate) fn configured_policy(config: &AxiomConfig) -> Result<SideEffectPolicy> {
    Ok(SideEffectPolicy {
        filesystem_read: parse_action(&config.policy.filesystem_read)?,
        filesystem_write: parse_action(&config.policy.filesystem_write)?,
        network: parse_action(&config.policy.network)?,
        process: parse_action(&config.policy.process)?,
        git: parse_action(&config.policy.git)?,
    })
}

pub(crate) fn record_audit(proof: &mut ProofRecorder, audit: RecordingSideEffectAuditSink) {
    for decision in audit.into_decisions() {
        proof.record_policy_decision(decision_proof(&decision));
    }
}

pub(crate) fn decision_proof(decision: &SideEffectDecision) -> PolicyDecisionProof {
    PolicyDecisionProof {
        event_id: axiom_proof::trace::new_event_id("policy"),
        skill_id: decision.evaluation.request.skill_id.clone(),
        operation: decision.evaluation.request.operation.clone(),
        classes: decision
            .evaluation
            .request
            .classes
            .iter()
            .map(|class| format!("{class:?}").to_ascii_lowercase())
            .collect(),
        action: format!("{:?}", decision.evaluation.action).to_ascii_lowercase(),
        outcome: format!("{:?}", decision.outcome).to_ascii_lowercase(),
        target: decision
            .evaluation
            .request
            .target
            .as_deref()
            .map(axiom_proof::redact_text),
        reason: decision.evaluation.reason.clone(),
    }
}

fn parse_action(value: &str) -> Result<PolicyAction> {
    match value {
        "allow" => Ok(PolicyAction::Allow),
        "ask" => Ok(PolicyAction::Ask),
        "deny" => Ok(PolicyAction::Deny),
        _ => Err(anyhow!("invalid side-effect policy action: {value}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_configured_policy_class() {
        let mut config = AxiomConfig::default();
        config.policy.filesystem_read = "deny".to_string();
        config.policy.filesystem_write = "allow".to_string();
        config.policy.network = "ask".to_string();
        config.policy.process = "deny".to_string();
        config.policy.git = "allow".to_string();

        let policy = configured_policy(&config).expect("configured policy");

        assert_eq!(policy.filesystem_read, PolicyAction::Deny);
        assert_eq!(policy.filesystem_write, PolicyAction::Allow);
        assert_eq!(policy.network, PolicyAction::Ask);
        assert_eq!(policy.process, PolicyAction::Deny);
        assert_eq!(policy.git, PolicyAction::Allow);
    }

    #[test]
    fn invalid_policy_action_fails_closed_during_mapping() {
        let mut config = AxiomConfig::default();
        config.policy.process = "sometimes".to_string();

        let error = configured_policy(&config).expect_err("invalid action must fail");

        assert!(error
            .to_string()
            .contains("invalid side-effect policy action"));
    }
}
