# 347 — Hook-based agent session status

> Tracking issue: [#347](https://github.com/yicheng47/runner/issues/347)
> Priority: P1. Platforms: macOS and Windows.
> Decision, 2026-09-13: next step after v0.8.9. The release keeps the merged title-spinner and byte-activity heuristics from [#585](https://github.com/yicheng47/runner/pull/585) unchanged.
> Design direction, 2026-09-14: define the richer status model and its UI before implementation. Working, Needs you, and Ready are the everyday states. This supersedes the earlier decision to defer needs-you presentation. Proposal for review; runtime capability verification is still required.
> Scope decision, 2026-09-14: ship the minimum each runtime can actually prove, and grow the vocabulary as capabilities are verified — see [Minimum viable coverage](#minimum-viable-coverage). Runner never gains an approval or answer control of its own, in this feature or later: showing that a session is waiting on you, and focusing that pane when you click it, is the whole of Runner's job. The CLI keeps its prompt.

## Motivation

An agent can be thinking, running a quiet tool, waiting for approval, or ready for another turn while the same foreground process owns its terminal. Process detection cannot distinguish those states. Byte traffic measures output, and a title spinner is presentation that can change with CLI versions or user settings. Neither is a reliable agent lifecycle contract.

Use lifecycle events from Claude Code and Codex to determine agent status. Both currently support command hooks; the local versions checked on 2026-09-13 were Claude Code 2.1.270 and Codex 0.154.0 (`hooks` stable and enabled). Study cmux's hook adapters for the integration shape, then verify each event's semantics and injection mechanism against the supported CLI versions before implementation.

## Scope

- Design Working / Needs you / Ready for Runner-spawned Claude Code, Codex and TRAE CLI sessions, alongside startup, stopped, error, and unavailable-status presentation. Implement the subset each runtime can prove first; an unprovable detail is left out, never guessed. Keep one backend owner for normalized activity and its source; expose only the details each adapter can prove. Update existing status consumers and mission routing together.
- Use per-runtime adapters to turn lifecycle events into session transitions. Prompt submission starts work; confirmed main-agent turn completion ends it. Handle cancellation, failure, exit, and resume explicitly. `SessionStart` alone does not prove either readiness or ongoing work.
- Distinguish main-agent events from subagent events. A subagent finishing cannot make its parent Ready. Bind reports to the owning Runner session and process generation so a delayed event from a replaced process cannot change the new session.
- Account for hook ordering and continuations. A `Stop` hook can be blocked by another hook and continue the turn; it must not unconditionally announce final readiness. A `PermissionRequest` may be automatically approved without showing a human prompt; it must not unconditionally mean waiting for the user.
- Prefer the small cmux-style command-hook bridge into Runner's existing CLI/IPC and session event path. Verify whether that path can carry authenticated, session-scoped reports before introducing another receiver. Bound failures so reporting cannot prevent the agent from progressing.
- Compose hooks additively for each spawn and preserve the user's hooks, configuration, authentication, conversation storage, and resume behavior. Verify Claude's additional settings and Codex's current hook configuration route. Do not assume that `-c` accepts hook definitions or that redirecting `CODEX_HOME` to a mirror preserves everything. No global configuration rewrite as an incidental installation step.
- Support native macOS and Windows, including executable paths with spaces, command quoting, and the actual hook shell used by each CLI. No Unix-shell-only helper dependency.

## Status model

The primary question is whether the agent is working, needs a decision, or can accept another turn. Keep three everyday states; explain tool activity and the reason for a wait with secondary text. Do not create a separate colored status for each tool or infer thinking, reading, writing, or testing from terminal prose.

| State | Meaning | Visible label and detail | Treatment |
| --- | --- | --- | --- |
| Working | A main-agent turn or explicitly observed operation is active. | `Working`; optional `Using tools` or `Compacting context` when verified. | Muted spinner. Reserve amber for human attention. |
| Needs you | An unresolved interaction requires the user, with evidence that the interaction actually reached the user. | `Approval needed`, `Answer needed`, or `Needs you` when the reason is unavailable. Plan approval uses `Approval needed`. | Amber hand for approval; amber message bubble for an answer; amber triangle when the reason is unavailable, because the circle-exclamation belongs to Error. |
| Ready | The live agent is confirmed available for another turn. | `Ready`. A last-turn outcome can say `Response finished` or `Interrupted` in the tooltip. | Small muted hollow circle. New unread output retains the separate accent dot. |

Lifecycle and observability supply the remaining presentation cases; these are not more kinds of agent work.

| Case | Meaning and presentation |
| --- | --- |
| Starting / Resuming | Runner is launching or reattaching the process. Muted spinner with the corresponding label. `SessionStart` alone does not establish Ready. Once attachment finishes, absent activity evidence resolves to Status unavailable rather than an indefinite Starting animation. |
| Stopped | The process ended normally or Runner stopped it. Muted square and `Stopped`; retain the existing resume/restart controls. |
| Error | A confirmed terminal turn failure needs recovery, or the process crashed. Red circle-exclamation and `Error`; detail distinguishes `Response failed` from `Process exited · code N`. A failed tool that the agent can recover from remains Working. |
| Status unavailable | The process is live but activity cannot be established, or an explicit bridge failure invalidates the last observation. Muted circle-question and `Status unavailable`; terminal use remains available. This never means Ready or a failed agent. |

Track process lifecycle, agent activity, last-turn outcome, attention, and observation source separately rather than adding every combination to one status enum. Ready describes availability, not successful completion of the user's task. Completed, interrupted, and failed are outcomes; unread is attention. A live session can fail a response without crashing, and an interrupted session can become Ready without a successful-result notification.

For a blocking interaction, Needs you takes precedence over Working even if another tool is still running. For an explicitly nonblocking question, preserve Working and add needs-you attention; display `Answer needed` with `Still working` in its tooltip. Keep unresolved interaction identities until each is answered, dismissed, cancelled, or invalidated by the owning turn/process. Merely focusing the pane does not clear them. Ordinary prose ending in a question is not a structured human-wait signal.

### Transitions and routing

- An accepted prompt or verified work continuation enters Working and clears the previous outcome. A keypress or attempted submit alone does not prove that the agent accepted input.
- A verified human interaction adds needs-you attention and, when blocking, enters Needs you. A correlated answer/approval/dismissal clears that interaction. Return to Working only when continuation is established; if another interaction remains, retain its attention.
- Confirmed main-agent turn completion with no blocking continuation enters Ready and records the outcome. Completion while its pane is not being viewed creates unread attention. A subagent stop or a single completed tool cannot end the parent turn.
- Interruption ends the active attempt, but return to Ready only when prompt availability is established. No new completion dot or error alert for a user-requested interruption. If readiness cannot be established, show Status unavailable with the interruption in the tooltip.
- Confirmed unrecovered turn failure shows Error until new accepted work, explicit recovery, or session end. An observed automatic retry remains Working. Process exit always overrides live activity and clears obsolete waits.
- Only confirmed Ready, with no unresolved interaction or recovery requirement, passes the status part of automatic inbox delivery. Preserve the existing draft, input, and delivery gates. Working, Needs you, Starting, Error, Stopped, unknown readiness, and estimated idle all block automatic injection. Manual terminal interaction remains available.
- Last-turn completion and idle readiness do not complete a mission. Mission outcomes continue to use mission events. Queued inbox messages are distinct from an agent asking the user for approval or an answer.

### Capability evidence and open adapter questions

Reviewed the current [Claude Code hooks reference](https://code.claude.com/docs/en/hooks) and [Codex hooks reference](https://developers.openai.com/codex/hooks) on 2026-09-14. This is a design capability audit, not an integration test against the installed versions. Revalidate event availability and ordering before enabling a capability.

| Capability | Claude Code | Codex | Design consequence |
| --- | --- | --- | --- |
| Work starts / tools / compaction | `UserPromptSubmit`, tool hooks, `PreCompact` / `PostCompact`. | Equivalent named events are documented. | Show Working; advertise details only with a verified interval, not merely a tool proposal. |
| Permission waiting | `Notification(permission_prompt)` is a candidate for a surfaced prompt. `PermissionRequest` can be resolved by hooks. | `PermissionRequest` precedes the final decision; hooks may allow it. No separate surfaced-human-prompt event is established by this audit. | Verify a surfaced wait and its resolution. Raw requests cannot produce an amber alert. |
| Question / plan waiting | `AskUserQuestion` / `ExitPlanMode` can be answered through hooks. Elicitation notification events are also documented. | No dedicated question-display hook is established by this audit. | Tool names alone do not prove a human wait. Verify tool coverage, automatic answers, display, and resolution before enabling this detail. |
| Turn completion | `Stop` can cause continuation; `Notification(idle_prompt)` is a candidate readiness signal whose timing needs verification. | `Stop` can cause continuation. | A reporting hook finishing first cannot know the outcome of concurrent decision hooks. Establish a final boundary before Ready or inbox delivery. |
| Interrupt / fatal response error | `Stop` excludes user interruptions; `StopFailure` reports API-ended turns. | `Interrupt` is documented; no equivalent terminal-error hook is established here. | Capability-specific outcomes; never infer interruption merely from an Escape key or an error merely from tool failure. |

The local cmux adapters corroborate the permission caveat: `CLI/FeedEventClassifier.swift` treats Codex permission requests as telemetry because its approval reviewer can resolve them. cmux is an integration reference, not proof that every mapping meets Runner's readiness contract.

Before implementation, resolve five gaps with the supported CLI versions: additive configuration and trust; final turn completion after all decision hooks; actual prompt display and dismissal; interruption readiness; and fatal failure versus retry. If hooks cannot expose a boundary, explicitly choose a supported supplemental signal or document that capability as unavailable. Do not silently add transcript parsing, terminal-text classification, or a timeout that pretends to confirm it. Rich status coverage differs by runtime until these gaps are closed; [Minimum viable coverage](#minimum-viable-coverage) says what ships without them.

### Minimum viable coverage

Ship the states that fall out of the events both CLIs already document, and leave every refinement to a later phase. A state that cannot be proven is not shown, and it is never approximated.

Phase 1 vocabulary, the everyday three plus what Runner already owns:

| State | Evidence | Notes |
| --- | --- | --- |
| Working | Prompt submission and tool events. | The plain label only. No `Using tools`, no `Compacting context` yet. |
| Needs you — `Approval needed` | The permission event, when no decision follows it promptly. | One amber state with one reason. Because a hook can resolve a request without a human ever seeing it, hold the amber for a short interval and cancel it if a decision arrives first; the spike fixes that interval against real auto-approvals, and the same interval covers auto-answered questions. |
| Needs you — `Answer needed` | The question event, on the same hold-and-cancel as an approval. | Structurally the same interaction, so it costs one event name and one glyph. Map per tool rather than with a blanket ask state: a question takes the bubble and `Answer needed`, a plan approval takes the hand and `Approval needed`. Both block the turn, so neither needs the nonblocking case. |
| Ready | The turn-completion event, with no continuation. | Availability only. Keeps the existing inbox delivery gate; no new outcome text in the tooltip yet. |
| Starting, Stopped, Error `Process exited · code N` | Runner's own `SessionStatus`. | Already known without any hook. Free. |
| Status unavailable | No adapter, untrusted hooks, or a failed bridge. | Also free, and the honest default for anything unproven. |
| Unread response | Existing accent dot. | Unchanged. |

Deferred until the spike or a later runtime version proves them: `Working · Compacting context` and `Using tools`; `Error · Response failed` as distinct from a process exit; the interruption outcome in the Ready tooltip; elapsed time on a wait; and the nonblocking-ask `Still working` case, which is the only part of the answer story that waits — every ask phase 1 can see blocks the turn. Each is designed on the canvas and each is additive — none of them changes the phase-1 shape of a header, a row, or a card.

Per-runtime expectation going in:

| Runtime | Phase 1 target |
| --- | --- |
| Claude Code | The full phase-1 vocabulary, including `Answer needed` — it documents question, plan and elicitation events. The strongest event set of the three. |
| Codex | Working, `Approval needed` and Ready if its permission and completion events behave; otherwise Working and Ready only, with waits left unshown rather than guessed. No question-display event turned up in the audit, so `Answer needed` stays dark here until the spike finds one — a Codex question reads as Working, which is wrong but not a lie about who is waiting. |
| TRAE CLI | Expect Status unavailable and treat anything better as a bonus. Only the *MCP config* is verified as Codex-shaped: `mcp_set_integration` writes it with the same `codex_write_at` and the same `[mcp_servers.runner]` TOML at `~/.trae/traecli.toml`, `runtime_defaults` parses both as TOML, and `runners/logic.rs` groups `Codex \| Trae` for permission modes. The hook shape is unchecked — `traecli` is not installed on the dev machine, and the only external evidence found on 2026-09-14 is [bytedance/trae-agent#397](https://github.com/bytedance/trae-agent/issues/397), an open and unanswered April 2026 request for lifecycle hooks that therefore do not exist in that agent. That issue is the open-source `trae-cli`, which is a different thing from the `traecli` Runner targets (YAML `trae_config.yaml` vs. our TOML), so it is suggestive, not decisive. Point the spike at TRAE only if the binary is installed; otherwise it takes the no-adapter path and gets no bespoke work. Experimental, on by default only on macOS, unvalidated on Windows, and it must not hold up the other two. |
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

The frames draw the full target vocabulary; [Minimum viable coverage](#minimum-viable-coverage) says which states phase 1 actually lights up. The glyphs reuse the existing lucide vocabulary, and four of them are not in `crates/runner-app/src/assets.rs` yet: `hand`, `message-circle`, `circle-alert` and `circle-question-mark`. `square.svg` and `triangle-alert.svg` are already there, and the spinner and the hollow Ready circle are drawn, not iconography.

### Pane header

Replace the existing five-pixel color-only status dot after the session title with a 12-pixel icon and an 11-pixel label. Keep the current header height and controls. Put status after the flexible, truncating title; avoid shifting the controls during transitions. A normal header shows `Working`, `Ready`, `Approval needed`, or `Answer needed`; work details live in the tooltip rather than a constantly changing title.

A pane header only exists from two panes up — `pane_identity_visible` is false below that — so a one-pane tab shows no status anywhere on the chat surface today, and its sidebar row is the only signal a user has. Give the tab bar the status while the tab holds one pane: same glyph, same label, same tooltip, after the truncating title and before the kebab, so the kebab, stop and fork controls keep their positions. Splitting hands status down to the pane headers and the tab bar drops it, because every pane's own header is then on screen; collapsing back to one pane returns it. The state itself never changes on a split, only where it is drawn. A shell tab shows no agent status in either place, matching `pane_identity_shows_status` today; process detection for shells is [#586](./586-shell-status-detection.md).

At narrow widths, remove normal-state text before truncating the title further; keep the status icon and tooltip. Reserve room for the attention label when possible; below 320 logical pixels, icon-only is allowed for every state. Preserve the full accessible label. Allow status labels to grow for translation rather than assuming a fixed English width.

Tooltips explain the state in plain language: `Waiting for you to approve a command`, `Waiting for your answer`, `Working · Compacting context`, `Ready · Last response interrupted`, or `Status unavailable · Agent is still connected`. Show one short reason when available, not raw hook names or full command payloads. Elapsed time is useful only for a wait, for example `Waiting for 2m`; silence never changes the state.

Clicking a needs-you indicator focuses the corresponding terminal pane. The agent CLI continues to own approval and answer controls. Runner does not duplicate the question, auto-answer it, or synthesize an approval keypress. Avoid a new banner consuming terminal rows for information already visible in the header.

### Sidebar tabs and project rollups

Keep the current compact row layout and runtime/layout icon. The trailing attention slot shows, in priority order: an unresolved error, needs-you, working, unread completion, status unavailable, or nothing. A ready viewed tab stays quiet. Status unavailable ranks last because it is missing knowledge rather than a call for attention, but it still has to be drawn — an empty slot reads as quiet and ready, which is the one thing an unobserved session is not. An estimated working session rolls up as working and keeps its estimate marker; estimated idle contributes nothing and never counts as ready. A tooltip lists concurrent conditions, such as `1 approval needed · 1 working · 1 unread response`, so the dominant icon does not erase the other states. Project and section rollups use the same ordering. Error attention is acknowledged by viewing the affected pane; the pane's Error status remains until recovery. Needs-you attention persists until resolution.

Clicking the attention indicator opens the affected pane, preserving the existing row-click tab behavior. If multiple panes need attention, choose the oldest unresolved error first, then the oldest unresolved request; use pane order as a stable tie-breaker. Do not rotate the target simply because it was clicked. A collapsed project indicator expands the project and reveals the affected tab. Keep attention visible while row actions or shortcut pills appear; reserve separate space instead of replacing the warning with an ellipsis or shortcut.

A row is too narrow for a label, so the glyph carries the state and the tooltip names it. The label appears only where there is room for it: pane headers, the single-pane tab bar, and runner cards.

Unread completion remains the existing accent dot; clear it when that response's pane is actually viewed, not merely because another pane in the tab is active. A new turn may hide the dot behind a working indicator without deleting its unread record. Restoring normal status must not recreate a previously acknowledged completion.

### Mission workspace

Use the same icon, label, and meaning in runner cards and pane headers. The runner subtitle can add a detail because it has more space, for example `Working · Compacting context`. The avatar's presence dot goes back to meaning only what it can prove — the process is live or it is not — and stops carrying agent activity, which is what makes the amber-versus-accent split disappear. Each runner tab in the mission tab strip gets the same glyph on the same rules as a sidebar row: glyph only, label in the tooltip. The mission sidebar row aggregates runner attention; it must not describe an entire mission as Ready when only one runner is ready. Its tooltip reports counts. Clicking its attention indicator opens the affected runner. Preserve the existing pending-asks and inbox surfaces and avoid double-counting the same Runner ask if it already supplies an interaction identity.

### Visual rules

- Use existing theme tokens: muted text for normal activity/readiness/unavailability, warning for human attention, danger for error, accent for unread completion. This intentionally removes the current inconsistency where Busy is amber in chat headers but accent-colored on mission avatars. Semantic colors follow the selected theme; do not add fixed colors to status components.
- Shape plus label or tooltip must distinguish each state. Spinner = Working/Starting; hollow circle = Ready; hand = approval; message bubble = answer; triangle = a wait whose reason is unavailable; square = Stopped; circle-exclamation = Error; circle-question = Status unavailable. Working and Starting differ by their label and tooltip.
- Animate only ongoing work/startup, with a static equivalent for reduced motion. Human waits do not blink or pulse. Screen-reader labels carry state and reason; announce a new wait or error once rather than on each timer update.
- Do not add sound, OS notifications, a status dashboard, permission controls, tool-history panels, or configurable status palettes in this feature.

## Source precedence and migration

The v0.8.9 detector is documented in [architecture §5.10](../arch/arch.md#510-busy--idle-inference) and the [archived #584 spec](./archive/584-title-status-detection.md). This issue replaces that agent detector in a later release; it does not change the 0.8.9 release contents.

Once a supported adapter is active, lifecycle status owns the session. PTY output, title updates, and a silence timeout cannot override it. Long silence during a healthy turn is normal, not evidence that hooks failed or the agent is Idle. Track missing capability or explicit bridge failure separately from silence; never label an unobserved state as confirmed readiness for inbox delivery. Preserve the existing submit/wake behavior only where it agrees with the lifecycle model, with its precedence covered by tests.

Remove title-spinner classification when the hook detector lands. Terminal titles remain display data under [#587](./587-terminal-provided-titles.md). For unsupported runtimes, any retained byte detector is explicitly a heuristic source; it is not a fallback that silently takes ownership from an active hook adapter. Label its activity `Working · estimated` or `Idle · estimated` in the header tooltip, with a distinct estimate marker on the icon — the canvas sets a `~` against the glyph. Never call estimated idle Ready or derive needs-you, completion, or safe inbox delivery from it. A supported runtime with missing/untrusted hooks or an explicitly failed bridge shows Status unavailable after startup. Long silence with a healthy adapter preserves the observed state. This stricter delivery behavior is a deliberate migration change and needs coverage for unsupported mission runners.

Shell command status is separate work under [#586](./586-shell-status-detection.md): process detection first, optional semantic shell integration later. Neither shell foreground detection nor OSC 133 reveals the lifecycle inside an interactive agent.

## Implementation phases

1. Review the frames above across direct chats, single-pane and split tabs, sidebar rollups, and missions; they landed 2026-09-14 and this review is what phase 1 is waiting on.
2. Run a capability spike against the installed CLIs — a throwaway hook script, not production code. Drive a real approval prompt, an auto-approved one, a question, a plan approval, a completed turn, an interrupt and a failed turn on Claude Code and Codex, then, only if `traecli` is installed, point the same script at TRAE CLI to find out whether it has hooks at all. Record which events fire, in what order, and how long an auto-approval takes to resolve. The result fixes the phase-1 vocabulary per runtime and the amber hold interval, and it is what the implementation plan in `docs/impls/` is written from.
3. Implement the session-scoped command bridge, normalized state, source ownership, and only the capabilities the spike confirmed, through the existing backend status path. Remove title-spinner classification as the adapters replace it.
4. Apply the phase-1 presentation and routing gates to all consumers. Validate lifecycle edge cases, unavailable capabilities, and inbox eligibility on macOS and Windows before rollout.
5. Add the deferred details as later verification allows, one at a time. Each is additive to the phase-1 layout and none of them is a reason to delay phase 1.

## Verification

- Exercise prompt → quiet tool → completion with continued terminal animation in both CLIs. Status stays Working through quiet work and becomes Ready on the verified completion boundary, independently of title settings.
- Exercise repeated turns, user cancellation, errors, process exit, resume, and process replacement. Old-generation and subagent completion events cannot make an active main turn Ready.
- Exercise automatic approval, a real human approval prompt, and a stop hook that requests continuation. Do not confuse approval telemetry or an attempted stop with readiness.
- Test duplicate reports, event ordering, absent hooks, and bridge failure. Silence alone never switches an active hook session back to byte inference.
- Verify existing user hooks still execute and configuration/authentication/resume storage remains intact. Test paths with spaces on macOS and native Windows.
- Cover mission inbox eligibility as well as direct-chat status, including unsupported runners and unknown readiness.
- Verify approval/answer attention survives pane focus, concurrent work, and project collapse, then clears on the correct resolution. An automatically answered question or approved request must not flash amber.
- Verify successful completion, interruption, recoverable tool failure, fatal turn failure, and process crash produce different outcomes. One pane finishing must not clear another pane's active wait.
- Review the Pencil frames in dark/light themes and narrow panes. Check title truncation, stable header controls, action/shortcut visibility, non-color distinctions, and reduced motion. At implementation time, run relevant backend/terminal and runner-app tests plus workspace Clippy.

## References

- [Codex hooks](https://developers.openai.com/codex/hooks) and [Claude Code hooks](https://code.claude.com/docs/en/hooks) — event and configuration contracts must be verified at implementation time.
- cmux: `CLI/CMUXCLI+AgentHookDefinitions.swift` and `CLI/FeedEventClassifier.swift` in the local `~/repos/gui/cmux` clone; the Codex permission classifier explicitly accounts for automatic approval.
- [Original hook proposal](./archive/52-hook-based-session-status.md) — historical design, superseded by this spec; its Tauri receiver, assumed event mapping, and silence-based freshness decay are not implementation requirements.
