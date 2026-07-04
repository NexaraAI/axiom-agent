# Axiom Agent

A terminal-first Rust CLI that routes requests through Skill Lens, executes safe modular skills, handles project coding tasks, and records proof reports.

## What is Axiom Agent?

Axiom is not a chatbot. It is a terminal agent that understands what you need and picks the right tools.

When you send a message, Skill Lens scans your installed skills and selects only the ones that match your request. Those skill cards get injected as compact context for the LLM. If the model asks to run a tool, Axiom Engine executes it with safety checks and sends the result back.

Coder Mode handles project-level tasks: scan the workspace, generate a plan, show diffs, apply changes after approval, run tests.

Proof Mode records what happened during each session: what was asked, which skills were selected, what tools ran, what files changed, and how it ended.

## Current Status

v0.5.0-beta. Terminal foundation is built and release assets are being validated.

What works:
- Terminal CLI with onboarding, chat, coding mode, and proof commands
- LLM chat through OpenAI-compatible providers and Cloudflare AI Gateway
- Skill Lens intent matching and skill card injection
- 11 built-in skills with TOML manifests
- Built-in tool execution (file.read, file.write, project.scan, web.fetch, git.status, git.diff)
- Skill registry with cache, local fixture fallback, bundles, install tracking, lifecycle state, trust levels, and skill updates
- Core binary updater plumbing with release channels, checksum verification, staged install, and rollback
- Offline mock provider for demos and integration tests
- One-shot `axiom run` command for scripts and non-interactive demos
- Test-safe `AXIOM_HOME` config isolation
- Coder mode with project scan, plan, patch, diff preview, safe writes, and test detection
- Proof Mode with JSON traces, Markdown reports, and secret redaction
- npm installer scaffold (not publicly released yet)

What is not done yet:
- npm package is not published. Install from source for now.
- GitHub Release assets are built by the release workflow from version tags.
- Streaming responses are not implemented.
- External executable skill binaries are not supported.
- Core binary updates require published GitHub Releases before normal installs can activate.
- No desktop, mobile, or web interfaces.

## Features

- **8 Rust crates** in a Cargo workspace: cli, core, llm, engine, lens, coder, proof, update
- **Skill Lens**: rule-based intent matching picks relevant skills per message
- **11 skills**: file.read, file.write, project.scan, web.fetch, shell.powershell.safe, shell.bash.safe, shell.zsh.safe, git.status, git.diff, python.write, python.run
- **Provider switching**: OpenAI-compatible endpoints and Cloudflare AI Gateway
- **Coder Mode**: scan, plan, diff, approve, test
- **Proof Mode**: JSON traces and Markdown reports with secret redaction
- **Safety**: workspace-only file access, blocked secret paths, approval-gated writes and commands
- **Registry**: remote HTTPS registry with SHA-256 verification, cache, bundled fallback, trust checks, and controlled skill updates
- **Core updater**: release channel checks, verified binary downloads, staged install, backups, and rollback

## Installation

From source (recommended for now):

```bash
cargo build -p axiom-cli
cargo run -p axiom-cli
```

npm installer scaffold exists but is not published yet:

