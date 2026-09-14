# 590 — Runtime model discovery and cache

Tracking: [#590](https://github.com/yicheng47/runner/issues/590) · P2 · In progress. Initial implementation: [PR #591](https://github.com/yicheng47/runner/pull/591).

## Motivation

Runner v0.9.0's hardcoded Codex catalog omitted GPT-6 Astra even though the installed CLI exposed it. The same problem applies to every agent runtime: available models depend on the installed CLI, provider, account, and configuration. Model fields should use each runtime's own catalog without making the user wait for discovery.

Decision, 2026-09-14: use cmux's existing Codex and Claude Code model-query approach, small per-runtime adapters, ten-minute cache, and shared in-flight queries. These query routes are the implementation choice. Runner also persists its last good catalogs across restarts and serves them immediately during refresh. Offline operation and any query failure must be silent in the UI and keep the last good catalog. Successful model queries must be logged. TRAE keeps its current behavior.

## Scope

- Enable discovery for **Codex and Claude Code**. A Codex-only implementation does not complete this feature.
- Keep TRAE CLI's current behavior: Default is its only model suggestion, with existing custom model entry, configured-default handling, effort choices, and launch behavior preserved. Do not add TRAE model queries or caching in this scope.
- Feed Start Chat, runner create/edit, and crew-slot model fields through one backend catalog. Settings → Agents → Refresh refreshes model catalogs for installed, enabled Codex and Claude Code runtimes; executable discovery continues to cover all runtimes.
- Support macOS and native Windows wherever the runtime itself is available. Honor executable overrides, runtime configuration, and the launch environment.
- Preserve Default/inherited choices, custom model text, and saved model/effort selections throughout discovery and failure.
- Carry model-specific effort metadata when available; do not infer it from a model name or copy another runtime's capabilities.

Shell has no model catalog. pi and GitHub Copilot CLI join this path when their adapters land under [#539](./539-pi-runtime.md) and [#540](./540-copilot-cli-runtime.md); Copilot remains parked under #540. This work does not add runtimes or change agent enablement defaults.

The discovery, cache, and model-capability requirements below apply to Codex and Claude Code. This changes existing fields' data and behavior. New controls or layout require a Pencil pass. Lifecycle hooks and their Windows rollout remain separate work under [#347](./347-hook-based-session-status.md).

## Runtime adapters

| Runtime | Query | Fallback and verification boundary |
| --- | --- | --- |
| Codex | Effective executable with `debug models`; parse visible JSON entries. | Reuse #591. A compatible CLI-owned `models_cache.json` can seed an empty Runner cache; retain the small built-in catalog for a first run without discovery. |
| Claude Code | cmux's stream-JSON control request with subtype `list_models`, without a user prompt. | Follow cmux's response parsing and default-model extraction. Preserve compatible aliases and provider-specific IDs. Older unsupported CLIs use cache or fallback. |

Run Codex's effective executable with `debug models`. For Claude, run the effective executable with `-p --input-format stream-json --output-format stream-json --include-partial-messages --verbose`, write one newline-terminated control request to stdin, and close stdin as cmux's pipeline does:

```json
{"type":"control_request","request_id":"runner-list-models","request":{"subtype":"list_models"}}
```

Match the response to that request ID and use cmux's normalization of models and the runtime default. Adapt process execution to Runner's existing Rust support, executable overrides, and native Windows handling. Compatibility probes validate this chosen route; a failed or unsupported query follows the cache policy below.

The adapter owns its query, parser, source inputs, local fallback, and timeout. Shared code owns scheduling, persistence, failure policy, and publication. Use ordinary runtime dispatch and the existing SQLite/event paths; no plugin framework or separate model service.

Local evidence on 2026-09-14: Codex 0.154.0 documents `debug models` as JSON output. Claude Code 2.1.270's cmux-style control query returned five options, including Default, with a matching successful control response. A local rejecting proxy observed startup connection attempts to Anthropic, telemetry, and configured integrations while the command still returned its model list. This verifies the local CLI route; Runner integration and native Windows checks remain pending.

### Network boundary

Runner invokes the installed CLI and does not call a provider's model API directly. In Codex 0.154.0, `debug models` uses `OnlineIfUncached`: a usable CLI cache avoids the model-catalog request; otherwise, eligible provider/auth configurations fetch the remote catalog. Codex's own cache has a five-minute TTL, separate from Runner's ten-minute cache. The `--bundled` option reads only the binary's bundled catalog, so it is not the discovery route.

In the inspected Claude Code 2.1.270 binary, `list_models` synchronously builds options from existing CLI state, settings, and cached catalog/capability data; its handler does not perform a model-catalog HTTP request. Launching the CLI can nevertheless trigger network activity during initialization, including authentication/configuration, telemetry, and configured integrations. The successful proxy-blocked probe is evidence that the list can be returned despite those failed connections, not a guarantee that CLI startup never uses the network. Neither query submits an inference prompt.

Runner's success log means the CLI returned a usable catalog, not that the CLI fetched new data from its server. Settings Refresh forces a new CLI query while leaving the CLI's internal cache/refresh policy in charge. Keep the cache and silent-failure requirements below even when the handler can answer locally.

Future runtimes must supply a verified noninteractive listing route and preserve provider-qualified IDs. OpenCode in cmux is an adapter reference, not additional Runner scope.

## Cache and refresh behavior

### Immediate reads

Load compatible persisted catalogs into memory at startup. Model fields read a snapshot immediately; opening a field, switching runtimes, and launching a chat never wait for a query. After executable discovery, startup may refresh stale catalogs for installed, enabled agents. Opening a model field or switching its runtime requests freshness for that source, including after hours of app uptime. Rendering itself never starts subprocesses.

Keep the last good catalog for the same source regardless of age. Ten minutes is the refresh threshold, not an expiration date for usable suggestions. Keep the displayed list and selection stable until a valid replacement arrives. With no compatible Runner cache, use a verified CLI-owned catalog if available, otherwise existing fallback suggestions plus Default and custom text.

### Scheduling

- A successful catalog is fresh for ten minutes; fresh cache hits do not invoke the CLI.
- A stale or missing catalog schedules one background query while the current list stays visible. Do not continuously poll unused runtimes.
- Coalesce requests per runtime/source across windows, fields, startup, and Refresh. A slow or failed runtime must not delay another runtime's results.
- Settings Refresh bypasses freshness and failure cooldown but joins an already-running query for that source. Repeated clicks do not spawn duplicates or clear the list.
- After failure, automatic requests wait ten minutes before retrying that source. Track last attempt separately from last success, so failure neither renews freshness nor causes retries on every field open.
- Executable or relevant configuration changes select a new source and permit a query. Obsolete results cannot replace the active source's memory or persisted catalog.

### Silent failure

Offline operation, connectivity errors, missing authentication, unavailable commands, unsupported protocols, nonzero exits, malformed output, timeouts, and empty/unusable results preserve the last good catalog. No error toast, dialog, banner, login prompt, console window, disabled model field, or cleared selection. Manual Refresh follows the same policy and finishes its existing checking state normally.

Discovery is optional background work. Handle process-spawn errors, child crashes, and control-protocol errors inside the discovery worker; they must not fail app startup, catalog reads, or chat launch, block the UI, or stop another runtime's discovery. Capture command output without forwarding failures into a terminal pane or notification. Every failure clears the query's in-flight/checking state so a later refresh can recover.

Never replace a good cache with an empty list or built-in fallback after a failed query. Never advance its successful-capture timestamp on failure. On a first run without any cache, Default/custom entry and applicable built-in suggestions remain usable. Model-query failure does not change whether an installed runtime is considered available; the CLI remains responsible for accepting a selected model at launch.

Cache read/write failures are silent too. A failed disk write leaves the successful in-memory result usable. No network-connectivity preflight is needed: run the bounded query and apply the same failure policy regardless of cause.

### Source identity and persistence

Generalize the existing SQLite `_app_state` storage to versioned per-runtime/source records. Store normalized models, optional capabilities/default metadata, source identity, provenance, and last successful capture time. Do not store raw output or credentials.

A source includes runtime, effective executable path and replacement fingerprint, effective configuration home, and relevant configuration/provider/profile identity from the launch environment. Observe known configuration/authentication changes through nonsecret metadata; never put tokens, keys, or credential-bearing URLs into cache keys or logs. Manual Refresh covers account/entitlement changes with no observable local change.

Project or runner-specific provider/configuration overrides must not borrow an unrelated global catalog. Include their context in the source/query where Runner can reproduce it. Otherwise use Default/custom values and compatible fallbacks without treating global suggestions as verified for that context. Never forward arbitrary runner arguments or prompts into discovery.

Stale reuse is limited to the matching source. Retain old source records for returning to them, but do not publish their delayed results over a new source. Ignore incompatible cache schemas or unprovable CLI-owned catalogs silently. CLI-owned disk fallback keeps its provenance; reading it is not a successful live query and must not reset a good catalog's success time.

## Model and effort fields

Preserve IDs, display labels, descriptions, order, aliases, and provider qualifiers. Deduplicate by exact selectable ID within a source, filter explicitly hidden/disabled entries, and keep the Default/inherited row first. Missing optional metadata does not invalidate a usable model ID.

Updates change suggestions without rewriting saved values, the user's text, or an explicit effort selection. Keep custom/saved values visible even when absent from the refreshed list; discovery is not a launch allowlist. Do not mix built-in suggestions into a successful discovered list and accidentally restore hidden entries.

Distinguish absent effort metadata from an explicit lack of configurable effort. Use the selected model's proven options when available; otherwise retain the runtime's existing fallback behavior. Preserve unknown explicit values and let the CLI validate them. Inherited models follow the existing runner/slot/runtime precedence before resolving effort suggestions.

Publish accepted changes through `runtime/changed`. Already-open forms and other windows update suggestions without losing edits or focus. Cache age and query errors do not appear in the model picker.

## Process execution and logs

Use the existing headless process support off the UI thread, with structured argv/stdin rather than cmux's Unix shell pipeline. Support spaced executable paths and native Windows wrappers, honor runtime environment/home overrides, bound output, and terminate/reap owned processes and descendants on timeout or cancellation. Windows queries must not open a console window.

Use cmux's five-second Codex and thirty-second Claude deadlines initially. The longer Claude deadline accommodates stream initialization while the UI keeps using cache. Queries submit no inference prompt, resume no existing conversation, and do not install/update CLIs or rewrite user configuration. Verify control-only Claude startup and suppress unrelated hooks/MCP initialization with supported query-mode controls where needed.

Log one **info** event, `runtime_model_query_succeeded`, for each parsed, usable query result: runtime, query method, model count, and elapsed milliseconds. Cache hits and local-disk fallbacks are not query successes. If a valid result became obsolete during the query, log `applied=false` and do not publish it. Persistence failure after a valid query is a separate debug diagnostic.

Failures may produce a bounded **debug** diagnostic containing runtime, duration, and a classified reason (`timeout`, `query_failed`, `invalid_output`, or `empty_catalog`). Do not log raw stdout/stderr or credentials. Cache hits need no info log. Ordinary offline use must not create repeated warnings or any user-facing error.

## Implementation phases

1. **Port cmux's query paths.** Reuse #591's Codex command and implement cmux's Claude control request and response parsing. Capture sanitized outputs with CLI versions and verify model/default fields, optional effort metadata, control-only behavior, and native Windows execution. Unsupported versions follow the existing fallback policy.
2. **Generalize discovery.** Replace Codex-only state, cache keys, and refresh guards with per-runtime/source records, shared queries, demand-driven freshness, persistent last-good retention, failure cooldown, and success logging.
3. **Wire discovery and fields.** Add the Claude adapter alongside Codex and refresh already-open model/effort fields across windows. Apply the same silent-failure behavior at startup and on manual Refresh. Preserve TRAE's existing behavior.
4. **Verify both platforms.** Run focused parser/cache/process tests and macOS/Windows smoke checks. Record unavailable runtime/platform combinations; untested supported combinations remain pending.

## Current PR boundary

PR #591 already queries Codex in a background worker, persists parsed models in SQLite, reuses a matching fresh startup cache, publishes a stale startup cache before querying, supports Settings Refresh and executable overrides, and adds GPT-6 Astra to the built-in fallback. Its source identity covers executable path/mtime and Codex home. Reported validation includes backend tests, workspace Clippy, formatting, and a live macOS Codex probe returning six visible models.

Claude discovery, demand-driven expiry, independent coalescing, complete configuration/provider identity, per-model effort, and the full failure/logging contract above remain continuation work. Native Windows discovery still needs a smoke test. Keep #590 open until the Codex/Claude scope and supported-platform checks are complete. TRAE discovery is not required for completion. The hook-status README notes already in #591 are separate from this feature.

## Verification and acceptance

- [ ] Installed, enabled Codex and Claude Code runtimes query through their own verified adapters; failure in one does not block the other.
- [ ] TRAE keeps Default as its only model suggestion and preserves custom model entry, configured defaults, effort choices, and launch behavior. Startup, field opens, and Settings Refresh do not issue TRAE model queries.
- [ ] Fresh persistent cache survives relaunch without a query. Stale cache appears immediately during one background refresh, including in a long-running app.
- [ ] Multiple fields/windows and repeated Refresh clicks share a query. Failure cooldown prevents automatic retry loops; manual Refresh bypasses it.
- [ ] With populated caches, disconnect the network and force Refresh for each runtime. Lists, selections, focus, and runtime availability stay unchanged, with no error UI or console flash. Reconnect and verify a later success replaces the catalog.
- [ ] Cold start without a usable cache/query retains Default/custom entry and applicable fallback suggestions. Empty/malformed output, nonzero exit, missing auth, and timeout cannot replace a good cache or renew its success timestamp.
- [ ] Inject discovery spawn failures, child crashes, control errors, and hung queries. App startup, model fields, and normal chat launch remain usable; cached lists/selections survive, no error reaches the UI, checking finishes, and a later successful refresh recovers.
- [ ] Successful queries emit one info log with runtime, model count, and duration; cache hits/fallbacks do not. Failures remain debug diagnostics without raw output or credentials.
- [ ] Executable replacement, overrides, configuration homes, provider/profile changes, and project contexts cannot reuse unrelated catalogs. Delayed old queries cannot overwrite the active source in memory or SQLite.
- [ ] Start Chat, runner create/edit, and crew-slot fields update across windows without rewriting custom/saved model or effort values. Hidden entries stay out of general suggestions.
- [ ] Model-specific effort, absent metadata, explicit no-effort capability, and inherited defaults follow the defined behavior.
- [ ] On macOS and native Windows, test spaced executable paths and supported wrappers, relaunch cache reuse, forced Refresh without UI blocking/console flash, timeout cleanup, and launching a chat with a discovered model for each available runtime. Record CLI versions and unavailable combinations.
- [ ] Verify Claude's query creates no user turn and leaves configuration/existing conversations unchanged.
- [ ] Run relevant backend tests and workspace Clippy; include `runner-app` tests if field behavior changes. Run formatting and `git diff --check`.

## References

- [cmux cache](https://github.com/manaflow-ai/cmux/blob/main/Packages/macOS/CmuxControlSocket/Sources/CmuxControlSocket/MobileTaskModels/MobileTaskModelDiscovery.swift): ten-minute per-provider memory cache and shared queries. Runner adopts that structure while persisting and retaining stale catalogs on failure; cmux's current method awaits an expired refresh and caches the returned fallback result.
- [cmux provider strategies](https://github.com/manaflow-ai/cmux/blob/main/Packages/macOS/CmuxControlSocket/Sources/CmuxControlSocket/MobileTaskModels/MobileTaskModelProviderStrategy.swift): Codex command/disk fallback, Claude `list_models`, and per-runtime deadlines. Inspected through the GitHub API on 2026-09-14.
- [Claude model configuration](https://code.claude.com/docs/en/model-config): aliases, provider-specific model values, defaults, and restrictions. This documents selection semantics, not the `list_models` wire protocol.
- [Codex 0.154.0 debug-model command](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/cli/src/main.rs): `run_debug_models_command` selects `OnlineIfUncached` unless `--bundled` is set. The [matching models manager](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/models-manager/src/manager.rs) owns the five-minute cache and conditional remote refresh.
- Local evidence, 2026-09-14: CLI versions/help, the `list_models` handler and option-building functions embedded in Claude Code 2.1.270, and a control-only Claude query through a local rejecting proxy. The query exited 0 in approximately one second and returned five options despite observed connection failures; no user prompt was sent.
