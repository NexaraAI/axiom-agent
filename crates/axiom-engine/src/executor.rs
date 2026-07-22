use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    net::IpAddr,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

use async_trait::async_trait;
use axiom_core::{
    atomic_write, is_secret_path, run_command_bounded, Workspace, SECRET_GIT_PATHSPEC_EXCLUSIONS,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;

use crate::{
    check_manifest_compatibility, current_axiom_version, InstalledSkill, Permission, Platform,
    PolicyAction, PolicyOutcome, SideEffectAuditSink, SideEffectClass, SideEffectDecision,
    SideEffectPolicy, SideEffectRequest, SkillLifecycleState, SkillType, TrustLevel,
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
    pub web_fetch_https_only: bool,
    pub web_fetch_allowed_hosts: Vec<String>,
    pub web_fetch_denied_hosts: Vec<String>,
    pub web_fetch_use_system_proxy: bool,
    pub auto_approve_medium_risk: bool,
    /// Provider credential environment variables that must never be inherited
    /// by a tool child process.
    pub credential_env_names: Vec<String>,
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

#[async_trait(?Send)]
pub trait SkillExecutor: Send + Sync {
    fn id(&self) -> &'static str;

    fn descriptor(&self) -> ExecutorDescriptor {
        ExecutorDescriptor {
            id: self.id().to_string(),
            input_schema: json!({"type": "object"}),
            output_schema: json!({"type": "object"}),
            permissions: Vec::new(),
            side_effects: Vec::new(),
            deterministic_fixture: json!({}),
        }
    }

    async fn execute(
        &self,
        request: &ToolRequest,
        context: &SkillExecutionContext,
        approval: &mut dyn SkillApproval,
    ) -> Result<Value, SkillExecutionError>;

    /// Execute with an explicit side-effect policy and audit destination.
    ///
    /// The default keeps third-party executor implementations source
    /// compatible. All built-in executors override this method and authorize
    /// their side effects before touching external state.
    async fn execute_with_policy(
        &self,
        request: &ToolRequest,
        context: &SkillExecutionContext,
        approval: &mut dyn SkillApproval,
        _policy: &SideEffectPolicy,
        _audit: &mut dyn SideEffectAuditSink,
    ) -> Result<Value, SkillExecutionError> {
        self.execute(request, context, approval).await
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecutorDescriptor {
    pub id: String,
    pub input_schema: Value,
    pub output_schema: Value,
    pub permissions: Vec<Permission>,
    pub side_effects: Vec<SideEffectClass>,
    pub deterministic_fixture: Value,
}

impl ExecutorDescriptor {
    pub fn is_complete(&self) -> bool {
        !self.id.is_empty()
            && self.input_schema.get("type").is_some()
            && self.output_schema.get("type").is_some()
            && !self.permissions.is_empty()
    }
}

pub struct ExecutorRegistry {
    executors: BTreeMap<&'static str, Box<dyn SkillExecutor>>,
}

impl ExecutorRegistry {
    pub fn with_builtin_executors() -> Self {
        let mut registry = Self {
            executors: BTreeMap::new(),
        };
        registry.register(Box::new(FileReadExecutor));
        registry.register(Box::new(FileWriteExecutor));
        registry.register(Box::new(ProjectScanExecutor));
        registry.register(Box::new(WebFetchExecutor));
        registry.register(Box::new(GitStatusExecutor));
        registry.register(Box::new(GitDiffExecutor));
        registry
    }

    pub fn register(&mut self, executor: Box<dyn SkillExecutor>) {
        self.executors.insert(executor.id(), executor);
    }

    pub fn get(&self, skill_id: &str) -> Option<&dyn SkillExecutor> {
        self.executors.get(skill_id).map(Box::as_ref)
    }

    pub fn supported_skill_ids(&self) -> Vec<&'static str> {
        self.executors.keys().copied().collect()
    }

    pub fn descriptors(&self) -> Vec<ExecutorDescriptor> {
        self.executors
            .values()
            .map(|executor| executor.descriptor())
            .collect()
    }
}

struct FileReadExecutor;
struct FileWriteExecutor;
struct ProjectScanExecutor;
struct WebFetchExecutor;
struct GitStatusExecutor;
struct GitDiffExecutor;

#[async_trait(?Send)]
impl SkillExecutor for FileReadExecutor {
    fn id(&self) -> &'static str {
        "file.read"
    }

    fn descriptor(&self) -> ExecutorDescriptor {
        ExecutorDescriptor {
            id: self.id().to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["path"],
                "additionalProperties": false,
                "properties": {"path": {"type": "string", "minLength": 1}}
            }),
            output_schema: json!({
                "type": "object",
                "required": ["path", "content", "bytes"]
            }),
            permissions: vec![Permission::FileSystemRead],
            side_effects: vec![SideEffectClass::FilesystemRead],
            deterministic_fixture: json!({"path": "README.md"}),
        }
    }

    async fn execute(
        &self,
        request: &ToolRequest,
        context: &SkillExecutionContext,
        approval: &mut dyn SkillApproval,
    ) -> Result<Value, SkillExecutionError> {
        let policy = SideEffectPolicy::backward_compatible(context.auto_approve_medium_risk);
        let mut audit = crate::NoopSideEffectAuditSink;
        self.execute_with_policy(request, context, approval, &policy, &mut audit)
            .await
    }

    async fn execute_with_policy(
        &self,
        request: &ToolRequest,
        context: &SkillExecutionContext,
        approval: &mut dyn SkillApproval,
        policy: &SideEffectPolicy,
        audit: &mut dyn SideEffectAuditSink,
    ) -> Result<Value, SkillExecutionError> {
        let path = string_arg(request, "path")?;
        authorize_side_effect(
            policy,
            audit,
            approval,
            SideEffectRequest::new(
                self.id(),
                "file.read",
                [SideEffectClass::FilesystemRead],
                Some(path),
            ),
        )?;
        file_read(request, context)
    }
}

