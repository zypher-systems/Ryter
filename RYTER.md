# Ryter (this repository)

Instructions for Ryter (and any other coding agent) working **in this repo**.

## Product

Ryter is a Linux-first BYOK terminal coding harness. The user talks to an **orchestrator**. Plan / Architect / Build / Audit are **specialist kinds** with their own context, not costumes on one body. Do not mention or copy Conn.

## Layout

```
crates/ryter-core/     types, config, providers, tools, session, crew, MCP
crates/ryter-tui/      ratatui TUI only
crates/ryter-cli/      clap binary `ryter`
prompts/               orchestrator + specialist prompts (include_str'd)
docs/guide.md          user guide
config.example.toml    shipped config shape
```

Edition 2024, MSRV 1.88, Apache-2.0. `#![forbid(unsafe_code)]`.

## Non-negotiables

- The orchestrator must not write product source (`crates/*/src`, tests that ship). Builders in worktrees do that. Tests already assert the tool mask.
- Spend: unknown rates are `$?.??`, never a fake `$0.00`. Budget stop is a real stop (headless exit 3).
- SpaceXAI (`https://api.x.ai/v1`, Responses, `XAI_API_KEY`, `grok-4.6`) and OpenRouter are built-in equals.
- `ryter --version` must not open config, keyring, or the network.
- TUI chrome is structural — borders group and separate, they never decorate. Panels are bordered; the base layout uses hairline rules. Golden snapshots under `crates/ryter-tui/snapshots/` are the contract; regenerate with `UPDATE_SNAPSHOTS=1` only when the change is intended.
- Specialists get a fresh window (pass note + task + this file + ROADMAP/DECISIONS). Do not dump the orchestrator transcript into a builder.
- `ROADMAP.md` and `DECISIONS.md` are required project memory. Update them as work changes. Decisions are the *why* (including obscure risks); the code is the *what*. The orchestrator must not invent a why when a decision is recorded.
- `[subagents] max` ≥ 1. Builders always use a git worktree. Auditor pass auto-merges; fail retries then blocks.
- MCP is a small JSON-RPC 2.0 stdio subset in-tree. Do not add the `rmcp` crate.
- Landlock non-off profiles fail closed. Do not silently no-op a requested sandbox.

## How to verify

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
cargo run -p ryter-cli -- --version
```

Do not add SQLite, telemetry, or a hosted proxy that holds user keys.

## Style

Match the crate you are in: short `///` docs on public items, table-driven tests, `Error::Config` / `Error::Io` for recoverable failures. Keep PRs inside the three-crate layout.
