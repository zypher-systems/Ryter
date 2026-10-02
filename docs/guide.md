# Ryter user guide

Ryter is a Bring-Your-Own-Key terminal coding harness. One model works in your project, with you. It wears one of three hats at a time (**plan**, **build**, **review**), and each hat can run on a model of its own.

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

**Reasoning.** Each model has a reasoning level you choose: **Tab** on a model in `/models`, for any seat, steps it through `auto → low → medium → high → model's own`. The model card shows the level in use (`reasoning  auto · medium`). The choice follows the model into every hat that uses it, and is saved to `~/.ryter/reasoning.toml`; `[model_reasoning]` in `config.toml` does the same by hand, keyed by model id.

**Auto** means Ryter picks by hat: `high` for the plan hat, `medium` for build and review. `[reasoning_effort]` overrides that per hat (`build`, `plan`, `review`). **Model's own** sends nothing. Beware: with no setting, some models think for minutes before acting. The level is sent only to OpenRouter connections.

Where a saved key (`/provider` set-key, `ryter connections set-key`) is stored: on **Linux**, `~/.ryter/keys/<connection>`, readable only by you (mode 0600). Linux's kernel keyring is in memory and doesn't survive a reboot, so it isn't used to store keys. On **macOS**, the keychain (`service=ryter`, `account=connection:<name>`), falling back to the file if the keychain refuses.

If `config.toml` contains `api_key` and is group/world-readable, Ryter refuses to start until the mode is `0600`.

First launch writes `config.example.toml` to `~/.ryter/config.toml` if that file is missing. Precedence, high wins: CLI flags > `RYTER_*` env > trusted project `.ryter/config.toml` > `~/.ryter/config.toml` > sidecars (`hats.toml`, `mcp.toml`, `hooks.toml`, `connections.toml`, `settings.toml`) > built-ins.

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

**A rail runs down the left** (screens 110 columns and wider). It carries:
- the Ryter name, the project and branch;
- the session's title, when it began, and how many turns it has had;
- **the hat**, as a block in its color: `BUILD` green, `PLAN` cyan, `REVIEW` yellow, with what it does (`edits files, runs commands`) and `tab` to switch;
- the model, its reasoning level, and the context gauge with tokens used of the window;
- **spend**: this turn, the session, the project, the budget, and a bar per turn;
- the files the last turn changed, with `+`/`−` counts.

The prompt sits in a box of the hat's color, with the keys that matter now on its lower edge. The rail replaces the view strip and the bottom bar, which come back when the rail is hidden (`^b`, or `[ui] panel = false`) or the screen is narrower than 110 columns. The rest is the same either way:

- **A view strip** across the top names the views and lights the one on screen: `chat` and `changes ^t` (the workbench).
- **One reading column on a timeline**, centered, at most 112 columns wide. Each question starts with its time and `●`. The model's text hangs off `◆`, each tool step off `├─` (`edit   src/config.rs ······· +9 −1`, `run    npm test ····· ✓ 4 passed`), and edits show as full-row green and red diffs.
- **Every turn closes with what it came to**, measured by Ryter: `└─ ✓ 5 tools · 2 files (2 changed, +3 −0) · 1 command (1 ok) · 7.9s · $0.001`.
- **Finished turns fold to one line:** what was asked, led by dots to that summary, ending in `▸`. The newest turn stays open. `^o` opens every turn and every edit whole, and folds them again.
- **The composer** is a rule and a `›` prompt beneath the column. The rule's left end takes the hat's color.
- **The bottom bar** holds what the header and cards used to show: the hat (`BUILD`, `PLAN`, `REVIEW`), the project and branch, the model, the context gauge, and the cost this turn, this session, and for the project, against the budget. The keys that matter now are on its right. When space runs short, it drops keys first, then the project, the model, and the gauge. The hat and the costs stay.
- **`$`** on an empty composer opens the **spend drawer** above it. It shows this turn, the session, and the project side by side, with spend by hat for the session and the project, and the budget and warning level. `b` sets a budget, `⏎` opens the full `/spend` table.

**The workbench** (`^T`, or `/changes` on the ledger) shows what changed beside the chat, in three panes:
- **Left:** the files changed (this turn, or since the last commit with `tab`), the turn's commands and what came of them, and the turns.
- **Middle:** the chat.
- **Right:** the selected file's changes, one at a time, with line numbers on both sides.

Its keys: `↑↓` pick a file, `j`/`k` move between its changes, `x` undoes the selected change alone, `X` undoes the whole file (after a `y`), and `u` undoes the turn. `x` and `X` are recorded like a turn, so `/undo` brings them back. While the workbench is open, keys go to it and not the composer; `esc` or `^T` returns to the chat.

