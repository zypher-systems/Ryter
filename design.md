# Ryter TUI — Design Contract

Status: **proposed / not implemented**
Target release: **0.2.0** (feature add — new UI surface, no product rename)
Work branch: `0.2.0-patch` → `dev` → `main`
Applies to: `crates/ryter-tui` (with small, listed additions to `crates/ryter-core`)

This document is a **contract**. Every requirement is numbered (`R-<AREA>-<NN>`). An
implementation is done when every requirement in its section is satisfied and the tests
in §15 pass. Where this document conflicts with current behavior, **this document wins**
and the current behavior is to be replaced.

---

## 1. Why this redesign

The TUI today works but reads like a log file with a status column bolted on. Concretely:

| Problem | Where it lives today |
| --- | --- |
| No scrollback — older turns are silently destroyed when you send a new message | `chat.rs:23-48`, `take_tail` |
| No syntax highlighting; code fences render flat in `theme.tool` | `chat.rs`, no `syntect` in the tree |
| Markdown support is a hand-rolled subset with no nested lists, blockquotes, links, or rules | `chat.rs` `markdown_lines` |
| Speakers are implied by color and alignment only — no username, no model name | `chat.rs` `render_user` / `render_assistant` |
| `AgentEvent::Reasoning` is received and thrown away, so the UI is silent while the model thinks | `run.rs:1702` |
| Single-line composer, no newline key, no visible shortcuts | `run.rs:933-997`, `draw.rs` `draw_composer` |
| Sidebar is a flat `key`/`value` dump with no grouping, no gauges, no spend context | `draw.rs:158-251` |
| Slash menu is a flat alphabetical-ish list of ~30 bare names with no descriptions, no grouping, no fuzzy match | `view.rs:557-592`, `draw.rs:1234-1260` |
| Config commands land in inconsistent places — some open overlays, some dump text into chat (`/doctor`, `/context`, `/help`) | `commands.rs:121-285` |
| Wrapping counts `char`s, so CJK and emoji misalign | `chat.rs` `wrap_words` |

The redesign keeps the architecture (ratatui 0.29 + crossterm 0.28, worker thread +
`AgentEvent` channel, `View` as the single drawable state) and replaces the presentation
layer.

---

## 2. Design principles

1. **The conversation is the product.** The chat pane gets the space, the typography
   budget, and the color budget. Everything else is support.
2. **Never lose the user's place.** The composer is always on screen. The user's message
   for the in-flight turn is always on screen. Scrolling up never fights you.
3. **Say what is happening.** The model thinking, a tool running, a specialist working —
   each has a visible, honest indicator. Silence is a bug.
4. **One way to configure a thing.** Every configurable surface is a popout panel reached
   from the command palette. Nothing important is a chat-line side effect.
5. **Structure over decoration.** Borders and color carry meaning (ownership, grouping,
   state). They are not garnish.
6. **Degrade, never break.** 16-color terminals, 80×24, no mouse, no truecolor — all
   supported, with a documented reduction.

### 2.1 Chrome policy change (supersedes a RYTER.md non-negotiable)

`RYTER.md` currently states: *"TUI: quiet chrome, no outer boxes. Snapshots at 80×24 and
120×40 must stay free of `┌┐└┘`."*

- **R-CHROME-01** This rule is **retired**. Box-drawing characters are permitted anywhere
  they improve grouping or legibility.
- **R-CHROME-02** `RYTER.md` must be edited to replace the rule with: *"TUI chrome is
  structural — borders group and separate, they never decorate. Panels are bordered;
  the base layout uses hairline rules."*
- **R-CHROME-03** The four `assert!(!s.contains('┌'))`-style assertions in
  `crates/ryter-tui/src/lib.rs:71-74` and the `'╔'` assertion at `lib.rs:86` must be
  removed, not weakened.
- **R-CHROME-04** A `DECISIONS.md` entry must record *why* the rule was retired
  (config surfaces grew past what borderless offset can disambiguate; floating panels
  stacked over a scrolling transcript need a hard edge to read as modal).

---

## 3. Global layout

### 3.1 Vertical stack

Replaces the fixed 8-row stack at `draw.rs:35-47`.

| Row | Constraint | Region | Notes |
| --- | --- | --- | --- |
| 0 | `Length(1)` | Header | identity, session, cwd/branch |
| 1 | `Length(1)` | Hairline | |
| 2 | `Min(8)` | Body | chat column ∥ gutter ∥ info panel |
| 3 | `Length(activity_h)` | Activity strip | `0` idle-with-no-history, `1` collapsed, `3..=12` expanded |
| 4 | `Length(1)` | Hairline | |
| 5 | `Length(composer_h)` | Composer | `3..=10`, auto-grows with content |
| 6 | `Length(1)` | Hint bar | context-sensitive shortcuts |

- **R-LAYOUT-01** The command palette is **no longer an inline row** in this stack. It is a
  floating panel (§7.4) anchored above the composer. Opening it must not resize the chat.
