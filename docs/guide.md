# Ryter user guide

Ryter is a Bring-Your-Own-Key terminal coding harness. You talk to an orchestrator. Independent specialists do the work.

Linux is the first platform. macOS and Windows are later ports.

## Install

Rust 1.88+ (see `rust-toolchain.toml`).

```sh
cargo build -p ryter-cli
./target/debug/ryter --version    # must not open config, keyring, or the network
./target/debug/ryter doctor
```

Put the `ryter` binary on your `PATH` if you want. Data lives in `~/.ryter/` (`RYTER_HOME` overrides).

## Keys and connections

SpaceXAI and OpenRouter are compiled in as equals. Other OpenAI-compatible or Anthropic Messages endpoints are user connections.

| | SpaceXAI | OpenRouter |
| --- | --- | --- |
| `kind` | `spacexai` | `openrouter` |
| Base URL | `https://api.x.ai/v1` | `https://openrouter.ai/api/v1` |
| Wire | Responses (default) | Chat Completions |
| Env | `XAI_API_KEY` | `OPENROUTER_API_KEY` |
| Default model | `grok-4.6` | `anthropic/claude-sonnet-4.6` |

Credential order per connection: TOML `api_key` → `env_key` → the stored key → well-known env (`OPENROUTER_API_KEY`, `XAI_API_KEY`).

**Reasoning.** Each model has a reasoning level you choose: **Tab** on a model in `/models`, on a role in `/crew`, or on a seat in the crew builder steps it through `auto → low → medium → high → model's own`. The model card shows the level in use (`reasoning  auto · medium`). The choice follows the model into every role that uses it, and is saved to `~/.ryter/reasoning.toml`; `[model_reasoning]` in `config.toml` does the same by hand, keyed by model id.

**Auto** means Ryter picks by role: `high` for the plan hat and the architect, `medium` for every role that acts. `[reasoning_effort]` overrides that per role (`build`, `plan`, `review`, `lead`, `architect`, `builder`, `auditor`). **Model's own** sends nothing. Beware: with no setting, some models think for minutes before acting. The level is sent only to OpenRouter connections.

Where a saved key (`/provider` set-key, `ryter connections set-key`) is stored: on **Linux**, `~/.ryter/keys/<connection>`, readable only by you (mode 0600). Linux's kernel keyring is in memory and doesn't survive a reboot, so it isn't used to store keys. On **macOS**, the keychain (`service=ryter`, `account=connection:<name>`), falling back to the file if the keychain refuses.

If `config.toml` contains `api_key` and is group/world-readable, Ryter refuses to start until the mode is `0600`.

First launch writes `config.example.toml` to `~/.ryter/config.toml` if that file is missing. Precedence, high wins: CLI flags > `RYTER_*` env > trusted project `.ryter/config.toml` > `~/.ryter/config.toml` > sidecars (`crew.toml`, `mcp.toml`, `hooks.toml`, `connections.toml`, `settings.toml`) > built-ins.

`--connection` / `/provider` pick the endpoint. In the TUI, `/` then arrows + Enter runs the highlighted command.

```
/provider                    floating list; Enter selects
                             no key → paste it in the same window; * marks a saved key
                             selected provider becomes the default
/models                      floating list; type to filter, arrows, Enter
```

CLI:

```
ryter connections                         list
ryter connections add NAME --kind openai  user row in ~/.ryter/connections.toml
ryter connections remove NAME             user rows only (built-ins stay)
ryter connections test NAME               list models (needs a key)
ryter connections set-key spacexai
ryter models [connection]
```

`/settings` edits budget, warn, max, sandbox, inbound MCP, and `[features] web` (persists `~/.ryter/settings.toml`). Sandbox changes apply on the next launch.

## Talking to Ryter

`ryter` on a tty opens the TUI. Top to bottom: a one-line header (`ryter · orchestrator`, project path, version), the chat transcript with a right-hand **info panel** of cards (session, model + context gauge, spend + budget gauge, tasks, crew, mcp), the **activity strip** while a turn runs, the bordered **composer**, and a hint bar showing the keys that matter right now. `^b` hides the info panel; it also drops automatically under 80 columns.

