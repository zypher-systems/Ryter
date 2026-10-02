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

The simulated provider scripts plan/build/review/test turns through the real CLI,
permission gate, filesystem tools, session persistence and spending code. Its
builder writes reference answers; its verdicts and charges are synthetic. Those
scores validate the harness, not a model's ability. The broken-builder control
leaves the fixture unchanged while simulated reviewers/testers claim it passed;
the hidden checks must identify those false passes. The `runner-script` review
command is refused by the review hat; its approved Test-hat command does run.

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
Test hat's policy. The complete interactive approval protocol is separately
covered by `agent::tests::acceptance`, including plan approval, review, approved
run commands, resume, undo/redo, later user edits and commit receipts.

`--task NAME` selects tasks; `--output PATH` chooses a new report directory.
`report.json` records source hashes, prerequisites, baseline/reference results,
per-hat outcomes, hidden acceptance, false review/test passes and recorded cost.
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

| Task | Language | What it asks for |
|---|---|---|
| `cli-limit-flag` | Python | a new flag on a small CLI |
| `pager-off-by-one` | Python | a one-function bug fix |
| `rename-across-files` | Python | a rename and its callers |
| `slugify` | Python | a new function with edge cases |
| `inventory-reorder` | Python | a feature across four modules, one of them new |
| `settings-overrides` | Python | bugs in two modules that only show together |
| `rust-durations` | Rust | a parser, its errors and a formatter, across three modules |
| `ts-event-bus` | TypeScript | new behaviour in two modules, run by Node with no build |
| `split-stats-report` | Python | work split into three steps, two of them independent |
| `runner-script` | Python | a feature in a project whose tests run only through its own script |

Visible checks are deliberately weaker than the hidden tests. Work that passes
the checks and its review but fails the hidden tests is a **false pass**: the
number that says how far a review can be trusted.

## Recorded live result

See [the October 2 acceptance report](../docs/acceptance-2026-10-02.md): six of ten
implementations passed hidden checks, five completed with both verification hats
passing, and three incorrect implementations received false passes from each hat.
The authorized $5 run recorded $0.418755831. No live calls run in CI.

`completed` measures hidden acceptance; `flow_completed` additionally requires all
four phases to exit successfully and both verification hats to pass. Explicit
Markdown verdicts are parsed like the application. Recalculate an existing report
without executing commands or calling a provider with
`python3 bench/run.py --rescore PATH/report.json --output NEW_REPORT.json`.
