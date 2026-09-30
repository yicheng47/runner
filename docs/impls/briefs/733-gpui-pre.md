# 733 — Move GPUI from the yanked gpui-ce 0.3.3 to gpui-pre =0.3.7

Do [#733](https://github.com/yicheng47/runner/issues/733). Jason requested this mission on 2026-09-30. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/chore-733-gpui-pre`, on the existing branch `chore/733-gpui-pre`, created from `origin/main` at `258a938`. The root checkout stays on main. Do not create another branch or worktree, and do not share a Cargo target directory. Other worktrees under `.worktrees/` (`fix-755-…`, `fix-762-…`, `spike-733-zed-gpui`) belong to other work; treat them as another machine's checkout and do not touch them.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions.
- [`733-gpui-pre.md`](../archive/733-gpui-pre.md), the archived plan. **It wins over this brief on any detail.** It lists every dependency line, the entry point, each API break with its call sites and fix, the behavior changes, the six tests that fail after the port, the docs to update, and the verification.
- Issue #733 for the reasoning.

## What is already known

A throwaway probe compile on 2026-09-30 ported `runner-app` mechanically on macOS: `cargo check --all-targets` and Clippy with `-D warnings` passed, and 519 of 525 `runner-app` tests passed. Its diff is at `target/733-probe/probe.patch` in this worktree (ignored by git), with the test log beside it. It is a reference for where the breaks are, not the implementation: it took shortcuts, was never formatted, fixed none of the six failing tests, and was never built on Windows. Review every hunk you take from it. Plan section 3 is the list to work from.

## Deliverables

1. **Dependencies** (plan §1). `gpui-pre =0.3.7` everywhere `gpui-ce` is today. macOS adds `gpui_platform` (`gpui-pre-platform`, `font-kit`). **Windows adds `gpui_windows` (`gpui-pre-windows`, `default-features = false`), not `gpui_platform`**: `gpui-pre-platform` turns GPUI's `windows-manifest` back on, which would put GPUI's manifest next to Runner's and break the CI step "Verify Runner manifest and icon". Commit the updated `Cargo.lock`.
2. **Entry point** (plan §2). `gpui_platform::application()` on macOS; `Application::with_platform(Rc::new(gpui_windows::WindowsPlatform::new(false)…))` on Windows.
3. **Port** (plan §3): focus and blur take `cx` (116 calls, plus five helpers that must take `&mut App`), `ShapedLine::paint` gains `TextAlign::Left, None` in the terminal element, `max_offset()` is a `Point`, `BoxShadow { inset }`, `Menu { disabled }`, `Corner` → `Anchor`, `flex_shrink(1.)`. The smallest change that keeps today's behavior at each site.
4. **The six failing tests** (plan §5). Find the GPUI change behind each one. Fix the code where today's rendering should hold (the Settings › Agents header must wrap at 320 px; Start Chat's Reset must reset, not close the modal). Change an expectation only when the new layout is correct and the old number was a gpui-ce artifact, and give the reason in the handoff. Never weaken an assertion just to make it pass.
5. **Profiles** (plan §7). Rename the `gpui-ce` entries to `gpui-pre`; give `gpui-pre-macos`, `gpui-pre-windows` and `gpui-pre-apple` `opt-level = 3` in `dev`.
6. **Docs** (plan §8): `arch.md` (stack row, bump procedure, risk line), `windows.md`, `docs/tech/gpui-rendering.md` and `docs/tech/README.md` (re-check their claims against the 0.3.7 source), `README.md` and `README.zh-CN.md` together, the #701 spec, and the stale `debug_bounds` comment in `settings_page.rs`. The completed plan lives in `docs/impls/archive/733-gpui-pre.md` with a status line naming PR #767; its index entry is under Archive and inbound links point there.

**Stop and report** before going further if the Windows CI build shows something structural (a missing platform API, a link failure beyond the manifest, a renderer change), rather than working around it.

## Boundaries

No behavior change beyond plan §4: do not adopt `request_attention`, accessibility or the new IME hooks, and do not change `inactive_frame_interval` or `app_owns_titlebar_drag`. No `[patch.crates-io]` entries unless a build or test shows the need, each with a comment. Crews never run the dev app (`make run`) or drive Jason's Runner; Jason smoke-tests. Do not start extra agents, crews or subagents.

## Review, verification and authorization

The coder owns implementation and checks. The reviewer waits for an explicit Runner handoff, then reviews the full branch diff against the plan with must-fix findings first and file:line pointers. Focus: the Windows dependency shape (no `windows-manifest`), every focus change keeping the same target and timing, the terminal paint calls, each of the six test fixes (code fix or justified expectation change), and docs that still name gpui-ce. Iterate until the reviewer posts `NO REMAINING MUST-FIX ISSUES`.

Run `cargo test --locked --workspace --profile ci`, workspace Clippy with warnings denied (`cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`), macOS updater Clippy (`--features updater`), `cargo fmt --all --check`, `git diff --check`, `cargo update --dry-run` (must resolve), `cargo tree -i gpui-ce` (must find nothing) and `cargo tree -d | grep gpui` (no duplicates). Record the exact commands and exit codes. Windows CI has repeatedly failed on imports or helpers used only by `cfg(unix)` tests; gate any such import or helper with `#[cfg(unix)]`.

After a clean review, Jason authorizes squashing all work on this branch, this brief included, into one commit on top of current `origin/main` with a subject that names the change (for example `chore(ui): move GPUI from the yanked gpui-ce 0.3.3 to gpui-pre 0.3.7`), pushing `chore/733-gpui-pre`, and opening a PR against main whose body says `Fixes #733`. The body names each behavior change from plan §4, each test fix and why, and CI build times (cold and warm) against the last green main run. If main has moved, rebase; never merge main into the branch. Review or CI fixes after the push are amended into the same commit and pushed with `git push --force-with-lease`. Drive CI green on macOS and Windows, including the manifest step. Do not merge, delete the branch or worktree, or cut a nightly or release. Final Runner handoff: PR URL, what changed, tests and exit codes, CI result, the reviewer's verdict, and Jason's smoke list from the plan's Verification section. Then stand by.