Every message is a left-aligned block under a speaker header — your name (from `[ui] username`, then `git user.name`, then `$USER`), the model name, `· system`, or a one-line tool row (`· read_file  path  0.1s`). Markdown renders with headings, lists, quotes, tables, and fenced code with syntax highlighting and a line-number gutter. Long model turns end with a summary line (`3 tools · 12.4k tok · 0:42 · $0.01`).

Type a message and press `Enter`. `Shift+Enter` (or `Alt+Enter`) inserts a newline; paste is bracketed so multi-line text lands in one message. While a turn runs, `Enter` queues the next message. `↑`/`↓` on an empty composer walk prompt history.

`/` (or `^p`) opens the **command palette**: fuzzy-matched, grouped by category, with a description and keybinding column. `Enter` runs the command, `→` opens its panel, `Tab` completes. Every configuration command opens a **panel** — a bordered popout with a title, status, and legend line — and panels stack: `/crew` → pick a role → `/models` opens on top; `Esc` closes one level.

The **activity strip** shows what the model is doing (`⠙ writing · edit docs/guide.md · 0:34 · 1.2k tok`). Reasoning streams there as a one-line ticker; `^r` expands it to a scrollable pane (`Alt+PgUp`/`Alt+PgDn`). Reasoning is display-only — it is never saved or sent back to a model. `[ui] reasoning = "off"` hides it.

**Scrolling.** The transcript follows the bottom until you scroll up (`PgUp`, `Shift+↑`, or the mouse wheel over the chat); then it holds still and the scrollbar turns amber. `Ctrl+End` reattaches. `Ctrl+↑`/`Ctrl+↓` jump between turns. While detached, a sticky header at the top of the chat keeps the in-flight user message in view.

`/sessions` (alias `/resume`) is one browser for this directory’s sessions: `Enter` resumes, `r` renames, `d` deletes (type the short id to confirm), `n` starts a new one. `/agents` lists running specialists; `Enter` or `k` kills one, `K` kills all. `ryter sessions` and `ryter resume [id]` are the CLI equivalents.

Permission prompts, `ask_user` questions, and the first-run “trust this project?” prompt are **modals** with a heavy top border. They block input until you answer (`y` / `n` / `a` for permissions, number keys or free text for questions), but the turn keeps streaming behind them.

### Keys

`/help` (or `F1`) lists every binding, filterable by typing. The essentials:

| Key | Does |
| --- | --- |
| `Enter` / `Shift+Enter` | send / newline |
| `/` or `^p` | command palette |
| `Esc` | back one level: close panel → close palette → cancel turn |
| `^c` | clear composer → cancel turn → quit (press twice within 2 s) |
| `^d` | quit when the composer is empty |
| `^r` | toggle the reasoning pane |
| `^b` | toggle the info panel |
| `^l` | redraw |
| `PgUp` / `PgDn` | scroll the transcript one viewport |
| `Shift+↑` / `Shift+↓` | scroll one row |
| `Ctrl+Home` / `Ctrl+End` | top / bottom (re-engages follow) |
| `Ctrl+↑` / `Ctrl+↓` | previous / next turn |
| `F1` | `/help` |

Inside a panel: `↑↓` move, `PgUp`/`PgDn` page, `Enter` activate, `Tab` next field, `Space` or `←→` change a toggle/select, `Esc` back. `Esc` on a form with changes (`/settings`, `/budget`) asks whether to save them; `^s` still saves at once. Each panel’s legend line names its own extra keys (`t` test a connection, `d` remove, `c` compact, and so on).

Mouse: wheel scrolls the chat, clicking a card opens its panel, clicking the activity strip toggles the reasoning pane. Text selection uses your terminal’s own modifier (Shift+drag on most). `[ui] mouse = false` turns capture off entirely.

Without a tty, use headless:

```sh
ryter -p "add a --json flag" --always-approve
ryter -p "…" --json                  # NDJSON AgentEvent stream
ryter -c -p "continue"               # continue the latest session: transcript, tasks, open patch
```

`--always-approve` treats Ask as Allow. Deny still wins.

An empty folder, or one that is not a git repository, works as is. Before the crew's first build, Ryter runs `git init` (your `init.defaultBranch`, else `main`), writes a `.gitignore` for secrets and caches unless one exists, commits what is already there as the starting point, and says so in the chat. A repository with no commits gets just the first commit.

