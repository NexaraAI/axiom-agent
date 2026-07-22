# Testing

All Axiom tests run without API keys and without writing to real user config.

## Config Isolation

Set `AXIOM_HOME` for local tests:

```bash
AXIOM_HOME=/tmp/axiom-test-home cargo run -p axiom-cli -- doctor
```

When `AXIOM_HOME` is set, Axiom writes to:

```text
config.toml
skills/
  installed_skills.json
proofs/
updates/
registry-cache/
sessions/
checkpoints/
outputs/
input-history.txt
```

Without it, Axiom uses the normal platform config directory.

## Offline Setup

Use the mock provider and bundled registry:

```bash
cargo run -p axiom-cli -- onboarding --non-interactive --provider mock --workspace ./demo-workspace --yes
```

For a pinned registry:

```bash
cargo run -p axiom-cli -- onboarding --non-interactive --provider mock --workspace ./demo-workspace --registry ./fixtures/skill-registry --yes
```

The mock provider lives in `axiom-llm`. It returns deterministic responses and exists for tests and demos.

## One-Shot Chat

`axiom run` exercises the normal chat pipeline once:

```bash
axiom run "hello"
axiom run "read README.md and summarize it"
axiom run "hello" --no-tools --no-proof
```

It loads config, runs Skill Lens, injects skill cards, calls the configured provider, executes the bounded multi-tool loop when requested and allowed, records Proof Mode output, persists the session, prints the final response, and exits.

## Local Test Gate

Run:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo deny check
node scripts/smoke-test.js
node scripts/e2e-test.js
node scripts/release-check.js
node scripts/security-check.js
npm pack --dry-run
cargo build -p axiom-cli --release --locked
```

`scripts/e2e-test.js` creates a temporary `AXIOM_HOME` and workspace, relocates
the Axiom binary outside the checkout, runs non-interactive onboarding, derives
and verifies the current config schema plus legacy migration, and checks doctor,
provider/model listing and switching, inline model discovery, skill commands,
`axiom run`, persisted-session listing/resume, cost-ledger status, coder
plan-only, proof list, and updater status. It uses the binary's embedded
starter registry and mock provider, so catalog tests
make no external request.

Workspace tests also run deterministic malformed-input/property corpora over
patch and tool-request parsing, SSE event framing and safe control-block
projection, workspace path containment, proof redaction, credential-store
fallback, and provider catalog 401/429/malformed/oversized responses. These are
fast regression lanes; long-running coverage-guided fuzzing remains a separate
RC hardening gate.

`scripts/release-check.js` verifies version sync, repository URLs, registry
URL, workflows, docs, license, npm status text, current config-schema
documentation, the v1 RC checklist, and tracked-file safety.

`scripts/security-check.js` scans project files for obvious secrets and ignores safe placeholder examples.

CI runs full-workspace/all-feature locked tests plus the Node lane on Ubuntu,
Windows, and macOS, and runs Linux formatting, strict-Clippy,
security/release checks, and `cargo deny check` lanes. The tag-triggered release
workflow repeats the complete Rust, Node, dependency, and package policy gate
before its build matrix, then runs isolated E2E against each exact native
release binary before upload. The dependency policy rejects known advisories,
wildcard Cargo requirements, unapproved registries/Git sources, OpenSSL
dependencies, and licenses outside the reviewed allowlist. Dependabot monitors
Cargo, npm, and SHA-pinned GitHub Actions weekly; security updates must still be
enabled in the repository settings.

Passing local and CI automation is not release authorization. Record platform,
credential-store, terminal accessibility, tagged provenance, npm OIDC, and
operator evidence in [V1_RC_CHECKLIST.md](V1_RC_CHECKLIST.md).
