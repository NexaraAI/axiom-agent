# Axiom — Implementation History and Extended Roadmap

> Historical planning baseline. It does not describe the current candidate
> state and does not supersede itself. The bounded agent loop, identity,
> terminal chat, streaming, compaction, resumable sessions, Coder correction
> loops, policy enforcement, and Proof Mode described below are implemented.
> [V1_PLAN.md](V1_PLAN.md) and [V1_RC_CHECKLIST.md](V1_RC_CHECKLIST.md) are the
> authoritative v1 scope and go/no-go record. Full-screen TUI and executable
> third-party skill sandboxing remain explicitly post-v1.

The original plan is preserved below as implementation history and as a source
for post-v1 ideas. Its code-line references and “current problem” statements
refer to the pre-hardening v0.5.1-beta baseline, not today's working tree.

## Historical baseline: why the plan existed

Three classes of problems were found in the current code:

1. **There is no agent loop.** `axiom-cli/src/chat.rs:171-351` runs one LLM call, at most one tool, then one follow-up. Coder Mode (`code_commands.rs:311-474`) runs plan, patch, apply, test, then finishes with no branch on test failure. For any task needing read, edit, test, fix, retry, the user re-prompts by hand each round. This is the root cause of "normal things feel messed up."

2. **Identity and intent are broken.** `axiom-lens/src/intent.rs:61` does `lower.contains("git")`. The word "git" appears as a substring inside many ordinary English sentences ("you're a git agent", "legit", "digit"), pulling `git.status` and `git.diff` into the context. The same `intent.rs:49` matches `contains("open")` so "I'm open to ideas" pulls `file.read`. On prompts that genuinely match nothing ("who are you", "what can you do"), `build_skill_context_message` returns `None` (`prompt_builder.rs:4-6`), so the LLM is invoked with raw history and no system persona at all. The model has no notion of being Axiom and answers "what can you do" from the few skill cards that happen to be injected, producing answers like "I can only do git status and diff." There is no agent system prompt anywhere in the codebase (grep for `who are you|identity|you are axiom` returns no matches).

3. **The terminal UI is monochrome and bare.** No color library is present (grep for `colored|crossterm|ratatui|yansi` in any `Cargo.toml` returns nothing). All output is plain `println!` (`chat.rs:484-557`, `onboarding.rs:56-83`). The chat prompt is `print!("axiom> ")` (`chat.rs:499`). Errors, lens notices, tool results, and banners all look the same. The name "Axiom" invites a strong visual identity and currently has none.

Plus the structural issues covered in the architectural review: tool execution is a hardcoded `match` on six skill IDs (`executor.rs:167-175`), no streaming (`openai_compat.rs:95`), no context compaction (`chat.rs:34` grows unbounded), no retry or timeout (default `reqwest::Client::new()`), full-file overwrite in coder (`patch.rs:25-27`), a divergent dead `skills/` tree in the repo root (`C:\Axiom\skills\file_read_text\skill.toml:1`), a parsed-but-dead `token_budget` field (`manifest.rs:88`), and a 6-literal test-command allowlist (`code_commands.rs:977`).

---

## Phase 0 — Triage (v0.5.2)

Same-week fixes that remove the worst confusion before any structural change. Each is small, isolated, and bisectable. Nothing here changes the architecture.

### P0.1 Identity system message

Add a fixed `system` message prepended to every chat and coder turn, independent of skill selection. It tells the model it is Axiom Agent, that installed skills are capabilities not the sum of its identity, and that it can answer identity and capability questions directly without invoking a tool. Even when `build_skill_context_message` returns `None` (`prompt_builder.rs:4-6`), the model still knows who it is.

The message is generated once per session and cached. For capability questions, the message includes a short enumerated list of installed skill IDs (not just the currently selected ones), so "what can you do" lists real capabilities rather than only the cards the substring Lens happened to pick for that prompt.

Acceptance: "who are you" returns an Axiom-branded response. "what can you do" lists the actual installed skills, not just `git.status` and `git.diff`.

Implementation site: new `crates/axiom-cli/src/identity.rs` returning the system message; called from `chat.rs:194` before the optional skill-context push; called from `code_commands.rs` plan path.

### P0.2 Stop the substring identity bleed

The `contains`-based intent matcher in `intent.rs:29-90` matches substrings of English words. Immediate fixes without rewriting the matcher:

- Word-boundary checks. Replace `lower.contains(needle)` with a `matches_word(&lower, needle)` helper that requires the needle to appear surrounded by non-alphanumeric boundaries (or at string start/end). This kills `legit` matching `git`, `digital` matching `git`, `open-minded` matching `open`, `writes` matching `write`.
- Identity-intent bypass. Before running intent analysis, check the prompt against a small list of identity, capability, and smalltalk patterns (`who are you`, `what can you do`, `hi`, `hello`, `thanks`, `help me understand`). If matched, set `candidate_skill_ids` to empty and `task_type = "identity"`. The identity system message (P0.1) handles the response. No skill cards are injected.

