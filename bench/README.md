# Benchmark tasks

The runner validates every selected reference solution and confirms that its
unchanged fixture fails hidden acceptance before any provider call. It uses fresh
repositories and isolated homes for simulated runs, and writes reports and logs
under ignored `target/` directories.

```sh
cargo build --workspace --locked
python3 scripts/acceptance.py
python3 bench/run.py --mode reference
python3 bench/run.py --mode simulated
# Intentional broken builder: verifies false-pass detection.
python3 bench/run.py --mode simulated --simulate-broken-build --task pager-off-by-one
```

The simulated provider scripts plan, build and audit turns through the real CLI,
permission gate, filesystem tools, session persistence and spending code. The
runner still passes `--hat review` for the third phase: that name selects the
audit hat. Its builder writes reference answers; its verdicts and charges are
synthetic. Those scores validate the harness, not a model's ability. The
simulated auditor runs the project's checks through the run file the harness
provides (`run_project`, action `test`), as the audit hat may. The
broken-builder control leaves the fixture unchanged while the simulated auditor
claims it passed; the hidden checks must identify that false pass. One task
shows the audit hat's limit: `runner-script`'s checks are a script of the
project's own (`sh dev lint`), and headless `--always-approve` goes only as far
as the audit hat's gate, which refuses that; its `checks_pass` is false while
the scripted verdict still says PASS. In the TUI the user's approval of the run
file goes further.

Live runs are opt-in and use the app's existing configured connection and key.
The runner never copies credentials. It explicitly selects the model for every
hat and uses `workspace` sandboxing, so live mode requires Linux/Landlock and a
private Ryter home outside shared scratch. Use only a budget the user approved:

```sh
python3 bench/run.py --mode live --connection openrouter \
  --model qwen/qwen3-coder-flash --budget 5.00
```

Live mode writes ordinary session/spending records and remembers its last route
in the configured Ryter home. It trusts only its newly generated fixture projects
for their budget and run-file configuration. Each session gets at most $0.35 of
the total allowance, with a $0.10 reserve for the last in-flight request. It stops
on incomplete accounting. This is an application-side budget, not an OpenRouter
account credit limit. The model and conservative fallback rates are pinned in the
runner; review current provider pricing before changing them. Reported provider
costs take precedence. No CI job runs live mode.

Plan replies are headless proposals, followed by the harness's explicit build
instruction. Headless `--always-approve` covers the fixture's run file within the
audit hat's policy. The complete interactive approval protocol is separately
covered by `agent::tests::acceptance`, including plan approval, an audit, approved
run commands, resume, undo/redo, later user edits and commit receipts.

`--task NAME` selects tasks; `--output PATH` chooses a new report directory.
`report.json` records source hashes, prerequisites, baseline/reference results,
per-hat outcomes, hidden acceptance, false review passes and recorded cost.
Hidden tests are copied into the live project only after all hats finish. Inspect
saved phase events to distinguish expected negative checks from harness errors.

Each task:

```
<task>/task.toml   title, brief, files (scope), checks (the gate), accept (hidden), needs
<task>/repo/       the fixture the work is done in
<task>/hidden/     acceptance tests, copied in only after the work is done
<task>/solution/   a reference answer; never shown to the model
```

`needs` lists commands that must succeed for the task to run on a machine, such
as `cargo --version`.

| Task | Level | Language | What it asks for |
|---|---|---|---|
| `cli-limit-flag` | standard | Python | a new flag on a small CLI |
| `pager-off-by-one` | standard | Python | a one-function bug fix |
| `rename-across-files` | standard | Python | a rename and its callers |
| `slugify` | standard | Python | a new function with edge cases |
| `inventory-reorder` | standard | Python | a feature across four modules, one of them new |
| `settings-overrides` | standard | Python | bugs in two modules that only show together |
| `rust-durations` | standard | Rust | a parser, its errors and a formatter, across three modules |
| `ts-event-bus` | standard | TypeScript | new behaviour in two modules, run by Node with no build |
| `split-stats-report` | standard | Python | work split into three steps, two of them independent |
| `runner-script` | standard | Python | a feature in a project whose tests run only through its own script |
| `expr-precedence` | hard | Python | a parser with four precedence levels, a right-associative power, unary minus and error positions |
| `migrate-dependencies` | hard | Python | dependency order with a stable tie-break, cycle reporting, a run that survives a failure, atomic state |
| `rust-csv-borrow` | hard | Rust | a quoted-field CSV reader whose fields borrow from the input unless they must not |
| `ts-debounce` | hard | TypeScript | debounce with leading, trailing and maxWait against an injected clock, and throttle on it |

