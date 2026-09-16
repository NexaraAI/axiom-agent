# Skills

Skills use TOML manifests and compact LLM-facing skill cards.

Required manifest fields:

- `schema_version` (`1.0`; legacy `0.1` remains readable)
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

Optional v1 metadata includes `keywords`, `examples`, `depends_on`, `provides`, `hooks`, `side_effects`, `idempotent`, `cache_key`, `input_schema`, and `output_schema`. Install and registry refresh reject unsupported future schemas, malformed IDs, duplicate dependencies/capabilities, invalid hooks, and invalid schema shapes. Lens places available dependencies before the skill that needs them; missing and cyclic dependencies block selection and execution.

## Execution

Only these built-in tool IDs execute in the current stage: `file.read`, `file.write`, `project.scan`, `web.fetch`, `git.status`, and `git.diff`. They are registered as compiled-in typed executors. All other installed skills are prompt context only until a sandboxed external executor model lands. In particular, `python.write` and `python.run` can guide the model but cannot execute Python themselves; `workflow` and `guard` skills also do not run yet.

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

## Manifest hooks

A manifest may declare hooks that the agent loop fires around the skill it is
executing:

```toml
[hooks]
pre = "hook.snapshot"         # after the call is accepted, before the skill runs
post = "hook.lint_changed"    # after the skill succeeds
on_error = "hook.git_restore" # after the skill fails
```

Each hook names another skill, which must be executable in the session (a `tool`
skill backed by a registered executor); otherwise the hook is recorded as
unavailable and the turn continues. Hooks are ordinary tools: they run through
the same `[policy]` side-effect rules, approval hook, and Proof Mode audit as any
other call.

The hook receives the arguments of the tool that triggered it, so a post-write
lint hook sees the path that changed. That is the whole hook contract — a hook
skill should declare an input schema compatible with the tool it attaches to.

Every guard is reported in the tool observation instead of being silently
dropped:

- **Re-entrancy** — a hook never fires hooks of its own, and a skill cannot hook
  itself; both are recorded as `skipped_reentrancy`.
- **Budget** — hooks share the turn's wall clock and a per-turn allowance (24 by
  default), and each hook is capped at 30 seconds.
- **Failure** — a failing hook is recorded for the model to see; it never fails
  the turn.

Hook results are appended to the tool result the model receives, so a failing
`post` hook (a lint error, for example) can steer the next iteration.

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

The registry flow supports remote manifests and prompt cards, plus the six built-in entrypoints listed above. An external skill install currently stores its metadata but cannot be dispatched: unknown executable entrypoints are installed disabled or quarantined and never executed.

Lifecycle details: [SKILL_LIFECYCLE.md](SKILL_LIFECYCLE.md).

## Local Registry Fixture

The single source of truth for published skill manifests is the separate `axiom-skills` registry. The Axiom Agent repository includes `fixtures/skill-registry/` as the compile-time source and test oracle for the immutable starter registry embedded in the CLI. Packaged binaries materialize that starter registry under `AXIOM_HOME/bundled-registry/<generation>` for offline setup; they do not depend on the checkout at runtime. It contains:

- `registry.json`
- OS essential bundles
- skill manifests and README files

The npm installer installs the Axiom binary; it does not embed the remote skill catalog. The binary does include the small starter bundle used for offline onboarding.

## Token budgets

`llm_card.token_budget` contributes to the deterministic Lens selection budget. Cards are ranked, dependencies are placed first, and selection stops before the configured combined card budget is exceeded. Agent context compaction independently bounds older conversation history while preserving identity, todo state, selected skill context, and recent observations.
