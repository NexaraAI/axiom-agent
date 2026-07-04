# Core Updater

The core updater updates the Axiom Rust binary. It is separate from skill updates.

- Core update: replaces the `axiom` executable after checksum verification.
- Skill update: updates installed skill manifests from the skill registry.

Stage 12 adds release-ready plumbing. It will become active for normal users after tagged GitHub Releases are published for this repo.

## Commands

```powershell
axiom update status
axiom update check
axiom update install
axiom update rollback
axiom update set-channel stable
axiom update set-channel nightly
axiom update set-channel dev
axiom update set-policy manual
axiom update set-policy notify
axiom update set-policy auto-patch
```

## Config

```toml
[update]
channel = "stable"
policy = "notify"
release_repo = "https://github.com/NexaraAI/axiom-agent"
check_interval_hours = 24
allow_prerelease = false
backup_previous_binary = true
verify_checksums = true
```

Cached fields such as `last_checked_at`, `last_available_version`, and `last_update_error` are also stored in config.

## Channels

- `stable`: uses non-prerelease GitHub Releases.
- `nightly`: can use prereleases.
- `dev`: local testing channel. It can read mocked release metadata from a local JSON file or directory.

## Policies

- `manual`: only check when the user runs `axiom update check`.
- `notify`: check on explicit command and show cached notices on startup.
- `auto-patch`: allows patch updates without an extra confirmation during explicit update install flow. Minor and major updates still require confirmation.

Normal chat startup does not make network calls. It only reads cached update information and may print one short notice.

## Safety

Axiom downloads a release asset and `SHA256SUMS`, finds the matching checksum line, and verifies the binary before staging or installing it.

If the checksum is missing, installation is blocked. If the checksum mismatches, installation fails and no binary replacement is attempted.

Axiom does not download or run release scripts. Releases contain prebuilt binaries and checksum metadata only.

## Install Modes

- `cargo-dev`: running from `target/debug` or `target/release`. Install is blocked.
- `npm-global`: running from the npm package vendor binary. Axiom tries to update in place if permissions allow.
- `standalone`: direct binary path. Axiom can replace it if writable.
- `unknown`: check works; install handles errors conservatively.

For Cargo builds, use:

```text
Axiom is running from a Cargo build, so it will not replace this binary.
```

## Staging And Rollback

Updater files live under the user config directory:

```text
updates/
  downloads/
  staged/
  backups/
  update-state.json
```

If `AXIOM_HOME` is set, this tree is written under `$AXIOM_HOME/updates`. Integration tests use that path so updater status checks do not touch real user config.

Install flow:

1. Download binary to `downloads/`.
2. Download `SHA256SUMS`.
3. Verify checksum.
4. Copy the verified binary to `staged/`.
5. Back up the current binary under `backups/`.
6. Replace the current binary if possible.
7. Run a post-install version check where possible.

`axiom update rollback` restores the previous binary backup if one exists.

On Windows, a running executable may be locked. If replacement fails, Axiom keeps the verified staged binary and reports a pending update instead of leaving a partial install.

## Release Checks

Before tagging or publishing, run:

```bash
node scripts/release-check.js
node scripts/security-check.js
```

These checks do not create releases and do not publish npm. They verify repository metadata, workflows, docs, tracked-file safety, and obvious secret leaks.
