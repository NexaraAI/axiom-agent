# Skills

Skills use TOML manifests and compact LLM-facing skill cards.

Required manifest fields:

- `id`
- `name`
- `version`
- `description`
- `risk_level`
- `permissions`
- `platforms`
- `entrypoint`
- `author`
- `license`
- `category`
- `skill_type`
- `min_axiom_version`

Supported skill types:

- `prompt`
- `tool`
- `workflow`
- `guard`

The LLM receives selected `SkillCard` data, not full manifests.

## Execution

Only `tool` skills execute in the current stage. `prompt`, `workflow`, and `guard` skills parse and can guide the model, but Axiom does not run them yet.

The provider-independent tool request format:

````text
```axiom-tool
{"skill_id":"file.read","arguments":{"path":"README.md"}}
```
````

Built-in executors:

- `file.read`
- `file.write`
- `project.scan`
- `web.fetch`
- `git.status`
- `git.diff`

Axiom constrains execution to the active workspace where applicable. It blocks secret-looking paths (`.env`, private keys, `.pem`, `.key` files) by default. Medium-risk actions prompt for terminal approval unless policy allows auto execution.

## Installed Skills

Axiom stores installed skills under the config directory:

```text
skills/
  installed_skills.json
  file.read/
    skill.toml
    README.md
```

`installed_skills.json` tracks skill id, version, install and update timestamps, source, registry URL, manifest URL, optional checksum, enabled status, lifecycle state, trust level, update errors, runtime errors, and health counters.

Sources:

- `remote`: the configured remote registry.
- `bundled`: the built-in fixture fallback.
- `local`: explicit local registry development installs.

## Registry Installs

You can search and install skills from the configured registry:

```powershell
axiom skill search python
axiom skill install python.write
axiom skill install-bundle essential.windows
axiom skill update --check
axiom skill update python.write
axiom skill update --all
axiom skill update --apply-patches
axiom skill health
axiom skill disable python.write
axiom skill enable python.write
axiom skill reset-stats python.write
axiom skill remove python.write
```

For local development and tests:

```powershell
axiom skill install python.write --from-local-registry fixtures/skill-registry
```

The registry flow supports remote manifests and prompt cards, plus built-in entrypoints that Axiom already implements. Unknown external executable entrypoints install as disabled or quarantined so they cannot run.

Lifecycle details: [SKILL_LIFECYCLE.md](SKILL_LIFECYCLE.md).

## Local Registry Fixture

The Axiom Agent repository includes `fixtures/skill-registry/` for tests and offline fallback. It contains:

- `registry.json`
- OS essential bundles
- skill manifests and README files

Published registry skills live in the separate `axiom-skills` GitHub repository. The npm installer installs the Axiom binary; it does not embed the remote skill catalog.
