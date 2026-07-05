# Installation

## From npm (recommended)

```bash
npm install -g axiom-agent@beta
axiom
```

The npm package is a thin installer and wrapper. It detects your OS and architecture, downloads the matching prebuilt Rust binary from GitHub Releases, verifies `SHA256SUMS`, stores the binary under `vendor/bin/`, and exposes the `axiom` command through `bin/axiom.js`.

Axiom itself is Rust. Node.js handles installation and command forwarding only.

## From Source

```bash
cargo build -p axiom-cli
cargo run -p axiom-cli -- doctor
```

## First Run

After installation or a source build:

```bash
axiom
```

If no config exists, onboarding starts. Once onboarding finishes, `axiom` opens terminal chat.

For non-interactive setup:

```bash
axiom onboarding --non-interactive --provider mock --workspace ./demo-workspace --yes
axiom onboarding --non-interactive --skip-provider --workspace ./demo-workspace --yes
```

`--provider mock` creates an offline demo config with no API keys. `--provider openai-compatible` and `--provider cloudflare` require `--model`. Use `--registry <url-or-path>` to pin the skills registry during setup.

## Test-Safe Config

Set `AXIOM_HOME` to isolate config writes:

```bash
AXIOM_HOME=/tmp/axiom-test-home axiom doctor
```

Axiom stores config and runtime state under that directory when set:

```text
config.toml
skills/
  installed_skills.json
proofs/
updates/
registry-cache/
```

Without `AXIOM_HOME`, Axiom uses the platform config directory.

## Local Development Install

Set `AXIOM_AGENT_BINARY_PATH` to skip GitHub downloads during development.

Windows PowerShell:

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

Name the Rust binary `axiom` on Linux/macOS and `axiom.exe` on Windows. The Cargo config already handles this.

## Release Repository Configuration

`package.json` points at the release repository:

```text
https://github.com/NexaraAI/axiom-agent
```

To test alternate release locations, override it without editing package metadata:

```bash
AXIOM_AGENT_RELEASE_REPO=https://github.com/example/axiom-agent npm install -g axiom-agent
```

## Supported Binary Assets

- `axiom-x86_64-pc-windows-msvc.exe`
- `axiom-x86_64-unknown-linux-gnu`
- `axiom-x86_64-apple-darwin`
- `axiom-aarch64-apple-darwin`

The installer fails with a clear error on unsupported platforms.

## In-Place Updates

After installing from a release binary, the updater can check and stage binary updates:

```bash
axiom update status
axiom update check
axiom update install
```

The updater uses the same release asset names as the npm installer and verifies `SHA256SUMS` before replacing a binary. Cargo builds support checks, but `install` is blocked because self-replacing `target/debug` or `target/release` builds is not a supported install mode.

For npm-global installs, Axiom tries to update the `vendor/bin` binary if permissions allow. If the package location is read-only, reinstall with npm after a new release.
