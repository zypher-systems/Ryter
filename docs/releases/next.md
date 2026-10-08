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

Smaller:

- **An audit's verdict and its findings agree.** A verdict of pass is no longer taken over a finding that failed; the auditor is told to make it a fail, or the finding a pass with what it saw. An audit read PASS above "1 of 5 failed" on a real run, and offered no repair.
- **An empty folder says it is empty.** `list_dir` and `glob` answered with nothing at all, which a model took for a listing that had failed.
- **`/rename`, `/budget` and `/settings` open their fields on what they hold,** as `/provider` has since 0.22.0: the title, the amount, the name are there to edit.
- **The reason on a question's card is the model's latest words.** The card could open before the screen had caught up, and give as the reason what the model had said a response or two earlier.

## Why

To copy a command or a block of code out of a reply you had to hold your terminal's modifier, and what came out had the timeline, the box around the code and its line numbers in it. The user's words: "hold click to highlight a section and releasing copies the text to clipboard."

## Good to know

- Your terminal's own selection still works with its modifier (Shift+drag on most), and `[ui] mouse = false` turns Ryter's handling of the mouse off.
- If nothing reaches the clipboard: your terminal ignores OSC 52 and none of the tools above is installed. Inside `tmux`, OSC 52 needs `set -g set-clipboard on`.
- Text that wrapped on the screen is copied one line per row you saw.

## Tried before release

- **Driven in the real binary,** offline against the simulated provider, with the mouse reports a terminal sends and a stand-in `wl-copy`: a sentence into a code block (eight lines, no frame), a click (nothing copied), `Alt`+drag, the pointer held above a sixty-line reply until the pane reached its top (63 lines copied), and the message box.
- **The card's reason:** the same four-card run that showed the stale reason, three times over, with the right one each time.
- **Not yet tried:**
  - By a person, in a real terminal emulator, with a real clipboard.
  - Anything on a Mac.
