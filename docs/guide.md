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

### Updates

A release installed with `install.sh` keeps itself up to date:

- **At launch,** at most once a day, Ryter asks GitHub for the latest published release. Drafts are never offered. The check runs off the screen's thread, so startup doesn't wait for it.
- **If a newer release is out,** Ryter installs it in the background and says so in the chat: "Ryter 0.9.1 is installed. Restart Ryter to use it." The Ryter you're running keeps working until you quit it.
- **`ryter update`** checks and installs now, and `ryter update --check` only says whether a newer release is out.
- **`[update] mode`** (or `/settings` → *updates* → *on launch*) sets what happens at launch:
  - `install`, the default;
  - `notify`, which says a newer release is out and leaves installing to `ryter update`;
  - `off`, which doesn't check.

  Headless runs (`ryter -p`, `ryter mcp serve`) never check.

Before it installs anything, Ryter checks:

1. **The signature.** `SHA256SUMS.sig` must be an ed25519 signature over the release's `SHA256SUMS`, made with Ryter's release key. Ryter is built with the public half of that key (`release/ryter-release.pub.pem`), so a tampered release, or one signed with any other key, is refused. Releases before 0.9.0 aren't signed and are never installed this way.
2. **The checksum.** The download must match its line in `SHA256SUMS`.
3. **The binary itself.** It must run and report the version the release says, within 10 seconds. A binary that hangs is stopped, along with anything it started, and nothing is installed.

Only then does it replace the installed binary, the way `install.sh` does. Ryter writes the new one next to the old one and renames it into place.

Ryter won't replace itself in these cases, and says what to do instead:

- **A build from source** (`cargo build`, `cargo install`): only binaries the release workflow built update themselves. At launch, and from `ryter update --check`, a build from source is told when a newer release is out and how to update it.
- **A folder you can't write to:** for example, a binary installed under `/usr/local/bin` by another account. Ryter never uses `sudo`. Rerun the install script as the account that installed it.

`RYTER_UPDATE_URL` points the check at a mirror (`<url>/latest`, files under `<url>/download/<tag>/`). A mirror's releases must still carry a valid signature from the release key.

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

**Reasoning.** Each model has a reasoning level you choose: **Tab** on a model in `/models`, for any seat, or on a seat in the crew builder steps it through `auto → low → medium → high → model's own`. The model card shows the level in use (`reasoning  auto · medium`). The choice follows the model into every role that uses it, and is saved to `~/.ryter/reasoning.toml`; `[model_reasoning]` in `config.toml` does the same by hand, keyed by model id.

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

`ryter` on a tty opens the TUI in the **ledger** layout (`[ui] layout = "ledger"`, the default since 0.6.0).

**In solo mode, a rail runs down the left** (screens 110 columns and wider). It carries:
- the Ryter name, the project and branch;
- the session's title, when it began, and how many turns it has had;
- **the hat**, as a block in its color: `BUILD` green, `PLAN` cyan, `REVIEW` yellow, with what it does (`edits files, runs commands`) and `tab` to switch;
- the model, its reasoning level, and the context gauge with tokens used of the window;
- **spend**: this turn, the session, the project, the budget, and a bar per turn;
- the files the last turn changed, with `+`/`−` counts.

The prompt sits in a box of the hat's color, with the keys that matter now on its lower edge. The rail replaces the view strip and the bottom bar, which come back when the rail is hidden (`^b`, or `[ui] panel = false`) or the screen is narrower than 110 columns. The rest is the same either way:

- **A view strip** across the top names the views and lights the one on screen: `chat`, `changes ^t` (the workbench), and `crew board` (crew mode; `/crew` from solo mode).
- **One reading column on a timeline**, centered, at most 112 columns wide. Each question starts with its time and `●`. The model's text hangs off `◆`, each tool step off `├─` (`edit   src/config.rs ······· +9 −1`, `run    npm test ····· ✓ 4 passed`), and edits show as full-row green and red diffs.
- **Every turn closes with what it came to**, measured by Ryter: `└─ ✓ 5 tools · 2 files (2 changed, +3 −0) · 1 command (1 ok) · 7.9s · $0.001`.
- **Finished turns fold to one line:** what was asked, led by dots to that summary, ending in `▸`. The newest turn stays open. `^o` opens every turn and every edit whole, and folds them again.
- **The composer** is a rule and a `›` prompt beneath the column. The rule's left end takes the mode's color.
- **The bottom bar** holds what the header and cards used to show: the mode (`BUILD`, `CREW · LEAD`), the project and branch, the model, the context gauge, and the cost this turn, this session, and for the project, against the budget. The keys that matter now are on its right. When space runs short, it drops keys first, then the project, the model, and the gauge. The mode and the costs stay.
- **`$`** on an empty composer opens the **spend drawer** above it. It shows this turn, the session, and the project side by side, with spend by role for the session and the project, and the budget, task cap, and warning level. `b` sets a budget, `⏎` opens the full `/spend` table.

