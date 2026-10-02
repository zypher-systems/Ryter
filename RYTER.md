# Ryter (this repository)

Instructions for Ryter (and any other coding agent) working **in this repo**.

## Product

Ryter is a Linux-first BYOK terminal coding harness. One model works in the user's project, in one conversation, wearing one of three **hats**: plan, build, review. Each hat can run on its own model. There is no crew mode: it was removed (`DECISIONS.md`, 2026-10-01), and nothing should bring back a lead, specialists, a task queue or worktrees without the user deciding so. Do not mention or copy Conn.

## Layout

```
crates/ryter-core/     types, config, providers, tools, session, the review gate, MCP
crates/ryter-tui/      ratatui TUI only
crates/ryter-cli/      clap binary `ryter`
prompts/               the one prompt every hat shares (include_str'd)
docs/guide.md          user guide
config.example.toml    shipped config shape
```

Edition 2024, MSRV 1.88, Apache-2.0. `#![forbid(unsafe_code)]`.

## Non-negotiables

- The permission gate enforces the hat: plan and review change nothing in the project (the plan hat may write project memory), and the build hat asks before it changes anything. Tests assert this.
- Spend: unknown rates are `$?.??`, never a fake `$0.00`. Budget stop is a real stop (headless exit 3).
- SpaceXAI (`https://api.x.ai/v1`, Responses, `XAI_API_KEY`, `grok-4.6`) and OpenRouter are built-in equals.
- `ryter --version` must not open config, keyring, or the network.
- TUI chrome is structural — borders group and separate, they never decorate. Panels are bordered; the base layout uses hairline rules. Golden snapshots under `crates/ryter-tui/snapshots/` are the contract; regenerate with `UPDATE_SNAPSHOTS=1` only when the change is intended.
- Every hat shares one system prompt and one tool list, so a hat switch keeps the provider's prompt cache. The hat is a note on each message.
- `ROADMAP.md` and `DECISIONS.md` are required project memory. Update them as work changes. Decisions are the *why* (including obscure risks); the code is the *what*. Do not invent a why when a decision is recorded.
- Nothing is committed unless the user commits it. A review's verdict holds only for the files it read; the commit receipt says so.
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
