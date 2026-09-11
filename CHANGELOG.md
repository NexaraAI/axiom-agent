# Changelog

All notable changes to Axiom are documented here. Versions follow semantic
versioning. Stable releases document user-visible changes, configuration or
proof migrations, security fixes, and upgrade actions.

## 1.0.15

This release introduces catalog-aware model resolution and suggestion for `/model`, automatic PowerShell syntax normalization (`&&` to `;`) with POSIX utility shims on Windows, OS-targeted shell tool selection, synthetic `<tool_call>` tag suppression, and humanized provider error formatting.

Install it:

```bash
npm install -g axiom-agent@1.0.15
```

### New Features & Resiliency Improvements

- **Intelligent Model Resolution & Catalog Validation**:
  - `/model <query>` and `axiom model use <query>` now validate against the active provider's catalog before switching.
  - If a model query is ambiguous (e.g. `/model nemo` or `/model muse`), Axiom presents a list of close catalog matches and keeps the existing model intact instead of switching to a non-existent ID.
  - Automatically resolves unique matches (e.g. `/model ultra` resolves to `nemotron-3-ultra-free`).
  - Added `/model force <id>` and `axiom model use <id> --force` to bypass validation for custom reverse proxies or unlisted models.
- **PowerShell Command Normalization & Windows Shims**:
  - Automatically normalizes bash-style chain operators (`&&` and `||` outside of string quotes) to `;` when invoking Windows PowerShell, eliminating `The token '&&' is not a valid statement separator in this version` errors on Windows PowerShell 5.1.
  - Injects native compatibility shims into PowerShell executions: strips alias shadowing for `curl` and `wget` to invoke the real system binaries, and provides `grep`, `head`, and `which` helper functions.
- **OS-Aware Shell Candidate Selection**:
  - `axiom-lens` now strictly selects `shell.powershell.safe` on Windows for generic shell and dev-server tasks, preventing models from attempting to run bash/zsh tools on Windows hosts.
- **Synthetic `<tool_call>` Leak Suppression**:
  - `ControlBlockProjector` and the CLI terminal renderer now intercept and suppress raw `<tool_call>...</tool_call>` XML tags emitted in assistant prose by certain open models.
- **Human-Readable Error Summaries**:
  - Enhanced `summarize_body` in `axiom-llm` to parse JSON error responses from providers and format them cleanly (e.g. `Model muse is not supported (ModelError)`) rather than printing raw escaped JSON.

## 1.0.14

This release eliminates OpenCode credential corruption through automatic self-healing, halts generative repetition loops in streaming responses, implements strict prompt prefix stability for prompt caching, and introduces smart file reading to eliminate baseline context bloat.

Install it:

```bash
npm install -g axiom-agent@1.0.14
```

### New Features & Resiliency Improvements

- **Credential Self-Healing & OpenCode 401 Resolution**:
  - Added `sanitize_secret` to detect accidental double-pasting in masked terminal prompts (`key + key`), duplicate prefixes (`sk-`, `nvapi-`, `gsk_`), surrounding quotes, and shell export syntax.
  - Implemented automatic keychain self-healing: upon resolving credentials from the OS Credential Manager (WinCred) or local fallback store, Axiom automatically sanitizes the key and repairs the stored entry in the background.
- **Real-Time Streaming Repetition Loop Guard**:
  - Implemented a sliding window repetition detector in `axiom-llm` streaming that intercepts autoregressive runaway loops (3+ repetitions of 30+ byte diverse blocks).
  - Automatically truncates duplicate cycles and cleanly terminates the stream, preventing 100,000+ token runaway bills.
- **Session History Poisoning Protection on Cancellation**:
  - Interrupted runs (Ctrl+C / cancellation) now sanitize partial stream accumulator output and truncate to 1,500 characters before saving to session history, keeping context clean for subsequent turns.
- **Prompt Caching & Context Bloat Optimization**:
  - Unified identity, skill context, and plan mode directives into a single canonical system message to preserve prompt cache prefix stability across multi-turn chats.
  - Deterministically sorted `tool_definitions` by name and `skill_cards` by ID.
  - Added OpenRouter app attribution and prompt caching headers (`HTTP-Referer: https://axiom.nexara.ai`, `X-Title: Axiom Agent`).
  - Added smart pagination in `file_read`: large files (>400 lines) default to 300 lines with explicit pagination hints.
- **Codex CLI Architectural Inspirations**:
  - Added action-driven preambles (1-2 sentence updates with immediate tool invocation) and strict `No Tool Output Simulation` directives forbidding synthetic tool JSON blocks.

## 1.0.13

This release resolves OpenRouter reasoning compatibility, adds live tool execution spinners, protects against infinite verification loops, and provides autonomous recovery from command timeouts and declined approvals.

Install it:

```bash
npm install -g axiom-agent@1.0.13
```

### Bug Fixes & Resiliency Improvements

- **Live Tool Execution Spinner & Timer**:
  - Replaced static terminal tool execution printouts with active 80ms pulsing spinners displaying real-time elapsed seconds (`⠋ Axiom Tool: executing <skill> (<Xs>)`).
  - Added clean completion and timeout indicators (`✔ Axiom Tool: completed` and `⏱ Axiom Tool: timed out`).
  - Implemented automatic spinner drop cleanup to eliminate terminal display artifacts on cancellations.
