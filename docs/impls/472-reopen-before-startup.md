# 472 — Reopen during startup must not abort the app

Tracking issue: [#472](https://github.com/yicheng47/runner/issues/472). Bug, P1, macOS. Baseline `main` at `236eb8c` (2026-09-06). Companion: [#473](https://github.com/yicheng47/runner/issues/473) (slow quit), which is what makes this race easy to hit; it is a separate mission and out of scope here.

## What ships

One feature branch. A macOS reopen Apple event (Dock click or `open -a Runner` while the app is still launching) that reaches the app before startup has set the app-store global is ignored; a reopen after startup behaves exactly as today (opens `main` if no window with that label is open, then activates). A regression test proves the reopen handler does not panic on an `App` with no globals set. The app never aborts from the reopen path.

## Why it aborts today

- `crates/runner-app/src/main.rs:1081` — `application.on_reopen(...)` is registered before `application.run(...)`. gpui invokes the reopen callback only when the app has no open windows, which is exactly the state during startup. The callback's first call, `window_label_is_open` (`main.rs:1400`), goes through `global_app_store` (`crates/runner-app/src/app_store.rs:644`), which is `cx.global::<GlobalAppStore>()` — a panic when the global is unset. `open_runner_window` (`main.rs:1298`) would then hit `cx.global::<GlobalNativePaths>()` the same way.
- `main.rs:1125-1126` — both globals are set inside the run closure, after fonts, wake install, settings load and `AppStore::new`. The crash on 2026-09-03 landed 176 ms after the startup banner.
- The panic sits inside gpui's `should_handle_reopen`, an `extern "C"` AppKit callback (`_handleAEReopen`), so it cannot unwind and the process aborts with SIGABRT. The crash report confirms the reopen was dispatched straight from `[NSApplication run]`'s event pump, not from inside the run closure.
- Startup already ends with `cx.activate(true)` (`main.rs:1276`) after restoring the windows, so a reopen during startup has nothing to do.

## Fix shape

- Move the closure body into a free function `handle_reopen(cx: &mut App)` in `main.rs` and register it with `application.on_reopen(handle_reopen)`. Keep the registration where it is.
- `handle_reopen` returns early when `cx.try_global::<GlobalAppStore>()` is `None`. One short comment saying why: startup is still running and will open and activate the windows itself. Do not activate, log, or open anything in that branch.
- The rest of the function is unchanged: if `!window_label_is_open(cx, "main")`, `open_runner_window("main".into(), None, None, cx)` with the existing `eprintln!` on error, then `cx.activate(true)`.
- Regression test in the existing `#[cfg(test)]` module at the bottom of `main.rs` (`native_root_tests`, `main.rs:1451`), following the `gpui::TestAppContext::single()` + `cx.update(|cx| ...)` pattern in `crates/runner-app/src/updater.rs:570`: call `handle_reopen(cx)` on a fresh context with no globals and assert `cx.windows().is_empty()`. Without the guard this test panics with `no state of type … GlobalAppStore exists`; confirm that once by running it against the unguarded function before committing, and say so in the handoff.

## Rules of the road

- Do not change `window_label_is_open` to read `cx.windows()` instead of the backend window registry, and do not move `set_global` earlier in the run closure. Both are plausible alternatives; neither is this fix. Note either as a follow-up in the handoff if you think it is warranted.
- No new modules, traits, or helpers beyond `handle_reopen`. No changes outside `crates/runner-app/src/main.rs`.
- Do not launch the Runner app (`make run`); the human smoke-tests. Verify with `cargo test -p runner-app`, `make clippy`, `make fmt`.
- Mission authorization: after the reviewer reports clean, PR mode is authorized — commit on the feature branch, push, open the PR, and drive CI green (`gh pr checks <n> --watch`; the required check is `Rust / macOS`). Do not merge: the human merges after their own check.

## Verification

- `cargo test -p runner-app` green, including the new test. `make clippy` and `make fmt` clean.
- Handoff lists the diff (should be small: one function, one registration line, one test), the checks run, and the confirmation that the test fails without the guard.
- Reviewer checks: the guard is the only behaviour change; the post-startup path is byte-for-byte the old closure body; the test really exercises `handle_reopen` and not a stub.

## Jason's smoke test (after landing)

1. With several sessions open, ⌘Q, and click the Dock icon again as soon as the icon disappears. Or run `open -a Runner; open -a Runner` in one line. Expect: no "Runner quit unexpectedly" dialog, one window, log shows no `panic` line.
2. With the app running, close every window (⌘W until none), click the Dock icon. Expect: the main window comes back, as before.

## Non-goals

The teardown speed in #473, the quit-confirmation gate in #491 (it will reuse the reopen path and must find it guarded), Windows (gpui never invokes the reopen callback there), and any change to how windows are registered or looked up.