- **R-LAYOUT-02** The composer and hint bar are the last two constraints and are never
  `0`. No overlay, panel, menu, or stream may reduce them. (Satisfies "the user prompt
  never leaves the screen.")
- **R-LAYOUT-03** `composer_h = clamp(wrapped_line_count, 1, 8) + 2`. When content exceeds
  8 wrapped rows the composer scrolls internally and shows a `⋮` marker in its right
  border.
- **R-LAYOUT-04** When `composer_h` grows, the body (`Min(8)`) shrinks. If the body would
  drop below 8 rows, the composer stops growing and scrolls internally instead.

### 3.2 Horizontal split of the body

| Column | Constraint | Content |
| --- | --- | --- |
| Left | `Min(40)` | Chat transcript |
| Middle | `Length(1)` | Scroll gutter (scrollbar track) |
| Right | `Length(panel_w)` | Info panel |

- **R-LAYOUT-05** `panel_w` by total terminal width: `< 80` → `0` (hidden);
  `80..=99` → `26`; `100..=139` → `30`; `>= 140` → `34`.
- **R-LAYOUT-06** The info panel is togglable with `Ctrl+B` regardless of width. Toggling
  is session state, not persisted.
- **R-LAYOUT-07** When the panel is hidden, its most critical facts (model, context %,
  spend) move to the right side of the header.
- **R-LAYOUT-08** The gutter column renders a scrollbar track (§5.6) whose thumb reflects
  chat scroll position. It is blank when all content fits.

### 3.3 Wireframe — 120×40, streaming, panel visible

```
 ryter  ·  orchestrator            add a --json flag to the CLI            ~/workspace/ryter (0.2.0-patch)
────────────────────────────────────────────────────────────────────────────────────────────────────────────
 Dusty                                                        19:42  ┃ ╭─ session ──────────────────────╮
 Add a `--json` flag to the CLI and make `ryter sessions`            ┃ │ add a --json flag              │
 emit machine-readable output.                                       ┃ │ s_8f21c4         phase  build  │
                                                                     ┃ ╰────────────────────────────────╯
 grok-4.6                                              19:42  $0.012 ┃ ╭─ model ────────────────────────╮
 I'll put it on the CLI and thread it through the session lister.    ┃ │ spacexai ●        grok-4.6     │
                                                                     ┃ │ context ████████░░░░░░  38%    │
 ```rust                                                             ┃ │        192k / 500k tokens      │
 #[derive(Parser)]                                                   ┃ │ $2.00/M in   ·   $6.00/M out   │
 struct Args {                                                       ┃ ╰────────────────────────────────╯
     /// Emit machine-readable JSON.                                 ┃ ╭─ spend ────────────────────────╮
     #[arg(long)]                                                    ┃ │ session            $0.42       │
     json: bool,                                                     ┃ │ budget  ██░░░░░░░░░░  8%       │
 }                                                                   ┃ │        $0.42 of $5.00          │
 ```                                                                 ┃ │ orchestrator       $0.30       │
                                                                     ┃ │ builder            $0.12       │
 · read  Cargo.toml                                        0.2s      ┃ ╰────────────────────────────────╯
 · edit  crates/ryter-cli/src/main.rs                      0.4s      ┃ ╭─ tasks ─────────────────  2/4 ─╮
                                                                     ┃ │ ✓ add --json flag              │
 builder  cli flags                                           19:43  ┃ │ ✓ thread through lister        │
 Flag added; `sessions` now takes `--json` and prints one            ┃ │ ◐ update docs/guide.md         │
 object per line.                                                    ┃ │ ○ add CLI smoke test           │
                                                                     ┃ ╰────────────────────────────────╯
 grok-4.6                                                     19:43  ┃ ╭─ crew ──────────────────  1/4 ─╮
 Docs next. Updating `docs/guide.md` with the new flag and an        ┃ │ builder   cli flags            │
 example of the line-delimited output shape.█                        ┃ │           running  0:34  $0.12 │
                                                                     ┃ ╰────────────────────────────────╯
────────────────────────────────────────────────────────────────────────────────────────────────────────────
 ⠙ writing  ·  edit docs/guide.md  ·  0:34  ·  1.2k tok                                    ^r  reasoning  ▾
────────────────────────────────────────────────────────────────────────────────────────────────────────────
╭─ you ─────────────────────────────────────────────────────────────────────────────────────────── build ─╮
│ ›                                                                                                       │
╰─────────────────────────────────────────────────────────────────────────────────────────────────────────╯
 enter send    ⇧enter newline    / commands    ^r reasoning    ^b panel    esc cancel    ^c quit
```

### 3.4 Wireframe — 80×24, panel visible, sticky turn header engaged

```
 ryter · orchestrator        add a --json flag          ~/workspace/ryter
──────────────────────────────────────────────────────────────────────────
 Dusty  Add a --json flag to the CLI and make…              ↑ 42   ┃ model
 ‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥‥ ▓ spacexai ●
 grok-4.6                                    19:42  $0.012  ░ grok-4.6
 I'll put it on the CLI and thread it through the          ░ ███░░ 38%
 session lister.                                            ░
                                                            ░ spend
 ```rust                                                    ░ $0.42
 #[arg(long)]                                               ░ ██░░░ 8%
 json: bool,                                                ░
 ```                                                        ░ tasks
                                                            ░ ✓ --json
 · edit  crates/ryter-cli/src/main.rs           0.4s        ░ ◐ docs
──────────────────────────────────────────────────────────────────────────
 ⠙ writing · edit docs/guide.md · 0:34                        ^r  ▾
──────────────────────────────────────────────────────────────────────────
╭─ you ─────────────────────────────────────────────────────── build ─╮
│ ›                                                                   │
╰─────────────────────────────────────────────────────────────────────╯
 enter send   ⇧enter nl   / cmds   ^r think   esc cancel
```

The second row is the **sticky turn header** (§5.4): the in-flight user message collapsed
to one line, held at the top of the chat viewport while its response scrolls beneath it.

---

## 4. Header

- **R-HEAD-01** Left: `ryter` in dim, `·`, then the active speaker context in bold —
  `orchestrator`, or `orchestrator → builder` during a handoff.
- **R-HEAD-02** Center: the session title (from `View::session_id` / `Session` event
  title), truncated with `…`, dimmed. Omit when the terminal is under 100 columns.
- **R-HEAD-03** Right: `cwd` and, when present, `(branch)`. Branch is colored by git
  cleanliness if that is cheap to know; otherwise dim.
- **R-HEAD-04** The busy `…` indicator at `draw.rs:93` is **removed**. Busy state is owned
  by the activity strip (§6) and the composer border (§7).
- **R-HEAD-05** When the info panel is hidden (`R-LAYOUT-07`), append
  `model · ctx% · $spend` to the right group before `cwd`.
- **R-HEAD-06** A budget warning (`spend >= warn_usd`) turns the spend figure `warn`; a
  budget stop (`spend >= budget_usd`, non-zero) turns it `error` and prefixes `!`.

---

## 5. Chat transcript

This section replaces `crates/ryter-tui/src/chat.rs` wholesale. The new module set is
`chat/mod.rs`, `chat/markdown.rs`, `chat/highlight.rs`, `chat/wrap.rs`, `chat/cache.rs`.

### 5.1 Message model

- **R-CHAT-01** `LogLine` (`view.rs:44-59`) is replaced by a richer `Message`:

  ```rust
  pub struct Message {
      pub id: u64,              // monotonic, stable across re-render
      pub turn: u64,            // groups a user message with everything it caused
      pub kind: MessageKind,
      pub body: String,         // raw markdown / text, mutated during streaming
      pub at: OffsetTimestamp,  // wall-clock for the header
      pub meta: MessageMeta,    // cost, duration, tool status, specialist role
      pub rev: u64,             // bumped on every mutation; render-cache key
  }

  pub enum MessageKind {
      User,
      Assistant { model: String },
      Specialist { role: String, model: String },
      Tool { name: String, status: ToolStatus },
      Merge,
      System { level: SystemLevel },   // Info | Warn | Error
  }
  ```

- **R-CHAT-02** `View.lines: Vec<LogLine>` becomes `View.messages: Vec<Message>`, still
  oldest-first, still append-only within a session.
- **R-CHAT-03** Streaming appends to `body` of the last `Assistant` message and bumps
  `rev`. It must not allocate a new message per token.
- **R-CHAT-04** Messages are never dropped from `View.messages` during a session. Trimming
  happens only at `/new` or `/resume`.

### 5.2 Speaker attribution

Every message opens with a **speaker header row**, replacing today's right-aligned user
bubbles and unlabeled assistant text.

```
 Dusty                                                            19:42
 grok-4.6                                                 19:42  $0.012
 builder  cli flags                                               19:43
 · read  Cargo.toml                                                0.2s
 ! tool error  bash exited 1                                      19:44
```

- **R-CHAT-05** The header row is: leading glyph (kind-specific), speaker name in the
  kind's accent color and bold, optional secondary label in dim, then right-aligned
  metadata in dim.
- **R-CHAT-06** Speaker names by kind:

  | Kind | Name shown | Accent |
  | --- | --- | --- |
  | User | resolved username (§5.3) | `theme.user` |
  | Assistant | `short_model(model)` (e.g. `grok-4.6`) | `theme.assistant` |
  | Specialist | `<role>` + dim task label | `theme.phase(role)` |
  | Tool | tool name after a `·` glyph | `theme.tool` |
  | Merge | `merge` | `theme.build` |
  | System | `system` / `warning` / `error` | `theme.dim` / `warn` / `error` |

- **R-CHAT-07** Right-aligned metadata: `HH:MM` for user; `HH:MM  $cost` for assistant and
  specialist (omit cost when unknown — **never print `$0.00` for unknown**, per the
  existing `$?.??` rule); elapsed duration for tools.
- **R-CHAT-08** Consecutive messages from the same speaker within the same turn collapse:
  the second and later omit the header and continue under the first.
- **R-CHAT-09** Message bodies are **left-aligned for every kind**. Right-aligned user
  bubbles (`chat.rs:92-110`) are removed — they fight markdown, code blocks, and tables.
- **R-CHAT-10** Bodies are indented one column from the header for a quiet visual gutter.
  User message bodies additionally get a `▎` left rule in `theme.user` so a user turn is
  scannable at a glance without color alone.

### 5.3 Username resolution

- **R-CHAT-11** Resolve in order: `[ui] username` in `~/.ryter/config.toml` → `git config
  user.name` → `$USER` → literal `you`.
- **R-CHAT-12** Resolution happens once at startup and is cached on `View.username`. A
  failing `git` call must not block startup or emit an error.
- **R-CHAT-13** The name is truncated to 20 columns in the header.

### 5.4 Viewport, anchoring, and scrollback

This is the heart of the "the user prompt never leaves the screen" requirement, and it has
two distinct guarantees.

**Guarantee A — the composer never moves.** Covered by `R-LAYOUT-02`.

**Guarantee B — the in-flight user message stays visible.**

- **R-SCROLL-01** `View` gains scroll state:

  ```rust
  pub struct ChatScroll {
      pub offset: usize,        // rows from the top of the fully rendered document
      pub follow: bool,         // true = stick to bottom
      pub anchor_turn: Option<u64>,
  }
  ```

- **R-SCROLL-02** On submit, `follow = true` and `anchor_turn = Some(new_turn)`. The
  viewport scrolls so the new user message's header row is the first visible row.
- **R-SCROLL-03** While `follow` is true and the anchored turn's content still fits, the
  view stays anchored at that turn's top — output fills downward and the user message
  stays naturally visible.
- **R-SCROLL-04** Once the anchored turn's content exceeds the viewport, the view switches
  to bottom-follow so new tokens are visible, **and** the anchored user message is rendered
  as a **sticky header**: one row at the top of the chat viewport containing the speaker
  name and the message collapsed to a single truncated line, drawn over the scrolled
  content, followed by a dotted rule (`‥`) in `theme.dim`.
- **R-SCROLL-05** The sticky header shows `↑ N` on the right, where `N` is how many rows
  above the viewport the real message sits. Pressing `Ctrl+↑` jumps to it.
- **R-SCROLL-06** The sticky header disappears when the real message is on screen, when
  the turn completes and the user scrolls, or when the user starts a new turn.
- **R-SCROLL-07** Any upward scroll input sets `follow = false`. The view then does not
  move on its own, no matter how much output streams in.
- **R-SCROLL-08** When `follow = false` and new content arrives, a floating pill appears at
  the bottom-right of the chat pane: `↓ N new` in `theme.accent` on `theme.panel_bg`.
  `End` or `Ctrl+End` re-engages follow and clears it.
- **R-SCROLL-09** Scrolling to the exact bottom re-engages `follow = true` automatically.
- **R-SCROLL-10** Scroll inputs: `PgUp`/`PgDn` (one viewport minus two rows),
  `Shift+↑`/`Shift+↓` (one row), `Ctrl+Home` (top), `Ctrl+End` (bottom, re-follow),
  `Ctrl+↑`/`Ctrl+↓` (previous/next turn boundary), mouse wheel (three rows).
- **R-SCROLL-11** Mouse capture is enabled (`crossterm::event::EnableMouseCapture`) but
  restricted to wheel events and clicks inside panels. Text selection must remain possible
  via the terminal's own modifier (documented in `docs/guide.md` as Shift+drag on most
  terminals). `[ui] mouse = false` disables capture entirely.
- **R-SCROLL-12** Plain `↑`/`↓` in the composer with no palette open move the **cursor**
  within multiline input, and only move history when the composer is empty (§7.3). They
  never scroll the chat.

### 5.5 Markdown rendering

