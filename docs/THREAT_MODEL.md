# Axiom Agent Repository Threat Model

## Overview

Axiom Agent is a terminal-native coding agent distributed as a Rust binary and
an npm wrapper. Its primary runtime accepts operator prompts, sends selected
context to a configured language-model provider, parses model-authored control
messages, executes a fixed set of built-in tools, edits a selected workspace,
runs bounded project tests, persists resumable sessions, and emits Proof Mode
records. The repository also contains skill-registry clients, a self-updater,
an npm post-install binary downloader, and GitHub Actions release automation.

The assets that matter most are:

- files inside and outside the selected workspace, especially source, secrets,
  credentials, and executable configuration;
- provider API credentials held in the OS credential manager or process
  environment;
- operator intent, approvals, configured cost limits, and the integrity of the
  Plan -> Tool -> Observe -> Reflect state machine;
- session, checkpoint, cost-ledger, updater, and proof data under the Axiom
  configuration directory;
- installed Axiom binaries, npm wrapper files, release checksums, SBOMs, and
  provenance records; and
- the distinction between prompt-only/quarantined skill metadata and the six
  compiled-in executors that v1 can actually run.

Primary runtime code lives in `crates/axiom-cli`, `crates/axiom-agent`,
`crates/axiom-engine`, `crates/axiom-coder`, `crates/axiom-llm`,
`crates/axiom-core`, `crates/axiom-proof`, and `crates/axiom-update`. The Node
installer and wrapper live under `bin/` and `scripts/`. Fixtures, tests,
screenshots, and documentation are not deployed authority by themselves, but
release scripts and workflows are security-sensitive because they determine
what users install.

## Threat Model, Trust Boundaries, and Assumptions

### Actors and input ownership

- **Operator-controlled:** CLI arguments, prompts, provider/model selection,
  configuration, approval answers, workspace selection, registry overrides,
  update channel, environment variables, and explicit development overrides.
- **Attacker-controlled or untrusted:** model responses and SSE frames, native
  and fenced tool-call arguments, project/repository file contents, Git diffs,
  tool output, fetched web content, DNS answers, registry/index/manifests,
  provider catalog payloads, release metadata, HTTP response sizes, and any
  repository content opened from an untrusted project.
- **Developer/release-owner-controlled:** compiled executor code, default
  manifests, dependency lockfiles, GitHub workflow definitions, npm metadata,
  tagged source, release checksums, and trusted-publisher configuration.

### Trust boundaries

1. **Terminal/operator -> CLI.** The CLI may act with all permissions of the
   current OS account. Approval prompts are an authorization boundary, not a
   sandbox. Noninteractive commands must preserve equivalent explicit flags
   and fail closed when required input is absent.
2. **CLI -> remote model provider.** Prompts, selected skill cards, and relevant
   workspace context cross a privacy boundary. Provider output is never trusted
   as authority merely because it has an assistant role.
3. **Model/control data -> agent engine.** `axiom-tool`, native tool calls, todo
   updates, and patch payloads cross from untrusted text into typed control
   structures. JSON schemas, installed/lifecycle state, caps, policy, and
   transition checkpoints must all succeed before a side effect.
4. **Untrusted workspace -> Coder.** Project files may contain prompt injection,
   malformed manifests, symlinks, huge inputs, or concurrent edits. Paths must
   stay within the canonical workspace, file bases must still match, and
   project text must remain labeled data rather than system instructions.
5. **Tool/process -> host OS.** File writes, Git, tests, and network access are
   externally observable. The central allow/ask/deny policy and proof audit are
   the authorization boundary; `Command::new` argument vectors and command
   allowlists avoid an implicit shell. All configured provider credential names
   are removed from child environments, and Git diff disables external diff and
   textconv/fsmonitor drivers so repository configuration cannot turn inspection
   into a credential-bearing subprocess. Git secret pathspecs mirror the central
   secret-file policy, and both output streams are drained with bounded retention.
6. **`web.fetch` -> network.** URLs and DNS are attacker-controlled. HTTPS,
   host allow/deny rules, private/reserved-address rejection, DNS pinning,
   disabled redirects, response caps, and the no-proxy default constrain SSRF.
   Provider endpoints are a separate operator-controlled boundary so local
   Ollama or LM Studio configuration does not relax the web tool. Remote
   providers require HTTPS, plain HTTP is limited to literal loopback hosts,
   embedded credentials are rejected, redirects and ambient proxy discovery
   are disabled, and response/event/aggregate parsing is byte-bounded.
7. **Registry -> installed skill state.** Registry text can influence Lens and
   install metadata. Schema/checksum/trust/lifecycle checks apply, and external
   executable entrypoints remain quarantined in v1. Only registered compiled-in
   executor IDs are advertised to a model as runnable.
8. **Local persistence -> later executions.** Config, sessions, identity,
   checkpoints, proof, cost, registry, and update state survive process restarts.
   They require version checks, redaction, atomic replacement, and restrictive
   permissions where supported. A corrupted or future schema must not silently
   downgrade behavior.
