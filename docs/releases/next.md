# Next

Unreleased: what is in `dev` since 0.23.0. These notes become the next release's `vX.Y.Z.md`.

## What changed

- **Select text with the mouse.** Hold the left button and drag: what you drag over is highlighted, and letting go copies it to the clipboard. The last row says `copied 9 lines` for a moment. A click without a drag copies nothing, and clicks do what they did.
- **In the conversation the selection is the text, not the screen.** The timeline beside it is never in it. The highlight stays on the words while a reply streams in or the pane scrolls, and holding the pointer above or below the pane scrolls it and selects on, so more than a screenful can be copied.
- **A code block is copied as code:** without its box or its line numbers, with its own indentation.
- **`Alt` while dragging selects a rectangle.**
- **The message box, the cards and an open panel can be selected too,** each kept to the part of the screen the drag began in.
- **Where the copy goes:** the terminal is asked by OSC 52, which also works over `ssh`, and your desktop's tool is run as well when it is installed (`wl-copy`, `xclip` or `xsel`, `pbcopy`).

Guide: "Selecting text".

## Why

To copy a command or a block of code out of a reply you had to hold your terminal's modifier, and what came out had the timeline, the box around the code and its line numbers in it. The user's words: "hold click to highlight a section and releasing copies the text to clipboard."

## Good to know

- Your terminal's own selection still works with its modifier (Shift+drag on most), and `[ui] mouse = false` turns Ryter's handling of the mouse off.
- If nothing reaches the clipboard: your terminal ignores OSC 52 and none of the tools above is installed. Inside `tmux`, OSC 52 needs `set -g set-clipboard on`.
- Text that wrapped on the screen is copied one line per row you saw.

## Tried before release

- **Driven in the real binary,** offline against the simulated provider, with the mouse reports a terminal sends and a stand-in `wl-copy`: a sentence into a code block (eight lines, no frame), a click (nothing copied), `Alt`+drag, the pointer held above a sixty-line reply until the pane reached its top (63 lines copied), and the message box.
- **Not yet tried:**
  - By a person, in a real terminal emulator, with a real clipboard.
  - Anything on a Mac.
