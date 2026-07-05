# Skill Registry

The Axiom Skills Registry holds reusable skill manifests and bundles. It lives in the `axiom-skills` GitHub repository, separate from the main Axiom Agent repo. npm installs the Axiom binary; skills come from the registry.

What you can do with the registry:

- Use local fixture registries for tests and offline fallback.
- Fetch from HTTPS registry URLs (HTTP is allowed for localhost development only).
- Resolve relative manifest and bundle URLs against `registry.json`.
- Verify registry entries with optional SHA-256 checksums.
- Install manifests, prompt cards, and built-in Axiom entrypoints.
- Cache registry data with stale-cache fallback.
- Check for skill updates and install them in a controlled way.
- Track lifecycle state and trust metadata for installed skills.

External executable skill binaries are not supported yet.

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

The default remote URL points at the NexaraAI `axiom-skills` registry. If loading fails and fallback is enabled, onboarding and skill commands use the bundled local fixture registry instead.

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

`axiom skill update --check` compares installed skill versions against the active registry. `axiom skill update <skill_id>` prompts before installing that update. `--all` prompts before applying compatible updates. `--apply-patches` applies compatible patch updates when policy allows it.

Update output shows: skill id, current version, available version, lifecycle state, source, trust level, update type, and compatibility result.

## Cache

Axiom stores the registry cache under the user config directory:

```text
registry-cache/
  registry.json
  bundles/
  skills/
  cache-metadata.json
```

The cache respects `registry_cache_ttl_hours`. If a refresh fails and a stale cache exists, Axiom uses the stale cache and prints a warning. If no cache exists and fallback is enabled, Axiom falls back to the bundled fixture registry.

## Trust Rules

When a registry entry includes `sha256`, Axiom verifies the downloaded manifest or bundle content before installing. A checksum mismatch fails the install.

When you use a custom registry, Axiom warns:

```text
Custom registries can change agent behavior. Only use registries you trust.
```

Trusted skills come from the official NexaraAI registry or the bundled fixture registry. Community custom registry skills show a warning. Untrusted custom skills require explicit confirmation. You cannot install or execute blocked skills.
