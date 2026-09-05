# 478 — Consolidation pass 1: stale comments, dead code, unwrap audit

Tracking issue: [#478](https://github.com/yicheng47/runner/issues/478). Chore, P2. Baseline `main` at `e127646` (2026-09-04). Shipped 2026-09-05 in [#479](https://github.com/yicheng47/runner/pull/479); the review surfaced the unwired sidecar installers, fixed separately as [#480](https://github.com/yicheng47/runner/issues/480). This is the first of four passes; the file splits (passes 2–4) are **not** in scope here and get their own briefs.

## What ships

One feature branch, three commits (one per numbered section), no behavior change: (1) every comment that describes the Tauri/webview/xterm.js era is rewritten to describe the GPUI app or deleted; (2) code with no caller outside its own tests is removed; (3) every non-test `unwrap()` / `expect()` on an IO, parse, environment, or user-data path propagates an error instead of panicking. Tests, `make clippy`, and `make fmt` stay green after each commit.

## Where it touches

**1. Stale comments** — `grep -rn -i "tauri\|webview\|xterm" --include='*.rs' crates cli` finds 91 lines in 32 files. The dense ones: `crates/runner-app/src/terminal/glyphs.rs` (9), `crates/runner-backend/src/session/pty_runtime.rs` (8), `session/runtime.rs` (7), `ops/session.rs` (7), `session/manager/mod.rs` (5), `events.rs` (5), `wake.rs`, `lib.rs`, `cli_install.rs` (4 each). Rules:

- `glyphs.rs` and `glyph_data.rs` cite xterm.js as the **source of the box-drawing geometry** (see `crates/runner-app/LICENSE.xterm`). That is provenance, not staleness — keep every such line.
- Rewrite, don't just delete, when the comment carries a real why. `lib.rs:6` ("the Tauri app today, the native GPUI binary in Phase 3") → the GPUI binary is the only frontend. `windows.rs:3` ("several Tauri webview windows") → several GPUI windows. `cli_install.rs:181` ("The Tauri exe itself is `runner`") → the binary is `Runner`; say what the guard is actually for now. `runner-core/src/error.rs:1-4` points at `src-tauri/src/error.rs`, which does not exist — point at `runner-backend/src/error.rs`. `runner-backend/Cargo.toml:46-50` promises a tmux fallback on Windows that was retired with impl 0011 — say what `libc` and `portable-pty` are for today.
- `docs/` is out of scope except where a code comment you touch links to a doc path that moved; fix the link.

**2. Dead code** — seed list, then look for more with the same test: a `pub` item whose only references are in `#[cfg(test)]` blocks or `tests/`.

- `crates/runner-backend/src/session/launch.rs`: `LaunchScript`, `render_launch_script`, `write_launch_script`, `expand_home`, and the `shell_quote` uses that exist only to serve them (check `shell_quote`'s remaining callers before touching it — it has five outside the file). Agents are spawned through `CommandBuilder` in `pty_runtime.rs:156`; no `launch.sh` is ever written. Delete the items and their tests together.
- Sweep `runner-backend` and `runner-app` for others: for each `pub fn` / `pub struct` / `pub enum` in `crates/*/src`, grep the workspace for the name outside its defining file and outside test code. Report what you find before deleting anything beyond the seed list; if a candidate looks like a deliberate extension point (a comment says so, or `#[allow(dead_code)]` names a reason), leave it and list it in the handoff.
- Items compiled only under a `cfg` (`updater.rs` behind `all(target_os = "macos", feature = "updater")`, the `cfg(target_os)` arms in `pty_runtime.rs`) are not dead; leave them. Stale `#[allow(dead_code)]` markers are in scope: `mcp/mod.rs:104` still says `socket_path` is "used by Phase 3/4 settings UI" while `crates/runner-app/src/bootstrap.rs:303` calls it — drop the allow and the comment.

**3. Unwrap and expect audit** — scope is non-test code only: exclude `tests.rs`, `tests/`, `fixtures`, and `#[cfg(test)]` blocks. Mutex `lock().unwrap()` (284 sites, including ones split across lines) is fine and stays. For every other site, classify:

- **Invariant** (a value the same function just inserted, a regex compiled from a literal, a `MainThreadMarker` on the main thread): keep, but prefer `expect("why this cannot fail")` over a bare `unwrap()` so the panic message says what broke.
- **IO / parse / env / user data**: propagate. `cli/src/env.rs:69-72` unwraps four `RUNNER_*` env vars after a presence check — restructure so the check and the read are one `match`. `crates/runner-backend/src/router/mod.rs` (6 sites around `:823-:1662`), `session/manager/lifecycle.rs:121,201,210`, `windows.rs:141,151`, `crates/runner-terminal/src/terminal.rs:523,1034,1057` — inspect each; most are `.lock()` continuations, some are not. The `expect()` clusters in `crates/runner-app/src/surfaces/crews.rs` (7), `main.rs`, `logging.rs`, `assets.rs` (4 each), `ui/session_overlay.rs`, `surfaces/start_chat.rs`, `surfaces/runners.rs` (3 each): fonts and bundled assets failing to load are invariants (keep, with a message); a settings file, a path, or a DB row is not.
- Propagation follows the local pattern: `Result<_, Error>` with `Error::msg` in the backend, `anyhow::Context` in the app where it already exists, a logged fallback where the function has no error channel (`log::warn!` in the backend, `tracing::warn!` in the app). Do not add a new error type.

## Rules of the road

- No behavior change. If a comment fix or an unwrap change makes you want to change logic, note it in the handoff and leave the logic alone.
- Mission authorization: commits on the task feature branch are authorized, one per numbered section, so the reviewer can read them separately. Push, PR, and merge are **not** authorized — the human lands the branch.
- Do not launch the Runner app (`make run`) — the human smoke-tests. Verify with `cargo test --workspace`, `make clippy`, `make fmt`.
- Follow existing patterns; no new modules, traits, or helpers beyond what a deletion or an error propagation strictly needs.
- Passes 2–4 (splitting `mission_workspace.rs`, `sidebar.rs`, `crews.rs`, `runners.rs`) are out of scope. Do not move code between files in this pass.

## Verification

Per commit: `cargo test --workspace`, `make clippy`, `make fmt`, all green. The handoff lists: number of comment lines rewritten vs deleted; every item deleted under section 2 with the grep that proved it dead; every site under section 3 with its classification (invariant kept / propagated / logged fallback), so the reviewer can spot-check without re-deriving. Reviewer checks the diff for accidental behavior change first, then the classifications.

## Non-goals

File splits, renames, moving code between modules, refactors of the surfaces, new abstractions, doc-only edits outside a touched comment's link, and anything in `docs/impls/archive/` or `docs/features/archive/`.
