# Provider proof

Listing models proves a key works. It does not prove a tool call survives the trip. This page is the record of what each built-in connection has been seen to do, and how to see it again.

There are two halves. The **replay** runs offline in every `cargo test`: what a provider really sent, played back over a local socket through the real HTTP client and the real agent loop. The **live run** spends real money by hand, makes those recordings, and adds a dated row below.

## What a run proves

On each connection, in the build hat, with the real system prompt and the real tool list:

| Case | What is checked |
|---|---|
| Tool round trip | The model calls `read_file`, the call runs, the result goes back under its id, with any thinking signature unchanged (on Anthropic in the thinking block, on OpenRouter in `reasoning_details`), and the answer says what the file said |
| Parallel calls | Two calls in one reply, each answered under its own id |
| Truncation | A reply cut at the output limit says so and is not read as finished |
| Cancellation | Esc mid-reply stops the turn; nothing half-made runs or reaches the conversation |
| Accounting | Each spend row matches the bill as the wire states it, cached tokens included, and is priced from the provider's own figure or the price book |
| Dropped stream | The same reply cut off, cleanly and abruptly: the turn fails and says so, and nothing half-made runs |

The live run and the replay share one set of checks (`crates/ryter-core/src/agent/tests/wire.rs`), so a kept capture cannot drift from what was proven. Whether Anthropic's model thinks before it calls is its own choice, so the run says whether the capture has reasoning to echo, and a kept test fails on a Claude capture without a signature. Local models are not part of this; they are not built in.

## Running it

It uses the keys Ryter already holds, or the usual variables (`XAI_API_KEY`, `OPENROUTER_API_KEY`, `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`). A connection with no key is skipped and says so.

```sh
RYTER_LIVE=1 cargo test -p ryter-core --lib live_tool_round_trip -- --ignored --nocapture
```

| Variable | Meaning | Default |
|---|---|---|
| `RYTER_LIVE_BUDGET` | Dollars per connection. As with `/budget`, it stops the next call: the last one can pass it by its own cost | `0.50` |
| `RYTER_LIVE_MODELS` | `connection=model,connection=model` | each connection's default model |
| `RYTER_LIVE_ONLY` | `connection,connection` | every connection |

The model must have a price, in the book or from the provider, or the budget cannot see it and the run stops.

A run that holds writes `crates/ryter-core/fixtures/wire/<connection>/` and adds a row here. Add a newly recorded connection to `RECORDED` in `wire.rs`; a test fails until the list and the folder agree. A model that goes another way (a look around first, one call where two were asked) gets one more try; it prints what the model did, and that try's replies stay in `target/wire/<connection>/failed/`, so the cause can be read without paying again. A run that fails keeps nothing in `fixtures/`. What went through the recorder and failed its replay stays in `target/wire/<connection>/`.

A capture is kept only once it is clean: ids and signatures are renamed in order of appearance, the copy of the prompt a Responses server sends back is dropped, and the recorder refuses a capture holding the key, a home or temp path, or anything shaped like a key. The tap that records is compiled into test builds only, so no release can write a reply to disk.

Run it once per provider before each release.

## The trail

Newest last. Tokens are the run's whole: prompt, the part of it read from cache, and output.

| Date | Ryter | Commit | Connection | Model | Served by | Replies | Prompt | Cached | Output | Cost |
|---|---|---|---|---|---|---|---|---|---|---|
| 2026-10-30 | 0.42.0 | 78677c0 | anthropic | claude-opus-5-5 | - | 5 | 43623 | 27778 | 201 | $0.0888 |
| 2026-10-30 | 0.42.0 | 17d66c6 | anthropic | claude-opus-5-5 | - | 5 | 43713 | 27807 | 233 | $0.0897 |
| 2026-10-30 | 0.42.0 | 51090eb | openrouter | anthropic/claude-sonnet-4.6 | Claude Platform on AWS | 5 | 35459 | 17202 | 320 | $0.0779 |
| 2026-10-30 | 0.42.0 | 7650e47 | openrouter | deepseek/deepseek-v4.1-flash | Together | 5 | 32943 | 19840 | 260 | $0.0044 |
| 2026-10-30 | 0.42.0 | 7650e47 | openrouter | anthropic/claude-sonnet-4.6 | Claude Platform on AWS | 5 | 35527 | 17202 | 284 | $0.0776 |
