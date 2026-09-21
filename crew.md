# Crew design contract

Status: **implemented** on `0.2.0-patch` (`5cde460`), open questions at the end.
Scope: who does what, how work moves from a request to your branch, and the rules
that make it safe to leave running. `design.md` owns the TUI; this owns the crew.

The promise, in one line:

> You talk to one agent. It runs a crew of builders in parallel, and **nothing
> reaches your branch until your checks pass and an auditor signs off.**

Every rule below exists to keep that sentence true.

---

## 1. Roles

| Role | Runs | Window | Writes | Returns |
| --- | --- | --- | --- | --- |
| **Orchestrator** | your tree, conversational | the session transcript | project memory only | answers; tasks via `todo_write`; memory updates |
| **Architect** | your tree, one-shot | fresh: memory + brief | project memory only; never source | `notes/architect.md`, tasks, DECISIONS, a short summary |
| **Builder** | its own worktree, parallel | fresh: memory + its brief | product source in its owned paths | a structured handback |
| **Auditor** | the builder's worktree | fresh: brief + handback + checks + diff | nothing | findings + `VERDICT: PASS/FAIL` |

- The planner was folded into the architect. Two sequential read-and-write-a-note
  roles re-read the repo twice and lost detail at the handoff. `planner` still
  parses, and old sessions, logs, and saved crew rows still load.
- The orchestrator never writes product source. This is enforced by the tool mask,
  not by the prompt.
- **Project memory has serial writers only**: the orchestrator and the architect,
  both in your tree. Builders are denied `ROADMAP.md`, `DECISIONS.md` and `notes/`,
  because N parallel copies of those files conflict on every merge. Their decisions
  come back in the handback, and the orchestrator records them.

## 2. Phases are guardrails, not a pipeline

| Phase | Runs from the queue | Meaning |
| --- | --- | --- |
| `plan` | architect | Design. Nothing writes product source. (`/architect` and `/design` are aliases.) |
| `build` | builders, gated by checks + auditor | Make changes. |
| `audit` | auditor | Review what exists; report. |

The phase limits what *may* run. The orchestrator decides what *does* run.

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
 builder in worktree ──► commit
        │
        ▼
 integrate: merge YOUR branch into the worktree ──conflict──► resolver builder (in the worktree)
        │                                                          │ → must be re-audited
        ▼                                                          ▼
 gate ① checks: [auditor] checks, run by the harness ── fail ──► rejected → retry with the output
        │
        ▼
 gate ② auditor: brief + handback + check output + full diff ── FAIL ──► rejected → retry with findings
        │ VERDICT: PASS
        ▼
 land (serialized): your branch moved?  → integrate again, re-run checks
                    dirty file overlap? → keep branch, tell you which files
                    else git merge --no-ff (cannot conflict: branch already contains yours)
```

**Invariants.** Each one is pinned by a test.

| # | Invariant | Test |
| --- | --- | --- |
| C-01 | Nothing lands without a `VERDICT: PASS`. With the auditor off, work waits on its branch. | `without_an_auditor_nothing_lands` |
| C-02 | Checks run before the auditor; a failing check never costs an audit. | `a_failing_check_rejects_before_audit` |
| C-03 | The auditor sees everything landing would change, new files included. | `the_auditor_sees_new_files` |
| C-04 | Conflicts are resolved in the worktree; your checkout never holds a marker or a half-merge. | `conflicts_resolve_in_the_worktree_and_are_re_audited` |
| C-05 | Code written by a resolver is re-audited. | same |
| C-06 | Your uncommitted edits block a merge only if the task touched the same files, and they always survive. | `dirty_files_block_only_when_they_overlap` |
| C-07 | A task is one `--no-ff` merge commit, so `git revert -m 1` undoes it. | `builder_passes_the_gate_and_lands_as_one_commit` |
| C-08 | Builders cannot write project memory. | `only_serial_roles_write_project_memory` |
| C-09 | Disjoint scopes run together; overlapping or undeclared ones wait. | `disjoint_scopes_run_together_overlapping_ones_wait`, `unscoped_tasks_run_alone` |
| C-10 | Prompts state exactly what the runtime parses. | `prompts_state_the_contracts_the_runtime_parses` |
| C-11 | Tool calls reassemble from real streaming shapes on every backend. | `streamed_tool_calls_reassemble_on_every_backend` |

**Re-gating rule.** The checks re-run whenever the tree changes. The auditor
re-runs only when *the builder's own code* changed, meaning a resolver ran. A clean
merge of someone else's already-audited work does not change what this builder
wrote, and the checks catch semantic breakage between the two.

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
- **Crew → orchestrator:** after a batch, `drain_crew` returns a report (status,
  handback, checks, audit per task). The orchestrator gets another round to tell
  you what happened and to record decisions. The report is also kept in the session
  (`notes/crew.md`), so later turns can see it.
- **Orchestrator → you:** what merged, what was rejected and why, and which branches
  are waiting on you.

## 7. Open questions

These are product calls, not engineering ones. Recommendations are in
`docs/product-direction.md`.

1. **Name.** "Orchestrator" is accurate but reads as infrastructure. In a crew
   metaphor, *lead* is the human-sounding word. Recommendation: rename it in the UI
   only.
2. **Independent auditor by default.** Crew roles follow the orchestrator's model,
   so today the auditor is usually the same model as the builder, and a pass is not
   an independent opinion. Should setup require, or strongly default to, a different
   model or provider for the auditor?
3. **More than one auditor.** For example, a correctness auditor plus a security
   auditor for paths matching a pattern. Is the rule all-must-pass or a quorum?
4. **A fast path for trivial edits.** A typo fix currently costs builder + checks +
   audit. Should the orchestrator be allowed to *propose* a small edit that you
   approve with `y`? Your approval is a sign-off.
5. **Pull requests instead of local merges.** For teams, "land" should probably mean
   "open a PR with the audit as the description".
6. **Per-task budget.** A crew multiplies spend. Should a task stop at a cost cap
   the way a session does?
