# Ryter TUI: the hat rack screen (design contract)

Status: **built** on `hat-rack-patch` (2026-10-02). Where the build differs from the first
draft of this contract, the requirement below was rewritten to say what was built, and §18
lists each change with its reason.
Designed with the user on 2026-10-02, from mockups reviewed over three rounds.
Applies to: `crates/ryter-tui`, with small listed additions to `crates/ryter-core` and `crates/ryter-cli`.
Work branch: `hat-rack-patch`, cut from `dev`.

This document is a **contract**. Every requirement is numbered (`R-<AREA>-<NN>`). The work
is done when every requirement is met and the tests in §13 pass. Where it conflicts with
the current ledger screen, this document wins. It does not change the classic layout, the
workbench (`^t`), the command palette, or any popout panel beyond their colors.

`design.md` at the repository root is the earlier contract for the classic screen and is
kept as history.

---

## 1. What changes, and why

The ledger screen works, but it tells the user little at a glance and has no identity.
This redesign keeps the ledger's reading column and timeline and changes what surrounds it.

| Today (ledger) | After |
| --- | --- |
| One rail on the left: name, session, hat, model, context, cost, last changes | Two side columns: the **hat rack** on the left, **instruments** on the right |
| The hat is a word in the rail and a chip on the message box | The hat sets the accent color of the whole screen, and a fedora watermark sits behind the transcript in that color |
| Spend is one session figure | Each hat tracks its own spend in the rack; the right column shows session and project |
| A new session starts in the build hat | A new session starts in the **plan** hat, with a setting to choose |
| A views strip across the top (`chat`, `changes ^t`) | A top bar that names the hats and shows which have been worn |

Hats are the product's callsign. The screen should say which hat is on without the user
reading a word.

---

## 2. Principles

1. **The transcript is the product.** It gets the width. Side columns give way first.
2. **The hat is the color.** One accent, set by the active hat. Everything else is neutral
   and does not change on `Tab`.
3. **Nothing in a side column grows with the session.** The hat rack is one fixed block per hat.
   The list of turns is the transcript.
4. **Each fact appears once.** Per-hat figures are in the rack. Session and project
   figures are in the instruments. The top bar does not repeat either.
5. **Degrade, never break.** 80×24, sixteen colors and no color are all supported, with
   the reductions written down in §9.

---

## 3. Layout

### 3.1 Regions

| Region | Size | Content |
| --- | --- | --- |
| Top bar | 1 row + 1 hairline | name, the hats, working folder and branch |
| Hat rack | 30 columns, left | one block per hat (§5) |
| Transcript | the rest | the ledger's reading column, with the watermark behind it (§7) |
| Instruments | 34 columns, right (30 at mid width) | model, context, spend, guard, changes (§6) |
| Composer | 1 hairline + 1 to 8 text rows + 1 blank | hat chip and the message box, the width of the transcript column |
| Hint bar | 1 row | context-sensitive keys; the status line at narrow width |

- **R-LAYOUT-01** The composer and the hint bar are always on screen. No column, panel or
  overlay reduces them.
- **R-LAYOUT-02** Side columns are separated from the transcript by a one-cell vertical
  hairline. They have no outer box. They run from under the top bar to the blank row
  above the hint bar: the composer is the transcript column's, the same width as it, and
  the side columns continue beside it.
- **R-LAYOUT-03** The hat rack and instruments have a panel background one step lighter
  than the screen background. The transcript uses the screen background.

### 3.2 Width tiers

| Terminal width | Hat rack | Instruments | Top bar | Hint bar |
| --- | --- | --- | --- | --- |
| `>= 132` | shown, 30 | shown, 34 | hat names | keys |
| `100..=131` | hidden | shown, 30, condensed | hat names with turn counts | keys |
| `< 100` | hidden | hidden | hat names with turn counts | status line (§6.3) |

- **R-LAYOUT-04** The tiers above are decided by terminal width alone. These replace
  `RAIL_W` and `RAIL_MIN_SCREEN` in `rail.rs`.
- **R-LAYOUT-05** `Ctrl+B` keeps its binding. At `>= 132` it hides and shows both side
  columns together. Below that, where one or both columns are folded away, it opens the
  hat rack and instruments as one popout panel over the transcript; `Esc`, or `Ctrl+B`
  again, closes it. No new key is added. In the panel the rack and the instruments are
  side by side (stacked on a screen too narrow for that), and `↑↓` scroll it on a screen
  too short to show it whole.
