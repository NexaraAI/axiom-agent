use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use semver::Version;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    manifest::{ManifestError, SkillManifest},
    registry::{RegistryClient, RegistryError},
};

#[derive(Debug, Error)]
pub enum InstalledSkillError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse installed_skills.json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("registry error: {0}")]
    Registry(#[from] RegistryError),
    #[error("manifest error: {0}")]
    Manifest(#[from] ManifestError),
}

pub type Result<T> = std::result::Result<T, InstalledSkillError>;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledSkills {
    #[serde(default)]
    pub skills: BTreeMap<String, InstalledSkillRecord>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledSkillRecord {
    pub id: String,
    pub version: Version,
    pub installed_at: String,
    pub source: String,
    #[serde(default)]
    pub registry_url: Option<String>,
    #[serde(default)]
    pub manifest_url: Option<String>,
    #[serde(default)]
    pub checksum: Option<String>,
    pub enabled: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InstalledSkill {
    pub record: InstalledSkillRecord,
    pub manifest: SkillManifest,
}

impl InstalledSkills {
    pub fn load_from_dir(skills_dir: impl AsRef<Path>) -> Result<Self> {
        let path = installed_skills_path(skills_dir.as_ref());
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&content)?)
    }

    pub fn save_to_dir(&self, skills_dir: impl AsRef<Path>) -> Result<()> {
        let skills_dir = skills_dir.as_ref();
        fs::create_dir_all(skills_dir)?;
        let path = installed_skills_path(skills_dir);
        let content = serde_json::to_string_pretty(self)?;
        fs::write(path, content)?;
        Ok(())
    }

    pub fn upsert(&mut self, record: InstalledSkillRecord) {
        self.skills.insert(record.id.clone(), record);
    }

    pub fn enabled_records(&self) -> impl Iterator<Item = &InstalledSkillRecord> {
        self.skills.values().filter(|record| record.enabled)
    }
}

pub fn load_installed_skills(skills_dir: impl AsRef<Path>) -> Result<Vec<InstalledSkill>> {
    let skills_dir = skills_dir.as_ref();
    let installed = InstalledSkills::load_from_dir(skills_dir)?;
    let mut skills = Vec::new();

    for record in installed.skills.values() {
        let manifest_path = skills_dir.join(&record.id).join("skill.toml");
        if manifest_path.exists() {
            skills.push(InstalledSkill {
                record: record.clone(),
                manifest: SkillManifest::from_path(manifest_path)?,
            });
        }
    }

    Ok(skills)
}

pub fn install_bundle_from_local_registry(
    registry_root: impl AsRef<Path>,
    bundle_id: &str,
    skills_dir: impl AsRef<Path>,
) -> Result<Vec<InstalledSkillRecord>> {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async {
        let client = RegistryClient::from_local_path(registry_root)?;
        install_bundle_from_registry_client(&client, bundle_id, skills_dir, "local").await
    })
}

pub fn install_skill_from_local_registry(
    registry_root: impl AsRef<Path>,
    skill_id: &str,
    skills_dir: impl AsRef<Path>,
) -> Result<InstalledSkillRecord> {
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
    runtime.block_on(async {
        let client = RegistryClient::from_local_path(registry_root)?;
        install_skill_from_registry_client(&client, skill_id, skills_dir, "local").await
    })
}

pub async fn install_bundle_from_registry_client(
    client: &RegistryClient,
    bundle_id: &str,
    skills_dir: impl AsRef<Path>,
    source_label: &str,
) -> Result<Vec<InstalledSkillRecord>> {
    let (bundle, _resource) = client.fetch_bundle(bundle_id).await?;
    let mut installed = Vec::with_capacity(bundle.skills.len());

    for skill_id in bundle.skills {
        installed.push(
            install_skill_from_registry_client(
                client,
                &skill_id,
                skills_dir.as_ref(),
                source_label,
            )
            .await?,
        );
    }

    Ok(installed)
}

pub async fn install_skill_from_registry_client(
    client: &RegistryClient,
    skill_id: &str,
    skills_dir: impl AsRef<Path>,
    source_label: &str,
) -> Result<InstalledSkillRecord> {
    let skills_dir = skills_dir.as_ref();
    fs::create_dir_all(skills_dir)?;
    let (manifest, resource) = client.fetch_skill_manifest(skill_id).await?;
    let target_dir = skills_dir.join(&manifest.id);
    fs::create_dir_all(&target_dir)?;
    fs::write(target_dir.join("skill.toml"), &resource.content)?;

    let mut installed = InstalledSkills::load_from_dir(skills_dir)?;
    let record = InstalledSkillRecord {
        id: manifest.id.clone(),
        version: manifest.version.clone(),
        installed_at: now_timestamp(),
        source: source_label.to_string(),
        registry_url: Some(client.registry_location()),
        manifest_url: Some(resource.resolved_location),
        checksum: resource.sha256,
        enabled: is_entrypoint_allowed(&manifest.entrypoint),
    };
    installed.upsert(record.clone());
    installed.save_to_dir(skills_dir)?;

    Ok(record)
}

pub fn installed_skills_path(skills_dir: &Path) -> PathBuf {
    skills_dir.join("installed_skills.json")
}

fn is_entrypoint_allowed(entrypoint: &str) -> bool {
    entrypoint == "prompt-only" || entrypoint.starts_with("builtin:")
}

fn now_timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!("unix:{seconds}")
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn installs_skill_from_local_registry_fixture() {
        let dir = unique_temp_dir();
        let skills_dir = dir.join("skills");
        let registry_root = fixture_registry_root();

        let record = install_skill_from_local_registry(&registry_root, "python.write", &skills_dir)
            .expect("install skill");
        let installed = InstalledSkills::load_from_dir(&skills_dir).expect("load installed");

        assert_eq!(record.id, "python.write");
        assert!(record.enabled);
        assert_eq!(record.source, "local");
        assert!(record.registry_url.is_some());
        assert!(record.manifest_url.is_some());
        assert!(skills_dir.join("python.write").join("skill.toml").exists());
        assert!(installed.skills.contains_key("python.write"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn installing_bundle_creates_installed_skills_json() {
        let dir = unique_temp_dir();
        let skills_dir = dir.join("skills");
        let registry_root = fixture_registry_root();

        let records =
            install_bundle_from_local_registry(&registry_root, "essential.windows", &skills_dir)
                .expect("install bundle");
        let installed = InstalledSkills::load_from_dir(&skills_dir).expect("load installed");

        assert!(records.iter().any(|record| record.id == "file.read"));
        assert!(installed_skills_path(&skills_dir).exists());
        assert!(installed.skills.contains_key("shell.powershell.safe"));
        let _ = fs::remove_dir_all(dir);
    }

    fn fixture_registry_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/skill-registry")
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("axiom-engine-installed-test-{nanos}"))
    }
}