## Solo mode and hats

Ryter starts in solo mode: one model in your project. `Tab` switches its hat (build → plan → review), `Shift+Tab` goes back, and `/build`, `/plan`, `/review` jump to one. The header, the message box's badge, and its border all show the hat in its own color. A switch applies to your next message. The model can also offer a switch itself: after a plan ("carry out the plan?") or a review ("fix these?") it asks with a yes/no prompt, and on `y` it carries on in the new hat in the same turn.

| Hat | May | May not |
| --- | --- | --- |
| **build** (default) | edit files and run commands; edits and commands that change things ask (or run with "allow all" / `--always-approve`); destructive commands always ask; outside the project, writes ask **every time** (see below) | read secrets, push, run inline interpreter code |
| **plan** | read, search, run read-only commands, write `notes/` and project memory | edit source, run anything that changes the project |
| **review** | read, run the tests and linters, read-only git | write anything, not even by redirect |

**What the chat shows.** The model narrates as it works: what it's doing next and why, each choice between approaches with its reason, and what it thinks went wrong when something fails. Each tool step shows what came of it, measured by Ryter: `new · 48 lines`, `rewrote · 76 lines (was 89)`, an edit's changed lines, `✓ 13 passed`, or `✗ exit 1` with the cause. Reads fold into one line, and a divider closes each turn that did work (`6 files (3 new, 3 changed, +153 −15) · 9 commands (9 ok) · 2:41`).

**Outside the project.** The build hat can write elsewhere on your machine, such as `/tmp` or another folder, but only by asking each time. The prompt says "outside the project" and offers only `y` (allow once) or `n`. "Allow all" and `--always-approve` cover the project, not the rest of the machine, so headless refuses these writes.

Some places are refused however they're asked for:
- **Never read or written:** credentials (`~/.ryter`, `~/.ssh`, `~/.gnupg`, `~/.aws`, `~/.config/gh`, and the like) and secret files.
- **Never written:** shell startup files (`~/.bashrc`, `~/.zshrc`, `~/.profile`, …) and system folders (`/etc`, `/usr`, …).

Reading outside the project is an ordinary question. Plan and review never write outside, and crew builders stay in their worktrees.

Every hat shares one system prompt (`prompts/solo.md`) and one tool list. The hat is a one-line note in front of each message, and the permission gate enforces it, so switching never throws away the provider's prompt cache.

Before each build turn, Ryter snapshots your files as a git object under `refs/ryter/undo/`. Your branch, staging area, and files aren't touched. `/undo` puts the files back as they were before the last build turn that changed them, deleting files it created. A folder that isn't a repository gets git set up first, and Ryter says so. It won't make one in your home folder or at the root.

### Review and commit

**`/changes`** lists every changed file with its diff. You can compare against either of two points:
- **Uncommitted** (the default): the last commit. This is what `/commit` would commit.
- **Last turn** (press `Tab`): the snapshot taken when the latest build turn started, so you see just what that turn did.

Keys:
- `↑↓` picks a file, and `PgUp`/`PgDn` scroll its diff.
- `x` puts one file back as it was at that point, deleting it if the file is new. Ryter snapshots first, so `/undo` brings it back.
- `c` opens `/commit`.

The panel updates when a turn finishes. You can open it while a turn is running, but undoing a file and committing wait for the turn to finish.

**`/commit`** lists the uncommitted files, all ticked; `space` unticks one.
- **The message.** The model drafts it from the diff, your project's recent commit subjects (so it follows your style), and the conversation, including the model's narration of why. `e` edits it in the message box (`Shift+Enter` or `Alt+Enter` for a new line), and `d` drafts again. The draft is one small model call at low reasoning, and its cost shows in the spend card.
- **Committing.** `Enter` commits only the ticked files, with your git identity and your hooks. Anything you'd staged yourself stays staged and out of this commit. If a hook refuses, the panel shows why.
- **The receipt.** With receipts on (the default; `t` switches, and `[ui] receipts` remembers), the message ends with a trailer:

  ```
  Ryter: deepseek-pro-latest · $0.34 · tests ✓ 13 passed
  ```

  - **Model:** the models used since the previous commit.
  - **Cost:** what this project spent since the previous commit, across sessions, read from the spend logs. It shows `$0.34+ (some prices unknown)` when a call had no known price.
  - **Tests:** the latest test run's result. It says `tests not rerun after the last edit` if the model changed files after that run, or `no tests run`.

  `git log --grep "Ryter:"` finds them later.

