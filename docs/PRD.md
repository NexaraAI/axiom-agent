# Axiom v1 Product Contract

## Product

Axiom v1 is a terminal-native coding agent. It provides guided provider setup,
interactive chat, one-shot automation, bounded multi-step tool use, project
editing with review and recovery, resumable sessions, and proof reports.

The repository and package are versioned `1.0.0-rc.1`. This release candidate
is the final public validation step before the stable v1 promotion.

## Supported release targets

The release workflow builds and tests these binary targets:

- Windows x86-64 (`x86_64-pc-windows-msvc`)
- Linux x86-64 glibc (`x86_64-unknown-linux-gnu`)
- macOS x86-64 (`x86_64-apple-darwin`)
- macOS Apple silicon (`aarch64-apple-darwin`)

The npm wrapper supports only targets for which the matching GitHub Release
binary and checksum exist. Source builds may work elsewhere, but they are not
v1 release targets until added to the tested matrix.

## Required user journeys

A v1 release must let a user:

1. install the verified binary directly or through the npm wrapper;
2. start `axiom` and complete friendly provider/model onboarding;
3. use a hosted provider, local OpenAI-compatible endpoint, or deterministic
   mock provider for tests and demos;
4. chat interactively without configuring a separate messaging channel;
5. complete a bounded multi-step tool task with visible policy decisions;
6. review and approve a coding plan and individual patch hunks;
7. recover from interruption or a failed write using saved session/workspace
   checkpoints; and
8. inspect a redacted Proof report and verify release provenance.

## Provider contract

First-class presets cover Groq, OpenRouter, Gemini, GitHub Models, NVIDIA NIM,
OpenAI, Cloudflare AI Gateway, Ollama, and LM Studio. Custom OpenAI-compatible endpoints
can supply their own completion and model-catalog URLs. Onboarding may configure
one or two providers, stores secrets in the native credential manager when
available, and uses environment variables as the headless fallback. Model
catalog discovery does not send an inference prompt.

Hosted free tiers, limits, and model availability are provider-controlled and
can change. Axiom must describe them as availability options, never as a
guarantee of zero cost. The mock provider is for tests and demos only.

## Execution and safety contract

Only compiled-in executors run in v1: `file.read`, `file.write`,
`project.scan`, `web.fetch`, `git.status`, and `git.diff`. Other installed
manifests may contribute prompt context but do not become executable code.

Agent autonomy is bounded by iteration, tool-call, time, token, cost, and error
caps. Workspace containment and secret-path rules apply before file access.
Built-in tool execution and Coder file writes pass through configurable
`allow`, `ask`, or `deny` policy. Detected Coder test commands use a separate
strict allowlist and approval gate. Axiom checkpoints before agent file writes,
records policy-routed approvals and outcomes, and fails closed on malformed or
unsupported requests.

Optional per-session and UTC-month cost budgets require both configured token
rates. With complete pricing, Axiom records local per-session spend in
`cost-ledger.json`, blocks Chat and Coder provider calls when a persistent budget
is exhausted, and reports both through `axiom cost`. With unknown/partial pricing, persistent
enforcement and new cost recording are unavailable and must be reported as such.

Coder may scan, plan, propose minimal conflict-aware hunks, apply approved
changes, and run detected allowlisted project tests. It does not delete,
commit, push, deploy, or execute arbitrary shell commands.

Model-invoked `web.fetch` requires HTTPS and ignores system proxy discovery by
default. Operators can constrain public destinations with exact-host and
`*.domain` patterns; deny matches are evaluated before allow matches. Redirects
are disabled. Localhost, `.local`, loopback, private/reserved addresses, and
hostnames resolving to them remain hard blocked and cannot be allowlisted.
Allowing HTTP or enabling the system proxy does not relax that SSRF boundary.
These controls do not apply to separately configured model-provider endpoints.

## Terminal and state contract

The inline terminal UI is the v1 interface. It supports live safe text
rendering, line editing, input history, bracketed paste, explicit multiline
capture, durable large-output references, recovery commands, semantic themes,
`NO_COLOR`, and plain redirected output. A full-screen TUI is not required for
v1.

Config, sessions, proofs, lifecycle data, checkpoints, output references, and
update state live under the platform config directory or `AXIOM_HOME`. State
writes use atomic replacement. The current versioned config and session schemas
accept older beta state where documented; unknown future schemas fail closed.
`axiom doctor --json` reports the loaded and supported config schemas and
migration status. It validates config syntax without contacting configured web
hosts or treating proxy reachability as a mandatory check.

## Distribution contract

Release binaries must be built from a clean matching tag with locked
dependencies. The GitHub Release must include platform binaries,
`SHA256SUMS`, an SPDX JSON SBOM, and GitHub artifact attestations. npm
publishing uses a deliberately selected `beta`, `rc`, or `latest` dist-tag and
trusted-publisher OIDC; stable promotion uses `latest` only after the same
audited commit passes the v1 exit criteria.

The authoritative go/no-go record is [V1_RC_CHECKLIST.md](V1_RC_CHECKLIST.md).

## Explicit non-goals for v1

- arbitrary external executable skill binaries or WASM modules;
- unrestricted sub-agent swarms;
- a full-screen TUI;
- desktop, mobile, or web application layers;
- autonomous commit, push, deployment, or destructive file operations; and
- guaranteed free hosted inference.
