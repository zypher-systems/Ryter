# Ryter

Ryter is Zypher Systems’ terminal AI coding harness.

You talk to an **orchestrator**. Independent specialists (plan, architect, build, audit) do the work in parallel. Builders each get a git worktree; an auditor reviews the diff; a pass merges without a prompt. Bring your own keys — **SpaceXAI** and **OpenRouter** ship built-in.

Linux first. Apache-2.0.

## Quick start

```sh
# Rust 1.88+
cargo build -p ryter-cli
export XAI_API_KEY=...          # and/or OPENROUTER_API_KEY
./target/debug/ryter doctor
./target/debug/ryter            # TUI (tty)
./target/debug/ryter -p "say hi" --always-approve
```

Copy `config.example.toml` to `~/.ryter/config.toml`. If you put `api_key` in that file, `chmod 600` it.

Full usage: [docs/guide.md](docs/guide.md).

## How it works

The conversation is always the orchestrator. It reads, asks, and updates a task list. It does **not** edit `src/`.

Open tasks become specialists, up to `[subagents] max` at once:

| Phase | Who runs |
| --- | --- |
| Plan | planners |
| Architect | architects |
| **Build** (default) | builders in worktrees, then an auditor |
| Audit | extra review specialists |

A builder pass that the auditor accepts is merged automatically. `/auditor off` skips the gate.

## CLI

```
ryter                         TUI
ryter -p TEXT [--json]        one headless turn
ryter --mode plan|architect|build|audit
ryter --connection spacexai|openrouter
ryter --sandbox off|workspace|read-only
ryter spend [session]
ryter connections [list|add|remove|test|set-key]
ryter models [connection]
ryter sessions
ryter resume [id]
ryter handoff <phase|back> [--note ...]
ryter mcp serve               inbound MCP on stdio
ryter serve --socket          inbound MCP on a unix socket
ryter serve --bind HOST:PORT --token …
ryter doctor
ryter trust                   trust this dir’s .ryter/
ryter --version               no config, keyring, or network
```

Budget exceeded exits `3`. Unknown model prices display as `$?.??`, never a fake `$0.00`.

## License

Apache-2.0. See `LICENSE`.
