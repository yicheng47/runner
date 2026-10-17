# 941 — pi session titles are the literal `π`

Fix [#941](https://github.com/yicheng47/runner/issues/941). Jason requested this mission on 2026-10-17. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-941-pi-session-title`, on the existing branch `fix/941-pi-session-title`, created from `origin/main` at `045ead84`. The root checkout stays on main. Do not create another branch or worktree, and do not share a Cargo target directory.

## The bug

Runner titles chats from the terminal OSC title (`provider_title()`), stripping each CLI's own chrome. `decoration()` in `crates/runner-core/src/protocol/session_title.rs` knows `codex`, `claude`, `copilot`, `gemini`, `grok`, `opencode`, and the cwd basename, but not pi's app prefix.

pi sets its title to `π - <sessionName> - <cwdBasename>`, or `π - <cwdBasename>` when the session has no name (`updateTerminalTitle()` in pi's `dist/modes/interactive/interactive-mode.js`; `APP_TITLE` is `π`, or the configured package name when `piConfigName` is set). So:

- An unnamed pi session, the common case for Runner-spawned chats, yields the title `π` instead of no title and the role handle fallback. Live sessions on the dev daemon confirm it: `media-manager` and `music-curator` both report `live_title: "π"`.
- A named pi session (`pi --name`, `/name`, `pi.setSessionName()`) yields `π - <name>` instead of `<name>`.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions.
- Issue #941.
- `crates/runner-core/src/protocol/session_title.rs`: `provider_title()`, `decoration()`, and the existing runtime tests (`copilot_titles_keep_only_the_topic`, `claude_spinner_frames_leave_the_topic_unchanged`, `grok_status_titles_carry_no_topic`, `codex_ascii_spinner_frames_leave_the_topic_unchanged`).
- The call sites, to confirm one shared parser serves them all: `crates/runner-terminal/src/terminal/model.rs:190`, `crates/runner-daemon/src/session/manager/terminal.rs:177`, `crates/runner-core/src/protocol/session.rs:106`, `crates/runner-app/src/surfaces/mission_workspace/feed.rs:30`.
- `crates/runner-daemon/src/runtimes/pi/mod.rs`: `pi_defaults()` and `launch_args()`, to see that Runner does not pass `--name` (out of scope here).

## Deliverables

1. **Recognize pi's prefix.** Add `π` and `pi` to the decoration vocabulary in `provider_title()` (or the equivalent minimal change) so that:
   - `π - <cwdBasename>` → `None` (Runner falls back to the role handle, as it does for the other runtimes' bare app titles);
   - `π - <name> - <cwdBasename>` → `Some("<name>")`;
   - `π - <name>` without a cwd → `Some("<name>")`;
   - `π` alone → `None`.
   Keep the existing behavior for every other runtime bit-for-bit; do not loosen the decoration match into something that strips real topic words.
2. **Tests.** Extend the module's existing tests with a pi case covering the four forms above (use a cwd like `/repos/runner` so the basename trimming is exercised), plus a named session whose name coincides with nothing in the chrome list. Add a lowercase `pi` form for the configured-package-name case. Keep the assertions in the file's existing style.
3. **Scope.** No `--name` plumbing on spawn, no change to the pi adapter, no title-generation redesign, no UI change. If the guide documents session titles and states the old rule, update it; otherwise leave `docs/guide/` alone. Do not add a feature spec.

## Boundaries

Crews never run the dev app or drive Jason's Runner for UI checks; Jason smoke-tests. Do not start extra agents, crews or subagents. Do not touch pi's global settings or the user's pi configuration.

## Review, verification and authorization

The coder owns implementation and checks. The reviewer waits for an explicit Runner handoff, then reviews the full branch diff against #941 with must-fix findings first and file:line pointers. Focus on: only the intended titles changing (no other runtime regressed), `π` handled without depending on a terminal locale, and tests that would fail if the decoration entry were removed. Iterate until the reviewer posts `NO REMAINING MUST-FIX ISSUES`.

Run `cargo test --locked -p runner-core --profile ci`, workspace Clippy with warnings denied (`cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`), macOS updater Clippy (`--features updater`), `cargo fmt --all --check`, and `git diff --check`. Add `cargo test --locked -p runner-terminal --profile ci` and `-p runner-app` only if their call sites or tests need it. Record the exact commands and exit codes. Windows CI has repeatedly failed on imports or helpers used only by `cfg(unix)` tests; gate any such import or helper with `#[cfg(unix)]`.

After a clean review, Jason authorizes squashing all work on this branch into clean history on top of current `origin/main`, this brief folded into a commit, usually one commit with a subject that names the fix (for example `fix(ui): strip pi's app prefix from session titles`), pushing `fix/941-pi-session-title`, and opening a PR against main whose body says `Fixes #941` and mentions that `--name` plumbing is a follow-up. If main has moved, rebase; never merge main into the branch. Review or CI fixes after the push are folded into the commit they correct and pushed with `git push --force-with-lease`. Drive CI green on macOS and Windows. Do not merge, delete the branch or worktree, or cut a nightly or release. Final Runner handoff: PR URL, what changed, tests and exit codes, CI result, the reviewer's verdict, and what Jason should smoke-test (a pi chat's sidebar title falls back to the role name; a pi session named with `/name` shows that name). Then stand by.