The full Lens v2 (Phase 2) replaces this matcher entirely. P0.2 is a stopgap so the bleeding stops now.

Acceptance: "who are you, you legit digital agent" selects no skills. "I'm open to ideas" does not select `file.read`.

Implementation site: `crates/axiom-lens/src/intent.rs` (add `matches_word`, identity bypass), plus a test in `intent.rs` mod tests covering the regression cases.

### P0.3 Remove the dead in-repo skills tree

`C:\Axiom\skills\` uses IDs `file_read_text`, `file_write_text`, `project_scan`, `shell_run_safe`, `web_fetch` with entrypoints like `builtin:file_read_text` (`C:\Axiom\skills\file_read_text\skill.toml:1,8`). The executor match (`executor.rs:167-175`) only dispatches `file.read`, `file.write`, `project.scan`, `web.fetch`, `git.status`, `git.diff`. The four in-repo skills can never run. The registry source of truth is `AxiomSkills\skills\`. This duplicate divergent tree misleads anyone reading the repo and silently suggests the project supports skills it cannot execute.

Action: delete `C:\Axiom\skills\`. Update `docs/SKILLS.md` and the README to state that the single source of truth for skill manifests is the `axiom-skills` registry, and that `fixtures/skill-registry/` is a test-only fallback copy.

### P0.4 Enforce `token_budget` or remove it

`SkillManifest.llm_card.token_budget` (`manifest.rs:88`) is parsed onto the struct but `build_skill_context_message` (`prompt_builder.rs:3-42`) never reads it. Either enforce a per-card budget (sum the budgets of selected cards and truncate the list when over the cap) in this phase, or drop the field from the manifest.

Decision: enforce in Phase 1 D6 alongside context compaction. For Phase 0, add a test that asserts the current behavior (parsed-and-unused) so the change in Phase 1 is intentional and visible. Document the dead field in `docs/SKILLS.md` as a known issue until Phase 1 lands.

### P0.5 Document the executor reality

Until Phase 2 ships, document in `docs/SKILLS.md` that only the six built-in executor IDs can run, that prompt-type skills (`python.write`, `python.run`) install and inject context but do not execute code, and that `axiom skill install <external>` installs metadata that cannot be dispatched. Today's README implies all listed skills run.

---

## Phase 1 — The Agent Loop (v0.6)

This is the largest perceived quality jump. It introduces the loop primitive that everything else depends on. Phase 0 lands first; Phase 1 assumes P0.1 and P0.2 are in.

### D1. New crate `axiom-agent` — the loop controller

A library crate containing the agent loop state machine. Owns the turn loop that today lives inlined in `chat.rs:171-351` and `code_commands.rs:311-474`.

State machine:

```
Plan -> Tool -> Observe -> Reflect -> Tool | Done | GiveUp
```

- `Plan`: LLM call with full context (identity message, skill cards, history, todo list). Returns a final answer (Done), one or more tool calls, or a todo update.
- `Tool`: dispatch tool calls through `axiom-engine`.
- `Observe`: collect tool results, record to Proof, update cost ledger.
- `Reflect`: LLM call with results appended; decides next step.
- `Done`: final answer emitted to user.
- `GiveUp`: caps exceeded; surface partial result and the reason.

Caps (config-driven, defaults in a new `[agent]` table in `axiom-core/src/config.rs`):

- `max_iterations = 12` (LLM calls per user turn)
- `max_tool_iterations = 20` (tool calls per user turn)
- `max_tokens = 200000` (approx context cap that triggers compaction; see D6)
- `max_cost_usd = 1.0` (per turn)
- `max_wall_seconds = 300` (per turn)
- `max_consecutive_tool_errors = 3` (give-up threshold)

Public API the CLI calls:

```rust
pub struct AgentLoop { /* caps, provider, installed_skills, proof */ }
impl AgentLoop {
    pub async fn run_turn(&mut self, user_message: ChatMessage) -> Result<TurnResult>;
}
pub enum TurnResult { Done(String), GiveUp { partial: String, reason: GiveUpReason } }
```

CLI `ChatSession::send_user_message_with_options` (`chat.rs:171`) becomes a thin wrapper: build context, call `AgentLoop::run_turn`, append result to history, return to terminal. Coder Mode (`code_commands.rs:311`) calls `AgentLoop` with a coder-specific tool subset and a system prompt asking for `axiom-patch` output.

Acceptance: a single user prompt that needs three or more tool rounds completes without manual re-prompts. Proofs for such turns record three or more tool events. Old single-turn behavior is recovered by setting `max_iterations = 1`.

Implementation site: new `crates/axiom-agent/` with `lib.rs`, `loop.rs`, `caps.rs`, `todo.rs`, `context.rs`, `ledger.rs`. Add to `Cargo.toml` workspace members.

### D2. TodoList state

A `TodoList` carried on the agent loop. The model emits and updates it; the loop injects the current list into every `Plan` and `Reflect` prompt so the model knows what remains. Drives long tasks to completion rather than terminating mid-task.

```rust
pub struct TodoList { pub items: Vec<TodoItem> }
pub struct TodoItem { pub title: String, pub status: TodoStatus }
pub enum TodoStatus { Pending, InProgress, Completed, Blocked }
```

Acceptance: a prompt like "add a failing test, run it, fix the code until it passes" drives three or more todo transitions across iterations and finishes in `Completed` state.

Implementation site: `crates/axiom-agent/src/todo.rs`. Injected into the prompt alongside `axiom-lens/src/prompt_builder.rs`.

### D3. Coder self-correction

`code_commands.rs:441-448` captures `result.exit_code` but never branches on it. After D1, Coder runs through the loop. Add a coder-specific branch: on non-zero test exit, feed stdout and stderr back as an `Observe` result, ask the model to produce a corrected `axiom-patch`. Loop cap (default 3 corrective iterations) before `GiveUp`.

For undo on failed iterations, add a `git stash create` checkpoint in `axiom-coder/src/patch.rs` before each apply. Full-file overwrite (`patch.rs:25-27`) stays for this phase but is now reversible per iteration. Hunks and 3-way apply are Phase 2.

Acceptance: a coder task that deliberately fails tests on first apply self-corrects within the cap and either converges or reports `GiveUp` with reason `MaxCorrectionsReached`. Today the same task reports success on failing tests (`code_commands.rs:463`).

Implementation site: `crates/axiom-agent` (loop wiring), `crates/axiom-coder/src/patch.rs` (stash checkpoint), `crates/axiom-cli/src/code_commands.rs` (call site).

### D4. Streaming responses

Wire `OpenAiCompatibleProvider::stream_chat` (`openai_compat.rs:95-100`, currently `NotImplemented`) to SSE. The trait method and `ChatStream` (`streaming.rs:3-26`) already exist. Print deltas in the chat loop as they arrive. Tool-call detection switches to accumulation-then-parse once the stream closes.

Cloudflare gateway streaming is secondary and may ship after OpenAI-compatible streaming. Mock provider stays non-streaming.

Acceptance: during an LLM call the terminal shows incremental output instead of freezing until completion. This is the single highest perceived-quality win for the least code.

Implementation site: `crates/axiom-llm/src/openai_compat.rs`, `crates/axiom-llm/src/streaming.rs`, `crates/axiom-cli/src/chat.rs` print path.

### D5. Native tool-calling API

Add `tools` and `tool_choice` to the request body in `openai_format.rs:6-21`. Built from `installed_skills` LLM cards (Phase 1 maps the existing card fields to OpenAI tool schema; richer schema lands in Phase 2 with manifest extensions). Parse structured `tool_calls` from the response instead of string-indexing the `axiom-tool` fenced block (`executor.rs:115-128`).

Keep `extract_tool_request` as a fallback path for providers or models without function-calling. The loop tries structured `tool_calls` first, falls back to the text block, then to `Done`.

Acceptance: tool-call reliability rises measurably. A run of 50 mock-provider coder tasks shows fewer `MissingToolBlock` failures than the baseline.

Implementation site: `crates/axiom-llm/src/openai_format.rs`, `crates/axiom-engine/src/executor.rs`, `crates/axiom-agent/src/loop.rs`.

### D6. Context compaction

Add a tokenizer dependency (`tiktoken-rs` or `tokenizers`) to `Cargo.toml`. In `ChatSession::history` (`chat.rs:34`), before each `Plan` and `Reflect` call, count tokens; if over `max_tokens`, summarize the oldest turns into one `system` summary message and drop the originals. Never send an unbounded history.

Compaction strategy: drop oldest turns until under 70 percent of the cap, then insert one `system` message titled "Prior conversation summary" containing a short LLM-generated summary of the dropped turns. The most recent K turns and the full todo list are never compacted. Also enforce `SkillManifest.llm_card.token_budget` here: sum the budgets of selected skill cards and truncate the list when the sum exceeds the per-turn card budget cap. This closes P0.4.

Acceptance: a long chat session that previously hit provider context-too-large errors now continues. Token usage monitor (D8) shows a steady-state ceiling.

Implementation site: `crates/axiom-agent/src/context.rs`, `Cargo.toml` dependency, `crates/axiom-cli/src/chat.rs:34` history buffering.

### D7. Retry, backoff, timeout

Construct `reqwest::Client` with `Builder::connect_timeout` and `Builder::timeout` (today: `Client::new()` with defaults = no timeout) in `openai_compat.rs:27` and `cloudflare_gateway.rs:39`. Wrap `provider_chat` in 3-attempt exponential backoff on `429` and `5xx`. `LlmError` gains a `Retried { attempts, last }` variant.

Acceptance: a transient 5xx during normal use no longer aborts the turn on first contact. No HTTP call hangs longer than the configured timeout.

Implementation site: `crates/axiom-llm/src/openai_compat.rs`, `crates/axiom-llm/src/cloudflare_gateway.rs`, `crates/axiom-llm/src/provider.rs`.

### D8. Cost and token ledger

Per-turn and per-session token counts and estimated cost. Displayed on a status line in the terminal, appended to the Proof trace. Feeds the `max_cost_usd` and `max_tokens` caps in D1.

Acceptance: the user can see in-chat how many tokens and approximate dollars the current session has used. Caps are enforced, not advisory.

Implementation site: `crates/axiom-agent/src/ledger.rs`, `crates/axiom-proof/src/trace.rs`, `crates/axiom-cli/src/chat.rs` display.

### D9. Blood-red themed terminal UI

Add a color and styling layer. The aesthetic is dark terminal with strong blood-red accents, fitting the "Axiom" identity. All output that today uses plain `println!` (`chat.rs:484-557`, `onboarding.rs:56-83`) routes through the new layer. The layer respects `NO_COLOR` and a config toggle `[ui].color = true|false` for users who want plain output or screen-reader-friendly mode.

Dependencies: `colored` (string coloring, zero deps) for general text, or `nu-ansi-term` (smaller, no global state). Avoid `crossterm` and `ratatui` in Phase 1 — they land in Phase 4 for the TUI. Just colored output here, no full-screen mode.

Theme palette (named in code, not hardcoded hex everywhere):

- `axiom_red` = bright red (`#E53935` or ANSI 196) — banner, prompt, Axiom prefix on assistant lines.
- `axiom_dark` = dark red (`#8B0000` or ANSI 88) — borders, separators, dimmed accents.
- `axiom_ember` = orange-red (`#FF6F00` or ANSI 202) — warnings, high-risk skill notices, pending todos.
- `axiom_ash` = warm white (`#F5E6E6` or ANSI 250) — assistant response text.
- `axiom_smoke` = dim warm gray (`#8A7A7A` or ANSI 240) — lens notices, metadata, timestamps.
- `axiom_green` = warm green (`#7CB342` or ANSI 113) — completed todos, passing tests, success.
- `axiom_bone` = off-white (`#ECE0E0` or ANSI 254) — user echo, normal output.

