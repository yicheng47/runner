# #1498 — Cut CI and test time

Jason asked on 2026-11-28 for the neon duo to take [#1498](https://github.com/yicheng47/runner/issues/1498). The issue is the spec: read it in full, including its timing tables, scope, non-goals, verification and open questions. No separate `docs/features/` spec is needed.

## Starting point

- Branch `feat/1498-ci-test-time`, worktree `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1498-ci-test-time`, cut from `origin/main` at `e30922d6`. Use this worktree's own `target/`.
- Scratch files go under `/tmp/1498-*` only.
- In flight elsewhere, not on `main`; do not edit their checkouts: PR #1509 `feat/1508-libghostty-vt` (0.21's main feature) adds Zig and `ghostty-vt-sys` steps to all three jobs in `.github/workflows/ci.yaml` and edits `crates/runner-terminal/tests/snapshot.rs`; PR #1524 (Windows reveal and Ctrl+Enter) and the #1365 neon duo in `.worktrees/fix-1365-windows-codex-first-turn`. Whichever of this PR and #1509 lands second rebases onto the other; keep this PR's `ci.yaml` edits close to the steps it actually changes so that rebase stays small.
- Other missions are running on this Mac: run workspace-wide or heavy local test suites one at a time, not alongside theirs. The timings that matter come from GitHub's runners, not local runs.

## Problem and outcome

Windows is the slowest required check (about 30 min), macOS 16–23 min, Linux 8–11. The cost is test execution, not compilation, and on Windows it is a fixed per-test cost for tests that touch the filesystem. The outcome is a shorter CI wall time, with Windows at or below macOS, without dropping coverage that can catch a platform bug.

## Read first

- `.github/workflows/ci.yaml` — the three Rust jobs; the `Test` step is the target.
- `.config/nextest.toml` — the `ci` profile, the `real-daemon` test group and its cap.
- The latest Windows, macOS and Linux jobs linked in the issue, for the per-test timings.
- `AGENTS.md` (Development Commands, "Reproduce CI with …"), `docs/arch/windows.md#validation` and `docs/arch/linux.md`, which describe how CI runs the tests and must stay accurate.

## Deliverables

Work through the issue's scope in order, measuring after each step before taking the next:

1. **Windows temp off `C:`.** `TMP`/`TEMP` to `${{ runner.temp }}` for the Windows `Test` step, and print Defender's state once (`Get-MpComputerStatus`, `Get-MpPreference` exclusions). Record in the PR whether the disk, Defender or something else was the cost. Try a Dev Drive only if the temp move is not enough.
2. **Platform-independent sweeps on Linux only.** Select them with a nextest filter kept in one place in the repo (the CI workflow or `.config/nextest.toml`), not `cfg` in test code, so every test still runs locally on every platform and Linux CI keeps the full suite. Each excluded group carries a short comment naming why it cannot catch a platform bug. Tests touching files, git, paths, line endings, PTYs, file watchers or platform `cfg` code stay on every platform. If `runner-terminal`'s `recording_split_points` moves to Linux only, keep other `runner-terminal` tests on every platform, since #1509 makes the parser a natively built library per platform.
3. **Shard Windows only if steps 1 and 2 leave it above macOS.** If you shard, a final job must report as `Rust / Windows`, because branch protection requires that name. Do not change branch protection or other repository settings; if a design needs that, stop and ask Jason through Runner. Skip sharding when Windows is already at or below macOS.
4. **Windows test threads**, only if tests still wait on I/O after step 1. Keep the `real-daemon` cap.
5. **Measure macOS** once Windows is under it, and record in the PR what remains and whether it deserves a follow-up issue. Do not optimize macOS in this mission.

Update the docs named under Read first when the way CI selects or runs tests changes, including how to reproduce a platform's CI run locally.

## Boundaries

The issue's non-goals hold: no paid or larger runners, no sccache or shared compilation cache, no dropped or weakened check (fmt, Clippy, the updater Clippy pass, the installer check, the Windows resource check, the CLI reference and lockfile checks), and no test deleted from every platform. `nightly.yml`, `release.yml` and the other workflows do not run the test suite and stay unchanged. No product code changes, except a test-only change that a step genuinely needs, stated in the PR.

## Validation and handoff

This is CI and test-selection work with no app-UI surface, so no live checks are expected; the reviewer's clean review confirms that or names any it finds.

- Before/after timings for the Windows job and its test execution on two warm-cache runs each, plus macOS if it changes, in a table in the PR description with run links.
- Per-platform test counts, explained: every test or group moved to Linux only, with its reason, and Linux still running the full suite.
- The tests affected by the temp move pass on three consecutive Windows runs with no new FLAKY entries.
- `Rust / macOS` and `Rust / Windows` still report on the final head, and every job is green.

The coder implements and measures; the reviewer reviews the diff, the measurements and the exclusion list, and checks that no excluded test exercises platform-specific code. The final handoff names the PR, its head commit, the timing table and the exclusion list.

## Authorization

The neon duo (coder and reviewer) may commit, push this branch and open one PR against `main`. Since CI runs only on pull requests, the coder may open it as a draft early and re-run its CI (`gh run rerun`) to collect measurements; measurement-only commits are folded out or into the final commits before the PR is marked ready.
