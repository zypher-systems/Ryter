<!--
Notes for the next release: everything in `dev` that isn't in `main` yet.
Each patch adds its part here. At release, this file becomes
`docs/releases/vX.Y.Z.md`, with a summary paragraph and the Upgrade section
added at the top, and this comment removed.
-->

## What was wrong

- **Crew mode did not finish real work, and cost money trying.** It ran a lead, an architect, builders and an auditor without you. On a real project (a Docker web app) one session spent $4.36 and its first task was rejected seven times: the architect made the task too big, the auditor could not verify it, and the lead told you Docker was blocked on a machine where it worked. What one task cost varied about a hundredfold from run to run. No real project was completed that way.
- **Every hat ran on one model.** You could not plan with a strong model and build with a cheap one, or have a different model review the work, without changing the model by hand at each step.
- **There were two reviewers.** The review hat critiqued your changes on the model you were working with, and `/audit` ran a second model apart from the conversation, with its own chooser, its own limit and its own card in the chat. Neither checked the work against a plan, and a commit didn't say whether the work had been reviewed.
- **A plan was a message, approved by a question about something else.** The plan hat wrote its plan into the chat, and work started when you answered a yes/no card about switching hats. There was no way to say "change this part first", and the plan was nowhere but the chat's history.
- **Nothing used the product.** A review reads the change and runs the tests. Nobody started the product and tried it the way its user would, so work that passed its tests and its review could still be broken on the first page.
- **What you agreed after a plan was approved lived only in the chat.** Tell the builder "skip the export button for now" and the plan still said to build it. A reviewer holding the work against the plan had no way to tell that from a mistake.
- **The review hat could read any file on the machine.** A command that only reads (`cat`, `head`, `grep`, `ls`) ran in the review hat without a prompt wherever it pointed, so `cat ~/.ssh/id_rsa` or `cat /etc/passwd` would have run and sent the file to the model. The plan and build hats were not affected. This is in every release that has the review hat, 0.10.0 included.
- **The build hat asked about everything it did.** `cargo build`, `npm install`, `docker compose up`, a script of the project's own, `cd app`: each was a prompt, and the answer was yes each time. Writing a file to `/tmp` asked every time, with no way to allow it for the session, and the plan and review hats could not keep a test's output anywhere but their notes folder.
- **With Docker and Podman both installed, the model used either,** and a project built with one was started with the other.
- **Under a sandbox profile, a refusal looked like a broken tool.** A command the profile stopped failed with "Permission denied", and the model reported that Docker or a toolchain was missing.
- **A reviewer couldn't test a project that tests in containers.** The review hat's shell refused every `docker` and `podman` command, so on a Docker project it could not run the tests, and the model said there was no Docker on the machine.
- **`/models` crashed Ryter beside the rail.** With the side rail showing and a terminal narrower than about 158 columns, opening `/models` ended the program with "index outside of buffer". The panel asks for 124 columns and was sized to the whole screen, not to the space beside the rail, so it was drawn past the right edge. It has done this since 0.9.1. Hiding the rail (`^b`) or a wider terminal avoided it.
- **The `workspace` sandbox was unusable, and nothing said what the profiles meant.** `/settings` had a `profile` field with three names and no explanation. Under `workspace` on a real machine:
  - `git` could not start, because it opens `/dev/null` for writing and `/dev` was read-only;
  - `cargo`, `rustc` and anything else installed under the home folder were refused;
  - nothing could make a temporary file, since `/tmp` is shut and `TMPDIR` pointed nowhere;
  - no file could be moved from one folder to another, so `cargo` could not build a library ("Invalid cross-device link");
  - `docker compose build` stopped at "~/.docker/buildx/.lock: permission denied".
- **Your rules could only be read or changed by hand outside Ryter, or through the model.** There was no place in Ryter to see what your rules were, and `/rules` did nothing without something to remember.
- **An update could fail with "Text file busy".** Ryter checks a downloaded release by running it. If another part of Ryter started a command at that moment, the check was refused and a good update was not installed.

## What changed

