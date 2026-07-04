use semver::Version;
use serde::{Deserialize, Serialize};

use crate::UpdateError;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseChannel {
    #[default]
    Stable,
    Nightly,
    Dev,
}

impl ReleaseChannel {
    pub fn parse(value: &str) -> Result<Self, UpdateError> {
        match value {
            "stable" => Ok(Self::Stable),
            "nightly" => Ok(Self::Nightly),
            "dev" => Ok(Self::Dev),
            other => Err(UpdateError::InvalidChannel(other.to_string())),
        }
    }
}

impl std::fmt::Display for ReleaseChannel {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Stable => "stable",
            Self::Nightly => "nightly",
            Self::Dev => "dev",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdatePolicy {
    Manual,
    #[default]
    Notify,
    AutoPatch,
}

impl UpdatePolicy {
    pub fn parse(value: &str) -> Result<Self, UpdateError> {
        match value {
            "manual" => Ok(Self::Manual),
            "notify" => Ok(Self::Notify),
            "auto-patch" => Ok(Self::AutoPatch),
            other => Err(UpdateError::InvalidPolicy(other.to_string())),
        }
    }
}

impl std::fmt::Display for UpdatePolicy {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Manual => "manual",
            Self::Notify => "notify",
            Self::AutoPatch => "auto-patch",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateKind {
    Patch,
    Minor,
    Major,
    Same,
    Downgrade,
}

impl std::fmt::Display for UpdateKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Patch => "patch",
            Self::Minor => "minor",
            Self::Major => "major",
            Self::Same => "same",
            Self::Downgrade => "downgrade",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionComparison {
    pub current: Version,
    pub latest: Version,
    pub kind: UpdateKind,
    pub update_available: bool,
    pub install_allowed_without_confirmation: bool,
}

pub fn parse_version(value: &str) -> Result<Version, UpdateError> {
    let normalized = value.trim().trim_start_matches('v');
    Version::parse(normalized).map_err(|_| UpdateError::InvalidSemver(value.to_string()))
}

pub fn compare_versions(
    current: &Version,
    latest: &Version,
    policy: UpdatePolicy,
) -> VersionComparison {
    let kind = if latest == current {
        UpdateKind::Same
    } else if latest < current {
        UpdateKind::Downgrade
    } else if latest.major > current.major {
        UpdateKind::Major
    } else if latest.minor > current.minor {
        UpdateKind::Minor
    } else {
        UpdateKind::Patch
    };

    VersionComparison {
        current: current.clone(),
        latest: latest.clone(),
        kind,
        update_available: matches!(
            kind,
            UpdateKind::Patch | UpdateKind::Minor | UpdateKind::Major
        ),
        install_allowed_without_confirmation: kind == UpdateKind::Patch
            && policy == UpdatePolicy::AutoPatch,
    }
}

pub fn is_newer_version(current: &Version, candidate: &Version) -> bool {
    candidate > current
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_patch_minor_major_same_and_downgrade() {
        let current = Version::new(0, 1, 0);

        assert_eq!(
            compare_versions(&current, &Version::new(0, 1, 1), UpdatePolicy::Notify).kind,
            UpdateKind::Patch
        );
        assert_eq!(
            compare_versions(&current, &Version::new(0, 2, 0), UpdatePolicy::Notify).kind,
            UpdateKind::Minor
        );
        assert_eq!(
            compare_versions(&current, &Version::new(1, 0, 0), UpdatePolicy::Notify).kind,
            UpdateKind::Major
        );
        assert_eq!(
            compare_versions(&current, &Version::new(0, 1, 0), UpdatePolicy::Notify).kind,
            UpdateKind::Same
        );
        assert_eq!(
            compare_versions(&current, &Version::new(0, 0, 9), UpdatePolicy::Notify).kind,
            UpdateKind::Downgrade
        );
    }

    #[test]
    fn invalid_semver_is_rejected_cleanly() {
        assert!(matches!(
            parse_version("not-a-version"),
            Err(UpdateError::InvalidSemver(_))
        ));
    }

    #[test]
    fn policy_auto_patch_only_allows_patch_without_confirmation() {
        let current = Version::new(0, 1, 0);
        let patch = compare_versions(&current, &Version::new(0, 1, 1), UpdatePolicy::AutoPatch);
        let minor = compare_versions(&current, &Version::new(0, 2, 0), UpdatePolicy::AutoPatch);
        let manual = compare_versions(&current, &Version::new(0, 1, 1), UpdatePolicy::Manual);

        assert!(patch.install_allowed_without_confirmation);
        assert!(!minor.install_allowed_without_confirmation);
        assert!(!manual.install_allowed_without_confirmation);
    }
}
