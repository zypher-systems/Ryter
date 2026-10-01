# Roadmap

Living plan for Ryter. The lead updates this as work lands.

## What 1.0 means

1.0 is a finished, workable product: someone new installs Ryter, points a crew at a real project, and can stop watching it. Every item below has to be true first. Until then releases go 0.8, 0.9, 0.10 and on, since minor numbers have no ceiling. When the list is done, the user decides whether that is 1.0.

- [ ] **Setup works the first time.** Install, add a key, choose a crew, and land a first task on Linux and macOS, with no step the guide doesn't cover. Only models the account can use are offered (done in 0.6.5).
- [ ] **Review means something.** The crew can look at what it built before it signs off: the terminal viewer for terminal apps, the browser for web apps, with screenshots for models that take images.
- [ ] **Ryter only touches the project it was started in.** It never creates a repository in a folder that holds other projects (done in 0.8.1), never commits outside the project, and says what it set up. (On 2026-09-28 it made `~/workspace` a repository and committed 29 other projects into it.)
- [ ] **Cost is predictable.** A task's cap asks before it stops (done in 0.7.1), project cost counts only the project, and a crew run shows what it is likely to cost before it starts.
- [ ] **A failed run loses nothing that was paid for.** Work stays on its branch and resumes where it stopped (done in 0.6.5 and 0.7.1), and no failure path deletes committed work.
- [ ] **The benchmark says it works.** `ryter bench` runs a suite of real multi-file tasks. The land rate and the cost per task are published, and every release is checked against them.
- [ ] **The model has the tools the job needs:** skills it loads itself, pages it can show the user, and images it can see.
- [ ] **No known data-loss or wrong-number bugs are open.**
- [ ] **The guide matches the product:** every command and screen in `docs/guide.md` works as the release does.

## Now

Work that is in `dev` and not yet released is marked *Unreleased*, and its notes collect in `docs/releases/next.md`. An entry gets its version when it's released.

- **Unreleased: your rules for every project.** `~/.ryter/RYTER.md` goes into every role's instructions, ahead of the project's `RYTER.md`. The model changes it with `update_rules` (taught by the built-in `rules` skill, also `/rules`), and only after the user has seen the change and said yes: `--always-approve` doesn't skip it, a headless run saves nothing, and the sandbox keeps the file read-only.
- **Unreleased: the shell tests are race-free.** `a_detached_process_holding_the_pipe_does_not_hang` raced `setsid` against the group kill under CI load. Its job now leaves the group with `set -m` and writes its pid before the command goes on. Both background tests find and stop their process by that pid (`rustix`), not `pkill -f`, `pgrep -f` or `setsid`.
- **0.9.1 (`0.9.1-patch`) — the crew is chosen in one visit to `/models`.** Two panes: the seats (lead or solo, architect, builder, auditor) with their models, and the chosen seat's models. Enter sets the model and goes back to the seats on the next one, rather than closing. The panel chrome gained `input_indent` (the search row beside a left column) and `keys_in_body` (per-pane key hints). Also the first release that 0.9.0 can update to.
- **0.9.0 (`0.9.0-patch`) — Ryter updates itself.**
  - At launch, at most once a day, Ryter installs a newer signed release in the background and says to restart. `[update] mode`: install, notify, or off.
  - `ryter update [--check]` does the same on demand.
  - Releases are signed: the workflow signs `SHA256SUMS` with the `RYTER_SIGNING_KEY` secret, and Ryter checks the signature against `release/ryter-release.pub.pem` before it installs. Only release-workflow builds replace themselves.
- **0.8.2 (`0.8.2-patch`) — an MCP server's last words.** `LiveServer::gone` waits up to 500 ms for the stderr reader to reach its end (`stderr_done`) and for the child to be reaped, so the exit reason reaches the error. Found by a flaky CI run of `a_server_that_exits_says_so` on 0.8.1.
- **0.8.1 (`0.8.1-patch`) — a folder of projects stays as it is.**
  - `git::holds_repos` finds repositories in a folder's folders, or one level down, when the folder isn't a repository with commits. It skips only folders the folder's own ignore rules leave out, in git's order of precedence (with the first `.gitignore` when Ryter will write it). `ensure_repo` refuses a first commit there, and git checks again at any depth by staging into a private copy of the index. A crew turn stops before its first model call. Solo mode edits without `/undo` there.
  - The rail and the `$` drawer name the folder the project cost is counted in when it isn't the one Ryter runs in (`View::project_root`).
  - A failed turn's error reaches the screen before the turn closes, so it reads `✕ failed`.
  - From the 0.8.0 review: headless pages say headless, and `load_skill` refuses hidden files.
