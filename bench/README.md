# Benchmark suite

`ryter bench` runs each task here through the real crew and reports what
landed, what passed the hidden tests, and what it cost. See `docs/cost.md`.

Each task:

```
<task>/task.toml   title, brief, files (scope), checks (the gate), accept (hidden)
<task>/repo/       the fixture the crew works in
<task>/hidden/     acceptance tests, copied in only after the work lands
<task>/solution/   a reference answer; never shown to the crew
```

A test (`bench::tests::the_starter_suite_is_sound`) proves every task is sound:
the fixture fails its visible check where the task is a fix, and the reference
solution passes both the visible checks and the hidden tests. Add a task by
copying one and keeping that test green.

Visible checks are deliberately weaker than the hidden tests. A crew that
passes the checks and the auditor but fails the hidden tests is a **false
pass** — the number that says how far the auditor can be trusted.
