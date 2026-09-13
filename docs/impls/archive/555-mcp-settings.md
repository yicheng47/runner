# 555 — Settings → MCP: implementation brief

Tracking issue: [#555](https://github.com/yicheng47/runner/issues/555). Feature, P2. Spec: [`../features/555-mcp-settings.md`](../features/555-mcp-settings.md) — read it in full first; it is the source of truth for behavior, copy, and the UI shape. Design signed off 2026-09-11; the frames are listed in the spec header. Branch: **`feat/555-mcp-settings` already exists and is checked out** — it carries this brief; work on it, do not create another.

## What ships

A Settings → MCP pane shaped like Settings → Skills: a runtime dropdown, search, a meta line with the selected runtime's config path, one list of every MCP server any agent's global config holds, a toggle per row meaning *registered in the selected runtime's config*, Runner's own server pinned as the first row, click-to-detail showing the entry as stored per agent, and Edit as a native-text editor (JSON for Claude Code, TOML for codex and trae) with an "Also update" toggle that translates to the other agents. Adding servers stays with the agents' CLIs. The `Runner MCP` line leaves the Agents cards.

## Where the code is

- `crates/runner-backend/src/ops/mcp.rs` — the writer to generalise. `claude_code_status_at` / `claude_code_write_at` (`serde_json::Value`, `mcpServers.runner`) and `codex_status_at` / `codex_write_at` (`toml_edit::DocumentMut`, `[mcp_servers.runner]`, `OpenOptions` mode 0600) each touch exactly the `runner` entry and keep the rest of the file. `mcp_integration_status` and `mcp_set_integration` stay as the Runner row's backend; the private `Client` enum maps `claude_code` / `codex` / `trae`, trae sharing the codex TOML path. `serde_json` has `preserve_order` on in this crate; keep it that way and keep `toml_edit` — never re-serialise a user's file through a typed struct.
- `crates/runner-backend/src/ops/skills.rs` — the per-runtime catalog pattern (`skill_catalogs`, `set_global_enabled`, `read_skill`, `save_skill`): one catalog per runtime, an on/off write that touches one key, a read and a save of raw text. The MCP ops mirror this shape.
- `crates/runner-app/src/surfaces/settings/skills.rs` — the pane to copy structurally: `SkillsPane` (catalog load through `cx.background_spawn` + `cx.spawn`, generation guard, runtime dropdown, search, meta line, rows with toggles at `:261`, `render` at `:354`), the detail entity (`open` / `finish_open` / `set_enabled` / `edit` / `save` / `request_dismiss` with the discard confirm, `render_modal` at `:770`), and its tests from `:1197` (`test_store`, both-runtime detail state, cancel-edit confirm, long-paragraph wrap).
- `crates/runner-app/src/surfaces/settings/agents.rs` — `McpClient` (`:49`, with `for_runtime`, `config_path`, `status`), `McpRowAction` (`:100`), `mcp_busy` / `mcp_errors` (`:145-150`), `set_mcp_integration` (`:395`), the row presentation at `:1193-1210` and its `mcp_row_presentation` tests. These move to the new pane; the `runner_property_line(.., "MCP", "Runner MCP")` call at `:824` and the registration sentence in the footnote go.
- `crates/runner-app/src/surfaces/settings_page.rs` — `SettingsPane` (`:36`), `from_route` (`:51`), slug (`:68`), label (`:84`), icon (`:100`), `INTEGRATION_PANES` (`:117`), the `skills` entity slot (`:190`), lazy construction (`:693`), the takeover for the detail modal (`:978`), search grouping (`:1148`), and the nav tests around `:2059-2070` whose counts change. Note `from_route(Some("mcp"))` currently maps to Agents (`:2066`); it now maps to the new pane.
- `crates/runner-app/src/assets.rs` — `plug.svg` is already in the icon table (`:114`); no new asset.
- Design: `design/runner.pen` (encrypted; you cannot read it). The spec describes every frame in words and that description is what you implement. If a visual detail you need is missing from the spec, ask the human through Runner rather than inventing it.

## Fix shape

### Phase 1 — backend (`ops/mcp.rs`)

1. `McpServerDefinition { Stdio { command, args, env }, Http { url, headers } }` with `from_claude(&Value)`, `from_toml(&Table)`, `to_claude() -> Value` (`type` + fields) and `write_toml(&mut Table)` that sets only the keys it owns and leaves every other key on the table. `sse` / `ws` on Claude Code and anything without `command` or `url` parse to `None`.
2. `McpServerEntry { name, clients: BTreeMap<McpClientId, McpServerClientEntry { registered, native_text, definition, conflicting } > }` where `native_text` is the entry pretty-printed as it sits in that file (JSON object for Claude Code; the `[mcp_servers.<name>]` table, sub-tables included, for codex and trae) and `conflicting` is true when another client holds the same name with a different definition. Make the client enum `pub` (rename the private `Client`, keep `parse`) so the app stops keeping its own.
3. `mcp_catalog(state) -> McpCatalog { runner: McpIntegrationStatus, servers: Vec<McpServerEntry> }`: read the three files, skip `runner`, sort by name, and report a file that fails to parse as an error string on that client's slot for every row rather than dropping the client.
4. `mcp_copy_server(state, from, to, name)` (list toggle on: parse the source entry's definition, refuse with a clear message when it is `None`, write the target's format, keeping unmodelled keys on an existing target entry), `mcp_remove_server(state, client, name)` (toggle off), and `mcp_edit_server(state, client, name, native_text, also: &[client])` (validate — a JSON object for Claude Code, a TOML table body for codex and trae — write it as the entry, then translate through the definition into each `also` client; reject before any write when the text does not parse, or parses to no definition while `also` is non-empty). Each client's write is independent; a failure names the file and the other writes stay.
5. Tests, `tempfile`-based like the existing ones, per the spec's Phase 2 list: hand-added server appears with one client; copy writes only the entry (assert the rest of the TOML byte-for-byte); copy onto an existing entry keeps `startup_timeout_sec`; conflict detection; edit with `also` resolves it; invalid text leaves both files untouched; `sse` listed, editable, refused for copy; malformed file reported and untouched; `runner` never in `servers`.

### Phase 2 — app

1. `crates/runner-app/src/surfaces/settings/mcp.rs`: `McpPane` after `SkillsPane` — runtime dropdown over the detected, enabled runtimes the writer knows; search over names; meta line `<config path> · N servers · M off` and the caption from the spec; the list card with the pinned Runner row (`built-in` badge, description, toggle wired to `mcp_set_integration` with the busy/generation guards and the `initialized_mcp_clients` record from `agents.rs`), then a row per server (name, `stdio`/`http` badge, mono command-or-URL with `KEY=•••` masking, warning glyph + caption when conflicting, toggle wired to copy/remove); empty state per the spec; hover tint and click-to-detail.
2. The detail entity: header (name, badge, Edit, close), description sentence per transport, `<path> → <entry key>` with Reveal config (reveal the file, `OpenFileLinks`-style like the Skills path row), `Registered in` pills, the raised registration toggle row for the current runtime, the doc panel with the per-agent segmented switch showing `native_text`. Runner's row opens the same detail with no Edit.
3. Edit mode in the same entity, like the skill editor: `editing` badge, the doc panel as a `TextField::textarea` holding `native_text`, the toggle row becomes one "Also update <agent>'s entry" per other agent that holds it (on by default), Save disabled until the text parses (parse locally with the same crates), Cancel with the discard confirm, footer copy from the spec.
4. `settings_page.rs`: `SettingsPane::Mcp` with slug `mcp`, label `MCP`, icon `plug.svg`, `INTEGRATION_PANES` = Agents · Skills · MCP, lazy entity slot and takeover like Skills, tests updated. `agents.rs` loses the `Runner MCP` line, its footnote sentence, `McpClient` and the presentation helpers; the presentation tests move with them.
5. Tests in `mcp.rs` following the Skills ones: rows reflect registered/conflict state per runtime; the pinned row is first regardless of sort; detail shows both bodies for a conflict; edit Save is blocked on invalid text; Cancel confirms before discarding; long mono lines wrap within the modal.

### Phase 3 — smoke (the human's)

Do not run `make run`. The spec's Verification list is the smoke script.

### Phase 4 — docs

`docs/arch/arch.md`: the Settings section describes the catalog in one paragraph, and references to Runner registration on Settings → Agents point at Settings → MCP; `docs/features/73-runner-skills.md`'s "later spec" pointer resolves to #555.

## Rules of the road

- Touch exactly the one entry you own in each config file and keep everything else byte-for-byte; when a test removes the last entry, do not delete the parent table or key (a comment attached to it would go with it).
- No new crates. `serde_json` (`preserve_order`) and `toml_edit` are already here.
- Keep the macOS and Windows platform chrome untouched; this is shared settings content.
- Commits on the feature branch are authorized, one per phase (backend, app, docs), each green on `cargo test --workspace`, `make clippy` (also with `--features updater`), and `make fmt` before it is made. Stage by path only; never `git add -A` or `git add .`. No push, no PR, no merge — the human lands the branch.

## Non-goals

Adding servers from Runner, project-scoped `.mcp.json` and `--mcp-config`, per-runner picks, `enabled = false` handling, auth flows, spawning servers, runtimes the writer does not know (pi, shell). Do not change `ui/scrollbar.rs`, `ui/list.rs`, or any other pane.
