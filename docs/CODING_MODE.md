# Coding Mode

Axiom Coder is the terminal project-coding mode for Axiom Agent. Normal chat answers general questions and simple code snippets. Coder mode is for workspace-aware tasks such as fixing build errors, editing project files, scanning the repository, explaining the codebase, or running tests.

## Commands

```powershell
axiom code
axiom code "fix cargo error"
axiom code --plan-only "add a README section explaining installation"
axiom code --scan
axiom code --diff
axiom code --apply "create a simple Python hello world script"
axiom code --test
axiom code --explain
```

For offline plan demos:

```powershell
axiom onboarding --non-interactive --provider mock --workspace .\demo-workspace --yes
axiom code --plan-only "add a small test"
```

The `mock` provider returns a deterministic plan. It is for tests and demos only and does not require API keys.

Inside `axiom code`:

```text
!help
!exit
!scan
!plan TASK
!apply TASK
!diff
!test
!explain
!model current
!model use MODEL
!provider current
!provider list
!provider use PROVIDER
!skills
!clear
```

## Chat Auto-Routing

When normal chat sees a project-level coding request, Axiom Lens can route into coder mode.

Default config:

```toml
[coder]
auto_route_from_chat = true
auto_route_mode = "ask"
approval_mode = "safe"
```

Modes:

- `off`: never route from chat.
- `ask`: detect project coding tasks and ask before switching.
- `smart`: switch automatically for obvious project coding tasks and ask for ambiguous ones.

Simple code generation, regex help, language examples, and conceptual questions stay in normal chat.

## Project Scan

Coder mode scans the active workspace and detects:

- Rust: `Cargo.toml`
- Node: `package.json`
- Python: `pyproject.toml`, `requirements.txt`, `setup.py`
- Java: `pom.xml`, `build.gradle`
- Generic: fallback

It ignores `.git`, `node_modules`, `target`, `dist`, `build`, `.venv`, `venv`, `__pycache__`, `.next`, and `.cache`.

## Patch Format

Provider output for edits must use:

````text
```axiom-patch
{
  "summary": "short summary",
  "test_command": "cargo test",
  "changes": [
    {
      "path": "relative/path.txt",
      "action": "create_or_update",
      "content": "full new file content"
    }
  ]
}
```
````

v0.1 supports full-file `create_or_update` only. Complex partial patches are intentionally deferred.

## Safety Model

Axiom Coder must:

- stay inside the active workspace;
- block `.env`, `.env.*`, `*.pem`, `*.key`, `id_rsa`, `id_dsa`, `credentials.json`, and `token.json`;
- show a plan before changes;
- show a diff before writes;
- ask before file writes;
- ask before test commands.

Axiom Coder must not:

- delete files;
- run arbitrary shell commands;
- install packages without confirmation;
- push to remote git;
- deploy;
- hide changes.

Even `trusted` approval mode asks before writes in v0.1.

Coder plan-only, apply, and test flows create Proof Mode reports when proof is enabled.

## Test Commands

Safe test detection currently supports:

- Rust: `cargo test`
- Node: `npm test`, `pnpm test`, `yarn test`
- Python: `python -m pytest`, `pytest`

Commands are never run automatically.

## Proof Reports

When Proof Mode is enabled, coder tasks record project scan context, selected Skill Lens cards, plan text, patch summary, diff, approval decisions, files written through Axiom Engine, detected test commands, command results, and the final result or recoverable error.

Reports are stored under the user config proofs directory and can be inspected with:

```powershell
axiom proof latest
axiom proof show latest
axiom proof export latest --format json
```
