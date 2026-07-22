# Release and RC Operations

Axiom publishes verified GitHub Release binaries and a matching npm wrapper.
The manifests are set to `1.0.0-rc.1` for the first v1 release candidate.

The release owner has authorized `1.0.0-rc.1` for prerelease publication. Push
the tag only after the local gate passes; the tag starts the GitHub binary
workflow, and npm publication follows only after those artifacts verify. Stable
promotion remains blocked by the outstanding items in
[V1_RC_CHECKLIST.md](V1_RC_CHECKLIST.md).

## Version Sync

Keep these versions identical:

- `Cargo.toml` `[workspace.package] version`
- `package.json` `version`
- every workspace package recorded in `Cargo.lock`
- every internal Cargo path dependency's exact `=version` pin

Check them with:

```bash
npm run check-version-sync
```

Run the full local release gate with:

```bash
npm run release-check
npm run security-check
```

`release-check` verifies version sync (including internal exact pins and the
lockfile), NexaraAI repository URLs, the default skills registry, workflow
files, docs, license, and confirms that tracked release artifacts (`target/`,
`node_modules/`, `.env`, proof logs, `vendor/bin` binaries, PDBs, and compiler
scratch binaries) are absent. Root `rust_out.exe` and `rust_out.pdb` files are
explicitly ignored and forbidden from release tracking.

It also verifies that the current config schema is documented consistently and
that v1 uses the RC checklist rather than the historical v0.5-only checklist.

`security-check` scans project files for obvious secrets: provider API key assignments, bearer tokens, `sk-` keys, GitHub tokens, private key blocks, and tracked `.env` content. It skips safe placeholder examples in docs.

Release workflows pin every GitHub Action to a reviewed full commit SHA and use
the minimum `GITHUB_TOKEN` permissions required by each job. The local release
check rejects mutable action tags.

## GitHub Release Flow

`.github/workflows/release.yml` triggers on tags like:

```bash
v1.0.0-rc.1
```

That example is the planned first RC tag, not an instruction to create it from
the current beta-versioned working tree.

The workflow builds these targets:

- `x86_64-pc-windows-msvc`
- `x86_64-unknown-linux-gnu`
- `x86_64-apple-darwin`
- `aarch64-apple-darwin`

Before the target matrix starts, the validation job requires the Git tag to
equal `v` plus the synchronized Cargo/npm version and requires a matching
changelog heading. It then runs locked metadata, formatting, strict full-
workspace/all-feature Clippy and tests, cargo-deny, Node smoke and isolated E2E,
release/security policy checks, and an npm package dry-run. The build matrix
cannot start unless every validation gate passes.

Each target is built on a native runner: Windows x86-64, Linux x86-64, macOS
Intel, and macOS Apple silicon. Before upload, the runner executes the full
offline E2E suite against that exact release binary through
`AXIOM_E2E_BINARY`. All target builds use `Cargo.lock` with `--locked`.

The Linux x86-64 artifact is intentionally built on Ubuntu 22.04. Its supported
binary floor is glibc 2.35; newer glibc systems are expected to work, while
older glibc distributions are outside the v1 prebuilt-binary contract and
should build from source.

Uploaded asset names match the npm platform resolver:

- `axiom-x86_64-pc-windows-msvc.exe`
- `axiom-x86_64-unknown-linux-gnu`
- `axiom-x86_64-apple-darwin`
- `axiom-aarch64-apple-darwin`

The release job is configured to generate `SHA256SUMS` and
`axiom-agent.spdx.json`. Both the npm installer and `axiom update install`
download the binary and checksum file, then refuse to install if verification
fails. The SPDX JSON file is the release SBOM generated from the tagged
workspace and packaged binaries. These artifacts and attestations count as
release evidence only after they are downloaded and verified from the actual
candidate workflow.

GitHub's OIDC-backed attestation service signs SLSA build provenance for every release file and attaches the SBOM as an attestation for the binary checksum set. Verify a downloaded binary with:

```bash
gh attestation verify ./axiom-x86_64-unknown-linux-gnu --repo NexaraAI/axiom-agent
```

The release job requires only short-lived `id-token`, `attestations`, and artifact-metadata permissions for signing; no signing key is stored in repository secrets.

Tags whose validated semantic version contains a hyphen, including
`v1.0.0-rc.1` and beta builds, are created as GitHub prereleases and are not
marked latest. Only a stable version can become the latest GitHub Release.

## Dependency policy

