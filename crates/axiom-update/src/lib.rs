pub mod checksum;
pub mod github_release;
pub mod installer;
pub mod platform;
pub mod state;
pub mod version;

use thiserror::Error;

pub const CHECKSUM_FILE_NAME: &str = "SHA256SUMS";
pub const MAX_UPDATE_BINARY_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_UPDATE_CHECKSUM_BYTES: usize = 1024 * 1024;
pub(crate) const MAX_RELEASE_METADATA_BYTES: usize = 4 * 1024 * 1024;

const MAX_UPDATE_REDIRECTS: usize = 5;
const UPDATE_DOWNLOAD_TIMEOUT_SECS: u64 = 60;

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
    #[error("release metadata contains duplicate asset entries: {0}")]
    DuplicateAsset(String),
    #[error("release asset name is unsafe: {0}")]
    UnsafeAssetName(String),
    #[error("checksum not found for asset. Update blocked: {0}")]
    MissingChecksum(String),
    #[error("invalid SHA256SUMS manifest: {0}")]
    InvalidChecksumManifest(String),
    #[error("SHA256SUMS contains a duplicate asset entry: {0}")]
    DuplicateChecksumEntry(String),
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
    #[error("invalid update download URL: {0}")]
    InvalidDownloadUrl(String),
    #[error("update download resolved to a blocked private/reserved address: {0}")]
    PrivateDownloadAddress(String),
    #[error("update download is too large: {bytes} bytes exceeds limit {limit} bytes")]
    DownloadTooLarge { bytes: usize, limit: usize },
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

pub async fn download_release_asset_bytes(
    repo_url: &str,
    tag_name: &str,
    asset_name: &str,
    url: &str,
) -> Result<Vec<u8>, UpdateError> {
    github_release::validate_release_asset_url(repo_url, tag_name, asset_name, url)?;
    download_update_bytes(url, MAX_UPDATE_BINARY_BYTES).await
}

pub async fn download_release_checksum_bytes(
    repo_url: &str,
    tag_name: &str,
    url: &str,
) -> Result<Vec<u8>, UpdateError> {
    github_release::validate_release_asset_url(repo_url, tag_name, CHECKSUM_FILE_NAME, url)?;
    download_update_bytes(url, MAX_UPDATE_CHECKSUM_BYTES).await
}

async fn download_update_bytes(url: &str, limit: usize) -> Result<Vec<u8>, UpdateError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|error| UpdateError::InvalidDownloadUrl(error.to_string()))?;
    validate_github_download_url(&parsed)?;

    tokio::time::timeout(
        std::time::Duration::from_secs(UPDATE_DOWNLOAD_TIMEOUT_SECS),
        download_update_bytes_inner(parsed, limit),
    )
    .await
    .map_err(|_| {
        UpdateError::Network(format!(
            "update download timed out after {UPDATE_DOWNLOAD_TIMEOUT_SECS} seconds"
        ))
    })?
}