- **Crew mode is removed. Ryter is one mode.** One model works in your project, with you, in the plan, build or review hat.
  - **Gone:** the lead, the architect, builders and auditors; the task queue, worktrees and patches; the crew board, the crew builder, the ready-made crews; `/crew`, `/crews`, `/solo`, `/agents` and `/auditor`; the per-task cap and the auditor's settings; `ryter crew …`; and `ryter --hat crew`.
  - **Typing one of those commands** says it was part of crew mode and points at the hats and `/models`.
  - **Your configuration still loads.** `[subagents]`, `[auditor]`, crew rows under `[specialists]`, `crew.toml` and the old settings are ignored, with no error. The model your lead ran on is the model every hat uses.
  - **Your sessions still open.** A session saved in crew mode keeps its conversation and its spend, and carries on in the build hat.
  - **What crew mode cost is still counted.** A project's total includes it, and `/spend` and `ryter spend --project` name it as "crew mode (removed)".
  - **`ryter serve` and `ryter mcp serve`** now work on a message in the build hat, in your project, where they used to hand it to the lead. Edits inside the project are allowed without asking, as nobody is there to ask; outside it they are refused.
  - **`ryter bench` is removed** with its published results. The ten tasks stay in `bench/` for a benchmark that runs the hats, which is not built yet.
  - **Hooks:** the `Handoff` event is gone, and `SessionStart` sends `hat` where it sent `phase`.
  - **For large work,** the model is told to present a plan that splits it into steps and build one step at a time.
- **Each hat can have its own model.** `/models` lists *All hats*, *Plan*, *Build* and *Review*. A hat follows *All hats* until you give it a model.
  - The rail and the status line show the model your next message goes to.
  - The hats share one conversation. When a model that hasn't read it takes over, the chat says what re-reading it costs: "review hat · grok-4.7 re-reads 42k tokens, about $0.13".
  - In the chat, each model is named as it takes over, so a reply reads as the model that wrote it.
- **A plan is read and approved in its own panel.** The model shows its plan (goal, steps, files, risks, how to verify) in a scrolling panel:
  - `y` approves: the plan is saved as `.ryter/plans/<date>-<title>.md` in the project, and the model builds from it in the build hat, in the same turn.
  - `e` adjusts: you type what to change, and the model shows the revised plan.
  - `n` rejects: nothing is saved.
- **The review hat reads the project, `/tmp` and your home folder, and nothing else.** A read-only command that points anywhere else is refused, and credentials are refused wherever they are. A command inside one of the project's containers is still judged by the container's own paths.
- **Less asking in the build hat.** These now run without a prompt:
  - your toolchains, whatever the subcommand (`cargo`, `npm`, `pnpm`, `python`, `pip`, `uv`, `go`, `make`, and their kind);
  - the project's own programs (`./scripts/setup.sh`, `bin/cms-admin`, `./manage.py`), and programs you installed under your home folder;
  - the project's containers, with `docker` or `podman`: build, `compose up`, `down`, `run`, `exec`, `restart`, `logs`, and `docker run` with the project's folders mounted;
  - `cd` into a folder of the project.

  These still ask: edits; system programs that change files by hand (`mkdir`, `cp`, `sed -i`); deleting and moving; publishing and signing in (`cargo publish`, `npm login`, `docker push`); tools for a service somewhere else (`gh`, `aws`, `kubectl`, `curl`); and in Docker, removing volumes, stopping or removing a container by name, another machine, and giving a container the host (`--privileged`, a mount of `/` or the Docker socket).
