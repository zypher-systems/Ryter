# TUI redesign — review gaps

Status: **open**
Against: `0.2.0-patch` (`60adb20` Implement 0.2.0 TUI redesign)
Evidence: golden snapshots under `crates/ryter-tui/snapshots/`
Not a new design contract. `design.md` still wins on intent. This file is what is still wrong on screen.

The 0.2 direction is right: conversation in the center, composer always on screen, command palette, one panel per config surface, activity strip for thinking, info cards instead of a flat key/value dump. These gaps are why it is not done.

---

## G-01 — Panels do not sit in a layer

**Severity:** high
**Where:** `panel/chrome.rs` (and every panel/modal snapshot)

Popouts are a rectangle painted over the live body. The right-hand cards show through the panel edge.

| Snapshot | Leak |
| --- | --- |
| `panel-help-100x30.txt` | Sidebar shreds into the key list: `k-4.6`, `ns`, `$?.??` |
| `panel-settings-80x24.txt` | `┃ d`, `┃ 6`, `──╯` through the form |
| `panel-crew-120x40.txt` | `ce unknown`, `end ────`, `sks`, `ew 0/4` |
| `modal-permission-80x24.txt` | Double pipe `│╭─ session`; `ld` / `──╯` in the interrupt |

`design.md` retired the no-box rule so a floating panel would have a **hard edge** and read as modal. Structural chrome that you can read through is decoration that failed.

**Done when:** A panel or interrupt fully covers or dims everything it sits on. No sidebar title, gauge, or border fragment is visible inside the panel. Snapshots at 80×24 and 100×30 for help, settings, crew, and permission are clean.

---

## G-02 — Help is the least readable panel

**Severity:** high
**Where:** `snapshots/panel-help-100x30.txt`

Help is the screen that should teach the product. At 100×30 it clips `welcome`, overruns the info column, and mixes keybindings with leftover card text. A first-run user opening `/help` or `F1` gets the worst frame in the suite.

**Done when:** Help is a self-contained panel at 80×24 and 100×30. Body text is not clipped by the sidebar. The filter composer and hint bar still fit.

---

## G-03 — Empty info cards keep the space the chat is supposed to own

**Severity:** medium
**Where:** `snapshots/idle-120x40.txt`, `idle-160x50.txt`

Principle 1 in `design.md`: the conversation gets the space. At rest the chat is blank and the right column still stacks five cards, three of them empty (`no tasks yet`, `no specialists running`, `price unknown`). `idle-160x50.txt` is mostly empty air.

Width already has a drop order (`< 80` hides the panel). There is no drop order for **empty content**. 80×24 dropping tasks/crew is the right idea; larger sizes should do the same until those cards have something to say.

**Done when:** Tasks and crew cards are absent until they have rows. Idle 120×40 / 160×50 give the extra rows to chat (or to a quieter empty state), not to three vacant boxes.

---

## G-04 — Spend prints `$0.00` next to `$?.??`

**Severity:** high (product rule, not taste)
**Where:** every idle and most panel snapshots, spend card

```
│ session          $?.?? │
│ bud ░░░░░░░░░░░░   0%  │
│     $0.00 of $5.00     │
```

`RYTER.md`: unknown rates are `$?.??`, never a fake `$0.00`. The session line is honest; the budget line is not. A 0% bar backed by `$0.00` while the session total is unknown is the exact lie the spend system was built to avoid.

**Done when:** If session spend is unknown, the budget remainder does not render as `$0.00`. Use `$?.??` (or omit the dollar figure) until a priced turn exists. Snapshots no longer contain `$0.00` beside `$?.??`.

---

## G-05 — Hint bar silently drops keys at 80 columns

**Severity:** medium
**Where:** `snapshots/stream-collapsed-80x24.txt`

Idle 80×24 shows `^c quit`. Streaming 80×24 ends at `^b panel` and drops quit. Context-sensitive hints that overflow by truncation lose the key you actually need (cancel/quit while a turn is running).

**Done when:** The 80-wide streaming hint bar still names cancel/quit, or uses a documented shorter form rather than chopping the right end.

---

## G-06 — Permission interrupt still fights the sidebar at 80×24

**Severity:** medium (subset of G-01, called out because it is safety UI)

Destructive-tool Ask is the one overlay that must be unmistakable. `modal-permission-80x24.txt` is readable in the middle and noisy at the edges: session card pipes, `ld`, `━━━` colliding with chat. Heavy top edge (`┏`) is the right vocabulary; the collision is not.

**Done when:** The permission modal is a single closed shape. No session/model/spend fragments inside it. `y` / `n` / `a` remain the only actions.

---

## G-07 — Untitled session uuid is the first thing in the column

**Severity:** low
**Where:** session card on every idle snapshot

`untitled` / `0193abcd` / `build` is operator chrome. Fine in `/sessions`. As the top card of an empty product it is a shrug. Title can stay “untitled” until the first user turn; the raw id does not need that slot.

**Done when:** The session card leads with title + phase (and auditor/tools). The id is secondary, truncated, or only in `/sessions`.

---

## Out of scope here

These are already listed in `ROADMAP.md` Now / Next. Do not treat them as this review’s findings:

- O-01 `Spend` as the `busy` fallback until headless/MCP read `TurnFinished`
- O-02 `ryter doctor --json`
- O-03 `light` theme experimental
- O-04 per-message copy / clipboard

---

## Suggested order

1. G-01 + G-02 + G-06 (overlay layer — one fix, three screens)
2. G-04 (spend honesty)
3. G-03 (empty cards)
4. G-05 (hint overflow)
5. G-07 (session card)

Regenerate snapshots with `UPDATE_SNAPSHOTS=1` only after the change is intended. Review the snapshot diff; do not accept leaks as golden.