- **R-LAYOUT-06** `[ui] panel = false` starts with the side columns hidden, as today.
- **R-LAYOUT-07** When the body is shorter than the rack needs (23 rows), each rack block
  drops its hat-specific rows first (§5.3). If it still does not fit, the rack is hidden
  as it is below 132 columns.
- **R-LAYOUT-08** A popout wider than the transcript column (`/models`, `/settings`) takes
  the whole body, and the side columns are not drawn under it. One that fits the column
  (a plan to approve, a permission prompt, the hat rack panel) leaves them in sight.

### 3.3 Wireframe: 132×32, build hat

```
 RYTER │ ● PLAN   ◆ BUILD   ● REVIEW   ○ TEST                                                       ~/workspace/shop · search-patch
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
                             │                                                                    │
 HAT RACK                    │   ● empty-query fix ·················· plan approved · $0.004  ▸   │ MODEL
                             │   ● build the plan ················· 2 files · +8 −1 · $0.012  ▸   │ deepseek-pro-latest
 ● PLAN                      │   ● review ···························· ✗ 1 blocking · $0.040  ▸   │ connection         openrouter ●
   grok-4.6                  │                                                                    │ reasoning                medium
   1 turn             $0.004 │   ● dusty                                                  19:47   │
   plans          1 approved │   │ fix the blocking finding                                       │ CONTEXT
                             │                                                                    │ ━━━━━━━━━━──────────────── 38%
 ◆ BUILD                     │   ◆ deepseek-pro-latest · build hat                        19:47   │ 97k / 256k tokens
   deepseek-pro-latest       │   │                                                                │
   2 turns            $0.021 │   ├─ edit app/server.js ···························· +1 −1  0.0s   │ SPEND
   files                   2 │   │   5 -   const q = (req.query.q || '').trim()                   │ session                  $0.065
   lines              +13 −2 │   │   5 +   const q = String(req.query.q ?? '').trim()             │ project                   $4.82
   tests         ✓ 14 passed │   ├─ edit test/search.test.js ························· +4  0.0s   │ budget                      off
                             │   │   7 +                                                          │
 ● REVIEW                    │   │   8 + test('a missing query finds nothing', async () => {      │ GUARD
   claude-opus-5.5           │   │   9 +   assert.deepEqual(await search(), [])                   │ sandbox               workspace
   1 turn             $0.040 │   │  10 + })                                                       │ this hat        edits ask first
   verdicts         ✗ 1 fail │   ├─ run  npm test ··························· ✓ 14 passed  2.1s   │
                             │   └─ ✓ 3 tools · 2 files · +5 −1 · 4.4s · $0.009                   │ CHANGES             uncommitted
 ○ TEST                      │                                                                    │ app/server.js             +2 −1
   grok-4.6                  │   ┌────────────────────────────────────────────────────────────┐   │ test/search.test.js         new
   not worn yet              │   │ Review again with claude-opus-5.5? about $0.04             │   │ tests               ✓ 14 passed
                             │   │ y review  n skip                                           │   │
                             │   └────────────────────────────────────────────────────────────┘   │
                             │                                                                    │
────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────────
 BUILD  › what should change? · Tab: review · / for commands

 y review    n skip    tab review    ^t changes    / commands    ^b panels    ^c quit
```

The hairline above the composer, the `BUILD` chip, the `◆ BUILD` chip in the top bar, the
active block in the rack, the model name and the context gauge are in the build hat's
color. The fedora watermark is behind the transcript and is not drawn here.

### 3.4 Wireframe: 100×30, build hat