UI elements to theme:

- **Banner.** ASCII "AXIOM" wordmark in `axiom_red` over `axiom_dark` underline, replacing the current plain `println!("Axiom chat")` at `chat.rs:484`. One-line tagline below in `axiom_smoke`: "terminal agent — type !help for commands".
- **Chat prompt.** `axiom_red` `axiom> ` with a dim trailing space, replacing `print!("axiom> ")` at `chat.rs:499`. A right-aligned status segment on the same line shows provider and model in `axiom_smoke` when the line is long enough (terminal width permitting).
- **Lens notice.** `axiom_smoke` "Axiom Lens: selected <skills>" at `chat.rs:543`. Today it is plain text indistinguishable from the model output.
- **Tool notice.** `axiom_ember` for high-risk skills, `axiom_smoke` for low-risk, with a small icon (no emoji — ASCII like `[!]` or `[*]`) prefix. Replaces `chat.rs:553`.
- **Assistant line.** `axiom_ash` with `axiom_red` `Axiom: ` prefix, replacing `chat.rs:555`.
- **Error line.** `axiom_ember` (or ANSI 124) with `Error: ` prefix, replacing `chat.rs:557`.
- **Provider / model / workspace header.** `axiom_smoke` labels with `axiom_bone` values, replacing `chat.rs:485-493`.
- **Onboarding.** All `println!` in `onboarding.rs:56-83` routed through the theme, with numbered options in `axiom_red` and descriptions in `axiom_smoke`.
- **Status line.** New in Phase 1: a single line above the prompt showing turn number, iteration count, tokens used, and estimated cost (from D8), in `axiom_smoke` with numeric values in `axiom_bone`. Updated per turn.

