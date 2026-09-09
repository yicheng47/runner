# 530 — Fold the MCP settings pane into Agents

Tracking: [#530](https://github.com/yicheng47/runner/issues/530). Status: **shipped 2026-09-09 in [#534](https://github.com/yicheng47/runner/pull/534)** (with three in-mission additions: select menus fit their option descriptions, no scrollbar thumb on a sub-pixel range, Appearance before Missions in the nav). Specced 2026-09-09, P2, designed 2026-09-09 on the existing `Settings — Agents` frame (`n1krgH`) in `design/runner.pen`: one card per runtime (`card_claude` `AConK`, `card_codex` `N67UH`, `card_trae` `r6iFF`), each block ending in a `props` group with the `Model · Effort` line and the `Runner MCP` line; `cmp/SettingsNav` Integrations is `nav_agents` · `nav_skills`. Jason picked the two-line layout with a button from three drawn alternatives (checkbox sentence, property row with a button, two-line with a button); the alternatives were removed from the canvas.

## Motivation

Settings → MCP does one thing: it registers Runner as an MCP tool server inside each agent's config — the `runner` entry in `~/.claude.json`, the `runner` table in `~/.codex/config.toml` — one row per agent, plus the binding directory those entries point at. Nothing in it manages the host's MCP servers. But a nav item named **MCP** under *Integrations* promises exactly that, and the deferred half of [#73](https://github.com/yicheng47/runner/issues/73), an agent-agnostic MCP-server catalog, will want that name later with no baggage.

The pane's content already has the Agents pane's shape: one row per runtime. Registering Runner is one-time setup per agent, not something people revisit, so it belongs on the agent's row rather than behind its own nav item.

The default also differs by platform today. On Windows the app registers Runner into every detected, enabled agent at startup and whenever the runtime list or the enabled-agents set changes, once per client (`app_store/mcp_defaults_windows.rs`, marked in `initialized_mcp_clients`). On macOS that module is compiled out, so a fresh install stays unregistered until the user finds the MCP pane and presses Register. The pane therefore shows a state the app already set on one platform and is the only way to reach it on the other.

## Scope

### Register by default, on both platforms

`initialize_mcp_defaults` loses its `cfg(windows)` gates and runs on macOS and Windows alike: at startup, on runtime refresh, and when the enabled-agents set changes, every detected and enabled runtime with an MCP config is registered once and recorded in `initialized_mcp_clients`. The once-per-client rule is what makes Unregister stick — a client the user turned off is never re-registered by the default pass. Existing macOS installs already carry the `initialized_mcp_clients` setting (it defaults empty), so the first launch after this lands registers their agents the same way a fresh Windows install does. The module is renamed to drop the `_windows` suffix; what it writes (`ops/mcp.rs`) is unchanged.

### Agents pane

- **One card per runtime.** The single runtimes card with hairlines becomes one `SettingsCard` per runtime, 16 px apart, each holding the block it holds today (header with name, badge, Enabled toggle and command; the executable field with **Browse** and **Reset** — the latter renamed from "Reset to auto").
- **Label/value lines.** The faint mono `Model: … · Effort: …` caption (feature 65) becomes a label/value line: labels (`Model`, `Effort`) in the UI font at `muted`, values in the mono font at `text`, the label column 92 px wide so the lines below align. The two are separated by a `·`.
- **Runner MCP line.** A second line in the same shape under it: label `Runner MCP`, a 6 px dot (accent when registered, faint when not), the status in mono — `Registered in ~/.claude.json` at `text`, `Not registered` at `muted`, `Registered to another Runner · <configured command>` at `warning` for an entry that points at a different binary — and at the right edge a small 6 px-radius button: outline **Unregister** (border `border_strong`, text `text`) in the normal case, since the default pass has already registered the agent, and tinted **Register** (10% accent fill, 20% accent border, accent text — the Detected badge's tint) for a client the user turned off, whose write failed, or that points at another Runner (Register replaces the entry). The action is the existing `ops/mcp.rs` write; nothing about what is written changes.
- A failed write reports in the row's existing red validation caption, which stays the last line of the block, not in a separate error card.
- Every runtime with an MCP config gets the line: claude-code (`~/.claude.json`), codex (`~/.codex/config.toml`) and trae (`~/.trae/traecli.toml`), which `ops/mcp.rs` already writes. A runtime the writer does not know gets no line, not a disabled one — the same rule the Skills pane applies to its toggle.
- **The binding directory is not shown.** It is read-only, derived from the MCP socket's location under the app data dir, and only matters when an entry points at another Runner, which the line above already says. The MCP pane's "Current binding" field, its copy button and environment badge, the manual-config snippet card and the shield note all go with the pane.
- The pane footnote gains one sentence: registration writes only the `runner` entry in each agent's config.

### Nav

`MCP` leaves the Integrations group, which becomes `Agents · Skills`. `SettingsPane::Mcp`, its route slug, icon and `surfaces/settings/mcp.rs` are deleted. Anything that deep-linked to `settings/mcp` lands on Agents.

### Non-goals

- Managing the host's MCP servers. That is the later catalog spec, which reclaims the `MCP servers` name.
- Changing the registration format or the binding-dir semantics in `ops/mcp.rs`.
- A per-platform default. Both platforms register by default; the only opt-out is Unregister on the row.

## Implementation phases

1. **Design** — done 2026-09-09: `Settings — Agents` (`n1krgH`) split into one card per runtime with the label/value `Model · Effort` line, the `Runner MCP` line with its button, a TRAE card, Browse and Reset labels, and the footnote sentence; `cmp/SettingsNav` Integrations reads Agents · Skills (the canvas never had those two entries; MCP removed). No new frame.
2. **App** — `app_store`: the MCP-defaults pass unconditional on both platforms, module renamed; `surfaces/settings/agents.rs`: one card per runtime, the label/value lines, the `Runner MCP` line and button per runtime row with the four states above, the Reset button relabelled, error routing into the validation caption; `settings_page.rs`: pane, route and nav entry removed, `INTEGRATION_PANES` updated; the MCP pane's state tests (`mcp_row_presentation`) move to the Agents pane's; the settings-nav search test count adjusted. `Settings — MCP` (`N0eIeV`) is deleted from the canvas when this lands.
3. **Docs** — `docs/arch` references to Settings → MCP repointed at Agents; this spec archives on landing.

## Verification

- A fresh macOS profile with `claude` and `codex` on PATH registers both at first launch and records them in `initialized_mcp_clients`; Unregister on codex sticks across restarts and a runtime refresh; re-enabling a disabled agent registers it once.
- Agents shows a `Runner MCP` line on the Claude Code, Codex and TRAE cards; Register writes exactly the `runner` entry (diff `~/.claude.json` / `~/.codex/config.toml` / `~/.trae/traecli.toml` before and after); Unregister removes it and nothing else; a read-only config file surfaces the error in the row's caption; an entry pointing at another binary shows the warning state and Register replaces it.
- Settings nav search no longer finds MCP; `settings/mcp` routes to Agents.
- Existing `ops/mcp.rs` tests unchanged.