Headless, `ryter -p` runs in build; `--hat plan|review|crew` picks another. Headless nobody can approve an edit, so pass `--always-approve` to let build change files.

## Crew mode

`/crew` switches to crew mode. The first time, the crew builder opens (below); once a crew is saved, `/crew` switches straight to it, and in crew mode `/crew` opens the crew's settings. `/solo` goes back. In crew mode the right-hand panel adds the tasks and crew cards.

## The lead

In crew mode every message you send goes to the lead. You never talk to a specialist; they report back through the chat. The lead's prompt is `prompts/orchestrator.md` (overridable). It may read the repo, grep, glob, and call `todo_write`. It cannot write product source. For each request it does one of these:

| The request | What the lead does |
| --- | --- |
| A question | answers it |
| A trivial edit | `propose_edit`: you see the diff and press `y` |
| A precise change | writes builder tasks itself |
| Something that needs a design | queues an architect task; the architect's builder tasks run in the same pass |
| "Design it, don't build yet" | the same, with `hold`: the design waits for your go-ahead |

There is no mode to switch. Specialists get a **fresh window**: their task brief, `RYTER.md` / `AGENTS.md`, and the project memory scoped to their files, not the chat history.

Project markdown is loaded from the working tree without a trust gate: `RYTER.md`, or `AGENTS.md` if `RYTER.md` is absent.

## Build workers, auditor, merge

`todo_write` **is** the work queue. After a lead turn with no remaining tool calls, Ryter drains pending tasks, up to `[subagents] max` in parallel (must be ≥ 1). Each task names its role: architect tasks run first, then builders, each gated by checks and an auditor. Tasks declare the `files` they own; disjoint tasks run in parallel, overlapping or undeclared ones one at a time.

Crew roles default to the lead’s current provider and model. Assign a different model per role with `/crew` (Enter on a role, first picker row is `default`). That is also how you split providers. Optional `[specialists.*]` tables in `~/.ryter/config.toml` pin the same overrides.

Each builder task:

1. `git worktree add` under `~/.ryter/worktrees/<session>/<task>/` on branch `ryter-<8hex>-<slug>`
2. The builder implements its brief and ends with a handback (`STATUS / FILES / DECISIONS / NOTES`)
3. The runtime commits, then merges **your branch into the worktree**. Conflicts are resolved there by a builder — never in your checkout — and the resolution is re-audited
4. **Checks**: `[auditor] checks` run in the worktree (set them per project in `.ryter/config.toml`). A failure rejects the work before any audit
5. **Auditor**: reviews the brief, handback, check output, and full diff, and ends with `VERDICT: PASS` or `VERDICT: FAIL`
6. **Land**: one `--no-ff` merge commit (undo with `git revert -m 1`). If your branch moved meanwhile, it re-integrates and re-checks first. If you have uncommitted edits to the same files, it stops and keeps the branch
7. Rejected → retry with the findings, up to `[auditor] max_retries`, then `blocked`

Tasks land on a patch branch (`ryter/patch-…`), not yours. When every task in the patch is done and the combined checks pass, the patch lands on your branch as **one commit** (`git revert -m 1` undoes it all). A blocked task holds the patch until you retry or drop it; the lead says what it is waiting on.

Auditors must be different models from the lead and the builder — otherwise builds refuse to start and say how to fix it. Assign one in `/crew`, or list a panel under `[[auditor.panel]]` (all must pass; cheapest first; seats may have a `focus` and `paths`).

For a trivial change the lead can `propose_edit`: you see the diff and press `y`. Only a person can approve it.

**The crew builder** is where a crew is set up. It opens the first time you type `/crew`, and from the crew settings with `b`. When you save, you're in crew mode. It walks through seven steps:

1. A starting point: skiff, schooner, galleon, or your current crew.
2. The lead.
3. The architect.
4. The builder.
5. The auditor.
6. Budget: pick the job size (small, medium, or large) and see an estimate per task, per design, and for the whole job, with a suggested cap.
7. Review.

