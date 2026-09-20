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

Credential order per connection: TOML `api_key` → `env_key` → OS keyring (`service=ryter`, `account=connection:<name>`) → well-known env.

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

`ryter` on a tty opens the TUI. The header is one line: phase, connection/model, spend. There is no outer box. Type a message, or `/` for slash commands.

`/resume` lists sessions for this directory (newest first). `/rename <title>` names the current one. `/delete` removes a session. `/agents` lists running specialists; Enter kills that one. `ryter sessions` and `ryter resume [id]` are the CLI equivalents.

Without a tty, use headless:

```sh
ryter -p "add a --json flag" --always-approve
ryter -p "…" --json                  # NDJSON AgentEvent stream
ryter --mode plan -p "what should we build?"
```

`--always-approve` treats Ask as Allow. Deny still wins.

## Orchestrator, phases, handoff

The orchestrator prompt is `prompts/orchestrator.md` (overridable). It may read the repo, grep, glob, and call `todo_write`. It cannot write product source.

**Phase** only changes which specialist *kinds* may run. Default is **Build**.

| Command | Phase | Specialists |
| --- | --- | --- |
| `/plan` | Plan | planners |
| `/architect` | Architect | architects |
| `/build` | Build | builders + auditor gate |
| `/audit` | Audit | extra reviewers |

`/handoff architect` (or `ryter handoff architect --note "…"`) writes a pass note for the current phase and switches. Empty notes are allowed. `/handoff back` goes to the previous phase. The orchestrator transcript is not cleared. Specialists get a **fresh window**: pass note + task + `RYTER.md` / `AGENTS.md`, not the chat history.

Project markdown is loaded from the working tree without a trust gate: `RYTER.md`, or `AGENTS.md` if `RYTER.md` is absent.

## Build workers, auditor, merge

`todo_write` **is** the work queue. After an orchestrator turn with no remaining tool calls, Ryter drains pending tasks for the **current phase**, up to `[subagents] max` in parallel (must be ≥ 1). Plan → planners, Architect → architects, Build → builders + auditor, Audit → extra auditors.

Crew roles default to the orchestrator’s current provider and model. Assign a different model per role with `/crew` (Enter on a role, first picker row is `default`). That is also how you split providers. Optional `[specialists.*]` tables in `~/.ryter/config.toml` pin the same overrides.

Each builder:

1. `git worktree add` under `~/.ryter/worktrees/<session>/<task>/` on branch `ryter-<8hex>-<slug>`
2. Implements the task with a builder prompt and a builder tool mask
3. Commits
4. Auditor (if enabled) sees the diff and replies `PASS` or `FAIL`
5. Pass → merge into the session branch (rebase once on conflict; if that still fails, one extra builder turn in the worktree, then `blocked`). Fail → retry up to `[auditor] max_retries`, then `blocked` with no merge
6. Worktree is removed

`/auditor on|off` is session-only unless you also change config. Status line shows the gate. `/auditor off` merges without review.

Planners and architects run in-process (no worktree). Nested subagents are not supported.

## Spend

Every model call is priced before the next request. Roll-ups: session, turn, role, connection. Persisted in `spend.jsonl`.

Sources, high wins: TOML `[pricing."<model>"]` → OpenRouter catalog (when ingested) → shipped SpaceXAI table. Provider-reported cost on a stream wins for that turn.

Unknown rates show `$?.??` plus token counts. Ryter never invents `$0.00` for an unpriced model.

`[spend] session_budget_usd` stops the loop (exit `3` in headless). `0` means no cap. `[spend] enabled = false` still counts in memory and prints a warning.

`/spend` is a floating table (session total, by role, by provider). `ryter spend` prints the same roll-up on the CLI.

## Slash commands

Built-ins: `/quit` `/new` `/resume` `/rename` `/delete` `/agents` `/spend` `/settings` `/provider` `/models` `/crew` `/handoff` `/plan` `/architect` `/build` `/audit` `/auditor` `/mcp` `/skills` `/hooks` `/theme` `/context` `/compact` `/doctor` `/cancel` `/help`.

User-invocable skills and `~/.ryter/commands/*.md` join the palette. Built-ins win on a name clash.

## MCP

**Outbound.** `[mcp_servers.<name>]` stdio children. The orchestrator discovers with `search_tool` and calls with `use_tool`. Child env does not inherit API keys unless that server’s `env` table asks. In the TUI, `/mcp` is a floating menu: add a server (name → command → args), Enter toggles it, Backspace removes it.

