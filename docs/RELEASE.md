# Release

Stage 8 adds the foundation for GitHub Release binaries and npm installation. It does not publish anything automatically.

## Version Sync

Keep these versions identical:

- `Cargo.toml` `[workspace.package] version`
- `package.json` `version`

Check them with:

```bash
npm run check-version-sync
```

The broader local release gate is:

```bash
npm run release-check
npm run security-check
```

`release-check` verifies version sync, NexaraAI repository URLs, the default skills registry, workflow files, docs, license, and that tracked release artifacts such as `target/`, `node_modules/`, `.env`, proof logs, and `vendor/bin` binaries are absent.

`security-check` scans project files for obvious secrets such as provider API key assignments, bearer tokens, `sk-` keys, GitHub tokens, private key blocks, and tracked `.env` content. It ignores safe placeholder examples in docs.

## GitHub Release Flow

`.github/workflows/release.yml` triggers on tags like:

```bash
v0.1.0
```

It builds:

- `x86_64-pc-windows-msvc`
- `x86_64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`

The uploaded asset names match the npm platform resolver:

- `axiom-x86_64-pc-windows-msvc.exe`
- `axiom-x86_64-unknown-linux-gnu`
- `axiom-x86_64-apple-darwin`
- `axiom-aarch64-apple-darwin`

The release job also generates `SHA256SUMS`. The npm installer and `axiom update install` both download the binary and checksum file and refuse to install if verification fails.

## Core Updater Contract

The core updater reads GitHub Releases from:

```text
https://github.com/NexaraAI/axiom-agent
```

It expects the asset names listed above plus `SHA256SUMS`. Stable channel ignores prereleases. Nightly channel can use prereleases. Dev channel can read mocked release metadata from a local JSON file or directory for testing.

This repository still needs a tagged release before normal users can install through the updater. Do not claim update availability until matching release assets exist.

## npm Publish Flow

`.github/workflows/npm-publish.yml` runs smoke tests and `npm pack --dry-run`.

Publishing is manual only:

1. Start the workflow manually.
2. Set `publish` to `true`.
3. Ensure `NPM_TOKEN` is configured as a repository secret.

Release publish events run validation but do not publish to npm.

## Placeholder Repository URL

The GitHub URL in `package.json` is currently a placeholder. Before public release, update:

- `repository.url`
- `axiomAgent.releaseRepo`

The installer also supports `AXIOM_AGENT_RELEASE_REPO` for testing alternate release locations.

## Local Preflight

Windows:

```powershell
cargo build -p axiom-cli --release
$env:AXIOM_AGENT_BINARY_PATH = "C:\Axiom\target\release\axiom.exe"
npm install -g .
axiom --version
axiom doctor
```

Full preflight before creating a tag:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features
cargo test
node scripts/smoke-test.js
node scripts/e2e-test.js
node scripts/release-check.js
node scripts/security-check.js
```

The E2E test uses a temporary `AXIOM_HOME`, a temporary workspace, the local skill registry fixture, and the `mock` provider. It does not require real API keys and should not make network calls.

Linux/macOS:

```bash
cargo build -p axiom-cli --release
export AXIOM_AGENT_BINARY_PATH="$PWD/target/release/axiom"
npm install -g .
axiom --version
axiom doctor
```
