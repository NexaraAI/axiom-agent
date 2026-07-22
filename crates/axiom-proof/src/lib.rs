pub mod export;
pub mod recorder;
pub mod redaction;
pub mod report;
pub mod storage;
pub mod trace;

pub use recorder::{
    new_approval, new_tool_call, LensSelectionRecord, ProofExportPaths, ProofRecorder,
    ProofSettings,
};
pub use redaction::{redact_text, register_secret_for_redaction, summarize_text};
pub use storage::{
    export_trace_to_format, find_proof, latest_proof, list_proofs, load_proof_trace, ProofFormat,
    ProofIndexEntry,
};
pub use trace::{
    validate_trace_version, AgentRuntimeProof, ApprovalProof, CheckpointProof, CommandProof,
    ErrorProof, FileReadProof, FileWriteProof, LensProof, PatchProof, PolicyDecisionProof,
    ProofMode, ProofStatus, ProofTrace, ProofTraceLoadError, ProofTraceVersionError,
    SkillCardProof, SkillsProof, TestProof, ToolCallProof, CURRENT_TRACE_VERSION,
    SUPPORTED_TRACE_VERSIONS,
};
