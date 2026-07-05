# v0.5.0-beta Release Checklist

Use this checklist before creating a `v0.5.0-beta` release tag. Do not publish to npm until you confirm the release assets and checksums.

## Pre-Release Checks

Run from the repo root:

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

The E2E test must use a temporary `AXIOM_HOME`, the mock provider, and the local skill registry fixture. It must not require API keys or make real network calls.

## Repo Checks

- `main` branch is clean.
- CI is green for the commit you plan to tag.
- `README.md` is accurate.
- `LICENSE` is present.
- No secrets are committed.
- No built binaries are committed.
- No `target/`, `node_modules/`, proof logs, `.env` files, or local test output are tracked.
- The skills repo is reachable: `https://github.com/NexaraAI/axiom-skills`.
- The default registry URL is `https://raw.githubusercontent.com/NexaraAI/axiom-skills/main/registry.json`.
- Bundled registry fixtures remain in this repo for tests and offline fallback.

## Tagging

Tag only after the checks above pass:

```bash
git tag v0.5.0-beta
git push origin v0.5.0-beta
```

## After Tag

1. Watch the GitHub Actions release workflow.
2. Verify release assets for Windows, Linux, Intel macOS, and Apple Silicon macOS.
3. Verify `SHA256SUMS`.
4. Test npm local install using the release binary.
5. Publish to npm after you confirm release assets.

## Rollback

If the release fails before npm publish:

1. Delete the failed tag if needed.
2. Delete the failed GitHub Release if one was created.
3. Fix the issue on `main`.
4. Rerun the pre-release checks.
5. Create a fresh tag after you verify the fix.

Do not publish to npm until you confirm release assets and checksums.
