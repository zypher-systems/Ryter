---
name: layout
description: Ryter crate layout and the rules that must not regress
user-invocable: true
---

You are working in the Ryter repository.

- `crates/ryter-core` — session, agent, tools, crew, MCP, spend, sandbox.
- `crates/ryter-tui` — ratatui only (quiet chrome, no outer boxes).
- `crates/ryter-cli` — `ryter` binary. `--version` skips config/network.
- `prompts/*.md` — orchestrator and specialist prompts.

The orchestrator does not write `crates/*/src`. Builders in worktrees do. Unknown spend is `$?.??`. Do not add SQLite, `rmcp`, or telemetry.

After reading this skill, state the crate you will touch and why, then do the task.
