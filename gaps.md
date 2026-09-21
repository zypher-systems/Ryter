# TUI redesign — review gaps

Status: **resolved** (all G-01..G-07 landed)
Against: `0.2.0-patch` (`60adb20` Implement 0.2.0 TUI redesign)
Fixed in: `8e317d1` (G-01/G-02/G-04/G-06), `aa51ad0` (G-03/G-05/G-07)
Evidence: golden snapshots under `crates/ryter-tui/snapshots/`
Not a new design contract. `design.md` still wins on intent.

The 0.2 direction is right: conversation in the center, composer always on screen, command palette, one panel per config surface, activity strip for thinking, info cards instead of a flat key/value dump. These were the gaps.

---

## G-01 — Panels do not sit in a layer — **fixed**

**Was:** high · `panel/chrome.rs`, every panel/modal snapshot

**Corrected diagnosis.** Nothing leaked *through* the panel edge: `chrome::draw_frame`
already renders `Clear`, and the `┃` inside `panel-help` was that panel's own
scrollbar. What actually happened is that `info::draw` kept painting the sidebar
underneath a centred float, so the panel covered the cards' left half and left
their tails beside its border (`k-4.6`, `ns`, `ew 0/4`, `┃ build`). Not a leak —
an uncovered region.

`dim_region` only rewrites `fg`, so on a real terminal the cards *were* dimmed.
The snapshot harness strips every style, which is why the captures looked worse
than the product. See **S-01** below.

**Fixed by:** the sidebar is not painted while a panel is open (`draw.rs`), and
the chat scrollbar is suppressed too — a panel owns the scroll keys, so a live
scrollbar beside it was misleading as well as noisy. Regression test:
`an_open_panel_leaves_no_card_fragments`.

---

## G-02 — Help is the least readable panel — **fixed**

**Was:** high · `snapshots/panel-help-100x30.txt`

Two causes: the cards stole the width (G-01), and `panel::rect` reserved ten
rows of the terminal, giving Help 20 rows for ~40 rows of keybindings at 100×30.
Now a panel may use the body it owns, with two rows of margin.

---

## G-03 — Empty info cards keep the space the chat is supposed to own — **fixed**

**Was:** medium · `snapshots/idle-120x40.txt`, `idle-160x50.txt`

`cards::tasks` / `cards::crew` return `Option<Card>`, as `cards::mcp` already
did. Regression test: `empty_cards_are_absent_not_blank`.

**One correction to the original "done when".** Giving the rows "to chat" does
not follow: chat is a *vertical* sibling of the sidebar, so dropping a card
cannot lengthen it. `idle-160x50` still has blank rows because the transcript is
top-anchored — `ChatScroll::resolve` offsets to `doc_rows - viewport` (zero when
content is short) and `layout::frame` pads below. Whether a short transcript
should sit *above* the composer like a terminal chat is a real design question,
but it belongs in `design.md`, not here. Tracked in `ROADMAP.md` Next.

---

## G-04 — Spend prints `$0.00` next to `$?.??` — **fixed**

**Was:** high (product rule) · every idle and most panel snapshots

Unknown spend now renders `?%` and `$?.?? of $5.00` in both the sidebar card and
the `/spend` panel, which had the same bug. The header carried it too: its
`unwrap_or(0.0)` made unknown spend compare as *under* budget and colour itself
safe. No snapshot contains `$0.00` any more.

Worth keeping in view: with an unpriced model `over_budget` correctly never
trips, so there is effectively **no cap at all**. The bar no longer claims
otherwise, but the gap is real — see `ROADMAP.md`.

---

## G-05 — Hint bar silently drops keys at 80 columns — **fixed**

**Was:** medium · `snapshots/stream-collapsed-80x24.txt`

Hints carry a priority (`Hint::Essential/Useful/Optional`); overflow drops the
optional ones first and never cancel, quit, or the permission answers. Also
fixed an off-by-4: the first hint was charged for a separator it does not draw,
which is why the 80-column bar overflowed in the first place. Regression test:
`hint_bar_never_drops_cancel_or_quit`, down to 32 columns.

---

## G-06 — Permission interrupt still fights the sidebar at 80×24 — **fixed**

**Was:** medium (safety UI) · subset of G-01

Resolved with G-01; the modal is now a single closed shape with only transcript
text behind it.

**The "double pipe `│╭─ session`" was not a leak** — that is the chat gutter
scrollbar next to the card border, by design. Chasing it would have wasted a
pass.

**What this review missed, and it was the more serious half:** the modal
accepted `KeyCode::Enter` as an alias for allow. `Enter` is the send key in the
composer, so a reflex press approved a destructive call on the one overlay that
must be unmistakable — and the modal body never advertised it. Only `y` allows
now (`c7d1a25`).

---

## G-07 — Untitled session uuid is the first thing in the column — **fixed**

**Was:** low · session card on every idle snapshot

Title and phase lead the card; the id stays in `/sessions`. Regression test:
`session_card_leads_with_title_and_phase`.

---

## S-01 — The snapshot harness cannot see style — **open**

**Severity:** medium · `draw.rs::buffer_to_string`

The finding under G-01. `buffer_to_string` keeps symbols and discards every
style, so:

- `R-POP-04` ("dim everything behind a panel") is **unassertable**. `dim_region`
  could regress to a no-op and every snapshot would still pass.
- Judgments about "noise" are made against an artifact strictly uglier than the
  product, which is how G-01 came to be described as a leak.

**Done when:** a second dump mode emits a per-cell attribute map (or a
fg-class character per cell) alongside the glyph capture, and at least one test
asserts that cells behind an open panel are dimmed.

---

## Out of scope here

Already in `ROADMAP.md` Now / Next:

- O-01 `Spend` as the `busy` fallback until headless/MCP read `TurnFinished`
- O-02 `ryter doctor --json`
- O-03 `light` theme experimental
- O-04 per-message copy / clipboard — note `Ctrl+G` now releases the mouse so
  the terminal's own selection works in the meantime

---

Regenerate snapshots with `UPDATE_SNAPSHOTS=1` only after the change is
intended. Review the snapshot diff; do not accept leaks as golden.
