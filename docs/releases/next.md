# Next

Unreleased: what is in `dev` since 0.21.0. These notes become the next release's `vX.Y.Z.md`.

## What changed

- **The search provider has a row in `/provider`.** Under the connections, a **web search** row shows the provider in use and whether its key is in place. Enter on it chooses Tavily, SearXNG or off; Tavily goes straight to the key prompt, SearXNG asks for your server's address (`http://localhost:8080` offered); `k` on the row re-enters the key. The choice is saved to `~/.ryter/settings.toml` as `search_provider` and `search_url`, which win over `[search]` in `config.toml`, and the running session searches with it at once. The typed `/provider set-key tavily` still works.

## Why

The panel showed only model connections, so the one place a person would look for the search key had nothing to offer, and the guide's answer was a command to remember. The user's words: "we need to add that as an option under the menu and not expect users to remember the long form command."
