# Next

Unreleased: what is in `dev` since 0.24.0. These notes become the next release's `vX.Y.Z.md`.

## What changed

- **A model chosen for "All hats" is every hat's.** In `/models`, choosing a model on the *All hats* seat now puts every hat back to following it: a hat that had its own model drops it, and the chat says which did ("model · grok-4.7 · all hats follow it (plan and audit had models of their own)"). On that seat the key reads `enter set for all hats`. `/model <id>` does the same.

## Why

With each hat on its own model, a new choice for *All hats* changed only a setting nothing was using: every hat kept its model until each was put back to `default` by hand. The user's words: "If an all hats model is selected should that not automatically change all the hats to default so they use that?"

## Good to know

- Set *All hats* first, then the hats that should differ: the cursor already moves in that order.
- A hat's own model is dropped without a question. The line in the chat names what was dropped.

## Tried before release

- **Driven in the real binary,** offline against the simulated provider: `/models` with Plan and Audit on models of their own, a model chosen for *All hats*, every seat then reading "follows all hats", the line in the chat, and `~/.ryter/hats.toml` left empty.
