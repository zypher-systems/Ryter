# Decisions

Why, not what. Specialists append when they make a non-obvious choice.

### 2026-09-21 — Shell commands are judged per segment, not by substring
- **By:** orchestrator
- **Decision:** `decide_bash` splits a command the way a shell would (quote-aware, including `$( )` and backticks) and takes the most restrictive verdict across segments. `NEVER` denies privilege escalation, disk/device writes, host config, and outbound shells for every role. An interpreter with no script file is denied. Destruction is judged before the role: non-writing roles never destroy, builders may inside their worktree, escaping it prompts.
- **Chosen vs rejected:** Rejected keeping a denylist of substrings. Rejected a pure allowlist with Ask for everything else: builders carry no `user_io`, so Ask is Deny for them, and a strict allowlist would have made the crew feature unusable.
- **Why:** The old gate matched four substrings against the whole string, so `rm -fr`, `rm -r -f`, `git clean -fdx` and `find -delete` were auto-allowed, and everything after the first command in a chain was never examined — `cargo test && rm -rf ~` passed the auditor's allowlist on its first two words. The unit that matters is the command a shell actually runs, and the question that matters is *reach*: destruction inside a disposable worktree is ordinary work, destruction outside it is not.
- **Where:** `crates/ryter-core/src/tools/policy.rs` (`segments`, `program`, `decide_segment`, `decide_git`)
- **Residual risk:** Still a heuristic. A builder that writes a script and runs it defeats the analysis by design — that is visible in the transcript, which is the trade. `program()` sees through `env`/`time`/`VAR=`, but an unusual wrapper could hide a command. `git` is judged by subcommand, so a new destructive verb needs adding to `GIT_NEVER`.

### 2026-09-21 — `Enter` is never an alias for allow
- **By:** orchestrator
- **Decision:** The permission modal accepts only `y` to allow. `Esc` and `n` deny; `a` still needs a second press.
- **Chosen vs rejected:** Rejected keeping `Enter` as a convenience accelerator.
- **Why:** `Enter` is the send key in the composer, so it is the most reflexively pressed key in the product. An undocumented alias on the one overlay that must be unmistakable means a habit can approve `rm -rf`. The modal body never advertised it either, so the UI was lying about its own contract.
- **Where:** `crates/ryter-tui/src/panel/modal.rs`
- **Residual risk:** Users who learned the alias will press `Enter` and see nothing happen. That is the safe direction.

### 2026-09-21 — Auto-merge refuses a dirty checkout and lands as one commit
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