Rewrite of `markdown_lines`. Feature coverage is a contract, not a suggestion.

- **R-MD-01** Block elements supported: ATX headings `#`–`######`, paragraphs, fenced code
  blocks (``` and ~~~, with info string), indented code blocks, unordered lists (`-`, `*`,
  `+`), ordered lists, **nested lists to 4 levels**, block quotes (`>`, nestable), thematic
  breaks (`---`, `***`), and GFM pipe tables.
- **R-MD-02** Inline elements supported: `**bold**`, `*italic*`/`_italic_`,
  `` `code` ``, `~~strikethrough~~`, autolinks, `[text](url)`, and escaped literals.
- **R-MD-03** Headings render as bold in `theme.fg` with a level-proportional prefix:
  `h1` gets a full-width rule underneath in `theme.dim`; `h2` gets a half-width rule;
  `h3`–`h6` get no rule and step down in brightness.
- **R-MD-04** Lists render with real bullets by depth: `•`, `◦`, `▪`, `·`. Ordered lists
  keep their numbers, right-aligned within the marker column. Continuation lines align to
  the text column, not to the marker.
- **R-MD-05** Block quotes render with a `▏` left rule in `theme.dim` and body text one
  step dimmer than `theme.fg`.
- **R-MD-06** Inline code renders on `theme.code_bg` with one space of padding on each
  side, in `theme.code_fg`.
- **R-MD-07** Links render as the link text in `theme.link` underlined, followed by the URL
  in `theme.dim` parentheses when it differs from the text and the line has room; otherwise
  the URL is dropped and the text keeps the underline.
- **R-MD-08** Tables render with box-drawing borders, per-column width computed from
  content and clamped to the pane, header row bold on `theme.panel_bg`, and per-column
  alignment honored from the delimiter row (`:---`, `:---:`, `---:`). Cells that overflow
  are truncated with `…`. A table wider than the pane is horizontally truncated with a `▸`
  marker in the last column rather than wrapped.
- **R-MD-09** Fenced code blocks render inside a bordered block with the language name in
  the top border, on `theme.code_bg`, with syntax highlighting per §5.6.
- **R-MD-10** Code blocks get a dim right-aligned line-number gutter when the block is
  4 lines or longer and the pane is at least 60 columns wide.
- **R-MD-11** An unterminated fence during streaming renders as a code block anyway,
  re-highlighted as the content grows. It must not break the rest of the transcript.
- **R-MD-12** Markdown is rendered for `Assistant`, `Specialist`, and `Merge` bodies. `User`
  bodies get inline-only markdown (code spans, bold) — block parsing is skipped so a user
  typing `# TODO` does not produce a heading. `Tool` and `System` bodies are plain.

### 5.6 Syntax highlighting

- **R-SYN-01** Add `syntect` with the fancy-regex backend (no oniguruma C dependency) and
  `two-face` for the extended syntax set. Load definitions lazily on the first code block
  and cache the `SyntaxSet` in a `OnceLock`.
- **R-SYN-02** **Do not use syntect's bundled TextMate themes.** Parse with syntect, then
  map scope stacks onto the Ryter palette so highlighting always obeys the user's theme.
- **R-SYN-03** Scope→palette mapping (longest-prefix wins):

  | Scope prefix | Palette slot |
  | --- | --- |
  | `comment` | `syn_comment` (italic) |
  | `string`, `constant.character` | `syn_string` |
  | `constant.numeric`, `constant.language` | `syn_number` |
  | `keyword`, `storage` | `syn_keyword` (bold) |
  | `entity.name.function`, `support.function` | `syn_function` |
  | `entity.name.type`, `entity.name.class`, `support.type`, `storage.type` | `syn_type` |
  | `variable.parameter`, `variable.other.member` | `syn_variable` |
  | `punctuation`, `meta.brace` | `syn_punct` |
  | `entity.name.tag`, `meta.attribute`, `meta.annotation` | `syn_attr` |
  | `invalid` | `error` |
  | anything else | `theme.code_fg` |

- **R-SYN-04** Language is taken from the fence info string first, then guessed from a
  filename mentioned in the preceding tool row, then falls back to plain text.
- **R-SYN-05** On a 16-color terminal the nine syntax slots collapse to six
  (`comment`, `string`, `keyword`, `type`, `function`, default) per §9.3.
- **R-SYN-06** Highlighting a single code block must never exceed 8 ms at 200 lines on the
  reference machine. Blocks over 2000 lines render unhighlighted with a dim
  `(highlighting skipped — 2000+ lines)` note in the block border.
- **R-SYN-07** `diff` and `patch` get special handling regardless of syntect: `+` lines in
  `success`, `-` lines in `error`, `@@` hunks in `accent`, context in `dim`.

### 5.7 Wrapping and width correctness

- **R-WRAP-01** Add `unicode-width` and `unicode-segmentation`. All width math uses
  `UnicodeWidthStr::width`, not `chars().count()`. This replaces every width computation in
  `chat.rs`, `draw.rs`, and the composer.
- **R-WRAP-02** Wrap on grapheme cluster boundaries with word-preference; break inside a
  word only when a single word exceeds the line width.
- **R-WRAP-03** Zero-width joiners, combining marks, and emoji sequences must not be split.
- **R-WRAP-04** Code blocks do not word-wrap. They hard-wrap at the block width with a `↳`
  continuation marker in `theme.dim`, so indentation stays truthful.
- **R-WRAP-05** Tabs in code expand to 4 columns before measurement.

### 5.8 Render performance

Full scrollback plus syntect at every frame is not viable without caching. This is a
requirement, not an optimization.

- **R-PERF-01** `chat/cache.rs` memoizes rendered `Vec<Line<'static>>` per message, keyed by
  `(message.id, message.rev, pane_width, theme_generation)`. A cache hit must not re-parse
  markdown or re-run syntect.
- **R-PERF-02** During streaming only the last message is invalidated per frame. Every
  earlier message is served from cache.
- **R-PERF-03** The cache is an LRU bounded to 2000 entries or 20 MB, whichever trips first.
- **R-PERF-04** Total row counts per message are cached alongside the lines so scroll math
  is O(messages), not O(rendered rows).
- **R-PERF-05** A full redraw at 120×40 with 500 cached messages must complete in under
  16 ms. Add a `cargo test --release` benchmark-style test asserting a generous ceiling so
  regressions are caught without being flaky.
- **R-PERF-06** The event loop redraws at most 30 times per second. Token events coalesce:
  drain the whole channel, then draw once.

### 5.9 Scrollbar

- **R-BAR-01** The 1-column gutter between chat and panel renders a track in `theme.dim`
  (`│`) and a thumb in `theme.accent` (`┃`), sized proportional to viewport/document.
- **R-BAR-02** The thumb is at least 1 row tall.
- **R-BAR-03** The gutter is blank (spaces, not `│`) when the document fits the viewport.
- **R-BAR-04** When `follow = false`, the thumb renders in `theme.warn` to signal detached
  scroll.

---

## 6. Activity strip (thinking output)

A dedicated region between the chat and the composer, showing what the model is doing
right now. Today `AgentEvent::Reasoning` is discarded at `run.rs:1702`; it becomes the
content of this strip.

### 6.1 Collapsed (default)

```
 ⠙ writing  ·  edit docs/guide.md  ·  0:34  ·  1.2k tok                    ^r  reasoning  ▾
```

- **R-ACT-01** Height 1. Present whenever a turn is in flight, and after a turn while
  reasoning history exists for it. Height 0 only at a fresh session with no history.
- **R-ACT-02** Fields, left to right: spinner, phase verb, current activity, elapsed,
  tokens this turn. Right: the expand affordance `^r  reasoning  ▾`.
- **R-ACT-03** Spinner is the braille cycle `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏` at 80 ms, in `theme.accent`.
  It freezes and dims when not busy.
- **R-ACT-04** Phase verb derives from the most recent event: `thinking` (Reasoning),
  `writing` (Token), `<tool>` (ToolCall), `waiting` (permission/ask outstanding),
  `merging` (SubagentFinished for a builder), `cancelling` (cancel requested),
  `done`/`stopped`/`failed` at terminal states.
- **R-ACT-05** Current activity is the tool name plus its most identifying argument
  (path for `read`/`edit`, first 40 chars of command for `bash`, query for `web_search`),
  truncated with `…`.
- **R-ACT-06** Elapsed is `M:SS`, from turn submit.
- **R-ACT-07** Token count is cumulative output tokens this turn, humanized (`1.2k`).
  Until the `Spend` event arrives it is an estimate and is rendered dim with a `~` prefix.
- **R-ACT-08** On turn completion the strip holds a terminal summary for the life of the
  turn: `✓ done · 4 tools · 0:41 · $0.012` (or `✕ failed …`, `⊘ cancelled …`).

### 6.2 Expanded

`Ctrl+R` toggles.

```
────────────────────────────────────────────────────────────────────────────────
 ⠙ thinking                                                    0:34   ^r close ▴
 The user wants a --json flag. The CLI is clap-based, so I should add the
 flag to the top-level Args struct and thread an output-mode enum down into
 the sessions subcommand rather than branching at each print site.
 Checking whether ryter-core already has a serde representation for the
 session row before I invent one.
────────────────────────────────────────────────────────────────────────────────
```

- **R-ACT-09** Height `clamp(content_rows + 2, 3, min(12, body_height / 3))`.
- **R-ACT-10** Reasoning text renders word-wrapped in `theme.dim` italic. No markdown, no
  highlighting — it is a stream, not a document.
- **R-ACT-11** The pane auto-follows the reasoning tail. `Alt+PgUp`/`Alt+PgDn` scroll it
  independently without affecting chat scroll.
- **R-ACT-12** Reasoning is retained per turn in `Message`-adjacent storage
  (`View.reasoning: BTreeMap<u64 /*turn*/, String>`), capped at 64 KB per turn, oldest
  turns evicted past 20 turns.
- **R-ACT-13** Reasoning is **display only**. It is never added to the orchestrator
  transcript, never persisted to the session file, and never sent back to a provider.
- **R-ACT-14** `[ui] reasoning = "collapsed" | "expanded" | "off"` sets the startup state.
  `off` also discards `Reasoning` events, restoring today's behavior for users who do not
  want to pay the screen space.
- **R-ACT-15** Models that emit no reasoning must not produce an empty expanded pane —
  expanding shows a dim `no reasoning stream from this model`.

---

## 7. Composer

Replaces `draw_composer` and the key handling at `run.rs:933-997`.

### 7.1 Appearance

```
╭─ you ──────────────────────────────────────────────────────── build ─╮
│ › Add a --json flag to the CLI and make `ryter sessions`             │
│   emit machine-readable output.                                      │
╰──────────────────────────────────────────────────────────────────────╯
 enter send    ⇧enter newline    / commands    ^r reasoning    ^c quit
```

- **R-COMP-01** Bordered with rounded corners. Border color encodes state: `theme.dim`
  idle, `theme.accent` focused-and-nonempty, `theme.build`/phase color during handoff,
  `theme.warn` while a turn is in flight, `theme.error` on a rejected submit.
- **R-COMP-02** Top-left border title is the mode: `you` normally; `→ architect` during a
  handoff; `api key` in secret capture; `filter` when a panel owns the composer.
- **R-COMP-03** Top-right border shows the current phase in the phase accent color.
- **R-COMP-04** The prompt glyph is `›` in `theme.prompt`, `→` for handoff, `🔒`/`key` for
  secret capture. Continuation rows indent to align with the text column.
- **R-COMP-05** Empty composer shows dim placeholder text: `ask the orchestrator, or / for
  commands`. In handoff mode: `pass note for <phase>…`.
- **R-COMP-06** The cursor is a solid block `█` in `theme.prompt`, painted manually
  (keeping today's approach from `draw.rs:1322-1336`), correctly positioned for multi-byte
  and wide characters.
- **R-COMP-07** Secret capture renders `•` per grapheme and **must not** be included in any
  `render_to_string` output used by tests or logs.
- **R-COMP-08** A right-aligned dim counter appears in the bottom border when input exceeds
  200 characters: `1,204 chars`.

### 7.2 Editing

- **R-COMP-09** Multiline input. `Enter` submits; `Shift+Enter` and `Alt+Enter` insert a
  newline. Pasted text containing newlines inserts them literally and never auto-submits.
- **R-COMP-10** Bracketed paste is enabled. A paste over 2000 characters is accepted and
  summarized in the composer as `[pasted 4,210 chars, 118 lines]`, expanded on submit. The
  full text is held in `View.paste_buffer`.
- **R-COMP-11** Cursor movement: `←`/`→`, `Ctrl+←`/`Ctrl+→` by word, `Home`/`End` by line,
  `Ctrl+A`/`Ctrl+E` by line, `↑`/`↓` between lines when multiline.
- **R-COMP-12** Editing: `Backspace`, `Delete`, `Ctrl+W` delete word back, `Ctrl+U` delete
  to line start, `Ctrl+K` delete to line end.
- **R-COMP-13** Prompt history: when the composer is empty, `↑` recalls the previous
  submitted prompt, `↓` moves forward. History is 100 entries, per session, in memory only.
- **R-COMP-14** `Ctrl+C` with a non-empty composer clears it; with an empty composer while
  busy it cancels; with an empty composer while idle it quits. Quitting requires a second
  `Ctrl+C` within 2 seconds, with the hint bar showing `press ^c again to quit`.
- **R-COMP-15** `Esc` precedence: close panel → close palette → cancel handoff → cancel
  in-flight turn → clear composer.
- **R-COMP-16** Submitting while busy queues the message and shows `queued` in the
  composer border; it is sent when the turn completes. `Esc` clears the queue.

### 7.3 Composer as panel input

- **R-COMP-17** Panels that need text (filters, new names, values) take over the composer
  rather than drawing their own input field. The border title changes to the field name and
  the hint bar switches to the panel's keys. This gives one consistent editing surface.

---

## 8. Info panel (right column)

Replaces `draw_sidebar` (`draw.rs:158-251`). Grouped, bordered cards, one per concern.

### 8.1 Cards, in order

**session**

- **R-PANEL-01** Session title (truncated), short session id, phase in phase color, and
  auditor/tools state as compact badges: `auditor ✓`, `tools ask`.

**model**

- **R-PANEL-02** Connection name with a `●` in `success` when a key is resolvable, `○` in
  `warn` when not. Model short id. Price as `$2.00/M in · $6.00/M out`, or dim `price
  unknown` — never a fabricated zero.
- **R-PANEL-03** Context gauge: an 12–20 cell bar plus `NN%`, and a second dim line
  `192k / 500k tokens`. Bar color: `success` under 60%, `warn` 60–84%, `error` 85%+.
- **R-PANEL-04** At 85%+ the card shows an action hint `/compact to reclaim` in `warn`.

**spend**

- **R-PANEL-05** Session total, large and in `theme.fg`. `$?.??` when any turn was unpriced
  — the existing honesty rule is preserved.
- **R-PANEL-06** Budget gauge when `budget_usd > 0`: bar, percent, and `$0.42 of $5.00`.
  Colors mirror `R-PANEL-03`, using `warn_usd` as the amber threshold.
- **R-PANEL-07** Up to 4 rows of by-role spend, descending, with the remainder collapsed
  into `+N more  $X.XX`. Full breakdown lives in the `/spend` panel.

**tasks**

- **R-PANEL-08** Card title shows progress: `tasks ─── 2/4 ─`.
- **R-PANEL-09** Status glyphs and colors: `○` pending (`dim`), `◐` running (`accent`),
  `✓` done (`success`), `✕` blocked (`error`). Done items render struck-through where the
  terminal supports it, dim otherwise.
- **R-PANEL-10** Titles wrap to a second indented line rather than truncating at 16 chars
  as they do today. The card shows up to 8 tasks, then `+N more`.
- **R-PANEL-11** Running tasks sort to the top, then pending, then blocked, then done.

**crew**

- **R-PANEL-12** Card title shows `crew ─── 1/4 ─` (running vs `max_crew`).
- **R-PANEL-13** Each specialist: role in phase color, task label, then a second dim line
  with status, elapsed `M:SS`, and known spend.
- **R-PANEL-14** Empty state is a dim `no specialists running`, not `none`.
- **R-PANEL-15** A specialist blocked on the auditor renders its status in `warn`.

**mcp** (only when servers are configured or inbound is listening)

- **R-PANEL-16** Connected outbound server count and inbound listen address, each with a
  connection dot.

### 8.2 Panel behavior

- **R-PANEL-17** Cards are bordered with the card name in the top-left of the border and
  optional counters in the top-right.
- **R-PANEL-18** When vertical space is short, cards drop in this order: `mcp`, `crew`,
  `tasks`, `spend` detail rows, `session` detail rows. `model` and the spend total never
  drop.
- **R-PANEL-19** Values never truncate mid-number. Given the choice, truncate a label.
- **R-PANEL-20** The trailing `/provider  /models` hint (`draw.rs:239-243`) is removed;
  discovery now lives in the palette and the hint bar.
- **R-PANEL-21** The panel is read-only. Clicking a card opens the corresponding panel
  (`model` → `/models`, `spend` → `/spend`, `crew` → `/crew`, `tasks` → no-op).

---

## 9. Theme and color

### 9.1 Extended palette

`Theme` (`theme.rs:11-39`) grows from 13 to the following. Every new key is optional in
`~/.ryter/themes/*.toml` and derives from an existing key when absent, so current theme
files keep working.

| Key | Purpose | Derives from when absent |
| --- | --- | --- |
| `assistant` | assistant speaker name | `fg` |
| `accent` | selection, spinner, scrollbar thumb, focus | `plan` |
| `success` | ok states, gauges under threshold, diff `+` | `build` |
| `error` | errors, over-budget, diff `-` | `warn` shifted red |
| `panel_bg` | popout and card background | `sidebar_bg` |
| `panel_border` | popout and card border | `dim` |
| `panel_title` | popout title text | `fg` |
| `selection_bg` | highlighted row background | `accent` at 20% over `panel_bg` |
| `code_bg` | code block / inline code background | `bg` lightened 4% |
| `code_fg` | code default foreground | `fg` |
| `link` | link text | `plan` |
| `sticky_bg` | sticky turn header background | `bg` lightened 3% |
| `gauge_track` | unfilled gauge cells | `dim` |
| `syn_comment` | | `dim` |
| `syn_string` | | `build` |
| `syn_number` | | `architect` |
| `syn_keyword` | | `plan` |
| `syn_function` | | `assistant` |
| `syn_type` | | `architect` |
| `syn_variable` | | `fg` |
| `syn_punct` | | `dim` |
| `syn_attr` | | `audit` |

- **R-THEME-01** Derivation is implemented so a one-key theme file still produces a
  coherent palette.
- **R-THEME-02** `Theme` gains a `generation: u32` bumped on every load, used as the render
  cache key (`R-PERF-01`).
- **R-THEME-03** `/theme` changes persist to `~/.ryter/settings.toml` under `[ui] theme`.
  Today the change is session-only (`run.rs:768-774`) — that is a bug, fix it.

### 9.2 Shipped themes

- **R-THEME-04** Ship three: `dark` (current truecolor, extended), `light` (new, for light
  terminals), `default-16` (the safe 16-color fallback, extended).
- **R-THEME-05** Every shipped theme must pass a contrast check: body text on background
  and dim text on background both at or above WCAG AA for normal text (4.5:1). Add a test
  that computes contrast ratios for the shipped palettes.

### 9.3 Degradation

- **R-THEME-06** Truecolor is used when `COLORTERM` is `truecolor` or `24bit`. Otherwise
  the active theme's RGB values are quantized to the 256-color cube.
- **R-THEME-07** When `TERM` indicates 16 colors (or `[ui] colors = 16`), load
  `default-16`. The nine syntax slots collapse to six: `comment`, `string`,
  `keyword`, `type`, `function`, default.
- **R-THEME-08** `NO_COLOR` produces a monochrome render where every distinction is carried
  by glyph, weight, and layout. Speaker headers, tool rows, and diff lines must still be
  unambiguous. Add a snapshot test for this mode.
- **R-THEME-09** No information may be conveyed by color alone anywhere in the UI. Every
  colored state has a glyph or text companion.

---

## 10. Command palette

The `/` menu is rebuilt from a filtered string list into a categorized, described,
fuzzy-matched palette that is the single entry point to every function.

### 10.1 Appearance

```
                    ╭─ commands ──────────────────────── 6 of 34 ─╮
                    │ model                                        │
                    │                                              │
                    │  MODEL & PROVIDERS                           │
                    │ ›/models        switch model           ▸     │
                    │  /provider      switch connection      ▸     │
                    │  /crew          per-role models        ▸     │
                    │                                              │
                    │  CONTEXT                                     │
                    │  /context       inspect window         ▸     │
                    │  /compact       shrink transcript            │
                    │                                              │
                    │  SESSION                                     │
                    │  /rename        set session title            │
                    ╰─ enter run · tab complete · esc close ───────╯
╭─ commands ───────────────────────────────────────────────────────╮
│ /model                                                           │
╰──────────────────────────────────────────────────────────────────╯
```

- **R-PAL-01** The palette is a floating panel anchored directly above the composer,
  left-aligned with it, width `min(72, chat_width)`, height
  `clamp(rows + 2, 6, body_height - 4)`. It overlays the chat; it does not resize it.
- **R-PAL-02** The filter text lives in the composer (per `R-COMP-17`), echoed at the top
  of the panel so the eye does not have to travel.
- **R-PAL-03** Rows are grouped under dim uppercase category headers. Headers are skipped
  when a filter is active and the results are fewer than 8 — a short list reads better flat.
- **R-PAL-04** Each row: selection marker `›`, command name in `theme.fg`, one-line
  description in `theme.dim`, and a right-side affordance — `▸` when it opens a panel, the
  keybinding when one exists, `⌁` for user skills, `⌘` for user commands.
- **R-PAL-05** The panel title shows `N of M` matches.
- **R-PAL-06** The bottom border carries the key legend.
- **R-PAL-07** Zero matches renders `no command matches "<filter>"` plus
  `press enter to send it as a message instead`.

### 10.2 Matching

- **R-PAL-08** Fuzzy subsequence matching over `name + aliases + description`, implemented
  in-tree in `palette/match.rs` — no new dependency.
- **R-PAL-09** Score order: exact name > name prefix > name subsequence with consecutive
  runs > alias match > description match. Ties break on category order, then alphabetically.
- **R-PAL-10** Matched characters in the name render in `theme.accent` bold.
- **R-PAL-11** Matching is case-insensitive. `/` separators in user command names are
  matchable.
- **R-PAL-12** Recently used commands get a small score boost (last 10, in-memory).

### 10.3 Keys

- **R-PAL-13** `↑`/`↓` move; `Tab` completes the name into the composer without running;
  `Enter` runs; `Esc` closes and leaves the composer text; a second `Esc` clears it.
- **R-PAL-14** `→` on a `▸` row opens its panel directly.
- **R-PAL-15** `Ctrl+P` opens the palette from an empty composer without typing `/`.
- **R-PAL-16** Typing a space after a complete command name closes the palette and lets the
  user type arguments (preserving today's behavior).

### 10.4 Full command inventory

Every existing function survives. The change is organization, description, and where the
command lands. **Panel** means it opens a popout (§11); **inline** means it acts
immediately; **hidden alias** means it is matchable and runnable but not listed.

#### Session

| Command | Aliases | Description | Lands |
| --- | --- | --- | --- |
| `/new` | | Start a fresh session | inline (confirm if the current session has content) |
| `/sessions` | `/resume` | Browse, resume, rename, or delete sessions | **panel** (§11.5) |
| `/rename` | | Set the session title | panel §11.5, focused on rename; `/rename <title>` stays inline |
| `/delete` | | Delete a saved session | panel §11.5, delete mode |
| `/quit` | `/exit` | Leave Ryter | inline |

#### Model & providers

| Command | Aliases | Description | Lands |
| --- | --- | --- | --- |
| `/models` | `/model` | Switch the orchestrator model | **panel** (§11.2) |
| `/provider` | `/providers`, `/connections` | Switch or add a connection, set API keys | **panel** (§11.1) |
| `/crew` | | Assign models per specialist role, save presets | **panel** (§11.3) |

#### Agents & phase

| Command | Aliases | Description | Lands |
| --- | --- | --- | --- |
| `/agents` | | Running specialists; kill one | **panel** (§11.4) |
| `/phase` | | Change the campaign phase | **panel** (§11.11) |
| `/handoff` | | Hand off to a phase with a pass note | panel §11.11, then composer note mode |
| `/plan` | | Hand off to the planner | inline → composer note mode |
| `/architect` | | Hand off to the architect | inline → composer note mode |
| `/build` | | Hand off to a builder | inline → composer note mode |
| `/audit` | | Hand off to the auditor | inline → composer note mode |
| `/cancel` | | Stop the in-flight turn | inline |

#### Context

| Command | Aliases | Description | Lands |
| --- | --- | --- | --- |
| `/context` | | Inspect window use, message counts, largest contributors | **panel** (§11.12) — today it only dumps a line |
| `/compact` | | Summarize and shrink the transcript | inline, with a before/after toast |
| `/spend` | | Spend by role, connection, and turn | **panel** (§11.6) |

#### Configuration

| Command | Aliases | Description | Lands |
| --- | --- | --- | --- |
| `/settings` | | Budget, warn, max crew, sandbox, inbound MCP, web | **panel** (§11.7) |
| `/theme` | | Switch and preview themes | **panel** (§11.8) — with live preview |
| `/tools` | `/permissions`, `/ask`, `/auto`, `/always` | Tool permission mode | **panel** (§11.9) |
| `/auditor` | | Auditor gate on or off | **panel** (§11.9) |

`/ask`, `/auto`, and `/always` become **hidden aliases** that jump straight to the
resulting state. They keep working; they stop cluttering the list.

#### Extensions

| Command | Aliases | Description | Lands |
| --- | --- | --- | --- |
| `/mcp` | | Inbound and outbound MCP servers, tokens, client links | **panel** (§11.10) |
| `/skills` | | Browse, run, create, and remove skills and user commands | **panel** (§11.10) |
| `/hooks` | | Lifecycle hooks | **panel** (§11.10) |

#### Help & diagnostics

| Command | Aliases | Description | Lands |
| --- | --- | --- | --- |
| `/help` | `/?` | Keymap and command reference | **panel** (§11.13) — today it dumps one long line |
| `/doctor` | | Environment, keys, sandbox, and provider checks | **panel** (§11.14) — today it dumps text |

#### User-supplied

- **R-PAL-17** Skills (`user_invocable`) and `~/.ryter/commands/*.md` appear in a trailing
  `YOUR COMMANDS` category with their own descriptions from the skill frontmatter, marked
  `⌁` and `⌘`.
- **R-PAL-18** A user command whose name collides with a built-in is listed with a dim
  `(shadowed)` note; the built-in still wins, matching current behavior.

### 10.5 Command registry

- **R-PAL-19** Commands become data, not a `match` arm and a parallel string array. Replace
  `view.rs:557-592` and the dispatch in `commands.rs:121-285` with:

  ```rust
  pub struct CommandSpec {
      pub name: &'static str,
      pub aliases: &'static [&'static str],
      pub category: Category,
      pub description: &'static str,
      pub usage: Option<&'static str>,
      pub opens_panel: bool,
      pub keybinding: Option<&'static str>,
      pub hidden: bool,
      pub run: fn(&mut View, &str) -> Action,
  }

  pub const COMMANDS: &[CommandSpec] = &[ /* … */ ];
  ```

- **R-PAL-20** `/help` (§11.13) is generated from `COMMANDS`. The hardcoded `help_text()`
  string at `commands.rs:116-118` is deleted.
- **R-PAL-21** A test asserts every `CommandSpec` has a non-empty description, that no two
  specs share a name or alias, and that every non-hidden spec is reachable from the palette.

---

## 11. Popout panels

A shared panel system replaces the twelve ad-hoc `Overlay` draw functions in `draw.rs`.

### 11.0 Common contract

- **R-POP-01** A panel is a bordered floating box centered horizontally, vertically
  positioned at 1/3 from the top, drawn over everything except the composer and hint bar
  (which remain visible and never occluded, per `R-LAYOUT-02`).
- **R-POP-02** Size: width `clamp(preferred, 40, terminal_width - 8)`; height
  `clamp(content + chrome, 8, terminal_height - 10)`. Under 60 columns or 20 rows a panel
  goes full-screen-minus-composer instead of floating.
- **R-POP-03** Chrome: rounded border in `panel_border` on `panel_bg`; title in
  `panel_title` in the top-left of the border; a right-side counter or status in the top
  border; a key legend in the bottom border.
- **R-POP-04** The content behind a panel dims one step. Implemented by rendering the base
  frame, then a `Clear` + a dim-style overlay on the panel's shadow region, then the panel.
- **R-POP-05** A panel drops a one-column shadow to the right and one row below in a
  darkened background, so it visibly floats.
- **R-POP-06** Lists scroll with `↑`/`↓`/`PgUp`/`PgDn`, wrap at the ends, and show a
  scrollbar in the right border when content overflows.
- **R-POP-07** Universal keys: `Esc` back one level (or close at the top level),
  `Enter` activate, `Tab`/`Shift+Tab` move between fields in form panels, `?` show that
  panel's keys inline in the bottom border.
- **R-POP-08** Panels stack. `Esc` pops one. `/models` opened from `/crew` returns to
  `/crew` on `Esc` or on selection — preserving today's `assign_role` behavior generically.
- **R-POP-09** Destructive actions require typed or explicit confirmation, never a bare
  `Enter`.

### 11.0.1 Form widget kit

Every configuration panel is built from these, in `panel/widgets.rs`:

- **R-POP-10** `Toggle` — label, `[ on ]`/`[ off ]` pill, `Space`/`Enter`/`←`/`→` flips.
- **R-POP-11** `Select` — label, current value, `←`/`→` cycles or `Enter` opens a sub-list.
- **R-POP-12** `TextField` — label, value, `Enter` edits via the composer (`R-COMP-17`).
- **R-POP-13** `SecretField` — same, masked, with a `reveal` action and a redaction
  guarantee in all rendered output.
- **R-POP-14** `NumberField` — with min/max/step, `←`/`→` adjusts, typed entry validated
  with an inline red message on the field rather than a chat line.
- **R-POP-15** `ListRow` — icon, primary text, secondary dim text, right-aligned status,
  and per-row actions surfaced in the bottom legend.
- **R-POP-16** `Gauge` — label, bar, value, threshold colors.
- **R-POP-17** `Table` — column headers, alignment, selection.
- **R-POP-18** Every form panel has explicit `save` / `cancel` semantics shown in the
  legend. Nothing is written on keystroke unless the panel says `changes apply immediately`
  in its legend.

### 11.1 `/provider` — Connections

```
╭─ connections ────────────────────────────────── 3 ─╮
│  ● spacexai      xai · grok-4.6         key set    │
│ ›○ openrouter    openrouter             no key     │
│  ● local         openai-compat          key set    │
│                                                    │
│  + add connection                                  │
│  ⚙ test connection                                 │
╰─ enter use · k set key · t test · d remove · esc ──╯
```

- **R-POP-19** Rows show active dot, name, kind, default model, and key state.
- **R-POP-20** `k` opens a `SecretField` for the API key in the composer; the key is never
  echoed and never enters the render string.
- **R-POP-21** `t` runs a live connectivity test and shows per-row status
  (`testing…` → `ok 240ms` / `failed 401`) inline, not in chat.
- **R-POP-22** `+ add connection` is a multi-step form: name → kind → base URL → env key →
  default model, with back navigation and a review step before writing
  `~/.ryter/connections.toml`.
- **R-POP-23** `d` removes a connection behind a typed-name confirmation.

### 11.2 `/models` — Model picker

- **R-POP-24** Columns: model id, context window, input price, output price, connection.
  Prices right-aligned and column-aligned; unknown prices render `?`, never `0`.
- **R-POP-25** Live filter from the composer, matching id, short id, and connection.
- **R-POP-26** Sort cycling with `s`: by relevance, name, context, price.
- **R-POP-27** `loading` state renders a spinner row, not a blank panel.
- **R-POP-28** When opened from `/crew`, the title reads `model for builder` and the first
  row is `default (follows orchestrator)` — preserving the current contract tested at
  `lib.rs:216-247`.
- **R-POP-29** A detail footer shows the highlighted model's full id and full pricing,
  since the row truncates.

### 11.3 `/crew` — Specialist routing

- **R-POP-30** One row per role in `CREW_ROLES` with its resolved connection and model, and
  an explicit `default (grok-4.6)` rendering for unassigned roles — the existing behavior
  tested at `lib.rs:196-213` must survive.
- **R-POP-31** `Enter` opens `/models` scoped to that role; `r` resets a role to default.
- **R-POP-32** A presets section: `save preset`, and a list of saved presets with
  `Enter` to load and `d` to delete.
- **R-POP-33** The panel shows `max concurrent: N` with a link to `/settings`.

### 11.4 `/agents` — Running specialists

- **R-POP-34** Rows: role in phase color, task label, status, elapsed, known spend.
- **R-POP-35** `Enter` or `k` kills the highlighted specialist behind a `y/n` confirm.
- **R-POP-36** `K` kills all, behind a typed `kill all` confirmation.
- **R-POP-37** Empty state: `no specialists running` with a dim hint on how they spawn.

### 11.5 `/sessions` — Session browser

- **R-POP-38** Unifies `/resume`, `/rename`, and `/delete`, which today are three
  `ChoiceKind` variants.
- **R-POP-39** Rows: title, relative time (`2h ago`), message count, total spend, phase.
- **R-POP-40** Sorted most-recent-first; filterable by title from the composer.
- **R-POP-41** `Enter` resumes, `r` renames in place, `d` deletes behind a typed
  confirmation, `n` starts a new session.
- **R-POP-42** The current session is marked and cannot be deleted from here.

### 11.6 `/spend` — Spend detail

- **R-POP-43** Three stacked tables: total with budget gauge; by role; by connection.
- **R-POP-44** Each row: name, calls, input tokens, output tokens, cached tokens, USD.
- **R-POP-45** Unknown-price calls are counted in a distinct `unpriced: N calls` row so the
  `$?.??` total is explainable rather than mysterious.
- **R-POP-46** `e` exports the breakdown to `~/.ryter/spend-<session>.csv` and reports the
  path in the panel footer.

### 11.7 `/settings` — Settings form

- **R-POP-47** Grouped sections using the widget kit:

  | Group | Fields |
  | --- | --- |
  | spend | `session_budget_usd` (Number), `warn_usd` (Number) |
  | agents | `subagents.max` (Number 1–16), `auditor` (Toggle) |
  | tools | permission mode (Select: ask / always), `features.web` (Toggle) |
  | sandbox | profile (Select), with a dim note that non-off profiles fail closed |
  | mcp | inbound (Toggle), bind address (TextField) |
  | ui | theme (Select), username (TextField), reasoning (Select), mouse (Toggle), panel (Toggle) |

- **R-POP-48** Each field shows its current value, its default, and where it came from
  (`config`, `project`, `default`) in dim text on the right. This makes the precedence
  chain in `docs/guide.md` visible instead of folklore.
- **R-POP-49** Validation is inline and blocks save; it does not write a chat line.
- **R-POP-50** `Ctrl+S` saves to `~/.ryter/settings.toml`; `Esc` with unsaved edits asks
  `discard changes? y/n`.

### 11.8 `/theme` — Theme picker

- **R-POP-51** List of built-ins plus `~/.ryter/themes/*.toml` stems.
- **R-POP-52** Moving the selection **live-previews** the theme on the whole UI behind the
  panel. `Esc` reverts; `Enter` commits and persists (`R-THEME-03`).
- **R-POP-53** A swatch strip shows the palette's key colors so a theme can be judged
  without committing.

### 11.9 `/tools` and `/auditor` — Small toggles

- **R-POP-54** These share a compact two-to-three-row panel with an explanatory sentence
  per option, because "ask vs always" is a security decision and deserves prose.
- **R-POP-55** `always` is rendered in `warn` with the sentence *"tools run without asking,
  including destructive ones."*

### 11.10 `/mcp`, `/skills`, `/hooks` — Multi-pane panels

- **R-POP-56** The existing `McpPane` / `SkillsPane` / `HooksPane` drill-down state machines
  are preserved, but re-skinned onto the shared panel and widget kit. The ad-hoc per-pane
  draw code is removed.
- **R-POP-57** Every "add" wizard gains: a step indicator (`step 2 of 3`), back navigation
  with `Esc`, per-step validation, and a review step showing exactly what will be written
  and to which file.
- **R-POP-58** `/mcp` shows per-server connection status with a dot and last-error text,
  and offers `reconnect` on a row.
- **R-POP-59** `/mcp` inbound tokens stay masked with an explicit `reveal` action, matching
  today's `mcp_reveal` behavior.
- **R-POP-60** `/skills` rows show the source path in dim and offer `Enter` run, `e` open in
  `$EDITOR`, `d` remove.
- **R-POP-61** `/hooks` rows show event, matcher, and target, with the four lifecycle events
  presented as a `Select` rather than a free-text field.

### 11.11 `/phase` and `/handoff`

- **R-POP-62** Four rows, one per phase, each with its accent color and a one-line
  description of what that specialist kind does.
- **R-POP-63** Selecting a phase closes the panel and puts the composer into handoff note
  mode with the phase color on the composer border (`R-COMP-01`, `R-COMP-02`).

### 11.12 `/context` — Context inspector (new)

- **R-POP-64** Replaces today's single `Context` line. Shows: tokens used, window, percent
  with a gauge, message count, and a breakdown by contributor (system prompt, project
  memory files, transcript, tool output) with per-contributor token estimates.
- **R-POP-65** Offers `/compact` as an action from the panel, with an estimate of what it
  would reclaim.
- **R-POP-66** Requires a small `ryter-core` addition: `AgentEvent::Context` gains a
  `breakdown: Vec<(String, u64)>` field, `#[serde(default)]` so existing session logs
  deserialize.

### 11.13 `/help` — Keymap and command reference (new)

- **R-POP-67** Two tabs: `keys` and `commands`, switched with `←`/`→`.
- **R-POP-68** `keys` is generated from the keymap table in §12 grouped by context.
- **R-POP-69** `commands` is generated from `COMMANDS` (`R-PAL-19`), grouped by category,
  with usage strings.
- **R-POP-70** Both tabs are filterable from the composer.

### 11.14 `/doctor` — Diagnostics (new)

- **R-POP-71** Replaces the chat dump. One row per check with `✓`/`!`/`✕`, the check name,
  and a result summary.
- **R-POP-72** Checks run asynchronously with per-row spinners; the panel is usable while
  they run.
- **R-POP-73** Failed checks expand on `Enter` to show the remedy text.
- **R-POP-74** `c` copies the full report to the clipboard when one is available, otherwise
  writes `~/.ryter/doctor-report.txt` and shows the path.

### 11.15 Modal interrupts

`Permission` and `AskUser` are not browsable panels — they block the turn.

- **R-POP-75** They render with a `theme.warn` (permission) or `theme.accent` (ask) border
  and a distinctly heavier top border, so they are never mistaken for a config panel.
- **R-POP-76** The permission modal shows tool name, a syntax-highlighted argument preview
  (the command, the diff, the path), and the three choices spelled out:
  `y allow once · n deny · a allow all this session`. The existing `y · allow` text
  asserted at `lib.rs:257` must remain present in some form.
- **R-POP-77** `a` (allow all) shows an inline warning sentence before it takes effect.
- **R-POP-78** The `ask_user` modal supports both free text (via the composer) and numbered
  choices (`1`–`9` select directly).
- **R-POP-79** Modal interrupts take priority over any open panel and suppress the palette.
- **R-POP-80** The legacy one-row inline ask bar (`draw_ask`, `draw.rs:1262-1278`) and the
  now-unused `View.permission: Option<String>` field are removed.

---

## 12. Keymap

The full binding table. `/help` renders this; a test asserts they do not diverge.

### Global

| Key | Action |
| --- | --- |
| `Ctrl+C` | clear composer → cancel turn → quit (double-press) |
| `Ctrl+D` | quit when the composer is empty |
| `Ctrl+P` | open the command palette |
| `Ctrl+R` | toggle the reasoning pane |
| `Ctrl+B` | toggle the info panel |
| `Ctrl+L` | redraw |
| `F1` | open `/help` |
| `Esc` | back one level (§`R-COMP-15`) |

### Chat scroll

| Key | Action |
| --- | --- |
| `PgUp` / `PgDn` | scroll one viewport |
| `Shift+↑` / `Shift+↓` | scroll one row |
| `Ctrl+Home` / `Ctrl+End` | top / bottom (bottom re-engages follow) |
| `Ctrl+↑` / `Ctrl+↓` | previous / next turn |
| wheel | scroll three rows |

### Composer

| Key | Action |
| --- | --- |
| `Enter` | send |
| `Shift+Enter`, `Alt+Enter` | newline |
| `←` `→` `Home` `End` `Ctrl+A` `Ctrl+E` | move |
| `Ctrl+←` `Ctrl+→` | move by word |
| `↑` `↓` | move between lines, or prompt history when empty |
| `Ctrl+W` `Ctrl+U` `Ctrl+K` | delete word back / to start / to end |
| `/` at position 0 | open the palette |

### Palette

| Key | Action |
| --- | --- |
| `↑` `↓` | move |
| `Tab` | complete name |
| `Enter` | run |
| `→` | open the command's panel |
| `Esc` | close |

### Panels

| Key | Action |
| --- | --- |
| `↑` `↓` `PgUp` `PgDn` | move |
| `Enter` | activate |
| `Tab` `Shift+Tab` | next / previous field |
| `Space` | toggle |
| `←` `→` | cycle a select |
| `Ctrl+S` | save |
| `?` | show this panel's keys |
| `Esc` | back / close |

- **R-KEY-01** Bindings are declared in one `KEYMAP` table with a context discriminant.
- **R-KEY-02** A test asserts no two bindings collide within a context.
- **R-KEY-03** No binding may shadow a terminal-critical sequence (`Ctrl+Z`, `Ctrl+S`
  flow control is handled by disabling XON/XOFF in raw mode, `Ctrl+Q`).

---

## 13. State and module changes

### 13.1 New module layout for `ryter-tui`

```
src/
  lib.rs
  run.rs            event loop, terminal setup, worker wiring
  view/
    mod.rs          View struct, session state
    scroll.rs       ChatScroll, anchoring, sticky header math
    history.rs      prompt history
  chat/
    mod.rs          message model, render orchestration
    markdown.rs     block + inline parser → Vec<Line>
    highlight.rs    syntect integration, scope→palette mapping
    wrap.rs         unicode-aware wrapping
    cache.rs        LRU render cache
  composer/
    mod.rs          multiline buffer, cursor, paste
    draw.rs
  panel/
    mod.rs          panel stack, geometry, chrome
    widgets.rs      Toggle, Select, TextField, SecretField, Number, ListRow, Gauge, Table
    providers.rs  models.rs  crew.rs  agents.rs  sessions.rs
    spend.rs  settings.rs  theme.rs  toggles.rs  mcp.rs  skills.rs
    hooks.rs  phase.rs  context.rs  help.rs  doctor.rs  modal.rs
  palette/
    mod.rs          palette state + draw
    registry.rs     COMMANDS table
    match.rs        fuzzy scorer
  activity.rs       thinking strip
  info/
    mod.rs          info panel
    cards.rs
  theme.rs          extended palette, derivation, degradation
  keymap.rs         KEYMAP table
  draw.rs           top-level frame assembly only
```

- **R-MOD-01** `draw.rs` shrinks to frame assembly. Its current ~1380 lines of per-overlay
  drawing move into `panel/`.
- **R-MOD-02** Each panel module owns its state struct, key handling, and draw. The
  monolithic `Overlay` enum becomes a `PanelStack` of boxed panel states.

### 13.2 `View` changes

Added: `username`, `messages`, `scroll: ChatScroll`, `reasoning`, `activity: Activity`,
`composer: Composer` (struct, not `String`), `palette: Option<Palette>`,
`panels: PanelStack`, `prompt_history`, `panel_visible: bool`, `turn: u64`,
`queued_prompt: Option<String>`.

Removed: `lines`, `slash`, `overlay`, `permission`, `secret_for`, `secret_buf`
(moved into the composer/modal state), `hooks_help` (generated by the panel).

- **R-STATE-01** `View` remains terminal-free and agent-free, so `render_to_string` stays
  the test harness.
- **R-STATE-02** `View` stays `Clone` for snapshot tests.

### 13.3 `ryter-core` changes

Kept deliberately minimal.

- **R-CORE-01** `AgentEvent::Context` gains `#[serde(default)] breakdown: Vec<(String, u64)>`
  (§11.12).
- **R-CORE-02** `AgentEvent::ToolCall` gains `#[serde(default)] summary: Option<String>` —
  a short human label the core can produce better than the TUI can guess (§6, `R-ACT-05`).
- **R-CORE-03** `AgentEvent::ToolResult` gains `#[serde(default)] duration_ms: Option<u64>`
  for the tool row's elapsed metadata (`R-CHAT-07`).
- **R-CORE-04** A new `AgentEvent::TurnStarted { turn: u64 }` and
  `AgentEvent::TurnFinished { turn: u64, tools: u32, duration_ms: u64 }` so the activity
  strip's terminal summary (`R-ACT-08`) is authoritative instead of inferred from `Spend`.
- **R-CORE-05** All additions are `#[serde(default)]` or new variants, so existing session
  files and MCP subscribers keep deserializing. Headless and MCP consumers must be checked
  for exhaustive matches on `AgentEvent`.
- **R-CORE-06** Nothing in this redesign may cause `ryter --version` to touch config, the
  keyring, or the network.

### 13.4 Event handling

- **R-EVT-01** `Reasoning` is consumed and routed to the activity strip (§6), no longer
  dropped at `run.rs:1702`.
- **R-EVT-02** `ToolResult` errors create a `System { level: Error }` message with the
  actual output, not today's bare `"tool error"`.
- **R-EVT-03** `busy` is driven by `TurnStarted`/`TurnFinished`, with `Spend`, `Error`, and
  `Cancelled` as fallbacks, fixing the current gap where the header spinner lags the stream.
- **R-EVT-04** `Compacted` produces a visible chat marker: a dim horizontal rule reading
  `transcript compacted · 180k → 42k tokens`, so history loss is never silent.
- **R-EVT-05** Event draining coalesces per frame (`R-PERF-06`).

---

## 14. Configuration additions

New `[ui]` section in `~/.ryter/settings.toml`, documented in `config.example.toml` and
`docs/guide.md`.

```toml
[ui]
username  = "Dusty"        # else git user.name, else $USER, else "you"
theme     = "dark"         # persisted by /theme
reasoning = "collapsed"    # collapsed | expanded | off
mouse     = true           # false disables mouse capture entirely
panel     = true           # info panel visible at startup
colors    = "auto"         # auto | truecolor | 256 | 16
timestamps = true          # show HH:MM on speaker headers
line_numbers = true        # line numbers in code blocks
```

- **R-CFG-01** Every key is optional with the default shown.
- **R-CFG-02** Unknown keys warn once at startup in a `System` message rather than failing.
- **R-CFG-03** `[ui]` follows the existing precedence chain: CLI > env > trusted project
  `.ryter/config.toml` > `~/.ryter/config.toml` > sidecars > built-ins.
- **R-CFG-04** `config.example.toml` and `docs/guide.md` are updated in the same change.

---

## 15. Testing contract

Existing harness (`render_to_string` + string assertions) is kept and extended.

- **R-TEST-01** Golden snapshots at 80×24, 100×30, 120×40, and 160×50 for: idle session,
  mid-stream with reasoning collapsed, mid-stream with reasoning expanded, palette open,
  each panel, permission modal, `NO_COLOR` mode, and a 16-color terminal.
- **R-TEST-02** The box-corner assertions at `lib.rs:71-74` and `lib.rs:86` are deleted
  (`R-CHROME-03`). New assertions check that panels **do** have borders and titles.
- **R-TEST-03** Scroll tests: composer is present in every render regardless of message
  count; the in-flight user message or its sticky header is present in every mid-stream
  render; scrolling up detaches follow; new content while detached does not move the
  viewport; `Ctrl+End` re-attaches.
- **R-TEST-04** Markdown tests, table-driven over `(input, expected rows)`: each block type
  in `R-MD-01`, each inline type in `R-MD-02`, nested lists to 4 levels, a GFM table with
  all three alignments, an unterminated fence.
- **R-TEST-05** Highlight tests: a Rust block produces at least three distinct foreground
  colors; a `diff` block colors `+`/`-` correctly; an unknown language does not panic; a
  2000-line block takes the skip path.
- **R-TEST-06** Wrap tests: CJK, emoji with ZWJ, combining marks, a 300-character
  unbreakable token, and tab expansion — each asserting the rendered row width never
  exceeds the pane width.
- **R-TEST-07** Palette tests: fuzzy ranking order from `R-PAL-09`; every `CommandSpec` is
  reachable; no duplicate names or aliases; `/mdl` ranks `/models` first.
- **R-TEST-08** Keymap tests: no intra-context collisions; `/help` output matches `KEYMAP`.
- **R-TEST-09** Secret redaction test: with a key in `secret_buf`, the key's characters
  appear nowhere in `render_to_string` output.
- **R-TEST-10** Theme tests: contrast ratios for shipped themes (`R-THEME-05`); a one-key
  theme file derives a full palette; 16-color degradation produces no `Color::Rgb`.
- **R-TEST-11** Performance test (`--release`): 500 cached messages redraw at 120×40 under
  a generous ceiling.
- **R-TEST-12** `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D
  warnings`, and `cargo fmt --all -- --check` must pass, per `RYTER.md`.
- **R-TEST-13** The CI workflow at `.github/workflows/ci.yml` runs build and e2e only. No
  container image build or publish is added.

---

## 16. Non-goals

Explicitly out of scope, so the contract has edges:

- Image or sixel rendering.
- Mouse-driven text selection replacing the terminal's own.
- A second frontend (web, GUI).
- Persisting reasoning to session files.
- Changing the agent loop, provider layer, sandbox, or tool set.
- Renaming, rescoping, or splitting the three crates.
- Adding SQLite, telemetry, or a hosted proxy (forbidden by `RYTER.md`).
- Adding the `rmcp` crate (forbidden by `RYTER.md`).

---

## 17. Dependencies added

| Crate | Version | Why | Notes |
| --- | --- | --- | --- |
| `syntect` | `5` | code highlighting | `default-features = false`, `features = ["default-fancy"]` — fancy-regex backend, no oniguruma C dependency |
| `two-face` | `0.4` | extended syntax and theme set | syntaxes only; themes unused per `R-SYN-02` |
| `unicode-width` | `0.2` | correct display width | |
| `unicode-segmentation` | `1` | grapheme-cluster wrapping | |

- **R-DEP-01** No fuzzy-matching crate; the scorer is in-tree (`R-PAL-08`).
- **R-DEP-02** No markdown crate in this release; the parser stays in-tree. Swapping to
  `pulldown-cmark` is a candidate follow-up and must be its own decision.
- **R-DEP-03** `cargo tree` growth must be reviewed; syntect's default oniguruma path is
  explicitly avoided so the build stays pure Rust.
- **R-DEP-04** `#![forbid(unsafe_code)]` stays on `ryter-tui`.

---

## 18. Implementation phases

Each phase is independently shippable and leaves the TUI working.

| Phase | Scope | Exit criteria |
| --- | --- | --- |
| **1 — Foundation** | `Message` model, unicode wrapping, render cache, scroll state, sticky turn header, scrollbar | `R-CHAT-01..04`, `R-SCROLL-*`, `R-WRAP-*`, `R-PERF-*`; snapshots updated |
| **2 — Reading** | Speaker headers, username resolution, markdown rewrite, syntect, extended theme | `R-CHAT-05..13`, `R-MD-*`, `R-SYN-*`, `R-THEME-*` |
| **3 — Writing** | Multiline composer, paste, history, keymap table, hint bar | `R-COMP-*`, `R-KEY-*` |
| **4 — Awareness** | Activity strip, reasoning routing, core event additions, info panel rebuild | `R-ACT-*`, `R-PANEL-*`, `R-CORE-*`, `R-EVT-*` |
| **5 — Command surface** | Command registry, palette, fuzzy matcher, panel framework + widget kit | `R-PAL-*`, `R-POP-01..18` |
| **6 — Panels** | Port all panels; add `/context`, `/help`, `/doctor`; remove the inline ask bar | `R-POP-19..80`, `R-MOD-*` |
| **7 — Polish** | `[ui]` config, degradation modes, docs, full snapshot suite | `R-CFG-*`, `R-TEST-*`, `R-CHROME-02/04` |

- **R-PHASE-01** Each phase is a PR from `0.2.0-patch` into `dev`, reviewed by an outside
  agent per the project's review rules.
- **R-PHASE-02** `ROADMAP.md` moves each phase through Now → Done as it lands.
- **R-PHASE-03** `DECISIONS.md` records: retiring the no-boxes rule (`R-CHROME-04`), taking
  the syntect dependency, dropping right-aligned user bubbles, and making reasoning
  display-only.

---

## 19. Decisions locked in this contract

Recorded here so implementation does not relitigate them.

1. **Both pinning guarantees.** The composer is structurally pinned *and* the in-flight
   user message stays visible via anchoring plus a sticky header.
2. **Full scrollback** with keyboard and mouse-wheel navigation, stick-to-bottom with
   auto-release, and an opt-out for mouse capture.
3. **Borders are allowed.** The `RYTER.md` no-boxes non-negotiable is retired and its tests
   deleted.
4. **syntect + two-face** for highlighting, with Ryter's own palette rather than TextMate
   themes.
5. **Full command palette**: categories, fuzzy match, descriptions, keybinding column, and
   a popout panel for every configuration command. No function is removed; near-duplicate
   commands become hidden aliases.
6. **Username** resolves `[ui] username` → `git user.name` → `$USER` → `you`.
7. **Reasoning** shows as a collapsed one-line ticker, expandable to a scrollable pane via
   `Ctrl+R`, display-only and never fed back to a model.

---

## 20. Open items

Not blockers, but they need an answer before the phase that touches them.

- **O-01 (Phase 4)** Should `Spend` remain the fallback signal for `busy`, or is
  `TurnFinished` enough once headless and MCP consumers are updated?
- **O-02 (Phase 6)** Should `/doctor` results also be writable as JSON for CI use, or is
  that a `ryter doctor --json` CLI concern instead?
- **O-03 (Phase 7)** Does the `light` theme ship in 0.2.0 or wait until the palette has
  settled against real use?
- **O-04 (Phase 2)** Do we want a per-message `c` copy action (copy body, copy last code
  block) in this release, or defer with the clipboard question in `R-POP-74`?
