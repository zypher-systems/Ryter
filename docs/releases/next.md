Unreleased: what is in `dev` since 0.18.0. These notes become the next release's `vX.Y.Z.md` when the user calls the release.

## What changed

- **One key on the audit card.** `⏎` repairs a failed audit in the build hat and closes a passed one; `y` is gone from the legend and the keys. An audit that filed nothing and gave no verdict says `no verdict` in its title and closes on `⏎`, with no repair offered.
- **The round cap's stop message names all three ways to raise it:** *rounds a turn* in `/settings`, `[limits] rounds` in `config.toml`, and `--rounds N` for one run. The guide says the same.
- **The allow card's own key row keeps its keys on a narrow column.** The scope is said in a word, `a allow edits this session`, and a key that does not fit is left out without taking the ones after it, so `⏎`, `a` and `n` are on the row at 100 by 30 and 80 by 24.
- **The classic layout's foot line** shows a card's keys in the warn color while a card is open, as the rack screen's does.
