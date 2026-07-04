use semver::Version;
use serde::{Deserialize, Serialize};

use crate::{Platform, RegistrySkillEntry, SkillManifest};

pub const OFFICIAL_REGISTRY_URL: &str =
    "https://raw.githubusercontent.com/NexaraAI/axiom-skills/main/registry.json";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillLifecycleState {
    #[default]
    Enabled,
    Disabled,
    UpdateAvailable,
    Incompatible,
    Quarantined,
    FailedUpdate,
}

impl std::fmt::Display for SkillLifecycleState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Enabled => "enabled",
            Self::Disabled => "disabled",
            Self::UpdateAvailable => "update_available",
            Self::Incompatible => "incompatible",
            Self::Quarantined => "quarantined",
            Self::FailedUpdate => "failed_update",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    #[default]
    Trusted,
    Community,
    Untrusted,
    Blocked,
}

impl std::fmt::Display for TrustLevel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Trusted => "trusted",
            Self::Community => "community",
            Self::Untrusted => "untrusted",
            Self::Blocked => "blocked",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompatibilityResult {
    pub compatible: bool,
    pub reason: String,
}

impl CompatibilityResult {
    pub fn compatible() -> Self {
        Self {
            compatible: true,
            reason: "compatible".to_string(),
        }
    }

    pub fn incompatible(reason: impl Into<String>) -> Self {
        Self {
            compatible: false,
            reason: reason.into(),
        }
    }
}

impl std::fmt::Display for CompatibilityResult {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.reason)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillLifecycleAssessment {
    pub enabled: bool,
    pub state: SkillLifecycleState,
    pub trust_level: TrustLevel,
    pub compatibility: CompatibilityResult,
}

pub fn current_axiom_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or_else(|_| Version::new(0, 1, 0))
}

pub fn is_supported_entrypoint(entrypoint: &str) -> bool {
    entrypoint == "prompt-only" || entrypoint.starts_with("builtin:")
}

pub fn is_official_registry_location(location: &str) -> bool {
    location == OFFICIAL_REGISTRY_URL
}

pub fn is_bundled_registry_location(location: &str) -> bool {
    let normalized = location.replace('\\', "/");
    normalized.ends_with("fixtures/skill-registry")
        || normalized.ends_with("fixtures/skill-registry/registry.json")
}

pub fn is_trusted_registry_location(location: &str, source_label: &str) -> bool {
    is_official_registry_location(location)
        || source_label == "bundled"
        || is_bundled_registry_location(location)
}

pub fn check_registry_entry_compatibility(
    entry: &RegistrySkillEntry,
    current_version: &Version,
    platform: &Platform,
) -> CompatibilityResult {
    if &entry.min_axiom_version > current_version {
        return CompatibilityResult::incompatible(format!(
            "requires Axiom >= {}",
            entry.min_axiom_version
        ));
    }

    if let Some(max_version) = &entry.max_axiom_version {
        if current_version > max_version {
            return CompatibilityResult::incompatible(format!("requires Axiom <= {max_version}"));
        }
    }

    if !entry.platforms.is_empty() {
        let platform_name = platform.as_str();
        if !entry
            .platforms
            .iter()
            .any(|candidate| candidate == platform_name)
        {
            return CompatibilityResult::incompatible(format!(
                "platform {} is not supported",
                platform_name
            ));
        }
    }

    CompatibilityResult::compatible()
}

pub fn check_manifest_compatibility(
    manifest: &SkillManifest,
    current_version: &Version,
    platform: &Platform,
) -> CompatibilityResult {
    if &manifest.min_axiom_version > current_version {
        return CompatibilityResult::incompatible(format!(
            "requires Axiom >= {}",
            manifest.min_axiom_version
        ));
    }

    if let Some(max_version) = &manifest.max_axiom_version {
        if current_version > max_version {
            return CompatibilityResult::incompatible(format!("requires Axiom <= {max_version}"));
        }
    }

    if !manifest.is_platform_compatible(platform) {
        return CompatibilityResult::incompatible(format!(
            "platform {} is not supported",
            platform.as_str()
        ));
    }

    CompatibilityResult::compatible()
}

