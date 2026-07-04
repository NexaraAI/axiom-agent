# Roadmap

- v0.5.0-beta: terminal foundation, config, provider chat, Skill Lens, built-in tool execution, remote-ready skill registry foundation, skill lifecycle and update controls, core updater plumbing, npm installer scaffold, Axiom Coder, Proof Mode, offline mock demos, isolated E2E tests, release assets, and release safety checks.
- Next: stronger project editing workflows, richer patch application, proof analytics, safer multi-step workflows, registry publishing workflow, and richer trust review.
- Later: external skill binary model and app layers only after the CLI is stable.

The `axiom-skills` repository is separate from this Axiom Agent repo. The current local registry fixture stays in-tree for tests, examples, and offline fallback.

Before public npm publishing, release readiness means:

- `AXIOM_HOME` isolates automated tests from real user config.
- `axiom onboarding --non-interactive` can set up a demo workspace without prompts.
- `axiom run` covers one-shot chat, Skill Lens, one tool loop, and proof recording.
- `scripts/e2e-test.js`, `scripts/release-check.js`, and `scripts/security-check.js` pass without API keys or network calls.
