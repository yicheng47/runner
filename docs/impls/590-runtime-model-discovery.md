# 590 — Model discovery continuation: Claude crew brief

Jason approved the feature spec and requested this Claude crew mission on 2026-09-14. Continue [PR #591](https://github.com/yicheng47/runner/pull/591) for [#590](https://github.com/yicheng47/runner/issues/590) in `/Users/jason/repos/yicheng47/runner` on the existing branch `fix/codex-model-discovery`. The starting implementation commit is `15e9834bf03c5c8a4a1d00230862b3477ca789fb`.

Read `AGENTS.md` and `docs/features/590-runtime-model-discovery.md` before editing. The approved spec is authoritative; the older GitHub issue/PR descriptions still contain earlier scope. This brief, the feature spec, and their index entries are mission preparation. Preserve them and any unrelated changes. The supervising session is handing implementation to this crew and will not edit code concurrently.

## Ownership and authorization

Use the existing two-slot crew: Claude Code `coder` owns implementation and checks; Codex `reviewer` waits for an explicit Runner handoff, then reviews the change against the approved spec. Review the inherited #591 implementation as well as the continuation so problems in the combined feature are covered. Communicate and iterate through Runner until no must-fix findings remain. End turns after handoffs; delivery is push.

Stay on the named branch in this checkout. No additional agents, missions, worktrees, or checkouts. Leave implementation uncommitted for Jason's smoke test. No implementation commits, push, PR creation/edit, merge, issue closure, branch deletion, or app restart/update is authorized. The existing PR does not grant PR mode. If preparation documents remain dirty, they are expected, authorized task changes, not a reason to stop or stash them.

## Deliverable

- Use cmux's chosen query methods for **Codex and Claude Code**: `codex debug models`, and Claude's control-only stream-JSON `list_models` request. The exact request/flags and source links are in the spec. Adapt to Runner's effective executable/environment and native process support; retain model IDs, aliases, defaults, visibility, and supported effort metadata.
- Preserve **TRAE CLI's current behavior**: Default-only suggestions, existing custom text, configured defaults, effort and launch behavior. Do not add TRAE model discovery or change its enabled state.
- Generalize #591's backend model cache to runtime/source-scoped memory and SQLite last-good records. Show cached data immediately, refresh at ten-minute freshness on demand, coalesce same-source queries across fields/windows/Refresh, and preserve cache during refresh. Manual Refresh bypasses Runner's TTL/cooldown; the CLI remains in charge of its own internal cache.
- Make discovery optional background work. Spawn errors, crashes, nonzero exit, protocol errors, unusable output, and timeout must fail silently, preserve cache/selections, clear checking/in-flight state, and never fail app startup, catalog reads, or chat launch. With no cache, retain Default/custom values and applicable fallbacks. Avoid retry storms and cross-runtime blocking.
- Log usable query success at info with runtime, query method, model count, duration, and whether the result was applied. Cache hits and disk fallbacks are not query successes; errors are bounded debug diagnostics, never error UI or raw output. A successful CLI result does not prove a fresh network fetch.
- Honor executable/configuration/provider source identity, reject obsolete results, update already-open model/effort fields without losing edits/focus, and clean up owned query processes on macOS/Windows. Follow existing local patterns; keep changes scoped and simple.

## Starting points and evidence

The initial Codex implementation lives in `crates/runner-backend/src/runtime_status/models.rs`, with wiring in `runtime_status.rs`, `ops/runtime.rs`, `shell_path.rs`, and `crates/runner-app/src/bootstrap.rs`. Model consumers include `surfaces/start_chat.rs`, `surfaces/runners/`, `surfaces/crews/add_slot.rs`, and `ui/model_field.rs`. Reuse the existing SQLite `_app_state`, `runtime/changed`, and headless `ProcessTree` support.

Installed versions inspected here: Codex 0.154.0 and Claude Code 2.1.270. Codex's command uses `OnlineIfUncached`; its own cache TTL is five minutes. Claude's handler builds models synchronously from local state, but process initialization can make network calls and start configured integrations. A control-only probe through a rejecting proxy returned five options successfully while observed connections failed. Preserve normal user configuration/authentication/conversations; do not send inference prompts just to discover models. Verify and bound unrelated startup work.

The earlier #591 validation is historical, not validation of the continuation: backend tests and workspace Clippy passed, and the macOS Codex query returned six visible models. The host is macOS. Implement native Windows paths and portable regression coverage, then leave an explicit native Windows smoke checklist if no Windows environment is available; do not claim those checks passed or close #590.

## Verification and handoff

Exercise real CLI response parsing and meaningful cache/concurrency/process regressions: reopened SQLite, stale display during refresh, coalescing, manual bypass, failure cooldown/recovery, stale-source rejection, failed cache writes, empty/malformed/error output, child crash/timeout/cleanup, and Default/custom/model-specific effort behavior. Verify that injected query failures cannot affect startup or normal chat launch. Cover cross-window fields and TRAE preservation where shared consumers change.

Run relevant backend tests, workspace Clippy with `--all-targets --profile ci -- -D warnings`, formatting and `git diff --check`; include `runner-app` tests if UI consumers change. Use appropriate permissions for local socket/process tests and report environmental limits. Update implementation notes and smoke instructions with actual results.

Final Runner handoff: branch and changed files, resulting behavior, checks/results, reviewer verdict, remaining platform verification, and a short manual smoke checklist. Stop at clean working-tree review for Jason; leave #590 and #591 open.