**Mission control** (crew mode on the ledger) is the crew's screen. Five tiles run across the top. The plan is on the left, at full height, with a legend. On the right are the lanes, and the LEAD box with the lead's conversation and the prompt. The strip names who fills each seat and where the patch lands. `tab` picks a lane and `⏎` opens its transcript. Before there is a plan, the board says so:
- **Tiles:** spend against the budget (or, with no budget, what the last minute cost), tasks landed out of all of them, the checks run on each task, the crew's time, model calls, and retries, and the **pulse**: the crew's tokens a second now, a bar per second, and how long since any worker was last heard from (yellow after 30 seconds of silence).
- **The plan:** drawn as boxes, each in its state's color, left to right by what waits on what, and joined by `──┬─▶` / `└─▶`. A blocked task's full reason is listed under the drawing. A plan too big to draw in the space shows as a tree instead. It's drawn from the agent's queue snapshots (`after`, and the manifest-first rule), so it shows exactly what the scheduler enforces. Each task is marked `✓` landed, `◐` building, `◑` in audit, `✕` blocked (with the reason, wrapped rather than cut), `○` waiting on something, or `◇` proposed.
- **Lanes:** a live card per worker, updated a few times a second while it works. The first row says who is acting (builder or auditor), on which task, with which model, and what it is doing right now, for how long: `WAITING` for the model's first byte (after 20 seconds, "the provider is slow"), `THINKING`, `WRITING` a file edit or its reply, or `RUNNING` a tool or command. The second row is the meter: lines of the file written so far, tokens (estimated while streaming), tokens a second, the task's cost, and its tool calls. Under it are the last three lines of what the worker is producing: its reasoning, the file it is writing, or the command's output as it runs. On a short screen the cards drop their output lines, then become one row a worker.

Everything a specialist reports is also kept in the session's `activity.jsonl`.

**The workbench** (`^T`, or `/changes` on the ledger) shows what changed beside the chat, in three panes:
- **Left:** the files changed (this turn, or since the last commit with `tab`), the turn's commands and what came of them, and the turns.
- **Middle:** the chat.
- **Right:** the selected file's changes, one at a time, with line numbers on both sides.

Its keys: `↑↓` pick a file, `j`/`k` move between its changes, `x` undoes the selected change alone, `X` undoes the whole file (after a `y`), and `u` undoes the turn. `x` and `X` are recorded like a turn, so `/undo` brings them back. While the workbench is open, keys go to it and not the composer; `esc` or `^T` returns to the chat.

`[ui] layout = "classic"` (also in `/settings`, applied at once) brings back the 0.5 screen. That's a header row, the chat with a right-hand **info panel** of cards (session, model + context gauge, spend + budget gauge, tasks, crew, mcp), the **activity strip** while a turn runs, the bordered **composer**, and a hint bar. `^b` shows or hides the info panel there (it also drops automatically under 80 columns), and the rail on the ledger.

Every message is a left-aligned block under a speaker header — your name (from `[ui] username`, then `git user.name`, then `$USER`), the model name, `· system`, or a one-line tool row (`· read_file  path  0.1s`). Markdown renders with headings, lists, quotes, tables, and fenced code with syntax highlighting and a line-number gutter. Long model turns end with a summary line (`3 tools · 12.4k tok · 0:42 · $0.01`).

**Edits show what changed.** A `write` or `search_replace` row shows the change itself: numbered lines with two lines of context, added lines on a green row and removed ones on a red row, highlighted like the file. The numbers are the file's own, measured by the tool against the file on disk. A long edit shows its first 12 rows and says how many more there are; `^o` shows every edit whole, and `/changes` has the full diff of the turn or of everything uncommitted. The turn's closing line counts the real lines added and removed. In 16 colors and `NO_COLOR` the text carries the color, or the `+`/`-` sign alone.

Type a message and press `Enter`. `Shift+Enter` (or `Alt+Enter`) inserts a newline; paste is bracketed so multi-line text lands in one message. While a turn runs, `Enter` queues the next message. `↑`/`↓` on an empty composer walk prompt history.

`/` (or `^p`) opens the **command palette**: fuzzy-matched, grouped by category, with a description and keybinding column. `Enter` runs the command, `→` opens its panel, `Tab` completes. Every configuration command opens a **panel** — a bordered popout with a title, status, and legend line — and panels stack: `/models` → `b` → the crew builder opens on top; `Esc` closes one level.

The **activity strip** shows what the model is doing (`⠙ writing · edit docs/guide.md · 0:34 · 1.2k tok`). Reasoning streams there as a one-line ticker; `^r` expands it to a scrollable pane (`Alt+PgUp`/`Alt+PgDn`). Reasoning is display-only — it is never saved or sent back to a model. `[ui] reasoning = "off"` hides it.

**Scrolling.** The transcript follows the bottom until you scroll up (`PgUp`, `Shift+↑`, or the mouse wheel over the chat); then it holds still and the scrollbar turns amber. `Ctrl+End` reattaches. `Ctrl+↑`/`Ctrl+↓` jump between turns. While detached, a sticky header at the top of the chat keeps the in-flight user message in view.

`/sessions` (alias `/resume`) is one browser for this directory’s sessions: `Enter` resumes, `r` renames, `d` deletes (type the short id to confirm), `n` starts a new one. `/agents` lists running specialists; `Enter` or `k` kills one, `K` kills all. `ryter sessions` and `ryter resume [id]` are the CLI equivalents.

**Approving.** When the build hat needs your yes, a card opens just above the message box, with the chat still readable behind it. It has these rows:
- **what** the call does (`edit stats.js`, `run cargo test`);
- **why**: the model's own words just before it asked;
- **risk** in plain words, including whether `/undo` can put it back;
- **the change**: the diff for an edit, or the command.

The keys are on the card's last row:
- **`⏎`** allows this call. An Enter pressed in the first half-second after the card opens is ignored, since it may have been meant to send a message.
- **`a`** allows the kind of action the card names for the rest of the session: "edits to files in the project", or "`cargo test` commands". Anything else still asks.
- **`n`** or **`Esc`** denies.
- **Commands that delete, move, or discard files** (`rm`, `mv`, `git reset --hard`, `git clean`, deleting a branch) and **writes outside the project** take only **`y`**. Enter says so instead of approving, and there's no `a` for them.

