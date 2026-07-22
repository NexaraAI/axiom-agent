# Changelog

All notable changes to Axiom are documented here. Versions follow semantic
versioning. Stable releases document user-visible changes, configuration or
proof migrations, security fixes, and upgrade actions.

## 1.0.0-rc.1

This is the first public v1 release candidate. It brings the complete terminal
agent, safety model, recovery flow, provider onboarding, proof trail, and
release pipeline together for final field testing before the stable release.

### Added

- A bounded multi-step chat agent loop with tool observations and proof events.
- Native OpenAI-style tool calls, fenced fallback calls, SSE response and tool-call
  accumulation, retry/timeout handling, cancellation, todo updates, token-aware
  context compaction, and provider-reported token/cost accounting.
- Durable atomic chat sessions with `axiom sessions` and `axiom resume`.
- Transition-level session checkpoints containing tool events, approvals,
  policy decisions, usage, and pre-write workspace checkpoint references.
- Local UTC-month cost ledger, `axiom cost`, and optional per-session/monthly
  budgets shared by Chat and every Coder plan/patch/correction call; enforcement
  fails transparently when model token pricing is unavailable.
- Interactive `!multi` prompt capture with exact blank-line preservation,
  explicit submit/cancel controls, and a matching noninteractive `axiom run` path.
- Interactive line editing, bracketed paste, persistent history, safe live SSE
  rendering, semantic theme presets, and durable `!show` tool-output references.
- Conflict-aware hunk patches, workspace recovery checkpoints, project-aware
  tests, bounded correction attempts, and patch scope confirmations in Coder.
- Canonical capped Coder LLM calls, plan-to-patch path checks, and per-hunk/new
  file approval before patch application.
- Versioned skill/registry schemas, manifest-driven ranking, dependencies,
  typed built-in executors, and enforced skill-card budgets.
- Axiom identity context, word-boundary Lens matching, and the blood-red inline
  terminal theme with `NO_COLOR` support.
- Config schema versioning, `axiom config migrate`, and `axiom doctor --json`.
- The current config schema with centralized filesystem/network/process/Git
  `allow`/`ask`/`deny` policy, selectable terminal themes, and model-invoked
  `web.fetch` HTTPS/host/proxy controls.
- `docs/V1_PLAN.md`, which defines the v1 product boundary and release gates.
- Release SBOM generation, GitHub artifact attestations, and npm trusted-publisher
  workflow configuration without a long-lived npm token.
- Cargo advisory/license/source policy enforcement, locked dependency checks,
  and weekly Dependabot updates for Rust, npm, and GitHub Actions.
- Release tag/changelog/package validation and explicit beta/rc/latest npm
  dist-tag selection with `beta` as the safe publishing default.
- Release-bound npm publication that verifies the exact `v<version>` checkout,
  matching GitHub Release, all four binaries, `SHA256SUMS`, and npm version
  availability before entering the protected publish environment.
- A packed-tarball install smoke that uses `AXIOM_AGENT_BINARY_PATH` and invokes
  the installed global shim on the declared Node.js 20 minimum.
- Full locked-workspace release validation before builds, native release-binary
  E2E smoke tests, automatic GitHub prerelease classification, and fail-closed
  npm semantic-version/dist-tag matching.
- Bounded HTTPS-only npm installer downloads with a trusted GitHub redirect
  allowlist, request timeout, streamed size caps, exclusive temporary files,
  checksum-before-install enforcement, and rollback-safe replacement.
- First-class Groq, OpenRouter, Gemini, GitHub Models, NVIDIA NIM, OpenAI,
  Ollama, and LM Studio onboarding presets, including optional authentication for trusted
  local OpenAI-compatible servers and rate-limited free defaults where safe.
- Deterministic malformed-input/property corpora for patch and tool request
  parsing, SSE framing, workspace path containment, and proof redaction.
- Provider readiness diagnostics in human and JSON `axiom doctor` output,
  including missing key-variable detection without secret disclosure.
- Guided one-or-two-provider onboarding with hidden credential paste, native OS
  credential storage, catalog-only model discovery/search, custom catalog URLs,
  per-provider model memory, and `axiom model`/`axiom provider` commands.
