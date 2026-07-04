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
pub use redaction::{redact_text, summarize_text};
pub use storage::{
    export_trace_to_format, find_proof, latest_proof, list_proofs, ProofFormat, ProofIndexEntry,
};
pub use trace::{
    ApprovalProof, CommandProof, ErrorProof, FileReadProof, FileWriteProof, LensProof, PatchProof,
    ProofMode, ProofStatus, ProofTrace, SkillCardProof, SkillsProof, TestProof, ToolCallProof,
};