#[async_trait(?Send)]
impl SkillExecutor for FileWriteExecutor {
    fn id(&self) -> &'static str {
        "file.write"
    }

    fn descriptor(&self) -> ExecutorDescriptor {
        ExecutorDescriptor {
            id: self.id().to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["path", "content"],
                "additionalProperties": false,
                "properties": {
                    "path": {"type": "string", "minLength": 1},
                    "content": {"type": "string"}
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["path", "bytes_written", "created"]
            }),
            permissions: vec![Permission::FileSystemWrite],
            side_effects: vec![SideEffectClass::FilesystemWrite],
            deterministic_fixture: json!({"path": "axiom-fixture.txt", "content": "fixture"}),
        }
    }

    async fn execute(
        &self,
        request: &ToolRequest,
        context: &SkillExecutionContext,
        approval: &mut dyn SkillApproval,
    ) -> Result<Value, SkillExecutionError> {
        let policy = SideEffectPolicy::backward_compatible(context.auto_approve_medium_risk);
        let mut audit = crate::NoopSideEffectAuditSink;
        self.execute_with_policy(request, context, approval, &policy, &mut audit)
            .await
    }

    async fn execute_with_policy(
        &self,
        request: &ToolRequest,
        context: &SkillExecutionContext,
        approval: &mut dyn SkillApproval,
        policy: &SideEffectPolicy,
        audit: &mut dyn SideEffectAuditSink,
    ) -> Result<Value, SkillExecutionError> {
        let path = string_arg(request, "path")?;
        authorize_side_effect(
            policy,
            audit,
            approval,
            SideEffectRequest::new(
                self.id(),
                "file.write",
                [SideEffectClass::FilesystemWrite],
                Some(path),
            ),
        )?;
        file_write(request, context)
    }
}

#[async_trait(?Send)]
impl SkillExecutor for ProjectScanExecutor {
    fn id(&self) -> &'static str {
        "project.scan"
    }

    fn descriptor(&self) -> ExecutorDescriptor {
        ExecutorDescriptor {
            id: self.id().to_string(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "path": {"type": "string"},
                    "max_depth": {"type": "integer", "minimum": 0, "maximum": 32}
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["root", "files", "ignored"]
            }),
            permissions: vec![Permission::ProjectScan, Permission::FileSystemRead],
            side_effects: vec![SideEffectClass::FilesystemRead],
            deterministic_fixture: json!({"path": ".", "max_depth": 2}),
        }
    }

    async fn execute(
        &self,
        request: &ToolRequest,
        context: &SkillExecutionContext,
        approval: &mut dyn SkillApproval,
    ) -> Result<Value, SkillExecutionError> {
        let policy = SideEffectPolicy::backward_compatible(context.auto_approve_medium_risk);
        let mut audit = crate::NoopSideEffectAuditSink;
        self.execute_with_policy(request, context, approval, &policy, &mut audit)
            .await
    }

    async fn execute_with_policy(
        &self,
        request: &ToolRequest,
        context: &SkillExecutionContext,
        approval: &mut dyn SkillApproval,
        policy: &SideEffectPolicy,
        audit: &mut dyn SideEffectAuditSink,
    ) -> Result<Value, SkillExecutionError> {
        let path = optional_string_arg(request, "path").unwrap_or_else(|| ".".to_string());
        authorize_side_effect(
            policy,
            audit,
            approval,
            SideEffectRequest::new(
                self.id(),
                "project.scan",
                [SideEffectClass::FilesystemRead],
                Some(path),
            ),
        )?;
        project_scan(request, context)
    }
}

#[async_trait(?Send)]
impl SkillExecutor for WebFetchExecutor {
    fn id(&self) -> &'static str {
        "web.fetch"
    }

    fn descriptor(&self) -> ExecutorDescriptor {
        ExecutorDescriptor {
            id: self.id().to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["url"],
                "additionalProperties": false,
                "properties": {"url": {"type": "string", "format": "uri"}}
            }),
            output_schema: json!({
                "type": "object",
                "required": ["url", "status", "content_type", "text"]
            }),
            permissions: vec![Permission::Network],
            side_effects: vec![SideEffectClass::Network],
            deterministic_fixture: json!({"url": "https://example.com"}),
        }
    }

    async fn execute(
        &self,
        request: &ToolRequest,
        context: &SkillExecutionContext,
        approval: &mut dyn SkillApproval,
    ) -> Result<Value, SkillExecutionError> {
        let policy = SideEffectPolicy::backward_compatible(context.auto_approve_medium_risk);
        let mut audit = crate::NoopSideEffectAuditSink;
        self.execute_with_policy(request, context, approval, &policy, &mut audit)
            .await
    }

    async fn execute_with_policy(
        &self,
        request: &ToolRequest,
        context: &SkillExecutionContext,
        approval: &mut dyn SkillApproval,
        policy: &SideEffectPolicy,
        audit: &mut dyn SideEffectAuditSink,
    ) -> Result<Value, SkillExecutionError> {
        let target = validated_web_target(request, context)?;
        authorize_side_effect(
            policy,
            audit,
            approval,
            SideEffectRequest::new(
                self.id(),
                "http.get",
                [SideEffectClass::Network],
                Some(target),
            ),
        )?;
        web_fetch(request, context).await
    }
}

