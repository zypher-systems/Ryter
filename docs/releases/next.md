# Next

Unreleased: what is in `dev` since 0.22.0. These notes become the next release's `vX.Y.Z.md`.

## What changed

- **The build hat can ask to run a command as root.** When the work needs a system package or another command as root, the model runs `sudo <command>` and you are asked, twice over. First a card, "run as root", showing the whole command: only `y` runs it, there is no "always", and `/tools always`, `--always-approve`, your `[permissions]` rules and `/yolo` do not answer it. Then sudo's own question for your password, in a panel in Ryter. What you type is shown as dots and goes to sudo only: not to the model, not into the conversation, not to the session's log or to disk.
- **The password can be kept for five minutes.** That is the default, in memory only, so a second root command shows its card and runs. `Tab` on the panel turns it off. A password sudo turns down is dropped and asked for again, and the panel says so.
- **Root opens nothing that was refused.** The command after `sudo` is judged as the build hat's own first: `sudo bash -c …`, `sudo systemctl …`, `sudo chown …`, a secret file and a write to `/etc` by name are refused as before. Root reads nothing your account can't: a root command that names a file or folder you may not open is refused (`/etc/shadow`, root's folder, a disk in `/dev`), and so is one whose files can't be seen before it runs (`sudo cat $F`, `sudo xargs cat`, `sudo find … -exec`). An interpreter as root is refused (`sudo python3 x.py`, `sudo sh install.sh`); a program of the project's own runs (`sudo ./install.sh`, `sudo make install`). Only the plain spelling is read; sudo's own options, a wrapper in front of it, and `su`, `doas` and `pkexec` stay refused, and the refusal tells the model the spelling that works.
- **Where it is not offered.** Plan, scribe and audit; headless runs; `.ryter/run.toml`; and any sandbox profile but `off`, under which the system stops `sudo` from gaining privilege. In each the model is told to name the command for you.
- **A desktop's own askpass no longer answers for a command Ryter runs.** `SUDO_ASKPASS` is taken out of every command's environment except an approved root command, where it names Ryter's.

Guide: "As root".

## Why

On a real run a Rust project needed the ALSA development headers. `sudo dnf install` was refused, as every `sudo` was, and the model did what models do with a wall: it spent fifteen calls on workarounds, then restructured the project so the part that needed the package was never built. The plan was no longer the plan, and nothing on screen had asked. The user's words: "if it wants to run a command with sudo and I am ok with it I should be given a prompt to type in sudo credentials."

## Good to know

- Nothing is installed outside `~/.ryter`: a `sudo` wrapper and a `ryter-askpass` link in `~/.ryter/bin`, used only for a root command you approved. Ryter itself never runs as root.
- What a root command changes, `/undo` does not put back.
- A wrong password counts against your account as it would in a terminal; where `pam_faillock` is on, a cancelled panel can count too.
- Linux and macOS.

## Tried before release

- The real TUI, end to end, with a stand-in `sudo` that calls the askpass as `sudo -A` does and never authenticates: the card, `y`, a password turned down and the panel saying so, a second accepted, the next root command answered from memory, `sudo -n` refused with its hint, a card denied. The typed password was in no request to the model, no event of the session's log and no file under the Ryter home.
- Not tried by Ryter's own checks: a real `sudo` authentication.