Every pull request verifies that `Cargo.lock` is current and runs cargo-deny over all features and supported v1 targets. `deny.toml` fails known advisories, wildcard dependency requirements, unreviewed registries/Git sources, forbidden TLS dependencies, and licenses outside the reviewed allowlist. `.github/dependabot.yml` schedules Cargo, npm, and GitHub Actions updates; repository owners must also enable Dependabot alerts and security updates in GitHub settings.

## Core Updater Contract

The core updater reads GitHub Releases from:

```text
https://github.com/NexaraAI/axiom-agent
```

It expects the asset names listed above plus `SHA256SUMS`. The stable channel ignores prereleases. The nightly channel can use prereleases. The dev channel can read mocked release metadata from a local JSON file or directory for testing.

Both normal installs and updater installs require a matching tagged GitHub Release with those assets. Do not claim update availability for a version until you have uploaded matching release assets.

The npm installer accepts only HTTPS downloads from the configured GitHub
repository and reviewed GitHub release-asset hosts. It revalidates every
redirect, allows at most five, applies a 60-second request timeout, caps binary
and checksum downloads, writes a private exclusive temporary file, and verifies
`SHA256SUMS` before installation. Existing binaries are moved to a temporary
backup and restored if replacement or final permission setup fails. Node smoke,
release, and security checks run the installer policy self-test so these
boundaries cannot silently regress.

## npm Publish Flow

`.github/workflows/npm-publish.yml` runs smoke tests and `npm pack --dry-run`.

Publishing is manual and is configured for npm trusted publishing (OIDC):

1. In the `axiom-agent` package settings on npmjs.com, configure the GitHub Actions trusted publisher for organization `NexaraAI`, repository `axiom-agent`, and workflow filename `npm-publish.yml`; allow `npm publish`.
2. Create the GitHub environment `npm-publish`, require release-owner approval,
   and restrict deployment to reviewed release tags. The workflow's publish job
   is bound to this environment.
3. Require 2FA and disallow token-based publishing in the package publishing-access settings.
4. Start the workflow by hand, enter the already-published GitHub Release tag,
   set `publish` to `true`, and deliberately choose the npm dist-tag: `beta`,
   `rc`, or `latest`. The safe default is `beta`.

The publish job uses Node 24 and pinned npm 11.5.1, requests an OIDC token, and
runs without `NPM_TOKEN` or `NODE_AUTH_TOKEN`. npm trusted publishing generates
package provenance automatically for this public package. If the npm-side
publisher registration is absent or does not match the workflow filename,
publishing fails closed with `ENEEDAUTH`.

The workflow validates the package semantic version against the selected
dist-tag before both its dry-run/smoke path and the guarded publish path.
`*-beta[.*]` accepts only `beta`, `*-rc[.*]` accepts only `rc`, and a version
without a prerelease component accepts only `latest`. Other prerelease labels
fail closed. The publish command always includes the validated dist-tag.

The checkout is pinned to the entered `v<package version>` tag, and its commit
must equal the tag target. The workflow then reads that exact GitHub Release
and requires all four platform binaries plus `SHA256SUMS`. Immediately before
publishing it repeats those checks and asks npm whether the exact immutable
version already exists; an existing version or an inconclusive registry query
stops publication. Both validation and publish jobs install the generated
tarball with `AXIOM_AGENT_BINARY_PATH` and execute its installed global shim.

`package.json` also declares public access and a safe `beta` publish tag. Its
`prepublishOnly` lifecycle guard independently validates the effective npm
`--tag`, including local `npm publish` commands. When cutting an RC or stable
version, change `publishConfig.tag` with the version (`rc` or `latest`); a
mismatch fails before upload.

Release publish events run validation but do not push to npm.

Install the release candidate with:

```bash
npm install -g axiom-agent@rc
```

Registry audit note (2026-07-20): npm's live `latest` tag still resolves to the
published `0.5.1-beta`. That external tag predates these guards and must be
removed by a package owner before any stable announcement:

```bash
npm dist-tag rm axiom-agent latest
npm view axiom-agent dist-tags --json
```

Do not run that mutation from CI. A release owner must confirm npm ownership,
remove it deliberately, and attach the readback to the RC evidence.

Every npm version is immutable. A later candidate must use a new prerelease
number across Cargo, npm, internal exact pins, `Cargo.lock`, and the exact
changelog heading. Never try to overwrite an existing package version.

The `@rc` tag is reserved for release candidates. Keep `@latest` reserved for
the reviewed stable promotion.

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
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
cargo test --workspace --all-features --locked
cargo deny check
node scripts/smoke-test.js
node scripts/e2e-test.js
node scripts/release-check.js
node scripts/security-check.js
node scripts/check-dist-tag.js --self-test
npm run packed-smoke
npm pack --dry-run
cargo build -p axiom-cli --release --locked
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
