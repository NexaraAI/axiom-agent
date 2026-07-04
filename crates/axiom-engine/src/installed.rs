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
    lifecycle::{
        assess_skill_lifecycle, check_manifest_compatibility, current_axiom_version,
        SkillLifecycleState, TrustLevel,
    },
    manifest::{ManifestError, SkillManifest},
    registry::{RegistryClient, RegistryError},
    Platform,
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
    #[error("skill is not installed: {0}")]
    MissingSkill(String),
    #[error("skill is incompatible: {id}: {reason}")]
    IncompatibleSkill { id: String, reason: String },
    #[error("skill is blocked: {0}")]
    BlockedSkill(String),
    #[error("skill cannot be enabled in state {state}: {id}")]
    CannotEnableSkill {
        id: String,
        state: SkillLifecycleState,
    },
    #[error("invalid skill id: {0}")]
    InvalidSkillId(String),
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
    #[serde(default)]
    pub updated_at: Option<String>,
    pub source: String,
    #[serde(default)]
    pub registry_url: Option<String>,
    #[serde(default)]
    pub manifest_url: Option<String>,
    #[serde(default)]
    pub checksum: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default)]
    pub state: SkillLifecycleState,
    #[serde(default)]
    pub trust_level: TrustLevel,
    #[serde(default)]
    pub last_checked_at: Option<String>,
    #[serde(default)]
    pub last_update_error: Option<String>,
    #[serde(default)]
    pub last_runtime_error: Option<String>,
    #[serde(default)]
    pub success_count: u64,
    #[serde(default)]
    pub failure_count: u64,
    #[serde(default)]
    pub last_used_at: Option<String>,
    #[serde(default)]
    pub average_latency_ms: Option<u64>,
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

impl InstalledSkillRecord {
    pub fn is_selectable(&self) -> bool {
        self.enabled
            && self.trust_level != TrustLevel::Blocked
            && !matches!(
                self.state,
                SkillLifecycleState::Disabled
                    | SkillLifecycleState::Incompatible
                    | SkillLifecycleState::Quarantined
            )
    }

    pub fn is_executable(&self) -> bool {
        self.is_selectable()
    }

    pub fn mark_success(&mut self, latency_ms: u64) {
        self.success_count = self.success_count.saturating_add(1);
        let previous_successes = self.success_count.saturating_sub(1);
        self.average_latency_ms = Some(match (self.average_latency_ms, previous_successes) {
            (Some(current_average), successes) if successes > 0 => {
                ((current_average.saturating_mul(successes)).saturating_add(latency_ms))
                    / self.success_count
            }
            _ => latency_ms,
        });
        self.last_used_at = Some(now_timestamp());
        self.last_runtime_error = None;
    }

    pub fn mark_failure(&mut self, error: impl Into<String>) {
        self.failure_count = self.failure_count.saturating_add(1);
        self.last_used_at = Some(now_timestamp());
        self.last_runtime_error = Some(error.into());
    }

