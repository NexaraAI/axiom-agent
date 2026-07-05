# Proof Mode

Proof Mode records what happened during terminal agent tasks. Use it for transparency, debugging, safety review, and portfolio proof of work.

Proof Mode is on by default:

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

Axiom stores proof files in the user config directory:

- Windows: `%APPDATA%\axiom-agent\proofs`
- Linux/macOS: `~/.config/axiom-agent/proofs`

If you set `AXIOM_HOME`, Axiom stores proofs under `$AXIOM_HOME/proofs` instead. Integration tests use this to avoid writing to real user config.

The directory structure:

```text
proofs/
  2026-07-04/
    session-abc123/
      task-001.json
      task-001.md
```

Each task gets a stable session id, task id, and event ids for recorded actions.

## What Gets Recorded

Proof traces can record:

- user request;
- mode: chat, skill, coder, onboarding, or update;
- provider and model;
- workspace path;
- Axiom Lens selected skills and routing decision;
- tool calls and summaries;
- core update checks, installs, rollback, channel changes, policy changes, and failures;
- skill update checks, skill update installs, enable, disable, remove, and reset-stats actions;
- file reads and writes;
- approvals requested and the user decision;
- patches and diffs;
- safe commands and test results;
- recoverable errors;
- final response and summary.

Chat creates one proof task per user message. `axiom run` creates the same chat proof trace for a single non-interactive message. `axiom update` commands create update proof tasks. `axiom skill run`, skill update commands, and lifecycle commands create skill proof tasks. Axiom Coder records plan-only, apply, and test flows.

## Redaction

Proof Mode never stores secrets. It redacts:

- API-key-looking values;
- Bearer tokens;
- `.env` style assignments;
- private key blocks;
- credential and token fields;
- long captured text over `proof.max_capture_chars`.

Execution safety blocks secret-looking file paths (`.env`, `.env.*`, `*.pem`, `*.key`, `credentials.json`, `token.json`) before Axiom reads or writes their contents.

Markdown reports are readable and compact. JSON traces keep structured fields, but summarize and redact tool outputs and command output.

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

`show` accepts `latest`, full task ids, and unique partial task ids. `open` prints the report path in v0.1.

Inside chat:

```text
!proof on
!proof off
!proof status
!proof latest
```

## Portfolio and Debugging Use

Markdown proof reports are clean enough to review later. They show what you asked, what Axiom selected, what it executed, what it changed, which approvals you gave, and how the task ended.

Proof Mode does not store raw provider headers, API tokens, full secret file contents, or large command outputs.

Core update proofs record compact metadata: current version, available version, channel, policy, asset name, checksum result, install result, rollback result, and error summary where available. They exclude full release JSON.

Skill update proofs record compact metadata: old version, new version, registry source, selected action, status, and error summary where available. They exclude full registry contents.
