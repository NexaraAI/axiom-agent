# Axiom v1.0 Release Plan

## Product boundary

v1.0 is a stable, terminal-native coding agent. A user must be able to install
it, understand its capabilities and limits, inspect and approve side effects,
complete a bounded multi-step coding task, resume an interrupted task, and
verify where the released binary and npm package came from.

v1.0 is not a promise to run arbitrary registry code. Community manifests may
be discovered and used as prompt context, but executable extensions remain
disabled until their sandbox, permission model, signing, revocation, and
independent security review are complete. Likewise, unrestricted autonomous
sub-agents and a full-screen TUI are post-v1 enhancements, not release
criteria. This keeps the first stable release trustworthy rather than merely
feature-rich.

The existing `PHASE1_PLAN.md` is preserved implementation history. This
document is the authoritative product, safety, quality, and release plan for
turning the implemented candidate into v1.

## Release status

The v1 implementation has reached `1.0.0-rc.1`. The candidate is published on
the prerelease channel for final platform and field validation; it is not the
stable v1 promotion. [V1_RC_CHECKLIST.md](V1_RC_CHECKLIST.md) is the live
go/no-go record for automated, external, and operator gates.

## Current assessment

| Area | Current state | v1 gap |
| --- | --- | --- |
| Provider setup | Guided onboarding configures one or two providers, accepts hidden credentials through the OS credential manager, performs catalog-only discovery, preserves one model per provider, supports custom OpenAI-compatible endpoints, and has deterministic credential/catalog failure fixtures. | Real native credential-store and hosted-provider RC smoke tests on every supported OS; final offline/error-state review. |
| Agent runtime | Chat, `axiom run`, and Coder LLM calls use the bounded agent loop with native/fallback tools, retry policy, cancellation, todos, safe live SSE projection, token-aware compaction, caps, and transition-level durable checkpoints. | Cross-platform interruption/resume and long-session acceptance tests; final compatibility sign-off. |
| Skills | Built-ins use a typed executor registry with declared schemas, permissions, side effects, and fixtures; manifests/indexes are versioned and validated; dependencies, lifecycle filters, and card budgets are enforced. | Complete hostile registry/executor corpus review and final health diagnostics acceptance. |
| Coding | Plans and patches run through the canonical capped LLM path; plan-to-patch scope is checked; base-hashed minimal hunks are conflict-aware and approved individually; writes checkpoint first; recursive monorepo detection chooses argument-vector test commands with a validated in-workspace working directory; detected tests can drive bounded re-approved corrections. | Real-project RC acceptance across the supported project types. |
| Safety | A central allow/ask/deny evaluator covers built-in tools plus every Coder scan, process, Git, and write path and records decisions; detected Coder tests retain a separate allowlist/approval gate; state is atomic; the persistent cost ledger serializes cross-process writes; Unix private state uses restrictive modes; `web.fetch` defaults to HTTPS/no proxy, applies deny-first exact/wildcard host policy, hard-blocks private targets, disables redirects, pins public DNS results, and caps streamed bytes; remote provider endpoints require HTTPS and do not redirect; deterministic malformed-input corpora and dependency policy run in tests. | Verify Windows/macOS permissions and network controls, run long fuzz lanes, review the version-bound threat model, and obtain security-owner sign-off. |
| UX | Inline chat has live safe output, persistent line history, bracketed paste, exact multiline capture, durable `!show` output, recovery commands, blood-red/ash/high-contrast/plain themes, redirected-output plain mode, session IDs, and resume. | Manual screen-reader, narrow-terminal, shell compatibility, and redirected-I/O acceptance on the release matrix. |
| Release | Cross-platform native build and exact-binary smoke jobs, checksums, pinned GitHub Actions, SPDX SBOM generation, GitHub artifact attestations, npm OIDC publishing, semantic-version/dist-tag guards, governance docs, and release checks exist. | External trusted-publisher registration, reproducibility evidence, tagged artifact verification, and RC testing. |
| Documentation | The v1 product contract, support targets, runtime, Coder, skills, safety, config/session compatibility, release provenance, governance, testing, and RC checklist describe the candidate behavior and distinguish it from the published beta. | Operator results/sign-off, final RC notes, and synchronized version cut. |