```bash
# Local testing only
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

Run `axiom` with no arguments. If no config exists, onboarding starts. It asks for your provider, API key environment variable, and model. After setup, it installs the essential skill bundle for your OS.

Config is saved to:
- Windows: `%APPDATA%\axiom-agent\config.toml`
- Linux/macOS: `~/.config/axiom-agent/config.toml`

API keys are never stored in config. Provider entries reference environment variable names.

For scripted setup and tests:

```bash
AXIOM_HOME=/tmp/axiom-test-home \
cargo run -p axiom-cli -- onboarding --non-interactive --provider mock --workspace ./demo-workspace --yes
```

`AXIOM_HOME` changes the config root for the process. When set, Axiom writes `config.toml`, `skills/installed_skills.json`, `proofs/`, `updates/`, and `registry-cache/` under that directory instead of the real user config directory.

The built-in `mock` provider is for tests and demos only. It returns deterministic responses and does not require API keys.

## Chat Mode

`axiom` or `axiom chat` opens terminal chat.

For one-shot non-interactive chat:

```bash
axiom run "hello"
axiom run "read README.md and summarize it"
axiom run "hello" --no-tools --no-proof
```

`axiom run` uses the same Skill Lens, skill context injection, provider call, one tool loop, and Proof Mode recording as normal chat.

Chat commands:

```text
!help         Show available commands
!exit         Leave chat
!model use X  Switch model
!provider use X  Switch provider
!clear        Clear history
!proof on/off Toggle proof recording
!skills       List installed skills
!lens on/off  Toggle Skill Lens
```

When Lens is on, chat shows which skills were selected:

```text
Axiom Lens: selected python.write, file.write
```

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

Skills are TOML manifests with `[llm_card]` sections. Types: `prompt` (LLM guidance), `tool` (executable), `workflow`, `guard`.

Only `tool` skills with built-in entrypoints execute in v0.1. Prompt skills guide the model but do not run code.

Installed skills live in the platform config directory. The registry supports bundles (groups of skills for a platform) and individual installs.

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

The default remote registry is `https://raw.githubusercontent.com/NexaraAI/axiom-skills/main/registry.json`. The registry URL is configurable. If it fails, onboarding falls back to the bundled fixture at `fixtures/skill-registry/`.

Installed skills have lifecycle state and trust metadata. Disabled, incompatible, quarantined, and blocked skills are not selected by Skill Lens and cannot execute. External executable skill binaries are still not supported; unknown external entrypoints are installed disabled or quarantined.

## Coder Mode

`axiom code` opens the coding assistant. It scans the workspace, builds project context, asks the LLM for a plan, shows diffs, and writes files after approval.

```bash
axiom code --scan        # Scan workspace
axiom code --plan-only "task"  # Plan without writing
axiom code --apply "task"      # Plan and apply
axiom code --test        # Run detected test command
```

Coder mode does not commit, push, deploy, delete files, or run arbitrary commands.

## Proof Mode

Proof Mode records terminal agent sessions. Each task gets a JSON trace and a Markdown report.

Recorded data: user request, provider, model, selected skills, tool calls, approvals, file writes, commands, test results, errors, and final response. Secrets are redacted.

```bash
axiom proof list
axiom proof latest
axiom proof show latest
axiom proof export latest --format markdown
axiom proof clean --older-than 30
```

Proofs are stored at:
- Windows: `%APPDATA%\axiom-agent\proofs`
- Linux/macOS: `~/.config/axiom-agent/proofs`

## Safety Model

Axiom enforces workspace-only file access. Secret-looking paths (`.env`, `*.pem`, `*.key`, `credentials.json`) are blocked. Medium and high risk actions require terminal approval. Tool execution stays within built-in executors; external binaries are rejected.

Coder mode shows plans and diffs before writes. Even in trusted approval mode, v0.1 asks before every file write.

Core updates verify `SHA256SUMS` before staging a binary. Missing or mismatched checksums block installation. Axiom does not execute release scripts.

## Core Updates

```bash
axiom update status
axiom update check
axiom update install
axiom update rollback
axiom update set-channel stable
axiom update set-policy notify
```

`stable` uses normal releases, `nightly` can use prereleases, and `dev` is for local mocked release metadata. Running from Cargo `target/` supports update checks but blocks self-replacement.

## Development Setup

```bash
cargo fmt
cargo clippy --all-targets --all-features
cargo test
node scripts/smoke-test.js
node scripts/e2e-test.js
node scripts/release-check.js
node scripts/security-check.js
```

Doctor check:

```bash
cargo run -p axiom-cli -- doctor
```

Offline demo:

```bash
cargo run -p axiom-cli -- onboarding --non-interactive --provider mock --workspace ./demo-workspace --yes
cargo run -p axiom-cli -- run "read README.md and summarize it"
cargo run -p axiom-cli -- code --plan-only "explain how to add a test"
```

See `docs/TESTING.md` and `docs/DEMO.md` for isolated local runs without API keys.

## Roadmap

- v0.5.0-beta: terminal foundation, config, chat, Skill Lens, tool execution, registry, npm scaffold, Coder mode, Proof Mode, offline demos, release assets, and release safety checks
- Next: stronger editing workflows, richer patch application, proof analytics, and broader skill ecosystem polish
- Later: external skill binaries, remote registry publishing, app layers

## License

MIT