`ask_user` questions and the first-run “trust this project?” prompt are modals with a heavy top border. `^c` on a prompt while a turn runs stops the turn.

**Pickers.** In `/models` and `/help`, typing filters the list from a search row at the top of the panel. The model picker opens on the model you're using. On OpenRouter it lists only the models your account can use, with tool support: models your privacy settings (such as zero data retention), provider rules, or guardrails leave with no provider are left out. Change those at [openrouter.ai/settings/privacy](https://openrouter.ai/settings/privacy), and the list follows the next time it refreshes. Model lists also leave out what can't hold a conversation in a terminal: batch routes (`:batch`, which answer hours later without streaming) and models that make images or audio. A model id you type into `config.toml` or `--model` is used as given.

**When a turn stops by itself**, the chat says why:
- the model made the same call and got the same result five times (it is told at the third);
- it used the 40 rounds one message may have (say “continue”);
- the provider sent nothing but keep-alives for 5 minutes (15 for a local server), or reported an error mid-reply;
- a reply was cut off at the output limit three times in a row.

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
| `^b` | show or hide the rail (ledger) or the info panel (classic) |
| `^o` | show every edit and finished turn whole, or folded |
| `$` | the spend drawer (ledger, when the composer is empty) |
| `^t` | the workbench (ledger); `/changes` on the classic screen |
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

A folder of projects is different. That's a folder like `~/workspace` that isn't a repository with commits itself, but holds other projects' repositories, in its folders or one level further down. There, the crew stops at your first message, before any model call and without writing anything. It names the repositories it found and asks you to start Ryter in the project's own folder. If they're the project's own dependencies (a git package under `node_modules`, say), add their folder to `.gitignore`. Only folders git will ignore, by the folder's own rules, are left out of the search. Before the first commit, git itself checks that it holds no other repository at any depth. If it would, Ryter undoes the `git init` and `.gitignore` it just made. If you do want one repository there, run `git init` and make the first commit yourself.

## Solo mode and hats

Ryter starts in solo mode: one model in your project. `Tab` switches its hat (build → plan → review), `Shift+Tab` goes back, and `/build`, `/plan`, `/review` jump to one. The header, the message box's badge, and its border all show the hat in its own color. A switch applies to your next message. The model can also offer a switch itself: after a plan ("carry out the plan?") or a review ("fix these?") it asks with a yes/no prompt, and on `y` it carries on in the new hat in the same turn.

| Hat | May | May not |
| --- | --- | --- |
| **build** (default) | edit files and run commands; edits and commands that change things ask (or run with `a` for that kind of action / `--always-approve`); destructive commands always ask; outside the project, writes ask **every time** (see below) | read secrets, push, run inline interpreter code |
| **plan** | read, search, run read-only commands, write `notes/` and project memory | edit source, run anything that changes the project |
| **review** | read, run the tests and linters, read-only git | write anything, not even by redirect; install, format, or fix |

**Audits: a second opinion.** `/audit` asks a different model to review your uncommitted changes before you commit (`/second` also works). You choose the model and how much one audit may spend; Ryter never chooses either.

- **Offered after your changes.** When a build turn finishes having changed files, Ryter offers an audit with what it would cost: `y` runs it, with no second question; `n` passes; `s` stops the offers. `/settings` (audit offers) or `[ui] offer_audit` turns them back on. Nothing is offered after a turn that only talked or read, or one that was cancelled or cut short.
- **The first time,** `/second` opens a chooser. It lists every model from every connection you have a key for, from the live catalog, each with what *this* review should cost in dollars. The model doing the work isn't offered, and neither are models that can't use tools. A model from the same vendor is marked as less independent, and one with no known price can't be chosen, since no limit could hold it. Then you type your limit per review, in dollars; there's no preset. The choice is saved in `~/.ryter/review.toml`, and `/audit model` changes it.
- **Every review asks first,** with the model, what it will read, and a cost range against your limit. After a few reviews it also shows what your last ones with that model cost. `n` spends nothing.
- **It keeps to your limit before spending, not after.** Each step is priced before it's sent. A step that would pass the limit isn't sent. When about one step's room is left, or three quarters of the limit is spent, the reviewer is told to stop exploring and write up what it has. Either way you get what it found, marked if it was cut short.
- **In the chat,** an audit carries a rule in the auditor's color on every line, and a long one folds after 14 lines; `^o` shows it whole.
- **It can't change anything.** It reads the diff and the conversation, and may read files and run the tests, under the review hat's rules. It creates no files. Findings come marked **blocking** or **note**, with a verdict and the cost, and the model you work with gets them with your next message, so "fix those" works.
- **A saved reviewer that disappears** (retired, or no longer priced) brings the chooser back. Ryter never falls back to another model on its own.

**What the chat shows.** The model narrates as it works: what it's doing next and why, each choice between approaches with its reason, and what it thinks went wrong when something fails. Each tool step shows what came of it, measured by Ryter: `new · 48 lines`, `rewrote · 76 lines (was 89)`, an edit's changed lines, `✓ 13 passed`, or `✗ exit 1` with the cause. Reads fold into one line, and a divider closes each turn that did work (`6 files (3 new, 3 changed, +153 −15) · 9 commands (9 ok) · 2:41`).

**Outside the project.** The build hat can write elsewhere on your machine, such as `/tmp` or another folder, but only by asking each time. The prompt says "outside the project" and offers only `y` (allow once) or `n`. "Allow all" and `--always-approve` cover the project, not the rest of the machine, so headless refuses these writes.

Some places are refused however they're asked for:
- **Never read or written:** credentials (`~/.ryter`, `~/.ssh`, `~/.gnupg`, `~/.aws`, `~/.config/gh`, and the like) and secret files.
- **Never written:** shell startup files (`~/.bashrc`, `~/.zshrc`, `~/.profile`, …) and system folders (`/etc`, `/usr`, …).

