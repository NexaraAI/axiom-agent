use serde_json::{Map, Value};

use crate::{redact_text, ProofTrace};

const REDACTED: &str = "[REDACTED]";

pub fn to_json(trace: &ProofTrace) -> serde_json::Result<String> {
    serde_json::to_string_pretty(&redacted_value(trace)?)
}

/// Returns a clone suitable for any durable or user-visible proof export.
///
/// The conversion deliberately happens through `serde_json::Value`: every
/// string in the complete serialized shape is visited, including strings in
/// structs added after this code was written. Recorder-level redaction remains
/// useful for limiting in-memory exposure, but this boundary is the fail-safe.
pub(crate) fn redacted_trace(trace: &ProofTrace) -> serde_json::Result<ProofTrace> {
    serde_json::from_value(redacted_value(trace)?)
}

fn redacted_value(trace: &ProofTrace) -> serde_json::Result<Value> {
    let mut value = serde_json::to_value(trace)?;
    // These root-level fields are serde enum/version discriminators. Redacting
    // a value such as `completed` would make a valid trace impossible to load
    // again, so preserve the known structural strings while continuing to
    // redact every user-controlled field below them.
    if let Value::Object(object) = &mut value {
        redact_root_json_object(object);
    } else {
        redact_json_value(&mut value);
    }
    Ok(value)
}

