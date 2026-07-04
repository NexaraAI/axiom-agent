# Installation

Axiom Agent is currently installed from source for normal development and testing. The npm installer scaffold exists, but the package is not published yet.

Source install:

```bash
cargo build -p axiom-cli
cargo run -p axiom-cli -- doctor
```

After npm is published, the intended install command is:

```bash
npm install -g axiom-agent
axiom
```

The npm package is a thin installer and wrapper. It detects the current OS and architecture, downloads the matching prebuilt Rust binary from GitHub Releases, verifies `SHA256SUMS`, stores the binary in the installed package under `vendor/bin/`, and exposes the `axiom` command through `bin/axiom.js`.

Axiom itself remains Rust. Node.js is only used for installation and command forwarding.

## First Run

After installation or a source build:

```bash
axiom
```

If no config exists, onboarding starts. After onboarding is complete, `axiom` opens terminal chat.

For non-interactive setup:

```bash
axiom onboarding --non-interactive --provider mock --workspace ./demo-workspace --yes
axiom onboarding --non-interactive --skip-provider --workspace ./demo-workspace --yes
```

`--provider mock` creates an offline demo config with no API keys. `--provider openai-compatible` and `--provider cloudflare` require `--model`. Use `--registry <url-or-path>` to pin the skills registry used by setup.

## Test-Safe Config

Set `AXIOM_HOME` to isolate config writes:

```bash
AXIOM_HOME=/tmp/axiom-test-home axiom doctor
```

When set, Axiom stores config and runtime state under that directory:

```text
config.toml
skills/
  installed_skills.json
proofs/
updates/
registry-cache/
```

If `AXIOM_HOME` is not set, Axiom uses the normal platform config directory.

## Local Development Install

Use `AXIOM_AGENT_BINARY_PATH` to avoid GitHub downloads while developing.

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

The Rust binary must be named `axiom` on Linux/macOS and `axiom.exe` on Windows. The current Cargo binary is already configured that way.

## Release Repository Placeholder

`package.json` currently contains a placeholder release repository:

```text
https://github.com/NexaraAI/axiom-agent
```

Replace it before publishing if the final repository changes. For testing, override it without editing package metadata:

```bash
AXIOM_AGENT_RELEASE_REPO=https://github.com/example/axiom-agent npm install -g axiom-agent
```

## Supported Binary Assets

- `axiom-x86_64-pc-windows-msvc.exe`
- `axiom-x86_64-unknown-linux-gnu`
- `axiom-x86_64-apple-darwin`
- `axiom-aarch64-apple-darwin`

Unsupported platforms fail with a clear installer error.

## In-Place Updates

After Axiom is installed from a release binary, the core updater can check and stage binary updates:

```bash
axiom update status
axiom update check
axiom update install
```

The updater uses the same release asset names as the npm installer and verifies `SHA256SUMS` before replacing a binary. Running from a Cargo build supports checks, but install is blocked because self-replacing `target/debug` or `target/release` builds is not a supported install mode.

For npm-global installs, Axiom tries to update the installed `vendor/bin` binary if permissions allow. If the package location is not writable, reinstall with npm after a release is available.