**Inbound.** `/mcp` → inbound shows the links a client needs:

- stdio: `ryter mcp serve`
- unix attach (this TUI): `unix:///…/ryter.sock`
- TCP: `127.0.0.1:8765` plus a bearer token (`initialize.params.token`)
- snippet: `{"mcpServers":{"ryter":{"command":"ryter","args":["mcp","serve"]}}}`

Enter on **token** creates or rotates a `ryt_…` secret stored in `~/.ryter/keys/mcp-inbound.toml` (mode 0600). Enter on a link copies it into the chat so you can paste it. Live flags persist in `~/.ryter/mcp.toml` (does not rewrite `config.toml`).

Another agent can also spawn `ryter mcp serve` (stdio), or `ryter serve --socket` / `--bind`. Tools: `ryter_prompt`, `ryter_status`, `ryter_spend`, `ryter_set_phase`, `ryter_cancel`. Resources: `ryter://session/transcript`, `ryter://session/spend`. Keys are never returned.

TCP requires `--token` (or `RYTER_MCP_TOKEN`) on `initialize.params.token`. Binding `0.0.0.0` / `::` requires `--i-mean-it`.

`ryter mcp serve` uses a restricted always-approve: workspace file edits, read, grep, tests; deny `rm -rf`, credential paths, work outside cwd. `--always-approve` only widens this if `[mcp] allow_dangerous = true`.

`[mcp] inbound = false` disables the server. Esc or `/cancel` (or `ryter_cancel`) stops the in-flight turn and kills bash process groups.

## Customization

| Kind | Where |
| --- | --- |
| Prompts | `prompts/*.md`; override `~/.ryter/prompts/` then trusted `.ryter/prompts/` |
| Skills | `/skills` floating list. Files: `~/.ryter/skills/<name>/SKILL.md` (frontmatter `user-invocable`). Enter runs (optional args). Add writes a stub you edit. Backspace deletes a user skill (not a project overlay). |
| User slash | Same `/skills` list (`command` rows). `~/.ryter/commands/<name>.md` (`$ARGUMENTS`) |
| Hooks | `/hooks` floating list. Add: event → command or URL → optional matcher. Backspace removes. Live list is `~/.ryter/hooks.toml` (does not rewrite `config.toml`). Command gets JSON on stdin; exit 2 or HTTP 403 denies. |
| Themes | `/theme default-16` or `dark`, or `~/.ryter/themes/<name>.toml` |

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

`/context` prints estimated tokens vs the model window (500k for `grok-4.6`, 200k otherwise). Auto-compact at 85%: older turns collapse to tools used, files touched, and the latest pass note; the last four user turns stay. `/compact` forces a pass. Resume reads the rewritten `transcript.jsonl`.

## Doctor and sandbox

`ryter doctor` (and `/doctor`) checks OS, tty, home, config, both built-in connections (key set/missing, never printed), spend catalog, git, Landlock, sandbox profile, and whether `.ryter/` is trusted. No network.

`--sandbox workspace` Landlock-restricts the tool thread to the project tree (writable) plus `~/.ryter`. `--sandbox read-only` makes the project tree read-only. `--sandbox off` is the default. A non-off profile **refuses to start** if the kernel cannot enforce Landlock. `/tmp` itself is not granted; scratch is `~/.ryter/tmp`. Sandboxed runs use a current-thread tokio runtime.

## Safety

- One gate: `decide(role, tool, args)` → Allow / Ask / Deny. Role masks omit tools the model should not see.
- Orchestrator: read, list, grep, glob, `todo_write`, MCP. Cannot write `src/`.
- Planner / architect: notes + read tools.
- Builder: full tool set in its worktree.
- Auditor: read + test/lint bash.
- Denied even for builders: `.env`, `*.pem`, `*credential*`, `~/.ssh`, Ryter credential files.
- Destructive bash is Ask. In the TUI: `y` allow this call, `n` deny, `a` allow for the rest of the session. Headless (no TUI) fail-closes.
- `ask_user` lets the orchestrator ask a question (choices or free text).
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

No SQLite. `ryter spend` / `ryter handoff` use the latest session for this directory.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | ok |
| 1 | error (including not a tty without `-p`) |
| 3 | spend budget exceeded |
