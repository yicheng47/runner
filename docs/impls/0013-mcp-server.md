# MCP Server

> Implements [#40](https://github.com/yicheng47/runner/issues/40).
> Spec: [docs/features/03-mcp-server.md](../features/03-mcp-server.md).
> Reference: [quill PR #211](https://github.com/yicheng47/quill/pull/211) — rmcp 1.7 MCP implementation in a Tauri app.

## Context

The only way to set up a Runner workspace today is clicking through the app UI. Issue #40 asks for an MCP server that exposes the existing CRUD surface (crews, runners, slots) as MCP tools so external agents (Claude Code, Codex) can configure Runner from the terminal.

The quill project shipped MCP with rmcp 1.7 (PR #211). Quill started with in-process HTTP then pivoted to stdio because MCP clients overwhelmingly expect stdio. Runner faces an extra constraint: MCP mutations need the live `AppHandle` to emit Tauri events for real-time UI updates, so a subprocess that opens SQLite directly (quill's approach) won't work.

### Transport pivot: HTTP → Unix socket + stdio bridge

The original spec called for HTTP on a TCP port. This has a practical problem: port conflicts. Any fixed or configurable port can clash with whatever else the user runs locally.

**Solution**: Unix domain socket + stdio bridge binary.

```
Claude Code ←stdio→ runner mcp (bundled CLI) ←Unix socket→ Runner.app
```

- **Runner.app** listens on a Unix socket at `$APPDATA/runner/mcp.sock` — deterministic path, no port, no conflicts.
- **`runner mcp`** subcommand (bundled CLI, already shipped in `cli/`) bridges stdio ↔ Unix socket. Claude Code spawns it as a subprocess.
- The app handles all MCP requests in-process with full `AppState` access, so Tauri event emission works.
- User config is just `"command": "runner", "args": ["mcp"]` — no URL to copy.

## Architecture

**Transport**: stdio (client-facing) bridged to a Unix domain socket (app-internal IPC).

**Crate**: `rmcp` 1.7. The app-side server uses `transport-async-rw` over a `UnixStream`. The CLI bridge reads stdio and forwards over the socket.

**Module layout**:
```
src-tauri/src/mcp/
├── mod.rs          — McpHandle lifecycle (start/stop)
├── server.rs       — RunnerMcpHandler, ServerHandler impl, tool_router()
├── state.rs        — McpState (Arc<DbPool> + AppHandle)
└── tools/
    ├── mod.rs      — module registry
    ├── crew.rs     — 5 crew CRUD tools
    ├── runner.rs   — 6 runner CRUD tools
    └── slot.rs     — 6 slot CRUD tools

cli/src/
└── mcp.rs          — `runner mcp` subcommand: stdio ↔ Unix socket bridge
```

**State flow**: MCP tool → `self.state.db.get()` → call existing pure function from `commands::*` → emit `*/changed` Tauri event via `self.state.app_handle` → return JSON result.

**Lifecycle**: The Unix socket listener starts unconditionally in `setup()` (no toggle needed — a socket file has no port-conflict risk and no security surface beyond what filesystem permissions already provide). On `ExitRequested`, the listener shuts down and the socket file is removed.

## Phase 1 — Scaffolding

### Dependencies

**`src-tauri/Cargo.toml`** (replaces the current HTTP deps):
```toml
rmcp = { version = "1.7", features = ["server", "transport-async-rw"] }
schemars = "1"
tokio = { version = "1", features = ["net", "rt", "io-util"] }
```

Drop `axum` and `tokio-util` — no HTTP server needed.

**`cli/Cargo.toml`** — add:
```toml
tokio = { version = "1", features = ["net", "rt", "io-util", "io-std"] }
```

### `mcp/state.rs`

```rust
#[derive(Clone)]
pub(crate) struct McpState {
    pub db: Arc<db::DbPool>,
    pub app_handle: AppHandle,
}
```

### `mcp/server.rs`

- `RunnerMcpHandler` struct carrying `McpState` (mirrors quill's `QuillMcpHandler`).
- `tool_router()` aggregator — empty in Phase 1, each Phase 2 tool file adds a `r.merge(Self::<name>_router())` line.
- `#[tool_handler] impl ServerHandler` with `get_info()` returning server name `"runner"`, version from `CARGO_PKG_VERSION`, `enable_tools()` capability.
- `accept_connection(stream: UnixStream, state: McpState)` — takes one accepted Unix socket connection and serves MCP over it using rmcp's async read/write transport. One connection = one MCP session.

### `mcp/mod.rs`

```rust
pub struct McpHandle { ... }
```

- `start(socket_path, state)` — spawns a tokio task that `loop { accept }` on the `UnixListener`, calling `accept_connection` for each client. Stores a `CancellationToken` + `JoinHandle`.
- `stop()` — cancels the token, removes the socket file.
- `socket_path()` — returns the path for the CLI bridge to connect to.

### `lib.rs` changes

- Add `mod mcp;`
- Add `pub mcp: Arc<mcp::McpHandle>` to `AppState`
- In `setup()`: start the Unix socket listener at `app_data_dir.join("mcp.sock")`. Always-on — no toggle needed.
- On `RunEvent::ExitRequested`: call `state.mcp.stop()` to clean up the socket file.
- Remove the `mcp_enable`/`mcp_disable`/`mcp_status` Tauri commands (no longer needed — the socket is always on). Replace with a single `mcp_config_snippet` command that returns the JSON snippet for the user's `~/.claude.json`.

### `cli/src/mcp.rs` — stdio ↔ Unix socket bridge

The `runner mcp` subcommand:
1. Resolves the socket path: `$APPDATA/runner/mcp.sock` (same path the app writes to).
2. Connects to the Unix socket via `UnixStream::connect`.
3. Bridges stdin/stdout ↔ the socket stream (bidirectional byte copy).
4. Exits when either side closes.

This is a thin pipe — no MCP parsing, no state. The rmcp protocol flows end-to-end through the bridge transparently.

### `cli/src/main.rs` changes

Add `Mcp` variant to the `Cmd` enum:
```rust
/// Serve MCP over stdio, bridging to the running Runner app.
Mcp,
```

Dispatch in `main()`:
```rust
Cmd::Mcp => mcp::run(),
```

### Verification (Phase 1)

- App boots; log shows `mcp: listening on /path/to/mcp.sock`
- `runner mcp` connects and completes the MCP handshake over stdio
- `echo '<initialize JSON>' | runner-cli mcp` returns a valid handshake response
- App quit removes the socket file
- Adding `{"command": "runner", "args": ["mcp"]}` to `~/.claude.json` → `claude mcp list runner` shows empty tool list

## Phase 2 — CRUD tools

### Tool pattern

Each tool file follows the quill pattern (see `quill/src-tauri/src/mcp/tools/bookmarks.rs` for the minimal reference):

```rust
use rmcp::{tool, tool_router, ErrorData};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ToolNameArgs {
    /// Doc comment → MCP parameter description.
    pub field: String,
}

#[tool_router(router = name_router, vis = "pub(crate)")]
impl RunnerMcpHandler {
    #[tool(description = "...")]
    pub async fn tool_name(
        &self,
        Parameters(args): Parameters<ToolNameArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let conn = self.state.db.get()
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        let result = commands::entity::operation(&conn, ...)
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?;
        // For mutations:
        self.state.app_handle.emit("entity/changed", ()).ok();
        Ok(CallToolResult::success(vec![Content::json(&result)?]))
    }
}
```

### `mcp/tools/crew.rs` — 5 tools

| Tool | Wraps | Event |
|------|-------|-------|
| `crew_list` | `commands::crew::list` | — |
| `crew_get` | `commands::crew::get` | — |
| `crew_create` | `commands::crew::create` | `crew/changed` |
| `crew_update` | `commands::crew::update` | `crew/changed` |
| `crew_delete` | `commands::crew::delete` | `crew/changed` |

Args for `crew_create`: reuse `CreateCrewInput` (already derives `Deserialize`; add `JsonSchema`). Same for `UpdateCrewInput`.

### `mcp/tools/runner.rs` — 6 tools

| Tool | Wraps | Event |
|------|-------|-------|
| `runner_list` | `commands::runner::list` | — |
| `runner_get` | `commands::runner::get` | — |
| `runner_get_by_handle` | `commands::runner::get_by_handle` | — |
| `runner_create` | `commands::runner::create` | `runner/changed` |
| `runner_update` | `commands::runner::update` | `runner/changed` |
| `runner_delete` | `commands::runner::delete` | `runner/changed` |

### `mcp/tools/slot.rs` — 6 tools

| Tool | Wraps | Event |
|------|-------|-------|
| `slot_list` | `commands::slot::list` | — |
| `slot_create` | `commands::slot::create` | `slot/changed` |
| `slot_update` | `commands::slot::update` | `slot/changed` |
| `slot_delete` | `commands::slot::delete` | `slot/changed` |
| `slot_set_lead` | `commands::slot::set_lead` | `slot/changed` |
| `slot_reorder` | `commands::slot::reorder` | `slot/changed` |

### `server.rs` — wire routers

```rust
pub(crate) fn tool_router() -> ToolRouter<Self> {
    let mut r = ToolRouter::new();
    r.merge(Self::crew_router());
    r.merge(Self::runner_router());
    r.merge(Self::slot_router());
    r
}
```

### Frontend event listeners

Add Tauri event listeners for `crew/changed`, `runner/changed`, `slot/changed` to trigger data re-fetches. These events are only emitted by MCP mutations — the existing Tauri command path refreshes on the invoke response.

Key files:
- Crew list page / sidebar: listen for `crew/changed` → re-fetch crew list
- Runner list page: listen for `runner/changed` → re-fetch runner list
- Crew detail / slot panel: listen for `slot/changed` → re-fetch slot list

### `JsonSchema` on existing input types

The existing `CreateCrewInput`, `UpdateCrewInput`, `CreateRunnerInput`, `UpdateRunnerInput`, `UpdateSlotInput` structs need `#[derive(JsonSchema)]` added. The existing `Serialize`/`Deserialize` derives stay; `JsonSchema` layers on top.

### Verification

- `claude mcp list runner` shows all 17 tools
- `claude mcp call runner crew_create '{"name":"test"}'` creates crew; Runner sidebar updates live
- `cargo test --workspace` covers tool wrappers
- `pnpm exec tsc --noEmit` and `pnpm run lint` clean

## Phase 3 — Pencil design

Design the MCP settings pane in `design/runners-design.pen` before writing any frontend code. Covers:
- Toggle for enabling/disabling MCP (controls whether the socket listener runs)
- "Copy Claude Code config" / "Copy Codex config" buttons
- Connection status indicator

Phase 4 implements the approved design.

## Phase 4 — Settings UI

### New pane in `SettingsModal.tsx`

Add `"mcp"` to the `Pane` union and `PANES` array. Implement to match the Phase 3 Pencil design.

Pane contents:
- **Copy buttons**: "Copy Claude Code config" and "Copy Codex config" — copy the snippet to clipboard. The snippet points at the bundled `runner` binary path with `["mcp"]` args.
- **Status indicator**: shows whether the socket listener is active.

### Verification

- Copy button → paste into `~/.claude.json` → `claude mcp list runner` works

## Files to create

| File | Purpose |
|------|---------|
| `src-tauri/src/mcp/mod.rs` | McpHandle lifecycle (Unix socket listener) |
| `src-tauri/src/mcp/server.rs` | RunnerMcpHandler + ServerHandler impl |
| `src-tauri/src/mcp/state.rs` | McpState struct |
| `src-tauri/src/mcp/tools/mod.rs` | Tool module registry |
| `src-tauri/src/mcp/tools/crew.rs` | 5 crew tools |
| `src-tauri/src/mcp/tools/runner.rs` | 6 runner tools |
| `src-tauri/src/mcp/tools/slot.rs` | 6 slot tools |
| `cli/src/mcp.rs` | `runner mcp` stdio ↔ Unix socket bridge |

## Files to modify

| File | Change |
|------|--------|
| `src-tauri/Cargo.toml` | Replace HTTP deps with Unix socket deps |
| `src-tauri/src/lib.rs` | Start socket listener in setup(), stop on ExitRequested |
| `src-tauri/src/commands/mod.rs` | Update `pub mod mcp;` |
| `src-tauri/src/commands/mcp.rs` | Replace enable/disable with `mcp_config_snippet` |
| `src-tauri/src/commands/crew.rs` | Add `JsonSchema` derive to input types |
| `src-tauri/src/commands/runner.rs` | Add `JsonSchema` derive to input types |
| `src-tauri/src/commands/slot.rs` | Add `JsonSchema` derive to input types |
| `cli/Cargo.toml` | Add tokio dep |
| `cli/src/main.rs` | Add `Mcp` subcommand variant + dispatch |
| `src/components/SettingsModal.tsx` | New MCP pane |
| `src/lib/settings.ts` | MCP storage keys + helpers |
| Pages showing crews/runners/slots | Event listeners for live refresh |
