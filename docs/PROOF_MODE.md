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
retention_days = 30
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
- ordered agent transitions, cap outcomes, and usage ledger updates;
- centralized side-effect policy decisions;
- tool calls and summaries;
- core update checks, installs, rollback, channel changes, policy changes, and failures;
- skill update checks, skill update installs, enable, disable, remove, and reset-stats actions;
- file reads and writes;
- approvals requested and the user decision;
- pre-write workspace checkpoint references;
- patches and diffs;
- safe commands and test results;
- recoverable errors;
- final response and summary.

Chat creates one proof task per user message. `axiom run` creates the same chat proof trace for a single non-interactive message. `axiom update` commands create update proof tasks. `axiom skill run`, skill update commands, and lifecycle commands create skill proof tasks. Axiom Coder records plan-only, apply, and test flows.

New configurations prune valid dated proof directories older than 30 days when
they persist a new proof. To avoid surprising upgrades, existing configurations
that predate `retention_days` load with it disabled (`0`) until you opt in. Set
`retention_days = 0` to disable automatic pruning, or use `axiom proof clean
--older-than DAYS` for an explicit cleanup. Automatic retention never follows
symbolic links or Windows directory junctions outside the proof root.

## Redaction

Proof Mode applies mandatory best-effort secret redaction before durable
storage. No heuristic can guarantee recognition of every arbitrary secret, so
review proof content before sharing it. Axiom redacts:

- the original user request before JSON or Markdown serialization;
- API-key-looking values;
- standalone tokens with known provider formats;
- exact secret values present in credential environment variables;
- Bearer tokens;
- `.env` style assignments;
- private key blocks;
- credential and token fields;
- long captured text over `proof.max_capture_chars`.

Redaction and capture limits are mandatory for durable proof artifacts. The
legacy `proof.redact_secrets` setting remains readable for config compatibility,
but setting it to `false` cannot disable proof redaction.

Execution safety blocks secret-looking file paths (`.env`, `.env.*`, `*.pem`, `*.key`, `credentials.json`, `token.json`) before Axiom reads or writes their contents.

Interactive provider credentials are stored by the native OS credential
manager. Proof records capture the provider and model names, never the
credential-manager entry value or request authorization header.

Markdown reports are readable and compact. JSON traces keep structured fields, but summarize and redact tool outputs and command output.

`axiom proof export` writes a privacy warning to stderr because exports can
still contain prompts, code, project paths, and operational metadata after
automatic redaction. Review the exported artifact before posting or sending it.

Full successful tool results may also be saved under the session `outputs/`
directory for `!show ID`. The saved JSON is redacted before persistence and the
terminal prints a bounded preview; Proof independently stores a redacted summary.
Durable session history and terminal input history use the same mandatory
secret redactor before writing.

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

Proof Mode intentionally does not capture raw provider headers or full secret
file contents, and it bounds command output. Recognized API tokens are redacted;
operators should still treat proof files as potentially sensitive project data.

Core update proofs record compact metadata: current version, available version, channel, policy, asset name, checksum result, install result, rollback result, and error summary where available. They exclude full release JSON.

Skill update proofs record compact metadata: old version, new version, registry source, selected action, status, and error summary where available. They exclude full registry contents.
