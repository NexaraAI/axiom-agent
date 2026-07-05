# Offline Demo

Run this demo without API keys using the mock provider.

## Setup

Set up an isolated config root:

```bash
mkdir -p /tmp/axiom-demo
AXIOM_HOME=/tmp/axiom-demo/home cargo run -p axiom-cli -- onboarding --non-interactive --provider mock --workspace /tmp/axiom-demo/workspace --yes
```

Windows PowerShell:

```powershell
$env:AXIOM_HOME = "$env:TEMP\axiom-demo-home"
cargo run -p axiom-cli -- onboarding --non-interactive --provider mock --workspace "$env:TEMP\axiom-demo-workspace" --yes
```

This creates config, workspace, and starter skills. The `mock` provider returns deterministic responses for tests and demos.

## Run Chat Once

Create a README in the demo workspace, then ask Axiom to read it:

```bash
echo "# Demo" > /tmp/axiom-demo/workspace/README.md
AXIOM_HOME=/tmp/axiom-demo/home cargo run -p axiom-cli -- run "read README.md and summarize it"
```

Expected behavior:

- Skill Lens selects relevant file/project skills.
- The mock provider requests `file.read`.
- Axiom Engine reads `README.md` inside the workspace.
- The mock provider returns a deterministic summary.
- Proof Mode records the trace under `AXIOM_HOME/proofs`.

## Run Coder Plan-Only

```bash
AXIOM_HOME=/tmp/axiom-demo/home cargo run -p axiom-cli -- code --plan-only "add a small test"
```

The mock provider returns a short plan. No files change.

## Inspect State

```bash
AXIOM_HOME=/tmp/axiom-demo/home cargo run -p axiom-cli -- skill installed
AXIOM_HOME=/tmp/axiom-demo/home cargo run -p axiom-cli -- proof list
AXIOM_HOME=/tmp/axiom-demo/home cargo run -p axiom-cli -- update status
```

You can also install via `npm install -g axiom-agent@beta` and run these commands with `axiom` instead of `cargo run`.
