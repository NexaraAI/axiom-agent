# Skill Lens

Axiom Skill Lens analyzes each user message and selects a small set of relevant installed skill cards.

This saves tokens and improves weak model performance by avoiding full skill-library injection.

Current rule-based signals:

- Python, `.py`, or script requests select `python.write` and `python.run` when installed.
- URL, website, fetch, search, or web requests select `web.fetch`.
- File read/write/save requests select `file.read` and `file.write`.
- Git status or diff requests select `git.status` and `git.diff`.
- Run, test, command, terminal, or shell requests select platform shell-safe skills.

Chat injects selected cards as compact system context before the user message. Actual tool execution is a later stage.

## Lifecycle Filtering

Skill Lens only considers installed skills that are enabled, compatible with the current platform and Axiom version, and not blocked by trust policy.

It ignores skills in these states:

- `disabled`
- `incompatible`
- `quarantined`

Blocked skills are also ignored. A skill with `update_available` can still be selected until the user updates or disables it.
