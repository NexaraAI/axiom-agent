# Skill Lifecycle

Axiom tracks installed skills as local records plus a copied `skill.toml` manifest. The record acts as the safety gate. Both Skill Lens and Axiom Engine check it before selecting or executing a skill.

## States

- `enabled`: Lens can select the skill, and Engine can execute it (if it is a supported tool).
- `disabled`: the user turned it off. Lens ignores it and Engine blocks it.
- `update_available`: the installed version works, but the registry has a newer compatible version.
- `incompatible`: the skill does not match the Axiom version or platform. Axiom disables it.
- `quarantined`: the manifest has an unsupported entrypoint (e.g., an external executable). Axiom disables it.
- `failed_update`: the last update attempt failed. Axiom keeps the old manifest and version.

Skill Lens never selects disabled, incompatible, quarantined, or blocked skills. Engine never executes them.

## Trust Levels

- `trusted`: bundled skills and skills from the official NexaraAI registry.
- `community`: valid custom registry skills with enough metadata to review.
- `untrusted`: custom registry skills with a missing checksum, suspicious metadata, or unsupported entrypoints.
- `blocked`: incompatible or known unsupported skills. Axiom refuses to install or execute them.

Community installs show a warning. Untrusted installs require explicit confirmation. Blocked skills fail closed.

## Compatibility

Axiom checks:

- `min_axiom_version`
- optional `max_axiom_version`
- the platform
- `skill_type`
- `entrypoint`
- permissions and risk metadata

Axiom does not support external executable entrypoints yet. It only accepts `prompt-only` and built-in entrypoints like `builtin:file.read`.

## Updates

Commands:

```powershell
axiom skill update --check
axiom skill update SKILL_ID
axiom skill update --all
axiom skill update --apply-patches
```

Update checks compare `installed_skills.json` with the configured registry. Output shows: installed version, available version, state, source, trust level, update type, and compatibility result.

Update types: `patch`, `minor`, `major`. Minor and major updates require confirmation. `--apply-patches` applies compatible patch updates when `skills.auto_update_policy = "auto-patch"`.

If an update fails, Axiom keeps the old manifest and installed version, sets `state = "failed_update"`, and records `last_update_error`.

## Policy

```toml
[skills]
auto_update_policy = "notify"
```

Supported values:

- `manual`: Axiom never checks unless you run a skill update command.
- `notify`: Axiom shows available updates but does not install them.
- `auto-patch`: Axiom applies patch updates through `--apply-patches`; minor and major updates still require confirmation.

Chat startup makes no registry network calls. If a local cache shows available updates, chat prints one short notice.

## Registry Cache

Axiom stores registry cache files under the user config directory:

```text
registry-cache/
  registry.json
  bundles/
  skills/
  cache-metadata.json
```

Cache metadata records source URL, fetch time, TTL, last error, and whether Axiom used stale data. If the cache is valid, Axiom uses it. If a refresh fails and stale cache exists, Axiom uses stale cache with a warning. If no cache exists, Axiom falls back to the bundled fixture registry.

## Health

```powershell
axiom skill health
axiom skill reset-stats SKILL_ID
```

A successful execution increments `success_count`, updates `average_latency_ms`, and clears `last_runtime_error`. A failed execution increments `failure_count` and records the error. Axiom does not auto-disable trusted skills on failure, but repeated failures show up in health output.

## Enable, Disable, Remove

```powershell
axiom skill enable SKILL_ID
axiom skill disable SKILL_ID
axiom skill remove SKILL_ID
```

You can only enable compatible, non-blocked skills. Disabling a skill keeps the installed manifest but prevents Lens selection and Engine execution. Removing a skill deletes the installed record and local manifest directory but leaves the registry cache intact.
