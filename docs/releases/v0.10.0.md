<!--
Notes for the next release: everything in `dev` that isn't in `main` yet.
Each patch adds its part here. At release, this file becomes
`docs/releases/vX.Y.Z.md`, with a summary paragraph and the Upgrade section
added at the top, and this comment removed.
-->

## What was wrong

- **Ryter had rules for a project, but none for you.** A project's `RYTER.md` reached the model, but there was nowhere to put rules that hold in every project. The nearest thing, `~/.ryter/prompts/`, replaces Ryter's own instructions and doesn't add to them. The model also had no way to save a rule when you told it one.
- **A shell-tool test failed now and then in CI.** It starts a background process that detaches itself with `setsid`, then checks that Ryter stops waiting for it. On a busy machine, the command could finish, and Ryter could stop its process group, before `setsid` had detached. Ryter then correctly reported that it had stopped the background process, but the test expected the other outcome. Ryter behaved correctly both ways.
- **Two of these tests used `pkill -f` and `pgrep -f`** to find their process by a pattern, and needed `setsid`, `pkill` and `pgrep` to be installed.

## What changed

- **Your rules, for every project: `~/.ryter/RYTER.md`.** Ryter puts the file into the instructions of every role on every message, ahead of the project's own `RYTER.md`. Where the two differ, the project's win. Edit it by hand whenever you like.
- **The model can save a rule when you tell it one.** Say "from now on…", or type `/rules <what to remember>`. A new built-in `rules` skill covers when a rule is worth saving, which file it belongs in, and how to write it. A new `update_rules` tool makes the change.
- **Nothing is saved without you.** Ryter shows the change line by line and asks every time.
  - Only `y` saves it. There's no "always", and `--always-approve` doesn't skip it.
  - If you say no, the file is left as it was.
  - A headless run saves nothing.
  - Under `--sandbox` the file can be read but not changed.
- **The shell tool's background tests can't race.** Nothing changes in how Ryter works.
  - The background job now writes its process ID to a file before the command goes on. The test that detaches it gives it its own process group through job control (`set -m`). It has left the group before Ryter could stop it, so the race can't happen.
  - Both tests find their process by the ID it wrote. They clean up by that ID too, with no pattern matching and no extra programs.

## Tried before release

- **Your rules, in the TUI with deepseek-v4.1-flash:**
  - **Saving:** I told it "From now on, always answer me in British spelling. Save that as one of my rules for every project." It loaded the `rules` skill and called `update_rules`. The prompt showed the three new lines of `~/.ryter/RYTER.md`, with only *y allow once* and *n deny*. After `y`, the chat said "rules · saved to ~/.ryter/RYTER.md", and the file held the rule.
  - **A brand-new session:** asked to quote my standing rules, it quoted that rule exactly, and wrote "centre" in its next sentence.
  - **Saying no:** `/rules remove the British spelling rule` showed the removal as a diff. I answered `n`. The file was unchanged, and the model said so.
- **The shell tests:** the fixed tests passed 15 runs out of 15 with every CPU core busy.