- **0.8.0 (`0.8.0-patch`) — skills the model loads, and pages.**
  - Skills are listed in the solo and lead prompts (name and description), and `load_skill` reads one. `model-invocable: false` keeps one to the palette.
  - A built-in `canvas` skill, and a `show_page` tool that saves a page to `~/.ryter/pages/<session>/`, sealed from the network, and opens it (`[ui] open_pages`).
  - Covers part of the 1.0 item "the model has the tools the job needs". Next on it: images end to end, a terminal-app viewer, and a browser.
- **0.7.1 (`fix/0.7.1`) — caps that ask, retries that resume, rejections counted.**
  - At its dollar cap a task asks the user (`crew::raise_cap`: +$5, +$2, or stop) and carries on in place when raised. `Meter` keeps raised caps per task, and `Task.cap_usd` keeps them across runs.
  - A task still at its cap asks, or stops, before spending anything.
  - A stop after the build is committed sets `Task.gate_next` with the handback, and the next run skips the builder.
  - Rejections are counted in all per task (`Meta.rejections`), shown on the lane card, and at every third the user is told to choose a stronger builder. Ryter names no model and switches nothing.
  - `/models` has a tab per seat (lead or solo, architect, builder, auditor; `←→`), and `/crew` only switches the mode. The ready-made and saved crews are `/crews`.
  - Found in a $9.22 run: five rejections, a cap stop, and the lead recreating the task and rebuilding it.
- **0.7.0 (`feat/0.7.0`) — live lanes and the rail.** Chosen from eight designs on the canvas: C1 for crew, S2 for solo.
  - **Engine:** specialists send `SubagentLive` while they stream: waiting, thinking, writing, or running, with the output's last three lines. Reports are throttled to 200 ms, and a change of phase is sent at once. `bash` passes its newest output through `ToolContext.live`.
  - **Board:** a card per worker with a chip, a meter and the output tail; a PULSE tile; spend in the last minute; `^r` hides reasoning. Cards shrink to fit.
  - **Solo:** a rail with the name, the session, the hat as a colour block, the model and context, spend by turn, and the last turn's files. The prompt is boxed in the hat's colour. `^b` and `[ui] panel` drive it.
  - Found when a crew run sat still for ten minutes while its architect thought, and the solo screen had no name and the costs at the bottom.
- **0.6.5 (`fix/0.6.5`) — only models the account can use.** OpenRouter's models come from `/models/user`, which applies the account's privacy settings, provider rules, and guardrails, with tools filtered client-side. The full tool list is the fallback.
  - Before a crew run, `Agent::refused_seats` asks each connection's `Provider::refused` about the seats it will call: in the catalog but not on the account's list. A refused seat pauses the crew before anyone is paid.
  - An auditor whose provider fails is `SignOff::Unreachable`, and the task keeps its branch. Any non-cancel failure after the builder committed keeps it too.
  - A paused crew is drained once a turn.
  - Found when a run paid $4.43 for design and build, then the auditor (claude-fable-5.1) was refused under zero data retention and the branch was deleted.
- **0.6.4 (`fix/0.6.4`) — the model picker opens at once.** OpenRouter's models are listed with `?supported_parameters=tools` (0.16 s, while the full catalog stalled for minutes).
  - Each connection's last model list is kept (`llm::model_cache`), and the picker opens on it while a fresh one downloads on a thread of its own, with a client of its own.
  - After 30 s the picker says the provider is slow; the download goes on for up to 5 minutes and fills the cache.
  - Turns no longer wait behind a model list.
  - Found when OpenRouter's `/models` took 75 s and more to trickle in; Reeve looked instant because it kept its last list.
- **0.6.3 (`fix/0.6.3`) — mission control as designed.** The crew screen is design D's layout:
  - tiles for SPEND, TASKS, CHECKS, ELAPSED;
  - the plan on the left at full height, with a legend at its foot;
  - on the right, the lanes, each with its task's cost, and the LEAD box holding the conversation and the prompt;
  - the strip names the seats' models and the patch.

  `tab` picks a lane and `⏎` opens its transcript. The bar says `esc stop the crew`. A blocked task's reason is the audit's first finding, not its `[model · review]` header. Left out of the mockup on purpose: lane progress bars (there is no real measure of progress) and `p plan`.
