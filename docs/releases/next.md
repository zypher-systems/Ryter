# Next

Unreleased: what is in `dev` since 0.20.0. These notes become the next release's `vX.Y.Z.md`.

## What changed

- **The plan and audit hats can ask after a dependency.** A new tool, `check_package`, gives a dependency's latest release and its known security advisories in one call: the version and its release date from the package's registry (crates.io, npm, PyPI, the Go module proxy), and the advisories on record for the version in use from OSV.dev, each with its aliases, its severity where the record gives one, a one-line summary and the versions that fix it. Fixed public hosts, read-only, no key, on by default, and no `[features] web` needed. The plan hat is told to ask before it pins a version; the audit hat to ask what is known against the dependencies. The build and scribe hats are refused it and told who asks.

- **`web_search` looks where you tell it to.** `[search] provider = "tavily"`, with a key stored like a connection's (`ryter connections set-key tavily`, `/provider set-key tavily`, or `TAVILY_API_KEY`), or `provider = "searxng"` with the `url` of a server of your own. Each result comes back as its title, address, date where known and a snippet; `max_results` is yours (1 to 10, 5 by default). The DuckDuckGo page scrape is gone; without a `[search]` section the tool says what to set. `[features] web` is now on by default, and `web_search` and `web_fetch` belong to the plan and audit hats, as `check_package` does; the build and scribe hats are refused them and told who uses them, and a refusal with the feature off says where it is turned on.

## Why

Building anything means knowing what is current and what is broken, and the web search behind `[features] web` was a scrape of a search page that nobody had turned on. The registries and OSV.dev answer the actual question exactly, for nothing, so that is the first thing Ryter learns to look up. Where it sits is the user's placing (2026-10-05): plan, yes; audit, yes; build and scribe, no. The search provider is the user's choice too: Tavily for its free tier, and SearXNG beside it so that nobody is made to take a key.