pub fn assess_skill_lifecycle(
    manifest: &SkillManifest,
    registry_location: &str,
    source_label: &str,
    checksum: Option<&str>,
    current_version: &Version,
    platform: &Platform,
) -> SkillLifecycleAssessment {
    let compatibility = check_manifest_compatibility(manifest, current_version, platform);
    if !compatibility.compatible {
        return SkillLifecycleAssessment {
            enabled: false,
            state: SkillLifecycleState::Incompatible,
            trust_level: TrustLevel::Blocked,
            compatibility,
        };
    }

    let trusted_source = is_trusted_registry_location(registry_location, source_label);
    let supported_entrypoint = is_supported_entrypoint(&manifest.entrypoint);
    let suspicious_metadata =
        manifest.author.trim().is_empty() || manifest.license.trim().is_empty();

    let trust_level = if trusted_source && supported_entrypoint && !suspicious_metadata {
        TrustLevel::Trusted
    } else if !supported_entrypoint || suspicious_metadata || checksum.is_none() {
        TrustLevel::Untrusted
    } else {
        TrustLevel::Community
    };

    if !supported_entrypoint {
        return SkillLifecycleAssessment {
            enabled: false,
            state: SkillLifecycleState::Quarantined,
            trust_level,
            compatibility: CompatibilityResult::incompatible(
                "entrypoint is not supported by this Axiom version",
            ),
        };
    }

    SkillLifecycleAssessment {
        enabled: trust_level != TrustLevel::Blocked,
        state: SkillLifecycleState::Enabled,
        trust_level,
        compatibility,
    }
}

#[cfg(test)]
mod tests {
    use semver::Version;

    use crate::{RiskLevel, SkillType};

    use super::*;

    #[test]
    fn parses_lifecycle_state_and_trust_level() {
        let state: SkillLifecycleState =
            serde_json::from_str("\"update_available\"").expect("state parses");
        let trust: TrustLevel = serde_json::from_str("\"community\"").expect("trust parses");

        assert_eq!(state, SkillLifecycleState::UpdateAvailable);
        assert_eq!(trust, TrustLevel::Community);
    }

    #[test]
    fn official_registry_is_trusted() {
        assert!(is_trusted_registry_location(
            OFFICIAL_REGISTRY_URL,
            "remote"
        ));
    }

    #[test]
    fn custom_registry_without_checksum_is_untrusted() {
        let manifest = test_manifest("prompt-only");
        let assessment = assess_skill_lifecycle(
            &manifest,
            "https://example.com/registry.json",
            "remote",
            None,
            &Version::new(0, 1, 0),
            &Platform::Windows,
        );

        assert_eq!(assessment.trust_level, TrustLevel::Untrusted);
        assert_eq!(assessment.state, SkillLifecycleState::Enabled);
    }

    #[test]
    fn unsupported_entrypoint_is_quarantined() {
        let manifest = test_manifest("external:run");
        let assessment = assess_skill_lifecycle(
            &manifest,
            "https://example.com/registry.json",
            "remote",
            Some("checksum"),
            &Version::new(0, 1, 0),
            &Platform::Windows,
        );

        assert!(!assessment.enabled);
        assert_eq!(assessment.state, SkillLifecycleState::Quarantined);
        assert_eq!(assessment.trust_level, TrustLevel::Untrusted);
    }

    #[test]
    fn min_version_blocks_compatibility() {
        let mut manifest = test_manifest("prompt-only");
        manifest.min_axiom_version = Version::new(9, 0, 0);

        let assessment = assess_skill_lifecycle(
            &manifest,
            OFFICIAL_REGISTRY_URL,
            "remote",
            None,
            &Version::new(0, 1, 0),
            &Platform::Windows,
        );

        assert_eq!(assessment.state, SkillLifecycleState::Incompatible);
        assert_eq!(assessment.trust_level, TrustLevel::Blocked);
    }

    fn test_manifest(entrypoint: &str) -> SkillManifest {
        SkillManifest {
            id: "test.skill".to_string(),
            name: "Test Skill".to_string(),
            version: Version::new(0, 1, 0),
            description: "Test skill.".to_string(),
            category: "test".to_string(),
            skill_type: SkillType::Prompt,
            risk_level: RiskLevel::Low,
            permissions: Vec::new(),
            platforms: vec![Platform::Windows],
            entrypoint: entrypoint.to_string(),
            author: "Test".to_string(),
            license: "MIT".to_string(),
            min_axiom_version: Version::new(0, 1, 0),
            max_axiom_version: None,
            llm_card: None,
            updates: Default::default(),
            input_schema: toml::Value::Table(toml::map::Map::new()),
            output_schema: toml::Value::Table(toml::map::Map::new()),
        }
    }
}