Each seat step says what the role does, puts a ★ recommendation first, and lists every model you can reach (type to filter). The auditor step refuses the lead's and builder's models. Review sends each model one tiny request with a tool (well under a cent) and saves only when all of them answer. That catches what no catalog shows: an OpenRouter data policy such as zero data retention that leaves a model no provider, missing tool support, a model you have no access to, or no credits. `ryter crew check` runs the same test on your saved crew. Esc on first launch means "later"; the builder doesn't come back on its own.

The estimate uses token counts measured on paid runs (`crates/ryter-core/src/estimate.rs`). It is rough, and a job is usually larger than it looks.

Three ready-made crews sit in `/crew`, picked from every model you can reach at today's prices, so they never name a model you can't use or one that has gone stale:

| Crew | Cost | Builder | Architect and auditor |
| --- | --- | --- | --- |
| **skiff** | low | a budget model | the best of the budget models; the auditor is still a different model from another vendor |
| **schooner** | balanced | a budget model | strong models (the default suggestion, `s` in `/crew`) |
| **galleon** | high | a strong model | strong models, the auditor from another vendor |

Enter on one previews it; `y` applies it and keeps your previous crew as the `before-suggest` preset. `ryter crew tiers` shows all three; `ryter crew suggest --tier galleon --apply` applies one from the shell. Strong seats prefer established vendors when one is close in price: price is the only signal before `ryter bench`, and on a live catalog it put an obscure model ahead of Claude Opus. None of the crews changes the lead. A local model server works as a connection with no key: `ryter connections add box --kind ollama --model qwen3-coder:30b`. `ryter bench` runs `bench/` through the crew and reports what landed, what passed hidden tests, and the cost per accepted task — it spends real money.

Crew spend is metered per task and role and counts against `[spend] session_budget_usd`; each task also stops at `task_budget_usd` / `task_max_tokens`. See `docs/cost.md`.

`/auditor on|off` is session-only unless you also change config. With the auditor off, **nothing merges**: finished work waits on its branch. After each batch the lead gets the crew report, tells you what landed, and records builder decisions in `DECISIONS.md` — builders never write project memory themselves.

The architect runs in-process (no worktree) and writes tasks straight into the queue builders read from. Nested subagents are not supported.

## Spend

Every model call is priced before the next request. Roll-ups: session, turn, role, connection. Persisted in `spend.jsonl`.

Sources, high wins: TOML `[pricing."<model>"]` → OpenRouter catalog (when ingested) → shipped SpaceXAI table. Provider-reported cost on a stream wins for that turn.

Unknown rates show `$?.??` plus token counts. Ryter never invents `$0.00` for an unpriced model.

A session budget is optional. With one, the crew stops when spend reaches it, says what finished and what didn't, and waits (exit `3` in headless). Without one, nothing stops on cost and you watch the spend card.

| Command | Effect |
| --- | --- |
| `/budget` | the budget panel: spend against the cap, on/off, the cap, the warning level, and the per-task cap (`^s` saves) |
| `/budget 5` | cap this session at $5 |
| `/budget +2` | raise the cap by $2, e.g. after hitting it |
| `/budget off` | no cap |

The **budget** card on the right shows the cap, how much is used and left, or `off`; click it to open the panel. Changes apply at once and are saved as your default (`~/.ryter/settings.toml`, the same value as *budget usd* in `/settings`). A trusted project's `[spend] session_budget_usd` overrides your default in that project. There is no session budget until you set one; the crew builder suggests one sized to the job. Each task is still capped at `[spend] task_budget_usd` ($3 by default; the crew builder raises it when your crew's normal design would not fit), which catches one runaway task whether or not there is a session budget. `[spend] enabled = false` still counts in memory and prints a warning.

**Project cost.** A project is its git repository (the folder, outside one), so sessions started in any subfolder count toward it. The spend card shows `project` under the session total, and `p` in `/spend` switches to the project view: the total across sessions, this month, solo vs. crew, and breakdowns by role, model, and month. `ryter spend --project` prints the same. Nothing extra is recorded: every call is already in its session's `spend.jsonl`, and a running total in `~/.ryter/projects/` means only new lines are read. Calls with no known price are counted and shown (`$14.20+`), never added as $0.

