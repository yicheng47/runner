# 347 — Hook-based agent session status

> Tracking issue: [#347](https://github.com/yicheng47/runner/issues/347)
> Priority: P1. Platforms: macOS and Windows.
> Shipped on macOS for Claude Code, Codex and GitHub Copilot CLI (PRs #588, #589, #609); #347 closed 2026-09-16. Windows hook status and the remaining details continue in [610](../610-windows-hook-status.md) ([#610](https://github.com/yicheng47/runner/issues/610)).
> Decision, 2026-09-13: next step after v0.8.9. The release keeps the merged title-spinner and byte-activity heuristics from [#585](https://github.com/yicheng47/runner/pull/585) unchanged.
> Design direction, 2026-09-14: define the richer status model and its UI before implementation. Working, Needs you, and Idle are the everyday states. This supersedes the earlier decision to defer needs-you presentation. Proposal for review; runtime capability verification is still required.
> Mechanism decision, 2026-09-14: the hook bridge is a port of cmux's working implementation, not a new design — per-invocation CLI injection, fire-and-forget hook scripts, and a Runner-owned script directory. cmux is GPL-3.0-or-later and Runner is GPL-3.0, so deriving from it is clean with attribution. What is ours is the product design: the status model and the surfaces on the canvas. See [Mechanism: port, do not invent](#mechanism-port-do-not-invent).
> Scope decision, 2026-09-14: ship the minimum each runtime can actually prove, and grow the vocabulary as capabilities are verified — see [Minimum viable coverage](#minimum-viable-coverage). Runner never gains an approval or answer control of its own, in this feature or later: showing that a session is waiting on you, and focusing that pane when you click it, is the whole of Runner's job. The CLI keeps its prompt.

> Review correction, 2026-09-14: Claude Code 2.1.270 emits `idle_prompt` after a user-tunable idle delay, not immediately at turn completion. Jason changed slice 1 to use `Stop` and `StopFailure` for Idle and retain a real `Notification(idle_prompt)` as secondary confirmation. Continuation can briefly overturn the dot; Busy/Idle gates no behavior. The [slice 1 brief](../impls/archive/gpui-rewrite/briefs/347-slice-1-claude-hook-bridge.md) records the revised contract. TRAE's documented immediate idle notification was not verified; its adapter is now outside the plan.

> Runtime support decision, 2026-09-14: TRAE CLI hook integration is unsupported in Runner and is no longer a planned adapter. Jason no longer has an enterprise account to validate it. Keep TRAE on estimated terminal activity/title status; the retained TRAE manual audit below is historical upstream evidence, not implemented Runner support. Windows status hooks remain a separate agreed follow-up.

## Motivation

An agent can be thinking, running a quiet tool, waiting for approval, or ready for another turn while the same foreground process owns its terminal. Process detection cannot distinguish those states. Byte traffic measures output, and a title spinner is presentation that can change with CLI versions or user settings. Neither is a reliable agent lifecycle contract.

Use lifecycle events from Claude Code and Codex to determine agent status. Both currently support command hooks; the local versions checked on 2026-09-13 were Claude Code 2.1.270 and Codex 0.154.0 (`hooks` stable and enabled). Study cmux's hook adapters for the integration shape, then verify each event's semantics and injection mechanism against the supported CLI versions before implementation.

## Scope

- Design Working / Needs you / Idle for supported Runner-spawned Claude Code and Codex sessions, alongside startup, stopped, error, and unavailable-status presentation. Implement the subset each runtime can prove first; an unprovable detail is left out, never guessed. Keep one backend owner for normalized activity and its source; expose only the details each adapter can prove. Update existing status consumers and mission routing together.
- Use per-runtime adapters to turn lifecycle events into session transitions. Prompt submission starts work; confirmed main-agent turn completion ends it. Handle cancellation, failure, exit, and resume explicitly. `SessionStart` alone does not prove either readiness or ongoing work.
- Distinguish main-agent events from subagent events. A subagent finishing cannot make its parent Idle. Bind reports to the owning Runner session and process generation so a delayed event from a replaced process cannot change the new session.
- Account for hook ordering and continuations. A `Stop` hook can be blocked by another hook and continue the turn; it must not unconditionally announce final readiness. A `PermissionRequest` may be automatically approved without showing a human prompt; it must not unconditionally mean waiting for the user.
- Port cmux's command-hook bridge into Runner's existing CLI/IPC and session event path rather than designing a transport. Verify whether that path can carry authenticated, session-scoped reports before introducing another receiver. Bounding failures is a property of the ported mechanism, not something to re-derive.
- Compose hooks additively for each spawn and preserve the user's hooks, configuration, authentication, conversation storage, and resume behavior. Verify Claude's additional settings and Codex's current hook configuration route. Do not assume that `-c` accepts hook definitions or that redirecting `CODEX_HOME` to a mirror preserves everything. No global configuration rewrite as an incidental installation step.
- Support native macOS and Windows, including executable paths with spaces, command quoting, and the actual hook shell used by each CLI. No Unix-shell-only helper dependency.

## Status model

Jason merged the visible Ready and Idle labels into **Idle** on 2026-09-14. The backend keeps `Activity::Ready` separate from baseline `Activity::Idle`; the presentation maps both to one label and identifies estimated observations only in the tooltip.

The primary question is whether the agent is working, needs a decision, or can accept another turn. Keep three everyday states; explain tool activity and the reason for a wait with secondary text. Do not create a separate colored status for each tool or infer thinking, reading, writing, or testing from terminal prose.

| State | Meaning | Visible label and detail | Treatment |
| --- | --- | --- | --- |
| Working | A main-agent turn or explicitly observed operation is active. | `Working`; optional `Using tools` or `Compacting context` when verified. | Muted spinner. Reserve amber for human attention. |
| Needs you | An unresolved interaction requires the user, with evidence that the interaction actually reached the user. | `Approval needed`, `Answer needed`, or `Needs you` when the reason is unavailable. Plan approval uses `Approval needed`. | Amber hand for approval; amber message bubble for an answer; amber triangle when the reason is unavailable, because the circle-exclamation belongs to Error. |
| Idle | The agent’s turn has ended, or terminal activity is estimated to be idle. | `Idle`; baseline observations add `estimated from terminal activity` in the tooltip. A last-turn outcome can later explain completion or interruption. | Small muted hollow circle. New unread output retains the separate accent dot. |

Lifecycle and observability supply the remaining presentation cases; these are not more kinds of agent work.

| Case | Meaning and presentation |
| --- | --- |
| Starting / Resuming | Runner is launching or reattaching the process. Muted spinner with the corresponding label. `SessionStart` alone does not establish Idle. Once attachment finishes, absent activity evidence resolves to Status unavailable rather than an indefinite Starting animation. |
| Stopped | The process ended normally or Runner stopped it. Muted square and `Stopped`; retain the existing resume/restart controls. |
| Error | A confirmed terminal turn failure needs recovery, or the process crashed. Red circle-exclamation and `Error`; detail distinguishes `Response failed` from `Process exited · code N`. A failed tool that the agent can recover from remains Working. |
| Status unavailable | The process is live but activity cannot be established, or an explicit bridge failure invalidates the last observation. Muted circle-question and `Status unavailable`; terminal use remains available. This never means Idle or a failed agent. |

Track process lifecycle, agent activity, last-turn outcome, attention, and observation source separately rather than adding every combination to one status enum. Idle describes inactivity, not successful completion of the user's task. Completed, interrupted, and failed are outcomes; unread is attention. A live session can fail a response without crashing, and an interrupted session can become Idle without a successful-result notification.

For a blocking interaction, Needs you takes precedence over Working even if another tool is still running. For an explicitly nonblocking question, preserve Working and add needs-you attention; display `Answer needed` with `Still working` in its tooltip. Keep unresolved interaction identities until each is answered, dismissed, cancelled, or invalidated by the owning turn/process. Merely focusing the pane does not clear them. Ordinary prose ending in a question is not a structured human-wait signal.

### Transitions and routing

- An accepted prompt or verified work continuation enters Working and clears the previous outcome. A keypress or attempted submit alone does not prove that the agent accepted input.
- A verified human interaction adds needs-you attention and, when blocking, enters Needs you. A correlated answer/approval/dismissal clears that interaction. Return to Working only when continuation is established; if another interaction remains, retain its attention.
- Main-agent turn completion enters Idle and records the outcome. No runtime publishes a boundary that a continuation hook cannot overturn, so Idle is entered on the turn-end event and corrected by the next work event rather than waiting for a confirmation that does not exist; this is safe precisely because Idle gates nothing. Completion while its pane is not being viewed creates unread attention. A subagent stop or a single completed tool cannot end the parent turn.
- Interruption ends the active attempt, but return to Idle only when prompt availability is established. No new completion dot or error alert for a user-requested interruption. If readiness cannot be established, show Status unavailable with the interruption in the tooltip.
- Confirmed unrecovered turn failure shows Error until new accepted work, explicit recovery, or session end. An observed automatic retry remains Working. Process exit always overrides live activity and clears obsolete waits.
- Working does not block automatic inbox delivery, and Idle is not a precondition for it. Delivery injects a nudge — `[inbox] unread messages — run \`runner msg read\` to view.` — not the message body, which lives in the inbox projection. A nudge typed into a working agent is queued by its TUI and read on the next turn, and a lost one is recovered by the existing reconciliation nudge. Gating the doorbell on a confirmed Idle would add a stall for no benefit and would silently strand any runtime whose readiness cannot be confirmed, which is the failure [#359](https://github.com/yicheng47/runner/issues/359) removed.
- Needs you is the one status that blocks delivery, because an open approval or question dialog consumes keystrokes. A nudge injected at `Do you want to proceed?` is not queued text; it is an answer to that prompt. Block while an interaction is unresolved, queue the delivery, and release it when the interaction clears — the existing outbox and retry path already do exactly this for a human draft.
- Every other gate stays as it is: the draft check, the recent-typing window, in-flight, and the unavailable-session check are unchanged, and they remain the only conditions that matter for the rest of the model. Manual terminal interaction remains available throughout.
- Last-turn completion and idle readiness do not complete a mission. Mission outcomes continue to use mission events. Queued inbox messages are distinct from an agent asking the user for approval or an answer.

### Capability evidence and open adapter questions

All three agent runtimes were checked against the binaries installed on 2026-09-14 — Claude Code 2.1.270, Codex 0.154.0, and TRAE CLI 0.120.52 — using each one's own hook reference. This replaces the earlier documentation-only audit, which was wrong in both directions: it treated TRAE as a Codex derivative with no hook story, and its Codex question audit was subsequently corrected by the slice-3 native TUI probes: question tool hooks exist, but do not prove a surfaced prompt.

| Capability | Claude Code | Codex | TRAE CLI |
| --- | --- | --- | --- |
| Runner hook integration | Supported on macOS; Windows follow-up | Lifecycle/interruption on macOS; Windows follow-up | Unsupported; estimated status only, no enterprise test account |
| Events published | 32 | 12 | 13 |
| Work starts, tools, compaction | `UserPromptSubmit`, `PreToolUse` / `PostToolUse`, `PreCompact` / `PostCompact` | same four, same names | same, plus `post_tool_use_failure` |
| Surfaced permission wait, distinct from a raw request | `Notification(permission_prompt)`, with `PermissionRequest` and `PermissionDenied` alongside | **no** — `PermissionRequest` precedes other hook decisions and automatic review; elapsed time cannot prove a surfaced dialog | `notification(permission_prompt)` before the dialog, plus `permission_request` when it is actually shown |
| Agent asking a question | `Notification(elicitation_dialog)`, plus `Elicitation` / `ElicitationResult` and an `agent_needs_input` subtype | `PreToolUse(request_user_input)` fires even for rejected calls. Plan-mode rollout calls also precede hook decisions; neither proves a shown question | `notification(elicitation_dialog)`, documented as firing only for `AskUserQuestion` |
| Turn-end signals | Main-agent `Stop`, or `StopFailure` on an API error. `Notification(idle_prompt)` is delayed and user-disableable; it is secondary confirmation only. | `Stop`, which another hook can continue | `notification(idle_prompt)` is documented as immediate, after continuation hooks, and main-agent only; unverified historical manual claim; no Runner adapter planned |
| Fatal turn failure | `StopFailure`, "the turn ends due to an API error" | no | no |
| User interruption | not published as its own event | `Interrupt` | not in the 13 |
| Main versus subagent | `SubagentStart` / `SubagentStop` | `SubagentStart` / `SubagentStop` | `subagent_start` / `subagent_stop`, plus `agent_id` on every payload |
| Installing a hook | settings-file hooks need no trust step; workspace trust applies to project skill and subagent frontmatter hooks | **explicit trust required** — "before a non-managed hook can run, Codex requires you to review and trust the exact hook definition", managed via `/hooks` and hashed per definition in `~/.codex/config.toml` | merges by execution identity across user and project config, no trust gate |
| Hook config | `hooks` in `settings.json`, PascalCase event keys | `~/.codex/hooks.json`, PascalCase keys, normalised to snake_case internally | `hooks` list in `~/.trae/traecli.yaml` or `.trae/traecli.yaml`, or `hooks.json`; event names accept snake_case, camelCase or PascalCase |

Claude Code supplies work, surfaced-interaction, question, and fatal-failure events, but its idle notification does not supply an immediate final boundary after continuation hooks. TRAE documents such a boundary, but that manual claim remains unverified and no Runner adapter is planned. Codex supplies work, tool, completion and native interruption hooks. It supplies question *attempts*, but no hook confirming that a human prompt opened. Stop may be continued by another hook; following work corrects the display and Idle gates no delivery.

The slice-3 audit used both requested implementations: cmux at `015991fbd90cef2ce16b46a58d6f1a265ef81998` and Orca at `c1e15c40`. cmux injects invocation-layer hooks additively, classifies raw Codex permission requests as native-prompt attention rather than its own approval UI, and searches structured rollout calls for questions. Orca installs/trusts managed hooks, maps raw PermissionRequest and PreToolUse(request_user_input) directly to waiting, rolls child status up to the parent, and can infer resolution from keystrokes. Runner adopts the additive transport shape, not those waiting/ownership/inference policies.

Native Codex 0.154.0 localhost fixtures disprove the proposed approval settle interval: both a shown command approval and an automatically allowed request emit the same PermissionRequest shape (without a tool-use ID). A user PermissionRequest hook can take time and then allow the command without a dialog. No arbitrary quiet interval establishes that a human is deciding.

The inherited Plan-mode question correlation was also disproved. A shown question and a valid question denied by a user PreToolUse hook both emit PreToolUse(request_user_input), a Plan turn_context, and a correlated response_item/function_call before resolution. The denied call never opens a question. Answering the real question emits PostToolUse and a correlated function_call_output; Escape emits Interrupt and a turn_aborted record instead. Default mode rejects the tool by default; enabling default_mode_request_user_input allows a question, with isBlocking=false visible over app-server but absent from hooks/rollout calls. App-server is research evidence only; normal Runner sessions remain native PTYs.

**Slice 3 therefore implements no Codex Approval needed or Answer needed, and adds no Codex human-wait delivery hold.** This is a known protection gap for real Codex dialogs, not a claim that the dialogs do not exist. The independent typed-draft and recent-typing gates remain. A future adapter needs a surfaced-interaction signal plus correlated resolution; merely adding a delay or parsing a question call is insufficient. See the [slice-3 smoke checklist and evidence](../tests/347-codex-hooks-smoke.md).

Codex injection uses `--enable hooks --dangerously-bypass-hook-trust -c hooks.<Event>=...`. Local fresh/resume fixtures verified that user hooks.json, user config.toml hooks, and trusted project hooks each run once alongside the invocation layer, with their bytes unchanged. Runner does not copy lower-layer definitions, persist hooks/trust entries, redirect CODEX_HOME, or touch authentication/conversations. The bypass flag applies to enabled hooks for the whole invocation, including hooks supplied by trusted project/repository configuration. New or changed definitions from those layers run without the normal per-hook review; project trust still controls which project layers load. Codex displays its native warning. `--enable hooks` also overrides a stored `features.hooks=false` for this invocation without changing the file. An explicit invocation opt-out (`--disable hooks` or `-c features.hooks=false`) or hook override disables Runner injection and its watcher. The verified CLI is 0.154.0; a future CLI removing these launch flags would require an adapter update.

Hook timeout is in **seconds**. Runner uses a two-second bound and a short stdin-to-side-file reporter that acknowledges with an empty JSON object and exits zero. It never emits an approval decision, denies a tool, rewrites input, or adds turn context. TOML serialization and shell quoting preserve paths with spaces and quotes. The shared watcher follows Claude's existing local feed and runtime output path; it owns cleanup and falls back on explicit bridge loss. A new root SessionStart owns in-process navigation regardless of source, including `/new` (which 0.154.0 reports as startup) and returning to a previously visited conversation; other-session work/end events remain ignored. A native Interrupt may follow Stop while another Stop hook is still pending, and must supersede that provisional completion before the matching abort restores Idle. Silent hooks do not expire an observation. Native Windows remains baseline-only; no POSIX helper is injected there.

### TRAE CLI — historical manual audit; Runner hooks unsupported

The TRAE-specific evidence here remains its installed manual audit: `trae-cli 0.120.52`, built 2026-08-12, installed 2026-09-14. Its own manual (`traecli doc hooks`) documents 13 lifecycle events, and the shape is Claude Code's, not Codex's — `hook_event_name` in PascalCase, `hookSpecificOutput`, `agent_id`, `transcript_path`, and `--permission-mode default|plan|bypass_permissions`. The earlier assumption that TRAE is Codex-shaped came from Runner's MCP writer and was wrong about everything except that one file.

It answers four of the five gaps outright, for this runtime — everything except fatal failure versus retry, which has no event:

- **Surfaced permission wait.** `notification` carries `notification_type: permission_prompt`, documented as firing *before* the approval dialog appears, and a separate `permission_request` event fires when the dialog is actually up. A raw request and a shown prompt are therefore distinguishable without a timing heuristic.
- **Agent asking a question.** `notification` carries `notification_type: elicitation_dialog`, documented as firing only when the agent calls `AskUserQuestion`. `Answer needed` needs no inference here.
- **Documented final turn boundary.** TRAE's `notification` with `notification_type: idle_prompt` is documented as immediate when the main agent finishes and no stop hook continues, and never for a subagent. That behavior was not verified before the adapter was removed from the plan; the similar Claude vocabulary proved insufficient evidence. A `stop` hook may continue the turn, so it can produce a temporary display error. Idle is not a prerequisite for inbox delivery.

Also available and directly useful: `pre_compact` / `post_compact` for `Working · Compacting context`; `post_tool_use_failure` for the recoverable-tool-failure case that must stay Working; `subagent_start` / `subagent_stop` plus `agent_id` for the main-versus-subagent rule; and `session_start` with `source` (`startup` / `resume` / `clear`) and `session_end` with `reason` for the generation binding. Not available: any equivalent of `StopFailure`, so `Error · Response failed` stays unreachable on TRAE as well.

The hook contract itself is settled, and it is friendly to a reporter that only observes. A hook that needs no feedback exits 0 and writes nothing; stdout is only interpreted when it is valid JSON, and non-JSON output is treated as empty. Exit code 2 blocks the operation, and any other non-zero code is a non-blocking error that is logged while the agent continues. So a reporting hook that never exits 2 and never prints JSON cannot alter a turn, which is the bounded-failure property the scope section asks for. Runner must also never emit `continue: false`, `decision: "block"`, or a `permissionDecision` — those are the intervention channels, and this feature only watches.

One hard requirement comes with that: **hooks run synchronously and have no default timeout**, so an unbounded reporter blocks the agent. Every hook Runner installs must carry an explicit short `timeout`. An `http` hook defaults to 30s, which is far too long to sit in front of a turn.

Additive configuration, gap one, is answered here too. Hooks merge across user and project levels, deduplicated by execution identity — `type` + `command` for command hooks, `type` + `url` + `headers` + `allowed_env_vars` for http hooks — with the higher-priority level winning for a matching identity. A Runner hook whose command string is its own therefore composes beside the user's hooks instead of replacing them, and re-registering is idempotent.

Two more things worth carrying into the bridge design. Hooks are configured as a `hooks` array in `~/.trae/traecli.yaml` or `.trae/traecli.yaml` (or a `hooks.json`), with `matchers` accepting snake_case, camelCase or PascalCase event names interchangeably. And a hook may be `type: http`, which POSTs the identical JSON payload to a URL with configurable headers — a local receiver sidesteps the command-quoting and path-with-spaces problem that the scope section raises for native Windows, and it fails non-blockingly by design. Confirm whether Claude Code and Codex offer the same before assuming one transport for all three.

The Claude Code 2.1.270 review resolved the earlier timing assumption: `Stop` is main-agent only and can fire twice (`turn_end` and `turn_end_reactions`), while `StopFailure` fires instead of `Stop` when an API error ends the turn. `Notification(idle_prompt)` waits for `messageIdleNotifThresholdMs` without interaction (60,000 ms by default), is suppressed during a dialog or loading, and is disabled entirely at zero. These signals are not interchangeable. Slice 1 maps both turn-end events to Idle and allows the next work event to correct a continuation; repeated Idle reports are idempotent. TRAE's claimed immediate notification remains unverified; its adapter is no longer planned.

### Mechanism: port, do not invent

cmux provides the transport reference; each adapter still verifies event semantics and composition against the installed CLI. Take from cmux:

- **Per-invocation CLI injection.** Pass hooks as `-c hooks.<Event>=[…]` on the command line of the process Runner spawns, with `--enable hooks --dangerously-bypass-hook-trust` on Codex. No hook definitions or trust entries are persisted; the bypass applies to enabled hooks for this invocation and the user’s configuration files remain untouched. Claude Code uses its own additional-settings surface. TRAE receives no Runner status hooks.
- **Bounded observer hook bodies.** cmux backgrounds its larger CLI call with a watchdog. Runner only spools stdin into its existing local feed, acknowledges Codex with `{}`, and exits zero under a two-second CLI timeout; no detached worker or human decision belongs in the observer.
- **A script file in a Runner-owned directory**, outside user agent configuration. Runner invokes it through the verified POSIX hook shell and drains stdin if it is missing. Native Windows transport remains separate work.
- **Serialized TOML values** for injected commands. The serializer selects quoting that round-trips even triple quotes; shell quoting is a separate layer.

Do not port cmux's product decisions. Its feed, its approval reviewer, and its classifier answer a different question than Working / Needs you / Idle, and its Codex permission classifier raises native prompt attention without owning the approval flow. The status model and every surface in [UI design](#ui-design) are Runner's.

Explicitly not ported: writing `trusted_hash` entries into `~/.codex/config.toml` or installing into `~/.codex/hooks.json`. cmux built that and left it unshipped on purpose, and the same reasoning holds here.

### Minimum viable coverage

Ship the states that fall out of the events both CLIs already document, and leave every refinement to a later phase. A state that cannot be proven is not shown, and it is never approximated.

Phase 1 vocabulary, the everyday three plus what Runner already owns:

| State | Evidence | Notes |
| --- | --- | --- |
| Working | Prompt submission and tool events. | The plain label only. No `Using tools`, no `Compacting context` yet. |
| Needs you — `Approval needed` | Claude `PermissionRequest` matched to a pending tool, `PreToolUse(ExitPlanMode)` with a tool-use ID, or fallback `Notification(permission_prompt)`. | Command and plan approvals raise attention immediately. IDless PermissionRequest is correlated by tool name and structured input; owning hooks or transcript results clear the wait. Automatically handled permission requests can briefly raise attention, while ordinary tools allowed without PermissionRequest do not. Sandbox network approvals retain the delayed notification fallback. This is attention for the CLI prompt; Runner has no Plan mode control or planning state. |
| Needs you — `Answer needed` | Claude `PreToolUse(AskUserQuestion)` with a tool-use ID, or a surfaced MCP elicitation notification. | The named question tool raises attention immediately, following cmux, without waiting for Claude’s six-second permission notification. Hook results or correlated transcript tool results clear it, including automatic answers. Plan approval uses the same early detection with Approval needed. Ordinary MCP forms retain the delayed surfaced notification and resolve through ElicitationResult; a notification with no pending form is ignored. |
| Idle | Main-agent `Stop`, corrected by the next work event if a hook continued the turn, or estimated inactivity from the baseline detector. | Display only — it gates nothing, so an Idle that a continuation overturns is a flicker rather than a delivery bug. No runtime offers a continuation-proof boundary; this is why that is acceptable rather than a compromise. No new outcome text in the tooltip yet. |
| Response failed | Claude `StopFailure` records a Failed turn outcome. | Red circle-exclamation and `Response failed`, matching Pencil node `f9fpUs`. Shared pane/tab and mission status presentations retain it until new work or a lifecycle transition. The tooltip says `Response failed · Agent is still connected`; no completion dot is created. Sidebar failure attention remains deferred. |
| Starting, Stopped, Error `Process exited · code N` | Runner's own `SessionStatus`. | Already known without any hook. Free. |
| Status unavailable | A live process with no observation at all. | Rare, because the baseline detector is always running underneath. Not the state for a runtime that simply has no adapter — that is `Working · estimated`. |
| Unread response | Existing accent dot. | Unchanged. |

Deferred until the spike or a later runtime version proves them: `Working · Compacting context` and `Using tools`; sidebar attention for response failures; the interruption outcome in the Idle tooltip; elapsed time on a wait; and the nonblocking-ask `Still working` case, which is the only part of the answer story that waits — every ask phase 1 can see blocks the turn. Each is designed on the canvas and each is additive — none of them changes the phase-1 shape of a header, a row, or a card.

Per-runtime expectation going in:

| Runtime | Phase 1 target |
| --- | --- |
| Claude Code | The full phase-1 vocabulary and then some — it is the only runtime that can also reach `Error · Response failed`, via `StopFailure`. Surfaced prompts and questions have `Notification` subtypes. Turn-end display uses `Stop` with continuation risk; slice 1 also maps `StopFailure` to Idle because that slice adds no error state. `idle_prompt` is secondary confirmation, not an immediate boundary. This is the reference adapter; TRAE hook integration is unsupported. It remains the runtime most users will see first. |
| Codex | Slice 3 on macOS: Working from accepted prompt/tool/compaction hooks, Idle from Stop (corrected by later work), native Interrupt with Ready recovered from the matching turn_aborted record, and explicit bridge-loss fallback. SessionStart alone does not prove readiness. No Approval needed or Answer needed: raw requests and question attempts cannot prove a shown human prompt, even with Plan-mode rollout correlation. Windows and explicit invocation hook overrides/opt-out remain baseline-only. |
| TRAE CLI | **Hook integration unsupported in Runner.** Estimated terminal activity/title status remains available. Jason no longer has an enterprise account for validation, so the adapter is removed from the plan. The historical manual audit above is not a support claim. |
| Shell | No agent status in either the pane header or the tab bar. Process detection is [#586](./586-shell-status-detection.md). |

## UI design

The canvas is `design/runner.pen`, the active product canvas, following the same `Spec — … (issue) · v1` convention that #567, #570 and #574 used. Frames, 2026-09-14:

| Frame | Node | What it settles |
| --- | --- | --- |
| Spec — Session status (347) · v1 · vocabulary | `esVJZ` | Every state as glyph, label, secondary text and treatment, in three bands: everyday, lifecycle, observability. |
| Spec — Session status (347) · v1 · pane header | `Z9PuC` | Today's 5px dot against the glyph-and-label header, narrow-width degradation at 480/360/296px, and the tooltip set. |
| Spec — Session status (347) · v1 · single-pane tab | `TCevM` | Where status lives when a tab holds one pane and there is no pane header. |
| Spec — Session status (347) · v1 · sidebar rollups | `FTDZ6` | The trailing attention slot for single-pane tabs, multi-pane rollups, and collapsed projects and sections. |
| Spec — Session status (347) · v1 · mission workspace | `CC4wy` | Runner cards, the mission tab strip, and the mission row rollup. |
| Runner chat — approval wait · 2-pane (347) | `nygy1` | Full screen: one pane holding a permission prompt, one working, with the sidebar rollup. |
| Runner chat — approval wait · single pane (347) | `MdmRR` | Full screen: the same wait on a one-pane tab, carried by the tab bar. |
| Light — Session status vocabulary (347) | `wLIlB` | The vocabulary in the light theme; amber, red and accent all shift with the theme tokens. |
| Light — Session status pane header (347) | `mFFha` | The pane header and its narrow widths in the light theme. |
| Light — Runner chat approval wait · single pane (347) | `E4lcBK` | The single-pane screen in the light theme. |

The frames draw the full target vocabulary; [Minimum viable coverage](#minimum-viable-coverage) says which states phase 1 actually lights up. The glyphs reuse the existing lucide vocabulary, and four of them are not in `crates/runner-app/src/assets.rs` yet: `hand`, `message-circle`, `circle-alert` and `circle-question-mark`. `square.svg` and `triangle-alert.svg` are already there, and the spinner and the hollow Idle circle are drawn, not iconography.

### Pane header

Jason selected option C on 2026-09-14: `347 · Header options · C · Selected — after buttons + divider`, node `iP5dP` in `design/runner.pen`. This revises the placement and typography in the original v1 header frames. In a single-pane tab, put status after the inline header controls, separated by a 1 × 14 logical-pixel divider using the strong border token and a 10-pixel gap before the status. Keep the existing header height. Normal status text is 10 pixels; attention and error labels remain 11 pixels. Stopped and Idle use text alone when the label fits; working and attention retain their glyphs. Single-pane tab headers reserve at least 108 pixels for a labeled status, growing for longer text, and 16 pixels for icon-only status. Split-pane status uses its content width. Work details remain in the tooltip. During a single-pane agent resume, the Resume button stays visible and disabled while the status owns the only spinner, including at icon-only widths.

A pane header only exists from two panes up — `pane_identity_visible` is false below that — so a one-pane tab shows no status anywhere on the chat surface today, and its sidebar row is the only signal a user has. Give the tab bar the status while the tab holds one pane: the same compact treatment and tooltip, after the kebab, Resume/Stop, and Fork buttons. In split-pane headers it follows the title’s inline menu without a divider. The full-layout reference is Pencil frame `X7FJf`, with Claude, Codex, and an empty pane; the compact preview is `ZWAnA`. Equal-width left identity/status and right action areas keep the 20-pixel drag handle centered in the pane; the split and close-pane buttons stay at the trailing edge. Splitting hands status down to the pane headers and the tab bar drops it, because every pane's own header is then on screen; collapsing back to one pane returns it. The state itself never changes on a split, only where it is drawn. A shell tab shows no agent status in either place, matching `pane_identity_shows_status` today; process detection for shells is [#586](./586-shell-status-detection.md).

At narrow widths, remove normal-state text before truncating the title further; keep the status icon and tooltip. Reserve room for the attention label when possible; below 320 logical pixels, icon-only is allowed for every state. Preserve the full accessible label. Allow status labels to grow for translation rather than assuming a fixed English width.

Tooltips explain the state in plain language: `Waiting for you to approve a command`, `Waiting for your answer`, `Working · Compacting context`, `Idle · Last response interrupted`, or `Status unavailable · Agent is still connected`. Show one short reason when available, not raw hook names or full command payloads. Elapsed time is useful only for a wait, for example `Waiting for 2m`; silence never changes the state.

Clicking a needs-you indicator focuses the corresponding terminal pane. The agent CLI continues to own approval and answer controls. Runner does not duplicate the question, auto-answer it, or synthesize an approval keypress. Avoid a new banner consuming terminal rows for information already visible in the header.

### Sidebar tabs and project rollups

Keep the current compact row layout and runtime/layout icon. The trailing attention slot shows, in priority order: an unresolved error, needs-you, working, unread completion, status unavailable, or nothing. A ready viewed tab stays quiet. Status unavailable ranks last because it is missing knowledge rather than a call for attention, but it still has to be drawn — an empty slot reads as quiet and ready, which is the one thing an unobserved session is not. An estimated working session rolls up as working and identifies the estimate in its tooltip; estimated idle contributes nothing and never proves readiness. A tooltip lists concurrent conditions, such as `1 approval needed · 1 working · 1 unread response`, so the dominant icon does not erase the other states. Project and section rollups use the same ordering. Error attention is acknowledged by viewing the affected pane; the pane's Error status remains until recovery. Needs-you attention persists until resolution.

Clicking the attention indicator opens the affected pane, preserving the existing row-click tab behavior. If multiple panes need attention, choose the oldest unresolved error first, then the oldest unresolved request; use pane order as a stable tie-breaker. Do not rotate the target simply because it was clicked. A collapsed project indicator expands the project and reveals the affected tab. Keep attention visible while row actions or shortcut pills appear; reserve separate space instead of replacing the warning with an ellipsis or shortcut.

A row is too narrow for a label, so the glyph carries the state and the tooltip names it. The label appears only where there is room for it: pane headers, the single-pane tab bar, and runner cards.

Unread completion remains the existing accent dot; clear it when that response's pane is actually viewed, not merely because another pane in the tab is active. A new turn may hide the dot behind a working indicator without deleting its unread record. Restoring normal status must not recreate a previously acknowledged completion.

### Mission workspace

Use the same icon, label, and meaning in runner cards and pane headers. The card header's trailing corner is already spoken for — #542 gave it Stop and Restart — so status lives on the subtitle line and never competes for that cluster. The subtitle can carry a detail because it has the room: at the real rail width the card is 240 pixels, leaving 216 for the line, which fits `Working · Compacting context` with margin; anything longer truncates there rather than growing the card. The avatar's presence dot goes back to meaning only what it can prove — the process is live or it is not — and stops carrying agent activity, which is what makes the amber-versus-accent split disappear. Its alpha carries activity today, a dimmed `#00FF9C66` when idle against a solid `#00FF9C` when live, and that is the other half of the same inconsistency. Each runner tab in the mission tab strip gets the same glyph on the same rules as a sidebar row: glyph only, label in the tooltip. The mission sidebar row aggregates runner attention; it must not describe an entire mission as Idle when only one runner is ready. Its tooltip reports counts. Clicking its attention indicator opens the affected runner. Preserve the existing pending-asks and inbox surfaces and avoid double-counting the same Runner ask if it already supplies an interaction identity.

### Visual rules

- Use existing theme tokens: muted text for normal activity/readiness/unavailability, warning for human attention, danger for error, accent for unread completion. This intentionally removes the current inconsistency where Busy is amber in chat headers but accent-colored on mission avatars. Semantic colors follow the selected theme; do not add fixed colors to status components.
- Shape plus label or tooltip must distinguish each state. Spinner = Working/Starting; hollow circle = Idle; hand = approval; message bubble = answer; triangle = a wait whose reason is unavailable; square = Stopped; circle-exclamation = Error; circle-question = Status unavailable. Working and Starting differ by their label and tooltip.
- Animate only ongoing work/startup, with a static equivalent for reduced motion. Human waits do not blink or pulse. Screen-reader labels carry state and reason; announce a new wait or error once rather than on each timer update.
- Do not add sound, OS notifications, a status dashboard, permission controls, tool-history panels, or configurable status palettes in this feature.

## Source precedence and migration

The v0.8.9 detector is documented in [architecture §5.10](../arch/arch.md#510-busy--idle-inference) and the [archived #584 spec](./archive/584-title-status-detection.md). This issue layers on top of that detector in a later release rather than replacing it, and does not change the 0.8.9 release contents.

Status has two layers, and the lower one is permanent. **The baseline is terminal observation** — byte activity plus title-spinner classification, the v0.8.9 detector — and it is never removed. It runs for every agent session, needs nothing from the runtime, and answers one question: is output moving. **The upgrade is a hook adapter**, which answers the real questions, and it exists per runtime. This is a layering, not a migration: nothing gets deleted when the adapters land.

The consequence that matters for the roadmap is that **a new runtime is supported the day it is added, without an adapter**. It gets the baseline immediately, and an adapter later if and when its CLI turns out to have usable hooks. Adapter work is never a precondition for shipping a runtime, and a runtime with no hook story is not a gap in this feature — it is the lower layer doing its job. The same holds in reverse: a runtime whose hooks regress, or whose bridge fails mid-session, falls back rather than going dark.

Precedence between the layers:

- An active adapter owns the session. PTY output, title updates, and a silence timeout cannot override it. Long silence during a healthy turn is normal, not evidence that hooks failed or that the agent is Idle.
- With no adapter, or after an explicit bridge failure, the baseline owns the session. It is always labelled as an estimate — `Working · estimated` or `Idle · estimated` in the tooltip, with no extra marker beside the glyph (Jason’s 2026-09-14 smoke-test revision supersedes the canvas tilde) — so the user can tell a measurement from a guess.
- The baseline can only ever produce Working and Idle. It never establishes confirmed readiness, needs-you, a turn outcome, or an error, and therefore never holds a delivery. That keeps an estimate away from the one status with consequences.
- Status unavailable is not the state for "this runtime has no hooks" — that is baseline-estimated. It is reserved for a live process with no observation at all, which is rare once the baseline is always present.

The permanent part of the baseline is **byte activity**: any process that produces output, with no assumption about what the CLI writes. Title-spinner classification is not a peer of it but an opportunistic refinement inside it, and it is transitional. `classify_title` is generic — a title beginning `U+2800`–`U+28FF` is Busy — and self-arming, so a runtime that never animates is never affected, but its entire evidence base is two fixtures, `codex-title-working.ndjson` and `claude-session.ndjson`, from the two runtimes that are getting adapters first. Where it works it is about to be superseded, and where it would be needed — a new runtime with no hooks — it may not fire at all. It is also presentation Runner does not control, able to degrade silently when a CLI changes its title format, and [#587](./archive/587-terminal-provided-titles.md) is about to make the same channel user-visible display data.

Remove title-spinner classification in slice 6, on a named condition rather than someday: **when every runtime that animates its title has an adapter**. That is Claude Code and Codex; title classification is retained through slice 3 and removed separately in slice 6. Keep it if a runtime turns up that animates its title and has no usable hooks; it is twelve lines and comes back cheaply. Preserve the existing submit/wake behaviour, with its precedence over both layers covered by tests. Delivery behaviour is deliberately not made stricter by this feature; the only new condition is the needs-you hold, which no baseline-only session can trigger, so unsupported runners keep delivering exactly as they do today.

Shell command status is separate work under [#586](./586-shell-status-detection.md): process detection first, optional semantic shell integration later. Neither shell foreground detection nor OSC 133 reveals the lifecycle inside an interactive agent.

## Implementation phases

1. Review the frames above across direct chats, single-pane and split tabs, sidebar rollups, and missions; they landed 2026-09-14 and this review is what phase 1 is waiting on.
2. Port the bridge mechanism and exercise Claude's turn-end and continuation sequence. TRAE hook integration is unsupported and outside the plan. The Claude review established that its idle notification is delayed and user-disableable. Claude and Codex turn-end displays accept the temporary error from a continued `Stop`; delivery remains independent of Idle.
3. Build what the port does not give us: the per-runtime adapters that turn events into transitions, the normalized state with its source ownership, generation binding, and the main-versus-subagent rule, carried through the existing backend status path. Only capabilities phase 2 confirmed are enabled. Title-spinner classification stays as the baseline layer; the adapter takes precedence over it rather than replacing it.
4. Apply the phase-1 presentation and routing gates to all consumers. Validate lifecycle edge cases, unavailable capabilities, and inbox eligibility on macOS and Windows before rollout.
5. Add the deferred details as later verification allows, one at a time. Each is additive to the phase-1 layout and none of them is a reason to delay phase 1.

## Verification

- Exercise prompt → quiet tool → completion with continued terminal animation in both CLIs. Status stays Working through quiet work and becomes Idle on the verified completion boundary, independently of title settings.
- Exercise repeated turns, user cancellation, errors, process exit, resume, and process replacement. Old-generation and subagent completion events cannot make an active main turn Idle.
- Exercise automatic approval, a real human approval prompt, and a stop hook that requests continuation. Do not confuse approval telemetry or an attempted stop with readiness.
- Test duplicate reports, event ordering, absent hooks, and bridge failure. Silence alone never switches an active hook session back to byte inference.
- Verify existing user hooks still execute and configuration/authentication/resume storage remains intact. Test paths with spaces on macOS and native Windows.
- Cover mission delivery as well as direct-chat status: a nudge must still reach a working agent, an unsupported runner, and a session whose readiness is unknown, and must be held only while an approval or question is unresolved, then released when it clears.
- Verify approval/answer attention survives pane focus, concurrent work, and project collapse, then clears on the correct resolution. Ordinary tools allowed without PermissionRequest must not create a wait. Claude permission requests and named questions raise attention before execution; an automatic decision must clear it on the owning result. Escape cancellation must clear via the correlated transcript result even when Claude emits no Stop/PostToolUseFailure hook.
- Verify successful completion, interruption, recoverable tool failure, fatal turn failure, and process crash produce different outcomes. One pane finishing must not clear another pane's active wait.
- Review the Pencil frames in dark/light themes and narrow panes. Check title truncation, stable header controls, action/shortcut visibility, non-color distinctions, and reduced motion. At implementation time, run relevant backend/terminal and runner-app tests plus workspace Clippy.

## References

- [Codex hooks](https://developers.openai.com/codex/hooks) and [Claude Code hooks](https://code.claude.com/docs/en/hooks) — event and configuration contracts must be verified at implementation time.
- cmux (`~/repos/harness/cmux`, [manaflow-ai/cmux](https://github.com/manaflow-ai/cmux), GPL-3.0-or-later — compatible with Runner's GPL-3.0, so derivation is clean with attribution): `CLI/CMUXCLI+CodexFireAndForgetHooks.swift` is the mechanism to port, `CLI/CMUXCLI+AgentHookCatalog.swift` the event catalog, `CLI/FeedEventClassifier.swift` the classifier whose product decisions are *not* ours, and `docs/agent-session-tracking-spec.md` the record of why config-rewriting installation was left unshipped.
- [Original hook proposal](./archive/52-hook-based-session-status.md) — historical design, superseded by this spec; its Tauri receiver, assumed event mapping, and silence-based freshness decay are not implementation requirements.

Slice-3 reference source: Orca (`~/repos/ai/orca`, `stablyai/orca`, `c1e15c40`) — `src/main/codex/codex-hook-{definition,script,status}.ts`, `src/shared/agent-hook-listener/providers/codex-{events,state}.ts`, `src/shared/agent-question-answered-intent.ts`, and `src/main/agent-hooks/server/`. Local Codex source at `f1433fc71f` supplied research leads; installed 0.154.0 fixture results, not that unpinned source, establish the adapter boundary.
