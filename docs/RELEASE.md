# Release

Axiom has release workflow plumbing for GitHub Release binaries and npm installation. This document covers the manual `0.5.0-beta` release path. Nothing publishes on its own.

## Version Sync

Keep these versions identical:

- `Cargo.toml` `[workspace.package] version`
- `package.json` `version`

Check them with:

```bash
npm run check-version-sync
```

Run the full local release gate with:

```bash
npm run release-check
npm run security-check
```

`release-check` verifies version sync, NexaraAI repository URLs, the default skills registry, workflow files, docs, license, and confirms that tracked release artifacts (`target/`, `node_modules/`, `.env`, proof logs, `vendor/bin` binaries) are absent.

`security-check` scans project files for obvious secrets: provider API key assignments, bearer tokens, `sk-` keys, GitHub tokens, private key blocks, and tracked `.env` content. It skips safe placeholder examples in docs.

## GitHub Release Flow

`.github/workflows/release.yml` triggers on tags like:

```bash
v0.5.0-beta
```

The workflow builds these targets:

- `x86_64-pc-windows-msvc`
- `x86_64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`

Uploaded asset names match the npm platform resolver:

- `axiom-x86_64-pc-windows-msvc.exe`
- `axiom-x86_64-unknown-linux-gnu`
- `axiom-x86_64-apple-darwin`
- `axiom-aarch64-apple-darwin`

The release job also generates `SHA256SUMS`. Both the npm installer and `axiom update install` download the binary and checksum file, then refuse to install if verification fails.

## Core Updater Contract

The core updater reads GitHub Releases from:

```text
https://github.com/NexaraAI/axiom-agent
```

It expects the asset names listed above plus `SHA256SUMS`. The stable channel ignores prereleases. The nightly channel can use prereleases. The dev channel can read mocked release metadata from a local JSON file or directory for testing.

Both normal installs and updater installs require a matching tagged GitHub Release with those assets. Do not claim update availability for a version until you have uploaded matching release assets.

## npm Publish Flow

`.github/workflows/npm-publish.yml` runs smoke tests and `npm pack --dry-run`.

Publishing is manual:

1. Start the workflow by hand.
2. Set `publish` to `true`.
3. Confirm `NPM_TOKEN` is configured as a repository secret.

Release publish events run validation but do not push to npm.

The `axiom-agent@beta` package is published on npm. Install it with:

```bash
npm install -g axiom-agent@beta
```

## Release Repository URL

Keep the GitHub URL in `package.json` as:

```text
https://github.com/NexaraAI/axiom-agent
```

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
npm pack --dry-run
```

The E2E test uses a temporary `AXIOM_HOME`, a temporary workspace, the local skill registry fixture, and the `mock` provider. It needs no real API keys and makes no network calls.

Linux/macOS:

```bash
cargo build -p axiom-cli --release
export AXIOM_AGENT_BINARY_PATH="$PWD/target/release/axiom"
npm install -g .
axiom --version
axiom doctor
```
