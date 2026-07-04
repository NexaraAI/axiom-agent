use std::{fs, path::Path};

use axiom_core::{AxiomError, Workspace};
use serde::{Deserialize, Serialize};
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
    pub content: String,
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

    let workspace = Workspace::new(workspace_root)?;
    for change in &patch.changes {
        validate_change(change, &workspace)?;
    }

    Ok(())
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

    block_secret_path(&change.path)?;
    let _resolved = workspace.resolve_inside(&change.path)?;
    Ok(())
}

pub fn diff_for_patch(
    patch: &AxiomPatch,
    workspace_root: impl AsRef<Path>,
) -> Result<PatchPreview, PatchError> {
    let workspace = Workspace::new(workspace_root)?;
    let mut diff = String::new();

    for change in &patch.changes {
        validate_change(change, &workspace)?;
        let resolved = workspace.resolve_inside(&change.path)?;
        let old_content = if resolved.exists() {
            fs::read_to_string(&resolved)?
        } else {
            String::new()
        };
        diff.push_str(&diff_for_change(
            &change.path,
            &old_content,
            &change.content,
        ));
    }

    Ok(PatchPreview { diff })
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

fn block_secret_path(path: &str) -> Result<(), PatchError> {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    let file_name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let blocked_name = matches!(
        file_name.as_str(),
        ".env" | "id_rsa" | "id_dsa" | "credentials.json" | "token.json"
    );
    let blocked_env_variant = file_name.starts_with(".env.");
    let blocked_extension = file_name.ends_with(".pem") || file_name.ends_with(".key");
    let blocked_phrase = normalized.contains("private_key") || normalized.contains("private-key");

    if blocked_name || blocked_env_variant || blocked_extension || blocked_phrase {
        Err(PatchError::SecretPath(path.to_string()))
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
    fn rejects_path_outside_workspace() {
        let dir = unique_temp_dir();
        let patch = AxiomPatch {
            summary: "bad".to_string(),
            test_command: None,
            changes: vec![FileChange {
                path: "../outside.txt".to_string(),
                action: PatchAction::CreateOrUpdate,
                content: "no".to_string(),
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
                content: "SECRET=1".to_string(),
            }],
        };

        let error = validate_patch(&patch, &dir).expect_err("secret path");

        assert!(matches!(error, PatchError::SecretPath(_)));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn generates_diff_for_changed_file() {
        let diff = diff_for_change("src/main.rs", "fn old() {}\n", "fn new() {}\n");

        assert!(diff.contains("--- a/src/main.rs"));
        assert!(diff.contains("-fn old() {}"));
        assert!(diff.contains("+fn new() {}"));
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        std::env::temp_dir().join(format!("axiom-coder-patch-test-{nanos}"))
    }
}