fn redact_json_value(value: &mut Value) {
    match value {
        Value::String(text) => *text = redact_string(text),
        Value::Array(values) => {
            for value in values {
                redact_json_value(value);
            }
        }
        Value::Object(object) => redact_json_object(object),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn redact_json_object(object: &mut Map<String, Value>) {
    let original = std::mem::take(object);
    for (key, mut value) in original {
        let redacted_key = redact_text(&key);
        if is_secret_key(&key) {
            value = Value::String(REDACTED.to_string());
        } else {
            redact_json_value(&mut value);
        }
        object.insert(redacted_key, value);
    }
}

fn redact_root_json_object(object: &mut Map<String, Value>) {
    let original = std::mem::take(object);
    for (key, mut value) in original {
        let redacted_key = redact_text(&key);
        if is_secret_key(&key) {
            value = Value::String(REDACTED.to_string());
        } else if !is_structural_trace_key(&key) {
            redact_json_value(&mut value);
        }
        object.insert(redacted_key, value);
    }
}

fn is_structural_trace_key(key: &str) -> bool {
    matches!(key, "trace_version" | "mode" | "status")
}

fn redact_string(input: &str) -> String {
    let trimmed = input.trim();
    let is_json_container = (trimmed.starts_with('{') && trimmed.ends_with('}'))
        || (trimmed.starts_with('[') && trimmed.ends_with(']'));
    if is_json_container {
        if let Ok(mut nested) = serde_json::from_str::<Value>(trimmed) {
            redact_json_value(&mut nested);
            if let Ok(redacted) = serde_json::to_string(&nested) {
                return redacted;
            }
        }
    }
    redact_text(input)
}

fn is_secret_key(key: &str) -> bool {
    let mut normalized = String::with_capacity(key.len());
    let mut previous_was_lower_or_digit = false;
    for character in key.chars() {
        if character.is_ascii_alphanumeric() {
            if character.is_ascii_uppercase()
                && previous_was_lower_or_digit
                && !normalized.ends_with('_')
            {
                normalized.push('_');
            }
            normalized.push(character.to_ascii_lowercase());
            previous_was_lower_or_digit =
                character.is_ascii_lowercase() || character.is_ascii_digit();
        } else {
            if !normalized.ends_with('_') {
                normalized.push('_');
            }
            previous_was_lower_or_digit = false;
        }
    }
    let normalized = normalized.trim_matches('_');

    matches!(
        normalized,
        "api_key"
            | "apikey"
            | "api_token"
            | "authorization"
            | "credential"
            | "credentials"
            | "password"
            | "private_key"
            | "secret"
            | "secret_key"
            | "token"
    ) || [
        "_access_key",
        "_access_token",
        "_api_key",
        "_api_token",
        "_authorization",
        "_client_secret",
        "_credential",
        "_credentials",
        "_password",
        "_private_key",
        "_refresh_token",
        "_secret",
        "_secret_key",
        "_token",
    ]
    .iter()
    .any(|suffix| normalized.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use serde_json::{Map, Value};

    use super::*;
    use crate::{
        register_secret_for_redaction, ApprovalProof, CheckpointProof, CommandProof, ErrorProof,
        FileReadProof, FileWriteProof, LensProof, PatchProof, PolicyDecisionProof, ProofMode,
        ProofStatus, SkillCardProof, SkillsProof, TestProof, ToolCallProof,
    };

    #[test]
    fn json_and_markdown_redact_every_string_family_at_the_export_boundary() {
        let secret = ["opaque", "proof", "boundary", "4f1a", "payload"].join("-");
        let nested_secret = ["opaque", "nested", "boundary", "7e2b", "payload"].join("-");
        register_secret_for_redaction(&secret);
        let tagged = |label: &str| format!("{label}-{secret}");

        let sensitive_key = ["access", "token"].join("_");
        let mut nested = Map::new();
        nested.insert(sensitive_key.clone(), Value::String(nested_secret.clone()));
        nested.insert(
            "semantic".to_string(),
            Value::String(tagged("nested-semantic")),
        );

        let mut trace = ProofTrace::new(
            ProofMode::Coder,
            tagged("session"),
            tagged("task"),
            tagged("prompt"),
        );
        trace.started_at = tagged("started");
        trace.ended_at = Some(tagged("ended"));
        trace.status = ProofStatus::Completed;
        trace.provider = Some(tagged("provider"));
        trace.model = Some(tagged("model"));
        trace.workspace = Some(tagged("workspace"));
        trace.lens = LensProof {
            enabled: true,
            selected_skill_ids: vec![tagged("selected-skill")],
            reason_summary: Some(tagged("lens-reason")),
            auto_routed_to_coder: true,
            auto_route_mode: Some(tagged("route-mode")),
        };
        trace.skills = SkillsProof {
            installed_skill_count: 1,
            selected_cards: vec![SkillCardProof {
                id: tagged("card-id"),
                summary: tagged("card-summary"),
                risk_level: tagged("card-risk"),
            }],
            executed_skill_ids: vec![tagged("executed-skill")],
        };
        trace.tool_calls.push(ToolCallProof {
            event_id: tagged("tool-event"),
            skill_id: tagged("tool-skill"),
            arguments_summary: Value::Object(nested).to_string(),
            success: true,
            started_at: tagged("tool-started"),
            ended_at: Some(tagged("tool-ended")),
            risk_level: Some(tagged("tool-risk")),
            permission_result: Some(tagged("permission")),
            output_summary: Some(tagged("tool-output")),
            error: Some(tagged("tool-error")),
        });
        trace.file_reads.push(FileReadProof {
            event_id: tagged("read-event"),
            path: tagged("read-path"),
            bytes: Some(17),
            allowed: false,
            blocked_reason: Some(tagged("blocked-reason")),
        });
        trace.file_writes.push(FileWriteProof {
            event_id: tagged("write-event"),
            path: tagged("write-path"),
            bytes_written: Some(23),
            created: true,
            overwrote: false,
            approved: true,
            diff_summary: Some(tagged("write-diff")),
        });
        trace.commands.push(CommandProof {
            event_id: tagged("command-event"),
            command: tagged("command"),
            cwd: tagged("cwd"),
            allowed: true,
            approved: true,
            exit_code: Some(0),
            stdout_summary: Some(tagged("stdout")),
            stderr_summary: Some(tagged("stderr")),
        });
        trace.approvals.push(ApprovalProof {
            approval_id: tagged("approval-id"),
            action: tagged("approval-action"),
            risk_level: tagged("approval-risk"),
            prompt: tagged("approval-prompt"),
            user_decision: tagged("approval-decision"),
            timestamp: tagged("approval-time"),
        });
        trace.policy_decisions.push(PolicyDecisionProof {
            event_id: tagged("policy-event"),
            skill_id: tagged("policy-skill"),
            operation: tagged("policy-operation"),
            classes: vec![tagged("policy-class")],
            action: tagged("policy-action"),
            outcome: tagged("policy-outcome"),
            target: Some(tagged("policy-target")),
            reason: tagged("policy-reason"),
        });
        trace.patches.push(PatchProof {
            event_id: tagged("patch-event"),
            summary: tagged("patch-summary"),
            changed_files: vec![tagged("patch-file")],
            diff: tagged("patch-diff"),
            approved: true,
            applied: true,
        });
        trace.checkpoints.push(CheckpointProof {
            event_id: tagged("checkpoint-event"),
            checkpoint_id: tagged("checkpoint-id"),
            path: tagged("checkpoint-path"),
            files: vec![tagged("checkpoint-file")],
            restored: true,
            reason: tagged("checkpoint-reason"),
        });
        trace.tests.push(TestProof {
            event_id: tagged("test-event"),
            detected_command: tagged("test-command"),
            ran: true,
            approved: true,
            exit_code: Some(0),
            passed: Some(true),
            output_summary: Some(tagged("test-output")),
        });
        trace.errors.push(ErrorProof {
            event_id: tagged("error-event"),
            error_type: tagged("error-type"),
            message: tagged("error-message"),
            stage: tagged("error-stage"),
            recoverable: true,
        });
        trace.final_response = Some(tagged("final-response"));
        trace.summary = Some(tagged("summary"));
        trace.redactions.push(tagged("redaction-note"));

        let json = to_json(&trace).expect("serialize redacted proof");
        let markdown = crate::report::markdown_summary(&trace).expect("render redacted proof");

        for output in [&json, &markdown] {
            assert!(!output.contains(&secret));
            assert!(!output.contains(&nested_secret));
            assert!(output.contains(REDACTED));
        }
        assert!(json.contains("nested-semantic-[REDACTED]"));
        assert!(markdown.contains("provider-[REDACTED]"));
        assert!(markdown.contains("command-[REDACTED]"));
        assert!(markdown.contains("checkpoint-reason-[REDACTED]"));

        let json: Value = serde_json::from_str(&json).expect("parse proof JSON");
        assert_eq!(json["status"], "completed");
        assert_eq!(json["file_reads"][0]["bytes"], 17);
        assert_eq!(json["commands"][0]["exit_code"], 0);
        let arguments = json["tool_calls"][0]["arguments_summary"]
            .as_str()
            .expect("arguments string");
        let arguments: Value = serde_json::from_str(arguments).expect("nested JSON remains valid");
        assert_eq!(arguments[&sensitive_key], REDACTED);
        assert_eq!(arguments["semantic"], "nested-semantic-[REDACTED]");
    }

    #[test]
    fn camel_case_secret_keys_are_redacted_at_nested_export_boundaries() {
        let secret = ["opaque", "camel", "case", "secret", "8d19"].join("-");
        let mut value = serde_json::json!({
            "metadata": {
                "accessToken": secret,
                "clientSecret": "another-secret"
            }
        });

        redact_json_value(&mut value);

        assert_eq!(value["metadata"]["accessToken"], REDACTED);
        assert_eq!(value["metadata"]["clientSecret"], REDACTED);
    }

    #[test]
    fn root_trace_discriminators_remain_loadable() {
        let trace = ProofTrace::new(ProofMode::Chat, "session", "task", "prompt");
        let exported = redacted_trace(&trace).expect("redacted trace");

        assert_eq!(exported.trace_version, trace.trace_version);
        assert_eq!(exported.mode, trace.mode);
        assert_eq!(exported.status, trace.status);
    }
}
