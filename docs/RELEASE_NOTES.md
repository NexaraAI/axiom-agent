# Axiom {{VERSION}}

Axiom {{VERSION}} is available on Windows, Linux (x86-64 and ARM64), and macOS.

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
- One secret redactor instead of two. The chat session had its own weaker
  copy of the JSON key rules, which missed `private_key` and could write it
  to persisted session state in the clear.
- A single tool result can no longer dominate a turn. Tool results are capped
  at 8,000 characters in the agent loop, which previously applied no bound at
  all. Full results are still saved and remain available to `!show`.
- `github.search` returns the ten fields worth reading instead of the full
  83-field GitHub API object, about 89% of which was derived URLs and flags.
- Fetch results reach the model as text rather than as re-escaped JSON, so
  newlines are newlines instead of `\n`.
- `test.run` applies the destructive-command blocklist, so a command
  supplied as a test can no longer reach around the guard that protects
  every other spawn.
- `lint.check` refuses secret-looking paths, matching every other
  file-reading tool.
- Credential and gateway/MCP secret variable names are validated by one
  rule set, so a config naming a reserved variable like `TEMP` fails at
  load time instead of when the gateway starts.
- The secret-path check that stops a symlink from pointing at a secret is
  now a single function rather than five copies, so it cannot be dropped
  in one place and left in four.

## Upgrade note

Re-run `axiom setup` once after installing: it verifies saved keys and
offers messaging setup. No config migration is required.

One behavior change to be aware of: an unknown provider/variant pair now
returns no model rather than a fallback from a second, stale table. The
provider defaults themselves are unchanged.

Each attached binary has a SHA-256 checksum, an SPDX SBOM, and GitHub build
provenance. Verify with `gh attestation verify` as shown in docs/RELEASE.md.
