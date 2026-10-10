# #855 — Option+arrow word navigation

Candidate: `fix/855-option-arrow-word-navigation`, based on `c169ac8a`; automated checks on macOS arm64, 2026-10-10. Scope: encoder behavior and bounded shell PTYs only; no app or real agent was launched.

## Choice and boundaries

For legacy input on macOS, plain Option+Left/Right now sends `ESC b` / `ESC f`, the existing Meta word-navigation commands. This fixes zsh/bash without changing startup files or requiring Runner shell integration, including nested shells using those bindings. Ctrl/Shift combinations, other Alt-modified special keys, unmodified arrows and application-cursor mode keep their existing encodings. Windows/Linux retain xterm Alt+arrow sequences. No runtime-name dispatch is involved, and the double-Escape encoding from before `1453c6bd` is not restored.

Any negotiated kitty keyboard flag takes precedence over the new mapping: Option+arrows continue to send `CSI 1;3D/C`, including in application-cursor mode, while Alt+b/f letters retain their CSI-u encoding under kitty disambiguation. Applications such as pi therefore receive arrow events rather than ambiguous legacy Meta bytes. Custom shell/editor keymaps can override Meta-b/f; the shell checks cover Emacs editing mode, not a guarantee of word movement in every custom or vi keymap.

## Selected checks (before execution)

| Check | Connection to this diff |
| --- | --- |
| Encoder matrix | Both directions, all eight Ctrl/Alt/Shift combinations, normal/application cursor modes, each kitty flag individually and all flags together; ensures only plain macOS legacy Option+arrows change and remain navigation input. Existing special-key and kitty tables cover other keys. |
| Real zsh/bash PTYs | Temporary home/startup files, with/without integration, both cursor modes. Zsh reports the actual unsent buffer and cursor after each of five left and five right presses, including boundaries, through a test-only Ctrl+O widget. Bash 3.2 verifies marker placement after four left and four right presses in a safe quoted `printf` draft, then deliberately submits it and checks the exact output. These assertions expose literal C/D or [C/[D] insertion. |
| Full runner-terminal tests | Exercise the changed encoder plus existing parser, terminal-mode and recording checks. |
| Focused daemon shell tests | Run the reused PTY fixture and ensure its optional-integration setup preserves existing shell startup tests; no daemon production behavior changes. |
| Workspace and updater Clippy, formatting, whitespace | Required cross-crate/feature compilation and style gates. Mac-only helpers stay inside the existing Unix-gated test module. Windows encoder assertions run in CI. |
| Native feature check (pending) | No maintained case exactly covers Option+arrows. In zsh, bash, Codex and Claude Code, type `alpha bravo charlie delta`, press Option+Left repeatedly then Option+Right repeatedly; capture the intact unsent draft and cursor positions, with no inserted characters or submission. |
| TERM-INPUT-02 / pi (pending, navigation portion) | Confirm repeated Option+Left/Right and Alt+B still navigate an unsent draft under negotiated kitty. Multiline submission and unrelated lifecycle/IME checks are outside this fix's scope. |

## Agent compatibility evidence

These are primary-source parser/keymap evidence, not native passes or checks of Jason's installed agent versions:

- [Crossterm parser at `101fa5da`](https://github.com/crossterm-rs/crossterm/blob/101fa5dabd409fd02a4468cd997bfe28cde64073/src/event/sys/unix/parse.rs#L77) parses an Escape followed by a character as that character with the Alt modifier. [Codex default editor keymap at `c3d3b142`](https://github.com/openai/codex/blob/c3d3b142d10f4316b46e35aad7e5317e7e506cb7/codex-rs/tui/src/keymap.rs#L1705) binds Alt+b/f to backward/forward word movement, alongside Alt+arrows. Together these support compatibility of the new legacy bytes with Codex defaults.
- [Ink parser at `26d2c3f8`](https://github.com/vadimdemedes/ink/blob/26d2c3f83008142061c22267482489588cc3823c/src/parse-keypress.ts#L535) identifies `ESC b/f` as the corresponding character with `meta=true`. [Claude Code's text-editing reference](https://code.claude.com/docs/en/interactive-mode#text-editing) documents Alt+B/F for word movement. This supports Claude Code compatibility without launching it or probing accounts.
- [Kitty protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/#functional-key-definitions) defines the modified arrow forms; the negotiated path and its assertions retain those bytes.

## Commands and results

| Command | Result |
| --- | --- |
| `cargo test --locked -p runner-terminal --profile ci` | Passed: 101 tests, 1 pre-existing ignored test; no failures. |
| `cargo test --locked -p runner-daemon --profile ci shell_integration::` | Passed: 17 tests, no failures; 8 Option+arrow shell/mode/integration variants. |
| `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` | Passed, exit 0 on the final diff. |
| `cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings` | Passed, exit 0 on the final diff. |
| `cargo fmt --all --check` | Passed after formatting. |
| `git diff --check` | Passed. |

Fixture development retained three failed attempts with the same focused shell-test command: the first expected bash's word-end positions in zsh and used a Ctrl+X snapshot binding that did not fire in bash 3.2; the second sent Ctrl+O before the line editors were ready; the third passed zsh but bash's snapshot still did not fire. The final fixture waits for the prompt and draft echo, uses zsh's actual word-start positions, and checks bash through safe `printf` output. Final focused run: 17/17 passed. These were fixture failures, not native validation results.

Negative control: temporarily disabled the new encoder branch and ran `cargo test --locked -p runner-daemon --profile ci option_arrows_move_by_word -- --nocapture`. Both new tests failed as expected (0 passed, 2 failed): zsh captured cursor 28 and draft `alpha bravo charlie delta;3D` after the first left press; bash received repeated `;3D`/`;3C` text and failed the exact marker-output assertion. This confirms the tests reject the previous wire encoding. The fix was restored before final checks.

Shell binaries: `/bin/zsh` 5.9 and `/bin/bash` 3.2.57. Temporary fixtures and their PTY sessions are stopped by the test fixture's `Drop`; no production configuration is written. Native checks above remain pending for Jason. This is focused feature validation, not a live regression pass.

Working-tree review: reviewer reported `NO REMAINING MUST-FIX ISSUES` through Runner on 2026-10-10, independently verified the agent compatibility sources, and reran the terminal tests (101 passed, 1 ignored), focused shell tests (17 passed), and whitespace check. No reviewer edits. This authorizes the brief's commit/PR/CI phase; it does not establish a native pass.
