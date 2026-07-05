# Skill Lens

Skill Lens analyzes each user message and picks a small set of relevant installed skill cards.

This saves tokens and helps weaker models perform better by avoiding full skill-library injection.

Rule-based signals:

- Python, `.py`, or script requests pick `python.write` and `python.run` when installed.
- URL, website, fetch, search, or web requests pick `web.fetch`.
- File read/write/save requests pick `file.read` and `file.write`.
- Git status or diff requests pick `git.status` and `git.diff`.
- Run, test, command, terminal, or shell requests pick platform shell-safe skills.

Chat injects the selected cards as compact system context before the user message. Tool execution happens in a later stage.

## Lifecycle Filtering

Skill Lens only considers installed skills that are enabled, compatible with the platform and Axiom version, and not blocked by trust policy.

It skips skills in these states:

- `disabled`
- `incompatible`
- `quarantined`

It also skips blocked skills. A skill with `update_available` can still be selected until the user updates or disables it.
