# Axiom Agent

A terminal-first Rust CLI that routes requests through Skill Lens, executes safe modular skills, handles project coding tasks, and records proof reports.

## What is Axiom Agent?

Axiom is a terminal agent that understands what you need and picks the right tools.

When you send a message, Skill Lens scans your installed skills and selects the ones that match your request. Those skill cards get injected as compact context for the LLM. If the model asks to run a tool, Axiom Engine executes it with safety checks and sends the result back.

Coder Mode handles project-level tasks: scan the workspace, generate a plan, show diffs, apply changes after approval, run tests.

Proof Mode records what happened during each session: what you asked, which skills were selected, what tools ran, what files changed, and how the session ended.

## Current Status

This repository contains Axiom `1.0.0-rc.1`, the first public v1 release
candidate. Install it from the dedicated npm RC channel:

```bash
npm install -g axiom-agent@rc
```

RC builds are published separately from the stable `latest` channel. Final
promotion depends on the evidence tracked in
[the v1 RC checklist](docs/V1_RC_CHECKLIST.md).

What works:
- Terminal CLI with onboarding, chat, coding mode, and proof commands
- LLM chat through OpenAI-compatible providers and Cloudflare AI Gateway
- Configurable multi-step agent loop with native tool calls, fallback tool blocks, retries, cancellation, context compaction, todo state, and usage/cost caps
- OpenAI-compatible SSE transport with fragmented content/tool-call accumulation and safe live terminal rendering
- Transition-level durable chat sessions with `axiom sessions` and `axiom resume <session-id>`
- Skill Lens intent matching and skill card injection
- 11 built-in skills with TOML manifests
- Built-in tool execution (file.read, file.write, project.scan, web.fetch, git.status, git.diff)
- Skill registry with cache, local fixture fallback, bundles, install tracking, lifecycle state, trust levels, and skill updates
- Core binary updater plumbing with release channels, checksum verification, staged install, and rollback
- Offline mock provider for demos and integration tests
- One-shot `axiom run` command for scripts and non-interactive demos
- Test-safe `AXIOM_HOME` config isolation
- Coder mode on the canonical capped agent runtime, with plan-to-patch checks, per-hunk approval, conflict-aware hunks, recovery checkpoints, project-aware tests, and bounded correction attempts
- Central side-effect policy for built-in tools and Coder file writes, with allow/ask/deny decisions recorded in Proof Mode
- Line editing, bracketed paste, input history, multiline capture, theme presets, plain redirected output, and durable `!show` tool-output references
- Proof Mode with JSON traces, Markdown reports, policy/approval events, and secret redaction
- npm wrapper with version-bound GitHub release downloads and checksum verification

What is not done yet:
- External executable skill binaries are not supported.
- Core binary updates require published GitHub Releases before normal installs can activate.
- Stable v1 promotion still requires RC feedback, native credential-store checks, and release-owner sign-off.
- A full-screen TUI is a post-v1 enhancement; v1 ships the inline chat UI.
- No desktop, mobile, or web interfaces.

## Features

- **9 Rust crates** in a Cargo workspace: agent, cli, core, llm, engine, lens, coder, proof, update
- **Skill Lens**: rule-based intent matching picks relevant skills per message
- **11 installed skill cards**: file.read, file.write, project.scan, web.fetch, shell.powershell.safe, shell.bash.safe, shell.zsh.safe, git.status, git.diff, python.write, python.run
- **Six executable built-ins**: file.read, file.write, project.scan, web.fetch, git.status, git.diff
- **Provider switching**: guided setup, credential-manager storage, catalog-only model discovery, OpenAI-compatible endpoints, and Cloudflare AI Gateway
- **Coder Mode**: scan, plan, verify, review each hunk, checkpoint, apply, and test
- **Proof Mode**: JSON traces and Markdown reports with policy decisions, approvals, and secret redaction
- **Safety**: workspace-only file access, blocked secret paths, centralized policy for built-in tools/Coder writes, separately allowlisted and approved Coder tests, and pre-write recovery checkpoints
- **Registry**: remote HTTPS registry with SHA-256 verification, cache, bundled fallback, trust checks, and controlled skill updates
- **Core updater**: release channel checks, verified binary downloads, staged install, backups, and rollback

## Installation

Install from npm (recommended):

```bash
npm install -g axiom-agent@rc
axiom
```

From source:

```bash
cargo build -p axiom-cli
cargo run -p axiom-cli
```

Local npm testing with a source-built binary:

```bash
cargo build -p axiom-cli --release
export AXIOM_AGENT_BINARY_PATH="$PWD/target/release/axiom"
npm install -g .
axiom
```

Windows:

```powershell
cargo build -p axiom-cli --release
$env:AXIOM_AGENT_BINARY_PATH = "C:\Axiom\target\release\axiom.exe"
npm install -g .
axiom
```

## First Run

Run `axiom` with no arguments. If no config exists, onboarding starts. It offers Groq, OpenRouter, Gemini, GitHub Models, NVIDIA NIM, OpenAI, Cloudflare, Ollama, LM Studio, and custom OpenAI-compatible endpoints. Local providers can run without an API key. After setup, it installs the essential skill bundle for your OS.

Interactive onboarding can configure one provider or two at once. Credential
paste is hidden and saved through the native OS credential manager; Axiom then
fetches the model catalog without sending an inference request and lets you
search or choose a model. Each provider remembers its own model.

```bash
axiom provider list
axiom provider use openrouter
axiom model list --filter free
axiom model use openrouter/free
```

Axiom saves config to:
- Windows: `%APPDATA%\axiom-agent\config.toml`
- Linux/macOS: `~/.config/axiom-agent/config.toml`

API keys are never stored in config. Provider entries reference environment variable names.

For a rate-limited hosted free option, set `OPENROUTER_API_KEY` and choose the `openrouter` preset, which defaults to `openrouter/free`. Groq, Gemini, and NVIDIA NIM also offer account-dependent free access or tiers. Hosted limits and model availability can change; Ollama and LM Studio run models locally. See [provider setup](docs/PROVIDERS.md).

For scripted setup and tests:

```bash
AXIOM_HOME=/tmp/axiom-test-home \
cargo run -p axiom-cli -- onboarding --non-interactive --provider mock --workspace ./demo-workspace --yes
```

`AXIOM_HOME` changes the config root for the process. When set, Axiom writes `config.toml`, `skills/installed_skills.json`, `proofs/`, `updates/`, and `registry-cache/` under that directory instead of the real user config directory.

The built-in `mock` provider is for tests and demos. It returns deterministic responses and does not require API keys.

## Chat Mode

`axiom` or `axiom chat` opens terminal chat.

For one-shot non-interactive chat:

```bash
axiom run "hello"
axiom run "read README.md and summarize it"
axiom run "hello" --no-tools --no-proof
```

`axiom run` uses the same Skill Lens, identity context, capped multi-step tool loop, and Proof Mode recording as normal chat. Set `[agent].loop_enabled = false` to temporarily recover the legacy single-tool behavior while upgrading.

The loop compacts older history before it reaches the configured context ceiling and shows provider-reported turn/session usage after every response. Cost is shown only when both token rates are configured for the active model:

```toml
[agent]
max_tokens = 200000
max_cost_usd = 1.0
session_budget_usd = 5.0
monthly_budget_usd = 25.0
input_cost_per_million_tokens = 2.0
output_cost_per_million_tokens = 8.0
```

Rates are model- and provider-specific; update them when you switch models.
When both rates are present, `max_cost_usd` is enforced per turn, persistent
session/monthly budgets can block the next Chat or Coder provider call, and all
Chat/Coder model costs are recorded in the local UTC-month ledger. Inspect it
with:

```bash
axiom cost
```

Both token rates must be configured together; a partial pair is invalid. With
neither rate configured, persistent cost enforcement and new cost recording
are unavailable. Axiom reports that state instead of inventing an estimate.
Proof Mode includes estimates only when pricing is complete.

Chat commands:

```text
!help         Show available commands
!exit         Leave chat
!multi        Enter a multiline prompt; finish with !send or discard with !cancel
!show [ID]    List or display durable full tool output
!checkpoints  List recovery snapshots created before agent writes
!restore ID   Restore a recovery snapshot after confirmation
!model use X  Switch model
!provider use X  Switch provider
!clear        Clear history
!proof on/off Toggle proof recording
!skills       List installed skills
!lens on/off  Toggle Skill Lens
```

`!multi` preserves blank lines and submits the whole block as one recorded turn. For scripts, pass the same multiline text to `axiom run` as a single argument or via your shell's normal quoting mechanism.