- **`/tmp` and your home folder are open to every hat,** to read and write, without a prompt. Still refused, to read or write: credentials, your tools' saved logins (`~/.npmrc`, `~/.pypirc`, `~/.cargo/credentials.toml`, `~/.git-credentials`), your shell history, and your browser's and mail client's folders. Never written: shell startup files. Still asking each time: writing in another git repository under your home folder (reading one is open), and deleting or moving anything outside the project. Anywhere else outside the project is as it was. The plan and review hats still change nothing in the project itself.
- **Docker is preferred when it is installed.** The model is told which container tool the machine has, and to use Docker when it has both, unless you ask for Podman.
- **`/tmp` is open under the sandbox profiles.** `workspace` and `read-only` shut it and pointed `TMPDIR` at a folder of Ryter's own, which served tools that ask where temporary files go and failed every script that names `/tmp`. Both profiles now let commands read and write `/tmp` and `/var/tmp`, and `TMPDIR` is left alone. Your home folder stays shut under a profile (beyond your tools and their caches): to have it open to commands, the profile has to be `off`.
- **Under a sandbox profile the model is told what the profile shuts,** so "Permission denied" is reported as the profile and not as a missing tool.
- **A temporary file is made without a prompt.** `mktemp` asked in the build hat and was refused in the others; `echo $f` asked because a variable was read as a path.
- **A fourth hat: Test.** It uses the product as its user would: starts it, runs its tests, tries it, and says what works and what doesn't.
  - **It is on `Tab`** after review, and has its own seat in `/models`.
  - **`Tab` goes round in the order the work does:** plan, build, review, test, then plan again. It went build, plan, review. A session still opens in build, so from there `Tab` is review and `Shift+Tab` is plan.
  - **It has a conversation of its own.** The tester doesn't read what you and the builder said: it judges the product from the plan, the decisions and from using it. In the test hat the chat shows the tester's thread; in any other hat it shows the conversation they share. The thread continues through the session and comes back when the session is resumed.
  - **You can look at either while the other works.** What a running turn says goes into the conversation it is part of, whichever is on screen.
  - **What it may run:** everything the build hat runs without asking, and requests to the project's own address (`curl localhost:8000/…`). It can't edit, delete or move the project's files.
  - **How the project runs is its own file,** `.ryter/run.toml`: the start command, an address that answers once it is up, the test commands, and the stop command. The tester proposes it and you approve it on a panel (`y` approve, `e` adjust, `n` reject). Ryter then runs those commands itself, and keeps a start command that stays in the foreground (`npm run dev`) running.
  - **A run file is approved by what it says.** One that came with the project, or was changed after you approved it, is shown to you again before anything in it runs.
  - **The product is left running** after a test, and the chat says where. `/stop` stops it, quitting asks whether to stop it, and the next session in that project is told it is still up.
  - **Its report comes back into the main conversation** as a card: a pass is one line, a failure is written out with what was expected, what happened and how to see it again. The model you build with is given the same report, so "fix 3" works. The tester's working stays in its own thread.
  - **The full report is a file** under `.ryter/tests/`, named for the day and the plan. An earlier report is never written over.
  - **A test is offered after a review that passed,** with what it should cost; `/test` asks for one at any time; `s` on the offer stops the offers.
  - **A failed test offers its fixes** in the build hat. The fixes are new work, so a review of them is offered, and a test after that.
  - **The commit receipt says whether the work was tested:** "test ✓ kimi-k3", "test ✗ kimi-k3", "not tested", or "not tested after the last change".
- **Ryter's own files are not the work.** Approved plans, the decisions file, the run file and test reports are kept in the project. A review no longer counts them as changed files, and writing one no longer makes a review read "not reviewed after the last change".
- **A plan's file is named for your date.** It was named for the date in UTC, so a plan approved in the evening was filed under tomorrow's.
- **A review's cost is shown to the tenth of a cent** ("$0.003"), as a turn's is. One that cost less than half a cent read "$0.00".
- **Where the work differs from the plan, the difference and its reason are recorded.** An approved plan is not edited. The entries go in `.ryter/decisions.md` in the project, under the plan they belong to: what the plan said, what is built instead, why, and who decided.
  - An entry is added when you tell the model to leave out, add or change something the plan says, and when the model finds a step can't be done as written and takes another way to the same goal.
  - The chat says "decision recorded: No export button in this pass". Nothing is asked.
  - A review is pointed at the plan's entries: a difference recorded there is not reported as a defect. One with no entry still is.
  - The file is yours to edit. A decision needs a plan approved in the same session, and a reviewer can't record one.
- **One reviewer: the review hat.** `/audit`, and the offer after a build turn that changed files, run a review in the review hat, on the model you gave it in `/models`.
  - **It asks first,** as before: the model, what it reviews, and a cost range. If the review hat has no model of its own, the prompt says the reviewer is the model that built the work.
  - **It is a turn in the conversation.** The reviewer has read what you asked for, and the model you build with reads its findings next. The hat you were in comes back when it ends.
  - **It checks the change against the plan you approved,** then for bugs, tests and safety, and ends with `VERDICT: PASS` or `VERDICT: FAIL`.
  - **A failed review offers its fixes** in the build hat, and a review of the fixes is offered after.
  - **The commit receipt says whether the work was reviewed:** "review ✓ grok-4.7", "review ✗ grok-4.7", "not reviewed", or "not reviewed after the last change". A verdict holds only for the files the reviewer read.
  - **The limit is a setting:** `/settings` → *review usd* (0 is no limit). It holds any turn in the review hat.
  - **Your 0.10.0 choice carries over.** The model you chose for `/audit` becomes the review hat's model, and its limit becomes the review limit, until you change them in `/models` and `/settings`.
  - **Gone:** the reviewer chooser, `/audit model`, the audit's own card in the chat, and the rule that the reviewer must be a different model. A custom `second.md` prompt is no longer read; the review's instructions are in `solo.md`.
