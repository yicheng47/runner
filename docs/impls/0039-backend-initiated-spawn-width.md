# Backend-initiated spawn width

## Status

Planned. Tracking issue [#367](https://github.com/yicheng47/runner/issues/367).

## Problem

Spawns the frontend initiates carry a measured grid; spawns the backend initiates carry nothing and fall to `DEFAULT_PTY_SIZE` (`session/manager/mod.rs:52`, 80×24). Two paths reach that state today, both through MCP:

- `mission_start_impl` (`commands/mission.rs:488-494`) hardcodes `None` for `initial_size`. Its only caller is the MCP tool (`mcp/tools/mission.rs:436`); the UI's Start Mission modal goes through `mission_start_impl_with_size` (`:867`) with real dims.
- `session_start_direct` via MCP (`mcp/tools/session.rs:47-55`) passes `None, None` for `cols`/`rows`. `session_start_direct_impl` (`commands/session.rs:715-724`) accepts them; the UI supplies them, the MCP tool does not.

Forking at 80 also seeds `last_pty_cols = Some(80)`. When the pane is first opened and pushes its real width, the cols-gate fires (`session/manager/output.rs:538-546`):

```rust
let cols_changed = { … state.last_pty_cols != Some(cols) … };
if cols_changed && runtime_clears_on_resize(session_id, pool) {
    self.purge_output_buffer_keep_modes(session_id);
}
```

`runtime_clears_on_resize` (`:796`) covers claude-code, codex, qoder, and trae — every agent runtime. So the entire transcript the agent produced before its tab was first visited is discarded, and snapshot replay has nothing to restore. The longer the session ran unobserved, the more is lost.

The tree already documents this failure, but only as fixed for the *respawn* paths — `commands/mission.rs:1413-1416` and `MissionWorkspace.tsx:629-630` describe exactly this mechanism. Initial spawn via MCP was left behind.

This is not [#366](https://github.com/yicheng47/runner/issues/366)/[#363](https://github.com/yicheng47/runner/issues/363). Those are about the *estimate* feeding launch auto-resume, where the resulting width is wrong. Here the width ends up correct and the history is gone. Shared mechanism, different cause, different fix site.

### Why an estimate cannot fix this

The gate is exact equality on cols. An estimate that lands one column off purges exactly as thoroughly as no estimate at all. There is no partial credit, so a computed approximation buys nothing — it must match the destination pane exactly or it is wasted work.

That rules out deriving the grid backend-side from window geometry (the issue's option 2). It would mean a second copy of the chrome constants in Rust — the duplication that already bites `runtimeClearsOnResize` (`output.rs:770` / `RunnerTerminal.tsx:168`) — in exchange for a number that is *almost* right, which is worth nothing here.

The only source of an exactly-correct destination width is a real measurement of a real pane. So: reuse the last one.

## Key Decisions

1. **Cache a measured grid; never compute one.** The frontend writes the mission pane grid and the chat pane grid to the backend as it measures them. Backend-initiated spawns read the cache when the caller supplied no size. Rust gains no knowledge of chrome constants, header heights, or panel widths.
2. **Persist it in `_app_state`, not in memory.** Follow the established shape in `db.rs`: private `app_state_get`/`app_state_set` plus a typed `pub fn` pair per key, exactly as `login_shell_env_lkg` (`:218-232`) and `runtime_overrides` (`:234-260`) do. Persisting is what covers the first MCP-started mission after a cold launch — an in-memory cache would be empty precisely when the app hasn't rendered a pane yet.
3. **Two keys, because they are two different boxes.** `missionPaneAreaBox` (`terminalSizing.ts:180`) subtracts the runners rail, the topbar, and the slot-tab strip. `chatPaneAreaBox` (`:210`) subtracts the side panel and is then divided by `paneBoxForSession`. A mission spawn must not read a chat measurement. Store `mission` and `chat` grids separately and have each spawn path read its own.
4. **Write from where the grid is already known.** The measurement is whatever the pane pushes to `session.resize` — same value, same moment. Do not add a separate measuring pass; do not measure on a timer.
5. **No cached grid means 80×24, unchanged.** A fresh install whose first action is an MCP `mission_start` still loses that first session's early scrollback. That is the honest residual: guessing would not avoid the purge (see above), and it self-heals as soon as any pane has rendered once. Do not add a fallback estimate to paper over it.
6. **Do not touch the purge gate.** It is correct given genuinely-80-col bytes: replaying absolute-positioned 80-col frames into a wide grid is the box-drawing shredding impl 0020 / [#306](https://github.com/yicheng47/runner/issues/306) removed. This fix removes the cause, not the safeguard.
7. **Optional explicit dims on the MCP tools are a non-goal.** A caller that knew the geometry could pass it (the issue's option 3), but MCP callers are agents with no view of the window, so the parameter would sit unused while widening the tool surface.

## Open questions

- **Staleness.** The cache is not invalidated when the window resizes with no pane mounted, so a spawn can read a grid that no longer matches. The cost of being wrong is exactly today's behavior — one purge — so this is not worth a watcher. Confirm the reasoning holds rather than adding invalidation.
- **Rows.** The gate is cols-only, so a stale rows value costs nothing. Both are stored because the spawn takes a pair; no accuracy claim is made about rows.
- **Multi-window** (impl 0018). Last writer wins across windows. Acceptable while windows are near-identical; note it rather than solve it.
- **Whether the mission grid should be per-project.** Different projects may sit at different rail widths. Probably not worth it — the rail width is global. Confirm.

## Goals

- A mission started through MCP and left unopened for several minutes shows its full transcript when its slot tab is first visited.
- A direct chat started through MCP behaves the same.
- Sessions started from the UI are byte-for-byte unaffected.

## Non-Goals

- Changing the cols-gate purge, `runtime_clears_on_resize`, or snapshot replay.
- Anything in #363/#366/impl 0038's launch auto-resume estimate.
- Recovering scrollback already purged by shipped versions.
- Adding `cols`/`rows` parameters to the MCP tool schemas.

## Implementation Phases

### Phase 1 — persisted pane-grid cache

- Add `_app_state` keys for the last measured mission grid and chat grid, with typed getters/setters in `db.rs` mirroring `login_shell_env_lkg`.
- Add a command the frontend calls with `{ surface, cols, rows }`.
- Unit-test the round trip, including the absent-key case returning `None`.

### Phase 2 — write side

- Have the mission workspace and the chat surface record their measured grid alongside the resize they already push.
- Do not introduce a new measurement path; reuse the value being sent to `session.resize`.

### Phase 3 — read side

- `mission_start_impl`: read the cached mission grid and pass it as `initial_size` instead of the hardcoded `None`.
- MCP `session_start_direct`: read the cached chat grid and pass it as `cols`/`rows`.
- Both fall back to today's behavior when the cache is empty.
- Test that an explicit caller-supplied size still wins over the cache.

### Phase 4 — validation

- `cargo fmt --check`, `cargo clippy --workspace`, `cargo test --workspace`, `pnpm exec tsc --noEmit`, `pnpm run lint`, `pnpm test`.

## Verification

Automated:

- [ ] Cache round-trips through `_app_state` and survives a pool reopen.
- [ ] Absent key yields `None`; spawn then uses `DEFAULT_PTY_SIZE`.
- [ ] `mission_start_impl` with a cached grid forks at that grid; with an explicit size the explicit value wins.
- [ ] Mission and chat caches do not read each other.
- [ ] A resize to the same cols the session forked at does not purge (the gate's no-op case is what this fix relies on).

Manual (Jason smoke-tests):

- [ ] Open a mission workspace once so a grid is cached. Start a mission through MCP, leave the slot tab closed for a few minutes, then open it — full transcript present, no truncation at the top.
- [ ] Same for an MCP-started direct chat.
- [ ] Start a mission from the UI modal — unchanged.
- [ ] Quit, relaunch, immediately start an MCP mission without opening any pane — still forks at the persisted grid.
