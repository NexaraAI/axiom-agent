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