- **A reviewer tests in the project's containers.** The review hat may now:
  - run a test or lint command in one of the project's containers: `docker compose run --rm web pytest`, `docker compose exec web ruff check .`, `docker exec <container> …` (`podman` the same). The command inside answers to the same rules as outside, so `ruff format .`, `pip install` or a shell are still refused.
  - look at what is running: `docker compose ps`, `docker compose logs web`, `docker ps`, `docker images`.
  - It still may not build, start, stop or remove containers, use `docker run`, mount a folder, or point Docker at another machine. A refusal says what does run in containers, and that Docker is there.
- **No panel is drawn off the screen.** A panel is sized to the space it is drawn in, and whatever it asks for is cut to the screen. `/models` opens beside the rail at any width.
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
  - run `docker build` and `docker compose build`: Docker's build lock folder, `~/.docker/buildx`, is writable. The registry logins beside it stay shut.

  The rest of your home folder, `~/.ssh`, the tools' saved logins and Ryter's keys stay shut. Rootless Podman can't run under a profile, and `/settings` and the guide say so.
- **A key read from the environment is kept out of every command**, whatever its variable is called. Only `XAI_API_KEY` and `OPENROUTER_API_KEY` were, so a key under another name was handed to each command the model ran.
- **Under a sandbox, Ryter's keys can't be read from its own process.** `/proc` is readable in the sandbox, and a command could read a key from Ryter's environment or memory there. Ryter now closes its process to other processes when a profile is on.
- **The updater waits for a file that is still being written.** The check of a downloaded release waits up to two seconds for "Text file busy" to pass.

## What you lose

- **Unattended work.** Nothing builds without you now. Each change is one you asked for, in a hat you chose.
- **A check of a model before you rely on it.** `ryter crew check` and the crew builder sent each model a tiny request with a tool, to catch a data policy that refuses it, missing tool support, or no credits. Today a hat's model that your account can't use fails on its first message, with the provider's reason. Checking it when it is chosen is on the roadmap.
- **The benchmark.** There is no measured land rate or cost per task for this release.

## Tried before release

- **Without crew mode,** on a home folder written by 0.10.0 (crew rows, `[subagents]`, `[auditor]`, `crew.toml`, old settings, a `review.toml`) and a session saved in crew mode:
  - **From the command line:** `ryter -p` answered in the build hat and `--hat review` on the reviewer's model. `--hat crew` said "crew mode was removed" and exited 1, leaving no session behind. `ryter bench` and `ryter crew check` were unknown commands. `ryter sessions` listed the crew session as a build session, and `ryter spend --project` showed "crew mode (removed) $3.25" for what that session had cost.
  - **In the TUI:** `/crew` and `/auditor off` each said the command was part of crew mode. Resuming the crew session showed its conversation and the build hat, and the next message went out with the hat's note and all fifteen tools. `/models` showed *All hats*, *Plan*, *Build* and *Review*, with no guided setup. `/settings` had no agents section, and `/budget` no per-task cap.
  - **A bug this found:** a resumed crew session first sent its next message bare, with no tools, because it was still run as the lead. Fixed, with a test.
- **`ryter mcp serve`,** driven by hand over stdio against a stand-in provider: a `ryter_prompt` went out with the build hat's note and all fifteen tools, the model's `write` was run without a prompt, and the file was in the project. My pipe closed before the prompt's reply came back, so the reply itself wasn't seen.
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
- **A reviewer and containers:**
  - In the TUI, in the review hat, against a stand-in provider on a machine with Docker: `docker ps` ran and listed the containers; `docker compose run --rm web pytest -q` ran (Docker answered that the test folder has no compose file); `docker compose up -d --wait` and `docker run -v /:/host …` were refused, each with "Docker is here, and tests and linters do run in the project's containers".
  - Under the real `workspace` sandbox on this machine: `docker version`, `docker ps`, `docker compose version` and `docker buildx ls` all worked. Without the new grant `docker buildx ls` failed with "buildx/.lock: permission denied".
  - Rootless Podman under the sandbox: `podman ps` failed on its database, and a user namespace could not be set up at all (`unshare -Urm` failed where it works outside the sandbox).