- **OpenRouter Reasoning HTTP 400 Resolution**:
  - Enforced exclusive `reasoning.effort` payload for OpenRouter models to prevent HTTP 400 rejection.
  - Enhanced HTTP 400 auto-recovery in `OpenAiCompatibleProvider` to catch `reasoning` alongside `reasoning_effort` and transparently strip incompatible parameters on retry.
- **Loop Controller Autonomous Timeout & Approval Recovery**:
  - Exempted command timeouts (exit code 124), user cancellations, and declined approvals from counting towards fatal `consecutive_tool_errors`.
  - Added autonomous recovery directives to tool observations instructing the model to inspect partial disk outputs, background processes, or adapt approach rather than aborting.
- **Stage 4 Reviewer & Debugger Guardrails**:
  - Prevented Stage 4 verification checks from running on aborted turns, provider errors, or turns without code modifications.
  - Capped automated verification retries to prevent infinite debugger feedback loops.
  - Fixed Windows `program not found` in test execution by executing commands via PowerShell.
  - Updated test runner detection to verify that `package.json` actually contains a test script before proposing `npm test`.
- **Anti-Promissory Chatter Rules**:
  - Added strict prompt directives preventing models from narrating future intent without issuing tool calls in the same turn.
  - Added Windows PowerShell syntax guidance (using `;` instead of `&&`, and `$HOME` instead of `~`).

## 1.0.12

This release introduces comprehensive skill installation and management (local paths, GitHub repositories, registry, and built-in catalogs), dedicated GitHub search and deep web research engines, multi-step thinking and writing state indicators, modular skills for clean humanized code and game building, and robust JSON tool-call repair.

Install it:

```bash
npm install -g axiom-agent@1.0.12
```

### New Features & Improvements

- **Skill Installation & Extensibility**:
  - Added support for installing skills from local directories (`/skill install ./path/to/skill` or `axiom skill install ...`), GitHub repositories (`/skill install https://github.com/owner/repo` or `github:owner/repo`), and the built-in skill catalog.
  - Added interactive slash commands: `/skill list`, `/skills list`, `/skill install <id>`, and `/skills install <id>`.
  - Automatic dynamic skill loading from `.axiom/skills` directory and runtime skill discovery.
- **Deep Research & GitHub Search**:
  - Implemented `github.search` executor supporting repositories, organization repositories, releases, readmes, and repository detail modes.
  - Implemented anti-bot resilient DuckDuckGo POST form search with automatic fallback to Wikipedia OpenSearch API.
  - Added modular skills: `deep-research`, `github-research`, `humanized-codes`, `game-builder`, and `research-first`.
  - Modularized domain-specific knowledge into addable/removable skills rather than monolithic system prompts.
- **Interactive Thinking & Writing Status Indicators**:
  - Added continuous multi-step thinking spinners across agent loop iterations, displaying live elapsed duration and buffering states.
  - Dynamic tool composing and writing indicators (`Writing arguments for <tool>...`).
  - Added robust JSON repair in `extract_tool_request` to automatically fix unclosed braces, trailing delimiters, and unclosed quotes.
  - Sanitized raw tool execution blocks to prevent unparsed JSON from leaking into terminal output.

## 1.0.11

This release fixes custom reply adaptation for interactive questions (`question.ask`), enforces a strict research-first protocol across all agent operations, eliminates overthinking loops on conversational queries, and prevents internal chain-of-thought preambles from leaking into chat output.

Install it:

```bash
npm install -g axiom-agent@1.0.11
```

### New Features & Improvements

- **Custom Answer & Clarification Adaptation (`question.ask`)**:
  - Resolved prompt contradiction where `question.ask` results were previously labeled as untrusted data with instructions to never follow them.
  - Formatted user responses (options and custom write-in replies) as trusted, immediate top-priority instructions.
  - Updated loop reflection instruction to directly fulfill user clarification replies rather than falling back to the original request.
- **Research-First Operating Protocol**:
  - Added dedicated `research-first` builtin skill and system prompt directives enforcing online research (`web.fetch`) before taking actions or answering questions on unfamiliar platforms, APIs, or domain concepts.
  - Exposed `query` in `web.fetch` tool schema so models actively utilize web search capabilities.
  - Fixed URL-encoding in `validated_web_target` when search queries contain spaces.
  - Enhanced orchestrator to automatically detect research and platform discovery inquiries (e.g. Minecraft modding distribution platforms, library alternatives).
- **Overthinking Loop Elimination**:
  - Relaxed `question.ask` in identity instructions: restricted to critical technical forks, destructive confirmations, or explicit user polls.
  - Mandated direct, comprehensive answers for advisory, conceptual, planning, and community management questions without unneeded filesystem scans (`project.scan`) or MCQ interrogations.
- **Thinking Preamble Projection & Leak Prevention**:
  - Upgraded `ControlBlockProjector` in `axiom-llm` to recognize unstructured chain-of-thought prefixes (`Here's a thinking process:`, `Thinking Process:`) and route them into `reasoning_delta` (`💭 Thinking:`) instead of leaking into visible assistant responses (`◆ Axiom: ...`).
  - Added matching handling in `render.rs` assistant formatting.

