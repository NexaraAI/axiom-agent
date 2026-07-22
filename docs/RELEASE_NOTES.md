# Axiom 1.0 RC1

Axiom 1.0 RC1 is ready for public testing on Windows, Linux, and macOS.

This release brings the full terminal workflow together: friendly provider
setup, direct chat, project editing with review, resumable sessions, and a
clear proof trail for tool calls, patches, tests, and results.

## Install

```bash
npm install -g axiom-agent@rc
axiom
```

The first launch walks through provider and model setup. Hosted providers,
local Ollama or LM Studio, and custom OpenAI-compatible endpoints are supported.

## Highlights

- Work with Axiom directly in the terminal; no messaging channel is required.
- Review tool use and file changes before they happen.
- Recover project files from checkpoints when a coding run needs to be undone.
- Resume interrupted conversations without replaying completed tool actions.
- Switch providers and models from the command line or during a chat.
- Export readable proof reports with sensitive values removed.
- Install verified native binaries through the npm wrapper.

## Release-candidate note

This is a prerelease for final field testing, not the stable `latest` build.
Please report rough edges through GitHub Issues. Each attached binary has a
SHA-256 checksum, an SPDX SBOM, and GitHub build provenance.
