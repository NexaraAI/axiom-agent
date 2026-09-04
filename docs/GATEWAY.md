# Messaging Gateway (Telegram / Discord)

Chat with Axiom from your phone through a Telegram or Discord bot.
The bots use the same active provider/model as the terminal.

## Status: early setup (tokens only)

`axiom onboarding` (bonus step), `axiom gateway setup`, and
`axiom gateway status` collect and inspect bot tokens today.
The live bot runner is still being built: nothing connects yet,
and `axiom doctor` reports tokens as "saved (bot runner pending)".

## Setup

```bash
axiom gateway setup      # Telegram and/or Discord tokens
axiom gateway status     # what the bots will use
axiom gateway disable --telegram   # forget a token (also wipes it locally)
```

Tokens resolve like provider keys: env var, OS keychain, then the private
local fallback file. They are redacted from proofs/sessions and scrubbed
from child processes.

Restrict who can talk to your bot: during setup, enter allowed chat
(Telegram) or server (Discord) IDs. An empty allowlist means "decide later" —
do not share the bot link until IDs are set.

## Model and provider control

Bots follow the global active provider/model:

```bash
axiom provider use <name>
axiom model use <id>
axiom model list --filter <text>   # live provider catalog
```

Bot-side command contract (goes live with the runner):

| Command | Effect |
|---|---|
| `/start`, `/help` | Greeting + command list, no tools |
| `/status` | Active provider/model, no tools |
| `/models [filter]` | Live catalog search (max 25 shown) |
| `/model <id>` | Switch model (same validation as `axiom model use`) |
| `/provider <name>` | Switch provider, restores its saved model |

Rules the runner must enforce:

- Unknown chat IDs are ignored silently when an allowlist is set.
- `/model` and `/provider` accept exact IDs only (no fuzzy guessing that could
  switch you onto a paid model by accident).
- File writes and network fetches from bot sessions follow the same
  allow/ask/deny side-effect policy as terminal chat, and every turn is
  recorded by Proof Mode.
- Bot tokens never appear in logs, errors, or replies.
