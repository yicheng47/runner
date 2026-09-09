# 530 — Fold the MCP settings pane into Agents

Tracking issue: [#530](https://github.com/yicheng47/runner/issues/530). Spec: [530](../features/530-fold-mcp-pane-into-agents.md). Feature, P2. Design: `design/runner.pen` `Settings — Agents` (`n1krgH`), committed `eb64a76`. Branch: **`feat/530-fold-mcp-pane-into-agents` already exists and is checked out** — it carries the design and this brief; work on it, do not create another. Phases 2 and 3 of the spec.

## What ships

Settings → MCP is gone. Runner registers itself as an MCP server in every detected, enabled agent by default on macOS as Windows already does, once per client so Unregister sticks. The Agents pane shows one card per runtime; each card ends in a label/value `Model · Effort` line and a `Runner MCP` line with the state and an Unregister or Register button. The binding directory, the manual-config snippets and the shield note go with the pane. `ops/mcp.rs` is unchanged.

## Where the code is

- `crates/runner-app/src/app_store.rs:22` `mod mcp_defaults_windows` behind `cfg(windows)`; the `initialize_mcp_defaults` calls at `:354` (construction), `:374` (runtime refresh) and `:505` (enabled-agents change) are gated the same way, as are `StoreRefreshKind::Runtimes` (`:35`) and the `"runtime/changed"` arm (`:43`). `app_store/mcp_defaults_windows.rs` is the pass itself, with two tests. `app_settings.rs:199` `initialized_mcp_clients` is `cfg(windows)` too.
- `crates/runner-backend/src/ops/mcp.rs:308` `mcp_integration_status`, `:329` `mcp_set_integration(client, enabled)`, `McpClientStatus` (`registered`, `matches_current`, `command`, `args`, `error`). Clients are `claude-code`, `codex`, `trae`.
- `crates/runner-app/src/surfaces/settings/mcp.rs`: `McpClient` (`:12`, keys/titles/subtitles), `refresh` (`:133`), `set_integration` (`:190`), `mcp_row_presentation` (`:555`, the state model with its test `derives_each_mcp_row_state` at `:654`), `binding_location` (`:537`).
- `crates/runner-app/src/surfaces/settings/agents.rs:577` `render_runtime_row` (header with name, badge, Enabled toggle; the executable field with Browse and "Reset to auto" at `:668`; the mono `Model: … · Effort: …` caption at `:685` via `runtime_defaults_caption` `:874`; the validation/detected caption last); `:704` `render` puts every row in one `SettingsCard` at `:744` with the footnote at `:750`.
- `crates/runner-app/src/surfaces/settings_page.rs`: `SettingsPane::Mcp` at `:37`, route `:53`, key `:71`, label `:88`, icon `:105`, `INTEGRATION_PANES` `:125`, the pane entity `:199`, construction `:659`, render `:1141`, nav test `:2055`; the nav search count test currently asserts 12.
- `README.md:141` says "Settings → MCP registers it with Claude Code and Codex on both platforms, plus TRAE CLI on macOS".

## Fix shape

1. **Defaults on both platforms.** Rename `app_store/mcp_defaults_windows.rs` to `app_store/mcp_defaults.rs` and drop every `cfg(windows)` named above, including `initialized_mcp_clients` (serde default keeps old macOS settings files loading as empty, so the first launch registers their agents). The `"runtime/changed"` refresh kind stays platform-neutral. Do not change what `ops/mcp.rs` writes.
2. **Agents pane, one card per runtime.** `render` emits one `SettingsCard` per runtime with the existing spacing between cards; the header and executable row are unchanged except **"Reset to auto" → "Reset"**. Replace the mono defaults caption with a label/value line: labels `Model` and `Effort` in the UI font at `theme::muted()`, values in `theme::UI_MONOSPACE_FONT` at `theme::text()`, a `·` between, the label column 92 px so the next line aligns.
3. **Runner MCP line** on every runtime whose key `ops/mcp.rs` knows (claude-code, codex, trae; anything else gets no line): label `Runner MCP`, a 6 px dot, mono status, spacer, one small button (6 px radius, `ButtonSize::Sm`). States from `McpClientStatus`, reusing `mcp_row_presentation` moved into `agents.rs`: registered and current → accent dot, `Registered in ~/.claude.json` (the per-client path from `McpClient::subtitle`'s file) at `text`, outline **Unregister**; not registered → faint dot, `Not registered` at `muted`, tinted **Register** (10% accent fill, 20% accent border, accent text); registered elsewhere → warning dot, `Registered to another Runner · <command args>` at `warning`, tinted **Register** (replaces the entry); error → the message in the row's red validation caption, button **Register** as retry; busy → button disabled. The write is `mcp_set_integration`; refresh the status after it the way `McpPane::set_integration` does.
4. **Pane, nav, footnote.** Delete `SettingsPane::Mcp`, its route/key/label/icon, the entity and its arms, `surfaces/settings/mcp.rs`; `from_route(Some("mcp"))` → `Agents`; `INTEGRATION_PANES` = Agents, Skills; nav count 12 → 11. The Agents footnote gains "Registration writes only the `runner` entry in each agent's config." The Agents pane owns the status fetch (`mcp_integration_status`) on open and after each write.
5. **Docs.** `README.md:141` reads "Settings → Agents registers it with Claude Code, Codex and TRAE CLI on both platforms"; leave the screenshot file alone. Any `docs/arch` mention of Settings → MCP repoints at Agents (grep; none were found at brief time).

## Rules of the road

- `ops/mcp.rs` and its tests unchanged. No new settings key beyond lifting the gate on `initialized_mcp_clients`.
- Keep UI copy exactly as the frame gives it. No tooltip, no separate error card, no binding-dir field anywhere.
- Windows-only tests cannot be claimed as run on this Mac; keep the defaults-pass tests platform-neutral so they run here.
- Do not launch the Runner app (`make run`); Jason smoke-tests. Verify with `cargo test -p runner-app -p runner-backend`, `make clippy` (with `--features updater` too), `make fmt`.
- Mission authorization: after the reviewer reports clean, PR mode is authorized — commit on this branch, push, open the PR, drive CI green (`gh pr checks <n> --watch`; the required check is `Rust / macOS`). Do not merge: Jason merges after his own check. No worktrees, no extra checkouts, no extra agents.

## Tests

- `mcp_defaults`: the existing two tests, now compiled on macOS; a third proving a client in `initialized_mcp_clients` is never re-registered by the pass.
- `agents.rs`: the moved `derives_each_mcp_row_state` plus the mapping from each state to dot tone, status text and button label; runtimes outside the writer's set get no line.
- `settings_page.rs`: nav count 11, `from_route(Some("mcp"))` → Agents, no `Mcp` variant; the existing Agents `VisualTestContext` tests, if any, updated for the per-card layout and the two new lines at two rem sizes.
- `app_settings`: a settings JSON without `initializedMcpClients` decodes to empty on macOS.

## Jason's smoke test (after landing)

1. Fresh profile with `claude`, `codex` and `traecli` on PATH: first launch registers all three and lists them in the settings file; Agents shows three cards, each with `Registered in …` and Unregister.
2. Unregister codex, quit, relaunch, press Refresh: codex stays `Not registered`. Register again: the entry is back and nothing else in `~/.codex/config.toml` changed.
3. Point `~/.claude.json`'s `runner` entry at another path by hand: the line shows `Registered to another Runner` in warning; Register replaces it.
4. Make `~/.trae/traecli.toml` read-only and press Register: the error lands in the red caption under the field.
5. Settings nav has no MCP item; search for "mcp" finds nothing; `settings/mcp` opens Agents.

## Non-goals

Host MCP-server management (the later catalog spec), any change to the registration format or the binding-dir semantics, a per-platform default, and the design of the Skills pane.