```
 RYTER │ ● PLAN 1   ◆ BUILD 2   ● REVIEW 1   ○ TEST                             shop · search-patch
────────────────────────────────────────────────────────────────────────────────────────────────────
                                                                      │
   ● review ······························ ✗ 1 blocking · $0.040  ▸   │ MODEL
                                                                      │ build          deepseek-pro
   ● dusty                                                    19:47   │
   │ fix the blocking finding                                         │ CONTEXT
                                                                      │ ━━━━━━━━──────────── 38%
   ◆ deepseek-pro-latest · build hat                          19:47   │ 97k / 256k tokens
   ├─ edit app/server.js ···································· +1 −1   │
   │   5 -   const q = (req.query.q || '').trim()                     │ SPEND
   │   5 +   const q = String(req.query.q ?? '').trim()               │ session              $0.065
   ├─ edit test/search.test.js ······························· +4 ▸   │ project               $4.82
   ├─ run  npm test ··································· ✓ 14 passed   │ budget                  off
   └─ ✓ 3 tools · 2 files · +5 −1 · 4.4s · $0.009                     │
                                                                      │ GUARD
   ┌──────────────────────────────────────────────────────────────┐   │ sandbox           workspace
   │ Review again with claude-opus-5.5? about $0.04               │   │ this hat         asks first
   │ y review  n skip                                             │   │
   └──────────────────────────────────────────────────────────────┘   │ CHANGES
                                                                      │ 2 files               +9 −1
                                                                      │ tests           ✓ 14 passed
                                                                      │
                                                                      │
                                                                      │
────────────────────────────────────────────────────────────────────────────────────────────────────
 BUILD  › what should change? · Tab: review · / for commands

 y review   n skip   tab review   ^b hat rack   / commands   ^c quit
```

### 3.5 Wireframe: 80×24, build hat

```
 RYTER │ ● PLAN 1  ◆ BUILD 2  ● REVIEW 1  ○ TEST                   search-patch

  ● review ·········································· ✗ 1 blocking · $0.040  ▸

  ● dusty                                                                19:47
  │ fix the blocking finding

  ◆ deepseek-pro-latest · build hat                                      19:47
  ├─ edit app/server.js ················································ +1 −1
  │   5 -   const q = (req.query.q || '').trim()
  │   5 +   const q = String(req.query.q ?? '').trim()
  ├─ edit test/search.test.js ··········································· +4 ▸
  ├─ run  npm test ··············································· ✓ 14 passed
  └─ ✓ 3 tools · 2 files · +5 −1 · 4.4s · $0.009

  Review again with claude-opus-5.5? about $0.04             y review  n skip




────────────────────────────────────────────────────────────────────────────────
 BUILD  › what should change? · Tab: review

 deepseek-pro  ctx ━━━───── 38% │ $0.065 · budget off   ^b hat rack  / commands
```

At this size the top bar has no hairline under it, and offers are one line, not a box.

---

## 4. Top bar

```
 RYTER │ ● PLAN   ◆ BUILD   ● REVIEW   ○ TEST                    ~/workspace/shop · search-patch
```

- **R-TOP-01** Left: `RYTER` in bold, a divider, then the hats in `Tab` order: plan,
  build, review. (Four until 2026-10-03, when the test hat was removed; §18.)
- **R-TOP-02** The active hat is a filled chip in its color with dark text, marked `◆`.
- **R-TOP-03** A hat that has had at least one turn this session shows `●` in its own
  color and its name in the body color. A hat with no turns shows `○` and its name dim.
- **R-TOP-04** The bar states which hats have been worn. It does not imply an order: no
  connectors, arrows or step numbers between hats, and no check marks.
- **R-TOP-05** When the hat rack is not on screen (below 132 columns, or hidden), each
  worn hat's name is followed by its turn count. When the rack is on screen, the counts
  are omitted.
- **R-TOP-06** Right: the working folder and the branch, dim. The folder shortens to its
  last component below 132 columns and is dropped below 100. A long path keeps its end
  and takes at most 48 columns.
- **R-TOP-08** The session's title sits between the hats and the folder, dim and centered
  in the room there, at 100 columns and wider when at least 16 columns are free.
- **R-TOP-07** The top bar replaces the ledger's views strip. While the workbench is open
  the bar's left side reads as the workbench does today (`chat esc`, `changes ^t`).

---

## 5. Hat rack (left column)

The rack is one block per hat (three since 2026-10-03), always in the same order and always the same height.
It never lists turns.

### 5.1 A block

```
 ◆ BUILD                        marker and name
   deepseek-pro-latest          the hat's model, dim
   2 turns            $0.021    turns this session, and what the hat has cost
   files                   2    hat-specific rows (§5.3)
   lines              +13 −2
   tests         ✓ 14 passed
```

- **R-RACK-01** The column is headed `HAT RACK`, dim and letter-spaced.
- **R-RACK-02** Every block shows the marker and name, the hat's model, its turn count and
  its spend. The marker follows the top bar: `◆` active, `●` worn, `○` not worn.