## Release principles

1. **Safe by default.** Every write, command, network request, and update has
   an explicit policy, visible scope, and proof record.
2. **Bounded autonomy.** Iterations, tool calls, elapsed time, token use, and
   spend are enforced caps—not advisory settings.
3. **Honest capability model.** The CLI never implies a skill executes when it
   only contributes context.
4. **Recoverability.** Agent and coder tasks checkpoint before side effects;
   interruption, provider failure, and patch conflicts have a clear recovery
   path.
5. **Verifiable distribution.** A v1 binary and npm package are traceable to
   the tagged source and release workflow.
6. **Compatibility before convenience.** Existing beta configuration and
   proofs migrate safely or fail with a precise, documented remediation.

## Milestone 0 — Freeze the v1 contract

### Deliverables

- Replace the beta PRD and fragmented roadmap language with a concise v1
  product contract: supported operating systems, providers, tool types,
  approval modes, network behavior, data storage, and non-goals.
- Define a semver and compatibility policy for config TOML, proof JSON, skill
  manifests, registry index schema, CLI command output, and npm wrapper.
- Add versioned config and proof migrations with backups and a `axiom doctor`
  compatibility report. Unknown future versions must fail closed rather than
  silently dropping fields.
- Add `CHANGELOG.md`, `SECURITY.md`, `CONTRIBUTING.md`, a support policy, and
  a release owner/checklist. Add an MSRV policy and pin it in
  `rust-toolchain.toml`.
- Update every user document to describe the actual multi-step runtime and
  executable-skill boundary. The old architecture description of a one-tool
  loop must not survive the v1 cut.

### Acceptance

- An upgrade from the latest v0.5 beta preserves a valid config, installed
  skill lifecycle state, and proof history, or reports exactly what needs
  manual intervention.
- `axiom doctor --json` reports versions, migrations, executable skills,
  update provenance, sandbox availability, and failed mandatory checks.
- All public commands have stable help text and an explicit exit-code contract.

## Milestone 0.5 — Make provider setup self-explanatory

### Deliverables

- Start guided onboarding automatically on a fresh install and show the exact
  settings needed for each provider in plain language.
- Allow one or two providers to be configured in one pass. The first is active;
  every provider retains its own last selected model.
- Accept credentials through hidden terminal input, store them in the native OS
  credential manager, and retain environment variables as the headless and CI
  fallback. Never write credential values to TOML, Proof Mode, logs, or shell
  history.
- Fetch model catalogs with provider `GET` endpoints only. Catalog discovery
  must not issue a chat, completion, embedding, image, or other billable
  inference request.
- Show a bounded model list with search/filter selection. If discovery is
  unavailable, explain why and permit an explicit model ID.
- Support custom OpenAI-compatible chat and model-catalog URLs with optional
  authentication.
- Ship `axiom model current|list|use`, `axiom provider current|list|use`, and
  matching inline chat commands.

### Acceptance

- A fresh user can configure OpenRouter plus Ollama without manually editing
  TOML, and switching providers restores the correct model for each.
- Credential paste is hidden; a repository/config/proof scan contains no pasted
  value; subsequent launches load the native credential automatically.
- A contract server verifies that model discovery performs only `GET /models`
  (or the configured catalog URL) and never calls a completion endpoint.
- Keyring-unavailable, missing-key, offline-local-server, empty-catalog, 401,
  429, malformed-catalog, and oversized-catalog states all produce actionable
  fallback output without destroying existing settings.

## Milestone 1 — Complete the agent runtime

### Deliverables

- Finish `axiom-agent` as the sole Plan → Tool → Observe → Reflect → Done /
  GiveUp state machine for chat, `axiom run`, and Coder. Remove duplicated
  loop logic only after compatibility tests prove parity.
- Make `TodoList` model-updatable through a structured response envelope;
  render pending, active, completed, and blocked items in every iteration.
