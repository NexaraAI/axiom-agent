# Skill Lifecycle

Axiom tracks installed skills as local records plus a copied `skill.toml` manifest. The record is the safety gate. Skill Lens and Axiom Engine both consult it before a skill can be selected or executed.

## States

- `enabled`: the skill can be selected and, if it is a supported tool, executed.
- `disabled`: the user turned it off. Lens ignores it and Engine blocks it.
- `update_available`: the installed version is usable, but the registry has a newer compatible version.
- `incompatible`: the skill does not match the current Axiom version or platform. It is disabled.
- `quarantined`: the manifest has an unsupported entrypoint, such as an external executable. It is disabled.
- `failed_update`: the last update attempt failed. Axiom keeps the old manifest and version.

Disabled, incompatible, quarantined, and blocked skills are not selected by Skill Lens and cannot execute.

## Trust Levels

- `trusted`: bundled skills and skills from the official NexaraAI registry.
- `community`: valid custom registry skills with enough metadata to review.
- `untrusted`: custom registry skills with missing checksum, suspicious metadata, or unsupported entrypoints.
- `blocked`: incompatible or known unsupported skills. They cannot be installed or executed.

Community installs show a warning. Untrusted installs require explicit confirmation. Blocked skills fail closed.

## Compatibility

Axiom checks:

- `min_axiom_version`
- optional `max_axiom_version`
- current platform
- `skill_type`
- `entrypoint`
- permissions and risk metadata

External executable entrypoints are not supported yet. Axiom only accepts `prompt-only` and built-in entrypoints such as `builtin:file.read`.

## Updates

Commands:

```powershell
axiom skill update --check
axiom skill update SKILL_ID
axiom skill update --all
axiom skill update --apply-patches
```

Update checks compare `installed_skills.json` with the configured registry and show current version, available version, state, source, trust level, update type, and compatibility result.

Update types are `patch`, `minor`, and `major`. Minor and major updates require confirmation. `--apply-patches` only applies compatible patch updates when `skills.auto_update_policy = "auto-patch"`.

If an update fails, Axiom keeps the old manifest and old installed version, sets `state = "failed_update"`, and records `last_update_error`.

## Policy

```toml
[skills]
auto_update_policy = "notify"
```

Supported values:

- `manual`: never check unless the user runs a skill update command.
- `notify`: show that updates are available, but do not install.
- `auto-patch`: allow patch updates through `--apply-patches`; minor and major updates still require confirmation.

Chat startup does not make registry network calls. If a local cache already shows updates, chat prints one short notice.

## Registry Cache

Axiom stores registry cache files under the user config directory:

```text
registry-cache/
  registry.json
  bundles/
  skills/
  cache-metadata.json
```

Cache metadata records source URL, fetch time, TTL, last error, and whether stale cache was used. If the cache is valid, Axiom uses it. If refresh fails and stale cache exists, Axiom uses stale cache with a warning. If no cache exists, Axiom can fall back to the bundled fixture registry.

## Health

```powershell
axiom skill health
axiom skill reset-stats SKILL_ID
```

Successful execution increments `success_count`, updates `average_latency_ms`, and clears `last_runtime_error`. Failed execution increments `failure_count` and records the last runtime error. Trusted skills are not automatically disabled just because they fail, but repeated failures are shown in health output.

## Enable, Disable, Remove

```powershell
axiom skill enable SKILL_ID
axiom skill disable SKILL_ID
axiom skill remove SKILL_ID
```

Enable only works for compatible, non-blocked skills. Disable keeps the installed manifest but prevents selection and execution. Remove deletes the installed record and local manifest directory, but it does not clear the registry cache.
