# Decisions

Why, not what. The lead records non-obvious choices, its own and the crew's.

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

### 2026-09-21 — `Enter` is never an alias for allow
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
