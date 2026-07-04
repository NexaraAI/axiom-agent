# Installation

The first public install method for Axiom Agent is npm:

```bash
npm install -g axiom-agent
axiom
```

The npm package is a thin installer and wrapper. It detects the current OS and architecture, downloads the matching prebuilt Rust binary from GitHub Releases, verifies `SHA256SUMS`, stores the binary in the installed package under `vendor/bin/`, and exposes the `axiom` command through `bin/axiom.js`.

Axiom itself remains Rust. Node.js is only used for installation and command forwarding.

## First Run

After installation:

```bash
axiom
```

If no config exists, onboarding starts. After onboarding is complete, `axiom` opens terminal chat.

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
