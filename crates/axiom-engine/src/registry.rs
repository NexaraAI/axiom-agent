use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::SkillManifest;

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse registry JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("failed to parse bundle TOML: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("manifest error: {0}")]
    Manifest(#[from] crate::manifest::ManifestError),
    #[error("skill is not in registry: {0}")]
    MissingSkill(String),
    #[error("bundle is not in registry: {0}")]
    MissingBundle(String),
    #[error("invalid registry URL: {0}")]
    InvalidRegistryUrl(String),
    #[error("HTTP registry request failed: {0}")]
    Http(String),
    #[error("HTTP registry returned status {status}: {url}")]
    HttpStatus { url: String, status: u16 },
    #[error("checksum mismatch for {id}")]
    ChecksumMismatch { id: String },
}

pub type Result<T> = std::result::Result<T, RegistryError>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryIndex {
    pub schema_version: String,
    pub name: String,
    pub updated_at: String,
    #[serde(default)]
    pub skills: Vec<RegistrySkillEntry>,
    #[serde(default)]
    pub bundles: Vec<RegistryBundleEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistrySkillEntry {
    pub id: String,
    pub version: Version,
    pub category: String,
    #[serde(default)]
    pub platforms: Vec<String>,
    pub manifest_url: String,
    #[serde(default, alias = "checksum")]
    pub sha256: Option<String>,
    pub min_axiom_version: Version,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryBundleEntry {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub bundle_url: String,
    #[serde(default, alias = "checksum")]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillBundle {
    pub id: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub platforms: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalRegistrySource {
    pub registry_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRegistrySource {
    pub registry_url: String,
    pub timeout_secs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrySource {
    Local(LocalRegistrySource),
    Http(HttpRegistrySource),
}

#[derive(Debug, Clone)]
pub struct RegistryClient {
    source: RegistrySource,
    index: RegistryIndex,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryResource {
    pub content: String,
    pub resolved_location: String,
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillUpdate {
    pub id: String,
    pub installed_version: Version,
    pub registry_version: Version,
}

impl RegistryIndex {
    pub fn skill_entry(&self, skill_id: &str) -> Option<&RegistrySkillEntry> {
        self.skills.iter().find(|entry| entry.id == skill_id)
    }

    pub fn bundle_entry(&self, bundle_id: &str) -> Option<&RegistryBundleEntry> {
        self.bundles.iter().find(|entry| entry.id == bundle_id)
    }

    pub fn list_skills(&self) -> &[RegistrySkillEntry] {
        &self.skills
    }

    pub fn list_bundles(&self) -> &[RegistryBundleEntry] {
        &self.bundles
    }
}

impl SkillBundle {
    pub fn parse_toml(content: &str) -> Result<Self> {
        Ok(toml::from_str(content)?)
    }

    pub fn load_from_path(path: impl AsRef<Path>) -> Result<Self> {
        let content = fs::read_to_string(path)?;
        Self::parse_toml(&content)
    }
}

impl RegistryClient {
    pub async fn from_source(source: RegistrySource) -> Result<Self> {
        let index = match &source {
            RegistrySource::Local(source) => load_registry_from_path(&source.registry_path)?,
            RegistrySource::Http(source) => {
                load_registry_from_url_with_timeout(&source.registry_url, source.timeout_secs)
                    .await?
            }
        };

        Ok(Self { source, index })
    }

    pub fn from_local_path(path: impl AsRef<Path>) -> Result<Self> {
        let source = LocalRegistrySource {
            registry_path: normalize_registry_path(path.as_ref()),
        };
        let index = load_registry_from_path(&source.registry_path)?;
        Ok(Self {
            source: RegistrySource::Local(source),
            index,
        })
    }

    pub async fn from_url(url: impl Into<String>) -> Result<Self> {
        Self::from_source(RegistrySource::Http(HttpRegistrySource {
            registry_url: url.into(),
            timeout_secs: 10,
        }))
        .await
    }

    pub fn index(&self) -> &RegistryIndex {
        &self.index
    }

    pub fn source(&self) -> &RegistrySource {
        &self.source
    }

    pub fn source_label(&self) -> &'static str {
        match self.source {
            RegistrySource::Local(_) => "local",
            RegistrySource::Http(_) => "remote",
        }
    }

    pub fn registry_location(&self) -> String {
        match &self.source {
            RegistrySource::Local(source) => source.registry_path.display().to_string(),
            RegistrySource::Http(source) => source.registry_url.clone(),
        }
    }

    pub fn list_skills(&self) -> &[RegistrySkillEntry] {
        self.index.list_skills()
    }

    pub fn list_bundles(&self) -> &[RegistryBundleEntry] {
        self.index.list_bundles()
    }

    pub fn search_skills(&self, query: &str) -> Vec<RegistrySkillEntry> {
        let query = query.to_ascii_lowercase();
        self.index
            .skills
            .iter()
            .filter(|entry| {
                entry.id.to_ascii_lowercase().contains(&query)
                    || entry.category.to_ascii_lowercase().contains(&query)
                    || entry
                        .platforms
                        .iter()
                        .any(|platform| platform.to_ascii_lowercase().contains(&query))
            })
            .cloned()
            .collect()
    }

    pub async fn fetch_skill_manifest(
        &self,
        skill_id: &str,
    ) -> Result<(SkillManifest, RegistryResource)> {
        let entry = self
            .index
            .skill_entry(skill_id)
            .ok_or_else(|| RegistryError::MissingSkill(skill_id.to_string()))?;
        let resource = self
            .fetch_resource(&entry.manifest_url, entry.sha256.as_deref(), &entry.id)
            .await?;
        let manifest = SkillManifest::parse_toml(&resource.content)?;
        Ok((manifest, resource))
    }

    pub async fn fetch_bundle(&self, bundle_id: &str) -> Result<(SkillBundle, RegistryResource)> {
        let entry = self
            .index
            .bundle_entry(bundle_id)
            .ok_or_else(|| RegistryError::MissingBundle(bundle_id.to_string()))?;
        let resource = self
            .fetch_resource(&entry.bundle_url, entry.sha256.as_deref(), &entry.id)
            .await?;
        let bundle = SkillBundle::parse_toml(&resource.content)?;
        Ok((bundle, resource))
    }

    async fn fetch_resource(
        &self,
        relative_or_url: &str,
        sha256: Option<&str>,
        id: &str,
    ) -> Result<RegistryResource> {
        let (content, location) = match &self.source {
            RegistrySource::Local(source) => {
                let registry_dir = source
                    .registry_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."));
                let path = registry_dir.join(relative_or_url);
                (fs::read_to_string(&path)?, path.display().to_string())
            }
            RegistrySource::Http(source) => {
                let url = resolve_registry_relative_url(&source.registry_url, relative_or_url)?;
                let content = fetch_url_text(&url, source.timeout_secs).await?;
                (content, url)
            }
        };

        if let Some(expected) = sha256 {
            verify_sha256(id, content.as_bytes(), expected)?;
        }

        Ok(RegistryResource {
            content,
            resolved_location: location,
            sha256: sha256.map(ToString::to_string),
        })
    }
}

pub fn load_registry_from_path(path: impl AsRef<Path>) -> Result<RegistryIndex> {
    let path = normalize_registry_path(path.as_ref());
    let content = fs::read_to_string(path)?;
    Ok(serde_json::from_str(&content)?)
}

pub async fn load_registry_from_url(url: &str) -> Result<RegistryIndex> {
    load_registry_from_url_with_timeout(url, 10).await
}

async fn load_registry_from_url_with_timeout(
    url: &str,
    timeout_secs: u64,
) -> Result<RegistryIndex> {
    validate_remote_registry_url(url)?;
    let content = fetch_url_text(url, timeout_secs).await?;
    Ok(serde_json::from_str(&content)?)
}

pub fn resolve_registry_relative_url(base: &str, relative_or_url: &str) -> Result<String> {
    validate_remote_registry_url(base)?;
    if relative_or_url.starts_with("https://") || relative_or_url.starts_with("http://") {
        validate_remote_registry_url(relative_or_url)?;
        return Ok(relative_or_url.to_string());
    }

    let base = reqwest::Url::parse(base)
        .map_err(|error| RegistryError::InvalidRegistryUrl(error.to_string()))?;
    let resolved = base
        .join(relative_or_url)
        .map_err(|error| RegistryError::InvalidRegistryUrl(error.to_string()))?;
    validate_remote_registry_url(resolved.as_str())?;
    Ok(resolved.to_string())
}

pub fn validate_remote_registry_url(url: &str) -> Result<()> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|error| RegistryError::InvalidRegistryUrl(error.to_string()))?;
    match parsed.scheme() {
        "https" => Ok(()),
        "http" if is_localhost(&parsed) => Ok(()),
        _ => Err(RegistryError::InvalidRegistryUrl(
            "remote registries must use HTTPS, except localhost HTTP for development".to_string(),
        )),
    }
}

pub fn verify_sha256(id: &str, bytes: &[u8], expected: &str) -> Result<()> {
    let digest = Sha256::digest(bytes);
    let actual = format!("{digest:x}");
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(RegistryError::ChecksumMismatch { id: id.to_string() })
    }
}

pub fn check_skill_updates(
    installed: &crate::InstalledSkills,
    registry: &RegistryIndex,
) -> Vec<SkillUpdate> {
    installed
        .skills
        .values()
        .filter_map(|record| {
            let entry = registry.skill_entry(&record.id)?;
            (entry.version > record.version).then_some(SkillUpdate {
                id: record.id.clone(),
                installed_version: record.version.clone(),
                registry_version: entry.version.clone(),
            })
        })
        .collect()
}

pub fn essential_bundle_id_for_os(os: &str) -> Option<&'static str> {
    match os {
        "windows" => Some("essential.windows"),
        "linux" => Some("essential.linux"),
        "macos" => Some("essential.macos"),
        _ => None,
    }
}

pub fn current_essential_bundle_id() -> Option<&'static str> {
    essential_bundle_id_for_os(std::env::consts::OS)
}

fn normalize_registry_path(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.join("registry.json")
    } else {
        path.to_path_buf()
    }
}

async fn fetch_url_text(url: &str, timeout_secs: u64) -> Result<String> {
    validate_remote_registry_url(url)?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(timeout_secs))
        .build()
        .map_err(|error| RegistryError::Http(error.to_string()))?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|error| RegistryError::Http(error.to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(RegistryError::HttpStatus {
            url: url.to_string(),
            status: status.as_u16(),
        });
    }
    response
        .text()
        .await
        .map_err(|error| RegistryError::Http(error.to_string()))
}

