# Hats: one model, or a crew, in one app

Status: built (v1), with these decisions from review:

- Ryter starts in **normal mode**, in the **build** hat. `Tab` cycles build → plan → review.
- **Crew is a mode, not a Tab stop.** `/crew` enters it: the crew builder the first time, straight in after that. `/normal` leaves it.
- The message box shows the mode as a colored badge. The right-hand panel is wider, and the crew's cards appear only in crew mode.
- One model for all hats. `/undo` checkpoints are in. A second-opinion review on the auditor model is for later.

The sketch below is kept as written. Where it differs from the list above, the list wins.

## The idea

Ryter keeps the crew, and adds the way most people already work: **one model
that switches hats**. `Tab` cycles the hat. The same model and the same
conversation carry through every hat, with no worktrees and no handoffs. When a
job is big enough, the last Tab stop sends it to the crew.

```
 ryter  ·  plan  build  review  crew        grok-4.6 · 12% · $0.41   ~/app (main)
          ─────  ▔▔▔▔▔
```

The hat under the bar is active. The four stops are one row, so a user can see
where they are and what the next Tab does.

| Hat | What it's for | Can | Can't |
| --- | --- | --- | --- |
| **plan** | Think before touching anything: read the code, ask questions, propose an approach | read, search, web, write `notes/plan.md` and project memory | edit source, run commands |
| **build** | Do the work in your own tree | everything plan can, plus edit files and run commands (behind the usual permission gate) | nothing extra; destructive commands still ask |
| **review** | Critique what changed: run the tests, read the diff, list problems | read, search, run tests/linters, `git diff` | edit anything |
| **crew** | Hand the job to the lead and the crew: design, parallel builders, independent audits, one patch | route work, as the lead does today | edit source itself |

**The default is build.** A new user types a request and it gets done, exactly
like any single-agent tool. Nothing about crews, auditors, or independence
comes up until they Tab to **crew** for the first time, and that is when the
crew builder opens. First-run friction goes away: today, a user with one key
meets the auditor rule on their first build.

## How switching works

- **Instant, no ceremony.** `Tab` next hat, `Shift+Tab` previous, or `/plan`,
  `/build`, `/review`, `/crew`. There are no handoff notes and no fresh window.
  The model sees the whole conversation, so "now build what we planned" just
  works.
- **Switching doesn't break caching.** The tool list and system prompt stay
  the same in every solo hat, so the provider's prompt cache survives a switch.
  Two things change:
  - **A one-line hat note** goes in front of your next message: `[hat: review —
    read-only; run tests, critique the diff, end with findings]`. It is cheap
    and sits after the cached prefix.
  - **The permission gate knows the hat.** A write attempted in plan is refused
    with "switch to build (Tab) to edit". The model always sees the same tools;
    the hat decides which ones may run.

  The alternative, a different tool list per hat, is cleaner for the model but
  re-bills the whole context at every switch. On a long session that's the
  difference between a switch costing ~nothing and costing a full prompt.
- **Mid-turn:** a switch applies to the next message, never to a turn already
  running.

## Each hat in detail

### plan
- **Prompt:** understand the request, read what's relevant, ask when something
  is ambiguous, and end with a short plan: files, steps, risks, how to verify.
  Write it to `notes/plan.md` when it's longer than a screen.
- **Tools:** read_file, list_dir, grep, glob, web, `ask_user`, and write
  limited to `notes/` and memory. Same gate as the crew lead today.
- **Ends with:** "Tab to build to carry this out."

### build
- **Prompt:** implement, match the existing style, run the project's tests for
  what changed, and say what was done and what wasn't.
- **Tools:** the builder's set (read, write, search_replace, bash), but **in
  your own tree**, so the gate is a normal agent's, not a worktree builder's:
  - Edits ask (or run, with `--always-approve` or "allow all").
  - Read-only commands run; anything else asks; destructive commands always
    ask.

  This is a new policy row. Today a builder may run any command because it's
  confined to a throwaway worktree, and that must not carry over.
- **Checkpoint:** before the first edit of a turn, Ryter records a git
  checkpoint (a ref, not a commit on your branch). `/undo` restores it. This
  is cheap insurance, and it is what makes build safe to be the default.

### review
- **Prompt:** the auditor's contract, aimed at your own work: what changed
  since the last checkpoint (or `git diff`), does it do what was asked, is it
  tested, what's risky. It ends with findings, blocking ones first.
- **Tools:** read, search, the auditor's command allow-list (tests, linters,
  read-only git). No edits.
- **Second opinion (optional):** if an auditor model is configured, review can
  run on that model instead of the current one, one keypress in the review
  hat. That keeps the crew's best idea, independent review, in solo mode, at
  the price of one review.

### crew
- Exactly today's flow: the lead routes, the architect designs, builders run
  in parallel worktrees, auditors sign off, and one patch lands. The crew
  builder opens the first time it's needed.
- **What the crew sees:** the lead gets the conversation so far, so "send what
  we planned to the crew" works. Builders still get only their briefs.

## Cost

| Way of working | Rough cost per task (docs/cost.md model) |
| --- | --- |
| build hat, strong model | 1.0× (the single-agent baseline) |
| build hat, cheap model | 0.1–0.3× |
| build + one second-opinion review | ~1.1× |
| crew, schooner | ~0.4–0.6× of a strong single agent, plus the design when there is one |

Solo mode also gives the benchmark its baseline for free: `ryter bench --hat
build` versus `--crew schooner`, on the same tasks, in the same harness.

## What changes in the code

| Area | Change |
| --- | --- |
| core `role.rs` | A `Solo` role (spend shows it as the hat name), and a `Hat` enum stored in the session |
| core `tools/policy.rs` | Per-hat rules for Solo: plan (lead's gate), build (in-tree gate: ask on writes and non-read-only commands), review (auditor's allow-list) |
| core `agent.rs` | The hat note on each user message; `/undo` checkpoints for build; crew hat = today's lead path |
| `prompts/` | `solo.md` (shared: who you are, the hats exist) plus three short hat notes |
| TUI | `Tab`/`Shift+Tab` cycle the hat (the composer stops inserting literal tabs); the header chip row; a hat-colored composer border; `/plan` `/build` `/review` `/crew` |
| CLI | `ryter --hat plan|build|review|crew -p …`; headless defaults to build |
| onboarding | First launch goes straight to build; the crew builder opens on the first switch to crew |
| docs | README "How it works", guide, `crew.md` becomes "the crew hat" |

Roughly: the policy row and checkpoints are the careful parts (they decide
what runs in your tree). The rest is plumbing the crew work already built:
sessions, permission prompts, spend, header, palette.

## Decisions for you

1. **Default hat for new users:** build (recommended), or crew?
2. **Is crew a Tab stop**, or a separate command (`/crew`) so Tab cycles only
   the three solo hats?
3. **Second-opinion review on the auditor model:** in v1, or later?
4. **One model for all hats,** or a model per hat (e.g. plan on a strong model,
   build on a cheap one)? Recommended: one model in v1. Per-hat models are
   where this starts turning back into a crew.
5. **`/undo` checkpoints** in v1? Recommended, since build is the default and
   edits your tree directly.
