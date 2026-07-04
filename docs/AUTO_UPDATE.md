# Auto Update

Axiom has two update paths:

- Core binary updates through `axiom update`.
- Skill manifest updates through `axiom skill update`.

Core binary updates are documented in [UPDATER.md](UPDATER.md). The updater can check release metadata, resolve the correct platform asset, verify `SHA256SUMS`, stage a binary, keep a backup, and roll back. Normal user installs require published GitHub Release assets before update installation can succeed.

Skill updates are documented in [SKILL_LIFECYCLE.md](SKILL_LIFECYCLE.md).
