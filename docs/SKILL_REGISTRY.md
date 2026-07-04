# Skill Registry

Axiom Skills Registry is the source of reusable skill manifests and bundles. The main Axiom Agent repository is separate from the future `axiom-skills` GitHub repository. Later, npm will install the Axiom binary; skills will still come from the registry.

Stage 7 supports:

- Local fixture registries for tests and offline fallback.
- HTTPS registry URLs, with HTTP allowed only for localhost development.
- Relative manifest and bundle URLs resolved against `registry.json`.
- Optional SHA-256 verification for registry entries.
- Manifests, prompt cards, and built-in Axiom entrypoints only.

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
      "min_axiom_version": "0.1.0"
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

The default remote URL is a placeholder until the separate `axiom-skills` repository exists. If loading it fails and fallback is enabled, onboarding and skill commands use the bundled local fixture registry.

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
```

`axiom skill update --check` compares installed skill versions with the active registry. It reports available updates but does not install them yet.

## Trust Rules

If a registry entry includes `sha256`, Axiom verifies the downloaded manifest or bundle content before installing it. A checksum mismatch fails the install.

If a custom registry is used, Axiom warns:

```text
Custom registries can change agent behavior. Only use registries you trust.
```

When `allow_untrusted_registries = false`, installs from custom remote registries require terminal confirmation. The bundled fixture registry and explicit local development registries are trusted for Stage 7.
