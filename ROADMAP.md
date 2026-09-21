# Roadmap

Living plan for Ryter. The lead updates this as work lands.

## Now

- 0.2.0 TUI redesign (`design.md`) — all seven phases landed on `0.2.0-patch`; PR to `dev` for outside review, then `dev` → `main`
- `gaps.md` review closed: G-01..G-07 all fixed. One new item opened there — S-01, the snapshot harness cannot see style, so `R-POP-04` (dim behind a panel) is unassertable
- Resolve `design.md` open items in review: O-01 drop `Spend` as the `busy` fallback once headless/MCP consumers read `TurnFinished`; O-02 `ryter doctor --json` (CLI, not the panel); O-03 `light` theme shipped but marked experimental; O-04 per-message copy deferred with the clipboard question (mitigated: `Ctrl+G` releases the mouse so terminal selection works)

## Next

Product direction: `docs/product-direction.md`. Crew contract: `crew.md`. Cost model: `docs/cost.md`.

- **Grow the benchmark, then run it.** `ryter bench` and a 4-task starter suite exist; before its numbers mean much it needs multi-file and multi-task (parallel) cases, a Rust and a TypeScript task, and a run per candidate tiering. Then fold the measurements back into `docs/cost.md` and the suggester's ceiling and band.
- **Tiered defaults at first run.** `ryter crew suggest` exists; first-run setup and the builds-paused message should offer it directly rather than pointing at it.
- **Crew spend where people look.** Per-role lines (builder / auditor / architect) on the spend card and `/spend`; per-task cost on the crew review surface. A cost preview before a batch, with a threshold that asks.
- **`/patch` surface.** Show the open patch (tasks, what it waits on), land now, drop it.
- **Checks auto-detect** on first use (`Cargo.toml` → `cargo test`, `package.json` → its test script, `pyproject.toml` → `pytest`, `go.mod` → `go test`), written to `.ryter/config.toml` after the user confirms.
- **Recorded wire fixtures + a live smoke test.** Tool calling was broken on two backends while 212 tests passed, because every test used idealized deltas. Record real SSE per provider (tool calls, parallel calls, truncation) and replay those; add one nightly live round trip per built-in provider.
- **Crew review surface.** One panel for every task: state, diff, handback, checks, audit, cost; merge a waiting branch, retry with a note, reject, revert a merged task (`git revert -m 1`). The tasks card opens it.
- **Refuse unpriced models** in the crew without an explicit opt-in: the dollar caps cannot trip on unknown spend (the token cap still does).
- **Keep specialist transcripts** in the session directory. Live run 3's auditor was refused a command, and there was no record of which one.
- **From the live runs:** a task is recorded as landed on the patch only when its whole batch finishes (batch barrier); parallel jobs can overshoot the session budget by about one round each (~$0.10 seen); the retry brief says "start clean" but the worktree is reused; the architect is the slowest and dearest role (~$0.70 per design on Opus), so measure whether a cheaper architect designs as well.
- **Streamed `bash` output**, and **reconcile the context gauge** with the provider's real `input_tokens` rather than bytes/4.
- **`ryter run tasks.toml`** unattended, producing branches or PRs with the audit as the description.
- **macOS without the sandbox**, labelled Linux-only.
- Anthropic extended thinking + tools (thinking blocks must be echoed back on `messages`); compaction that keeps a summary rather than a file list; `ryter-cli` tests; confirm grok-4.6's context window (500k in `window_for`, 256k in fixtures).

Previously listed:

- 0.2.1 — light-theme contrast pass on real terminals, kitty keyboard protocol for Shift+Enter where the terminal supports it, `/theme` custom `~/.ryter/themes/*.toml` preview of the full syntax palette
- 0.3.0 — Landlock default-on once it is boring; consider ABI V4 for `AccessNet` (the sandbox is filesystem-only today)
- 0.3.0 — headless `ryter -p` output that reuses the TUI markdown renderer for `--pretty`

## Later

