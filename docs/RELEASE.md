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

The release job also generates `SHA256SUMS`. The npm installer downloads the binary and checksum file for the package version and refuses to install if verification fails.

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

Linux/macOS:

```bash
cargo build -p axiom-cli --release
export AXIOM_AGENT_BINARY_PATH="$PWD/target/release/axiom"
npm install -g .
axiom --version
axiom doctor
```