A task's level is `level` in its `task.toml`; a task that says none is
standard. `--level standard` or `--level hard` picks one level on either
runner; without it every task runs. The standard tasks are the ones a current
model solves; the hard ones each turn on a rule the brief states and the
visible tests do not check, the kind of thing the published runs showed a
model getting wrong (`ts-event-bus`, where every Mistral run broke `off` and
emitted the wildcard twice): a parser's precedence and where an error points,
an order that must be the same on every run and a run that must leave the
state file consistent after a failure, a reader that must not copy, and the
timing of leading, trailing and maxWait calls.

Visible checks are deliberately weaker than the hidden tests. Work that passes
the checks and its review but fails the hidden tests is a **false pass**: the
number that says how far a review can be trusted.

## Running the bench from the binary

The fixtures (each task's `task.toml`, `repo/` and `hidden/`, never a solution)
are embedded in `ryter` at build time (`crates/ryter-bench`), so a user runs the
same tasks on their own model and key without a checkout:

```sh
ryter bench --list                               # the tasks and their levels
ryter bench --level hard --budget 5              # the hard tasks on the default route
ryter bench --task slugify --connection anthropic -m claude-sonnet-4-5 --budget 1
```

`/benchmark` in the TUI is the same run from a form: connection, models (picked
from the connection's list, with context and prices), level, budget and
sandbox, with the tasks ticking through in a panel and the headline in the chat.
Several models (`-m` repeated from the shell, `-m connection=model` for one on
another connection) run in turn under the same budget each and end in a
comparison, a line per model, most solved first then cheapest, and the tasks
against the models: the way to pick the cheapest model with a clean sheet for
a job. Either way each task runs in a fresh copy under the build hat as
its own `ryter` process (`--json --hat build --always-approve`, the user's
environment for its login, a per-task cap in the copy's `.ryter/config.toml`,
`--no-remember` so the run leaves the user's default connection and last model
alone), and is scored by its hidden tests in the bare environment described
below, under the same sandbox as the build hat had: the tests import and build
what the model wrote. The hat runs under a leader process of Ryter's own
(`ryter bench-lead`), which on Linux is a child subreaper: a process of the
hat's whose parent exits is reparented to the leader rather than to `init`, so
the hat's tree is whole however a process detaches (`setsid`, a scrubbed
environment, a daemon's double fork), and when the hat ends, or the runner
ends it, the leader stops and kills everything under itself before it exits.
A task whose tree could not be emptied is not scored. The scoring is done on
a copy of the model's tree made only then (build output and `.git` left out).
On macOS there is no subreaper: the leader ends what is still in the tree,
and a process whose parent has already exited is not found; with the sandbox
`off` (the default there) such a process could read the copy the hidden
tests go into. The hidden tests travel in the binary deflated and
masked, so the binary, which the sandbox lets a model read, does not show them
to `grep`; a cover, not a secret. The run goes to
`~/.ryter/bench/<date-time>-<model>/` (`-2`, `-3` when the minute's name is
taken): `report.json`, `RESULTS.md`, and per task the prompt, the build hat's
events and stderr; the project copies are removed once scored. The budget, the
reserve and the per-task cap work as in the comparison runner; `--sandbox` is
the build hat's (`workspace` on a Linux whose kernel has Landlock, `off`
elsewhere). Ctrl-C, or quitting the TUI, ends the task in progress with
everything it started and skips the rest; a task cut short, in the model's
turn or in its scoring, is not scored, and what it cost still counts. The
command exits 1 whenever the run did not go to its end. The tool comparison
stays here in `bench/compare.py`, since it depends on which other tools are
installed and how each reports its cost.

## Comparing with other tools: tasks solved per dollar

`bench/compare.py` gives the same tasks to Ryter and to the other coding agents
installed on the machine (Claude Code, Codex, OpenCode, Kilo), each in a fresh copy
of the task's project with the same brief, and scores every one by the hidden
tests. The number that comes out is tasks solved per dollar, with the cost per
solved task, the median minutes and the time-outs beside it.

```sh
python3 bench/compare.py --run ryter=anthropic:claude-sonnet-4-5 \
    --run claude=claude-sonnet-4-5 --run opencode=anthropic/claude-sonnet-4-5 \
    --budget 10 --dry-run                       # validate, print the commands, call nothing
python3 bench/compare.py --run ryter=openai:gpt-5.2 --run codex=gpt-5.2 \
    --price gpt-5.2=1.75,0.175,14 --budget 10   # the run
```

- `--run tool=model`, once per tool and model; `ryter=connection:model` names
  the Ryter connection. The same model on every tool is the fair comparison;
  the runner does not check that, the table shows what ran. Two models of
  one tool are two runs, each with its own column and directory.
- Every tool runs as you have it set up: its own login, its own settings and
  hooks. Nothing is copied between tools, and no key is read. Ryter uses the
  connection's key from your Ryter home. The scoring does not: the fixture's
  git and the hidden tests run under the same bare environment the
  reference solution was proven in (a private `HOME`, no git config, an
  offline Cargo), so nothing in your shell can pass or fail a tree for a tool.
- Each tool is run so that it asks nobody anything: Ryter with
  `--always-approve` under the `workspace` sandbox (Linux), Claude Code with
  `--dangerously-skip-permissions`, Codex with `--sandbox workspace-write`,
  OpenCode and Kilo with `--auto`. The project is a throwaway copy under `target/`.
- The prompt is one scope paragraph (work only here, no installing, no
  network, no commit) and the task's brief, which names the check to pass.
  Ryter also gets the project's run file with that check as `test` and
  `check`; the others are told the command in the brief.
- Cost is what the tool reports: Ryter's spend events, Claude Code's
  `total_cost_usd`, OpenCode's and Kilo's cost per step. Claude Code's figure is its own
  estimate at list prices, whatever the login, and the table marks it so (†):
  on a subscription nothing is billed per token. Codex reports tokens only, so a
  Codex run needs `--price model=input,cached,output` (dollars per million)
  and its cost is marked as priced from that table; OpenCode falls back to the
  table when it reports no cost (a subscription login). A task a tool could
  not price stops the run, as incomplete accounting does in live mode.
- `--budget` is dollars for the whole run, all tools together, from $0.11 to
  50. The runner stops before a task when what is left, past the reserve
  ($0.10), is under a cent; where a tool has a cap of its own (Ryter's
  session budget, Claude Code's `--max-budget-usd`) it gets what is left.
  Codex and OpenCode have none, so the per-task `--timeout` (600 s) is their
  only bound.
- One attempt per task, tasks in order, every tool on a task before the next
  task, so a stopped run compares the same tasks. `--level hard` (or
  `standard`) runs one level; `--task` names tasks.
- The output directory holds `report.json`, `RESULTS.md` (the table), and per
  task and run (`<task>/<tool>-<model>`) the prompt, the tool's output, stderr
  and the project as it was left. `--into DIR` runs the given tools again
  into an earlier report, replacing their results and spend once every
  fixture has been proven again; `--render DIR` rewrites its table from
  `report.json` without running anything. The published runs are under
  `docs/bench/`.

No CI job runs this, and the runner refuses to start a paid task on a fixture
whose reference solution does not pass or whose unchanged project does. The
solved tree it proves that with is removed before any tool starts. The runner
needs Python 3.12 or newer.

## Recorded live result

See [the October 2 acceptance report](../docs/acceptance-2026-10-02.md), made on
0.12.0 with four hats: six of ten implementations passed hidden checks, five
completed with both verification hats passing, and three incorrect
implementations received false passes from each hat. The authorized $5 run
recorded $0.418755831. The benchmark still runs three phases: plan, build and
audit. The scribe is not a phase. The audit runs the checks; `flow_completed`
and the false-pass count are the audit's alone. No live calls run in CI.

`completed` measures hidden acceptance; `flow_completed` additionally requires all
three phases to exit successfully and the review to pass. Explicit
Markdown verdicts are parsed like the application. Recalculate an existing report
without executing commands or calling a provider with
`python3 bench/run.py --rescore PATH/report.json --output NEW_REPORT.json`.