Each chat is stored atomically after internal runtime transitions. Before an
agent file write, Axiom creates and persists a workspace checkpoint before the
side effect can run. The header prints the session ID, and sessions can be
listed or resumed later:

```bash
axiom sessions
axiom resume session-1234abcd
```

Resuming restores provider, model, workspace, compacted history, todo state, Skill Lens setting, and the session usage ledger. A resumed turn does not replay earlier tool calls.

When Lens is on, chat shows which skills were selected:

```text
Axiom Lens: selected python.write, file.write
```

Chat uses Axiom's blood-red terminal theme by default. Interactive terminals
support persistent input history and bracketed paste; redirected input/output
uses a plain line-oriented fallback. Set `NO_COLOR=1` for plain output, or add
one of the supported palette names to `config.toml`:

```toml
[ui]
theme = "blood_red" # blood_red, ash, high_contrast, none
```

`[ui].color = false` and `theme = "none"` also disable ANSI styling. Long tool
results get a bounded preview and a durable ID; use `!show ID` to inspect the
full saved result.

Tool skills can execute during chat. The model requests tools using a provider-independent `axiom-tool` block, Axiom Engine runs the tool, and the result goes back to the model.

## Skill Lens

Skill Lens analyzes each message and picks a small set of matching skills. This keeps LLM context compact and improves accuracy.

Current signals:
- Python keywords select `python.write`, `python.run`
- URL or web keywords select `web.fetch`
- File keywords select `file.read`, `file.write`
- Git keywords select `git.status`, `git.diff`
- Shell keywords select platform-specific safe shell skills

Project-level coding requests can route to Coder Mode.

## Skills and Registry

Skills are versioned TOML manifests with optional `[llm_card]` sections. Types: `prompt` (LLM guidance), `tool` (executable), `workflow`, `guard`. Manifest dependencies are co-selected before dependent skills, lifecycle/trust checks still apply at execution time, and the combined selected-card context is bounded by the Lens budget.

