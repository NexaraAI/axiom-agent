# Installation

## From npm (recommended)

```bash
npm install -g axiom-agent@rc
axiom
```

`@rc` installs the v1 release candidate. The stable `@latest` channel remains
separate until the candidate has completed its release review.

The npm package is a thin installer and wrapper. It detects your OS and architecture, downloads the matching prebuilt Rust binary from GitHub Releases, verifies `SHA256SUMS`, stores the binary under `vendor/bin/`, and exposes the `axiom` command through `bin/axiom.js`.

Installer downloads require HTTPS and an exact GitHub repository URL. Redirects
are limited to five hops and every hop must remain on the reviewed GitHub asset
hosts. Requests time out after 60 seconds; binaries are capped at 256 MiB and
checksum files at 1 MiB both by declared length and streamed bytes. Axiom writes
an exclusive private temporary file, verifies its checksum, and only then moves
it into place. Replacing an existing npm-managed binary first moves the previous
copy aside and restores it if final installation or permission setup fails.

Axiom itself is Rust. Node.js handles installation and command forwarding only.
Node.js 20 or newer is required for the npm installer and wrapper.

The Linux x86-64 prebuilt binary is produced on Ubuntu 22.04 and supports glibc
2.35 or newer. On an older glibc distribution, build Axiom from source instead
of using the prebuilt npm binary.

## From Source

```bash
cargo build -p axiom-cli
cargo run -p axiom-cli -- doctor
```

## First Run

After installation or a source build:

```bash
axiom
```

If no config exists, onboarding starts. Once onboarding finishes, `axiom` opens terminal chat.

Interactive onboarding can configure one provider or two. It explains each
required setting, accepts credentials with hidden input, uses the native OS
credential manager when available, fetches model names without sending an
inference request, and remembers a model for each provider.

For non-interactive setup:

```bash
axiom onboarding --non-interactive --provider mock --workspace ./demo-workspace --yes
axiom onboarding --non-interactive --provider openrouter --workspace ./project --yes
axiom onboarding --non-interactive --provider groq --workspace ./project --yes
axiom onboarding --non-interactive --provider nvidia --workspace ./project --yes
axiom onboarding --non-interactive --provider ollama --workspace ./project --yes
axiom onboarding --non-interactive --skip-provider --workspace ./demo-workspace --yes
```

`--provider mock` creates an offline demo config. Presets are available for `groq`, `openrouter`, `gemini`, `github-models`, `nvidia`, `ollama`, `lm-studio`, `openai`, and `cloudflare`. Local Ollama and LM Studio configs need no API key by default. OpenAI, LM Studio, and Cloudflare require `--model`; other preset models can also be overridden. See [Providers](PROVIDERS.md) for keys, endpoints, defaults, and free-tier caveats. Use `--registry <url-or-path>` to pin the skills registry during setup.

The CLI contains its complete starter registry and materializes it under
`AXIOM_HOME/bundled-registry/<generation>` when needed. Mock and skip-provider
setup use that local copy directly, while keeping the configured registry URL
unchanged; hosted-registry setup can fall back to it when the configured
registry is unavailable. Installed binaries never require the Axiom source
checkout to install the essential skills. On an existing config,
`--skip-provider` preserves the selected provider and model rather than
clearing them.

## Test-Safe Config

Set `AXIOM_HOME` to isolate config writes:

```bash
AXIOM_HOME=/tmp/axiom-test-home axiom doctor
```

Axiom stores config and runtime state under that directory when set:

```text
config.toml
skills/
  installed_skills.json
proofs/
updates/
registry-cache/
sessions/
checkpoints/
outputs/
input-history.txt
cost-ledger.json
```

Without `AXIOM_HOME`, Axiom uses the platform config directory.

## Config Compatibility

New configurations include `config_version` set to the current schema. Inspect
the exact value and compatibility with `axiom doctor --json`; older beta
configurations remain readable and can be migrated explicitly with:

```bash
axiom config migrate
```

Migration writes a versioned backup beside `config.toml` before updating it.
Configurations from a newer schema fail closed and require an Axiom update.
The current schema adds terminal theme, side-effect policy, persistent cost
budgets, and model-invoked web-fetch network settings while preserving older
provider, Coder, Proof, and workspace fields.

The default side-effect policy is:

```toml
[policy]
filesystem_read = "allow"
filesystem_write = "ask"
network = "ask"
process = "ask"
git = "ask"
```

Each value must be `allow`, `ask`, or `deny`. Keep `ask` for side effects until
you have reviewed the workspace and provider prompts.

### Cost budgets

Persistent budgets are optional and require explicit model pricing:

```toml
[agent]
max_cost_usd = 1.0
session_budget_usd = 5.0
monthly_budget_usd = 25.0
input_cost_per_million_tokens = 2.0
output_cost_per_million_tokens = 8.0
```

`max_cost_usd` caps one turn. The session and UTC-month budgets use the atomic
local `cost-ledger.json` and can block the next provider call before more spend
is incurred. Run `axiom cost` to see the current UTC month, recorded spend by
session, configured limits and remaining monthly amount, pricing availability,
and ledger path.

Input and output rates are provider/model-specific and must be configured
together. A partial pair is invalid config. With neither rate configured,
Axiom cannot estimate cost: persistent enforcement and new cost-ledger records
are unavailable, and the CLI reports that limitation instead of inventing a
price.

### `web.fetch` network controls

Defaults in a newly generated config are:

```toml
[network]
web_fetch_https_only = true
web_fetch_allowed_hosts = []
web_fetch_denied_hosts = []
web_fetch_use_system_proxy = false
```

`web_fetch_allowed_hosts` and `web_fetch_denied_hosts` accept host patterns
only, without URL syntax:

- `example.com` matches exactly `example.com`.
- `*.example.com` matches subdomains such as `docs.example.com`, but does not
  match the apex `example.com`.
- Matching is case-insensitive, and deny patterns take priority over allow
  patterns.
- An empty allowlist permits any otherwise-safe public host. A non-empty
  allowlist rejects hosts that do not match it.
- Patterns containing a scheme, port, path, credentials, or whitespace are
  invalid configuration.

HTTPS is required unless `web_fetch_https_only` is explicitly set to `false`
for a reviewed public target. Localhost, `.local`, loopback, private, reserved,
and public names resolving to blocked addresses remain hard blocked regardless
of the host lists or HTTPS setting. Redirects are always disabled.

Axiom ignores system proxy configuration for `web.fetch` by default. Set
`web_fetch_use_system_proxy = true` only after reviewing the proxy and its
effect on request confidentiality. DNS/private-address validation still runs;
the proxy setting is not an SSRF bypass.

These controls apply only to model-invoked `web.fetch`. Provider endpoints are
configured separately: remote URLs require HTTPS, plain HTTP is limited to
literal loopback hosts, embedded URL credentials are rejected, and redirects
are disabled. This preserves local Ollama and LM Studio use without permitting
cleartext remote provider traffic.

### Doctor expectations

After migration, `axiom doctor --json` should report equal
`config_schema_version` and `supported_config_schema_version` values and
`config_migration_required: false`. Loading the config validates network host
pattern syntax. Doctor deliberately does not contact allowlisted hosts or test
system-proxy reachability, so a passing mandatory-check summary is not a
network-connectivity test. Use `axiom config list` to inspect the effective
`[network]` values.

## Local Development Install

Set `AXIOM_AGENT_BINARY_PATH` to skip GitHub downloads during development.

Windows PowerShell:

```powershell
cargo build -p axiom-cli --release
$env:AXIOM_AGENT_BINARY_PATH = "C:\Axiom\target\release\axiom.exe"
npm install -g .
axiom --version
axiom doctor
```

Linux/macOS:

```bash
cargo build -p axiom-cli --release
export AXIOM_AGENT_BINARY_PATH="$PWD/target/release/axiom"
npm install -g .
axiom --version
axiom doctor
```

Name the Rust binary `axiom` on Linux/macOS and `axiom.exe` on Windows. The Cargo config already handles this.

## Release Repository Configuration

`package.json` points at the release repository:

```text
https://github.com/NexaraAI/axiom-agent
```

To test alternate release locations, override it without editing package metadata:

```bash
AXIOM_AGENT_RELEASE_REPO=https://github.com/example/axiom-agent npm install -g axiom-agent
```

## Supported Binary Assets

- `axiom-x86_64-pc-windows-msvc.exe`
- `axiom-x86_64-unknown-linux-gnu`
- `axiom-x86_64-apple-darwin`
- `axiom-aarch64-apple-darwin`

The installer fails with a clear error on unsupported platforms.

## In-Place Updates

After installing from a release binary, the updater can check and stage binary updates:

```bash
axiom update status
axiom update check
axiom update install
```

The updater uses the same release asset names as the npm installer and verifies `SHA256SUMS` before replacing a binary. Cargo builds support checks, but `install` is blocked because self-replacing `target/debug` or `target/release` builds is not a supported install mode.

For npm-global installs, Axiom tries to update the `vendor/bin` binary if permissions allow. If the package location is read-only, reinstall with npm after a new release.