## 1.0.10

This release introduces an interactive Command Palette (`/commands`, `/palette`, `/menu`, `/`) with full keyboard navigation and quick controls, gateway-decided reasoning effort in auto mode, provider-specific reasoning schema adaptations (OpenRouter, Anthropic, OpenAI, Groq), dynamic endpoint model switching across all variants (Default, low, medium, high, xhigh), and a first-class `/models` catalog discovery alias.

Install it:

```bash
npm install -g axiom-agent@1.0.10
```

### New Features & Improvements

- **Interactive Command Palette (`/commands`, `/palette`, `/menu`, `/`)**:
  - Added an interactive TUI card with arrow-key and enter navigation providing instant access to essential controls:
    - Work Mode toggle (Plan Mode vs Build Mode)
    - Model Variant selection (`Default`, `low`, `medium`, `high`, `xhigh`)
    - Reasoning/Thinking mode toggle (`auto`, `on`, `off`)
    - Permission Mode switch (`velocity`, `full_machine`, `strict`)
    - Model catalog viewer and switcher
    - Visual theme changer (`axiom`, `blood_red`, `ash`, `high_contrast`)
    - Workspace test execution (`/test`)
    - Task queue, Audit Proof, Skills, Checkpoints, and Session Clear.
- **Gateway-Decided Reasoning & Auto Mode**:
  - In auto mode (`thinking: None`), synthetic `reasoning_effort` and `thinking` parameters are omitted, allowing gateways (OpenCode Zen, OpenRouter, Gemini, Ollama) to decide reasoning effort naturally without HTTP 400 or SSE stream parse errors.
- **Provider-Specific Reasoning Adaptations**:
  - OpenRouter: unified `"reasoning": { "effort": "...", "max_tokens": ... }` and `"effort": "none"`.
  - Anthropic: `"thinking": { "type": "enabled", "budget_tokens": ... }` and `"type": "disabled"`.
  - OpenAI / GitHub Models: `"reasoning_effort": "..."`.
  - Groq: `"reasoning_format": "parsed"` and `"reasoning_effort": "..."`.
- **Dynamic Endpoint Variant Switching**:
  - Configured concrete endpoint models for every supported provider (`openrouter`, `gemini`, `github-models`, `groq`, `opencode`, `gmicloud`, `nvidia`, `openai`, `ollama`, `ollama_cloud`, `lm-studio`).
  - Switching variants (`/variant <low|medium|high|xhigh|Default>`) now actively switches models according to provider capabilities rather than staying on the active model.
- **Model Catalog Alias (`/models`)**:
  - Added `/models` and `/models <filter>` as direct aliases for `/model list [FILTER]` with autocomplete hints.

## 1.0.9

This release fixes terminal cursor desynchronization and text collision during multiline prompt typing and line wrapping, and adds native Shift+Enter and Alt+Enter multiline keybindings.

Install it:

```bash
npm install -g axiom-agent@1.0.9
```

### Bug Fixes & Improvements

- **Terminal Line Wrapping & Multiline Text Collision Fix**:
  - Separated raw ANSI escape sequences from Rustyline's base prompt width calculation using `Highlighter::highlight_prompt`.
  - Fixed issue where the styled prompt (`│ axiom ❯ `) caused Rustyline to miscalculate visual prompt width by counting non-printing ANSI escape codes as visible columns, causing text to wrap prematurely and overwrite earlier lines when typing past 1 line.
  - Added native `Shift+Enter`, `Alt+Enter`, and `Ctrl+J` keybindings to insert newlines directly in the chat prompt for seamless multiline input.
  - Added unit test coverage for plain prompt width invariant and highlighter styling.

## 1.0.8

This release adds automated in-session package updates via `/update`, robust model variant switching with verified OpenCode Zen free models (`nemotron-3.5-lightning-free`, `nemotron-3-ultra-free`, `mimo-v2.5-free`), mandatory `x-opencode-session` session header support for OpenCode Zen, and Windows executable file-lock tolerance during updates.

Install it:

```bash
npm install -g axiom-agent@1.0.8
```

### New Features & Improvements

- **Automated `/update` Command**:
  - Typing `/update` inside chat now automatically detects the latest version from GitHub releases and performs the upgrade in place (`npm install -g axiom-agent@latest` for npm installations or native staged updater for standalone binaries).
  - Also added npm global installation handling to `axiom update install` CLI command.
- **Robust Model Variant Switching (`/variant`)**:
  - `/variant` menu and command now show the exact mapped model ID for each variant (e.g. `Switched variant to 'high' (model: nemotron-3-ultra-free)`).
  - Clarified variant behavior when a provider does not define a specific variant model (preserves current model with clear informative notification).
  - Updated default variant model mappings for `opencode` and `zen` to verified, available free models: `nemotron-3.5-lightning-free` (default & medium), `mimo-v2.5-free` (low & light), and `nemotron-3-ultra-free` (high).
