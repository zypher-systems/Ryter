# Ryter user guide

Ryter is a Bring-Your-Own-Key terminal coding harness. One model works in your project, with you. It wears one of four hats at a time (**plan** and **build**, then the specialists **audit** and **scribe**), and each hat can run on a model of its own.

Linux is the first platform. Release binaries also support macOS; Landlock requires Linux. Windows is not currently supported.

## Install

Rust 1.88+ (see `rust-toolchain.toml`).

```sh
cargo build -p ryter-cli
./target/debug/ryter --version    # must not open config, keyring, or the network
./target/debug/ryter doctor
```

Put the `ryter` binary on your `PATH` if you want. Data lives in `~/.ryter/` (`RYTER_HOME` overrides).

### Updates

The bootstrap installer authenticates `SHA256SUMS.sig` with the same pinned Ed25519 release key as the updater before checking or extracting the archive. A download mirror cannot supply a different key. It requires OpenSSL 3+; on a Mac with Homebrew OpenSSL, use `RYTER_INSTALL_OPENSSL="$(brew --prefix openssl@3)/bin/openssl"` when running the script. It refuses unsigned releases, including releases from before signing was introduced. Failed verification preserves an existing installation.

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

**Auto** means Ryter picks by hat: `high` for the plan hat, `medium` for build and audit. `[reasoning_effort]` overrides that per hat (`build`, `plan`, `audit`; `review` still reads as `audit`). **Model's own** sends nothing. Beware: with no setting, some models think for minutes before acting. The level is sent only to OpenRouter connections.

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

The hat that is on sets the screen's one accent color: plan is cyan, build green, audit amber. Everything else stays the same under every hat. A fedora in that color sits faintly behind the conversation (`[ui] watermark = false`, or *watermark* in `/settings`, turns it off; it is left out on 16-color and no-color terminals).

- **The bar across the top** names the hats in their two rows, plan and build, then a dot, then audit and scribe. The hat that is on is a filled chip marked `◆`; a hat that has had a turn this session is marked `●` in its own color, and one that hasn't is `○`. The bar says which hats have been worn, not an order to wear them in. Then the session's title, the folder and the branch.
- **The hat rack** runs down the left (screens 132 columns and wider). It has one block a hat, always in the same place: plan and build, a hairline marked `specialists`, then the specialists below it. Each block: the hat's model, how many turns it has had this session and what it has cost, and the figures that are its own. Plan: plans approved and rejected. Build: files and lines it changed in the session, and the latest run of the project's tests as three rows, `success`, `warning` (skipped or ignored) and `failure`. Audit: audits failed and passed. A hat with no turns yet says `not worn yet`. The block of the hat that is on is tinted.
- **The instruments** run down the right (100 columns and wider): the model your next message goes to, its connection and reasoning level; the context gauge with tokens used of the window; the **pulse**, how many tokens a second the model is writing, with the last eight seconds as bars, `idle` between turns; **spend** for the session and for the project, and the budget; the sandbox and what this hat may do; and what is **uncommitted**, a file a line. The `GUARD` card also names `plan.md` and `audit.md` under `.ryter/`, the approved plan and the latest audit, or `none`.
- **The prompt** sits under a rule in the hat's color, after the hat's name as a chip, the width of the conversation's column. The keys that matter now are on the last row.

The side columns give way to the conversation. Under 132 columns the hat rack folds away, and the bar counts each hat's turns instead. Under 100 the instruments fold away too, and the last row becomes a status line: the model, the context gauge, the session's cost and the budget. `^b` hides and shows the columns on a wide screen; on a narrower one it opens the hat rack and the instruments as a panel (`↑↓` scroll, `esc` or `^b` closes). `[ui] panel = false` starts with them hidden. A panel too wide to fit beside the columns, such as `/models`, takes the whole screen while it is open.

The rest of the screen:

- **One reading column on a timeline**, centered, at most 112 columns wide. Each question starts with its time and `●`. The model's text hangs off `◆`, each tool step off `├─` (`edit   src/config.rs ······· +9 −1`, `run    npm test ····· ✓ 4 passed`), and edits show as full-row green and red diffs.
- **Every turn closes with what it came to**, measured by Ryter: `└─ ✓ 5 tools · 2 files (2 changed, +3 −0) · 1 command (1 ok) · 7.9s · $0.001`.
- **Finished turns fold to one line:** what was asked, led by dots to that summary, ending in `▸`. The newest turn stays open. `^o` opens every turn and every edit whole, and folds them again.
- **`$`** on an empty composer opens the **spend drawer** above it. It shows this turn, the session, and the project side by side, with spend by hat for the session and the project, and the budget and warning level. `b` sets a budget, `⏎` opens the full `/spend` table.

**The workbench** (`^T`, or `/changes` on the ledger) shows what changed beside the chat, in three panes. It has the screen to itself, with a strip across the top naming the views (`chat`, `changes ^t`) and one bar at the foot:
- **Left:** the files changed (this turn, or since the last commit with `tab`), the turn's commands and what came of them, and the turns.
- **Middle:** the chat.
- **Right:** the selected file's changes, one at a time, with line numbers on both sides.

Its keys: `↑↓` pick a file, `j`/`k` move between its changes, `x` undoes the selected change alone, `X` undoes the whole file (after a `y`), and `u` undoes the turn. `x` and `X` are recorded like a turn, so `/undo` brings them back. While the workbench is open, keys go to it and not the composer; `esc` or `^T` returns to the chat.

`[ui] layout = "classic"` (also in `/settings`, applied at once) brings back the 0.5 screen. That's a header row, the chat with a right-hand **info panel** of cards (session, model + context gauge, spend + budget gauge, mcp), the **activity strip** while a turn runs, the bordered **composer**, and a hint bar. `^b` shows or hides the info panel there (it also drops automatically under 80 columns).

Every message is a left-aligned block under a speaker header — your name (from `[ui] username`, then `git user.name`, then `$USER`), the model name, `· system`, or a one-line tool row (`· read_file  path  0.1s`). Markdown renders with headings, lists, quotes, tables, and fenced code with syntax highlighting and a line-number gutter. Long model turns end with a summary line (`3 tools · 12.4k tok · 0:42 · $0.01`).

