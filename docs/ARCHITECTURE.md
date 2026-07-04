# Architecture

Axiom Agent is a Cargo workspace with focused crates. The product is a skill-powered terminal agent, not only a chatbot.

- `axiom-cli`: terminal command interface and display.
- `axiom-core`: config, sessions, workspace safety, and shared errors.
- `axiom-llm`: provider traits and provider skeletons.
- `axiom-engine`: skill manifests, remote and local registry clients, bundles, installed skill storage, and built-in tool execution.
- `axiom-lens`: prompt intent analysis, skill card selection, and compact prompt context.
- `axiom-coder`: coding project scan, plan prompts, patch parsing, diff previews, safety validation, and test command detection.
- `axiom-proof`: proof trace types, redaction, JSON export, Markdown reports, and proof lookup.
- `axiom-update`: future safe update checks.

## Installation Flow

```text
npm install -g axiom-agent
-> postinstall detects OS and architecture
-> local AXIOM_AGENT_BINARY_PATH copy, or GitHub Release download
-> SHA256SUMS verification
-> bin/axiom.js forwards commands to the Rust binary
-> axiom starts onboarding or chat
```

The npm package is intentionally thin. It does not implement agent logic and does not replace the Rust CLI. The release repository URL in package metadata is a placeholder until the final GitHub location is chosen, and development can override it with `AXIOM_AGENT_RELEASE_REPO`.

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

The CLI does not parse skill manifests directly. Engine owns skill storage and parsing. Lens owns selection. LLM owns provider transport.

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

Coder mode keeps normal chat history separate from coding session history. Auto-routing from chat can ask first or switch automatically for obvious project-level coding tasks, but it never grants write permissions. File writes and command execution remain approval-gated.

## Proof Flow

```text
Chat, skill run, or coder task
-> axiom-cli starts ProofRecorder with config-derived settings
-> Lens selection, tool calls, approvals, file writes, commands, patches, tests, and errors are recorded
-> axiom-proof redacts secrets and summarizes large outputs
-> JSON trace and Markdown report are written under the user config proofs directory
-> axiom proof commands list, show, export, and clean reports
```

`axiom-proof` owns trace shape, storage traversal, redaction, and report rendering. The CLI owns terminal display and command routing. Axiom Engine still owns executable skill behavior, while coder mode records metadata about approved plans, patches, and command results.

## Registry Flow

```text
Configured registry URL
-> HTTPS registry fetch or local fixture load
-> registry.json schema parse
-> bundle selection by OS or command
-> manifest fetch with optional sha256 verification
-> install skill.toml into user config directory
-> installed_skills.json source tracking
```

Onboarding first attempts the configured registry. If it fails and `fallback_to_bundled_registry = true`, it installs the OS essential bundle from `fixtures/skill-registry/`. This preserves offline setup and keeps tests independent of GitHub.

Stage 7 never executes remote code. Registry downloads are limited to manifests and bundles. A skill is enabled only when its entrypoint is `prompt-only` or a built-in Axiom executor such as `builtin:file_read`, `builtin:file_write`, `builtin:web_fetch`, `builtin:git_status`, or `builtin:git_diff`.
