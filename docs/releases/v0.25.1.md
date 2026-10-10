The conversation sits at the left and the sidebar grows with the screen, so a wide terminal has no blank margin where the rack was. `Shift+Tab` is the direct switch again, both ways.

## Upgrade

```sh
ryter update
```

Coming from 0.25.0:

- **No blank left margin on a wide terminal.** The conversation's column starts at the left edge, a cell in, and stays at most 112 columns wide; the spare width on a wide screen is between it and the sidebar, where the sidebar takes up to ten of it for model names and paths (30 columns on a narrow screen, 40 on a wide one).
- **`Shift+Tab` goes straight to the other row again.** From plan or build it puts on the specialist you last wore (audit, when you haven't), and `Tab` goes round the specialists; from a specialist it goes back to the primary hat you last wore. The picker that 0.25.0 opened on the way into the specialists is gone.

## Why

The first run of 0.25.0 on a wide terminal: with the rack gone, the column centred in what was left of the sidebar left a wide blank on the left; and a box on the way into the specialists with none on the way back read as two gestures on one key. `DECISIONS.md` 2026-10-10 (third entry).

## Tried before release

The real binary at 220×50 and 120×40 against the simulated provider; the golden snapshots at four sizes; a test pinning the column's place and the sidebar's width at six screen widths.
