# 723 — Cursor Agent CLI runtime

> Tracking issue: [#723](https://github.com/yicheng47/runner/issues/723). Cursor only; Grok remains separately scheduled.
> Status: draft PR, default-off. macOS backend smoke tested; native Windows unverified.
> Contract: [Runtime integration](../arch/runtime-integration.md).

## Motivation

Run the separately installed Cursor Agent CLI as a Runner runtime, including role chats, runtime-only chats and crew missions.

## Scope

Use runtime key `cursor`, UI name Cursor and detect only executable `cursor-agent`. Executable overrides in Settings → Agents take priority; users whose installation exposes only `agent` can select its absolute path there. Discovery does not probe the generic `agent` command.

Reuse Runner's existing PTY, environment, permission, role, mission and session pipelines. The Cursor adapter sends a fresh persona and mission goal as one positional prompt, and the exact selected model ID through `--model`. Attended chats strip bypass and mode flags, then pass `--trust` for the workspace selected in Runner; this avoids a separate workspace-trust prompt while keeping Cursor's configured tool approval policy. Cursor implements `--trust` by writing a persistent per-workspace `.workspace-trusted` marker in its native workspace configuration; it remains after the child exits. Runner passes the native flag and does not add a global trust/configuration writer. Bypass missions use `--force --sandbox disabled --approve-mcps --trust`; native explicit denials still apply. No common launch-blocker or mission preflight framework is added.

Cursor shares the managed Runner skill in `.agents/skills`; no automatic copy is installed in `.cursor/skills`. Settings reads both skill roots.

Enabled discovery queries the effective executable's `models` command through the existing background/cache pipeline with a 15-second timeout. Preserve account-specific IDs, labels and default markers, including effort/speed variants. Refresh bypasses the cache TTL; failures retain the previous list or Default model. No fixed model list or separate effort flag is added.

## Conversation identity

The adapter assigns a fresh UUID using `--new-session-id` and resumes only an explicit valid UUID using `--resume`. Both were verified against CLI `2026.10.01-e373342`; `--new-session-id` is a hidden native option, so older CLI compatibility is not claimed.

Cursor stores chats in `<config>/chats/<MD5 of canonical cwd>/<UUID>/store.db` on macOS/Linux. Resolve `CURSOR_CONFIG_DIR`, then `<XDG_CONFIG_HOME>/cursor`, then the role process's home and the normal `.cursor` root. Both variables were honoured by CLI `2026.10.01-e373342` in isolated macOS probes on 2026-10-10. The observer and resume probe use the same captured login-shell variables plus role overrides; inherited fallback reads use the isolated environment seam in tests. Shell discovery captures both config variables. Probe the exact cwd/key before recovery; missing history follows Runner's existing guarded automatic-resume and explicit fresh-fallback behavior. Windows skips the filesystem probe and lets native `--resume` validate history, because its cwd hash/storage layout is unverified and Rust canonicalization adds a verbatim prefix.

On macOS/Linux, observe the launched process's open database, never the newest global/cwd chat. Cursor closes the old store on `/clear`, `/resume` and `/fork`. Multiple open stores retain the last verified key and reset the empty-scan streak. A unique store resets the streak and captures its key. Clear only after three consecutive successful empty scans, spaced at least 500 ms apart (at least one second from first to third); failed scans retain identity and reset the streak. This lets a transient close before graceful-quit EOF retain the key. After `/clear`, a sustained empty state clears the key so Runner cannot accidentally resume the old context; a replacement appearing within the debounce window rekeys directly. A unique replacement updates it through Runner's existing generation-checked key persistence. macOS reads raw paths through the system file-descriptor API, preserving Unicode and literal escape sequences; Linux uses `/proc/<pid>/fd`. Clearing reports that finish after EOF are ignored. Windows has no process-store observer in this change.

A small shared watcher callback supplies the spawned PID. Key persistence accepts a cleared value while preserving the current-running-incarnation guard. Other runtime adapters retain their existing behavior. Exact Cursor mission resumes retain the app's current Bypass flags. Cursor's plugin hooks are not used for identity because the tested CLI omits plugin-only hooks in several turn/event checks.

## Implementation Phases

1. Runtime registration, dynamic models, executable detection/override and one shared managed skill.
2. Native launch, explicit identity/resume, process-bound rekey and missing-history handling; bounded macOS chat/mission/tool/bus smoke.
3. Native desktop terminal fixtures and Windows smoke remain platform certification work. Semantic status hooks, native fork, MCP management and updater UI remain separate capabilities; status uses the terminal baseline.

## Verification

Focused tests cover launch/model/prompt arguments, permissions, distinct fresh IDs, exact resume, cwd/config storage, rekey/debounced clear/ambiguity, output-forwarder survival after clearing, model cache behavior, executable discovery and skill ownership. See [the validation record](../tests/723-cursor-agent-runtime.md).

| Platform | Evidence | Remaining gaps |
| --- | --- | --- |
| macOS Apple Silicon | CLI `2026.10.01-e373342`; 31 dynamic models; real chat replies and stop/resume; unattended crew tool and bus message | Full native UI/approval/paste/resize/wheel/interrupt fixture suite unverified |
| Linux | Storage/argument unit tests; `/proc` observer implementation | No live Linux CLI smoke |
| Windows | Shared argument and capability expectations | No native CLI/ConPTY smoke or active-store rekey observer |
