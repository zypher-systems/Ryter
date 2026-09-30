# Decisions

Why, not what. The lead records non-obvious choices, its own and the crew's.

### 2026-09-30 — The model loads skills; pages are a skill plus one tool
- **By:** lead
- **Decision:**
  - **Skills:** `prompt::conversation_system` lists the skills the model may use (name and description) for solo and the lead. `load_skill` (handled in the agent loop) returns a skill's body and the names of the files in its folder. `load_skill` with `file` reads one of them (`Skill::read_file`): only a relative path inside the folder, resolved through links, up to 256 KB. Naming the folder's absolute path was no use. `read` refuses `~/.ryter` as a secret, and the sandbox didn't grant it. Skills load from the built-in set (`skill::bundled_skills`, compiled in), then `~/.ryter/skills`, then a trusted project's `.ryter/skills`, with later ones replacing earlier by name. `model-invocable: false` keeps a skill slash-only.
  - **Pages:** the `canvas` skill carries the know-how: facts only from the session, structure, one self-contained file, light and dark, SVG charts. `show_page {title, html}` saves the page to `~/.ryter/pages/<session id>/<slug>.html`. `page::sealed` writes `<!doctype html>` and a Content-Security-Policy ahead of the whole page, less any leading doctype of its own. It then emits a notice with the `file://` link. It opens the page when a person is attached, `[ui] open_pages` is on, the thread isn't sandboxed (`sandbox::active`), and there is a desktop. "Opened" means the opener exited 0 within two seconds, or was still running then. Deleting a session deletes its pages.
  - **Hooks:** the tools the agent loop runs itself (`request_hat`, `load_skill`, `show_page`) go through `tools::with_hooks`, the same `PreToolUse`/`PostToolUse` steps as every other tool.
  - **Sandbox:** `~/.ryter/pages` joins the writable set. `~/.ryter/skills` is readable.
- **Chosen vs rejected:**
  - Skill versus tool: the canvas is mostly know-how, so it's a skill. Only showing the page needs code, a small tool.
  - Rejected writing pages into the project. They're for the user, not the code, and a page in the repo drew an audit offer.
  - Rejected keeping pages in the session's folder. Its name is the whole project path, `%2F`-encoded, so the link wrapped over three lines and needed double encoding.
  - Rejected inserting the CSP at the first `<head>` in the text. Two external reviews of PR #31 showed a `<head>` inside a comment, or a script placed before the real head, got past it. Both leaks were reproduced in headless Chromium. Putting the policy ahead of the whole page leaves nothing for the parser to read first.
  - The CSP blocks loading anything and form posts. It can't stop a script navigating away, but the model already has a shell. The policy is there so a page never pulls in a CDN script, font or tracker.
  - Crew specialists don't get these tools yet. They don't talk to the user.
- **Why:** the user asked what would make the harness more useful, naming a canvas and a headless browser. Model-loaded skills come first because every later ability can be taught as one.
- **Where:** `crates/ryter-core/src/skill.rs`, `skills/canvas/SKILL.md`, `page.rs`, `prompt.rs`, `agent.rs` (`load_skill`, `show_page`), `tools/mod.rs`, `tools/policy.rs`, `config.rs` (`open_pages`, and its settings-file key), `session.rs`, `sandbox.rs` (`active`, `writable_set`, `readable_set`), `tools/mod.rs` (`with_hooks`); `crates/ryter-tui/src/panel/settings.rs`, `run/worker.rs`
- **Residual risk:** a model may still skip the skill and write HTML into the project. The tool description and the skill both say not to, and in the last live run the model complied.

### 2026-09-29 — Every model is chosen in /models; /crew only switches the mode
- **By:** lead
- **Decision:**
  - `/models` has a tab per seat: the lead (`Solo` in solo mode), architect, builder and auditor (`Models.assign_role`, `←→`). The lead's tab lists its connection's models. A role's tab lists every connection's models and `default`, and the first move there sends `ListCrewModels`.
  - Each tab says what its seat runs on now, and a `used by` column marks the seats. `⏎` sets the seat's model, `Tab` its reasoning, and `b` opens the crew builder.
  - `/crew` switches to crew mode. In crew mode it says where the models are. The old crew panel is `/crews`: the ready-made crews, presets, and roles.
  - Every message that said `/crew` for a model now says `/models`.
- **Chosen vs rejected:** the user picked role tabs in `/models` over a separate `/seats` command, and over keeping `/crew` for settings with a key for the mode. The first tab reads `Solo` in solo mode and `Lead` in crew mode, where the sketch said `Lead` in both: in solo mode there is no lead.
- **Why:** "to change the models you have to use /crew models and that is not clear." In solo mode `/crew` switched the mode, and in crew mode it opened the settings. 0.7.1's advice to choose a stronger builder would have switched the user's mode.
- **Where:** `crates/ryter-tui/src/panel/models.rs`, `palette/registry.rs` (`run_crew`, `/crews`), `panel/crew.rs` (title), messages in `crew.rs`, `agent.rs`, `run/actions.rs`, `panel/crew_builder.rs`, `ryter-cli`

### 2026-09-29 — A task's cap asks; a stopped task resumes where it was
- **By:** lead
- **Decision:**
  - **Asking at the cap:** when a charge crosses a task's dollar cap, `run_specialist` asks through `Bill.ask` (a `CapAsk`, from the lead's `UserIo`), on a blocking thread so other workers go on. Raising the cap (`Meter::raise_task_cap`) keeps the step's reply and continues. Stopping keeps the branch (`keep_branch`) and tells the lead that recreating the task starts the build over. A token cap still stops. Headless has no `UserIo` and stops as before.
  - **Before a retry:** a task at its cap asks, or stops, before its first step (`run_build_task`). It used to pay for one call and stop.
  - **Resuming at the gate:** once the builder's work is committed, a stop that is not a rejection sets `TaskOutcome.gate_next` and the handback, and the agent stores both on the task. The next run with the branch present skips the builder. A rejection clears it.
  - **Rejections:** `failed()` marks `rejected`. The agent counts them per task id in the session meta, which survives requeueing and recreation, and shows them on the board (`TaskView.rejections`). At every third, `builder_advice` tells the user, as a notice and in the crew report, to choose a stronger builder in `/crew → builder`. It names no model and switches nothing, and the lead is told not to pick one for them.
  - **Titles:** `UserIo::ask_as` titles Ryter's own questions (`task budget`, `checks`), shown as `Ryter asks · …`.
- **Chosen vs rejected:**
  - The steps are +$5 and +$2, as the user chose.
  - Rejected resetting a requeued task's retries. It gets one attempt, as before. The count and the label now say how many in all.
  - Rejected asking on the token cap. Raising dollars doesn't lift it.
  - Rejected naming a stronger builder, and rejected switching to one: the model is the user's choice. A named pick was built first, from the crew builder's high tier, and removed at the user's word.
- **Why:** a $9.22 run: five audit rejections, a cap stop partway through a step, and a lead that recreated the task (a retry could not get past its spent cap), rebuilding it for $2.19 more.
- **Where:** `crates/ryter-core/src/crew.rs` (`CapAsk`, `raise_cap`, `build_inner`, `failed`), `meter.rs` (task caps), `queue.rs` (`cap_usd`, `gate_next`, `handback`, `TaskView.rejections`), `session.rs` (`rejections`), `agent.rs` (`builder_advice`), `user_io.rs` (`ask_as`); `crates/ryter-tui/src/crewboard.rs`, `panel/modal.rs`
- **Residual risk:** the advice says a stronger builder is needed but not which one. The crew builder's tiers and `/models` prices are where to look.

### 2026-09-29 — Crew members report live; solo gets a rail
- **By:** lead
- **Decision:**
  - **Live progress:** a specialist's step sends `AgentEvent::SubagentLive` (phase, target, estimated tokens, lines written, and the last three lines of output) while it streams (`crew::LiveStep`). The phases are waiting for the first byte, thinking, writing (a reply, or a tool call such as a file edit), and running. A change of phase is sent at once, anything else at most every 200 ms. The partial tool call is read (`partial_call`, `partial_str`) only when a report is due.
  - **Command output:** `ToolContext.live` carries a sink. `shell::run_command_live` passes the newest three lines of whichever pipe moved to it every 250 ms while the command runs.
  - **The board (design C1):** each lane is a card with a chip (WAITING, THINKING, WRITING, RUNNING), a meter, and the output tail. A PULSE tile shows the crew's tokens a second, a bar per second, and the time since the last byte. With no budget, SPEND shows what the last minute cost. Cards shrink to fit, from five rows down to one.
  - **The rail (design S2):** in solo mode on the ledger, at 110 columns and wider, `rail.rs` draws the name, the session, the hat as a block in its color, the model and context, spend with a bar per turn, and the last turn's changed files. The prompt is boxed in the hat's color (`composer::draw::draw_boxed`) with the keys on its lower edge. The view strip and the bottom bar are not drawn while it shows. `[ui] panel` and `^b` now drive the rail on the ledger, as they drive the info panel on the classic screen.
  - **The session's title** is sent to the TUI when the first message sets it, so the rail names the session.
- **Chosen vs rejected:**
  - The user picked S2 and C1 from eight options on the design canvas (four for solo, four for crew).
  - Rejected sending every stream delta. A long file edit arrives in thousands of fragments, and each report read the whole call so far.
  - Rejected logging live events to `activity.jsonl`: several a second is noise. Finished tool calls are still logged.
  - Rejected exact token counts while streaming. Usage arrives only when a step ends, so the count is characters ÷ 4 and shown with `~`.
  - C1's `r` (reasoning on and off) is `^r` on the board: the lead's prompt is always taking keys, so `r` would be typed. Elsewhere `^r` still toggles the reasoning pane. S2's `u undo` is `/undo` for the same reason.
  - The prompt's keys are on its box's lower edge, not inside it: the prompt row holds the typing and its placeholder.
- **Why:** "Setting there for 10 minutes seeing no movement is not a good experience." The board reported only finished tool calls, so a model thinking or writing a long edit looked frozen. In solo mode the name was nowhere on screen, the costs were in the bottom bar, and the hat was a small chip at its left end.
- **Where:** `crates/ryter-core/src/event.rs` (`SubagentLive`, `LivePhase`), `crew.rs` (`LiveStep`, `running_tool`), `tools/shell.rs` (`run_command_live`), `tools/mod.rs` (`LiveOutput`), `agent.rs` (title event); `crates/ryter-tui/src/crewboard.rs` (`lane_cards`, `pulse_tile`), `rail.rs`, `draw.rs` (`draw_with_rail`), `composer/draw.rs`, `view/mod.rs` (`LaneLive`, pulse)
- **Residual risk:** a model that doesn't stream its reasoning shows WAITING until it writes. A command faster than 250 ms shows no output while running. Tools other than `bash` show no running output.

### 2026-09-28 — OpenRouter lists the account's models, and the crew checks its seats first
- **By:** lead
- **Decision:**
  - **The list:** OpenRouter models come from `/models/user` (the account's list), keeping rows whose `supported_parameters` include `tools`. That endpoint ignores `?supported_parameters`. If it fails, the list falls back to `/models?supported_parameters=tools`.
  - **The seat check:** `Provider::refused` returns the models that are in the tools catalog but not on the account's list. `Agent::refused_seats` asks once per connection before a crew run. It checks the architect when a design is pending, and the builder and the auditor panel when a build is ahead (builder tasks pending, or a design that isn't held). If any seat is refused, the crew pauses: nothing runs, and the tasks stay queued.
  - **Kept work:** a seat's provider error during sign-off is `SignOff::Unreachable`. The task is kept like any work that must not land (`keep_branch`), and a patch landing waits. Any other non-cancel error after the builder committed keeps the branch too (`crew::has_work`).
  - **Once a turn:** `drain_crew_inner` says whether the crew paused, and the turn doesn't drain it again.
- **Chosen vs rejected:**
  - Rejected treating "not on the account's list" alone as refused. A routing suffix (`:nitro`) or an id OpenRouter doesn't list would be blocked. A model on neither list is left for its first request to explain.
  - Rejected probing each seat with a request, as the crew builder does. It costs a little and takes seconds. The two lists are free and come back in about 0.2 s.
  - Rejected checking the auditor for a held design. A design-only run must not be refused for want of an auditor.
  - Rejected keeping half-done work when the builder's own call fails mid-task. It isn't committed, and a retry starts over as before.
- **Why:** a live run paid $2.56 (architect) and $1.86 (builder) before its auditor, claude-fable-5.1, was refused under the account's zero data retention setting. The refusal then deleted the builder's branch. The picker had offered the model because it listed the whole catalog.
- **Where:** `crates/ryter-core/src/llm/http.rs` (`list_models`, `refused`), `llm/mod.rs` (`Provider::refused`, `explain_error`), `agent.rs` (`refused_seats`, `drain_crew_inner`), `crew.rs` (`SignOff::Unreachable`, `has_work`)
- **Residual risk:** a stale model cache from before the upgrade still lists refused models until the picker's background refresh replaces it, which takes about a second. The seat check catches them anyway. It covers OpenRouter only; other providers' refusals still surface on the first call, with the work kept.

### 2026-09-28 — Model lists come from a cache first, and OpenRouter's from its tools filter
- **By:** lead
- **Decision:**
  - **Where the list comes from:** OpenRouter connections list models with `?supported_parameters=tools` (`http::models_url`).
  - **The cache:** every connection's last list is kept in `~/.ryter/cache/models/<connection>.json` (`llm::model_cache`). The picker and the crew builder open on it, with a note in the corner (`from 2 h ago · refreshing`).
  - **The fetch:** it runs on a thread and client of its own (`worker::fetch_models`). After 30 s the picker says the provider is slow; after 300 s it gives up and keeps the cached list.
- **Chosen vs rejected:**
  - Rejected fetching on the worker's thread, as before. A turn waited behind a slow catalog.
  - Rejected sharing the agent's client with the fetch thread. Its pooled connections live on the worker's runtime, which only runs while the worker is busy.
  - Rejected a hard 30 s cutoff. The download that finishes later is what fills the cache.
  - Models that take no tools are no longer listed on OpenRouter. They were dimmed before, and neither the build hat nor the crew can use them.
