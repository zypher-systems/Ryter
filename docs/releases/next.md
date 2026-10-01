<!--
Notes for the next release: everything in `dev` that isn't in `main` yet.
Each patch adds its part here. At release, this file becomes
`docs/releases/vX.Y.Z.md`, with a summary paragraph and the Upgrade section
added at the top, and this comment removed.
-->

## What was wrong

- **A crew could not get past a task whose auditor ran out of steps.** On a real project (a Docker web app) the first builder task never landed, after six rounds and $2.83:
  - **Specialists stopped without warning.** A builder has 40 steps and an auditor 12. They were told to stop and write up only when a spending limit was set. With none, the run just ended, and the half-sentence beside its last tool call became its report.
  - **A review with no verdict counted as a rejection.** The auditor's last words were "I'll use Podman to build and run the six checks". That was read as a FAIL. The builder was run again on work nobody had faulted, the round was added to its rejections, and you were told to choose a stronger builder.
  - **The auditor was told to do what it can't.** With no checks set, its instructions said "build it and run its tests yourself". Its shell runs test commands only, and refuses `docker` and `podman`, so it spent its steps looking for a way round.
  - **A refusal read as the machine's limit.** The auditor was told the command was "blocked". It reported that Docker was blocked, the lead told you so, and it sent the builder to Podman. Docker worked on that machine all along.

## What changed

- **A specialist's last step is for writing up.** On the last of its steps it is told so, gets no tools, and is told what its answer must contain. Its report says it reached its limit. This now happens with or without a spending limit.
- **A review with no verdict decides nothing.**
  - The auditor is asked once for its verdict, with no tools.
  - If it still gives none, the task stops at the gate with its work kept on its branch. It isn't rejected, no retry is spent, the builder isn't run again, and nobody is told to change the builder.
  - Setting the task to pending audits it again without rebuilding it.
- **The auditor knows what it can't run.** Its instructions and each refusal say that its shell runs test runners, linters and read-only commands, and refuses containers, servers, installs and the project's own shell scripts. They also say the limit is the auditor's own and that a builder can run those commands.
- **Work the auditor can't run goes on as "reviewed, not run".** It ends with `VERDICT: UNVERIFIED`, the task lands on the patch marked, and the patch waits until checks have built and tested it. That rule already existed for code that can't be built until another task lands. For a project tested in a container, put those commands in `[auditor] checks` and Ryter runs them itself.
- **The benchmark has a tenth task, `runner-script`:** a feature in a project whose tests run only through its own script, which the auditor's shell refuses.

## Tried before release

- **The crew's new behaviour, with scripted models:** six new tests, each of which fails on 0.10.0:
  - An auditor that ends on "I'll use Podman to build and run the six checks" is asked once for a verdict. With none, the task is blocked and unrejected, the builder isn't called again, and the next run calls only the auditor.
  - Asked for its verdict, an auditor that gives one is taken at it.
  - A builder and an auditor that work until one step is left are each told "this is the last of your steps", with no tools, and the report says they reached the limit.
  - With no checks, the auditor's brief names what its shell refuses, and no longer says to build the project itself.
  - `docker compose build`, `podman compose up` and `./dev test` are refused to an auditor with "a limit on the auditor, not on this machine". `sudo` is not blamed on the role.
- **The new benchmark task, once, with a real crew** (lead deepseek-v4.1-flash, builder glm-5.3, auditor grok-4.7): it landed and passed the hidden tests in 75 seconds for $0.098. The auditor used 7 of its 12 steps: it read the change, wrote a scratch test for the rounding and the unknown currency, and gave its verdict. It didn't try the project's script.
- **Not yet tried:** the project this came from. It needs this build run against it, which is the next step.
