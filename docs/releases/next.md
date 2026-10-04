Unreleased: what is in `dev` since 0.18.0. These notes become the next release's `vX.Y.Z.md` when the user calls the release.

## What changed

- **One key on the audit card.** `⏎` repairs a failed audit in the build hat and closes a passed one; `y` is gone from the legend and the keys. An audit that filed nothing and gave no verdict says `no verdict` in its title and closes on `⏎`, with no repair offered.
- **The round cap's stop message names all three ways to raise it:** *rounds a turn* in `/settings`, `[limits] rounds` in `config.toml`, and `--rounds N` for one run. The guide says the same.
- **The allow card's own key row keeps its keys on a narrow column.** The scope is said in a word, `a allow edits this session`, and a key that does not fit is left out without taking the ones after it, so `⏎`, `a` and `n` are on the row at 100 by 30 and 80 by 24.
- **The classic layout's foot line** shows a card's keys in the warn color while a card is open, as the rack screen's does.
- **The run-file card is a question under the conversation.** "How this project runs" now takes the same inset slot as the allow card, with the reply still in view above it, instead of floating over the chat. Its note reads *ryter runs these without asking*. The foot's `esc` says what the card's `n` key does: `reject` on the run-file card, `not now` on the audit's cost card, `deny` on an allow card.
- **The plan and scribe hats may look at the product.** A `curl` GET or HEAD to the project's own address, saving nothing into the project, and a project program's own `--help`, `-h`, `--version` or `-V` (`.venv/bin/tasks --help`) run in both hats. Anything that sends, saves into the project or does more is refused as before, and the refusal now says the hat *only looks* and what it may run, instead of claiming the command "changes things".
- **`export NAME=value` sets a variable as `NAME=value` does.** The gate used to forget a name that `export` set, so `export F=/tmp/x; rm -f "$F"` asked as an unreadable path while `F=/tmp/x; rm -f "$F"` asked as a scratch-space deletion, with different card text. Both forms, quoted or not, on one line or several, now get the same decision and the same card. `declare`, `typeset`, `local` and `readonly` likewise. What asks has not changed.
- **The allow card names the command that asked.** The title and the `what` row show the segment the gate stopped on (`run  rm -f "$TASKS_FILE"`), not the script's first word (`run  set -e`); the whole script stays in the body.
- **An audit that used the product says so.** When the audit started the product, ran its tests, or called its address, audit.md's Tree section adds that whatever the product wrote to its own data, files git ignores, was not restored, and the card's footer says `product data not restored`. The checkpoint covers tracked and untracked files, as before.

## Why

From a driving run on 2026-10-04: two projects, a FastAPI service and a terminal to-do app, taken through plan, build, audit and scribe with real models for $0.94 in all. The flow held; these five are what the run showed a user would hit.