- Enforce all configured caps: LLM iterations, tool calls, wall time,
  consecutive errors, context tokens, and estimated USD cost. Persist the
  per-turn and per-session ledger in Proof Mode.
- Add cancellation at safe boundaries, a clear GiveUp reason, and idempotent
  cleanup. A cancellation must never leave an unrecorded partially applied
  patch.
- Implement OpenAI-compatible SSE streaming with accumulation for tool-call
  parsing. Cloudflare streaming follows only after its framing is covered by
  contract tests.
- Add native OpenAI-style `tools`/`tool_choice` request support and parse
  structured tool calls. Keep the fenced `axiom-tool` form only as a tested
  provider fallback.
- Compact context above a measured token threshold. Preserve identity, current
  todo state, recent turns, approvals, and tool results; summarize only older
  history. Enforce selected-skill card budgets in the same component.

### Acceptance

- A fixture task that needs three tool rounds completes from one prompt, emits
  three proof tool events, and never exceeds its configured caps.
- `max_iterations = 1` and `loop_enabled = false` have documented, tested
  behavior.
- A simulated 429/5xx, stream disconnect, tool error, cancellation, token cap,
  and wall-time cap produce an actionable terminal result and complete proof.
- A long fixture session stays under its context ceiling and retains its task,
  approvals, and latest tool observations.

## Milestone 2 — Make Coder trustworthy

### Deliverables

- Replace complete-file patch payloads with validated unified hunks and a
  three-way apply strategy. Detect changed bases and conflicts before writes.
- Make every coder iteration create a recoverable checkpoint before applying a
  patch. Use an explicit workspace snapshot/checkpoint abstraction rather than
  assuming a clean Git checkout.
- Feed failed test stdout/stderr into the shared agent loop. Limit correction
  attempts, retain user approval for changed scope, and report a useful partial
  result if convergence fails.
- Parse project manifests for test commands: Cargo workspaces, package scripts,
  Python, Go, Maven/Gradle, Deno/Bun, and common monorepo layouts. Use a
  policy-driven command allowlist rather than a fixed literal list.
- Require a plan-to-patch verification pass that confirms each changed hunk
  maps to the approved task. Require a second confirmation if the patch grows
  beyond the approved file/scope budget.
- Add an interactive non-TUI diff review; a full-screen hunk viewer can follow
  once the underlying hunk model is stable.

### Acceptance

- A deliberately failing first patch self-corrects or returns
  `MaxCorrectionsReached` with the exact test output and a recoverable
  checkpoint.
- An externally edited file produces a conflict, never a silent overwrite.
- A package with `"test": "vitest run"` and a Cargo workspace both select
  and execute the correct allowed test command.
- No Coder path can delete, commit, push, deploy, or execute an arbitrary shell
  command without a separately approved and documented capability.

## Milestone 3 — Ship a real, bounded skill platform

### Deliverables

- Replace the executor's tool-ID match with a typed executor registry. Each
  built-in executor exposes its ID, JSON schema, permissions, side effects,
  and deterministic test fixtures.
- Extend manifests with `keywords`, examples, dependencies, provides,
  idempotence, cache keys, declared side effects, and schema version. Validate
  them on install and on registry refresh with precise diagnostics.
- Replace hardcoded Lens keywords with manifest-driven ranking, risk filtering,
  dependencies, explicit card budgets, and an identity/smalltalk bypass.
- Add executor health diagnostics that distinguish disabled, incompatible,
  untrusted, prompt-only, unsupported, and runnable skills.
- Version the registry protocol and add a fixture corpus for invalid manifests,
  downgrade attacks, checksum mismatches, malicious URLs, dependency cycles,
  and lifecycle transitions.

### Acceptance

- Adding a new compiled-in executor does not require editing a central match.
- A manifest dependency is co-selected, a disabled/blocked skill is never
  exposed as runnable, and a schema error identifies the manifest field.
- Lens selection is deterministic under a fixture registry and never selects a
  skill for identity-only prompts.

### Explicit v1 boundary

External binaries and WASM modules do **not** execute in v1 unless all of the
following are delivered and independently reviewed: capability sandboxing,
filesystem/network/process isolation, resource limits, declared side-effect
verification, signature/trust policy, revocation, and hostile-module tests.
Otherwise they remain quarantined, which is safer and more honest.

