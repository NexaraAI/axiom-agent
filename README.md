# Axiom Agent

A terminal-first Rust CLI that routes requests through Skill Lens, executes safe modular skills, handles project coding tasks, and records proof reports.

## What is Axiom Agent?

Axiom is not a chatbot. It is a terminal agent that understands what you need and picks the right tools.

When you send a message, Skill Lens scans your installed skills and selects only the ones that match your request. Those skill cards get injected as compact context for the LLM. If the model asks to run a tool, Axiom Engine executes it with safety checks and sends the result back.

Coder Mode handles project-level tasks: scan the workspace, generate a plan, show diffs, apply changes after approval, run tests.

Proof Mode records what happened during each session: what was asked, which skills were selected, what tools ran, what files changed, and how it ended.

## Current Status

v0.1.0. Terminal foundation is built.

What works:
- Terminal CLI with onboarding, chat, coding mode, and proof commands
- LLM chat through OpenAI-compatible providers and Cloudflare AI Gateway
- Skill Lens intent matching and skill card injection
- 11 built-in skills with TOML manifests
- Built-in tool execution (file.read, file.write, project.scan, web.fetch, git.status, git.diff)
- Skill registry with remote fetch, local fixture fallback, bundles, and install tracking
- Coder mode with project scan, plan, patch, diff preview, safe writes, and test detection
- Proof Mode with JSON traces, Markdown reports, and secret redaction
- npm installer scaffold (not publicly released yet)

What is not done yet:
- npm package is not published. Install from source for now.
- GitHub Release URLs are placeholders until the release workflow runs against this repo.
- Streaming responses are not implemented.
- External executable skill binaries are not supported.
- Auto-updater is a placeholder crate.
- No desktop, mobile, or web interfaces.

## Features

- **8 Rust crates** in a Cargo workspace: cli, core, llm, engine, lens, coder, proof, update
- **Skill Lens**: rule-based intent matching picks relevant skills per message
- **11 skills**: file.read, file.write, project.scan, web.fetch, shell.powershell.safe, shell.bash.safe, shell.zsh.safe, git.status, git.diff, python.write, python.run
- **Provider switching**: OpenAI-compatible endpoints and Cloudflare AI Gateway
- **Coder Mode**: scan, plan, diff, approve, test
- **Proof Mode**: JSON traces and Markdown reports with secret redaction
- **Safety**: workspace-only file access, blocked secret paths, approval-gated writes and commands
- **Registry**: remote HTTPS registry with SHA-256 verification and bundled fixture fallback

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

## Chat Mode

`axiom` or `axiom chat` opens terminal chat.

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
```

The remote registry URL is configurable. If it fails, onboarding falls back to the bundled fixture at `fixtures/skill-registry/`.

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

## Development Setup

```bash
cargo fmt
cargo clippy --all-targets --all-features
cargo test
node scripts/smoke-test.js
npm run check-version-sync
```

Doctor check:

```bash
cargo run -p axiom-cli -- doctor
```

## Roadmap

- v0.1: terminal foundation, config, chat, Skill Lens, tool execution, registry, npm scaffold, Coder v0.1, Proof v0.1
- v0.2: stronger editing workflows, richer patch application
- v0.3: proof analytics, multi-step workflows
- v0.4: safe update checks, skill update installation
- Later: external skill binaries, remote registry publishing, app layers

## License

MIT
