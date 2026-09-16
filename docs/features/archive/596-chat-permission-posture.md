# 596 — Chats inherit the agent's own permission default

> Tracking issue: [#596](https://github.com/yicheng47/runner/issues/596)
> Status: shipped 2026-09-15 in [#598](https://github.com/yicheng47/runner/pull/598).
> Priority: P1 — Trae chats are unusable and always have been.
> Platforms: macOS and Windows.
> Decision, 2026-09-15: `MissionPermissionMode` is the only place a permission posture is decided. Chats assert nothing.

## Motivation

Runner picks a permission posture for every chat, including chats the user never configured, and for TRAE CLI the value it picks is not one that CLI accepts — so every Trae chat has always died on spawn. Across both databases there are four Trae sessions ever, and all four crashed 163–330 ms after start.

`runtime_direct_runner` (`session/manager/mod.rs:1688`) builds a runtime-only chat's argv with `apply_permission_mode(runtime, &[], default_permission_mode())`, and `default_permission_mode()` is `PermissionMode::Auto` (`ops/runner.rs:59`). For Trae that becomes `--permission-mode auto`. TRAE CLI 0.120.52 accepts only `default`, `plan` and `bypass_permissions`, so `traecli` exits immediately. This was verified outside Runner: `traecli` on a PTY with Runner's exact env, cwd and positional prompt boots its TUI and stays up; only the flag kills it.

The narrower problem is that the runner row's permission mode reaches chats at all. Mission slots already converge to `MissionPermissionMode` (app-wide, defaults to `Bypass`), which strips the row's permission flags at spawn and appends its own pair — `spawn.rs:639` even says "Direct chats never pass here." So the row's mode only ever lands on chats: it asserts a posture on an attended session the user did not choose, and invents one for runtime-only chats that have no runner template at all.

Current behavior:

| | claude-code | codex | trae |
| --- | --- | --- | --- |
| Mission (`Bypass`, app-wide) | `--permission-mode bypassPermissions` | `--ask-for-approval never --sandbox danger-full-access` | `--permission-mode bypass_permissions` |
| Chat (row args, create-time `Auto`) | `--permission-mode auto` | `--ask-for-approval on-request --sandbox workspace-write` | `--permission-mode auto` — invalid, crashes |

## Scope

### Chats assert nothing

Every chat spawn path strips the runner row's permission flags and appends nothing, so each agent runs on its own default. This covers all four entry points:

- `spawn_direct` / `spawn_runtime_direct` — via `spawn_direct_inner`.
- `spawn_fork` — a fork of a chat is still a chat.
- The resume path in `spawn.rs` around 2100–2122, whose `if snap.mission_id.is_some()` branch converges mission slots and whose `else` branch currently replays the row's flags untouched.

`strip_permission_flags(runtime, args)` (`router/runtime.rs:456`) already does exactly this and is what mission convergence uses; reuse it rather than writing a second stripper. Every other arg on the row — model, effort, `--debug`, anything user-supplied — is left alone.

### What does not change

- **Nothing is stored differently.** Rows keep their `args` verbatim, there is no migration, and `ops::runner::create`/`update` keep baking the chosen mode. The runner form's dropdown still means something, because `MissionPermissionMode::RunnerDefault` reads the row.
- **Mission spawns are untouched.** `MissionPermissionMode` stays the single place a permission posture is decided, including its codex-specific Bypass arm (`danger-full-access` rather than `workspace-write`).
- **No UI surface or behavior change.** Correct the stale permission descriptions in Settings and the Trae runner form; keep the offered permission modes unchanged. This is spawn-path behavior only.

### The invalid Trae value

`permission_mode_args(Trae, PermissionMode::Auto)` must stop emitting `--permission-mode auto`. Trae has no auto-approve middle ground — its three values are `default` (ask), `plan` (plan-only) and `bypass_permissions` (never ask) — so `Auto` returns `Vec::new()`, matching the `(Codex, AcceptEdits) => Vec::new()` precedent already in the same match for the same reason.

This still matters after the chat change: a mission set to `MissionPermissionMode::Auto` routes through `permission_mode_args(runtime, PermissionMode::Auto)` and would kill a Trae slot exactly the same way. `(Trae, Bypass) => bypass_permissions` is correct and stays.

The doc comment at `router/runtime.rs:362-366` claims `auto` is a native preset "declared by `traecli --help`". It is not, and may never have been; correct it to the three values above.

## Implementation Phases

### 1. Chats stop applying the row's permission flags

Strip at each chat spawn entry point, symmetrically with the mission convergence at `spawn.rs:639`, and leave a comment saying why — the posture belongs to `MissionPermissionMode`, and a chat is attended.

### 2. Trae's `Auto` emits no flag

Fix the mapping, the doc comment, and the test at `ops/runner.rs:1176` that currently pins `--permission-mode auto` for Trae.

### 3. Verify

Cover both changes with tests beside the existing permission tests.

## Verification

- A runtime-only Trae chat starts and stays running. This is the bug that motivated the feature, and today it is a 100% crash.
- A Trae chat started from a runner template also starts, whatever mode that template carries.
- A Trae mission slot still receives `--permission-mode bypass_permissions` under the default app-wide `Bypass`, and no permission flag under `MissionPermissionMode::Auto`.
- claude-code and codex chats spawn with no permission flags at all, and keep every other arg — model, effort, and any user-supplied flags.
- claude-code and codex mission slots are byte-for-byte unchanged in all three `MissionPermissionMode` values, `RunnerDefault` included.
- A resumed chat and a forked chat both assert no posture; a resumed mission slot still re-reads the app-wide mode.
- Runner rows are unchanged on disk: creating or editing a runner still writes the chosen mode's flags to `args`.
- Run `cargo test -p runner-backend`, workspace Clippy, and formatting checks per `AGENTS.md`; update existing tests where expectations change.
