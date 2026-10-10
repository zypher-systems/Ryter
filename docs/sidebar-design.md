# One column beside the conversation: the sidebar screen

Status: approved 2026-10-10 (mockup canvas: https://claude.ai/artifact/CpYpB64B8CZRqP4V63PRd3, the Route E boards E0 to E3). Not built yet; §17 will record where the build differs. The boards were drawn on 0.24.1's transcript; the transcript this contract keeps is 0.25.0's (§1).

This contract replaces the screen described by `docs/hat-rack-design.md` and the rack half of `docs/specialists-design.md`. Where they disagree, this one wins. The audit and scribe hats, the plan file and the keys between rows are unchanged and stay where they are described.

## 1. What changes, and why

On 2026-10-10 the first group of developers to use Ryter said the way it works is good and the way it looks is a toy. What they were looking at: a 64-column fedora drawn behind the conversation, a column of gauges that read `0% · idle · $0 · off · off · none · none` on a fresh session, a top bar of filled chips, uppercase labels, and a saturated cyan on black. The transcript, the help panel and the command palette did not get the comment.

The same day, in a separate session, 0.25.0 took the first pass on the existing layout (`DECISIONS.md` 2026-10-10, "The screen keeps the hats and stops performing them"): the watermark off unless asked for, the hat that is on the one coloured thing, transcript rows led by space instead of dots, the prompt's placeholder the hat's question alone, the rack headed `HATS`, no rows of zeros, and the pulse card only while a turn runs. That pass stopped the screen performing the hats. It left the layout as it was: a top bar of chips, a hat rack column that only a 132-column screen shows, and a `GUARD` card that still reads `sandbox · this hat · plan.md none · audit.md none` on every session.

The brief for the redesign, in the user's words: unique and useful, not what every other harness looks like, and the useful parts visible without opening anything. So the sidebar stays, permanently, and is rebuilt so that every row on it is a measurement, an event or a name. The decoration goes: no fedora anywhere, no top bar, no filled chips but one, no label that reads `off` or `none` as a steady state. The name stays on screen in every hat, in capitals.

What goes: the watermark altogether (`watermark.rs`, the `[ui] watermark` setting, the `mark` and `rack` tints), the top bar, the hat rack column, the `PULSE` and `GUARD` cards, uppercase labels, the 132-column tier, the welcome line.

What stays: the transcript as 0.25.0 left it (space-led rows, a folded turn ending in what it came to, a tool's time from a second up, the closing line `^o` opens), the composer with its one filled hat chip, the hint bar and its keys, every panel and popout, the workbench, the classic layout.

## 2. Principles

- **R-PRIN-01** A row on the sidebar is a measurement, an event or a name. It is never a label whose steady value is `off`, `none` or `idle`. A fact that is usually `none` is shown only when it is not.
- **R-PRIN-02** One filled element on the screen: the composer's hat chip. Everything else is text on the background, with hairlines.
- **R-PRIN-03** A colour names a thing: a hat, a warning, an error, an addition, a removal. Nothing is coloured for decoration.
- **R-PRIN-04** `RYTER`, in capitals, is on screen in every hat and every state. It is the only word in capitals on the screen.
- **R-PRIN-05** What the sidebar says about the turn in flight and what the status row under the user's message says come from the same source and never disagree.

## 3. Layout

### 3.1 Regions

| Region | Size | Content |
| --- | --- | --- |
| Transcript | the rest, from row 0 | the ledger's reading column, unchanged |
| Sidebar | 30 columns, right: a hairline, a pad, 28 of text | §4 |
| Composer | 1 hairline + 1 to 8 text rows + 1 blank | the hat chip and the message box, the width of the transcript |
| Hint bar | 1 row | keys for the current state; the status line below 100 columns |

- **R-LAYOUT-01** There is no top bar and no hairline above the body. The transcript starts on the first row. What the top bar carried moves: the name, the session's title, the folder and the branch to the sidebar's header (§4.1); the hat marks to the hats block (§4.2); the workbench's `chat esc · changes ^t` row stays the workbench's own, drawn by it, as the one screen with a header row.
- **R-LAYOUT-02** The sidebar has no outer box and no panel tint: `sidebar_bg` equals `bg` in the shipped themes. It is separated from the transcript by a one-cell vertical hairline that runs from the first row to the blank row above the hint bar; the composer is the transcript's width and the hairline continues beside it.
- **R-LAYOUT-03** The composer and the hint bar are always on screen. No column, panel or overlay reduces them.

### 3.2 Width tiers

| Terminal width | Sidebar | Hint bar |
| --- | --- | --- |
| `>= 100` | shown, 30 columns | keys |
| `< 100` | hidden | status line (§9) |

- **R-LAYOUT-04** Two tiers, decided by width alone. `rail::Tier::Wide` (the rack) goes; `Mid` is the sidebar and `Narrow` is none. `BOTH_MIN` and `RACK_W` go with it.
- **R-LAYOUT-05** `Ctrl+B` keeps its binding. At 100 columns and wider it hides and shows the sidebar. Below that it opens the sidebar as a popout panel over the transcript, the same blocks stacked, `↑↓` scrolling it on a screen too short to show it whole; `Esc` or `Ctrl+B` closes it. `[ui] panel = false` starts with the sidebar hidden.
- **R-LAYOUT-06** When the body is shorter than the sidebar needs, blocks give up rows in this order until it fits: `changes` to its summary row alone, the `turns` row, the third row of `now`, `permissions`, the `project` row of `spend`. If it still does not fit, the sidebar is hidden as it is below 100 columns, and `Ctrl+B` opens it as the panel.
- **R-LAYOUT-07** A popout wider than the transcript (`/models`, `/settings`) takes the whole body and the sidebar is not drawn under it. One that fits the transcript (a plan to approve, a permission card, the specialists picker) leaves the sidebar in sight.

### 3.3 Wireframe: 120×40, build hat, a command running

```
                                                                                          │ RYTER
  12:53  ●  add a greet function                                 ✓ answered · 1 tool  ▸   │ add a greet function
                                                                                          │ project · main
  12:54  ●  you                                                                           │
         │  do it                                                                         │ hats
         │                                                                                │   plan       1 turn   $0.002
         ├─ write  app.py                                                        +3 −0    │ ▸ build      1 turn   $0.003
         │    1   x = 1                                                                   │   ──────────── specialists
         │    2 +                                                                         │   audit
         │    3 + def greet(name):                                                        │   scribe
         │    4 +     return f"hi {name}"                                                 │   turns      ●●
         │                                                                                │
         ◆  claude-opus-5.5                                                       $0.002  │ now
         │  Now the tests.                                                                │   claude-opus-5.5
         │                                                                                │   running python3 -m pytest
         ├─ run    python3 -m pytest -q                                          ⠧  0:04  │   18 tok/s  in 1.2k  out 0.3k
         │  ⠧ running · 0:04                                                              │
                                                                                          │ context
                                                                                          │   ━━───────────────────   1%
                                                                                          │   1k of 200k
                                                                                          │
                                                                                          │ spend
                                                                                          │   session     $0.005  no cap
                                                                                          │   project     $0.415
                                                                                          │
                                                                                          │ changes
                                                                                          │   app.py               +3 −0
                                                                                          │   1 file uncommitted
                                                                                          │
                                                                                          │ permissions
                                                                                          │   asks first · sandbox off
                                                                                          │
                                                                                          │
                                                                                          │
                                                                                          │
                                                                                          │
──────────────────────────────────────────────────────────────────────────────────────────│
   build   › type to queue the next message, / for commands                               │
                                                                                          │
 enter send    tab plan    ⇧tab specialists    / commands    ^t changes    ^b sidebar    esc cancel    ^c quit
```

`▸ build` and the chip ` build ` are in the build colour; `audit` and `scribe` in the specialist colour; `plan` in the plan colour. The hairline and the word `specialists` are in the hairline colour. `running` is in the hat colour. The two dots under `turns` are blue then green. The transcript's rows are 0.25.0's: led by space, the folded turn ending in what it came to, the model's name neutral, the folded turn's dot dim.

### 3.4 Wireframe: 80×24, same moment

```
  12:54  ●  you
         │  do it
         │
         ├─ write  app.py                                                  +3 −0
         │    1   x = 1
         │    2 +
         │    3 + def greet(name):
         │    4 +     return f"hi {name}"
         │
         ◆  claude-opus-5.5                                               $0.002
         │  Now the tests.
         │
         ├─ run    python3 -m pytest -q                                  ⠧  0:04
         │  ⠧ running · 0:04






────────────────────────────────────────────────────────────────────────────────
   build   › type to queue the next message, / for commands

 build · opus-5.5  ━━──────── 1%  $0.005 · no cap   ⇧tab specialists  ^b sidebar
```

## 4. The sidebar

Blocks in order, each a dim lowercase heading and its rows, one blank row between blocks, no boxes. Text is 28 columns wide; values are right-aligned where the row is a figure.

### 4.1 Header

```
RYTER
add a greet function
project · main
```

- **R-HEAD-01** Row 1: `RYTER` in bold, the body colour. Row 2: the session's title, or `new session` dim until it has one. Row 3, dim: the working folder's last component and the branch; outside a repository the folder alone. A long title or folder is cut from the right with `…`.
- **R-HEAD-02** The header is three rows and a blank row, always.

### 4.2 Hats

```
hats
  plan       1 turn   $0.002
▸ build      2 turns  $0.004
  ──────────── specialists
  audit ✓    1 turn   $0.012
  scribe
  turns      ●●●●
```

- **R-HATS-01** Every hat, in `rail::HATS` order: the primary hats, a separator row, the specialists. The separator is a hairline of twelve cells and the word `specialists`, both in the hairline colour. The block replaces the hat rack and the top bar's hat strip.
- **R-HATS-02** The hat on is marked `▸` and its name is bold, in the hat's colour. Every other hat is two spaces and its name in its colour. A hat with no turns this session shows its name and nothing else: no `no turns yet`, no `—`.
- **R-HATS-03** A hat with turns shows `N turn` or `N turns` and its spend, from `rail::hat_spend` (`$?.??` when a model on it is unpriced), right-aligned.
- **R-HATS-04** A specialist whose latest turn ended in a verdict carries the verdict after its name: `✓` in the success colour for a pass, `✗` in the error colour for a fail. The audit's rack rows (`audits  N fail  N pass`, `changed nothing`, `docs  N written`) are retired; the folded turns in the transcript carry them.
- **R-HATS-05** `turns`: one dot per turn this session in the colour of the hat the turn started in, oldest first. The row shows the newest fifteen; older ones drop off the left. Absent while the session has no turn.
- **R-HATS-06** When `.ryter/plan.md` or `.ryter/audit.md` exists, one more dim row ends the block: `plan.md 12:53 · audit.md 12:57`, each with the time it was written today or its date otherwise, `audit.md writing…` during an audit. Absent when neither file exists (R-PRIN-01).
- **R-HATS-07** The block's height is its hats plus the separator, the heading and the `turns` row, and grows only when a hat is added to the product.

### 4.3 Now

```
now
  claude-opus-5.5
  running python3 -m pytest
  18 tok/s  in 1.2k  out 0.3k
```

- **R-NOW-01** Row 1: the hat's model, cut from the right to fit. Row 2: the turn's live verb from `activity::Verb`, the same text as the status row under the user's message: `thinking · 1:40`, `writing`, `running <command or tool>`, `waiting for the model`, `waiting for you · allow?`, and `idle`, dim, when no turn is in flight. The verb is in the hat colour. Row 3, dim: the rate over the last two seconds (counted as the pulse counted it, four characters to a token) and this turn's tokens in and out. Row 3 is blank while idle. The `PULSE` card (since 0.25.0 shown only while a turn runs) and its sparkline are retired; this block is what it was for.
- **R-NOW-02** While a question is open (a permission, a plan, an `ask_user`, the trust prompt, a `sudo` password), rows 2 and 3 are in the warning colour and row 3 names the question: `allow?  run git push`, `plan?  approve or reject`, `question`, `trust?`, `password`. The `waiting for you` row in the transcript and the hint bar are in the warning colour at the same time, as today.

### 4.4 Context

- **R-CTX-01** `context`, then a gauge of 21 cells with the used part in the hat colour (the warning colour past the threshold the gauge uses today) and the rest in the hairline colour, the percentage right-aligned; then `<used> of <window>`, dim.

### 4.5 Spend

- **R-SPEND-01** `session` with the session's spend in bold, then `no cap` dim, or `of $N` with the colouring `instruments::session_spend` gives it today when a budget is set. `project` with the figure `ryter spend --project` reports, `≥` when it includes unpriced calls. Per-hat spend is the hats block's.

### 4.6 Changes

- **R-CHG-01** As the `CHANGES` card today (R-INST-06 of the hat rack contract), lowercase: up to six files, each `path  +A −D`, `new` or `binary`, the path cut from the left so the file name stays; then `N files uncommitted` (`1 file`); `+N more` when there are more than six. `nothing uncommitted` or `no repository here` when so, dim. Read as today: at startup, when a turn ends, after a revert and after a commit.

### 4.7 Permissions

- **R-PERM-01** One row: what this hat may do, in the hat colour (`read only`, `asks first`, `checkpoint`, `docs only`: `instruments::hat_may` in its condensed form), then `· sandbox <profile>` dim. `· yolo` in the warning colour while `/yolo` is on. The `GUARD` card is retired; its `plan.md` and `audit.md` rows are R-HATS-06.

## 5. The opening screen

```
  RYTER  ·  project  ·  main  ·  nothing uncommitted

  plan     reads and proposes, changes nothing          ◆ on
  build    makes the changes                            tab
  2 specialists: audit · scribe                         ⇧tab

  /sessions to resume one of 12 · /help for keys
```

- **R-OPEN-01** A session with no turn shows this block at the top of the transcript, in place of the welcome line (R-START-08 of the hat rack contract is retired). The hat the session opens in carries `◆ on` in its colour; the other primary carries `tab`; the specialists row names them in the specialist colour and carries `⇧tab`. The last row counts this project's saved sessions and is `/help for keys` alone when there are none.
- **R-OPEN-02** The block is drawn, not sent: it is not a turn, it is not saved, and it is gone once the session has a turn. Specialists that do not fit the row collapse to `N specialists`.

## 6. Keys

- **R-KEY-01** `Tab` is unchanged: the next hat in the row the user is in.
- **R-KEY-02** `Shift+Tab` in the primary row opens the **specialists picker** above the composer, where the command palette opens: one row per specialist, `name · what it does · what it may do · its model`, `›` on the specialist last worn (audit when none), typing filters, `↑↓` move, `Enter` puts the hat on and opens the composer on its request (R-COMP-18), `Esc` closes. In the specialist row `Shift+Tab` returns to the primary last worn, build when none, as today. `Tab` in the specialist row cycles the specialists as today. The sidebar's hats block highlights the row under the picker's cursor.
- **R-KEY-03** `Ctrl+B` is R-LAYOUT-05. The hint bar names it `^b sidebar`. `/plan`, `/build`, `/scribe` and `/audit` are unchanged.
- **R-KEY-04** No other binding changes.

## 7. Composer and hint bar

- **R-COMP-01** The composer is as today: a hairline above it in the hat colour, no box, the hat chip, `›` in the hat colour, the text. The chip is lowercase: ` plan `, ` build `, ` audit `, ` scribe `. Its text is a near-black of the chip's hue and the pair meets WCAG AA.
- **R-COMP-02** Placeholders are as today, lowercase.
- **R-COMP-03** The hint bar is as today, lowercase, with `^b sidebar` in place of `^b hat rack`. While a question is open it is in the warning colour, as today.

## 8. Colour

### 8.1 Hats

- **R-COLOR-01** Plan is blue. Build is green. Every specialist is the one specialist colour, the theme's `audit` slot: audit, scribe and any hat added later. The `architect` slot no longer colours a hat; it stays in the theme for syntax and for user theme files. Adding a specialist adds no colour.
- **R-COLOR-02** The hat on colours: the composer's chip and `›`, `▸` and its name in the hats block, the model's speaker name in the transcript, the gauge's used part, the `now` verb and the `permissions` value. Nothing else changes with the hat.
- **R-COLOR-03** Colours that carry their own meaning keep it under every hat: success and added lines, errors and removed lines, warnings, the user's speaker mark. 0.25.0's rule that the hat on is the one coloured thing holds on the transcript: a folded turn's dot stays dim and the model's name neutral. The hats block is the exception, on purpose: a hat's name there and a `turns` dot are in that hat's own colour, because the ledger names hats by colour and the `turns` row is where a past turn's hat is read.
- **R-COLOR-04** The user's speaker mark is a neutral grey-blue, so it is not read as the plan hat.
- **R-COLOR-05** The watermark tints (`Theme::mark`, `mark_band`) and the rack tint (`Theme::rack`) are removed from the theme. A user theme file has no keys for them, so none breaks.

### 8.2 The shipped dark theme

| Slot | Value | Use |
| --- | --- | --- |
| `bg`, `sidebar_bg`, `composer_bg` | `#0B0C0F` | the one background |
| `panel_bg` | `#0E1013` | popouts and cards |
| `rule` | `#22262D` | hairlines, the gauge track |
| `faint` | `#434955` | timeline strokes, the separator, leaders |
| `fg` | `#D6D9DF` | body text, `RYTER` |
| `dim` | `#848B97` | headings, secondary text |
| `user` | `#AEB7C6` | the user's speaker mark |
| `plan` | `#7FA7DB` | plan hat |
| `build`, `success` | `#8DC29B` | build hat, passes, added lines |
| `audit` | `#D0B077` | every specialist |
| `warn` | `#D9A866` | a question open, a budget near its cap |
| `error` | `#D6908A` | failures, removed lines |
| `code_bg` | `#13151A` | code and diff rows |
| `selection_bg` | `#181B21` | the picker's row, a selection |

- **R-COLOR-06** Every text colour above meets WCAG AA on `bg`, and the chip's text meets AA on each hat colour; `shipped_themes_meet_wcag_aa` covers the pairs.
- **R-COLOR-07** The `light` theme gets the same structure with its own values. Nobody has looked at the light theme on this screen; it is §16's.

## 9. Degradation

| Mode | Reduction |
| --- | --- |
| 256 colours | Colours quantise as today. |
| 16 colours | Plan blue, build green, specialists yellow, in the terminal's own colours. The chip is reverse video in the hat's colour. |
| No colour | The hat on is `▸` and bold in the hats block; the chip is reverse video; the verdict marks and the dots keep their glyphs. |
| `< 100` columns | The sidebar folds away (R-LAYOUT-05). The hint bar becomes the status line: the hat, the model's last component, the gauge and percentage, the session's spend and cap, then as many keys as fit, `^b sidebar` first. |
| Too short for the sidebar | R-LAYOUT-06. |

- **R-DEGRADE-01** In every mode the hat on can be told from the others without colour, by `▸` and weight, and a question open can be told by its text.

## 10. State and module changes

### 10.1 `ryter-core`

- **R-CORE-01** `rack::Rack` keeps the order of the session's turns by hat (`turn_hats: Vec<Role>`, appended where `turns` is counted), for R-HATS-05. Everything else the sidebar shows exists: `HatTotals` for the hats block, `review::Changes` for `changes`, `project::project_spend` for `project`, `session::list` for the opening screen's count, the plan and audit files' timestamps from the files.
- **R-CORE-02** `[ui] watermark` is retired: a settings file that still has it, `true` or `false`, loads silently, like `offer_audit`, and `config.example.toml` drops the line. 0.25.0 kept the drawing behind the setting because deleting it was the user's question; the user has answered it (§14). `start_hat`, `panel`, `layout`, `theme`, `colors` and `bell` are unchanged.

### 10.2 `ryter-tui`

- **R-TUI-01** `rail.rs` becomes `sidebar.rs` and absorbs `instruments.rs` and `info/`'s solo-screen cards: the tiers, `HATS`, `PRIMARY`, `SPECIALISTS`, `SEPARATOR_LABEL`, `hat_name`, `hat_mark`, `hat_model`, `hat_spend`, `hat_may`, `gauge_color`, `session_spend` and `budget` stay as functions of it. `watermark.rs` is removed. `draw_top_bar` is removed, and `draw_solo` lays out transcript, sidebar, composer and hint bar.
- **R-TUI-02** The `^b` panel below 100 columns draws the sidebar's blocks with the same functions, stacked, scrolling.
- **R-TUI-03** The `now` block reads `activity::Activity` (the verb, its detail, the elapsed time, the rate, the turn's tokens), the same object the status row reads.
- **R-TUI-04** The specialists picker is a `Panel` like the command palette, listing `SPECIALISTS` with `Role::describe` (new, one line each: "checks the work, may run it", "writes the docs, changes no code"), `hat_may` and `hat_model`.
- **R-TUI-05** `/settings` drops the *watermark* row. The `STARTUP` section and `start in` stay.

## 11. Testing contract

- **R-TEST-01** Golden snapshots are regenerated at 80×24 (no sidebar, status line), 100×30, 120×40 and 160×50 (sidebar). `solo-132x40.txt` is removed with its tier. The `html_gallery` test from 0.25.0 is kept and is how the pass is looked at in colour beside what it changed.
- **R-TEST-02** Each hat has a snapshot at 120×40 with a turn in flight, and a styled-buffer test asserts the cells listed in R-COLOR-02 carry that hat's colour and nothing outside the list changes with the hat.
- **R-TEST-03** The hats block, table-driven: no turns (name only, no other text on the row); turns and spend; `$?.??`; a verdict mark after a pass and a fail; the separator row's text and colour; the `turns` dots in order; sixteen turns show fifteen dots, the oldest gone; the `plan.md · audit.md` row present with the files and absent without them.
- **R-TEST-04** The `now` block and the status row say the same verb for thinking, writing, running, waiting for the model, waiting for you and idle, from one `Activity`; both are in the warning colour while a question is open, and row 3 names the question.
- **R-TEST-05** R-LAYOUT-06 at 100×26 and 100×20: the blocks shrink in the stated order, then the sidebar hides; `^b` then opens the panel.
- **R-TEST-06** `^b` at 160, 120 and 80 columns: the sidebar hides and shows, then the panel opens and `Esc` closes it.
- **R-TEST-07** No watermark: `watermark.rs` is gone, no cell in any frame carries the old tints, and a config with `watermark = true` or `false` loads with no warning.
- **R-TEST-08** Contrast: `shipped_themes_meet_wcag_aa` covers every text colour over `bg` and the chip's text over each hat colour, in dark and light.
- **R-TEST-09** The opening block: present with no turn, with `◆ on` following `start_hat` and `--hat`; gone after the first turn; the sessions count with and without earlier sessions.
- **R-TEST-10** The picker: `Shift+Tab` opens it in the primary row and returns to a primary in the specialist row; typing filters; `Enter` puts the hat on and the composer opens on its request; `Esc` leaves the hat as it was.
- **R-TEST-11** No test depends on the machine: no real home folder, no real terminal size, no network.
- **R-TEST-12** Before the patch is called ready, the real TUI is driven against the simulated provider and used end to end in a truecolor terminal and a 16-colour one, through all four hats, with a question open in each place R-NOW-02 names.

## 12. Non-goals

- A new specialist. `scout`, `bench` and `ops` on the mockups are stand-ins to show the layout at five; no specialist hat is added before 1.0 (ROADMAP, "What 1.0 means").
- A colour per specialist.
- The fedora in any form: watermark, logomark, icon.
- Changes to the classic layout, the workbench, the palette, the panels, the plan panel or the audit popout.
- Animation on hat switch.
- The light theme's values (§16).
- New key bindings.

## 13. Documentation that changes with the work

- `README.md`: `docs/screenshots/build.png` and its alt text show the hat rack and the side panel; owed new ones since 0.25.0, taken on this screen.
- `docs/guide.md`: "The screen" (the top bar, the hat rack, the watermark, the instruments, `^b`, the status line, the colours, `[ui] watermark`), the keys table, `/settings`. `config.example.toml`: the `watermark` line.
- `design.md`: the solo-screen requirements that name the rack, the top bar or the watermark.
- `docs/hat-rack-design.md` and `docs/specialists-design.md`: a status line at the top pointing here.
- `DECISIONS.md`: the second entry of 2026-10-10. `ROADMAP.md`: the Now entry until it ships; the release note, under `docs/releases/`, says what 0.25.0 did and what this does.

## 14. Decisions locked in this contract

By the user, 2026-10-10, over five rounds of mockups:

- The sidebar is permanent. A layout with nothing beside the conversation "looks like every other harness" and was rejected.
- No fedora anywhere. The watermark was what the developers saw first; a small logomark on the opening screen "looks terrible".
- No top bar. The name, the title and the folder live at the top of the sidebar.
- `RYTER` in capitals, on screen in every hat.
- Plan blue, build green, one colour for every specialist. The primaries and the specialists are separated by the hairline with the word on it.
- Every sidebar row is a measurement, an event or a name (R-PRIN-01).
- 0.25.0, from a separate session the same day, is the first half and is kept: the transcript's rows, the placeholder, the pulse shown only during a turn. This contract is the second half, the layout, and where it reverses a 0.25.0 choice (the watermark deleted rather than off by default; the hats block coloured by hat) it does so on the user's word.

## 15. What the build must not quietly change

- The transcript's rows, the folded turns, the status row, the reasoning pane, the allow card and its inset.
- `Tab` within a row, `Shift+Tab` between rows, the composer opening on a specialist's request, the audit's card and cost range.
- Headless runs, the classic layout, the workbench.

## 16. Open items

- The light theme on this screen: nobody has looked at it.
- The exact blue, once seen on a real terminal beside the green.
- Whether the `turns` row earns its place after a week of use.
- Whether `Shift+Tab` should switch directly while there are only two specialists, with the picker arriving at three.
- Whether the opening block should list the last few sessions by title instead of counting them.
- Whether a folded turn's dot should take its hat's colour again now that one accent is the transcript's rule; the `turns` row is where that is read for now.
- Where the `plan.md · audit.md` row belongs if the hats block grows past seven hats.

## 17. What changed in the building

Built on `sidebar-screen-patch` (2026-10-10). Where the build differs from the contract above:

| Change | Why |
| --- | --- |
| The `plan.md` and `audit.md` rows are one row each (`plan.md <day> <heading>`, `audit.md writing…`), present only for a file that exists, instead of one row naming both (R-HATS-06) | Two names, two times and the dots don't fit 28 columns; the old guard card's rule is kept: when the day and the heading don't both fit, the heading. |
| A specialist's verdict mark is the latest verdict its turns ended in, kept on the ledger (`HatTotals::last_verdict`) rather than the view's `last_review`, which the next edit clears (R-HATS-04) | The ledger says what the hat did; whether the verdict still describes the tree is the commit receipt's question. |
| The `now` block's third row says the rate and this turn's output tokens (`18 tok/s · 300 tokens`), not tokens in and out (R-NOW-01) | A turn's input tokens are not counted per turn anywhere; the status row says the same figure. |
| The verb row keeps the status row's words whole and drops the elapsed time when the two don't fit side by side (R-NOW-01) | `waiting for the model` is 21 columns; cutting the words would break R-PRIN-05. |
| The `permissions` block is two rows when the sandbox profile's name would not fit beside what the hat may do (R-PERM-01) | A profile's name is not cut. |
| The hat on's `▸` spins while its model works, as the top bar's chip did | The user's rule that a thinking model must look alive where the eye is; the bar that carried the spinner is gone. |
| The `^b` panel on a narrow screen is the sidebar's rows without its `RYTER` row (R-LAYOUT-05) | The panel's title says where this is. |
| The specialists picker's `Tab` moves down the list; `Shift+Tab` in the specialist row still returns to the primary hat last worn (R-KEY-02) | As the contract says; noted because the picker swallows `Tab` while it is open. |
| The picker shows what a hat may do in the sidebar's words (`checkpoint`, `asks first`) and the model's short name | Room: 84 columns for four columns of text. |
| The sixteen-colour theme is not held to the contrast test's hat and chip pairs (R-COLOR-06) | Its sixteen are the terminal's own, only approximated by the test's table; plan is `LightBlue`, build `Green`, the specialists `Yellow`, the user `White`. |
| The `review offer` permission card no longer exists (it went with the offer after a build) and its test case went with it | Its border was the audit colour by coincidence: the old theme's `warn` and `audit` were the same value. |
