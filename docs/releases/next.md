The screen is rebuilt around one sidebar: every row on it is a measurement, an event or a name. No bar across the top, no fedora anywhere, `RYTER` at the top of the sidebar, plan blue, build green and one colour for every specialist. `Shift+Tab` opens the specialists to choose from.

## Upgrade

```sh
ryter update
```

From 0.8.2 or earlier, run the install script once:

```sh
curl -fsSL https://raw.githubusercontent.com/zypher-systems/ryter/main/install.sh | sh
```

Coming from 0.25.0, this looks different:

- **One column beside the conversation, always.** The hat rack on the left and the instruments on the right are one sidebar on the right, from 100 columns up. It starts with `RYTER`, the session's title and where this is; then the hats as a ledger, what the model is doing now, the context gauge, spend with its cap, the files touched, and what the hat may do.
- **No bar across the top.** The conversation starts on the first row. The name, the title and the folder moved to the sidebar's head; the hats' marks to its ledger.
- **The fedora is gone**, the setting with it. A `config.toml` that still has `watermark` loads without a word.
- **Plan is blue, build is green, every specialist is amber.** A new specialist needs no new colour. Names are lowercase; only `RYTER` is in capitals.
- **`Shift+Tab` from plan or build opens the specialists to choose from**, with what each does, what it may do and its model. `Enter` puts the hat on and the prompt opens on its request, as before. From a specialist, `Shift+Tab` goes back to the primary hat you last wore.

## What changed

### The sidebar

- **hats.** Every hat, in its colour: plan and build, a hairline marked `specialists`, the specialists under it. The hat on is marked `▸`, and the mark spins while its model works. A hat with turns carries how many and what they cost; a specialist carries its last verdict, `✓` or `✗`; a hat with no turns is its name alone, and nothing says `no turns yet`. A row of dots is the session's shape, one a turn in its hat's colour. `plan.md` and `audit.md` are named only when they exist.
- **now.** The model, then what it is doing in the status row's own words, then how fast it is writing and how much it has written. While a question waits on you the block is amber and its last row names the question: `allow?  run git push`.
- **context, spend, changes, permissions.** The gauge; the session's cost against its cap and the project's cost; the files touched, a line each, then how many; what this hat may do and the sandbox. The `PULSE` and `GUARD` cards are gone: the pulse is the `now` block, and a guard that read `off · none · none` on every fresh session said nothing.
- **A short screen** gives up rows in order (the files list, the `turns` row, the rate, the permissions, the project's cost) before folding the sidebar away. Under 100 columns the last row is a status line: the hat, its model, the gauge, the cost and its cap. `^b` hides the sidebar, or opens it as a panel where it is folded.

### The opening screen

A new session opens on a block at the head of the conversation: `RYTER`, the folder, the branch and what is uncommitted; each hat with what it does and the key that reaches it, `◆ on` beside the one the session opens in; how many sessions this project has to `/sessions` back to. It is drawn, not sent, and gone once there is a turn. The welcome line is retired.

### The palette

The dark theme is muted: one background, hairlines a step apart, plan `#7FA7DB`, build `#8DC29B`, the specialists `#D0B077`, the user's mark a neutral grey-blue so it is not read as the plan hat. Every colour that names a thing meets WCAG AA on the background, and the chip's text on each hat's colour does too. The light theme keeps its structure; nobody has looked at it on this screen yet.

## Why

The first group of developers to use Ryter said it works well and looks like a toy. 0.25.0 took the toy signals off the old layout. This is the layout: the user's brief was unique and useful, not what every other harness looks like, with the useful parts visible without opening anything. A gauge that measures is a cockpit; a gauge that reads `off` is a prop. The contract is `docs/sidebar-design.md`; `DECISIONS.md` 2026-10-10 has the five routes that were drawn and why this one.

## Good to know

- Nothing about what the hats do changed. `Tab` is where it was.
- `[ui] panel`, `start_hat`, `theme` and `colors` are unchanged. `[ui] watermark` is retired and ignored.
- The README's screenshot is from before the screen pass and is owed a new one.

## Tried before release

The golden snapshots at four sizes in every hat, the opening screen, the allow card and the picker; the real binary against the simulated provider in a truecolor terminal and a sixteen-colour one, through all four hats, with a question open.
