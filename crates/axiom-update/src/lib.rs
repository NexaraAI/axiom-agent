pub mod checksum;
pub mod github_release;
pub mod installer;
pub mod platform;
pub mod state;
pub mod version;

use thiserror::Error;

pub const CHECKSUM_FILE_NAME: &str = "SHA256SUMS";

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("invalid semver: {0}")]
    InvalidSemver(String),
    #[error("invalid release channel: {0}")]
    InvalidChannel(String),
    #[error("invalid update policy: {0}")]
    InvalidPolicy(String),
    #[error("invalid release repository URL: {0}")]
    InvalidReleaseRepo(String),
    #[error("no release found for channel {0}")]
    NoReleaseForChannel(version::ReleaseChannel),
    #[error("no prebuilt Axiom binary is available for this platform: {os}/{arch}")]
    UnsupportedPlatform { os: String, arch: String },
    #[error("release asset is missing: {0}")]
    MissingAsset(String),
    #[error("checksum not found for asset. Update blocked: {0}")]
    MissingChecksum(String),
    #[error("checksum mismatch for {asset}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        asset: String,
        expected: String,
        actual: String,
    },
    #[error("install blocked: {0}")]
    InstallBlocked(String),
    #[error("missing staged binary: {0}")]
    MissingStagedBinary(String),
    #[error("no rollback backup is available")]
    NoRollbackAvailable,
    #[error("release JSON error: {0}")]
    ReleaseJson(String),
    #[error("update state JSON error: {0}")]
    StateJson(String),
    #[error("network request failed: {0}")]
    Network(String),
    #[error("HTTP request returned status {0}")]
    HttpStatus(u16),
    #[error("post-install verification failed: {0}")]
    PostInstallVerification(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreUpdateCheck {
    pub current_version: semver::Version,
    pub latest_version: semver::Version,
    pub kind: version::UpdateKind,
    pub update_available: bool,
    pub install_allowed_without_confirmation: bool,
    pub channel: version::ReleaseChannel,
    pub asset_name: String,
    pub checksum_asset_name: String,
    pub release: github_release::ReleaseMetadata,
}

pub fn build_update_check(
    current_version: &str,
    releases: &[github_release::ReleaseMetadata],
    channel: version::ReleaseChannel,
    policy: version::UpdatePolicy,
    platform: &platform::PlatformAsset,
) -> Result<CoreUpdateCheck, UpdateError> {
    let current_version = version::parse_version(current_version)?;
    let release = github_release::select_latest_release(releases, channel)?;
    let latest_version = version::parse_version(&release.tag_name)?;
    let comparison = version::compare_versions(&current_version, &latest_version, policy);
    let asset = github_release::find_asset(&release, &platform.asset_name)?;
    let checksum = github_release::find_checksum_asset(&release)?;

    Ok(CoreUpdateCheck {
        current_version,
        latest_version,
        kind: comparison.kind,
        update_available: comparison.update_available,
        install_allowed_without_confirmation: comparison.install_allowed_without_confirmation,
        channel,
        asset_name: asset.name.clone(),
        checksum_asset_name: checksum.name.clone(),
        release,
    })
}

pub async fn download_url_bytes(url: &str) -> Result<Vec<u8>, UpdateError> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .user_agent("axiom-agent-updater")
        .build()
        .map_err(|error| UpdateError::Network(error.to_string()))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| UpdateError::Network(error.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(UpdateError::HttpStatus(status.as_u16()));
    }
    response
        .bytes()
        .await
        .map(|bytes| bytes.to_vec())
        .map_err(|error| UpdateError::Network(error.to_string()))
}

pub use checksum::{expected_sha256_for_asset, sha256_hex, verify_asset_from_sums, verify_sha256};
pub use github_release::{
    find_asset, find_checksum_asset, github_releases_api_url, parse_releases_json,
    select_latest_release, GitHubReleaseClient, ReleaseAsset, ReleaseCheck, ReleaseMetadata,
};
pub use installer::{
    backup_path_for, detect_installation_mode, ensure_install_allowed, install_staged_update,
    rollback_update, run_binary_version_check, stage_verified_update, InstallOutcome,
    InstallationMode, RollbackOutcome, StageUpdateRequest, StagedUpdate,
};
pub use platform::{current_platform_asset, resolve_platform_asset, PlatformAsset};
pub use state::{now_timestamp, UpdateDirs, UpdateState, UpdateStatus};
pub use version::{
    compare_versions, is_newer_version, parse_version, ReleaseChannel, UpdateKind, UpdatePolicy,
    VersionComparison,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_update_check_reports_notify_only() {
        let releases = parse_releases_json(mock_releases()).expect("releases");
        let platform = resolve_platform_asset("linux", "x86_64").expect("platform");

        let check = build_update_check(
            "0.1.0",
            &releases,
            ReleaseChannel::Stable,
            UpdatePolicy::Notify,
            &platform,
        )
        .expect("check");

        assert!(check.update_available);
        assert_eq!(check.kind, UpdateKind::Patch);
        assert!(!check.install_allowed_without_confirmation);
    }

    #[test]
    fn build_update_check_allows_auto_patch_only_for_patch() {
        let releases = parse_releases_json(mock_releases()).expect("releases");
        let platform = resolve_platform_asset("linux", "x86_64").expect("platform");

        let check = build_update_check(
            "0.1.0",
            &releases,
            ReleaseChannel::Stable,
            UpdatePolicy::AutoPatch,
            &platform,
        )
        .expect("check");

        assert!(check.install_allowed_without_confirmation);
    }

    fn mock_releases() -> &'static str {
        r#"[
  {
    "tag_name": "v0.1.1",
    "prerelease": false,
    "draft": false,
    "assets": [
      {"name": "axiom-x86_64-unknown-linux-gnu", "browser_download_url": "https://example.com/linux"},
      {"name": "SHA256SUMS", "browser_download_url": "https://example.com/sums"}
    ]
  }
]"#
    }
}