- **Why:** OpenRouter's full `/models` stalled near its end for minutes, and the picker spun until it ended. The tools-filtered list came back in 0.16 s. Reeve looked instant because it showed its last list while refetching.
- **Where:** `crates/ryter-core/src/llm/http.rs`, `llm/model_cache.rs`, `event.rs` (`ModelsNote`), `crates/ryter-tui/src/run/worker.rs`, `panel/models.rs`
- **Residual risk:** a model OpenRouter doesn't mark with `tools` is invisible in the picker, even where it could chat. It can still be set by id.

### 2026-09-28 — Mission control draws the queue; the workbench undoes one change at a time
- **By:** lead
- **Decision:**
  - **Queue snapshots.** The agent sends `AgentEvent::Tasks` (every `TaskView` plus the open patch) after a `todo_write` and at each step of a crew run: batch taken, results in, held tasks proposed, run ended. `TaskView.waits_on` is a task's `after` plus the task laying the foundation (`TaskQueue::views`), so the board shows what the scheduler enforces.
  - **The TUI keeps every edge it has seen** (`View::task_edges`), so a landed prerequisite stays drawn. A design's tasks hang under the design only when they wait on nothing else.
  - **The board** (`crewboard.rs`) has tiles, a plan tree, and lanes. A lane's `acting` comes from `SubagentActivity.role`, so a builder's worktree in review shows as "in audit".
  - **Specialist activity** goes to `activity.jsonl` as well as to the sink (`crew::Progress.log`).
  - **The workbench** (`workbench.rs`) reads the changes against the last turn's checkpoint or `HEAD`, and builds each file's diff from `review::file_versions`, numbering hunks as `FileDiff::new` does. `x` sends `RevertHunk`, and `review::revert_hunk` swaps that hunk's new lines for its old ones through `diff::revert_hunk`, recorded like a file revert (`Agent::revert_recorded`).
- **Chosen vs rejected:**
  - First shipped as a text tree. That read as a list, not the drawing the design showed, so 0.6.2 draws boxes (`plan_drawing`). Each task is placed at the top of the block its followers fill, under its first prerequisite, so connectors never cross. Other prerequisites and full blocked reasons are listed under the drawing. The tree remains the fallback when the drawing won't fit.
  - Rejected progress bars in lanes. There is no real measure of a worker's progress, and the user asked for an accurate picture. Lanes show who, what, and for how long.
  - Rejected typing in the composer while the workbench is open: `x` undoes a change there.
  - `x` undoes at once, since `/undo` brings it back. `X` asks first.
- **Why:** the user wanted designs B and D with A in one 0.6.0, and the board to stay an accurate drawing of what is going on. A crew run the night before had hidden everything that mattered: a blocked scaffold, tasks started without it, and 80 calls of builder workarounds.
- **Where:** `crates/ryter-core/src/queue.rs` (`TaskView`, `PatchView`, `views`), `event.rs`, `agent.rs` (`emit_tasks`, `revert_hunk`), `crew.rs` (`Progress`), `diff.rs` (`revert_hunk`), `review.rs`, `crates/ryter-tui/src/crewboard.rs`, `workbench.rs`, `draw.rs`, `run/keys.rs`
- **Residual risk:**
  - Lanes are matched to tasks by title.
  - A change more than about four hunks down scrolls into view only as you move to it.
  - The workbench is on the ledger only; the classic screen keeps `/changes`.

### 2026-09-28 — The ledger is the default screen; the 0.5 layout stays as `classic`
- **By:** lead
- **Decision:**
  - `[ui] layout = "ledger"` (default) draws one centered reading column, at most 112 columns (`draw::LEDGER_COLUMN`), on a timeline. The gutter is added when rows are placed, so the render cache is unchanged: `chat::layout::GUTTER`, the time and `●`, `◆`, `├─`, `│`, `└─`.
  - Every turn ends with a `SystemLevel::Receipt` line built at `TurnFinished` from `TurnTally` and the turn's spend (`View::turn_spend_from`).
  - Every finished turn except the newest folds to one line; `^o` (`diffs_expanded`) opens them.
  - A new turn is pinned four rows down, so the folded turns above it stay in sight.
  - The composer is a rule and a prompt. One bottom bar replaces the header, the hint bar, and the info cards. `$` on an empty composer opens a docked spend drawer (`panel::spend_drawer`).
  - `layout = "classic"` is the 0.5 screen, byte for byte: every existing snapshot is unchanged, because `View::new` starts classic and the app takes the config's layout.
- **Chosen vs rejected:**
  - From five options on a design canvas, the user picked A (the ledger) as the default, B (the workbench) as a mode to switch to, and D (mission control) for crew mode. Rejected C (quiet): it hides too much for a tool whose job is showing what it changes.
  - Rejected removing the classic layout. The user wanted to be able to go back, so it is one setting away.
  - Turn costs show three decimals under $0.10 (`chat::turn_usd`): a turn is often a fraction of a cent, and `$0.00` hid it.
- **Why:** the user asked for real design options, not color changes. The cards took a third of the width to show five numbers, and nothing said what each turn had done or cost.
- **Where:** `crates/ryter-tui/src/draw.rs` (`draw_ledger`, `draw_status_bar`), `chat/layout.rs`, `chat/mod.rs` (`ledger_header`, `turn_usd`), `composer/draw.rs`, `run/events.rs` (`receipt`), `panel/spend_drawer.rs`, `crates/ryter-core/src/config.rs` (`UiConfig.layout`)
- **Residual risk:**
  - A folded turn opens only with `^o`, all at once; clicking a fold line does nothing yet.
  - The status bar's drop order is fixed.
  - Sessions from before 0.6.0 have no closing lines, so their turns don't fold.

### 2026-09-28 — The crew builds in order: prerequisites land first, and blocked means blocked
- **By:** lead
- **Decision:**
  - **`Task.after`:** the ids a task needs done first. `TaskQueue::take_ready` starts a task only when each of them is done, which for a builder task means landed on the patch; an unknown id counts as unmet.
  - **Foundation first:** when the tree tasks branch from has no build manifest at its root (`queue::MANIFESTS`), the first unfinished builder task that declares one runs alone, and other builder tasks wait for it (`TaskQueue::foundation`), declared or not.
  - **Waiting is reported, once:** when nothing can start, the crew report lists what each waiting task waits on and why (`TaskQueue::waiting`). `crew_has_pending` counts only tasks that can start (`startable`), so the lead isn't sent back to the crew after every reply.
  - **`STATUS: BLOCKED` from a builder** is committed and kept on its branch without checks, audit, or a retry. The line of its NOTES that says what would unblock it becomes the task's reason (`blocked_reason`).
  - **`VERDICT: UNVERIFIED`** (`crew::Verdict`): with no checks run, an auditor can pass code it can't build because something outside the task hasn't landed. The task lands on the patch listed in `Patch.unverified`, and `land_patch` refuses to land a patch with unverified work until checks are set.
  - **Check detection** (`checks::detect`): with no `[auditor] checks`, Ryter reads the patch tree's manifest before each builder batch and before landing, and asks once a session (`Meta.checks_offered`). The answers are: save them to `.ryter/config.toml` (trusted projects only), use them this session, or no. Headless says it in the crew report instead.
- **Chosen vs rejected:**
  - Rejected auditing on review alone until everything merges, as the rule. A compiler finds errors a review misses, and fixing them per task, while the builder has its context, is cheaper than one integration task that owns everyone's errors. `UNVERIFIED` covers the case where a build is impossible.
  - Rejected detecting checks silently. `npm test` runs the project's own scripts, outside the permission gate, so the user says yes once.
  - Rejected special-casing the word "scaffold". The manifest rule catches the case whatever it is called, and `after` covers the rest.
- **Why:** a crew run on a new Rust app spent $2.29, mostly on retries. The scaffold was blocked (a missing system library, which the builder worked around with `sudo` and downloads into `~/.local`). Three tasks built on it started anyway, with no `Cargo.toml`, and each was rejected for its absence. No check built the finished patch, because none were set.
- **Where:** `crates/ryter-core/src/queue.rs`, `crew.rs`, `checks.rs`, `agent.rs` (`drain_crew`, `offer_checks`, `crew_has_pending`), `config.rs` (`save_project_checks`), `session.rs`, `prompts/*.md`
- **Residual risk:**
  - A plan that splits dependent work without `after`, in a project that already has a manifest, still runs it in parallel.
  - Builders are told not to use `sudo` or write outside their worktree, but nothing enforces it while the sandbox is off.
  - Detected checks are a starting point (`cargo test`, not clippy).

### 2026-09-27 — Outbound MCP calls wait with a deadline and match replies by id
- **By:** lead
- **Decision:**
  - Each server's stdout is read on its own thread into a channel, and its stderr is drained into a 2 KB tail.
  - A request waits for the message with its own id: up to a deadline (`timeout_secs`, default 120s; 60s for `initialize` and `tools/list`), until the stdout closes, or until the turn is cancelled. It then sends `notifications/cancelled`.
  - While waiting, it skips non-JSON lines, notifications, and replies to abandoned requests, and answers server requests: `ping` gets `{}`, anything else gets -32601.
  - `McpHub::route` finds the server under the hub lock and releases it; the call holds only that server's lock (`call_tool`).
  - `isError: true` is returned as an error. `Error::Mcp` carries MCP failures without a misleading `provider:`/`config:` prefix.
  - `search_tool` matches every query word against key and description, and lists each tool's arguments from its `inputSchema`.
- **Chosen vs rejected:**
  - Rejected a shared id counter and concurrent calls per server. One call at a time per server, with id matching, is enough to stop the shift, and much simpler.
  - The 120s default is a backstop, not a limit on normal work: Esc is the way out, and `timeout_secs` raises it per server.
- **Why:** a server that never answered hung the turn for good, with the hub locked. The reference server (`server-everything`) sends a notification before its `initialize` reply, so it failed to connect at all ("missing field id"). Any notification mid-session shifted every later result by one.
- **Where:** `crates/ryter-core/src/mcp/client.rs`, `tools/mod.rs` (`mcp_use`), `config.rs` (`McpServerConfig.timeout_secs`), `error.rs`
- **Residual risk:**
  - Server-to-client features Ryter refuses (sampling, roots, elicitation) make those tools fail.
  - Progress notifications aren't shown.
  - After a timeout the server may still be running the abandoned tool, so the next call to it can be slow.

### 2026-09-27 — A budget stops what it can't price; Anthropic prompts counted whole
- **By:** lead
- **Decision:**
  - **`Usage.input_tokens` is always the whole prompt.** Anthropic's `input_tokens` leaves out cache reads and writes, so the parser adds them back and marks both (`cached_tokens`, the new `cache_write_tokens`). `Rates::cost` prices plain input, reads, and writes at their own rates (`cache_write_per_million`, falling back to input). Before, writes weren't counted, and reads were subtracted from a count that already excluded them, so Anthropic spend showed about half and the context gauge showed a few thousand tokens for a 100k prompt.
  - **With a session budget, an unpriced model is stopped after one call.** The call's reply is kept, and a notice says why. `Meta.unpriced_model` records the model, and the next call checks it before sending (`Agent::unpriced_stop`); `Error::Budget { unpriced }` says what to do. A crew run with a session budget stops at an unpriced charge the same way (`Meter::charge`).
  - **A crew task's caps count earlier runs.** `Task.spent` keeps the task's tally in `tasks.json`; each drain seeds the meter with it (`Meter::with_prior`) and writes it back.
- **Chosen vs rejected:**
  - Rejected stopping mid-turn right after the unpriced call. The reply would be lost from the transcript, and the next message would start without it.
  - Rejected checking before the first call. A model can be missing from the price book and still have its cost reported by the provider (OpenRouter), so the first call is what tells.
  - Rejected a solo token cap for now. Any default would be a number Ryter chose for the user; a price makes the budget the user already set work.
  - Without a budget nothing changes: the spend card shows `$?.??`, as before.
- **Why:** the user can pay $25–$90 for one review. A budget that silently doesn't apply is worse than none.
- **Where:** `crates/ryter-core/src/llm/parse.rs` (`usage_from`), `spend.rs`, `agent.rs` (`over_budget`, `unpriced_stop`), `meter.rs`, `session.rs`, `queue.rs`, `error.rs`
- **Residual risk:**
  - One unpriced call is always made before the stop.
  - Long-context rates don't raise the cache-write rate.
  - A crew task id reused after its task was removed inherits nothing, but one removed and re-added with the same id during a single run would.

### 2026-09-27 — Model lists show only models that can chat
- **By:** lead
- **Decision:** `parse_models_json` drops catalog rows that can't hold a conversation here (`http::converses`), so `/models`, the `/audit` chooser, `ryter models`, and crew suggestions never list them:
  - `:batch` routes, which queue a request and answer later without streaming;
  - models whose output types include image or audio;
  - for catalogs that don't give output types, ids naming embeddings, speech, transcription, images, moderation, realtime, or reranking (`NOT_CHAT`).
- **Chosen vs rejected:**
  - Rejected dimming them the way tool-less models are dimmed. A tool-less model can still answer; these can't be used in a chat at all.
  - Router models (`openrouter/…`) are kept although their catalog rows claim image output: they list every output of the models they may route to.
  - `:free` and local tags (`qwen3-coder:30b`) are kept.
- **Why:** the user found batch and image-generation models in the picker. Out of 458 OpenRouter models, 85 were of that kind.
- **Where:** `crates/ryter-core/src/llm/http.rs`
- **Residual risk:** the id patterns are a heuristic for catalogs without output types, so a chat model whose id contains `image` or `audio` would be hidden there. A model id set by hand is still used as given.

