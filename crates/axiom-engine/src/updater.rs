use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use axiom_core::atomic_write;
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::{
    assess_skill_lifecycle, check_registry_entry_compatibility, current_axiom_version,
    installed::{mark_skill_update_failed, now_timestamp, InstalledSkillError},
    registry::CachedRegistrySource,
    InstalledSkillRecord, InstalledSkills, Platform, RegistryClient, RegistryIndex, RegistrySource,
    SkillLifecycleState, TrustLevel,
};

pub type CacheLoadResult = std::result::Result<CachedRegistry, crate::registry::RegistryError>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SkillAutoUpdatePolicy {
    Manual,
    #[default]
    Notify,
    AutoPatch,
}

impl SkillAutoUpdatePolicy {
    pub fn parse(value: &str) -> Self {
        match value {
            "manual" => Self::Manual,
            "auto-patch" => Self::AutoPatch,
            _ => Self::Notify,
        }
    }
}

impl std::fmt::Display for SkillAutoUpdatePolicy {
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
pub enum UpdateType {
    Patch,
    Minor,
    Major,
}

impl std::fmt::Display for UpdateType {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Patch => "patch",
            Self::Minor => "minor",
            Self::Major => "major",
        };
        formatter.write_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCompatibility {
    pub compatible: bool,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillUpdateStatus {
    pub id: String,
    pub current_version: Version,
    pub available_version: Version,
    pub state: SkillLifecycleState,
    pub source: String,
    pub trust_level: TrustLevel,
    pub update_type: UpdateType,
    pub compatibility: UpdateCompatibility,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillUpdateApplication {
    pub id: String,
    pub old_version: Version,
    pub new_version: Version,
    pub state: SkillLifecycleState,
    pub enabled: bool,
    pub trust_level: TrustLevel,
    pub update_type: UpdateType,
    pub registry_source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoUpdatePlan {
    pub policy: SkillAutoUpdatePolicy,
    pub notify_count: usize,
    pub patch_skill_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct CachedRegistry {
    pub client: RegistryClient,
    pub source_label: String,
    pub location: String,
    pub metadata: RegistryCacheMetadata,
    pub warning: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryCacheMetadata {
    pub source_url: String,
    pub fetched_at: String,
    pub ttl_hours: u64,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub used_stale_cache: bool,
}

pub fn update_type_for_versions(current: &Version, available: &Version) -> UpdateType {
    if available.major > current.major {
        UpdateType::Major
    } else if available.minor > current.minor {
        UpdateType::Minor
    } else {
        UpdateType::Patch
    }
}

pub fn check_skill_update_statuses(
    installed: &InstalledSkills,
    registry: &RegistryIndex,
    registry_location: &str,
    current_version: &Version,
    platform: &Platform,
) -> Vec<SkillUpdateStatus> {
    installed
        .skills
        .values()
        .filter_map(|record| {
            let entry = registry.skill_entry(&record.id)?;
            if entry.version <= record.version {
                return None;
            }
            let compatibility =
                check_registry_entry_compatibility(entry, current_version, platform);
            Some(SkillUpdateStatus {
                id: record.id.clone(),
                current_version: record.version.clone(),
                available_version: entry.version.clone(),
                state: if compatibility.compatible {
                    SkillLifecycleState::UpdateAvailable
                } else {
                    record.state
                },
                source: registry_location.to_string(),
                trust_level: record.trust_level,
                update_type: update_type_for_versions(&record.version, &entry.version),
                compatibility: UpdateCompatibility {
                    compatible: compatibility.compatible,
                    reason: compatibility.reason,
                },
            })
        })
        .collect()
}

pub fn policy_plan(policy: SkillAutoUpdatePolicy, updates: &[SkillUpdateStatus]) -> AutoUpdatePlan {
    let patch_skill_ids = if policy == SkillAutoUpdatePolicy::AutoPatch {
        updates
            .iter()
            .filter(|update| {
                update.update_type == UpdateType::Patch && update.compatibility.compatible
            })
            .map(|update| update.id.clone())
            .collect()
    } else {
        Vec::new()
    };

    AutoUpdatePlan {
        policy,
        notify_count: updates.len(),
        patch_skill_ids,
    }
}

pub fn mark_update_check_results(
    skills_dir: impl AsRef<Path>,
    updates: &[SkillUpdateStatus],
) -> Result<InstalledSkills, InstalledSkillError> {
    let skills_dir = skills_dir.as_ref();
    let mut installed = InstalledSkills::load_from_dir(skills_dir)?;
    let checked_at = now_timestamp();
    for record in installed.skills.values_mut() {
        record.last_checked_at = Some(checked_at.clone());
        if record.state == SkillLifecycleState::UpdateAvailable {
            record.state = if record.enabled {
                SkillLifecycleState::Enabled
            } else {
                SkillLifecycleState::Disabled
            };
        }
    }

    for update in updates {
        if let Some(record) = installed.skills.get_mut(&update.id) {
            if update.compatibility.compatible && record.enabled {
                record.state = SkillLifecycleState::UpdateAvailable;
            }
        }
    }

    installed.save_to_dir(skills_dir)?;
    Ok(installed)
}

pub async fn apply_skill_update(
    client: &RegistryClient,
    skills_dir: impl AsRef<Path>,
    skill_id: &str,
) -> Result<SkillUpdateApplication, InstalledSkillError> {
    let skills_dir = skills_dir.as_ref();
    let result = apply_skill_update_inner(client, skills_dir, skill_id).await;
    if let Err(error) = &result {
        if !matches!(error, InstalledSkillError::IncompatibleSkill { .. }) {
            let _ = mark_skill_update_failed(skills_dir, skill_id, error.to_string());
        }
    }
    result
}

async fn apply_skill_update_inner(
    client: &RegistryClient,
    skills_dir: &Path,
    skill_id: &str,
) -> Result<SkillUpdateApplication, InstalledSkillError> {
    let mut installed = InstalledSkills::load_from_dir(skills_dir)?;
    let old_record = installed
        .skills
        .get(skill_id)
        .cloned()
        .ok_or_else(|| InstalledSkillError::MissingSkill(skill_id.to_string()))?;
    let entry = client
        .index()
        .skill_entry(skill_id)
        .ok_or_else(|| crate::registry::RegistryError::MissingSkill(skill_id.to_string()))?;
    if entry.version <= old_record.version {
        return Err(InstalledSkillError::IncompatibleSkill {
            id: skill_id.to_string(),
            reason: "no newer registry version is available".to_string(),
        });
    }

    let platform = Platform::current();
    let current_version = current_axiom_version();
    let registry_compatibility =
        check_registry_entry_compatibility(entry, &current_version, &platform);
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
        client.source_label(),
        resource.sha256.as_deref(),
        &current_version,
        &platform,
    );
    if !assessment.compatibility.compatible {
        return Err(InstalledSkillError::IncompatibleSkill {
            id: skill_id.to_string(),
            reason: assessment.compatibility.reason,
        });
    }
    if assessment.trust_level == TrustLevel::Blocked {
        return Err(InstalledSkillError::BlockedSkill(skill_id.to_string()));
    }

    let manifest_path = skills_dir.join(skill_id).join("skill.toml");
    replace_file_atomically(&manifest_path, &resource.content)?;

    let now = now_timestamp();
    let mut new_record = InstalledSkillRecord {
        id: manifest.id.clone(),
        version: manifest.version.clone(),
        installed_at: old_record.installed_at.clone(),
        updated_at: Some(now.clone()),
        source: old_record.source.clone(),
        registry_url: Some(client.registry_location()),
        manifest_url: Some(resource.resolved_location),
        checksum: resource.sha256,
        enabled: old_record.enabled && assessment.enabled,
        state: assessment.state,
        trust_level: assessment.trust_level,
        last_checked_at: Some(now),
        last_update_error: None,
        last_runtime_error: old_record.last_runtime_error.clone(),
        success_count: old_record.success_count,
        failure_count: old_record.failure_count,
        last_used_at: old_record.last_used_at.clone(),
        average_latency_ms: old_record.average_latency_ms,
    };
    if !old_record.enabled || old_record.state == SkillLifecycleState::Disabled {
        new_record.enabled = false;
        new_record.state = SkillLifecycleState::Disabled;
    }

    installed.upsert(new_record.clone());
    installed.save_to_dir(skills_dir)?;

    Ok(SkillUpdateApplication {
        id: new_record.id,
        old_version: old_record.version.clone(),
        new_version: manifest.version,
        state: new_record.state,
        enabled: new_record.enabled,
        trust_level: new_record.trust_level,
        update_type: update_type_for_versions(&old_record.version, &entry.version),
        registry_source: client.registry_location(),
    })
}

pub async fn load_registry_with_cache(
    registry_location: &str,
    cache_dir: impl AsRef<Path>,
    bundled_registry_root: impl AsRef<Path>,
    ttl_hours: u64,
    fallback_to_bundled_registry: bool,
) -> CacheLoadResult {
    let cache_dir = cache_dir.as_ref();
    let bundled_registry_root = bundled_registry_root.as_ref();
    fs::create_dir_all(cache_dir)?;
    fs::create_dir_all(cache_dir.join("bundles"))?;
    fs::create_dir_all(cache_dir.join("skills"))?;

    if let Some(metadata) = load_cache_metadata(cache_dir)? {
        let registry_path = registry_cache_registry_path(cache_dir);
        if metadata.source_url == registry_location
            && cache_is_valid(&metadata)
            && registry_path.exists()
        {
            let client =
                RegistryClient::from_source(RegistrySource::Cached(CachedRegistrySource {
                    registry_path,
                    original_location: registry_location.to_string(),
                    timeout_secs: 10,
                }))
                .await?;
            return Ok(CachedRegistry {
                client,
                source_label: "cache".to_string(),
                location: registry_location.to_string(),
                metadata,
                warning: None,
            });
        }
    }

    match load_registry_client_from_location(registry_location).await {
        Ok(client) => {
            let source_label = client.source_label().to_string();
            let metadata = RegistryCacheMetadata {
                source_url: registry_location.to_string(),
                fetched_at: now_timestamp(),
                ttl_hours,
                last_error: None,
                used_stale_cache: false,
            };
            write_cache(cache_dir, client.index(), &metadata)?;
            Ok(CachedRegistry {
                client,
                source_label,
                location: registry_location.to_string(),
                metadata,
                warning: None,
            })
        }
        Err(error) => {
            let registry_path = registry_cache_registry_path(cache_dir);
            if registry_path.exists() {
                let mut metadata =
                    load_cache_metadata(cache_dir)?.unwrap_or_else(|| RegistryCacheMetadata {
                        source_url: registry_location.to_string(),
                        fetched_at: "unix:0".to_string(),
                        ttl_hours,
                        last_error: None,
                        used_stale_cache: false,
                    });
                metadata.last_error = Some(error.to_string());
                metadata.used_stale_cache = true;
                write_cache_metadata(cache_dir, &metadata)?;
                let client =
                    RegistryClient::from_source(RegistrySource::Cached(CachedRegistrySource {
                        registry_path,
                        original_location: registry_location.to_string(),
                        timeout_secs: 10,
                    }))
                    .await?;
                let warning = format!("Using stale registry cache. ({error})");
                return Ok(CachedRegistry {
                    client,
                    source_label: "cache".to_string(),
                    location: registry_location.to_string(),
                    metadata,
                    warning: Some(warning),
                });
            }

            if fallback_to_bundled_registry {
                let client = RegistryClient::from_local_path(bundled_registry_root)?;
                let metadata = RegistryCacheMetadata {
                    source_url: registry_location.to_string(),
                    fetched_at: "unix:0".to_string(),
                    ttl_hours,
                    last_error: Some(error.to_string()),
                    used_stale_cache: false,
                };
                return Ok(CachedRegistry {
                    client,
                    source_label: "bundled".to_string(),
                    location: bundled_registry_root.display().to_string(),
                    metadata,
                    warning: Some(format!(
                        "Configured registry unavailable. Using bundled fallback registry. ({error})"
                    )),
                });
            }

            Err(error)
        }
    }
}

pub fn registry_cache_dir(config_dir: impl AsRef<Path>) -> PathBuf {
    config_dir.as_ref().join("registry-cache")
}

pub fn registry_cache_registry_path(cache_dir: impl AsRef<Path>) -> PathBuf {
    cache_dir.as_ref().join("registry.json")
}

pub fn registry_cache_metadata_path(cache_dir: impl AsRef<Path>) -> PathBuf {
    cache_dir.as_ref().join("cache-metadata.json")
}

async fn load_registry_client_from_location(
    location: &str,
) -> Result<RegistryClient, crate::registry::RegistryError> {
    let path = PathBuf::from(location);
    if path.exists() {
        Ok(RegistryClient::from_local_path(path)?)
    } else {
        Ok(RegistryClient::from_url(location).await?)
    }
}

fn write_cache(
    cache_dir: &Path,
    index: &RegistryIndex,
    metadata: &RegistryCacheMetadata,
) -> Result<(), crate::registry::RegistryError> {
    atomic_write(
        registry_cache_registry_path(cache_dir),
        serde_json::to_string_pretty(index)?.as_bytes(),
    )?;
    write_cache_metadata(cache_dir, metadata)?;
    Ok(())
}

fn write_cache_metadata(
    cache_dir: &Path,
    metadata: &RegistryCacheMetadata,
) -> Result<(), crate::registry::RegistryError> {
    atomic_write(
        registry_cache_metadata_path(cache_dir),
        serde_json::to_string_pretty(metadata)?.as_bytes(),
    )?;
    Ok(())
}

fn load_cache_metadata(
    cache_dir: &Path,
) -> Result<Option<RegistryCacheMetadata>, crate::registry::RegistryError> {
    let path = registry_cache_metadata_path(cache_dir);
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path)?;
    Ok(Some(serde_json::from_str(&content)?))
}

fn cache_is_valid(metadata: &RegistryCacheMetadata) -> bool {
    let Some(fetched_at) = parse_unix_timestamp(&metadata.fetched_at) else {
        return false;
    };
    let now = current_unix_timestamp();
    let ttl_seconds = metadata.ttl_hours.saturating_mul(60).saturating_mul(60);
    now.saturating_sub(fetched_at) <= ttl_seconds
}

fn parse_unix_timestamp(value: &str) -> Option<u64> {
    value.strip_prefix("unix:")?.parse().ok()
}

fn current_unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default()
}

fn replace_file_atomically(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temp_path = path.with_extension("toml.update");
    let backup_path = path.with_extension("toml.bak");
    fs::write(&temp_path, content)?;

    if backup_path.exists() {
        fs::remove_file(&backup_path)?;
    }
    if path.exists() {
        fs::rename(path, &backup_path)?;
    }

    match fs::rename(&temp_path, path) {
        Ok(()) => {
            if backup_path.exists() {
                let _ = fs::remove_file(&backup_path);
            }
            Ok(())
        }
        Err(error) => {
            if backup_path.exists() {
                let _ = fs::rename(&backup_path, path);
            }
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;
    use crate::{InstalledSkillRecord, SkillLifecycleState, TrustLevel};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn detects_patch_minor_and_major_update_types() {
        assert_eq!(
            update_type_for_versions(&Version::new(1, 2, 3), &Version::new(1, 2, 4)),
            UpdateType::Patch
        );
        assert_eq!(
            update_type_for_versions(&Version::new(1, 2, 3), &Version::new(1, 3, 0)),
            UpdateType::Minor
        );
        assert_eq!(
            update_type_for_versions(&Version::new(1, 2, 3), &Version::new(2, 0, 0)),
            UpdateType::Major
        );
    }

    #[test]
    fn update_check_detects_patch_update() {
        let mut installed = InstalledSkills::default();
        installed.upsert(record("file.read", Version::new(0, 1, 0)));
        let registry = registry_with_version("file.read", Version::new(0, 1, 1), "0.1.0", None);

        let updates = check_skill_update_statuses(
            &installed,
            &registry,
            "test",
            &Version::new(0, 1, 0),
            &Platform::Windows,
        );

        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].update_type, UpdateType::Patch);
    }

    #[test]
    fn min_and_max_versions_block_updates() {
        let mut installed = InstalledSkills::default();
        installed.upsert(record("file.read", Version::new(0, 1, 0)));

        let min_blocked = registry_with_version("file.read", Version::new(0, 1, 1), "9.0.0", None);
        let updates = check_skill_update_statuses(
            &installed,
            &min_blocked,
            "test",
            &Version::new(0, 1, 0),
            &Platform::Windows,
        );
        assert!(!updates[0].compatibility.compatible);

        let max_blocked = registry_with_version(
            "file.read",
            Version::new(0, 1, 1),
            "0.1.0",
            Some(Version::new(0, 0, 9)),
        );
        let updates = check_skill_update_statuses(
            &installed,
            &max_blocked,
            "test",
            &Version::new(0, 1, 0),
            &Platform::Windows,
        );
        assert!(!updates[0].compatibility.compatible);
    }

    #[test]
    fn auto_patch_policy_only_plans_patch_updates() {
        let updates = vec![
            status("a", UpdateType::Patch, true),
            status("b", UpdateType::Minor, true),
            status("c", UpdateType::Patch, false),
        ];

        let plan = policy_plan(SkillAutoUpdatePolicy::AutoPatch, &updates);

        assert_eq!(plan.patch_skill_ids, vec!["a"]);
        assert!(policy_plan(SkillAutoUpdatePolicy::Manual, &updates)
            .patch_skill_ids
            .is_empty());
    }

    #[tokio::test]
    async fn registry_cache_valid_path_works() {
        let dir = temp_dir();
        let cache_dir = dir.join("cache");
        let registry_root = fixture_registry_root();

        let first = load_registry_with_cache(
            &registry_root.display().to_string(),
            &cache_dir,
            &registry_root,
            24,
            true,
        )
        .await
        .expect("cache load");
        assert!(first.client.index().skill_entry("file.read").is_some());

        let second = load_registry_with_cache(
            &registry_root.display().to_string(),
            &cache_dir,
            &registry_root,
            24,
            true,
        )
        .await
        .expect("cache hit");
        assert_eq!(second.source_label, "cache");
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn registry_cache_stale_fallback_works() {
        let dir = temp_dir();
        let cache_dir = dir.join("cache");
        let registry_root = fixture_registry_root();
        load_registry_with_cache(
            &registry_root.display().to_string(),
            &cache_dir,
            &registry_root,
            0,
            true,
        )
        .await
        .expect("prime cache");

        let stale =
            load_registry_with_cache("not-a-url-or-path", &cache_dir, &registry_root, 0, true)
                .await
                .expect("stale cache");

        assert_eq!(stale.source_label, "cache");
        assert!(stale.warning.is_some());
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn registry_cache_falls_back_to_bundled_fixture() {
        let dir = temp_dir();
        let cache_dir = dir.join("cache");
        let registry_root = fixture_registry_root();

        let loaded =
            load_registry_with_cache("not-a-url-or-path", &cache_dir, &registry_root, 24, true)
                .await
                .expect("bundled fallback");

        assert_eq!(loaded.source_label, "bundled");
        assert!(loaded.client.index().skill_entry("file.read").is_some());
        let _ = fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn update_failure_keeps_old_version() {
        let dir = temp_dir();
        let skills_dir = dir.join("skills");
        fs::create_dir_all(skills_dir.join("test.skill")).expect("skill dir");
        fs::write(
            skills_dir.join("test.skill/skill.toml"),
            test_manifest("test.skill", "0.1.0"),
        )
        .expect("old manifest");
        let mut installed = InstalledSkills::default();
        installed.upsert(record("test.skill", Version::new(0, 1, 0)));
        installed.save_to_dir(&skills_dir).expect("save installed");

        let registry_root = dir.join("registry");
        fs::create_dir_all(&registry_root).expect("registry dir");
        fs::write(
            registry_root.join("registry.json"),
            r#"{
  "schema_version": "0.1",
  "name": "Test",
  "updated_at": "test",
  "skills": [{
    "id": "test.skill",
    "version": "0.1.1",
    "category": "test",
    "platforms": ["windows", "linux", "macos"],
    "manifest_url": "skills/test.skill/missing.toml",
    "min_axiom_version": "0.1.0"
  }],
  "bundles": []
}"#,
        )
        .expect("registry");
        let client = RegistryClient::from_local_path(&registry_root).expect("client");

        apply_skill_update(&client, &skills_dir, "test.skill")
            .await
            .expect_err("missing manifest should fail");
        let installed = InstalledSkills::load_from_dir(&skills_dir).expect("load installed");
        let record = installed.skills.get("test.skill").expect("record");

        assert_eq!(record.version, Version::new(0, 1, 0));
        assert_eq!(record.state, SkillLifecycleState::FailedUpdate);
        assert!(record.last_update_error.is_some());
        let _ = fs::remove_dir_all(dir);
    }

    fn status(id: &str, update_type: UpdateType, compatible: bool) -> SkillUpdateStatus {
        SkillUpdateStatus {
            id: id.to_string(),
            current_version: Version::new(0, 1, 0),
            available_version: Version::new(0, 1, 1),
            state: SkillLifecycleState::UpdateAvailable,
            source: "test".to_string(),
            trust_level: TrustLevel::Trusted,
            update_type,
            compatibility: UpdateCompatibility {
                compatible,
                reason: if compatible {
                    "compatible".to_string()
                } else {
                    "blocked".to_string()
                },
            },
        }
    }

    fn record(id: &str, version: Version) -> InstalledSkillRecord {
        InstalledSkillRecord {
            id: id.to_string(),
            version,
            installed_at: "test".to_string(),
            updated_at: None,
            source: "test".to_string(),
            registry_url: None,
            manifest_url: None,
            checksum: None,
            enabled: true,
            state: SkillLifecycleState::Enabled,
            trust_level: TrustLevel::Trusted,
            last_checked_at: None,
            last_update_error: None,
            last_runtime_error: None,
            success_count: 0,
            failure_count: 0,
            last_used_at: None,
            average_latency_ms: None,
        }
    }

    fn registry_with_version(
        id: &str,
        version: Version,
        min_axiom_version: &str,
        max_axiom_version: Option<Version>,
    ) -> RegistryIndex {
        RegistryIndex {
            schema_version: "0.1".to_string(),
            name: "Test".to_string(),
            updated_at: "test".to_string(),
            skills: vec![crate::RegistrySkillEntry {
                id: id.to_string(),
                version,
                category: "test".to_string(),
                platforms: vec!["windows".to_string()],
                manifest_url: "skills/file.read/skill.toml".to_string(),
                sha256: None,
                min_axiom_version: Version::parse(min_axiom_version).expect("version"),
                max_axiom_version,
            }],
            bundles: Vec::new(),
        }
    }

    fn test_manifest(id: &str, version: &str) -> String {
        format!(
            r#"
id = "{id}"
name = "Test Skill"
version = "{version}"
description = "Test skill."
category = "test"
skill_type = "prompt"
risk_level = "low"
permissions = []
platforms = ["windows", "linux", "macos"]
entrypoint = "prompt-only"
author = "Test"
license = "MIT"
min_axiom_version = "0.1.0"
"#
        )
    }

    fn fixture_registry_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/skill-registry")
    }

    fn temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time")
            .as_nanos();
        let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("axiom-updater-test-{nanos}-{counter}"))
    }
}