- **R-RACK-03** The active block has a faint background tint of its hat's color across
  the column's full width, and its name is in that color. Other blocks have no tint.
- **R-RACK-04** The model line is the model that hat would run on now: its own if one is
  set in `/models`, otherwise the one every hat shares.
- **R-RACK-05** A hat with no turns shows its model and the line `not worn yet`, and no
  figures.
- **R-RACK-06** All figures cover the current session only, including turns restored by
  `ryter resume`.
- **R-RACK-07** Spend follows the existing rule: an unknown rate is `$?.??`, never
  `$0.00`. A hat with turns that cost nothing known shows `$?.??`.
- **R-RACK-08** The rack's height does not depend on the number of turns. A session of
  forty turns draws the same rows as a session of four.

### 5.2 Blocks are separated by one blank row

No rules between blocks. The tint on the active block is the only background.

### 5.3 Hat-specific rows

| Hat | Rows | Source |
| --- | --- | --- |
| plan | `plans  N approved` (and `· N rejected` when any) | plan panel outcomes |
| build | `files  N`, `lines  +A −D`, then `success N` / `warning N` / `failure N` for the latest run of the project's tests (skipped and ignored tests are warnings) | checkpoints, the last test run's summary line |
| review | `verdicts  ✓ N pass` or `✗ N fail`; `✓ N  ✗ N` when both occurred | review verdicts |

A count of
zero is dim. A test run whose summary line has no counts to read (`Ran 5 tests`) shows one
row, `checks`, with the line as the tool printed it.

- **R-RACK-09** `files` and `lines` in the build block are gross for the session: every
  file the build hat changed and every line it added or removed, whether or not the
  change is still uncommitted. The net, uncommitted state is the instruments' `CHANGES`
  card (§6.1).
- **R-RACK-10** The build block's check rows are the most recent test run by any hat.
  They are omitted until one has run.
- **R-RACK-11** When the body is short (R-LAYOUT-07) the hat-specific rows are dropped
  from every block at once, leaving name, model, and turns with spend.

---

## 6. Instruments (right column)

### 6.1 Cards, in order

| Card | Rows |
| --- | --- |
| `MODEL` | the active hat's model in the hat color; `connection  <name> ●` (green `●` when the connection has a key, a red `○` when it has none); `reasoning  <effort>` |
| `CONTEXT` | a one-row gauge in the hat color with the percentage; `<used> / <window> tokens` |
| `PULSE` | eight bars, one a second, oldest first, and the rate: `▁▂▃▅▇▆▃▁  42 tok/s` while the model writes, `idle` three seconds after its last token |
| `SPEND` | `session`, `project`, `budget` |
| `GUARD` | `sandbox  <profile>`; `this hat  <what it may do>` |
| `CHANGES` | header right-aligned `uncommitted`; one row per changed file with `+A −D` or `new` |

- **R-INST-01** Cards are a dim, letter-spaced heading followed by rows, with one blank
  row between cards. No boxes.
- **R-INST-02** `session` is the whole session's spend, in bold. `project` is the total
  across sessions for this repository, the figure `ryter spend --project` reports,
  prefixed `≥` when it includes unpriced calls. `budget` is the cap, or `off`.
- **R-INST-03** There is no per-hat spend in this column. That is the rack's.
- **R-INST-04** `this hat` reads `read only` for plan and review and `edits ask first` for
  build, in the hat color.
- **R-INST-05** Retired with the test hat (2026-10-03): there is one conversation.
- **R-INST-06** `CHANGES` lists at most six files, then `+N more`. With nothing changed
  since the last commit it reads `nothing uncommitted`, and outside a git repository `no
  repository here`. The list is read from git without writing to the repository, at
  startup and when a turn ends, a file is reverted or a commit is made.
- **R-INST-07** The context gauge turns to the warning color at the threshold the current
  gauge uses.
- **R-INST-10** The pulse counts the model's streamed text and reasoning, four characters
  to a token, as the activity strip does. The rate is tokens over the last two seconds.
  The bars are in the hat color while the model writes and dim when idle. The last test
  run is not in this column: it is the build block's.

### 6.2 Condensed (100 to 131 columns)