`[ui] layout = "classic"` (also in `/settings`, applied at once) brings back the 0.5 screen. That's a header row, the chat with a right-hand **info panel** of cards (session, model + context gauge, spend + budget gauge, mcp), the **activity strip** while a turn runs, the bordered **composer**, and a hint bar. `^b` shows or hides the info panel there (it also drops automatically under 80 columns), and the rail on the ledger.

Every message is a left-aligned block under a speaker header — your name (from `[ui] username`, then `git user.name`, then `$USER`), the model name, `· system`, or a one-line tool row (`· read_file  path  0.1s`). Markdown renders with headings, lists, quotes, tables, and fenced code with syntax highlighting and a line-number gutter. Long model turns end with a summary line (`3 tools · 12.4k tok · 0:42 · $0.01`).

**Edits show what changed.** A `write` or `search_replace` row shows the change itself: numbered lines with two lines of context, added lines on a green row and removed ones on a red row, highlighted like the file. The numbers are the file's own, measured by the tool against the file on disk. A long edit shows its first 12 rows and says how many more there are; `^o` shows every edit whole, and `/changes` has the full diff of the turn or of everything uncommitted. The turn's closing line counts the real lines added and removed. In 16 colors and `NO_COLOR` the text carries the color, or the `+`/`-` sign alone.

Type a message and press `Enter`. `Shift+Enter` (or `Alt+Enter`) inserts a newline; paste is bracketed so multi-line text lands in one message. While a turn runs, `Enter` queues the next message. `↑`/`↓` on an empty composer walk prompt history.

`/` (or `^p`) opens the **command palette**: fuzzy-matched, grouped by category, with a description and keybinding column. `Enter` runs the command, `→` opens its panel, `Tab` completes. Every configuration command opens a **panel** — a bordered popout with a title, status, and legend line — and panels stack: one opened from another sits on top, and `Esc` closes one level.

The **activity strip** shows what the model is doing (`⠙ writing · edit docs/guide.md · 0:34 · 1.2k tok`). Reasoning streams there as a one-line ticker; `^r` expands it to a scrollable pane (`Alt+PgUp`/`Alt+PgDn`). Reasoning is display-only — it is never saved or sent back to a model. `[ui] reasoning = "off"` hides it.

**Scrolling.** The transcript follows the bottom until you scroll up (`PgUp`, `Shift+↑`, or the mouse wheel over the chat); then it holds still and the scrollbar turns amber. `Ctrl+End` reattaches. `Ctrl+↑`/`Ctrl+↓` jump between turns. While detached, a sticky header at the top of the chat keeps the in-flight user message in view.

`/sessions` (alias `/resume`) is one browser for this directory’s sessions: `Enter` resumes, `r` renames, `d` deletes (type the short id to confirm), `n` starts a new one. `ryter sessions` and `ryter resume [id]` are the CLI equivalents.

**Approving.** When the build hat needs your yes, a card opens just above the message box, with the chat still readable behind it. It has these rows:
- **what** the call does (`edit stats.js`, `run cargo test`);
- **why**: the model's own words just before it asked;
- **risk** in plain words, including whether `/undo` can put it back;
- **the change**: the diff for an edit, or the command.