9. **Release/update infrastructure -> installed binary.** GitHub metadata,
   release assets, `SHA256SUMS`, npm post-install downloads, and OIDC publishing
   cross the software-supply-chain boundary. Version synchronization, bounded
   HTTPS downloads, checksum verification, atomic staging/rollback, SHA-pinned
   Actions, SBOMs, and attestations protect this boundary. Checksums fetched
   from the same compromised release are not a substitute for verified
   provenance and protected release ownership.

### Security invariants

- No model, project file, web page, tool output, registry description, or diff
  becomes a trusted system instruction.
- No path accepted by an executor, patch, checkpoint restore, test working
  directory, updater asset, or saved-output lookup escapes its intended root.
- Every externally observable built-in side effect receives one final policy
  outcome before execution and can be tied to an approval/proof event.
- Denied, cancelled, capped, incompatible, disabled, untrusted, or unsupported
  work fails closed and cannot be represented as successfully executed.
- File edits are conflict-aware and recoverable; a failed multi-file write or
  test cycle retains a pre-write checkpoint rather than silently losing data.
- Provider credential values are never intentionally written to config,
  terminal history, durable sessions/saved tool output, proof, policy targets,
  or model-catalog logs. These persistence paths apply mandatory recursive
  exact-value and token-shape redaction before writing. Because arbitrary
  secrets cannot be recognized with certainty, Proof remains sensitive project
  data and exports carry an explicit review warning.
- Cost, token, iteration, tool, error, and wall-time caps are enforced before a
  subsequent provider/tool step, not reported only after the fact.
- Updates and npm installs never execute an unverified partial download, and a
  failed replacement preserves the current or staged recoverable binary.

### Assumptions and exclusions

The local OS, current user account, terminal, native credential manager, and
the running Axiom binary are assumed not already compromised. An attacker with
the same user's arbitrary-code execution can generally read prompts and files
without exploiting Axiom. A deliberately approved unsafe operation is not an
authorization bypass, although misleading scope, hidden consequences, or an
approval that does not match the actual action remains in scope. Provider
availability, model answer quality, and billing accuracy are operational risks;
cap bypass, credential exposure, or falsely labeled cost is security-relevant.
External skill execution is out of scope for v1 because the product explicitly
quarantines it; any path that executes it anyway is in scope and severe.

## Attack Surface, Mitigations, and Attacker Stories

### Provider transport and agent control parsing

An adversarial provider or prompt-injected project may emit fragmented SSE,
oversized or malformed JSON, hidden control blocks, repeated tool calls, or
instructions disguised as tool output. `axiom-llm` accumulates bounded response
formats and separates visible streaming text from control blocks. Explicit caps
cover response bodies, SSE event buffers, aggregate assistant content,
tool-call counts/names/arguments, and retained error text;
`axiom-agent` parses structured calls, advertises exact executor schemas,
limits iterations/tools/tokens/time/cost/errors, and persists state transitions.
Tool observations and compacted archives are labeled untrusted and kept at user
privilege. Relevant classes are parser differentials, instruction-privilege
escalation, cap bypass, replay after resume, and confused-deputy tool use.

### Workspace, patches, checkpoints, and commands

A malicious repository may use traversal, junctions/symlinks, secret-looking
paths, duplicate patch anchors, very large files, nested package manifests, or
test metadata intended to cause arbitrary execution. `Workspace::resolve_inside`
combines lexical checks with canonical existing-ancestor containment. Coder
uses bounded context, secret-path blocks, base SHA-256 hashes, minimal hunks,
scope/file/byte caps, hunk review, safe command detection, separately validated
working directories, and pre-write snapshots. Important failure classes are
TOCTOU path swaps, silent overwrite, checkpoint escape, shell injection,
unapproved scope growth, and deletion/commit/push/deploy capability creep.
The shared secret predicate is applied to every path component both before and
after canonical workspace resolution, preventing a benign-looking symlink,
junction, or secret-named directory alias from reaching in-workspace credential
material. Git diff excludes the same families case-insensitively and recursively
before content capture, disables external diff/textconv/fsmonitor hooks, and
retains at most bounded stdout/stderr bytes.

### Network and registry data

An attacker may submit a URL targeting cloud metadata or internal services,
return a public DNS name that resolves privately, redirect to a blocked host,
stream an unbounded body, or publish malformed/downgraded registry metadata.
`web.fetch` rejects credentials in URLs, private/reserved hosts and resolved
addresses, applies deny-first host policy, pins public DNS answers, disables
redirects and proxy discovery by default, and enforces time/size caps. Registry
logic requires supported schemas, controlled URL resolution, checksums,
dependency/lifecycle validation, and quarantines unsupported entrypoints.
Relevant classes are SSRF, DNS rebinding, resource exhaustion, downgrade,
dependency-cycle, prompt injection, and registry-to-code-execution escalation.

