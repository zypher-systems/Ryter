<!--
Notes for the next release: everything in `dev` that isn't in `main` yet.
Each patch adds its part here. At release, this file becomes
`docs/releases/vX.Y.Z.md`, with a summary paragraph and the Upgrade section
added at the top, and this comment removed.
-->

## What was wrong

- **A reviewer couldn't test a project that tests in containers, and said the machine had no Docker.** The auditor's shell, and the review hat's, refused every `docker` and `podman` command, so on a Docker project neither could run the tests. The auditor's own instructions said it "refuses containers", and it reported that to the user as "there is no docker or podman in my environment". Docker was there all along.
- **`docker compose build` failed under the `workspace` sandbox** with "~/.docker/buildx/.lock: permission denied". A crew's builder spent its steps working around it and committed its scratch files.

- **`/models` crashed Ryter beside the rail.** With the side rail showing and a terminal narrower than about 158 columns, opening `/models` ended the program with "index outside of buffer". The panel asks for 124 columns and was sized to the whole screen, not to the space beside the rail, so it was drawn past the right edge. It has done this since 0.9.1. Hiding the rail (`^b`) or a wider terminal avoided it.

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
- **Your rules could only be read or changed by hand outside Ryter, or through the model.** There was no place in Ryter to see what your rules were, and `/rules` did nothing without something to remember.
- **Every hat ran on one model.** You could not plan with a strong model and build with a cheap one, or have a different model review the work, without changing the model by hand at each step.
- **There were two reviewers.** The review hat critiqued your changes on the model you were working with, and `/audit` ran a second model apart from the conversation, with its own chooser, its own limit and its own card in the chat. Neither checked the work against a plan, and a commit didn't say whether the work had been reviewed.
- **A plan was a message, approved by a question about something else.** The plan hat wrote its plan into the chat, and work started when you answered a yes/no card about switching hats. There was no way to say "change this part first", and the plan was nowhere but the chat's history.

## What changed

- **A reviewer tests in the project's containers.** The auditor and the review hat may now:
  - run a test or lint command in one of the project's containers: `docker compose run --rm web pytest`, `docker compose exec web ruff check .`, `docker exec <container> …` (`podman` the same). The command inside answers to the same rules as outside, so `ruff format .`, `pip install` or a shell are still refused.
  - look at what is running: `docker compose ps`, `docker compose logs web`, `docker ps`, `docker images`.
  - They still may not build, start, stop or remove containers, use `docker run`, mount a folder, or point Docker at another machine. A refusal now says what does run in containers, and that Docker is there.
- **`docker build` works under a sandbox profile.** Its lock folder, `~/.docker/buildx`, is writable. The registry logins beside it (`~/.docker/config.json`) stay shut.
- **`/settings` and the guide say that rootless Podman can't run under `workspace` or `read-only`.** It needs its state in your home folder and a user namespace of its own, and a sandboxed command gets neither.

- **No panel is drawn off the screen.** A panel is sized to the space it is drawn in, and whatever it asks for is cut to the screen. `/models` opens beside the rail at any width.

- **A specialist's last step is for writing up.** On the last of its steps it is told so, gets no tools, and is told what its answer must contain. Its report says it reached its limit. This now happens with or without a spending limit.
- **A review with no verdict decides nothing.**
  - The auditor is asked once for its verdict, with no tools.
  - If it still gives none, the task stops at the gate with its work kept on its branch. It isn't rejected, no retry is spent, the builder isn't run again, and nobody is told to change the builder.
  - Setting the task to pending audits it again without rebuilding it.
- **The auditor knows what it can't run.** Its instructions and each refusal say that its shell runs test runners, linters and read-only commands, and refuses containers, servers, installs and the project's own shell scripts. They also say the limit is the auditor's own and that a builder can run those commands.
- **Work the auditor can't run goes on as "reviewed, not run".** It ends with `VERDICT: UNVERIFIED`, the task lands on the patch marked, and the patch waits until checks have built and tested it. That rule already existed for code that can't be built until another task lands. For a project tested in a container, put those commands in `[auditor] checks` and Ryter runs them itself.
- **You can set how many steps each specialist gets.** `/settings` → *agents* has builder, architect and auditor steps, with the defaults (40, 30, 12) in the labels. They are also `[subagents.steps]` in `config.toml`. A change applies from the next task.
- **Each hat can have its own model.** `/models` now lists *All hats*, *Plan*, *Build* and *Review*. A hat follows *All hats* until you give it a model.
  - The rail and the status line show the model your next message goes to.
  - The hats share one conversation. When a model that hasn't read it takes over, the chat says what re-reading it costs: "review hat · grok-4.7 re-reads 42k tokens, about $0.13".
  - In crew mode `/models` shows the crew's seats, as before.
  - In the chat, each model is named as it takes over, so a reply reads as the model that wrote it.
