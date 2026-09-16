# 590 — Runtime model discovery and cache

Tracking: [#590](https://github.com/yicheng47/runner/issues/590) · P2 · Shipped in [PR #591](https://github.com/yicheng47/runner/pull/591) on 2026-09-14 and closed 2026-09-16; the native Windows smoke (checklist item 6) is outstanding as a follow-up, and Copilot discovery is a follow-up under [540](./540-copilot-cli-runtime.md).

## Goal and scope

Runner's hardcoded Codex catalog omitted models exposed by the installed CLI. Use cmux's query routes for Codex and Claude Code, cache the last usable answer, and update existing model pickers without making users wait. Query failures must be silent in the UI; successful queries must be logged.

Scope reduced with Jason on 2026-09-14: one cached catalog per runtime, small CLI adapters, and existing picker updates. The CLI owns provider and configuration interpretation. Runner does not inspect provider URLs, credentials, profiles, configuration files, or project settings for model discovery. Runner-specific commands, arguments, and environment overrides share the runtime's standard suggestions; custom model text remains available and the CLI validates it at launch. Jason subsequently approved a narrow addition: cache the effort metadata already returned with each model and filter existing effort dropdowns from it. Provider/configuration inspection remains outside this change.

TRAE CLI keeps its current Default-only suggestions, custom model input, configured defaults, effort choices, and launch behavior. No TRAE query or cache is added. Shell has no catalog. Additional runtimes and the Windows hook work under [#347](./347-hook-based-session-status.md) remain separate.

## Queries

| Runtime | Query | Deadline |
| --- | --- | --- |
| Codex | Effective executable with `debug models`; parse visible JSON models. | 5 seconds |
| Claude Code | Effective executable with `-p --input-format stream-json --output-format stream-json --include-partial-messages --verbose --safe-mode --no-session-persistence`; send the control request below and close stdin. | 30 seconds |

```json
{"type":"control_request","request_id":"runner-list-models","request":{"subtype":"list_models"}}
```

Claude parsing accepts only the matching successful control response. Preserve selectable IDs, aliases, labels, descriptions, and order; deduplicate exact IDs and use Claude's Default marker to identify its default model. Codex entries must have `visibility: list`. Retain Codex `supported_reasoning_levels` and Claude `supportsEffort`/`supportedEffortLevels` on each model as optional supported-effort IDs. A known non-configurable model has an empty list; unavailable metadata remains unknown. Current Claude responses omit the fields for non-configurable models while providing them for others; responses that omit effort metadata throughout retain the runtime fallback. Runner supplies its own Default/inherited row. Empty, malformed, unsupported, or failed responses never replace a usable cache.

Use the existing effective executable and login-shell environment. Run queries from the user's home directory through Runner's headless process support, with bounded output and owned-process cleanup on completion or timeout. Claude's extra flags suppress hooks, MCP servers, plugins, and transcript persistence. Neither query sends an inference prompt.

Runner makes no direct provider HTTP request. Inspected Codex 0.154.0 uses `OnlineIfUncached`: its own five-minute cache can satisfy the command, while a cold cache can trigger a backend request for eligible configurations. Inspected Claude Code 2.1.270 builds `list_models` from CLI state, but process initialization can still make network requests. A usable CLI answer does not prove a fresh backend fetch. Runner does not read Codex's `models_cache.json` directly; the CLI manages its own cache.

## Cache and refresh

Store models and their supported effort levels together in one last-good catalog per runtime in memory and SQLite `_app_state`, under `runtime_model_catalog:<runtime>`. Match the effective executable path, executable modification time/size, and configuration home (`CODEX_HOME` or `CLAUDE_CONFIG_DIR`, otherwise the usual home directory). Do not retain catalogs for multiple installations or inspect their provider configuration. Existing version-4 records remain readable so this simplification preserves the saved offline cache; obsolete extra metadata is ignored.

- Load the saved catalog before background executable discovery. Matching cached suggestions remain available while refresh runs.
- A successful catalog is fresh for ten minutes. Startup, model-form opens, and runtime selections request freshness; this is on-demand refresh, not a repeating timer.
- Settings → Agents → Refresh bypasses Runner's freshness and failed-attempt cooldown. The CLI still controls its own internal cache.
- Coalesce queries per runtime, including overlapping requests from multiple windows. Codex and Claude queries run independently.
- Query failure preserves the previous catalog and success timestamp. Automatic attempts wait ten minutes after a failure; manual Refresh can retry immediately. With no compatible cache, retain existing built-in suggestions and Default/custom input.
- Executable changes request a new catalog. Discard results for an executable/configuration home that is no longer current. Account or provider changes at the same installation use the shared suggestions until a later query or manual Refresh.
- A failed cache write keeps the new usable result in memory and leaves the last persisted record available for restart.

Only installed, enabled Codex and Claude Code runtimes are queried. Executable discovery continues to cover all runtimes. Discovery failure cannot change runtime availability, block app startup or chat launch, clear selections, or create error UI.

## Integration and logging

`runtime_status/models.rs` owns scheduling, last-good persistence, process execution, and publication; `models/codex.rs` and `models/claude.rs` own only their query and parser. Use existing SQLite and `runtime/changed` paths. Start Chat, runner create/edit, and crew-slot forms request freshness and consume the shared catalog. Refresh suggestions without rewriting typed models or changing focus. Start Chat and runner/slot editing filter their existing effort choices for the selected model, always retaining Default. A model with no configurable effort leaves only a disabled Default choice. Incompatible explicit effort choices in an open form reset to Default on model or catalog changes; compatible choices remain. Choosing Default model always clears the effort override and disables the effort selector at Default, even when the inherited model is known. Show known default model and effort values in the field labels without storing explicit overrides; unknown values remain simply Default. Named models with unknown capabilities and caches without metadata retain existing runtime-level choices. Default keeps existing inheritance semantics, and the CLI remains authoritative for custom configurations and inherited settings; filtering does not guarantee every launch combination succeeds. Existing intentional resets when switching runtimes remain unchanged. Settings' Model/Effort labels use content width and a shared text baseline.

Log each usable query result at info as `runtime_model_query_succeeded`, with runtime, method, model count, duration, and whether it was applied. Cache hits are not query successes. Failed queries log a classified reason at debug; persistence failure is also debug-only. Do not log raw CLI output or credentials.

## Validation

Automated coverage includes captured CLI response parsing, optional effort metadata, per-model filtering, version-4 cache compatibility, restart reuse, stale reads, ten-minute freshness/cooldown, forced refresh, concurrent request coalescing, obsolete-source rejection, nonzero exits, empty/malformed output, crashes, spawn failure, timeout cleanup, failed persistence, and TRAE query exclusion. Cache tests verify that model IDs and effort levels survive restart and failed refresh together. On 2026-09-14, `cargo test --locked -p runner-backend -p runner-app --no-fail-fast` passed all 1,130 tests after the effort filtering and Default-control changes. Workspace Clippy (`--all-targets --profile ci -- -D warnings`), formatting, and `git diff --check` passed. Jason confirmed the final macOS smoke test passed after the Default-control changes. Native Windows manual verification remains pending.

Historical macOS evidence from the larger implementation, 2026-09-14: Codex 0.154.0 returned six visible models; Claude Code 2.1.270 returned four models plus its default. Claude returned the same options with and without safe-mode/persistence flags; the flagged query created no transcript or hook events. At 21:49 CST the development app logged successful applied Codex (six models, 59 ms) and Claude (four models, 3,216 ms) queries without app errors. Jason restarted offline at 21:54; no model queries or errors were logged, and SQLite retained the same catalogs and original capture timestamps. This verifies fresh-cache reuse in that build, not the new simplified build or a failed stale refresh.

Smoke-test checklist for the simplified build (macOS smoke confirmed; native Windows pending):

1. Open Start Chat, runner create/edit, and a crew-slot form with populated caches. Lists should appear immediately. Keep custom text and a compatible effort while refreshing from another window; both and keyboard focus should survive. After a successful Refresh populates effort metadata, switch to a model that excludes the chosen effort and confirm it resets to Default; Haiku should leave only Default. Choose Default model after an explicit effort: effort must clear to Default and become disabled, with known defaults shown only in labels. Select a named model again and confirm its supported efforts become selectable. Verify the same filtered choices after an offline restart.
2. Press Settings → Agents → Refresh twice quickly. Confirm one in-flight query per runtime, a success log for each usable result, and no cleared lists or console flash.
3. Disconnect the network and force Refresh. If the CLI fails, cached lists and selections should remain without error UI. If its own cache supplies a usable answer, a success log is expected. Reconnect and refresh again.
4. Relaunch inside ten minutes and confirm cache reuse without a CLI query. After ten minutes, open a model form and confirm a background refresh with stale suggestions still visible.
5. Change an executable override and confirm its catalog refreshes. Disable an agent and confirm startup/form opens/Refresh do not query it. TRAE must retain its existing behavior throughout.
6. On native Windows, repeat cache reuse, forced offline Refresh, spaced executable paths and supported CLI shims, and chat launch with a discovered model. Check for console flashes and timeout cleanup. Record CLI versions and unavailable combinations. Native Windows verification is pending.

## References

- [cmux cache](https://github.com/manaflow-ai/cmux/blob/main/Packages/macOS/CmuxControlSocket/Sources/CmuxControlSocket/MobileTaskModels/MobileTaskModelDiscovery.swift): ten-minute cache and shared queries; Runner additionally persists and retains stale catalogs on failure.
- [cmux provider strategies](https://github.com/manaflow-ai/cmux/blob/main/Packages/macOS/CmuxControlSocket/Sources/CmuxControlSocket/MobileTaskModels/MobileTaskModelProviderStrategy.swift): Codex command and Claude `list_models` routes, inspected on 2026-09-14.
- [Claude model configuration](https://code.claude.com/docs/en/model-config): model selection semantics, rather than documentation of the control protocol. Protocol evidence came from local Claude Code 2.1.270 inspection and control-only probes, including a successful response through a rejecting proxy despite startup connection failures.
- [Codex 0.154.0 command](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/cli/src/main.rs) and [models manager](https://github.com/openai/codex/blob/rust-v0.154.0/codex-rs/models-manager/src/manager.rs): `OnlineIfUncached`, internal cache, and conditional remote refresh.
