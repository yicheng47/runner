# 555 — Settings → MCP: one catalog of MCP servers for every agent

> Tracking issue: [#555](https://github.com/yicheng47/runner/issues/555)
> Priority: P2.

## Motivation

Every agent Runner spawns reads its MCP servers from its own global config: `mcpServers` in `~/.claude.json`, `[mcp_servers.*]` in `~/.codex/config.toml`, and the same table in `~/.trae/traecli.toml`. A server the user wants in every session — a GitHub server, a database, their own tool — has to be added three times in two formats, and nothing in Runner shows which agent currently has which server. The runner is the unit Runner cares about, and a runner's runtime is a detail the crew designer picks; the MCP servers that runner can reach should not depend on which config file its runtime happens to read.

[#73](https://github.com/yicheng47/runner/issues/73) narrowed the original agent-agnostic catalog to skills and deferred the MCP half to "a later spec". [#530](https://github.com/yicheng47/runner/issues/530) then removed the old `Settings → MCP` pane because all it did was register Runner's own server, and moved that as a `Runner MCP` line onto each Agents card, reserving the `MCP` nav name for the catalog. This is the later spec. It brings the pane back as the catalog, and Runner's own server — the `runner` entry `ops/mcp.rs` writes so agents can drive crews, missions and the bus — becomes the catalog's built-in first entry, managed here and no longer on the Agents cards. The Agents pane goes back to being about the agent binary: enabled, executable, model and effort.

## Scope

### Model: the agents' configs are the catalog

There is no Runner-side store of MCP servers. The catalog is the union of the agents' global MCP entries, keyed by server name, read fresh when the pane opens and after every write. Sources are the files `ops/mcp.rs` already reads and writes: `mcpServers` in `~/.claude.json` (Claude Code's user scope), `[mcp_servers.<name>]` in `~/.codex/config.toml` (codex) and `~/.trae/traecli.toml` (trae). A server added outside Runner — `claude mcp add`, a hand edit of `config.toml` — shows up in the catalog on the next read with the chips of the agents that have it. A server removed from every agent disappears from the catalog; that is what Remove means. This is the same posture as the Skills pane and the existing Runner registration: structured read-modify-write of one entry, everything else in the file preserved.

`McpServerDefinition` is what one row means, independent of any agent's format:

- **stdio** — `command`, `args`, `env`.
- **http** — `url`, `headers`.

Translation per client: Claude Code gets `{ "type": "stdio", "command", "args", "env" }` or `{ "type": "http", "url", "headers" }`; codex and trae get `command`/`args`/`env` or `url`/`http_headers` in the entry table. Keys on an existing entry that the definition does not model (codex `startup_timeout_sec`, `tool_timeout_sec`, `enabled`, `cwd`; Claude Code's `type: "sse"`) survive an edit untouched. An entry whose transport the form does not model (Claude Code `sse` or `ws`) is listed with its chips but Edit is disabled with a caption pointing at the agent's config; Remove still works.

Two agents holding the same name with different definitions is a **conflict**: the row shows a warning glyph and the tooltip names the agents; opening Edit shows the first agent's definition (in nav order: Claude Code, codex, trae) and Save writes it to every checked agent, which resolves it.

### Settings → MCP

A new pane under Integrations, so the group reads `Agents · Skills · MCP`; route slug `settings/mcp` is reclaimed. Pencil-first like the Skills pane.

- **Runner card** at the top. Title `Runner`, description in one sentence (what the server lets an agent do: crews, missions, the bus, direct chats). One row per agent the writer knows — Claude Code, codex, trae — in the shape of the #530 `Runner MCP` line: a 6 px dot (accent when registered, faint when not), the status in mono (`Registered in ~/.claude.json`, `Not registered`, `Registered to another Runner · <configured command>` at `warning`), and the button at the right edge: outline **Unregister** in the normal case, tinted **Register** for a client the user turned off, whose write failed, or that points at another Runner. A failed write reports in a red caption under the row. The command is not editable and the binding directory is not shown, as #530 decided. Every runtime the writer knows gets a row whether or not its binary is detected; the row for an agent that is not installed says so in its caption and its button is disabled.
- **Servers card** below it. One row per catalog name, sorted by name: the name in the UI font, a `stdio` / `http` badge, the command with its args or the URL in mono at `muted` (env values and header values masked as `KEY=•••` in the row; the count of env vars if there are more than two), one chip per agent (filled with the agent's short name when registered there, outline when not; clicking a chip registers the row's definition into that agent or removes it, and an agent that is not installed has a disabled chip), and a trailing overflow menu with **Edit** and **Remove**. Remove confirms once and removes the entry from every agent that has it. The card header holds the count and an **Add server** button. Empty state: one line saying no agent has an MCP server registered, with the Add server button.
- **Add / Edit modal.** Fields: **Name** (required; `[A-Za-z0-9_-]`, unique in the catalog on Add, read-only on Edit — renaming is Remove then Add, same as the CLIs), **Transport** (select: `stdio`, `http`), then for stdio **Command** (required), **Args** (one per line), **Env** (`KEY=value` per line); for http **URL** (required) and **Headers** (`Name: value` per line). Then **Agents**: one checkbox per agent the writer knows, default checked for every detected and enabled agent on Add, and for the agents that hold the entry on Edit; an agent that is not installed is unchecked and disabled. Save writes the definition to every checked agent and removes the entry from every unchecked agent that had it; each agent's write is independent, and a failure surfaces in the modal under the Agents list naming the file, with the other writes kept. Cancel discards.
- **Footnote.** One sentence: Runner writes only the named entry in each agent's config; running sessions see the change on their next launch.

### Agents pane

The `Runner MCP` line and its button leave every runtime card; the `props` group keeps the `Model · Effort` line. The registration sentence in the pane footnote goes with it. `initialize_mcp_defaults`, the once-per-client default registration pass with its `initialized_mcp_clients` marker, is unchanged; Unregister on the Runner card records the client the same way the Agents button did, so a client the user turned off stays off.

### Follow-ons this spec sets up but does not build

- **Per-runner MCP picks** — the runner-level analog of #73 M3: a runner declares which catalog servers it wants and Runner narrows the set at spawn (Claude Code's `--strict-mcp-config` with `--mcp-config`). The catalog is the list that picker would show.
- **Copilot** — [#540](https://github.com/yicheng47/runner/issues/540) planned Runner registration into `mcp-config.json` as an Agents row; when that runtime lands, its registration row and its column in the catalog live on this pane instead.

### Out of scope

- Project-scoped servers: Claude Code's `.mcp.json` and local scope, and per-session `--mcp-config`. The pane is global, like the Skills pane's global on/off.
- Enabling or disabling a server without unregistering it (codex `enabled = false`). Registration is the toggle.
- OAuth and other auth flows the CLIs run themselves (`claude mcp` login); the pane stores what the config file stores.
- Spawning a server to list its tools or check its health.
- Runtimes whose config the writer does not know (pi keeps MCP out per [#539](https://github.com/yicheng47/runner/issues/539); the shell has none). They get no rows, no chips and no checkbox, not disabled ones.

## Implementation Phases

### Phase 1 — design

`design/runner.pen`: a `Settings — MCP` frame with the Runner card and the Servers card, `· add server` (the modal, stdio and http variants), `· conflict` (a row with the warning glyph), `· empty`; `cmp/SettingsNav` Integrations gains `nav_mcp` after `nav_skills`; the `Settings — Agents` frame (`n1krgH`) cards `card_claude` (`AConK`), `card_codex` (`N67UH`), `card_trae` (`r6iFF`) lose the `Runner MCP` line from their `props` group. Jason signs off before Phase 3 starts.

### Phase 2 — backend

`crates/runner-backend/src/ops/mcp.rs`:

- `McpServerDefinition { Stdio { command, args, env }, Http { url, headers } }` and `McpServerEntry { name, definition: Option<McpServerDefinition>, clients: BTreeMap<client, McpServerClientStatus { registered, conflicting, editable, error }> }`.
- `mcp_catalog(state) -> McpCatalog { runner: McpIntegrationStatus, servers: Vec<McpServerEntry> }`, reading the three files with the parsers already there (`serde_json` for `~/.claude.json`, `toml_edit` for the two TOML files), skipping the `runner` entry, and reporting a file that fails to parse as an error on every row's status for that client rather than dropping the client.
- `mcp_set_server(state, client, name, &McpServerDefinition)` and `mcp_remove_server(state, client, name)`: the existing `claude_code_write_at` / `codex_write_at` shape generalised over a name and a definition; the Runner-specific functions (`mcp_integration_status`, `mcp_set_integration`) stay as they are and share the helpers.
- Name validation at the op boundary.
- Tests, `tempfile`-based like the existing ones: stdio and http round-trip per client; an edit keeps `startup_timeout_sec` on a codex entry and `type: "sse"` is reported non-editable; a hand-added server in `~/.claude.json` appears with only the Claude Code chip; the same name with different commands is reported `conflicting`; a malformed file is left untouched and reported; `runner` never appears in `servers`.

### Phase 3 — app

- `crates/runner-app/src/surfaces/settings/mcp.rs`: the pane (Runner card, Servers card, chips, overflow menu, remove confirm) and the add/edit modal, following `skills.rs` for the catalog load and background-task pattern and `agents.rs` for the busy/generation guards.
- The `McpClient` enum, the four-state row presentation, `set_mcp_integration` and `refresh_mcp_status` move from `agents.rs` to `mcp.rs`, with the `mcp_row_presentation` tests; `agents.rs` drops the `runner_property_line(.., "MCP", "Runner MCP")` line and its footnote sentence.
- `settings_page.rs`: `SettingsPane::Mcp`, route slug, nav label `MCP`, a lucide `plug` icon added to the icon set, `INTEGRATION_PANES` reads Agents · Skills · MCP; the settings-nav search test count adjusted.

### Phase 4 — docs

`docs/arch` references to Runner registration on Settings → Agents repointed at Settings → MCP; the Settings section of `arch.md` describes the catalog in one paragraph; #73's "later spec" pointer resolves to this issue; this spec archives on landing.

## Verification

- [ ] Fresh profile with `claude` and `codex` on PATH: Settings → MCP shows the Runner card with both registered (the default pass ran) and trae `Not registered` with its button disabled and a not-installed caption; Unregister on codex sticks across a restart and a runtime refresh.
- [ ] Agents cards show `Model · Effort` and no `Runner MCP` line; settings nav search for `mcp` finds the MCP pane; `settings/mcp` routes to it.
- [ ] `claude mcp add github -- gh-mcp` outside Runner, reopen the pane: a `github` row with the Claude Code chip filled and the codex chip outline; clicking the codex chip writes `[mcp_servers.github] command = "gh-mcp"` and nothing else changes in `config.toml` (diff before and after).
- [ ] Add server, http, URL and one header, all agents checked: `~/.claude.json` gets `type: "http"` with `headers`, both TOML files get `url` and `http_headers`; the row masks the header value.
- [ ] Edit that server's URL: all three entries change, and a `startup_timeout_sec` hand-added to the codex entry beforehand is still there.
- [ ] Hand-edit the codex entry to a different command: the row shows the conflict glyph naming codex; Edit shows the Claude Code definition; Save clears the conflict.
- [ ] Remove: confirm once; the entry is gone from all three files; the row is gone; the `runner` entries and every other server are untouched.
- [ ] A read-only `~/.claude.json`: the write fails, the modal names the file, the codex and trae writes still land.
- [ ] `runner-backend` and `runner-app` tests pass; `make verify` green.
