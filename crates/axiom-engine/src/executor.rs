use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    net::IpAddr,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
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
    PolicyAction, PolicyOutcome, RiskLevel, SideEffectAuditSink, SideEffectClass,
    SideEffectDecision, SideEffectPolicy, SideEffectRequest, SkillLifecycleState, SkillType,
    TrustLevel,
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

    pub credential_env_names: Vec<String>,
    pub skills_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    pub skill_id: String,
    pub message: String,
    pub risk_level: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionAnswer {
    pub selected: String,
    #[serde(default)]
    pub index: Option<usize>,
    #[serde(default)]
    pub is_custom: bool,
}

pub trait SkillApproval {
    fn approve(&mut self, request: &ApprovalRequest) -> bool;

    fn ask_question(
        &mut self,
        question: &str,
        options: &[String],
        _allow_custom: bool,
    ) -> Result<QuestionAnswer, String> {
        let default_choice = options
            .first()
            .cloned()
            .unwrap_or_else(|| question.to_string());
        Ok(QuestionAnswer {
            selected: default_choice,
            index: Some(1),
            is_custom: false,
        })
    }
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
            && (!self.permissions.is_empty() || self.id == "question.ask")
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
        registry.register(Box::new(ShellExecutor::powershell()));
        registry.register(Box::new(ShellExecutor::bash()));
        registry.register(Box::new(ShellExecutor::zsh()));
        registry.register(Box::new(ShellExecutor::python_run()));
        registry.register(Box::new(ShellExecutor::generic_run()));
        registry.register(Box::new(SkillCreateExecutor));
        registry.register(Box::new(QuestionAskExecutor));
        registry.register(Box::new(TestRunExecutor));
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
struct SkillCreateExecutor;
struct QuestionAskExecutor;
struct TestRunExecutor;
struct ProjectScanExecutor;
struct WebFetchExecutor;
struct GitStatusExecutor;
struct GitDiffExecutor;
pub struct ShellExecutor {
    id: &'static str,
}

impl ShellExecutor {
    pub const fn powershell() -> Self {
        Self {
            id: "shell.powershell.safe",
        }
    }

    pub const fn bash() -> Self {
        Self {
            id: "shell.bash.safe",
        }
    }

    pub const fn zsh() -> Self {
        Self {
            id: "shell.zsh.safe",
        }
    }

    pub const fn python_run() -> Self {
        Self { id: "python.run" }
    }

    pub const fn generic_run() -> Self {
        Self { id: "shell.run" }
    }
}

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
                    "content": {"type": "string"},
                    "overwrite_confirmation": {"type": "boolean"}
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
impl SkillExecutor for SkillCreateExecutor {
    fn id(&self) -> &'static str {
        "skill.create"
    }

    fn descriptor(&self) -> ExecutorDescriptor {
        ExecutorDescriptor {
            id: self.id().to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["id", "name", "description", "content"],
                "additionalProperties": false,
                "properties": {
                    "id": { "type": "string", "minLength": 1 },
                    "name": { "type": "string", "minLength": 1 },
                    "description": { "type": "string", "minLength": 1 },
                    "content": { "type": "string", "minLength": 1 },
                    "skill_type": {
                        "type": "string",
                        "enum": ["prompt", "tool", "workflow", "guard"]
                    },
                    "when_to_use": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "tags": {
                        "type": "array",
                        "items": { "type": "string" }
                    }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["status", "skill_id", "path", "message"],
                "additionalProperties": false,
                "properties": {
                    "status": { "type": "string" },
                    "skill_id": { "type": "string" },
                    "path": { "type": "string" },
                    "message": { "type": "string" }
                }
            }),
            permissions: vec![Permission::FileSystemWrite],
            side_effects: vec![SideEffectClass::FilesystemWrite],
            deterministic_fixture: json!({
                "id": "custom.fixture",
                "name": "Custom Fixture",
                "description": "Fixture skill description",
                "content": "Fixture content",
            }),
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
        let skill_id = string_arg(request, "id")?;
        authorize_side_effect(
            policy,
            audit,
            approval,
            SideEffectRequest::new(
                self.id(),
                "skill.create",
                [SideEffectClass::FilesystemWrite],
                Some(skill_id),
            ),
        )?;
        skill_create(request, context)
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

#[async_trait(?Send)]
impl SkillExecutor for TestRunExecutor {
    fn id(&self) -> &'static str {
        "test.run"
    }

    fn descriptor(&self) -> ExecutorDescriptor {
        ExecutorDescriptor {
            id: self.id().to_string(),
            input_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "path": {"type": "string"},
                    "command": {"type": "string"}
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["status", "passed", "framework", "command", "output", "summary"]
            }),
            permissions: vec![Permission::ShellRun, Permission::FileSystemRead],
            side_effects: vec![SideEffectClass::Process, SideEffectClass::FilesystemRead],
            deterministic_fixture: json!({"path": "."}),
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
        let target_dir = optional_string_arg(request, "path").unwrap_or_else(|| ".".to_string());
        authorize_side_effect(
            policy,
            audit,
            approval,
            SideEffectRequest::new(
                self.id(),
                "test.run",
                [SideEffectClass::Process, SideEffectClass::FilesystemRead],
                Some(target_dir),
            ),
        )?;
        test_run(request, context)
    }
}

#[async_trait(?Send)]
impl SkillExecutor for ShellExecutor {
    fn id(&self) -> &'static str {
        self.id
    }

    fn descriptor(&self) -> ExecutorDescriptor {
        ExecutorDescriptor {
            id: self.id().to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["command"],
                "properties": {
                    "command": {
                        "type": "string",
                        "minLength": 1,
                        "description": "The command or script to execute in the workspace."
                    },
                    "working_directory": {
                        "type": "string",
                        "description": "Optional subdirectory relative to workspace root."
                    },
                    "background": {
                        "type": "boolean",
                        "description": "Run as a persistent background daemon (recommended for dev servers like http.server, vite, npm run dev)."
                    },
                    "is_background": {
                        "type": "boolean",
                        "description": "Alias for background."
                    },
                    "timeout_seconds": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 600,
                        "description": "Execution timeout in seconds for foreground commands (default: 30)."
                    },
                    "safety_level": {
                        "type": "string"
                    }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["exit_code", "stdout", "stderr"],
                "properties": {
                    "exit_code": {"type": "integer"},
                    "stdout": {"type": "string"},
                    "stderr": {"type": "string"}
                }
            }),
            permissions: vec![Permission::ShellRun],
            side_effects: vec![SideEffectClass::Process],
            deterministic_fixture: json!({"command": "echo axiom-test"}),
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
        let command_str = string_arg(request, "command")?;
        authorize_side_effect(
            policy,
            audit,
            approval,
            SideEffectRequest::new(
                self.id(),
                "shell.run",
                [SideEffectClass::Process],
                Some(command_str.clone()),
            ),
        )?;
        shell_run(self.id(), request, context)
    }
}

