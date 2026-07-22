# Roadmap

The v1 delivery plan, release gates, and researched supply-chain requirements
are tracked in [V1_PLAN.md](V1_PLAN.md).

- Published beta: terminal foundation, config, provider chat, Skill Lens,
  built-in tool execution, registry lifecycle, updater plumbing, npm wrapper,
  Coder, Proof Mode, offline demos, and release-safety checks.
- Implemented in the v1 candidate source: native/fallback multi-tool runtime,
  safe live streaming, cancellation and caps, transition-level durable sessions,
  pre-write recovery checkpoints, canonical Coder LLM execution, plan-to-patch
  validation, per-hunk approval, centralized built-in/write policy and proof
  events, guided one-or-two-provider onboarding, native credential storage,
  catalog-only model discovery, persistent line history, bracketed paste,
  multiline input, durable `!show` output, semantic themes, atomic state writes,
  HTTPS-only/no-proxy web fetch defaults with deny-first host policy and pinned
  public DNS, HTTPS/no-redirect remote provider transports, cross-process cost
  ledger locking and persistent Chat/Coder budgets, recursive monorepo test detection,
  native release-binary smoke jobs, SBOM generation, artifact attestations,
  semantic-version/dist-tag guards, and npm OIDC workflow configuration.
- Before RC.1: finish the local full gate, then complete cross-platform CI,
  native credential-store/provider smoke tests, manual accessibility checks,
  tagged artifact/checksum/SBOM/attestation verification, npm trusted-publisher
  registration, and release-owner sign-off.
- Later: independently reviewed external executable skills, a full-screen TUI,
  remote registry publishing workflows, and desktop/mobile/web layers.

The `axiom-skills` repository is separate from this Axiom Agent repo. The local registry fixture stays in-tree for tests, examples, and offline fallback.

The `axiom-agent@beta` package is published on npm. No v1 RC or stable package
is claimed yet. The authoritative go/no-go record is
[V1_RC_CHECKLIST.md](V1_RC_CHECKLIST.md); the short list below is only the
local automated baseline:

- `AXIOM_HOME` isolates automated tests from real user config.
- `axiom onboarding --non-interactive` sets up a demo workspace without prompts.
- `axiom run` covers one-shot chat, Skill Lens, the bounded multi-tool loop, and proof recording.
- `axiom sessions` and `axiom resume <id>` preserve versioned transition state,
  approvals, policy decisions, todo, workspace checkpoint reference, and usage.
- `scripts/e2e-test.js`, `scripts/release-check.js`, and `scripts/security-check.js` pass without API keys or network calls.
