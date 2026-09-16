# Model Context Protocol (MCP)

Axiom speaks MCP in **both** directions:

- As an MCP **client**, it launches the servers you declare in `[[mcp.servers]]`
  and wraps every tool they publish as an ordinary Axiom tool. Each call is
  validated and then authorized through the same `[policy]` side-effect rules
  and approval hooks that guard Axiom's built-ins.
- As an MCP **server**, `axiom mcp serve` exposes Axiom's own tool registry to
  other MCP clients over stdio, gated by the same policy plus a non-interactive
  approval mode.

Both directions share the JSON-RPC 2.0 framing, speak protocol revisions
`2025-06-18`, `2025-03-26`, and `2024-11-05` (newest advertised first), and
exchange newline-delimited JSON over stdio.

## Security model

Integrating a third-party MCP server is a trust boundary, so nothing about it is
implicit:

- **Declaring a server grants nothing.** Servers are inert until
  `enabled = true` on both the `[mcp]` section and the server itself.
- **Remote tools are never silently trusted.** A tool's side-effect classes come
  from the server's own MCP annotations. The protocol's defaults are pessimistic
  (not read-only, destructive, open-world), so an un-annotated third-party tool
  is gated as a *process* that *writes* and reaches the *network*. `Process` is
  always present because calling a remote tool runs someone else's program.
- **Same policy, same approvals.** A remote call flows through the identical
  authorization path as a built-in tool, including `ask` prompting and Proof Mode
  audit records. `auto_approve` only relaxes `ask` to `allow` for the classes
  named; `deny` is never overridden.
- **Outputs are validated.** Results are checked against the tool's output schema
  before they are handed to the model.
- **Failures are contained.** A server that will not start or hand-shake is
  reported as a warning and skipped; it never breaks the rest of the session.

## Declaring servers

Add servers to the Axiom config (see `axiom config path`):

```toml
[mcp]
enabled = true             # master switch; false leaves every server inert
connect_timeout_secs = 20  # handshake bound when launching a server
request_timeout_secs = 60  # bound on tools/call
max_response_bytes = 1000000

[[mcp.servers]]
name = "github"            # short id used in tool names: mcp.github.<tool>
command = "npx"
args = ["-y", "@modelcontextprotocol/server-github"]
env_from_secret = ["GITHUB_PERSONAL_ACCESS_TOKEN"]  # resolved from the credential store
enabled = true
auto_approve = false       # true relaxes `ask` for this server's classes
# side_effects = ["network", "process"]  # override annotation-derived classes
# allow_tools = ["search_issues"]        # empty means "all tools the server lists"
# deny_tools = ["delete_repository"]

[[mcp.servers.tools]]
name = "search_issues"     # per-tool overrides win over the server's
enabled = true
auto_approve = true
# side_effects = ["network"]
```

Accepted side-effect class names: `filesystem_read`, `filesystem_write`,
`network`, `process`, `git`.

`env` passes literal variables (plaintext in the config); prefer
`env_from_secret` for anything sensitive — it resolves names from Axiom's
credential store (or the process environment) and forwards the values only to
the server process. Secrets never appear in proofs, sessions, or errors.

## Inspecting the client

```bash
axiom mcp list                       # show configured servers without starting them
axiom mcp tools                      # connect the enabled servers and list their tools
axiom mcp tools --server github      # inspect one server, even if disabled
axiom mcp tools --include-disabled   # connect servers marked enabled = false too
```

`axiom mcp tools` prints each tool as `mcp.<server>.<tool>` with the
side-effect classes Axiom will enforce and whether `auto_approve` applies. Use
that output to verify what a server actually offers before trusting it in a
session — and to see exactly what a server's instructions say.

Once configured, remote tools are advertised to the model alongside the
built-ins during chat (`axiom chat`, `axiom run`). Name collisions resolve in
favor of built-ins; a malformed remote definition is skipped rather than failing
the turn.

## Serving Axiom to other clients

```bash
axiom mcp serve                    # expose Axiom's tools over stdio
axiom mcp serve --read-only        # only tools that cannot mutate the machine
axiom mcp serve --approve          # approve `ask` decisions instead of refusing
axiom mcp serve --allow file.read --allow git.diff
axiom mcp serve --deny shell.run
```

`axiom mcp serve` never writes to stdout except the protocol itself — all
diagnostics go to stderr, so it can be launched directly by another client:

```jsonc
// e.g. a client's MCP configuration
{ "mcpServers": { "axiom": { "command": "axiom", "args": ["mcp", "serve"] } } }
```

Approval is non-interactive here: an `ask` decision is **refused** unless the
server was started with `--approve`. `deny` is always refused. Tools that only
make sense inside Axiom's own session (`question.ask`, `subagent.run`,
`skill.create`) are never exposed, and `--read-only` also hides anything that
writes, spawns a process, or otherwise mutates state. Disabled or quarantined
installed skills are not offered.

The advertised name for a built-in uses underscores (`file.read` → `file_read`),
and `tools/call` accepts the exposed name, the dotted skill id, or the
underscore id.

## Limits and failure modes

| Limit | Default | Config |
|---|---|---|
| Server handshake | 20s | `mcp.connect_timeout_secs` |
| `tools/call` | 60s | `mcp.request_timeout_secs` |
| Frame size | 1,000,000 bytes | `mcp.max_response_bytes` |

A frame larger than the limit is rejected instead of buffered, so a misbehaving
peer cannot exhaust memory. Spawned server processes are terminated when the
client drops them, and `axiom mcp serve` ends when its client closes the stream.

## Related documents

- [`docs/THREAT_MODEL.md`](THREAT_MODEL.md) — trust boundaries and mitigations.
- [`docs/SKILLS.md`](SKILLS.md) — Axiom's own tool and skill model.
- [`docs/GATEWAY.md`](GATEWAY.md) — the messaging gateway, which shares the same
  side-effect policy.
