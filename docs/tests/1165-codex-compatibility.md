# #1165 — Codex minimum-version evidence

Date: 2026-11-11. Platform: macOS arm64. This is CLI compatibility evidence; it does not validate Runner's native UI or claim Windows/Linux coverage.

The catalog minimum is **0.149.0**. The first executable hook release works for Runner's session-scoped, enabled `runner_hooks` MCP server. The later dormant-server fix is not required for that launch pattern.

## Portable execution probes

Downloaded official `codex-aarch64-apple-darwin.tar.gz` assets from the `rust-v0.147.0`, `rust-v0.148.0`, `rust-v0.149.0`, `rust-v0.163.0` and `rust-v0.164.0` releases into `/tmp/runner-1165-compat/<version>/`. Each executable ran with a separate `CODEX_HOME` and working directory in that scratch tree. Only the existing authentication file was copied into the scratch homes; no installed executable or global agent configuration was changed. Authentication contents are excluded from evidence.

Each authenticated `codex exec --json --skip-git-repo-check` requested one tiny response on `gpt-5.6-sol` with low reasoning effort, using Runner's `--enable hooks`, `--dangerously-bypass-hook-trust`, update-notice suppression and terminal-title settings. The hook flags configured every event Runner supplies: MCP handlers for SessionStart, UserPromptSubmit, PreToolUse, PostToolUse, PreCompact, PostCompact, Stop and Interrupt, and a command handler for SessionEnd. The MCP `report` tool was a local stdio recorder in place of Runner's daemon endpoint. Its configuration matched Runner's enabled, non-required, per-session server. Hook inputs included Runner's event/session/transcript fields, source or turn ID, and compaction trigger where applicable. The SessionEnd command was a harmless `true`; the probe targeted MCP parsing and execution.

| Version | Configuration | Authenticated turn | MCP report calls |
| --- | --- | --- | --- |
| 0.147.0 | Refused `mcp_tool` with the field error | Never started | None |
| 0.148.0 | Accepted; warned that MCP invocation was not available yet | Returned `OK`, exit 0 | None |
| 0.149.0 | Accepted | Returned `OK`, exit 0 | SessionStart, UserPromptSubmit, Stop |
| 0.163.0 | Accepted | Returned `OK`, exit 0 | SessionStart, UserPromptSubmit, Stop |
| 0.164.0 | Accepted | Returned `OK`, exit 0 | SessionStart, UserPromptSubmit, Stop |

The release archives contain the Codex executable alone. These isolated runs also emitted a missing `codex-code-mode-host` warning; no Code Mode tool was requested, and the turn and recorded hook calls completed. Tool-use, compaction and interrupt hooks were configured and parsed but deliberately not triggered by the tiny prompt. The intermediate 0.150–0.162 releases were not individually executed; 0.149 and 0.163 bracket the relevant range, supported by the upstream implementation and the scope of the subsequent fix.

Scratch evidence: `exact-output.log` and `exact-hooks.jsonl` under each version directory, with the driver and stdio recorder at `/tmp/runner-1165-compat/probe.py` and `mcp.py`. The paths are local run artifacts, not durable repository evidence; the observed results are recorded above.

## Upstream explanation

[Recognize MCP tool hook configurations, #37363](https://github.com/openai/codex/pull/37363) adds parsing and explicitly skips execution. [Enable MCP tool hooks in Codex sessions, #39296](https://github.com/openai/codex/pull/39296) adds execution through the shared MCP runtime for connected, cataloged, allowed tools. The observed 0.147/0.148/0.149 behavior matches that boundary.

[Start dormant MCP servers for hook calls, #55263](https://github.com/openai/codex/pull/55263) concerns cached, dormant servers, including model-hidden tools in subagents, and startup within the hook timeout. Runner explicitly enables its server in the top-level session; the probes delivered lifecycle reports before that change. This is evidence for Runner's current launch pattern, not a claim that every cached-subagent hook scenario works on 0.149.
