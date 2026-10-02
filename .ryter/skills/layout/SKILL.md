---
name: layout
description: Ryter crate layout and the rules that must not regress
user-invocable: true
---

You are working in the Ryter repository.

- `crates/ryter-core` — session, agent, tools, the review gate, MCP, spend, sandbox.
- `crates/ryter-tui` — ratatui only. Chrome is structural: borders group and separate, they never decorate. The solo screen's contract is `docs/hat-rack-design.md`.
- `crates/ryter-cli` — `ryter` binary. `--version` skips config/network.
- `prompts/solo.md` — the one prompt every hat shares.

The plan and review hats change nothing in the project; the build hat asks first. Unknown spend is `$?.??`. Do not add SQLite, `rmcp`, or telemetry.

After reading this skill, state the crate you will touch and why, then do the task.