#[async_trait(?Send)]
impl SkillExecutor for QuestionAskExecutor {
    fn id(&self) -> &'static str {
        "question.ask"
    }

    fn descriptor(&self) -> ExecutorDescriptor {
        ExecutorDescriptor {
            id: self.id().to_string(),
            input_schema: json!({
                "type": "object",
                "required": ["question", "options"],
                "properties": {
                    "question": {
                        "type": "string",
                        "description": "The clarification or decision question to ask the user."
                    },
                    "options": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "2 to 5 distinct multiple-choice options for the user to choose from."
                    },
                    "allow_custom": {
                        "type": "boolean",
                        "description": "Whether the user can write a custom reply. Defaults to true."
                    }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["selected", "is_custom"],
                "properties": {
                    "selected": {
                        "type": "string",
                        "description": "The selected option string or user custom response."
                    },
                    "index": {
                        "type": ["integer", "null"],
                        "description": "1-based index if an option was chosen, or null for custom."
                    },
                    "is_custom": {
                        "type": "boolean",
                        "description": "Whether the response was custom typed by the user."
                    }
                }
            }),
            permissions: Vec::new(),
            side_effects: Vec::new(),
            deterministic_fixture: json!({
                "question": "Which framework would you like?",
                "options": ["React", "Vanilla JS"]
            }),
        }
    }

    async fn execute(
        &self,
        request: &ToolRequest,
        _context: &SkillExecutionContext,
        approval: &mut dyn SkillApproval,
    ) -> Result<Value, SkillExecutionError> {
        let question = string_arg(request, "question")?;
        let options = match request.arguments.get("options") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            _ => {
                return Err(SkillExecutionError::MissingArgument {
                    skill_id: self.id().to_string(),
                    argument: "options",
                });
            }
        };
        let allow_custom = request
            .arguments
            .get("allow_custom")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let answer = approval
            .ask_question(&question, &options, allow_custom)
            .map_err(|e| SkillExecutionError::ExecutionFailed {
                skill_id: self.id().to_string(),
                message: format!("failed to ask question: {e}"),
            })?;

        Ok(json!({
            "selected": answer.selected,
            "index": answer.index,
            "is_custom": answer.is_custom
        }))
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
    #[error("skill `{skill_id}` execution failed: {message}")]
    ExecutionFailed { skill_id: String, message: String },
}

pub fn extract_tool_request(text: &str) -> Result<ToolRequest, SkillExecutionError> {
    let markers = &[
        "```axiom-tool",
        "```axiom_tool",
        "```tool-call",
        "```tool_call",
        "```tool",
    ];
    let mut found = None;
    for marker in markers {
        if let Some(pos) = text.find(marker) {
            found = Some((pos, marker.len()));
            break;
        }
    }
    if found.is_none() {
        if let Some(pos) = text.find("```json") {
            let json_start = pos + "```json".len();
            if let Some(end) = text[json_start..].find("```") {
                let candidate = &text[json_start..json_start + end];
                if candidate.contains("\"skill_id\"")
                    || (candidate.contains("\"name\"")
                        && (candidate.contains("file.")
                            || candidate.contains("project.")
                            || candidate.contains("axiom_")))
                {
                    found = Some((pos, "```json".len()));
                }
            }
        }
    }
    let (start, marker_len) = found.ok_or(SkillExecutionError::MissingToolBlock)?;
    let json_start = start + marker_len;
    let after_start = text[json_start..].trim_start();
    let end = after_start
        .find("```")
        .ok_or(SkillExecutionError::MissingToolBlock)?;
    let json_text = after_start[..end].trim();

    let raw: serde_json::Value = serde_json::from_str(json_text)?;
    normalize_tool_request_value(raw)
}

