# Benchmark tasks

These ten tasks were the suite for `ryter bench`, which ran each one through
crew mode and reported what landed, what passed the hidden tests, and what it
cost. Crew mode was removed, and `ryter bench` went with it. The tasks are
kept for a benchmark that runs the plan, build and review hats on them, which
is on the roadmap. **Nothing runs them today**, and no test checks that they
are still sound.

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