    pub fn mark_failed_update(&mut self, error: impl Into<String>) {
        self.state = SkillLifecycleState::FailedUpdate;
        self.last_update_error = Some(error.into());
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
    let entry = client
        .index()
        .skill_entry(skill_id)
        .ok_or_else(|| RegistryError::MissingSkill(skill_id.to_string()))?;
    let platform = Platform::current();
    let current_version = current_axiom_version();
    let registry_compatibility =
        crate::check_registry_entry_compatibility(entry, &current_version, &platform);
    if !registry_compatibility.compatible {
        return Err(InstalledSkillError::IncompatibleSkill {
            id: skill_id.to_string(),
            reason: registry_compatibility.reason,
        });
    }

    let (manifest, resource) = client.fetch_skill_manifest(skill_id).await?;
    let assessment = assess_skill_lifecycle(
        &manifest,
        &client.registry_location(),
        source_label,
        resource.sha256.as_deref(),
        &current_version,
        &platform,
    );
    if assessment.trust_level == TrustLevel::Blocked {
        return Err(InstalledSkillError::BlockedSkill(manifest.id));
    }
    if assessment.state == SkillLifecycleState::Incompatible {
        return Err(InstalledSkillError::IncompatibleSkill {
            id: manifest.id,
            reason: assessment.compatibility.reason,
        });
    }

    let target_dir = skills_dir.join(&manifest.id);
    fs::create_dir_all(&target_dir)?;
    fs::write(target_dir.join("skill.toml"), &resource.content)?;

    let mut installed = InstalledSkills::load_from_dir(skills_dir)?;
    let now = now_timestamp();
    let record = InstalledSkillRecord {
        id: manifest.id.clone(),
        version: manifest.version.clone(),
        installed_at: now.clone(),
        updated_at: Some(now),
        source: source_label.to_string(),
        registry_url: Some(client.registry_location()),
        manifest_url: Some(resource.resolved_location),
        checksum: resource.sha256,
        enabled: assessment.enabled,
        state: assessment.state,
        trust_level: assessment.trust_level,
        last_checked_at: None,
        last_update_error: None,
        last_runtime_error: None,
        success_count: 0,
        failure_count: 0,
        last_used_at: None,
        average_latency_ms: None,
    };
    installed.upsert(record.clone());
    installed.save_to_dir(skills_dir)?;

    Ok(record)
}

pub fn enable_skill(skills_dir: impl AsRef<Path>, skill_id: &str) -> Result<InstalledSkillRecord> {
    validate_skill_id(skill_id)?;
    let skills_dir = skills_dir.as_ref();
    let mut installed = InstalledSkills::load_from_dir(skills_dir)?;
    let mut record = installed
        .skills
        .get(skill_id)
        .cloned()
        .ok_or_else(|| InstalledSkillError::MissingSkill(skill_id.to_string()))?;
    let manifest = SkillManifest::from_path(skills_dir.join(skill_id).join("skill.toml"))?;
    let compatibility =
        check_manifest_compatibility(&manifest, &current_axiom_version(), &Platform::current());
    if !compatibility.compatible {
        record.enabled = false;
        record.state = SkillLifecycleState::Incompatible;
        record.trust_level = TrustLevel::Blocked;
        installed.upsert(record.clone());
        installed.save_to_dir(skills_dir)?;
        return Err(InstalledSkillError::IncompatibleSkill {
            id: skill_id.to_string(),
            reason: compatibility.reason,
        });
    }
    if record.trust_level == TrustLevel::Blocked {
        return Err(InstalledSkillError::BlockedSkill(skill_id.to_string()));
    }
    if matches!(
        record.state,
        SkillLifecycleState::Incompatible | SkillLifecycleState::Quarantined
    ) {
        return Err(InstalledSkillError::CannotEnableSkill {
            id: skill_id.to_string(),
            state: record.state,
        });
    }

    record.enabled = true;
    if record.state == SkillLifecycleState::Disabled {
        record.state = SkillLifecycleState::Enabled;
    }
    installed.upsert(record.clone());
    installed.save_to_dir(skills_dir)?;
    Ok(record)
}

pub fn disable_skill(skills_dir: impl AsRef<Path>, skill_id: &str) -> Result<InstalledSkillRecord> {
    validate_skill_id(skill_id)?;
    let skills_dir = skills_dir.as_ref();
    let mut installed = InstalledSkills::load_from_dir(skills_dir)?;
    let mut record = installed
        .skills
        .get(skill_id)
        .cloned()
        .ok_or_else(|| InstalledSkillError::MissingSkill(skill_id.to_string()))?;
    record.enabled = false;
    record.state = SkillLifecycleState::Disabled;
    installed.upsert(record.clone());
    installed.save_to_dir(skills_dir)?;
    Ok(record)
}

pub fn reset_skill_stats(
    skills_dir: impl AsRef<Path>,
    skill_id: &str,
) -> Result<InstalledSkillRecord> {
    validate_skill_id(skill_id)?;
    let skills_dir = skills_dir.as_ref();
    let mut installed = InstalledSkills::load_from_dir(skills_dir)?;
    let mut record = installed
        .skills
        .get(skill_id)
        .cloned()
        .ok_or_else(|| InstalledSkillError::MissingSkill(skill_id.to_string()))?;
    record.success_count = 0;
    record.failure_count = 0;
    record.last_runtime_error = None;
    record.average_latency_ms = None;
    installed.upsert(record.clone());
    installed.save_to_dir(skills_dir)?;
    Ok(record)
}

pub fn remove_skill(skills_dir: impl AsRef<Path>, skill_id: &str) -> Result<()> {
    validate_skill_id(skill_id)?;
    let skills_dir = skills_dir.as_ref();
    let mut installed = InstalledSkills::load_from_dir(skills_dir)?;
    installed
        .skills
        .remove(skill_id)
        .ok_or_else(|| InstalledSkillError::MissingSkill(skill_id.to_string()))?;
    installed.save_to_dir(skills_dir)?;

    let skill_dir = skills_dir.join(skill_id);
    if skill_dir.exists() {
        fs::remove_dir_all(skill_dir)?;
    }

    Ok(())
}

pub fn record_skill_execution_success(
    skills_dir: impl AsRef<Path>,
    skill_id: &str,
    latency_ms: u64,
) -> Result<Option<InstalledSkillRecord>> {
    update_skill_record(skills_dir, skill_id, |record| {
        record.mark_success(latency_ms)
    })
}

pub fn record_skill_execution_failure(
    skills_dir: impl AsRef<Path>,
    skill_id: &str,
    error: impl Into<String>,
) -> Result<Option<InstalledSkillRecord>> {
    let error = error.into();
    update_skill_record(skills_dir, skill_id, |record| {
        record.mark_failure(error.clone())
    })
}

pub fn mark_skill_update_failed(
    skills_dir: impl AsRef<Path>,
    skill_id: &str,
    error: impl Into<String>,
) -> Result<Option<InstalledSkillRecord>> {
    let error = error.into();
    update_skill_record(skills_dir, skill_id, |record| {
        record.mark_failed_update(error.clone())
    })
}

pub fn installed_skills_path(skills_dir: &Path) -> PathBuf {
    skills_dir.join("installed_skills.json")
}

fn update_skill_record(
    skills_dir: impl AsRef<Path>,
    skill_id: &str,
    update: impl FnOnce(&mut InstalledSkillRecord),
) -> Result<Option<InstalledSkillRecord>> {
    validate_skill_id(skill_id)?;
    let skills_dir = skills_dir.as_ref();
    let mut installed = InstalledSkills::load_from_dir(skills_dir)?;
    let Some(mut record) = installed.skills.get(skill_id).cloned() else {
        return Ok(None);
    };
    update(&mut record);
    installed.upsert(record.clone());
    installed.save_to_dir(skills_dir)?;
    Ok(Some(record))
}

fn validate_skill_id(skill_id: &str) -> Result<()> {
    if skill_id.trim().is_empty()
        || skill_id.contains('/')
        || skill_id.contains('\\')
        || skill_id.contains(':')
        || skill_id.contains("..")
    {
        return Err(InstalledSkillError::InvalidSkillId(skill_id.to_string()));
    }
    Ok(())
}

pub(crate) fn now_timestamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format!("unix:{seconds}")
}

