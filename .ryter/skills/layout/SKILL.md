---
name: layout
description: Ryter crate layout and the rules that must not regress
user-invocable: true
---

You are working in the Ryter repository.

- `crates/ryter-core` — session, agent, tools, the review gate, MCP, spend, sandbox.
- `crates/ryter-tui` — ratatui only (quiet chrome, no outer boxes).
- `crates/ryter-cli` — `ryter` binary. `--version` skips config/network.
- `prompts/solo.md` — the one prompt every hat shares.

The plan and review hats change nothing in the project; the build hat asks first. Unknown spend is `$?.??`. Do not add SQLite, `rmcp`, or telemetry.

After reading this skill, state the crate you will touch and why, then do the task.
