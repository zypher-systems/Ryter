# Acceptance evidence — 2026-10-02

The live run completed ten isolated coding tasks using the app’s configured OpenRouter connection and `qwen/qwen3-coder-flash`. Recorded provider cost was **$0.418755831 of the authorized $5 total**, across **524 requests**. All recorded calls had complete accounting. No further paid run was used to improve these results.

**Six implementations passed the hidden tests. Five also finished all four phases with passing review and Test-hat outcomes. Three failing implementations received a review pass and a Test-hat pass.** This is one small model/configuration sample, not a claim that reviews or model-generated changes are reliable enough to skip external review.

| Task | Hidden acceptance | Review verdict | Test report | Recorded USD |
| --- | --- | --- | --- | --- |
| cli-limit-flag | pass | PASS | pass | $0.022281 |
| inventory-reorder | fail | PASS | pass | $0.048101 |
| pager-off-by-one | pass | PASS | pass | $0.030756 |
| rename-across-files | pass | PASS | pass | $0.017154 |
| runner-script | pass | PASS | pass | $0.038854 |
| rust-durations | fail | not reached | not reached | $0.053856 |
| settings-overrides | pass | PASS | fail | $0.043633 |
| slugify | pass | PASS | pass | $0.042848 |
| split-stats-report | fail | PASS | pass | $0.067980 |
| ts-event-bus | fail | PASS | pass | $0.053293 |

## Reproducible failures

- **inventory-reorder:** `needs_reorder()` and its report retained insertion order instead of sorting by SKU. Two hidden assertions failed despite both verification hats passing.
- **rust-durations:** build stopped at the benchmark’s deliberately restricted 32,768-token context allowance (32,882 estimated tokens including tools and output). Admission refused the next request; review/test were not reached. The partial implementation also failed unit-order and overflow checks. This was the configured fixture limit, not the model’s advertised maximum context.
- **split-stats-report:** a header after a data row was silently accepted instead of raising the specified error. Both verification hats passed.
- **ts-event-bus:** the implementation removed the existing `off` method and invoked wildcard handlers twice when emitting `*`. Both verification hats passed.
- **settings-overrides:** hidden acceptance passed, but the Test hat reported failure after trying forbidden edits/inline code and unavailable interactive operations. That task counts as an implementation pass, not a successful complete flow.

The initial scorer missed `## VERDICT: PASS`. The parser now follows the application’s Markdown handling, and the report was recalculated from saved event streams without new provider calls. The original report remains intact. The corrected false-review count is three, including the event-bus task.

## Controls and scope

Before paid calls, every unchanged fixture failed hidden acceptance and every reference solution passed visible and hidden checks. Ten simulated CLI tasks passed; an intentionally unchanged builder produced false review/test passes that the scorer detected. Simulated cost is synthetic accounting, not provider spending or a model-quality result.

The application source exercised by the paid run was the acceptance patch (`dbb9154`) including the active-session sandbox patch. The runner was started before its final check-process environment isolation edit; final offline checks use isolated homes. Live CLI processes intentionally used the app’s configured credentials through the normal loading path, without copying or displaying them. The app’s previous last-used model selection was restored after the run.

Each paid task used a fresh Git repository under `target`, the workspace sandbox, an allocation no larger than $0.35, a 32K context override and an 8K output allowance. An aggregate reserve and incomplete-accounting stop guarded the $5 run cap. Price fallback used the highest listed model tier; provider-reported charges took precedence. Pricing was checked against [OpenRouter’s model pricing](https://openrouter.ai/qwen/qwen3-coder-flash/pricing) before the run. Headless plan replies were proposals followed by an explicit build instruction; they did not exercise the interactive approval UI.

The final combined source passed 614 workspace tests (4 optional tests ignored), the locked build, clippy with warnings denied, formatting, seven CLI scenarios, three scoring regressions, ten simulated tasks, the false-pass control, five installer fixtures and dependency advisory/license/source checks.

The core acceptance test separately covers plan approval, build, review, an approved Test run, resume, undo/redo, later user edits and commit receipts from a nested project. Seven CLI acceptance cases cover real subprocess routing, all hats, budgets, interrupted accounting and recovery. Provider protocol fixtures cover interleaved tool-call streams; these are sanitized regression fixtures, not recordings of the paid run. TUI geometry/snapshot tests pass, but this work does not claim a complete human-operated TUI acceptance session.

## Local evidence and replay

The ignored local directory `target/bench-live-2026-10-02` retains fixture hashes, baseline/reference checks, per-phase NDJSON/stderr, generated projects, and hidden failure output. `report.json` is the original; `report-rescored.json` includes the corrected verdicts and separate `flow_completed` field. These artifacts are intentionally not committed because session transcripts can contain project content. The summary above contains no credentials.

```sh
python3 -m unittest discover -s bench -p 'test_*.py'
python3 scripts/acceptance.py
python3 bench/run.py --mode simulated
python3 bench/run.py --mode simulated --simulate-broken-build --task pager-off-by-one
# No model calls; writes a new report (choose an unused output file):
python3 bench/run.py --rescore target/bench-live-2026-10-02/report.json --output target/rescored-copy.json
```

The user’s configured application and existing processes were left running. The fixtures are command-line libraries/programs; no test server, login or container was started for this benchmark. Version remains 0.11.0, with no release or main-branch change.