- macOS / Windows
- SQLite FTS over notes when DECISIONS.md outgrows a prompt
- Session search across transcripts from `/sessions`

## Done

### 0.2.0-patch — the lead routes the crew; live-tested (2026-09-21)

- One conversation with the lead; phases, `/plan` `/build` `/handoff`, `--mode`, and the MCP phase tool removed. Tasks carry a role; the architect's tasks build in the same pass; `hold` for design-only
- Five paid live runs on a real crew (Opus architect, grok-4.6 builder, glm-5.3 auditor, deepseek lead), about $4.3 in total. Run 3: a precise two-module request; the lead wrote the tasks, two builders ran in parallel, both were audited, one patch commit, 25 tests, $0.15 in 2 minutes. Runs 4–5: a request that needed a design; the architect split it into three tasks, the $1.40 cap stopped the third, and `ryter -c -p continue` finished it; one 12-file commit, 76 tests, a working CLI, $1.72, of which $0.91 was the architect
- Fixed from those runs: tool errors ending tasks, cut-off replies lost, empty architect results counted as done, audit probes and `.pyc` files committed, builders serialized on blocking tools, crew reports overwritten, auditors re-running checks, inline-code refusals that gave no way round, the lead not knowing the date, a budget stop that said only "budget exceeded" and left the lead unaware on the next message, no way to continue a session headless, an architect that wrote its design three times
- Specialists report activity to the chat as they work

### 0.2.0-patch — local models, tiered crews, benchmark (2026-09-21)

- `local` connections (Ollama / LM Studio / llama.cpp): keyless, $0, fail fast when down; Anthropic endpoint fixed; first socket-level provider test
- `ryter crew suggest [--apply]` and `s` in `/crew`: cheap builder, independent strong auditor, strong architect, fenced against price-only traps found on a live catalog
- `ryter bench`: real crew, fresh repos, hidden acceptance tests, false passes, cost per accepted task; starter suite with a soundness test
- Crew spend written to disk as charged, so an interrupted run still has a record

### 0.2.0-patch — cost, patch, independent sign-off (2026-09-21)

- Crew metered: every specialist round priced, attributed, logged, and counted against the budget; per-task USD and token caps
- Scoped specialist context (~10k → ~1–2k tokens/round for builders on this repo); stable per-turn system prompt; rolling cache breakpoint on Anthropic routes; retry in place; per-role limits
- Patch as the unit: tasks land on an integration branch; the user's branch gets one commit when every task is done and the combined checks pass
- Auditors must differ from lead and builder; auditor panel with focus/paths, first FAIL stops
- `propose_edit` fast path, approved by a person
- `docs/cost.md` cost model

### 0.2.0-patch — crew rebuilt around sign-off (2026-09-21)

- Streamed tool calls reassemble on every backend; Anthropic models can call tools (they could not); Messages specialists receive their role prompt
- Build pipeline: integrate into the worktree, harness-run checks, auditor `VERDICT: PASS`, serialized `--no-ff` land; nothing lands with the auditor off; conflicts never touch the user's checkout and are re-audited
- Tasks carry `brief` + `files`; disjoint scopes run in parallel; the architect writes the real queue
- Planner folded into architect; phases plan → build → audit, old names still parse
- Serial memory writers; builder handback contract; crew report returned to the orchestrator
- All four prompts rewritten with output contracts pinned by tests; `grep` gains `path` / `include`
- `crew.md` design contract; `docs/product-direction.md`

### 0.2.0-patch — review fixes (2026-09-21)

