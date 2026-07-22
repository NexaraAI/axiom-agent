use std::{fs, path::Path};

use axiom_core::{is_secret_path, AxiomError, Workspace};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AxiomPatch {
    pub summary: String,
    #[serde(default)]
    pub test_command: Option<String>,
    #[serde(default)]
    pub changes: Vec<FileChange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub action: PatchAction,
    #[serde(default)]
    pub base_sha256: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub hunks: Vec<PatchHunk>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatchHunk {
    /// One-based line in the base file. Use 1 for an insertion into an empty
    /// file.
    pub old_start: usize,
    #[serde(default)]
    pub old_lines: Vec<String>,
    #[serde(default)]
    pub new_lines: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PatchAction {
    CreateOrUpdate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchPreview {
    pub diff: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedPatch {
    pub summary: String,
    pub test_command: Option<String>,
    pub files: Vec<PreparedFileChange>,
    pub diff: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedFileChange {
    pub path: String,
    pub content: String,
    pub observed_sha256: Option<String>,
    pub existed: bool,
}

#[derive(Debug, Error)]
pub enum PatchError {
    #[error("no axiom-patch block found")]
    MissingPatchBlock,
    #[error("failed to parse axiom patch JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("patch has no file changes")]
    EmptyPatch,
    #[error("patch path is empty")]
    EmptyPath,
    #[error("patch action is not supported for {path}")]
    UnsupportedAction { path: String },
    #[error("patch must contain exactly one edit form (`content` or `hunks`) for {path}")]
    AmbiguousEdit { path: String },
    #[error("existing file patch is missing base_sha256: {0}")]
    MissingBaseHash(String),
    #[error("base_sha256 must be 64 hexadecimal characters: {0}")]
    InvalidBaseHash(String),
    #[error("patch hunks overlap or are out of order for {0}")]
    OverlappingHunks(String),
    #[error("patch conflict for {path}: {reason}")]
    Conflict { path: String, reason: String },
    #[error("patch contains the same resolved path more than once: {0}")]
    DuplicatePath(String),
    #[error("patch has too many or oversized hunks for: {0}")]
    HunkLimit(String),
    #[error("workspace path error: {0}")]
    Workspace(#[from] AxiomError),
    #[error("blocked secret path: {0}")]
    SecretPath(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

pub fn parse_axiom_patch(text: &str) -> Result<AxiomPatch, PatchError> {
    let json_text = extract_patch_json(text)?;
    let patch = serde_json::from_str(json_text.trim())?;
    Ok(patch)
}

pub fn validate_patch(
    patch: &AxiomPatch,
    workspace_root: impl AsRef<Path>,
) -> Result<(), PatchError> {
    if patch.changes.is_empty() {
        return Err(PatchError::EmptyPatch);
    }

    prepare_patch(patch, workspace_root).map(|_| ())
}

pub fn validate_change(change: &FileChange, workspace: &Workspace) -> Result<(), PatchError> {
    if change.path.trim().is_empty() {
        return Err(PatchError::EmptyPath);
    }

    if !matches!(change.action, PatchAction::CreateOrUpdate) {
        return Err(PatchError::UnsupportedAction {
            path: change.path.clone(),
        });
    }

    if change.content.is_some() != change.hunks.is_empty() {
        return Err(PatchError::AmbiguousEdit {
            path: change.path.clone(),
        });
    }
    if let Some(base) = &change.base_sha256 {
        if base.len() != 64 || !base.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(PatchError::InvalidBaseHash(change.path.clone()));
        }
    }
    if change.hunks.len() > 128
        || change
            .hunks
            .iter()
            .any(|hunk| hunk.old_lines.len() > 5_000 || hunk.new_lines.len() > 5_000)
    {
        return Err(PatchError::HunkLimit(change.path.clone()));
    }
    validate_hunk_order(change)?;

    block_secret_path(&change.path)?;
    let resolved = workspace.resolve_inside(&change.path)?;
    block_secret_path(&resolved)?;
    Ok(())
}

pub fn diff_for_patch(
    patch: &AxiomPatch,
    workspace_root: impl AsRef<Path>,
) -> Result<PatchPreview, PatchError> {
    Ok(PatchPreview {
        diff: prepare_patch(patch, workspace_root)?.diff,
    })
}

pub fn prepare_patch(
    patch: &AxiomPatch,
    workspace_root: impl AsRef<Path>,
) -> Result<PreparedPatch, PatchError> {
    if patch.changes.is_empty() {
        return Err(PatchError::EmptyPatch);
    }
    let workspace = Workspace::new(workspace_root)?;
    let mut files = Vec::with_capacity(patch.changes.len());
    let mut diff = String::new();
    let mut resolved_paths = std::collections::BTreeSet::new();
    for change in &patch.changes {
        validate_change(change, &workspace)?;
        let resolved = workspace.resolve_inside(&change.path)?;
        if !resolved_paths.insert(resolved.clone()) {
            return Err(PatchError::DuplicatePath(change.path.clone()));
        }
        let existed = resolved.exists();
        let old_content = if existed {
            fs::read_to_string(&resolved)?
        } else {
            String::new()
        };
        let observed_sha256 = existed.then(|| sha256_hex(old_content.as_bytes()));
        let content = materialize_change(change, &old_content, observed_sha256.as_deref())?;
        diff.push_str(&diff_for_change(&change.path, &old_content, &content));
        files.push(PreparedFileChange {
            path: change.path.clone(),
            content,
            observed_sha256,
            existed,
        });
    }
    Ok(PreparedPatch {
        summary: patch.summary.clone(),
        test_command: patch.test_command.clone(),
        files,
        diff,
    })
}

pub fn verify_prepared_patch(
    patch: &PreparedPatch,
    workspace_root: impl AsRef<Path>,
) -> Result<(), PatchError> {
    let workspace = Workspace::new(workspace_root)?;
    for file in &patch.files {
        block_secret_path(&file.path)?;
        let resolved = workspace.resolve_inside(&file.path)?;
        block_secret_path(&resolved)?;
        let current_sha256 = if resolved.exists() {
            Some(sha256_hex(&fs::read(&resolved)?))
        } else {
            None
        };
        if current_sha256 != file.observed_sha256 {
            return Err(PatchError::Conflict {
                path: file.path.clone(),
                reason: "file changed after preview; regenerate the patch".to_string(),
            });
        }
    }
    Ok(())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn materialize_change(
    change: &FileChange,
    old_content: &str,
    observed_sha256: Option<&str>,
) -> Result<String, PatchError> {
    if observed_sha256.is_some() && change.base_sha256.is_none() {
        return Err(PatchError::MissingBaseHash(change.path.clone()));
    }
    let base_matches = match (change.base_sha256.as_deref(), observed_sha256) {
        (Some(expected), Some(observed)) => expected.eq_ignore_ascii_case(observed),
        (None, None) => true,
        (Some(_), None) => false,
        (None, Some(_)) => false,
    };

    if let Some(content) = &change.content {
        if !base_matches {
            return Err(PatchError::Conflict {
                path: change.path.clone(),
                reason: "base SHA-256 does not match the current file".to_string(),
            });
        }
        return Ok(content.clone());
    }

    apply_hunks(change, old_content, base_matches)
}

fn apply_hunks(
    change: &FileChange,
    old_content: &str,
    base_matches: bool,
) -> Result<String, PatchError> {
    let had_trailing_newline = old_content.ends_with('\n');
    let mut lines = old_content.lines().map(str::to_string).collect::<Vec<_>>();
    let mut hunks = change.hunks.iter().collect::<Vec<_>>();
    hunks.sort_by_key(|hunk| std::cmp::Reverse(hunk.old_start));

    for hunk in hunks {
        let expected_index = hunk.old_start.saturating_sub(1);
        let index = if base_matches {
            if !lines_match(&lines, expected_index, &hunk.old_lines) {
                return Err(PatchError::Conflict {
                    path: change.path.clone(),
                    reason: format!("hunk context does not match at line {}", hunk.old_start),
                });
            }
            expected_index
        } else {
            locate_hunk(&lines, &hunk.old_lines).ok_or_else(|| PatchError::Conflict {
                path: change.path.clone(),
                reason: format!(
                    "base changed and hunk at line {} could not be relocated uniquely",
                    hunk.old_start
                ),
            })?
        };
        let end = index + hunk.old_lines.len();
        lines.splice(index..end, hunk.new_lines.clone());
    }

    let mut content = lines.join("\n");
    if (!content.is_empty() && had_trailing_newline)
        || (!change.hunks.is_empty() && old_content.is_empty() && !content.is_empty())
    {
        content.push('\n');
    }
    Ok(content)
}

fn locate_hunk(lines: &[String], old_lines: &[String]) -> Option<usize> {
    if old_lines.is_empty() {
        return None;
    }
    let matches = (0..=lines.len().saturating_sub(old_lines.len()))
        .filter(|index| lines_match(lines, *index, old_lines))
        .take(2)
        .collect::<Vec<_>>();
    (matches.len() == 1).then_some(matches[0])
}

fn lines_match(lines: &[String], index: usize, expected: &[String]) -> bool {
    index <= lines.len()
        && index + expected.len() <= lines.len()
        && lines[index..index + expected.len()] == *expected
}

fn validate_hunk_order(change: &FileChange) -> Result<(), PatchError> {
    let mut previous_end = 0;
    for hunk in &change.hunks {
        if hunk.old_start == 0 || hunk.old_start < previous_end {
            return Err(PatchError::OverlappingHunks(change.path.clone()));
        }
        previous_end = hunk.old_start.saturating_add(hunk.old_lines.len());
    }
    Ok(())
}

pub fn diff_for_change(path: &str, old_content: &str, new_content: &str) -> String {
    let mut diff = String::new();
    diff.push_str(&format!("--- a/{path}\n"));
    diff.push_str(&format!("+++ b/{path}\n"));

    if old_content == new_content {
        diff.push_str(" unchanged\n");
        return diff;
    }

    diff.push_str("@@\n");
    for line in old_content.lines() {
        diff.push('-');
        diff.push_str(line);
        diff.push('\n');
    }
    for line in new_content.lines() {
        diff.push('+');
        diff.push_str(line);
        diff.push('\n');
    }

    if old_content.ends_with('\n') && !new_content.ends_with('\n') {
        diff.push_str("\\ No newline at end of new file\n");
    }

    diff
}

fn extract_patch_json(text: &str) -> Result<&str, PatchError> {
    let trimmed = text.trim();
    if trimmed.starts_with('{') {
        return Ok(trimmed);
    }

    let fence = "```axiom-patch";
    if let Some(start) = text.find(fence) {
        let json_start = start + fence.len();
        let after_start = text[json_start..].trim_start();
        let end = after_start
            .find("```")
            .ok_or(PatchError::MissingPatchBlock)?;
        return Ok(after_start[..end].trim());
    }

    let label = "axiom-patch:";
    if let Some(start) = text.find(label) {
        return Ok(text[start + label.len()..].trim());
    }

    Err(PatchError::MissingPatchBlock)
}

fn block_secret_path(path: impl AsRef<Path>) -> Result<(), PatchError> {
    let path = path.as_ref();
    if is_secret_path(path) {
        Err(PatchError::SecretPath(path.display().to_string()))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[test]
    fn parses_axiom_patch_block() {
        let patch = parse_axiom_patch(
            r#"```axiom-patch
{
  "summary": "Update README",
  "test_command": "cargo test",
  "changes": [
    {
      "path": "README.md",
      "action": "create_or_update",
      "content": "hello"
    }
  ]
}
```"#,
        )
        .expect("patch");

        assert_eq!(patch.summary, "Update README");
        assert_eq!(patch.changes[0].path, "README.md");
    }

    #[test]
    fn rejects_invalid_patch_json() {
        let error = parse_axiom_patch("```axiom-patch\nnot-json\n```").expect_err("invalid");

        assert!(matches!(error, PatchError::Json(_)));
    }

    #[test]
    fn malformed_patch_corpus_never_panics() {
        let mut state = 0xd1b5_4a32_d192_ed03_u64;
        for length in 0..512 {
            let mut input = String::with_capacity(length + 32);
            if length % 3 == 0 {
                input.push_str("```axiom-patch\n");
            }
            for _ in 0..length {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                input.push(char::from((state as u8) & 0x7f));
            }
            if length % 5 == 0 {
                input.push_str("\n```");
            }
            let _ = parse_axiom_patch(&input);
        }
    }

    #[test]
    fn rejects_path_outside_workspace() {
        let dir = unique_temp_dir();
        let patch = AxiomPatch {
            summary: "bad".to_string(),
            test_command: None,
            changes: vec![FileChange {
                path: "../outside.txt".to_string(),
                action: PatchAction::CreateOrUpdate,
                base_sha256: None,
                content: Some("no".to_string()),
                hunks: Vec::new(),
            }],
        };

        let error = validate_patch(&patch, &dir).expect_err("outside path");

        assert!(matches!(error, PatchError::Workspace(_)));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_secret_file_patch() {
        let dir = unique_temp_dir();
        let patch = AxiomPatch {
            summary: "bad".to_string(),
            test_command: None,
            changes: vec![FileChange {
                path: ".env.local".to_string(),
                action: PatchAction::CreateOrUpdate,
                base_sha256: None,
                content: Some("SECRET=1".to_string()),
                hunks: Vec::new(),
            }],
        };

        let error = validate_patch(&patch, &dir).expect_err("secret path");

        assert!(matches!(error, PatchError::SecretPath(_)));
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_patch_through_symlink_alias_to_secret() {
        use std::os::unix::fs::symlink;

        let dir = unique_temp_dir();
        fs::create_dir_all(&dir).expect("dir");
        let secret = "SECRET=1\n";
        fs::write(dir.join(".env"), secret).expect("secret");
        symlink(".env", dir.join("notes.txt")).expect("symlink");
        let patch = patch_for(FileChange {
            path: "notes.txt".to_string(),
            action: PatchAction::CreateOrUpdate,
            base_sha256: Some(sha256_hex(secret.as_bytes())),
            content: Some("changed\n".to_string()),
            hunks: Vec::new(),
        });

        let error = prepare_patch(&patch, &dir).expect_err("resolved secret path");

        assert!(matches!(error, PatchError::SecretPath(_)));
        assert_eq!(
            fs::read_to_string(dir.join(".env")).expect("secret"),
            secret
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(unix)]
    #[test]
    fn preflight_rejects_alias_retargeted_to_secret() {
        use std::os::unix::fs::symlink;

        let dir = unique_temp_dir();
        fs::create_dir_all(&dir).expect("dir");
        fs::write(dir.join("safe.txt"), "before\n").expect("safe");
        fs::write(dir.join(".env"), "SECRET=1\n").expect("secret");
        symlink("safe.txt", dir.join("notes.txt")).expect("symlink");
        let patch = patch_for(FileChange {
            path: "notes.txt".to_string(),
            action: PatchAction::CreateOrUpdate,
            base_sha256: Some(sha256_hex(b"before\n")),
            content: Some("after\n".to_string()),
            hunks: Vec::new(),
        });
        let prepared = prepare_patch(&patch, &dir).expect("prepare safe alias");
        fs::remove_file(dir.join("notes.txt")).expect("remove alias");
        symlink(".env", dir.join("notes.txt")).expect("retarget alias");

        let error = verify_prepared_patch(&prepared, &dir).expect_err("resolved secret path");

        assert!(matches!(error, PatchError::SecretPath(_)));
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(windows)]
    #[test]
    fn rejects_patch_through_junction_alias_to_scoped_credentials() {
        let dir = unique_temp_dir();
        let credential_dir = dir.join(".aws");
        fs::create_dir_all(&credential_dir).expect("credential dir");
        let secret = "SECRET=1\n";
        fs::write(credential_dir.join("credentials"), secret).expect("credentials");
        let junction = dir.join("notes");
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&credential_dir)
            .output()
            .expect("create junction");
        assert!(
            output.status.success(),
            "junction creation failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let patch = patch_for(FileChange {
            path: "notes/credentials".to_string(),
            action: PatchAction::CreateOrUpdate,
            base_sha256: Some(sha256_hex(secret.as_bytes())),
            content: Some("changed\n".to_string()),
            hunks: Vec::new(),
        });

        let error = prepare_patch(&patch, &dir).expect_err("resolved credential path");

        assert!(matches!(error, PatchError::SecretPath(_)));
        assert_eq!(
            fs::read_to_string(credential_dir.join("credentials")).expect("credentials"),
            secret
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rejects_changes_with_both_or_neither_edit_form() {
        let dir = unique_temp_dir();
        fs::create_dir_all(&dir).expect("dir");
        let hunk = PatchHunk {
            old_start: 1,
            old_lines: vec!["before".to_string()],
            new_lines: vec!["after".to_string()],
        };
        for change in [
            FileChange {
                path: "file.txt".to_string(),
                action: PatchAction::CreateOrUpdate,
                base_sha256: None,
                content: Some("after".to_string()),
                hunks: vec![hunk],
            },
            FileChange {
                path: "file.txt".to_string(),
                action: PatchAction::CreateOrUpdate,
                base_sha256: None,
                content: None,
                hunks: Vec::new(),
            },
        ] {
            let error = validate_change(&change, &Workspace::new(&dir).expect("workspace"))
                .expect_err("exactly one edit form is required");
            assert!(matches!(error, PatchError::AmbiguousEdit { .. }));
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn generates_diff_for_changed_file() {
        let diff = diff_for_change("src/main.rs", "fn old() {}\n", "fn new() {}\n");

        assert!(diff.contains("--- a/src/main.rs"));
        assert!(diff.contains("-fn old() {}"));
        assert!(diff.contains("+fn new() {}"));
    }

    #[test]
    fn existing_full_file_replacement_requires_matching_base_hash() {
        let dir = unique_temp_dir();
        fs::create_dir_all(&dir).expect("dir");
        fs::write(dir.join("file.txt"), "before\n").expect("file");
        let mut patch = patch_for(FileChange {
            path: "file.txt".to_string(),
            action: PatchAction::CreateOrUpdate,
            base_sha256: None,
            content: Some("after\n".to_string()),
            hunks: Vec::new(),
        });

        let missing = prepare_patch(&patch, &dir).expect_err("base hash is required");
        assert!(matches!(missing, PatchError::MissingBaseHash(_)));

        patch.changes[0].base_sha256 = Some(sha256_hex(b"different\n"));
        let mismatch = prepare_patch(&patch, &dir).expect_err("wrong base should conflict");
        assert!(matches!(mismatch, PatchError::Conflict { .. }));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn hunk_relocates_across_a_non_overlapping_external_edit() {
        let dir = unique_temp_dir();
        fs::create_dir_all(&dir).expect("dir");
        let base = "alpha\nbeta\ngamma\n";
        fs::write(dir.join("file.txt"), "inserted\nalpha\nbeta\ngamma\n").expect("file");
        let patch = patch_for(FileChange {
            path: "file.txt".to_string(),
            action: PatchAction::CreateOrUpdate,
            base_sha256: Some(sha256_hex(base.as_bytes())),
            content: None,
            hunks: vec![PatchHunk {
                old_start: 2,
                old_lines: vec!["beta".to_string()],
                new_lines: vec!["BETA".to_string()],
            }],
        });

        let prepared = prepare_patch(&patch, &dir).expect("unique hunk relocates");

        assert_eq!(prepared.files[0].content, "inserted\nalpha\nBETA\ngamma\n");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn ambiguous_hunk_context_is_a_conflict() {
        let dir = unique_temp_dir();
        fs::create_dir_all(&dir).expect("dir");
        let base = "one\ntarget\n";
        fs::write(dir.join("file.txt"), "target\none\ntarget\n").expect("file");
        let patch = patch_for(FileChange {
            path: "file.txt".to_string(),
            action: PatchAction::CreateOrUpdate,
            base_sha256: Some(sha256_hex(base.as_bytes())),
            content: None,
            hunks: vec![PatchHunk {
                old_start: 2,
                old_lines: vec!["target".to_string()],
                new_lines: vec!["changed".to_string()],
            }],
        });

        let error = prepare_patch(&patch, &dir).expect_err("ambiguous context should fail");

        assert!(matches!(error, PatchError::Conflict { .. }));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn preflight_detects_changes_made_after_preview() {
        let dir = unique_temp_dir();
        fs::create_dir_all(&dir).expect("dir");
        let base = "before\n";
        fs::write(dir.join("file.txt"), base).expect("file");
        let patch = patch_for(FileChange {
            path: "file.txt".to_string(),
            action: PatchAction::CreateOrUpdate,
            base_sha256: Some(sha256_hex(base.as_bytes())),
            content: None,
            hunks: vec![PatchHunk {
                old_start: 1,
                old_lines: vec!["before".to_string()],
                new_lines: vec!["after".to_string()],
            }],
        });
        let prepared = prepare_patch(&patch, &dir).expect("prepare");
        fs::write(dir.join("file.txt"), "external\n").expect("external edit");

        let error = verify_prepared_patch(&prepared, &dir).expect_err("preflight conflict");

        assert!(matches!(error, PatchError::Conflict { .. }));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn duplicate_resolved_paths_are_rejected() {
        let dir = unique_temp_dir();
        fs::create_dir_all(dir.join("sub")).expect("dir");
        let first = FileChange {
            path: "same.txt".to_string(),
            action: PatchAction::CreateOrUpdate,
            base_sha256: None,
            content: Some("one".to_string()),
            hunks: Vec::new(),
        };
        let mut patch = patch_for(first.clone());
        patch.changes.push(FileChange {
            path: "sub/../same.txt".to_string(),
            ..first
        });

        let error = prepare_patch(&patch, &dir).expect_err("duplicate path should fail");

        assert!(matches!(error, PatchError::DuplicatePath(_)));
        let _ = fs::remove_dir_all(dir);
    }

    fn patch_for(change: FileChange) -> AxiomPatch {
        AxiomPatch {
            summary: "test".to_string(),
            test_command: None,
            changes: vec![change],
        }
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("axiom-coder-patch-test-{nanos}"))
    }
}
