# #1149 — Inside a session, `runner` reaches the daemon that spawned it

Jason approved a claude duo mission on 2026-11-04. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1149-session-runner-cli` on `fix/1149-session-runner-cli`, based on `origin/main` at `cb7cf423`. Issue: [#1149](https://github.com/yicheng47/runner/issues/1149), P1, milestone 0.19.

## Problem

The daemon puts the bundled CLI first on a session's PATH (`compose_path`: mission shim dir, bundled bin dir, shell PATH, fallback dirs). A login or interactive shell re-runs the user's startup files, and installers commonly prepend `~/.local/bin` there; the Codex and Antigravity installers do on Jason's Mac. `~/.local/bin/runner` is the installed app's command link, so a runtime whose tool runs commands in a login shell, such as Codex, resolves `runner` to the installed CLI:

```
PATH="<dev app data>/bin:/usr/bin:/bin"
zsh -c   'command -v runner'  → <dev app data>/bin/runner
zsh -lc  'command -v runner'  → ~/.local/bin/runner   (installed app)
```

In a development-build Codex chat, `runner mission start` therefore creates the mission in the installed Runner, where it cannot link to the dev chat (#748), and a QA test chat can write into the installed app's data. Claude Code keeps the session's PATH order and is not affected. A mission slot's per-slot shim is prepended the same way, so slots of an affected runtime likely bypass their shim too. With one build installed both commands reach the same daemon, so the visible damage is mostly dev next to installed.

## Intended outcome

Inside any session Runner spawns, every `runner` invocation reaches the CLI of the daemon that spawned the session, whatever the shell's startup files do to PATH. Outside a session nothing changes.

Suggested shape; the coder decides the details and the reviewer checks them:

- **Daemon.** Export the session's own CLI path to every session the daemon spawns, in one place (`base_spawn_spec`, next to `SESSION_ID_ENV`), covering direct chats, mission slots, fork and resume. Define the variable in `runner-core`'s protocol. For a mission slot, point it at the slot's shim, not the bundled binary, so a hand-off restores the slot's baked variables. Check whether terminal panes get the bundled bin dir today and treat them consistently. Hook runtimes' `RUNNER_HOOK_EXECUTABLE` stays as it is.
- **CLI.** At the start of `main`, when the variable is set and names an existing executable that is not the running one, hand the invocation to it with the same arguments, stdin, stdout and stderr: `exec` on Unix; on Windows spawn it, wait, and exit with its code. Guard against loops: the slot shim execs the bundled binary while the variable still names the shim, so the hand-off must happen at most once per invocation. A missing or unreadable target falls through to the running CLI unchanged. Compare executables robustly (symlinks such as `~/.local/bin/runner`, canonical paths).

## Read first

- `AGENTS.md`, including Test Scope and Crew Missions (crews are duos; `qa` joins after review).
- `crates/runner-daemon/src/session/launch.rs`: `compose_path`.
- `crates/runner-daemon/src/session/manager/spawn_spec.rs`: `base_spawn_spec`; callers in `direct_spawn.rs`, `mission_spawn.rs`, `fork.rs`, `resume.rs`.
- `crates/runner-daemon/src/runtimes/helpers.rs`: `hook_env`, `hook_executable`.
- `crates/runner-daemon/src/cli_install.rs`: `install_session_runner_shim`, `install_exe_shim` (Windows).
- `crates/runner-cli/src/main.rs`, `crates/runner-cli/src/env.rs`.
- `docs/arch/arch.md` §9, the `runner` CLI.

## Deliverables and boundaries

- The daemon export and the CLI hand-off above, on macOS, Windows and Linux.
- Tests: every spawn path exports the variable with the right target (bundled CLI for chats, the shim for slots); the CLI hands off to a different executable, does not loop through the shim, falls through when the target is missing, and passes the exit code and arguments through. Use stub executables in temp directories; no real agents.
- `docs/arch/arch.md` §9 says how a session's `runner` resolves. No guide page changes unless CLI help text changes; then regenerate with `make guide-cli`.
- Non-goals: changing `compose_path` order, editing anyone's shell profile, renaming `runner-dev`, changing hook delivery, or the companion protocol.

## Validation and handoffs

Name the changed behaviors and the check that covers each before running anything. Run the affected `runner-daemon` and `runner-cli` tests with `--locked` and this worktree's own target directory, workspace Clippy with warnings denied, `cargo fmt --all --check` and `git diff --check`. Drive the required macOS and Windows CI checks green. No full smoke.

@coder implements and hands the diff to @reviewer. @reviewer reviews code only. This change alters where an agent's `runner` commands land in the running app, so live checks are required; the clean review names them, starting from these:

1. In the dev app built from the candidate, a Codex chat whose login shell puts `~/.local/bin` first: `command -v runner` may still show `~/.local/bin/runner`, but `runner mission list --json` returns the dev daemon's missions, and `runner mission start --title "qa 1149" -q` creates the mission in the dev app, grouped under that chat.
2. A dev mission with a Codex slot: the slot's `runner msg post` lands in that dev mission's feed.
3. A dev Claude Code chat still reaches the dev daemon.

Before the fix, reproduce only read-only (`command -v runner`, `runner mission list`); never start or change anything in the installed Runner. The mission's operator adds the codex `qa` role after the clean review to run these checks on the candidate commit. qa's record goes to @coder for the commit.

## Authorization

Jason approved this claude duo mission: commit, push this branch and open one PR against `main`, labelled per AGENTS.md (it touches session launch, so `core-review` applies), with `needs-live-test` until qa's checks pass. Stop at the open PR with green CI and report through Runner with the PR URL, final SHA, changes and validation.
