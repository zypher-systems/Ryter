# The Test hat: approved design

Designed with the user on 2026-10-01, a mockup for each choice. Nothing here is built yet. This page is what the build is held to; where the build has to differ, it says so up front.

## What was decided

- **The hats are Plan, Build, Review and Test.** Test uses the product as a user would: it starts it, exercises it, and reports.
- **A real hat.** It is on `Tab` with the others, and its model is a seat in `/models` beside *All hats*, *Plan*, *Build* and *Review*. No separate interface.
- **Its own thread.** The tester doesn't read the conversation the other hats share. It starts from a brief: the approved plan, the plan's entries in `.ryter/decisions.md`, the files that changed, and how the project runs. Its thread continues through the session, so it knows what it did before.
- **A report comes back.** The tester's working (commands, logs, dead ends) stays in its thread. Its report goes to three places: the main conversation as one message, a file under `.ryter/tests/`, and the commit receipt.
- **Scope of this version:** it tests through commands and web requests. It doesn't drive a browser.

## The Test hat's screen

`Tab` to Test and the chat shows the tester's thread. `Tab` again and the main conversation is back as it was left, holding only the reports.

```
  R Y T E R         │ TEST THREAD · 2 runs this session · tab: main chat
  cms · main        │
                    │ 14:02 ● Ryter
   TEST             │       │ Test the work against the plan
   uses the product │       │ plans/2026-10-01-cms.md · 7 files changed
  plan·build·review │       ◆ kimi-k3
                    │       ├─ start  docker compose up -d --wait ·· ✓ 14s
  MODEL             │       ├─ run    pytest -q ·············· ✓ 21 passed
  kimi-k3           │       ├─ get    /manage/ ················· ✗ 500
                    │       ├─ stop   docker compose down ············ ✓
  SPEND             │       └─ ✗ 2 of 5 failed · 1m40s · $0.21
  this run   $0.21  │ ╭────────────────────────────────────────────────╮
                    │ │ › ask the tester, or: retest 3 · Tab: plan      │
                    │ ╰────────────────────────────────────────────────╯
```

## The report in the main conversation

Failures open, passes one line. The build hat can fix from this without opening anything.

```
  ·  switched to the test hat (kimi-k3)
  ▣  test · kimi-k3 · ✗ 2 of 5 failed · 1m40s · $0.21
  │  ✓ 1  the stack starts and is healthy
  │  ✓ 2  first-run setup creates the admin
  │  ✗ 3  /manage/ after login
  │       expected the page list
  │       got 500: NoReverseMatch 'pages:list'
  │       to see it: start the stack, log in, open /manage/
  │  ✗ 4  publish a page · not reached (needs 3)
  │  ✓ 5  pytest in the container · 21 passed
  │  full report  .ryter/tests/2026-10-01-cms-2.md
  ·  switched to the build hat
```

A failed test works like a failed review: it offers its fixes in the build hat, and the fixes are new work.

## When a test is offered

After a review that passed. `/test` asks for one at any time, and `s` stops the offers.

```
  build turn changed files
        ▼
  ┏━ review this work? ━━━━━━━━━━━━━━━━━━━━━━━━━━┓
  │ grok-4.7 · reviews 7 files · about $0.04–0.31 │
  │ ⏎ review   n not now   s stop offering        │
  ╰───────────────────────────────────────────────╯
        │  VERDICT: PASS
        ▼
  ┏━ test this work? ━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
  │ kimi-k3 · starts the stack, runs 5 scenarios  │
  │ from the plan, stops it · about $0.10–0.60    │
  │ ⏎ test   n not now   s stop offering          │
  ╰───────────────────────────────────────────────╯
```

## How the project runs: `.ryter/run.toml`

The first time a test is asked for, the model reads the project and proposes the commands. The user approves, adjusts or rejects them in a panel, as with a plan. They are saved and reused.

```
  ┏━ how this project runs ━━━━━━━━━━━━━━━━━━━━━━━━━━┓
  │ start  docker compose up -d --wait               │
  │ ready  http://localhost:8000/healthz             │
  │ test   docker compose run --rm web pytest -q     │
  │        docker compose run --rm web ruff check .  │
  │ stop   docker compose down                       │
  │                                                  │
  │ saved to .ryter/run.toml                         │
  │ the tester runs these without asking             │
  │ y approve   e adjust   n reject                  │
  ╰──────────────────────────────────────────────────╯

  .ryter/run.toml
    start = "docker compose up -d --wait"
    ready = "http://localhost:8000/healthz"
    test  = ["docker compose run --rm web pytest -q",
             "docker compose run --rm web ruff check ."]
    stop  = "docker compose down"
```

## What the tester may run

The mockup first approved here had "anything else asks first". The user changed it the same day: too much asking. The tester runs what the build hat runs without asking, and never edits or writes project files.

```
  runs without asking
    the commands in .ryter/run.toml
    reading files, git status, test runners, linters
    your toolchains and the project's own programs
    the project's containers: build, up, down, run, exec, logs
    requests to the project's own address (curl localhost:8000/…)
    writing to /tmp and your home folder

  asks first (y / n / a allow this kind for the session)
    removing volumes, stopping a container by name, docker push
    publishing, and tools for a service elsewhere (gh, aws, kubectl)
    curl https://example.com/…
    a system program that isn't a toolchain

  never
    editing or writing project files
    sudo, push, and the rest of the never-run list
```

## After a test

The project is left running until the user says. Ryter stops only what it started, with the approved stop command, and never removes volumes.

```
  ▣  test · kimi-k3 · ✗ 2 of 5 failed · 1m40s · $0.21
  │  …
  │  full report  .ryter/tests/2026-10-01-cms-2.md
  ·  the project is still running at http://localhost:8000
     /stop stops it (docker compose down)

  on quit, with it still up:
  ┏━ stop the project? ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━┓
  │ Ryter started it for the test at 14:02.         │
  │ docker compose down                             │
  │ ⏎ stop it   n leave it running                  │
  ╰─────────────────────────────────────────────────╯
```

## Decisions: `.ryter/decisions.md`

Built (see the guide, "When the work differs from the plan"). The tester's brief points at the plan's entries, as the reviewer's does: a difference recorded there is not a failure.
