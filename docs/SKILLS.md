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

Only `tool` skills execute in the current stage. `prompt`, `workflow`, and `guard` skills parse and can guide the model, but they do not run yet.

The provider-independent tool request format is:

````text
```axiom-tool
{"skill_id":"file.read","arguments":{"path":"README.md"}}
```
````

Implemented built-in executors:

- `file.read`
- `file.write`
- `project.scan`
- `web.fetch`
- `git.status`
- `git.diff`

Execution is constrained to the active workspace where applicable. Secret-looking paths such as `.env`, private keys, `.pem`, and `.key` files are blocked by default. Medium-risk actions ask for terminal approval unless policy allows auto execution.

## Installed Skills

Installed skills are stored under the Axiom config directory:

```text
skills/
  installed_skills.json
  file.read/
    skill.toml
    README.md
```

`installed_skills.json` tracks skill id, version, install time, source, registry URL, manifest URL, optional checksum, and enabled status.

Sources are:

- `remote` for the configured remote registry.
- `bundled` for the built-in fixture fallback.
- `local` for explicit local registry development installs.

## Registry Installs

Skills can be searched and installed from the configured registry:

```powershell
axiom skill search python
axiom skill install python.write
axiom skill install-bundle essential.windows
axiom skill update --check
```

For local development and tests:

```powershell
axiom skill install python.write --from-local-registry fixtures/skill-registry
```

Stage 7 supports remote manifests and prompt cards, plus built-in entrypoints already implemented by Axiom. Unknown external executable entrypoints install disabled so they cannot run.

## Local Registry Fixture

The main Axiom Agent repository includes `fixtures/skill-registry/` for tests and offline fallback. It contains:

- `registry.json`
- OS essential bundles
- skill manifests and README files

Future published skills will come from a separate `axiom-skills` GitHub repository. The later npm installer will install the Axiom binary; it will not embed the whole remote skill catalog.
