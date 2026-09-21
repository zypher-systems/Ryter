# Roadmap

Living plan for Ryter. Orchestrator and specialists update this as work lands.

## Now

- 0.2.0 TUI redesign (`design.md`) — all seven phases landed on `0.2.0-patch`; PR to `dev` for outside review, then `dev` → `main`
- `gaps.md` review closed: G-01..G-07 all fixed. One new item opened there — S-01, the snapshot harness cannot see style, so `R-POP-04` (dim behind a panel) is unassertable
- Resolve `design.md` open items in review: O-01 drop `Spend` as the `busy` fallback once headless/MCP consumers read `TurnFinished`; O-02 `ryter doctor --json` (CLI, not the panel); O-03 `light` theme shipped but marked experimental; O-04 per-message copy deferred with the clipboard question (mitigated: `Ctrl+G` releases the mouse so terminal selection works)

## Next

Product direction and its reasoning: `docs/product-direction.md`. Crew contract: `crew.md`.

- **Make "tested and independently reviewed" true by default.** Checks auto-detect on first use (`Cargo.toml` → `cargo test`, `package.json` → its test script, `pyproject.toml` → `pytest`, `go.mod` → `go test`), written to `.ryter/config.toml` after the user confirms. First-run setup puts the auditor on a different model or provider from the builder; warn in `/crew` and `doctor` when they match.
- **Recorded wire fixtures + a live smoke test.** Tool calling was broken on two backends while 212 tests passed, because every test used idealized deltas. Record real SSE per provider (tool calls, parallel calls, truncation) and replay those; add one nightly live round trip per built-in provider.
- **Crew review surface.** One panel for every task: state, diff, handback, checks, audit, cost; merge a waiting branch, retry with a note, reject, revert a merged task (`git revert -m 1`). The tasks card opens it.
- **Cost per task and a cap per task.** A crew multiplies spend. Also: refuse unpriced models without an explicit opt-in, since `over_budget` cannot trip on unknown spend.
- **Task benchmark.** ~20 real tasks in fixture repos, end to end: landed / rejected / conflicted, cost per landed task, wall time, per model pairing. Tune prompts and default pairings against it.
- **Fast path for trivial edits.** Orchestrator proposes a small diff, applied on `y` (user approval is sign-off). Open question in `crew.md`.
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