Acceptance: screenshots of `axiom chat` show a clearly identifiable blood-red themed UI distinct from generic CLI output. `NO_COLOR=1 axiom chat` produces plain output. `[ui] color = false` in config produces plain output. Screen-reader users get the same content with no color codes.

Implementation site: new `crates/axiom-cli/src/ui/theme.rs` exposing the named palette; `crates/axiom-cli/src/ui/render.rs` with helpers (`banner`, `prompt`, `lens_notice`, `tool_notice`, `assistant`, `error`, `status_line`) that all output goes through; config field in `axiom-core/src/config.rs` under `[ui]`.

### D10. Better error and status rendering

Pair with D9. Today errors are `println!("Axiom error: {error}")` single-line dumps (`chat.rs:557`). Add structured rendering: which step failed (LLM call, tool execution, parse, approval), which skill, which iteration, and a suggestion when known (retry, different model, rephrase). Long tool outputs get a collapsible-style preview: first N lines, then `axiom_smoke ... N more lines (use !show <id> to view)`. Errors from the loop caps (D1) render as `GiveUp` with the reason in `axiom_ember`.

Acceptance: a tool failure shows a multi-line structured error with the failing skill ID, the iteration count, and a hint, not a bare `anyhow` dump.

Implementation site: `crates/axiom-cli/src/ui/render.rs`, `crates/axiom-agent/src/loop.rs` (structured error types passed up).

