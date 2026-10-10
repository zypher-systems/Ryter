The screen reads as a tool: the watermark is off unless asked for, the hat that is on is the one colored thing, and the transcript's rows are quieter.

## Upgrade

```sh
ryter update
```

From 0.8.2 or earlier, run the install script once:

```sh
curl -fsSL https://raw.githubusercontent.com/zypher-systems/ryter/main/install.sh | sh
```

Coming from 0.24.1, this looks different:

- **The fedora behind the conversation is gone unless you ask for it.** `[ui] watermark = true` in `~/.ryter/config.toml`, or *watermark* in `/settings`, brings it back.

## What changed

- **One colored thing.** The hat that is on colors its chip, the prompt's rule, its block in the rack, the model's name and the context gauge, as before. The other hats' marks in the top bar and the rack are plain now, a folded turn's dot is dim, and the model's name in the conversation is neutral.
- **Transcript rows are led by space, not dots.** A folded turn ends in what it came to (`✓ 2 tools · 2 files (1 new, 1 changed, +8 −1)  ▸`) without its time and cost, which stay on the turn's closing line that `^o` opens. A tool row shows its time once it is a second or more; `0.0s` beside every edit is gone.
- **The prompt asks its question and no more.** `what should change?`, `what should we plan?`. The keys are on the hint bar beneath it, as before.
- **The rack is headed `HATS`.** A hat with no turns says `no turns yet`, and the build block's check rows leave out a count of zero.
- **The pulse is shown while a turn runs.** It is the last card in the instruments, so nothing above it moves when it comes and goes. Between turns it is not there.

## Why

A few people who saw Ryter said the same thing: it works well and looks like a toy. The execution was fine; the register was wrong. A mascot in the work area, four hat colors on every row, dot leaders, help said twice and gauges that read `idle` made the screen playful where a tool people trust at work is quiet. The hats stay; the screen stops performing them. `DECISIONS.md` 2026-10-10 has the choices and what was rejected.

## Good to know

- Nothing about what the hats do changed. `Tab` and `⇧Tab` are where they were, and the hint bar still names them.
- `watermark = true` in `[ui]` is the one setting whose default changed. A file that set it stays as it was.
- The README's screenshots are from before this pass.
