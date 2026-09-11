# 555 — Settings → MCP: one catalog of MCP servers for every agent

> Tracking issue: [#555](https://github.com/yicheng47/runner/issues/555)
> Priority: P2.
> Design: `design/runner.pen`, SETTINGS row at y 20953: `Settings — MCP` (`S4ZUkV`), `· Codex` (`EdDY5`), `· server detail` (`J4GKLQ`), `· edit server` (`VKKcK`, JSON), `· edit server · Codex` (`gI12M`, TOML), `· conflict` (`e2kKf`), `· empty` (`Jdsq6`); `cmp/SettingsNav` gained `nav_mcp` (`FCMH4`); the `Settings — Agents` cards lost their `Runner MCP` line. Signed off 2026-09-11.

## Motivation

Every agent Runner spawns reads its MCP servers from its own global config: `mcpServers` in `~/.claude.json`, `[mcp_servers.*]` in `~/.codex/config.toml`, and the same table in `~/.trae/traecli.toml`. A server the user wants in every session — a GitHub server, a database, their own tool — has to be added once per agent in that agent's format, and nothing in Runner shows which agent currently has which server. The runner is the unit Runner cares about, and a runner's runtime is a detail the crew designer picks; the MCP servers that runner can reach should not depend on which config file its runtime happens to read.

[#73](https://github.com/yicheng47/runner/issues/73) narrowed the original agent-agnostic catalog to skills and deferred the MCP half to "a later spec". [#530](https://github.com/yicheng47/runner/issues/530) then removed the old `Settings → MCP` pane because all it did was register Runner's own server, and moved that as a `Runner MCP` line onto each Agents card, reserving the `MCP` nav name for the catalog. This is the later spec. It brings the pane back as the catalog, shaped like the Skills pane: pick a runtime, see every server any agent has, and flip one on or off for that runtime. Runner's own server — the `runner` entry `ops/mcp.rs` writes so agents can drive crews, missions and the bus — is the catalog's pinned first row, managed here and no longer on the Agents cards, so the Runner section never grows with the runtime list. The Agents pane goes back to being about the agent binary: enabled, executable, model and effort.

## Scope

### Model: the agents' configs are the catalog

There is no Runner-side store of MCP servers. The catalog is the union of the agents' global MCP entries, keyed by server name, read fresh when the pane opens, on Refresh, and after every write. Sources are the files `ops/mcp.rs` already reads and writes: `mcpServers` in `~/.claude.json` (Claude Code's user scope), `[mcp_servers.<name>]` in `~/.codex/config.toml` (codex) and `~/.trae/traecli.toml` (trae). A server added outside Runner — `claude mcp add`, a hand edit of `config.toml` — shows up in the catalog on the next read. A server removed from every agent disappears from the catalog. This is the same posture as the Skills pane and the existing Runner registration: structured read-modify-write of one entry, everything else in the file preserved.

Runner does not add servers. Adding is the agent's own job (`claude mcp add`, editing `config.toml`), which also keeps the auth flows the CLIs run themselves out of Runner. What Runner adds is the cross-agent view and the one-toggle copy: a server one agent has can be registered into another from this pane.

`McpServerDefinition` is the runtime-neutral shape of one entry, used only when an entry is copied from one agent to another:

- **stdio** — `command`, `args`, `env`.
- **http** — `url`, `headers`.

Translation per client: Claude Code gets `{ "type": "stdio", "command", "args", "env" }` or `{ "type": "http", "url", "headers" }`; codex and trae get `command`/`args`/`env` or `url`/`http_headers` in the entry table. When a copy lands on an agent that already has the entry, keys on that entry the definition does not model (codex `startup_timeout_sec`, `tool_timeout_sec`, `enabled`, `cwd`) survive. An entry whose transport the definition does not model (Claude Code `sse` or `ws`) is listed and editable in its own file, but cannot be copied to another agent; the toggle for the other runtime is disabled with a caption pointing at that agent's CLI.

Two agents holding the same name with different definitions is a **conflict**: the list row shows a warning glyph and a caption naming the other agent (`Codex has a different definition: …`), and the detail view shows both bodies side by side through its per-agent switch. It resolves by editing either copy with "Also update" on, or by hand in the file.

### Settings → MCP

A new pane under Integrations, so the group reads `Agents · Skills · MCP`; route slug `settings/mcp` is reclaimed. It follows the Skills pane element for element.

- **Title row.** `MCP` and a **Refresh** button.
- **Controls.** A runtime dropdown listing every detected, enabled runtime the writer knows (Claude Code, Codex, TRAE CLI; a runtime that is not installed is simply absent) and a search field over server names.
- **Meta line.** The selected runtime's config path in mono and `N servers · M off`, then a caption: toggles register or unregister a server in that runtime's config, the named entry is the only thing Runner writes, Runner's own server is pinned first, new servers come from the agent's own tooling, running sessions see changes on their next launch.
- **List.** One card, one row per catalog name, sorted by name, with the **Runner** row pinned first: name, a `built-in` badge at `accent`, its one-line description, and a toggle. Every other row: name, a `stdio` / `http` badge, the command with its args or the URL in mono (env values and header values masked as `KEY=•••`), a warning glyph and caption when the row is in conflict, and a toggle that means *registered in the selected runtime's config*. On writes the entry the first agent in nav order holds into this runtime's file; off removes it from this runtime's file and nothing else. Hover tints the row; click opens the detail. Toggling the Runner row is the #530 Register/Unregister for that client, including the `initialized_mcp_clients` record so a client the user turned off stays off.
- **Empty state.** When the selected runtime holds nothing but Runner's entry, the card shows the Runner row and, below the hairline, `No other servers in <config path> yet.` with a line pointing at the agent's CLI and at switching runtime to copy a server another agent already has.
- **Detail modal.** The Skills detail, on a server: header with the name, transport badge, an **Edit** button and close; a one-sentence description of what the transport means; the path down to the entry key (`~/.claude.json → mcpServers.github`) with **Reveal config**; a `Registered in` row of pills for every agent that holds the entry; a raised `Registered in <runtime>` toggle row with the write-scope sentence; and a doc panel showing the entry exactly as stored in the file, with a segmented switch per agent that has it, so a conflict is two different bodies. Runner's own row opens the same detail with no Edit.
- **Edit modal.** The detail in edit mode, like the skill editor: an `editing` badge in the header, the same description, path and pills, then the doc panel as a text editor holding the entry in the selected runtime's native text — JSON for Claude Code, a TOML table for codex and trae — so what you edit is what the file holds, extra keys included. Above it the toggle row becomes **Also update <other agent>'s entry**, one per other agent that holds the server, on by default: Save parses the edited text into the definition and writes those agents' formats too, keeping their extra keys; off leaves their copies alone. The footer says only the named entry changes in each file and that invalid JSON or TOML cannot be saved; Save is disabled until the text parses. Cancel discards. Renaming is not an edit; the name is the key, so it is remove-and-re-add through the agent's CLI.

### Agents pane

The `Runner MCP` line and its button leave every runtime card; the `props` group keeps the `Model · Effort` line. The registration sentence in the pane footnote goes with it. `initialize_mcp_defaults`, the once-per-client default registration pass with its `initialized_mcp_clients` marker, is unchanged.

### Follow-ons this spec sets up but does not build

- **Per-runner MCP picks** — the runner-level analog of #73 M3: a runner declares which catalog servers it wants and Runner narrows the set at spawn (Claude Code's `--strict-mcp-config` with `--mcp-config`). The catalog is the list that picker would show.
- **Copilot** — [#540](https://github.com/yicheng47/runner/issues/540) planned Runner registration into `mcp-config.json` as an Agents row; when that runtime lands, it is one more entry in this pane's runtime dropdown and one more format for the writer.

### Out of scope

- Adding servers from Runner. The CLIs own that, auth flows included.
- Project-scoped servers: Claude Code's `.mcp.json` and local scope, and per-session `--mcp-config`. The pane is global, like the Skills pane's global on/off.
- Enabling or disabling a server without unregistering it (codex `enabled = false`). Registration is the toggle; `enabled` stays an editable key in the TOML.
- OAuth and other auth flows the CLIs run themselves; the pane stores what the config file stores.
- Spawning a server to list its tools or check its health.
- Runtimes whose config the writer does not know (pi keeps MCP out per [#539](https://github.com/yicheng47/runner/issues/539); the shell has none). They are absent from the dropdown and the pills.

## Implementation Phases

### Phase 1 — design

Done 2026-09-11, see the header. The row frames are the reference for copy and layout; the Skills frames (`s8mYyG`, `cwayt`, `peYFQ`) are the reference for anything the MCP frames share with them.

### Phase 2 — backend

`crates/runner-backend/src/ops/mcp.rs`:

- `McpServerDefinition { Stdio { command, args, env }, Http { url, headers } }` and `McpServerEntry { name, clients: BTreeMap<client, McpServerClientEntry { registered, native_text, definition: Option<McpServerDefinition>, conflicting } > }`, where `native_text` is the entry as it sits in that client's file (pretty JSON for Claude Code, the TOML table for codex and trae) and `definition` is `None` for transports the definition does not model.
- `mcp_catalog(state) -> McpCatalog { runner: McpIntegrationStatus, servers: Vec<McpServerEntry> }`, reading the three files with the parsers already there (`serde_json` for `~/.claude.json`, `toml_edit` for the two TOML files), reporting the `runner` entry through `runner` rather than `servers`, and reporting a file that fails to parse as an error for that client rather than dropping the client.
- `mcp_copy_server(state, from_client, to_client, name)`: parse the source entry into the definition and write the target's format, keeping unmodelled keys on an existing target entry. This is the list toggle turning on.
- `mcp_remove_server(state, client, name)`: the list toggle turning off.
- `mcp_edit_server(state, client, name, native_text, also: &[client])`: validate the text (JSON object for Claude Code, a TOML table for codex and trae), write it into that client's file as the entry, then for each client in `also` translate through the definition. A text that does not parse, or parses to something the definition cannot model while `also` is non-empty, is rejected before any write.
- The Runner-specific functions (`mcp_integration_status`, `mcp_set_integration`) stay as they are and share the helpers.
- Tests, `tempfile`-based like the existing ones: a hand-added server in `~/.claude.json` appears with only the Claude Code client registered; copy to codex writes `[mcp_servers.<name>]` and nothing else changes in `config.toml`; copy onto an existing codex entry keeps `startup_timeout_sec`; the same name with different commands is reported `conflicting`; edit with `also` resolves it; edit with invalid text leaves both files untouched; an `sse` entry is listed, editable, and refused for copy; a malformed file is reported and left untouched; `runner` never appears in `servers`.

### Phase 3 — app

- `crates/runner-app/src/surfaces/settings/mcp.rs`: the pane (runtime dropdown, search, meta line, caption, list with the pinned Runner row, toggles, conflict caption, empty state), the detail modal and the edit modal, following `skills.rs` for the catalog load, the background-task pattern, the detail takeover and the in-modal editor, and `agents.rs` for the busy/generation guards on the Runner row.
- The `McpClient` enum, the four-state row presentation, `set_mcp_integration` and `refresh_mcp_status` move from `agents.rs` to `mcp.rs`, with the `mcp_row_presentation` tests; `agents.rs` drops the `runner_property_line(.., "MCP", "Runner MCP")` line and its footnote sentence.
- `settings_page.rs`: `SettingsPane::Mcp`, route slug, nav label `MCP`, a lucide `plug` icon added to the icon set, `INTEGRATION_PANES` reads Agents · Skills · MCP; the settings-nav search test count adjusted.

### Phase 4 — docs

`docs/arch` references to Runner registration on Settings → Agents repointed at Settings → MCP; the Settings section of `arch.md` describes the catalog in one paragraph; #73's "later spec" pointer resolves to this issue; this spec archives on landing.

## Verification

- [ ] Fresh profile with `claude` and `codex` on PATH: Settings → MCP opens on Claude Code with the Runner row pinned and on; switching to Codex shows the Runner row on there too (the default pass ran); TRAE CLI is absent from the dropdown; turning Runner off for Codex sticks across a restart and a runtime refresh.
- [ ] Agents cards show `Model · Effort` and no `Runner MCP` line; settings nav search for `mcp` finds the MCP pane; `settings/mcp` routes to it.
- [ ] `claude mcp add github -- gh-mcp` outside Runner, Refresh: a `github` row on Claude Code with its toggle on; switch to Codex: the same row with its toggle off; turn it on: `[mcp_servers.github] command = "gh-mcp"` appears and nothing else changes in `config.toml` (diff before and after).
- [ ] Hand-edit the codex entry to a different command, Refresh: the row shows the conflict glyph naming Codex; the detail's segmented switch shows the two bodies; Edit from Claude Code with "Also update Codex's entry" on and Save clears the conflict and keeps a `startup_timeout_sec` hand-added to the codex entry.
- [ ] Edit on Codex shows the TOML table; a save with a syntax error is refused with the file untouched.
- [ ] Turning a row off on every runtime removes it from the catalog; the `runner` entries and every other server are untouched.
- [ ] A read-only `~/.claude.json`: the write fails, the pane says which file, the codex write still lands.
- [ ] `runner-backend` and `runner-app` tests pass; `make verify` green.
