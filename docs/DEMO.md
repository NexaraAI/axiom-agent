# Offline Demo

This demo works before npm is published and does not require API keys.

## Setup

Use an isolated config root:

```bash
mkdir -p /tmp/axiom-demo
AXIOM_HOME=/tmp/axiom-demo/home cargo run -p axiom-cli -- onboarding --non-interactive --provider mock --workspace /tmp/axiom-demo/workspace --yes
```

Windows PowerShell:

```powershell
$env:AXIOM_HOME = "$env:TEMP\axiom-demo-home"
cargo run -p axiom-cli -- onboarding --non-interactive --provider mock --workspace "$env:TEMP\axiom-demo-workspace" --yes
```

The setup creates config, workspace, and essential skills. The provider is `mock`, which is deterministic and for tests and demos only.

## Run Chat Once

Create a README in the demo workspace, then ask Axiom to read it:

```bash
echo "# Demo" > /tmp/axiom-demo/workspace/README.md
AXIOM_HOME=/tmp/axiom-demo/home cargo run -p axiom-cli -- run "read README.md and summarize it"
```

Expected behavior:

- Skill Lens selects relevant file/project skills.
- The mock provider asks for `file.read`.
- Axiom Engine reads `README.md` inside the workspace.
- The mock provider returns a final deterministic summary.
- Proof Mode records the trace under `AXIOM_HOME/proofs`.

## Run Coder Plan-Only

```bash
AXIOM_HOME=/tmp/axiom-demo/home cargo run -p axiom-cli -- code --plan-only "add a small test"
```

The mock provider returns a short plan. No files are changed.

## Inspect State

```bash
AXIOM_HOME=/tmp/axiom-demo/home cargo run -p axiom-cli -- skill installed
AXIOM_HOME=/tmp/axiom-demo/home cargo run -p axiom-cli -- proof list
AXIOM_HOME=/tmp/axiom-demo/home cargo run -p axiom-cli -- update status
```

This is the safest way to demo Axiom before npm publishing and before public release assets exist.
