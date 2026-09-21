# Crew design contract

Status: **implemented** on `0.2.0-patch` (`5712c51`). Cost model: `docs/cost.md`.
Scope: who does what, how work moves from a request to your branch, and the rules
that make it safe to leave running. `design.md` owns the TUI; this owns the crew.

The promise, in one line:

> You talk to one agent. It runs a crew of builders in parallel, and your branch
> receives the finished change **as one patch, only after your checks pass and
> auditors that are different models from the lead and builder sign off.**

Every rule below exists to keep that sentence true.

---

## 1. Roles

| Role | Runs | Window | Writes | Returns |
| --- | --- | --- | --- | --- |
| **Lead** (`orchestrator` in code) | your tree, conversational | the session transcript | project memory only | answers; tasks via `todo_write`; memory updates |
| **Architect** | your tree, one-shot | fresh: memory + brief | project memory only; never source | `notes/architect.md`, tasks, DECISIONS, a short summary |
| **Builder** | its own worktree, parallel | fresh: memory + its brief | product source in its owned paths | a structured handback |
| **Auditor** (one or a panel) | the builder's worktree | fresh: brief + handback + checks + diff | nothing | findings + `VERDICT: PASS/FAIL` |

- **Auditors must be different models from the lead and the builder.** A pass from
  the model that directs or wrote the code isn't a second opinion. Builds refuse
  before any builder spends a token when this doesn't hold. Routes don't disguise
  a model: `x-ai/grok-4.6`, `grok-4.6` and `grok-4.6-latest` are one model.
- **Panel.** `[[auditor.panel]]` seats all must pass, in order, stopping at the
  first FAIL. A seat can have a `focus` (e.g. "security") and `paths` globs; at
  least one seat must cover every change.
- **Context is scoped.** Builders and auditors see `notes/architect.md` and the
  DECISIONS entries naming their files, not the whole memory (`docs/cost.md`).

- The planner was folded into the architect. Two sequential read-and-write-a-note
  roles re-read the repo twice and lost detail at the handoff. `planner` still
  parses, and old sessions, logs, and saved crew rows still load.
- The lead never writes product source. This is enforced by the tool mask,
  not by the prompt.
- **Project memory has serial writers only**: the lead and the architect,
  both in your tree. Builders are denied `ROADMAP.md`, `DECISIONS.md` and `notes/`,
  because N parallel copies of those files conflict on every merge. Their decisions
  come back in the handback, and the lead records them.

## 2. The lead routes; there are no phases

Every user message goes to the lead. Each task carries a `role`: `architect` tasks
run first in a drain, and the builder tasks they write run in the same drain. A
task with `hold: true` waits for the user's go-ahead (for "design only"). Phases
and handoffs were removed (see `DECISIONS.md`): switching them by hand confused
users and models alike.

## 3. Tasks

```json
{ "id": "t3", "title": "parse --since flag",
  "brief": "Add --since <date> to `ryter sessions` … done when `sessions --since 2026-01-01` filters and a test covers it.",
  "files": ["crates/ryter-cli/src/main.rs"] }
```

- **`brief` is the builder's entire spec.** It used to be the title alone.
- **`files` is the scope the task owns.** Disjoint scopes run in parallel. Overlapping
  or undeclared scopes run one at a time, because an undeclared task could touch
  anything. Overlap is by path component: `src/tui` overlaps `src/tui/draw.rs`, but
  not `src/tuix`.
- A `todo_write` cannot drop a task that is running.

## 4. The build pipeline

```
 open patch: ryter/patch-<id>-<n>, branched from your branch
      │
      ├─ task ─► builder in its worktree ─► commit
      │            ▼
      │          integrate the patch branch into the worktree ─conflict─► resolver (in the worktree) → re-audit
      │            ▼
      │          gate ① checks (harness) ─fail─► retry IN PLACE with the output
      │            ▼
      │          gate ② auditor panel, in order, first FAIL stops ─FAIL─► retry in place with findings
      │            ▼ all PASS
      │          land the task on the patch branch (serialized, cannot conflict)
      ├─ task … (disjoint scopes in parallel)
      ▼
 every task done? ─no─► patch waits (blocked task: retry or drop it; you have nothing to act on)
      ▼ yes
 your branch moved? → integrate it into the patch worktree; any resolution → re-audit
      ▼
 combined checks on the whole patch ─fail─► patch waits; a fix task lands into the same patch
      ▼
 your uncommitted edits overlap the patch? → waits and names the files
      ▼
 ONE --no-ff commit on your branch · `git revert -m 1` undoes the whole patch
```

**Invariants.** Each one is pinned by a test.