- **R-INST-08** At 30 columns: `MODEL` is one row (`<hat>  <model>`, the model name
  truncated from the right); `CHANGES` is `N files  +A −D`. `CONTEXT`, `PULSE`, `SPEND`
  and `GUARD` keep their rows.

### 6.3 Status line (below 100 columns)

```
 deepseek-pro  ctx ━━━───── 38% │ $0.065 · budget off   ^b hat rack  / commands
```

- **R-INST-09** Below 100 columns the hint bar becomes a status line: model, context
  gauge and percentage, session spend, budget, then as many keys as fit. Project spend is
  not shown at this width; it is in the `^b` panel.

---

## 7. Watermark

A fedora, drawn behind the transcript in the active hat's color. It is the one hat graphic
on the screen and the same shape for every hat.

### 7.1 The shape

64 half-cells wide and 30 half-cells tall, so 64 columns by 15 rows. `#` is the hat, `=`
is the band. This bitmap is the source of truth.

```
                      ########    ########
                    ###########  ###########
                   ##########################
                  ############################
                 ##############################
                 ##############################
                ################################
                ################################
                ################################
               ##################################
               ##################################
               ##################################
              ####################################
              ####################################
              ####################################
              ####################################
             ======================================
             ======================================
             ======================================
             ======================================
             ######################################
      ####################################################
  ############################################################
################################################################
################################################################
 ##############################################################
    ########################################################
          ############################################
                  ############################

```

- **R-MARK-01** The watermark is a background tint: the hat color blended over the screen
  background at 7% for the hat and 14% for the band.
- **R-MARK-02** It is centered horizontally in the transcript area and placed so its top
  is one third of the way down. It does not scroll with the text.
- **R-MARK-03** On a row with no text, each cell draws its two half-rows exactly, using
  `▀` or `▄` where only one half is inside the shape. On a row that has text anywhere
  under the hat, every cell takes the tint of its top half as its background, whole: a
  half-block in the gap between two words reads as a stray mark.
- **R-MARK-04** The watermark never changes a cell's foreground color or its text.
- **R-MARK-05** A cell that already has a meaningful background keeps it: diff added and
  removed rows, code blocks, selection, the sticky turn header. The watermark shows only
  where the background would otherwise be the screen background.
- **R-MARK-06** Body and dim text over the tint must still meet WCAG AA against it. The
  theme test that checks shipped themes covers both tints for every hat color.
- **R-MARK-07** It is drawn only when the transcript area is at least 66 columns by 17
  rows. Otherwise it is omitted; it is never scaled or cropped.
- **R-MARK-08** It is omitted in 16-color and no-color modes, while the workbench is
  open, and when `[ui] watermark = false`.
- **R-MARK-09** On a hat switch the watermark changes color with everything else, in the
  same frame.

---

## 8. Color

### 8.1 What the hat changes

- **R-COLOR-01** The active hat's color is the screen's one accent. It colors: the top
  bar chip, the active rack block's name and tint, the model name and context gauge in
  the instruments, the `this hat` value, the watermark, the hairline above the composer,
  the composer's hat chip and prompt mark, and the speaker name of the model's turns in
  that hat.
- **R-COLOR-02** The screen background, panel background, hairlines, body text and dim
  text are neutral and do not change with the hat.
- **R-COLOR-03** Colors that carry their own meaning keep it under every hat: success and
  added lines green, failure and removed lines red, the user's speaker color, warnings.
  Each hat's dot in the top bar, the rack and the transcript's folded turns stays in that
  hat's own color.
- **R-COLOR-04** An offer to switch hats (the review offer after a build, the test offer
  after a review) is bordered in the color of the hat it offers.

### 8.2 The shipped dark theme

The `dark` theme takes these values. Slot names are unchanged, so user theme files keep
working.

| Slot | Value | Use |
| --- | --- | --- |
| `bg` | `#0A0B0E` | screen background |
| `sidebar_bg`, `panel_bg` | `#0F1115` | side columns, top bar, hint bar |
| rule | `#23262D` | hairlines, gauge track |
| faint | `#3D424C` | timeline strokes, dividers |
| `fg` | `#E1E4EA` | body text |
| `dim` | `#8C93A1` | secondary text |
| `user` | `#7CA7FF` | the user's speaker mark |
| `plan` | `#4FD8FF` | plan hat |
| `build` | `#7CF29A` | build hat, success, added lines |
| `audit` | `#FFB454` | review hat |
| `architect` | `#C79BFF` | test hat |
| `error` | `#FF8A80` | failure, removed lines |

