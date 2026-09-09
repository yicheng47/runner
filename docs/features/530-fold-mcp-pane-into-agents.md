# 530 — Fold the MCP settings pane into Agents

Tracking: [#530](https://github.com/yicheng47/runner/issues/530). Status: specced 2026-09-09, P2, not scheduled. Design happens on the existing `Settings — Agents` frame (`n1krgH`) in `design/runner.pen` before the app phase.

## Motivation

Settings → MCP does one thing: it registers Runner as an MCP tool server inside each agent's config — the `runner` entry in `~/.claude.json`, the `runner` table in `~/.codex/config.toml` — one row per agent, plus the binding directory those entries point at. Nothing in it manages the host's MCP servers. But a nav item named **MCP** under *Integrations* promises exactly that, and the deferred half of [#73](https://github.com/yicheng47/runner/issues/73), an agent-agnostic MCP-server catalog, will want that name later with no baggage.

The pane's content already has the Agents pane's shape: one row per runtime. Registering Runner is one-time setup per agent, not something people revisit, so it belongs on the agent's row rather than behind its own nav item.

The default also differs by platform today. On Windows the app registers Runner into every detected, enabled agent at startup and whenever the runtime list or the enabled-agents set changes, once per client (`app_store/mcp_defaults_windows.rs`, marked in `initialized_mcp_clients`). On macOS that module is compiled out, so a fresh install stays unregistered until the user finds the MCP pane and presses Register. The pane therefore shows a state the app already set on one platform and is the only way to reach it on the other.

## Scope

### Register by default, on both platforms

`initialize_mcp_defaults` loses its `cfg(windows)` gates and runs on macOS and Windows alike: at startup, on runtime refresh, and when the enabled-agents set changes, every detected and enabled runtime with an MCP config is registered once and recorded in `initialized_mcp_clients`. The once-per-client rule is what makes Unregister stick — a client the user turned off is never re-registered by the default pass. Existing macOS installs already carry the `initialized_mcp_clients` setting (it defaults empty), so the first launch after this lands registers their agents the same way a fresh Windows install does. The module is renamed to drop the `_windows` suffix; what it writes (`ops/mcp.rs`) is unchanged.

### Agents pane

- Each runtime row gains a second caption line under the executable field, in the same faint style as the existing `Model: … · Effort: …` line (feature 65): `Runner MCP: registered in ~/.claude.json` or `Runner MCP: not registered`. One ghost button sits at the row's right edge on that line — **Unregister** in the normal case, since the default pass has already registered the agent, and **Register** for a client the user turned off or whose write failed. The action is the existing `ops/mcp.rs` write; nothing about what is written changes.
- A failed write reports in the row's existing red validation caption, not in a separate error card.
- Runtimes without an MCP config file (trae, qoder) get no line, not a disabled one — the same rule the Skills pane applies to its toggle.
- The **binding directory** becomes a third card at the bottom of the pane, `Runner MCP server`, holding the one field and its caption (*"Directory the client entries point at to launch Runner's MCP server."*). It applies to every registration and changes almost never; if Agents ever feels long it can move to Diagnostics.
- The pane footnote gains one sentence: registration writes only the `runner` entry in each agent's config.

### Nav

`MCP` leaves the Integrations group, which becomes `Agents · Skills`. `SettingsPane::Mcp`, its route slug, icon and `surfaces/settings/mcp.rs` are deleted. Anything that deep-linked to `settings/mcp` lands on Agents.

### Non-goals

- Managing the host's MCP servers. That is the later catalog spec, which reclaims the `MCP servers` name.
- Changing the registration format or the binding-dir semantics in `ops/mcp.rs`.
- A per-platform default. Both platforms register by default; the only opt-out is Unregister on the row.

## Implementation phases

1. **Design** — `Settings — Agents` (`n1krgH`) gains the two caption lines with their buttons and the binding-dir card; `cmp/SettingsNav` drops MCP. No new frame.
2. **App** — `app_store`: the MCP-defaults pass unconditional on both platforms, module renamed; `surfaces/settings/agents.rs`: the caption line and button per runtime row, the binding-dir card, error routing into the validation caption; `settings_page.rs`: pane, route and nav entry removed, `INTEGRATION_PANES` updated; the MCP pane's tests move to the Agents pane's; the settings-nav search test count adjusted.
3. **Docs** — `docs/arch` references to Settings → MCP repointed at Agents; this spec archives on landing.

## Verification

- A fresh macOS profile with `claude` and `codex` on PATH registers both at first launch and records them in `initialized_mcp_clients`; Unregister on codex sticks across restarts and a runtime refresh; re-enabling a disabled agent registers it once.
- Agents shows `Runner MCP` lines for claude-code and codex only; Register writes exactly the `runner` entry (diff `~/.claude.json` / `~/.codex/config.toml` before and after); Unregister removes it and nothing else; a read-only config file surfaces the error in the row's caption.
- The binding-dir field round-trips and the written entries point at it.
- Settings nav search no longer finds MCP; `settings/mcp` routes to Agents.
- Existing `ops/mcp.rs` tests unchanged.