### Phase 1 sequencing

D1 first. D1 unblocks D2 and D3 and is the structural change everything hangs on. D9 (UI) is independent of the loop and can land in parallel with any item. Order by impact: D1, D9, D4 (feel), D7 (robustness), D2, D3 (capability), D6 (long sessions), D5 (tool reliability), D8 (visibility), D10 (polish). P0 cleanup (P0.3, P0.5) lands with D1. P0.4 closes when D6 lands.

### Phase 1 risk

- D1 is a refactor of the two largest files (`chat.rs`, `code_commands.rs`). Mitigation: gate behind config `agent.loop_enabled` defaulting true; existing one-shot path stays available at `false` until proven.
- D5 changes the tool protocol. Mock-provider tests rely on the text block (`mock.rs:97-104`). Mitigation: dual-path parser first, remove text path only after tests pass on structured.
- D9 adds a new dep. Mitigation: choose a tiny zero-state crate (`nu-ansi-term`); honor `NO_COLOR` and config toggle from day one.

---

## Phase 2 — Real Skills (v0.7)

Phase 1 makes loops real but the executor is still a hardcoded `match` on six skill IDs (`executor.rs:167-175`). Installing any other skill manifests metadata that can never run. Phase 2 turns the skill ecosystem from marketing into reality and makes the Lens smart enough to deserve the name.

### D11. Executor registry replacing the `match`

Replace `crates/axiom-engine/src/executor.rs:167-175` with a `BTreeMap<SkillId, Box<dyn SkillExecutor>>` populated at startup. Define a trait:

```rust
#[async_trait]
pub trait SkillExecutor: Send + Sync {
    fn id(&self) -> &str;
    async fn execute(&self, args: Value, ctx: &SkillExecutionContext) -> Result<Value, SkillExecutionError>;
}
```

Built-in executors (`file.read`, `file.write`, `project.scan`, `web.fetch`, `git.status`, `git.diff`) register themselves. A skill whose `entrypoint` has no registered executor returns `UnsupportedSkill`, and `axiom skill health` surfaces it as a real diagnostic (today it silently cannot run).

This is the structural change that makes Phase 2's manifest fields and Phase 3's external binaries safe: dispatch is decoupled from compiled-in match arms.

Acceptance: a community skill installed from the registry whose ID matches a registered executor runs without forking the crate. A skill with no registered executor is reported by `axiom skill health`.

Implementation site: `crates/axiom-engine/src/executor.rs` (registry), `crates/axiom-engine/src/lib.rs` (built-in registration).

### D12. Manifest extensions

Add fields to `SkillManifest` (`manifest.rs:22`):

- `depends_on: Vec<SkillId>` — skills that must be co-selected.
- `provides: Vec<String>` — capability tags for cross-skill matching ("filesystem-read", "network-fetch").
- `hooks: SkillHooks { pre: Option<String>, post: Option<String>, on_error: Option<String> }` — declarative lifecycle hooks resolved by the loop.
- `side_effects: Vec<SideEffect>` — declared write surface (filesystem, network, process) for sandboxing (Phase 3).
- `idempotent: bool` — D14 cache eligibility.
- `cache_key: Option<String>` — D14 cache scope.
- `examples: Vec<String>` — D13 Lens training signals.
- `keywords: Vec<String>` — D13 Lens matching terms, replacing the hardcoded `intent.rs` lists.

Lens uses `depends_on` to co-select prerequisites. The loop uses `hooks` to fire pre/post/error handlers (wired in Phase 3 with the loop already in place).

Acceptance: a skill manifest with `depends_on = ["file.read"]` causes Lens to select both when the dependent skill is chosen. Schema validation (D19) enforces the new fields.

Implementation site: `crates/axiom-engine/src/manifest.rs` (struct + serde), `crates/axiom-engine/src/registry.rs` (validation).

### D13. Lens v2 — manifest-driven selection

Delete the hardcoded `intent.rs:29-90` substring rules (the same rules P0.2 patched as a stopgap). Replace with manifest-driven matching:

- Read `keywords` and `examples` from each skill's manifest.
- Score by fuzzy match against the prompt (using a small embedder or weighted Jaccard over token sets — not full embeddings yet, those are optional in D20).
- Add recency-weighted usage stats — already tracked in `installed_skills.json` (`installed.rs`) but unused today — as a tiebreaker.
- Enforce `SkillCard.token_budget` (`manifest.rs:88`, dead in v0.5) by summing selected card budgets and truncating the list when over the cap.
- Apply a `max_risk_level` config filter before selection (closes the parsed-but-unused `IntentAnalysis.risk_level` from `intent.rs:27`).
- Only fall back to substring `contains` matching if the manifest has no `keywords` and no `examples` (backward compatibility with existing skills).

