# Roadmap

- v0.1: terminal foundation, config, provider chat, Skill Lens, built-in tool execution, remote-ready skill registry foundation, skill lifecycle and update controls, core updater plumbing, npm installer scaffold, Axiom Coder v0.1, Proof Mode v0.1, offline mock demos, isolated E2E tests, and release safety checks.
- v0.2: stronger project editing workflows and richer patch application.
- v0.3: richer proof analytics, safer multi-step workflows, and optional proof sharing/export polish.
- v0.4: broader skill ecosystem polish, registry publishing workflow, and richer trust review.
- Later: external skill binary model and app layers only after the CLI is stable.

The `axiom-skills` repository is separate from this Axiom Agent repo. The current local registry fixture stays in-tree for tests, examples, and offline fallback.

Before public npm publishing, release readiness means:

- `AXIOM_HOME` isolates automated tests from real user config.
- `axiom onboarding --non-interactive` can set up a demo workspace without prompts.
- `axiom run` covers one-shot chat, Skill Lens, one tool loop, and proof recording.
- `scripts/e2e-test.js`, `scripts/release-check.js`, and `scripts/security-check.js` pass without API keys or network calls.