#[async_trait(?Send)]
impl SkillExecutor for GitStatusExecutor {
    fn id(&self) -> &'static str {
        "git.status"
    }

    fn descriptor(&self) -> ExecutorDescriptor {
        git_executor_descriptor(self.id(), "status")
    }

    async fn execute(
        &self,
        request: &ToolRequest,
        context: &SkillExecutionContext,
        approval: &mut dyn SkillApproval,
    ) -> Result<Value, SkillExecutionError> {
        let policy = SideEffectPolicy::backward_compatible(context.auto_approve_medium_risk);
        let mut audit = crate::NoopSideEffectAuditSink;
        self.execute_with_policy(request, context, approval, &policy, &mut audit)
            .await
    }

    async fn execute_with_policy(
        &self,
        request: &ToolRequest,
        context: &SkillExecutionContext,
        approval: &mut dyn SkillApproval,
        policy: &SideEffectPolicy,
        audit: &mut dyn SideEffectAuditSink,
    ) -> Result<Value, SkillExecutionError> {
        authorize_side_effect(
            policy,
            audit,
            approval,
            git_side_effect(self.id(), "status"),
        )?;
        git_command(request, context, "status")
    }
}

#[async_trait(?Send)]
impl SkillExecutor for GitDiffExecutor {
    fn id(&self) -> &'static str {
        "git.diff"
    }

    fn descriptor(&self) -> ExecutorDescriptor {
        git_executor_descriptor(self.id(), "diff")
    }

    async fn execute(
        &self,
        request: &ToolRequest,
        context: &SkillExecutionContext,
        approval: &mut dyn SkillApproval,
    ) -> Result<Value, SkillExecutionError> {
        let policy = SideEffectPolicy::backward_compatible(context.auto_approve_medium_risk);
        let mut audit = crate::NoopSideEffectAuditSink;
        self.execute_with_policy(request, context, approval, &policy, &mut audit)
            .await
    }

    async fn execute_with_policy(
        &self,
        request: &ToolRequest,
        context: &SkillExecutionContext,
        approval: &mut dyn SkillApproval,
        policy: &SideEffectPolicy,
        audit: &mut dyn SideEffectAuditSink,
    ) -> Result<Value, SkillExecutionError> {
        authorize_side_effect(policy, audit, approval, git_side_effect(self.id(), "diff"))?;
        git_command(request, context, "diff")
    }
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
    #[error("skill `{skill_id}` has an unavailable dependency: {dependency}")]
    MissingDependency {
        skill_id: String,
        dependency: String,
    },
    #[error("skill dependency cycle detected at: {0}")]
    DependencyCycle(String),
    #[error("unsupported built-in skill: {0}")]
    UnsupportedSkill(String),
    #[error("{skill_id} {direction} schema validation failed: {message}")]
    SchemaValidation {
        skill_id: String,
        direction: &'static str,
        message: String,
    },
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
    #[error("side-effect policy denied: {0}")]
    SideEffectPolicyDenied(Box<SideEffectDecision>),
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    #[error("network target is blocked by the private-address policy: {0}")]
    PrivateNetworkUrl(String),
    #[error("network host is blocked by web.fetch policy: {0}")]
    NetworkHostDenied(String),
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
    let policy = SideEffectPolicy::backward_compatible(context.auto_approve_medium_risk);
    let mut audit = crate::NoopSideEffectAuditSink;
    execute_installed_tool_with_policy(
        request,
        installed_skills,
        context,
        approval,
        &policy,
        &mut audit,
    )
    .await
}

/// Execute an installed tool through a caller-supplied side-effect policy.
///
/// Every built-in executor records exactly one final policy decision before it
/// performs its external side effect. The original [`execute_installed_tool`]
/// API delegates here with its historical approval behavior.
pub async fn execute_installed_tool_with_policy(
    request: &ToolRequest,
    installed_skills: &[InstalledSkill],
    context: &SkillExecutionContext,
    approval: &mut dyn SkillApproval,
    policy: &SideEffectPolicy,
    audit: &mut dyn SideEffectAuditSink,
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
    validate_runtime_dependencies(
        skill,
        installed_skills,
        &mut std::collections::BTreeSet::new(),
    )?;

    let registry = ExecutorRegistry::with_builtin_executors();
    let executor = registry
        .get(&request.skill_id)
        .ok_or_else(|| SkillExecutionError::UnsupportedSkill(request.skill_id.clone()))?;
    let descriptor = executor.descriptor();
    validate_schema_value(&request.arguments, &descriptor.input_schema).map_err(|message| {
        SkillExecutionError::SchemaValidation {
            skill_id: request.skill_id.clone(),
            direction: "input",
            message,
        }
    })?;
    let output = executor
        .execute_with_policy(request, context, approval, policy, audit)
        .await?;
    validate_schema_value(&output, &descriptor.output_schema).map_err(|message| {
        SkillExecutionError::SchemaValidation {
            skill_id: request.skill_id.clone(),
            direction: "output",
            message,
        }
    })?;

    Ok(SkillExecutionResult {
        skill_id: request.skill_id.clone(),
        output,
    })
}

fn validate_runtime_dependencies(
    skill: &InstalledSkill,
    installed_skills: &[InstalledSkill],
    visiting: &mut std::collections::BTreeSet<String>,
) -> Result<(), SkillExecutionError> {
    if !visiting.insert(skill.manifest.id.clone()) {
        return Err(SkillExecutionError::DependencyCycle(
            skill.manifest.id.clone(),
        ));
    }
    for requirement in &skill.manifest.depends_on {
        let dependency = installed_skills.iter().find(|candidate| {
            candidate.manifest.id == *requirement
                || candidate.manifest.provides.contains(requirement)
        });
        let Some(dependency) = dependency else {
            return Err(SkillExecutionError::MissingDependency {
                skill_id: skill.manifest.id.clone(),
                dependency: requirement.clone(),
            });
        };
        let compatibility = check_manifest_compatibility(
            &dependency.manifest,
            &current_axiom_version(),
            &Platform::current(),
        );
        if !dependency.record.is_executable() || !compatibility.compatible {
            return Err(SkillExecutionError::MissingDependency {
                skill_id: skill.manifest.id.clone(),
                dependency: requirement.clone(),
            });
        }
        validate_runtime_dependencies(dependency, installed_skills, visiting)?;
    }
    visiting.remove(&skill.manifest.id);
    Ok(())
}