### Credentials, state, proof, and terminal output

Credential input is hidden and stored through the native OS credential backend,
with validated, non-process-control environment-variable names as a headless
fallback. The selected provider resolves the value directly into its HTTP
client; native-keyring values are never copied into the process environment,
resolved values are registered only with the bounded in-memory proof redactor,
and every configured provider credential name is scrubbed from tool child
processes. Persistent JSON/TOML
uses versioned loaders and atomic writes; Unix state files are created mode
`0600`, while Windows relies on inherited user-profile ACLs. Proof redaction and
bounded summaries reduce leakage. Durable chat/session history and saved tool
outputs are also redacted, and an existing terminal input-history file is
sanitized before it is loaded. Proof recursively redacts its complete trace at
the persistence/export boundary, defaults new configurations to 30-day
dated-directory retention while preserving legacy configurations at disabled
retention until opt-in, refuses to prune through symlinks or Windows junctions,
and warns before export. Prompts and source excerpts can still be sensitive in
live memory/provider requests, and operators control whether Proof Mode is
enabled/exported.
Terminal rendering disables ANSI when redirected or `NO_COLOR`/plain mode is
active. Relevant classes are secret leakage, malicious terminal escapes,
future-schema downgrade, partial-write corruption, unsafe retention, and path
traversal through session/proof/output IDs.

### Updater, npm installer, and CI release

Attackers may tamper with release metadata, swap an asset and checksum, exploit
asset-name traversal, force unbounded/redirected downloads, corrupt a binary
during replacement, or steal a long-lived publishing token. The updater selects
a platform-specific expected filename, binds metadata and initial asset URLs to
the exact GitHub owner/repository/tag/name, verifies SHA-256, bounds and constrains
downloads, stages atomically, requires the installed binary to report the exact
selected semantic version, retains backups, and reports rollback failure. The npm
wrapper downloads the matching tagged asset to a private temporary path,
verifies `SHA256SUMS`, and retains a rollback copy during replacement. Workflows
use full-SHA Action pins, minimal default permissions, npm OIDC trusted
publishing, exact tag/commit and release-asset binding, duplicate-version and
dist-tag guards, cross-platform builds, SBOM creation, and GitHub attestations.
Release-owner verification of the tag and attestation is still required because
repository or publisher compromise can defeat same-origin checksums.

## Severity Calibration (Critical, High, Medium, Low)

### Critical

- A remotely reachable path from model/project/registry input to arbitrary host
  command execution outside the documented built-in capability and approval
  boundary.
- A workspace or updater path escape that overwrites an arbitrary executable,
  credential, shell profile, or system file with attacker-chosen content.
- Compromise of the tagged release/npm workflow that distributes an
  attacker-controlled binary as an apparently valid Axiom release.
- Execution of an external/quarantined skill binary without the explicitly
  deferred sandbox, trust, and authorization controls.

### High

- `web.fetch` SSRF reaching cloud metadata, loopback, or private control-plane
  services from an attacker-influenced prompt.
- Provider credentials, private keys, or substantial secret source content
  leaking into proof, logs, saved history, registry requests, or an unintended
  model request.
- Policy/approval mismatch that lets a model write files, run a process, or use
  Git after the operator denied that action or approved materially narrower
  scope.
- Update checksum/provenance bypass or partial replacement that persistently
  installs an unverified binary or destroys the only usable binary/backup.

### Medium

- Repeatable cap, cancellation, or resume flaws that cause bounded but
  unauthorized cost, duplicate idempotent operations, or denial of service
  without arbitrary code execution or sensitive-data loss.
- Proof/session corruption or omission that materially misrepresents an action
  but leaves the actual workspace recoverable and does not conceal a high-impact
  side effect.
- A registry downgrade or lifecycle inconsistency that exposes misleading
  prompt context while compiled external execution remains impossible.
- Bounded terminal/control-sequence injection that can mislead an operator but
  cannot execute commands or persist outside the current terminal session.

### Low

- Cosmetic theme/plain-output inconsistencies, inaccurate non-security health
  counters, or diagnostics that do not change enforcement.
- Local-only crashes on malformed input that require the operator to supply the
  payload and do not corrupt state, leak secrets, or bypass a boundary.
- Documentation or provenance-display defects where the underlying checksum,
  policy, and release enforcement remains correct.

Severity is lowered when exploitation requires pre-existing arbitrary code as
the same OS user, deliberate approval of an accurately described operation, or
control of a developer-only fixture that cannot reach packaged/runtime code.
It is raised when the behavior is reachable from a normal prompt, untrusted
workspace, provider response, registry, or update check and crosses one of the
boundaries above without an additional trusted decision.

Model: axiom-threat-model/v1
Source snapshot: axiom-source/v1:sha256:f212dc166318a576eb9e791ef005e4bcdee143ffa468c2f555640e8fda0d6eec
