# Auto Update

Axiom has two update paths:

- Core binary updates through `axiom update`.
- Skill manifest updates through `axiom skill update`.

Core binary updates are documented in [UPDATER.md](UPDATER.md). The updater checks release metadata, resolves the correct platform asset, verifies `SHA256SUMS`, stages a binary, keeps a backup, and rolls back on failure. Normal user installs need published GitHub Release assets before update installation can succeed.

Skill updates are documented in [SKILL_LIFECYCLE.md](SKILL_LIFECYCLE.md).
