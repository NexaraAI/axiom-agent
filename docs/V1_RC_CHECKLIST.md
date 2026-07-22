# Axiom v1 RC Checklist

This is the go/no-go record for the first v1 release candidate. Check an item
only when its evidence comes from the exact commit proposed for the tag.

## Current release decision

**RC.1 CANDIDATE — publish only as a prerelease on the `rc` channel.**

The source and package manifests are `1.0.0-rc.1`. This candidate may be used
for public RC testing, but it must not be promoted to stable until the remaining
platform, provenance, and owner gates below are signed off.

Install the candidate from:

```bash
npm install -g axiom-agent@rc
```

Never advertise `axiom-agent@latest` until the audited stable commit is
promoted to `v1.0.0`.

As observed on 2026-07-20, the live npm `latest` tag incorrectly points to the
published beta. A package owner must remove that stale external tag and attach
an `npm view axiom-agent dist-tags --json` readback before RC sign-off.

## Candidate identity and freeze

- [ ] Record release owner, candidate commit SHA, UTC timestamp, and intended
      version below.
- [x] Freeze features; allow only release-blocking fixes after RC.1.
- [x] Update Cargo/npm versions together to `1.0.0-rc.1` and regenerate the
      lockfile intentionally; update every internal exact path-dependency pin
      and set `publishConfig.tag` to `rc`.
- [x] Add a matching `CHANGELOG.md` heading and final RC notes.
- [x] Confirm `npm run check-version-sync` passes.
- [ ] Confirm the candidate worktree and submodules are clean.
- [x] Review the complete diff from the last published beta.

| Field | Required value |
| --- | --- |
| Release owner | Cyrusbye |
| Candidate version | `1.0.0-rc.1` |
| Commit SHA | Pending |
| Validation started (UTC) | 2026-07-22 |
| Final decision | RC publication authorized; stable promotion pending |

## Local automated gate

Run from a clean checkout with the pinned toolchain and no production API keys:

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

- [x] Formatting passes.
- [x] Strict Clippy passes.
- [x] All-feature workspace tests pass.
- [ ] Dependency advisory/license/source policy passes.
- [x] npm wrapper smoke test passes.
- [x] Isolated offline E2E passes with the current config/session schemas.
- [x] Release-policy and secret scans pass.
- [x] Package dry-run contains only intended files.
- [x] Packed tarball installs through `AXIOM_AGENT_BINARY_PATH` and its global
      `axiom` shim executes on Node 20.
- [x] npm dist-tag policy accepts only beta-to-beta, rc-to-rc, and
      stable-to-latest mappings.
- [x] Locked release build completes.

Local results are necessary but not sufficient for an RC decision.

### Working-tree validation snapshot

On 2026-07-22, formatting, locked workspace check, strict Clippy, all-feature
workspace tests (352 passed), locked release build, Node smoke, isolated E2E,
release/security/version checks, dist-tag and publish-readiness self-tests,
packed global-install smoke, and the 16-file npm package dry-run passed on the
Windows development working tree. `cargo-deny` was not available locally, so
the dependency advisory/license/source gate remains open for CI or a release
machine with that pinned tool installed. This is informational pre-RC evidence,
not candidate evidence: the tree was still changing, was not a clean tagged
commit, and cross-platform/external/operator gates were not run.

## Compatibility and recovery

- [ ] Fresh onboarding writes the current config schema and `axiom doctor --json`
      reports no migration requirement.
- [ ] Upgrade a real copy of the latest beta config; confirm a versioned backup
      and preserved providers, models, policy defaults, workspace, and Proof
      settings.
- [ ] Unknown future config/session/proof versions fail closed with actionable
      diagnostics.
- [ ] Resume after cancellation and after a completed tool result does not
      repeat a non-idempotent side effect.
- [ ] Pre-write checkpoint creation is a hard barrier; `!checkpoints` and
      `!restore ID` recover the expected workspace state.
- [ ] Core updater install/rollback passes against isolated release fixtures.

## Supported platform matrix

Each row requires a clean CI result plus an installed-binary smoke test. A
source build on the maintainer's machine does not satisfy another row.

| Target | Unit/E2E | npm wrapper | Credential store | Terminal UX | Status |
| --- | --- | --- | --- | --- | --- |
| Windows x86-64 MSVC | Pending | Pending | Pending | Pending | Blocked |
| Linux x86-64 glibc 2.35+ | Pending | Pending | Pending | Pending | Blocked |
| macOS x86-64 | Pending | Pending | Pending | Pending | Blocked |
| macOS Apple silicon | Pending | Pending | Pending | Pending | Blocked |

- [ ] CI test matrix is green on Windows, Linux, and macOS from the candidate
      commit.
- [ ] Both macOS release architectures are built and smoke-tested.
- [ ] The Linux x86-64 release is built on Ubuntu 22.04 and verified on the
      documented glibc 2.35 compatibility floor.
- [ ] `NO_COLOR`, `ui.theme = "none"`, redirected stdout, redirected stdin,
      narrow terminal, bracketed paste, multiline input, and input history are
      manually checked without losing text meaning.
- [ ] Screen-reader/plain-output review finds no color-only status, risk, or
      error information.

## Provider and model setup

- [ ] Native credential save/reload works on Windows Credential Manager,
      macOS Keychain, and the supported Linux secret service.
- [ ] Headless credential-store failure falls back to documented environment
      variables without writing the secret to config, Proof, session, or logs.
- [ ] Groq, OpenRouter, Gemini, GitHub Models, NVIDIA NIM, and OpenAI catalog
      discovery are smoke-tested without sending an inference prompt; Cloudflare manual model
      selection is smoke-tested without making any network request.