- **The Test hat,** in the TUI against a stand-in provider, with the test hat on a second model:
  - After one message in the build hat, `Tab` three times: the rail showed TEST, "uses the product" and the tester's model, the chat was empty under "TEST THREAD · its own conversation · tab: main chat", and the prompt read "ask the tester".
  - "test the list": the request went to the tester's model with that one message and eleven tools. It ran the project's `./run-tests.sh` and a `curl` to `localhost` with no prompt. Its attempt to write a file in the project was refused: "the test hat can't edit files".
  - `Tab` back to build: the main chat as it was left, with nothing of the tester's. A message there went to the main model with the main conversation only.
  - `ryter resume`, with the session left in the test hat: it opened on the tester's thread, and the next message continued it (the request carried all nine messages of that thread).
- **The whole chain,** in the TUI against a stand-in provider, with the review and test hats each on a model of their own and a small real web server as the product:
  - A build turn changed a file, and a review was offered. `⏎`: the review passed, and a test was offered: "tester-k on local (the test hat's model) · starts the project and tests what changed, leaves it running · about $0.05–$0.49".
  - `⏎`: the screen went to the tester's thread. It proposed the run file (`y`), started the server, ran the tests, requested a page that wasn't there, and filed a report with one failure.
  - The main conversation then showed "switched to the test hat (tester-k)", the card "▣ test · tester-k · ✗ 1 of 3 failed · 4.3s · $0.006" with the failure written out and "full report .ryter/tests/2026-10-01-greeting-page.md", "the project is still running at …", and "switched to the build hat".
  - "fix what the test found?" `⏎`: the main model was sent the report and wrote the missing file, and a review of the fix was offered.
  - The tester's thread: "TEST THREAD · 1 run this session", its steps (`setup`, `start`, `test`, `run`, `report`), its turn closed with "✗ 1 of 3 failed", and the prompt read "ask the tester, or: retest 3".
  - `/commit` after the fix: the receipt ended "not reviewed after the last change · not tested after the last change".
  - `/test` on its own asked first, with no "stop offering" key.
  - Quit with the product up, stop it, `ryter resume`: the tester's thread was back, with Ryter's request to it as one line.
  - **Bugs this found:** the report named the model every hat uses, not the tester's own; the report's file was dated in UTC; and the run file and the report, being new files in the project, would have made a passed review read as out of date. All fixed, with tests.
  - Not tried: any of it with a real model, which is the next step.
- **The run file,** in the TUI against a stand-in provider, with a small real web server as the product:
  - The tester proposed a start command, a ready address and one test command. The panel showed them under "how this project runs". `e` and "also run ./lint.sh": the model proposed again with both. `y`: the chat said "run file · approved and saved to .ryter/run.toml", and the file held the four lines.
  - Start: the server came up in 0.3s and the chat said "the project is running at http://127.0.0.1:57341/, started 20:56 · /stop stops it". The tests ran ("✓ 2 passed"), and `curl` to the server returned 200 with no prompt.
  - `/stop`: "the project was stopped (ended the start command)", and the server's process was gone.
  - `^c` with it up: "stop the project?". `⏎` stopped it and Ryter exited. In another run `n` left it up and Ryter exited; the next session said the project was still running, and `/stop` there said it had no stop command, gave the process number, and left it alone.
  - **Bugs this found:** `curl -o /dev/null` was treated as writing outside the project and asked every time; and stopping a held start command waited five seconds for a process that had already gone. Both fixed, with tests.
  - Not tried: a start command through Docker, and a product that takes long to come up.