The keys are on the card's last row:
- **`⏎`** allows this call. An Enter pressed in the first half-second after the card opens is ignored, since it may have been meant to send a message.
- **`a`** allows the kind of action the card names for the rest of the session: "edits to files in the project", or "`cargo test` commands". Anything else still asks.
- **`n`** or **`Esc`** denies.
- **Commands that delete, move, or discard files** (`rm`, `mv`, `git reset --hard`, `git clean`, deleting a branch) and **writes outside the project that ask** (see "Outside the project") take only **`y`**. Enter says so instead of approving, and there's no `a` for them.

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
ryter -c -p "continue"               # continue the latest session
```

`--always-approve` treats Ask as Allow. Deny still wins.

## Hats

One model works in your project, in the build hat to start with. `Tab` switches its hat (build → plan → review → test), `Shift+Tab` goes back, and `/build`, `/plan`, `/review` jump to one. The header, the message box's badge, and its border all show the hat in its own color. A switch applies to your next message. The model can also offer a switch itself: after a review ("fix these?") it asks with a yes/no prompt, and on `y` it carries on in the new hat in the same turn. A plan has its own panel, below.

| Hat | May | May not |
| --- | --- | --- |
| **build** (default) | edit files and run commands. Your toolchains, the project's own programs and its containers run without asking; edits ask (or run with `a` for the session / `--always-approve`); so do commands that delete, publish, or that Ryter doesn't know (see "What runs without asking") | read secrets, push, run inline interpreter code |
| **plan** | read, search, run read-only commands, and show you a plan to approve | edit source, run anything that changes the project |
| **review** | read, run the tests and linters, read-only git | write anything, not even by redirect; install, format, or fix |
| **test** | start the product, run its tests and use it: everything the build hat runs without asking, and requests to the project's own address (`curl localhost:8000/…`) | edit or write the project's files, delete or move anything in it |

**A model for each hat.** Every hat runs on one model until you give a hat its own. `/models` lists the seats on the left: *All hats*, then *Plan*, *Build*, *Review* and *Test*, each showing its model or "follows all hats". Pick a seat, pick a model, `⏎`, and the cursor moves to the next seat, so one visit sets them all. To put a hat back, choose `default` at the top of its list. The choice is kept in `~/.ryter/hats.toml`.

- **Where it shows:** the rail and the status line name the model your next message goes to, which is the current hat's.
- **What a switch costs:** the hats share one conversation. A model that hasn't read it yet reads all of it at the full price the first time, and Ryter says so in the chat as it happens: "review hat · grok-4.7 re-reads 42k tokens, about $0.13". Nothing stops; the line is there so the cost isn't a surprise. Going back to a model that has read the conversation costs the same again if its provider's cache has lapsed.
- **A use for it:** a strong model for the plan, a cheaper one to build it, and a different one to review, so the review isn't the model that built it marking its own work.

**The test hat has a conversation of its own.** The plan, build and review hats share one conversation. The tester doesn't read it: it judges the product from the plan, the decisions and from using it, not from the builder's account of the work.

- **`Tab` to Test** and the chat shows the tester's thread, under a line that names it ("TEST THREAD"). `Tab` away and the shared conversation is back as you left it. Each keeps its own scroll position.
- **It continues through the session.** The tester remembers what it tried before, so "retest the health check" works. Both conversations are saved, and both come back when you resume the session.
- **A turn stays in its own conversation.** You can look at the main chat while a test runs, or at the tester's thread while a build does: what a running turn says goes to the conversation it is part of. The model can't switch into or out of the test hat in the middle of a turn; that is yours to do with `Tab`.
- **What it may run:** what the build hat runs without asking (your toolchains, the project's programs, its containers), plus requests to the project's own address: `curl` to `localhost`, `127.0.0.1` or a `.localhost` name, saving only to `/tmp` or your home folder. What the build hat asks about, it asks about. It can't edit, delete or move the project's files, and a redirect into the project is refused.
- **Its own model:** the *Test* seat in `/models`. Starting a test thread on a different model costs nothing extra, since there is no conversation for it to re-read.
- **Not built yet:** the report that comes back into the main conversation, `/test`, and the offer after a passed review. Today the tester's findings stay in its thread, and you carry them to the build hat yourself. The approved design is in `docs/test-hat.md`.

**How a project runs: `.ryter/run.toml`.** The tester needs four things from a project: the command that starts it, an address that answers once it is up, its test commands, and the command that stops it.

```toml
start = "docker compose up -d --wait"
ready = "http://localhost:8000/healthz"
test  = [
    "docker compose run --rm web pytest -q",
    "docker compose run --rm web ruff check .",
]
stop  = "docker compose down"
```

- **The model drafts it, you approve it.** The first time the tester needs it, it reads the project and proposes the commands in a panel: `y` approves and saves the file, `e` says what to change, `n` rejects. Leave out what the project doesn't have.
- **Ryter runs what you approved, itself.** Starting waits until `ready` answers (any answer that isn't a server error), for up to five minutes. A start command that stays in the foreground (`npm run dev`, `cargo run`) is kept running by Ryter; through the shell tool it would be cut off when the command didn't return.
- **Approval is of what the file says.** It is kept in `~/.ryter/run-approved.toml`, not in the project. A run file that came with a clone, or that anyone changed since (you, the model, a `git pull`), is shown to you again before anything in it runs.
- **Limits:** a command Ryter runs for nobody (`sudo`, inline code) can't be in it, and `ready` has to be an address on this machine. Headless, `--always-approve` is your yes to the file as it stands; without it nothing in the file runs.

**The product is left running.** After a test the product stays up, so you can look at what the tester saw. The chat says where it is.

- **`/stop`** stops it: the `stop` command, or, when the file has none, ending the start command Ryter is holding.
- **Quitting asks.** With the product still up, `^c` shows "stop the project?": `⏎` stops it and leaves, `n` leaves it running, `esc` stays.
- **A later session knows.** Left running, it is remembered: the next session in that project says so, and `/stop` there runs the `stop` command. A start command left running with no `stop` command is yours to end; Ryter gives you its process number and doesn't end a process it can't be sure is the one it started.
- **Only what Ryter started.** It never stops containers or processes it didn't start, and never removes volumes unless your `stop` command says to.

**Approving a plan.** When the model has a plan, it shows it in a panel instead of writing it into the chat: the goal, the steps, the files, the risks, and how to verify it. `↑`/`↓` and `PgUp`/`PgDn` scroll a long one. You answer:

- **`y` approves.** The plan is saved in the project as `.ryter/plans/<date>-<title>.md`, Ryter switches to the build hat, and the model builds from that file in the same turn. Earlier plans are kept beside it, and whether to commit them is yours to decide.
- **`e` adjusts.** Type what should change. The model revises the plan and shows it again. Nothing is saved yet.
- **`n` (or `Esc`) rejects.** Nothing is saved, and the hat stays as it was.

`Enter` approves nothing here. The panel waits as long as you take to read. A headless run (`ryter -p`) has nobody to approve a plan, so nothing is saved and the plan comes back as the answer.

**When the work differs from the plan.** An approved plan is not edited afterwards. Where the work comes to differ from it, the difference and its reason are recorded in `.ryter/decisions.md` in the project, under the plan they belong to:

```
## plan: 2026-10-01-cms.md