## Milestone 4 — Security and data-integrity hardening

### Deliverables

- Centralize policy evaluation for filesystem, process, network, Git, updates,
  and future executors. Make every decision traceable in Proof Mode.
- Write configuration, lifecycle state, registry cache, proofs, checkpoints,
  and updates atomically with restrictive permissions where the platform
  supports them. Recover safely from a power loss between temp write and rename.
- Treat fetched web pages, registry descriptions, tool output, diffs, and
  project files as untrusted data. Keep them out of system instructions and
  label them clearly in prompts and UI.
- Tighten network controls: HTTPS by default, URL allow/deny policy, redirect
  limits, private-address/loopback policy, response limits, and proxy behavior.
- Add a dependency-security lane: `cargo audit` or `cargo deny`, lockfile
  enforcement, license policy, advisory review procedure, and Dependabot or
  Renovate for Rust, npm, and GitHub Actions dependencies.
- Add property and fuzz tests for tool requests, patch parsing, manifest/registry
  parsing, config migration, path traversal, and proof redaction.
- Publish a vulnerability reporting and disclosure process in `SECURITY.md`.

### Acceptance

- Fault-injection tests never leave corrupt state or a partially replaced
  executable.
- Corpus/fuzz tests find no panic or path escape for malformed external data.
- CI fails on an unreviewed high/critical advisory, a forbidden license, or a
  changed lockfile without its policy metadata.
- The release threat model documents trust boundaries and mitigations for every
  side-effecting subsystem.

## Milestone 5 — Complete the terminal experience

### Deliverables

- Finish the inline renderer: status line with provider/model, iteration,
  todo summary, tokens, estimated cost, tool state, and structured errors.
- Complete `NO_COLOR`, `[ui].color`, and accessibility tests. Keep text content
  identical in plain mode; do not rely on color alone for risk or error meaning.
- Add multiline input, bracketed-paste handling, input history, and a clear
  noninteractive equivalent for every interactive command.
- Add `!show`/saved output references for large tool output, concise previews,
  and proof links rather than flooding the terminal.
- Add the command palette and full-screen TUI only after inline mode reaches
  feature parity. The TUI is a release candidate enhancement, not a dependency
  for v1.0 GA.
- Add themes only as palette swaps over the same semantic renderer; preserve
  blood-red as default and `none` as the accessible plain option.

### Acceptance

- A narrow terminal, `NO_COLOR=1`, redirected output, and screen-reader/plain
  mode retain complete and understandable content.
- A multiline 30-line task is submitted as one turn and recorded intact.
- Long tool output is bounded in the viewport but fully available in Proof Mode.

## Milestone 6 — Proof, checkpoints, and operational controls

### Deliverables

- Persist agent checkpoints after every state transition: session ID, identity
  version, compacted history, todo list, tool events, approvals, ledger, and
  workspace checkpoint reference.
- Implement `axiom resume <id>`, `axiom sessions`, cancellation cleanup,
  proof/checkpoint retention, and export/import policy.
- Extend proof records with agent iteration, cap decisions, stream/provider
  failures, retry attempts, migration version, and update provenance.
- Add configurable per-session and monthly budgets, with forecasts and hard
  stop behavior before the next provider call.
- Add privacy controls: local storage location, retention defaults, redaction
  rules, explicit export warnings, and an opt-out of proof capture.

### Acceptance

- Interrupting after a tool result then resuming continues from the checkpoint
  without rerunning a non-idempotent tool.
- Proof exports are schema-versioned, redact fixture secrets, and explain why a
  cap or policy denied an action.

## Milestone 7 — Release engineering and supply-chain trust

### Deliverables

- Pin every GitHub Action to reviewed full commit SHAs and set minimal explicit
  permissions in each workflow. Enable repository policy requiring SHA-pinned
  actions and restrict permitted action sources.
- Build release binaries from a clean tag after the full test matrix; add
  platform smoke tests for the npm-installed wrapper on Windows, Linux, and
  both macOS architectures.