- **`/tmp` under the sandbox,** in a headless run with `--sandbox workspace` on this machine: `echo x > /tmp/…` and the `write` tool both wrote to `/tmp`; `cargo --version` ran; `echo y > ~/probe.txt` failed with "Permission denied", as the profile says it will; `cat ~/.ssh/known_hosts` was refused by the gate. The model's instructions said "`/tmp` is open. The rest of the user's folder is shut".
- **Less asking,** in a headless run in the build hat against a stand-in provider, with nobody to ask, so anything that needs a yes is refused:
  - Ran: `docker compose version`; `docker ps` piped to `wc`; a file written to scratch space with the `write` tool; `echo hi > ~/note.txt` (with the home folder pointed at a scratch one); a program in `~/.local/bin`; `cargo test > /tmp/out.txt`.
  - Held back: `echo x >> ~/.bashrc` (refused); `rm -rf ~/note.txt` and a write to `/opt` (outside the project, a yes each time); `docker compose down -v` and `npm publish` (need approval).
  - The request's instructions said "Docker and Podman are both installed. Use Docker".
  - In the TUI: `docker compose version`, `cd scripts && ./setup.sh` and `cargo --version > /tmp/out.txt` ran with no prompt. `docker compose down -v` stopped at a prompt that said "removes containers' data (volumes) · nothing undoes it", took only `y`, and offered no "allow for this session".
  - Not tried: a real `docker compose up` or build through Ryter (the machine's containers are the user's), and any of this with a real model.
- **Decisions,** in the TUI against a stand-in provider, with the review hat on a second model:
  - I approved a two-step plan and the model built step 1. I typed "skip the export button for now": the chat said "decision recorded: No export button in this pass", and `.ryter/decisions.md` held the entry under the plan's name, signed "you" with the local time.
  - A second one, made by the model, was signed "build hat (main-model)".
  - `/audit`: the request to the reviewer's model named the plan and said "is recorded in `.ryter/decisions.md`, under `## plan: 2026-10-01-page-list-with-export.md` (2 entries). Read them".
  - The stand-in provider called the tool because its script said to. Whether a real model records a decision when it should, without being told, has not been tried.
- **The plan panel,** in the TUI against a stand-in provider that presented a five-section plan:
  - The panel showed the plan under its title, each heading with its section under it, all on screen without scrolling.
  - `e`, then "Stream the rows; skip the button for now": the model was told those words and showed a revised plan. Nothing had been saved.
  - `y`: the chat said "plan · approved and saved to .ryter/plans/2026-10-01-add-csv-export-to-reports.md" and "switched to the build hat", the file held the plan under its title, and the model went on in the same turn.
  - `n` on a second plan: nothing more was saved, and the hat stayed on plan.
- **`/models` beside the rail,** in the TUI at 110 columns, where 0.10.0 crashes: it opened, with every model's name and prices in view. A new test opens every panel at every width from 40 to 200 columns, with and without the rail; it crashes on 0.10.0.
- **The `/rules` panel,** in the TUI, with its own home folder and no model:
  - It opened on the every-project rules: both tabs, the file, "4 rules", and a long rule wrapped under its own text.
  - `a`, a typed rule and Enter added it under the selected line, and the file held it. `d` asked "remove …?", and `y` removed it.
  - Tab showed the project's file as "not created yet". The first rule added there created `RYTER.md`.
  - `e` handed the file to the editor, and the panel showed the editor's change when reopened. It first failed here with `VISUAL` set to nothing, which is fixed.
- **The `workspace` sandbox, on a real machine:**
  - **By hand,** inside the profile: `git`, `cargo`, `rustc`, `node`, `npm` and `python3` ran; `git commit` worked; a Rust program and a Cargo project built; `mktemp` made its file in `~/.ryter/tmp`. `~/.ssh`, the home folder and `/tmp` were refused. `docker ps` still answered, as the table says it will.
  - **Real builds,** before crew mode was removed: a crew built all ten benchmark tasks inside the profile (Python, Rust, TypeScript, and a project run by its own script). That run is how the missing right to move files between folders was found and fixed.
- **The sandbox table,** in the TUI: under the `profile` field at 110 and 80 columns, with the chosen profile in capitals as the choice moved.
- **The updater's wait,** with a test that holds a program open for writing and lets go 150 ms later: the check waits and passes. The updater tests, which failed about one run in four before, passed six runs in a row.
- **Not yet tried:**
  - Any of this with a real model on a real project. Everything above used a stand-in provider. The next step is the Docker project crew mode failed on, taken from a plan to a reviewed change with the hats.
  - `ryter serve` over a socket or TCP. Only `ryter mcp serve` on stdio was tried.
