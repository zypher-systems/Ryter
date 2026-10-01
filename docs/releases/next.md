<!--
Notes for the next release: everything in `dev` that isn't in `main` yet.
Each patch adds its part here. At release, this file becomes
`docs/releases/vX.Y.Z.md`, with a summary paragraph and the Upgrade section
added at the top, and this comment removed.
-->

## What was wrong

- **A shell-tool test failed now and then in CI.** It starts a background process that detaches itself with `setsid`, then checks that Ryter stops waiting for it. On a busy machine, the command could finish, and Ryter could stop its process group, before `setsid` had detached. Ryter then correctly reported that it had stopped the background process, but the test expected the other outcome. Ryter behaved correctly both ways.
- **Two of these tests used `pkill -f` and `pgrep -f`** to find their process by a pattern, and needed `setsid`, `pkill` and `pgrep` to be installed.

## What changed

- **The shell tool's background tests can't race.** Nothing changes in how Ryter works.
  - The background job now writes its process ID to a file before the command goes on. The test that detaches it gives it its own process group through job control (`set -m`). It has left the group before Ryter could stop it, so the race can't happen.
  - Both tests find their process by the ID it wrote. They clean up by that ID too, with no pattern matching and no extra programs.

## Tried before release

- **The shell tests:** the fixed tests passed 15 runs out of 15 with every CPU core busy.