- [ ] Ollama, LM Studio, and a custom OpenAI-compatible endpoint complete
      no-key onboarding where configured.
- [ ] Empty/malformed catalogs, 401, 429/retry exhaustion, timeout, and
      oversized response errors are understandable and recoverable.
- [ ] Provider documentation avoids guarantees about free tiers or model
      availability.

## Runtime, Coder, and policy acceptance

- [ ] Native and fallback tool-call loops complete a multi-tool task and stop
      at every configured cap.
- [ ] `axiom cost` reports the UTC month, per-session spend, configured budgets,
      pricing availability, and local ledger path; complete pricing enforces
      session/monthly limits before the next provider call.
- [ ] Missing or partial token pricing records no invented cost and clearly
      reports that persistent budget enforcement/new recording is unavailable.
- [ ] Fragmented SSE tool/todo control blocks never leak into live assistant
      text while the exact accumulated response still parses.
- [ ] Filesystem, network, process, and Git `allow`/`ask`/`deny` decisions match
      config and appear in the redacted Proof/session record.
- [ ] `web.fetch` defaults to HTTPS with system proxy use disabled; exact and
      `*.domain` host patterns match as documented, deny wins over allow, and
      redirects remain disabled.
- [ ] Empty/non-empty allowlists, explicit denies, HTTP opt-in, and system-proxy
      opt-in cannot reach localhost, `.local`, loopback, private/reserved, or
      public names resolving to blocked addresses.
- [ ] Coder plan-to-patch scope, per-hunk approval, conflict handling, patch
      caps, checkpoint rollback, and bounded correction attempts pass.
- [ ] Real Rust, Node, Python, Go, Maven/Gradle, and representative monorepo
      projects choose only documented test commands.
- [ ] External executable skill entrypoints remain disabled/quarantined.

## Security review

- [ ] Review the version-bound [repository threat model](THREAT_MODEL.md) and
      confirm its assumptions still match the candidate snapshot.
- [ ] Threat boundaries cover provider data, web content, registry metadata,
      workspace files/diffs, tool output, policy decisions, credentials,
      updates, npm installation, and Proof export.
- [ ] No unresolved critical/high issue affects workspace containment, secret
      handling, arbitrary execution, update integrity, or recovery.
- [ ] Malformed-input/property corpora and the dedicated long-running fuzz lane
      complete for patch, tool request, SSE, registry/manifest, migration,
      path, and redaction parsers.
- [ ] Unix private state modes and Windows/macOS access behavior are verified on
      real release targets.
- [ ] Security owner records explicit approval below.

## Tagged artifacts and provenance

- [ ] Push the tag only after every pre-tag item above is complete.
- [ ] Tag exactly matches synchronized package version and changelog heading.
- [ ] GitHub Release contains all four expected binaries, `SHA256SUMS`, and
      `axiom-agent.spdx.json`.
- [ ] Each matrix runner completed isolated E2E against its exact native
      release binary before artifact upload.
- [ ] The RC is marked as a GitHub prerelease and is not marked latest.
- [ ] Recompute every SHA-256 checksum from downloaded artifacts.
- [ ] Verify every binary with `gh attestation verify --repo
      NexaraAI/axiom-agent`.
- [ ] Verify the SBOM attestation and inspect the SPDX document.
- [ ] Install each downloaded binary and run `axiom --version`, `axiom doctor`,
      offline onboarding, one-shot chat, session resume, and updater status.
- [ ] Installer policy tests reject HTTP, credentials in URLs, untrusted
      redirect hosts, redirect overflow, oversized binary/checksum streams, and
      checksum mismatch without leaving a partial destination.
- [ ] Record release workflow URL and immutable artifact evidence below.

## npm RC publishing

- [ ] npm package settings register `NexaraAI/axiom-agent` and
      `npm-publish.yml` as the trusted publisher.
- [ ] GitHub environment `npm-publish` requires release-owner approval and
      allows only reviewed release tags.
- [ ] Publishing access requires 2FA and disallows long-lived token publishing.
- [ ] Run the workflow once with `publish=false`; inspect `npm pack --dry-run`.
- [ ] After GitHub artifact verification, deliberately run with `publish=true`
      and dist-tag `rc`—never `latest` for an RC.
- [ ] Confirm the npm workflow checks out exactly `v<package version>`, binds to
      the matching non-draft GitHub Release, and sees all four binaries plus
      `SHA256SUMS`.
- [ ] Confirm the workflow rejects an npm version that already exists. The next
      publish must use a unique prerelease; `0.5.1-beta` must not be reused.
- [ ] Verify npm provenance, repository metadata, package file list, and the
      resolved binary checksum from a clean machine.
- [ ] Confirm the workflow's version/dist-tag guard rejects RC-to-`latest`,
      stable-to-`rc`, and beta-to-`rc` mismatches before `npm publish`.
- [ ] Confirm `npm install -g axiom-agent@rc` installs the candidate on each
      supported target before announcing it.

## Sign-off and evidence

| Gate | Owner | Evidence URL/SHA | Decision |
| --- | --- | --- | --- |
| Engineering | Pending | Pending | Not approved |
| Security | Pending | Pending | Not approved |
| Accessibility/UX | Pending | Pending | Not approved |
| Release/provenance | Pending | Pending | Not approved |

The release owner changes the current decision only after every mandatory item
has evidence. A failed publish, missing artifact, checksum mismatch,
attestation failure, credential leak, workspace escape, or data-loss regression
is an immediate stop-and-rollback condition.