Reading outside the project is an ordinary question. Plan and review never write outside, and crew builders stay in their worktrees.

**What review may run** is judged by the command's form, not the tool's name. `cargo test`, `cargo clippy`, `cargo fmt --check`, `npm test`, `npm run lint`, `npx vitest run`, `npx tsc --noEmit`, `npx prettier --check`, `pytest`, `ruff check`, `black --check`, `go test`, `go vet`, `make test`, and the like run. `cargo fmt`, `npm install`, `npm run format`, `npx <any package>`, `ruff --fix`, `make install`, and `python -m pip install` don't. Review may still run the project's own code, which is what tests do.

**Commands the gate refuses in every hat:**
- the never-run list (`sudo`, `ssh`, `dd`, `mkfs`, `systemctl`, `crontab`, …), however it's wrapped: `env -i sudo`, `timeout 5 sudo`, `nice dd`, `xargs ssh`, `find -exec sudo`, `busybox rm`, `s\udo`;
- a command whose name comes from a variable or `$(…)`, and `eval`;
- inline code for an interpreter (`python -c`, `node -e`, `deno eval`, heredocs): write it to a file and run the file;
- `git -c` settings that name a program (`alias.x=!cmd`, `core.sshCommand`, `core.pager`, …) and `git --exec-path`.

`env` alone, which prints every variable, is not read-only. Commands run without Ryter's own API keys in their environment.

Every hat shares one system prompt (`prompts/solo.md`) and one tool list. The hat is a one-line note in front of each message, and the permission gate enforces it, so switching never throws away the provider's prompt cache.

Before each build turn changes anything, Ryter snapshots your files as a git object under `refs/ryter/undo/`, and again when the turn ends. Your branch, staging area, and files aren't touched. `/undo` puts back what the last build turn changed, and only that: the files it edited go back as they were, files it created are deleted, and anything you changed yourself since, in your editor or anywhere else, stays as you left it.

- **If you've since edited a file that turn changed,** `/undo` doesn't touch anything and names the file, since undoing would lose your edits. `/undo force` goes ahead anyway.
- **`/redo` reverses an undo,** forced or not, as long as no build turn has run since. If you've edited those files after the undo, it asks for `/redo force` the same way.
- **Gitignored files the model writes** (a local config, say) are saved before it writes them, so `/undo` puts them back too. Snapshots skip ignored files, and a command run through the shell isn't covered.

A folder that isn't a repository gets git set up first, and Ryter says so. It won't make one in your home folder, at the root, or in a folder that holds other projects' repositories. There, solo mode edits without snapshots, and `/undo` is unavailable.

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

`/crew` switches to crew mode, and that's all it does. The first time, the crew builder opens (below); once a crew is saved, `/crew` switches straight to it. `/solo` goes back. In crew mode the right-hand panel adds the tasks and crew cards.

**Every model is chosen in `/models`.** The seats are on the left: the lead (`Solo` in solo mode), architect, builder and auditor, each with the model it runs on now. The models for the chosen seat are on the right.
- **Choosing a seat:** `↑↓` picks one. `→` or `⏎` moves to its models, and typing starts a filter there straight away.
- **Setting a model:** `⏎` sets the highlighted model for the seat and takes you back to the seats, on the next one. You can set the whole crew in one visit: pick, `⏎`, pick, `⏎`. A `✓` marks each seat you've set.
- **Other keys:** `←` goes back to the seats without setting anything. `Tab` steps the highlighted model's reasoning, `s` sorts, `b` opens the guided crew builder, and `Esc` closes.

It works the same in solo and crew mode, so you can set up the crew before you switch to it. **`/crews`** holds the ready-made crews and your saved ones, to preview, apply, save or delete.

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