| # | Invariant | Test |
| --- | --- | --- |
| C-01 | Nothing lands without a `VERDICT: PASS`. With the auditor off, work waits. | `without_an_auditor_nothing_lands` |
| C-02 | Checks run before the auditor; a failing check never costs an audit. | `a_failing_check_rejects_before_audit` |
| C-03 | The auditor sees everything landing would change, new files included. | `the_auditor_sees_new_files` |
| C-04 | Conflicts are resolved in a worktree; your checkout never holds a marker or a half-merge. | `conflicts_resolve_in_the_worktree_and_are_re_audited` |
| C-05 | Code written by a resolver is re-audited. | same |
| C-06 | Your uncommitted edits block a merge only if they overlap it, and they always survive. | `dirty_files_block_only_when_they_overlap` |
| C-07 | A task is one `--no-ff` commit on the patch; a patch is one on your branch. | `builder_passes_the_gate_and_lands_as_one_commit`, `two_tasks_land_as_one_patch_commit` |
| C-08 | Builders cannot write project memory. | `only_serial_roles_write_project_memory` |
| C-09 | Disjoint scopes run together; overlapping or undeclared ones wait. | `disjoint_scopes_run_together_overlapping_ones_wait`, `unscoped_tasks_run_alone` |
| C-10 | Prompts state exactly what the runtime parses. | `prompts_state_the_contracts_the_runtime_parses` |
| C-11 | Tool calls reassemble from real streaming shapes on every backend. | `streamed_tool_calls_reassemble_on_every_backend` |
| C-12 | Auditors are different models from the lead and the builder; otherwise nothing starts. | `auditors_must_differ_from_lead_and_builder`, `builds_pause_when_the_auditor_is_the_lead` |
| C-13 | A blocked task holds the whole patch; your branch gets nothing until it is retried or dropped. | `a_blocked_task_holds_the_whole_patch` |
| C-14 | The panel stops at the first FAIL; path-scoped seats skip unrelated changes. | `the_panel_stops_at_the_first_fail`, `a_path_scoped_seat_skips_unrelated_changes` |
| C-15 | Every specialist round is metered and counts against the session budget and task caps. | `every_specialist_round_is_metered`, `crew_spend_reaches_the_session`, `a_task_cap_stops_the_task_and_keeps_its_branch` |
| C-16 | A rejected task is fixed in its own worktree, not rebuilt. | `a_retry_fixes_the_rejected_attempt_in_place` |
| C-17 | The fast path needs a person: no blanket approval stands in, and headless it is refused. | `a_proposed_edit_is_refused_without_a_person` |

**Re-gating rule.** The checks re-run whenever the tree changes. Auditors re-run
only when *someone else's* code enters a task (a resolver ran). A clean merge of
already-audited work doesn't change what a builder wrote, and the checks,
including the final combined run on the patch, catch the interactions.

## 4a. The fast path

For a typo, a one-line fix, or a config value, the lead calls `propose_edit`
(at most 20 lines a side). You see the diff in the permission modal and press `y`;
your approval is the sign-off. It needs a person: `--always-approve` and the
session-wide `a` don't apply, and headless it is refused. It edits your tree
uncommitted, like your own edit.

## 5. Gate configuration

```toml
# .ryter/config.toml — per project, trusted projects only (it runs commands)
[auditor]
enabled = true            # off = nothing merges automatically
max_retries = 2           # builder attempts after a rejection
checks = ["cargo test --workspace", "cargo clippy --workspace -- -D warnings"]
check_timeout_secs = 1200
```

With no checks configured, the auditor is told to run the tests itself. That is
weaker: whether anything gets tested is then up to a model.

## 6. How results travel

- **Builder → auditor:** the handback (`STATUS / FILES / DECISIONS / NOTES`).
- **Crew → lead:** after a batch, `drain_crew` returns a report (status,
  handback, checks, audit per task). The lead gets another round to tell
  you what happened and to record decisions. The report is also kept in the session
  (`notes/crew.md`), so later turns can see it.
- **Orchestrator → you:** what merged, what was rejected and why, and which branches
  are waiting on you.

## 7. Decisions and open questions

Decided on 2026-09-21: auditors must be different models (C-12); a panel of
auditors; the fast path; a patch as the unit you receive; cost treated as a
first-class constraint (`docs/cost.md`).

Still open:

1. **Setup defaults.** Independence means setup must ask for two models. It
   should default to a cheap builder and a strong auditor/architect from another
   provider (`docs/cost.md` §4).
2. **Abandoning a whole patch.** Today you drop its tasks one by one. A
   `/patch drop` would be clearer.
3. **Pull requests.** An optional alternative to local landing, for teams:
   the patch becomes a PR with the audits as its description.
