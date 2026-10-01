# Benchmark

`ryter bench` runs each task in `bench/` through the real crew, in a fresh repository: a builder does the work, the task's checks and the auditor decide whether it lands, and tests the crew never saw decide whether it was right. This page is the last published run. `ryter bench --publish docs/bench` replaces it and says how the new run compares.

## The last run

- **Ryter:** 0.10.0 on 2026-10-01
- **Crew:** lead `deepseek/deepseek-v4.1-flash`, builder `deepseek/deepseek-v4.1-flash`, auditor `deepseek/deepseek-v4-pro-0813`
- **Cap:** $1.00 per task

| Measure | Result |
|---|---|
| Landed (checks and the auditor passed) | 9 of 9 |
| Accepted (the hidden tests passed too) | 9 of 9 |
| False passes (landed, but wrong) | 0 |
| Total cost | $0.204 |
| Cost per accepted task | $0.023 |

One run of each task is a small sample, and the same crew can take several times as many steps on one run as on the next. Read the cost as what this run cost, not as what the next will.

## By task

| Task | Runs | Landed | Accepted | Cost per run | Time per run |
|---|---|---|---|---|---|
| `cli-limit-flag` | 1 | 1 | 1 | $0.012 | 15 s |
| `inventory-reorder` | 1 | 1 | 1 | $0.026 | 23 s |
| `pager-off-by-one` | 1 | 1 | 1 | $0.006 | 9 s |
| `rename-across-files` | 1 | 1 | 1 | $0.004 | 5 s |
| `rust-durations` | 1 | 1 | 1 | $0.020 | 25 s |
| `settings-overrides` | 1 | 1 | 1 | $0.019 | 24 s |
| `slugify` | 1 | 1 | 1 | $0.029 | 36 s |
| `split-stats-report` | 1 | 1 | 1 | $0.045 | 56 s |
| `ts-event-bus` | 1 | 1 | 1 | $0.042 | 61 s |
