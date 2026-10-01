---
name: rules
description: Save, change or remove a standing rule of the user's. Use it when the user says how they always want something done ("from now on", "always", "never", "stop doing that"), asks you to remember a preference, or asks to see or edit their rules.
user-invocable: true
---

# Rules: what the user wants every time

The user keeps standing rules in two files. Ryter puts both in your instructions on every message, so a rule written there holds in later sessions.

- **Every project:** `RYTER.md` in Ryter's home folder. That is `~/.ryter/RYTER.md` unless `RYTER_HOME` puts the folder elsewhere, and your instructions name the file where it really is, under "The user's rules". You change it with `update_rules`. Your file tools can't reach it.
- **This project only:** `RYTER.md` at the top of the project, shown under "Project instructions". Edit it with your ordinary file tools.

Where the two differ, the project's wins.

## When to save a rule

- The user states a preference that should outlast this conversation: "from now on", "always", "never", "I prefer", "stop doing X".
- The user corrects the same thing a second time.
- The user asks you to remember something about how they work.

Don't save what only matters to this task, or what the project's files already say. If you're not sure the user means it as a standing rule, ask them before saving.

## Which file

- A rule about the user themselves, or about how they like any work done, goes in the every-project file: tone, spelling, how to report, what to ask before doing.
- A rule about this codebase goes in the project's `RYTER.md`: its commands, its conventions, what not to touch.
- If it could be either, ask the user which.

## How to write one

- One rule per bullet, in the user's own words where you can. Say what to do, not only what to avoid.
- Be specific enough to act on. "Run `cargo test` before saying a change is done" is a rule; "be careful" isn't.
- Add the reason when the user gave one, after the rule: "…, because the CI run takes twenty minutes". A reason lets a later session judge a case the rule doesn't quite cover.
- Group rules under short headings once there are more than a handful.
- Keep the file short. It is read on every message. Fold a new rule into one that already covers the same ground, and don't keep two rules that disagree: change the old one.

## Changing the every-project rules

1. Start from the rules as your instructions show them. If there are none yet, start a new file.
2. Make the one change: add, reword or remove a rule. Leave every other rule exactly as it is, word for word.
3. Call `update_rules` with the whole file as it should read afterwards.
4. Ryter shows the user what would change and asks them. They may say no. If they do, the file is unchanged: ask what they'd like instead, and don't try again with the same text.

To show the user their rules, quote them from your instructions. Don't call `update_rules` to look.

## After

Say in one line what you saved and in which file. Follow the new rule from now on, in this conversation too.
