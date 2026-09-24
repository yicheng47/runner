# 575 — Splits and new terminals follow the shell's live cwd

> Tracking issue: [#575](https://github.com/yicheng47/runner/issues/575)
> Priority: P3, milestone 0.14. Depends on [#574](./archive/574-terminal-tab-split.md), shipped.
> Brief: [`docs/impls/briefs/575-live-cwd.md`](../impls/briefs/575-live-cwd.md). Platforms: macOS for the injected shell integration; the parser and the fallback are shared with Windows.

## Motivation

[574](./archive/574-terminal-tab-split.md) made a terminal-tab split spawn a shell in the directory the split-from pane's shell was *spawned* in. After `cd crates/runner-app` a split still opens at the tab's original directory. Ghostty's split opens where you are: the shell reports every directory change with OSC 7 (`ESC ] 7 ; file://host/path ST`), and Ghostty injects the hook that sends it. Runner should do the same for its terminals.

## Behavior

- **A terminal split opens where the shell is.** Split Right, Split Down, and New terminal on a terminal tab start the new shell in the split-from (or focused) shell's live cwd: the last directory that shell reported through OSC 7, when it still exists. Otherwise they start in that shell's spawn cwd, exactly as today, and then the existing chain: project directory → `settings.default_working_dir` → `$HOME`.
- **Runner's zsh and bash report their directory without any setup.** Runner injects a small prompt hook at spawn. The user's own startup files load unchanged, a shell whose configuration already sends OSC 7 keeps its own hook and Runner's stands down, and nothing is written to the user's rc files.
- **Relative file links follow the shell too.** ⌘-click on `lib.rs:12` resolves against the live cwd first and falls back to the spawn cwd, so output printed after a `cd` and output printed before it both resolve.
- Nothing changes in the UI, in chat sessions, or in the terminal drawer's cwd.

## Decisions

### Parsing

- **Where.** `vte` 0.15 routes OSC 7 to its unhandled branch and drops it (`osc_dispatch`, `vte-0.15.0/src/ansi.rs`; [586](./586-shell-status-detection.md) found the same for OSC 133). `TerminalSession::feed_output` therefore scans the raw chunk before handing it to the parser, beside the existing DEC 2031 scan. The scan runs only for shell-runtime sessions; agent sessions never scan, so their output path is unchanged.
- **Scanner.** A small state machine per session: it looks for the ESC bytes in the chunk, recognizes the `ESC ] 7 ;` introducer, and collects the payload up to BEL (`0x07`) or ST (`ESC \`). An introducer or a terminator split across two PTY reads is carried to the next chunk: at most three bytes of a partial introducer, or the unterminated payload. ESC followed by anything but `\`, CAN and SUB abort the sequence, as they do in `vte`. A payload over 16 KiB is abandoned. `0x9C` (C1 ST) is not a terminator: in UTF-8 output it is a continuation byte, and a raw CJK path can contain it. The byte work per chunk is one pass looking for ESC; parsing happens only when a report completes, which is once per prompt.
- **Report.** `file://host/path`, scheme case-insensitive. The host must be empty, `localhost`, or this machine's hostname (`gethostname`, `COMPUTERNAME` on Windows) or its first label, compared ASCII case-insensitively; zsh's `$HOST` and bash's `$HOSTNAME` both come from `gethostname`, so a user's own hook that sends `Jasons-Mac-Studio.local` matches while the hostname stays the same. The path is cut at `?` or `#` as a URI's path is, percent-decoded, and must be absolute; on Windows `/C:/…` becomes `C:/…`. Anything else is ignored: another scheme, a foreign host (an `ssh` session's remote shell), a relative path, invalid UTF-8 after decoding, garbage. An ignored report keeps the previous live cwd, so splitting while `ssh`'d elsewhere opens a local shell where you ran `ssh`, as Ghostty does.
- **Storage.** The live cwd is an in-memory `Option<PathBuf>` on the `TerminalSession`, beside the spawn cwd on the session row, which stays the fallback. It is not checked on arrival: `TerminalSession::live_cwd()` returns it only if it is a directory at the moment of reading, so a directory removed since the report falls back to the spawn cwd instead of failing the split. A respawned shell gets a fresh `TerminalSession` and starts without a live cwd.

### Where it applies

The rule: **a new shell starts where the session it is created from is.** The live cwd is consulted only for a shell session that has reported one; everything else keeps today's cwd.

- **Split Right / Split Down on a terminal tab** (`split_pane`): the split-from pane's shell.
- **New terminal on a terminal tab**: its split path is the same `split_pane`; when it fills an empty pane instead, `terminal_start_location` uses the focused pane's shell the same way.
- **Terminal drawer** (chat tab and mission): unchanged. A drawer shell is created from its tab's focused chat or from the mission ([469](./archive/469-terminal-drawer.md) anchors it there), and a chat's cwd does not move: agents are never scanned, and following cwd for chat sessions is a non-goal. A drawer shell that `cd`s elsewhere does not pull the next `+` away from the chat's repository. The one case where the drawer does follow a shell is the same rule applied through `terminal_start_location`: a chat tab whose focused pane is a legacy shell pane ([469](./archive/469-terminal-drawer.md)'s migration note).
- **New terminal tab from the sidebar**: unchanged; there is no source session.
- **Relative file links** (`link_at`): the live cwd first, then `link_cwd`'s spawn or project cwd. `resolve_file_candidate` already requires the joined path to exist, so a link printed before a `cd` fails the first candidate and resolves against the second. A relative name that exists in both directories resolves against the live one; per-line cwd tracking, as iTerm2 does it, is out of v1.
- **Relaunch.** A split's session row records the cwd it spawned in, which is now the live cwd, so relaunch brings each split back where it opened. The live cwd itself is not persisted, and a shell that relaunches or restarts comes back at its recorded spawn cwd as today.

### Shell integration

Runner injects only into its own terminal shells: runtime-only `shell` sessions from `session_start_shell` and their resumes. `apply_runtime_args` gates on the role's id, which is `runtime:shell` (`runtime_direct_role`) for exactly those sessions, and on `mission == false`. The runtime alone is not the gate. Agent sessions, role-backed shell chats and mission slots are untouched. Injection is Unix-only, chosen by the command's basename (`zsh`, `bash`). **Argv never changes; the injection only adds environment variables.** The scripts are embedded in the binary and written to `<app_data>/shell-integration/` at spawn, compared first and replaced through a temporary file and a rename. If writing fails, or the app data directory is not absolute, the shell spawns without injection and Runner logs a warning in the first case. A relative `ZDOTDIR` would resolve against the shell's cwd, which is why the second case is excluded.

**zsh: a `ZDOTDIR` wrapper, Ghostty's pattern.** Runner sets `ZDOTDIR=<app_data>/shell-integration/zsh` and, only when the user's environment carries one, `RUNNER_ZSH_ZDOTDIR=<their ZDOTDIR>`. zsh reads that directory's `.zshenv` first; it immediately restores the user's `ZDOTDIR`, or unsets it, and unsets `RUNNER_ZSH_ZDOTDIR`, then sources the user's own `.zshenv`. zsh then reads `.zprofile`, `.zshrc` and `.zlogin` from the user's `ZDOTDIR` by itself, in its own order, and a nested or `exec`'d zsh sees the user's environment only. The same file adds `__runner_osc7` to `precmd_functions` once.

**bash: a `PROMPT_COMMAND` bootstrap.** Runner sets `RUNNER_BASH_INTEGRATION=<app_data>/shell-integration/bash/runner.bash` and appends `. "$RUNNER_BASH_INTEGRATION"` to `PROMPT_COMMAND` in the spawn environment. bash starts as the same `bash -l` it is today and reads `/etc/profile` and the first of `~/.bash_profile`, `~/.bash_login` and `~/.profile` itself. At the first prompt, after all of that, the bootstrap sources the script once. The script replaces the bootstrap with `__runner_osc7` in `PROMPT_COMMAND`, whether it is a string or a bash 5.1 array, stops exporting `PROMPT_COMMAND`, unsets `RUNNER_BASH_INTEGRATION`, and reports the first directory. `$?` is preserved for the prompt and for any hook that runs after it.

Ghostty's bash approach (`--posix` plus `ENV`, with the script replaying the startup files) was considered and rejected. Apple's `/bin/bash` 3.2 ignores `ENV` in `--posix` mode and reads `~/.bash_profile` normally: probed, and Ghostty skips `/bin/bash` on macOS for this reason. That is the bash a macOS user most likely has. `--rcfile` does not work either: a login shell ignores it, and dropping `-l` would change `login_shell` for the user's own scripts. If the user's rc assigns `PROMPT_COMMAND` outright instead of extending it, the bootstrap is lost and that shell falls back to the spawn cwd. There is no error.

**The hook.** The hook percent-encodes every byte outside `A-Za-z0-9/._~-` (`sub dir/中文` → `sub%20dir/%E4%B8%AD%E6%96%87`) and prints `ESC ] 7 ; file://$dir ESC \` on every prompt. The host is left empty, where oh-my-zsh and Ghostty send `$HOST`: Runner's hook only ever runs in a local shell, and `$HOST` or `$HOSTNAME`, read once when the shell starts, goes stale when a laptop's hostname follows the network, which would turn every later report into a foreign one. It uses builtins only: `printf -v`, no command substitution in the steady state, and `emulate -L zsh` in zsh.

**No double hooks.** When the user's configuration already sends OSC 7, Runner's hook removes itself, and the shell ends with the user's hook only. The check is transitive:

- **Roots.** zsh: `precmd`, `chpwd`, `precmd_functions`, `chpwd_functions`, and the words of `PS1`/`RPS1`. bash: the words of `PROMPT_COMMAND` (string or array), bash-preexec's `precmd_functions`, and `PS1`.
- **Search.** Every function reachable from the roots is checked breadth-first, for a body containing `]7;`: a word in a body that names a defined function is followed. The body comes from `$functions[f]` in zsh; an autoload stub is loaded first with `autoload +X`, which defines the function without running it. In bash it comes from `declare -f`. Words are split on IFS set to whitespace plus shell punctuation, which is C-speed. zsh's `${//}` pattern substitution was 10× slower in the probe.
- **When.** The hook keeps two signatures: the hook lists (`precmd_functions`/`chpwd_functions` in zsh, `PROMPT_COMMAND`/`precmd_functions` in bash) and the prompt strings (`PS1`/`RPS1` in zsh, `PS1` in bash). A changed hook list, including at the first prompt, clears the cache of functions already found clean and rescans every root. A change to the prompt strings alone rescans only their words and skips cached functions. That catches a plugin that adds its hook after the first prompt, and a prompt set to an emitter later. It also keeps prompts that rebuild `PS1` on every `cd` (liquidprompt, `__git_ps1`) from paying for a full rescan each time. When neither changes, the cost is two string comparisons: about 22 µs per prompt for the whole zsh hook, measured. A prompt-only rescan on the heavy configuration below took 0.17 ms.
- **Removal.** zsh drops `__runner_osc7` from `precmd_functions`. bash drops it from `PROMPT_COMMAND` together with the separator beside it (`\n`, `; `, `;`), or leaves `:` in its place if it sits somewhere unusual.
- **Bounds.** zsh stops after 256 functions or 512 KiB of source: 37 ms on a synthetic 4.4 MB configuration of 800 chained functions, 0.25 ms on a plain one. bash stops after 64 functions (each `declare -f` is one fork) or 20,000 words. Past the bound, the answer is "not sent", and Runner's hook stays.

Probed with a direct hook, a helper-function hook, an autoloaded hook, a `PS1` that calls a helper, a bash-preexec-style `precmd_functions`, an emitter added to the hook list after the first prompt, and `PS1` set after the first prompt to a helper call or to a literal OSC 7. Each gave exactly one report per prompt, the user's. By structure, this covers oh-my-zsh's `termsupport` (a direct hook, active under `TERM=xterm-256color`), Apple's `update_terminal_cwd` (direct; loaded when `TERM_PROGRAM=Apple_Terminal` leaks in from a Terminal.app launch), vte.sh's `__vte_prompt_command` → `$(__vte_osc7)` (a helper) and WezTerm's `__wezterm_osc7` (in `precmd_functions`). The real scripts were not run.

What stays undetectable from inside the shell: an external script that prints the sequence, a sequence assembled at run time (`printf '\e]%d;' 7`), and anything past the bound. There, one extra report per prompt remains. The parser keeps the last report, and both describe the same directory. Runner's own hook installs once per shell, re-sourcing `~/.zshrc` does not add it again, and nested shells get no injection.

In bash, Runner's hook takes the bootstrap's place in `PROMPT_COMMAND`, and it preserves `$?`. A user hook placed after it that reads `PIPESTATUS` sees one element instead of the pipeline's. Starship and bash-preexec both put their hooks first, so they are unaffected. zsh restores `$?` and `$pipestatus` for every precmd hook (probed), so order does not matter there.

**Other shells** (fish, nu, sh, pwsh on macOS) get no injection. If a shell sends OSC 7 by itself, the report is used; otherwise the shell falls back to its spawn cwd.

**Windows: a documented follow-up, not v1.** PowerShell needs a `prompt` wrapper, injected through `-NoExit -Command` the way VS Code does it, that coexists with oh-my-posh and starship prompt functions, and native Windows is not available to this mission to verify it. The parser accepts `file://HOST/C:/…`, so a PowerShell profile that sends OSC 7 (oh-my-posh's `pwd: osc7`, or the one-line `prompt` snippet `docs/arch/windows.md` will carry) already works. A follow-up issue should cover the injected hook and Windows Terminal's OSC 9;9.

## Out of v1

- Showing the live cwd anywhere in the UI (identity line, sidebar row, side panel). It needs a design first.
- Following cwd for chat sessions, or scanning agent output at all.
- Persisting the live cwd across relaunch, or restarting an exited shell at its last live cwd. The `TerminalSession` and its live cwd go away when the shell exits.
- `kitty-shell-cwd://`, OSC 9;9 and OSC 1337 `CurrentDir`: other cwd reports, none of which Runner's injected hooks send.
- OSC 133 prompt marks. [586](./586-shell-status-detection.md)'s semantic phase can reuse this injection when it lands.

## Open items

- File the Windows follow-up (PowerShell prompt hook, OSC 9;9) once Jason agrees with the split.
- When both the live cwd and the spawn cwd are gone, the split still fails with "working directory does not exist", as it does since 574. The 64 relaunch rule (nearest existing ancestor → project → `$HOME`) could extend to splits; left as is to keep scope.

## Verification

- `runner-terminal`: the scanner on BEL and ST, an introducer and a terminator split at every byte offset across two chunks, two reports in one chunk (the last wins), an ESC abort, CAN/SUB, an overlong payload, and garbage. The report parser on percent-encoding, empty host and `localhost`, this host and its first label, a foreign host, another scheme, a relative path, `?`/`#`, and the Windows drive form. `live_cwd()` from fed output, `None` for an agent session, `None` once the directory is removed, and a foreign report keeping the previous value. Relative links resolving against the live cwd, then the spawn cwd.
- `runner-backend`: the injected environment and unchanged argv for zsh and bash, including a user `ZDOTDIR` and an inherited `PROMPT_COMMAND`; no change for fish, a role-backed shell, a mission shell or an agent runtime; script files written and rewritten when stale. Real shells: `/bin/zsh` and `/bin/bash` spawned through the PTY runtime with a temporary `HOME` (and a `ZDOTDIR` case), `cd` into a directory with a space and CJK characters, asserting the OSC 7 arrives, percent-encoded, and that the user's startup files printed their markers. For both shells, an rc that already sends OSC 7 directly, through a helper function, from `PS1`, or (zsh) through an autoloaded function, or that adds its emitter to the hook list or to `PS1` after the first prompt, gets exactly one report per prompt, and Runner's hook is gone from the hook list.
- `runner-app`: the cwd choice for split and New terminal: a live cwd that exists wins; a missing, removed or non-directory live cwd falls back to the spawn cwd, then the existing chain.
- The six gates in the brief, with exit codes.
- Jason's smoke test: [`docs/tests/575-live-cwd-smoke.md`](../tests/575-live-cwd-smoke.md).

## References

- Runner: `crates/runner-terminal/src/terminal.rs` (`feed_output`, `scan_scheme_sequences`, `link_at`, `file_target_from_uri`); `crates/runner-app/src/surfaces/chat.rs` (`split_pane`) and `surfaces/start_chat.rs` (`new_terminal`, `terminal_start_location`, `terminal_working_dir`); `crates/runner-backend/src/session/manager/spawn.rs` (`apply_runtime_args`), `ops/session.rs` (`session_start_shell`), `shell_path.rs` (`shell_login_args`).
- Ghostty: `src/shell-integration/zsh/.zshenv` and `src/termio/shell_integration.zig` (the `/bin/bash` exclusion on macOS). oh-my-zsh `lib/termsupport.zsh` (`omz_termsupport_cwd`).