- **R-COLOR-05** Text on a filled hat chip is a near-black of the same hue, and the pair
  meets WCAG AA.
- **R-COLOR-06** The active rack block's tint is the hat color blended over the panel
  background at 8%.
- **R-COLOR-07** The `light` theme gets the same structure with its existing hat colors.
  Its watermark tints are 6% and 12%, subject to R-MARK-06.

---

## 9. Degradation

| Mode | Reduction |
| --- | --- |
| 256 colors | Colors quantize as today. The watermark is kept if both tints still differ from the background after quantizing; otherwise omitted. |
| 16 colors | No watermark. No rack tint: the active block is marked by `◆` and a bold name. Chips are reverse video in the hat's ANSI color. |
| No color | No watermark, no tints. The active hat is `◆` and bold in the top bar and rack; chips are reverse video. |
| `< 132` columns | Hat rack folds away (R-LAYOUT-05). |
| `< 100` columns | Instruments fold away; status line (§6.3). |
| `< 66×17` transcript | No watermark (R-MARK-07). |

- **R-DEGRADE-01** In every mode the active hat can be told from the others without
  color, by its marker and weight.

---

## 10. Composer and hint bar

- **R-COMP-01** The composer has a hairline above it in the hat color and no box. Its
  first row starts with the hat chip, then `›` in the hat color, then the text.
- **R-COMP-02** The speaker name is no longer shown beside the chip. The user's name is
  on their turns in the transcript.
- **R-COMP-03** Placeholder text is per hat and names the next `Tab` stop: plan `what
  should we plan?`, build `what should change?`, review `ask about the review`, test
  `what should be tried?`.
- **R-COMP-04** Growth, scrolling and editing keys are unchanged.
- **R-COMP-05** The hint bar lists keys for the current state, dropping the least needed
  until they fit, as `prompt_keys` does today.

---

## 11. Starting hat

Today a new TUI session starts in the build hat.

- **R-START-01** A new `[ui]` key, `start_hat`, takes `plan`, `build`, `review` or
  `last`. The default is `plan`. It is the hat Ryter opens in when it starts a new
  session; `/new` inside the app keeps the hat the user is in.
- **R-START-02** `last` starts a new session in the hat this project's most recent
  session ended in, or in `plan` when the project has no earlier session. A session that
  ended in the test hat counts as `plan`, since a test needs something to test.
- **R-START-03** `test` is not a value. An unknown value warns as other unknown `[ui]`
  values do and falls back to `plan`.
- **R-START-04** A resumed session opens in the hat it was saved in, whatever the setting.
- **R-START-05** `ryter --hat <hat>` overrides the setting for that run.
- **R-START-06** Headless runs (`-p`) are unchanged: without `--hat` they use the build
  hat. Scripts depend on that.
- **R-START-07** `/settings` gains a `STARTUP` section above `SPEND` with one row,
  `start in  ‹ plan ›`, cycling the four values. Each value shows a one-line description
  under the row: `read and propose first`, `straight to work`, `open on a critique`,
  `the hat this project closed in`.
- **R-START-08** The welcome line names the hat: `starting in the plan hat · Tab to
  change it · /help for keys`.

---

## 12. State and module changes

### 12.1 `ryter-core`

- **R-CORE-01** Per-hat totals (`ryter_core::rack`): turns for each of the four hats,
  plus the outcome counts §5.3 needs (plans approved and rejected, review verdicts passed
  and failed, test checks passed and failed, files and lines the build hat changed). They
  are folded from the session's events. The screen feeds them live, and a resumed session
  reads them back from the session's event log, so both show the same rack. Nothing new
  is stored except one event, `planned`, emitted when the user approves or rejects a
  plan: the log did not record that. Per-hat spend comes from the spend rows, which
  already name the hat. A plan approved in the plan hat is built in the same turn, so a
  hat put on in the middle of a turn counts a turn of its own once it does something.
- **R-CORE-02** `UiConfig` gains `start_hat: String` (default `"plan"`) and
  `watermark: bool` (default `true`). Both are added to `UI_KEYS` and to
  `config.example.toml`, and `/settings` has a row for each.
