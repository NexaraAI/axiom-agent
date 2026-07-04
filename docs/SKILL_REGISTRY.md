# Skill Registry

Axiom Skills Registry is the source of reusable skill manifests and bundles. The main Axiom Agent repository is separate from the `axiom-skills` GitHub repository. Later, npm will install the Axiom binary; skills will still come from the registry.

The registry implementation supports:

- Local fixture registries for tests and offline fallback.
- HTTPS registry URLs, with HTTP allowed only for localhost development.
- Relative manifest and bundle URLs resolved against `registry.json`.
- Optional SHA-256 verification for registry entries.
- Manifests, prompt cards, and built-in Axiom entrypoints only.
- Registry caching with stale-cache fallback.
- Skill update checks and controlled update installation.
- Lifecycle state and trust metadata for installed skills.

External executable skill binaries are intentionally not supported yet.

## Remote Shape

```text
axiom-skills/
  registry.json
  bundles/
    essential.windows.toml
    essential.linux.toml
    essential.macos.toml
  skills/
    file.read/
      skill.toml
      README.md
    python.write/
      skill.toml
      README.md
```

## Registry JSON

```json
{
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
      "sha256": "optional",
      "min_axiom_version": "0.1.0",
      "max_axiom_version": "optional"
    }
  ],
  "bundles": [
    {
      "id": "essential.windows",
      "name": "Essential Windows Skills",
      "platform": "windows",
      "bundle_url": "bundles/essential.windows.toml",
      "sha256": "optional"
    }
  ]
}
```

## Config

```toml
[skills]
registry_url = "https://raw.githubusercontent.com/NexaraAI/axiom-skills/main/registry.json"
registry_cache_ttl_hours = 24
auto_update_policy = "notify"
local_dir = "skills"
allow_untrusted_registries = false
fallback_to_bundled_registry = true
```

The default remote URL points at the NexaraAI `axiom-skills` registry. If loading it fails and fallback is enabled, onboarding and skill commands use the bundled local fixture registry.

## Commands

```powershell
axiom skill registry current
axiom skill registry set <url>
axiom skill registry refresh
axiom skill search <query>
axiom skill bundles
axiom skill install <skill_id>
axiom skill install <skill_id> --registry <url>
axiom skill install <skill_id> --from-local-registry <path>
axiom skill install-bundle <bundle_id>
axiom skill installed
axiom skill info <skill_id>
axiom skill update --check
axiom skill update <skill_id>
axiom skill update --all
axiom skill update --apply-patches
axiom skill health
axiom skill enable <skill_id>
axiom skill disable <skill_id>
axiom skill reset-stats <skill_id>
axiom skill remove <skill_id>
```

`axiom skill update --check` compares installed skill versions with the active registry. `axiom skill update <skill_id>` asks before installing that update. `--all` asks before applying compatible updates. `--apply-patches` applies compatible patch updates only when policy allows it.

Update output includes skill id, current version, available version, lifecycle state, source, trust level, update type, and compatibility result.

## Cache

Registry cache is stored under the user config directory:

```text
registry-cache/
  registry.json
  bundles/
  skills/
  cache-metadata.json
```

The cache uses `registry_cache_ttl_hours`. If refresh fails and stale cache exists, Axiom uses the stale cache with a warning. If no cache exists and fallback is enabled, Axiom uses the bundled fixture registry.

## Trust Rules

If a registry entry includes `sha256`, Axiom verifies the downloaded manifest or bundle content before installing it. A checksum mismatch fails the install.

If a custom registry is used, Axiom warns:

```text
Custom registries can change agent behavior. Only use registries you trust.
```

Trusted skills come from the official NexaraAI registry or the bundled fixture registry. Community custom registry skills show a warning. Untrusted custom skills require explicit confirmation. Blocked skills cannot be installed or executed.
