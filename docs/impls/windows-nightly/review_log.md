# 437 — Windows nightly: review log

Dated record of code reviews for the Windows nightly program ([README](README.md), [impl log](impl_log.md), [plan](plan.md)). One entry per review session, newest at the bottom. Each entry names what was reviewed, how, and the findings in severity order; follow-ups carry a checkbox so the next mission can close them and note the fixing commit here.

## 2026-09-06 — uncommitted JASONPC tree (Claude, inline)

**Scope.** The working tree on `nightly-windows` at `541c565` plus its uncommitted changes: 29 modified files and the new `crates/runner-app/src/platform_ui/`, `design/windows-header.pen`, and `make.cmd`. Covers the 2026-09-06 workstreams in the impl log: platform_ui split and Windows title bar, platform fonts, Windows event-log shared lock, ConPTY output coalescing, home-directory cwd fallback, Windows close ownership, and the in-flight Cmd-to-Ctrl keymap change.

**Method.** Inline read of every diff, no agent fan-out. `cargo fmt --all --check` passed. Clippy and the test suite were not run: three Codex processes were editing the same checkout during the review, four files changed mid-read (`keymap.rs`, `file_links.rs`, `surfaces/chat.rs`, `terminal/element.rs`), and a cargo build was active. Treat the findings as a snapshot taken around 14:00 local.

### Findings

Severity order. Tick the box and add the fixing commit when closed.

- [ ] **Plain-Ctrl keymap swallows terminal keys** (`crates/runner-app/src/keymap.rs`, `windows_default`). The Cmd-to-Ctrl mapping was requested by Jason and stands; three bindings still collide with keys terminal programs rely on. `Ctrl+W` (`close-pane`, `system-close-window`) deletes the previous word in every shell, so muscle memory will close panes mid-command. `Ctrl+[` is the ESC byte; binding it to `pane-previous` / `mission-tab-previous` stops it reaching vim and the agent TUIs, and the impl log's Phase 1 checklist lists "Ctrl+[ as ESC" as an acceptance item. `Ctrl+D` (`split-pane-right`) eats EOF. Options: keep Shift on W, D, [ and ] only, or let a focused terminal win those four.
- [x] **Stale caption reservation in the list header** (`crates/runner-app/src/ui/list.rs:660`). Still pads the header right by three caption widths on Windows outside fullscreen. Every other caller goes through `caption_inset_for`, now zero; this one hardcodes the math, so list-page headers sit about 138 px left under the new title bar.
- [x] **Timing-sensitive Windows forwarder test** (`crates/runner-backend/src/session/manager/tests.rs`, `forwarder_coalesces_delayed_conpty_cursor_restore_without_waiting_for_eof`). Relies on 12 ms sleeps landing inside the 25 ms grace window after the 100 ms cap. Default Windows timer resolution is about 15.6 ms, so the 105 ms cases have roughly one tick of margin: passes on the i9, likely flaky on `windows-latest`. Drive the forwarder with a fake clock or widen the margins.
- [x] **Dead inset plumbing** (`surfaces/app_shell.rs` `caption_inset_for`; callers in `mission_workspace.rs`, `panes.rs`, `crews.rs`, `runners.rs`). Returns a constant zero and is still added at a dozen call sites; `chat_header_caption_inset` reduces to max(negative, 0). The test now asserts a constant. This change made them dead, so remove them in this change.
- [x] **Vestigial cfgs in the macOS chrome file** (`crates/runner-app/src/platform_ui/macos.rs`, lines 4–5 and 64–73). The file only compiles on macOS; the `target_os` branches survived the verbatim move and the not-macOS branch can never compile.
- [x] **`make.cmd` line endings.** The file is LF and cmd.exe has known label/goto misparse bugs with LF-only batch files. Add `make.cmd text eol=crlf` to `.gitattributes` so every clone gets CRLF regardless of `core.autocrlf`.

### Reads clean

- **Event-log shared lock** (`crates/runner-core/src/event_log/log.rs`). Correct on Windows: the reader lock releases when the handle drops and no writer path reopens the file under its own lock. Side effect: a concurrent reader now makes `try_append` report contention where the read used to fail instead; the forwarder's bounded retry covers it.
- **ConPTY output coalescing** (`session/manager/output.rs`). Bounded and order-preserving; status transitions are never delayed past the current burst. Tradeoff to remember: every Windows burst waits at least 25 ms of quiet before render, so keystroke echo gains that latency.
- **Home-directory fallback** (`resolve_spawn_cwd`, `ui/field.rs`). Consistent across mission spawn, direct chat, fork, resume, and the placeholder. Two behavior changes: session rows now persist the home path where they held NULL, and an empty explicit cwd falls through to the runner default, then home, instead of erroring.
- **Windows close path** (`platform_ui/windows.rs`, `finish_window_close`). Removing the window and vetoing the OS handler matches the gpui-ce double-destroy analysis in the impl log.

**Housekeeping.** Global `core.autocrlf` is `true` on JASONPC, hence the line-ending warnings on every git command; commits still normalize to LF. Follow-up: run `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace --no-fail-fast` once the Codex session has stopped editing, and record the result here.

## 2026-09-06 — review follow-up (Codex, inline)

Reviewed all six findings against the current tree. Five are fixed in the working tree; no fixing commit exists yet.