- **OpenCode Zen Session Routing**:
  - Injected mandatory `x-opencode-session` header across all OpenCode Zen chat completion and model discovery requests.
- **Windows File Lock Tolerance During Updates**:
  - Made binary replacement in npm installer tolerate locked process files on Windows so updates complete cleanly while Axiom is open.

## 1.0.7

This release introduces OpenCode Zen and GMI Cloud provider presets, an interactive thinking/reasoning mode toggle with automated 400 parameter fallback, instant turn cancellation with cross-platform child process termination guards, live syntax-highlighted code creation animation, an autonomous auto-testing engine, and runtime provider management with live model discovery.

Install it:

```bash
npm install -g axiom-agent@1.0.7
```

### New Features & Improvements

- **OpenCode Zen & GMI Cloud Provider Integration**:
  - Added OpenCode Zen preset (`https://opencode.ai/zen/v1`), aliases (`zen`, `opencode-zen`), credential resolution (`OPENCODE_API_KEY`, `OPENCODE_ZEN_API_KEY`, `ZEN_API_KEY`), and default models (`claude-3-7-sonnet`, `deepseek-v4-flash-free`, `big-pickle`).
  - Added GMI Cloud preset (`https://api.gmi-serving.com/v1`), aliases (`gmi`, `gmi-cloud`), credential resolution (`GMI_CLOUD_API_KEY`, `GMI_API_KEY`), and default models (`deepseek-ai/DeepSeek-V4-Pro`, `meta-llama/Llama-3.3-70B-Instruct`).
- **Thinking / Reasoning Mode Toggle (`/thinking [on|off|auto]`)**:
  - Added dynamic thinking toggle across OpenAI-compatible and Anthropic endpoints.
  - Interactive selection modal in TTY or direct arguments (`/thinking on`, `/thinking off`, `/thinking auto`, `/thinking status`).
  - Added `/thinking` commands in Telegram and Discord gateway bots.
  - Built-in 400 rejection fallback automatically retries without `thinking` or `reasoning_effort` if the endpoint does not support reasoning parameters.
- **Instant Turn Cancellation & Process Termination (`Esc` & `Ctrl+C`)**:
  - Pressing `Esc` (ASCII 27) or `Ctrl+C` immediately aborts active generation turns via cross-platform non-blocking console input polling (`_kbhit` / `_getch` on Windows, `libc::poll` on Unix).
  - Implemented `ChildProcessGuard` with RAII `Drop` termination (`child.kill()` and `child.wait()`) preventing orphaned or runaway processes.
- **Live Animated File Creation & Syntax Highlighting**:
  - Replaced silent background writes with real-time typewriter line-by-line animated terminal rendering during `file.write`.
  - Colorized syntax highlighting for HTML, CSS, JavaScript, TypeScript, Rust, Python, JSON, and Markdown with line numbering and completion stats.
- **Autonomous Auto-Testing Engine (`test.run` & `/test`)**:
  - Built-in `test.run` skill with auto-detection for Cargo (`cargo test`), NPM (`npm test`), Python (`pytest`, `unittest`, or syntax check), and HTML/Web structural integrity.
  - System prompt mandates auto-testing after file writes and modifications.
  - User slash command `/test [command]` allows manual test triggering in chat.
- **Runtime Provider Management & Live Model Discovery (`/provider add`)**:
  - Added `/provider add` in interactive chat and `axiom provider add` in CLI.
  - Live model catalog auto-fetching (`GET <base_url>/models`) for both standard presets and custom OpenAI-compatible endpoints with search and graceful manual fallback.

## 1.0.6

This release brings a complete TUI interface overhaul, interactive Select and MCQ widgets, OpenCode-aligned model variants, multi-agent coder research & verification loop, built-in auto-update notifications, dynamic reasoning effort, anti-freeze streaming, and full gateway parity.

Install it:

```bash
npm install -g axiom-agent@1.0.6
```

### New Features & Improvements

- **Signature Axiom Obsidian TUI & Interactive Widgets**:
  - Implemented `SelectWidget` with dark obsidian aesthetic, ember accents, arrow-key navigation, type-to-filter search, and hotkeys.
  - Interactive Multiple-Choice Questions (MCQ form) with built-in `question.ask` skill for structured agent clarification with write-in fallbacks.
  - Redesigned banner with slate borders, high-contrast labels, and visual badges for provider (`⚡`), model (`🧠`), reasoning effort (`🔥`), workspace (`📁`), and session (`🔑`).
  - Polished terminal notices: cyan-accented `◈ Lens:`, emerald `✔` tool completions, and crisp ash thinking deltas.
- **OpenCode Model Variants Architecture**:
  - Replaced legacy tier models with OpenCode-aligned model variant configuration supporting reasoning efforts (`none`, `low`, `medium`, `high`, `max`).
  - Provider configurations now support custom variants, system prompts, and automatic fallback when upstream APIs reject reasoning parameters.
- **Autonomous Multi-Agent Coder Loop**:
  - Integrated research-first coding workflow that gathers current documentation and dependencies prior to implementation.
  - Secondary verification subagent reviews and checks generated code before final feedback is reported to the user.
