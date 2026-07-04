use serde::{Deserialize, Serialize};

use crate::{parse_version, ReleaseChannel, UpdateError};

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
        let api_url = github_releases_api_url(&self.repo_url)?;
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(self.timeout_secs))
            .user_agent("axiom-agent-updater")
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
        response
            .json::<Vec<ReleaseMetadata>>()
            .await
            .map_err(|error| UpdateError::ReleaseJson(error.to_string()))
    }
}

pub fn github_releases_api_url(repo_url: &str) -> Result<String, UpdateError> {
    let trimmed = repo_url.trim().trim_end_matches('/');
    let without_scheme = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
        .ok_or_else(|| UpdateError::InvalidReleaseRepo(repo_url.to_string()))?;
    let mut parts = without_scheme.split('/');
    let owner = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| UpdateError::InvalidReleaseRepo(repo_url.to_string()))?;
    let repo = parts
        .next()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| UpdateError::InvalidReleaseRepo(repo_url.to_string()))?;
    Ok(format!(
        "https://api.github.com/repos/{owner}/{repo}/releases"
    ))
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
    release
        .assets
        .iter()
        .find(|asset| asset.name == asset_name)
        .ok_or_else(|| UpdateError::MissingAsset(asset_name.to_string()))
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

    fn mock_releases() -> &'static str {
        r#"[
  {
    "tag_name": "v0.2.0-nightly.1",
    "prerelease": true,
    "draft": false,
    "assets": [
      {"name": "axiom-x86_64-unknown-linux-gnu", "browser_download_url": "https://example.com/nightly-linux"},
      {"name": "SHA256SUMS", "browser_download_url": "https://example.com/nightly-sums"}
    ]
  },
  {
    "tag_name": "v0.1.1",
    "prerelease": false,
    "draft": false,
    "assets": [
      {"name": "axiom-x86_64-unknown-linux-gnu", "browser_download_url": "https://example.com/linux"},
      {"name": "SHA256SUMS", "browser_download_url": "https://example.com/sums"}
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