- **One reviewer: the review hat.** `/audit`, and the offer after a build turn that changed files, now run a review in the review hat, on the model you gave it in `/models`.
  - **It asks first,** as before: the model, what it reviews, and a cost range. If the review hat has no model of its own, the prompt says the reviewer is the model that built the work.
  - **It is a turn in the conversation.** The reviewer has read what you asked for, and the builder reads its findings next. The hat you were in comes back when it ends.
  - **It checks the change against the plan you approved,** then for bugs, tests and safety, and ends with `VERDICT: PASS` or `VERDICT: FAIL`.
  - **A failed review offers its fixes** in the build hat, and a review of the fixes is offered after.
  - **The commit receipt says whether the work was reviewed:** "review ✓ grok-4.7", "review ✗ grok-4.7", "not reviewed", or "not reviewed after the last change". A verdict holds only for the files the reviewer read.
  - **The limit is a setting:** `/settings` → *review usd* (0 is no limit). It holds any turn in the review hat.
  - **Your 0.10.0 choice carries over.** The model you chose for `/audit` becomes the review hat's model, and its limit becomes the review limit, until you change them in `/models` and `/settings`.
  - **Gone:** the reviewer chooser, `/audit model`, the audit's own card in the chat, and the rule that the reviewer must be a different model. A custom `second.md` prompt is no longer read; the review's instructions are in `solo.md`.
- **A plan is read and approved in its own panel.** The model shows its plan (goal, steps, files, risks, how to verify) in a scrolling panel:
  - `y` approves: the plan is saved as `.ryter/plans/<date>-<title>.md` in the project, and the model builds from it in the build hat, in the same turn.
  - `e` adjusts: you type what to change, and the model shows the revised plan.
  - `n` rejects: nothing is saved.
- **`/rules` opens a panel for your rules.** It shows the rules for every project and the rules for this one on two tabs, with the file and how many rules it holds.
  - `a` adds a rule under the selected line, `d` removes a line once you say yes, and `e` opens the file in your editor.
  - No model is called, so nothing is asked and nothing is spent.
  - `/rules <what to remember>` still gives that to the model, which shows you the change and asks before it saves.
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

- **A reviewer and containers:**
  - In the TUI, in the review hat, against a stand-in provider on a machine with Docker: `docker ps` ran and listed the containers; `docker compose run --rm web pytest -q` ran (Docker answered that the test folder has no compose file); `docker compose up -d --wait` and `docker run -v /:/host …` were refused, each with "Docker is here, and tests and linters do run in the project's containers".
  - Under the real `workspace` sandbox on this machine: `docker version`, `docker ps`, `docker compose version` and `docker buildx ls` all worked. Without the new grant `docker buildx ls` failed with the same "buildx/.lock: permission denied" the crew's builder hit.
  - Rootless Podman under the sandbox: `podman ps` failed on its database, and a user namespace could not be set up at all (`unshare -Urm` failed where it works outside the sandbox).
  - Not tried: a real crew or review run on the Docker project this came from. It needs this build.

- **`/models` beside the rail,** in the TUI at 110 columns, where 0.10.0 crashes: it opened, with every model's name and prices in view. A new test opens every panel at every width from 40 to 200 columns, with and without the rail; it crashes on 0.10.0.

- **The crew's new behaviour, with scripted models:** six new tests, each of which fails on 0.10.0:
  - An auditor that ends on "I'll use Podman to build and run the six checks" is asked once for a verdict. With none, the task is blocked and unrejected, the builder isn't called again, and the next run calls only the auditor.
  - Asked for its verdict, an auditor that gives one is taken at it.
  - A builder and an auditor that work until one step is left are each told "this is the last of your steps", with no tools, and the report says they reached the limit.
  - With no checks, the auditor's brief names what its shell refuses, and no longer says to build the project itself.
  - `docker compose build`, `podman compose up` and `./dev test` are refused to an auditor with "a limit on the auditor, not on this machine". `sudo` is not blamed on the role.