### No export button in this pass
- Plan said: step 4, an Export button on the page list
- Built instead: no export
- Why: you said "skip the export button for now"
- Decided by: you · 2026-10-01 14:20
```

- **When an entry is added:** when you tell the model to leave out, add or change something the plan says, and when the model finds a step can't be done as written and takes another way to the same goal. Those are signed "build hat" with the model's name.
- **You see each one.** The chat says "decision recorded: No export button in this pass". Nothing stops and nothing is asked.
- **A review reads them.** A difference recorded there was decided, so the reviewer doesn't report it as a defect. A difference with no entry is still a finding.
- **The file is yours.** Remove an entry you don't agree with, or add your own under the plan's heading. Whether to commit it is yours to decide, as with the plans.
- **Limits:** a decision needs a plan approved in this session, and only the plan and build hats record one. A reviewer can't.

**Review before you commit.** The review hat is the check on work before it is committed. Give it its own model in `/models` and it is a second opinion: a different model from the one that built the work. There is one reviewer, and it is this hat.

- **Offered after your changes.** When a build turn finishes having changed files, Ryter offers a review with what it should cost: `⏎` runs it, with no second question; `n` passes; `s` stops the offers. `/settings` (review offers) or `[ui] offer_audit` turns them back on. Nothing is offered after a turn that only talked or read, or one that was cancelled or cut short.
- **Asked for:** `/audit` (or `/second`) runs the same review whenever you want one.
- **Every review asks first,** naming the model, what it will read, and a cost range. After a few reviews it also shows what your last ones with that model cost. `n` spends nothing. If the review hat follows the model every hat uses, the prompt says the reviewer is the model that built the work.
- **It is a turn in the conversation,** in the review hat, and the hat you were in comes back when it ends. The reviewer has read what you asked for, and the model you build with reads the findings next, so "fix those" works. Because it reads the conversation, a reviewer on another model pays to read it once; the chat says what that costs.
- **It checks against the plan.** If you approved a plan, the reviewer is pointed at its file and checks that the change does what it says, all of it and nothing more. It is also pointed at that plan's entries in `.ryter/decisions.md`: what you decided to do differently is not held against the work. With no plan it checks against what you asked for. Then correctness, tests and safety.
- **It ends with a verdict:** `VERDICT: PASS` or `VERDICT: FAIL`, with findings marked **blocking** or **note**. The chat repeats it under the review: "review · grok-4.7 · ✗ blocking problems · $0.04".
- **A failed review offers its fixes.** The reviewer asks to switch to the build hat; say yes and the build hat fixes them in the same turn. The fixes are new work, so a review of them is offered.
- **The commit says whether the work was reviewed.** The receipt on `/commit` ends with "review ✓ grok-4.7", "review ✗ grok-4.7", "not reviewed", or "not reviewed after the last change". A verdict holds for the files the reviewer read: change one afterwards, by hand or with the model, and the receipt says so.
- **A limit, if you want one.** `/settings` → *review usd* is the most one turn in the review hat may spend (0 is no limit). Each step is priced before it's sent. When about one step's room is left, or three quarters of the limit is spent, the reviewer is told to stop exploring and write up. A step that would pass the limit isn't sent, and a review stopped that way has no verdict. A model with no known price isn't run under a limit, since the limit couldn't hold it.
- **It can't change anything.** The review hat reads, and runs tests and linters. Edits, installs and formatting are refused.
- **A project that tests in containers is tested there.** The reviewer may run a test or lint command in one of the project's containers (`docker compose run --rm web pytest`, `docker compose exec web ruff check .`; `podman` the same) and look at what is running (`docker compose ps`, `logs`). The command inside answers to the same rules as outside. It may not build, start or stop the stack, or use `docker run`: if the stack isn't up, it says the tests weren't run.
- **Coming from 0.10.0:** the model and limit you chose for `/audit` (`~/.ryter/review.toml`) are now the review hat's model and the review limit. Change them in `/models` and `/settings`. `/audit model` is gone.

**What the chat shows.** The model narrates as it works: what it's doing next and why, each choice between approaches with its reason, and what it thinks went wrong when something fails. Each tool step shows what came of it, measured by Ryter: `new · 48 lines`, `rewrote · 76 lines (was 89)`, an edit's changed lines, `✓ 13 passed`, or `✗ exit 1` with the cause. Reads fold into one line, and a divider closes each turn that did work (`6 files (3 new, 3 changed, +153 −15) · 9 commands (9 ok) · 2:41`).

**What runs without asking** in the build hat. A question for every `cargo build` and `docker compose up` was answered yes every time, so these run:

- **Looking:** `ls`, `cat`, `grep`, `git status`, `git diff` and the like.
- **Your toolchains,** whatever the subcommand: `cargo`, `npm`, `pnpm`, `yarn`, `bun`, `node`, `python`, `pip`, `uv`, `pytest`, `go`, `make`, `mvn`, `gradle`, `dotnet`, and the rest of their kind. `cargo install`, `npm install` and `pip install` included.
- **The project's own programs:** `./scripts/setup.sh`, `bin/cms-admin`, `./manage.py`, and `bash` given a script file of the project's.
- **Programs you installed under your home folder:** whatever `PATH` finds in `~/.cargo/bin`, `~/.local/bin`, a node or python manager's folder.
- **The project's containers,** with `docker` or `podman`: `build`, `compose build`, `up`, `down`, `run`, `exec`, `restart`, `logs`, `ps`, `pull`, and `docker run` with folders of the project's mounted.
- **`cd`** into a folder of the project.

These still ask:

- **Edits** to the project's files, until you press `a` on one.
- **Changing files by hand:** `mkdir`, `cp`, `sed -i` and other system programs that aren't a toolchain. `a` on the prompt allows that command for the session.
- **Deleting and moving:** `rm`, `mv`, `chmod`, `git reset --hard`, and removing a stack's volumes (`docker compose down -v`, `docker volume rm`, any `prune`). `y` only, with no "allow for this session".
- **Publishing and signing in:** `cargo publish`, `npm publish`, `npm login`, `docker push`, `docker login`.
- **Tools that work on a service somewhere else:** `gh`, `aws`, `gcloud`, `kubectl`, `terraform`, `curl`, `wget`.
- **In Docker:** stopping or removing a container by name (`docker stop`, `docker rm`), since it may not be this project's; another machine (`-H`, `--context`); a compose file outside the project; and giving a container the host (`--privileged`, a mount of `/`, the Docker socket, or a folder where keys are kept).

A toolchain runs the project's code: `cargo build` runs its build script and `npm install` its install scripts. If you don't want that unasked, a sandbox profile limits what any command can touch (see "Sandbox profiles").

**Docker or Podman.** When both are installed the model is told to use Docker, unless you ask for Podman. With one installed it is told which.

**Outside the project.** Two places are open to every hat, to read and to write, without a question:

- **Scratch space:** `/tmp` and your system's temporary folder.
- **Your home folder,** where tools keep their caches, configuration and builds.

With these exceptions:

- **Never read or written:** credentials (`~/.ryter`, `~/.ssh`, `~/.gnupg`, `~/.aws`, `~/.docker`, `~/.config/gh`, and the like), your tools' saved logins (`~/.npmrc`, `~/.pypirc`, `~/.cargo/credentials.toml`, `~/.git-credentials`), your shell history, your browser's and mail client's folders, and secret files (`.env`, `*.pem`, `*.key`) wherever they are.
- **Never written:** shell startup files (`~/.bashrc`, `~/.zshrc`, `~/.profile`, …) and system folders (`/etc`, `/usr`, …).
- **Another project:** a folder under your home that is a git repository other than this one can be read, but writing there asks each time in the build hat and is refused in the others.
- **Deleting or moving** anything outside the project asks each time, scratch space and your home folder included.

Anywhere else (`/opt`, `/srv`, another disk), the build hat asks each time and no other hat writes. Those prompts say "outside the project" and offer only `y` (allow once) or `n`. "Allow all" and `--always-approve` don't cover them, so headless refuses them.

The plan and review hats still change nothing in the project itself. What is open to them is scratch space and your home folder: a place to keep a test's output, not a way to edit the work.

**A sandbox profile is stricter than all of this.** With `workspace` or `read-only` chosen in `/settings`, the system itself shuts `/tmp` and your home folder (beyond your tools and their caches) to every command, whatever the rules above allow. The model is told so, and where its scratch folder is, so a refusal isn't reported as a broken tool. To have `/tmp` and your home folder open, the profile has to be `off`.

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

A folder that isn't a repository gets git set up before the first change, and Ryter says so: `git init` (your `init.defaultBranch`, else `main`), a `.gitignore` for secrets and caches unless one exists, and a first commit of what is already there. A repository with no commits gets just the first commit. It won't make one in your home folder, at the root, or in a folder that holds other projects' repositories (a folder like `~/workspace`). There, edits are made without snapshots, and `/undo` is unavailable.

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
  Ryter: deepseek-pro-latest · $0.34 · tests ✓ 13 passed · review ✓ grok-4.7
  ```

  - **Model:** the models used since the previous commit.
  - **Cost:** what this project spent since the previous commit, across sessions, read from the spend logs. It shows `$0.34+ (some prices unknown)` when a call had no known price.
  - **Tests:** the latest test run's result. It says `tests not rerun after the last edit` if the model changed files after that run, or `no tests run`.

  `git log --grep "Ryter:"` finds them later.

