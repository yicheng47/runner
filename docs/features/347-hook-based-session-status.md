# 347 — Hook-based agent session status

> Tracking issue: [#347](https://github.com/yicheng47/runner/issues/347)
> Priority: P1. Platforms: macOS and Windows.
> Decision, 2026-09-13: next step after v0.8.9. The release keeps the merged title-spinner and byte-activity heuristics from [#585](https://github.com/yicheng47/runner/pull/585) unchanged.

## Motivation

An agent can be thinking, running a quiet tool, waiting for approval, or ready for another turn while the same foreground process owns its terminal. Process detection cannot distinguish those states. Byte traffic measures output, and a title spinner is presentation that can change with CLI versions or user settings. Neither is a reliable agent lifecycle contract.

Use lifecycle events from Claude Code and Codex to determine agent status. Both currently support command hooks; the local versions checked on 2026-09-13 were Claude Code 2.1.270 and Codex 0.154.0 (`hooks` stable and enabled). Study cmux's hook adapters for the integration shape, then verify each event's semantics and injection mechanism against the supported CLI versions before implementation.

## Scope

- Start with accurate Busy/Idle for Runner-spawned Claude Code and Codex sessions. Keep existing status consumers and mission routing, with one backend owner for normalized activity and its source. A needs-you state can follow once actual human-wait events are verified; it is not required to replace the detector.
- Use per-runtime adapters to turn lifecycle events into session transitions. Prompt submission starts work; confirmed main-agent turn completion ends it. Handle cancellation, failure, exit, and resume explicitly. `SessionStart` alone does not prove either readiness or ongoing work.
- Distinguish main-agent events from subagent events. A subagent finishing cannot make its parent Idle. Bind reports to the owning Runner session and process generation so a delayed event from a replaced process cannot change the new session.
- Account for hook ordering and continuations. A `Stop` hook can be blocked by another hook and continue the turn; it must not unconditionally announce final readiness. A `PermissionRequest` may be automatically approved without showing a human prompt; it must not unconditionally mean waiting for the user.
- Prefer the small cmux-style command-hook bridge into Runner's existing CLI/IPC and session event path. Verify whether that path can carry authenticated, session-scoped reports before introducing another receiver. Bound failures so reporting cannot prevent the agent from progressing.
- Compose hooks additively for each spawn and preserve the user's hooks, configuration, authentication, conversation storage, and resume behavior. Verify Claude's additional settings and Codex's current hook configuration route. Do not assume that `-c` accepts hook definitions or that redirecting `CODEX_HOME` to a mirror preserves everything. No global configuration rewrite as an incidental installation step.
- Support native macOS and Windows, including executable paths with spaces, command quoting, and the actual hook shell used by each CLI. No Unix-shell-only helper dependency.

## Source precedence and migration

The v0.8.9 detector is documented in [architecture §5.10](../arch/arch.md#510-busy--idle-inference) and the [archived #584 spec](./archive/584-title-status-detection.md). This issue replaces that agent detector in a later release; it does not change the 0.8.9 release contents.

Once a supported adapter is active, lifecycle status owns the session. PTY output, title updates, and a silence timeout cannot override it. Long silence during a healthy turn is normal, not evidence that hooks failed or the agent is Idle. Track missing capability or explicit bridge failure separately from silence; never label an unobserved state as confirmed readiness for inbox delivery. Preserve the existing submit/wake behavior only where it agrees with the lifecycle model, with its precedence covered by tests.

Remove title-spinner classification when the hook detector lands. Terminal titles remain display data under [#587](./587-terminal-provided-titles.md). For unsupported runtimes, any retained byte detector is explicitly a heuristic source; it is not a fallback that silently takes ownership from an active hook adapter. Specify the unavailable-adapter and bridge-failure behavior before rollout, including what status consumers and routing do when readiness is unknown.

Shell command status is separate work under [#586](./586-shell-status-detection.md): process detection first, optional semantic shell integration later. Neither shell foreground detection nor OSC 133 reveals the lifecycle inside an interactive agent.

## Implementation phases

1. Verify the supported CLI event contracts and additive configuration routes, then specify state/source precedence and unavailable-adapter behavior.
2. Implement the session-scoped command bridge and Claude Code/Codex adapters through the existing backend status path; remove title-spinner classification as the adapters replace it.
3. Validate lifecycle edge cases and inbox eligibility on macOS and Windows before rollout. Richer needs-you presentation remains a follow-up.

## Verification

- Exercise prompt → quiet tool → completion with continued terminal animation in both CLIs. Status stays Busy through quiet work and becomes Idle on the verified completion boundary, independently of title settings.
- Exercise repeated turns, user cancellation, errors, process exit, resume, and process replacement. Old-generation and subagent completion events cannot make an active main turn Idle.
- Exercise automatic approval, a real human approval prompt, and a stop hook that requests continuation. Do not confuse approval telemetry or an attempted stop with readiness.
- Test duplicate reports, event ordering, absent hooks, and bridge failure. Silence alone never switches an active hook session back to byte inference.
- Verify existing user hooks still execute and configuration/authentication/resume storage remains intact. Test paths with spaces on macOS and native Windows.
- Cover mission inbox eligibility as well as direct-chat status, and run the relevant backend/terminal tests and workspace Clippy. Any later needs-you UI requires a Pencil design first.

## References

- [Codex hooks](https://developers.openai.com/codex/hooks) and [Claude Code hooks](https://code.claude.com/docs/en/hooks) — event and configuration contracts must be verified at implementation time.
- cmux: `CLI/CMUXCLI+AgentHookDefinitions.swift` and `CLI/FeedEventClassifier.swift` in the local `~/repos/gui/cmux` clone; the Codex permission classifier explicitly accounts for automatic approval.
- [Original hook proposal](./archive/52-hook-based-session-status.md) — historical design, superseded by this spec; its Tauri receiver, assumed event mapping, and silence-based freshness decay are not implementation requirements.