fn file_read(
    request: &ToolRequest,
    context: &SkillExecutionContext,
) -> Result<Value, SkillExecutionError> {
    let path = string_arg(request, "path")?;
    block_secret_path(&path)?;
    let workspace = Workspace::new(&context.workspace_root)?;
    let resolved = workspace.resolve_inside(&path)?;
    block_secret_path(&resolved)?;
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
) -> Result<Value, SkillExecutionError> {
    let path = string_arg(request, "path")?;
    let content = string_arg(request, "content")?;
    block_secret_path(&path)?;
    let workspace = Workspace::new(&context.workspace_root)?;
    let resolved = workspace.resolve_inside(&path)?;
    block_secret_path(&resolved)?;
    let created = !resolved.exists();

    if let Some(parent) = resolved.parent() {
        fs::create_dir_all(parent)?;
    }
    atomic_write(&resolved, content.as_bytes())?;

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
) -> Result<Value, SkillExecutionError> {
    let url = string_arg(request, "url")?;
    let parsed = validate_web_url(&url, context)?;
    let host = parsed
        .host_str()
        .ok_or_else(|| SkillExecutionError::InvalidUrl("URL host is required".to_string()))?;
    if is_private_network_host(host) {
        return Err(SkillExecutionError::PrivateNetworkUrl(host.to_string()));
    }
    let port = parsed.port_or_known_default().ok_or_else(|| {
        SkillExecutionError::InvalidUrl("URL port could not be determined".to_string())
    })?;
    let resolved_addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| SkillExecutionError::Network(format!("DNS resolution failed: {error}")))?
        .collect::<Vec<_>>();
    if resolved_addresses.is_empty() {
        return Err(SkillExecutionError::Network(
            "DNS resolution returned no addresses".to_string(),
        ));
    }
    if resolved_addresses
        .iter()
        .any(|address| is_private_network_address(address.ip()))
    {
        return Err(SkillExecutionError::PrivateNetworkUrl(host.to_string()));
    }

    let mut client_builder = reqwest::Client::builder()
        .timeout(Duration::from_secs(context.web_timeout_secs))
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(host, &resolved_addresses);
    if !context.web_fetch_use_system_proxy {
        client_builder = client_builder.no_proxy();
    }
    let client = client_builder
        .build()
        .map_err(|error| SkillExecutionError::Network(error.to_string()))?;
    let mut response = client
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
    if let Some(content_length) = response.content_length() {
        let content_length = usize::try_from(content_length).unwrap_or(usize::MAX);
        if content_length > context.max_web_response_bytes {
            return Err(SkillExecutionError::ResponseTooLarge {
                bytes: content_length,
                limit: context.max_web_response_bytes,
            });
        }
    }

    let mut bytes = Vec::with_capacity(
        response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or_default()
            .min(context.max_web_response_bytes),
    );
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| SkillExecutionError::Network(error.to_string()))?
    {
        append_bounded_response_chunk(&mut bytes, &chunk, context.max_web_response_bytes)?;
    }
    let text = String::from_utf8_lossy(&bytes).to_string();

    Ok(json!({
        "url": url,
        "status": status,
        "content_type": content_type,
        "text": text,
    }))
}

fn append_bounded_response_chunk(
    response: &mut Vec<u8>,
    chunk: &[u8],
    limit: usize,
) -> Result<(), SkillExecutionError> {
    let bytes = response.len().saturating_add(chunk.len());
    if bytes > limit {
        return Err(SkillExecutionError::ResponseTooLarge { bytes, limit });
    }
    response.extend_from_slice(chunk);
    Ok(())
}

fn validated_web_target(
    request: &ToolRequest,
    context: &SkillExecutionContext,
) -> Result<String, SkillExecutionError> {
    let url = string_arg(request, "url")?;
    let mut parsed = validate_web_url(&url, context)?;

    // Queries and fragments commonly carry tokens. They are never required to
    // identify the policy target and therefore never reach the audit stream.
    parsed.set_query(None);
    parsed.set_fragment(None);
    Ok(parsed.to_string())
}

fn validate_web_url(
    url: &str,
    context: &SkillExecutionContext,
) -> Result<reqwest::Url, SkillExecutionError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|error| SkillExecutionError::InvalidUrl(error.to_string()))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err(SkillExecutionError::InvalidUrl(
            "only http and https URLs are allowed".to_string(),
        ));
    }
    if context.web_fetch_https_only && parsed.scheme() != "https" {
        return Err(SkillExecutionError::InvalidUrl(
            "web.fetch requires HTTPS; set network.web_fetch_https_only = false only for an explicitly reviewed target"
                .to_string(),
        ));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(SkillExecutionError::InvalidUrl(
            "URLs with embedded credentials are not allowed".to_string(),
        ));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| SkillExecutionError::InvalidUrl("URL host is required".to_string()))?;
    if is_private_network_host(host) {
        return Err(SkillExecutionError::PrivateNetworkUrl(host.to_string()));
    }
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if context
        .web_fetch_denied_hosts
        .iter()
        .any(|pattern| host_matches_pattern(&host, pattern))
    {
        return Err(SkillExecutionError::NetworkHostDenied(host));
    }
    if !context.web_fetch_allowed_hosts.is_empty()
        && !context
            .web_fetch_allowed_hosts
            .iter()
            .any(|pattern| host_matches_pattern(&host, pattern))
    {
        return Err(SkillExecutionError::NetworkHostDenied(host));
    }
    Ok(parsed)
}

