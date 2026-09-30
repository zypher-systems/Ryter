---
name: canvas
description: Build a page to show the user something a terminal shows badly, such as a report, a comparison, a plan, a dashboard, a timeline, a chart or a diagram. Use it when the user asks to see, compare or lay something out, or when the answer has more structure than a few lines.
user-invocable: true
---

# Canvas: a page for the user

Compose one self-contained HTML page and pass the whole of it to `show_page`, with a title. Ryter saves it outside the project and opens it for the user. Calling `show_page` again with the same title replaces the page, so revise it in place.

Don't save the page into the project with `write`, and don't leave scripts or scratch files there to gather facts. The page is for the user, not part of their code. Read files and run read-only commands for what you need.

## When a page helps

- The answer is a comparison, a table wider than a terminal, a chart, a timeline, a diagram, or several sections the user will scan.
- The user asks to see, show, compare, visualize or lay something out.

Don't make a page for an answer that fits in a few sentences.

## Facts first

- Every number and name on the page comes from something you read or ran in this session. If you don't know a value, leave it out or mark it unknown. Never use sample or made-up data.
- Near the end, say where the facts came from in one short line: the files, the commands, the date.

## Structure

- Start with a title that names the subject. Follow it with one or two sentences giving the answer, and put the detail after that.
- Put the most important thing first. Use short section headings. Use tables for exact values and charts for shape.
- A page shows one subject. Make a second page for a second subject.

## The file

- Use one file: HTML, CSS in a `<style>` tag, and any script inline. Don't link to other files, web fonts, CDNs or images online. The page opens from disk, and Ryter blocks all network access in it. Draw charts and diagrams in inline SVG. Put any image in a `data:` URL.
- Start with `<!doctype html>`, `<meta charset="utf-8">`, `<meta name="viewport" content="width=device-width, initial-scale=1">` and a `<title>`.
- Keep it under about 200 KB.

## Look

- Define colours once, as CSS variables on `:root`, with a dark set under `@media (prefers-color-scheme: dark)`. Use only the variables after that.
- Use the system font for text: `system-ui, sans-serif`. Use `ui-monospace, monospace` for code, paths and ids. Give columns of numbers `font-variant-numeric: tabular-nums` so they line up.
- Centre the content, at most about 960px wide, with 24px of padding. At phone width nothing may scroll sideways: a wide table scrolls inside its own box.
- Body text needs a contrast of at least 4.5:1 against its background, in both themes. Never let colour alone carry meaning: pair it with a word or a shape.
- Use one accent colour for emphasis and a muted colour for secondary text. Skip gradients, shadows and decoration that carries no meaning.

## Charts in SVG

- Use bars to compare amounts and lines for change over time. With only a handful of numbers, use a table.
- Label the values directly, and the axes when they aren't obvious. Start bar axes at zero. Round numbers to what matters.
- Colour with `currentColor` and the CSS variables, so charts follow the theme.
- Give every chart a `<title>` in the SVG and a sentence of text near it that says what it shows.

## Template

```html
<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Crew cost by task</title>
<style>
  :root {
    --bg: #fbfaf7; --panel: #ffffff; --fg: #1d1c1a; --muted: #5d5a53;
    --line: #e2dfd8; --accent: #b4530a; --good: #1f7a3a; --bad: #b3261e;
  }
  @media (prefers-color-scheme: dark) {
    :root {
      --bg: #141413; --panel: #1c1b19; --fg: #eeebe4; --muted: #a8a49b;
      --line: #34322e; --accent: #f0a35e; --good: #7fcf8f; --bad: #f08a80;
    }
  }
  * { box-sizing: border-box; }
  body { margin: 0; background: var(--bg); color: var(--fg);
         font: 16px/1.55 system-ui, sans-serif; }
  main { max-width: 960px; margin: 0 auto; padding: 32px 24px 48px; }
  h1 { font-size: 28px; line-height: 1.2; margin: 0 0 8px; }
  .lede { font-size: 18px; color: var(--muted); margin: 0 0 28px; }
  h2 { font-size: 18px; margin: 32px 0 12px; }
  .card { background: var(--panel); border: 1px solid var(--line);
          border-radius: 10px; padding: 16px 20px; }
  .scroll { overflow-x: auto; }
  table { border-collapse: collapse; width: 100%; }
  th, td { text-align: left; padding: 8px 10px; border-bottom: 1px solid var(--line); }
  th { font-weight: 600; color: var(--muted); font-size: 14px; }
  td.num, th.num { text-align: right; font-variant-numeric: tabular-nums; }
  code { font-family: ui-monospace, monospace; font-size: 14px; }
  .source { color: var(--muted); font-size: 14px; margin-top: 32px; }
</style>
</head>
<body>
<main>
  <h1>Crew cost by task</h1>
  <p class="lede">One sentence with the answer.</p>
  <h2>By task</h2>
  <div class="card scroll">
    <table>
      <thead><tr><th>Task</th><th class="num">Cost</th></tr></thead>
      <tbody><tr><td><code>task-id</code></td><td class="num">$0.00</td></tr></tbody>
    </table>
  </div>
  <p class="source">From <code>spend.jsonl</code> in this session, read today.</p>
</main>
</body>
</html>
```

## After you show it

Tell the user in one line what the page shows, and give its path. Don't repeat the page's content in the chat.