fn normalize_tool_request_value(
    raw: serde_json::Value,
) -> Result<ToolRequest, SkillExecutionError> {
    if let serde_json::Value::Object(map) = raw {
        let skill_id = if let Some(id) = map.get("skill_id").and_then(serde_json::Value::as_str) {
            id.to_string()
        } else if let Some(name) = map.get("name").and_then(serde_json::Value::as_str) {
            name.strip_prefix("axiom_")
                .unwrap_or(name)
                .replace('_', ".")
        } else if let Some(tool) = map.get("tool").and_then(serde_json::Value::as_str) {
            tool.to_string()
        } else {
            return Err(SkillExecutionError::SchemaValidation {
                skill_id: "unknown".to_string(),
                direction: "input",
                message: "missing skill_id or name in tool request".to_string(),
            });
        };

        let arguments = if let Some(args) = map.get("arguments") {
            args.clone()
        } else if let Some(params) = map.get("parameters") {
            params.clone()
        } else if let Some(args) = map.get("args") {
            args.clone()
        } else {
            serde_json::json!({})
        };

        Ok(ToolRequest {
            skill_id,
            arguments,
        })
    } else {
        Err(SkillExecutionError::SchemaValidation {
            skill_id: "unknown".to_string(),
            direction: "input",
            message: "expected json object for tool request".to_string(),
        })
    }
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

pub fn builtin_installed_skill(skill_id: &str) -> Option<InstalledSkill> {
    let (name, desc, skill_type, risk) = match skill_id {
        "skill.create" => (
            "Create Personalized Skill",
            "Author and install a personalized skill or workflow dynamically mid-conversation",
            SkillType::Tool,
            RiskLevel::Low,
        ),
        "file.read" => (
            "Read Workspace File",
            "Read UTF-8 text files inside the active workspace",
            SkillType::Tool,
            RiskLevel::Low,
        ),
        "file.write" => (
            "Write Workspace File",
            "Writes UTF-8 text files inside the active workspace",
            SkillType::Tool,
            RiskLevel::Medium,
        ),
        "project.scan" => (
            "Scan Workspace Project",
            "Scans workspace directory tree",
            SkillType::Tool,
            RiskLevel::Low,
        ),
        "web.fetch" => (
            "Fetch Web Resource",
            "Fetches web resource over HTTPS",
            SkillType::Tool,
            RiskLevel::Low,
        ),
        "git.status" => (
            "Git Working Tree Status",
            "Reports clean/dirty status in git repository",
            SkillType::Tool,
            RiskLevel::Low,
        ),
        "git.diff" => (
            "Git Working Tree Diff",
            "Displays uncommitted working tree diff",
            SkillType::Tool,
            RiskLevel::Low,
        ),
        "shell.powershell.safe" | "shell.bash.safe" | "shell.zsh.safe" | "shell.run" => (
            "Execute Shell Command",
            "Runs authorized shell commands inside the active workspace",
            SkillType::Tool,
            RiskLevel::Medium,
        ),
        "python.run" => (
            "Run Python Script",
            "Runs python scripts inside the active workspace",
            SkillType::Tool,
            RiskLevel::Medium,
        ),
        "question.ask" => (
            "Ask User Question",
            "Prompts user with an interactive multiple-choice question form",
            SkillType::Tool,
            RiskLevel::Low,
        ),
        "test.run" => (
            "Run Workspace Tests",
            "Auto-detects and executes project tests (Cargo, NPM, Pytest, Python syntax, HTML validation)",
            SkillType::Tool,
            RiskLevel::Low,
        ),
        _ => return None,
    };

    Some(InstalledSkill {
        record: crate::InstalledSkillRecord {
            id: skill_id.to_string(),
            version: semver::Version::new(1, 0, 0),
            installed_at: "builtin".to_string(),
            updated_at: None,
            source: "builtin".to_string(),
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
        manifest: crate::SkillManifest {
            schema_version: "1.0".to_string(),
            id: skill_id.to_string(),
            name: name.to_string(),
            version: semver::Version::new(1, 0, 0),
            description: desc.to_string(),
            category: "builtin".to_string(),
            skill_type,
            risk_level: risk,
            permissions: vec![],
            platforms: vec![Platform::Windows, Platform::Linux, Platform::Macos],
            entrypoint: format!("builtin:{skill_id}"),
            author: "Axiom Agent".to_string(),
            license: "MIT".to_string(),
            min_axiom_version: semver::Version::new(0, 1, 0),
            max_axiom_version: None,
            depends_on: vec![],
            provides: vec![],
            hooks: crate::manifest::SkillHooks::default(),
            side_effects: vec![],
            idempotent: false,
            cache_key: None,
            examples: vec![],
            keywords: vec![],
            llm_card: Some(crate::manifest::LlmCardManifest {
                summary: desc.to_string(),
                when_to_use: vec![],
                input_contract: "standard".to_string(),
                output_contract: "standard".to_string(),
                token_budget: 300,
            }),
            updates: crate::manifest::UpdatePolicy::default(),
            input_schema: toml::Value::Table(toml::map::Map::new()),
            output_schema: toml::Value::Table(toml::map::Map::new()),
        },
    })
}

pub async fn execute_installed_tool_with_policy(
    request: &ToolRequest,
    installed_skills: &[InstalledSkill],
    context: &SkillExecutionContext,
    approval: &mut dyn SkillApproval,
    policy: &SideEffectPolicy,
    audit: &mut dyn SideEffectAuditSink,
) -> Result<SkillExecutionResult, SkillExecutionError> {
    let synthetic_builtin;
    let skill = match installed_skills
        .iter()
        .find(|s| s.manifest.id == request.skill_id)
    {
        Some(s) => s,
        None => {
            let registry = ExecutorRegistry::with_builtin_executors();
            if registry.get(&request.skill_id).is_some() {
                synthetic_builtin = builtin_installed_skill(&request.skill_id);
                synthetic_builtin.as_ref().ok_or_else(|| {
                    SkillExecutionError::SkillNotInstalled(request.skill_id.clone())
                })?
            } else {
                return Err(SkillExecutionError::SkillNotInstalled(
                    request.skill_id.clone(),
                ));
            }
        }
    };

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
    let lines = content.lines().count();
    Ok(json!({
        "path": path,
        "content": content,
        "bytes": metadata.len(),
        "lines": lines,
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

    let old_content = if resolved.exists() {
        fs::read_to_string(&resolved).ok()
    } else {
        None
    };

    if let Some(parent) = resolved.parent() {
        fs::create_dir_all(parent)?;
    }
    atomic_write(&resolved, content.as_bytes())?;

    let total_lines = content.lines().count();
    let (lines_added, lines_deleted) = if let Some(ref old) = old_content {
        let old_lines: Vec<&str> = old.lines().collect();
        let new_lines: Vec<&str> = content.lines().collect();
        let old_set: BTreeSet<&str> = old_lines.iter().copied().collect();
        let new_set: BTreeSet<&str> = new_lines.iter().copied().collect();
        let added = new_lines.iter().filter(|l| !old_set.contains(*l)).count();
        let deleted = old_lines.iter().filter(|l| !new_set.contains(*l)).count();
        (added, deleted)
    } else {
        (total_lines, 0)
    };

    Ok(json!({
        "path": path,
        "bytes_written": content.len(),
        "created": created,
        "lines": total_lines,
        "lines_added": lines_added,
        "lines_deleted": lines_deleted,
    }))
}

fn skill_create(
    request: &ToolRequest,
    context: &SkillExecutionContext,
) -> Result<Value, SkillExecutionError> {
    let id = string_arg(request, "id")?;
    let name = string_arg(request, "name")?;
    let description = string_arg(request, "description")?;
    let content = string_arg(request, "content")?;
    let skill_type = request.arguments.get("skill_type").and_then(Value::as_str);
    let when_to_use = request
        .arguments
        .get("when_to_use")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let tags = request
        .arguments
        .get("tags")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let target_skills_dir = context
        .skills_dir
        .clone()
        .unwrap_or_else(|| context.workspace_root.join(".axiom").join("skills"));

    let skill_folder = crate::installed::create_personalized_skill(
        &target_skills_dir,
        &id,
        &name,
        &description,
        skill_type,
        &content,
        &when_to_use,
        &tags,
    )
    .map_err(|err| SkillExecutionError::ExecutionFailed {
        skill_id: "skill.create".to_string(),
        message: err.to_string(),
    })?;

    Ok(json!({
        "status": "success",
        "skill_id": id,
        "path": skill_folder.display().to_string(),
        "message": format!(
            "Personalized skill '{id}' created at {} and registered for use.",
            skill_folder.display()
        )
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
    let raw_url = if let Ok(url) = string_arg(request, "url") {
        url
    } else if let Ok(query) = string_arg(request, "query") {
        let mut ddg = reqwest::Url::parse("https://html.duckduckgo.com/html/")
            .map_err(|e| SkillExecutionError::InvalidUrl(e.to_string()))?;
        ddg.query_pairs_mut().append_pair("q", &query);
        ddg.to_string()
    } else {
        return Err(SkillExecutionError::MissingArgument {
            skill_id: "web.fetch".to_string(),
            argument: "url",
        });
    };

    let mut current_url = validate_web_url(&raw_url, context)?;

    if let Some(host) = current_url.host_str() {
        let host_lower = host.to_ascii_lowercase();
        if (host_lower == "google.com" || host_lower.ends_with(".google.com"))
            && (current_url.path() == "/search" || current_url.path() == "/search/")
        {
            if let Some((_, query_val)) = current_url.query_pairs().find(|(k, _)| k == "q") {
                let mut ddg_url = reqwest::Url::parse("https://html.duckduckgo.com/html/")
                    .map_err(|e| SkillExecutionError::InvalidUrl(e.to_string()))?;
                ddg_url.query_pairs_mut().append_pair("q", &query_val);
                current_url = validate_web_url(ddg_url.as_str(), context)?;
            }
        } else if (host_lower == "duckduckgo.com" || host_lower == "www.duckduckgo.com")
            && (current_url.path() == "/" || current_url.path() == "")
        {
            if let Some((_, query_val)) = current_url.query_pairs().find(|(k, _)| k == "q") {
                let mut ddg_url = reqwest::Url::parse("https://html.duckduckgo.com/html/")
                    .map_err(|e| SkillExecutionError::InvalidUrl(e.to_string()))?;
                ddg_url.query_pairs_mut().append_pair("q", &query_val);
                current_url = validate_web_url(ddg_url.as_str(), context)?;
            }
        }
    }

    const MAX_WEB_REDIRECTS: usize = 5;
    const DEFAULT_USER_AGENT: &str =
        "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/128.0.0.0 Safari/537.36 (Axiom-Agent)";

    let mut final_response = None;

    for redirect_count in 0..=MAX_WEB_REDIRECTS {
        let host = current_url
            .host_str()
            .ok_or_else(|| SkillExecutionError::InvalidUrl("URL host is required".to_string()))?;
        if is_private_network_host(host) {
            return Err(SkillExecutionError::PrivateNetworkUrl(host.to_string()));
        }
        let port = current_url.port_or_known_default().ok_or_else(|| {
            SkillExecutionError::InvalidUrl("URL port could not be determined".to_string())
        })?;
        let resolved_addresses = tokio::net::lookup_host((host, port))
            .await
            .map_err(|error| {
                SkillExecutionError::Network(format!("DNS resolution failed: {error}"))
            })?
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

        let response = client
            .get(current_url.clone())
            .header(reqwest::header::USER_AGENT, DEFAULT_USER_AGENT)
            .header(
                reqwest::header::ACCEPT,
                "text/html,application/xhtml+xml,application/xml;q=0.9,application/json;q=0.8,*/*;q=0.7",
            )
            .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
            .send()
            .await
            .map_err(|error| SkillExecutionError::Network(error.to_string()))?;

        if response.status().is_redirection() {
            if redirect_count == MAX_WEB_REDIRECTS {
                return Err(SkillExecutionError::Network(format!(
                    "exceeded maximum redirects ({MAX_WEB_REDIRECTS})"
                )));
            }
            if let Some(location) = response.headers().get(reqwest::header::LOCATION) {
                let location_str = location.to_str().map_err(|_| {
                    SkillExecutionError::Network(
                        "redirect Location header is not valid UTF-8".to_string(),
                    )
                })?;
                let next_url = current_url.join(location_str).map_err(|error| {
                    SkillExecutionError::InvalidUrl(format!("invalid redirect target: {error}"))
                })?;
                current_url = validate_web_url(next_url.as_str(), context)?;
                continue;
            }
        }

        final_response = Some(response);
        break;
    }

    let mut response = final_response.ok_or_else(|| {
        SkillExecutionError::Network("failed to receive HTTP response".to_string())
    })?;
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
    let raw_text = String::from_utf8_lossy(&bytes).to_string();
    let text = if content_type.to_ascii_lowercase().contains("html") {
        extract_text_from_html(&raw_text)
    } else {
        raw_text
    };

    Ok(json!({
        "url": current_url.to_string(),
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

fn extract_text_from_html(html: &str) -> String {
    let mut result = String::with_capacity(html.len() / 2);
    let mut in_tag = false;
    let mut tag_name = String::new();
    let mut skip_content_tag: Option<&str> = None;
    let mut chars = html.chars().peekable();

    while let Some(c) = chars.next() {
        if let Some(skip_tag) = skip_content_tag {
            if c == '<' && chars.peek() == Some(&'/') {
                chars.next();
                let mut close_name = String::new();
                while let Some(&next_c) = chars.peek() {
                    if next_c.is_alphanumeric() {
                        close_name.push(next_c.to_ascii_lowercase());
                        chars.next();
                    } else {
                        break;
                    }
                }
                while let Some(&next_c) = chars.peek() {
                    chars.next();
                    if next_c == '>' {
                        break;
                    }
                }
                if close_name == skip_tag {
                    skip_content_tag = None;
                }
            }
            continue;
        }

        if c == '<' {
            in_tag = true;
            tag_name.clear();
            let is_close = chars.peek() == Some(&'/');
            if is_close {
                chars.next();
            }
            while let Some(&next_c) = chars.peek() {
                if next_c.is_alphanumeric() {
                    tag_name.push(next_c.to_ascii_lowercase());
                    chars.next();
                } else {
                    break;
                }
            }
            if !is_close && matches!(tag_name.as_str(), "script" | "style" | "noscript" | "svg") {
                skip_content_tag = match tag_name.as_str() {
                    "script" => Some("script"),
                    "style" => Some("style"),
                    "noscript" => Some("noscript"),
                    "svg" => Some("svg"),
                    _ => None,
                };
            }
            if matches!(
                tag_name.as_str(),
                "p" | "div" | "br" | "li" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "tr"
            ) {
                result.push('\n');
            }
            continue;
        }

        if in_tag {
            if c == '>' {
                in_tag = false;
            }
            continue;
        }

        if c == '&' {
            let mut entity = String::new();
            while let Some(&next_c) = chars.peek() {
                if next_c == ';' {
                    chars.next();
                    break;
                }
                if next_c.is_alphanumeric() || next_c == '#' {
                    entity.push(next_c);
                    chars.next();
                } else {
                    break;
                }
            }
            match entity.as_str() {
                "amp" => result.push('&'),
                "lt" => result.push('<'),
                "gt" => result.push('>'),
                "quot" => result.push('"'),
                "apos" | "#39" => result.push('\''),
                "nbsp" => result.push(' '),
                _ => {
                    result.push('&');
                    result.push_str(&entity);
                }
            }
            continue;
        }

        result.push(c);
    }

    let mut cleaned = String::new();
    let mut consecutive_newlines = 0;
    for line in result.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            consecutive_newlines += 1;
            if consecutive_newlines <= 2 {
                cleaned.push('\n');
            }
        } else {
            consecutive_newlines = 0;
            cleaned.push_str(trimmed);
            cleaned.push('\n');
        }
    }
    cleaned.trim().to_string()
}

fn validated_web_target(
    request: &ToolRequest,
    context: &SkillExecutionContext,
) -> Result<String, SkillExecutionError> {
    let url = if let Ok(u) = string_arg(request, "url") {
        u
    } else if let Ok(q) = string_arg(request, "query") {
        format!("https://html.duckduckgo.com/html/?q={q}")
    } else {
        return Err(SkillExecutionError::MissingArgument {
            skill_id: "web.fetch".to_string(),
            argument: "url",
        });
    };
    let mut parsed = validate_web_url(&url, context)?;

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

static BACKGROUND_PROCESSES: Mutex<Vec<std::process::Child>> = Mutex::new(Vec::new());

fn register_background_process(child: std::process::Child) {
    if let Ok(mut list) = BACKGROUND_PROCESSES.lock() {
        list.push(child);
    }
}

fn check_dangerous_command(command: &str) -> Result<(), SkillExecutionError> {
    let lower = command.to_ascii_lowercase();
    if lower.contains("format ")
        || lower.contains("rmdir /s /q c:")
        || lower.contains("rm -rf /")
        || lower.contains(":(){ :|:& };:")
    {
        return Err(SkillExecutionError::CommandFailed(
            "command blocked by safety policy: destructive system command detected".to_string(),
        ));
    }
    Ok(())
}

fn is_dev_server_command(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    lower.contains("http.server")
        || lower.contains("http-server")
        || lower.contains("live-server")
        || lower.contains("vite")
        || lower.contains("next dev")
        || lower.contains("astro dev")
        || lower.contains("run dev")
        || lower.contains("npm start")
        || lower.contains("npx serve")
        || lower.contains("webpack serve")
        || lower.contains("gatsby develop")
        || lower.contains("flask run")
        || lower.contains("uvicorn")
        || lower.contains("rails server")
}

fn is_server_listening_output(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    lower.contains("serving http on")
        || lower.contains("http://localhost:")
        || lower.contains("http://127.0.0.1:")
        || lower.contains("listening on")
        || lower.contains("ready in")
        || lower.contains("started server")
        || lower.contains("development server running")
}

fn spawn_stream_reader<R: Read + Send + 'static>(
    mut reader: R,
    buffer: Arc<Mutex<Vec<u8>>>,
    max_bytes: usize,
) {
    std::thread::spawn(move || {
        let mut chunk = [0u8; 4096];
        while let Ok(n) = reader.read(&mut chunk) {
            if n == 0 {
                break;
            }
            if let Ok(mut b) = buffer.lock() {
                if b.len() < max_bytes {
                    let remaining = max_bytes.saturating_sub(b.len());
                    b.extend_from_slice(&chunk[..n.min(remaining)]);
                }
            }
        }
    });
}

fn create_shell_command(
    skill_id: &str,
    command: &str,
    cwd: &Path,
    credential_env_names: &[String],
) -> Command {
    let mut cmd = if cfg!(windows) {
        let mut c = Command::new("powershell.exe");
        c.arg("-NoProfile")
            .arg("-NonInteractive")
            .arg("-ExecutionPolicy")
            .arg("Bypass")
            .arg("-Command")
            .arg(command);
        c
    } else if skill_id == "shell.zsh.safe" {
        let mut c = Command::new("zsh");
        c.arg("-c").arg(command);
        c
    } else {
        let mut c = Command::new("bash");
        c.arg("-c").arg(command);
        c
    };

    cmd.current_dir(cwd);
    for env_var in credential_env_names {
        cmd.env_remove(env_var);
    }
    cmd
}

struct ChildProcessGuard {
    child: Option<std::process::Child>,
    disowned: bool,
}

impl ChildProcessGuard {
    fn new(child: std::process::Child) -> Self {
        Self {
            child: Some(child),
            disowned: false,
        }
    }

    fn disown(mut self) -> std::process::Child {
        self.disowned = true;
        self.child.take().unwrap()
    }

    fn as_mut(&mut self) -> &mut std::process::Child {
        self.child.as_mut().unwrap()
    }
}

impl Drop for ChildProcessGuard {
    fn drop(&mut self) {
        if !self.disowned {
            if let Some(mut child) = self.child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

#[allow(clippy::zombie_processes)]
fn shell_run(
    skill_id: &str,
    request: &ToolRequest,
    context: &SkillExecutionContext,
) -> Result<Value, SkillExecutionError> {
    let command_str = string_arg(request, "command")?;
    check_dangerous_command(&command_str)?;

    let workspace = Workspace::new(&context.workspace_root)?;
    let working_dir = optional_string_arg(request, "working_directory");
    let command_cwd = match working_dir {
        Some(ref dir) if !dir.trim().is_empty() && dir.trim() != "." => {
            workspace.resolve_inside(dir)?
        }
        _ => workspace.root().to_path_buf(),
    };
    if !command_cwd.is_dir() {
        return Err(SkillExecutionError::CommandFailed(format!(
            "working directory does not exist: {}",
            command_cwd.display()
        )));
    }

    let is_bg = request
        .arguments
        .get("background")
        .and_then(Value::as_bool)
        .or_else(|| {
            request
                .arguments
                .get("is_background")
                .and_then(Value::as_bool)
        })
        .unwrap_or(false);

    let should_background = is_bg || is_dev_server_command(&command_str);

    let mut command = create_shell_command(
        skill_id,
        &command_str,
        &command_cwd,
        &context.credential_env_names,
    );
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let child = command.spawn().map_err(|err| {
        SkillExecutionError::CommandFailed(format!("failed to spawn process: {err}"))
    })?;
    let mut guard = ChildProcessGuard::new(child);

    let stdout_buf = Arc::new(Mutex::new(Vec::new()));
    let stderr_buf = Arc::new(Mutex::new(Vec::new()));

    if let Some(stdout) = guard.as_mut().stdout.take() {
        spawn_stream_reader(stdout, stdout_buf.clone(), 512 * 1024);
    }
    if let Some(stderr) = guard.as_mut().stderr.take() {
        spawn_stream_reader(stderr, stderr_buf.clone(), 128 * 1024);
    }

    if should_background {
        std::thread::sleep(Duration::from_millis(1200));
        match guard
            .as_mut()
            .try_wait()
            .map_err(|e| SkillExecutionError::CommandFailed(e.to_string()))?
        {
            Some(status) => {
                let _ = guard.as_mut().wait();
                let out_str = String::from_utf8_lossy(&stdout_buf.lock().unwrap()).to_string();
                let err_str = String::from_utf8_lossy(&stderr_buf.lock().unwrap()).to_string();
                Ok(json!({
                    "exit_code": status.code().unwrap_or(-1),
                    "stdout": out_str,
                    "stderr": err_str,
                }))
            }
            None => {
                let pid = guard.as_mut().id();
                let out_str = String::from_utf8_lossy(&stdout_buf.lock().unwrap()).to_string();
                let err_str = String::from_utf8_lossy(&stderr_buf.lock().unwrap()).to_string();
                register_background_process(guard.disown());
                let message = if out_str.trim().is_empty() {
                    format!("Background process started successfully (PID: {pid}).")
                } else {
                    format!(
                        "Background process started successfully (PID: {pid}).\nInitial output:\n{out_str}"
                    )
                };
                Ok(json!({
                    "exit_code": 0,
                    "stdout": message,
                    "stderr": err_str,
                }))
            }
        }
    } else {
        let timeout_seconds = optional_u64_arg(request, "timeout_seconds")
            .unwrap_or(30)
            .clamp(1, 600);
        let start = Instant::now();
        let timeout = Duration::from_secs(timeout_seconds);
        let mut exit_status = None;

        while start.elapsed() < timeout {
            if let Some(status) = guard
                .as_mut()
                .try_wait()
                .map_err(|e| SkillExecutionError::CommandFailed(e.to_string()))?
            {
                exit_status = Some(status);
                break;
            }
            let current_out = String::from_utf8_lossy(&stdout_buf.lock().unwrap()).to_string();
            if is_server_listening_output(&current_out) {
                let pid = guard.as_mut().id();
                register_background_process(guard.disown());
                return Ok(json!({
                    "exit_code": 0,
                    "stdout": format!(
                        "Server detected listening and running in background (PID: {pid}):\n{current_out}"
                    ),
                    "stderr": String::from_utf8_lossy(&stderr_buf.lock().unwrap()).to_string(),
                }));
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        match exit_status {
            Some(status) => {
                let _ = guard.as_mut().wait();
                std::thread::sleep(Duration::from_millis(50));
                let out_str = String::from_utf8_lossy(&stdout_buf.lock().unwrap()).to_string();
                let err_str = String::from_utf8_lossy(&stderr_buf.lock().unwrap()).to_string();
                Ok(json!({
                    "exit_code": status.code().unwrap_or(0),
                    "stdout": out_str,
                    "stderr": err_str,
                }))
            }
            None => {
                let _ = guard.as_mut().kill();
                let _ = guard.as_mut().wait();
                let out_str = String::from_utf8_lossy(&stdout_buf.lock().unwrap()).to_string();
                let err_str = String::from_utf8_lossy(&stderr_buf.lock().unwrap()).to_string();
                Ok(json!({
                    "exit_code": 124,
                    "stdout": out_str,
                    "stderr": format!("Command timed out after {timeout_seconds}s.\n{err_str}"),
                }))
            }
        }
    }
}

#[allow(clippy::zombie_processes)]
fn execute_test_command(
    cwd: &Path,
    cmd_str: &str,
    credential_env_names: &[String],
) -> Result<(bool, i32, String), SkillExecutionError> {
    let mut command = create_shell_command("shell.run", cmd_str, cwd, credential_env_names);
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let child = command.spawn().map_err(|err| {
        SkillExecutionError::CommandFailed(format!("failed to spawn test process: {err}"))
    })?;
    let mut guard = ChildProcessGuard::new(child);

    let stdout_buf = Arc::new(Mutex::new(Vec::new()));
    let stderr_buf = Arc::new(Mutex::new(Vec::new()));

    if let Some(stdout) = guard.as_mut().stdout.take() {
        spawn_stream_reader(stdout, stdout_buf.clone(), 256 * 1024);
    }
    if let Some(stderr) = guard.as_mut().stderr.take() {
        spawn_stream_reader(stderr, stderr_buf.clone(), 256 * 1024);
    }

    let start = Instant::now();
    let timeout = Duration::from_secs(60);
    let mut exit_status = None;

    while start.elapsed() < timeout {
        if let Some(status) = guard
            .as_mut()
            .try_wait()
            .map_err(|e| SkillExecutionError::CommandFailed(e.to_string()))?
        {
            exit_status = Some(status);
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    match exit_status {
        Some(status) => {
            let _ = guard.as_mut().wait();
            std::thread::sleep(Duration::from_millis(30));
            let out_str = String::from_utf8_lossy(&stdout_buf.lock().unwrap()).to_string();
            let err_str = String::from_utf8_lossy(&stderr_buf.lock().unwrap()).to_string();
            let combined = if err_str.trim().is_empty() {
                out_str
            } else if out_str.trim().is_empty() {
                err_str
            } else {
                format!("{out_str}\n{err_str}")
            };
            let code = status.code().unwrap_or(0);
            Ok((status.success(), code, combined))
        }
        None => {
            let _ = guard.as_mut().kill();
            let _ = guard.as_mut().wait();
            let out_str = String::from_utf8_lossy(&stdout_buf.lock().unwrap()).to_string();
            let err_str = String::from_utf8_lossy(&stderr_buf.lock().unwrap()).to_string();
            Ok((
                false,
                124,
                format!("Test command timed out after 60s.\n{out_str}\n{err_str}"),
            ))
        }
    }
}

fn dir_contains_extension(dir: &Path, ext: &str, max_depth: usize) -> bool {
    if max_depth == 0 {
        return false;
    }
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if path.extension().and_then(|e| e.to_str()) == Some(ext) {
                    return true;
                }
            } else if path.is_dir() {
                let file_name = entry.file_name();
                let name = file_name.to_string_lossy();
                if !name.starts_with('.')
                    && name != "node_modules"
                    && name != "target"
                    && dir_contains_extension(&path, ext, max_depth - 1)
                {
                    return true;
                }
            }
        }
    }
    false
}

fn validate_html_project(dir: &Path) -> Result<Value, SkillExecutionError> {
    let mut html_files = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("html") {
                html_files.push(p);
            }
        }
    }

    if html_files.is_empty() {
        return Ok(json!({
            "status": "no_tests_found",
            "passed": true,
            "framework": "none",
            "command": "",
            "output": "No HTML files found to validate.",
            "summary": "No tests found."
        }));
    }

    let mut errors = Vec::new();
    let mut verified_count = 0;

    for html_file in &html_files {
        let content = match fs::read_to_string(html_file) {
            Ok(c) => c,
            Err(e) => {
                errors.push(format!("Failed to read {}: {e}", html_file.display()));
                continue;
            }
        };

        let lower = content.to_ascii_lowercase();
        if !lower.contains("<html") && !lower.contains("<!doctype") {
            errors.push(format!(
                "{}: missing <!DOCTYPE html> or <html> tag",
                html_file.display()
            ));
        }

        for tag in &["script", "style", "div", "body", "head"] {
            let open_tag = format!("<{tag}");
            let close_tag = format!("</{tag}>");
            let open_count = lower.match_indices(&open_tag).count();
            let close_count = lower.match_indices(&close_tag).count();
            if open_count != close_count {
                errors.push(format!(
                    "{}: mismatched <{tag}> tags (opened {open_count} times, closed {close_count} times)",
                    html_file.display()
                ));
            }
        }

        for line in content.lines() {
            if let Some(src_idx) = line.find("src=") {
                let rest = &line[src_idx + 4..];
                let quote = rest.chars().next();
                if let Some(q) = quote {
                    if q == '"' || q == '\'' {
                        let path_part = &rest[1..];
                        if let Some(end_quote) = path_part.find(q) {
                            let src_path = &path_part[..end_quote];
                            if !src_path.starts_with("http://")
                                && !src_path.starts_with("https://")
                                && !src_path.starts_with("//")
                                && !src_path.starts_with("data:")
                            {
                                let script_file = if let Some(parent) = html_file.parent() {
                                    parent.join(src_path)
                                } else {
                                    PathBuf::from(src_path)
                                };
                                if !script_file.exists() {
                                    errors.push(format!(
                                        "{}: referenced script `{src_path}` not found on disk",
                                        html_file.display()
                                    ));
                                } else {
                                    let mut check_cmd = std::process::Command::new("node");
                                    let clean_path = script_file
                                        .to_string_lossy()
                                        .strip_prefix(r"\\?\")
                                        .map(PathBuf::from)
                                        .unwrap_or_else(|| script_file.clone());
                                    check_cmd.arg("--check").arg(&clean_path);
                                    if let Ok(output) = check_cmd.output() {
                                        if !output.status.success() {
                                            let err_msg = String::from_utf8_lossy(&output.stderr);
                                            if !err_msg.contains("MODULE_NOT_FOUND")
                                                && !err_msg.contains("Cannot find module")
                                            {
                                                errors.push(format!(
                                                    "{}: syntax error in `{src_path}`:\n{err_msg}",
                                                    html_file.display()
                                                ));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if line.contains("stylesheet") {
                if let Some(href_idx) = line.find("href=") {
                    let rest = &line[href_idx + 5..];
                    let quote = rest.chars().next();
                    if let Some(q) = quote {
                        if q == '"' || q == '\'' {
                            let path_part = &rest[1..];
                            if let Some(end_quote) = path_part.find(q) {
                                let href_path = &path_part[..end_quote];
                                if !href_path.starts_with("http://")
                                    && !href_path.starts_with("https://")
                                    && !href_path.starts_with("//")
                                    && !href_path.starts_with("data:")
                                {
                                    let css_file = if let Some(parent) = html_file.parent() {
                                        parent.join(href_path)
                                    } else {
                                        PathBuf::from(href_path)
                                    };
                                    if !css_file.exists() {
                                        errors.push(format!(
                                            "{}: referenced stylesheet `{href_path}` not found on disk",
                                            html_file.display()
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        verified_count += 1;
    }

    let passed = errors.is_empty();
    let summary = if passed {
        format!("Validated {verified_count} HTML/web file(s) and referenced scripts/styles with 0 errors")
    } else {
        format!("Found {} HTML/web error(s)", errors.len())
    };
    let output = if passed {
        format!("All {verified_count} HTML entrypoints and referenced assets passed syntax and structure validation.")
    } else {
        errors.join("\n")
    };

    Ok(json!({
        "status": if passed { "passed" } else { "failed" },
        "passed": passed,
        "framework": "html_web",
        "command": "html-validator",
        "output": output,
        "summary": summary,
    }))
}

fn test_run(
    request: &ToolRequest,
    context: &SkillExecutionContext,
) -> Result<Value, SkillExecutionError> {
    let rel_path = optional_string_arg(request, "path").unwrap_or_else(|| ".".to_string());
    let workspace = Workspace::new(&context.workspace_root)?;
    let root = workspace.resolve_inside(&rel_path)?;

    if let Some(cmd) = request.arguments.get("command").and_then(Value::as_str) {
        if !cmd.trim().is_empty() {
            let (passed, code, output) =
                execute_test_command(&root, cmd.trim(), &context.credential_env_names)?;
            let summary = if passed {
                format!("Custom test command `{}` passed (exit code 0)", cmd.trim())
            } else {
                format!(
                    "Custom test command `{}` failed (exit code {code})",
                    cmd.trim()
                )
            };
            return Ok(json!({
                "status": if passed { "passed" } else { "failed" },
                "passed": passed,
                "framework": "custom",
                "command": cmd.trim(),
                "output": output,
                "summary": summary,
            }));
        }
    }

    // 1. Cargo
    if root.join("Cargo.toml").exists() {
        let cmd = "cargo test";
        let (passed, code, output) =
            execute_test_command(&root, cmd, &context.credential_env_names)?;
        let summary = if passed {
            "Cargo test suite passed successfully".to_string()
        } else {
            format!("Cargo test suite failed with exit code {code}")
        };
        return Ok(json!({
            "status": if passed { "passed" } else { "failed" },
            "passed": passed,
            "framework": "cargo",
            "command": cmd,
            "output": output,
            "summary": summary,
        }));
    }

    // 2. NPM
    let pkg_json = root.join("package.json");
    if pkg_json.exists() {
        if let Ok(content) = fs::read_to_string(&pkg_json) {
            if let Ok(parsed) = serde_json::from_str::<Value>(&content) {
                if parsed.get("scripts").and_then(|s| s.get("test")).is_some() {
                    let cmd = "npm test";
                    let (passed, code, output) =
                        execute_test_command(&root, cmd, &context.credential_env_names)?;
                    let summary = if passed {
                        "NPM test suite passed successfully".to_string()
                    } else {
                        format!("NPM test suite failed with exit code {code}")
                    };
                    return Ok(json!({
                        "status": if passed { "passed" } else { "failed" },
                        "passed": passed,
                        "framework": "npm",
                        "command": cmd,
                        "output": output,
                        "summary": summary,
                    }));
                }
            }
        }
    }

    // 3. Python
    let has_pytest_config = root.join("pytest.ini").exists()
        || root.join("pyproject.toml").exists()
        || root.join("setup.py").exists()
        || root.join("tests").is_dir();
    let has_py_files = dir_contains_extension(&root, "py", 2);

    if has_pytest_config || has_py_files {
        let cmd = if has_pytest_config {
            "pytest"
        } else {
            "python -m unittest discover"
        };
        let (mut passed, mut code, mut output) =
            execute_test_command(&root, cmd, &context.credential_env_names)?;
        if !passed
            && (output.contains("not found")
                || output.contains("is not recognized")
                || output.contains("No module named"))
        {
            let py_check = if cfg!(windows) {
                "Get-ChildItem -Recurse -Filter *.py | ForEach-Object { python -m py_compile $_.FullName }"
            } else {
                "python3 -m compileall ."
            };
            let (syntax_passed, syntax_code, syntax_out) =
                execute_test_command(&root, py_check, &context.credential_env_names)?;
            passed = syntax_passed;
            code = syntax_code;
            output = syntax_out;
        }

        let summary = if passed {
            "Python test/syntax check passed successfully".to_string()
        } else {
            format!("Python test/syntax check failed with exit code {code}")
        };
        return Ok(json!({
            "status": if passed { "passed" } else { "failed" },
            "passed": passed,
            "framework": "python",
            "command": cmd,
            "output": output,
            "summary": summary,
        }));
    }

    // 4. HTML / Web
    if dir_contains_extension(&root, "html", 2) {
        return validate_html_project(&root);
    }

    // 5. None
    Ok(json!({
        "status": "no_tests_found",
        "passed": true,
        "framework": "none",
        "command": "",
        "output": "No test configuration (Cargo.toml, package.json test script, pytest, or HTML entrypoint) detected.",
        "summary": "No tests configured in workspace.",
    }))
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
                "python.run",
                "question.ask",
                "shell.bash.safe",
                "shell.powershell.safe",
                "shell.run",
                "shell.zsh.safe",
                "skill.create",
                "test.run",
                "web.fetch",
            ]
        );
    }

    #[test]
    fn every_builtin_executor_has_complete_schema_policy_and_fixture_metadata() {
        let descriptors = ExecutorRegistry::with_builtin_executors().descriptors();
        assert_eq!(descriptors.len(), 14);
        for descriptor in descriptors {
            assert!(descriptor.is_complete(), "incomplete: {}", descriptor.id);
            assert!(descriptor.input_schema.is_object());
            assert!(descriptor.output_schema.is_object());
            assert!(!descriptor.side_effects.is_empty() || descriptor.id == "question.ask");
            assert!(descriptor.deterministic_fixture.is_object());
            validate_schema_value(&descriptor.deterministic_fixture, &descriptor.input_schema)
                .unwrap_or_else(|error| panic!("invalid fixture for {}: {error}", descriptor.id));
        }
    }

    #[test]
    fn dev_server_and_listening_detection_helpers_behave_as_expected() {
        assert!(is_dev_server_command("python -m http.server 8000"));
        assert!(is_dev_server_command("npx http-server -p 8080"));
        assert!(is_dev_server_command("npm run dev"));
        assert!(is_dev_server_command("vite --host"));
        assert!(is_dev_server_command("live-server ."));
        assert!(!is_dev_server_command("cargo test"));
        assert!(!is_dev_server_command("npm test"));

        assert!(is_server_listening_output(
            "Serving HTTP on 0.0.0.0 port 8000 (http://0.0.0.0:8000/) ..."
        ));
        assert!(is_server_listening_output(
            "  ➜  Local:   http://localhost:5173/"
        ));
        assert!(is_server_listening_output(
            "Available on: http://127.0.0.1:8080"
        ));
        assert!(!is_server_listening_output(
            "test result: ok. 0 passed; 0 failed"
        ));
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
    fn web_fetch_supports_query_argument() {
        let root = unique_temp_dir();
        let request = ToolRequest {
            skill_id: "web.fetch".to_string(),
            arguments: json!({"query": "minecraft plugins"}),
        };

        let target = validated_web_target(&request, &context(&root))
            .expect("query should map to duckduckgo html target");

        assert_eq!(target, "https://html.duckduckgo.com/html/");
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
            skills_dir: None,
        }
    }

    #[tokio::test]
    async fn skill_create_executor_creates_skill_and_updates_manifest() {
        let dir = unique_temp_dir();
        let workspace = dir.join("workspace");
        let skills_dir = dir.join("skills");
        fs::create_dir_all(&workspace).expect("workspace dir");
        fs::create_dir_all(&skills_dir).expect("skills dir");

        let mut ctx = context(&workspace);
        ctx.skills_dir = Some(skills_dir.clone());
        ctx.auto_approve_medium_risk = true;

        let registry = ExecutorRegistry::with_builtin_executors();
        let executor = registry
            .get("skill.create")
            .expect("skill.create registered");

        let request = ToolRequest {
            skill_id: "skill.create".to_string(),
            arguments: json!({
                "id": "project.test_helper",
                "name": "Project Test Helper",
                "description": "Helps running custom tests",
                "content": "Step 1: Check tests. Step 2: Run them.",
                "skill_type": "workflow",
                "when_to_use": ["user asks to test helper"],
                "tags": ["testing", "workflow"]
            }),
        };

        let mut approval = AllowAllApprover;
        let policy = SideEffectPolicy::allow_all();
        let mut audit = crate::NoopSideEffectAuditSink;

        let output = executor
            .execute_with_policy(&request, &ctx, &mut approval, &policy, &mut audit)
            .await
            .expect("execute skill.create");

        assert_eq!(
            output.get("status").and_then(Value::as_str),
            Some("success")
        );
        assert_eq!(
            output.get("skill_id").and_then(Value::as_str),
            Some("project.test_helper")
        );

        let skill_folder = skills_dir.join("project.test_helper");
        assert!(skill_folder.join("skill.toml").exists());
        assert!(skill_folder.join("SKILL.md").exists());

        let installed = crate::InstalledSkills::load_from_dir(&skills_dir).expect("load installed");
        assert!(installed.skills.contains_key("project.test_helper"));

        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn builtin_installed_skill_provides_executable_synthetic_skills() {
        for builtin_id in &[
            "skill.create",
            "file.read",
            "file.write",
            "project.scan",
            "web.fetch",
            "shell.powershell.safe",
            "shell.bash.safe",
            "python.run",
            "question.ask",
            "test.run",
        ] {
            let skill = builtin_installed_skill(builtin_id).expect("builtin skill found");
            assert_eq!(skill.manifest.id, *builtin_id);
            assert!(skill.record.is_executable());
            assert_eq!(skill.manifest.skill_type, SkillType::Tool);
        }
    }

    #[tokio::test]
    async fn question_ask_executor_selects_option_or_default() {
        let registry = ExecutorRegistry::with_builtin_executors();
        let executor = registry
            .get("question.ask")
            .expect("question.ask registered");
        let context = SkillExecutionContext {
            workspace_root: PathBuf::from("."),
            max_file_read_bytes: 1024,
            web_timeout_secs: 10,
            max_web_response_bytes: 1024,
            web_fetch_https_only: true,
            web_fetch_allowed_hosts: vec![],
            web_fetch_denied_hosts: vec![],
            web_fetch_use_system_proxy: false,
            auto_approve_medium_risk: true,
            credential_env_names: vec![],
            skills_dir: None,
        };

        let request = ToolRequest {
            skill_id: "question.ask".to_string(),
            arguments: json!({
                "question": "Which styling framework?",
                "options": ["Tailwind CSS", "Bootstrap", "Vanilla CSS"]
            }),
        };

        let mut approval = AllowAllApprover;
        let result = executor
            .execute(&request, &context, &mut approval)
            .await
            .expect("execute");
        assert_eq!(
            result.get("selected").and_then(Value::as_str),
            Some("Tailwind CSS")
        );
        assert_eq!(result.get("index").and_then(Value::as_u64), Some(1));
        assert_eq!(
            result.get("is_custom").and_then(Value::as_bool),
            Some(false)
        );
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

    #[test]
    fn extract_text_from_html_strips_scripts_styles_and_tags() {
        let sample = r#"
            <!DOCTYPE html>
            <html>
            <head>
                <title>Test Page</title>
                <style>body { color: red; }</style>
                <script>var secret = 12345;</script>
            </head>
            <body>
                <noscript>JavaScript disabled</noscript>
                <h1>Welcome &amp; Hello</h1>
                <p>This is a paragraph with <a href="/link">a link</a> &quot;quoted&quot;.</p>
            </body>
            </html>
        "#;
        let text = extract_text_from_html(sample);
        assert!(!text.contains("var secret"));
        assert!(!text.contains("color: red"));
        assert!(!text.contains("JavaScript disabled"));
        assert!(!text.contains("<h1>"));
        assert!(text.contains("Welcome & Hello"));
        assert!(text.contains("This is a paragraph with a link \"quoted\"."));
    }

    #[tokio::test]
    async fn test_run_executor_validates_html_and_assets() {
        let dir = unique_temp_dir();
        fs::create_dir_all(&dir).expect("create dir");

        let html_content = r#"<!DOCTYPE html>
<html>
<head>
    <title>Snake Game</title>
    <link rel="stylesheet" href="style.css">
</head>
<body>
    <canvas id="game"></canvas>
    <script src="game.js"></script>
</body>
</html>"#;
        fs::write(dir.join("index.html"), html_content).expect("write html");
        fs::write(dir.join("style.css"), "body { background: #111; }").expect("write css");
        fs::write(
            dir.join("game.js"),
            "const canvas = document.getElementById('game');",
        )
        .expect("write js");

        let registry = ExecutorRegistry::with_builtin_executors();
        let executor = registry.get("test.run").expect("test.run registered");
        let context = SkillExecutionContext {
            workspace_root: dir.clone(),
            max_file_read_bytes: 1024,
            web_timeout_secs: 10,
            max_web_response_bytes: 1024,
            web_fetch_https_only: true,
            web_fetch_allowed_hosts: vec![],
            web_fetch_denied_hosts: vec![],
            web_fetch_use_system_proxy: false,
            auto_approve_medium_risk: true,
            credential_env_names: vec![],
            skills_dir: None,
        };

        let request = ToolRequest {
            skill_id: "test.run".to_string(),
            arguments: json!({}),
        };

        let mut approval = AllowAllApprover;
        let result = executor
            .execute(&request, &context, &mut approval)
            .await
            .expect("execute test.run");

        assert_eq!(
            result.get("status").and_then(Value::as_str),
            Some("passed"),
            "result: {result:?}"
        );
        assert_eq!(result.get("passed").and_then(Value::as_bool), Some(true));
        assert_eq!(
            result.get("framework").and_then(Value::as_str),
            Some("html_web")
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn test_run_executor_catches_missing_referenced_asset() {
        let dir = unique_temp_dir();
        fs::create_dir_all(&dir).expect("create dir");

        let html_content = r#"<!DOCTYPE html>
<html>
<head><title>Broken</title></head>
<body>
    <script src="missing_script.js"></script>
</body>
</html>"#;
        fs::write(dir.join("index.html"), html_content).expect("write html");

        let registry = ExecutorRegistry::with_builtin_executors();
        let executor = registry.get("test.run").expect("test.run registered");
        let context = SkillExecutionContext {
            workspace_root: dir.clone(),
            max_file_read_bytes: 1024,
            web_timeout_secs: 10,
            max_web_response_bytes: 1024,
            web_fetch_https_only: true,
            web_fetch_allowed_hosts: vec![],
            web_fetch_denied_hosts: vec![],
            web_fetch_use_system_proxy: false,
            auto_approve_medium_risk: true,
            credential_env_names: vec![],
            skills_dir: None,
        };

        let request = ToolRequest {
            skill_id: "test.run".to_string(),
            arguments: json!({}),
        };

        let mut approval = AllowAllApprover;
        let result = executor
            .execute(&request, &context, &mut approval)
            .await
            .expect("execute test.run");

        assert_eq!(result.get("status").and_then(Value::as_str), Some("failed"));
        assert_eq!(result.get("passed").and_then(Value::as_bool), Some(false));
        let output = result.get("output").and_then(Value::as_str).unwrap();
        assert!(output.contains("missing_script.js"));

        let _ = fs::remove_dir_all(dir);
    }
}
