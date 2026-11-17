# 1369 — Mid-turn inbox delivery on every runtime that accepts it

> Tracking issue: [#1369](https://github.com/yicheng47/runner/issues/1369). Scheduling and priority live on the issue and in the [roadmap](../roadmap.md). Platforms: macOS, Windows and Linux.
> Status: implemented, 2026-11-17; selected macOS live checks passed, Windows validation blocked pending a host.
> Follows: #1103 (0.19.1), which delivers to working Claude Code members and kept every other runtime idle-only; its spec is at a [fixed revision](https://github.com/yicheng47/runner/blob/5e437b517f4d7e2ad81a3a8d2bdf19372f044573/docs/features/1103-mid-turn-delivery.md). Related: #1108 (a Settings switch for mid-turn delivery), #753 and #1075 (why Codex stayed idle-only), PR #1076 (Runner's own leftover notice), #748 (reports to a linked chat stay idle-only).

## Motivation

A mission member hears about a new message through an inbox notice that Runner types into its composer: `[inbox] N new messages — run `runner msg read` to view.` Only Claude Code members get one while they work (`RuntimeMetadata::queues_input_mid_turn`). Every other runtime waits until the member is idle, so a Codex coder in a 30-minute turn does not see a correction sent five minutes in. Crews coordinate only at turn boundaries.

Codex was kept idle-only because a notice typed into a busy Codex composer could sit unsubmitted. In #753, Windows paste-burst detection turned Runner's Enter into a newline. In #1075, Runner then treated its own leftover notice as a person's draft and held delivery for 26 minutes. Neither reason stands any more:

- **Codex accepts input during a turn, and has for a while.** #766's QA on 2026-10-03 recorded it (`crates/runner-terminal/fixtures/input-codex-queued-submit`): a prompt submitted with Enter while Codex was Working showed "Messages to be submitted after next tool call" and was consumed in that turn, as Claude Code does. This is Codex's steer behaviour; `codex features list` on 0.169.0 reports `steer` as `removed true`, meaning the setting is gone and the behaviour is permanent. Where steer is off, in an older Codex or one configured without it, Codex's earlier queued-message behaviour still takes the text out of the composer and submits it as the next prompt when the turn ends; that is no worse than idle delivery. This is not re-verified here, since 0.169.0 no longer lets steer be turned off, and the leftover path covers a Codex that leaves the text in place.
- **A leftover notice is already safe.** PR #1076 tracks Runner's own unsent text as `InputState::Unsent`, never types a second notice over it, and presses Enter on it again at the turn end or a reconcile. A member that goes idle with unread mail is nudged again after the redelivery backoff.

The notice is only a nudge. The message is durable in the mission's event log, and the agent reads it with `runner msg read`. So a notice typed during a turn does not need confirming: it is best-effort, and today's idle delivery remains the confirmed path.

## Scope

### Behaviour

- **A working member of a qualifying runtime gets its notice during the turn.** This is today's `DeliveryReadiness::Working` path, which Claude Code already uses: a new message is attempted at once, and redelivery of an old backlog stays idle-only. The attempt is unchanged: reserve, type, Enter after `SUBMIT_DELAY`, confirm within `CONFIRM_WINDOW`, one second Enter if the probe still finds the notice.
- **Best-effort.** Nothing waits on a mid-turn attempt's outcome. Confirmed is handled as today. Any other outcome leaves Runner's notice as #1076's tracked leftover. It is resubmitted at the turn end, on a readiness change or at a reconcile, and Runner never types a second notice over it. If mail is still unread when the member goes idle, the existing confirmed idle delivery and reconcile apply. The worst case is one duplicate notice, which is harmless.
- **A miss during a turn is quiet until the turn ends.** Today an `Unsent` hold with unread mail posts the "inbox notice is still unsubmitted" `mission_warning` after `REDELIVERY_BACKOFF` (two minutes), even while the member is still busy. A leftover from a mid-turn attempt is expected to wait for the turn end, so its two minutes start when the member next reports idle, not when the attempt was made. The slot's "Message not submitted — press Return" pill still shows, because it is accurate.
- **Holds unchanged.** Draft, recent-typing, Needs you, startup, in-flight, unconfirmed and `Unsent` holds block a mid-turn attempt as they do now. Runner never types into a person's draft or over its own leftover.
- **One notice per new message, as for Claude Code today.** The issue proposed one notice per turn. This spec keeps the existing rule instead: every new message is attempted while the member works, and no notice is typed while an earlier one is still unsent. A confirmed notice already covers every unread message, so a burst of messages costs a few short notices that the agent reads together.

### Qualifying runtimes

A runtime qualifies when Enter pressed during a busy turn submits the typed text to the agent during that turn, without cancelling, restarting or corrupting the turn. A runtime that ignores Enter while busy, leaving the text in its composer, does not qualify: it would only turn every mid-turn message into a leftover.

| Runtime | Status |
|---|---|
| Claude Code | Qualifies today (#1103). |
| Codex | Qualifies on every supported version: with steer the agent reads the notice after its next tool call; without it Codex submits the notice when the turn ends. macOS live checks passed; Windows validation is pending. |
| pi, Antigravity, OpenCode | Native busy-composer qualification and automatic delivery reruns passed on the reviewed enabled macOS candidate. Enabled. |
| Copilot | Not probed because policy/authentication blocked startup. Stays idle-only. |
| Grok, Cursor | Do not qualify: input was queued until after the original turn's final reply. Stay idle-only. |
| Trae | Stays idle-only, not tested (Jason, 2026-11-17). |
| Shell | Not a mission agent; out of scope. |

The capability stays per runtime in `runtime_metadata.rs`. Rename `queues_input_mid_turn` to `accepts_input_mid_turn`, because Codex steers the text into the running turn rather than queueing it. Its comment states the qualification rule above. `only_claude_code_is_marked_for_mid_turn_delivery` becomes a test that names the qualifying runtimes. Codex's `min_version` (0.149.0) does not change: the capability needs no version check, because a Codex without steer still accepts the notice and only delivers it later.

## Non-goals

- Reports to a mission's linked chat. They stay idle-only under #748's decision. Delivering them during a turn is a separate follow-up.
- #1108's Settings switch. Revisit it once this ships; with most runtimes qualifying, it may no longer be needed.
- Delivery through hooks or a Codex app-server connection instead of the composer (#1046, #1012).
- Changes to how the probe locates a notice, to `SUBMIT_DELAY` or to the Windows paste-burst handling, unless the Windows live check shows they are needed.

## Risks

- **Codex paste-burst on Windows (#753).** A busy Codex turn redraws often, so Enter may land inside a paste burst and become a newline. The leftover path covers it, and the Windows live check measures how often it happens. If most mid-turn notices on Windows end as leftovers, the crew reports that rather than tuning `SUBMIT_DELAY` inside this change.
- **A runtime whose Enter interrupts a turn.** The qualification probe exists to catch this before the flag is set.
- **Steered text in the transcript.** A steered Codex notice shows in the transcript as a user message. Runner already ignores a submitted notice still visible in Codex history (`InputTracker`'s transcript handling), but the live check confirms that it does not read as a draft.

## Delivery

One pull request, `core-review`, by the approved codex trio:

1. **Codex.** Rename the capability, set it for Codex, and start an `Unsent` leftover's warning clock at the member's next idle. Tests and docs as below. qa runs the Codex live checks on macOS and Windows.
2. **The other runtimes.** qa runs the qualification probe for pi, Copilot, Antigravity, OpenCode, Grok and Cursor on the reviewed candidate. The coder sets the flag for each runtime that passes, the reviewer reviews that change, and qa reruns the probe for those runtimes on the final head.

## Verification

### Automated

Dispatcher and readiness tests with the fake injector (`crates/runner-daemon/src/router/tests/`):

- A Working Codex member with a new message gets one attempt; one with only an old backlog does not.
- An unconfirmed mid-turn attempt leaves an `Unsent` leftover, no second notice is typed over it during the turn, and the leftover is resubmitted when the member reports idle.
- No `mission_warning` while the member stays busy with a leftover, however long the turn; the warning follows two minutes after the member reports idle with the leftover still unsent and mail unread.
- Unread mail at idle after an unverified mid-turn attempt gets the existing confirmed idle delivery.
- Draft, recent-typing, Needs you, startup, in-flight and `Unsent` holds still block a Working Codex member.
- `accepts_input_mid_turn` is true exactly for the qualifying runtimes, and false for Trae, Shell and unknown keys.

### Live checks

Run by qa on the macOS development app, plus a Windows host for the Windows rows. Each uses an inexpensive model, low effort and a bounded tool window, such as a prompt asking the member to run `sleep 90` and then reply. Record event IDs and covering reads in `docs/tests/runs/1369-mid-turn-delivery-<platform>.md`.

1. **Codex on macOS, mid-turn** (MIS-MSG-01's Codex half, now inverted): while a Codex member is Working, post an addressed message. Pass: the notice is typed and submitted during the turn, the member runs `runner msg read` before the turn ends, and the steered notice in Codex's history does not hold later delivery.
2. **Codex on Windows, mid-turn** (MIS-MSG-03, now inverted): three notices during bounded tool windows with default paste detection. Pass: each is submitted during its turn, or, if Enter became a newline, resubmitted at that turn's end with no stacked second notice. Record how many needed the turn-end resubmit.
3. **Draft still holds** (MIS-MSG-01's draft clause): type a draft into a busy Codex composer, then post a message. Pass: nothing is typed into the draft; the notice goes after the draft is submitted or cleared.
4. **No stall warning during a long turn**: keep a Codex member busy for more than two minutes with unread mail. Pass: `runner mission show` lists no delivery warning while it works.
5. **Qualification probe** for pi, Copilot, Antigravity, OpenCode, Grok and Cursor, each where installed and signed in: post a message while the member is Working. Record whether Enter submitted it during the turn, was ignored, or disturbed the turn. Only "submitted, turn undisturbed" qualifies. A runtime that is not installed or not signed in stays idle-only and is recorded as not probed.
6. **Claude Code unchanged** (regression of #1103): one mid-turn message to a working Claude Code member is read before its turn ends.

Update MIS-MSG-01 and MIS-MSG-03 in [`docs/tests/regression/missions.md`](../tests/regression/missions.md) to the new expectations.

## Guide pages

- [`docs/guide/concepts/missions.md`](../guide/concepts/missions.md) and its zh-CN copy: the inbox paragraph that names Claude Code as the only runtime delivered to while it works.
- [`docs/tech/mission-message-delivery.md`](../tech/mission-message-delivery.md): the `Working` readiness row, the qualification rule, and the warning rule for a leftover from a mid-turn attempt.

## Decisions settled with this spec

- Mid-turn delivery is best-effort; idle delivery stays the confirmed fallback (Jason, 2026-11-17).
- P1, in 0.20 (Jason, 2026-11-17).
- Trae is not tested and stays idle-only (Jason, 2026-11-17).
- Codex's minimum version stays 0.149.0: Codex accepted Enter during a turn before steer became permanent, and without steer the notice is only delivered later.
- One notice per new message rather than one per turn, matching Claude Code today.
- Reports to a linked chat are out of scope.