- **The new benchmark task, once, with a real crew** (lead deepseek-v4.1-flash, builder glm-5.3, auditor grok-4.7): it landed and passed the hidden tests in 75 seconds for $0.098. The auditor used 7 of its 12 steps: it read the change, wrote a scratch test for the rounding and the unknown currency, and gave its verdict. It didn't try the project's script.
- **The `workspace` sandbox, on a real machine:**
  - **By hand,** inside the profile: `git`, `cargo`, `rustc`, `node`, `npm` and `python3` ran; `git commit` and `git worktree add` worked; a Rust program and a Cargo project built; `mktemp` made its file in `~/.ryter/tmp`. `~/.ssh`, the home folder and `/tmp` were refused. `docker ps` still answered, as the table says it will.
  - **The benchmark's reference solutions,** tested inside the profile: the Rust library, the TypeScript library, the project run by its own script, and the Python tasks all passed their visible and hidden tests.
  - **The whole suite with a real crew,** inside the profile (lead deepseek-v4.1-flash, builder glm-5.3, auditor grok-4.7):
    - All ten tasks landed, and nine passed the hidden tests.
    - The tenth, the TypeScript task, was a false pass, on the same hidden test a crew missed once before without a sandbox (`off` must still remove a `once` handler). It is not the sandbox's doing.
    - It took two runs. The first stopped the Rust task with "Invalid cross-device link", which is how the missing right to move files was found. Its builder spent 120 steps and $0.45 trying to work round it. Fixed, the same task landed in 7 builder steps.
    - Cost: $1.87 over both runs.
- **A model per hat,** in the TUI against a stand-in provider with three models:
  - `/models` listed *All hats*, *Plan*, *Build* and *Review*, each hat "follows all hats". I gave Review its own model; the seat showed it with a ✓, and `hats.toml` held it.
  - A message in the build hat went to the main model, one in the review hat to the reviewer's, and the next in build back to the main one: the provider's log showed each.
  - On the switch the chat said "review hat · reviewer-x re-reads 3.1k tokens, about $0.01". The provider reported 3,000 tokens read.
  - With Review selected, the rail's MODEL line named the reviewer's model.
- **The review, as one reviewer,** in the TUI against a stand-in provider, with a 0.10.0 `review.toml` in the home folder:
  - `/models` showed the review seat on the model from `review.toml`, and `/settings` showed its $2.00 limit.
  - A build turn changed a file. The offer named the reviewer's model, "reviews 1 file, +1 −1", and "about $0.03–$0.60 of your $2.00 limit". `⏎` ran it: the request went to the reviewer's model, the chat showed the review under that model's name and "review · reviewer-x · ✓ no blocking problems · $0.01", and the build hat came back.
  - `/commit` showed "review ✓ reviewer-x" in the receipt. After I changed the file by hand it showed "not reviewed after the last change".
  - A review that failed asked to switch to the build hat. On yes, the main model made the fix in the same turn, the chat said "✗ blocking problems", and a second review was offered. That one passed.
  - `/audit` with `n` said "review not run" and sent nothing. With a one-cent limit it said the first step would pass the limit, and asked nothing.
  - `s` on the offer stopped the offers and saved that.
- **The plan panel,** in the TUI against a stand-in provider that presented a five-section plan:
  - The panel showed the plan under its title, each heading with its section under it, all on screen without scrolling.
  - `e`, then "Stream the rows; skip the button for now": the model was told those words and showed a revised plan. Nothing had been saved.
  - `y`: the chat said "plan · approved and saved to .ryter/plans/2026-10-01-add-csv-export-to-reports.md" and "switched to the build hat", the file held the plan under its title, and the model went on in the same turn.
  - `n` on a second plan: nothing more was saved, and the hat stayed on plan.
- **The `/rules` panel,** in the TUI, with its own home folder and no model:
  - It opened on the every-project rules: both tabs, the file, "4 rules", and a long rule wrapped under its own text.
  - `a`, a typed rule and Enter added it under the selected line, and the file held it. `d` asked "remove …?", and `y` removed it.
  - Tab showed the project's file as "not created yet". The first rule added there created `RYTER.md`.
  - `e` handed the file to the editor, and the panel showed the editor's change when reopened. It first failed here with `VISUAL` set to nothing, which is fixed.
- **The sandbox table,** in the TUI: under the `profile` field at 110 and 80 columns, with the chosen profile in capitals as the choice moved.
- **Step limits:**
  - In the TUI, `/settings` showed the three limits with their defaults. I raised the builder's to 50 and lowered the auditor's to 10, saved, and restarted: both were kept, and marked as set by you.
  - With scripted models, an auditor given six steps was told "this is the last of your 6 steps" on its sixth, and a builder given five was stopped on its fifth, with the report saying so.
- **Not yet tried:** the project this came from. It needs this build run against it, which is the next step.