Headless, `ryter -p` runs in build; `--hat plan|review` picks another. Headless nobody can approve an edit, so pass `--always-approve` to let build change files.

## Choosing models

**Every model is chosen in `/models`.** The seats are on the left, each with the model it runs on now: *All hats*, then *Plan*, *Build* and *Review*. The models for the chosen seat are on the right.
- **Choosing a seat:** `↑↓` picks one. `→` or `⏎` moves to its models, and typing starts a filter there straight away.
- **Setting a model:** `⏎` sets the highlighted model for the seat and takes you back to the seats, on the next one. You can set every seat in one visit: pick, `⏎`, pick, `⏎`. A `✓` marks each seat you've set.
- **Other keys:** `←` goes back to the seats without setting anything. `Tab` steps the highlighted model's reasoning, `s` sorts, and `Esc` closes.

A model your account can't use (a data policy that refuses it, no tool support, no credits) fails on its first message, and the chat gives the provider's reason. Pick another in `/models`.

## Spend

Every model call is priced before the next request. Roll-ups: session, turn, hat, connection. Persisted in `spend.jsonl`.

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

A session budget is optional. With one, the turn stops when spend reaches it and says so (exit `3` in headless). Raise it and say continue. Without one, nothing stops on cost and you watch the spend card.

| Command | Effect |
| --- | --- |
| `/budget` | the budget panel: spend against the cap, on/off, the cap, and the warning level (`^s` saves) |
| `/budget 5` | cap this session at $5 |
| `/budget +2` | raise the cap by $2, e.g. after hitting it |
| `/budget off` | no cap |

