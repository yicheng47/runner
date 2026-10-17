# 826 — Targeted crew messaging that scales past three agents

> Tracking issue: [#826](https://github.com/yicheng47/runner/issues/826). Priority: P2, milestone 0.16. Platforms: macOS and Windows.
> Status: spec from Jason's 2026-10-08 design discussion, refreshed on 2026-10-17 against the delivery model that [#921](https://github.com/yicheng47/runner/pull/921) shipped for #907, with its open questions settled by Jason the same day; he kept the one-line pointer notification rather than typing message text into agents. Development started on `feat/826-targeted-messaging`. No design: nothing new is drawn except the composer's multi-target picker, which follows its existing single-target chip.
> Related: arch §4.2–§4.3 and §8.1/§8.5 (messages, inbox, delivery model), #907's [plan](../impls/907-reliable-delivery.md#relation-to-826), [#753](../tests/archive/753-inbox-delivery.md) (Windows Codex input delivery), [748](./748-mission-watch-delivery.md) (the starting chat's return path, on hold).
> Implementation contract: the [implementation plan](../impls/826-targeted-messaging.md) is the source of truth, confirmed by Jason on 2026-10-17. This spec records the motivation and mirrors the plan's decisions.

## Motivation

Jason found that with more than three agents in a mission, crew messaging gets messy and slow. Everyone being able to see the crew's traffic is fine; the problem is that seeing a message today usually means being woken by it, and every wake-up costs a turn.

## Findings (2026-10-08)

These findings predate reliable delivery; the [reconciliation](#reconciled-with-reliable-delivery-2026-10-17) below records what has changed since.

| Checked | Evidence | Consequence |
| --- | --- | --- |
| Addressing | `runner msg post --to` takes one roster handle (`crates/runner-cli`), and `Event.to` is `Option<String>` (`crates/runner-core/src/model.rs:113`). The composer has one `target` (`crates/runner-app/src/surfaces/mission_composer.rs`). Local logs hold broadcasts whose text starts with `@reviewer` or `@coe`. | Writing to two members needs a broadcast, which wakes everyone. A typed `@handle` without `--to` is a broadcast too. |
| Wake-up | `message_nudge` (`crates/runner-daemon/src/router/handlers.rs:162`) types `[inbox] new message from @X — run runner msg read to view.`; a broadcast nudges every other roster member. | Each message costs the recipient one turn to fetch the body, and a broadcast costs that turn in every slot. |
| Reads | `runner msg read` (`crates/runner-cli/src/msg.rs:83`) parses the whole log from offset 0 and prints every inbox message since the mission began. It records `inbox_read`, and the daemon keeps per-handle watermarks and unread counts (`event_bus/mod.rs`), but the CLI ignores them. | Across 673 reads in the 57 local mission logs, at most 13% of 4,771 printed messages were new; one read printed 36 messages to deliver 1. Agents re-read old instructions and their context fills. |
| Log content | `session_status` is 85–98% of most mission logs, and every `msg read` parses it. | Reads get slower as missions run longer and crews grow. |
| Retry | The reconciliation tick (`router/mod.rs:828`, 30 s interval, 2 min backoff) wakes a live slot only when it is idle, has unread messages and nothing pending, with a generic `[inbox] unread messages` line. | The safety net exists and never interrupts a busy agent; it only needs to send the messages themselves. |
| Prompts | Workers get `WORKER_COORDINATION_PREAMBLE` and the lead gets a Coordination section (`router/prompt.rs`), both describing pull-based reads. Crew conventions repeat delivery mechanics ("Message delivery is push, not pull") and invite standby acknowledgements; in the #782 mission, reviewer and qa each messaged coder within 22 s of the start. | Messaging mechanics belong in the preamble, which ships with the code that implements them, not in each crew's conventions. |

No local mission had more than three agents, so latency above three agents is not measured yet; verification below covers it.

## Reconciled with reliable delivery (2026-10-17)

The findings and decisions were written on 2026-10-08, before reliable delivery ([#921](https://github.com/yicheng47/runner/pull/921), after 0.16.4) replaced the outbox and the reconciliation tick. Checked against `main` at `5de4c249`:

| Changed since 2026-10-08 | Where | Consequence for this spec |
| --- | --- | --- |
| One dispatcher per mission pushes notifications composed from each subscriber's backlog when an attempt starts; redelivery is its 30-second, idle-only deadline with a two-minute backoff | arch [§8.5](../arch/arch.md#85-delivery-model), `router/dispatch.rs`; `message_nudge` now only signals a backlog | The dispatcher already types the pointer this spec keeps (decision 2); addressing changes only who gets a backlog. |
| Every attempt ends Confirmed, Unsubmitted or Unverified | arch §8.5 | A pasted message body would often end Unverified on a busy agent, which is why decision 2 keeps the pointer. |
| The person's composer messages and answers became messages from `human`, delivered by the pointer line | #907 decision 2, arch §8.1 | "`human_said` already delivers full text" is no longer true; the person's messages and answers stay one read away (decision 2). |
| A member cannot post `--to human`; `human` is not a subscriber | `runner-cli/src/msg.rs`, arch §8.4 and §8.5 | Decision 1 drops `human` as a recipient. |
| Inside a mission, `msg post` and `msg read` use the event log directly, without the daemon | arch §9.3 | Addressing cannot be resolved only in the daemon; the CLI and the daemon share one resolver. |
| `msg read` still parses the log from offset 0, prints every inbox message since mission start and ignores the ack | `runner-cli/src/msg.rs` | Phase 1 is still needed. It also starts a later member's subscription at its `role_joined`, the CLI fix #907 left here. |
| Start, add-role and respawn compose prompts in one place, and #873 makes the lead optional | `member_prompt_channels`, [#873](https://github.com/yicheng47/runner/issues/873) | Phase 3 edits one composer, and the lead text applies only when there is a lead. |

The findings' line references predate this change and are kept as dated evidence.

## Decisions

1. **Addressing decides who is woken.** A message wakes exactly its recipients. `--to` is repeatable (`--to coder --to qa`), and explicit recipients take precedence over leading mentions. Without explicit recipients, leading `@handle` tokens address the message and stay in its text; mentions later in the text do not address. `human` is not a recipient: `--to human` and a leading `@human` parsed as an address are errors, as is an unknown recipient; the error names the valid handles. One resolver in `runner-core`, used by the in-mission CLI before it appends, by the daemon's operations and by the composer, keeps the CLI, the composer and `--as` posts the same. In the log and in `mission feed --json`, `to` stays a string for one recipient and `null` for a broadcast, and becomes an array for several, so every existing line and every single-recipient message keeps its form. The subscription becomes `from ≠ h` and (`to` is empty or contains `h`). The composer's `@` picker allows several targets, and a targeted `human_said` becomes a message to all of them. A message with no recipients is a broadcast and still wakes every other member, because a broadcast is meant for everyone; a broadcast from the person wakes every member.
2. **The notification stays the pointer.** The dispatcher keeps typing #907's one-line `[inbox] N new messages — run runner msg read to view.`, now only to recipients. Jason decided on 2026-10-17 not to type message text into agents: Codex and Claude Code collapse a long paste into a placeholder, and a busy agent reports no turn start within the confirmation window, so a pasted body would often end Unverified and could reach the agent twice. The pointer is typed as keys, confirms reliably and repeats harmlessly. A wake-up still costs one read turn, and the person's answers and composer messages still arrive through that read (#907 decision 2). Readiness, confirmation, redelivery and acks follow arch §8.5 unchanged: the subscriber acks by reading.
3. **Reads return unread by default.** `runner msg read` prints the subscriber's messages after its ack, then the digest below. `--all` prints all crew messages from the subscriber's `role_joined`, or mission start for an original member, including messages between other members, the caller's sent messages and messages from `human`. `--since` selects messages after an event ID and `--from` filters by sender; either filter disables acknowledgements, including with `--all`. Unfiltered reads acknowledge only fully printed messages in the caller's own subscription. The [plan's read contract](../impls/826-targeted-messaging.md#read-contract) defines flag combinations and cursor behavior: a best-effort daemon query supplies membership and acknowledged-message offsets; the log fallback reconstructs the same state and preserves unread-only behavior. A message arriving between a read and its ack remains unread.
4. **Catch-up rides on the read.** An unfiltered default read that prints new inbox messages adds a digest of directed messages neither sent by nor addressed to the caller, after its previous ack and through the last inbox message printed. Show the latest ten in that interval in log order, one truncated line each (`@coder → @qa: candidate ready …`), then a pointer to `runner msg read --all` for the full messages and omitted entries. The cap is the same for every member, including a lead. Other members' messages never wake a member or advance its ack. History and filtered reads omit the digest; an empty default read emits neither a digest nor an ack.
5. **The preamble owns messaging behavior.** `WORKER_COORDINATION_PREAMBLE`, `TEAM_COORDINATION_PREAMBLE`, and the lead's Coordination section when there is a lead, say to run `runner msg read` when an `[inbox]` line arrives and that it shows only new messages, to run `runner msg read --all` for crew history since joining after a resume or whenever earlier messages are no longer in context, that `--since` and `--from` never mark messages read, how to address one or several members, to reply to the sender directly, to broadcast only when everyone must act, and not to send acknowledgements or standby messages. After this ships, the `codex duo`, `codex trio` and `claude pair` crew conventions drop their delivery-mechanics paragraphs.

## Non-goals

Typing message text into an agent's terminal (decision 2); agent-side polling; threads or per-message priority; waking a member for other members' traffic; changing the dispatcher's readiness holds, submit delay, confirmation window or redelivery timing; moving `session_status` out of the mission log (#797 covers status transport).

## Implementation phases

1. **Unread-only reads.** CLI default plus `--all` and the digest, from the ack the daemon already keeps, with subscriptions starting at `role_joined`. Independent of addressing; it can land at any time.
2. **Addressing.** `to` holds one or several handles, with readers accepting the old single-string form so existing logs stay valid; the shared resolver; repeatable `--to`; the multi-target composer and `human_said`; backlogs, unread counts and notifications only for recipients.
3. **Prompts and docs.** The three Coordination texts, arch §4.2, §4.3, §8.1, §8.5 (its Subscription entry, and dropping the note that #826 writes acks for the subscriber) and §9.3 (the optional read-cursor query), `runner help agents` with `make guide-cli`, and the guide pages: [Drive Runner from agents](../guide/guides/drive-runner-from-agents.md) for addressing and reads, and [Missions](../guide/concepts/missions.md) and [First mission](../guide/getting-started/first-mission.md) for the composer's several targets. The crew-convention cleanup follows the merge.

## Relation to 748

[748](./748-mission-watch-delivery.md) is on hold for a new design since 2026-10-17. Nothing here depends on it.

## Verification

- Unit tests: addressing resolution in the CLI and daemon paths (repeatable `--to`, leading mentions, mid-text mentions, unknown handles, rejection of `--to human` and leading `@human` without explicit recipients, explicit recipients overriding leading mentions); backlogs and notifications for direct, multi-recipient and broadcast messages; every read mode and flag combination in the plan, with `--since` and `--from` never acknowledging and `--all` acknowledging only the caller's inbox; join boundaries, digest contents, cap and ordering; stale cursors, read/ack interleaving, restart and offline parity; old single-string `to` lines and human-signal targets still parse and replay correctly.
- Live, in the development app under the standing authorization in `AGENTS.md`, with a four-member Codex crew: a scripted exchange where one member addresses another, then two, then broadcasts, and the person answers a question. Record per member the number of wake-ups and turns, and what each read printed. A member who is not addressed must stay idle, and every default read prints only new messages and the digest. Check `--all` and the non-acking `--since` read as specified in the plan's ADDR-1 case. In the composer, pick two targets and send; exactly those two are woken. Local live tests use only Codex, Grok or pi since 2026-10-16 ([procedure](../tests/full-smoke-test.md#computer-use-test-runtimes)).

## Settled questions (2026-10-17)

Jason chose these on 2026-10-17; the decisions above carry them.

- **The implementation plan governs.** During review, Jason confirmed that the current plan takes precedence over older spec wording. Decision 1 now rejects a leading `@human` when resolving recipients from text, replacing the earlier rule that left it as text and broadcast the message.
- **The notification stays the pointer** (decision 2). This replaces the same day's earlier choice of pasted message text with a shared size limit and pointer-only repeats, because a collapsed paste on a busy agent cannot be confirmed.
- **`to` is a string, an array or `null`** (decision 1). JSON readers accept both shapes; an older Runner reading a newer log skips a multi-recipient line, which matters only after a downgrade.
- **Filtered reads never acknowledge** (decision 3). During review, Jason confirmed that `--since`, like `--from`, leaves unread state unchanged so it cannot mark skipped messages read.
- **`--all` shows all crew messages since joining** (decision 3). Jason confirmed during review that it exposes traffic omitted from the digest while acknowledging only fully printed messages in the caller's own inbox.
- **The digest lives in default reads, capped at 10 for everyone** (decision 4), including a lead, which reads the rest with `runner msg read --all`.
- **Acks by ID are sound.** `EventLog::append` assigns each ID under the log's `flock`, strictly above the last ID on disk (arch §7.1, `event_log/ulid.rs`), so ID order is file order.
