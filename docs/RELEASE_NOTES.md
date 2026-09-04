# Axiom 1.0.0

Axiom 1.0 stable is ready on Windows, Linux (x86-64 and ARM64), and macOS.

Chat in the terminal, edit projects with review, resume sessions, chat from
Telegram or Discord, and keep a proof trail for everything the agent does.

## Install

```bash
npm install -g axiom-agent
axiom
```

The first launch walks through provider and model setup in about a minute.
Hosted providers, local Ollama or LM Studio, and custom OpenAI-compatible
endpoints are supported.

## Highlights

- Friendly guided onboarding with live model search and a key check that
  catches missing credentials before your first chat.
- Telegram and Discord bots (`axiom gateway run --telegram` / `--discord`)
  with `/models`, `/model`, `/provider`, and `/status` commands.
- Coder mode with plan review, per-hunk approval, recovery checkpoints,
  and project-aware tests.
- Durable sessions (`axiom sessions`, `axiom resume`), cost budgets
  (`axiom cost`), and proof reports for every turn.
- Fail-closed safety: workspace containment, allow/ask/deny side-effect
  policy, secret redaction, and verified installs.

## Upgrade note

Re-run `axiom setup` once after installing: it verifies saved keys and
offers messaging setup. No config migration is required.

Each attached binary has a SHA-256 checksum, an SPDX SBOM, and GitHub build
provenance. Verify with `gh attestation verify` as shown in docs/RELEASE.md.