- **0.6.2 (`fix/0.6.2`) — the plan is drawn.** Mission control's plan is boxes in their state's color, left to right by what waits on what, joined by `──┬─▶` / `└─▶`, as the design showed. A blocked task's full reason, and a task's other prerequisites, are listed under the drawing. A plan too big for the space falls back to the tree, and a long plan keeps its top rows with `… N more rows` instead of losing them.
- **0.6.1 (`fix/0.6.1`) — the views are in sight.** A strip across the top of the ledger names the views, `chat`, `changes ^t`, `crew board` (`/crew` in solo mode), and lights the one on screen. In crew mode the board shows before there's a plan, and the workbench replaces it rather than sitting under it. `^t` is in the bar's keys. In 0.6.0 the workbench was behind an unadvertised key, and a crew session that started with a question showed no board, so it looked like there was one view.
- **0.6.0 (`feat/0.6.0`) — the ledger layout.** The default screen is one reading column on a timeline:
  - questions `●`, model text `◆`, tool steps `├─` led by dots to what came of them, and a measured closing line `└─` per turn;
  - finished turns fold to one line (`^o` opens them);
  - a borderless prompt, and one bottom bar with the mode, project, model, context, and turn, session, and project cost against the budget;
  - `$` opens a spend drawer.

  `[ui] layout = "classic"` keeps the 0.5 screen, switchable in `/settings`.

  **Mission control** in crew mode: spend, tasks, patch, and time tiles; the plan as a tree drawn from `AgentEvent::Tasks` queue snapshots; one lane per worker. Specialist activity is kept in `activity.jsonl`.

  **The workbench** (`^T`, `/changes` on the ledger): changes, commands, and turns beside the chat and a diff inspector, with per-change undo (`review::revert_hunk`, recorded for `/undo`), file undo, and turn undo.

  All three designs (A, B, D) are from the design canvas.
- **0.5.7 (`fix/0.5.7`) — crew builds in order.** Tasks take `after` (ids that must land first); in a project with no build manifest, the task that creates it runs first and alone. Tasks waiting on a blocked one never start, and the lead is told what they wait on once, not after every reply. A builder's `STATUS: BLOCKED` skips checks, audit, and retries and carries its fix to the lead. Auditors may answer `VERDICT: UNVERIFIED` when the code can't be built yet; such work lands on the patch marked, and the patch needs checks to land. With no checks set, Ryter detects them from the manifest and asks once. From a real run that spent $2.29, mostly on retries against a scaffold that never landed.
- **0.5.6 (`fix/0.5.6`) — outbound MCP that doesn't hang.** Each server's stdout is read on its own thread; a call waits with a deadline (`timeout_secs`, default 120s; 60s to start) and stops on Esc, sending `notifications/cancelled`. Replies are matched by id: notifications, log lines, and late replies are skipped, and server requests are answered (`ping`) or refused. The hub is locked only to route a call, so one slow server doesn't stall the rest. `isError` results are errors; an exited server is named with its last stderr line. `search_tool` matches every word and lists each tool's arguments. `@modelcontextprotocol/server-everything` failed to connect before (a notification arrived ahead of the `initialize` reply). Inbound MCP is still to review.
- **0.5.5 (`fix/0.5.5`) — spend counts what you pay.** Anthropic prompts are counted whole: cache reads and cache writes (1.25× input, never counted before) are priced at their own rates, and OpenRouter's cache rates are read from its catalog. With a session budget set, a model with no price is not called again after its first unpriced call, in solo, lead, and crew; the reply that came back is kept. A crew task's caps count what it spent on earlier runs (`Task.spent`).
- **0.5.4 (`fix/0.5.4`) — model lists show only models that can chat.** `:batch` routes and image or audio models are left out of `/models`, the `/audit` chooser, and `ryter models`, read from the catalog's output types (id patterns where a catalog gives none); router models such as `openrouter/auto` stay.
- **0.5.3 (`fix/0.5.3`) — approving like Reeve, clearer menus.** The approval card docks above the composer (what, why, risk, change); `⏎` allows after a 500 ms guard, `a` allows one named kind of action for the session, `n` denies, destructive commands take only `y`. Panel keys on the panel's last row in color; search inside the model picker and help; the picker opens on the current model. Fixed: the picker's `$-1000000/M` footer for routers, `/provider` showing a connection's default model instead of the one in use, and form keys in the hint bar during a prompt. The look (palette) is still to decide: options on the comparison page, Copper proposed.

- **0.5.2 (`fix/0.5.2`) — `/undo` puts back what the turn changed, and only that.** The user's own edits since stay; `/undo` refuses over them unless forced; `/redo` reverses an undo; gitignored files the model writes are saved first.

- **0.5.1 (`fix/0.5.1`) — `/audit`, offered after changes.** `/second` is now `/audit` (kept as an alias), and Ryter offers an audit when a build turn changes files: `y` audit, `n` pass, `s` stop offering.

