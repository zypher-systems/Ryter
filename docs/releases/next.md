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
- **The `workspace` sandbox was unusable, and nothing said what the profiles meant.** `/settings` had a `profile` field with three names and no explanation. Under `workspace` on a real machine:
  - `git` could not start, because it opens `/dev/null` for writing and `/dev` was read-only;
  - `cargo`, `rustc` and anything else installed under the home folder were refused;
  - nothing could make a temporary file, since `/tmp` is shut and `TMPDIR` pointed nowhere;
  - no file could be moved from one folder to another, so `cargo` could not build a library ("Invalid cross-device link");
  - a crew's builders could not write in their worktrees.

## What changed

- **A specialist's last step is for writing up.** On the last of its steps it is told so, gets no tools, and is told what its answer must contain. Its report says it reached its limit. This now happens with or without a spending limit.
- **A review with no verdict decides nothing.**
  - The auditor is asked once for its verdict, with no tools.
  - If it still gives none, the task stops at the gate with its work kept on its branch. It isn't rejected, no retry is spent, the builder isn't run again, and nobody is told to change the builder.
  - Setting the task to pending audits it again without rebuilding it.
- **The auditor knows what it can't run.** Its instructions and each refusal say that its shell runs test runners, linters and read-only commands, and refuses containers, servers, installs and the project's own shell scripts. They also say the limit is the auditor's own and that a builder can run those commands.
- **Work the auditor can't run goes on as "reviewed, not run".** It ends with `VERDICT: UNVERIFIED`, the task lands on the patch marked, and the patch waits until checks have built and tested it. That rule already existed for code that can't be built until another task lands. For a project tested in a container, put those commands in `[auditor] checks` and Ryter runs them itself.
- **You can set how many steps each specialist gets.** `/settings` → *agents* has builder, architect and auditor steps, with the defaults (40, 30, 12) in the labels. They are also `[subagents.steps]` in `config.toml`. A change applies from the next task.
- **`/settings` compares the sandbox profiles.** Under the `profile` field is a table of what `off`, `workspace` and `read-only` each let commands read, write and run, with the chosen one picked out, when to use each, and what a sandbox doesn't stop (the network, and Docker). The default is still `off`. The guide has the same comparison, with what "your tools" covers.
- **The `workspace` and `read-only` profiles work with a real toolchain.** Commands can now:
  - write to `/dev/null` and the other standard devices, so `git` runs;
  - read and run your toolchains under your home folder (`~/.cargo/bin`, `~/.rustup`, node managers, `~/.local/bin`, pipx, uv), and any folder on your `PATH` there;
  - write those tools' download caches, so a build that fetches a dependency works;
  - make temporary files, in `~/.ryter/tmp`;
  - move a file between folders they may write (on Linux 5.19 or later);
  - write a crew's worktrees.

  The rest of your home folder, `~/.ssh`, the tools' saved logins and Ryter's keys stay shut.
- **A key read from the environment is kept out of every command**, whatever its variable is called. Only `XAI_API_KEY` and `OPENROUTER_API_KEY` were, so a key under another name was handed to each command the model ran.
- **Under a sandbox, Ryter's keys can't be read from its own process.** `/proc` is readable in the sandbox, and a command could read a key from Ryter's environment or memory there. Ryter now closes its process to other processes when a profile is on.
- **`ryter --sandbox workspace bench`** runs the benchmark's crew inside the sandbox.
- **The benchmark has a tenth task, `runner-script`:** a feature in a project whose tests run only through its own script, which the auditor's shell refuses.

## Tried before release

- **The crew's new behaviour, with scripted models:** six new tests, each of which fails on 0.10.0:
  - An auditor that ends on "I'll use Podman to build and run the six checks" is asked once for a verdict. With none, the task is blocked and unrejected, the builder isn't called again, and the next run calls only the auditor.
  - Asked for its verdict, an auditor that gives one is taken at it.
  - A builder and an auditor that work until one step is left are each told "this is the last of your steps", with no tools, and the report says they reached the limit.
  - With no checks, the auditor's brief names what its shell refuses, and no longer says to build the project itself.
  - `docker compose build`, `podman compose up` and `./dev test` are refused to an auditor with "a limit on the auditor, not on this machine". `sudo` is not blamed on the role.
- **The new benchmark task, once, with a real crew** (lead deepseek-v4.1-flash, builder glm-5.3, auditor grok-4.7): it landed and passed the hidden tests in 75 seconds for $0.098. The auditor used 7 of its 12 steps: it read the change, wrote a scratch test for the rounding and the unknown currency, and gave its verdict. It didn't try the project's script.
- **Step limits:**
  - In the TUI, `/settings` showed the three limits with their defaults. I raised the builder's to 50 and lowered the auditor's to 10, saved, and restarted: both were kept, and marked as set by you.
  - With scripted models, an auditor given six steps was told "this is the last of your 6 steps" on its sixth, and a builder given five was stopped on its fifth, with the report saying so.
- **Not yet tried:** the project this came from. It needs this build run against it, which is the next step.