- **Inbuilt Auto-Update Notifications**:
  - Startup check compares running version against npm registry and GitHub releases.
  - Displays a clean notification banner with direct instructions to run `axiom update` or `npm install -g axiom-agent`.
- **Command Prefix Normalization**:
  - Automatically normalizes `!` prefix commands (e.g., `!model`, `!effort`, `!help`) to slash commands in interactive chat.
- **Anti-Freeze Tool Streaming**:
  - Real-time terminal spinner displays live composed tool arguments (`Composing arguments for file.write (1.2 KB)...`), preventing apparent terminal freezes during tool call emission.
- **Telegram & Discord Gateway Parity**:
  - Added `/effort` and `/tier` slash commands in both Telegram and Discord gateways.
  - Gateway `/status` reports active provider, model, and reasoning effort.
  - Bot approver smoothly integrates with `question.ask` and all synthetic builtins.

## 1.0.5

This release enables autonomous shell command execution, local dev server hosting (e.g. `python -m http.server`, `vite`, `npm run dev`), terminal safe pasting, and accurate tool failure status reporting.

Install it:

```bash
npm install -g axiom-agent@1.0.5
```

### New Features & Improvements

- **Autonomous Shell & Terminal Execution (`shell.*` & `python.run`)**:
  - Registered full `ShellExecutor` in Axiom Engine supporting `shell.powershell.safe` (Windows), `shell.bash.safe` (Linux), `shell.zsh.safe` (macOS), `python.run`, and `shell.run`.
  - Workspace directory containment via `Workspace::resolve_inside` with credential environment variable sanitization.
  - Destructive system command protection (blocks disk wipes, system formatting, destructive directory removals).
- **Background Dev Server & Localhost Hosting**:
  - Direct support for persistent background daemon execution (`background: true`).
  - Automatic detection of local dev server commands (`http.server`, `vite`, `npm run dev`, `live-server`, `npx serve`, `next dev`, etc.).
  - Asynchronous stream capture monitors startup within initial window: if the process starts listening (e.g. `Serving HTTP on ...`, `http://localhost:...`), it detaches to background and returns the running PID and URL to the agent without blocking foreground turns.
- **Terminal Safe Pasting**:
  - Clean input sanitizer strips bracketed-paste escape sequences (`\x1b[200~`, `\x1b[201~`) and normalizes CRLF carriage returns across chat, confirmations (`[y/N]`), and onboarding prompts.
  - Multi-line pasting captures full multi-line code/prompts without premature single-line execution, displaying a clean indicator `📋 [Pasted N lines]`.
- **Accurate Tool Execution Status**:
  - Fixed misleading status reporting in CLI: `AgentTransitionKind::ToolCompleted` now accurately distinguishes `✔ Axiom Tool: completed` from `✖ Axiom Tool: failed`.
- **High-Agency Identity Operating Principles**:
  - Updated system prompt instructions to prioritize autonomous direct execution of commands, tests, and dev servers over lazily telling the user to run them in their terminal.
- **Core Skills Fallback**:
  - Ensured the platform's safe shell skill is always included in the fallback skill cards.

## 1.0.4

This release fixes terminal line-erasure on streaming completion and adds live streaming visibility for model thinking/reasoning processes.

Install it:

```bash
npm install -g axiom-agent@1.0.4
```

### Fixed & Improved

- **Fixed Disappearing Responses (Terminal Line-Erase Bug)**:
  - Eliminated redundant `Spinner::clear_line()` calls in `TerminalStreamRenderer::finish_line()`.
  - Single-line and short streaming responses are no longer wiped from the terminal screen when the stream concludes.
- **Thinking & Reasoning Process Visibility**:
  - Full support for live streaming of reasoning/thinking models (OpenAI-compatible `reasoning_content` and `reasoning` SSE deltas from NVIDIA NIM, DeepSeek, Groq, Ollama, OpenRouter, and Together AI).
  - Built-in projector extracts in-band `<think>...</think>` tags (from models like DeepSeek-R1 and Qwen-2.5-Coder-R1) and routes them into reasoning events while keeping visible output clean.
  - Subdued executive styling: Thinking processes stream under `💭 Thinking:` in subtle muted grey, cleanly finishing with a newline before the assistant's response (`◆ Axiom:`).
- **Dynamic Versioning in Dashboard Banner**:
  - CLI dashboard banner now dynamically reflects `CARGO_PKG_VERSION`.

## 1.0.3

This release resolves interactive confirmation UX and native tool calling compatibility across all LLM providers.

Install it:

```bash
npm install -g axiom-agent@1.0.3
```

### Fixed & Improved

- **Interactive Plan Confirmation**:
  - Replaced cryptic `[a/e/c]: ` prompt with an executive decision menu displaying clear options:
    `[1] Apply changes now`, `[2] Revise / edit plan`, `[3] Cancel`.
  - Pressing **Enter** defaults to **Apply** (`ApplyChoice::Apply`), preventing accidental cancellations.
  - Plan revision allows pressing Enter to keep current plan without token waste.
  - Cancelling or encountering errors during auto-routed tasks never exits to shell; it stays seamlessly in the chat session.