- **R-CORE-04** `review::uncommitted` lists what differs from the last commit for the
  instruments, reading only: it takes no snapshot. It counts the lines of at most fifty
  new files, so a folder of installed packages nobody has ignored does not stall a turn.
- **R-CORE-03** Finding the hat a project's last session ended in, for `start_hat =
  "last"`, reads session metadata only. It opens no conversation file.

### 12.2 `ryter-tui`

- **R-TUI-01** `rail.rs` becomes the hat rack. The instruments are a new module beside
  it. `draw_ledger` lays out the two columns by the tiers in §3.2.
- **R-TUI-02** The watermark is its own module: the bitmap as a constant, and one
  function that tints a buffer area after the transcript is drawn and before overlays.
- **R-TUI-03** The top bar replaces the views strip.
- **R-TUI-04** `Theme` gains what §8 needs (the rule and faint slots if they do not
  already exist under another name, the two watermark tints, the rack tint). Derived
  values are computed in `derive`, so a one-key theme file still yields a full palette.

### 12.3 `ryter-cli`

- **R-CLI-01** The TUI's starting hat comes from `start_hat` (§11). The headless paths
  keep `Role::SoloBuild`.

---

## 13. Testing contract

- **R-TEST-01** Golden snapshots are regenerated at the four existing sizes: 80×24 (no
  columns), 100×30 and 120×40 (instruments only), 160×50 (both columns). One snapshot is
  added at 132×40, the narrowest size with both.
- **R-TEST-02** Each of the four hats has a snapshot at 160×50, and a styled-buffer test
  asserts that the accent cells listed in R-COLOR-01 carry that hat's color.
- **R-TEST-03** Rack height: a view with four turns and a view with forty draw the same
  number of rack rows (R-RACK-08).
- **R-TEST-04** Rack figures: a table-driven test builds sessions with known turns and
  asserts each block's turns, spend and hat-specific rows, including `$?.??` for an
  unpriced hat and `not worn yet`.
- **R-TEST-05** Watermark: with it on, every cell that holds text has the symbol and
  foreground it has in the same frame drawn with it off (R-MARK-04); diff and code
  backgrounds are unchanged
  (R-MARK-05); it is absent in 16-color and mono, below 66×17, and with
  `watermark = false`.
- **R-TEST-06** Contrast: `shipped_themes_meet_wcag_aa` is extended to body and dim text
  over both watermark tints and the rack tint, for every hat color.
- **R-TEST-07** Starting hat: table-driven over the setting, `--hat`, a resumed session,
  `last` with and without an earlier session, `last` after a test-hat session, and an
  unknown value. A test asserts headless runs still default to build.
- **R-TEST-08** `Ctrl+B` at 160, 120 and 80 columns: columns toggle, then the popout
  panel opens and `Esc` closes it.
- **R-TEST-09** No test depends on the machine: no real home folder, no real terminal
  size, no network.
- **R-TEST-10** Before the patch is called ready, the real TUI is launched and used end
  to end in a truecolor terminal and a 16-color one, through all four hats.

---

## 14. Non-goals

- Different hat art per hat. One fedora; only its color changes.
- A list of turns in a side column.
- A hat card or any second hat graphic.
- Animation on hat switch.
- Images through terminal graphics protocols (kitty, sixel). The watermark is cells.
- Changes to the classic layout, the workbench, the palette or panel layouts.
- New key bindings.

---

## 15. Documentation that changes with the work

- `README.md`: the quick start says the TUI opens in the build hat; it opens in plan.
  The hat table and "How it works" stay.
- `docs/guide.md`: the screen description, `start_hat`, `watermark`, `Ctrl+B` at narrow
  widths.
- `config.example.toml`: the two new `[ui]` keys.
- `.ryter/skills/layout/SKILL.md`: its line "quiet chrome, no outer boxes" is already out
  of date and is corrected to the chrome rule in `RYTER.md`'s non-negotiables.
- `DECISIONS.md`: the decisions in §16, with their reasons.
- `ROADMAP.md`: an entry for this work.

---

## 16. Decisions locked in this contract

| Decision | Why |
| --- | --- |
| One fedora for every hat, colored by the hat | Four different hats were drawn and read as clutter at terminal resolution; one shape that changes color is a stronger mark. |
| The watermark is the only hat graphic | A hat card beside the watermark showed the same thing twice and cost five rows of instruments. |
| The rack is four fixed blocks, not a list of turns | A turn list repeats the transcript and outgrows the column in a long session. |
| The top bar shows hats worn, not a pipeline | `Tab` reaches any hat at any time; arrows and check marks would claim an order that isn't enforced. |
| Per-hat spend in the rack; session and project in the instruments | Each figure appears once. |
| New sessions start in plan | The user's choice. `start_hat` restores build for anyone who wants it. |
| Headless runs still default to build | Existing scripts call `ryter -p` and expect work to be done. |
| No new key; `Ctrl+B` opens the folded columns as a panel | The free control keys are either taken or unsafe (`Ctrl+H` is backspace in many terminals). |
| The hat changes the accent only | A full recolor on every `Tab` is disorienting; neutral chrome keeps the screen still. |

---

## 17. Open items

These are not decided. Each needs the user's answer before or during the work.

1. **The classic layout.** It is untouched here. Whether to keep maintaining a second
   layout once this ships is a separate decision.
2. **The `light` theme's hat colors.** R-COLOR-07 keeps the existing ones. They have not been
   mocked up with the watermark.
3. **Rejected plans and failed checks in the rack.** §5.3 shows them only when they
   occur. If the rows feel noisy in use, the alternative is totals only.

---

## 18. What changed in the building

Since 2026-10-03 the rack has two rows and the review hat is the audit hat: see `docs/specialists-design.md`, which wins where the two differ.

Each of these differs from the first draft. The requirement above already says what was
built; this is the list, with the reason.

| Change | Why |
| --- | --- |
| The session title is in the top bar (R-TOP-08) | It was an open item. The bar has the room at 100 columns and up, and the title is about the whole session, as the bar is. |
| The folded columns open side by side in one panel, which scrolls (R-LAYOUT-05) | It was an open item. At 80×24 the panel is shorter than the rack, so it has to scroll. |
| A wide popout takes the whole body and hides the side columns (R-LAYOUT-08) | Found in use: the plan panel overlapped half of the instruments and left fragments of it showing. |
| The test hat's model line names a model, not `follows build` (R-RACK-04) | A hat with no model of its own uses the one every hat shares, not the build hat's. The draft was wrong. |
| Two kinds of verdict or check show as counts alone (§5.3) | `✓ 3 passed ✗ 2 failed` is wider than the column. |
| The connection mark means "has a key", not "last call worked" (§6.1) | The screen has no record of the last call's outcome by connection, and a key is what a user can act on. |
| `nothing uncommitted`, not `none this session` (R-INST-06) | The card is about the files, not the session: work from an earlier session still shows there. |
| Rows with text are tinted a whole cell at a time (R-MARK-03) | Found in use: half-blocks in the gaps between words looked like stray marks. |
| `/new` keeps the hat (R-START-01) | It already did, on purpose: a new session in the middle of work stays in the mode the user is in. |
| One new event, `planned` (R-CORE-01) | The log recorded a plan's approval only as a notice in words. |
| The scrollbar is a thumb with no track on this screen | Found in use: a track beside the instruments' hairline is two lines where one separates. |
| The dark theme's `error` is set, not derived | The red derived from the new amber warning color was harsher than the mockups' `#FF8A80`. |

After a first real session on the released screen (2026-10-02), the user asked for these:

| Change | Why |
| --- | --- |
| The composer is the transcript column's width; the side columns run to the foot (R-LAYOUT-02) | A prompt that ran under the side columns read as overlapping them. |
| Checks are three rows, `success` / `warning` / `failure`, with the same words in the build and test blocks (§5.3) | One number with a mark in front took a second look; three named rows don't, and "tests" and "checks" meant the same thing in two places. |
| The tests row is gone from the instruments (§6.1) | The same number was on screen twice. |
| A `PULSE` card: tokens a second with eight seconds of history (R-INST-10) | How fast the model is writing was nowhere on screen. |

On 2026-10-03 the user removed the test hat:

| Change | Why |
| --- | --- |
| Three hats: plan, build, review. The top bar, the rack and `Tab` lose the test hat (R-TOP-01, §5, R-INST-04, R-INST-05) | The test hat's one distinction, never writing the project, was what made it fail in use, and it fit only a product with an address. `run_project` moved to the build and review hats. See `DECISIONS.md`, 2026-10-03. |