- Deterministic credential-store and model-catalog failure coverage for missing
  stores, malformed/empty catalogs, authentication, rate limiting, and size caps.
- An executable-embedded essential skill registry materialized under the user
  config directory, with relocated-binary E2E coverage for offline onboarding.
- Thirty-day default Proof retention for new configurations, legacy-safe
  disabled retention until opt-in, junction/symlink-safe automatic pruning, and
  an explicit privacy warning on export.

### Changed

- First-run no-argument startup now continues from onboarding and local doctor
  checks directly into terminal chat. Coder plans can be revised in place, and
  provider switching clears an incompatible stale model selection.
- Rerunning onboarding migrates legacy config and preserves existing policy,
  network, registry, UI, Coder, Proof, agent-cap, and update settings.
- One-shot provider overrides restore that provider's saved model, catalog
  displays are filterable and capped at 100 matches, incomplete provider setup
  stays in onboarding, and Cloudflare noninteractive setup requires an account ID.
- The duplicate in-repository skill manifests were removed; the published
  `axiom-skills` registry is the manifest source of truth.
- Version synchronization now covers every internal Cargo path dependency's
  exact pin and every workspace package entry in `Cargo.lock`; Linux x86-64
  release binaries use an Ubuntu 22.04/glibc 2.35 compatibility floor.

### Security

- Documentation now distinguishes executable built-ins from prompt-only skill
  cards so installed metadata is not mistaken for executable code.
- State files and tool writes use atomic replacement; Windows replacements use
  replace-existing/write-through semantics and Unix replacements sync the parent.
- `web.fetch` requires HTTPS and disables system proxy discovery by default;
  deny-first exact/wildcard host policy cannot override private/loopback hard
  blocks. Public DNS results are pinned, redirects and embedded credentials are
  rejected, and response limits are enforced while reading the body.
- Remote provider and model-catalog endpoints require HTTPS; plain HTTP is
  limited to literal loopback hosts. Provider URLs reject embedded credentials,
  query strings, and fragments, and provider clients do not follow redirects.
- Credential-variable names are validated and cannot replace process-control,
  dynamic-loader, proxy, or Axiom home variables.
- Native-keyring and environment credentials are passed directly to provider
  clients instead of hydrating process-global state. Test, diagnostic, and Git
  children scrub every configured provider credential name; Git diff disables
  external diff and textconv drivers.
- Secret-file policy is centralized and applied before and after canonical path
  resolution, closing symlink/junction aliases. Git inspection excludes those
  paths before capture and drains stdout/stderr with bounded retention.
- Secret directories and case variants are blocked consistently; Git exclusions
  are case-insensitive, recursive, and disable fsmonitor as well as external diff
  and text-conversion hooks.
- Proof prompts, terminal/session history, transition state, and saved tool
  outputs receive mandatory exact-value/token-shape redaction before durable
  persistence, even when a legacy config requests redaction off.
- Proof applies recursive redaction at the final persistence/export boundary so
  provider, model, path, approval, command, Lens, policy, and nested metadata
  cannot bypass recorder-level capture helpers.
- Provider success/error bodies, SSE events, aggregate assistant text, and tool
  arguments have explicit byte/count caps. Provider clients ignore system proxy
  discovery, including authenticated loopback endpoints.
- Proof export recognizes camelCase credential keys, preserves structural trace
  discriminators, avoids redacting semantic `sk-*` identifiers, and cannot
  prune through symlinked or Windows junctioned proof directories.
- The Rust updater binds metadata/assets to the exact GitHub repository, tag,
  and filename, separates metadata/checksum/binary size limits, rejects unsafe
  local source shadowing, verifies the installed semantic version exactly, and
  reports rollback failures.
- Built-in side effects pass through a centralized policy evaluator and record
  the decision in Proof/session state; Unix private atomic state files use
  restrictive permissions.

## 0.5.1-beta

- Republished the npm beta package with synchronized Cargo and npm versions.

## 0.5.0-beta

- Initial terminal CLI, skill registry, proof mode, coding mode, updater, npm
  installer, offline mock demos, and release-safety checks.
