# Architecture

Axiom Agent is a Cargo workspace with focused crates. It ships as a skill-powered terminal agent for code tasks, not a chatbot.

- `axiom-cli`: terminal command interface and display.
- `axiom-agent`: bounded Plan, Tool, Observe, Reflect, and Done/GiveUp runtime, context compaction, todo context, and usage accounting.
- `axiom-core`: config, sessions, workspace safety, and shared errors.
- `axiom-llm`: provider traits, provider transports, and the deterministic mock provider for tests and demos.
- `axiom-engine`: skill manifests, remote and local registry clients, registry cache, bundles, installed skill storage, lifecycle state, trust checks, skill updates, health stats, and built-in tool execution.
- `axiom-lens`: prompt intent analysis, skill card selection, and compact prompt context.
- `axiom-coder`: coding project scan, plan prompts, patch parsing, diff previews, safety validation, and test command detection.
- `axiom-proof`: proof trace types, redaction, JSON export, Markdown reports, and proof lookup.
- `axiom-update`: core binary release checks, version comparison, platform asset resolution, checksum verification, staging, backup, rollback, and update state.

## Installation Flow

```text
npm install -g axiom-agent@rc
-> postinstall detects OS and architecture
-> local AXIOM_AGENT_BINARY_PATH copy, or GitHub Release download
-> SHA256SUMS verification
-> bin/axiom.js forwards commands to the Rust binary
-> axiom starts onboarding or chat
```

The npm package is thin on purpose. It contains no agent logic and does not replace the Rust CLI. Developers can override the exact GitHub release repository with `AXIOM_AGENT_RELEASE_REPO`.

## Provider Setup Flow

```text
guided onboarding selects one or two providers
-> hidden credential input stores values in the native OS credential manager
-> config stores only provider endpoints and credential environment names
-> remote endpoints require HTTPS; HTTP is limited to literal loopback hosts
-> catalog-only GET discovers available model IDs when the provider documents one;
   Cloudflare uses explicit model-ID entry
-> the first provider becomes active; each provider retains its model choice
-> each provider resolves its credential directly from the current environment
   or native credential manager into the HTTP client; keyring values are never
   copied into the process environment
-> axiom model/provider commands inspect or switch the saved selection
```

The default OpenAI-compatible catalog URL is `<base_url>/models`. A provider
may configure a separate catalog URL, as GitHub Models does. Catalog discovery
is bounded to 4 MiB and never sends user messages or calls a completion
endpoint. Headless and noninteractive installations continue to use explicit
environment variables instead of requiring a desktop credential service.
Before Axiom launches any test, diagnostic, or Git child process, it removes
every configured provider credential variable from that child's environment.
Git diff execution also disables external diff, text-conversion, and fsmonitor
drivers and excludes secret paths case-insensitively before capture. Provider
HTTP clients do not use ambient system proxies.

## Web Tool Network Boundary

`[network]` applies only to model-invoked `web.fetch`; provider completion and
catalog endpoints use their separately reviewed provider configuration. Remote
provider URLs require HTTPS, URL credentials are rejected, redirects are
disabled, and plain HTTP is limited to literal loopback development hosts. The web
tool validates the URL before any request, requires HTTPS by default, evaluates
exact/`*.domain` denied hosts before an optional allowlist, resolves the target,
and rejects any blocked address. The approved public addresses are pinned for
the request. Redirects are disabled, response bytes are bounded while
streaming, and reqwest system proxy discovery is disabled unless the operator
opts in.