The published [`axiom-skills`](https://github.com/NexaraAI/axiom-skills) registry is the single source of truth for skill manifests. `fixtures/skill-registry/` is the compile-time source and test oracle for the immutable starter registry embedded in the CLI; packaged binaries materialize it under `AXIOM_HOME/bundled-registry/<generation>` for offline onboarding without needing this checkout. At present, only six built-in IDs execute: `file.read`, `file.write`, `project.scan`, `web.fetch`, `git.status`, and `git.diff`. Other installed skills, including `python.write` and `python.run`, provide LLM guidance but do not execute code. External executable skill metadata may be installed but cannot yet be dispatched.

You can find installed skills in the platform config directory. You can install bundles (groups of skills for a platform) or individual skills.

```bash
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
```

The default remote registry is `https://raw.githubusercontent.com/NexaraAI/axiom-skills/main/registry.json`. You can change the registry URL in config. If fetching fails, onboarding uses the starter registry compiled into the binary and materialized under `AXIOM_HOME/bundled-registry/<generation>`; installed binaries never need this repository checkout at runtime.

Installed skills carry lifecycle state and trust metadata. Skill Lens skips disabled, incompatible, quarantined, and blocked skills, and they cannot execute. External executable skill binaries are not supported yet; Axiom installs unknown external entrypoints as disabled or quarantined.

## Coder Mode

`axiom code` opens the coding assistant. It scans the workspace, builds project context, asks the LLM for a plan, validates base hashes and minimal hunks, shows diffs, checkpoints affected paths, and writes files after approval. Existing-file edits conflict instead of silently overwriting ambiguous external changes. Large patches are capped and require an additional confirmation after the configured scope threshold.

```bash
axiom code --scan        # Scan workspace
axiom code --plan-only "task"  # Plan without writing
axiom code --apply "task"      # Plan and apply
axiom code --test        # Run detected test command
```

Coder mode does not commit, push, deploy, delete files, or run arbitrary commands.

Interactive Coder also supports `!checkpoints` and `!restore CHECKPOINT_ID`. If an approved write fails part-way through, Axiom attempts to restore its checkpoint automatically. If detected tests fail, bounded test output is sent back for up to `[coder].max_correction_attempts`; every correction is revalidated, re-approved, and checkpointed. Plan, patch, correction, and conversational Coder calls share the persistent session/monthly budget ledger shown by `axiom cost`.

## Proof Mode

Proof Mode records terminal agent sessions. Each task gets a JSON trace and a Markdown report.

Recorded data: user request, provider, model, selected skills, runtime
transitions, policy decisions, tool calls, approvals, file writes, commands,
test results, errors, and final response. Axiom redacts secrets.

```bash
axiom proof list
axiom proof latest
axiom proof show latest
axiom proof export latest --format markdown
axiom proof clean --older-than 30
```

Axiom stores proofs at:
- Windows: `%APPDATA%\axiom-agent\proofs`
- Linux/macOS: `~/.config/axiom-agent/proofs`

## Safety Model

Axiom enforces workspace-only file access. It blocks secret-looking paths
(`.env`, `*.pem`, `*.key`, `credentials.json`). The `[policy]` configuration
sets filesystem, network, process, and Git side effects to `allow`, `ask`, or
`deny`; safe defaults allow reads and ask before side effects. Policy decisions
and approvals are recorded for policy-routed actions. Coder's detected test
commands retain a separate strict allowlist and approval gate. Tool execution
stays within built-in executors; Axiom rejects external binaries.

Coder mode shows plans and diffs before writes, validates patch scope, and creates a recovery checkpoint before applying an approved patch.

`web.fetch` is HTTPS-only by default. It rejects embedded credentials,
local/private/loopback/reserved destinations, redirects, and DNS results that
resolve to blocked addresses. Verified public addresses are pinned for the
request to reduce DNS-rebinding risk, and response bytes are stopped while
streaming as soon as the configured cap is crossed. System proxy discovery is
off by default.

The current config schema supports exact and subdomain host controls:

```toml
[network]
web_fetch_https_only = true
web_fetch_allowed_hosts = [] # empty allows any otherwise-safe public host
web_fetch_denied_hosts = ["blocked.example.com", "*.untrusted.example"]
web_fetch_use_system_proxy = false
```

An exact pattern matches only that host. `*.example.com` matches subdomains
such as `docs.example.com`, not the apex `example.com`. Deny patterns are
evaluated first. An allowlist cannot permit localhost, `.local`, private,
loopback, reserved, or private-resolving targets, and setting
`web_fetch_https_only = false` does not relax those hard blocks. These controls
apply to model-invoked `web.fetch`, not configured provider endpoints. Provider
transports separately require HTTPS for remote endpoints, accept plain HTTP only
for literal loopback development endpoints, reject URL credentials, and do not
follow redirects; local Ollama and LM Studio remain usable.

Core updates verify `SHA256SUMS` before staging a binary. Missing or mismatched checksums block installation. Axiom does not execute release scripts.

The release-focused security analysis is maintained in the
[repository threat model](docs/THREAT_MODEL.md).

## Core Updates

```bash
axiom update status
axiom update check
axiom update install
axiom update rollback
axiom update set-channel stable
axiom update set-policy notify
```

`stable` uses normal releases, `nightly` can use prereleases, and `dev` is for local mocked release metadata. Running from Cargo `target/` lets you check for updates but blocks self-replacement.

## Development Setup

```bash
cargo fmt
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo deny check
node scripts/smoke-test.js
node scripts/e2e-test.js
node scripts/release-check.js
node scripts/security-check.js
```

Doctor check:

```bash
cargo run -p axiom-cli -- doctor
```

After a config migration, `axiom doctor --json` should report matching loaded
and supported config schema versions with migration disabled. It validates host
pattern syntax while loading config, but intentionally does not contact
allowlisted hosts or probe the system proxy. Use `axiom config list` to inspect
the effective `[network]` controls.

Offline demo:

```bash
cargo run -p axiom-cli -- onboarding --non-interactive --provider mock --workspace ./demo-workspace --yes
cargo run -p axiom-cli -- run "read README.md and summarize it"
cargo run -p axiom-cli -- code --plan-only "explain how to add a test"
```

See `docs/TESTING.md` and `docs/DEMO.md` for isolated local runs without API keys.

## Roadmap

- Current: finish the local v1 implementation and close automated release gates.
- Next: cut `v1.0.0-rc.1` only after the cross-platform, provenance, credential,
  accessibility, and operator gates in [the v1 RC checklist](docs/V1_RC_CHECKLIST.md).
- Later: independently reviewed external executable skills, a full-screen TUI,
  remote registry publishing workflows, and app layers.

## License

MIT

Copyright © 2026 DemonZDevelopment.