fn host_matches_pattern(host: &str, pattern: &str) -> bool {
    let pattern = pattern.trim_end_matches('.').to_ascii_lowercase();
    if let Some(suffix) = pattern.strip_prefix("*.") {
        host.len() > suffix.len()
            && host.ends_with(suffix)
            && host.as_bytes()[host.len() - suffix.len() - 1] == b'.'
    } else {
        host == pattern
    }
}

fn is_private_network_host(host: &str) -> bool {
    let normalized = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if matches!(normalized.as_str(), "localhost" | "localhost.localdomain")
        || normalized.ends_with(".localhost")
        || normalized.ends_with(".local")
    {
        return true;
    }

    let Ok(address) = normalized.parse::<IpAddr>() else {
        return false;
    };
    is_private_network_address(address)
}

fn is_private_network_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => {
            let [first, second, third, _] = address.octets();
            first == 0
                || first == 10
                || first == 127
                || (first == 100 && (64..=127).contains(&second))
                || (first == 169 && second == 254)
                || (first == 172 && (16..=31).contains(&second))
                || (first == 192 && second == 0 && third <= 2)
                || (first == 192 && second == 168)
                || (first == 198 && (matches!(second, 18 | 19) || (second == 51 && third == 100)))
                || (first == 203 && second == 0 && third == 113)
                || first >= 224
        }
        IpAddr::V6(address) => {
            let segments = address.segments();
            address.is_loopback()
                || address.is_unspecified()
                || address.is_unique_local()
                || address.is_unicast_link_local()
                || address.is_multicast()
                || (segments[0] == 0x2001 && segments[1] == 0x0db8)
                || address
                    .to_ipv4_mapped()
                    .is_some_and(|ipv4| is_private_network_address(IpAddr::V4(ipv4)))
        }
    }
}

fn git_command(
    request: &ToolRequest,
    context: &SkillExecutionContext,
    command_name: &str,
) -> Result<Value, SkillExecutionError> {
    let path = optional_string_arg(request, "path").unwrap_or_else(|| ".".to_string());
    let workspace = Workspace::new(&context.workspace_root)?;
    let resolved = workspace.resolve_inside(&path)?;
    let mut command = hardened_git_command(&resolved, command_name, &context.credential_env_names)?;
    const MAX_GIT_OUTPUT_BYTES: usize = 2 * 1024 * 1024;
    const MAX_GIT_ERROR_BYTES: usize = 64 * 1024;
    let output_limit = usize::try_from(context.max_file_read_bytes)
        .unwrap_or(usize::MAX)
        .min(MAX_GIT_OUTPUT_BYTES);
    let output = run_command_bounded(&mut command, output_limit, MAX_GIT_ERROR_BYTES)?;

    if !output.status.success() {
        return Err(SkillExecutionError::CommandFailed(retained_child_text(
            &output.stderr,
            output.stderr_truncated,
            "git stderr",
        )));
    }

    let field = if command_name == "status" {
        "status"
    } else {
        "diff"
    };
    Ok(json!({
        field: retained_child_text(&output.stdout, output.stdout_truncated, "git output"),
    }))
}

fn retained_child_text(bytes: &[u8], truncated: bool, label: &str) -> String {
    let mut text = String::from_utf8_lossy(bytes).to_string();
    if truncated {
        text.push_str(&format!("\n...[{label} truncated]"));
    }
    text
}

fn hardened_git_command(
    resolved: &Path,
    command_name: &str,
    credential_env_names: &[String],
) -> Result<Command, SkillExecutionError> {
    let mut command = Command::new("git");
    command
        .arg("--no-pager")
        .arg("-c")
        .arg("core.fsmonitor=false")
        .arg("-C")
        .arg(resolved);
    match command_name {
        "status" => {
            command.arg("status").arg("--short").arg("--").arg(".");
        }
        "diff" => {
            command
                .arg("diff")
                .arg("--no-ext-diff")
                .arg("--no-textconv")
                .arg("--")
                .arg(".");
        }
        _ => {
            return Err(SkillExecutionError::UnsupportedSkill(format!(
                "git.{command_name}"
            )))
        }
    }
    for environment_variable in credential_env_names {
        command.env_remove(environment_variable);
    }
    command.args(SECRET_GIT_PATHSPEC_EXCLUSIONS);
    Ok(command)
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

fn git_side_effect(skill_id: &str, operation: &str) -> SideEffectRequest {
    SideEffectRequest::new(
        skill_id,
        format!("git.{operation}"),
        [SideEffectClass::Process, SideEffectClass::Git],
        Some(".".to_string()),
    )
}

fn git_executor_descriptor(skill_id: &str, operation: &str) -> ExecutorDescriptor {
    ExecutorDescriptor {
        id: skill_id.to_string(),
        input_schema: json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "path": {"type": "string", "minLength": 1}
            }
        }),
        output_schema: json!({
            "type": "object",
            "required": [operation],
            "additionalProperties": false,
            "properties": {
                (operation): {"type": "string"}
            }
        }),
        permissions: vec![Permission::ShellRun],
        side_effects: vec![SideEffectClass::Process, SideEffectClass::Git],
        deterministic_fixture: json!({}),
    }
}