async fn download_update_bytes_inner(
    mut current_url: reqwest::Url,
    limit: usize,
) -> Result<Vec<u8>, UpdateError> {
    for redirect_count in 0..=MAX_UPDATE_REDIRECTS {
        validate_github_download_url(&current_url)?;
        let host = current_url
            .host_str()
            .ok_or_else(|| UpdateError::InvalidDownloadUrl("host is required".to_string()))?
            .to_string();
        let port = current_url
            .port_or_known_default()
            .ok_or_else(|| UpdateError::InvalidDownloadUrl("port is required".to_string()))?;
        let addresses = resolve_public_addresses(&host, port).await?;

        let client = reqwest::Client::builder()
            .user_agent("axiom-agent-updater")
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .resolve_to_addrs(&host, &addresses)
            .build()
            .map_err(|error| UpdateError::Network(error.to_string()))?;
        let response = client
            .get(current_url.clone())
            .send()
            .await
            .map_err(|error| UpdateError::Network(error.to_string()))?;

        if response.status().is_redirection() {
            if redirect_count == MAX_UPDATE_REDIRECTS {
                return Err(UpdateError::InvalidDownloadUrl(format!(
                    "more than {MAX_UPDATE_REDIRECTS} redirects"
                )));
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .ok_or_else(|| {
                    UpdateError::InvalidDownloadUrl(
                        "redirect response did not include a Location header".to_string(),
                    )
                })?
                .to_str()
                .map_err(|_| {
                    UpdateError::InvalidDownloadUrl(
                        "redirect Location header is not valid UTF-8".to_string(),
                    )
                })?;
            current_url = current_url.join(location).map_err(|error| {
                UpdateError::InvalidDownloadUrl(format!("invalid redirect target: {error}"))
            })?;
            continue;
        }

        let status = response.status();
        if !status.is_success() {
            return Err(UpdateError::HttpStatus(status.as_u16()));
        }
        return read_bounded_response(response, limit).await;
    }

    Err(UpdateError::InvalidDownloadUrl(
        "redirect limit exhausted".to_string(),
    ))
}

fn validate_github_download_url(url: &reqwest::Url) -> Result<(), UpdateError> {
    if url.scheme() != "https" {
        return Err(UpdateError::InvalidDownloadUrl(
            "update assets must use HTTPS".to_string(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(UpdateError::InvalidDownloadUrl(
            "embedded URL credentials are not allowed".to_string(),
        ));
    }
    if url.fragment().is_some() {
        return Err(UpdateError::InvalidDownloadUrl(
            "URL fragments are not allowed".to_string(),
        ));
    }
    if url.port_or_known_default() != Some(443) {
        return Err(UpdateError::InvalidDownloadUrl(
            "update assets must use HTTPS port 443".to_string(),
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| UpdateError::InvalidDownloadUrl("host is required".to_string()))?;
    if !trusted_github_download_host(host) {
        return Err(UpdateError::InvalidDownloadUrl(format!(
            "untrusted update download host: {host}"
        )));
    }
    if blocked_download_host(host) {
        return Err(UpdateError::PrivateDownloadAddress(host.to_string()));
    }
    Ok(())
}

pub(crate) async fn resolve_public_addresses(
    host: &str,
    port: u16,
) -> Result<Vec<std::net::SocketAddr>, UpdateError> {
    if blocked_download_host(host) {
        return Err(UpdateError::PrivateDownloadAddress(host.to_string()));
    }
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| UpdateError::Network(format!("DNS resolution failed: {error}")))?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err(UpdateError::Network(
            "DNS resolution returned no addresses".to_string(),
        ));
    }
    if addresses
        .iter()
        .any(|address| blocked_download_address(address.ip()))
    {
        return Err(UpdateError::PrivateDownloadAddress(host.to_string()));
    }
    Ok(addresses)
}

pub(crate) async fn read_bounded_response(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>, UpdateError> {
    if let Some(length) = response.content_length() {
        let bytes = usize::try_from(length).unwrap_or(usize::MAX);
        if bytes > limit {
            return Err(UpdateError::DownloadTooLarge { bytes, limit });
        }
    }
    let mut bytes = Vec::with_capacity(
        response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or_default()
            .min(limit),
    );
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| UpdateError::Network(error.to_string()))?
    {
        let next_size = bytes.len().saturating_add(chunk.len());
        if next_size > limit {
            return Err(UpdateError::DownloadTooLarge {
                bytes: next_size,
                limit,
            });
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn trusted_github_download_host(host: &str) -> bool {
    host == "github.com"
        || host == "objects.githubusercontent.com"
        || host == "release-assets.githubusercontent.com"
}

fn blocked_download_host(host: &str) -> bool {
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if matches!(host.as_str(), "localhost" | "localhost.localdomain")
        || host.ends_with(".localhost")
        || host.ends_with(".local")
    {
        return true;
    }
    host.parse().is_ok_and(blocked_download_address)
}

fn blocked_download_address(address: std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(address) => {
            let [first, second, third, _] = address.octets();
            address.is_private()
                || address.is_loopback()
                || address.is_link_local()
                || address.is_broadcast()
                || address.is_documentation()
                || address.is_multicast()
                || address.is_unspecified()
                || first == 0
                || first >= 240
                || (first == 100 && (64..=127).contains(&second))
                || (first == 192 && second == 0 && third == 0)
        }
        std::net::IpAddr::V6(address) => {
            let segments = address.segments();
            address.is_loopback()
                || address.is_unspecified()
                || address.is_unique_local()
                || address.is_unicast_link_local()
                || address.is_multicast()
                || (segments[0] == 0x2001 && segments[1] == 0x0db8)
                || address
                    .to_ipv4_mapped()
                    .is_some_and(|ipv4| blocked_download_address(std::net::IpAddr::V4(ipv4)))
        }
    }
}

pub use checksum::{expected_sha256_for_asset, sha256_hex, verify_asset_from_sums, verify_sha256};
pub use github_release::{
    find_asset, find_checksum_asset, github_releases_api_url, parse_releases_json,
    select_latest_release, validate_release_asset_url, GitHubReleaseClient, ReleaseAsset,
    ReleaseCheck, ReleaseMetadata,
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

    #[test]
    fn update_download_network_policy_blocks_private_targets_and_scopes_redirect_hosts() {
        for host in [
            "localhost",
            "service.local",
            "127.0.0.1",
            "10.0.0.1",
            "[::1]",
        ] {
            assert!(blocked_download_host(host), "{host} should be blocked");
        }
        assert!(!blocked_download_host("github.com"));
        assert!(trusted_github_download_host("github.com"));
        assert!(trusted_github_download_host(
            "release-assets.githubusercontent.com"
        ));
        assert!(trusted_github_download_host(
            "objects.githubusercontent.com"
        ));
        assert!(!trusted_github_download_host("github.example.com"));
        assert!(!trusted_github_download_host("raw.githubusercontent.com"));
        assert!(!trusted_github_download_host(
            "attacker.githubusercontent.com"
        ));
    }

    #[test]
    fn update_download_url_requires_an_exact_reviewed_host() {
        for invalid in [
            "https://example.com/axiom",
            "https://raw.githubusercontent.com/a/b/main/axiom",
            "https://attacker.githubusercontent.com/axiom",
            "https://github.com.evil.example/axiom",
            "https://github.com:444/axiom",
            "https://user@github.com/axiom",
            "https://github.com/axiom#fragment",
        ] {
            let url = reqwest::Url::parse(invalid).expect("test URL");
            assert!(
                matches!(
                    validate_github_download_url(&url),
                    Err(UpdateError::InvalidDownloadUrl(_))
                ),
                "{invalid} should be rejected"
            );
        }

        for trusted in [
            "https://github.com/NexaraAI/axiom-agent/releases/download/v1/axiom",
            "https://objects.githubusercontent.com/github-production-release-asset/1/2?x=1",
            "https://release-assets.githubusercontent.com/github-production-release-asset/1/2?x=1",
        ] {
            let url = reqwest::Url::parse(trusted).expect("test URL");
            validate_github_download_url(&url).expect("reviewed host");
        }
    }

    #[tokio::test]
    async fn update_download_requires_https_before_network_access() {
        let error = download_update_bytes("http://example.com/axiom", MAX_UPDATE_BINARY_BYTES)
            .await
            .expect_err("plain HTTP must be rejected");
        assert!(matches!(error, UpdateError::InvalidDownloadUrl(_)));
    }

    fn mock_releases() -> &'static str {
        r#"[
  {
    "tag_name": "v0.1.1",
    "prerelease": false,
    "draft": false,
    "assets": [
      {"name": "axiom-x86_64-unknown-linux-gnu", "browser_download_url": "https://github.com/NexaraAI/axiom-agent/releases/download/v0.1.1/axiom-x86_64-unknown-linux-gnu"},
      {"name": "SHA256SUMS", "browser_download_url": "https://github.com/NexaraAI/axiom-agent/releases/download/v0.1.1/SHA256SUMS"}
    ]
  }
]"#
    }
}
