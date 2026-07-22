use serde::{Deserialize, Serialize};

use crate::{
    parse_version, read_bounded_response, resolve_public_addresses, ReleaseChannel, UpdateError,
    MAX_RELEASE_METADATA_BYTES,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    #[serde(default)]
    pub browser_download_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseMetadata {
    #[serde(default)]
    pub name: Option<String>,
    pub tag_name: String,
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub draft: bool,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseCheck {
    pub release: ReleaseMetadata,
    pub latest_version: semver::Version,
    pub update_available: bool,
}

#[derive(Debug, Clone)]
pub struct GitHubReleaseClient {
    repo_url: String,
    timeout_secs: u64,
}

impl GitHubReleaseClient {
    pub fn new(repo_url: impl Into<String>) -> Self {
        Self {
            repo_url: repo_url.into(),
            timeout_secs: 15,
        }
    }

    pub fn with_timeout(mut self, timeout_secs: u64) -> Self {
        self.timeout_secs = timeout_secs;
        self
    }

    pub async fn fetch_releases(&self) -> Result<Vec<ReleaseMetadata>, UpdateError> {
        let timeout = std::time::Duration::from_secs(self.timeout_secs);
        tokio::time::timeout(timeout, self.fetch_releases_inner())
            .await
            .map_err(|_| {
                UpdateError::Network(format!(
                    "GitHub release metadata request timed out after {} seconds",
                    self.timeout_secs
                ))
            })?
    }

    async fn fetch_releases_inner(&self) -> Result<Vec<ReleaseMetadata>, UpdateError> {
        let api_url = reqwest::Url::parse(&github_releases_api_url(&self.repo_url)?)
            .map_err(|error| UpdateError::InvalidReleaseRepo(error.to_string()))?;
        let host = api_url
            .host_str()
            .ok_or_else(|| UpdateError::InvalidReleaseRepo(self.repo_url.clone()))?
            .to_string();
        let port = api_url
            .port_or_known_default()
            .ok_or_else(|| UpdateError::InvalidReleaseRepo(self.repo_url.clone()))?;
        let addresses = resolve_public_addresses(&host, port).await?;
        let client = reqwest::Client::builder()
            .user_agent("axiom-agent-updater")
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .resolve_to_addrs(&host, &addresses)
            .build()
            .map_err(|error| UpdateError::Network(error.to_string()))?;
        let response = client
            .get(api_url)
            .send()
            .await
            .map_err(|error| UpdateError::Network(error.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(UpdateError::HttpStatus(status.as_u16()));
        }
        let body = read_bounded_response(response, MAX_RELEASE_METADATA_BYTES).await?;
        serde_json::from_slice(&body).map_err(|error| UpdateError::ReleaseJson(error.to_string()))
    }
}

pub fn github_releases_api_url(repo_url: &str) -> Result<String, UpdateError> {
    let (owner, repo) = parse_github_repo_url(repo_url)?;
    Ok(format!(
        "https://api.github.com/repos/{owner}/{repo}/releases"
    ))
}

pub fn validate_release_asset_url(
    repo_url: &str,
    tag_name: &str,
    asset_name: &str,
    asset_url: &str,
) -> Result<(), UpdateError> {
    let (owner, repo) = parse_github_repo_url(repo_url)?;
    if parse_version(tag_name).is_err()
        || !valid_release_path_component(tag_name)
        || !valid_release_asset_name(asset_name)
    {
        return Err(UpdateError::InvalidDownloadUrl(
            "release tag or asset name is not safe for an exact GitHub download path".to_string(),
        ));
    }
    let expected =
        format!("https://github.com/{owner}/{repo}/releases/download/{tag_name}/{asset_name}");
    if asset_url != expected {
        return Err(UpdateError::InvalidDownloadUrl(format!(
            "release asset URL does not match the configured repository, selected tag, and exact asset name (expected {expected})"
        )));
    }
    Ok(())
}

fn parse_github_repo_url(repo_url: &str) -> Result<(String, String), UpdateError> {
    let invalid = || UpdateError::InvalidReleaseRepo(repo_url.to_string());
    if repo_url.trim() != repo_url
        || repo_url.contains('\\')
        || !repo_url.starts_with("https://github.com/")
    {
        return Err(invalid());
    }

    let parsed = reqwest::Url::parse(repo_url).map_err(|_| invalid())?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some("github.com")
        || parsed.port().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(invalid());
    }

    let path = repo_url
        .strip_prefix("https://github.com/")
        .ok_or_else(invalid)?;
    let mut parts = path.split('/');
    let owner = parts.next().ok_or_else(invalid)?;
    let repo = parts.next().ok_or_else(invalid)?;
    if parts.next().is_some() || !valid_github_owner(owner) || !valid_github_repo(repo) {
        return Err(invalid());
    }
    Ok((owner.to_string(), repo.to_string()))
}

fn valid_github_owner(owner: &str) -> bool {
    (1..=39).contains(&owner.len())
        && owner.is_ascii()
        && owner
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        && owner
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && owner
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && !owner.contains("--")
}

fn valid_github_repo(repo: &str) -> bool {
    (1..=100).contains(&repo.len())
        && !matches!(repo, "." | "..")
        && repo.is_ascii()
        && repo
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_release_path_component(value: &str) -> bool {
    !value.is_empty()
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+'))
}

fn valid_release_asset_name(value: &str) -> bool {
    !matches!(value, "" | "." | "..") && valid_release_path_component(value)
}

pub fn parse_releases_json(input: &str) -> Result<Vec<ReleaseMetadata>, UpdateError> {
    serde_json::from_str(input).map_err(|error| UpdateError::ReleaseJson(error.to_string()))
}

pub fn select_latest_release(
    releases: &[ReleaseMetadata],
    channel: ReleaseChannel,
) -> Result<ReleaseMetadata, UpdateError> {
    let candidates = match channel {
        ReleaseChannel::Stable => releases
            .iter()
            .filter(|release| !release.draft && !release.prerelease)
            .collect::<Vec<_>>(),
        ReleaseChannel::Nightly => {
            let prereleases = releases
                .iter()
                .filter(|release| !release.draft && release.prerelease)
                .collect::<Vec<_>>();
            if prereleases.is_empty() {
                releases
                    .iter()
                    .filter(|release| !release.draft)
                    .collect::<Vec<_>>()
            } else {
                prereleases
            }
        }
        ReleaseChannel::Dev => releases.iter().filter(|release| !release.draft).collect(),
    };

    candidates
        .into_iter()
        .filter_map(|release| {
            parse_version(&release.tag_name)
                .ok()
                .map(|version| (version, release))
        })
        .max_by(|(left, _), (right, _)| left.cmp(right))
        .map(|(_, release)| release.clone())
        .ok_or(UpdateError::NoReleaseForChannel(channel))
}

pub fn find_asset<'a>(
    release: &'a ReleaseMetadata,
    asset_name: &str,
) -> Result<&'a ReleaseAsset, UpdateError> {
    let mut matching = release
        .assets
        .iter()
        .filter(|asset| asset.name == asset_name);
    let asset = matching
        .next()
        .ok_or_else(|| UpdateError::MissingAsset(asset_name.to_string()))?;
    if matching.next().is_some() {
        return Err(UpdateError::DuplicateAsset(asset_name.to_string()));
    }
    Ok(asset)
}

pub fn find_checksum_asset(release: &ReleaseMetadata) -> Result<&ReleaseAsset, UpdateError> {
    find_asset(release, "SHA256SUMS")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mock_release_metadata() {
        let releases = parse_releases_json(mock_releases()).expect("parse");

        assert_eq!(releases.len(), 3);
        assert_eq!(releases[0].tag_name, "v0.2.0-nightly.1");
        assert!(releases[0].prerelease);
    }

    #[test]
    fn stable_channel_ignores_prerelease() {
        let releases = parse_releases_json(mock_releases()).expect("parse");
        let selected = select_latest_release(&releases, ReleaseChannel::Stable).expect("selected");

        assert_eq!(selected.tag_name, "v0.1.1");
        assert!(!selected.prerelease);
    }

    #[test]
    fn nightly_channel_can_use_prerelease() {
        let releases = parse_releases_json(mock_releases()).expect("parse");
        let selected = select_latest_release(&releases, ReleaseChannel::Nightly).expect("selected");

        assert_eq!(selected.tag_name, "v0.2.0-nightly.1");
        assert!(selected.prerelease);
    }

    #[test]
    fn asset_lookup_success_and_failure() {
        let releases = parse_releases_json(mock_releases()).expect("parse");
        let release = select_latest_release(&releases, ReleaseChannel::Stable).expect("selected");

        assert!(find_asset(&release, "axiom-x86_64-unknown-linux-gnu").is_ok());
        assert!(matches!(
            find_asset(&release, "missing"),
            Err(UpdateError::MissingAsset(_))
        ));
        assert!(find_checksum_asset(&release).is_ok());
    }

    #[test]
    fn builds_github_releases_api_url() {
        assert_eq!(
            github_releases_api_url("https://github.com/NexaraAI/axiom-agent").expect("url"),
            "https://api.github.com/repos/NexaraAI/axiom-agent/releases"
        );
    }

    #[test]
    fn github_repo_url_is_exact_https_owner_and_repo() {
        for invalid in [
            "http://github.com/NexaraAI/axiom-agent",
            "https://user@github.com/NexaraAI/axiom-agent",
            "https://github.com:443/NexaraAI/axiom-agent",
            "https://github.com/NexaraAI/axiom-agent/",
            "https://github.com/NexaraAI/axiom-agent/releases",
            "https://github.com/NexaraAI/./axiom-agent",
            "https://github.com/NexaraAI/other/../axiom-agent",
            "https://github.com/NexaraAI/axiom-agent?tab=readme",
            "https://github.com/NexaraAI/axiom-agent#readme",
            "https://github.com/NexaraAI",
            "https://github.com/-invalid/axiom-agent",
            "https://github.com/invalid-/axiom-agent",
            "https://github.com/invalid--owner/axiom-agent",
            "https://github.com/NexaraAI/axiom%2Fagent",
            " https://github.com/NexaraAI/axiom-agent",
        ] {
            assert!(
                matches!(
                    github_releases_api_url(invalid),
                    Err(UpdateError::InvalidReleaseRepo(_))
                ),
                "{invalid} should be rejected"
            );
        }
    }

    #[test]
    fn duplicate_exact_release_assets_fail_closed() {
        let release = ReleaseMetadata {
            name: None,
            tag_name: "v1.0.0".to_string(),
            prerelease: false,
            draft: false,
            html_url: None,
            body: None,
            assets: vec![
                ReleaseAsset {
                    name: "SHA256SUMS".to_string(),
                    browser_download_url: "https://github.com/a/b/releases/download/v1/SHA256SUMS"
                        .to_string(),
                },
                ReleaseAsset {
                    name: "SHA256SUMS".to_string(),
                    browser_download_url: "https://github.com/a/b/releases/download/v1/SHA256SUMS"
                        .to_string(),
                },
            ],
        };

        assert!(matches!(
            find_checksum_asset(&release),
            Err(UpdateError::DuplicateAsset(name)) if name == "SHA256SUMS"
        ));
    }

    #[test]
    fn release_asset_url_is_bound_to_repo_tag_and_exact_name() {
        let repo = "https://github.com/NexaraAI/axiom-agent";
        let exact = "https://github.com/NexaraAI/axiom-agent/releases/download/v1.0.0/axiom.exe";
        validate_release_asset_url(repo, "v1.0.0", "axiom.exe", exact).expect("exact URL");

        for invalid in [
            "https://github.com/Other/axiom-agent/releases/download/v1.0.0/axiom.exe",
            "https://github.com/NexaraAI/axiom-agent/releases/download/v2.0.0/axiom.exe",
            "https://github.com/NexaraAI/axiom-agent/releases/download/v1.0.0/other.exe",
            "https://objects.githubusercontent.com/github-production-release-asset/1/2",
            "https://github.com/NexaraAI/axiom-agent/releases/download/v1.0.0/axiom.exe?x=1",
        ] {
            assert!(
                matches!(
                    validate_release_asset_url(repo, "v1.0.0", "axiom.exe", invalid),
                    Err(UpdateError::InvalidDownloadUrl(_))
                ),
                "{invalid} should be rejected"
            );
        }

        assert!(matches!(
            validate_release_asset_url(
                repo,
                "..",
                "axiom.exe",
                "https://github.com/NexaraAI/axiom-agent/releases/download/../axiom.exe"
            ),
            Err(UpdateError::InvalidDownloadUrl(_))
        ));
    }

    fn mock_releases() -> &'static str {
        r#"[
  {
    "tag_name": "v0.2.0-nightly.1",
    "prerelease": true,
    "draft": false,
    "assets": [
      {"name": "axiom-x86_64-unknown-linux-gnu", "browser_download_url": "https://github.com/NexaraAI/axiom-agent/releases/download/v0.2.0-nightly.1/axiom-x86_64-unknown-linux-gnu"},
      {"name": "SHA256SUMS", "browser_download_url": "https://github.com/NexaraAI/axiom-agent/releases/download/v0.2.0-nightly.1/SHA256SUMS"}
    ]
  },
  {
    "tag_name": "v0.1.1",
    "prerelease": false,
    "draft": false,
    "assets": [
      {"name": "axiom-x86_64-unknown-linux-gnu", "browser_download_url": "https://github.com/NexaraAI/axiom-agent/releases/download/v0.1.1/axiom-x86_64-unknown-linux-gnu"},
      {"name": "SHA256SUMS", "browser_download_url": "https://github.com/NexaraAI/axiom-agent/releases/download/v0.1.1/SHA256SUMS"}
    ]
  },
  {
    "tag_name": "v0.1.2-beta.1",
    "prerelease": true,
    "draft": true,
    "assets": []
  }
]"#
    }
}
