# Coding Mode

Axiom Coder is the terminal project-coding mode for Axiom Agent. Normal chat answers general questions and simple code snippets. Use coder mode for workspace-aware tasks: fixing build errors, editing project files, scanning the repository, explaining the codebase, or running tests.

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

The `mock` provider returns a deterministic plan. It exists for tests and demos only and requires no API keys.

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
- `smart`: switch for obvious project coding tasks, ask for ambiguous ones.

Simple code generation, regex help, language examples, and conceptual questions stay in normal chat.

## Project Scan

Coder mode scans the active workspace and detects:

- Rust: `Cargo.toml`
- Node: `package.json`, plus pnpm and Yarn lockfiles
- Python: `pyproject.toml`, `requirements.txt`, `setup.py`, `pytest.ini`
- Go: `go.mod`
- Java/JVM: `pom.xml`, `build.gradle`, `build.gradle.kts`
- Deno and Bun: `deno.json`, `deno.jsonc`, `bun.lockb`
- Generic: fallback

Detection searches bounded workspace depth and recognizes nested packages, so a
monorepo can receive scoped test commands without treating every package as the
repository root.

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
      "base_sha256": "64-character SHA-256 of the file that was inspected",
      "hunks": [
        {
          "old_start": 3,
          "old_lines": ["old line"],
          "new_lines": ["replacement line"]
        }
      ]
    }
  ]
}
```
````

New files may use `content` instead of `hunks` and omit `base_sha256`. Existing
files require the observed base hash and exactly one edit form (`content` or
`hunks`). Hunks are one-based, ordered, non-overlapping, and bounded. Axiom
checks the hash and hunk context during preview and checks the file again before
writing; concurrent edits become visible conflicts instead of being overwritten.

The patch is reviewed as constrained file units. Large scopes require an extra
confirmation, every accepted write is preceded by a recovery checkpoint, and a
rejected unit does not silently approve the rest. After tests fail, the model
receives bounded failure context and may propose a limited number of corrective
patches; each correction goes through the same preview, policy, approval,
checkpoint, and conflict checks.

## Safety Model

Axiom Coder must:

- stay inside the active workspace;
- block `.env`, `.env.*`, `*.pem`, `*.key`, `id_rsa`, `id_dsa`, `credentials.json`, and `token.json`;
- show a plan that can be applied, revised, or cancelled before changes;
- show a diff before writes;
- apply the configured centralized policy before file writes and test commands;
- preserve a checkpoint before an approved write.

Axiom Coder must not:

- delete files;
- run arbitrary shell commands;
- install packages without confirmation;
- push to remote git;
- deploy;
- hide changes.

Approval modes tune prompts but never bypass workspace, secret-path, patch,
policy, conflict, checkpoint, or runtime limits.

Coder plan-only, apply, and test flows create Proof Mode reports when proof is enabled.

## Test Commands

Safe test detection supports:

- Rust: `cargo test`
- Node: `npm test`, `pnpm test`, `yarn test`
- Python: `python -m pytest`, `pytest`
- Go: `go test ./...`
- Maven/Gradle: `mvn test`, `gradle test`
- Deno/Bun: `deno test`, `bun test`

Commands use a fixed safe-command allowlist, run in the detected package
directory, inherit no configured provider credential variables, and follow the
central process policy. A model-supplied arbitrary shell command is not run.

## Proof Reports

When Proof Mode is enabled, coder tasks record project scan context, selected Skill Lens cards, plan text, patch summary, diff, approval decisions, files written through Axiom Engine, detected test commands, command results, and the final result or recoverable error.

Axiom stores reports under the user config proofs directory. You can inspect them with:

```powershell
axiom proof latest
axiom proof show latest
axiom proof export latest --format json
```