The **budget** card on the right shows the cap, how much is used and left, or `off`; click it to open the panel. Changes apply at once and are saved as your default (`~/.ryter/settings.toml`, the same value as *budget usd* in `/settings`). A trusted project's `[spend] session_budget_usd` overrides your default in that project. There is no session budget until you set one. `[spend] enabled = false` still counts in memory and prints a warning.

**Project cost.** A project is its git repository (the folder, outside one), so sessions started in any subfolder count toward it. The spend card shows `project` under the session total. When the project is a repository around the folder you started in, the rail says which folder, for example `in ~/workspace`, and the `$` drawer names its project column for it. `p` in `/spend` switches to the project view: the total across sessions, this month, and breakdowns by hat, model, and month. A project that was worked on in crew mode, before it was removed, also shows what that cost. `ryter spend --project` prints the same. Nothing extra is recorded: every call is already in its session's `spend.jsonl`, and a running total in `~/.ryter/projects/` means only new lines are read. Calls with no known price are counted and shown (`$14.20+`), never added as $0.

`/spend` is a panel: session total, a budget gauge, and tables by hat and by connection; `p` switches to the project; `e` exports CSV. The info panel’s spend card shows the total, and the budget card below it shows the cap. `ryter spend` prints the roll-up on the CLI.

## Slash commands

