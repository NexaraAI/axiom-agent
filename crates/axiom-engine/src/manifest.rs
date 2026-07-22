use std::{fs, path::Path};

use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::permissions::Permission;

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse skill manifest TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("skill manifest field is missing or empty: {0}")]
    MissingField(&'static str),
    #[error("unsupported skill manifest schema version: {0}")]
    UnsupportedSchemaVersion(String),
    #[error("invalid skill manifest field `{field}`: {message}")]
    InvalidField {
        field: &'static str,
        message: String,
    },
}

pub type Result<T> = std::result::Result<T, ManifestError>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillManifest {
    #[serde(default = "default_manifest_schema_version")]
    pub schema_version: String,
    pub id: String,
    pub name: String,
    pub version: Version,
    pub description: String,
    #[serde(default = "default_category")]
    pub category: String,
    #[serde(default)]
    pub skill_type: SkillType,
    pub risk_level: RiskLevel,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    #[serde(default)]
    pub platforms: Vec<Platform>,
    pub entrypoint: String,
    pub author: String,
    pub license: String,
    #[serde(default = "default_min_axiom_version")]
    pub min_axiom_version: Version,
    #[serde(default)]
    pub max_axiom_version: Option<Version>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub provides: Vec<String>,
    #[serde(default)]
    pub hooks: SkillHooks,
    #[serde(default)]
    pub side_effects: Vec<SideEffect>,
    #[serde(default)]
    pub idempotent: bool,
    #[serde(default)]
    pub cache_key: Option<String>,
    #[serde(default)]
    pub examples: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub llm_card: Option<LlmCardManifest>,
    #[serde(default)]
    pub updates: UpdatePolicy,
    #[serde(default = "default_schema")]
    pub input_schema: toml::Value,
    #[serde(default = "default_schema")]
    pub output_schema: toml::Value,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillHooks {
    #[serde(default)]
    pub pre: Option<String>,
    #[serde(default)]
    pub post: Option<String>,
    #[serde(default)]
    pub on_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffect {
    FileSystem,
    Network,
    Process,
    Git,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdatePolicy {
    pub auto_update: bool,
    pub channel: String,
}

impl Default for UpdatePolicy {
    fn default() -> Self {
        Self {
            auto_update: false,
            channel: "stable".to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmCardManifest {
    pub summary: String,
    #[serde(default)]
    pub when_to_use: Vec<String>,
    pub input_contract: String,
    pub output_contract: String,
    pub token_budget: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillCard {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub when_to_use: Vec<String>,
    pub input_contract: String,
    pub output_contract: String,
    pub risk_level: RiskLevel,
    pub permissions: Vec<Permission>,
    pub token_budget: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl std::fmt::Display for RiskLevel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Critical => "critical",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillType {
    #[default]
    Tool,
    Prompt,
    Workflow,
    Guard,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Platform {
    Windows,
    Linux,
    Macos,
}

impl Platform {
    pub fn current() -> Self {
        match std::env::consts::OS {
            "windows" => Self::Windows,
            "macos" => Self::Macos,
            _ => Self::Linux,
        }
    }

    pub fn from_os(os: &str) -> Option<Self> {
        match os {
            "windows" => Some(Self::Windows),
            "linux" => Some(Self::Linux),
            "macos" => Some(Self::Macos),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::Macos => "macos",
        }
    }
}

impl SkillManifest {
    pub fn parse_toml(input: &str) -> Result<Self> {
        let manifest: Self = toml::from_str(input)?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn from_path(path: impl AsRef<Path>) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        Self::parse_toml(&content)
    }

    pub fn validate(&self) -> Result<()> {
        if !matches!(self.schema_version.as_str(), "0.1" | "1.0") {
            return Err(ManifestError::UnsupportedSchemaVersion(
                self.schema_version.clone(),
            ));
        }
        if self.id.trim().is_empty() {
            return Err(ManifestError::MissingField("id"));
        }
        if self.name.trim().is_empty() {
            return Err(ManifestError::MissingField("name"));
        }
        if self.description.trim().is_empty() {
            return Err(ManifestError::MissingField("description"));
        }
        if self.entrypoint.trim().is_empty() {
            return Err(ManifestError::MissingField("entrypoint"));
        }
        if self.author.trim().is_empty() {
            return Err(ManifestError::MissingField("author"));
        }
        if self.license.trim().is_empty() {
            return Err(ManifestError::MissingField("license"));
        }
        validate_skill_id("id", &self.id)?;
        if self
            .depends_on
            .iter()
            .any(|dependency| dependency.trim().is_empty())
        {
            return Err(ManifestError::InvalidField {
                field: "depends_on",
                message: "dependency IDs cannot be empty".to_string(),
            });
        }
        if self
            .depends_on
            .iter()
            .any(|dependency| dependency == &self.id)
        {
            return Err(ManifestError::InvalidField {
                field: "depends_on",
                message: "a skill cannot depend on itself".to_string(),
            });
        }
        validate_unique_ids("depends_on", &self.depends_on)?;
        validate_unique_ids("provides", &self.provides)?;
        for dependency in &self.depends_on {
            validate_skill_id("depends_on", dependency)?;
        }
        for capability in &self.provides {
            validate_skill_id("provides", capability)?;
        }
        for (field, hook) in [
            ("hooks.pre", self.hooks.pre.as_deref()),
            ("hooks.post", self.hooks.post.as_deref()),
            ("hooks.on_error", self.hooks.on_error.as_deref()),
        ] {
            if let Some(hook) = hook {
                validate_skill_id(field, hook)?;
            }
        }
        if let Some(card) = &self.llm_card {
            if card.summary.trim().is_empty() {
                return Err(ManifestError::MissingField("llm_card.summary"));
            }
            if card.token_budget == 0 {
                return Err(ManifestError::InvalidField {
                    field: "llm_card.token_budget",
                    message: "must be greater than zero".to_string(),
                });
            }
        }
        validate_json_schema_type("input_schema", &self.input_schema)?;
        validate_json_schema_type("output_schema", &self.output_schema)?;
        Ok(())
    }

    pub fn is_platform_compatible(&self, platform: &Platform) -> bool {
        self.platforms.is_empty() || self.platforms.iter().any(|candidate| candidate == platform)
    }

    pub fn to_skill_card(&self) -> SkillCard {
        let card = self.llm_card.clone().unwrap_or_else(|| LlmCardManifest {
            summary: self.description.clone(),
            when_to_use: Vec::new(),
            input_contract: "task".to_string(),
            output_contract: "response".to_string(),
            token_budget: 250,
        });

        SkillCard {
            id: self.id.clone(),
            name: self.name.clone(),
            summary: card.summary,
            when_to_use: card.when_to_use,
            input_contract: card.input_contract,
            output_contract: card.output_contract,
            risk_level: self.risk_level.clone(),
            permissions: self.permissions.clone(),
            token_budget: card.token_budget,
        }
    }
}

fn default_category() -> String {
    "general".to_string()
}

fn default_manifest_schema_version() -> String {
    "0.1".to_string()
}

fn validate_skill_id(field: &'static str, value: &str) -> Result<()> {
    let valid = !value.is_empty()
        && !value.starts_with('.')
        && !value.ends_with('.')
        && value.split('.').all(|segment| {
            !segment.is_empty()
                && segment.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || matches!(byte, b'-' | b'_')
                })
        });
    if valid {
        Ok(())
    } else {
        Err(ManifestError::InvalidField {
            field,
            message: format!(
                "`{value}` must use lowercase dot-separated identifiers with letters, numbers, `-`, or `_`"
            ),
        })
    }
}

fn validate_unique_ids(field: &'static str, values: &[String]) -> Result<()> {
    let mut seen = std::collections::BTreeSet::new();
    if let Some(duplicate) = values.iter().find(|value| !seen.insert(value.as_str())) {
        return Err(ManifestError::InvalidField {
            field,
            message: format!("duplicate value `{duplicate}`"),
        });
    }
    Ok(())
}

fn validate_json_schema_type(field: &'static str, schema: &toml::Value) -> Result<()> {
    let Some(schema_type) = schema
        .as_table()
        .and_then(|table| table.get("type"))
        .and_then(toml::Value::as_str)
    else {
        return Ok(());
    };
    if matches!(
        schema_type,
        "object" | "array" | "string" | "number" | "integer" | "boolean" | "null"
    ) {
        Ok(())
    } else {
        Err(ManifestError::InvalidField {
            field,
            message: format!("unsupported JSON Schema type `{schema_type}`"),
        })
    }
}

fn default_min_axiom_version() -> Version {
    Version::new(0, 1, 0)
}

fn default_schema() -> toml::Value {
    toml::Value::Table(toml::map::Map::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_skill_manifest() {
        let manifest = SkillManifest::parse_toml(
            r#"
id = "file.read"
name = "Read Text File"
version = "0.1.0"
description = "Reads UTF-8 text from a file inside the active workspace."
category = "files"
skill_type = "tool"
risk_level = "low"
permissions = ["file_system_read"]
platforms = ["windows", "linux", "macos"]
entrypoint = "builtin:file_read"
author = "Axiom Agent"
license = "MIT"
min_axiom_version = "0.1.0"

[llm_card]
summary = "Read text files inside the active workspace."
when_to_use = ["User asks to read a file"]
input_contract = "path"
output_contract = "content"
token_budget = 200

[updates]
auto_update = false
channel = "stable"

[input_schema]
type = "object"

[output_schema]
type = "object"
"#,
        )
        .expect("manifest parses");

        assert_eq!(manifest.id, "file.read");
        assert_eq!(manifest.schema_version, "0.1");
        assert_eq!(manifest.version, Version::new(0, 1, 0));
        assert_eq!(manifest.risk_level, RiskLevel::Low);
        assert_eq!(manifest.permissions, vec![Permission::FileSystemRead]);
        assert_eq!(manifest.skill_type, SkillType::Tool);
        assert!(manifest.depends_on.is_empty());
        assert!(manifest.side_effects.is_empty());
        assert_eq!(manifest.to_skill_card().id, "file.read");
    }

    #[test]
    fn parses_manifest_extensions_for_lens_and_execution_policy() {
        let manifest = SkillManifest::parse_toml(
            r#"
id = "project.lint"
name = "Project Linter"
version = "1.0.0"
description = "Runs a project linter."
risk_level = "medium"
permissions = []
platforms = ["windows"]
entrypoint = "builtin:project.lint"
author = "Axiom"
license = "MIT"
depends_on = ["file.read"]
provides = ["project-quality"]
side_effects = ["process"]
idempotent = true
cache_key = "workspace-hash"
keywords = ["lint", "quality"]
examples = ["lint this project"]

[hooks]
pre = "file.read"
"#,
        )
        .expect("extended manifest parses");

        assert_eq!(manifest.depends_on, vec!["file.read"]);
        assert_eq!(manifest.side_effects, vec![SideEffect::Process]);
        assert!(manifest.idempotent);
        assert_eq!(manifest.hooks.pre.as_deref(), Some("file.read"));
    }

    #[test]
    fn rejects_empty_required_field() {
        let error = SkillManifest::parse_toml(
            r#"
id = ""
name = "Broken"
version = "0.1.0"
description = "Broken manifest."
risk_level = "low"
permissions = []
platforms = ["windows"]
entrypoint = "builtin:broken"
author = "Axiom Agent"
license = "MIT"

[updates]
auto_update = false
channel = "stable"

[input_schema]
type = "object"

[output_schema]
type = "object"
"#,
        )
        .expect_err("empty id should fail");

        assert!(matches!(error, ManifestError::MissingField("id")));
    }

    #[test]
    fn rejects_future_schema_and_invalid_dependency_metadata() {
        let future = SkillManifest::parse_toml(
            r#"
schema_version = "99.0"
id = "test.skill"
name = "Test"
version = "1.0.0"
description = "Test skill."
risk_level = "low"
entrypoint = "prompt-only"
author = "Axiom"
license = "MIT"
"#,
        )
        .expect_err("future schema should fail closed");
        assert!(matches!(future, ManifestError::UnsupportedSchemaVersion(_)));

        let duplicate = SkillManifest::parse_toml(
            r#"
schema_version = "1.0"
id = "test.skill"
name = "Test"
version = "1.0.0"
description = "Test skill."
risk_level = "low"
entrypoint = "prompt-only"
author = "Axiom"
license = "MIT"
depends_on = ["file.read", "file.read"]
"#,
        )
        .expect_err("duplicate dependencies should fail");
        assert!(matches!(
            duplicate,
            ManifestError::InvalidField {
                field: "depends_on",
                ..
            }
        ));
    }
}
