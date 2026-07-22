# Coding Agent

`axiom code` is Axiom's interactive coding workflow. It scans the selected
workspace, asks the active model for a bounded plan, lets the user apply, revise,
or cancel that plan, previews constrained patches, checkpoints the workspace,
and runs detected project tests through the shared side-effect policy.

The implementation lives in `axiom-coder` with its terminal orchestration in
`axiom-cli`. Writes remain workspace-scoped, secret-looking paths are blocked,
existing-file edits require base hashes, large scopes require confirmation, and
Proof Mode records plans, approvals, patches, commands, tests, and recovery
attempts.

See [CODING_MODE.md](CODING_MODE.md) for commands and safety behavior.