**Edits show what changed.** A `write` or `search_replace` row shows the change itself: numbered lines with two lines of context, added lines on a green row and removed ones on a red row, highlighted like the file. The numbers are the file's own, measured by the tool against the file on disk. A long edit shows its first 12 rows and says how many more there are; `^o` shows every edit whole, and `/changes` has the full diff of the turn or of everything uncommitted. The turn's closing line counts the real lines added and removed. In 16 colors and `NO_COLOR` the text carries the color, or the `+`/`-` sign alone.

Type a message and press `Enter`. `Shift+Enter` (or `Alt+Enter`) inserts a newline; paste is bracketed so multi-line text lands in one message. While a turn runs, `Enter` queues the next message. `↑`/`↓` on an empty composer walk prompt history.

`/` (or `^p`) opens the **command palette**: fuzzy-matched, grouped by category, with a description and keybinding column. `Enter` runs the command, `→` opens its panel, `Tab` completes. Every configuration command opens a **panel** — a bordered popout with a title, status, and legend line — and panels stack: one opened from another sits on top, and `Esc` closes one level.

**One spinner.** While a turn runs the conversation ends on a row that moves, under the model's name: the spinner in the hat's color and what the model is doing (`⠙ thinking · 1:40 · 12k tokens`, `running cargo test · 0:03`, `waiting for the model · 0:42`, `waiting for you · allow?`). `^r`, or a click on that row, opens the turn's reasoning as a pane right under it, dim and following its tail (`Alt+PgUp`/`Alt+PgDn` scroll it); after the turn the pane sits under the turn's closing line, and `^r` or a click on its header closes it. While a model has sent nothing yet the pane says so, `waiting for the model · nothing streamed yet`, since some models think on the server and send nothing until they answer. Reasoning is display-only — it is never saved or sent back to a model. `[ui] reasoning` is `collapsed` (the row alone, the default), `expanded` (the pane open) or `off` (closed until `^r`).

**A busy model is unmistakable.** While a turn runs, the hat's chip on the top bar spins in place of its `◆`, and the conversation ends on a row that moves: `⠹ thinking · 1:40 · 12k tokens` while reasoning streams, `writing` while text does, `running cargo test · 0:03` while a tool runs, `waiting for the model · 0:42` when nothing at all has arrived for three seconds, and `waiting for you` while a prompt is open. The pulse card reads `waiting` then, `idle` only between turns. The row goes when the turn ends and its closing line takes its place. A model that sends its tool calls in one message (twenty files written in a row) shows each as it runs, a row a frame.

**Scrolling.** The transcript follows the bottom until you scroll up (`PgUp`, `Shift+↑`, or the mouse wheel over the chat); then it holds still and the scrollbar turns amber. `Ctrl+End` reattaches. `Ctrl+↑`/`Ctrl+↓` jump between turns. While detached, a sticky header at the top of the chat keeps the in-flight user message in view.

`/sessions` (alias `/resume`) is one browser for this directory’s sessions: `Enter` resumes, `r` renames, `d` deletes (type the short id to confirm), `n` starts a new one. `ryter sessions` and `ryter resume [id]` are the CLI equivalents.

**Approving.** When the build hat needs your yes, a card opens in the conversation's column, between the chat and the message box: inset from the column's edges so it reads as a card, with a heavy top edge in the warn color. The chat moves up to make room for it, so the last lines of the reply stay in sight above it; nothing is painted over. The card never takes more than a third of the column, and a change longer than that scrolls inside it with `↑↓` or the mouse wheel. It is announced where you are looking: the row under the model says `waiting for you · allow?`, the hat's chip in the top bar keeps spinning, and the foot line shows the card's keys in the warn color. `[ui] bell = true`, or *bell on a question* in `/settings`, rings the terminal's bell once when a card opens; it is off by default. The card has these rows:
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
- it used the rounds one message may have: 150 unless *rounds a turn* in `/settings`, `[limits] rounds` in `config.toml`, or `--rounds N` for one run says otherwise, and `0` lifts the cap (say “continue”);
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
| `^b` | show or hide the hat rack and the instruments (ledger; on a screen under 132 columns, open them as a panel), or the info panel (classic) |
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

Mouse: wheel scrolls the chat, clicking a card opens its panel, clicking the status row under the model, or the reasoning pane's header, toggles the pane. Text selection uses your terminal’s own modifier (Shift+drag on most). `[ui] mouse = false` turns capture off entirely.

Without a tty, use headless:

```sh
ryter -p "add a --json flag" --always-approve
ryter -p "…" --json                  # NDJSON AgentEvent stream
ryter -c -p "continue"               # continue the latest session
```

`--always-approve` treats Ask as Allow, except a write outside the project, scratch space and your home folder, which headless refuses. `--yolo` says yes to those too. Deny still wins, in both.

## Hats

