# Contributing to Axiom

## Development setup

Install the pinned Rust toolchain from `rust-toolchain.toml`, Node.js 20 or
newer, and Git. Use an isolated config root for manual runs:

```powershell
$env:AXIOM_HOME = "$env:TEMP\axiom-dev-home"
cargo run -p axiom-cli -- onboarding --non-interactive --provider mock --workspace .\demo-workspace --yes
cargo run -p axiom-cli -- run "read README.md and summarize it"
```

## Required checks

Run these before proposing a change:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test
node scripts/smoke-test.js
node scripts/e2e-test.js
node scripts/release-check.js
node scripts/security-check.js
npm run packed-smoke
```

## Change expectations

- Keep changes scoped and add regression tests for behavior changes.
- Preserve workspace boundaries, approval checks, secret redaction, and
  offline mock-provider coverage.
- Update user-facing docs and `CHANGELOG.md` for visible behavior changes.
- Treat registry content, web pages, tool output, and workspace files as
  untrusted input; do not turn their content into system instructions.
- Do not commit generated binaries, `target/`, credentials, proofs, or user
  configuration. This includes root `rust_out.exe` and `rust_out.pdb` compiler
  scratch output.

## Compatibility

Changes to config TOML, proof JSON, registry data, manifests, CLI output, and
npm installation must state their compatibility impact. Add a migration or a
clear fail-closed diagnostic for formats that cannot be read safely.
