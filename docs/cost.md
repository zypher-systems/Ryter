# What a crew costs

The worry: parallel builders, multiple models, and auditors on every task will
cost several times what a single agent costs, and nobody will pay that however
good the results are. This document models the cost honestly, says what changed
on 2026-09-21, and names the levers that are left.

**Short version.** With one model in every role, a crew costs about **1.5× a
single agent** per task: design and review aren't free. The design pays off only
through **role tiering**. The builder burns ~70% of the tokens, so running it on
a cheap model while a strong model designs and reviews brings a task to about
**0.35–0.55× a strong single agent**. Against a single agent that *also* runs the
cheap model, a crew costs about **2×**, and what that buys is a strong model's
design and review. Whether that buys strong-model quality is the question the
task benchmark has to answer (`ROADMAP.md`).

---

## 1. Where the money went before today

| Problem | Effect |
| --- | --- |
| **Crew spend wasn't metered.** `run_specialist` discarded every usage delta. | Builders, auditors, and the architect were missing from the spend total, the spend card, and the budget stop. The budget "real stop" only ever saw the lead. |
| **Every specialist got all project memory on every round.** | ~10,200 tokens of ROADMAP + DECISIONS + notes per round on this repo, before any work, and growing with every decision recorded. A 40-round builder resent ~400k tokens of it. |
| **Only the system prompt and tools were cacheable on Anthropic.** | The growing conversation, most of the input, was billed in full every round. |
| **The lead rebuilt its system prompt every round**, and it embeds memory the same turn edits. | Every memory edit changed the prompt prefix and threw away the provider's cache. |
| **A rejected task was rebuilt from scratch.** | A rejection cost roughly a second full build. |
| **An auditor got the builder's limits** (40 rounds, 16k output). | Headroom for a runaway review. |

## 2. What changed

| Change | Where |
| --- | --- |
| Every specialist round is priced, attributed to task and role, logged, and counted against the session budget. | `meter.rs`, `crew.rs` `run_specialist`, `agent.rs` `record_crew_spend` |
| Per-task caps: `[spend] task_budget_usd = 1.0`, `task_max_tokens = 1_000_000`. The token cap also stops unpriced models. A capped task keeps its branch. | `meter.rs` |
| Scoped context: builders and auditors get `notes/architect.md` plus the DECISIONS entries that name their files. Measured on this repo: **~10,200 → ~1,000–2,000 tokens per round.** | `memory.rs` `load_scoped_memory` |
| Rolling cache breakpoint on the newest message (Anthropic, and Claude via OpenRouter). | `llm/http.rs` |
| The lead's system prompt is built once per turn, and the crew report is no longer copied into it. | `agent.rs` `turn_inner`, `prompt.rs` |
| Retry in place: a rejected task fixes its own worktree. | `crew.rs`, `git.rs` `open_worktree` |
| Auditors are limited to 12 rounds and 4k output tokens. | `crew.rs` `limits` |
| The auditor panel runs in order and stops at the first FAIL, and path-scoped seats skip unrelated changes. Put the cheap reviewer first. | `crew.rs` `sign_off` |
| Checks run before any audit, so a broken build never costs a review. (Already true; still the cheapest gate.) | `crew.rs` `run_checks` |
| The fast path (`propose_edit`): a trivial edit costs one lead call, not a crew run. | `tools/`, `prompts/orchestrator.md` |

## 3. The model

Per task, prices per million tokens. The strong model **S** uses grok-4.6's rates
as shipped in `spend.rs`: $2.00 input, $0.50 cached, $6.00 output. Every other
model is a fraction *k* of S. Substitute your own rates.

Assumptions, all adjustable:

- A build is 20 rounds. Context grows 3k tokens per round; output is 400 tokens
  per round. The base prompt (system, tools, RYTER.md) is 8k.
- Caching: everything except the new tokens each round is a cache read.
- The auditor makes 2 rounds over the brief, handback, and a ~6k-token diff.
- The architect makes 10 rounds, amortized over 4 tasks. The lead spends
  3 rounds per task.
- 30% of tasks are rejected once. A retry in place costs ~25% of a build (before
  today: 100%).

