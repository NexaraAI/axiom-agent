# Security Policy

## Supported versions

Until v1.0 is released, the current `main` branch and latest beta are the
supported development targets. After v1.0, the latest stable minor release and
the current release candidate will receive security fixes.

## Reporting a vulnerability

Do not open a public issue for a suspected vulnerability involving arbitrary
code execution, workspace escape, secret exposure, update integrity, registry
trust, authentication material, or proof redaction.

Report it privately to the maintainers through the repository's configured
security-advisory channel. Include:

- affected Axiom version and operating system;
- a minimal reproducible example or proof of concept;
- impact and prerequisites;
- whether the report contains sensitive material; and
- a safe contact method for follow-up.

Maintainers will acknowledge receipt within five business days, confirm scope
or request clarification, and coordinate disclosure after a fix and release
plan exist. Please do not access data, persist changes, or disrupt systems
beyond what is necessary to demonstrate the issue safely.

## Security boundaries

- Axiom only executes built-in, installed tool IDs that pass lifecycle and
  policy checks.
- Workspace paths and secret-looking files are blocked by default.
- Side-effecting actions require configured approval and are recorded in Proof
  Mode when it is enabled.
- Model-invoked `web.fetch` requires HTTPS and ignores system proxy discovery
  by default. Exact-host and `*.domain` allow/deny patterns are evaluated with
  deny first; redirects are disabled.
- Host configuration cannot override the SSRF boundary. Localhost, `.local`,
  loopback, private/reserved addresses, and public names resolving to blocked
  addresses remain hard blocked even when HTTP or system proxy use is enabled.
- Provider endpoints are configured separately from `web.fetch`, so explicitly
  configured local inference servers do not weaken the web-tool boundary.
- Provider credentials entered interactively use hidden input and the native OS
  credential manager. Config stores only environment-variable/account labels;
  headless environments can supply credentials directly through process
  environment variables. Credential values are excluded from Proof Mode.
- Remote registries provide manifests and metadata; arbitrary external skill
  binaries remain quarantined.
- npm and binary releases are checksum-verified. The v1 workflow is configured
  to generate an SPDX SBOM and GitHub artifact attestations; a candidate is not
  approved until those outputs are verified from the tagged workflow.
- The npm installer requires HTTPS, permits redirects only among reviewed
  GitHub download hosts, caps redirects/time/bytes, writes an exclusive private
  temporary file, and installs only after checksum verification.
