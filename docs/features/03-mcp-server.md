# 03 — MCP server

> Tracking issue: [#40](https://github.com/yicheng47/runner/issues/40)

## Motivation

Today the only way to set up a Runner workspace is by clicking through
the app: create a crew, create runners, place slots, set lead. That's
fine for one or two crews; it's tedious when the user wants to script
"give me a five-runner crew with these handles, these prompts, these
models" — exactly the kind of task they'd hand to Claude Code or Codex
running in their terminal.

Runner already exposes its full CRUD surface as Tauri commands. Wrap
that surface as MCP tools and the user's external Claude Code / Codex
becomes a first-class client of the app: "create a crew called X,
populate it with these three runners, make Y the lead" works as a
single-turn tool call.

Pairs naturally with the bundled `runner` CLI that crew agents use to
talk to each other (arch §5.3). Same idea — make Runner scriptable from
outside — different audience: the bundled CLI is for *in-mission*
workers, the MCP surface is for *configuring* the app from a personal
agent.

## Scope

### In scope (v1)

- **In-app HTTP MCP server**, bound to `127.0.0.1` only, listening on a
  configurable port (default `7654`). The server is part of the
  Runner.app process — no separate bridge binary, no external daemon.
  Lifecycle is tied to the toggle: start when enabled, stop when
  disabled, stop on app shutdown.
- **Tool surface — CRUD only**, mirroring the existing Tauri commands
  one-for-one so MCP and UI stay isomorphic:
  - **Crews:** `crew_list`, `crew_get`, `crew_create`, `crew_update`,
    `crew_delete`
  - **Runners:** `runner_list`, `runner_get`, `runner_get_by_handle`,
    `runner_create`, `runner_update`, `runner_delete`
  - **Slots:** `slot_list`, `slot_create`, `slot_update`, `slot_delete`,
    `slot_set_lead`, `slot_reorder`
  - Input / output schemas are derived from the existing serde types in
    `src-tauri/src/commands/{crew,runner,slot}.rs` (`Runner`, `Crew`,
    `Slot`, etc.) so the MCP shapes don't drift from the UI shapes.
- **Mutations emit the same Tauri events** they already do (`crew/changed`,
  `runner/changed`, `slot/changed`). MCP-driven changes appear in the
  open UI live, with no refresh.
- **Settings page entry** under a new "MCP server" section:
  - Toggle: "Enable MCP server" (default off — opt-in).
  - Read-only field showing the current URL (`http://127.0.0.1:7654/mcp`).
  - "Copy Claude Code config" button — copies the exact JSON snippet
    the user pastes into `~/.claude.json` (or wherever Claude Code's
    MCP config lives).
  - "Copy Codex config" button — copies the equivalent TOML snippet
    Codex uses (`mcp_servers.runner` block in the Codex config).
  - Persisted in `localStorage` (toggle + port) — same place as other
    UI prefs. Server reads on app start; toggle change restarts the
    server in place.

### Out of scope (deferred)

- **Mission control via MCP.** Starting / stopping missions, posting
  human messages, reading event logs. Big surface area; needs its own
  spec round, especially around concurrent humans (one driving from
  the UI, one from CC). v1 is pure config CRUD.
- **Authentication / per-tool allow-lists.** Localhost-only is the
  authn boundary for v1. If we ever bind to a non-loopback interface
  or remote tunnel scenario, that spec adds a token. Per-tool
  allow-lists (e.g. "let CC create runners but not delete them")
  would be useful but doubles the settings UI; defer.
- **Stdio MCP transport.** Some Codex setups prefer stdio. Possible to
  add a `runner mcp serve` subcommand on the bundled CLI later if
  there's demand. Not v1.
- **Resource / prompt support.** MCP also supports `resources` (read
  arbitrary URIs) and `prompts` (canned prompt templates). Tools alone
  cover the user's stated CRUD goal; defer.
- **Multiple Runner.app instances.** Two apps on different ports, with
  the user picking which one CC talks to, would need a registry. v1
  allows only one running instance to bind the port; the second logs
  "port in use" and disables.

### Key decisions

1. **HTTP transport, not stdio.** "Runner.app should expose MCP server"
   in the user's words → the server must live in-app. Stdio transport
   would force a separate bridge binary that CC spawns and forwards
   over a socket; HTTP transport (which Claude Code natively supports
   via `"type": "http"`) lets CC connect to the in-app server
   directly. One fewer process, one fewer point of failure.
2. **`127.0.0.1`-only bind.** No localhost-vs-external setting. The
   threat model: anyone with shell access to the user's machine
   already has shell access to the user's machine. Anything beyond
   that warrants a real auth story, not a checkbox.
3. **CRUD parity with the Tauri command layer, not a parallel API.**
   Each MCP tool wraps the same underlying free function the Tauri
   command wraps (`commands::crew::create(&conn, ...)`, etc.). One
   set of validation, one set of events. Avoids the
   "MCP says it succeeded but the UI never noticed" class of bug.
4. **Toggle is opt-in, default off.** First launch shouldn't open a
   port without the user knowing. The user has to flip the toggle in
   Settings before CC can connect.
5. **Port is configurable but defaulted to 7654.** Letting the user
   pick avoids a clash with whatever else they're running on common
   ports (3000, 8080). Default `7654` is uncommon enough to mostly
   work out of the box; "RUN" on a phone keypad if you squint.
6. **Crate choice: `rmcp` (official Anthropic Rust SDK).** It tracks
   the spec, has Streamable HTTP transport built in, and matches the
   project's "use the official thing where one exists" preference
   (already true for `tauri-plugin-updater`, `portable-pty`, etc.).

## Implementation phases

### Phase 1 — MCP server scaffolding

- New module `src-tauri/src/mcp/` with:
  - `mod.rs` — server lifecycle (start, stop, restart).
  - `tools.rs` — tool registration + handlers.
  - `state.rs` — shared state passed to handlers (DbPool, AppHandle for
    event emit).
- Add `rmcp` and HTTP runtime deps to `src-tauri/Cargo.toml`. Pick
  whichever HTTP server `rmcp` integrates with cleanly (likely `axum`
  + `tokio`).
- Spawn the server on a Tokio task when enabled; cancel via
  `tokio_util::sync::CancellationToken` on disable.
- `AppState` gets a new field `mcp: Arc<McpHandle>` that owns the
  enable/disable state and the cancellation token.

### Phase 2 — CRUD tools

- Wire each existing free-function command (`commands::crew::list`,
  `commands::runner::create`, etc.) as an MCP tool.
- Tool inputs / outputs serialize via the existing serde types — no
  new DTOs.
- Tool errors map from `crate::error::Error` to MCP error responses
  with sensible messages (`"crew not found: <id>"`, etc.).
- After each mutating tool call, emit the same `*/changed` event
  the Tauri command would. Reuse the helper if one exists; otherwise
  add a tiny `emit_changed` shim.

### Phase 3 — Settings UI

- New `<MCPServerSection>` in `SettingsModal.tsx`:
  - Toggle (uses the same `Switch` primitive other settings use).
  - Port input (number field, default 7654).
  - URL display (`http://127.0.0.1:<port>/mcp`).
  - Two copy buttons: Claude Code config / Codex config.
- Persistence via a new `STORAGE_MCP_*` set of keys in
  `src/lib/settings.ts` (`STORAGE_MCP_ENABLED`, `STORAGE_MCP_PORT`),
  same `"1"`/`"0"` and number encoding the rest of the file uses.
- Two new Tauri commands: `mcp_enable(port)` / `mcp_disable()`. The
  Settings UI calls them; the backend holds the live truth.

### Phase 4 — Pencil design + polish

- Design the Settings section in `design/runners-design.pen` —
  matches the existing Settings sections' spacing, copy-button
  affordance, code-block styling for the snippet preview.
- Confirm error states: port already bound, server start failure
  surface as a toast or inline banner.
- Add a "Test connection" button that hits `/mcp` once and reports
  OK / failure.

## Verification

- [ ] With MCP enabled, `curl http://127.0.0.1:7654/mcp -X POST -d '<initialize JSON-RPC frame>'`
      returns a valid MCP handshake.
- [ ] Adding the snippet to Claude Code's `~/.claude.json` and running
      `claude mcp list runner` shows every CRUD tool.
- [ ] `claude mcp call runner crew_create '{"name":"test"}'` creates the
      crew and the open Runner UI's sidebar shows it within ~1s
      (event-driven, not polled).
- [ ] Disabling the toggle stops the server; subsequent CC tool calls
      fail with connection refused.
- [ ] Re-enabling restarts the server on the same or new port without
      restarting the app.
- [ ] Two concurrent CC sessions can call tools without corrupting
      each other (sqlite serialization handles this; smoke-confirm).
- [ ] `cargo test -p runner` covers the tool wrapper layer (one test
      per CRUD verb is enough — the underlying free functions are
      already tested).
- [ ] `pnpm tsc --noEmit` and `pnpm lint` clean.
