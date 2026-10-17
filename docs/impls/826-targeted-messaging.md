# 826 — targeted crew messaging: plan

Implementation plan for [#826](https://github.com/yicheng47/runner/issues/826), and the implementation contract confirmed by Jason on 2026-10-17. The [spec](../features/826-targeted-messaging.md) records the motivation and mirrors this plan. Jason settled the read behavior during review: `--since` never acks, and `--all` shows all crew messages since the caller joined while acknowledging only the caller's inbox. The one-line `[inbox]` notification stays. This plan builds on #907's delivery model in arch [§8.5](../arch/arch.md#85-delivery-model) and uses its terms (subscription, backlog, ack, dispatcher, notification, attempt, readiness, redelivery). Code line numbers are as of `e280932f`.

## What happens today

| Area | Where | Today |
| --- | --- | --- |
| Recipients | `Event.to` and `EventDraft.to: Option<String>` (`runner-core/src/model.rs` 114, 130) | One handle or a broadcast. |
| Posting | `runner msg post --to` (`runner-cli/src/command.rs` 461); in-mission posts append directly (`runner-cli/src/msg.rs`); outside posts go through `mission_post` (`runner-daemon/src/ops/mission.rs` 474) | Each path validates one handle on its own; a leading `@coder` is plain text. |
| Subscription | bus `event_targets` and recipients (`runner-daemon/src/event_bus/mod.rs` 361, 476); router `message_nudge` (`router/handlers.rs` 82) | `from ≠ h` and (`to` null or `to = h`); `human` is in no subscription. |
| Notification | `Dispatcher::advance` (`router/dispatch.rs` 394) | `[inbox] N new messages — run runner msg read to view.`, typed as keys. Unchanged by this plan. |
| Reads | `msg read` parses from offset 0 and prints the whole subscription (`runner-cli/src/msg.rs` 83), then writes `inbox_read { up_to }` (165) | Starts at mission start and ignores the ack, so most of each read is old. |
| Composer | one `target: Option<String>` (`runner-app/src/surfaces/mission_composer.rs` 14), one chip (`mission_workspace/composer.rs` 151) | One recipient or everyone. |
| Prompts | `WORKER_COORDINATION_PREAMBLE`, `TEAM_COORDINATION_PREAMBLE` and the lead's Coordination section (`router/prompt.rs` 118, 128, 200) | Describe pull-based reads with one recipient or a broadcast. |

IDs need no change: `EventLog::append` assigns each ID under the log's `flock`, above the last ID on disk (`event_log/ulid.rs`), so ID order is file order and acks by ID are sound.

## Decisions

1. **`Recipients` in `runner-core`.** `to` deserializes from a string, an array or `null`, and serializes a single recipient as a string, several as an array and a broadcast as `null`, so existing lines and single-recipient events stay byte-identical. `Event::recipients()` and `Event::addresses(handle)` replace every direct read of `to`.
2. **One resolver.** `runner_core::addressing::resolve(text, explicit, roster)` returns the recipients and the text: repeatable `--to` wins; otherwise leading `@handle` tokens address the message and stay in its text; mid-text mentions do not address. An unknown recipient, `--to human`, or leading `@human` without explicit recipients is an error naming the valid handles. The in-mission CLI, `mission_post` and the composer call it, so all three agree.
3. **Subscription.** `from ≠ h` and (`to` is empty or contains `h`), starting at `role_joined` for a member added later. The bus and `message_nudge` share `Event::addresses`, so a backlog, an unread count and a notification exist only for recipients. The dispatcher, the notification text and redelivery do not change.
4. **Reads.** `runner msg read` prints unread messages in the caller's subscription, then a digest of other members' directed messages. `--all` prints all crew messages since the caller's `role_joined`, or mission start for an original member, including messages between other members, the caller's sent messages and messages from `human`. `--since` selects messages after an event ID; `--from` filters by sender. Either filter disables acknowledgements, including when combined with `--all`. Unfiltered reads acknowledge only fully printed messages in the caller's own subscription; digest entries and other members' traffic never advance the ack. The read contract below defines selection and the daemon cursor optimization.
5. **Prompts.** The three Coordination texts say to run `runner msg read` when an `[inbox]` line arrives and that it shows only new messages, to run `runner msg read --all` for crew history since joining after a resume or whenever earlier messages are no longer in context, that `--since` and `--from` never mark messages read, how to address one or several members, to reply to the sender, to broadcast only when everyone must act, and not to send acknowledgements or standby messages. The lead text applies only when there is a lead.

## Read contract

Every mode excludes messages before the caller's membership boundary and prints in log order. Only `kind = message` events are displayed. Filtering never changes subscriptions, backlogs or notification recipients.

| Invocation | Messages printed | Ack |
| --- | --- | --- |
| `msg read` | Caller inbox after its latest ack, then the digest if any inbox messages were printed. | Last fully printed inbox message, only if newer than the current ack. |
| `msg read --all` | All crew messages since joining, without a separate digest. | Last fully printed message belonging to the caller's inbox, only if newer than the current ack. |
| `msg read --since ID` | Caller inbox after `ID`, even if some messages were previously acknowledged; no digest. | None. |
| `msg read --all --since ID` | All crew messages since joining and after `ID`; no digest. | None. |
| Any mode with `--from HANDLE` | Apply the sender filter to that mode's message selection; no digest. | None. |

The digest contains directed messages neither sent by nor addressed to the caller, after the effective previous ack and no later than the last inbox message printed in this read. Choose the latest ten in that interval, display them in log order as truncated single lines, and point to `runner msg read --all` for the full messages and omitted entries. This bounds digest progress to the inbox ack without adding a second durable cursor. An empty inbox read emits neither a digest nor an ack.

Add a read-only typed `mission_read_cursor(mission_id, handle)` query in `runner-core`'s protocol and the daemon's mission operations. It returns an optional cursor snapshot with the membership start offset (immediately after the human-authored `role_joined`, or 0 for an original member), latest ack ID and the byte offset immediately after that acknowledged inbox message. The bus publishes these fields together after each complete projection batch and rebuilds them on mount. With no ack, the read starts at the membership boundary; with no initialized cursor, the CLI uses the log fallback. The query never marks messages read.

The unfiltered default and `--from` without `--all` or `--since` may seek after the acknowledged message. History modes (`--all` or `--since`) start at the membership boundary. The CLI folds one captured batch of complete log entries before selecting output, applying any newer valid `inbox_read` signals for its own handle so a lagging projection cannot make sequential reads repeat already acknowledged messages. It writes an ack only after successful output and only through the last fully printed inbox message. Never seek past the `inbox_read` signal itself: a message appended between reading and acknowledging must remain unread.

Socket access is a bounded, best-effort query to the existing daemon, without auto-start. If the socket or cursor is unavailable, fold the log from offset 0 to recover the caller's human-authored `role_joined` boundary and latest valid ack, then apply exactly the same selection and ack rules. Default reads must remain unread-only offline. Stale acks never move the watermark backwards; messages appended after the captured read batch stay unread.

## Where it lands

| Area | Change |
| --- | --- |
| `runner-core` | `Recipients` with its serde forms; `addressing::resolve`; `Event::recipients` and `addresses`; typed `mission_read_cursor` request/response in `protocol/api.rs` and `protocol/mission.rs`; multi-recipient `PostMessageInput`. |
| `runner-cli` | Repeatable `--to`; posts through the resolver; the read contract and optional cursor query through the client adapter; help text for `msg post`, `msg read` and `help agents`. |
| `runner-daemon/src/event_bus` | Recipients from `Event::addresses`; per-member membership and acknowledged-message offsets, published as one cursor snapshot and rebuilt on mount. |
| `runner-daemon/src/router` | `message_nudge` through `Event::addresses`; multi-target `human_said` conversion in `handlers.rs`, including reopen repair; the three Coordination texts in `prompt.rs`. |
| `runner-daemon/src/ops/mission.rs` | `mission_post` through the resolver; validate and persist resolved `human_said` recipients before append; read-only cursor query. |
| `runner-app` | The composer holds several targets as chips following the existing one; the feed renders `@a → @b, @c`. |
| Docs | arch §4.2, §4.3, §8.1, §8.5 (Subscription and Who acks) and §9.3 (the optional read-cursor query); the guide pages the spec names; `make guide-cli`. |

## Commits

1. `feat(cli): read only unread mission messages by default` — decision 4 and the `role_joined` start.
2. `feat(mission): address a message to several members` — decisions 1–3, the composer and the feed.
3. `feat(mission): describe targeted messaging to members` — decision 5, `help agents`, arch, guide, `make guide-cli`, the test record and regression rows.

Each commit builds and passes its tests. The spec, this plan and the brief fold into the first.

## Tests

- **Recipients.** String, array and `null` round-trips; old lines parse; a single recipient serializes as a string.
- **Resolver.** Repeatable `--to`; leading mentions alone and several; a mid-text mention; an unknown handle; rejection of `--to human` and leading `@human` without explicit recipients; explicit `--to` overriding leading mentions; the in-mission CLI and `mission_post` agree on the same inputs.
- **Subscription.** Direct, several-recipient and broadcast messages give backlogs and notifications to exactly their recipients, on the dispatcher's manual clock with the fake port; a member added later starts at its `role_joined`.
- **Reads.** Every row of the read-contract table, including combined flags. With unread A then B, `--since A` prints B without an ack and the next default read still prints A and B. `--all` exposes traffic omitted from the digest, including the caller's sent messages, but advances only to its last printed inbox message. More than ten digest candidates select the latest ten in log order; an empty inbox produces no digest or ack; failed output emits no ack.
- **Read cursors.** A message appended between capturing a read batch and writing its ack is returned by the next read. A stale daemon cursor plus a newer ack in the log does not repeat acknowledged messages. Restarted projection, missing cursor and unavailable socket produce the same output and ack as the current projection, including the join boundary, already-read history and stale ack signals. Exercise the typed query through the CLI adapter; use deterministic fixtures, not sleeps.
- **Human messages.** Single- and multi-target `human_said` signals produce the same resolved recipients live and during reopen repair; legacy single-target payloads still work and a recipient array never falls back to broadcast.
- **Prompts and help.** Both skill renders and `runner help agents` pin the new rules; every Coordination text names unread-only reads and several recipients.

## Live checks

In the development app under the standing authorization in `AGENTS.md`, by the crew's QA slot, with an inexpensive model, low effort and a fresh `/tmp` folder for each fixture. Local live tests use only Codex, Grok or pi.

- **ADDR-1, the scripted exchange.** A four-member Codex crew: one member addresses another, then two (by `--to` and by leading mentions), then broadcasts, and the person answers a question. Per member, record wake-ups, turns and what each read printed. A member who is not addressed stays idle, and every default read prints only new messages and the digest. Explicitly inspect `--all` for messages between other members and verify it only acknowledges the caller's inbox; post unread A then B, read `--since A`, and verify the following default read still returns both.
- **COMPOSER-1.** In the mission composer, pick two targets as chips and send, then one, then none. Exactly the chosen members are woken, the feed shows the recipients, and the existing single-target and mention behavior (MIS-MSG-05) is unchanged.
- **Regression rows.** MIS-MSG-01, 02, 05 and 06, MIS-ADD-01 and MIS-ASK-02, with their expected results updated where they change.

The baseline is today's rule, not a second build: every broadcast wakes everyone, and every read reprints the whole subscription. The spec's findings give the read-volume baseline.

## Risks

- **Old consumers of `to`.** An older Runner skips multi-recipient lines after a downgrade; `mission feed --json` readers must accept an array.
- **Agents that ignore leading mentions.** Agents used to broadcasting may keep writing `@coder` mid-text. The Coordination texts show the leading form, and ADDR-1 records which form agents use.
- **An agent that lost its context.** Today every read reprints the history, which lets a resumed or compacted agent recover by accident. Unread-only reads remove that, so the Coordination texts send it to `--all`.
- **Less visibility of others' traffic.** Members no longer see directed messages meant for others except through the digest; the cap of 10 and `--all` cover the rest.

## After merge

The `codex duo`, `codex trio` and `claude pair` crews' conventions in Jason's Runner drop their delivery-mechanics paragraphs. That is user data, edited by Jason or on his request, not in this PR.

## Mission shape

The `codex trio` (coder as lead, reviewer, QA) in `.worktrees/feat-826-targeted-messaging` on `feat/826-targeted-messaging`, ending in one pull request against `main` with CI green on both platforms and no merge. The brief is [`briefs/826-targeted-messaging.md`](./briefs/826-targeted-messaging.md).