fn validate_schema_value(value: &Value, schema: &Value) -> std::result::Result<(), String> {
    if let Some(expected) = schema.get("type").and_then(Value::as_str) {
        let matches = match expected {
            "object" => value.is_object(),
            "array" => value.is_array(),
            "string" => value.is_string(),
            "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
            "number" => value.is_number(),
            "boolean" => value.is_boolean(),
            "null" => value.is_null(),
            other => return Err(format!("unsupported schema type `{other}`")),
        };
        if !matches {
            return Err(format!("expected {expected}"));
        }
    }

    let Some(object) = value.as_object() else {
        return Ok(());
    };
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for key in required.iter().filter_map(Value::as_str) {
            if !object.contains_key(key) {
                return Err(format!("missing required property `{key}`"));
            }
        }
    }
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if schema.get("additionalProperties").and_then(Value::as_bool) == Some(false) {
        if let Some(key) = object.keys().find(|key| !properties.contains_key(*key)) {
            return Err(format!("unknown property `{key}`"));
        }
    }
    for (key, property_schema) in properties {
        let Some(property) = object.get(&key) else {
            continue;
        };
        validate_schema_value(property, &property_schema)
            .map_err(|message| format!("property `{key}`: {message}"))?;
        if let (Some(value), Some(minimum)) = (
            property.as_str(),
            property_schema.get("minLength").and_then(Value::as_u64),
        ) {
            if value.chars().count() < usize::try_from(minimum).unwrap_or(usize::MAX) {
                return Err(format!("property `{key}` is shorter than {minimum}"));
            }
        }
        if let Some(number) = property.as_u64() {
            if property_schema
                .get("minimum")
                .and_then(Value::as_u64)
                .is_some_and(|minimum| number < minimum)
            {
                return Err(format!("property `{key}` is below minimum"));
            }
            if property_schema
                .get("maximum")
                .and_then(Value::as_u64)
                .is_some_and(|maximum| number > maximum)
            {
                return Err(format!("property `{key}` is above maximum"));
            }
        }
    }
    Ok(())
}

/// Evaluate, resolve, and audit one side-effect request before external work.
///
/// `Allow` and `Deny` never invoke the approver. `Ask` invokes it exactly once,
/// then records the final outcome before returning. Callers that perform side
/// effects outside a [`SkillExecutor`] should use this boundary rather than
/// evaluating [`SideEffectPolicy`] themselves.
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

