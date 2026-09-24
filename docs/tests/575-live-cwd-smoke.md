# 575 smoke test: splits follow the shell's live cwd

Checks for [#575](../features/575-live-cwd.md) on a build of this branch. Automated coverage runs zsh 5.9 and `/bin/bash` 3.2 in a temporary `HOME` and never your own startup files. These checks are the ones only a real session shows. About ten minutes.

## Automated evidence

```sh
cargo test --locked -p runner-terminal --profile ci osc7
cargo test --locked -p runner-backend --profile ci shell_integration
cargo test --locked -p runner-backend --profile ci only_terminal_shells_get_the_osc7_integration
cargo test --locked -p runner-app --profile ci a_shell_started_from_a_session
```

## Human checks, macOS

Setup: a terminal tab (sidebar `+` → New terminal) in the runner project.

1. **Split follows `cd`.** `cd crates/runner-app`, then `⌘D`: the new shell's `pwd` is `…/crates/runner-app`. `⇧⌘D` does the same downwards.
2. **New terminal follows too.** With the terminal tab active, palette → New terminal splits right in the focused shell's directory.
3. **Spaces and Chinese.** `mkdir -p "/tmp/575 测试/子目录" && cd "/tmp/575 测试/子目录"`, then split: `pwd` matches exactly.
4. **Deleted directory.** `mkdir /tmp/575-gone && cd /tmp/575-gone && rmdir /tmp/575-gone`, then split: the shell opens at the tab's original directory, with no error banner.
5. **Your config is untouched.** In a new terminal your prompt, aliases, history and plugins behave as before. `echo $ZDOTDIR` shows your own value, or nothing. `env | grep RUNNER_` shows no `RUNNER_ZSH_ZDOTDIR`.
6. **No double hook.** `echo $precmd_functions`. If your zsh already sends OSC 7 (oh-my-zsh's `termsupport` does under `xterm-256color`), `__runner_osc7` is not in the list and splits still follow. If it is in the list, your config sends none, and Runner's hook is the one reporting.
7. **Empty host.** `__runner_osc7 | cat -v` prints `^[]7;file:///…^[\` with three slashes and no hostname, so a hostname that changes with the network cannot turn later reports into foreign ones.
8. **Nested shell.** Run `zsh`, `cd /tmp`, `exit`: the inner shell loads your config normally. Splitting from the outer shell afterwards opens where the outer shell last reported.
9. **File links.** `cd crates/runner-terminal/src && ls`, then ⌘-click `terminal.rs`: it opens. Scroll up to output printed before the `cd` (for example `git status` at the repository root) and ⌘-click a path there: it still opens.
10. **Drawer unchanged.** In a chat tab, open the drawer, `cd /tmp` in its shell, press `+`: the new chip opens at the chat's directory, not `/tmp`.
11. **`ssh` (optional).** `ssh` to a machine that sends OSC 7, `cd` there, split: the new local shell opens where you ran `ssh`.
12. **Relaunch.** Quit with a split open that was made after a `cd`, relaunch: the split comes back in the directory it opened in.
13. **bash (optional).** Quit Runner and start the build from a terminal with `SHELL=/bin/bash` in its environment. Open a terminal tab: your bash profile loads as before, and `echo "$PROMPT_COMMAND"` ends in `__runner_osc7` (or lacks it if your bash config sends OSC 7). Checks 1 and 3 pass.

## Not covered here

- Windows has no injected hook in this version. `docs/arch/windows.md` carries an unverified PowerShell snippet, and the injected hook is a follow-up.
- Agent chats are never scanned or injected. A Claude Code or Codex chat starts exactly as before.