fn default_enabled() -> bool {
    true
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
    fn parses_legacy_installed_record_with_lifecycle_defaults() {
        let installed: InstalledSkills = serde_json::from_str(
            r#"{
  "skills": {
    "file.read": {
      "id": "file.read",
      "version": "0.1.0",
      "installed_at": "test",
      "source": "local",
      "enabled": true
    }
  }
}"#,
        )
        .expect("legacy installed record parses");
        let record = installed.skills.get("file.read").expect("record");

        assert_eq!(record.state, SkillLifecycleState::Enabled);
        assert_eq!(record.trust_level, TrustLevel::Trusted);
        assert_eq!(record.success_count, 0);
    }

    #[test]
    fn installing_bundle_creates_installed_skills_json() {
        let dir = unique_temp_dir();
        let skills_dir = dir.join("skills");
        let registry_root = fixture_registry_root();

        let bundle_id = crate::registry::essential_bundle_id_for_os(std::env::consts::OS)
            .expect("current platform has essential bundle");
        let records = install_bundle_from_local_registry(&registry_root, bundle_id, &skills_dir)
            .expect("install bundle");
        let installed = InstalledSkills::load_from_dir(&skills_dir).expect("load installed");

        assert!(records.iter().any(|record| record.id == "file.read"));
        assert!(installed_skills_path(&skills_dir).exists());
        assert!(installed.skills.contains_key("file.read"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn disable_enable_reset_stats_and_remove_update_installed_records() {
        let dir = unique_temp_dir();
        let skills_dir = dir.join("skills");
        let registry_root = fixture_registry_root();
        install_skill_from_local_registry(&registry_root, "file.read", &skills_dir)
            .expect("install skill");

        let disabled = disable_skill(&skills_dir, "file.read").expect("disable");
        assert!(!disabled.enabled);
        assert_eq!(disabled.state, SkillLifecycleState::Disabled);

        let enabled = enable_skill(&skills_dir, "file.read").expect("enable");
        assert!(enabled.enabled);
        assert_eq!(enabled.state, SkillLifecycleState::Enabled);

        record_skill_execution_success(&skills_dir, "file.read", 25).expect("success stats");
        record_skill_execution_failure(&skills_dir, "file.read", "boom").expect("failure stats");
        let reset = reset_skill_stats(&skills_dir, "file.read").expect("reset");
        assert_eq!(reset.success_count, 0);
        assert_eq!(reset.failure_count, 0);
        assert!(reset.last_runtime_error.is_none());

        remove_skill(&skills_dir, "file.read").expect("remove");
        let installed = InstalledSkills::load_from_dir(&skills_dir).expect("load installed");
        assert!(!installed.skills.contains_key("file.read"));
        assert!(!skills_dir.join("file.read").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn health_stats_update_on_success_and_failure() {
        let dir = unique_temp_dir();
        let skills_dir = dir.join("skills");
        let registry_root = fixture_registry_root();
        install_skill_from_local_registry(&registry_root, "file.read", &skills_dir)
            .expect("install skill");

        let success =
            record_skill_execution_success(&skills_dir, "file.read", 40).expect("success");
        assert_eq!(success.expect("record").success_count, 1);
        let failure =
            record_skill_execution_failure(&skills_dir, "file.read", "failed").expect("failure");
        let failure = failure.expect("record");
        assert_eq!(failure.failure_count, 1);
        assert_eq!(failure.last_runtime_error.as_deref(), Some("failed"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unsupported_entrypoint_installs_quarantined() {
        let dir = unique_temp_dir();
        let registry_root = dir.join("registry");
        write_test_registry(
            &registry_root,
            "external:run",
            &[std::env::consts::OS],
            "0.1.0",
        );
        let skills_dir = dir.join("skills");

        let record = install_skill_from_local_registry(&registry_root, "test.skill", &skills_dir)
            .expect("install quarantined skill");

        assert!(!record.enabled);
        assert_eq!(record.state, SkillLifecycleState::Quarantined);
        assert_eq!(record.trust_level, TrustLevel::Untrusted);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn platform_incompatible_skill_is_not_installed() {
        let dir = unique_temp_dir();
        let registry_root = dir.join("registry");
        let unsupported = if std::env::consts::OS == "windows" {
            "linux"
        } else {
            "windows"
        };
        write_test_registry(&registry_root, "prompt-only", &[unsupported], "0.1.0");
        let skills_dir = dir.join("skills");

        let error = install_skill_from_local_registry(&registry_root, "test.skill", &skills_dir)
            .expect_err("platform should be blocked");

        assert!(matches!(
            error,
            InstalledSkillError::IncompatibleSkill { .. }
        ));
        assert!(!skills_dir.join("test.skill").exists());
        let _ = fs::remove_dir_all(dir);
    }

    fn fixture_registry_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/skill-registry")
    }

    fn write_test_registry(
        registry_root: &Path,
        entrypoint: &str,
        platforms: &[&str],
        min_axiom_version: &str,
    ) {
        fs::create_dir_all(registry_root.join("skills/test.skill")).expect("registry dirs");
        let platforms_json = platforms
            .iter()
            .map(|platform| format!("\"{platform}\""))
            .collect::<Vec<_>>()
            .join(", ");
        fs::write(
            registry_root.join("registry.json"),
            format!(
                r#"{{
  "schema_version": "0.1",
  "name": "Test",
  "updated_at": "test",
  "skills": [{{
    "id": "test.skill",
    "version": "0.1.0",
    "category": "test",
    "platforms": [{platforms_json}],
    "manifest_url": "skills/test.skill/skill.toml",
    "min_axiom_version": "{min_axiom_version}"
  }}],
  "bundles": []
}}"#
            ),
        )
        .expect("registry json");
        fs::write(
            registry_root.join("skills/test.skill/skill.toml"),
            format!(
                r#"
id = "test.skill"
name = "Test Skill"
version = "0.1.0"
description = "Test skill."
category = "test"
skill_type = "prompt"
risk_level = "low"
permissions = []
platforms = [{platforms_json}]
entrypoint = "{entrypoint}"
author = "Test"
license = "MIT"
min_axiom_version = "{min_axiom_version}"
"#
            ),
        )
        .expect("skill toml");
    }

    fn unique_temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!("axiom-engine-installed-test-{nanos}"))
    }
}