Type `/` to open the palette; every built-in has a one-line description there. Configuration commands open panels: `/settings` `/provider` `/models` `/mcp` `/skills` `/rules` `/hooks` `/sessions` `/spend` `/theme` `/tools` `/context` `/doctor` `/help`. `/changes` and `/commit` open panels too (see [Review and commit](#review-and-commit)). Direct commands act immediately: `/undo [force]` `/redo [force]` `/new` `/rename <title>` `/budget [amount|+amount|off]` `/compact` `/cancel` `/quit`. Near-duplicates are hidden aliases (`/resume` → `/sessions`, `/model` → `/models`, `/connections` → `/provider`); `/delete [id]` stays as a hidden direct command.

User-invocable skills and `~/.ryter/commands/*.md` join the palette under **skills**. Built-ins win on a name clash.

## MCP

**Outbound.** `[mcp_servers.<name>]` stdio children. The model discovers them with `search_tool` and calls one with `use_tool`. Child env does not inherit API keys unless that server’s `env` table asks. In the TUI, `/mcp` is a panel: `Enter` toggles a server, `r` reconnects, `d` removes it (type the name to confirm), and `Enter` on the trailing `+ add server` row walks name → command → args → review. Each server row shows its live status (`connected · 5 tools`, `error: …`, `disabled`).

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
| Prompt | `prompts/solo.md`, the one prompt every hat shares; override with `~/.ryter/prompts/solo.md`, then a trusted project's `.ryter/prompts/solo.md` |
| Rules | `~/.ryter/RYTER.md` for every project; `RYTER.md` (or `AGENTS.md`) at a project's top for that project. See [Your rules](#your-rules). |
| Skills | `/skills` panel. Files: `~/.ryter/skills/<name>/SKILL.md` (frontmatter `user-invocable`, `model-invocable`). `Enter` runs (optional args), `e` opens the file in `$EDITOR`, `a` writes a stub, `d` deletes a user skill (not a project overlay or a built-in one). |
| User slash | Same `/skills` list (`command` rows). `~/.ryter/commands/<name>.md` (`$ARGUMENTS`) |
| Hooks | `/hooks` panel. `a` adds: event (`PreToolUse`, `PostToolUse`, `SessionStart`) → command or URL → optional matcher. `d` removes. Live list is `~/.ryter/hooks.toml` (does not rewrite `config.toml`). Command gets JSON on stdin; exit 2 or HTTP 403 denies. |
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

Ryter keeps your standing rules in two plain Markdown files, and puts both into the model's instructions on every message, whichever hat it is in:

- **`~/.ryter/RYTER.md`:** your rules for every project. How you like work reported, what to ask before doing, spelling, tone.
- **`RYTER.md` at the top of a project** (or `AGENTS.md` when there's no `RYTER.md`): rules for that project.

Your rules come first, and where the two differ the project's win. You can edit either file by hand at any time. Ryter loads up to 32 KB of the every-project file.

**The `/rules` panel.** `/rules` opens both files, one at a time, with no model involved:

- **Tab** switches between *every project* and *this project*. The line under the tabs names the file and says how many rules it holds.
- **`a`** adds a rule under the selected line. What you type becomes a bullet; a line you start with `#` is kept as a heading.
- **`d`** removes the selected line, after you answer `y`.
- **`e`** opens the file in your editor (`$VISUAL`, then `$EDITOR`, then `vi`).

The first rule you add creates the file. For this project that is `RYTER.md`, unless the project already keeps an `AGENTS.md`, which is then the one read and added to. A change takes effect from your next message.

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

### Sandbox profiles

A sandbox limits which files the model's commands can reach. It is enforced by the system (Linux's Landlock), not by Ryter's own rules, so it holds even for a command Ryter would have allowed. Choose a profile with `--sandbox <profile>`, or in `/settings` → *sandbox*, which shows this comparison. A change applies the next time Ryter starts. The default is `off`.

| | `off` | `workspace` | `read-only` |
|---|---|---|---|
| Project files | read, write | read, write | read |
| The rest of your home folder | read, write | no | no |
| Your tools | yes | yes | yes |
| Your keys | by rule only | never | never |
| `/tmp` | read, write | no | no |
| Network | yes | yes | yes |
| Docker | yes | yes | yes |

**When to use each:**

- **`off`:** you are watching each step. Ryter's own rules still apply: it asks before a command that changes things, and refuses to read your keys. Nothing stops a command you approved from reaching the rest of your machine.
- **`workspace`:** tools run without asking (`/tools always`, `--always-approve`, `ryter serve`), or you are working on code you don't trust. Commands can change only the project.
- **`read-only`:** you only want a review. Nothing in the project can be changed either, which also means nothing can be built into it.

**What "your tools" means.** Under `workspace` and `read-only`, commands can read and run:

- system folders (`/usr`, `/bin`, `/etc`), and where package managers install (`/opt`, `/nix`, `/snap`, Homebrew);
- toolchains under your home folder: `~/.cargo/bin`, `~/.rustup`, node version managers (`~/.nvm`, `~/.volta`, `fnm`, `asdf`, `mise`), `~/.pyenv`, `~/.bun`, `~/.deno`, `~/go/bin`, `~/.local/bin`, pipx and uv;
- any other folder on your `PATH` that is under your home folder, as that folder alone;
- your git identity (`~/.gitconfig` and `~/.config/git/config`).

A tool folder that is a symbolic link is left out, since a grant on a link is a grant on what it points at. If your `~/.npm` or `~/.cargo/registry` is a link to another disk, builds under the sandbox can't use that cache.

They can also write the tools' download caches (`~/.cargo/registry`, `~/.npm`, pip's, uv's, Go's and others), so a build that fetches a dependency works.

**What stays shut:** the rest of your home folder, `~/.ssh`, the tools' saved logins (`~/.cargo/credentials.toml`, `~/.npmrc`, `~/.config/git/credentials`), Ryter's keys, and `/tmp`. Ryter also closes its own process to the commands it runs, so a key held in its memory or its environment can't be read from `/proc`. Temporary files go to `~/.ryter/tmp`, and `TMPDIR` points commands there.

**What a sandbox doesn't do:**

- **It doesn't limit the network.**
- **It doesn't contain Docker.** A command that can reach the Docker socket can mount the whole machine. If that matters, don't give the account Docker access. `docker build` works under a profile (its lock folder, `~/.docker/buildx`, is writable; the registry logins beside it stay shut).
- **Rootless Podman can't run under it.** Podman keeps its state in your home folder and starts containers in a user namespace of its own, and a sandboxed command can do neither. Use Docker, or the `off` profile, for a project that needs Podman.
- **It needs Linux.** Elsewhere Ryter refuses to start with `workspace` or `read-only`, and it also refuses on a Linux kernel that can't enforce Landlock. On a kernel older than 5.19 a profile works, but a file can't be moved from one folder to another under it, so some builds fail (`cargo` building a library, for one).
- **Some of Ryter's own features are off under it:** your every-project rules can't be changed, and pages aren't opened in a browser.


## Safety

- One gate: `decide(hat, tool, args)` → Allow / Ask / Deny. Every hat is offered the same tools; the gate decides what each may do with them.
- **Build:** reading runs; edits and commands that change things ask; destruction always asks.
- **Plan:** reading and read-only commands; it may write the project's memory files and its own notes, nothing else.
- **Review:** reading, tests, linters and read-only git; no writes at all.
- Denied in every hat: `.env`, `*.pem`, `*credential*`, `~/.ssh`, Ryter credential files.
- Shell commands are judged per segment (`a && b` is two commands). Privilege escalation, disk writes, `git push`, and piping into a shell are denied. In the TUI a permission modal shows the tool and its arguments: `⏎` or `y` allow this call, `n` deny, `a` allow that kind of action for the rest of the session; destructive commands and writes outside the project take only `y` (see [Approving](#talking-to-ryter)). Headless (no TUI) fail-closes.
- `ask_user` lets the model ask a question; the TUI shows it as a modal (number keys pick a choice, or type free text).
- `[features] web = true` offers `web_fetch` / `web_search`. Localhost and private IPs are blocked.
- Hooks can still deny after the policy allows.

Logs append to `~/.ryter/logs/ryter.log` (no secrets). Project `.ryter/` overlays apply after `ryter trust`, or after the TUI “trust this project?” prompt.

## Project memory

A project can keep **why** on disk, not only in a conversation:

| File | Role |
| --- | --- |
| `ROADMAP.md` | Now / Next / Later / Done / Blocked. |
| `DECISIONS.md` | Decision records (chosen vs rejected, why, where). |
| `notes/*.md` | Notes kept beside them. |

When these files exist, the model **reads** them on every turn (capped), and is told to update them as work changes and to add a short entry to `DECISIONS.md` when it makes a non-obvious decision. When you ask why something is a certain way, it should quote `DECISIONS.md` and open the files it names. Ryter doesn't create them: a project that has none gets none until you or the model writes one. The plan hat may write these files; the review hat may not.

## Sessions

```
~/.ryter/sessions/<cwd-slug>/<id>/
  meta.json
  events.jsonl
  transcript.jsonl # the conversation the plan, build and review hats share
  test.jsonl       # the test hat's own conversation, once it has one
  spend.jsonl
  notes/           # the plan hat's notes, and project.log: the output of a product the tester started
```

A session saved in crew mode, before it was removed, still opens: its conversation and its spend are there, and it carries on in the build hat.

No SQLite. `ryter spend` uses the latest session for this directory.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | ok |
| 1 | error (including not a tty without `-p`) |
| 3 | spend budget exceeded |