fn block_secret_path(path: impl AsRef<Path>) -> Result<(), SkillExecutionError> {
    let path = path.as_ref();
    if is_secret_path(path) {
        Err(SkillExecutionError::SecretPath(path.display().to_string()))
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

    use crate::{
        InstalledSkill, InstalledSkillRecord, RecordingSideEffectAuditSink, SkillManifest,
    };

    use super::*;

    #[derive(Default)]
    struct CountingApprover {
        calls: usize,
        result: bool,
    }

    impl SkillApproval for CountingApprover {
        fn approve(&mut self, _request: &ApprovalRequest) -> bool {
            self.calls += 1;
            self.result
        }
    }

    #[test]
    fn direct_authorization_is_fail_closed_and_prompts_only_for_ask() {
        let request = || {
            SideEffectRequest::new(
                "coder.test",
                "test.run",
                [SideEffectClass::Process],
                Some("cargo test".to_string()),
            )
        };

        let mut allow_approver = CountingApprover::default();
        let mut allow_audit = RecordingSideEffectAuditSink::default();
        authorize_side_effect(
            &SideEffectPolicy::allow_all(),
            &mut allow_audit,
            &mut allow_approver,
            request(),
        )
        .expect("allow policy");
        assert_eq!(allow_approver.calls, 0);
        assert_eq!(allow_audit.decisions()[0].outcome, PolicyOutcome::Allowed);

        let mut deny_approver = CountingApprover {
            result: true,
            ..CountingApprover::default()
        };
        let mut deny_audit = RecordingSideEffectAuditSink::default();
        let denied = authorize_side_effect(
            &SideEffectPolicy::deny_all(),
            &mut deny_audit,
            &mut deny_approver,
            request(),
        )
        .expect_err("deny policy");
        assert!(matches!(
            denied,
            SkillExecutionError::SideEffectPolicyDenied(_)
        ));
        assert_eq!(deny_approver.calls, 0);
        assert_eq!(deny_audit.decisions()[0].outcome, PolicyOutcome::Denied);

        let mut ask_approver = CountingApprover {
            result: true,
            ..CountingApprover::default()
        };
        let mut ask_audit = RecordingSideEffectAuditSink::default();
        authorize_side_effect(
            &SideEffectPolicy::default(),
            &mut ask_audit,
            &mut ask_approver,
            request(),
        )
        .expect("approved ask policy");
        assert_eq!(ask_approver.calls, 1);
        assert_eq!(ask_audit.decisions()[0].outcome, PolicyOutcome::Allowed);

        let mut declined_approver = CountingApprover::default();
        let mut declined_audit = RecordingSideEffectAuditSink::default();
        let declined = authorize_side_effect(
            &SideEffectPolicy::default(),
            &mut declined_audit,
            &mut declined_approver,
            request(),
        )
        .expect_err("declined ask policy");
        assert!(matches!(declined, SkillExecutionError::ApprovalDenied(_)));
        assert_eq!(declined_approver.calls, 1);
        assert_eq!(declined_audit.decisions()[0].outcome, PolicyOutcome::Denied);
    }

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

    #[test]
    fn malformed_tool_request_corpus_never_panics() {
        let mut state = 0xa076_1d64_78bd_642f_u64;
        for length in 0..512 {
            let mut input = String::with_capacity(length + 32);
            if length % 2 == 0 {
                input.push_str("```axiom-tool\n");
            }
            for _ in 0..length {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                input.push(char::from((state as u8) & 0x7f));
            }
            if length % 7 == 0 {
                input.push_str("\n```");
            }
            let _ = extract_tool_request(&input);
        }
    }

    #[test]
    fn builtin_executor_registry_exposes_the_supported_tool_ids() {
        let registry = ExecutorRegistry::with_builtin_executors();

        assert_eq!(
            registry.supported_skill_ids(),
            vec![
                "file.read",
                "file.write",
                "git.diff",
                "git.status",
                "project.scan",
                "web.fetch",
            ]
        );
    }

    #[test]
    fn every_builtin_executor_has_complete_schema_policy_and_fixture_metadata() {
        let descriptors = ExecutorRegistry::with_builtin_executors().descriptors();
        assert_eq!(descriptors.len(), 6);
        for descriptor in descriptors {
            assert!(descriptor.is_complete(), "incomplete: {}", descriptor.id);
            assert!(descriptor.input_schema.is_object());
            assert!(descriptor.output_schema.is_object());
            assert!(!descriptor.side_effects.is_empty());
            assert!(descriptor.deterministic_fixture.is_object());
            validate_schema_value(&descriptor.deterministic_fixture, &descriptor.input_schema)
                .unwrap_or_else(|error| panic!("invalid fixture for {}: {error}", descriptor.id));
        }
    }

    #[test]
    fn git_descriptors_match_their_runtime_result_shapes() {
        let registry = ExecutorRegistry::with_builtin_executors();
        let status = registry.get("git.status").unwrap().descriptor();
        let diff = registry.get("git.diff").unwrap().descriptor();

        validate_schema_value(&json!({"status": " M README.md"}), &status.output_schema)
            .expect("git.status result should match its schema");
        validate_schema_value(&json!({"diff": "diff --git"}), &diff.output_schema)
            .expect("git.diff result should match its schema");
        assert!(validate_schema_value(&json!({"diff": "wrong"}), &status.output_schema).is_err());
    }

    #[test]
    fn git_diff_disables_external_drivers_and_scrubs_credentials() {
        let key = "AXIOM_TEST_ENGINE_GIT_SECRET_A81A0E66".to_string();
        let command = hardened_git_command(Path::new("."), "diff", std::slice::from_ref(&key))
            .expect("supported git command");
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();

        assert!(arguments.iter().any(|argument| argument == "--no-ext-diff"));
        assert!(arguments.iter().any(|argument| argument == "--no-textconv"));
        assert!(arguments
            .windows(2)
            .any(|pair| pair == ["-c", "core.fsmonitor=false"]));
        assert!(arguments
            .iter()
            .any(|argument| argument == ":(exclude,icase,glob)**/.env*"));
        assert!(command
            .get_envs()
            .any(|(name, value)| name == key.as_str() && value.is_none()));
    }

    #[test]
    fn git_diff_excludes_secret_files_before_their_contents_are_captured() {
        if Command::new("git").arg("--version").output().is_err() {
            return;
        }
        let root = unique_temp_dir();
        fs::create_dir_all(&root).expect("root");
        run_git(&root, &["init"]);
        fs::write(root.join("safe.txt"), "before\n").expect("safe fixture");
        fs::create_dir_all(root.join(".ENV")).expect("secret directory fixture");
        fs::write(root.join(".ENV").join("nested.txt"), "SECRET=before\n")
            .expect("nested secret fixture");
        fs::write(root.join("SIGNING.PEM"), "KEY=before\n").expect("key fixture");
        run_git(&root, &["add", "--", "."]);
        run_git(
            &root,
            &[
                "-c",
                "user.name=Axiom Test",
                "-c",
                "user.email=axiom@example.invalid",
                "commit",
                "-m",
                "fixture",
            ],
        );
        fs::write(root.join("safe.txt"), "after-safe\n").expect("safe change");
        fs::write(
            root.join(".ENV").join("nested.txt"),
            "SECRET=must-not-leak\n",
        )
        .expect("nested secret change");
        fs::write(root.join("SIGNING.PEM"), "KEY=must-not-leak\n").expect("key change");
        let request = ToolRequest {
            skill_id: "git.diff".to_string(),
            arguments: json!({"path": "."}),
        };

        let result = git_command(&request, &context(&root), "diff").expect("safe git diff");
        let diff = result["diff"].as_str().expect("diff text");

        assert!(diff.contains("after-safe"));
        assert!(!diff.contains("must-not-leak"));
        assert!(!diff.contains("SECRET="));
        assert!(!diff.contains("KEY="));
        let _ = fs::remove_dir_all(root);
    }

    fn run_git(root: &Path, arguments: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(arguments)
            .output()
            .expect("run git fixture command");
        assert!(
            output.status.success(),
            "git fixture command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
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
    async fn file_write_blocks_common_credential_paths() {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).expect("root");
        let request = ToolRequest {
            skill_id: "file.write".to_string(),
            arguments: json!({ "path": "credentials.json", "content": "secret" }),
        };
        let mut approval = AllowAllApprover;

        let error = execute_installed_tool(
            &request,
            &[installed_tool("file.write")],
            &context(&root),
            &mut approval,
        )
        .await
        .expect_err("credential path should fail");

        assert!(matches!(error, SkillExecutionError::SecretPath(_)));
        assert!(!root.join("credentials.json").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn file_tools_block_symlink_alias_to_secret() {
        use std::os::unix::fs::symlink;

        let root = unique_temp_dir();
        fs::create_dir_all(&root).expect("root");
        fs::write(root.join(".env"), "SECRET=value").expect("secret");
        symlink(".env", root.join("notes.txt")).expect("symlink");
        let mut approval = AllowAllApprover;

        for request in [
            ToolRequest {
                skill_id: "file.read".to_string(),
                arguments: json!({ "path": "notes.txt" }),
            },
            ToolRequest {
                skill_id: "file.write".to_string(),
                arguments: json!({ "path": "notes.txt", "content": "changed" }),
            },
        ] {
            let error = execute_installed_tool(
                &request,
                &[installed_tool(&request.skill_id)],
                &context(&root),
                &mut approval,
            )
            .await
            .expect_err("resolved secret path should fail");
            assert!(matches!(error, SkillExecutionError::SecretPath(_)));
        }

        assert_eq!(
            fs::read_to_string(root.join(".env")).expect("secret"),
            "SECRET=value"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn file_tools_block_junction_alias_to_scoped_credentials() {
        let root = unique_temp_dir();
        let credential_dir = root.join(".aws");
        fs::create_dir_all(&credential_dir).expect("credential dir");
        fs::write(credential_dir.join("credentials"), "SECRET=value").expect("credentials");
        let junction = root.join("notes");
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
        let mut approval = AllowAllApprover;

        for request in [
            ToolRequest {
                skill_id: "file.read".to_string(),
                arguments: json!({ "path": "notes/credentials" }),
            },
            ToolRequest {
                skill_id: "file.write".to_string(),
                arguments: json!({ "path": "notes/credentials", "content": "changed" }),
            },
        ] {
            let error = execute_installed_tool(
                &request,
                &[installed_tool(&request.skill_id)],
                &context(&root),
                &mut approval,
            )
            .await
            .expect_err("resolved credential path should fail");
            assert!(matches!(error, SkillExecutionError::SecretPath(_)));
        }

        assert_eq!(
            fs::read_to_string(credential_dir.join("credentials")).expect("credentials"),
            "SECRET=value"
        );
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
    async fn web_fetch_blocks_private_network_targets_before_any_request() {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).expect("root");
        for url in [
            "https://localhost:8080",
            "https://127.0.0.1:8080",
            "https://10.0.0.1",
            "https://[::1]",
            "https://service.local",
        ] {
            let request = ToolRequest {
                skill_id: "web.fetch".to_string(),
                arguments: json!({ "url": url }),
            };
            let mut approval = AllowAllApprover;
            let error = execute_installed_tool(
                &request,
                &[installed_tool("web.fetch")],
                &context(&root),
                &mut approval,
            )
            .await
            .expect_err("private target must be blocked");

            assert!(matches!(error, SkillExecutionError::PrivateNetworkUrl(_)));
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn web_fetch_requires_https_by_default() {
        let root = unique_temp_dir();
        let request = ToolRequest {
            skill_id: "web.fetch".to_string(),
            arguments: json!({"url": "http://example.com/docs"}),
        };

        let error = validated_web_target(&request, &context(&root))
            .expect_err("plain HTTP should be denied by the default context");

        assert!(matches!(error, SkillExecutionError::InvalidUrl(_)));
    }

    #[test]
    fn web_fetch_host_policy_is_deny_first_and_supports_subdomain_patterns() {
        let root = unique_temp_dir();
        let mut context = context(&root);
        context.web_fetch_allowed_hosts = vec!["*.example.com".to_string()];
        context.web_fetch_denied_hosts = vec!["blocked.example.com".to_string()];

        let allowed = ToolRequest {
            skill_id: "web.fetch".to_string(),
            arguments: json!({"url": "https://docs.example.com/guide?token=secret"}),
        };
        let denied = ToolRequest {
            skill_id: "web.fetch".to_string(),
            arguments: json!({"url": "https://blocked.example.com/"}),
        };
        let outside = ToolRequest {
            skill_id: "web.fetch".to_string(),
            arguments: json!({"url": "https://example.net/"}),
        };

        assert_eq!(
            validated_web_target(&allowed, &context).expect("allowed host"),
            "https://docs.example.com/guide"
        );
        assert!(matches!(
            validated_web_target(&denied, &context),
            Err(SkillExecutionError::NetworkHostDenied(_))
        ));
        assert!(matches!(
            validated_web_target(&outside, &context),
            Err(SkillExecutionError::NetworkHostDenied(_))
        ));
    }

    #[test]
    fn private_address_policy_covers_reserved_and_mapped_ranges() {
        for address in [
            "100.64.0.1",
            "192.0.2.1",
            "198.51.100.2",
            "203.0.113.4",
            "2001:db8::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(is_private_network_address(
                address.parse().expect("test address")
            ));
        }
        assert!(!is_private_network_address(
            "8.8.8.8".parse().expect("public address")
        ));
        assert!(is_private_network_host("service.localhost"));
    }

    #[test]
    fn bounded_web_response_stops_before_oversized_chunk_is_buffered() {
        let mut response = b"first".to_vec();
        append_bounded_response_chunk(&mut response, b"-ok", 8).expect("within limit");
        let error = append_bounded_response_chunk(&mut response, b"!", 8)
            .expect_err("oversized response must fail");

        assert!(matches!(
            error,
            SkillExecutionError::ResponseTooLarge { bytes: 9, limit: 8 }
        ));
        assert_eq!(response, b"first-ok");
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

    #[tokio::test]
    async fn missing_or_cyclic_dependencies_block_execution() {
        let root = unique_temp_dir();
        fs::create_dir_all(&root).expect("root");
        fs::write(root.join("hello.txt"), "hello").expect("file");
        let mut skill = installed_tool("file.read");
        skill.manifest.depends_on = vec!["missing.skill".to_string()];
        let request = ToolRequest {
            skill_id: "file.read".to_string(),
            arguments: json!({ "path": "hello.txt" }),
        };
        let mut approval = AllowAllApprover;

        let missing =
            execute_installed_tool(&request, &[skill.clone()], &context(&root), &mut approval)
                .await
                .expect_err("missing dependency should block");
        assert!(matches!(
            missing,
            SkillExecutionError::MissingDependency { .. }
        ));

        skill.manifest.depends_on = vec!["git.status".to_string()];
        let mut dependency = installed_tool("git.status");
        dependency.manifest.depends_on = vec!["file.read".to_string()];
        let cycle = execute_installed_tool(
            &request,
            &[skill, dependency],
            &context(&root),
            &mut approval,
        )
        .await
        .expect_err("dependency cycle should block");
        assert!(matches!(cycle, SkillExecutionError::DependencyCycle(_)));
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
            web_fetch_https_only: true,
            web_fetch_allowed_hosts: Vec::new(),
            web_fetch_denied_hosts: Vec::new(),
            web_fetch_use_system_proxy: false,
            auto_approve_medium_risk: false,
            credential_env_names: Vec::new(),
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