One model works in your project, in the plan hat to start with. The hats are in two rows: **plan** and **build**, the hats the work is done in, and below a line the **specialists**, **audit** and **scribe**. `Tab` moves within the row you are in: between plan and build, or round the specialists. `Shift+Tab` moves to the other row, onto the hat you last wore there (build, or audit, when you haven't). `/plan`, `/build` and `/scribe` jump straight to a hat; `/audit` runs an audit. The bar across the top shows both rows with a dot between them, and the hat rack, the prompt's chip and its rule all show the hat in its own color.

**The hat a session starts in** is `[ui] start_hat`, also *start in* at the top of `/settings`: `plan` (the default: read and propose first), `build` (straight to work), `audit` (open on an audit), or `last` (the hat this project's most recent session ended in; plan when there is none). A resumed session opens in the hat it was left in. `ryter --hat build` opens in that hat for one run, whatever the setting. `/new` keeps the hat you are in. Headless runs (`ryter -p`) are not affected: they are in the build hat unless `--hat` says otherwise. A switch applies to your next message. The model can also offer a switch itself: after an audit ("fix these?") it asks with a yes/no prompt, and on `y` it carries on in the new hat in the same turn. A plan has its own panel, below.

| Hat | May | May not |
| --- | --- | --- |
| **build** | edit files and run commands. Edits, your toolchains, scripts, inline code, the project's containers and ordinary git run without asking; what deletes, throws work away in git, publishes or leaves the machine asks (see "What asks") | read secrets, run a shell handed a command as text, gain privilege |
| **plan** | read, search, run read-only commands, look at the running product (a `GET` of its address, a project program's `--help` or `--version`), and show you a plan to approve | edit source, run anything that changes the project |
| **audit** | everything the build hat runs: the tests, the end-to-end checks, the product through the run file, inline code, the containers. A checkpoint before the turn puts back whatever it changed. Its one file is `.ryter/audit.md` | commit, push or discard work in git; keep a change in the project; read secrets |
| **scribe** | read, search, run read-only commands, look at the running product (a `GET` of its address, a project program's `--help` or `--version`), and write documentation: `.md`, `.txt` and their kind, anywhere in the project | write code, configuration or Ryter's own files; run anything that changes the project |

**A model for each hat.** Every hat runs on one model until you give a hat its own. `/models` lists the seats on the left: *All hats*, then *Plan*, *Build*, *Audit* and *Scribe*, each showing its model or "follows all hats". Pick a seat, pick a model, `⏎`, and the cursor moves to the next seat, so one visit sets them all. To put a hat back, choose `default` at the top of its list. The choice is kept in `~/.ryter/hats.toml`.

- **Where it shows:** the hat rack names each hat's model, and the instruments (or the status line on a narrow screen) name the one your next message goes to, which is the current hat's.
- **What a switch costs:** the hats share one conversation. A model that hasn't read it yet reads all of it at the full price the first time, and Ryter says so in the chat as it happens: "audit hat · grok-4.7 re-reads 42k tokens, about $0.13". Nothing stops; the line is there so the cost isn't a surprise. Going back to a model that has read the conversation costs the same again if its provider's cache has lapsed.
- **A use for it:** a strong model for the plan, a cheaper one to build it, and a different one to audit, so the audit isn't the model that built it marking its own work.

**How a project runs: `.ryter/run.toml`.** The build and audit hats can start the product, run its tests and stop it through one file, with commands you approved once: the command that starts it, an address that answers once it is up, its test commands, and the command that stops it. The model asks for it with `run_project`; the plan hat starts nothing.

```toml
start = "docker compose up -d --wait"
ready = "http://localhost:8000/healthz"
test  = [
    "docker compose run --rm web pytest -q",
    "docker compose run --rm web ruff check .",
]
stop  = "docker compose down"
```

- **The model drafts it, you approve it.** The first time a hat needs it, it reads the project and proposes the commands on a card under the conversation, where Ryter's questions are asked, with the reply still in view above it: `y` approves and saves the file, `e` says what to change, `n` rejects. Leave out what the project doesn't have.
- **You are shown every word.** A command longer than the panel wraps under itself, and `y` is taken only once the last row has been on screen: what you approve here runs without another question. A command that deletes or discards something is pointed out above the list.
- **Ryter runs what you approved, itself.** Starting waits for HTTP 200–399 from `ready`, for up to five minutes. HTTPS performs normal TLS certificate verification. A TCP connection or a 401/404 is not ready. If the address is already occupied, stop that service or choose another address before starting. A start command that stays in the foreground (`npm run dev`, `cargo run`) is kept running by Ryter; through the shell tool it would be cut off when the command didn't return.
- **Approval is of what you were shown.** Quoted spaces and line breaks in commands survive saving and reloading unchanged. Approval is kept in `~/.ryter/run-approved.toml`, not in the project. A run file that came with a clone, or that anyone changed since (you, the model, a `git pull`), is shown to you again before anything in it runs, and a file rewritten while you were reading is not the one you approved.
- **It is the project's own file, or it is not read.** Ryter writes the run file, and its plans, decisions and reports, into the project and never through a link: a project that arrives with a link where one of those files goes has the link replaced, not followed, and a link there is not read either.
- **Limits:** a command Ryter runs for nobody (`sudo`, inline code) can't be in it, and `ready` has to be an address on this machine. A `stop` of `pkill -f name` written out is refused with the fix: the pattern matches the shell that runs it.
- **Headless** (`ryter -p`), nobody can approve anything, so the model can't save a run file. One you wrote yourself runs with `--always-approve`, as far as that flag reaches in the build hat: nothing outside the project, scratch space and your home folder. It is not recorded as approved, so the TUI still asks.
- **A start that doesn't come up is taken down again.** If startup exits with an error, `ready` never answers, or you press `esc`, Ryter runs the approved stop command and ends its owned process group. If cleanup or `/stop` fails, the record stays available for retry, including after resume. Retry `/stop` before starting it again.

**The product is left running.** After `run_project` starts it, the product stays up, so you can look at it yourself. The chat says where it is.

- **`/stop`** stops it: the `stop` command, or, when the file has none, ending the start command Ryter is holding. A `stop` of `pkill -f name` matches the shell that runs it and ends that too; Ryter counts that as the stop working, and the model is told to write the pattern as `[n]ame` when it proposes one. A `stop` corrected in the run file after a failed cleanup is the one the retry uses.
- **Quitting asks.** With the product still up, `^c` shows "stop the project?": `⏎` stops it and leaves, `n` leaves it running, `esc` stays.
- **A later session knows.** Left running, it is remembered: the next session in that project says so, `run_project` doesn't start it a second time, and `/stop` there runs the `stop` command. A start command left running with no `stop` command is yours to end; Ryter gives you its process number and doesn't end a process it can't be sure is the one it started.
- **Only what Ryter started.** It never stops containers or processes it didn't start, and never removes volumes unless your `stop` command says to.

**Approving a plan.** When the model has a plan, it shows it in a panel instead of writing it into the chat: the goal, the steps, the files, the risks, and how to verify it. `↑`/`↓` and `PgUp`/`PgDn` scroll a long one. You answer:

- **`y` approves.** The plan is saved in the project as `.ryter/plan.md`, the fixed path every hat reads, and as `.ryter/plans/<date>-<title>.md`, the dated copy that stays when the next plan replaces it; Ryter switches to the build hat, and the model builds from that file in the same turn. Earlier plans are kept beside it, and whether to commit them is yours to decide.
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
- **An audit reads them.** A difference recorded there was decided, so the auditor doesn't report it as a defect. A difference with no entry is still a finding.
- **The file is yours.** Remove an entry you don't agree with, or add your own under the plan's heading. Whether to commit it is yours to decide, as with the plans.
- **Limits:** a decision needs a plan approved in this session, and only the plan and build hats record one. An auditor can't.

**Audit before you commit.** The audit hat is the check on work before it is committed; it was the review hat until 0.16, and `/review` still runs one. Give it its own model in `/models` and it is a second opinion: a different model from the one that built the work. There is one auditor, and it is this hat.

- **Asked for:** `/audit` (or `/second`) runs an audit of the uncommitted changes, from any hat; the hat you were in comes back when it ends. In the audit hat, asking in your own words runs one too. Nothing is offered after a build turn any more: the one hat change Ryter makes on its own is plan into build when a plan is approved.
- **Every audit asks first,** naming the model, what it will read, and a cost range. After a few audits it also shows what your last ones with that model cost. `n` spends nothing. If the audit hat follows the model every hat uses, the prompt says the auditor is the model that built the work.
- **It runs everything.** The auditor reads the diff against the plan and the decisions, then runs the project's tests through the run file, its end-to-end checks, and the product itself at its address. It may run what the build hat may: toolchains, scripts, containers, inline code.
- **It changes nothing, by a checkpoint.** Before the turn Ryter snapshots the files as it does before a build turn; after it, whatever the audit left changed is put back, and the audit says which files. Its own files, `.ryter/audit.md` and the dated copies under `.ryter/audits/`, are left as written. Git stays read-only for it. In a folder that is not a git repository there is no checkpoint: the audit is told, and held to tests, linters and reading for that turn.
- **It ends on a card.** The auditor files its findings with a tool, and the audit opens over the chat: the verdict and a summary, the findings worst first, each failure with where it is, what is wrong and what was run and seen, then what ran, whether the tree changed, and where it was written. `↑↓`, `PgUp`/`PgDn` and the mouse wheel scroll it; `o` opens the file; `n` or `esc` closes it. **A failed audit goes to the build hat on `⏎`:** the hat switches and the builder's next turn is the audit, under "Repair what this audit found; run the checks it ran." That switch is yours, not Ryter's. A passed audit has nothing to repair: `⏎` closes it. One with no verdict, from an auditor that filed nothing and gave none, says `no verdict` in its title and closes on `⏎` too. `y` is not one of the card's keys. An audit that gives its verdict in words without filing is said so in the chat, and its last `VERDICT:` line stands.
- **`audit.md` is read by the builder.** When `.ryter/audit.md` is newer than the plan it works from, the build hat reads it before changing anything and says which findings the work addresses. The guard card shows the latest audit's date, or `writing…` while one runs.
- **It checks against the plan.** If you approved a plan, the auditor is pointed at `.ryter/plan.md` and that plan's entries in `.ryter/decisions.md`: what you decided to do differently is not held against the work. With no plan it checks against what you asked for. Then whether it works, whether it is correct, whether it is safe.
- **The commit says whether the work was audited.** The receipt on `/commit` ends with "review ✓ grok-4.7", "review ✗ grok-4.7", "not reviewed", or "not reviewed after the last change". A verdict holds for the files the auditor read: change one afterwards, by hand or with the model, and the receipt says so.
- **A limit, if you want one.** `/settings` → *audit usd* (`audit_usd` under `[spend]`; `review_usd` still reads) is the most one turn in the audit hat may spend (0 is no limit). Each step is priced before it's sent. When about one step's room is left, or three quarters of the limit is spent, the auditor is told to stop and file. A step that would pass the limit isn't sent, and an audit stopped that way has no verdict. A model with no known price isn't run under a limit, since the limit couldn't hold it.
- **Coming from 0.15.0:** the offer of an audit after a build turn is gone, and so is its setting; an old settings file with it loads. `review_usd` is `audit_usd`; both names load.

**The scribe writes the documentation.** `Shift+Tab` to the specialists and `Tab` to it, or `/scribe`. It reads everything the plan hat reads and runs the same read-only commands, and like the plan hat it may look at the running product: a `curl` `GET` or `HEAD` of the project's own address (`localhost`, `127.0.0.1`), and a program of the project's own (`.venv/bin/tasks`, `./target/debug/app`) asked only `--help`, `-h`, `--version` or `-V`, so what it documents is what the product says. Anything else it may not run is refused with a note that it only looks, and which key reaches the build hat. It may write only documentation: `.md`, `.mdx`, `.txt`, `.rst`, `.adoc`, and a `README`, `CHANGELOG`, `LICENSE`, `CONTRIBUTING`, `NOTICE` or `AUTHORS` with no extension, anywhere in the project but Ryter's own folder. A write to anything else is refused with a note that code is the build hat's. It is told to write from what is there, `.ryter/plan.md` included, and to invent nothing. Asked for documentation and nothing more specific, it writes two documents: `README.md` (what the project is, how to build and run it) and `docs/guide.md` (how to use it; for a service, how to operate it), keeping either that exists; asked for one document, it writes that one. Its point is the model seat: give it a small, cheap model in `/models` (`[scribe]` in `hats.toml`, or `[specialists.scribe]` in `config.toml`), and the docs stop costing what the code costs. The hat rack counts the documents it wrote this session.

**What the chat shows.** The model narrates as it works: what it's doing next and why, each choice between approaches with its reason, and what it thinks went wrong when something fails. Each tool step shows what came of it, measured by Ryter: `new · 48 lines`, `rewrote · 76 lines (was 89)`, an edit's changed lines, `✓ 13 passed`, or `✗ exit 1` with the cause. Reads fold into one line, and a divider closes each turn that did work (`6 files (3 new, 3 changed, +153 −15) · 9 commands (9 ok) · 2:41`).

**What asks, in the build hat.** The work runs: edits, your toolchains, scripts wherever they are, inline code (`python3 -c`, a heredoc to `node`), system programs (`mkdir`, `cp`, `mv`, `chmod`, `sed -i`), the project's containers, `git commit`, and a command Ryter has never heard of. A checkpoint before each build turn is what `/undo` comes back to. A question is kept for what no checkpoint undoes:

- **Deleting:** `rm`, `rmdir`, `truncate`, `find -delete`, `find -exec`, and the volumes of a stack (`docker compose down -v`, `docker volume rm`, any `prune`).
- **Throwing work away in git:** `git reset --hard`, `git clean`, `git checkout -- <path>`, `git restore`, `git stash drop`, `git branch -D`.
- **Leaving the machine:** `git push`, `cargo publish`, `npm publish`, `docker push`, a login; `curl` or `wget` sending a file or data to a host that isn't this machine; and a tool that works on a service somewhere else (`gh pr create`, `aws s3 sync`, `kubectl apply`, `terraform apply`, `fly deploy`, and their kind). Their looks run: `gh pr view`, `kubectl get`, `terraform plan`, and a download.
- **Writing anywhere but the project, scratch space or your home folder:** `/opt`, `/srv`, another disk, and a `cd` there followed by work. Those prompts say "outside the project" and offer only `y` (allow once) or `n`.
- **The project's `.env`:** the build hat may write or copy one into being, asked every time (see "Safety").
- **A path only the shell can read:** `cat "$FILE"`, files handed over by `xargs`, a `cd "$DIR"` and everything after it. The gate can't see where it leads. `$PWD`, `$(pwd)` and `$HOME` it reads.

Refused in every hat, whatever is answered: reading a secret or a credential folder, `sudo` and its kind, a shell handed a command as text (`bash -c`, `sh <<<`, a pipe into `sh`), and rewiring the shell (`alias`, `HOME=`, `IFS=`, a coprocess). A variable that gives a program something else to load (`LD_PRELOAD`, `PATH=/tmp:$PATH`, `NODE_OPTIONS='--require …'`, `RUSTC_WRAPPER`, `DOCKER_HOST`) asks in the build hat and is refused in the others. Ordinary variables run: `NODE_ENV=test`, `DATABASE_URL=…`, `RUST_BACKTRACE=1`, `PATH="$HOME/.cargo/bin:$PATH"`.

**Your own rules.** `[permissions]` in `~/.ryter/config.toml` moves any of the asks above, either way, short of a refusal. It is read from your own file only: a project's `.ryter/config.toml` can set its models and its budget, not what the gate asks about, so a repository can't widen the gate for itself.

```toml
[permissions]
edit = "allow"                      # or "ask": every edit of the project's files

[permissions.bash]
"rm -rf target" = "allow"           # a pattern over one command, as it runs
"rm -rf node_modules" = "allow"
"git push*" = "allow"
"docker compose down -v" = "allow"
"cargo publish*" = "deny"
"mv *" = "ask"
```

`*` is any run of characters and `?` one. A rule is matched against each command of a line (`cd src && mv a b` meets `"mv *"`), and the most specific pattern wins, the stricter answer at a tie. A rule can't open what the gate refuses: `"*" = "allow"` leaves `sudo` and `.env` where they were.

**Three answers for the whole session.** `/tools` shows them. The *tools* row of `/settings` sets the running session's answer too, and is the one the next TUI session starts with; `--always-approve` and `--yolo` win over it, and headless takes its answer from the flags only:

- **ask** (the default): the questions above are asked.
- **always** (`/tools always`, `--always-approve`): every question is answered yes in advance, except a write outside the project, scratch space and your home folder, and the project's `.env`, which still ask. Headless, those are refused.
- **yolo** (`/yolo`, `/tools yolo`, `--yolo`): every question is yes, those two included: the model writes anywhere it can reach, deletes, pushes, and makes a `.env`, without a word. What is refused stays refused: it still can't read your keys or run `sudo`. Choose this on a machine you'd be fine wiping, with a sandbox profile that fits.

A toolchain runs the project's code: `cargo build` runs its build script and `npm install` its install scripts. Inline code and a script in `/tmp` can say anything a script of the project's can. If you don't want that unasked, a sandbox profile limits what any command can touch (see "Sandbox profiles"): with the questions this few, the profile is the boundary, not the prompt.

**Docker or Podman.** When both are installed the model is told to use Docker, unless you ask for Podman. With one installed it is told which.

**How a command is read.** The gate judges what will run, not what was written. Before it decides, it does what the shell would do:

- **Patterns and lists are expanded,** and every file they match is judged: `cat .en?` is `cat .env`, and `cat ~/.s?h/id_rsa` is the key. A pattern in quotes is a pattern for the program (`find -name '*.py'`), and is left alone.
- **A `cd` moves where the rest is judged from.** `cd app && npm test` is judged in `app/`. Where a `cd` may not have run (after `||`, in a subshell, under `if`), the rest is judged from both places.
- **An option's value is a path like any other:** `--file=.env` and `-f.env` get the answer `.env` gets.
- **The shell's own words are not the program:** in `if …; then make; fi` the command is `make`.
- **A redirect is read wherever it is written** (`echo x>file`), and a backslash at the end of a line joins it to the next.
- **A link is judged by what it points at,** whatever the link is called.

- **A here-document is what the command reads,** not commands: the lines between `<<EOF` and `EOF` are skipped. `python3 - <<EOF` is inline code, judged as that.
- **A process substitution is a pipe:** in `diff <(git show HEAD:f) f` the inner command is judged on its own, and the pipe is nowhere on disk.
- **A shell function is the commands in it:** `add() { curl …; }; add a; add b` is judged by the `curl`. In the plan and audit hats a function is refused, since it makes a name mean other commands.
- **`cd -` goes back** to where the last `cd` of the same command left, and stays put when there was none: commands start without an `OLDPWD`.

What the shell is told to read another way, the gate can't read at all, and refuses in every hat: setting `HOME`, `IFS`, `CDPATH`, `GLOBIGNORE` or `BASH_ENV`; `shopt`, `alias`, `hash`, `trap` and `enable`; a named coprocess; and `env -C`, which runs a command in another folder.

What the gate can read but not see through (a path in a variable, files handed over by `xargs`) is a question in the build hat (and the audit hat behind its checkpoint) and refused in plan and scribe. A variable set earlier in the same command to a plain value is read where it is used, whether set as `F=/tmp/x` or `export F=/tmp/x`, so `rm -f "$F"` after either is judged as `rm -f /tmp/x` is. When a script of several commands asks, the card's title and its `what` row name the command that asked, and the body shows the whole script. "Allow all", `--always-approve` and `--yolo` answer that question yes in advance, as they do any other.

**Outside the project.** Scratch space (`/tmp`, `/var/tmp` and your system's temporary folder) is open to every hat, to read and to write, without a question. Your home folder, where tools keep their caches, configuration and builds, is open to every hat to read, and to the build hat to write.

With these exceptions:

- **Never read or written:** credentials (`~/.ryter`, `~/.ssh`, `~/.gnupg`, `~/.aws`, `~/.docker`, `~/.config/gh`, and the like), your tools' saved logins (`~/.npmrc`, `~/.pypirc`, `~/.cargo/credentials.toml`, `~/.git-credentials`), your shell history, your browser's and mail client's folders, and secret files (`.env`, `*.pem`, `*.key`) wherever they are.
- **Never written:** shell startup files (`~/.bashrc`, `~/.zshrc`, `~/.profile`, …) and system folders (`/etc`, `/usr`, …).
- **What something runs later asks each time,** in the build hat, and is refused in the others: a tool's own configuration in your home folder (`~/.gitconfig`, `~/.cargo/config.toml`, `~/.config/pip`, `~/.curlrc`), a folder of programs (`~/.local/bin`, `~/.cargo/bin`, any folder on your `PATH`), a program that is already there, and the files Python loads at every start. A line in one of those is a command the next `git status` or `cargo test` runs.
- **Another project:** a folder under your home that is a git repository other than this one can be read, but writing there asks each time in the build hat and is refused in the others.
- **Deleting or moving** anything outside the project asks each time, scratch space and your home folder included.

Anywhere else (`/opt`, `/srv`, another disk), the build hat asks each time and no other hat writes. Those prompts say "outside the project" and offer only `y` (allow once) or `n`. "Allow all" and `--always-approve` don't cover them, so headless refuses them; `--yolo` does.

The plan hat changes nothing in the project and writes nothing in your home folder. What it may write is scratch space: a place to keep a test's output. Not a file a tool would read as configuration on its way up from the project: for a project kept in `/tmp`, that rules out a `conftest.py` or a `.cargo/config.toml` beside it, while `/tmp/out.txt` is fine. The audit hat runs as the build hat does, and changes nothing by a checkpoint instead (see "Audit before you commit").

**A sandbox profile is stricter than this for your home folder.** With `workspace` or `read-only` chosen in `/settings`, `/tmp` is open as it is here, but the system itself shuts your home folder (beyond your tools and their caches) to every command, whatever the rules above allow. The model is told so, so a refusal isn't reported as a broken tool. To have your home folder open to commands, the profile has to be `off`.

**What the audit hat may run.** Behind a checkpoint, what the build hat may: `cargo test`, `npm test`, `pytest`, `cargo fmt`, `npm install`, a script in `/tmp`, `python3 -c`, `docker compose up` and `run`, `curl` against the product. Whatever it leaves changed is put back when its turn ends, and the audit says so. Git is the exception: a commit or a push is not undone by putting files back, so git stays read-only for it (`status`, `diff`, `log`, `stash list` and `show`). In a folder that is not a git repository there is no checkpoint; the audit is told, and for that turn it may run only what the gate can read as a check or a look: `cargo test`, `cargo clippy`, `npm test`, `pytest`, `ruff check`, `go test`, `make test` and the like, the project's own scripts, and a `GET` of the project's address; `cargo fmt`, `npm install`, a script outside the project and inline code are refused.

`xargs` in front of a command that prints files (`cat`, `grep`) is refused in the plan hat and in that read-only audit: the files it is handed can't be checked, and a secret could be among them. Search with `grep -rn` or `rg` and a folder.

**Commands the gate refuses in every hat:**
- the never-run list (`sudo`, `ssh`, `dd`, `mkfs`, `systemctl`, `crontab`, …), however it's wrapped: `env -i sudo`, `timeout 5 sudo`, `nice dd`, `xargs ssh`, `find -exec sudo`, `busybox rm`, `s\udo`;
- a command whose name comes from a variable or `$(…)`, and `eval`;
- inline code for an interpreter, however the flag is spelled (`python -c`, `python3 -bc`, `node -pe`, `perl -E`, `php -r`, `bash -lc`), standard input, here-strings and heredocs: write it to a file and run the file;
- a tool handed a command as text: `make --eval`, `go test -exec`, `cargo --config`, `npm exec -c`, `python3 -m timeit`, `awk` calling `system()`;
- a secret handed to a program that isn't just looking at the file: `cp .env notes.txt`, `tar cf x.tar .env`, `source .env`, a copy or an archive of a folder that holds one, a variable set to it (`x=.env; cat $x`). `ls -l .env`, `test -f .env`, and a container's or `node`'s `--env-file` still run;
- a secret printed by any road: a pattern that matches it (`cat .en?`), an option's value (`diff --from-file=.env`), a link to it, a search through a folder that holds it (`grep -r KEY .`; name the folders, say which files with `--include`, or use `rg`, which leaves hidden and ignored files out), into a linked folder too where the search follows links (`grep -R`, `rg -L`), `git` (`git diff --no-index /dev/null .env`, `git show HEAD:.env`; and in a repository that tracks a secret, a `git` command that prints files has to say which: `git grep KEY -- src`, `git diff -- src`), or a command in one of the project's containers (`docker compose exec web cat /app/.env`);
- `git -c` settings that name a program (`alias.x=!cmd`, `core.sshCommand`, `core.pager`, …), `git --exec-path`, and `--upload-pack`.

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

Headless, `ryter -p` runs in build; `--hat plan|audit` picks another. Headless nobody can approve an edit, so pass `--always-approve` to let build change files.

## Choosing models

**Every model is chosen in `/models`.** The seats are on the left, each with the model it runs on now: *All hats*, then *Plan*, *Build* and *Audit*. The models for the chosen seat are on the right.
- **Choosing a seat:** `↑↓` picks one. `→` or `⏎` moves to its models, and typing starts a filter there straight away.
- **Setting a model:** `⏎` sets the highlighted model for the seat and takes you back to the seats, on the next one. You can set every seat in one visit: pick, `⏎`, pick, `⏎`. A `✓` marks each seat you've set.
- **Other keys:** `←` goes back to the seats without setting anything. `Tab` steps the highlighted model's reasoning, `s` sorts, and `Esc` closes.

A model your account can't use (a data policy that refuses it, no tool support, no credits) fails on its first message, and the chat gives the provider's reason. Pick another in `/models`.

## Spend

Every model call is priced before the next request. Roll-ups: session, turn, hat, connection. Persisted in `spend.jsonl`.

Overrides must supply both `input_per_million` and `output_per_million`; all supplied rates must be finite and nonnegative. Missing or invalid rates remain unknown. Explicit zero is valid for a free model.

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

Cancelled or failed streams keep any reported usage and estimated cost as a lower bound. If the final accounting is missing, the session is marked incomplete. A configured budget then blocks further model calls, including commit-message drafting, even after resume or a model change. Start a separate session or explicitly use `/budget off` to continue with unknown spend.

A session budget is optional. With one, the turn stops when spend reaches it and says so (exit `3` in headless). Raise it and say continue. Without one, nothing stops on cost and you watch the spend card.

| Command | Effect |
| --- | --- |
| `/budget` | the budget panel: spend against the cap, on/off, the cap, and the warning level (`^s` saves) |
| `/budget 5` | cap this session at $5 |
| `/budget +2` | raise the cap by $2, e.g. after hitting it |
| `/budget off` | no cap |

The **budget** card on the right shows the cap, how much is used and left, or `off`; click it to open the panel. Changes apply at once and are saved as your default (`~/.ryter/settings.toml`, the same value as *budget usd* in `/settings`). A trusted project's `[spend] session_budget_usd` overrides your default in that project. There is no session budget until you set one. `[spend] enabled = false` still counts in memory and prints a warning.

**Project cost.** A project is its git repository (the folder, outside one), so sessions started in any subfolder count toward it. The instruments show `project` under the session total. When the project is a repository around the folder you started in, the `$` drawer names its project column for that folder. `p` in `/spend` switches to the project view: the total across sessions, this month, and breakdowns by hat, model, and month. A project that was worked on in crew mode, before it was removed, also shows what that cost. `ryter spend --project` prints the same. Nothing extra is recorded: every call is already in its session's `spend.jsonl`, and a running total in `~/.ryter/projects/` means only new lines are read. Calls with no known price are counted and shown (`$14.20+`), never added as $0.

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

Another agent can also spawn `ryter mcp serve` (stdio), or `ryter serve --socket /tmp/ryter.sock` or `ryter serve --bind 127.0.0.1:8765`. Tools: `ryter_prompt`, `ryter_status`, `ryter_spend`, `ryter_cancel`. Resources: `ryter://session/transcript`, `ryter://session/spend`. The transcript is a plain-text snapshot of the active conversation, refreshed after worker operations. During a running turn it shows the previous snapshot. It retains recent messages within 64 KiB, at most 4 KiB per message, with explicit omission notices. Stored provider credentials are not included; conversation content is shared with authorized MCP clients.

TCP requires `--token` (or `RYTER_MCP_TOKEN`) on `initialize.params.token`. Binding `0.0.0.0` / `::` requires `--i-mean-it`.

`ryter mcp serve` uses a restricted always-approve: workspace file edits, read, grep, tests; deny `rm -rf`, credential paths, work outside cwd. `--always-approve` only widens this if `[mcp] allow_dangerous = true`.

Status and spend remain available during a prompt; the CLI reports the last completed turn’s snapshot. A second MCP prompt receives a busy response while one is active. Closing the prompt connection cancels its pending work.

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
| UI | `[ui]` in `~/.ryter/config.toml`: `username`, `theme`, `reasoning`, `mouse`, `panel`, `colors`, `timestamps`, `line_numbers`, `receipts`, `open_pages`. All optional; unknown keys warn once at startup. See `config.example.toml`. |

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
- **The chat links every page**, and Ryter opens it in your browser when there's a desktop. `[ui] open_pages = false` (`/settings` → *pages in browser*) keeps pages closed, and you get the link only. Under `--sandbox`, Ryter leaves pages closed too; open the displayed link yourself.
- **A page loads nothing from the network:** no scripts, fonts or images from the web, and no form posts. Ryter puts a Content-Security-Policy at the very top of every page, ahead of anything the page contains. Everything the page shows is inline.
- **Hooks see these tools too.** `PreToolUse` and `PostToolUse` hooks run for `show_page`, `load_skill` and `request_hat`, just as for `bash` or `write`. A hook can deny a page.
- **The skill tells the model** to use only facts from the session, to say where they came from, and to make the page work in light and dark and at phone width.

Tool output is bounded while it is read. Shell commands retain the first and last output bytes and report how much was omitted. File reads support `offset` and `limit`; a line over 32 KB is shortened explicitly. Search skips lines over 64 KB and reports that its results may be incomplete. Whole-file edit/diff tools stop at 2 MB; use a focused project command for larger files. Directory listings show at most 1,000 sorted names, and prompt memory includes at most 128 Markdown note files within its 48 KB total cap.

## Context

`/context` shows the active hat’s estimated context use, including tool schemas and an output allowance of up to one quarter of its window (at most 32,768 tokens). Its window comes from `[context_windows]`, a cached provider catalog, or a matching saved route. Without that information, the existing fallback is 500k for `grok-4.6` and 200k otherwise; these are estimates. For a model or local server with a different limit, set its model ID under `[context_windows]` in your config, for example `"your-model-id" = 32768`.

At 85%, compaction keeps older user constraints and assistant notes in an extract, retains the last four user turns, and shortens older bulky tool results when needed. Tool identities and the newest result batch stay intact. `/compact` or `c` in the panel forces a pass; resume reads the rewritten conversation. If the resulting request still exceeds the window, Ryter stops before calling the provider. Choose a larger-window model or start a new session with the remaining task. Counts still use bytes/4 rather than the provider’s tokenizer.

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
| `/tmp` | read, write | read, write | read, write |
| Network | yes | yes | yes |
| Docker | yes | yes | yes |

**When to use each:**

- **`off`:** you are watching each step. Ryter's own rules still apply: destructive commands ask; supported toolchains and project commands can run without asking. The gate refuses direct reads of protected credentials. Nothing stops a command you approved from reaching the rest of your machine.
- **`workspace`:** tools run without asking (`/tools always`, `--always-approve`, `ryter serve`), or you are working on code you don't trust. Commands can write the project, scratch directories, allowed tool caches, and the active session’s notes and pages.
- **`read-only`:** you only want a review. Nothing in the project can be changed either, which also means nothing can be built into it.

**What "your tools" means.** Under `workspace` and `read-only`, commands can read and run:

- system folders (`/usr`, `/bin`, `/etc`), and where package managers install (`/opt`, `/nix`, `/snap`, Homebrew);
- toolchains under your home folder: `~/.cargo/bin`, `~/.rustup`, node version managers (`~/.nvm`, `~/.volta`, `fnm`, `asdf`, `mise`), `~/.pyenv`, `~/.bun`, `~/.deno`, `~/go/bin`, `~/.local/bin`, pipx and uv;
- any other folder on your `PATH` that is under your home folder, as that folder alone, excluding folders that would expose Ryter’s private storage;
- your git identity (`~/.gitconfig` and `~/.config/git/config`).

A tool folder with a symbolic link anywhere below your home folder is left out, since a grant on a link is a grant on what it points at. If your `~/.npm` or `~/.cargo/registry` is a link to another disk, builds under the sandbox can't use that cache.

They can also write the tools' download caches (`~/.cargo/registry`, `~/.npm`, pip's, uv's, Go's and others), so a build that fetches a dependency works.

**Scratch space is open:** `/tmp` and `/var/tmp`, to read and write, under both profiles. Scripts and tools name `/tmp` outright, and with it shut they failed with "Permission denied".

**What stays shut:** the rest of your home folder, `~/.ssh`, the tools' saved logins (`~/.cargo/credentials.toml`, `~/.npmrc`, `~/.config/git/credentials`), and Ryter's keys. Ryter also closes its own process to the commands it runs, so a key held in its memory or its environment can't be read from `/proc`.

**Session access follows the session:** tools and command hooks can write only the active session’s notes and pages inside Ryter’s home. Other sessions, transcripts, spending records, metadata and approvals stay closed. New and resumed sessions get fresh scopes. Ryter’s own bookkeeping runs outside those scopes, so saving records still works. Automatic Git operations and approved project start/test/stop commands use the same profile.

The configured Ryter home must be outside the workspace and outside shared system/scratch directories. A home under `/tmp`, for example, would be exposed by the scratch grant and is refused under a profile. Linked session, page or skill storage is refused. Choose a private home outside those locations or use `off`.

Plans, decisions and test reports stored in the project follow the workspace’s access rights. Run-file approvals and lifecycle ownership are kept separately in Ryter’s home.

**Git metadata must be reachable too.** For sandboxed Git workflows, launch Ryter from the repository root. A nested project whose Git metadata is outside the granted workspace may not have Git checkpoints or review available; the filesystem profile does not grant parent repositories automatically.

**What a sandbox doesn't do:**

- Separately configured outbound MCP servers run with their own permissions; this profile applies to Ryter’s built-in commands.

- **It doesn't limit the network.**
- **It doesn't contain Docker.** A command that can reach the Docker socket can mount the whole machine. If that matters, don't give the account Docker access. `docker build` works under a profile (its lock folder, `~/.docker/buildx`, is writable; the registry logins beside it stay shut).
- **Rootless Podman can't run under it.** Podman keeps its state in your home folder and starts containers in a user namespace of its own, and a sandboxed command can do neither. Use Docker, or the `off` profile, for a project that needs Podman.
- **It needs Linux.** Elsewhere Ryter refuses to start with `workspace` or `read-only`, and it also refuses on a Linux kernel that can't enforce Landlock. On a kernel older than 5.19 a profile works, but a file can't be moved from one folder to another under it, so some builds fail (`cargo` building a library, for one).
- **Some of Ryter's own features are off under it:** your every-project rules can't be changed, and pages aren't opened in a browser.


## Safety

- One gate: `decide(hat, tool, args)` → Allow / Ask / Deny. Every hat is offered the same tools; the gate decides what each may do with them.
- **Build:** edits, toolchains, scripts, inline code, the project's containers and ordinary git run without asking; deleting, discarding work in git, publishing and leaving the machine ask; `[permissions]` moves any of those, short of a refusal. `/yolo` answers every question yes.
- **Plan:** reading and read-only commands; it may write the project's memory files and its own notes, nothing else.
- **Audit:** what the build hat runs, behind a checkpoint that puts the tree back when the turn ends; git stays read-only (`git stash list` and `show` included); its only files are `.ryter/audit.md` and the dated copies. Without a git repository there is no checkpoint, and it is held to reading, tests and linters, as the review hat was.
- Denied in every hat: `.env`, `*.pem`, `*credential*`, `~/.ssh`, Ryter credential files. An example file (`.env.example`, `.env.sample`) is not a secret. One exception: the build hat may **write** the project's own `.env` (or `.env.local`, `config/.env.production`, `local.env`) whole, or copy it from its example (`cp .env.example .env`), with your `y` each time, since a project that needs one can't run without it. The card shows what would be written; nothing reads it back, and an edit in place stays refused, as does every other hat. An approved plan, `a` and `--always-approve` don't cover it; headless refuses it.
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

If an interrupted append leaves a torn final record, resume keeps the valid history and reports the path of an exact backup. Complete or middle-of-file corruption stops resume for deliberate recovery. Recovered or inconsistent spending remains marked incomplete, so an enabled budget stops further requests.

```
~/.ryter/sessions/<cwd-slug>/<id>/
  meta.json
  events.jsonl
  transcript.jsonl # the conversation
  spend.jsonl
  notes/           # the plan hat's notes, and project.log: the output of a product run_project started
```

A session saved in crew mode, before it was removed, still opens: its conversation and its spend are there, and it carries on in the build hat.

No SQLite. `ryter spend` uses the latest session for this directory.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | ok |
| 1 | error (including not a tty without `-p`) |
| 3 | spend budget exceeded |

## Repeatable acceptance checks

From a source checkout, `cargo test --workspace` includes a complete simulated
plan → build → audit flow with approval, resume, undo/redo and commit
receipts. After building the CLI, `python3 scripts/acceptance.py` checks real CLI
processes against an isolated loopback provider, including spending, interrupted
streams, recovery and permission refusals. These checks spend no provider credit.

`python3 bench/run.py --mode simulated` exercises all ten retained benchmark
fixtures. See [the benchmark guide](../bench/README.md) for reference validation,
false-pass controls, saved reports and explicitly budgeted live runs. A model's
review or test pass is its reported verdict; it does not prove the hidden tests
will pass.

For headless runs, explicit `--model` or `--connection` flags override saved
hat-specific routes for that run. They do not rewrite those saved hat defaults.

## Dependency maintenance

`cargo deny --locked check advisories licenses sources` checks the shipped targets using `deny.toml` (cargo-deny 0.20.2). CI runs this on a free public Linux runner. The lockfile no longer uses the yanked `yoke-derive` release, and syntax highlighting no longer enables the unused YAML/plist loaders.

Two specific maintenance advisories remain acknowledged in the configuration: `bincode` through syntect’s bundled syntax assets, and the build-time `paste` macro through ratatui 0.29. These are unmaintained-dependency notices, not vulnerability exceptions. New vulnerability advisories fail the check. Replacing them requires an upstream serialization change and a separate terminal-library migration.

The license policy checks declared licenses and source registries. Release archives include a `third-party` folder with corresponding crate-source links and available license/copyright notices, generated by `python3 scripts/license_bundle.py OUTPUT`. The `option-ext` MPL-2.0 allowance is specific to that unmodified transitive dependency.
