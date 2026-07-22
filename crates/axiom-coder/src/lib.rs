pub mod checkpoint;
pub mod guard;
pub mod patch;
pub mod planner;
pub mod project_scan;
pub mod test_runner;

pub use checkpoint::{list_checkpoints, CheckpointError, CheckpointFile, WorkspaceCheckpoint};
pub use guard::ApprovalMode;
pub use patch::{
    diff_for_change, diff_for_patch, parse_axiom_patch, prepare_patch, sha256_hex, validate_patch,
    verify_prepared_patch, AxiomPatch, FileChange, PatchAction, PatchError, PatchHunk,
    PatchPreview, PreparedFileChange, PreparedPatch,
};
pub use planner::{
    build_fallback_plan, build_patch_prompt, build_patch_prompt_with_context, build_plan_prompt,
    verify_patch_against_plan, CodingPlan, PatchContextFile, PlanPatchVerification,
};
pub use project_scan::{scan_project, ProjectScanSummary, ProjectType};
pub use test_runner::{detect_test_commands, TestCommand};