Acceptance: regression tests for P0.2 still pass without the stopgap patches. A prompt "explain Python decorators" selects `python.write` from manifest keywords, not because the literal string "python" appeared. No skill is selected for "who are you."

Implementation site: `crates/axiom-lens/src/intent.rs` → split into `matcher.rs` (manifest-driven) and `legacy.rs` (substring fallback).

### D14. Parallel tool calls

The loop in D1 handles one tool per `Tool` step. Native function-calling APIs (D5) can return an array of `tool_calls`. Update the loop to dispatch N independent tool calls concurrently. Add a dependency check using `depends_on` (D12) to decide what can run in parallel. Cap concurrency at `max_parallel_tools = 4` (config).

Acceptance: a prompt that needs three independent `file.read` calls completes in the time of one call rather than three sequential rounds.

Implementation site: `crates/axiom-agent/src/loop.rs` (parallel dispatch), `crates/axiom-engine/src/executor.rs` (concurrent executor registry).

### D15. Better test command detection

Replace the `has_file` filename guessing in `axiom-coder/src/test_runner.rs:31-82` with real manifest parsing:

- Open `package.json` and read `scripts.test` (the current code never opens it, so a project whose test script is `vitest run` still reports `npm test`).
- Detect Cargo workspaces (root `Cargo.toml` with `[workspace]`).
- Detect Go (`go.mod`), Maven (`pom.xml`), Gradle (`build.gradle*`), Deno (`deno.json`), Bun (`bun.lockb`), Rake (`Rakefile`).
- Monorepo support: scan one or two levels deep, not just the root.

Replace the 6-literal execution allowlist at `code_commands.rs:977` with a configurable allow-regex. Default allows common `cargo test`, `npm test`, `pnpm test`, `yarn test`, `python -m pytest`, `pytest`, `go test`, `mvn test`, `gradle test`, plus any command matching a user-configured pattern.

Acceptance: a Node project with `"test": "vitest run"` detects and runs `npm test` (which executes `vitest run`), not a hardcoded `npm test` that may do something else. A `cargo test --no-run` invocation is no longer refused by the allowlist.

Implementation site: `crates/axiom-coder/src/test_runner.rs`, `crates/axiom-cli/src/code_commands.rs:977`, `[coder] test_allow_regex` config in `axiom-core/src/config.rs`.

### D16. Hunk-based patches with 3-way apply

Replace the full-file overwrite in `axiom-coder/src/patch.rs:25-27` with unified-diff hunks. Use the `similar` crate (already a common Rust dep, small) for diffing and 3-way apply. Detect conflicts before applying; surface them to the user instead of silently overwriting. The `git stash` checkpoint from D3 stays as the undo mechanism for failed applies.

Acceptance: a coder patch that touches a region of a file the user has since edited by hand produces a conflict notice, not a silent overwrite that destroys the user's edit.

Implementation site: `crates/axiom-coder/src/patch.rs` (hunk parsing, conflict detection), `crates/axiom-coder/src/planner.rs` (prompt asks for hunks not full files), add `similar` to `Cargo.toml`.

### D17. Plan verification before apply

Today the plan (`code_commands.rs:517`) is parsed by `parse_plan_response` (`planner.rs:115`) which only strips list bullets — no validation against the scan or the task. Add a verification step: after the plan is parsed, the loop runs a cheap LLM self-review pass checking the plan covers the user's task and does not edit files unrelated to the task. Failures block apply and request a new plan (bounded retries within D1 caps).

Acceptance: a coder plan that proposes to edit `README.md` for a task that asked to fix a test in `src/lib.rs` is caught by the verification pass and rejected before apply.

Implementation site: `crates/axiom-agent/src/loop.rs` (verification step), `crates/axiom-coder/src/planner.rs` (verify prompt).

### D18. Schema validation for skill manifests

Today `SkillManifest::validate` (`manifest.rs:169-189`) only rejects blank `id, name, description, entrypoint, author, license`. Add proper JSON-Schema-style validation:

- `input_schema` and `output_schema` are validated as JSON Schema objects (today they default to empty tables at `:226-228` and are never enforced at runtime).
- `permissions` cross-referenced against a known `Permission` enum (`manifest.rs:91`).
- `min_axiom_version` enforced at parse time, not only at `lifecycle.rs:152` compatibility check.
- `depends_on` IDs exist in the registry (D11).
- `entrypoint` either `prompt-only`, `builtin:<id>` with a registered executor, or reserved for Phase 3 external form.

Acceptance: a manifest with an unknown permission or a non-existent `depends_on` is rejected at install time with a clear error.

Implementation site: `crates/axiom-engine/src/manifest.rs`, `crates/axiom-engine/src/registry.rs`.

### Phase 2 sequencing

D11 first (enables D12, D14, D18). Then D13 (Lens v2, fixes the substring chaos permanently and removes the P0.2 stopgap). Then D15, D16, D17 (coder improvements). D12 (manifest extensions) earlier if D17 needs fields. D14 after D11. D18 alongside D11.

---