- Generate and verify SHA256 checksums, SBOMs, and GitHub binary attestations.
  Publish verification instructions with the release.
- Move npm publishing to npm trusted publishing/OIDC where the repository and
  package ownership permit it; publish with provenance and remove long-lived
  publish-token dependence after migration.
- Add a release workflow gate for version sync, clean worktree, changelog,
  migration tests, advisory scan, package contents, install/upgrade/rollback,
  and `gh attestation verify`.
- Add release channels (`stable`, `rc`, `nightly`) with prerelease semantics,
  signed/attested artifacts, rollback criteria, and a staged rollout plan.

### Acceptance

- Users can verify a released binary against checksums and workflow provenance.
- The npm package carries provenance through trusted publishing or
  `npm publish --provenance` and its repository metadata exactly matches the
  publishing repository.
- No release job has broader token permissions than needed; no action is tag
  pinned; a failed artifact or publish verification blocks the release.

## Milestone 8 — Quality gates and release candidates

### Test matrix

- Unit, integration, and end-to-end tests on supported OS/architecture pairs.
- Mock-provider deterministic tests plus recorded provider-contract tests that
  validate OpenAI-compatible responses, SSE chunks, tool calls, retries, and
  malformed payloads without production keys.
- Golden tests for CLI plain and colored output, config/proof migrations, patch
  conflicts, tool approval, and error rendering.
- Failure injection for network timeouts, rate limiting, interrupted writes,
  cancellation, corrupted cache, downgrade attempts, and full disks where
  practical.
- Property/fuzz corpus for external JSON/TOML/text input.
- Manual accessibility and real-project beta tests across Rust, Node, Python,
  Go, and at least one monorepo.

### RC sequence

1. Freeze features and cut `v1.0.0-rc.1` from a clean release branch.
2. Run the full matrix, security scan, upgrade/rollback tests, and release
   provenance dry run.
3. Publish an opt-in RC channel, collect proof-redacted telemetry only from
   explicit testers, triage every regression, and publish release notes.
4. Cut additional RCs only for fixes; no new features after RC.1.
5. Promote the same audited commit to `v1.0.0` after all exit criteria pass.

## v1.0 exit criteria

- All Milestones 0–4 and 6–8 are complete; Milestone 5 inline UX is complete.
- No known critical/high vulnerability without documented and accepted
  mitigation; no unresolved data-loss, workspace-escape, secret-leak, update,
  or arbitrary-execution defect.
- The full automated matrix is green from a clean checkout and the generated
  release artifacts pass checksum/provenance verification.
- Upgrade, fresh install, npm install, binary update, rollback, offline mock
  onboarding, guided multi-provider onboarding, credential reload, catalog-only
  model discovery, command-line model switching, no-color operation, and resume
  all pass documented acceptance tests.
- Documentation, help text, changelog, security policy, and release notes all
  describe the shipped behavior—not planned behavior.
- The release owner explicitly signs off on the threat model and RC results.

## Recommended execution order

1. Milestone 0, Milestone 0.5, and the remaining Phase 0 documentation/config compatibility.
2. Milestone 1 runtime completion, then Milestone 2 Coder safety.
3. Milestone 3 skill platform and Milestone 4 hardening in parallel only after
   the executor interface is stable.
4. Milestone 6 checkpoint/resume after the shared loop is canonical.
5. Milestone 5 UX over stable runtime events.
6. Milestone 7 supply-chain work starts immediately and becomes mandatory at
   RC; Milestone 8 grows with every milestone rather than being deferred.

## Research sources

- GitHub recommends minimal workflow permissions and full-SHA action pinning:
  <https://docs.github.com/en/code-security/tutorials/secure-your-organization/protect-against-threats>
- GitHub artifact attestations can provide provenance and SBOM metadata for
  released binaries:
  <https://docs.github.com/en/enterprise-cloud@latest/actions/concepts/security/artifact-attestations>
- npm trusted publishing uses OIDC and automatically generates provenance for
  eligible public packages:
  <https://docs.npmjs.com/trusted-publishers/>