| Scenario (per task) | Cost | vs. strong single agent |
| --- | --- | --- |
| Single agent, strong model, fresh task | $0.52 | 1.0× |
| Single agent, strong model, 5th task of a long session (~100k carried context) | $1.67 | 3.2× |
| **Ryter before today**, one model, Anthropic (conversation uncached) | $2.73 | **5.3×** |
| **Ryter before today**, one model, provider auto-caching | $1.03 | **2.0×** |
| Ryter now, one model for every role | $0.76 | 1.5× |
| Ryter now, builder + lead at ¼ the price, strong auditor + architect | $0.28 | 0.55× |
| Ryter now, builder + lead at ⅒ the price | $0.19 | 0.36× |
| Ryter now, builder on a local model (≈ free), strong auditor + architect | $0.12 | 0.24× |

Where a tiered task's money goes (¼-price builder): builder $0.13, architect
$0.07, auditor $0.05, lead $0.02, retries $0.01. The auditor and architect are a
**fixed quality tax of ~$0.12 per task**, independent of the builder's price.

Two things this model shows that are easy to miss:

1. **Parallelism doesn't cost tokens.** Four tasks cost the same run in parallel
   or in sequence. Parallelism buys wall-clock time. What costs is the fresh window
   each builder starts with, plus review. Both are now small.
2. **Isolation is a cost advantage, not only a safety one.** A single agent's
   later tasks carry the earlier ones' context (the 3.2× row). Compaction limits
   that, so treat the row as an upper range. A Ryter builder starts every task at
   the same small size.

<details>
<summary>The model, to rerun with your own rates</summary>

```python
S = dict(inp=2.00, cached=0.50, out=6.00)          # $/M tokens, strong model

def cost(uncached, cached, out, k=1.0):             # k = price as a fraction of S
    return k * (uncached*S["inp"] + cached*S["cached"] + out*S["out"]) / 1e6

def run(c0, rounds, growth, out_per_round, cache=True):
    total = sum(c0 + growth*i for i in range(rounds))
    uncached = c0 + growth*rounds if cache else total
    return uncached, total - uncached, out_per_round*rounds

BASE, R, G, O = 8000, 20, 3000, 400
single = cost(*run(BASE, R, G, O))

def ryter(memory, cached, builder_k, strong_k, retry_frac, arch_share=4):
    b     = cost(*run(BASE+memory, R, G, O, cached), builder_k)
    audit = cost(*run(BASE+memory+6000, 2, 500, 700, cached), strong_k)
    arch  = cost(*run(BASE+10_400, 10, 3000, 1000), strong_k) / arch_share
    lead  = cost(*run(BASE+10_400, 3, 1500, 300), builder_k)
    return b + audit + arch + lead + 0.3*retry_frac*b

print(single, ryter(1_500, True, 0.25, 1, 0.25))    # now, tiered
```
</details>

## 4. The levers that remain, by size

1. **Default to tiering.** Setup should put builders on the cheapest model that
   passes your checks, and the auditor and architect on a strong model from a
   *different* provider. Independence is already required, so setup has to ask for
   two models anyway. Make the cheap-builder / strong-reviewer split the default,
   not something the user has to discover.
2. **Local builders.** A `local` connection (`--kind ollama`, `lmstudio`, or
   `llamacpp`) makes the builder nearly free, and `crew suggest` picks it. What's left is the quality tax, ~$0.12 per task in the
   model above. Test this against the benchmark before recommending it.
3. **Fewer builder rounds through better briefs.** Rounds are the multiplier in
   every row. The architect has already read the relevant code, so it should put
   paths, symbols, and the exact acceptance check in each brief so builders don't
   rediscover them. Measure rounds per task in the benchmark.
4. **Skip the architect for small requests.** The lead can write tasks itself.
   The architect's ~$0.07 share is worth paying for multi-file designs, not for
   "add a flag".
5. **A cost preview and a batch threshold.** Before a batch runs, show the task
   count and an estimate (from the benchmark and the meter's history), and ask
   when it exceeds a threshold. The meter already enforces the ceilings; a preview
   removes surprise.
6. **Make crew cost visible where people look.** The meter emits per-role spend
   events; the spend card and `/spend` should show builder, auditor, and architect
   separately, and the crew review surface should show per-task cost.

What not to do: batch audits across a patch to save money. Per-task review is
what lets one bad task be rejected without discarding the others, and at ~$0.05 a
task it is the cheapest part of the pipeline.

## 5. How to know

The meter now produces the real numbers: cost per task, per role, and per model,
in `spend.jsonl`. `ryter bench` turns them into the figure that decides whether
the product works (`ryter bench --crew <preset>` to compare tierings):

> **Cost per landed task, at a given land rate.**

Compare it against a single agent on the same tasks, at each tiering. If a
cheap-builder crew lands as much as a strong single agent at half the cost, that's
the pitch. If it doesn't, the benchmark will show which role needs the stronger
model.
