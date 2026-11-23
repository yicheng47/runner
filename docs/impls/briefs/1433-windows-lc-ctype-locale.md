# #1433 — Restrict `LC_CTYPE=UTF-8` locale fallback to macOS

Jason approved `neon trio` on 2026-11-23. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/fix-1433-windows-lc-ctype-locale` on `fix/1433-windows-lc-ctype-locale` (base `origin/main` `117f911e`, its own `target/`). The root checkout stays on `main`. Other worktrees under `.worktrees/` (`fix-1395-remote-project-drag`, `fix-narrow-island-gaps`, `test-flaky-process-probe-tests`) belong to other work; never inspect or edit them.

## Problem and outcome

In Runner's integrated terminal (`pwsh`) on Windows, non-ASCII (Chinese) git branch names and commit logs render as `<E6><A0><B7><E5><BC><8F>` when paged through `less`, whereas Windows Terminal and `git --no-pager` render `样式` cleanly.

Root cause: `ensure_utf8_locale` in `crates/runner-daemon/src/session/manager/spawn_spec.rs` unconditionally injects `LC_CTYPE=UTF-8` whenever `LANG`, `LC_ALL`, and `LC_CTYPE` are absent from the process and spawn environments. That fallback was added for macOS Dock-launched GUI apps (#461, mirroring Alacritty's macOS-only `alacritty/src/macos/locale.rs`), because bare `UTF-8` is a macOS/BSD locale name. On Windows (where POSIX locale variables are normally unset), injecting `LC_CTYPE=UTF-8` stops Git for Windows (`compat/mingw.c`) from setting its own `LC_CTYPE=C.UTF-8` default and passes bare `UTF-8` to MSYS2 `less.exe`. MSYS2/Cygwin's `newlib` `__loadlocale()` rejects bare `UTF-8`, leaving `less` in the `C` (`ASCII`) locale; `nl_langinfo(CODESET)` returns `ANSI_X3.4-1968`, so `less` selects its `ascii` charset and prints UTF-8 bytes `>= 0x80` as `<XX>` hex escapes. The same bare `UTF-8` value is also invalid in glibc on locale-free Linux environments.

Outcome: keep the `LC_CTYPE=UTF-8` fallback strictly on macOS (`#[cfg(target_os = "macos")]`). On Windows and Linux, Runner does not inject `LC_CTYPE=UTF-8` when no locale is present, so Windows sessions match Windows Terminal (allowing Git for Windows and MSYS2's `C.UTF-8` defaults to work cleanly) while explicit `role.env` and inherited process locale variables continue to pass through unchanged on all platforms.

## Read first

- Issue #1433 and issue #461 (original macOS Dock-launch context).
- `crates/runner-daemon/src/session/manager/spawn_spec.rs`: `LOCALE_VARS`, `ensure_utf8_locale`, and `agent_env`.
- `crates/runner-daemon/src/session/manager/spawn.rs`: test re-export of `ensure_utf8_locale` and `agent_env`.
- `crates/runner-daemon/src/session/manager/tests/spawn_env.rs`: `utf8_locale_fallback_applies_only_when_no_locale_present` and `spawn_env_respects_configured_locale_and_falls_back_to_utf8`.
- `crates/runner-daemon/src/session/manager/tests/golden.rs` and `crates/runner-daemon/src/session/manager/tests/unlisted.rs`: verify `agent_env` callers stay consistent.

## Deliverables

1. **macOS-only locale fallback.** In `crates/runner-daemon/src/session/manager/spawn_spec.rs`, gate the `LC_CTYPE=UTF-8` fallback to `#[cfg(target_os = "macos")]` so Windows and Linux spawns never inject bare `LC_CTYPE=UTF-8`. Update the doc comment on `ensure_utf8_locale` and `agent_env` to explain why the fallback is macOS-only (bare `UTF-8` is rejected by MSYS2/Cygwin and glibc, whereas Windows Terminal leaves POSIX locale vars unset so Git for Windows and MSYS2 default to `C.UTF-8`). Keep `role.env` and inherited process environment behavior unchanged on all platforms, and avoid any unused-symbol or Clippy warnings under `--all-targets -D warnings` on macOS, Windows, and Linux.
2. **Tests.** Update `crates/runner-daemon/src/session/manager/tests/spawn_env.rs` (and any other affected tests) so:
   - On macOS (`#[cfg(target_os = "macos")]`), `ensure_utf8_locale` and `spawn_direct` still verify `LC_CTYPE=UTF-8` is injected only when neither the process env nor `env` sets `LANG`, `LC_ALL`, or `LC_CTYPE`.
   - On non-macOS (`#[cfg(not(target_os = "macos"))]`), tests verify `agent_env` and `spawn_direct` do **not** inject `LC_CTYPE` when no locale is configured, while explicit `role.env` locale overrides (such as `LC_ALL=zh_CN.UTF-8`) still pass through.
   - Check whether adding a target-independent helper or unit test makes both behaviors testable without dead-code warnings across platforms.
3. **Docs check.** Confirm no `docs/guide/` page documents this internal fallback; no guide edits are expected unless a reference exists.

Non-goals: injecting `LESSCHARSET=utf-8` (symptom workaround that leaves broken `LC_CTYPE=UTF-8` for other MSYS2/Git Bash tools), changing ConPTY or Windows console code page (`chcp 65001`), adding a Linux `C.UTF-8` fallback, or changing UI, `shell_path.rs`, or `pty_runtime.rs`.

## Validation and live checks

- **Automated:** run `cargo test -p runner-daemon spawn_env`, `cargo test -p runner-daemon golden`, `cargo test -p runner-daemon unlisted`, `make clippy`, `cargo fmt --all -- --check`, and `git diff --check`, then drive CI green on `Rust / macOS`, `Rust / Windows`, and `Rust / Linux`.
- **Live QA (`@qa` after a clean review, macOS development app, `script/qa-lock acquire --timeout 540`):**
  1. Verify macOS #461 parity is preserved in the development app: start a fresh terminal session in a `/tmp` git repo with a Chinese branch name (e.g. `feature_样式`) — including a locale-stripped dev daemon/app launch simulating a Dock launch (`env -u LANG -u LC_ALL -u LC_CTYPE`) if cleanly testable without disturbing pre-existing dev sessions, otherwise verifying the spawned terminal session's locale and `git branch` / `git log` output through `less` — and confirm Chinese branch names render cleanly without `<XX>` hex escapes or Mac Roman mojibake. Clean up `/tmp` fixtures and archive/close only QA-created sessions before releasing `qa-lock`.
  2. Record the macOS run and evidence in `docs/tests/runs/1433-windows-lc-ctype-locale-mac.md`.
  3. **Native Windows gap:** the primary bug occurs on native Windows (`pwsh` + Git for Windows MSYS2 `less`), which cannot be live-tested from macOS. Keep the `needs-live-test` label on the PR and state plainly in the PR description and QA verdict that native Windows live verification (opening Runner's integrated `pwsh` terminal on Windows without `LESSCHARSET`/`LC_CTYPE` set, confirming `$env:LC_CTYPE` is empty, and confirming `git branch` / `git log` render Chinese branch names cleanly through `less`) remains pending on Windows.

## Authorization

Approved crew: `neon trio` (coder, reviewer, qa). Commit on `fix/1433-windows-lc-ctype-locale`, folding this brief into the implementation commit before push, and open a PR against `main` with `Fixes #1433` and `risk-low`. `@reviewer` adds `reviewed` on the final head; keep `needs-live-test` until native Windows live verification runs. Drive CI green across all required checks, then stop. Do not merge.
