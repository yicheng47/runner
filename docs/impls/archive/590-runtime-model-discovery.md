# 590 — Model discovery continuation

Continue [PR #591](https://github.com/yicheng47/runner/pull/591) for [#590](https://github.com/yicheng47/runner/issues/590) on `fix/codex-model-discovery`. The starting implementation is `15e9834bf03c5c8a4a1d00230862b3477ca789fb`. The [feature spec](../../features/archive/590-runtime-model-discovery.md) defines the current scope.

Jason stopped the original Claude crew and requested inline completion, then explicitly requested removing the overbuilt parts on 2026-09-14. This replaces the original mission brief. Implementation stayed inline in the existing checkout. Jason confirmed the final macOS smoke test passed and authorized committing, updating PR #591, merging it, and preparing v0.9.1. Native Windows manual verification remains pending.

## Implementation

- Keep Codex `debug models` and Claude's control-only `list_models` query as small adapters using the existing effective executable and login-shell environment. Preserve model IDs, aliases, labels, descriptions, visibility, and the Claude default marker.
- Share one background scheduler, bounded headless process helper, and one last-good memory/SQLite record per runtime. Keep ten-minute freshness and failure cooldown, manual Refresh bypass, query coalescing, silent failures, and success logs. Read existing version-4 caches after the simplification.
- Match only executable path/replacement metadata and configuration home. Let the CLI interpret providers and configuration. Remove provider/profile/project inspection, multiple-source history, and associated configuration observers/resolvers/tests. Following Jason’s later approval, retain only per-model effort metadata from the existing queries and the small dropdown filter.
- Use existing `runtime/changed` handling to update model suggestions in Start Chat, runner create/edit, and crew slots. Preserve custom text, compatible selections and focus. Filter effort choices from each model’s cached metadata, retaining Default and the existing fallback when metadata is unknown; incompatible explicit choices in open forms reset to Default. Default model always clears and disables effort at Default; show known defaults in labels without pinning them. Keep current inheritance and launch behavior. Keep the Settings Model/Effort alignment fix.
- Query only installed, enabled Codex and Claude Code runtimes. Preserve TRAE's behavior and all existing launch paths.

## Verification and handoff

Keep focused tests for parsers, restart/cache compatibility, freshness/failure cooldown, manual bypass, coalescing, obsolete results, failed persistence, bad responses, process crashes/timeouts/cleanup, and TRAE exclusion. Run workspace tests, workspace Clippy with `--all-targets --profile ci -- -D warnings`, formatting, and `git diff --check`.

Record actual results in the feature spec and report the reduced diff size. Historical macOS startup/offline evidence and the final smoke-test confirmation are recorded there; native Windows verification remains pending. Merge #591 after CI passes; keep #590 open for the outstanding Windows verification and remaining runtime follow-ups.
