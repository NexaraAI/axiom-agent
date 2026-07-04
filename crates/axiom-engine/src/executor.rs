use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use axiom_core::Workspace;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

use crate::{
    check_manifest_compatibility, current_axiom_version, InstalledSkill, Platform,
    SkillLifecycleState, SkillType, TrustLevel,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolRequest {
    pub skill_id: String,
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkillExecutionContext {
    pub workspace_root: PathBuf,
    pub max_file_read_bytes: u64,
    pub web_timeout_secs: u64,
    pub max_web_response_bytes: usize,
    pub auto_approve_medium_risk: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub skill_id: String,
    pub message: String,
    pub risk_level: String,
}

pub trait SkillApproval {
    fn approve(&mut self, request: &ApprovalRequest) -> bool;
}

#[derive(Debug, Default)]
pub struct AllowAllApprover;

impl SkillApproval for AllowAllApprover {
    fn approve(&mut self, _request: &ApprovalRequest) -> bool {
        true
    }
}

#[derive(Debug, Default)]
pub struct DenyAllApprover;

impl SkillApproval for DenyAllApprover {
    fn approve(&mut self, _request: &ApprovalRequest) -> bool {
        false
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillExecutionResult {
    pub skill_id: String,
    pub output: Value,
}

#[derive(Debug, Error)]
pub enum SkillExecutionError {
    #[error("failed to parse tool request JSON: {0}")]
    ToolRequestJson(#[from] serde_json::Error),
    #[error("no axiom-tool block found")]
    MissingToolBlock,
    #[error("skill is not installed or enabled: {0}")]
    SkillNotInstalled(String),
    #[error("skill is disabled or blocked: {skill_id} (state: {state}, trust: {trust})")]
    SkillBlocked {
        skill_id: String,
        state: SkillLifecycleState,
        trust: TrustLevel,
    },
    #[error("skill is incompatible: {skill_id}: {reason}")]
    SkillIncompatible { skill_id: String, reason: String },
    #[error("skill is not executable in this stage: {0}")]
    SkillNotExecutable(String),
    #[error("unsupported built-in skill: {0}")]
    UnsupportedSkill(String),
    #[error("missing argument `{argument}` for {skill_id}")]
    MissingArgument {
        skill_id: String,
        argument: &'static str,
    },
    #[error("workspace path error: {0}")]
    Workspace(#[from] axiom_core::AxiomError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("blocked secret path: {0}")]
    SecretPath(String),
    #[error("file is too large: {bytes} bytes exceeds limit {limit} bytes")]
    FileTooLarge { bytes: u64, limit: u64 },
    #[error("approval denied: {0}")]
    ApprovalDenied(String),
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error("network request failed: {0}")]
    Network(String),
    #[error("response is too large: {bytes} bytes exceeds limit {limit} bytes")]
    ResponseTooLarge { bytes: usize, limit: usize },
    #[error("safe command failed: {0}")]
    CommandFailed(String),
}

pub fn extract_tool_request(text: &str) -> Result<ToolRequest, SkillExecutionError> {
    let start_marker = "```axiom-tool";
    let start = text
        .find(start_marker)
        .ok_or(SkillExecutionError::MissingToolBlock)?;
    let json_start = start + start_marker.len();
    let after_start = text[json_start..].trim_start();
    let end = after_start
        .find("```")
        .ok_or(SkillExecutionError::MissingToolBlock)?;
    let json_text = after_start[..end].trim();

    Ok(serde_json::from_str(json_text)?)
}

pub async fn execute_installed_tool(
    request: &ToolRequest,
    installed_skills: &[InstalledSkill],
    context: &SkillExecutionContext,
    approval: &mut dyn SkillApproval,
) -> Result<SkillExecutionResult, SkillExecutionError> {
    let skill = installed_skills
        .iter()
        .find(|skill| skill.manifest.id == request.skill_id)
        .ok_or_else(|| SkillExecutionError::SkillNotInstalled(request.skill_id.clone()))?;

    if !skill.record.is_executable() {
        return Err(SkillExecutionError::SkillBlocked {
            skill_id: request.skill_id.clone(),
            state: skill.record.state,
            trust: skill.record.trust_level,
        });
    }

    let compatibility = check_manifest_compatibility(
        &skill.manifest,
        &current_axiom_version(),
        &Platform::current(),
    );
    if !compatibility.compatible {
        return Err(SkillExecutionError::SkillIncompatible {
            skill_id: request.skill_id.clone(),
            reason: compatibility.reason,
        });
    }

    if skill.manifest.skill_type != SkillType::Tool {
        return Err(SkillExecutionError::SkillNotExecutable(
            request.skill_id.clone(),
        ));
    }

    let output = match request.skill_id.as_str() {
        "file.read" => file_read(request, context)?,
        "file.write" => file_write(request, context, approval)?,
        "project.scan" => project_scan(request, context)?,
        "web.fetch" => web_fetch(request, context, approval).await?,
        "git.status" => git_command(request, context, "status")?,
        "git.diff" => git_command(request, context, "diff")?,
        other => return Err(SkillExecutionError::UnsupportedSkill(other.to_string())),
    };

    Ok(SkillExecutionResult {
        skill_id: request.skill_id.clone(),
        output,
    })
}

fn file_read(
    request: &ToolRequest,
    context: &SkillExecutionContext,
) -> Result<Value, SkillExecutionError> {
    let path = string_arg(request, "path")?;
    block_secret_path(&path)?;
    let workspace = Workspace::new(&context.workspace_root)?;
    let resolved = workspace.resolve_inside(&path)?;
    let metadata = fs::metadata(&resolved)?;
    if metadata.len() > context.max_file_read_bytes {
        return Err(SkillExecutionError::FileTooLarge {
            bytes: metadata.len(),
            limit: context.max_file_read_bytes,
        });
    }

    let content = fs::read_to_string(&resolved)?;
    Ok(json!({
        "path": path,
        "content": content,
        "bytes": metadata.len(),
    }))
}

fn file_write(
    request: &ToolRequest,
    context: &SkillExecutionContext,
    approval: &mut dyn SkillApproval,
) -> Result<Value, SkillExecutionError> {
    let path = string_arg(request, "path")?;
    let content = string_arg(request, "content")?;
    block_secret_path(&path)?;
    let workspace = Workspace::new(&context.workspace_root)?;
    let resolved = workspace.resolve_inside(&path)?;
    let created = !resolved.exists();
    let parent_missing = resolved.parent().is_some_and(|parent| !parent.exists());

    if parent_missing {
        require_approval(
            approval,
            "file.write",
            "Create missing parent directories before writing?",
            "medium",
        )?;
    }

    let prompt = if created {
        format!("Create new file `{path}`?")
    } else {
        format!("Overwrite existing file `{path}`?")
    };
    require_approval(approval, "file.write", &prompt, "medium")?;

    if let Some(parent) = resolved.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&resolved, content.as_bytes())?;

    Ok(json!({
        "path": path,
        "bytes_written": content.len(),
        "created": created,
    }))
}

fn project_scan(
    request: &ToolRequest,
    context: &SkillExecutionContext,
) -> Result<Value, SkillExecutionError> {
    let path = optional_string_arg(request, "path").unwrap_or_else(|| ".".to_string());
    let max_depth = optional_u64_arg(request, "max_depth").unwrap_or(4) as usize;
    let workspace = Workspace::new(&context.workspace_root)?;
    let root = workspace.resolve_inside(&path)?;
    let mut files = Vec::new();
    let mut ignored = BTreeSet::new();
    scan_dir(
        &workspace,
        &root,
        &root,
        max_depth,
        0,
        &mut files,
        &mut ignored,
    )?;

    Ok(json!({
        "root": path,
        "files": files,
        "ignored": ignored.into_iter().collect::<Vec<_>>(),
    }))
}

async fn web_fetch(
    request: &ToolRequest,
    context: &SkillExecutionContext,
    approval: &mut dyn SkillApproval,
) -> Result<Value, SkillExecutionError> {
    let url = string_arg(request, "url")?;
    let parsed = reqwest::Url::parse(&url)
        .map_err(|error| SkillExecutionError::InvalidUrl(error.to_string()))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(SkillExecutionError::InvalidUrl(
            "only http and https URLs are allowed".to_string(),
        ));
    }

    if !context.auto_approve_medium_risk {
        require_approval(
            approval,
            "web.fetch",
            &format!("Allow network access to `{url}`?"),
            "medium",
        )?;
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(context.web_timeout_secs))
        .build()
        .map_err(|error| SkillExecutionError::Network(error.to_string()))?;
    let response = client
        .get(parsed)
        .send()
        .await
        .map_err(|error| SkillExecutionError::Network(error.to_string()))?;
    let status = response.status().as_u16();
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = response
        .bytes()
        .await
        .map_err(|error| SkillExecutionError::Network(error.to_string()))?;
    if bytes.len() > context.max_web_response_bytes {
        return Err(SkillExecutionError::ResponseTooLarge {
            bytes: bytes.len(),
            limit: context.max_web_response_bytes,
        });
    }
    let text = String::from_utf8_lossy(&bytes).to_string();

    Ok(json!({
        "url": url,
        "status": status,
        "content_type": content_type,
        "text": text,
    }))
}

fn git_command(
    request: &ToolRequest,
    context: &SkillExecutionContext,
    command_name: &str,
) -> Result<Value, SkillExecutionError> {
    let path = optional_string_arg(request, "path").unwrap_or_else(|| ".".to_string());
    let workspace = Workspace::new(&context.workspace_root)?;
    let resolved = workspace.resolve_inside(&path)?;
    let output = match command_name {
        "status" => Command::new("git")
            .arg("-C")
            .arg(&resolved)
            .arg("status")
            .arg("--short")
            .output()?,
        "diff" => Command::new("git")
            .arg("-C")
            .arg(&resolved)
            .arg("diff")
            .arg("--")
            .output()?,
        _ => unreachable!("unsupported git command"),
    };

    if !output.status.success() {
        return Err(SkillExecutionError::CommandFailed(
            String::from_utf8_lossy(&output.stderr).to_string(),
        ));
    }

    let field = if command_name == "status" {
        "status"
    } else {
        "diff"
    };
    Ok(json!({
        field: String::from_utf8_lossy(&output.stdout).to_string(),
    }))
}

fn scan_dir(
    workspace: &Workspace,
    scan_root: &Path,
    current: &Path,
    max_depth: usize,
    depth: usize,
    files: &mut Vec<String>,
    ignored: &mut BTreeSet<String>,
) -> Result<(), SkillExecutionError> {
    if depth > max_depth {
        return Ok(());
    }

    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.file_type()?.is_dir() {
            if ignored_dir(&name) {
                ignored.insert(name);
                continue;
            }
            let _ = workspace.resolve_inside(&path)?;
            scan_dir(
                workspace,
                scan_root,
                &path,
                max_depth,
                depth + 1,
                files,
                ignored,
            )?;
        } else if let Ok(relative) = path.strip_prefix(scan_root) {
            files.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }

    files.sort();
    Ok(())
}

fn string_arg(request: &ToolRequest, name: &'static str) -> Result<String, SkillExecutionError> {
    request
        .arguments
        .get(name)
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .ok_or_else(|| SkillExecutionError::MissingArgument {
            skill_id: request.skill_id.clone(),
            argument: name,
        })
}

fn optional_string_arg(request: &ToolRequest, name: &str) -> Option<String> {
    request
        .arguments
        .get(name)
        .and_then(Value::as_str)
        .map(ToString::to_string)
}

fn optional_u64_arg(request: &ToolRequest, name: &str) -> Option<u64> {
    request.arguments.get(name).and_then(Value::as_u64)
}

fn require_approval(
    approval: &mut dyn SkillApproval,
    skill_id: &str,
    message: &str,
    risk_level: &str,
) -> Result<(), SkillExecutionError> {
    let request = ApprovalRequest {
        skill_id: skill_id.to_string(),
        message: message.to_string(),
        risk_level: risk_level.to_string(),
    };

    if approval.approve(&request) {
        Ok(())
    } else {
        Err(SkillExecutionError::ApprovalDenied(message.to_string()))
    }
}

fn block_secret_path(path: &str) -> Result<(), SkillExecutionError> {
    let normalized = path.replace('\\', "/").to_ascii_lowercase();
    let file_name = Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let blocked_name = matches!(
        file_name.as_str(),
        ".env" | ".env.local" | ".env.production" | "id_rsa" | "id_dsa" | "id_ecdsa" | "id_ed25519"
    );
    let blocked_extension = file_name.ends_with(".pem") || file_name.ends_with(".key");
    let blocked_phrase = normalized.contains("private_key") || normalized.contains("private-key");

    if blocked_name || blocked_extension || blocked_phrase {
        Err(SkillExecutionError::SecretPath(path.to_string()))
    } else {
        Ok(())
    }
}

fn ignored_dir(name: &str) -> bool {
    matches!(
        name,
        ".git" | "node_modules" | "target" | "dist" | "build" | ".venv" | "__pycache__"
    )
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use semver::Version;

    use crate::{InstalledSkill, InstalledSkillRecord, SkillManifest};

    use super::*;

    #[test]
    fn parses_textual_tool_request_block() {
        let request = extract_tool_request(
            r#"Here is the request:
```axiom-tool
{
  "skill_id": "file.read",
  "arguments": { "path": "README.md" }
}
```
"#,
        )
        .expect("parse request");

        assert_eq!(request.skill_id, "file.read");
        assert_eq!(request.arguments["path"], "README.md");
    }

    #[tokio::test]
    async fn file_read_reads_inside_workspace() {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).expect("root");
        fs::write(root.join("hello.txt"), "hello").expect("write file");
        let request = ToolRequest {
            skill_id: "file.read".to_string(),
            arguments: json!({ "path": "hello.txt" }),
        };
        let mut approval = AllowAllApprover;

        let result = execute_installed_tool(
            &request,
            &[installed_tool("file.read")],
            &context(&root),
            &mut approval,
        )
        .await
        .expect("execute file.read");

        assert_eq!(result.output["content"], "hello");
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn file_read_blocks_secret_paths() {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).expect("root");
        fs::write(root.join(".env"), "SECRET=value").expect("write file");
        let request = ToolRequest {
            skill_id: "file.read".to_string(),
            arguments: json!({ "path": ".env" }),
        };
        let mut approval = AllowAllApprover;

        let error = execute_installed_tool(
            &request,
            &[installed_tool("file.read")],
            &context(&root),
            &mut approval,
        )
        .await
        .expect_err("secret path should fail");

        assert!(matches!(error, SkillExecutionError::SecretPath(_)));
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn file_write_requires_approval() {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).expect("root");
        let request = ToolRequest {
            skill_id: "file.write".to_string(),
            arguments: json!({ "path": "new.txt", "content": "hello" }),
        };
        let mut approval = DenyAllApprover;

        let error = execute_installed_tool(
            &request,
            &[installed_tool("file.write")],
            &context(&root),
            &mut approval,
        )
        .await
        .expect_err("approval should be required");

        assert!(matches!(error, SkillExecutionError::ApprovalDenied(_)));
        assert!(!root.join("new.txt").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn project_scan_ignores_generated_directories() {
        let root = unique_temp_dir();
        fs::create_dir_all(root.join("src")).expect("src");
        fs::create_dir_all(root.join("target")).expect("target");
        fs::write(root.join("src").join("main.rs"), "fn main() {}").expect("write file");
        fs::write(root.join("target").join("artifact"), "ignored").expect("write ignored");
        let request = ToolRequest {
            skill_id: "project.scan".to_string(),
            arguments: json!({ "path": ".", "max_depth": 4 }),
        };
        let mut approval = AllowAllApprover;

        let result = execute_installed_tool(
            &request,
            &[installed_tool("project.scan")],
            &context(&root),
            &mut approval,
        )
        .await
        .expect("execute project.scan");

        assert_eq!(result.output["files"][0], "src/main.rs");
        assert!(result.output["ignored"]
            .as_array()
            .expect("ignored array")
            .iter()
            .any(|value| value == "target"));
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn web_fetch_rejects_non_http_urls() {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).expect("root");
        let request = ToolRequest {
            skill_id: "web.fetch".to_string(),
            arguments: json!({ "url": "file:///etc/passwd" }),
        };
        let mut approval = AllowAllApprover;

        let error = execute_installed_tool(
            &request,
            &[installed_tool("web.fetch")],
            &context(&root),
            &mut approval,
        )
        .await
        .expect_err("file URL should fail");

        assert!(matches!(error, SkillExecutionError::InvalidUrl(_)));
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn disabled_skill_cannot_execute() {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).expect("root");
        let mut skill = installed_tool("file.read");
        skill.record.enabled = false;
        skill.record.state = SkillLifecycleState::Disabled;
        let request = ToolRequest {
            skill_id: "file.read".to_string(),
            arguments: json!({ "path": "hello.txt" }),
        };
        let mut approval = AllowAllApprover;

        let error = execute_installed_tool(&request, &[skill], &context(&root), &mut approval)
            .await
            .expect_err("disabled skill should be blocked");

        assert!(matches!(error, SkillExecutionError::SkillBlocked { .. }));
        let _ = fs::remove_dir_all(root);
    }

    #[tokio::test]
    async fn incompatible_skill_cannot_execute() {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).expect("root");
        let mut skill = installed_tool("file.read");
        skill.manifest.min_axiom_version = Version::new(99, 0, 0);
        let request = ToolRequest {
            skill_id: "file.read".to_string(),
            arguments: json!({ "path": "hello.txt" }),
        };
        let mut approval = AllowAllApprover;

        let error = execute_installed_tool(&request, &[skill], &context(&root), &mut approval)
            .await
            .expect_err("incompatible skill should be blocked");

        assert!(matches!(
            error,
            SkillExecutionError::SkillIncompatible { .. }
        ));
        let _ = fs::remove_dir_all(root);
    }

    fn installed_tool(skill_id: &str) -> InstalledSkill {
        let manifest = SkillManifest::parse_toml(&format!(
            r#"
id = "{skill_id}"
name = "Test Tool"
version = "0.1.0"
description = "Test tool."
category = "test"
skill_type = "tool"
risk_level = "low"
permissions = []
platforms = ["windows", "linux", "macos"]
entrypoint = "builtin:{skill_id}"
author = "Axiom Agent"
license = "MIT"
min_axiom_version = "0.1.0"
"#
        ))
        .expect("manifest parses");

        InstalledSkill {
            record: InstalledSkillRecord {
                id: skill_id.to_string(),
                version: Version::new(0, 1, 0),
                installed_at: "test".to_string(),
                updated_at: None,
                source: "test".to_string(),
                registry_url: None,
                manifest_url: None,
                checksum: None,
                enabled: true,
                state: SkillLifecycleState::Enabled,
                trust_level: TrustLevel::Trusted,
                last_checked_at: None,
                last_update_error: None,
                last_runtime_error: None,
                success_count: 0,
                failure_count: 0,
                last_used_at: None,
                average_latency_ms: None,
            },
            manifest,
        }
    }

    fn context(root: &Path) -> SkillExecutionContext {
        SkillExecutionContext {
            workspace_root: root.to_path_buf(),
            max_file_read_bytes: 2_000_000,
            web_timeout_secs: 5,
            max_web_response_bytes: 1_000_000,
            auto_approve_medium_risk: false,
        }
    }

    fn unique_temp_dir() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "axiom-engine-executor-test-{nanos}-{id}-{:?}",
            std::thread::current().id()
        ))
    }
}
