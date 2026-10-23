# Tasks solved per dollar: Mistral Large 4 on Ryter, OpenCode and Kilo (2026-10-22)

The ten bench tasks, given to three tools on the same model,
`mistralai/mistral-large-4-0`, each through OpenRouter, each in a fresh copy
of the same project with the same brief, scored by the hidden tests. Made with
`bench/compare.py` as described in [`bench/README.md`](../../bench/README.md);
the table below is the run's `RESULTS.md` as written.

Every tool solved nine of the ten and failed the same one. Ryter did its nine
for $0.30, Kilo for $0.41, OpenCode for $0.52.

## What was compared

- **The same model by the same route.** Ryter's `openrouter` connection,
  OpenCode's and Kilo's `openrouter` provider, each with the user's OpenRouter
  key, so every cost is what OpenRouter charged. No estimate row this time.
- **One prompt per task, no questions.** The scope paragraph and the task's
  brief, as in the [Opus 5.5 run](2026-10-22-opus-5-5.md). Ryter ran its build
  hat with `--always-approve` under the `workspace` sandbox and had the
  project's run file with the check; OpenCode and Kilo ran with `--auto` and
  were told the command in the brief.
- **Each tool as installed**, with its own settings. Kilo's prompts were the
  largest: 84,000 to 270,000 cached tokens per task against OpenCode's 41,000
  to 170,000 and Ryter's 27,000 to 106,000. Cached reads are cheap, which is
  why Kilo still came in under OpenCode.
- **Ten tasks, one attempt each, 600 seconds allowed.** None timed out; the
  longest was OpenCode's four and a half minutes on the task all three failed.

## Observations

- **The same failure three times.** On `ts-event-bus` every tool's work passed
  the visible test and failed the hidden one on `off` no longer removing a
  handler, a `once` handler included; Ryter's and Kilo's also called the
  wildcard's handlers twice when emitting the wildcard itself. The brief says
  existing behaviour must not change. The same two faults appeared in the
  October 2 run of a smaller model. This is the model, not the tools: it
  rewrites the bus and drops what was there.
- OpenCode spent $0.18 and 22,000 output tokens on that failing task, four
  times the other two; it kept trying.
- Ryter edited the visible Rust test file on `rust-durations`
  (`tests/basic.rs`) besides the source; the hidden tests, which it never saw,
  passed. OpenCode and Kilo left it alone.
- Nothing asked a question, nothing was committed.

## The run


Ryter `ea3aadb`; tools as installed: ryter (ryter 0.37.0), opencode (opencode v2.0.28), kilo (7.8.12).
Budget $10.00, spent $1.233; 600 s per task; one attempt per task.

| Run | Solved | Cost | Cost per solved | Solved per $ | Median minutes | Timed out |
|---|---|---|---|---|---|---|
| `ryter=openrouter:mistralai/mistral-large-4-0` | 9/10 | $0.302 | $0.034 | 29.8 | 0.7 | 0 |
| `opencode=openrouter/mistralai/mistral-large-4-0` | 9/10 | $0.523 | $0.058 | 17.2 | 0.7 | 0 |
| `kilo=openrouter/mistralai/mistral-large-4-0` | 9/10 | $0.408 | $0.045 | 22.0 | 0.7 | 0 |

| Task | `ryter=openrouter:mistralai/mistral-large-4-0` | `opencode=openrouter/mistralai/mistral-large-4-0` | `kilo=openrouter/mistralai/mistral-large-4-0` |
|---|---|---|---|
| cli-limit-flag | ✓ $0.020 | ✓ $0.032 | ✓ $0.048 |
| inventory-reorder | ✓ $0.021 | ✓ $0.042 | ✓ $0.032 |
| pager-off-by-one | ✓ $0.011 | ✓ $0.023 | ✓ $0.017 |
| rename-across-files | ✓ $0.009 | ✓ $0.021 | ✓ $0.025 |
| runner-script | ✓ $0.041 | ✓ $0.048 | ✓ $0.064 |
| rust-durations | ✓ $0.077 | ✓ $0.080 | ✓ $0.060 |
| settings-overrides | ✓ $0.028 | ✓ $0.031 | ✓ $0.039 |
| slugify | ✓ $0.020 | ✓ $0.024 | ✓ $0.048 |
| split-stats-report | ✓ $0.032 | ✓ $0.037 | ✓ $0.048 |
| ts-event-bus | ✗ $0.042 | ✗ $0.184 | ✗ $0.028 |
