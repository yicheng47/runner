# Runtime executable discovery + overrides

## Status

Implemented. Tracking issue [#279](https://github.com/yicheng47/runner/issues/279), scoped to built-in runtimes only — the user-defined custom-runtime extension is explicitly cut (see spec 37 "Out of scope"). Spec: `docs/features/37-agent-runtime-executable-settings.md`. Design: `design/runner-setting.pen`, frame `Settings — Agents` (node `Zes2l`) and `Spec — Agent runtime row states` (node `cXdkp`).

## Problem

Runner launches agent CLIs by bare catalog names (`claude`, `codex`) resolved inside the child PTY against a PATH composed at spawn time (`launch.rs:78`). The PATH's best ingredient comes from a login-shell probe (`shell_path.rs:84`) that is fragile in four ways:

1. **It blocks startup.** `resolve_login_shell_env()` runs synchronously inside Tauri `setup` (`lib.rs:171`) before the first window paint; worst case adds ~2.5 s (2 s deadline + 500 ms drain grace) to every cold start.
2. **It is all-or-nothing with no memory.** On timeout the app silently falls to launchd's stripped env — one `log::warn` no user ever sees. A launch that timed out loses the PATH a previous launch captured successfully. Slow zsh/Oh My Zsh inits are exactly the setups that also rely on version managers, so the users most likely to time out are the users most hurt by the fallback.
3. **The fallback seed is thin.** `FALLBACK_CLI_DIRS` (`launch.rs:32`) covers Homebrew, `~/.local/bin`, `~/.cargo/bin`, `~/.npm-global/bin` — but not the version-manager shim dirs (mise, asdf, volta, fnm, nvm, pnpm, bun) where agent CLIs increasingly live.
4. **Nothing is observable or fixable.** Runner never resolves the executable itself, so "not installed", "probe timed out", and "PATH missing one dir" all present identically: `command not found` printed by the shell into a freshly spawned dead PTY. There is no refresh short of relaunching the app, and no override of any kind.

Reference point: Orca converged on the same architecture-level answers (probe `$SHELL -ilc`, no shell/PATH configuration on POSIX, per-agent executable override as the escape hatch) but hardened the probe: non-blocking startup, 5 s timeout, typed failure reasons, sync seed covering version-manager dirs, on-demand refresh from their settings pane. This impl adopts those refinements without adopting their agent-inside-login-shell spawn model — Runner keeps the agent as PTY root (clean exit observation, byte-quiescence idle detection).

## Key Decisions

1. **Discovery becomes a background service with a persisted last-known-good result.** `resolve_login_shell_env` grows into a small discovery module that returns a structured `DiscoveryResult { shell, outcome, duration, env }` where `outcome ∈ {Ok, Timeout, SpawnError, EmptyCapture, NoShell}`. At startup, `setup` seeds the manager synchronously from the last-known-good snapshot persisted in `_app_state` (key `login_shell_env_lkg`, JSON: env + shell + captured_at) — a fast SQLite read — then fires the real probe on a background thread. On success the probe swaps the live env, persists the new snapshot, and emits `runtime/changed`. On failure the last-known-good stays in effect and the failure reason is kept for the settings pane. Timeout goes from 2 s to 5 s — it no longer costs startup anything. Never re-probe implicitly beyond launch; explicit refresh only.
2. **`SessionManager.shell_env` becomes shared and swappable.** Today it is a plain owned field set once by `SessionManager::new` (`manager/mod.rs:606,714`). It becomes `Arc<RwLock<LoginShellEnv>>` (std `RwLock`; reads clone, writes are rare). The discovery service holds the same handle. `base_spawn_spec` (`spawn.rs:66,89`) reads through the lock. No spawn ever blocks on discovery: if the probe is still pending, spawn proceeds with the seeded env (last-known-good or default) exactly as today.
3. **Runner resolves executables itself, in Rust, by walking the composed PATH.** New resolver: split the same PATH `compose_path` produces for a direct chat (no shim/bundled dirs), test each `dir/<command>` for regular-file + executable bit (`0o111`), first hit wins. No shell involvement — aliases and functions can mask binaries from `command -v`, and the child never sees them anyway (same reasoning Orca documents in `posix-command-path-lookup.ts`). This resolution is what the settings pane displays and what spawn substitutes, so "what settings shows" and "what spawn does" cannot drift.
4. **Fatter sync seed.** Extend `FALLBACK_CLI_DIRS` composition with the version-manager dirs: `~/.local/share/mise/shims`, `~/.asdf/shims`, `~/.volta/bin`, `~/.bun/bin`, `~/.deno/bin`, `~/Library/pnpm`, `~/.local/share/fnm/aliases/default/bin`, and enumerated `~/.nvm/versions/node/*/bin` (newest first, only when the dir exists). Rationale: agent CLIs are typically `#!/usr/bin/env node` scripts — the shim dirs cover both the CLI and the `node` it needs. Keep the list curated and home-relative; dedup already happens in `compose_path`.
5. **Overrides live in `_app_state`, one JSON key, no migration.** `runtime_overrides` → `{"claude-code": "/abs/path", ...}`. `_app_state` (`db.rs:191`) is the existing backend KV store and overrides are exactly key/value app state. Set validates absolute + regular file + executable and returns a structured error for inline display; empty/None clears back to auto. localStorage is never the source of truth (spec 37 requirement — all windows and all Rust spawn paths must agree).
6. **Effective-command precedence is one function, applied at the existing choke points.** `effective_runtime_command(runtime) -> {command, source}` with precedence: valid override → detected absolute path → bare catalog name (only while discovery has produced nothing — the child PATH then does its best, as today). A vanished override (file deleted since set) is skipped for spawn — fall through to detected — and surfaced as the Invalid state in the pane; spawning something that works beats failing loudly on a stale preference, but the pane must not pretend the override is in effect. Substitution applies only when the stored command equals the runtime's catalog default: `runtime_direct_runner` (`manager/mod.rs:1153`, already the single choke point for runtime-only chats and runner-less resume), `resolve_runtime_override` (`manager/mod.rs:1119`, slot overrides), and the runner-backed spawn/resume paths where `runner.command` is loaded. A legacy runner row with a genuinely custom command is never touched.
7. **Not-found fails before the PTY, with a pointer.** When discovery has completed and neither override nor detection resolves the runtime, spawn commands return a structured error naming the runtime and directing to Settings → Agents, instead of forking a PTY that prints `command not found`. If discovery is still pending, spawn proceeds (decision 2) — never block or reject on an unfinished probe.
8. **Sessions keep recording the effective command; resume re-validates it.** `spawn_direct_inner` already stamps `sessions.agent_command` (`spawn.rs:755-770`) and runner-less resume already honors it (`spawn.rs:1067-1081` → `runtime_direct_runner(runtime, snap.agent_command)`). With substitution, the stamped value becomes an absolute path. Resume adds one check: if the stored command is an absolute path that no longer exists, re-resolve through `effective_runtime_command` instead of failing on a dead path (spec 37: "resume uses the same executable unless the file no longer exists").
9. **New Tauri surface follows the `<domain>/<verb>` event convention — and actually emits.** Commands: `runtime_status_list` (per-runtime: catalog fields + detected path + override + state + probe diagnostics), `runtime_set_override`, `runtime_clear_override`, `runtime_refresh` (force re-probe + re-resolve). All mutations and every probe completion emit `runtime/changed` with an empty payload; listeners refetch (the `runner/changed` pattern from `Runners.tsx:85`). Note the counter-precedent to avoid: the Tauri `runner_create/update` handlers mutate without emitting — do not copy that.
10. **The pane is read-mostly and mirrors the design nodes exactly.** New `AgentsPane` under Integrations (above MCP, icon `bot`, PaneKey `"agents"`): a shell-environment card (shell, probe outcome, duration, refresh) and one row per built-in runtime with the six states from `Spec — Agent runtime row states`: detected, override, not-found, checking, probe-timed-out, invalid-override. No shell picker, no PATH editor — the probe stays configuration-free.

## Goals

- Cold start never waits on the shell probe; a probe timeout on this launch cannot lose the PATH captured on a previous launch.
- Settings → Agents shows, for each built-in runtime, the absolute executable Runner will actually spawn, or a state that distinguishes "not installed" from "shell probe failed" from "override is broken".
- A user whose CLI lives behind a version manager can fix Runner without touching their shell: install → Refresh → detected, or Browse → override.
- Spawning a runtime whose executable cannot resolve fails before the PTY with actionable copy, not `command not found` in a dead terminal.
- A runner row with a custom command, and an existing session's recorded command, behave exactly as before.

## Non-Goals

- User-defined custom runtimes (registry-as-data, capability profiles) — cut from #279's scope for now; nothing here may preclude them, and nothing here builds them.
- Shell selection or PATH editing UI. `$SHELL` + fallback is the probe; the override is the escape hatch.
- Accepting aliases, shell functions, or non-absolute overrides.
- Changing the spawn model (agent stays PTY root; no login-shell wrapper à la Orca).
- Windows support beyond keeping the existing unsupported state.
- Unifying the hardcoded frontend `RUNTIME_OPTIONS` mirror (`src/components/ui/runtimes.ts`) with the backend catalog. Its `defaultCommand` values are the bare catalog names, which is precisely what new runner rows should keep storing (no frozen absolute paths) — leave it.

## Implementation Phases

### Phase 1 — backend: discovery service + shared env

- `shell_path.rs`: restructure around `DiscoveryResult` (shell, outcome enum, duration, `LoginShellEnv`); keep the marker probe and parser as-is; timeout 2 s → 5 s. Log line per probe: shell, duration, outcome, nothing else from the env (spec 37 diagnostics rule).
- `db.rs` / small store helper: read/write the `login_shell_env_lkg` and `runtime_overrides` keys in `_app_state`.
- `session/manager/mod.rs`: `shell_env` → `Arc<RwLock<LoginShellEnv>>`; constructor takes the handle; `base_spawn_spec` and `compose` read through it.
- `lib.rs` setup: synchronous seed from last-known-good → construct manager → spawn background probe thread → on completion swap env + persist + emit `runtime/changed`. Startup no longer blocks (delete the inline resolve at `:171`).
- `launch.rs`: extend the fallback seed per decision 4 (a `version_manager_dirs(home)` helper feeding `compose_path`; nvm enumeration behind an exists-check).
- Tests: LKG round-trip; timeout keeps prior env; probe outcome mapping; seed dirs dedup and ordering (shell PATH still wins over seed); manager reads swapped env on next spawn.

### Phase 2 — backend: resolution, overrides, spawn integration

- New `runtime_status` module (or extend `router/runtime.rs`): the PATH-walk resolver (decision 3), `effective_runtime_command` (decision 6), and per-runtime status assembly for the pane.
- Override store: get/set/clear with validation (absolute, regular file, executable bit) and structured validation errors.
- Choke-point integration: `runtime_direct_runner` (`manager/mod.rs:1153`) resolves through `effective_runtime_command` when no explicit command is passed; `resolve_runtime_override` (`:1119`) substitutes the effective command instead of raw `def.command`; runner-backed spawn/resume substitute only when `runner.command == catalog default`; resume re-validates a stored absolute `agent_command` (decision 8).
- Pre-spawn not-found error (decision 7) returned from `session_start_runtime`, `session_start_direct`, mission spawn, and resume paths.
- Watch item: `command_line_matches_recorded_agent` (`pty_runtime.rs:1094`) must keep matching when `agent_command` is an absolute path — add a test against a `ps`-style command line for both bare and absolute spawn forms.
- Watch item: the launch script must quote an absolute command path containing spaces the same way it already quotes args.
- Tests: precedence (override > detected > catalog); vanished-override fallthrough; custom runner command untouched; catalog-default runner substituted; resume with live stored path (no re-resolution), with dead stored path (re-resolves), runner-backed vs runtime-only; not-found error carries runtime name; probe-pending spawn proceeds with bare name.

### Phase 3 — Tauri commands + frontend Agents pane

- `commands/runtime.rs`: `runtime_status_list`, `runtime_set_override`, `runtime_clear_override`, `runtime_refresh`; register in `lib.rs`; every mutation and probe completion emits `runtime/changed`.
- `src/lib/api.ts`: `api.runtime.status()/setOverride()/clearOverride()/refresh()` + types.
- `src/components/settings/AgentsPane.tsx`: shell-environment card + per-runtime rows per the design nodes; `PaneHeader`/`SettingsCard`/`SettingsRow` primitives where they fit, `McpPane`'s row/status-line shapes for the richer rows; Browse via `@tauri-apps/plugin-dialog` file picker; inline validation errors from `runtime_set_override`; listen on `runtime/changed`.
- `src/pages/SettingsPage.tsx`: PaneKey `"agents"`, `PANES` entry (label "Agents", icon `Bot`), Integrations group before `mcp` (`:102`).
- Frontend checks: `pnpm exec tsc --noEmit`, `pnpm run lint`.

### Phase 4 — diagnostics, docs, verification

- Startup + spawn log lines that make timeout / not-found / active-override distinguishable in `runner_logs_reveal` output.
- `docs/arch/`: update the login-shell environment capture description (background probe, LKG, refresh) and document the command precedence.
- Spec 37 verification checklist pass; `cargo fmt` + `clippy` + `cargo test --workspace`.

## Verification

Unchecked items below require manual app verification.

- [ ] Cold start paints without waiting on the probe; probe result lands afterwards and the pane updates via `runtime/changed`.
- [ ] Kill the probe artificially (bogus slow rc) → previous launch's PATH still spawns agents; pane shows probe-timed-out with duration.
- [x] `runtime_status_list` detected path equals what a spawned PTY actually execs (same composed PATH).
- [ ] Override set/clear round-trips through `_app_state` and takes effect for direct chat, runner-backed chat (default command), mission slot, and resume — without app restart, in all windows.
- [x] Runner row with custom command spawns that command unchanged.
- [x] Vanished override: spawn falls back to detected; pane shows invalid state.
- [x] Not-found runtime: spawn fails pre-PTY naming the runtime; no dead terminal.
- [ ] Full checklist in spec 37 § Verification.