Project markdown is loaded from the working tree without a trust gate: `RYTER.md`, or `AGENTS.md` if `RYTER.md` is absent. Your own rules for every project come before it: see [Your rules](#your-rules).

## Build workers, auditor, merge

`todo_write` **is** the work queue. After a lead turn with no remaining tool calls, Ryter drains pending tasks, up to `[subagents] max` in parallel (must be ≥ 1). Each task names its role: architect tasks run first, then builders, each gated by checks and an auditor. Tasks declare the `files` they own; disjoint tasks run in parallel, overlapping or undeclared ones one at a time.

**Order.** A task can list the tasks it builds on in `after` (`"after": ["scaffold"]`). It starts only once they have landed on the patch, so it branches from their real code. In a project with no build manifest yet (`Cargo.toml`, `package.json`, `pyproject.toml`, `go.mod`, …), the task that creates one runs first and alone, and every other builder task waits for it, whether or not the plan says so. A task whose prerequisite is blocked never starts: the crew report lists what each one waits on, and the lead fixes that task instead of retrying the ones waiting.

Crew roles default to the lead’s current provider and model. Assign a different model per role in `/models` (the role's seat; the first row, `default`, follows the lead). That is also how you split providers. Optional `[specialists.*]` tables in `~/.ryter/config.toml` pin the same overrides.

Each builder task:

1. `git worktree add` under `~/.ryter/worktrees/<session>/<task>/` on branch `ryter-<8hex>-<slug>`
2. The builder implements its brief and ends with a handback (`STATUS / FILES / DECISIONS / NOTES`). `STATUS: BLOCKED` means something outside the task stopped it (a missing system package, a module another task owns): the work is kept on its branch, nothing is checked or audited, no retry is spent, and the lead tells you what to do (`sudo dnf install libpq-devel`). A builder has 40 steps, an architect 30 and an auditor 12. The last one is for writing up: the specialist is told so, gets no tools for it, and its report says it reached its limit
3. The runtime commits, then merges **your branch into the worktree**. Conflicts are resolved there by a builder — never in your checkout — and the resolution is re-audited
4. **Checks**: `[auditor] checks` run in the worktree (set them per project in `.ryter/config.toml`). A failure rejects the work before any audit. With none set, Ryter reads the project's files once there is a manifest (`Cargo.toml` → `cargo test`, `package.json` → its `build` and `test` scripts, `go.mod` → `go build`/`go test`, pytest or unittest for Python) and asks once whether to use them, for the session or saved to the project
5. **Auditor**: reviews the brief, handback, check output, and full diff, and ends with `VERDICT: PASS`, `VERDICT: FAIL`, or `VERDICT: UNVERIFIED`. `UNVERIFIED` is for code that reads right but that the auditor couldn't build or test, and only when no checks ran. Either something outside the task hasn't landed, or running it needs a command the auditor's shell refuses. It isn't a rejection. The task lands on the patch marked, and the patch doesn't reach your branch until checks have built and tested it
   - **What the auditor can run:** test runners, linters and read-only commands. Its shell refuses containers (`docker`, `podman`), servers, installs and the project's own shell scripts. That limit is the auditor's alone: a builder can run them. For a project that is tested inside a container, put those commands in `[auditor] checks`, and Ryter runs them itself before the audit
   - **A review with no verdict decides nothing.** The auditor is asked once for its verdict, with no tools. If it still gives none, the task stops at the gate with its work kept on its branch: it isn't rejected, no retry is spent, and the builder isn't run again. Setting the task to pending audits it again without rebuilding it
6. **Land**: one `--no-ff` merge commit (undo with `git revert -m 1`). If your branch moved meanwhile, it re-integrates and re-checks first. If you have uncommitted edits to the same files, it stops and keeps the branch
7. Rejected → retry with the findings, up to `[auditor] max_retries`, then `blocked`

Tasks land on a patch branch (`ryter/patch-…`), not yours. When every task in the patch is done and the combined checks pass, the patch lands on your branch as **one commit** (`git revert -m 1` undoes it all). A blocked task holds the patch until you retry or drop it; the lead says what it is waiting on.

Auditors must be different models from the lead and the builder — otherwise builds refuse to start and say how to fix it. Before the crew starts, Ryter checks that your OpenRouter account can use every seat it is about to call (a free lookup). If one is ruled out, say an auditor under zero data retention, nothing runs, the tasks stay queued, and you're told which seat to change. If a model still fails after the builder has finished (the auditor's provider is down, say), the task stops with its work kept on its branch. Assign one in `/models` (the Auditor seat), or list a panel under `[[auditor.panel]]` (all must pass; cheapest first; seats may have a `focus` and `paths`).

For a trivial change the lead can `propose_edit`: you see the diff and press `y`. Only a person can approve it.

**The crew builder** is where a crew is set up. It opens the first time you type `/crew`, and from `/models` or `/crews` with `b`. When you save, you're in crew mode. It walks through seven steps:

1. A starting point: skiff, schooner, galleon, or your current crew.
2. The lead.
3. The architect.
4. The builder.
5. The auditor.
6. Budget: pick the job size (small, medium, or large) and see an estimate per task, per design, and for the whole job, with a suggested cap.
7. Review.

Each seat step says what the role does, puts a ★ recommendation first, and lists every model you can reach (type to filter). The auditor step refuses the lead's and builder's models. Review sends each model one tiny request with a tool (well under a cent) and saves only when all of them answer. That catches what no catalog shows: an OpenRouter data policy such as zero data retention that leaves a model no provider, missing tool support, a model you have no access to, or no credits. `ryter crew check` runs the same test on your saved crew. Esc on first launch means "later"; the builder doesn't come back on its own.

The estimate uses token counts measured on paid runs (`crates/ryter-core/src/estimate.rs`). It is rough, and a job is usually larger than it looks.

Three ready-made crews sit in `/crews`, picked from every model you can reach at today's prices, so they never name a model you can't use or one that has gone stale:

| Crew | Cost | Builder | Architect and auditor |
| --- | --- | --- | --- |
| **skiff** | low | a budget model | the best of the budget models; the auditor is still a different model from another vendor |
| **schooner** | balanced | a budget model | strong models (the default suggestion, `s` in `/crews`) |
| **galleon** | high | a strong model | strong models, the auditor from another vendor |

Enter on one previews it; `y` applies it and keeps your previous crew as the `before-suggest` preset. `ryter crew tiers` shows all three; `ryter crew suggest --tier galleon --apply` applies one from the shell. Strong seats prefer established vendors when one is close in price: price is the only signal before `ryter bench`, and on a live catalog it put an obscure model ahead of Claude Opus. None of the crews changes the lead. A local model server works as a connection with no key: `ryter connections add box --kind ollama --model qwen3-coder:30b`. `ryter bench` runs `bench/` through the crew and reports what landed, what passed hidden tests, and the cost per accepted task — it spends real money.

Crew spend is metered per task and role and counts against `[spend] session_budget_usd`; each task also stops at `task_budget_usd` / `task_max_tokens`. See `docs/cost.md`.

**When a task reaches its cap**, it pauses and Ryter asks (`Ryter asks · task budget`). You see what the task has spent and on which roles, with three choices: add $5, add $2, or stop. Raising the cap carries the task on from exactly where it was: the step it paid for is kept, and the other workers go on while you decide. A raised cap stays with the task. If you stop, the task's branch keeps the work. The lead is told to raise the cap and requeue the task rather than recreate it, because recreating it starts the build over. A task that is still at its cap asks before it spends anything. Runs with no one to ask (headless) stop as before.

**A task that stops after its build is committed** (you stopped it at the cap during the audit, the auditor's provider refused, the merge failed) goes straight back to the checks and the audit on its next run. The builder isn't paid again.

**Rejections are counted per task, in all**, across the lead's requeues and recreations. The lane card shows them in yellow, and a blocked task says `rejected; blocked (5 rejections in all)`. At 3 rejections, and every 3 after, Ryter says more retries of the same builder rarely help and that you should choose a stronger one in `/models → builder`. Which model is your choice: Ryter names none and switches nothing. The lead is told to pass it on without picking one for you, and the cap question mentions it too.

`/auditor on|off` is session-only unless you also change config. With the auditor off, **nothing merges**: finished work waits on its branch. After each batch the lead gets the crew report, tells you what landed, and records builder decisions in `DECISIONS.md` — builders never write project memory themselves.

The architect runs in-process (no worktree) and writes tasks straight into the queue builders read from. Nested subagents are not supported.

## Spend

Every model call is priced before the next request. Roll-ups: session, turn, role, connection. Persisted in `spend.jsonl`.

Sources, high wins: TOML `[pricing."<model>"]` → OpenRouter catalog (when ingested) → shipped SpaceXAI table. Provider-reported cost on a stream wins for that turn.

Unknown rates show `$?.??` plus token counts. Ryter never invents `$0.00` for an unpriced model.

A budget can't stop what it can't price. With a session budget set, a call that comes back with no price is kept, and Ryter says so. It then won't call that model again until it has a price or the budget is off: in the TUI the next message says so instead of sending, and headless exits `3`. Give it one in `config.toml`:

```toml
[pricing."claude-sonnet-5"]
input_per_million = 3.0
cached_per_million = 0.3        # cache reads
cache_write_per_million = 3.75  # cache writes (Anthropic: 1.25x input)
output_per_million = 15.0
```

Prompt caching is priced in parts: cache reads at the cached rate, cache writes at the cache-write rate, the rest at the input rate. Each rate falls back to the input rate when unset.

A session budget is optional. With one, the crew stops when spend reaches it, says what finished and what didn't, and waits (exit `3` in headless). Without one, nothing stops on cost and you watch the spend card.

| Command | Effect |
| --- | --- |
| `/budget` | the budget panel: spend against the cap, on/off, the cap, the warning level, and the per-task cap (`^s` saves) |
| `/budget 5` | cap this session at $5 |
| `/budget +2` | raise the cap by $2, e.g. after hitting it |
| `/budget off` | no cap |

The **budget** card on the right shows the cap, how much is used and left, or `off`; click it to open the panel. Changes apply at once and are saved as your default (`~/.ryter/settings.toml`, the same value as *budget usd* in `/settings`). A trusted project's `[spend] session_budget_usd` overrides your default in that project. There is no session budget until you set one; the crew builder suggests one sized to the job. Each task is still capped at `[spend] task_budget_usd` ($3 by default; the crew builder raises it when your crew's normal design would not fit), which catches one runaway task whether or not there is a session budget. `[spend] enabled = false` still counts in memory and prints a warning.

**Project cost.** A project is its git repository (the folder, outside one), so sessions started in any subfolder count toward it. The spend card shows `project` under the session total. When the project is a repository around the folder you started in, the rail says which folder, for example `in ~/workspace`, and the `$` drawer names its project column for it. `p` in `/spend` switches to the project view: the total across sessions, this month, solo vs. crew, and breakdowns by role, model, and month. `ryter spend --project` prints the same. Nothing extra is recorded: every call is already in its session's `spend.jsonl`, and a running total in `~/.ryter/projects/` means only new lines are read. Calls with no known price are counted and shown (`$14.20+`), never added as $0.

`/spend` is a panel: session total, a budget gauge, and tables by role and by connection; `p` switches to the project; `e` exports CSV. The info panel’s spend card shows the total, and the budget card below it shows the cap. `ryter spend` prints the roll-up on the CLI.

## Slash commands

Type `/` to open the palette; every built-in has a one-line description there. Configuration commands open panels: `/settings` `/provider` `/models` `/crews` `/mcp` `/skills` `/hooks` `/sessions` `/agents` `/spend` `/theme` `/tools` `/auditor` `/context` `/doctor` `/help`. `/changes` and `/commit` open panels too (see [Review and commit](#review-and-commit)). Direct commands act immediately: `/undo [force]` `/redo [force]` `/new` `/rename <title>` `/budget [amount|+amount|off]` `/compact` `/cancel` `/quit`. Near-duplicates are hidden aliases (`/resume` → `/sessions`, `/model` → `/models`, `/connections` → `/provider`); `/delete [id]` stays as a hidden direct command.

User-invocable skills and `~/.ryter/commands/*.md` join the palette under **skills**. Built-ins win on a name clash.

## MCP

**Outbound.** `[mcp_servers.<name>]` stdio children. The orchestrator discovers with `search_tool` and calls with `use_tool`. Child env does not inherit API keys unless that server’s `env` table asks. In the TUI, `/mcp` is a panel: `Enter` toggles a server, `r` reconnects, `d` removes it (type the name to confirm), and `Enter` on the trailing `+ add server` row walks name → command → args → review. Each server row shows its live status (`connected · 5 tools`, `error: …`, `disabled`).

`search_tool` matches every word of its query against a tool's name and description, and lists each tool's arguments. A server has 60 seconds to start and list its tools. A tool call may run for 120 seconds, or for the server's `timeout_secs`:

```toml
[servers.browser]            # ~/.ryter/mcp.toml ([mcp_servers.browser] in config.toml)
command = "npx"
args = ["-y", "@playwright/mcp"]
timeout_secs = 300
```

Esc stops a call at once. Ryter then tells the server it stopped waiting, and skips the late reply if one comes. If a server exits, the tool error says so, with the last thing it wrote to stderr; `/mcp` → `r` reconnects it.

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
| Rules | `~/.ryter/RYTER.md` for every project; `RYTER.md` (or `AGENTS.md`) at a project's top for that project. See [Your rules](#your-rules). |
| Skills | `/skills` panel. Files: `~/.ryter/skills/<name>/SKILL.md` (frontmatter `user-invocable`, `model-invocable`). `Enter` runs (optional args), `e` opens the file in `$EDITOR`, `a` writes a stub, `d` deletes a user skill (not a project overlay or a built-in one). |
| User slash | Same `/skills` list (`command` rows). `~/.ryter/commands/<name>.md` (`$ARGUMENTS`) |
| Hooks | `/hooks` panel. `a` adds: event → command or URL → optional matcher. `d` removes. Live list is `~/.ryter/hooks.toml` (does not rewrite `config.toml`). Command gets JSON on stdin; exit 2 or HTTP 403 denies. |
| Themes | `/theme` panel previews as you move: `dark`, `light`, `default-16`, or `~/.ryter/themes/<name>.toml`. `Enter` persists to `~/.ryter/settings.toml`. `NO_COLOR` or a 16-color `TERM` degrades automatically. |
| UI | `[ui]` in `~/.ryter/config.toml`: `username`, `theme`, `reasoning`, `mouse`, `panel`, `colors`, `timestamps`, `line_numbers`, `receipts`, `offer_audit`, `open_pages`. All optional; unknown keys warn once at startup. See `config.example.toml`. |

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

**Skills the model uses on its own.** The model's instructions list every skill it may use, by name and one-line description, and it loads one with `load_skill` when a task calls for it. You can still start any skill yourself with its slash command. `model-invocable: false` keeps a skill to the palette. Write the `description` as when to use the skill, because that line is how the model decides. A skill in `~/.ryter/skills`, or in a trusted project's `.ryter/skills`, replaces a built-in skill of the same name.

**A skill's own files.** A skill kept as a folder (`<name>/SKILL.md`) can hold more files, such as a template, a checklist or a script. When the model loads the skill it sees their names, and it reads one with `load_skill` and `file`. It can read only files inside that skill's folder: no `..`, no absolute paths, and no links that lead out. Each file can be up to 256 KB.

**Built-in skills:** Ryter ships with two, `canvas` and `rules`.

### Your rules

Ryter keeps your standing rules in two plain Markdown files, and puts both into the instructions of every role (solo, the lead, the architect, builders and auditors) on every message:

- **`~/.ryter/RYTER.md`:** your rules for every project. How you like work reported, what to ask before doing, spelling, tone.
- **`RYTER.md` at the top of a project** (or `AGENTS.md` when there's no `RYTER.md`): rules for that project.

Your rules come first, and where the two differ the project's win. You can edit either file by hand at any time. Ryter loads up to 32 KB of the every-project file.

**Saving a rule from the chat.** Say how you want something done from now on, such as "from now on, answer in British spelling", or type `/rules <what to remember>`. The model loads the built-in `rules` skill and changes the every-project file with the `update_rules` tool. Before anything is saved, Ryter shows you the change line by line and asks:

- **The prompt holds the whole change.** Every added and removed line is there, and long lines are wrapped, not cut. This file isn't in your project, so `/changes` never shows it; the prompt is the only place to read the change.
- **A change longer than the prompt scrolls.** `↓` and `PgDn` move through it, `↑` and `PgUp` go back, and a line under the change says how many rows are left.
- **Only `y` saves it, and only once every row of the change has been on screen.** Before that, `y` tells you there is more to read. Ryter counts the rows it has drawn, not the keys pressed, so holding `PgDn` or pressing it many times at once skips nothing: each screenful starts where the last one ended. If you resize the window part-way through, the reading starts again from the top.
- **In a window too small to show any of the change,** the prompt says so, and `y` does nothing until there is room. There's no "always" for this prompt, and `--always-approve` doesn't skip it, because the file steers every later session.
- **The rules are plain text.** A change holding a character the screen wouldn't show as it is (a control code, an invisible character, or one that reverses the direction of text) is refused before you are asked.
- If you say no, the file is left as it was, and the model is told so.
- In a headless run (`ryter -p`) nobody can answer, so nothing is saved.
- Under `--sandbox` the file can be read but not changed. The model's shell commands run in the same sandbox, and a rules file the sandbox could write would need no asking.
- If you edit the file by hand while the question is on screen, nothing is saved over your edit.

The file may be a link to one you keep elsewhere, such as in a dotfiles folder. A link to anything inside Ryter's own folder is not read as rules, because that folder holds your keys. Under `--sandbox` a link isn't followed, so linked rules are left out there unless the file they point at is inside the project. If `RYTER_HOME` puts Ryter's folder somewhere else, the prompt and the model name the file where it really is.

A rule saved this way takes effect from your next message. For a rule that belongs to one project, the model edits that project's `RYTER.md` with its ordinary file tools, under the usual approvals for an edit.

### Pages (the canvas skill)

When a terminal isn't the right place for an answer (a report, a comparison, a chart, a plan to scan), the model builds a page. It loads the `canvas` skill, writes one self-contained HTML page, and shows it with `show_page`. You can also ask for one directly with `/canvas <what you want to see>`.

- **Where pages live:** `~/.ryter/pages/<session>/<title>.html`, outside your project. Deleting the session deletes its pages. Showing a page with the same title replaces it, so the model revises in place.
- **The chat links every page**, and Ryter opens it in your browser when there's a desktop. `[ui] open_pages = false` (`/settings` → *pages in browser*) keeps pages closed, and you get the link only. Under `--sandbox`, Ryter leaves pages closed too: a browser started from the sandboxed thread would run inside the sandbox.
- **A page loads nothing from the network:** no scripts, fonts or images from the web, and no form posts. Ryter puts a Content-Security-Policy at the very top of every page, ahead of anything the page contains. Everything the page shows is inline.
- **Hooks see these tools too.** `PreToolUse` and `PostToolUse` hooks run for `show_page`, `load_skill` and `request_hat`, just as for `bash` or `write`. A hook can deny a page.
- **The skill tells the model** to use only facts from the session, to say where they came from, and to make the page work in light and dark and at phone width.

## Context

`/context` opens a panel with estimated tokens vs the model window (500k for `grok-4.6`, 200k otherwise), a gauge, and a breakdown by contributor (system prompt, project files, transcript, tool output); `c` compacts. The info panel’s model card shows the same gauge. Auto-compact at 85%: older turns collapse to tools used, files touched, and the latest pass note; the last four user turns stay. `/compact` forces a pass. Resume reads the rewritten `transcript.jsonl`.

## Doctor and sandbox

`ryter doctor` (and the `/doctor` panel, which runs the checks off-thread and can save the report with `c`) checks OS, tty, home, config, both built-in connections (key set/missing, never printed), spend catalog, git, Landlock, sandbox profile, and whether `.ryter/` is trusted. No network.

`--sandbox workspace` Landlock-restricts the tool thread to the project tree (writable) plus `~/.ryter/{tmp,logs,sessions,pages}`, and reads `~/.ryter/skills`. It never grants `~/.ryter/keys`. The sandbox is filesystem-only; it does not restrict network. `--sandbox read-only` makes the project tree read-only. `--sandbox off` is the default. A non-off profile **refuses to start** if the kernel cannot enforce Landlock. `/tmp` itself is not granted; scratch is `~/.ryter/tmp`. Sandboxed runs use a current-thread tokio runtime.

## Safety

- One gate: `decide(role, tool, args)` → Allow / Ask / Deny. Role masks omit tools the model should not see.
- Orchestrator: read, list, grep, glob, `todo_write`, MCP. Cannot write `src/`.
- Architect: read tools, project memory, `todo_write`.
- Builder: full tool set in its worktree; denied project memory files.
- Auditor: read tools + test/lint/read-only-git bash; no write tools.
- Denied even for builders: `.env`, `*.pem`, `*credential*`, `~/.ssh`, Ryter credential files.
- Shell commands are judged per segment (`a && b` is two commands). Privilege escalation, disk writes, `git push`, and piping into a shell are denied; destroying files outside the worktree is Ask. In the TUI a permission modal shows the tool and its arguments: `⏎` or `y` allow this call, `n` deny, `a` allow that kind of action for the rest of the session; destructive commands and writes outside the project take only `y` (see [Approving](#talking-to-ryter)). Headless (no TUI) fail-closes.
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

## Benchmark

`ryter bench` measures the crew on real tasks. Each task in `bench/` is a small repository with something to build or fix. The crew you have set up does the work in a fresh copy: a builder builds, the task's checks and the auditor decide whether it lands, and then tests the crew never saw decide whether it was right. It spends real money on your keys, capped per task.

```sh
ryter bench                         # every task, with your crew
ryter bench --only rust-durations   # one task (repeatable)
ryter bench --budget-usd 2          # the cap per task (default $1)
ryter bench --repeat 3              # each task three times
ryter bench --crew <preset>         # a saved crew, to compare
ryter bench --publish docs/bench    # write the results page and compare with the last
```

It reports four numbers:

- **Landed:** the checks and the auditor passed, so the work reached the branch.
- **Accepted:** the hidden tests passed too.
- **False passes:** landed but wrong, which is how often the auditor's sign-off was mistaken.
- **Cost per accepted task.**

The suite covers Python, Rust and TypeScript, single-file fixes and changes across several files, one piece of work split into three builder tasks, two of them side by side, and a project whose tests run only through its own script, which the auditor can't run. A task says what it needs (`cargo`, `node`, `python3`), and is skipped, by name, on a machine without it.

`--publish docs/bench` writes `docs/bench.md` (the page) and `docs/bench.json` (the same run as data). If a run was published there before, it says how this one compares. Cost is shown but never counted as worse: the same crew can take several times as many steps on one run as on the next. A task that ran last time and not this time is named. The published run is [docs/bench.md](bench.md), and each release is checked against it this way.

The published run is what the next one is measured against, so it is replaced only by a run that can stand in for it. Otherwise it is left exactly as it was, and `ryter bench` exits 1:

- **The run is worse:** a task is accepted less often, or is passed wrong more often. Both are counted per run of the task, so runs with a different `--repeat` still compare. If the new run is the truth, remove `docs/bench.json` and publish again.
- **A published task was skipped** because this machine lacks its tools.
- **Only part of the suite ran:** `--publish` can't be combined with `--only`.

A run that measured nothing is a failure whether you publish or not. `ryter bench` exits 1 when every task was skipped, and it stops at the first task no model answered: a key the provider refuses, a model your account can't reach, or a crew that won't start because the auditor is the same model as the builder. It says which, and spends nothing on the tasks left. `--repeat` must be at least 1 and `--budget-usd` above 0.

To add a task, copy one in `bench/` (see `bench/README.md`). A test proves every task sound: the hidden tests fail on the fixture, and pass on the reference solution.

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
