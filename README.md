# Ryter

Ryter is Zypher Systems’ terminal AI coding harness.

Two ways to work, in one app. **Solo mode**: one model in your project, and `Tab` switches its hat between **build**, **plan**, and **review**. **Crew mode** (`/crew`): you talk to a lead, an architect designs, builders work in parallel git worktrees, and independent auditors sign off before anything lands. You get one reviewed patch. Bring your own keys: **SpaceXAI** and **OpenRouter** are built in, and local model servers (Ollama, LM Studio, llama.cpp) work without one.

Linux first. Apache-2.0.

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/zypher-systems/ryter/main/install.sh | sh
```

This installs a prebuilt binary to `~/.local/bin` for Linux (x86_64 and arm64, static, any distro) or macOS (Apple Silicon and Intel). The download is checked against the release's `SHA256SUMS` first. `RYTER_VERSION=v0.2.0` pins a version and `RYTER_INSTALL_DIR` picks the folder. Linux gets the full feature set; on macOS everything works except the Landlock sandbox, which is Linux-only.

From source (Rust 1.88+): `cargo install --git https://github.com/zypher-systems/ryter ryter-cli`.

## Quick start

```sh
export OPENROUTER_API_KEY=...   # or XAI_API_KEY, or a local model: see below
cd your-project
ryter doctor                    # checks keys, terminal, sandbox; no network
ryter                           # the TUI, in solo mode's build hat
ryter -p "say hi"               # one headless turn
```

Copy `config.example.toml` to `~/.ryter/config.toml`. If you put `api_key` in that file, `chmod 600` it.

Full usage: [docs/guide.md](docs/guide.md).

## How it works

**Solo mode** is where Ryter starts. One model works in your files. `Tab` cycles its hat, and the message box shows the hat in its own color:

| Hat | Does |
| --- | --- |
| **build** | changes your files; edits and commands that change things ask first |
| **plan** | reads and proposes; changes nothing |
| **review** | runs the tests and critiques what changed; changes nothing |

Before each build turn Ryter checkpoints your files, and `/undo` puts them back. `/changes` shows what changed, file by file with diffs, and can undo a single file. `/commit` drafts the message from the diff and from what the model said about why, and commits the files you choose. Its receipt trailer records the model, the cost, and the test result (`Ryter: deepseek-pro-latest · $0.34 · tests ✓ 13 passed`). Nothing is committed unless you commit it.

**Crew mode**: type `/crew`. The first time, the crew builder walks you through choosing the lead, architect, builder, and auditor, with a recommendation for each, and a budget. `/solo` goes back. In crew mode you talk to the lead, which reads the repo, answers questions, and decides who does the work. It does **not** edit `src/`.

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
ryter --hat build|plan|review|crew -p TEXT   (default build)
ryter --connection spacexai|openrouter
ryter --sandbox off|workspace|read-only
ryter spend [session]
ryter spend --project         this repository, across sessions
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
