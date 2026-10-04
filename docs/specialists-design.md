# Two rows on the hat rack: primary hats and specialists

Status: approved 2026-10-03 (mockup: https://claude.ai/artifact/QzpLi5PFs6mPrbMk3rd5zs). Built in patches; §15 records where the build differs.

This contract extends `docs/hat-rack-design.md`. Where the two disagree, this one wins.

## 1. What changes, and why

Plan and build are the work; everything else is a tool the user reaches for. The rack shows that: plan and build above a hairline, the specialists below it. Tab moves within the row the user is in, Shift+Tab moves between rows, so the primary cycle never grows when a specialist is added. The review hat becomes the **audit** hat: it reads, runs the project's own tests and its end-to-end flows, starts the product through the run file, changes nothing, and ends in a popout the user acts on. A new **scribe** hat writes documentation only, on whatever model the user gives it. The approved plan lives at a fixed path every hat knows. The only hat change Ryter makes on its own is plan into build, when the plan is written.

Why: the first real sessions on three hats showed that a hat earns its place by a different permission profile or different model economics, not by a different brief. Audit and scribe have both. The test hat had neither, and went ([[DECISIONS.md, 2026-10-03]]). The field (OpenCode's build and plan agents, Claude Code's plan mode) has two primary modes and reaches for the rest.

## 2. Hats and rows

- **R-HAT-01** Four hats: `plan`, `build` (the primary row), `audit`, `scribe` (the specialist row). `Role::SoloReview` is renamed `Role::SoloAudit` with `#[serde(alias = "review")]`, so saved sessions, spend records and event logs still load. `Role::SoloScribe` is new.
- **R-HAT-02** `Role::row()` is `Row::Primary` or `Row::Specialist`. `Role::next_in_row()` cycles within the row: plan ↔ build; audit → scribe → audit. `Role::Crew` (saved crew sessions, the alias `test`) is build.
- **R-HAT-03** A hat's permission profile, prompt section, color and model seat are its own; adding a hat touches those four places and the rack. Nothing else enumerates hats.

## 3. Keys

- **R-KEY-01** `Tab`: the next hat in the current row. In the primary row that is the other primary hat.
- **R-KEY-02** `Shift+Tab`: the other row. Into the specialist row it puts on the specialist last worn this session, audit when none; back into the primary row, the primary last worn, build when none.
- **R-KEY-03** `/plan`, `/build`, `/audit`, `/scribe` jump straight to a hat from either row. `/review` is kept for one release as `/audit` with a notice.
- **R-KEY-04** The composer's hint and the foot line name both keys for where the user is: `tab build · ⇧tab specialists` in the primary row, `tab scribe · ⇧tab plan · build` in the specialist row.
- **R-KEY-05** Panels keep their own Tab meaning; these keys apply to the solo screen with no panel open, as today.

## 4. Top bar

- **R-TOP-01** `◆ PLAN   ○ BUILD   ·   ○ AUDIT   ○ SCRIBE`: the primary row, a middle dot, the specialist row. Marks as today: `◆` on, `●` worn this session, `○` not yet. The dot is in the hairline color.
- **R-TOP-02** Under 132 columns the row the user is not in folds to its name and a count of hats worn, `specialists ·1`; under 100 the bar shows the current row only.

## 5. Hat rack

- **R-RACK-01** Blocks in order: plan, build, a separator row, audit, scribe. The separator is a hairline across the column with the word `specialists` in the dim color at its right end.
- **R-RACK-02** Plan and build blocks are unchanged from `hat-rack-design.md` §5.
- **R-RACK-03** The audit block: model; turns and spend; `audits   N fail   N pass` counting verdicts this session; a fourth row for the last audit's effect on the tree, `changed nothing` or `restored N files`.
- **R-RACK-04** The scribe block: model; turns and spend; `docs   N written`, the documentation files it changed this session, each counted once.
- **R-RACK-05** The rack stays a fixed height. Under 132 columns it folds as today and `^b` opens it as a panel, with the separator kept.

## 6. Instruments

- **R-INST-01** The `GUARD` card gains two rows: `plan.md` with the approved plan's date and slug, or `none`; `audit.md` with the latest audit's date, `writing…` during an audit, or `none`.
- **R-INST-02** `this hat` in the guard card reads, for the audit hat, `checkpoint, restored` (the mechanism, §8); for the scribe, `docs only`.

## 7. Color

- **R-COL-01** Plan cyan, build green, audit amber (review's color), scribe violet (the theme's `architect` slot, free since the test hat went). The hat colors the chip, the prompt, the model's name, the context gauge, its rack block and the watermark, as today.

## 8. The audit hat

- **R-AUD-01** Tools: the build hat's list. `write` and `search_replace` are refused for everything but `.ryter/audit.md` and `.ryter/audits/`, and scratch space. `bash` and `run_project` are judged by the build hat's rule: toolchains, scripts, the product, curl, browser drivers, whatever the end-to-end suite needs. The fixed refusals hold (secrets, privilege, a shell handed a string).
- **R-AUD-02** It changes nothing in the project, enforced by the tree, not by a list: Ryter takes a checkpoint before an audit turn (`git::checkpoint`, as before a build turn), compares the tree after, and if it moved restores the checkpoint (`git::restore_checkpoint`) and records the paths. The audit's own files under `.ryter/` are left as written. A project that is not a repository gets no checkpoint; the audit says so and runs read-only commands only.
- **R-AUD-03** An audit turn ends with the `file_audit` tool, once, as its last call: `verdict` (`pass` | `fail`), `summary` (one line), `findings` (ordered, worst first, each `result` `fail` | `pass` | `not_reached`, `title`, optional `where` (path:line), `detail`, `saw` (what was run and what came back)), `ran` (the commands and tools it used). Ryter renders it to Markdown, writes `.ryter/audit.md` (replaced each time) and `.ryter/audits/<date>-<slug>.md` (kept), and emits `AgentEvent::Audited { model, verdict, headline, rows, file, restored, total_usd, duration_ms }`. A turn that ends without the call is noted in the chat; the hat is not punished further.
- **R-AUD-04** The popout: on `Audited` the TUI opens the audit card over the whole body, like the plan panel: the title bar `audit · <model> · ✗ 2 of 6 failed · 1 turn · $ · time`, the verdict and summary, the findings (a failure opened out with where, detail and what was seen; a pass one line; not-reached with why), then `ran`, `changed nothing` or `restored N files` with the paths, and the files written. Keys: `y` repair in build, `n` close, `o` open `audit.md` in the pages viewer, `↑↓`, `PgUp/PgDn` and the mouse wheel scroll, `esc` close.
- **R-AUD-05** `y` puts on the build hat and starts a turn whose user message is the audit (the file's text) under a one-line brief, "Repair what this audit found; run the checks it ran." That switch is the user's, not Ryter's.
- **R-AUD-06** `/audit` runs an audit on demand from any hat: the audit hat is put on for the turn, the brief is today's review brief (the plan, the decisions, the diff since the plan's base, the run file) plus "run everything the project has", and the hat the user was in comes back when the turn ends, popout open. From the audit hat itself, a plain message is an audit turn too.
- **R-AUD-07** The offer of a review after a build turn is removed, and so is `offer_audit`; an old settings file with it loads silently. The one automatic hat change is §9.
- **R-AUD-08** The spend limit `review_usd` applies to an audit turn under the name `audit_usd`, with `review_usd` read as an alias.
- **R-AUD-09** The builder's prompt says: when `.ryter/audit.md` exists and is newer than the plan's approval, read it before changing anything, and say which findings the work addresses.

## 9. The plan file

- **R-PLAN-01** Approving a plan writes it to `.ryter/plan.md` as well as `.ryter/plans/<date>-<slug>.md`, and that write is the one hat change Ryter makes on its own: plan into build, in the same turn, as today.
- **R-PLAN-02** Every hat's prompt knows `.ryter/plan.md`: the build works from it, the audit judges against it, the scribe documents it. A resumed session with no plan on record reads it when it is there.
- **R-PLAN-03** The plan popout scrolls with the mouse wheel as well as the keys. So does every popout that scrolls.

## 10. The scribe hat

- **R-SCR-01** Reads everything but secrets. Runs read-only commands, by the plan hat's rule. `write` and `search_replace` are allowed for documentation files anywhere in the project: `.md`, `.mdx`, `.txt`, `.rst`, `.adoc`, and `LICENSE`, `CHANGELOG`, `README` with no extension; refused for everything else, with the hint "the scribe writes documentation; press Tab to build for code". Never `.ryter/` secrets or the run file.
- **R-SCR-02** Prompt: write for the reader of the project, from the code and the plan; never invent behavior; say what was read.
- **R-SCR-03** Its own model seat in `/models` and `hats.toml` (`[scribe]`), as every hat has.

## 11. State

- **R-STATE-01** The TUI remembers the last primary and the last specialist worn in the session (view state, not saved). `start_hat = "last"` is unchanged.
- **R-STATE-02** `AgentEvent::Reviewed` stays for old logs and is read as an audit with no file; new audits emit `Audited`.

## 12. Testing contract

- Role: row and cycle for every hat; `review` and `test` aliases load; `Crew` is build.
- Keys: Tab within each row, Shift+Tab between rows with the remembered hat, from each of the four hats; a panel open leaves them alone.
- Rack: the separator row; each block's rows; the fixed height; the fold and the `^b` panel; snapshots at 160×50 for each hat.
- Audit: the tool's shape refused and accepted; the Markdown rendering; both files written; the event; the checkpoint taken, the tree restored when it moved and left when it didn't, the paths recorded; a project with no repository; `/audit` from each hat putting the user's hat back; `y` starting the repair turn in build with the audit as the message; the popout scrolling by key and wheel, `o`, `n`, `esc`; the review offer gone; an old `offer_audit` key loading silently.
- Plan: `plan.md` written on approval and replaced by the next; the popout's wheel scrolling.
- Scribe: each allowed extension written, code refused with the hint, read-only commands run, a write to `.ryter/run.toml` refused.
- Gate: nothing loosened for plan; the audit hat's bash judged as build's; its `write` refused outside its files.

## 13. Documentation that changes

`docs/guide.md` (hats table, keys, `/audit`, the audit file, the plan file, the scribe), `README.md`, `config.example.toml` (`[specialists.audit]`, `[specialists.scribe]`, `audit_usd`), `prompts/solo.md`, `docs/hat-rack-design.md` §18 (pointer here), `DECISIONS.md`, `ROADMAP.md`.

## 14. Non-goals

- The builder delegating a task to a specialist. A later decision of its own.
- A scout hat. The first candidate when the need shows.
- Changing what the plan and build hats may do.

## 15. What changed in the building

Patch 1 (rows, keys, review → audit, wheel scrolling), 2026-10-03:

- **R-KEY-01** With one specialist, `Tab` in the specialist row stays on audit; it goes round once the scribe exists.
- **R-KEY-03** There is no command that merely puts the audit hat on: `/audit` runs an audit (its behavior before this patch), `/review` is hidden and runs the same with a notice, and the hat is reached by `Shift+Tab`. `/plan` and `/build` jump as before.
- **R-PLAN-03** A panel that is open takes the wheel wherever the pointer is, not only over the panel: the panel owns the screen while it is open, and the chat under it never scrolled by the wheel then either.
- **R-RACK-03** The audit block's row is `audits ✗ N fail  ✓ N pass`, counts alone when both happened, as the other blocks do; the fourth row (`changed nothing` / `restored N files`) comes with patch 2.
- **R-INST-01** The guard card's `plan.md` and `audit.md` rows show the file's day and its first heading's first words, or `none`. The files themselves are written from patch 2 and 3; today the rows read `none` unless the user has made them.
- **R-TOP-02** Between 100 and 131 columns the row not in use folds to its name and the count of its hats worn (`specialists ·1`, `plan · build ·2`); under 100 only the current row is on the bar.
- `review_usd` keeps its name in the config file; the settings row is labelled *audit usd* (R-AUD-08 is patch 2).

Patch 2 (the audit hat), 2026-10-03:

- **R-AUD-01** Git is the exception to "the build hat's rule": a commit, a branch change or a push is not undone by putting files back, so git stays read-only for the audit hat (`status`, `diff`, `log`, `stash list`/`show`). Destruction (`rm`, `find -delete`, a stack's volumes) asks as it does for the build hat, since the checkpoint doesn't reach what git ignores.
- **R-AUD-02** The gate's tests keep the review hat's old answers for the audit hat *without* a checkpoint (`ctx.read_only`, the no-repository case), and one test asserts the build hat's answers behind a checkpoint. Checkpoints are compared by tree; what moved is restored path by path (`git::restore_paths`), the audit's own files excepted.
- **R-AUD-03** The event is `Audited { model, verdict, headline, summary, rows, ran, file, restored, checkpointed, filed, total_usd, duration_ms }`; `rows` are `mark n\ttitle\tright` with indented detail rows, which the panel lays out. An audit turn that neither filed, was asked for (`[Ryter] Audit …`) nor gave a `VERDICT:` line is a chat in the audit hat: no notice, no event, and a changed tree is still put back with a notice. An unfiled audit emits `Audited { filed: false }` with the verdict of its last words, and `/audit` still emits `Reviewed` after it, with the tree, for the commit receipt; the rack counts a turn's verdict once.
- **R-AUD-04** `o` opens the file with the system opener (`page::open`); the legend drops it when no file was written. The popout's width is 100 columns.
- **R-AUD-05** The repair brief is the audit file's text under "Repair what this audit found; run the checks it ran."; with no file, a line saying the audit was in the chat.
- **R-AUD-06** `/audit`'s cost confirmation card is kept; its `s stop offering` key went with the offers.
- **R-AUD-07** `offer_audit`/`offer_review` stay in `UI_KEYS` so old files load without a warning.
- **R-AUD-08** `[spend] review_usd` is read as `audit_usd` in the config file (the key is renamed before the table merge) and in `settings.toml`.
- **R-INST-02** `this hat` reads `checkpoint, restored` for the audit hat (`checkpoint` when condensed).
- **R-RACK-03** The fourth row is `tree   changed nothing` or `tree   restored N files`, from the last `Audited` event.

Patch 3 (the plan file), 2026-10-03:

- **R-PLAN-01** `plan::save_on` writes the dated copy and `.ryter/plan.md` with the same text; the approval notice names both. Headless, a plan can't be approved at all (there is nobody to show it to), so the headless path has nothing to write.
- **R-PLAN-02** The pickup happens at the first turn of a session with no plan on record: `plan::on_record` prefers the dated copy whose text matches `plan.md` (so the decisions recorded under it are found) and falls back to `plan.md` itself. The prompt's "Ryter's files in the project" section is one for every hat; the build hat's audit.md bullet moved into it.
- **Automatic hat changes:** the only ones found are a plan's approval (kept), `request_hat` after the user's yes, the audit turn putting the audit hat on for `/audit` and the user's hat back after it (a round trip, R-AUD-06), and a saved crew-mode or test-hat session opening in build. None was removed.
- **R-PLAN-03** The audit card takes the wheel as the plan panel does; a test covers it.

After the release PR's review (2026-10-03):

- **R-AUD-02** The rollback leaves everything under this session's `.ryter/` alone, not only the audit's files: a run file or a plan the user approved during the turn stays (`audit::kept_from_restore`). Git names paths from the repository's top; they are mapped to the session's folder before the test, so a session below the top keeps its own `.ryter/` and puts back the rest. When the user's yes to `request_hat`, or to a plan, takes the turn out of the audit hat, the audit closes at that moment (`Agent::close_audit`): tree put back, audit filed, card opened, and the hat that follows is not undone at the turn's end. A yes that puts the audit hat on in the middle of a turn arms an audit phase of its own, with a checkpoint, closed at the turn's end or at the next switch out. The card can therefore open while the turn goes on in the build hat; a `y` then queues the repair after it. A tree that cannot be compared with its checkpoint is put back whole, with a notice; a checkpoint that cannot be taken says why and holds the audit to read-only commands.
- **R-AUD-04** A chord (`Ctrl`/`Alt` with `y`, `n`, `o`) does nothing on the card; its `y` is ignored for the first half second, as every card's confirm key is.
- The project spend's hats' share counts audit, review and scribe.

Patch 4 (the scribe hat), 2026-10-03:

- **R-SCR-01** The documentation rule is by name alone: the extensions `md`, `mdx`, `txt`, `rst`, `adoc` in any case, and the bare names `README`, `CHANGELOG`, `LICENSE`, `CONTRIBUTING`, `NOTICE`, `AUTHORS`. The secret rule runs first, so `.env`, `*.pem` and the dotenv family are refused whatever they end in: `.env.md` is refused, as a secret, not written as a document. Everything under `.ryter/` is refused except the session's notes, which every hat may write. `ROADMAP.md` and `DECISIONS.md` are documentation, so the scribe writes them as the plan and build hats do.
- **R-SCR-03** The scribe is the last seat in `/models`; `start_hat` does not take `scribe` (a session starts in a primary hat or an audit), but `last` may open in it.
- **R-RACK-04** `docs   N written` counts the files the scribe's edits touched, each once, from the same tool results the build block counts; the block has no lines row.
- **R-KEY-03** `/scribe` (alias `/docs`) puts the hat on, as `/plan` and `/build` do; the composer's hint in the audit hat now names `Tab: scribe`.
- The models panel's "last seat" tests moved from audit to scribe; the rack without figures is twenty rows with four blocks, so the height at which it drops its figures moved up by four rows.

