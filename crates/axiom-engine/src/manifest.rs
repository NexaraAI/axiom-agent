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
}

pub type Result<T> = std::result::Result<T, ManifestError>;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillManifest {
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
    pub llm_card: Option<LlmCardManifest>,
    #[serde(default)]
    pub updates: UpdatePolicy,
    #[serde(default = "default_schema")]
    pub input_schema: toml::Value,
    #[serde(default = "default_schema")]
    pub output_schema: toml::Value,
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
        assert_eq!(manifest.version, Version::new(0, 1, 0));
        assert_eq!(manifest.risk_level, RiskLevel::Low);
        assert_eq!(manifest.permissions, vec![Permission::FileSystemRead]);
        assert_eq!(manifest.skill_type, SkillType::Tool);
        assert_eq!(manifest.to_skill_card().id, "file.read");
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
}
