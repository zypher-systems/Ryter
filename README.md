# Ryter

Ryter is Zypher Systems’ terminal AI coding harness.

You talk to one agent, the **lead**. A crew does the work: an architect designs, builders work in parallel git worktrees, and independent auditors sign off before anything merges. You get one patch. Bring your own keys: **SpaceXAI** and **OpenRouter** are built in, and local model servers (Ollama, LM Studio, llama.cpp) work without one.

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

You talk to one agent, the lead. It reads the repo, answers questions, and decides who does the work. It does **not** edit `src/`.

- A precise change: the lead writes builder tasks.
- Something that needs a design: the lead asks the architect, whose tasks go straight to builders.
- Builders work in parallel, each in its own git worktree, up to `[subagents] max` at once.
- Every builder's work passes the project's checks and an independent auditor before it joins the patch.
- The patch lands on your branch as **one commit**, and the lead tells you what landed and what it cost.

Nothing reaches your branch until the project's checks pass and an auditor signs off (`VERDICT: PASS`). With `/auditor off`, finished work waits on its branch for you. See `crew.md` for the full contract.

## CLI

```
ryter                         TUI
ryter -p TEXT [--json]        one headless turn
ryter -c -p TEXT              continue the latest session headless
ryter --connection spacexai|openrouter
ryter --sandbox off|workspace|read-only
ryter spend [session]
ryter connections [list|add|remove|test|set-key]
ryter models [connection]
ryter sessions
ryter resume [id]
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