- Tool calling on the Responses and Messages backends: `tool_calls` / `tool_call_id` were dropped on the request side, so every agentic turn past the first shipped an empty assistant message and an invalid `role: "tool"`. Broken out of the box, since the default connection is spacexai/responses. Per-backend body tests added.
- Permission gate rewritten to judge shell commands per segment; 13 table-driven tests where there were none.
- Safety coherence: `Enter` no longer allows on the permission modal, the auditor no longer bypasses the gate, auto-merge refuses a dirty checkout and lands `--no-ff` with a recorded undo sha.
- Provider reliability: retry with backoff and `Retry-After` on transient failures; idle-read timeout instead of a total request deadline.
- Cost control: every tool result capped at 32k keeping both ends, `read_file` pages with `offset`/`limit`, prompt caching on the Messages backend, truncation surfaced as `StopReason::Truncated`.
- Shell usable for real builds: `-c` instead of `-lc`, 120s default with a per-call `timeout_secs`.
- Secrets: a key lives in one store (keyring xor 0600 file, reported to the user), the SSRF guard resolves DNS and unwraps IPv4-mapped IPv6, the Landlock writable set no longer includes `keys/`.
- TUI: panels own the body (G-01/G-02/G-06), unknown spend renders `?%` not `$0.00` (G-04), empty cards absent (G-03), hint bar keeps cancel/quit (G-05), session card leads with title+phase (G-07), `Ctrl+G` releases the mouse for text selection.

### 0.2.0 — TUI redesign (2026-09-20)

- Phase 1 Foundation: `Message` model, unicode-aware wrapping, LRU render cache, `ChatScroll` with stick-to-bottom + auto-release, sticky turn header, scrollbar
- Phase 2 Reading: speaker headers (`[ui] username` → git → `$USER` → `you`), markdown block+inline renderer, syntect + two-face highlighting on Ryter’s palette, extended theme with light/dark/16-color/`NO_COLOR` degradation
- Phase 3 Writing: multiline composer with bracketed paste, prompt history, secret mode, single `KEYMAP` table, context-aware hint bar
- Phase 4 Awareness: activity strip (collapsed ticker / `Ctrl+R` pane), `TurnStarted`/`TurnFinished` core events with duration and breakdown, card-based info panel with drop order
- Phase 5 Command surface: `CommandSpec` registry, fuzzy command palette with categories and keybinding column, `PanelStack` framework, shared chrome, form widget kit
- Phase 6 Panels: every slash command is a bordered panel (`/settings` `/provider` `/models` `/crew` `/mcp` `/skills` `/hooks` `/sessions` `/agents` `/spend` `/theme` `/tools` `/auditor` `/phase`), new `/context` `/help` `/doctor`; permission / ask / trust are modal interrupts; inline ask bar removed
- Phase 7 Polish: `[ui]` config section, golden snapshot suite at 80×24 / 100×30 / 120×40 / 160×50, redraw budget test, `RYTER.md` chrome rule replaced, decisions recorded

### 0.1.x

- Rust workspace, BYOK SpaceXAI + OpenRouter, spend `$?.??`
- Session, agent loop, TUI, worktrees + auditor auto-merge
- MCP inbound/outbound, skills, hooks, themes
- Compact, doctor, Landlock opt-in
- Last-used provider/model, catalog price/window on load
- Project memory: ROADMAP + DECISIONS + notes
- Planner and architect spawn from the queue; tagged specialist chat; short handback
- Per-role `[specialists.*]` routing; queue batches run in parallel up to `max`
- `/crew` assigns models per role; factory default follows the orchestrator
- Real cancel (Esc / `/cancel` / `ryter_cancel`) kills bash process groups
- `ryter serve --socket` / `--bind` + token; TUI attaches on `~/.ryter/ryter.sock`
- Session `/resume` `/rename` `/delete`; `ryter sessions` / `ryter resume`
- `/agents` roster; Enter kills one specialist
- `/mcp` floating inbound/outbound menu, bearer tokens, client links
- `/skills` and `/hooks` floating menus (list, add stub / add hook, remove)
- TUI permission Ask (`y`/`n`/`a`) and `ask_user`
- `/spend` table overlay; `/settings` (budget, warn, max, sandbox, inbound, web)
- `ryter connections` list/add/remove/test; `ryter models`
- First-run writes `~/.ryter/config.toml`; TUI “trust this project?”
- Extra builder turn on merge conflict; `~/.ryter/logs/ryter.log`
- `[features] web` + `web_fetch` / `web_search` (SSRF-blocked)

## Blocked

- (none)
