# Proof Mode

Axiom Proof Mode records what happened during important terminal agent tasks. It exists for transparency, debugging, safety review, and portfolio proof of work.

Proof Mode is enabled by default:

```toml
[proof]
enabled = true
default_format = "markdown"
trace_json = true
redact_secrets = true
auto_export_markdown = true
max_capture_chars = 4000
```

## Storage

Proof files are stored in the user config directory:

- Windows: `%APPDATA%\axiom-agent\proofs`
- Linux/macOS: `~/.config/axiom-agent/proofs`

The directory structure is:

```text
proofs/
2026-07-04/
session-abc123/
task-001.json
task-001.md
```

Each task gets a stable session id, task id, and event ids for recorded actions.

## What Is Recorded

Proof traces can record:

- user request;
- mode: chat, skill, coder, onboarding, or update;
- provider and model;
- workspace path;
- Axiom Lens selected skills and routing decision;
- tool calls and summaries;
- file reads and writes;
- approvals requested and the user decision;
- patches and diffs;
- safe commands and test results;
- recoverable errors;
- final response and summary.

Chat creates one proof task per user message. `axiom skill run` creates a skill proof task. Axiom Coder records plan-only, apply, and test flows.

## Redaction

Proof Mode never intentionally stores secrets. It redacts:

- API-key-looking values;
- Bearer tokens;
- `.env` style assignments;
- private key blocks;
- credential and token fields;
- long captured text over `proof.max_capture_chars`.

Secret-looking file paths such as `.env`, `.env.*`, `*.pem`, `*.key`, `credentials.json`, and `token.json` are blocked by execution safety before contents are read or written.

Markdown reports are designed to be readable and compact. JSON traces keep structured fields, but tool outputs and command output are summarized and redacted.

## Commands

```powershell
axiom proof list
axiom proof latest
axiom proof show latest
axiom proof show <task-id>
axiom proof export latest --format markdown
axiom proof export latest --format json
axiom proof open latest
axiom proof clean --older-than 30
```

`show` supports `latest`, full task ids, and unique partial task ids. `open` prints the report path in v0.1.

Inside chat:

```text
!proof on
!proof off
!proof status
!proof latest
```

## Portfolio And Debugging Use

Markdown proof reports are meant to be clean enough to review later. They show what was asked, what Axiom selected, what it executed, what it changed, which approvals were given, and how the task ended.

Proof Mode does not store raw provider headers, API tokens, full secret file contents, or huge command outputs.