- **List-header caption reservation — fixed.** Removed the Windows-only three-button padding from `PaginatedListPage`. Caption width now applies only inside the Windows title bar.
- **Forwarder test timing — fixed.** Extracted the existing Windows coalescing loop into `coalesce_windows_output`, accepting receive and clock functions. Production passes the same output receiver and `Instant::now`; the 25 ms quiet period, 100 ms cap, one 25 ms repair grace, and 1 MiB size cap are unchanged. The regression schedules arrivals on a simulated clock and covers a repair at 12 ms, a repair at 105 ms, a split redraw repaired at 107 ms, and a repair after the 125 ms limit. A separate real-channel test checks delivery before EOF without scheduling arrivals inside a narrow timing window. Existing status-order and size-cap tests still exercise the forwarder.
- **Dead inset plumbing — fixed.** Removed the zero-valued helpers, redundant padding overrides and rail-width clamps, unused window arguments, and constant-only tests. Shared headers keep their existing rem-based padding; the mission rail already has a 200 px minimum. macOS layout values are unchanged.
- **macOS cfgs — fixed.** Removed the redundant macOS guards and unreachable non-macOS fallback inside the file already selected only for macOS.
- **Batch-file line endings — fixed.** Added `make.cmd text eol=crlf` to `.gitattributes` and normalized the local script to CRLF. `git check-attr` confirms the rule; the usage/goto path returns the expected exit code 2.
- **Plain-Ctrl terminal collisions — retained, not fixed.** The collisions are real. Jason explicitly requested Cmd → Ctrl, and the current plan already records app shortcuts taking precedence. Reintroducing Shift or giving terminal input priority would change that requested behavior, so this follow-up preserves the mapping. Ctrl+W still closes, Ctrl+D still splits, and Ctrl+[ still navigates in the ChatSplit/Mission contexts. Ctrl+C without a selection continues to propagate to the terminal. The finding remains open as a documented tradeoff.

Validation on JASONPC: `cargo test --workspace --locked -j12 --no-fail-fast -- --quiet` passed 997 tests with one ignored; `cargo clippy --workspace --all-targets --locked -j12 -- -D warnings`, formatting, and diff checks passed. `make.cmd build` rebuilt the development executables successfully. Native macOS execution and visual acceptance of the header spacing remain unverified on this Windows host.

## 2026-09-06 — second pass after the Codex follow-up (Claude, inline)

**Scope.** The same uncommitted tree after the follow-up above; 38 modified files plus the untracked `platform_ui/`, `windows-header.pen`, `make.cmd`, and this log. Clippy finished from cache in under a second, so the tree was stable during this pass.

**Fix verification.** All five ticked items check out in code. `ui/list.rs` no longer pads for caption buttons. `caption_inset_for`, `caption_inset`, and `chat_header_caption_inset` are gone with their callers, and `MISSION_RAIL_MIN` is 200 px so the dropped 120 px clamp was redundant. `platform_ui/macos.rs` has no cfg attributes left. `.gitattributes` carries `make.cmd text eol=crlf` and the file is CRLF on disk. The forwarder loop is extracted as `coalesce_windows_output` with injectable receive and clock; `windows_coalescer_preserves_delayed_cursor_repair_with_a_bounded_grace` runs on a simulated clock and asserts the exact elapsed time in each case, including the 125 ms cutoff, so the flake risk is closed.

**Checks run.** `cargo clippy --workspace --all-targets --locked -- -D warnings` passed. Unit tests: `cargo test --workspace --lib --bins` ran 70 + 187 (runner-app), 588 + 1 (runner-backend), 25 (runner-core), 54 (runner-terminal), 9 (cli). Integration tests for `runner-cli` and `runner-terminal` passed (16, 10, 1, 9). The `runner-app` integration tests did not run: a debug `Runner.exe` started at 14:22 held `target\debug\Runner.exe`, so cargo could not relink it; Codex's 997-test run earlier in the hour covered them.

### Findings

- [x] **`spawn_emits_idle_after_silence_and_busy_on_more_output_windows` depends on the caller's PATH** (`crates/runner-backend/src/session/pty_runtime.rs`, unmodified in this tree). Run from Git Bash it fails in 0.17 s with `got []`: the spawned `cmd` inherits a PATH with `C:\Program Files\Git\usr\bin` ahead of System32, so `timeout /t 2` resolves to the GNU coreutils binary, which rejects the switch and exits at once. With System32 first it passes in 4 s. Claude Code's Bash tool on Windows always runs through Git Bash, so any agent running `cargo test` from Claude Code hits this. Cheapest fix: spell the command as `%SystemRoot%\System32\timeout.exe` in the test, or note in the README that Windows tests run from PowerShell.
- [ ] **Plain-Ctrl terminal collisions** stay open as the documented tradeoff recorded in the follow-up above. Nothing new to add.

**Nit.** `render_crew_editor` in `surfaces/crews.rs` keeps a now-unused `_window` parameter; the caller has one to pass, so it is harmless.

## 2026-09-06 — second review follow-up (Codex, inline)

The PATH finding is valid and fixed in the working tree: both waits in the Windows PTY regression now call `%SystemRoot%\System32\timeout.exe`. With `C:\Program Files\Git\usr\bin` first on PATH, the original command reproduced the empty-transition failure in 0.17 s; the fixed regression passed in 4.06 s. This is a test command correction, with no PTY runtime behavior change. The requested plain-Ctrl mapping remains, and the harmless `_window` parameter is left alone. No fixing commit exists yet.