`/spend` is a panel: session total, a budget gauge, and tables by role and by connection; `p` switches to the project; `e` exports CSV. The info panel’s spend card shows the total, and the budget card below it shows the cap. `ryter spend` prints the roll-up on the CLI.

## Slash commands

Type `/` to open the palette; every built-in has a one-line description there. Configuration commands open panels: `/settings` `/provider` `/models` `/crew` `/mcp` `/skills` `/hooks` `/sessions` `/agents` `/spend` `/theme` `/tools` `/auditor` `/context` `/doctor` `/help`. `/changes` and `/commit` open panels too (see [Review and commit](#review-and-commit)). Direct commands act immediately: `/undo` `/new` `/rename <title>` `/budget [amount|+amount|off]` `/compact` `/cancel` `/quit`. Near-duplicates are hidden aliases (`/resume` → `/sessions`, `/model` → `/models`, `/connections` → `/provider`); `/delete [id]` stays as a hidden direct command.

User-invocable skills and `~/.ryter/commands/*.md` join the palette under **skills**. Built-ins win on a name clash.

## MCP

**Outbound.** `[mcp_servers.<name>]` stdio children. The orchestrator discovers with `search_tool` and calls with `use_tool`. Child env does not inherit API keys unless that server’s `env` table asks. In the TUI, `/mcp` is a panel: `Enter` toggles a server, `r` reconnects, `d` removes it (type the name to confirm), and `Enter` on the trailing `+ add server` row walks name → command → args → review. Each server row shows its live status (`connected · 5 tools`, `error: …`, `disabled`).

**Inbound.** `/mcp` → **inbound** shows the listener state and the links a client needs:

- stdio: `ryter mcp serve`
- unix attach (this TUI): `unix:///…/ryter.sock`
- TCP: `127.0.0.1:8765` plus a bearer token (`initialize.params.token`)
- snippet: `{"mcpServers":{"ryter":{"command":"ryter","args":["mcp","serve"]}}}`

Enter on **token** creates or rotates a `ryt_…` secret stored in `~/.ryter/keys/mcp-inbound.toml` (mode 0600); it is masked until you press `v`. Enter on a link copies it into the chat so you can paste it. Live flags persist in `~/.ryter/mcp.toml` (does not rewrite `config.toml`).

Another agent can also spawn `ryter mcp serve` (stdio), or `ryter serve --socket` / `--bind`. Tools: `ryter_prompt`, `ryter_status`, `ryter_spend`, `ryter_cancel`. Resources: `ryter://session/transcript`, `ryter://session/spend`. Keys are never returned.

TCP requires `--token` (or `RYTER_MCP_TOKEN`) on `initialize.params.token`. Binding `0.0.0.0` / `::` requires `--i-mean-it`.

`ryter mcp serve` uses a restricted always-approve: workspace file edits, read, grep, tests; deny `rm -rf`, credential paths, work outside cwd. `--always-approve` only widens this if `[mcp] allow_dangerous = true`.

`[mcp] inbound = false` disables the server. Esc or `/cancel` (or `ryter_cancel`) stops the in-flight turn and kills bash process groups.

## Customization

| Kind | Where |
| --- | --- |
| Prompts | `prompts/*.md`; override `~/.ryter/prompts/` then trusted `.ryter/prompts/` |
| Skills | `/skills` panel. Files: `~/.ryter/skills/<name>/SKILL.md` (frontmatter `user-invocable`). `Enter` runs (optional args), `e` opens the file in `$EDITOR`, `a` writes a stub, `d` deletes a user skill (not a project overlay). |
| User slash | Same `/skills` list (`command` rows). `~/.ryter/commands/<name>.md` (`$ARGUMENTS`) |
| Hooks | `/hooks` panel. `a` adds: event → command or URL → optional matcher. `d` removes. Live list is `~/.ryter/hooks.toml` (does not rewrite `config.toml`). Command gets JSON on stdin; exit 2 or HTTP 403 denies. |
| Themes | `/theme` panel previews as you move: `dark`, `light`, `default-16`, or `~/.ryter/themes/<name>.toml`. `Enter` persists to `~/.ryter/settings.toml`. `NO_COLOR` or a 16-color `TERM` degrades automatically. |
| UI | `[ui]` in `~/.ryter/config.toml`: `username`, `theme`, `reasoning`, `mouse`, `panel`, `colors`, `timestamps`, `line_numbers`, `receipts`. All optional; unknown keys warn once at startup. See `config.example.toml`. |

Project `.ryter/` overlays apply only after `ryter trust` (cwd is recorded in `~/.ryter/trusted.json`). Untrusted projects still load `RYTER.md` / `AGENTS.md`.

Skill example:

```markdown
---
name: review
description: Review the current diff
user-invocable: true
---
Look at `git diff` and report findings.
```

## Context

`/context` opens a panel with estimated tokens vs the model window (500k for `grok-4.6`, 200k otherwise), a gauge, and a breakdown by contributor (system prompt, project files, transcript, tool output); `c` compacts. The info panel’s model card shows the same gauge. Auto-compact at 85%: older turns collapse to tools used, files touched, and the latest pass note; the last four user turns stay. `/compact` forces a pass. Resume reads the rewritten `transcript.jsonl`.

## Doctor and sandbox

`ryter doctor` (and the `/doctor` panel, which runs the checks off-thread and can save the report with `c`) checks OS, tty, home, config, both built-in connections (key set/missing, never printed), spend catalog, git, Landlock, sandbox profile, and whether `.ryter/` is trusted. No network.

`--sandbox workspace` Landlock-restricts the tool thread to the project tree (writable) plus `~/.ryter/{tmp,logs,sessions}` — never `~/.ryter/keys`. The sandbox is filesystem-only; it does not restrict network. `--sandbox read-only` makes the project tree read-only. `--sandbox off` is the default. A non-off profile **refuses to start** if the kernel cannot enforce Landlock. `/tmp` itself is not granted; scratch is `~/.ryter/tmp`. Sandboxed runs use a current-thread tokio runtime.

## Safety

- One gate: `decide(role, tool, args)` → Allow / Ask / Deny. Role masks omit tools the model should not see.
- Orchestrator: read, list, grep, glob, `todo_write`, MCP. Cannot write `src/`.
- Architect: read tools, project memory, `todo_write`.
- Builder: full tool set in its worktree; denied project memory files.
- Auditor: read tools + test/lint/read-only-git bash; no write tools.
- Denied even for builders: `.env`, `*.pem`, `*credential*`, `~/.ssh`, Ryter credential files.
- Shell commands are judged per segment (`a && b` is two commands). Privilege escalation, disk writes, `git push`, and piping into a shell are denied; destroying files outside the worktree is Ask. In the TUI a permission modal shows the tool and its arguments: `y` allow this call, `n` deny, `a` allow for the rest of the session. Headless (no TUI) fail-closes.
- `ask_user` lets the orchestrator ask a question; the TUI shows it as a modal (number keys pick a choice, or type free text).
- `[features] web = true` offers `web_fetch` / `web_search`. Localhost and private IPs are blocked.
- Hooks can still deny after the policy allows.

Logs append to `~/.ryter/logs/ryter.log` (no secrets). Project `.ryter/` overlays apply after `ryter trust`, or after the TUI “trust this project?” prompt.

## Project memory

Ryter keeps **why** on disk, not in the orchestrator transcript:

| File | Role |
| --- | --- |
| `ROADMAP.md` | Now / Next / Later / Done / Blocked. Created on first run if missing. |
| `DECISIONS.md` | Decision records (chosen vs rejected, why, where). |
| `notes/*.md` | Phase pass notes (`plan`, `architect`, `build`, `audit`). |

The orchestrator and specialists **read** these every turn (capped). They **update** them as work changes. They must not paste chat logs. When you ask why something is a certain way, the orchestrator should quote `DECISIONS.md` and open the files it names.

Orchestrator may write only these memory files, never `src/`.

## Sessions

```
~/.ryter/sessions/<cwd-slug>/<id>/
  meta.json
  events.jsonl
  transcript.jsonl
  spend.jsonl
  notes/           # pass notes
  tasks.json
```

No SQLite. `ryter spend` uses the latest session for this directory.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | ok |
| 1 | error (including not a tty without `-p`) |
| 3 | spend budget exceeded |
