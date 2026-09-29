use std::path::Path;

use axiom_core::is_secret_path;

use crate::{
    ApprovalRequest, PolicyAction, PolicyOutcome, SideEffectAuditSink, SideEffectClass,
    SideEffectDecision, SideEffectPolicy, SideEffectRequest, SkillApproval, SkillExecutionError,
};

pub fn authorize_side_effect(
    policy: &SideEffectPolicy,
    audit: &mut dyn SideEffectAuditSink,
    approval: &mut dyn SkillApproval,
    side_effect: SideEffectRequest,
) -> Result<(), SkillExecutionError> {
    let evaluation = policy.evaluate(side_effect);
    let prompt = policy_approval_prompt(&evaluation.request);
    let outcome = match evaluation.action {
        PolicyAction::Allow => PolicyOutcome::Allowed,
        PolicyAction::Deny => PolicyOutcome::Denied,
        PolicyAction::Ask => {
            let request = ApprovalRequest {
                skill_id: evaluation.request.skill_id.clone(),
                message: prompt.clone(),
                risk_level: policy_risk_level(&evaluation.request).to_string(),
            };
            if approval.approve(&request) {
                PolicyOutcome::Allowed
            } else {
                PolicyOutcome::Denied
            }
        }
    };
    let action = evaluation.action;
    let decision = SideEffectDecision {
        evaluation,
        outcome,
    };
    audit.record(&decision);

    match (action, outcome) {
        (PolicyAction::Deny, _) => Err(SkillExecutionError::SideEffectPolicyDenied(Box::new(
            decision,
        ))),
        (PolicyAction::Ask, PolicyOutcome::Denied) => {
            Err(SkillExecutionError::ApprovalDenied(prompt))
        }
        _ => Ok(()),
    }
}

fn policy_approval_prompt(request: &SideEffectRequest) -> String {
    match request.target.as_deref() {
        Some(target) => format!(
            "Allow `{}` to perform `{}` on `{target}`?",
            request.skill_id, request.operation
        ),
        None => format!(
            "Allow `{}` to perform `{}`?",
            request.skill_id, request.operation
        ),
    }
}

fn policy_risk_level(request: &SideEffectRequest) -> &'static str {
    if request.classes == [SideEffectClass::FilesystemRead] {
        "low"
    } else {
        "medium"
    }
}

pub(super) fn block_secret_path(path: impl AsRef<Path>) -> Result<(), SkillExecutionError> {
    let path = path.as_ref();
    if is_secret_path(path) {
        Err(SkillExecutionError::SecretPath(path.display().to_string()))
    } else {
        Ok(())
    }
}