- **Universal Provider Tool Call Routing**:
  - `skill_id_for_native_tool` now handles all tool naming conventions used by diverse LLMs (including `web.fetch`, `axiom_web_fetch`, `web_fetch`, `axiom.web.fetch`).
  - Fixes `provider requested unknown Axiom function: web.fetch` when executing with models like NVIDIA Nemotron.

## 1.0.2

This release brings an autonomous harness overhaul inspired by Hermes Agent and OpenCode,
intelligent web research and search capabilities, model tiers with reasoning effort support,
and an executive terminal UI.

Install it:

```bash
npm install -g axiom-agent@1.0.2
```

### Added

- **Intelligent Web Fetch & Search Engine Routing**:
  - Safe redirect following: `web.fetch` now follows up to 5 HTTP redirects while enforcing
    HTTPS-only security, private-network SSRF protections, and pinned DNS resolutions at every hop.
  - Realistic browser headers: Configured modern `User-Agent` and `Accept` headers so public
    developer documentation, Modrinth, and technical sites return valid content rather than 403 Forbidden.
  - Automatic search engine query rewriting: Search queries and search engine targets (Google or DuckDuckGo)
    are transparently routed to server-side DuckDuckGo HTML rendering for instant, JS-free search results.
  - Direct `query` parameter support in `web.fetch`.
  - Expanded intent matching for "research", "lookup", "browse", "docs", "documentation", and "online".
  - Core workspace tool availability: `web.fetch` is now unconditionally retained alongside `file.read`,
    `file.write`, and `project.scan` so agents are never starved of reference capabilities.
