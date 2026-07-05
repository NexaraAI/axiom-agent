# Architecture

Axiom Agent is a Cargo workspace with focused crates. It ships as a skill-powered terminal agent for code tasks, not a chatbot.

- `axiom-cli`: terminal command interface and display.
- `axiom-core`: config, sessions, workspace safety, and shared errors.
- `axiom-llm`: provider traits, provider transports, and the deterministic mock provider for tests and demos.
- `axiom-engine`: skill manifests, remote and local registry clients, registry cache, bundles, installed skill storage, lifecycle state, trust checks, skill updates, health stats, and built-in tool execution.
- `axiom-lens`: prompt intent analysis, skill card selection, and compact prompt context.
- `axiom-coder`: coding project scan, plan prompts, patch parsing, diff previews, safety validation, and test command detection.
- `axiom-proof`: proof trace types, redaction, JSON export, Markdown reports, and proof lookup.
- `axiom-update`: core binary release checks, version comparison, platform asset resolution, checksum verification, staging, backup, rollback, and update state.

## Installation Flow

```text
npm install -g axiom-agent
-> postinstall detects OS and architecture
-> local AXIOM_AGENT_BINARY_PATH copy, or GitHub Release download
-> SHA256SUMS verification
-> bin/axiom.js forwards commands to the Rust binary
-> axiom starts onboarding or chat
```

The npm package is thin on purpose. It contains no agent logic and does not replace the Rust CLI. Developers can override the release repository with `AXIOM_AGENT_RELEASE_REPO`.

## Core Update Flow

```text
axiom update check
-> axiom-cli loads config and prints status
-> axiom-update fetches or parses release metadata
-> release channel filters stable/nightly/dev releases
-> semver comparison classifies patch/minor/major
-> platform resolver picks the expected release asset
-> update-state.json records compact check metadata
-> Axiom Proof records the update check

axiom update install
-> download binary and SHA256SUMS
-> verify checksum
-> stage verified binary under updates/staged
-> back up current binary under updates/backups
-> replace current binary when install mode allows
-> rollback if post-install verification fails
```

`axiom-cli` handles terminal prompts and display. `axiom-update` handles release, checksum, staging, install, and rollback logic. No release scripts get downloaded or executed.

## Skill Flow

```text
User message
-> Axiom Skill Lens
-> installed skills and registry metadata
-> selected compact skill cards
-> LLM context injection
-> optional provider-independent axiom-tool request
-> Axiom Engine built-in tool execution
-> provider response
-> Axiom Proof JSON and Markdown export
```

`axiom-cli` does not parse skill manifests. `axiom-engine` handles skill storage and parsing. `axiom-lens` handles selection. `axiom-llm` handles provider transport.

`axiom run "message"` uses this same flow once and exits. Integration tests, scripted demos, and automation use it as the non-interactive entry point. It can run one provider-requested tool loop unless you pass `--no-tools`, and it records Proof traces unless you pass `--no-proof`.

## Coder Flow

```text
User coding task
-> Axiom Lens route detection
-> axiom-cli starts coder session
-> axiom-coder scans project and builds plan/patch prompts
-> axiom-llm calls configured provider/model
-> axiom-coder parses and validates axiom-patch JSON
-> axiom-cli shows diff and asks confirmation
-> axiom-engine file.write applies approved full-file changes
```

Coder mode keeps normal chat history separate from coding session history. Auto-routing from chat can ask first or switch for obvious project-level coding tasks, but it never grants write permissions. File writes and command execution stay approval-gated.

## Proof Flow

```text
Chat, skill run, or coder task
-> axiom-cli starts ProofRecorder with config-derived settings
-> Lens selection, tool calls, approvals, file writes, commands, patches, tests, and errors are recorded
-> axiom-proof redacts secrets and summarizes large outputs
-> JSON trace and Markdown report are written under the user config proofs directory
-> axiom proof commands list, show, export, and clean reports
```

`axiom-proof` handles trace shape, storage traversal, redaction, and report rendering. `axiom-cli` handles terminal display and command routing. `axiom-engine` handles executable skill behavior. Coder mode records metadata about approved plans, patches, and command results.

## Registry Flow

```text
Configured registry URL
-> HTTPS registry fetch or local fixture load
-> registry cache read or refresh
-> registry.json schema parse
-> bundle selection by OS or command
-> manifest fetch with optional sha256 verification
-> compatibility and trust assessment
-> install skill.toml into user config directory
-> installed_skills.json source tracking
```

Onboarding first tries the configured registry. If that fails and `fallback_to_bundled_registry = true`, it installs the OS essential bundle from `fixtures/skill-registry/`. This preserves offline setup and keeps tests independent of GitHub.

For tests and demos, `AXIOM_HOME` overrides the config root. That path resolution lives in `axiom-core`, so CLI commands, proof recording, skills, registry cache, and updater state all share the same isolated root.

Axiom never executes remote skill code. Registry downloads are limited to manifests and bundles. A skill gets enabled when it is compatible, trusted enough for the install path, and its entrypoint is `prompt-only` or a built-in Axiom executor (`builtin:file.read`, `builtin:file.write`, `builtin:web.fetch`, `builtin:git.status`, or `builtin:git.diff`).

## Skill Lifecycle Flow

```text
installed_skills.json
-> Axiom Engine lifecycle and trust checks
-> Skill Lens selection filter
-> Axiom Engine execution filter
-> runtime success/failure health stats
-> optional proof trace summary
```

`axiom-cli` handles prompts and display. `axiom-engine` handles state transitions, compatibility checks, update application, cache behavior, and execution blocking. `axiom-lens` does not decide trust policy; it receives installed skills and ignores records that `axiom-engine` marks as disabled, incompatible, quarantined, or blocked.

## Mock Provider

The `mock` provider lives in `axiom-llm`. It is labeled for tests and demos only. It returns deterministic chat responses, can request `file.read` for README requests, returns a simple coder plan, emits a harmless `axiom-patch`, and summarizes one tool result. It makes no network calls and requires no API keys.
