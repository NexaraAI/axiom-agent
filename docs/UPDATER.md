# Core Updater

The core updater replaces the Axiom Rust binary. Skill updates are separate (see `SKILL_REGISTRY.md`).

- Core update: replaces the `axiom` executable after checksum verification.
- Skill update: updates installed skill manifests from the skill registry.

The updater has release-ready plumbing. It will become active after tagged GitHub Releases are published for this repo.

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

Axiom also stores cached fields in config: `last_checked_at`, `last_available_version`, and `last_update_error`.

## Channels

- `stable`: uses non-prerelease GitHub Releases.
- `nightly`: can use prereleases.
- `dev`: local testing channel. Reads mocked release metadata from a local JSON file or directory.

## Policies

- `manual`: check only when you run `axiom update check`.
- `notify`: check on explicit command and show cached notices on startup.
- `auto-patch`: apply patch updates without extra confirmation during explicit update install flow. Minor and major updates still require confirmation.

Normal chat startup makes no network calls. It reads cached update info and may print one short notice.

## Safety

Axiom accepts only an exact `https://github.com/<owner>/<repo>` release
repository. HTTP, credentials, ports, query strings, fragments, extra path
segments, and invalid GitHub owner/repository names are rejected. Local release
metadata is accepted only on the `dev` channel.

Release metadata requests ignore system proxies, do not follow redirects, and
stream at most 4 MiB. Asset downloads ignore system proxies and permit only the
exact reviewed hosts `github.com`, `objects.githubusercontent.com`, and
`release-assets.githubusercontent.com`. Every redirect is revalidated, DNS
resolved, checked for private/reserved addresses, and pinned before the next
request. The entire download has a 60-second deadline and at most five
redirects. Binaries are capped at 256 MiB; `SHA256SUMS` is capped separately at
1 MiB.

The complete checksum manifest is parsed before trust is granted. Malformed
hashes or lines, duplicate entries, unsafe names, and inexact asset matches fail
closed. For network updates, both initial asset URLs must exactly match the
configured owner/repository, selected release tag, and expected asset name;
redirect targets are then constrained by the transport rules above. Local dev
fixtures support metadata checks only and cannot install a binary. The selected
binary is verified before staging or installing.

A missing checksum blocks installation. A mismatched checksum fails the install, and Axiom does not attempt binary replacement.

Axiom does not download or run release scripts. Releases contain prebuilt binaries and checksum metadata.

## Install Modes

- `cargo-dev`: running from `target/debug` or `target/release`. Axiom blocks self-install.
- `npm-global`: running from the npm package vendor binary. Axiom tries to update in place if permissions allow.
- `standalone`: direct binary path. Axiom can replace it if writable.
- `unknown`: check works; Axiom handles install errors by preserving the current state.

For Cargo builds, Axiom prints:

```text
Axiom is running from a Cargo build, so it will not replace this binary.
```

## Staging And Rollback

Axiom stores updater files under the user config directory:

```text
updates/
  downloads/
  staged/
  backups/
  update-state.json
```

If `AXIOM_HOME` is set, this tree goes under `$AXIOM_HOME/updates`. Integration tests use that path so updater status checks do not touch real user config.

Install flow:

1. Download the binary and `SHA256SUMS` with the transport limits above.
2. Reject a missing, oversized, or mismatched checksum record.
3. Verify checksum.
4. Write the verified binary atomically to `staged/`.
5. Back up the current binary atomically under `backups/`.
6. Replace the current binary atomically if possible.
7. Require `--version` to report the exact selected release version.

Any execution failure or version mismatch fails verification. Axiom attempts to
restore the previous binary backup and reports both errors if rollback also
fails. Configured provider credential names are removed from the candidate
binary's verification environment. `axiom update rollback` can also restore
that backup explicitly.

On Windows, a running executable may be locked. If replacement fails, Axiom keeps the verified staged binary and reports a pending update instead of leaving a partial install.

## Release Checks

Before tagging or publishing, run:

```bash
node scripts/release-check.js
node scripts/security-check.js
```

These checks do not create releases and do not publish to npm. They verify repository metadata, workflows, docs, tracked-file safety, and obvious secret leaks.