fn is_localhost(url: &reqwest::Url) -> bool {
    matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stage7_registry_index() {
        let registry: RegistryIndex = serde_json::from_str(
            r#"{
                "schema_version": "0.1",
                "name": "Axiom Skills Registry",
                "updated_at": "2026-01-01T00:00:00Z",
                "skills": [
                    {
                        "id": "file.read",
                        "version": "0.1.0",
                        "category": "filesystem",
                        "platforms": ["windows", "linux", "macos"],
                        "manifest_url": "skills/file.read/skill.toml",
                        "min_axiom_version": "0.1.0"
                    }
                ],
                "bundles": [
                    {
                        "id": "essential.windows",
                        "name": "Essential Windows Skills",
                        "platform": "windows",
                        "bundle_url": "bundles/essential.windows.toml"
                    }
                ]
            }"#,
        )
        .expect("parse registry");

        assert_eq!(registry.schema_version, "0.1");
        assert!(registry.skill_entry("file.read").is_some());
        assert!(registry.bundle_entry("essential.windows").is_some());
    }

    #[test]
    fn resolves_relative_urls_against_registry_url() {
        let resolved = resolve_registry_relative_url(
            "https://raw.githubusercontent.com/NexaraAI/axiom-skills/main/registry.json",
            "skills/file.read/skill.toml",
        )
        .expect("resolve URL");

        assert_eq!(
            resolved,
            "https://raw.githubusercontent.com/NexaraAI/axiom-skills/main/skills/file.read/skill.toml"
        );
    }

    #[test]
    fn rejects_non_https_remote_url_except_localhost() {
        assert!(validate_remote_registry_url("http://example.com/registry.json").is_err());
        assert!(validate_remote_registry_url("http://localhost:8080/registry.json").is_ok());
        assert!(validate_remote_registry_url("https://example.com/registry.json").is_ok());
    }

    #[test]
    fn verifies_checksum_success_and_failure() {
        let digest = Sha256::digest(b"hello");
        let expected = format!("{digest:x}");

        verify_sha256("test", b"hello", &expected).expect("checksum ok");
        assert!(matches!(
            verify_sha256("test", b"hello", "bad"),
            Err(RegistryError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn loads_local_fixture_registry() {
        let client = RegistryClient::from_local_path(fixture_registry_root()).expect("client");

        assert!(client.index().skill_entry("python.write").is_some());
        assert!(client.index().bundle_entry("essential.windows").is_some());
    }

    #[test]
    fn loads_local_bundle() {
        let bundle = SkillBundle::load_from_path(
            fixture_registry_root().join("bundles/essential.windows.toml"),
        )
        .expect("bundle");

        assert!(bundle.skills.contains(&"file.read".to_string()));
    }

    #[tokio::test]
    async fn fetches_local_skill_manifest() {
        let client = RegistryClient::from_local_path(fixture_registry_root()).expect("client");
        let (manifest, resource) = client
            .fetch_skill_manifest("python.write")
            .await
            .expect("manifest");

        assert_eq!(manifest.id, "python.write");
        assert!(resource.resolved_location.contains("python.write"));
    }

    #[test]
    fn registry_search_matches_keyword_category_and_id() {
        let client = RegistryClient::from_local_path(fixture_registry_root()).expect("client");

        assert!(client
            .search_skills("python")
            .iter()
            .any(|entry| entry.id == "python.write"));
        assert!(client
            .search_skills("filesystem")
            .iter()
            .any(|entry| entry.id == "file.read"));
    }

    #[test]
    fn update_check_detects_newer_version() {
        let registry: RegistryIndex = serde_json::from_str(
            r#"{
                "schema_version": "0.1",
                "name": "Test",
                "updated_at": "2026-01-01T00:00:00Z",
                "skills": [
                    {
                        "id": "python.write",
                        "version": "0.2.0",
                        "category": "coding",
                        "platforms": ["windows"],
                        "manifest_url": "skills/python.write/skill.toml",
                        "min_axiom_version": "0.1.0"
                    }
                ],
                "bundles": []
            }"#,
        )
        .expect("registry");
        let mut installed = crate::InstalledSkills::default();
        installed.upsert(crate::InstalledSkillRecord {
            id: "python.write".to_string(),
            version: Version::new(0, 1, 0),
            installed_at: "test".to_string(),
            source: "local".to_string(),
            registry_url: None,
            manifest_url: None,
            checksum: None,
            enabled: true,
        });

        let updates = check_skill_updates(&installed, &registry);

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].registry_version, Version::new(0, 2, 0));
    }

    #[test]
    fn selects_os_specific_essential_bundle() {
        assert_eq!(
            essential_bundle_id_for_os("windows"),
            Some("essential.windows")
        );
        assert_eq!(essential_bundle_id_for_os("linux"), Some("essential.linux"));
        assert_eq!(essential_bundle_id_for_os("macos"), Some("essential.macos"));
        assert_eq!(essential_bundle_id_for_os("freebsd"), None);
    }

    fn fixture_registry_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/skill-registry")
    }
}