- **Hermes Agent & OpenCode Autonomous Harness Alignment**:
  - High Agency identity: System prompt updated with "Think -> Look -> Act -> Verify" execution loop.
    Strictly forbids lazy placeholders (`// TODO`) and unprompted raw code dumping in chat.
  - Multi-syntax tool call parser: Hardened engine extraction to support ````axiom-tool`, ````axiom_tool`,
    ````tool-call`, ````tool`, and ````json` blocks.
  - Modal-free autonomy: Defaulted `auto_route_mode` to `"off"` to eliminate disruptive modals during
    autonomous runs.
- **Model Tiers & Reasoning Effort**:
  - Model tier presets (`light`, `medium`, `high`) configured across NVIDIA, Groq, OpenAI, Anthropic,
    and Cloudflare.
  - New chat commands `/tier <tier>` and `!tier <tier>` to switch reasoning models on the fly.
  - Configured `reasoning_effort` pass-through for providers supporting advanced reasoning budgets.
- **Executive Terminal UI**:
  - Modern dashboard banner with clean borders, active tier indicators, workspace status, and shortcut tips.
  - Dynamic spinner progression (`Thinking...` -> `Buffering response...`) with atomic line clearing
    to prevent terminal text collisions.
  - Modernized status indicators: `⚡ Axiom:`, `⚙ Axiom Tool:`, `✔ Axiom Tool: completed`, `🔍 Axiom: verifying`.

### Fixed

- **Stream connection timeouts**: Raised HTTP client timeout to 300s, added 15s TCP keepalive,
  and enabled gzip response decompression on reqwest.

## 1.0.1

This release delivers battle-tested stability and UX refinements based on real-world
first-run testing across platforms.

Install it:

```bash
npm install -g axiom-agent@1.0.1
```

### Added

- **Pure Agent Harness Execution**: Re-engineered system prompt and intent analysis
  so Axiom acts as an active workspace harness: when asked to create, make, or build
  code/games/apps/scripts, it directly writes the files to the workspace using `file.write`
  instead of just chatting code blocks.
- **Animated Rust TUI**: Added an async Braille dots animation spinner (`⠋ Thinking... (0.4s)`)
  with live elapsed timers and tool execution status that smoothly and atomically clears
  when assistant tokens stream in.
- **Terminal Screen Clear**: Terminal screen is automatically wiped on first launch
  (during onboarding), on regular `axiom` startup, and on typing `/clear` (`!clear`).
- **TUI Dashboard Banner**: Structured header display highlighting active provider,
  model, workspace, and session info.

### Fixed

- **Automatic binary download on launch**: If npm 11 `allowScripts` blocks or skips
  the `postinstall` script, `bin/axiom.js` now detects the missing binary and
  automatically downloads the matching platform release on first run.
- **Onboarding UX stream**: Removed redundant duplicate welcome banners, cleaned
  the onboarding sequence, and smoothly transitioned directly into chat.
- **Sequential provider menu**: Fixed provider list numbering to be strictly
  sequential (1 to 11) with flexible selection matching by number or name.
- **Provider model prefix deduplication**: Cleaned up duplicated provider prefixes
  in the status bar (e.g., `nvidia/nvidia/nemotron...` -> `nvidia/nemotron...`).
- **Token usage fallback**: Implemented automatic token estimation when streaming
  providers do not include token usage chunks, ensuring session stats never show 0/0.
- **`web.fetch` HTML cleanup**: Stripped `<script>`, `<style>`, `<noscript>`, `<svg>`,
  and tags, decoding entities and collapsing whitespace to avoid dumping raw minified
  code into context.
- **Clean tool output preview**: Suppressed disruptive multiline JSON preview dumps
  after assistant replies; replaced with a single inline status pointer (`!show <id>`).
- **Lens smalltalk bypass**: Lens now safely bypasses tool selection for greetings
  and identity prompts ("what's up", "who are you", etc.), eliminating spurious tool matches.

## 1.0.0

This is the first stable v1 release, promoted from `1.0.0-rc.1` with the
full friendly overhaul, messaging gateways, and safety fixes below.

Install it from the stable channel:

```bash
npm install -g axiom-agent
```

### Added

- Telegram and Discord messaging gateways: `axiom gateway run --telegram`
  and `--discord` with per-chat sessions, `/models`, `/model`, `/provider`,
  `/status`, and `/help` bot commands, chat allowlists, and fail-closed tool
  approvals in bot context.
- `axiom gateway status`, `setup`, and `disable` for token management, plus
  gateway state in `axiom doctor`.
- `axiom setup` as a friendly alias for re-running onboarding.
- `axiom uninstall` (with `--delete-config --yes` for a full local wipe).
- Linux ARM64 prebuilt binary (`axiom-aarch64-unknown-linux-gnu`) so npm
  installs work on ARM devices and Termux (via proot).
- File-backed credential fallback: pasted keys persist in a private `0600`
  file when no OS keychain exists, instead of being silently discarded.
- Loud key check at the end of onboarding and actionable credential errors
  in chat with copy-paste fixes.

### Changed

- Friendlier first run: guided 3-step onboarding, non-TTY guards that never
  hang CI, low-RAM guidance, and a "what next" summary.
- Skill Lens intent now covers rust/js/ts/go and more, routes project
  questions to `project.scan`, and skill manifests carry honest keywords.
- Prompt-only skills (`python.run`, shell safeties) no longer advertise
  built-in execution they do not have.
- `axiom doctor` is read-only (never creates the workspace) and friendlier.
- npm wrapper warns on `AXIOM_AGENT_BINARY_PATH` overrides; checksum
  verification streams in constant memory.

### Security

- Confirm prompts fail closed on EOF/piped input (never auto-approve).
- Bounded reads for sessions, cost ledger, and checksum files.
- Skill registry entries carry SHA-256 integrity hashes.

### Upgrade actions

- Re-run `axiom onboarding` (or `axiom setup`) once: it verifies saved keys
  and offers messaging setup. No config migration is required.

## 1.0.0-rc.1

This is the first public v1 release candidate. It brings the complete terminal
agent, safety model, recovery flow, provider onboarding, proof trail, and
release pipeline together for final field testing before the stable release.

### Added

- A bounded multi-step chat agent loop with tool observations and proof events.
- Native OpenAI-style tool calls, fenced fallback calls, SSE response and tool-call
  accumulation, retry/timeout handling, cancellation, todo updates, token-aware
  context compaction, and provider-reported token/cost accounting.
- Durable atomic chat sessions with `axiom sessions` and `axiom resume`.
- Transition-level session checkpoints containing tool events, approvals,
  policy decisions, usage, and pre-write workspace checkpoint references.
- Local UTC-month cost ledger, `axiom cost`, and optional per-session/monthly
  budgets shared by Chat and every Coder plan/patch/correction call; enforcement
  fails transparently when model token pricing is unavailable.
- Interactive `!multi` prompt capture with exact blank-line preservation,
  explicit submit/cancel controls, and a matching noninteractive `axiom run` path.
- Interactive line editing, bracketed paste, persistent history, safe live SSE
  rendering, semantic theme presets, and durable `!show` tool-output references.
- Conflict-aware hunk patches, workspace recovery checkpoints, project-aware
  tests, bounded correction attempts, and patch scope confirmations in Coder.
- Canonical capped Coder LLM calls, plan-to-patch path checks, and per-hunk/new
  file approval before patch application.
- Versioned skill/registry schemas, manifest-driven ranking, dependencies,
  typed built-in executors, and enforced skill-card budgets.
- Axiom identity context, word-boundary Lens matching, and the blood-red inline
  terminal theme with `NO_COLOR` support.
- Config schema versioning, `axiom config migrate`, and `axiom doctor --json`.
- The current config schema with centralized filesystem/network/process/Git
  `allow`/`ask`/`deny` policy, selectable terminal themes, and model-invoked
  `web.fetch` HTTPS/host/proxy controls.
- `docs/V1_PLAN.md`, which defines the v1 product boundary and release gates.
- Release SBOM generation, GitHub artifact attestations, and npm trusted-publisher
  workflow configuration without a long-lived npm token.
- Cargo advisory/license/source policy enforcement, locked dependency checks,
  and weekly Dependabot updates for Rust, npm, and GitHub Actions.
- Release tag/changelog/package validation and explicit beta/rc/latest npm
  dist-tag selection with `beta` as the safe publishing default.
- Release-bound npm publication that verifies the exact `v<version>` checkout,
  matching GitHub Release, all four binaries, `SHA256SUMS`, and npm version
  availability before entering the protected publish environment.
- A packed-tarball install smoke that uses `AXIOM_AGENT_BINARY_PATH` and invokes
  the installed global shim on the declared Node.js 20 minimum.
- Full locked-workspace release validation before builds, native release-binary
  E2E smoke tests, automatic GitHub prerelease classification, and fail-closed
  npm semantic-version/dist-tag matching.
- Bounded HTTPS-only npm installer downloads with a trusted GitHub redirect
  allowlist, request timeout, streamed size caps, exclusive temporary files,
  checksum-before-install enforcement, and rollback-safe replacement.
- First-class Groq, OpenRouter, Gemini, GitHub Models, NVIDIA NIM, OpenAI,
  Ollama, and LM Studio onboarding presets, including optional authentication for trusted
  local OpenAI-compatible servers and rate-limited free defaults where safe.
- Deterministic malformed-input/property corpora for patch and tool request
  parsing, SSE framing, workspace path containment, and proof redaction.
- Provider readiness diagnostics in human and JSON `axiom doctor` output,
  including missing key-variable detection without secret disclosure.
- Guided one-or-two-provider onboarding with hidden credential paste, native OS
  credential storage, catalog-only model discovery/search, custom catalog URLs,
  per-provider model memory, and `axiom model`/`axiom provider` commands.
- Deterministic credential-store and model-catalog failure coverage for missing
  stores, malformed/empty catalogs, authentication, rate limiting, and size caps.
- An executable-embedded essential skill registry materialized under the user
  config directory, with relocated-binary E2E coverage for offline onboarding.
- Thirty-day default Proof retention for new configurations, legacy-safe
  disabled retention until opt-in, junction/symlink-safe automatic pruning, and
  an explicit privacy warning on export.

### Changed

- First-run no-argument startup now continues from onboarding and local doctor
  checks directly into terminal chat. Coder plans can be revised in place, and
  provider switching clears an incompatible stale model selection.
- Rerunning onboarding migrates legacy config and preserves existing policy,
  network, registry, UI, Coder, Proof, agent-cap, and update settings.
- One-shot provider overrides restore that provider's saved model, catalog
  displays are filterable and capped at 100 matches, incomplete provider setup
  stays in onboarding, and Cloudflare noninteractive setup requires an account ID.
- The duplicate in-repository skill manifests were removed; the published
  `axiom-skills` registry is the manifest source of truth.
- Version synchronization now covers every internal Cargo path dependency's
  exact pin and every workspace package entry in `Cargo.lock`; Linux x86-64
  release binaries use an Ubuntu 22.04/glibc 2.35 compatibility floor.

### Security

- Documentation now distinguishes executable built-ins from prompt-only skill
  cards so installed metadata is not mistaken for executable code.
- State files and tool writes use atomic replacement; Windows replacements use
  replace-existing/write-through semantics and Unix replacements sync the parent.
- `web.fetch` requires HTTPS and disables system proxy discovery by default;
  deny-first exact/wildcard host policy cannot override private/loopback hard
  blocks. Public DNS results are pinned, redirects and embedded credentials are
  rejected, and response limits are enforced while reading the body.
- Remote provider and model-catalog endpoints require HTTPS; plain HTTP is
  limited to literal loopback hosts. Provider URLs reject embedded credentials,
  query strings, and fragments, and provider clients do not follow redirects.
- Credential-variable names are validated and cannot replace process-control,
  dynamic-loader, proxy, or Axiom home variables.
- Native-keyring and environment credentials are passed directly to provider
  clients instead of hydrating process-global state. Test, diagnostic, and Git
  children scrub every configured provider credential name; Git diff disables
  external diff and textconv drivers.
- Secret-file policy is centralized and applied before and after canonical path
  resolution, closing symlink/junction aliases. Git inspection excludes those
  paths before capture and drains stdout/stderr with bounded retention.
- Secret directories and case variants are blocked consistently; Git exclusions
  are case-insensitive, recursive, and disable fsmonitor as well as external diff
  and text-conversion hooks.
- Proof prompts, terminal/session history, transition state, and saved tool
  outputs receive mandatory exact-value/token-shape redaction before durable
  persistence, even when a legacy config requests redaction off.
- Proof applies recursive redaction at the final persistence/export boundary so
  provider, model, path, approval, command, Lens, policy, and nested metadata
  cannot bypass recorder-level capture helpers.
- Provider success/error bodies, SSE events, aggregate assistant text, and tool
  arguments have explicit byte/count caps. Provider clients ignore system proxy
  discovery, including authenticated loopback endpoints.
- Proof export recognizes camelCase credential keys, preserves structural trace
  discriminators, avoids redacting semantic `sk-*` identifiers, and cannot
  prune through symlinked or Windows junctioned proof directories.
- The Rust updater binds metadata/assets to the exact GitHub repository, tag,
  and filename, separates metadata/checksum/binary size limits, rejects unsafe
  local source shadowing, verifies the installed semantic version exactly, and
  reports rollback failures.
- Built-in side effects pass through a centralized policy evaluator and record
  the decision in Proof/session state; Unix private atomic state files use
  restrictive permissions.

## 0.5.1-beta

- Republished the npm beta package with synchronized Cargo and npm versions.

## 0.5.0-beta

- Initial terminal CLI, skill registry, proof mode, coding mode, updater, npm
  installer, offline mock demos, and release-safety checks.