### 2026-09-27 — Enter approves again, with a guard; `a` allows one kind of action
- **By:** lead
- **Supersedes:** "`Enter` is never an alias for allow" (2026-09-21), for ordinary prompts.
- **Decision:**
  - **The prompt is a card docked above the composer** (`Panel::docked`), over an undimmed chat, with labeled rows: what, why (the model's last paragraph this turn, `View::last_words`), risk (in plain words, including what `/undo` reaches), and the change.
  - **`⏎` allows,** except within `ENTER_GUARD_MS` (500 ms) of the card opening: a press that arrived with the card is ignored.
  - **`a` allows the kind of action the card names for the session:** edits to project files, one command's program and subcommand, or a chain's exact text (`tools::allow_scope`). It's one press, stored in `ToolContext.allowed`, and replaces the blanket `sticky_approve`.
  - **Strict prompts take only `y`:** commands that delete, move, or discard files (`policy::destructive_command`: `rm`, `mv`, `find -delete`, `git reset --hard`, `git clean`, `git restore`, `git checkout -- .`, deleting a branch or tag, force-pushing), and writes outside the project. Enter explains instead, and there is no `a`.
- **Chosen vs rejected:**
  - Rejected keeping Enter off everywhere: the user found Reeve's `⏎ / a / n` better to work with.
  - The 2026-09-21 risk (a habit approves `rm -rf`) is met two ways instead. Destruction never takes Enter. And the guard stops the actual failure, a send-key press landing on a card that opened under it.
  - Rejected a blanket `a`. It approved every later call, destructive ones included, which is why it needed a second press.
- **Why:** faster, clearer approvals, modeled on Reeve's card, without giving up the reason Enter was removed.
- **Where:** `crates/ryter-tui/src/panel/modal.rs`, `panel/mod.rs` (docking, legend row), `crates/ryter-core/src/tools/mod.rs` (`allow_scope`, `strict_prompt`, `allowed`), `tools/policy.rs` (`command_scope`, `destructive_command`), `user_io.rs` (`ToolAsk`)
- **Residual risk:**
  - A command's scope is a heuristic, program plus first plain argument, so `a` on `npm install` also allows `npm install left-pad`.
  - The destructive list is kept by hand.
  - An Enter pressed more than half a second after a card opens approves it, typed or not.

### 2026-09-27 — Panel keys on the panel's last row; search inside pickers
- **By:** lead
- **Decision:** Every panel draws its legend as its last inner row, keys colored and labels dim (`panel::legend_line`), not as dim text in the bottom border. While a panel is open, the hint bar echoes that panel's keys. Pickers that opt in (`Panel::inline_input`: models, help) draw their filter as the panel's first row, with the cursor there; the composer says typing goes to the panel. The model picker opens on the current model and dims models without tool support.
- **Why:** from comparing Reeve's menus. Border text was hard to read. The filter typed at the foot of the screen was a screen away from the list it filtered. The hint bar showed form keys (`tab next field`) on a permission prompt. And the picker opened at the top of 458 models.
- **Where:** `crates/ryter-tui/src/panel/mod.rs`, `draw.rs` (`hints`), `composer/draw.rs`, `panel/models.rs`, `panel/help.rs`

### 2026-09-27 — `/undo` puts back what the turn changed, not the whole tree
- **By:** lead
- **Decision:**
  - A build turn now records a snapshot when it ends as well as when it starts (`TurnRecord.after`, set by `finish_turn_record` however the turn ends). `/undo` restores only the paths that differ between the two (`git::paths_between`, `git::restore_paths`).
  - When the user has since changed any of those paths, `/undo` refuses and names them. `/undo force` goes ahead.
  - Each undo keeps the files as they were just before it, and just after it, as refs. `/redo` reverses it, with the same refusal for edits since the undo, and `/redo force`. A new build turn, or a `/changes` revert, clears what `/redo` could reverse.
  - Before the build hat writes a gitignored file with `write` or `search_replace`, its content is saved as a git object (`TurnRecord.ignored`, `git::save_blob` / `put_blob`), because snapshots skip ignored files.
  - A `/changes` single-file revert is recorded like a turn, so `/undo` brings back that one file.
- **Chosen vs rejected:**
  - Rejected a partial undo when some files clash. Half a turn undone can leave code that doesn't build, so the user decides with `force`.
  - Rejected asking in a prompt. `/undo` is a direct command, and a refusal that names the files plus a `force` form keep it that way.
  - Rejected snapshotting ignored files for shell commands. The gate can't know what a command will write.
- **Why:** `/undo` reset the whole project to the snapshot, so a file the user created between turns was deleted and their own edits were reverted. The only copy left was an internal ref `/undo` couldn't reach. A gitignored file the model overwrote couldn't be undone at all ("nothing to undo"). Both were found in the 2026-09-26 audit and reproduced by tests before the fix.
- **Where:** `crates/ryter-core/src/agent.rs` (`undo_with`, `redo_with`, `save_ignored`, `finish_turn_record`, `revert_file`), `git.rs`, `session.rs` (`TurnRecord`, `SavedFile`, `Redo`)
- **Residual risk:**
  - Sessions from before 0.5.2 have no end snapshot for their older checkpoints. Undoing one of those still restores everything since, as before, though `/redo` now reverses it.
  - Ignored files changed by a shell command aren't saved.
  - The `refs/ryter/undo/` refs accumulate per session, as before.

### 2026-09-27 — Audits are offered after changes, by Ryter, and `/second` is `/audit`
- **By:** lead
- **Decision:**
  - The command is `/audit`, with `/second` kept as an alias. `/audit model` changes the choice.
  - When a build turn ends `Completed` and made a checkpoint (it changed files), and there is uncommitted work, Ryter offers an audit (`Agent::offer_audit`). The offer is the same yes/no prompt with the cost. `y` runs it, with no second question; `n` passes; `s` sets `[ui] offer_audit = false`.
  - With nobody chosen, the offer asks whether to choose, and only a yes opens the chooser.
  - An audit becomes a turn (busy, `Esc`) only once it's accepted, so a declined offer leaves the build turn's summary on screen.
- **Chosen vs rejected:**
  - Rejected having the model offer, through a new tool or a prompt rule. Ryter knows for certain whether files changed and what the audit would cost. A model offers inconsistently, and a new tool changes the tool list and the prompt cache.
  - Rejected auditing automatically without asking: each audit spends money.
- **Also:** an audit in the chat carries a rule in the auditor's color on every row, and folds after 14 rows when four or more are left (`chat::AUDIT_ROWS`; `^o` unfolds, as for edits). Unmarked and unfolded, a long audit filled the pane, its header scrolled off, and it read as the builder talking.
- **Why:** typing `/second` after every change was cumbersome, and "second" didn't say what it did.
- **Where:** `crates/ryter-core/src/second.rs` (`offer_audit`, `audit`, `run_audit`), `crates/ryter-tui/src/run/worker.rs` (the trigger), `panel/modal.rs` (`is_offer`), `config.rs` (`offer_audit`)
- **Residual risk:**
  - A turn whose only "change" was a command that asks (not a file edit) also makes a checkpoint, and gets an offer if anything is uncommitted.
  - The trigger in the worker has no automated test; it was tried live.

### 2026-09-27 — Independent review comes to solo as `/second`; the user chooses the model and the limit
- **By:** lead
- **Decision:**
  - `/second` has another model review the uncommitted diff and the conversation, under the review hat's gate, with its own prompt (`prompts/second.md`), through the crew's specialist loop (`crew::run_specialist`).
  - **The user chooses the reviewer and a dollar limit per review,** in the models panel's review mode. Rows come from the live catalog of every connection with a key, each with a price range for this review. Excluded: the working model, and models without tools. Marked: same vendor. Refused: unpriced. Nothing is preset. The choice is saved in `~/.ryter/review.toml`.
  - Every review asks first, with a range: one round at best, six exploring rounds at worst. Once there are two or more past reviews with the model, it also shows what those cost (`~/.ryter/reviews.jsonl`).
  - **The limit is kept before each step** (`Bill::wrap_up_usd`). A step is priced from what it will send and isn't sent if it would pass the limit. With about one step's room left, or 75% spent, the reviewer gets no more tools and is told to write up. A stop keeps the text written so far (`Bill::last_text`).
  - Reviews reason at `medium` unless the user set a level for the model. The diff given up front is larger than a commit draft's (60k characters in all, 16k per file).
  - A saved reviewer that is gone, retired, or unpriced brings the chooser back. There is never a fallback.
  - The review creates no files: `prompt::reading_messages` builds the prompt without creating project memory.
- **Chosen vs rejected:**
  - **Rejected auto-picking a reviewer** (`crew suggest`'s "strongest other vendor"), and **rejected tiers** ("thorough / balanced / light"). Models change too fast to curate. Ryter can't judge quality, only price. And any model Ryter picks is a bill Ryter picked: whoever didn't expect it blames the tool.
  - **Rejected a fixed cap ($1.00).** Real reviews the user has run cost $25 (kimi-k3) and $90 (gpt-5.6-sol). A cap below the job cuts it off and leaves the user paying for nothing.
  - **Rejected enforcing the limit after each call**, as the crew's task caps do. One expensive step would pass it with nothing to show.
  - Rejected the model triggering reviews itself.
  - Rejected building more crew setup first. The user found they don't reach for crew themselves.
- **Why:** crew mode's value is unproven, and its setup is where people stop. What people want from it is "a second model signs off", which needs none of the crew.
- **Where:** `crates/ryter-core/src/second.rs`, `crew.rs` (`Bill`, `run_specialist`), `meter.rs` (`price`), `config.rs` (`ReviewerConfig`), `prompt.rs` (`reading_messages`), `prompts/second.md`, `crates/ryter-tui/src/panel/models.rs` (`for_review`), `panel/modal.rs` (`question`)
- **Residual risk:**
  - Step prices are estimates: bytes ÷ 4, and output assumed at 4k. A model that writes far more in one step can pass the limit by that step's overage.
  - Cost ranges are rough until there is history.
  - Only uncommitted work can be reviewed.
  - A price sort puts free models first, which is visible but still a nudge.

### 2026-09-26 — The shell gate reads a command the way the shell will
- **By:** lead
- **Decision:**
  - **Wrappers are parsed, not skipped.** `WRAPPERS` lists each wrapper's options that take a value and its plain arguments before the command: `timeout 5`, `taskset MASK`, `flock FILE`. The program is found after them, recursively. `env -S`, `flock -c`, and a one-string `watch` hide the command and are refused. `busybox` and `toybox` count as wrappers.
  - **Words are read as the shell reads them:** `s\udo` is `sudo`.
  - **A substitution stays in its command** as a `$(…)` placeholder, and is judged as a segment of its own, first.
    - A program named by `$X` or `$(…)` is refused, and so is `eval`.
    - A `$(…)` argument counts as an unreadable path, except to `echo` and `printf`.
  - **Nested commands are judged:** `find -exec`, `fd -x`.
  - **Arguments a check can't see ask:** `xargs rm`, and a builder's `cd` out of its worktree.
  - **Git options are judged:** `-c` keys that name a program, and `--exec-path`, are refused. `-C`/`--git-dir`/`--work-tree` outside the project ask, or are refused for roles that can't write. `grep -O` is refused. `--output` asks.
  - **Review and the auditor are allowed by form (`checks_only`), not by tool name.** The auditor alone may install from the lockfile (`npm ci`, `npm install` with no packages, `cargo fetch`).
  - **Other read-only tools are judged by form too:** `sort --compress-program`, `rg --pre`, `fd -x`, `yq -i`.
  - **`node`, `deno`, `bun`, `tsx`, and `ts-node` are interpreters:** their inline code is refused, as `python -c` already was. `env` is off the read-only list.
- **Chosen vs rejected:**
  - Rejected matching the never-run list against every word of a command. `rg at src/` and `grep init` would be refused (`at` and `init` are on the list).
  - Rejected asking instead of refusing for `node -e` in the build hat. That would differ from `python -c`, and the reason is the same: the gate can't read the code.
  - Kept `cargo run`, `go run`, and scripts runnable in review. Tests run project code too. What review may not do is run the tools whose job is to change the tree.
- **Why:** The 2026-09-26 audit got `sudo`, `ssh`, and `dd` past the never-run list in every role, with wrapper options, escapes, and substitutions. It found that the review hat, in the user's own tree, ran `cargo fmt`, `npm install`, `npx <any package>`, and `node -e` code that writes files, without asking. A second pass by the lead found `find -exec sudo`, `git -c alias.x=!…`, `busybox rm`, `sort --compress-program`, `rg --pre`, and `fd -x`.
- **Where:** `crates/ryter-core/src/tools/policy.rs` (`WRAPPERS`, `parse`, `segments`, `words`, `decide_segment`, `exec_commands`, `checks_only`, `decide_git`, `git_config_runs`, `runs_or_edits`, `inline_code`)
- **Residual risk:**
  - The never-run list is a speed bump for a builder, not a boundary. A builder can write a script, a Makefile, or a `build.rs` that runs anything, and `awk 'BEGIN{system(…)}'` and GNU `sed e` escape too. The Landlock sandbox is the boundary, and it is off by default.
  - Shell reads can still reach secrets indirectly: `grep -r . .` reads `.env`, as does `ls -a | xargs cat`. Only direct paths are checked.
  - `echo $SECRET` prints a variable, and project secrets in the environment (tokens, cloud keys) stay visible to commands.
  - The option tables are kept by hand. A wrapper that isn't listed hides its command.
  - An independent adversarial pass on the new gate was attempted and did not run. The tests cover every bypass found so far, not the ones nobody has found.

### 2026-09-26 — Turns end by themselves, and say why
- **By:** lead
- **Decision:**
  - **The stream ends when the provider says so.** `[DONE]`, `response.completed`/`incomplete`, and `message_stop` end it; so does an error event inside the stream (OpenRouter's `error` chunk, `response.failed`, Anthropic's `error`), which is now an error, not an empty "completed" reply. `content_filter`, `refusal`, and context-window stops are errors that say which.
  - **A stall deadline that keep-alives don't reset:** 300 s without a real delta (900 s for a local server). Idle-read timeouts stay for dead sockets.
  - **Bytes are decoded whole.** A character split across two chunks waits for its other half.
  - **Repeats:** the same call with the same result in one turn is flagged at the 3rd time (appended to the tool result) and stops the turn at the 5th (`StopReason::Stuck`). A successful edit resets the count for other calls, since re-running a check after an edit is fair.
  - **The lead drains the crew once per turn** unless it queues new work. The round cap now says so in the chat.
  - **Every tool call gets an answer.** Esc, an error, or a stuck stop answers the calls left in the batch, and each turn first repairs a transcript an earlier stop left open.
  - **Prompts give up on cancel** (polled every 100 ms), and `^c` on a prompt mid-turn cancels the turn.
  - **Commands can't wait for a person:** `GIT_EDITOR`/`EDITOR`/`VISUAL=false`, pagers are `cat`, `GIT_TERMINAL_PROMPT=0`, and Ryter's own API keys are removed from the environment.
- **Chosen vs rejected:**
  - Rejected a total wall-clock bound per request: a long, legitimate reasoning turn looks the same as a stuck one from outside, until it sends a delta.
  - Rejected stopping on the first repeat: a model may re-read a file after a failed build for good reason.
  - Rejected `CI=1` and `NPM_CONFIG_YES` in the shell environment. The first changes how test runners behave; the second approves `npx` installs.
  - Rejected `setsid` for commands, which would stop tools that read `/dev/tty` (ssh, gpg prompts) from hanging, because it breaks process-group kills on cancel.
- **Why:** "Models keep spinning" had several causes, each proved with a test first:
  - a waiting patch made the lead take all 40 rounds on every message, at a paid call each;
  - OpenRouter's keep-alives kept a stuck request open for good;
  - an Esc during a command left the session unusable (the next request was rejected by the provider);
  - a failing edit was retried until the round cap.
- **Where:** `crates/ryter-core/src/llm/http.rs` (`sse_delta_stream`, `take_utf8`), `llm/parse.rs` (`parse_blocks`), `agent.rs` (`turn_inner`, `answer_unrun`), `session.rs` (`repair_unanswered`), `user_io.rs` (`wait`), `tools/shell.rs` (`NON_INTERACTIVE`)
- **Residual risk:**
  - A model that thinks silently for more than 5 minutes behind a keep-alive is cut off.
  - A program that reads `/dev/tty` still waits out the command timeout.
  - The repeat count compares exact results, so a loop whose output changes each time (a timestamp) is caught only by the round cap.

### 2026-09-26 — An edit's diff is measured by the tool and shown, never sent to the model
- **By:** lead
- **Decision:** `write` and `search_replace` diff the file before and after (the `similar` crate, Myers with a 250 ms deadline, 2 lines of context, hunks one or two lines apart joined) and put the `FileDiff` on the tool result event. The chat draws it as numbered rows tinted to the pane's edge; the permission prompt for an edit gets the same diff from a dry run. The transcript keeps its one-line summary.
- **Chosen vs rejected:**
  - Rejected rendering from the model's arguments (the old preview). It has no line numbers or context, and it shows nothing for a `write`.
  - Rejected sending the diff to the model. It already knows what it wrote, and every later round would pay for it.
  - Rejected a hand-rolled diff: rewrites need a real LCS, and `similar` is small and dependency-free.
- **Why:** People asked to see what is changing as it changes. The permission prompt also truncated its summary at 160 characters, so a `propose_edit` sign-off showed about two lines of the change it approved.
- **Where:** `crates/ryter-core/src/diff.rs`, `tools/fs.rs` (`preview`), `event.rs` (`ToolResult.diff`), `crates/ryter-tui/src/chat/diff.rs`, `panel/modal.rs`, `theme.rs` (`diff_add_bg`, `diff_del_bg`)
- **Residual risk:**
  - Edits made through `bash` (`sed -i`, codegen) show no diff; `/changes` still does.
  - A resumed session shows the old argument preview for past edits, because the diff isn't in the transcript.
  - Diffs are capped at 400 lines per event.

### 2026-09-23 — Read-only means the command's form, not just its name
- **By:** lead
- **Decision:**
  - **More read-only tools.** The list now includes byte and checksum tools: `xxd`, `od`, `hexdump`, `strings`, `base64`, `tac`, `rev`, `comm`, `paste`, `sha256sum`, `sha1sum`, `md5sum`, `shasum`, `cksum`.
  - **Output-file forms don't count as read-only.** `writes_output_file` refuses the forms of read-only commands that write a file:
    - `sort -o` and `--output`, including inside a cluster like `-no`;
    - `uniq in out`;
    - `tree -o`;
    - `xxd -r` and `xxd in out`;
    - `base64 -o` (macOS);
    - `find -fprint`, `-fprintf`, and `-fls`.
  - **`find -okdir`** joins `-exec`, `-execdir`, `-ok`, and `-delete` as destructive.
- **Chosen vs rejected:**
  - Rejected leaving `xxd` off the list. Reviewers use it to check line endings and trailing newlines, and only its `-r` and two-file forms write.
  - Rejected adding `awk` and `sed`. They can write files from inside their own scripts, which the gate can't read.
- **Why:** In real use the review hat refused `tail -c 50 .gitignore | xxd`. Checking why showed that `sort -o`, `uniq in out`, `tree -o`, and `find -fprint` could already write files from plan and review, because the list judged a command by its name alone.
- **Where:** `crates/ryter-core/src/tools/policy.rs` (`READ_ONLY`, `writes_output_file`, `read_only`, `deleting_find`)
- **Residual risk:** Another tool with a write option that isn't listed here would get through. The list is maintained by hand.

### 2026-09-23 — `2>&1` is a redirect, and `cd` into the project is allowed where commands are read-only
- **By:** lead
- **Decision:**
  - **`&` next to `>` stays in the redirect.** The segment splitter no longer splits at an `&` touching a `>`, so `2>&1`, `>&2`, and `&>file` stay whole. `writes_via_redirect` and `redirect_escapes` now read words through one parser (`redirect`), which tells copying a descriptor (`2>&1`, `>&-`) from writing a file (`&>out`, `>&out`, `&>>log`).
  - **`cd` into the project.** For roles that can't change files (plan, review, lead, architect, auditor), a `cd` into a folder inside the boundary is allowed. The rest of the chain is then judged from that folder, with it as the boundary.
- **Chosen vs rejected:**
  - Rejected putting `cd` on the read-only list. Later paths are checked from the project root, so `cd /etc && cat passwd`, or a symlink inside the new folder, would read past the check.
  - Rejected keeping the project root as the boundary after a `cd`. Narrowing it to the folder is never looser, and it catches a symlink where the command really runs.
  - The build hat keeps asking about `cd`, as before.
- **Why:** In real use the review hat refused `cd app && npm test 2>&1 | tail -25`, the usual way to test an app in a subfolder. Two rules caused it. `cd` wasn't allowed at all. And the `&` in `2>&1` split off a "command" named `1`, which every read-only role refused, so `cargo test 2>&1` failed too. Crew auditors hit the same rules.
- **Where:** `crates/ryter-core/src/tools/policy.rs` (`segments`, `decide_bash`, `cd_within`, `redirect`)
- **Residual risk:**
  - A `cd` inside a pipeline or subshell is treated as if it carries on to later commands. That's stricter than the shell, never looser.
  - `pushd` is still refused.

### 2026-09-23 — Review and commit inside Ryter: `/changes`, `/commit`, and a receipt
- **By:** lead
- **Decision:** `/changes` diffs a fresh snapshot of the files (the undo checkpoint's private-index method, so new files show and the staging area is untouched) against `HEAD` ("uncommitted") or the checkpoint where the latest build turn started ("last turn"). `x` undoes one file after taking a checkpoint, so `/undo` reverses it. The session records the turn's starting checkpoint separately (`turn_checkpoint`), so undoing a file doesn't move "last turn". `/commit` commits only the ticked paths (`git add -A -- <paths>`, then `git commit --only -- <paths>`), with the user's identity and hooks. The message is drafted by one tool-less, low-reasoning call from the diff (capped at 24k characters), the last 8 commit subjects, and the conversation without tool traffic. The optional `Ryter:` trailer gives the models and the project's spend since `HEAD`'s commit time, summed from every session's `spend.jsonl`, plus the latest test result. That result becomes "not rerun after the last edit" if the model edited files after the run. Receipts are on by default, shown in the preview, toggled with `t`, and saved as `[ui] receipts`.
- **Chosen vs rejected:** Rejected running git reads on the agent thread: a turn blocks that thread, and `/changes` should work mid-turn. Only writes (file undo, commit) wait for the turn. Rejected rename detection: a rename shows as a delete plus an add, so undoing one file stays one simple operation. Rejected the session's spend for the receipt: work spans sessions, and the logs already record every call with a timestamp. Rejected making the model commit through a tool: committing is the user's decision, and the harness does it exactly. Rejected receipts off by default: the preview shows the trailer before anything is committed, and it's one key to turn off.
- **Why:** The user had to leave Ryter to see the changes and commit after every task. The receipt records in the history what a change cost and how it was checked, which nothing else does.
- **Where:** `crates/ryter-core/src/review.rs`, `project.rs` (`spend_since`), `agent.rs` (`revert_file`, `draft_commit`, `one_shot`, `checkpoint_event`), `session.rs` (`turn_checkpoint`); TUI `panel/changes.rs`, `panel/commit.rs`, `run/worker.rs`, `run/events.rs` (latest test result); `prompts/solo.md`
- **Residual risk:**
  - The receipt's cost covers the project since the previous commit, so work thrown away in between is counted.
  - Tests the user ran outside Ryter don't count, and edits made with shell commands don't mark the tests as stale.
  - A binary file shows no diff.
  - The panels snapshot the files every time they refresh, which may be slow in very large repositories.

### 2026-09-22 — A command returns when its shell exits; what it left running is stopped
- **By:** lead
- **Decision:** `run_command` reads stdout and stderr on their own threads while the command runs. When bash exits, anything still in its process group (a server started with `&`) is killed, and the output says so: `[stopped the background processes this command left running]`. A process that left the group (`setsid`) and still holds a pipe gets 500 ms, then the command returns with what arrived and a note. The bash tool's description tells the model to start, test, and stop a server in one command. Process groups are probed and killed with bash's builtin `kill` (`kill -0 -- -PGID`, `kill -KILL -- -PGID`), in the shell tool and on cancel.
- **Chosen vs rejected:** Rejected leaving background processes running: nothing tracks them, they keep ports busy, and the next run collides. Rejected waiting for the pipes to close, the old behavior: a server never closes them. Rejected a background-job tool for now: the reported need was checking a server, which one command covers.
- **Why:** In a real solo build the model ran `(PORT=3210 node server.js &) && sleep 1.5 && curl …; pkill -f "PORT=3210"`. The `pkill` matched the command's own `bash -c` line, not the server. The server held the pipe, and Ryter waited on it past the timeout: the spinner never stopped. Separately, output over 64 KiB stalled every command until its timeout, because the pipes were only read after exit. On Ubuntu CI, procps-ng's `/usr/bin/kill -0 -PGID` called a dead group alive and a live one dead, and its group kill left a background process running.
- **Where:** `crates/ryter-core/src/tools/shell.rs` (`run_command`, `drain`, `group_alive`), `crates/ryter-core/src/cancel.rs` (`kill_group`), `crates/ryter-core/src/tools/mod.rs` (bash description)
- **Residual risk:** A server the model starts on purpose for the user is stopped too; the user starts long-running servers themselves. A reader thread stuck on a detached process's pipe lives until that process exits.

### 2026-09-22 — The chat narrates the work, and measures every step
- **By:** lead
- **Decision:** The solo prompt (and the lead's) asks for narration: what's next and why before each group of actions; each choice between approaches, and its reason, at the moment it's made; what went wrong and the next move after a failure. The chat shows each tool step with a verb and its measured outcome: `new · 48 lines`, `rewrote · 76 lines (was 89)`, an edit's changed lines, `✓ 13 passed`, or `✗ exit 1` with the cause line and tail inside the row. Lookups fold into one line, and a divider closes each working turn with files, commands, and time. The write, edit, and bash tools report what happened (created or rewrote, the lines that really changed, exit codes), so the model sees it too.
- **Chosen vs rejected:** Rejected only richer tool rows (the first proposal): the user is a developer who wants the reasoning, not only the activity. Rejected asking the model to report files and commands: Ryter measures those exactly, so the model spends its words on why. Rejected showing an edit's quoted context as removed and re-added: in a real run `.gitignore` showed two unchanged lines as churn. Rejected the last lines of a failure alone: npm put its cause above four lines of boilerplate.
- **Why:** "There was not much info returned in the chat… nothing more than wrote file." A 94-tool build had narration only at the end.
- **Where:** TUI `chat/toolview.rs`, `run/events.rs` (`on_tool_call`, `on_tool_result`, turn divider), `chat/mod.rs`, `chat/layout.rs`, `run/actions.rs` (replay through the live path, `strip_hat_note`); core `tools/fs.rs` (`changed_lines`, results), `tools/shell.rs` (exit code); `prompts/solo.md`, `prompts/orchestrator.md`
- **Residual risk:** Narration adds output tokens (a sentence or two per step). How well it's done depends on the model; test totals are recognised for common runners only.

### 2026-09-22 — The build hat asks, every time, to write outside the project
- **By:** lead
- **Decision:** A new gate decision, `AskOutside`, covers the build hat's writes outside the project: file tools, shell redirects, and commands with outside path arguments. It always prompts, marked "outside the project", with no "allow all"; `--always-approve` and a session's "allow all" don't satisfy it, so headless refuses. Refused whatever the prompt: credentials and secret files (read or write), and shell startup files, autostart entries, and system folders (write). `~`, `$HOME`, and `..` are resolved before judging. Reading outside is an ordinary ask. The file tools accept an approved outside path in the build hat only.
- **Chosen vs rejected:** Rejected keeping outside writes refused: a scratch venv in `/tmp` is normal work, and a command could already create one after asking while the write tool could not; the user saw a refusal with no question. Rejected letting "allow all" cover outside writes: it was given for the project. Rejected refusing system reads: `cat /etc/os-release` is how a model checks its environment.
- **Why:** In testing, the build hat's work in `/tmp` looked refused (the actual refusal was inline code, worded as if about location), and the user asked whether it could request to write outside the repo; it couldn't.
- **Where:** `tools/policy.rs` (`AskOutside`, `resolve_outside`, `forbidden`, `outside_segment`, `outside_decision`), `tools/mod.rs` (`OUTSIDE`, prompt handling), `tools/fs.rs` (`require_resolved`), TUI `panel/modal.rs`
- **Residual risk:** A script inside the project can still write anywhere when it runs. The gate judges commands, not what a program does, and the Landlock sandbox (`[sandbox] profile = "workspace"`) is the boundary for that.

### 2026-09-22 — Always send a reasoning effort to OpenRouter
- **By:** lead
- **Decision:** Every request to an OpenRouter connection carries `reasoning: {effort}`. The user sets a level per model with Tab in `/models`, `/crew`, or the crew builder (auto, low, medium, high, model's own), saved to `~/.ryter/reasoning.toml` and shown on the model card. The model's level wins wherever it runs. Auto falls back to `[reasoning_effort]` per role, then to the defaults: `high` for the plan hat and the architect, `medium` for every role that acts. Other providers get no field. The crew reads the choices from its meter.
- **Chosen vs rejected:** Rejected leaving the field out and letting each model choose: measured on glm-5.3-flashx with a real plan, no setting meant 191s and ~27k reasoning tokens before the first tool call (0.2.2 users saw "thinking" for minutes); `medium` took 7s, `high` 15s. Rejected raising the output ceiling further, as 0.2.2 did: the model filled it with reasoning. Rejected `low` for build: `medium` was as fast here, and it keeps some thinking for models that use it well. Checked that DeepSeek, Grok, Claude, GLM, GPT, and Qwen routes accept the field.
- **Why:** The user's build turns kept "writing everything into the thinking block". The user asked for the level to be their choice per model, not a hidden default: the same level means different things per model (medium is no visible reasoning on flashx, a few hundred tokens on Grok). This time the fix was checked in the TUI before release: plan, the hat prompt, then build in a fresh folder, and the app it wrote works.
- **Where:** `llm/mod.rs` (`CompletionRequest::reasoning`), `llm/http.rs` (`body`), `config.rs` (`reasoning_effort`, `effort_for`), `meter.rs` (`with_efforts`), `agent.rs`, `crew.rs`
- **Residual risk:** "medium" means different things per model; the effort each model gets is a judgment call to revisit with `ryter bench`. Direct Anthropic and xAI connections still get no setting (Anthropic's thinking is off unless asked for; Grok's isn't configurable).

### 2026-09-21 — Solo turns end like lead turns; cut-off replies continue
- **By:** lead
- **Decision:** Turn start/finish events are sent for every conversation turn (the lead's and every solo hat's), decided once at the start so a mid-turn hat switch can't lose the end. The conversation's output ceiling is 32,768 tokens, the same as a crew builder's. A reply cut off at the ceiling keeps its complete tool calls, drops a half-written one, and gets a note telling the model to continue in smaller steps and write code to files rather than reasoning. Three cut-offs in a row end the turn with a notice. An empty cut-off reply is stored as a placeholder, since some providers reject empty assistant messages.
- **Chosen vs rejected:** Rejected only raising the ceiling: a model that reasons a lot can use any ceiling, and the turn still has to survive it. Rejected a per-model ceiling: unused output isn't billed, so one generous ceiling costs nothing.
- **Why:** In hands-on testing, glm-5.3-flashx switched from plan to build, wrote the app into its reasoning until the 8,192-token ceiling, and returned nothing. Solo turns never sent TurnFinished, so the screen kept saying "thinking".
- **Where:** `agent.rs` (`turn`, `turn_inner`, `CONVERSATION_MAX_OUTPUT`, `MAX_CUTOFFS`), `prompts/solo.md`

### 2026-09-21 — On Linux, keys live in a 0600 file; the kernel keyring isn't storage
- **By:** lead
- **Decision:** Saving a key on Linux writes `~/.ryter/keys/<connection>` (mode 0600) and removes any kernel-keyring copy; lookups read the file before the keyring. macOS keeps the keychain, falling back to the file. Tests never touch the real keyring.
- **Chosen vs rejected:** Rejected the kernel keyring (`linux-native`): it is "completely in-memory and will not persist across reboots" (keyring crate docs), so 0.2.0 saved a key there, deleted the file, and lost the key at the next restart. Rejected Secret Service (GNOME Keyring, KWallet) for now: it needs libdbus, which breaks the static musl binaries, and headless servers have no desktop keyring. The file is what `~/.ssh`, the AWS CLI, and `gh` use without one.
- **Why:** Found while answering "where does Ryter store my OpenRouter key?". The user's key was safe only because it was saved before 0.2.0 preferred the keyring.
- **Where:** `config.rs` (`durable_keyring`, `store_secret_at`, `resolve_secret_with`, `keyring_*`)
- **Residual risk:** The key is plaintext on disk, protected by file permissions. A Secret Service backend could come back as an opt-in build feature.

### 2026-09-21 — The model asks to switch hats with a prompt, not in text
- **By:** lead
- **Decision:** A `request_hat` tool (solo hats only) shows the user a yes/no prompt ("switch to the build hat: carry out the plan"). On yes, the agent switches its role, permissions, and saved mode mid-turn, emits `ModeChanged` so the header and badge follow, and the model carries on in the new hat in the same turn. The prompt has no "allow all". Headless, it tells the user to rerun with `--hat`.
- **Chosen vs rejected:** Rejected treating "yes" typed in plan as consent to build: the next message still arrives in plan, and guessing intent from text would let a model talk its way out of a hat. Rejected leaving it to the prompt ("tell the user to press Tab"): it was already told, and a user who just read a plan wants to say yes, not learn a key.
- **Why:** In hands-on testing the plan hat ended with "want me to switch to build?" and there was no way to answer it.
- **Where:** `agent.rs` `request_hat`, `tools/mod.rs` (spec), `event.rs` `ModeChanged`, TUI `panel/modal.rs` (`is_hat`), `run/events.rs`, `prompts/solo.md`

### 2026-09-21 — Project cost is the sessions' logs, summed per repository
- **By:** lead
- **Decision:** Project cost adds up every session's `spend.jsonl` for sessions run in the project's git repository root or any folder under it; outside a repository, the folder. A running total with per-log byte offsets in `~/.ryter/projects/<root>.json` means each read covers only new lines; a missing, corrupt, or shrunk-log total is rebuilt. The spend card shows it; `p` in `/spend` shows totals by role, model, month, and solo vs. crew; `ryter spend --project` prints it. Unpriced calls are counted and flagged (`$14.20+`).
- **Chosen vs rejected:** Rejected a new end-of-session breadcrumb: every call is already logged as charged (since live run 2), and a session that crashes never reaches its end. Rejected keying by folder path: a subfolder or a different launch folder would split one project's history (the user's call: repository root). Rejected recomputing every log at every launch: it grows with every session.
- **Why:** Session cost resets with each session; "what has this project cost me?" is the question that decides between solo and crew, and between tiers.
- **Where:** core `project.rs`; TUI `info/cards.rs` (`project_label`), `panel/spend.rs`, `run/events.rs` (live add), `run/mod.rs`, `run/actions.rs` (refresh on `/spend`); CLI `ryter spend --project`
- **Residual risk:** A repository that is moved or renamed starts a new total; linking them by the repository's first commit is possible later. Sessions in a nested repository count toward the inner one only.

### 2026-09-21 — Two modes in one app: solo (one model, three hats) and crew
- **By:** lead
- **Decision:** Ryter starts in solo mode: one model in the user's tree, with `Tab` cycling build → plan → review. `/crew` enters crew mode (the crew builder the first time, straight in after); `/solo` leaves. The hats are roles (`SoloBuild`, `SoloPlan`, `SoloReview`) with one shared tool list and system prompt; the hat is a one-line note on each message, and the gate enforces it. Build checkpoints files before each turn (a commit object under `refs/ryter/undo/`, built with a private index) for `/undo`. Crew cards appear only in crew mode; the right-hand panel is wider (28/36/42 columns).
- **Chosen vs rejected:** Rejected a fork into a separate single-agent harness: the interface, providers, spend, budgets, permission gate, sessions, and memory are shared, and a mode is far cheaper than a second product. Rejected crew as a fourth Tab stop (the user's call): it changes who you talk to, not just what the model may do. Rejected a tool list per hat: every switch would re-bill the whole context. Rejected giving build the worktree builder's rules: a builder may run anything because its tree is thrown away; the user's tree isn't.
- **Naming:** first built as "normal mode"; renamed **solo** (the user's call) because "normal" made the crew sound abnormal and solo pairs with crew. Crew stays: a team with roles on one job, which is what it is (fleet would suggest many independent agents). `/normal` still works as an alias.
- **Why:** The user liked the interface most, and wanted the everyday single-agent flow without giving up the crew for big jobs. Solo mode also removes first-run friction (no crew, no auditor rule until `/crew`) and gives the benchmark its single-agent baseline.
- **Where:** core `role.rs`, `tools/policy.rs` (per-hat rows, `writes_via_redirect`), `tools/mod.rs`, `agent.rs` (`checkpoint_before_build`, `undo`), `git.rs` (`checkpoint`, `restore_checkpoint`), `prompts/solo.md`, `session.rs` (`mode`, `checkpoints`); TUI keys, header, composer badge, cards, `/crew` `/solo` `/build` `/plan` `/review` `/undo`; CLI `--hat`
- **Residual risk:** Build in headless needs `--always-approve` to edit, unlike the old headless crew. `/undo` restores files, not side effects of commands (installed packages, databases). The inbound MCP server still talks to the crew lead.

### 2026-09-21 — Ryter sets up git; budgets are opt-in; forms ask on Esc
- **By:** lead
- **Decision:** When the crew first needs a branch and the folder has no repository (or no commits), the harness runs `git init` (the user's `init.defaultBranch`, else `main`), writes a `.gitignore` for secrets and caches unless one exists, commits what is there, and tells the user with a notice. The session budget is off by default; the per-task cap defaults to $3 and the crew builder raises it to fit the chosen crew. `Esc` on a changed form asks "save your changes?" (y save · n discard · esc keep editing).
- **Chosen vs rejected:** Rejected having the lead model run `git init`: the lead can't run shell commands by design, and a deterministic harness step costs nothing and can't be skipped. Rejected refusing with "make a first commit": the user asked for work, not git chores. Rejected keeping a $5 default budget: stopping work the user didn't ask to stop surprised them twice on a large project. With no session budget the per-task cap is the only guard, and at $1 it would have cut off a strong model's design (~$1.04 on gpt-5.5), so it rose to $3. Rejected `^s` as the only way to save: people didn't find it.
- **Why:** The user's own testing: an empty folder hit "not a git repository", budgets of $5 and $10 stopped a real project, and `^s` felt clunky.
- **Where:** `git.rs` `ensure_repo`, `agent.rs` `open_patch`, `event.rs` `Notice`; `config.rs` spend defaults, `estimate.rs` `task_cap_for`; TUI `panel/widgets.rs` `save_prompt`, `panel/settings.rs`, `panel/budget.rs`, `panel/crew_builder.rs`
- **Residual risk:** The first commit includes whatever is already in the folder. The `.gitignore` covers common secrets (`.env*`, `*.pem`, `*.key`), but not every secret a project could hold. A folder inside another repository (a dotfiles-managed home, say) counts as a repository, so no new one is made there and the crew works in the outer one.

### 2026-09-21 — The user builds the crew; the tiers only recommend
- **By:** lead
- **Decision:** On first launch (no `crew.toml` and no `[specialists]`), and from `/crew` → `b`, a crew builder walks through a starting point, each of the four seats (lead included), the budget, and a review. Every seat shows a ★ recommendation and why the role matters, but any reachable model can be chosen. Before saving, each model gets one tiny request with a tool. The budget step estimates per task, per design, and per job size from token profiles measured on the paid live runs, and suggests a cap: on by default on first launch, one toggle from off.
- **Chosen vs rejected:** Rejected tiers as the product: a computed pick can be odd, and a curated list goes stale and still can't see an account's data policy. Rejected a monthly hand-maintained model list for the same reason. Rejected choosing the lead for the user: it runs on every message, and the user knows what they want to pay for it.
- **Why:** Users have their own reasons for picking models (zero data retention, a provider they trust, cost). A new user whose only key is SpaceXAI used to reach the auditor rule as a refusal on their first build; now the first thing they see is a guided setup.
- **Where:** TUI `panel/crew_builder.rs`, `run/actions.rs` (`probe_models`, `save_crew_setup`), `run/mod.rs` (first launch); core `tiering.rs` (`suggest_for`, `recommend_lead`, `probe`), `estimate.rs`, `config.rs` (`crew_unconfigured`); CLI `ryter crew check`
- **Residual risk:** The estimate's token profiles come from four runs on one crew, and models that reason heavily will exceed them. A probe proves a model answers with a tool, not that it builds well; that is what `ryter bench` is for.

### 2026-09-21 — Three ready-made crews: skiff, schooner, galleon
- **By:** lead
- **Decision:** `/crew` offers three crews by cost: **skiff** (budget models in every seat, the auditor still independent), **schooner** (budget builder, strong architect and auditor; the old `crew suggest`), and **galleon** (strong models everywhere, the builder included). Each is computed from the models the user can reach at current prices. For the strong seats, established vendors win when one is within half the top price; a price tie goes to the newer model; cloud `-latest` aliases are never picked.
- **Chosen vs rejected:** Rejected hard-coded model lists per tier: they go stale within months and name models the user may not have. Rejected pure price ranking for the strong seats: on the user's live OpenRouter catalog it seated `sakana/fugu-ultra` as the galleon auditor over Claude Opus, `claude-opus-4.5` over Opus 5 on a price tie, and then `openai/gpt-chat-latest`, a moving chat alias.
- **Why:** A larger project ran through a $5 and then a $10 budget. Choosing a cost level should be one decision, not three model picks.
- **Where:** `tiering.rs` (`Tier`, `suggest_tier`, `strongest`, `ESTABLISHED`), TUI `panel/crew.rs`, `ryter crew tiers`, `ryter crew suggest --tier`
- **Residual risk:** Price is still the main quality signal; the established-vendor list is a judgment call. What each tier really costs per task is unmeasured until `ryter bench` runs against all three.

### 2026-09-21 — The session budget is optional and one command away
- **By:** lead
- **Decision:** `/budget` opens a panel like the other configurable tools: live spend against the cap, a cap on/off switch that remembers the amount, the cap, the warning level, and the per-task cap (now saved in `settings.toml`). A **budget** card on the right shows the cap, used and left, or `off`, and opens the panel on click; the gauge moved there from the spend card. `/budget <n>` sets the cap, `/budget +n` raises it, and `/budget off` removes it. Changes apply to the running session and are saved as your default. The spend card and `/spend` say "budget off" rather than dropping the gauge. The default stays $5. `settings.toml` is now applied before a trusted project's config, so a project's `[spend]` cap overrides your saved default.
- **Chosen vs rejected:** Rejected defaulting to no budget. A crew can spend on its own, and one runaway task is caught only by the per-task cap. With the budget visible and one command from off, a $5 default costs anyone who doesn't want a cap one command. Rejected keeping the budget only in `/settings` as a number where 0 means off: the budget was there, and nobody would find it.
- **Why:** Some users want a hard stop, and others want to watch spend themselves. Both should be a choice made in the interface, not in a config file.
- **Where:** TUI `palette/registry.rs` `run_budget`, `run/actions.rs` `set_budget`, `info/cards.rs`, `panel/spend.rs`; `config.rs` `load_at`
- **Residual risk:** Saving any setting writes the current budget into `settings.toml`, including a project's cap if one was loaded. The budget should get its own saved key.

### 2026-09-21 — A budget stop says what it left, and the next message carries it
- **By:** lead
- **Decision:** When the session budget stops the crew, the harness writes the stop note itself: what is finished on the patch branch, what isn't, that nothing has landed, and how to continue. The crew report and note are kept in the session (`carry.md`) and put in front of the user's next message to the lead. `ryter -c -p` continues the latest session headless.
- **Chosen vs rejected:** Rejected letting the lead write the summary: the budget is spent, and the stop is the one moment it can't be asked. Rejected pushing the report into the transcript at stop time: it would leave two user turns in a row.
- **Why:** Live run 4 stopped at its cap with two of three tasks done. The user saw "budget exceeded", and headless had no way to continue. With the carried report, the continuation (run 5) requeued the task and landed the patch for $0.28.
- **Where:** `agent.rs` `budget_stop_note`, `turn_inner`; `session.rs` `set_carry` / `take_carry`; `ryter-cli` `--continue`
- **Residual risk:** A budget-stopped task is Blocked, and the lead has to set it back to pending. It did so in the live run, but the prompt doesn't spell it out.

### 2026-09-21 — The architect writes the design once
- **By:** lead
- **Decision:** `notes/architect.md` holds the design and the exact interfaces between tasks, in about 500 words. Briefs cite it instead of restating it, and DECISIONS entries are a few lines each.
- **Chosen vs rejected:** Rejected "put the interface in both briefs": every builder already sees `notes/architect.md`, so the rule made the most expensive model write the same thing three or four times.
- **Why:** In live run 4 the Opus architect cost $0.91 of $1.72: a 1,400-word note, a 1,300-word DECISIONS entry, and 2,700 words of briefs that repeated them.
- **Where:** `prompts/architect.md`
- **Residual risk:** Not re-measured after the change. Much of the architect's output was probably reasoning, which the prompt can't control; a per-role reasoning-effort setting would.

### 2026-09-21 — The lead routes the crew; there are no phases
- **By:** lead
- **Decision:** Every message goes to the lead. The lead answers, proposes a small edit, writes builder tasks itself, or queues an architect task, whose builder tasks run in the same drain. `hold: true` stops at the design. Plan/Build/Audit phases, `/plan` `/build` `/audit` `/handoff`, `ryter handoff`, and `--mode` are gone from the product; the header and composer say "lead".
- **Chosen vs rejected:** Rejected user-switched phases: a user who pasted a request into plan mode got a design and nothing else, and the models were confused about which phase they were in and what they could do. Rejected always running the architect: on a precise two-module request, the lead wrote the tasks itself and the whole run cost $0.15 (live run 3). The architect is worth its ~$0.70 on designs, not on specs.
- **Why:** The product is "tell the lead what you want and receive a patch". The user never talks to a specialist; specialists report through the chat.
- **Where:** `agent.rs` `drain_crew` (batches by task role), `queue.rs` (`role`, `hold`, merge-by-id, `dropped`), `prompts/orchestrator.md`, TUI header/composer/session card
- **Residual risk:** Routing quality is the lead model's judgment. A cheap lead that never calls the architect would build designs from thin briefs; the benchmark needs a multi-file case to catch that.

### 2026-09-21 — A specialist's failed tool call is a result, not the end of the task
- **By:** lead
- **Decision:** A tool error (unreadable file, bad path, failed command) goes back to the model as an error result; only cancellation ends a specialist. A reply cut off at the output limit is resumed (unparseable calls are dropped, the model is told it was cut off), up to three times. An architect that produces neither text nor tasks is Blocked, not Done. After every audit the worktree is reset, and `commit_all` never stages caches (`__pycache__`, `*.pyc`, `node_modules`, …).
- **Chosen vs rejected:** Rejected propagating tool errors: in live run 2 one `read_file` on a missing path killed a builder's whole task. Rejected raising output limits alone: the Opus architect hit its cap and the run returned nothing for $0.24.
- **Why:** Every failure mode here was seen in a paid live run, and each one wasted the spend before it.
- **Where:** `tools/mod.rs` `run_with_hooks`, `crew.rs` `run_specialist` / `sign_off` / `run_note_task`, `git.rs` `commit_all` / `discard_uncommitted`
- **Residual risk:** The cache exclude list is fixed; a stack with other generated directories needs a `.gitignore`.

### 2026-09-21 — Inline interpreter code stays refused; the refusal names the way round
- **By:** lead
- **Decision:** `python -c`, heredocs, and stdin-fed interpreters stay denied for every role. The denial now says to write a probe file and run it, which the gate allows, and the builder and auditor prompts say the same.
- **Chosen vs rejected:** Rejected allowing inline code for builders and auditors. A builder can already run any script it writes, so the refusal protects little against a builder, but the Landlock sandbox is off by default, so the gate is often the only check, and code on disk can at least be read.
- **Why:** In live run 3 both auditors met the refusal, were told only "outside policy", and hand-traced the code instead of probing it.
- **Where:** `tools/policy.rs` `bash_hint`, `tools/mod.rs` `gated_execute`, `prompts/auditor.md`, `prompts/builder.md`
- **Residual risk:** The gate is a guard against mistakes, not a sandbox. `[sandbox] profile = "workspace"` is the boundary.

### 2026-09-21 — Local connections are keyless and cost $0
- **By:** orchestrator
- **Decision:** A `local` connection kind (presets `ollama`, `lmstudio`, `llamacpp`) needs no key and sends no Authorization header. Its calls are priced at $0 in the meter and the lead's turns; its tokens still count against `task_max_tokens`. A refused connection fails at once with "is it running?". Idle timeout is 600s.
- **Chosen vs rejected:** Rejected leaving local calls unpriced (`$?.??`): the API cost is genuinely zero, and an unknown price would disable the dollar caps' meaning for the cheapest crew. Rejected retrying a refused local connection: a server that is not up will not be up in three seconds.
- **Why:** A local builder is the cheapest crew there is (`docs/cost.md`: ~0.24×), and nothing let a keyless server be connected.
- **Where:** `config.rs` (`connection_template`, `is_local`, `local_connections`), `llm/http.rs`, `meter.rs` (`with_free`), `agent.rs`
- **Residual risk:** $0 ignores electricity and hardware. Local models' tool use varies widely; `ryter bench` is how to tell whether one can build.

### 2026-09-21 — Crew suggestions rank by price, fenced by what price gets wrong
- **By:** orchestrator
- **Decision:** `ryter crew suggest` / `s` in `/crew` proposes a builder (a local model; else the lead's own model when builder-priced; else the cheapest in a band of 1/30–1/4 of the strongest), an auditor (strongest model from another vendor than lead and builder), and an architect (strongest under a ~$15/M blended ceiling). Excluded outright: router meta-models, negative prices, no tool support, windows under 64k, cloud `:variant` ids, and models more than 18 months older than the newest in the catalog.
- **Chosen vs rejected:** Rejected price alone: tuned against a live 446-model OpenRouter catalog, it picked a router priced at -1/token as the cheapest builder and 2023's gpt-4 as the strongest model. Rejected a hard-coded model list (stale within months). Rejected letting the builder floor overrule the lead model the user already chose.
- **Why:** Independence requires a second model, and tiering is where the savings are. Price is the only quality signal before a benchmark; the suggestion says so every time.
- **Where:** `crates/ryter-core/src/tiering.rs`, `ModelInfo.created` / `.tools`, TUI `panel/crew.rs`, `ryter crew suggest`
- **Residual risk:** Direct-provider catalogs (xAI, Anthropic) carry no release date or tool flag, so fewer fences apply to them. The ceiling and band are judgment calls to revisit with benchmark data.

### 2026-09-21 — The benchmark's ground truth is hidden tests
- **By:** orchestrator
- **Decision:** `ryter bench` runs each task through the real crew in a fresh repo and runs *hidden* acceptance tests, never shown to the crew, only after work lands. It reports landed, accepted, false passes (landed but failed hidden tests), and cost per accepted task. Every shipped task carries a reference solution, and a test proves each task is unsolved as shipped and solvable.
- **Chosen vs rejected:** Rejected measuring "landed" alone: that measures the auditor's opinion, not the work. Rejected running the lead in the loop: it adds variance and cost without telling us about builder/auditor tiering.
- **Why:** docs/cost.md's per-task numbers are assumptions; tiering decisions need measured cost per accepted task, and the false-pass rate is the only measurement of how far the auditor can be trusted.
- **Where:** `crates/ryter-core/src/bench.rs`, `bench/`, `ryter bench`
- **Residual risk:** Four small Python tasks are a smoke test, not a benchmark of real-world work; the suite must grow (multi-file, Rust/TS, parallel tasks) before its numbers mean much.

### 2026-09-21 — The user receives one patch, not a stream of merges
- **By:** user (product decision), orchestrator (design)
- **Decision:** In Build, tasks land on an integration branch (`ryter/patch-<id>-<n>`). The patch lands on the user's branch as one `--no-ff` commit only when every task in it is done, after a final run of the checks on the combined tree. A blocked task holds the whole patch until it is retried or dropped; a fix task lands into the same patch. If the user committed meanwhile, their branch is integrated in the patch worktree and any resolution is re-audited.
- **Chosen vs rejected:** Rejected landing each task on the user's branch as it passes (the previous design): the user saw a half-finished change and had to reason about partial states. Rejected pull requests as the default: more controlled, but the vision is that the user has nothing to act on until the whole change is in. PRs stay an option for teams.
- **Why:** The unit the user asked for is the change, not the task. One commit is one thing to review and one `git revert -m 1` to undo, and the combined checks catch tasks that pass alone but break each other.
- **Where:** `agent.rs` (`open_patch`, `try_land_patch`, `drain_crew`), `crew.rs` (`land_patch`), `session.rs` (`Patch`)
- **Residual risk:** A patch can wait indefinitely on a blocked task until someone retries or drops it; there is no `/patch drop` yet. Tasks were each gated against the patch as it stood, so a late-landing task is re-checked but not re-audited against earlier ones (the combined checks cover interactions).

### 2026-09-21 — Auditors must be different models from the lead and the builder
- **By:** user (product decision), orchestrator (design)
- **Decision:** Every auditor on the panel must be a different model from both the lead and the builder, compared after stripping the route (`x-ai/grok-4.6` = `grok-4.6` = `grok-4.6-latest`). Builds refuse before any builder runs and say how to fix it. `[[auditor.panel]]` seats all must pass, in order, stopping at the first FAIL; seats may have a focus and path globs, and at least one must cover every change.
- **Chosen vs rejected:** The user asked for "different from the lead". It is also enforced against the builder, because the builder defaults to the lead's model and a builder overridden onto the auditor's model would make the auditor review its own work. Rejected a quorum for now; all-must-pass matches "must sign off".
- **Why:** A pass from the model that wrote or directs the code is not a second opinion, and "a second model signs off" is the product's claim. Enforcing on resolved models also catches the silent fallback where an unresolvable auditor route becomes the lead's model.
- **Where:** `crew.rs` (`same_model`, `independence_problem`, `sign_off`, `Auditor`), `agent.rs` (`auditor_panel`), `config.rs` (`AuditorSeatConfig`)
- **Residual risk:** Model identity is by name. Two names for the same weights (a fine-tune, a vendor alias) pass the check.

### 2026-09-21 — The crew is metered and capped; context is scoped per role
- **By:** orchestrator
- **Decision:** Every specialist round is priced and attributed to a task and role, logged, and counted against the session budget. Each task has a USD cap and a billable-token cap (`[spend] task_budget_usd = 1.0`, `task_max_tokens = 1_000_000`). Builders and auditors see `notes/architect.md` plus the DECISIONS entries naming their files; the architect sees all memory. The lead's system prompt is built once per turn; a cache breakpoint rolls onto the newest message on Anthropic routes. Rejected tasks retry in their own worktree. Auditors are limited to 12 rounds / 4k output.
- **Chosen vs rejected:** Rejected batching audits across a patch (per-task review is what lets one bad task be rejected, and audits are the cheapest stage). Rejected caching the full memory instead of scoping it (scoping also shrinks what the model must attend to).
- **Why:** Crew spend was not measured at all, so the budget stop saw only the lead. Modelled per task (`docs/cost.md`), the old design cost 2× a single agent on auto-caching providers and ~5× on Anthropic; now ~1.5× with one model everywhere, and 0.35–0.55× with a cheap builder and strong reviewers. Tiering is the design's economic premise; the other changes remove waste so tiering can pay off.
- **Where:** `meter.rs`, `crew.rs` (`run_specialist`, `limits`, retry in place), `memory.rs` (`load_scoped_memory`), `llm/http.rs` (`mark_last_for_cache`), `agent.rs` (`turn_inner`, `record_crew_spend`), `docs/cost.md`
- **Residual risk:** The cost model's round counts and growth rates are assumptions until the task benchmark measures them. Relevance filtering of DECISIONS is by path mention; a decision that matters but names no path is missed by builders.

### 2026-09-21 — A fast path for trivial edits, approved by a person
- **By:** user (product decision), orchestrator (design)
- **Decision:** `propose_edit` lets the lead offer a replacement of at most 20 lines a side in one file. The user sees the diff and approves with `y`; that approval is the sign-off. `--always-approve` and the session-wide `a` do not apply, and headless it is refused.
- **Chosen vs rejected:** Rejected letting the lead write source directly. Rejected routing trivial edits through the crew (a builder, checks, and an audit to fix a typo cost more than the edit is worth, and send people to other tools for small work).
- **Why:** The rule "nothing merges without sign-off" is kept: a person seeing the exact diff is a stronger sign-off than a model.
- **Where:** `tools/mod.rs` (`propose_edit`, `gated_execute`), `tools/policy.rs` (`decide_proposal`), `user_io.rs` (diff summary), `prompts/orchestrator.md`
- **Residual risk:** The edit lands uncommitted in the user's tree; if it overlaps an open patch, the patch waits on it like any user edit.

### 2026-09-21 — Work lands only after checks and sign-off, integrated in the worktree
- **Amended** the same day by "The user receives one patch": tasks now land on a patch branch, and the patch lands on the user's branch; rejected tasks retry in place rather than from a fresh branch.
- **By:** orchestrator
- **Decision:** A build task is builder → commit → merge the user's branch *into the worktree* → harness-run `[auditor] checks` → auditor `VERDICT: PASS` → serialized `--no-ff` land. With the auditor off nothing lands; the branch waits. A conflict is resolved by a builder in the worktree and the result is re-audited. If the target moves while a task is gated, it re-integrates and re-runs the checks; the auditor re-runs only if a resolver changed code. A dirty user tree blocks only when its dirty files overlap the task's changed files.
- **Chosen vs rejected:** Rejected merging into the user's checkout and rebasing on conflict (the old path left the checkout mid-merge with markers when both failed). Rejected re-auditing after every clean re-integration (cost, and a clean merge of already-audited upstream work does not change what this builder wrote). Rejected letting the auditor decide whether to run tests.
- **Why:** The product promise is that you can stop watching. That needs a gate that is mechanical first (tests the harness runs), independent second (a reviewer that did not write the code), and never destructive to the user's checkout. Integrating into the worktree first means landing cannot conflict.
- **Where:** `crates/ryter-core/src/crew.rs` (`run_build_task`, `build_inner`, `run_checks`, `audit`), `git.rs` (`integrate`, `land`), `crew.md` §4
- **Residual risk:** The auditor is still the same model as the builder unless `/crew` says otherwise, so "independent" is a default to fix, not a fact (`crew.md` open question 2). With no checks configured, whether anything is tested is up to a model. An LLM resolving conflicts can misjudge intent; re-audit catches some of that, not all.

### 2026-09-21 — The planner is folded into the architect
- **By:** orchestrator
- **Decision:** Roles are orchestrator, architect, builder, auditor. Phases are plan → build → audit; the plan phase runs the architect. `planner` and the `architect` phase still parse; old sessions, logs, and saved crew rows load (a `planner` crew row routes the architect).
- **Chosen vs rejected:** Rejected keeping two pre-build roles. Rejected dropping the pre-build phase entirely (it is a useful guardrail: nothing writes source).
- **Why:** Planner and architect had identical tools and outputs and ran strictly in sequence, each in a fresh window that re-read the repository, losing detail at the handoff. Planning scope is largely what the orchestrator learns in conversation; the architect's job is to turn it into a shape and tasks.
- **Where:** `role.rs`, `phase.rs`, `prompts/architect.md`, TUI `/crew` and `/phase`
- **Residual risk:** One fresh window now carries both jobs, so a very large change may want the architect run more than once.

### 2026-09-21 — Project memory has serial writers; builders hand back
- **By:** orchestrator
- **Decision:** Only the orchestrator and the architect write `ROADMAP.md`, `DECISIONS.md`, and `notes/`. Builders are denied those paths and end with a `STATUS / FILES / DECISIONS / NOTES` handback; the auditor has no write tools. After a batch the orchestrator receives the crew report and records what matters.
- **Chosen vs rejected:** Rejected letting builders append to DECISIONS.md and resolving the conflicts. Rejected having the runtime append builder decisions verbatim (lossy in the other direction: every trivial choice would be recorded).
- **Why:** N builders in N worktrees each editing the same memory files conflicted on every parallel merge by construction, and the auditor's writes were discarded with its worktree. One writer at a time removes the conflict instead of resolving it.
- **Where:** `tools/policy.rs` (`decide_write`), `tools/mod.rs` (`tools_for`), `agent.rs` (`drain_crew`, `crew_report_message`), `prompts/builder.md`
- **Residual risk:** A decision reaches DECISIONS.md only if the orchestrator's model judges it worth recording.

### 2026-09-21 — Tasks carry a brief and a file scope
- **By:** orchestrator
- **Decision:** A task is `{id, title, brief, files}`. The brief is the builder's whole spec. The scheduler runs tasks with disjoint scopes in parallel and serializes overlapping or undeclared ones.
- **Chosen vs rejected:** Rejected running undeclared tasks in parallel and letting the merge sort it out (conflicts would become the common case). Rejected declared dependencies for now; queue order plus scope serialization covers the cases seen so far.
- **Why:** A task used to be a title string and a builder's brief was that title. Parallelism is the payoff for worktrees, and it is only safe when tasks do not touch the same files.
- **Where:** `queue.rs` (`Task`, `can_run_together`, `take_pending`), `todo_write` schema in `tools/mod.rs`
- **Residual risk:** Scopes are declared by a model. A builder can still edit outside its scope (it is told to say so in the handback, and the auditor checks scope), so overlap is prevented by convention plus review, not by the sandbox.

### 2026-09-21 — Shell commands are judged per segment, not by substring
- **By:** orchestrator
- **Decision:** `decide_bash` splits a command the way a shell would (quote-aware, including `$( )` and backticks) and takes the most restrictive verdict across segments. `NEVER` denies privilege escalation, disk/device writes, host config, and outbound shells for every role. An interpreter with no script file is denied. Destruction is judged before the role: non-writing roles never destroy, builders may inside their worktree, escaping it prompts.
- **Chosen vs rejected:** Rejected keeping a denylist of substrings. Rejected a pure allowlist with Ask for everything else: builders carry no `user_io`, so Ask is Deny for them, and a strict allowlist would have made the crew feature unusable.
- **Why:** The old gate matched four substrings against the whole string, so `rm -fr`, `rm -r -f`, `git clean -fdx` and `find -delete` were auto-allowed, and everything after the first command in a chain was never examined — `cargo test && rm -rf ~` passed the auditor's allowlist on its first two words. The unit that matters is the command a shell actually runs, and the question that matters is *reach*: destruction inside a disposable worktree is ordinary work, destruction outside it is not.
- **Where:** `crates/ryter-core/src/tools/policy.rs` (`segments`, `program`, `decide_segment`, `decide_git`)
- **Residual risk:** Still a heuristic. A builder that writes a script and runs it defeats the analysis by design — that is visible in the transcript, which is the trade. `program()` sees through `env`/`time`/`VAR=`, but an unusual wrapper could hide a command. `git` is judged by subcommand, so a new destructive verb needs adding to `GIT_NEVER`.

### 2026-09-21 — `Enter` is never an alias for allow (superseded 2026-09-27: Enter allows ordinary prompts after a 500 ms guard; destructive ones still take only `y`)
- **By:** orchestrator
- **Decision:** The permission modal accepts only `y` to allow. `Esc` and `n` deny; `a` still needs a second press.
- **Chosen vs rejected:** Rejected keeping `Enter` as a convenience accelerator.
- **Why:** `Enter` is the send key in the composer, so it is the most reflexively pressed key in the product. An undocumented alias on the one overlay that must be unmistakable means a habit can approve `rm -rf`. The modal body never advertised it either, so the UI was lying about its own contract.
- **Where:** `crates/ryter-tui/src/panel/modal.rs`
- **Residual risk:** Users who learned the alias will press `Enter` and see nothing happen. That is the safe direction.

### 2026-09-21 — Auto-merge refuses a dirty checkout and lands as one commit
- **Superseded** later the same day by "Work lands only after checks and sign-off": a dirty tree now blocks only when the dirty files overlap the task, and conflicts are resolved in the worktree.
- **By:** orchestrator
- **Decision:** Before merging a builder branch, refuse if `repo` has uncommitted changes and say why. Merge `--no-ff` so a task is one revertable commit, and report the pre-merge sha as an undo point. The auditor no longer runs with `always_approve: true`.
- **Chosen vs rejected:** Rejected refusing to merge onto `main` (people legitimately work there). Rejected a mandatory human review step for now — that needs a `/diff` surface first.
- **Why:** The merge runs in the user's working tree on whatever branch is checked out. On a dirty tree `git merge` can refuse or half-apply, and either way the user's in-progress work ends up mixed with a builder's. A fast-forward made the builder's commits indistinguishable from the user's history, so there was nothing to revert. And the auditor is the gate on all of this — gating everything else on a role that was itself ungated was incoherent.
- **Where:** `crates/ryter-core/src/crew.rs` (`run_build_task_inner`), `crates/ryter-core/src/git.rs` (`is_dirty`, `head`, `merge_branch`)
- **Residual risk:** The auditor is still usually the same model as the builder (crew defaults follow the orchestrator), so PASS is not an independent opinion. A `/diff` review surface before merge is the real fix and is on the roadmap.

### 2026-09-21 — Every tool result is capped; `read_file` pages
- **By:** orchestrator
- **Decision:** `ToolOutput` truncates at 32k bytes keeping both ends. `read_file` takes `offset`/`limit`, defaults to 2000 lines, and states how to continue. `bash` defaults to a 120s timeout with a per-call `timeout_secs` up to 600, and runs `-c` rather than `-lc`.
- **Chosen vs rejected:** Rejected capping only `bash`. Rejected a head-only truncation: a build's outcome is in its last lines.
- **Why:** Nothing capped output, and every result is appended to the transcript and re-billed on every later turn — one `cat Cargo.lock` is ~15k tokens for the rest of the session, and it drives an auto-compact that discards the conversation. Separately, the 30s shell timeout was below a cold `cargo test`, so the auditor could not run the commands its own allowlist exists to permit.
- **Where:** `crates/ryter-core/src/tools/mod.rs` (`cap_output`), `tools/fs.rs`, `tools/shell.rs`
- **Residual risk:** 32k is a guess, not a measurement. A model that needs a whole large file must page it, which costs turns. `bash` still buffers: there is no incremental output, so a long build shows nothing until it finishes.

### 2026-09-21 — A key lives in one store, and the SSRF guard resolves names
- **By:** orchestrator
- **Decision:** `store_secret_at` writes `home/keys/<name>` only when the keyring genuinely fails, removes a stale file when the keyring wins, creates at 0600 directly, and returns which store was used. `blocked_host` resolves a hostname and refuses unless every address is public; unresolvable names are refused; IPv4-mapped IPv6 is unwrapped. The Landlock writable set is enumerated (`tmp`, `logs`, `sessions`) instead of granting `~/.ryter`.
- **Chosen vs rejected:** Rejected trusting the keyring silently (the old code did both and swallowed the error). Rejected a denylist of metadata hostnames alone.
- **Why:** Three ways the same secret leaked. A plaintext key always existed on disk even when the keyring worked, so "use the keyring" was decorative. The SSRF check only tested literal IPs, so `metadata.google.internal` or any attacker-owned name pointing at 169.254.169.254 went straight through — the recorded residual risk said "DNS rebinding", but plain resolution was never checked at all. And the sandbox granted read/write on all of `~/.ryter`, which contains `keys/`, so even the `read-only` profile let a builder read every key.
- **Where:** `crates/ryter-core/src/config.rs` (`store_secret_at`, `SecretStore`), `tools/web.rs` (`blocked_host`), `sandbox.rs` (`writable_set`)
- **Residual risk:** Resolve-then-connect is still two steps, so DNS rebinding between them remains (the original concern, now the only one). Landlock has no negative rules, so anything added under `~/.ryter` that tools need must be listed explicitly. ABI V1 has no `AccessNet`: the sandbox is filesystem only and does not stop exfiltration.

### 2026-09-21 — A popout owns the body; the cards are not painted under it
- **By:** orchestrator
- **Decision:** While any panel is open, the info sidebar and the chat scrollbar are not drawn, the header switches to compact facts, and a panel may use the full body less two rows.
- **Chosen vs rejected:** Rejected widening every panel to the full body width (loses the floating read the border vocabulary was chosen for). Rejected adding more `Clear` calls inside the panel — nothing was leaking through it.
- **Why:** The cards kept their columns underneath a centred float, so a panel covered their left half and left sliced tails beside its border. `dim_region` only rewrites `fg`, so this was invisible on a real terminal and glaring in a style-stripped capture. The cards also cost `/help` the width it needed at 100×30, which is the first screen a new user reads. Hiding them while a panel has focus is one `if` and fixes all three; the header keeps the facts visible in that state.
- **Where:** `crates/ryter-tui/src/draw.rs`, `crates/ryter-tui/src/panel/mod.rs` (`rect`), `crates/ryter-tui/src/info/mod.rs`
- **Residual risk:** Transcript text still shows beside a panel, which is correct for a floating dialog but still looks like debris in a monochrome capture. The real gap is that snapshots cannot see style at all (`gaps.md` S-01).

### 2026-09-21 — Unknown spend renders as unknown, not as zero
- **By:** orchestrator
- **Decision:** With no priced turn yet, the budget gauge shows `?%` and `$?.?? of $<cap>` in the sidebar card, the `/spend` panel, and the header, and the header does not colour it as safe.
- **Chosen vs rejected:** Rejected hiding the budget row entirely (the cap is still worth knowing). Rejected keeping `unwrap_or(0.0)`.
- **Why:** `RYTER.md` already said unknown rates are `$?.??`, never a fake `$0.00`. A 0% bar backed by `$0.00` beside a session total of `$?.??` reads as "plenty of budget left" when the truth is "no idea", which is the exact fiction the spend system exists to prevent.
- **Where:** `crates/ryter-tui/src/info/cards.rs`, `crates/ryter-tui/src/panel/spend.rs`, `crates/ryter-tui/src/draw.rs`
- **Residual risk:** `over_budget` correctly does not trip on unknown spend, so an unpriced model has no cap at all. The UI no longer implies otherwise, but the guardrail is still absent.

### 2026-09-20 — User talks only to the orchestrator
- **By:** orchestrator
- **Decision:** One conversational partner; specialists are workers with their own context.
- **Chosen vs rejected:** Rejected “hats on one body” and rejected specialists as a second chat the user answers independently.
- **Why:** Interactive coding needs a single thread. Specialist Q&A either never reaches the orchestrator or gets copied (double-billed).
- **Where:** `prompts/orchestrator.md`, TUI header `ryter · orchestrator`
- **Residual risk:** Orchestrator can be blind to *why* unless DECISIONS.md is kept.

### 2026-09-20 — Why lives in DECISIONS.md, not in the orchestrator transcript
- **By:** orchestrator
- **Decision:** Specialists write short decision records on disk. Orchestrator loads ROADMAP + DECISIONS when answering “why,” not full specialist transcripts.
- **Chosen vs rejected:** Rejected stuffing Opus/GLM logs into a cheap orchestrator context.
- **Why:** Heterogeneous crews (Opus architect, DeepSeek orchestrator) do not share a mind. Code is the what; an obscure security call is the why. Relaying transcripts re-bills the same tokens every later turn.
- **Where:** `ROADMAP.md`, `DECISIONS.md`, `notes/*.md`, `crates/ryter-core/src/memory.rs`
- **Residual risk:** Specialists that skip the note leave the orchestrator guessing.

### 2026-09-20 — Planner/architect spawn; handback is notes not transcript
- **By:** orchestrator
- **Decision:** Queue drain follows phase: Plan→planner, Architect→architect, Build→builder+auditor, Audit→auditor. Handback is `notes/<phase>.md` plus session pass note. Chat shows tagged specialist text as display-only.
- **Chosen vs rejected:** Rejected stuffing specialist output into the orchestrator transcript.
- **Why:** User can see the work; orchestrator answers “why” from DECISIONS/notes/code. Fresh specialist windows stay cheap.
- **Where:** `crates/ryter-core/src/agent.rs` `drain_crew`, `crates/ryter-core/src/crew.rs` `run_note_task`, TUI `LogLine::Specialist`
- **Residual risk:** Drain is still sequential; per-role models not wired yet.

### 2026-09-20 — Crew defaults to the orchestrator
- **By:** orchestrator
- **Decision:** Planner, architect, builder, and auditor follow the live orchestrator provider and model until `/crew` (or an explicit `[specialists.*]` row) assigns one.
- **Chosen vs rejected:** Rejected shipping a factory split (OpenRouter Claude for plan/architect, SpaceXAI Grok for build/audit). Mixed crews stay allowed as an opt-in.
- **Why:** A split the user did not pick is surprising and can fail if only one key is set. The orchestrator they just selected is the obvious default.
- **Where:** `Config::route_for`, `Agent::specialist_stack`, TUI `/crew` (`crew_role_label`, default picker row)
- **Residual risk:** A previously saved `~/.ryter/crew.toml` still loads those assignments until the user picks `default` on each role.

### 2026-09-20 — `/skills` and `/hooks` are menus, not dump lines
- **By:** orchestrator
- **Decision:** `/skills` lists invocable skills and user commands, runs them (with optional args), and writes stubs under `~/.ryter/skills/` / `commands/`. `/hooks` lists/adds/removes lifecycle hooks persisted in `hooks.toml`.
- **Chosen vs rejected:** Rejected keeping `/skills` as a chat dump. Rejected a full in-TUI markdown editor for skill bodies (stub file + path in chat).
- **Why:** Skills are markdown on disk; hooks are config rows. Both need the same discover/add/remove loop as `/mcp`.
- **Where:** `Overlay::Skills`, `Overlay::Hooks`, `write_skill`, `save_hooks`
- **Residual risk:** Deleting a skill only works for files under `~/.ryter`; project overlay skills must be edited in git. Empty matcher means the hook runs on every matching event.

### 2026-09-20 — `/mcp` is its own window, not `/settings`
- **By:** orchestrator
- **Decision:** Inbound links + tokens and outbound server add/toggle/remove live in `/mcp`. Settings stay for later generic config. Live state is `~/.ryter/mcp.toml`; tokens are `~/.ryter/keys/mcp-inbound.toml` mode 0600.
- **Chosen vs rejected:** Rejected stuffing this into a general settings editor and rejected putting bearer tokens in `config.toml`.
- **Why:** MCP has two directions and a “what do I paste into Cursor” problem. A dedicated menu can show the stdio command, unix URI, TCP bind, and a one-line client snippet.
- **Where:** TUI `Overlay::Mcp`, `save_mcp`, `save_mcp_tokens`
- **Residual risk:** Unix listen is still started at TUI launch; turning inbound off in the menu does not unbind until restart. TCP enable from the menu does bind immediately.

### 2026-09-20 — Sessions are files you can resume
- **By:** orchestrator
- **Decision:** `/resume` picks a session for this cwd; `/rename` sets `meta.title`; `/delete` removes the directory only if `meta.json` is present. CLI: `ryter sessions`, `ryter resume [id]`.
- **Chosen vs rejected:** Rejected a global session switcher across directories and rejected deleting without the meta.json guard.
- **Why:** JSONL on disk is already the source of truth; the missing piece was a picker, not a new store.
- **Where:** `Session::list` / `find` / `set_title` / `remove`, TUI `ChoiceKind::Resume`
- **Residual risk:** Resume keeps the live HTTP provider; a session recorded on another connection only restores the model id if that connection still exists.

### 2026-09-20 — `/agents` kill is per-child cancel
- **By:** orchestrator
- **Decision:** Each specialist gets its own `Cancel`. Esc still fans out to all. `/agents` Enter cancels one; the queue item is `blocked`.
- **Chosen vs rejected:** Rejected using the parent turn cancel for kill-one (that would stop every sibling).
- **Why:** Parallel builders are the product; a runaway child should not take the rest of the batch down.
- **Where:** `Agent::running`, `Agent::kill_child`, TUI `Overlay::Agents`
- **Residual risk:** A builder killed after `git add` may leave a worktree; error paths now remove it, but a wedged `kill` can still race.

### 2026-09-20 — Cancel is a flag plus process-group kill
- **By:** orchestrator
- **Decision:** One `Cancel` token per turn (`AtomicBool` + registered pgids). TUI Esc (when busy), `/cancel`, and MCP `ryter_cancel` / `notifications/cancelled` set it. Bash children are `kill -KILL -$pgid`.
- **Chosen vs rejected:** Rejected only dropping the HTTP stream (leaves `sleep` running) and rejected `unsafe` `killpg`.
- **Why:** The plan required cancel to actually stop work. Unix process groups were already created for bash.
- **Where:** `crates/ryter-core/src/cancel.rs`, `tools/shell.rs`, `agent.rs` stream `select!`
- **Residual risk:** Same-connection cancel during stdio `ryter_prompt` needs the helper-thread read loop; a wedged provider that ignores drop may still run until timeout.

### 2026-09-20 — Inbound MCP on a socket is attach, TCP is token-gated
- **By:** orchestrator
- **Decision:** `ryter serve --socket` (default `~/.ryter/ryter.sock`, mode 0600) and TUI bind the same path when inbound is on. TCP `--bind` requires `--token` / `RYTER_MCP_TOKEN` on `initialize`; `0.0.0.0`/`::` requires `--i-mean-it`.
- **Chosen vs rejected:** Rejected HTTP wrapping and rejected binding unspecified addresses by default.
- **Why:** stdio is caller-owned; a running TUI needs attach; a local TCP port without a secret is an accidental open door.
- **Where:** `crates/ryter-core/src/mcp/listen.rs`, `ryter serve`
- **Residual risk:** Two TUI instances fighting one socket; stale sock is unlinked only if connect fails.

### 2026-09-20 — Specialists get a fresh window
- **By:** architect
- **Decision:** Pass note + task + project memory; never the orchestrator chat log.
- **Chosen vs rejected:** Rejected shared transcript across roles.
- **Why:** Cost and role confusion. The expensive model should think on the task, not the user’s small talk.
- **Where:** `crates/ryter-core/src/prompt.rs` `specialist_messages`
- **Residual risk:** Pass notes that are too thin starve the specialist.

### 2026-09-20 — Ask is a TUI prompt; headless stays fail-closed
- **By:** architect
- **Decision:** Destructive tools pause for `y` / `n` / `a`. `a` is session-sticky Allow. No TUI → deny with a clear error. `ask_user` is an orchestrator tool that uses the same channel.
- **Chosen vs rejected:** Rejected treating Ask as Allow in the TUI; rejected a second permission daemon.
- **Why:** The gate already existed; the missing piece was a human on the other end of it.
- **Where:** `crates/ryter-core/src/user_io.rs`, TUI `Overlay::Permission` / `Overlay::AskUser`
- **Residual risk:** A 300s timeout denies; a disconnected TUI denies.

### 2026-09-20 — Live knobs stay in sidecar files
- **By:** architect
- **Decision:** `/settings` writes `settings.toml`. User connections write `connections.toml`. First launch copies `config.example.toml` to `~/.ryter/config.toml` if missing. Trust is a TUI prompt when `.ryter/` exists and cwd is not in `trusted.json`.
- **Chosen vs rejected:** Rejected rewriting `config.toml` from the TUI.
- **Why:** Same pattern as crew/mcp/hooks; the user’s Test_Preset file stays untouched.
- **Where:** `save_settings`, `save_user_connections`, `write_default_config`
- **Residual risk:** Two files can disagree; load order is crew → mcp → hooks → connections → settings.

### 2026-09-20 — Web is opt-in and SSRF-blocked
- **By:** architect
- **Decision:** `[features] web = false` by default. When on, `web_fetch` / `web_search` are offered. Localhost, private, link-local, and unique-local addresses are refused.
- **Chosen vs rejected:** Rejected always-on browsing; rejected following redirects into private space.
- **Why:** A coding harness that can hit metadata IPs is a footgun.
- **Where:** `crates/ryter-core/src/tools/web.rs`
- **Residual risk:** DNS rebinding after the host check; HTML parsing for search is best-effort.

### 2026-09-20 — TUI borders are allowed; the no-boxes rule is retired
- **By:** architect
- **Decision:** `RYTER.md` now reads “TUI chrome is structural — borders group and separate, they never decorate.” Popout panels and modals are bordered (`╭╮╰╯`, heavy top edge for interrupts); the base layout keeps hairline rules. The four `┌┐└┘`/`╔` snapshot assertions are deleted, not weakened.
- **Chosen vs rejected:** Rejected keeping borderless offsets and adding more whitespace; rejected borders on every region (header, chat, info cards would compete with panels).
- **Why:** Config surfaces grew past what borderless offset can disambiguate — `/settings`, `/mcp`, `/crew`, `/provider` are forms with sections, and a floating panel stacked over a scrolling transcript needs a hard edge to read as modal. Bordered panels are also the only way a permission prompt can be unmistakable while text keeps streaming behind it.
- **Where:** `RYTER.md`, `crates/ryter-tui/src/panel/chrome.rs`, `crates/ryter-tui/snapshots/`
- **Residual risk:** Snapshot churn on any chrome tweak; the `UPDATE_SNAPSHOTS=1` path makes it cheap to accept an intended change and easy to accept an unintended one. Review the diff.

### 2026-09-20 — syntect + two-face for code highlighting
- **By:** architect
- **Decision:** Code blocks are tokenized with `syntect` (`fancy-regex` backend, no onig C build) using the `two-face` syntax bundle, and scopes are mapped onto Ryter’s own palette rather than a TextMate theme. Unknown or huge blocks (>2k lines) skip highlighting and render plain.
- **Chosen vs rejected:** Rejected `tree-sitter` (per-language grammars compiled in, heavy build); rejected `synoptic`/hand-rolled lexers (too few languages); rejected shipping TextMate themes (colors would not follow `/theme` or the 16-color degradation).
- **Why:** A coding harness spends most of its transcript on code. One dependency covers ~200 grammars, and a scope→palette map keeps every theme (including `default-16` and `NO_COLOR`) consistent.
- **Where:** `crates/ryter-tui/src/chat/highlight.rs`, `Cargo.toml`
- **Residual risk:** ~3 MB binary growth and a slower cold build; a pathological regex in a grammar could stall a render, bounded by the line cap.

### 2026-09-20 — User messages are left-aligned speaker blocks, not right-aligned bubbles
- **By:** architect
- **Decision:** Every message renders as a left-aligned block under a speaker header (`dusty`, `grok-4.6`, `· system`, tool rows). The user’s block is marked with a `▎` gutter in the accent color; nothing is right-aligned.
- **Chosen vs rejected:** Rejected chat-app bubbles pinned to the right edge.
- **Why:** Right-aligned text in a monospace terminal wraps badly, breaks copy/paste selection, fights the scrollbar column, and makes the sticky turn header impossible to place. Speaker + gutter gives the same “who said this” signal at zero layout cost.
- **Where:** `crates/ryter-tui/src/chat/mod.rs` (`header_row`, `render_message`)
- **Residual risk:** Long single-line user prompts look like a paragraph; the gutter is the only cue.

### 2026-09-20 — Reasoning is display-only
- **By:** architect
- **Decision:** Model reasoning streams into the activity strip (collapsed one-line ticker, `Ctrl+R` expands to a scrollable pane, `[ui] reasoning = "off"` hides it). It is never appended to the transcript, never persisted in the session, and never sent back to any model.
- **Chosen vs rejected:** Rejected storing reasoning as a message kind; rejected feeding a summary of it into the next turn.
- **Why:** Providers bill reasoning tokens and some forbid echoing them back; persisting it would bloat sessions and compaction. The user wants to *watch* the model think, not archive it.
- **Where:** `crates/ryter-tui/src/activity.rs`, `AgentEvent::Reasoning`, `crates/ryter-tui/src/run/events.rs`
- **Residual risk:** After a `/resume` the strip is empty for past turns; only the turn summary (`3 tools · 12.4k tok · 0:42`) survives.