Localhost, `.local`, loopback, private/reserved addresses, and hostnames that
resolve to them are hard failures. Host allowlists, HTTP opt-in, and proxy
opt-in do not bypass that boundary.

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
-> axiom-agent bounded multi-step loop
-> optional native function call or provider-independent axiom-tool fallback
-> centralized allow/ask/deny policy and Axiom Engine built-in tool execution
-> observation and reflection until Done or a configured cap
-> atomic session persistence after each internal transition
-> Axiom Proof JSON and Markdown export
```

`axiom-cli` does not parse skill manifests. `axiom-engine` handles skill storage and parsing. `axiom-lens` handles selection. `axiom-llm` handles provider transport.

`axiom run "message"` uses this same bounded loop for one user turn and exits. Integration tests, scripted demos, and automation use it as the non-interactive entry point. It can execute multiple provider-requested tool rounds unless you pass `--no-tools`, and it records Proof traces unless you pass `--no-proof`.

Before each model call, `axiom-agent` estimates the serialized context with the modern `o200k` tokenizer and compacts only older conversational messages. Identity, selected-skill context, current todo state, and recent messages stay verbatim. Provider-reported token usage is accumulated separately and remains authoritative; configured input/output token rates turn that ledger into an estimated cost cap.

OpenAI-compatible providers can use SSE. `axiom-llm` incrementally parses
events and accumulates fragmented content, usage, and native tool-call
arguments. A control-block projector withholds fragmented `axiom-tool` and
`axiom-todo` blocks from live terminal output while preserving the exact full
response for parsing. Safe assistant text renders incrementally; the completed
response is not printed a second time. Non-stream bodies, error summaries,
individual SSE events, aggregate response content, and tool-call fields are all
bounded before allocation can grow without limit.

Chat state is stored as versioned JSON under the config `sessions/` directory
using atomic replacement. The transition barrier persists compacted history,
todo state, usage, tool events, approvals, policy decisions, and the latest
workspace checkpoint reference before later side effects proceed. Before an
agent `file.write`, the CLI creates the workspace checkpoint and persists its
reference. `axiom sessions` lists saved sessions, `axiom resume <id>` restores
them, and `!checkpoints`/`!restore ID` expose recovery snapshots. Proof and
session IDs are aligned for new sessions.

## Coder Flow

```text
User coding task
-> Axiom Lens route detection
-> axiom-cli starts coder session
-> axiom-coder scans project and builds plan/patch prompts
-> axiom-agent performs capped, streaming provider/model calls with tools disabled
-> axiom-coder parses and validates axiom-patch JSON
-> plan-to-path scope, existing-file base SHA-256, and minimal hunks are verified
-> axiom-cli enforces hard scope caps and asks for per-hunk/diff/scope confirmation
-> axiom-coder snapshots affected paths to a recovery checkpoint
-> atomic writes apply the approved patch
-> an allowed detected test command runs
-> bounded failures can produce re-approved correction patches
```

Coder mode keeps normal chat history separate from coding session history. Auto-routing from chat can ask first or switch for obvious project-level coding tasks, but it never grants write permissions. File writes and command execution stay approval-gated. An external edit can be relocated only when the old hunk context is unique; ambiguous or overlapping changes fail as conflicts. Partial write failure triggers automatic checkpoint restoration, while checkpoints remain available for explicit `!restore` recovery.

Coder retains its purpose-built scan/plan/patch/test orchestration, but all of
its LLM calls use the canonical `axiom-agent` runtime with tools disabled. This
keeps provider retries, streaming, caps, and usage accounting consistent while
leaving patch application under the stricter Coder approval path.

Every Coder plan, patch, correction, and conversational model call checks the
shared persistent session/monthly budget before invocation, caps the canonical
runtime to the remaining priced amount, and records provider-reported usage in
the same cross-process cost ledger used by Chat and `axiom cost`.

## Proof Flow

```text
Chat, skill run, or coder task
-> axiom-cli starts ProofRecorder with config-derived settings
-> Lens selection, transitions, token/cost metrics, compaction, policy decisions, tool calls, approvals, checkpoints, file writes, commands, patches, tests, and errors are recorded
-> axiom-proof recursively redacts every persisted field and summarizes large outputs
-> valid dated proof directories older than the configured retention are pruned
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

Onboarding first tries the configured registry. If that fails and
`fallback_to_bundled_registry = true`, it installs the OS essential bundle from
an immutable registry embedded in the executable and materialized under
`$AXIOM_HOME/bundled-registry/<generation>`. Packaged binaries therefore retain offline
setup without depending on a source checkout; repository fixtures are only the
compile-time asset source and test oracle.

For tests and demos, `AXIOM_HOME` overrides the config root. That path resolution lives in `axiom-core`, so CLI commands, proof recording, skills, registry cache, and updater state all share the same isolated root.

Axiom never executes remote skill code. Registry downloads are limited to manifests and bundles. A skill gets enabled when it is compatible, trusted enough for the install path, and its entrypoint is `prompt-only` or a compiled-in Axiom executor. Built-ins are registered through a typed executor registry; runtime dependency checks still block missing, disabled, incompatible, or cyclic dependencies.

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
