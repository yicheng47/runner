# #1477 — Bump gpui-pre from 0.3.7 to 0.3.14

Jason approved `neon trio` on 2026-11-25. Work only in `/Volumes/samsung/repos/yicheng47/runner/.worktrees/chore-1477-gpui-pre-0.3.14` on `chore/1477-gpui-pre-0.3.14` (base `origin/main` `678da3ec`, its own `target/`). The root checkout stays on `main`. Other worktrees under `.worktrees/` belong to other work; never inspect or edit them.

## Problem and outcome

Runner pins `gpui-pre` `=0.3.7` (`zed@1a28cff`, 2026-09-28). 0.3.14 (`zed@f126950`, 2026-11-23) is the newest snapshot, re-checked on crates.io on 2026-11-25. It carries crash, rendering and idle-wakeup fixes Runner can hit; issue #1477 lists them with their Zed PRs. Outcome: every direct `gpui-pre*` pin on 0.3.14, the API port done with today's behavior kept, docs current, and the macOS idle-wakeup gap of #1135 measured before and after.

Issue #1477 is the spec: its Scope, "Behavior that changes without an edit", Verification and Out of scope sections are binding. This brief adds only the mission specifics.

## Read first

- Issue #1477, and `docs/impls/archive/733-gpui-pre.md` (the previous GPUI move and what broke).
- `docs/arch/arch.md`, the "GPUI bumps" paragraph (bump procedure) and the risk line.
- `crates/runner-app/Cargo.toml` (all `gpui-pre*` pins: macOS, Windows, Linux, normal and dev) and the `gpui-pre*` profile entries in the root `Cargo.toml`.
- The 0.3.14 crate sources in `~/.cargo/registry` once fetched, and Zed at `f126950` for behavior questions.

## Deliverables

1. **Pins.** Move every direct `gpui-pre`, `gpui-pre-platform` and `gpui-pre-windows` pin to `=0.3.14`. Update `Cargo.lock` for these crates only (`cargo update -p ... --precise`), never a general `cargo update`. Windows keeps `gpui-pre-windows` with default features off. Check that nothing Runner needs came only from features or dependencies GPUI has since dropped.
2. **Port.** Replace the deprecated `ScrollWheelEvent`/`on_scroll_wheel` (34 lines in 14 files) with `ScrollEvent`/`on_scroll`, and fix whatever else the compiler reports on macOS, Windows and Linux with the smallest change that keeps today's behavior. If the breakage is far beyond this, stop and report to the operator with the error list before porting further.
3. **Toolchain.** If 0.3.14 needs a newer Rust than 1.97.1, raise `rust-toolchain.toml` (and any CI pin) in its own commit, first in the PR.
4. **Tests.** A test or golden that changes because of Taffy 0.13 → 0.15 or UAX #14 line breaking is updated only when the new output is correct; list each one in the PR with the reason. Never weaken an assertion to pass.
5. **Build profiles.** Re-check the `gpui-pre*` entries in the root `Cargo.toml` against where the hot paths live in 0.3.14.
6. **Docs.** `docs/arch/arch.md` (stack row, risk line), `docs/arch/linux.md`, `docs/arch/windows.md`, `docs/tech/README.md` and `docs/tech/gpui-rendering.md`: new version, Zed commit and source paths, re-checking each claim about GPUI internals. No `docs/guide/` change is expected; say so in the PR if none.

Non-goals are the issue's Out of scope: no switch to Zed's official `gpui`, no Herdr patches (#1135, #1137), no new GPUI APIs.

## Validation and handoffs

**Coder (automated):** `make verify`; `cargo tree -d` shows one copy of each `gpui-pre-*` crate; `cargo tree -e features --target x86_64-pc-windows-msvc -p runner-app` shows no `windows-manifest`; CI green on macOS, Windows and Linux, including "Verify Runner manifest and icon". The PR names each upstream default that changes behavior (layout, wrapping, frame pacing, macOS 12 / SDK 26 minimums) as the issue lists. Hand the reviewed-ready head to `@reviewer`.

**Reviewer:** reviews the diff against the issue's Scope, checks no assertion was weakened, and that the lockfile moved only GPUI and its own dependency changes. The live checks below are already required: this is a cross-cutting UI framework change.

**qa (after a clean review, macOS development app, `script/qa-lock acquire --timeout 540`):** fresh `/tmp` folders, inexpensive models, tiny prompts; operate only sessions qa creates. Each check runs on the candidate head:

1. **Terminal:** rendering, trackpad and mouse-wheel scrolling (the `ScrollEvent` port) in a terminal, the mission feed, a Markdown preview and a long list, and text selection.
2. **Unfocused repaint:** agent output keeps streaming into a window while another app is frontmost.
3. **Input:** cursor blink; IME marked text and Chinese input in a chat terminal and in a text field.
4. **Window:** resize, titlebar drag, close and reopen, display sleep and wake (`pmset displaysleepnow`), and moving between displays if a second one is attached (otherwise a stated gap).
5. **Overlays:** menus, tooltips and popovers open, position and dismiss.
6. **Wrapping:** Markdown wrapping of English and Chinese text in the mission feed and a chat's Markdown view.
7. **Drag and drop:** a file dropped into a terminal, and a tab or pane drag.
8. **Idle wakeups (#1135):** with one idle window and no animation, record CPU and idle wakeups for 30 s focused and 30 s unfocused (`top -l 2 -s 30 -pid <pid> -stats pid,command,cpu,idlew`) on the candidate, and the same on the base `678da3ec`, built in this worktree by detaching to it with a clean tree and switching back to the branch afterwards. Report both in the PR and say whether #1135 looks fixed, narrowed or unchanged.

Record the run in `docs/tests/runs/1477-gpui-pre-0.3.14-mac.md` with screenshot evidence, commit it on the branch, and record the verdict per check in the PR description. A failure goes back to `@coder`, and the fixed head is reviewed again before qa reruns the affected checks.

**Native gaps:** Windows and Linux live QA cannot run from this Mac. Keep `needs-live-test` on the PR and list the same checks (1–7) as pending on Windows and Linux in the PR description.

## Authorization

Approved crew: `neon trio` (coder, reviewer, qa). Commit on `chore/1477-gpui-pre-0.3.14`, folding this brief into the implementation commit, and open a PR against `main` with `Fixes #1477`, `Refs #1135`, `risk-high`, `frontend-only` and `needs-live-test`. Drive CI green, then stop. Do not merge.