- **0.5.0 (`feat/0.5.0`, notes in `docs/releases/v0.5.0.md`) — `/second`, a second opinion in solo** (2026-09-27; DECISIONS entry of that date). The user chooses the reviewer from the live catalog, with each model priced for the review, and a dollar limit per review. Every review asks first, and the limit is kept before each step. Tried live: grok-4.7 caught both planted bugs in a `median` (a string sort, and the even-length case) for $0.02 of a $1.00 limit, and "fix those" fixed them with tests. Crew work is frozen except for bugs until a solo-versus-crew benchmark says it earns its setup.
- **0.4.0 (`feat/0.4.0`, notes in `docs/releases/v0.4.0.md`) — turns that end, and edits you can see** (2026-09-26; see the two DECISIONS entries of that date). Fixed, each with a test that failed first:
  - a waiting patch looped the lead to the round cap on every message;
  - keep-alives held a stuck stream open for good;
  - the stream waited for socket EOF after `[DONE]`;
  - errors sent mid-stream became empty "completed" replies;
  - characters split across chunks were corrupted;
  - Anthropic input tokens were dropped from spend;
  - Esc mid-command left the session unusable, and nothing repaired it;
  - the same failing call ran to the round cap;
  - the round cap stopped silently;
  - prompts ignored Esc and `^c`;
  - `git commit` without `-m` waited on vi;
  - `search_replace` misses gave no location, and CRLF files never matched;
  - an interrupted task stayed `running` for good;
  - the permission prompt cut a `propose_edit` to 160 characters.

  New: inline diffs in the chat, `^o`, and diff previews on edit prompts.
- **0.4.0 — the shell gate reads a command the way the shell will** (2026-09-26; DECISIONS entry of that date). Closed, each with a test:
  - the never-run list was bypassed through wrappers (`env -i sudo`, `timeout 5 sudo`, `nice dd`, `xargs ssh`), escapes (`s\udo`), computed names (`$(echo sudo) ls`, `$X`), `eval`, `find -exec`, `fd -x`, `busybox`, and `git -c alias.x=!…`;
  - the review hat ran `cargo fmt`, `npm install`, `npx <pkg>`, `make install`, and `node -e`;
  - read-only tools ran programs (`sort --compress-program`, `rg --pre`, `git grep -O`) or wrote (`yq -i`, `git diff --output`);
  - bare `env` was read-only.
- **Security, still open:** the never-run list doesn't bind a builder that writes a script (Landlock is the boundary, and it's off by default); shell reads can reach `.env` indirectly (`grep -r`, `xargs cat`); environment secrets other than Ryter's keys are visible to commands; no independent adversarial pass of the new gate yet.
- **Audit findings still open** (verified 2026-09-26, not yet fixed):
  - **Spend:** the lead and solo hats have no token cap (with a budget set, an unpriced model is now stopped instead; see 0.5.5).
  - **Headless:** `ryter -p` exits 0 on `MaxTurns`, `Truncated`, and `Stuck`, and `TurnFinished` carries no stop reason.
  - **Streams:**
    - Tool-call fragments are keyed by id, not `index`, so interleaved parallel calls merge.
    - A stream dropped mid-reply loses the partial text.
    - A `Retry-After` wait is followed by the backoff wait as well.
  - **Crew (from the 2026-09-28 run):**
    - Builders ran `sudo` and wrote into `~/.local` with the sandbox off; the prompt now forbids it, but nothing enforces it.
    - A builder's scratch folder inside its worktree (`.scratch/`, `.probe/`) is committed with the task.
    - Worktree paths are shown whole, which reads as work outside the project.
  - **Chat:** the user's own message is rendered as markdown, so `everything__echo` shows as "everythingecho" (found 2026-09-27).
  - **Shell:** programs that read `/dev/tty` (ssh or gpg prompts) wait out the command timeout.
  - **Compaction:** it can't shrink a single long turn, because it keeps at least four user turns.

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

### 0.2.0-patch — solo mode: one model, three hats (2026-09-21)

- Starts in build; `Tab` cycles build / plan / review; `/crew` enters crew mode (crew builder first), `/solo` leaves
- Per-hat permission rules in the user's tree; `/undo` checkpoints before each build turn; git set up for non-repositories (never in the home folder)
- Mode badge on the message box, mode in the header, crew cards only in crew mode, wider right-hand panel; `ryter --hat`
- Fixed on the way: `python3 -m …` and `python3 --version` were refused as inline code for every role; `2>/dev/null` was refused as a write outside the workspace

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