## Phase 3 — Resumable, Composable, Autonomous (v0.8)

Phase 2 makes skills real. Phase 3 makes sessions survive interruption and lets one task spawn sub-agents. This is what defines an agentic IDE-class tool.

### D19. Checkpoints and resume

Persist `ChatSession.history` and `TodoList` to `~/.config/axiom-agent/sessions/<id>.json` at the end of each turn (D1 loop) and on `Ctrl-C`. Wire `ProofRecorder::from_trace` (`recorder.rs:96-102`, exists and is dead) so `axiom resume <session>` reconstructs the session from the saved state plus the proof trace. A resumed session continues with its todo list, history, and cost ledger intact.

Acceptance: a long coding task interrupted by `Ctrl-C` can be resumed with `axiom resume <id>` and continues from the next pending todo item, not from scratch.

Implementation site: `crates/axiom-agent/src/session.rs` (new), `crates/axiom-cli/src/chat.rs:39` (`ChatSession::load` reads prior history), `crates/axiom-cli/src/main.rs` (new `Resume` subcommand).

### D20. Sub-agents via the compose skill type

Add a `SkillType::Compose` variant (the manifest already has `Workflow` and `Guard` as no-op variants at `manifest.rs:117`). A compose skill spawns a child `AgentLoop` with its own context window and a restricted tool subset. The child runs to `Done` or `GiveUp` and returns a result summary to the parent. Backed by the D11 executor registry, so spawning a sub-agent is just-another-skill.

Use cases: parallel research-then-summarize, explore a subtree while the parent keeps editing, run a risky change in a sandboxed sub-agent first and report back.

Optional in this phase: lightweight embedding-based Lens using `candle` or a local ONNX model for true semantic skill matching, replacing the fuzzy matcher from D13. Defer if the fuzzy matcher holds up.

Acceptance: a prompt "research the codebase for X, then implement Y" runs a research sub-agent to completion, returns a summary, and the parent agent implements Y using the summary.

Implementation site: `crates/axiom-agent/src/sub_agent.rs`, `crates/axiom-engine/src/manifest.rs` (Compose variant reuse).

### D21. Hooks firing in the loop

The manifest `hooks` (D12) are now real. The loop fires `pre_tool`, `post_tool`, `on_error`, and `on_complete` handlers declared on the skill being executed. Hooks are themselves skills (small prompt-type or tool-type skills pointed at by the manifest), so they compose via D11.

Built-in hook examples land in the ` axiom-skills` registry: `hook.diff_snapshot` (pre_tool on `file.write` — save a snapshot before write), `hook.lint_changed` (post_tool on `file.write` — run lint on the changed file), `hook.git_restore` (on_error — restore from the D3 stash).

Acceptance: a skill with `hooks.post = "hook.lint_changed"` runs the lint skill after applying the write, and a lint failure feeds back into the loop as an `Observe` result that may trigger a corrective iteration.

Implementation site: `crates/axiom-agent/src/loop.rs` (hook firing at each state transition), `crates/axiom-engine/src/executor.rs` (hook resolution against the registry).

### D22. Persisted cost ledger

Per-session and rolling USD totals persisted to `~/.config/axiom-agent/sessions/ledger.json`. Displayed in the status line (D9) and queryable via `axiom cost` (new subcommand). Feeds the D1 `max_cost_usd` cap across sessions if the user configures a daily or weekly budget.

Acceptance: `axiom cost` shows per-session and per-week totals. Setting `[agent] weekly_budget_usd = 5.0` causes the loop to `GiveUp` with reason `BudgetExceeded` when the week's spending exceeds the cap.

Implementation site: `crates/axiom-agent/src/ledger.rs` (persistence), `crates/axiom-cli/src/main.rs` (`Cost` subcommand).

### D23. Thinking modes

A toggle (`!think on` chat command or `[agent] thinking = true`) that injects a `system` scratchpad message persisted across turns (separate from `history`) where the model reasons step by step before emitting tool calls. The scratchpad is visible to the user in `axiom_smoke` and recorded in the proof but is not sent to the provider as user content — it lives in a dedicated system role.

Acceptance: with thinking on, the user sees a reasoning trace before each tool call, and tool-call selections improve on hard tasks. With thinking off, behavior matches Phase 1.

Implementation site: `crates/axiom-agent/src/loop.rs` (scratchpad handling), `crates/axiom-cli/src/chat.rs` (`!think` command).

### D24. External skill binaries — WASM sandbox

External executable skills were a known not-done item since v0.1 (`README.md:38`). With D11's executor registry decoupling dispatch from compiled-in match arms, external skills become safe to add. Implement a `wasmtime`-backed executor that runs sandboxed WASM modules registered as skills. Capability-scoped: filesystem read-only, network, process spawn — declared via the D12 `side_effects` field and enforced by the sandbox.

Acceptance: a community skill shipped as a `.wasm` binary installs via `axiom skill install <id>`, declares its side effects, and runs sandboxed with no access to undeclared resources. A skill that tries to read `~/.ssh/id_rsa` is blocked by the sandbox.

