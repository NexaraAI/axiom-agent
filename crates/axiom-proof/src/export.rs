use crate::ProofTrace;

pub fn to_json(trace: &ProofTrace) -> serde_json::Result<String> {
    serde_json::to_string_pretty(trace)
}
