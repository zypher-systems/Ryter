# Benchmark suite

`ryter bench` runs each task here through the real crew and reports what
landed, what passed the hidden tests, and what it cost. The last published run
is `docs/bench.md`; `ryter bench --publish docs/bench` replaces it and says how
the new run compares. See the guide's Benchmark section, and `docs/cost.md`.

Each task:

```
<task>/task.toml   title, brief, files (scope), checks (the gate), accept (hidden), needs
<task>/repo/       the fixture the crew works in
<task>/hidden/     acceptance tests, copied in only after the work lands
<task>/solution/   a reference answer; never shown to the crew
```

`needs` lists commands that must succeed for the task to run on a machine, such
as `cargo --version`. A task whose tools are missing is skipped, by name.

A task can be split into several builder tasks, the way an architect's plan
splits work. Tasks with no `after` run side by side:

```toml
[[tasks]]
id = "parse"
title = "Parse name,value lines"
brief = "..."
files = ["stats/parse.py"]

[[tasks]]
id = "report"
title = "Render the report"
brief = "..."
files = ["stats/report.py"]
after = ["parse"]
```

The gate's `checks` run at every stage of a split task, so they can only cover
what exists at every stage. The hidden tests cover the whole.

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
| `split-stats-report` | Python | three builder tasks: two side by side, one waiting on both |

A test (`bench::tests::the_suite_is_sound`) proves every task is sound: the
hidden tests fail on the fixture, and the reference solution passes both the
visible checks and the hidden tests. Add a task by copying one and keeping that
test green.

Visible checks are deliberately weaker than the hidden tests. A crew that
passes the checks and the auditor but fails the hidden tests is a **false
pass**: the number that says how far the auditor can be trusted.