Implementation site: `crates/axiom-engine/src/wasm_executor.rs` (new), `wasmtime` dependency in `Cargo.toml`, D11 registry registration of WASM executors.

### Phase 3 sequencing

D19 first (resume is the headline). D20 (sub-agents) and D21 (hooks) can run in parallel after D19. D22 alongside D19. D23 independent. D24 last (depends on D11, D12, and significant sandbox testing).

---

## Phase 4 — UX Polish (runs across v0.6 to v0.9)

UI work is not held for a single phase. D9 (Phase 1) lays the theme. Phase 4 collects the larger UX items that need a full-screen or richer interaction model.

### D25. Full-screen TUI mode

Optional full-screen mode via `axiom chat --tui` using `ratatui` + `crossterm`. Shows chat, lens status, todo list, current tool output, and cost ledger in split panes. Falls back to the inline mode from D9 when the terminal is too small or `--tui` is not passed. Blood-red theme carried through to the TUI borders and headers.

Acceptance: in a 120x40 terminal, `axiom chat --tui` shows a four-pane layout (chat left, todo top-right, cost bottom-right, tool output bottom) with the blood-red theme, and degrades gracefully to inline mode in an 80x24 terminal.

Implementation site: `crates/axiom-cli/src/tui/`, add `ratatui` and `crossterm` to `crates/axiom-cli/Cargo.toml`.

### D26. Inline diff viewer

For Coder Mode (Phase 1 D3), an interactive diff viewer with accept/reject per hunk once D16 lands hunks. Replaces the current print-the-diff-then-y/n flow (`code_commands.rs:377-411`). Uses `ratatui` widgets inside the TUI or a paged `less`-style viewer outside it.

Acceptance: a coder patch with three file changes shows each file's diff in turn with per-hunk accept/reject, and the loop continues with only accepted hunks.

### D27. Command palette

A fuzzy command launcher (Ctrl-P in TUI, or `axiom` with no args showing a menu) for every subcommand, installed skill, and chat dot-command. Replaces the current behavior of running `axiom` with no subcommand going straight to `startup` (`main.rs:262`).

Acceptance: `axiom` with no subcommand shows a searchable palette of actions (chat, code, resume, installed skills, proof list) instead of silently routing to onboarding or chat.

### D28. Multiline and rich input

Support paste of multiline input in chat (today `read_line` reads one line; `chat.rs:503`). Add a dedicated multiline mode toggle (Ctrl-O or `!multi`), bracketed paste detection, and a lightweight editor for long prompts. Optional `reedline` or `rustyline` integration for history navigation and basic emacs keybinds.

Acceptance: pasting a 30-line prompt into the chat input works and is submitted as one user turn.

### D29. Configurable themes

The blood-red theme is the default but not the only option. Add `[ui] theme = "blood_red" | "ash" | "high_contrast" | "none"` in config. `high_contrast` is an accessibility theme (no dim grays). `none` is the Phase 0 plain output. Themes are defined as palette swap in `crates/axiom-cli/src/ui/theme.rs`.

Acceptance: `axiom config set ui.theme ash` switches the palette live on the next chat start.

---

## Acceptance for the full plan

- Phase 0: `who are you` and `what can you do` work. `I'm open to ideas` does not select `file.read`. `C:\Axiom\skills\` removed. `docs/SKILLS.md` documents the executor reality.
- Phase 1: a single user prompt that needs three tool rounds completes without manual re-prompts. Coder self-corrects on test failure. Streaming shows incremental output. Retry/backoff handles 5xx. Context compaction keeps long sessions alive. Blood-red themed UI renders across banner, prompt, lens, tool, assistant, error, status line.
- Phase 2: community skills run without forking the crate. Lens selects from manifest keywords, not hardcoded substrings. Patches are hunk-based with conflict detection. Test detection reads `package.json` scripts. Parallel tool calls work.
- Phase 3: `Ctrl-C` then `axiom resume <id>` continues the task. Sub-agents spawn for research-then-edit. Hooks fire pre/post/error. Weekly budget caps work. Thinking mode shows reasoning traces. External WASM skills run sandboxed.
- Phase 4: `axiom chat --tui` shows the four-pane blood-red layout. Per-hunk diff accept/reject. Fuzzy command palette. Multiline paste. Configurable themes.

Regressions: `cargo fmt`, `cargo clippy --all-targets --all-features`, `cargo test` clean at every phase boundary. `node scripts/e2e-test.js`, `node scripts/release-check.js`, `node scripts/security-check.js` all still pass without API keys or network. `NO_COLOR=1` and `[ui] color = false` produce plain output at every phase.

## Out of scope for all phases

- Desktop, mobile, web, Tauri, Android apps. Explicitly deferred per the v0.5 PRD (`docs/PRD.md:4`).
- GitHub Release asset automation. Existing release workflow continues.
- A hosted skill marketplace. The registry stays a git repo at `NexaraAI/axiom-skills`.
