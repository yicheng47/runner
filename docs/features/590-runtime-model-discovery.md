# Runtime model discovery and cache

Tracking: [#590](https://github.com/yicheng47/runner/issues/590) · P2 · In progress

## Problem

Runner v0.9.0's hardcoded Codex catalog omitted GPT-6 Astra even though the installed CLI exposed it. Runtime catalogs also differ by provider and configuration, and reasoning levels can differ by model. Keep the UI responsive while discovering what the installed runtime actually supports.

## Intended behavior

- Serve pickers from memory immediately, backed by a persistent local cache that survives app restarts.
- Treat catalogs as fresh for 10 minutes. When stale data is needed, keep it visible and refresh in the background. Do not invoke a CLI per render or continuously poll unused runtimes.
- Share one in-flight query per source. Settings → Agents → Refresh bypasses the TTL; failures preserve the last good catalog and explicit user selections.
- Identify a source by runtime, effective executable, and relevant configuration/provider/host. Invalidate on meaningful changes without recording credentials.
- Preserve upstream model IDs, labels, descriptions, ordering, and supported reasoning levels where available. Exclude hidden or disabled entries and retain runtime defaults and a small fallback catalog.
- Bound subprocess work, honor runtime overrides and environment, and clean up owned processes on timeout on macOS and Windows.

This changes the data supplied to existing model pickers. Any new controls or layout require a separate Pencil pass.

## Initial Codex implementation

The initial PR queries `codex debug models` in the existing background discovery worker, with Codex's `models_cache.json` as a fallback. It persists the parsed visible catalog in Runner's SQLite `_app_state`, keyed by executable path, executable modification time, and Codex home. A matching fresh cache avoids a CLI query on startup; a stale cache is published before querying. Existing pickers read the in-memory catalog and receive `runtime/changed` updates. Settings Refresh forces a query, and executable override changes trigger discovery for the new source.

GPT-6 Astra is also included in the static fallback. Runtime defaults and the existing runtime-wide effort list remain unchanged. The PR also clarifies in both READMEs that Runner has no TRAE hook adapter and that Windows sessions currently estimate status from terminal activity and titles.

The 10-minute TTL is currently evaluated when discovery runs, such as startup or override changes. A long-running app does not yet request a refresh when a picker uses an expired catalog. Revision checks prevent older results from replacing newer ones, but model queries are not independently coalesced. Configuration/provider changes beyond executable identity and Codex home require manual Refresh. These limitations keep #590 open after the first PR.

## Remaining work

1. Add demand-driven background refresh in long-running apps and coalesce model queries per source.
2. Extend source identity for configuration/provider changes and preserve model-specific reasoning choices.
3. Audit Claude Code's stream-JSON `list_models` control request against supported CLI versions, then add discovery with compatible aliases and fallback behavior.
4. Define model discovery boundaries when [Copilot #540](./540-copilot-cli-runtime.md) and [pi #539](./539-pi-runtime.md) land. Their provider catalogs and protocols need their own checks.

## Validation and home-workstation handoff

The initial implementation has regression coverage for visible-model parsing, stale-result rejection, subprocess arguments and timeout cleanup, executable overrides, SQLite cache reuse after reopening the database, forced refresh, TTL expiry, and executable replacement. Live macOS discovery was checked with Codex 0.154.0 and returned GPT-6 Astra among six visible models. Native Windows discovery still needs a manual smoke test; this is separate from Windows hook support.

- [ ] On native Windows, start Runner with an installed Codex CLI and verify GPT-6 Astra appears after background discovery.
- [ ] Confirm the default choice and an explicit custom model remain usable; launch a chat with a discovered model.
- [ ] Relaunch within 10 minutes and confirm the cached list remains available without a new model query.
- [ ] Use Settings → Agents → Refresh and confirm the list updates without blocking the UI or opening a console window.
- [ ] Test a CLI executable override, including a path with spaces, and verify the list comes from that executable.
- [ ] With discovery unavailable, verify the last good list remains usable; on a clean cache, verify the static fallback includes GPT-6 Astra.

## References

- [cmux cache](https://github.com/manaflow-ai/cmux/blob/main/Packages/macOS/CmuxControlSocket/Sources/CmuxControlSocket/MobileTaskModels/MobileTaskModelDiscovery.swift): 10-minute in-memory cache with shared in-flight requests.
- [cmux provider strategies](https://github.com/manaflow-ai/cmux/blob/main/Packages/macOS/CmuxControlSocket/Sources/CmuxControlSocket/MobileTaskModels/MobileTaskModelProviderStrategy.swift): Codex command and disk fallback, Claude control request, and OpenCode command.
- [Orca native-chat enrichment](https://github.com/stablyai/orca/blob/main/src/renderer/src/components/native-chat/native-chat-session-option-enrichment.ts): per-agent/host memory reuse without timed expiry or persistent caching on this path.
- [Orca model commands](https://github.com/stablyai/orca/blob/main/src/shared/commit-message-agent-specs-primary.ts): runtime-specific discovery and fallbacks; other Orca surfaces can use different paths.
