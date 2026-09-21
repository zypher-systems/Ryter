# Product direction

A point of view on turning Ryter into something people choose over the tools they
already have. Opinions are marked as such. Push back on any of them.

---

## The market you are entering

Terminal coding agents are a crowded category. Claude Code, Codex CLI, Gemini CLI,
Aider, OpenCode and Goose already exist, and most are free or bundled with a
subscription. "A BYOK terminal harness with a nice TUI" is table stakes, not a
reason to switch. Ryter needs one sentence that none of them can say.

## The sentence

> **A coding crew you can leave running. Builders work in parallel, and nothing
> reaches your branch until your tests pass and a second model signs off.**

This is the right sentence because it is already the architecture: worktrees,
per-role models, the merge gate. It also answers the question people actually have
about agents, which is not "can it write code?" but **"can I stop watching it?"**
Every other agent writes into your working tree while you supervise. Ryter's claim
is that you don't have to.

Three things back the claim, and each is a feature to invest in:

1. **Trust.** An unreviewed change never lands. (The gate, `crew.md` §4.)
2. **Throughput.** Several tasks at once, in isolation. (Worktrees plus file scopes.)
3. **Cost control.** A cheap model for conversation, an expensive one where the
   thinking happens, and honest spend. (Per-role routing, `$?.??`, budget stop.)

The DECISIONS.md memory is a fourth strength. I'd treat it as supporting, not
headline: it is what keeps a mixed-model crew coherent over time.

## Who it is for, first

*Opinion:* individual developers and small teams who already pay for more than one
model provider, and who have a **backlog of well-defined work**: tests to write,
refactors, small features, migrations, lint debt. The pitch is "queue it, walk away,
review what merged."

Who it is *not* for yet: people who want tight, in-the-loop pair programming. Claude
Code and Cursor will beat a gated crew on latency every time, and that's fine. Don't
compete there.

---

## What has to exist for the sentence to be true

In priority order. The first three are what make the product; the rest make it good.

### 1. A review surface for the crew — the product's main screen

Today the gate's outcomes arrive as chat text. The crew *is* the product, so it needs
a first-class panel:

- every task with its state: queued / building / checking / auditing / merged /
  rejected / **waiting on you**
- per task: the diff, the handback, the check output, the audit, and the cost
- actions: merge a waiting branch, retry with a note, reject, and **revert a merged
  task** (`git revert -m 1` on its merge commit, which is cheap now that every task
  is one `--no-ff` commit)

This is also where the tasks card should open; it is the one card that opens nothing
today.

### 2. An independent auditor by default

Crew roles follow the orchestrator's model, so the auditor is usually the model that
wrote the code. That is not a second opinion. *Opinion:* first-run setup should ask
for two providers, or at least two models, and put the auditor on the one the builder
doesn't use. Warn in `/crew` and `ryter doctor` when they match. "A second model
signs off" is the headline, so make it true by default.

### 3. Checks that configure themselves

The gate is only as strong as `[auditor] checks`, and the default is none. On first
use in a project, detect and propose them: `Cargo.toml` → `cargo test`,
`package.json` → its `test` script, `pyproject.toml` → `pytest`, `go.mod` →
`go test ./...`. The user confirms, and the result is written to
`.ryter/config.toml`. Zero-config should mean *tested*, not *trusted*.

### 4. Evals: the investment everything else depends on

The most important finding from this review: tool calling was broken on two of three
backends, and Claude could not call tools at all, yet 212 tests passed. Every agent
test used a replay provider that delivered idealized deltas. You cannot tune prompts,
compare models, or claim trust without measuring real runs.

- **Recorded wire fixtures per provider.** Capture real SSE from each backend,
  including tool calls, parallel calls, and truncation, and replay those, not
  hand-built deltas.
- **A nightly live smoke test** against each built-in provider: one tool call round
  trip.
- **A task benchmark.** Around 20 real tasks in fixture repos, run end to end. Track
  landed / rejected / retried / conflicted, cost per landed task, and wall time, by
  model pairing. This is how you learn that, say, a cheap builder with a strong
  auditor beats a strong builder alone, and how you know a prompt change helped.

### 5. Cost per task, and a cap per task

A crew multiplies spend: builder + auditor + retries, times N in parallel. Show cost
per task in the review surface, give tasks a budget like sessions have, and stop
running unpriced models without an explicit opt-in. (Today an unpriced model has no
cap at all.)

### 6. A fast path for trivial edits

*Opinion:* the "orchestrator never writes source" rule is right for real work and
painful for a typo. Let the orchestrator propose a small diff inline and apply it
when you press `y`, since your approval is a sign-off. Without this, people will
reach for another tool for the small stuff and never come back.

### 7. Unattended mode as a product: `ryter run`

Once the crew can be trusted, the natural next step is to run it with no one
watching: `ryter run tasks.toml`, in CI or overnight, producing **branches or pull
requests** instead of local merges, with the audit as the PR description. This is the
team story, and it is where a gated crew clearly beats an interactive agent.

---

## What to stop investing in, for now

*Opinion.* Surface area is currently ahead of the core loop.

- **Inbound MCP (serve, socket attach, TCP tokens).** An interesting capability, but
  unrelated to the promise. Freeze it.
- **Themes, the light theme, custom palettes.** The TUI is good enough; polish the
  crew screen instead.
- **Hooks and skills menus.** Keep what exists, don't expand.
- **The phase ceremony.** Keep phases as guardrails, and let the orchestrator route.
  Users shouldn't have to drive a state machine.

And one to *pull forward*: **macOS without the sandbox.** Landlock is Linux-only,
but nothing else is, and a large share of developers are on macOS. Ship there and
label the sandbox as Linux-only.

## Naming

- **Orchestrator → lead** in the UI, keeping `orchestrator` in code. "Lead" fits a
  crew: lead, architect, builders, auditors. "Orchestrator" reads as infrastructure.
- **Auditor** is fine. "Reviewer" is more familiar to developers, but "auditor"
  signals *gate*, which is the point. Keep it.
- **Crew** is a good word. Use it everywhere: `/crew`, "crew report", "the crew is
  working".

## A 90-day shape

| Weeks | Ship | Why |
| --- | --- | --- |
| 1–3 | Recorded wire fixtures + a live smoke test; checks auto-detect; independent-auditor default | Make "tested and independently reviewed" true by default, and stop shipping broken backends |
| 4–7 | The crew review surface (diff, merge, retry, revert, per-task cost) | The product's main screen |
| 8–10 | Task benchmark; tune prompts and default model pairings against it | Evidence instead of opinion |
| 11–13 | `ryter run` + PR output; macOS | Unattended and team use; reach |

Measure one number above all: **the share of queued tasks that land without a human
touching them, and the cost per landed task.** If that number is good and trending up,
the product works.
