# Testing

Axiom tests should be safe to run without API keys and without writing to real user config.

## Config Isolation

Set `AXIOM_HOME` for local tests:

```bash
AXIOM_HOME=/tmp/axiom-test-home cargo run -p axiom-cli -- doctor
```

When `AXIOM_HOME` is set, Axiom writes:

```text
config.toml
skills/
  installed_skills.json
proofs/
updates/
registry-cache/
```

If it is not set, Axiom uses the normal platform config directory.

## Offline Setup

Use the mock provider and bundled registry:

```bash
cargo run -p axiom-cli -- onboarding --non-interactive --provider mock --workspace ./demo-workspace --yes
```

For a fully pinned registry:

```bash
cargo run -p axiom-cli -- onboarding --non-interactive --provider mock --workspace ./demo-workspace --registry ./fixtures/skill-registry --yes
```

The mock provider is deterministic and lives in `axiom-llm`. It is for tests and demos only.

## One-Shot Chat

`axiom run` exercises the normal chat pipeline once:

```bash
axiom run "hello"
axiom run "read README.md and summarize it"
axiom run "hello" --no-tools --no-proof
```

It loads config, runs Skill Lens, injects skill cards, calls the configured provider, executes one tool loop when requested and allowed, records Proof Mode output, prints the final response, and exits.

## Local Test Gate

Run:

```bash
cargo fmt
cargo clippy --all-targets --all-features
cargo test
node scripts/smoke-test.js
node scripts/e2e-test.js
node scripts/release-check.js
node scripts/security-check.js
npm pack --dry-run
```

`scripts/e2e-test.js` creates a temporary `AXIOM_HOME`, a temporary workspace, builds or locates the Axiom binary, runs non-interactive onboarding, checks doctor, skill commands, `axiom run`, coder plan-only, proof list, and updater status. It uses the local registry fixture and mock provider.

`scripts/release-check.js` verifies version sync, repository URLs, registry URL, workflows, docs, license, npm status text, and tracked-file safety.

`scripts/security-check.js` scans project files for obvious secrets and ignores safe placeholder examples.
