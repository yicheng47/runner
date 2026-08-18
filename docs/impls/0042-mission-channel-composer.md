# Mission channel composer: the human becomes a message author

## Status

Planned. Tracking issue [#378](https://github.com/yicheng47/runner/issues/378). Design: `design/mission-feed-composer.pen` — frame `c78FA` (feed + composer), `c78FB` (targeted chip state), `ASuQW` (@ mention picker), note `c78Nt`.

## Problem

The mission feed reads like a group channel but the human can't speak into it. Runners already have the full channel: `runner msg post "<text>"` appends a broadcast message (`to: null`), the router nudges every roster handle at a turn boundary, `runner msg read` projects `to == null OR to == handle`, and the worker preamble teaches both verbs (`router/prompt.rs:108-109`). The CLI even removed `--to human` deliberately — reporting to the channel replaced replying to a person. The only actor without a posting surface is the human: the workspace can emit `human_said` (a targeted stdin inject, lead by default) and `human_response` (ask-card answers), but no channel post. Issue #378's runner-side scope is therefore already shipped; what remains is the composer.

## Key Decisions

1. **Human channel posts are ordinary `message` events — no new signal type.** `EventDraft::message` with `from: "human"`, `to: None` (channel) or `to: Some(handle)` (targeted mail). Every consumer already handles them: `message_nudge` fans a broadcast out to the whole roster minus the sender (`handlers.rs:196` — "human" is never in the roster, so all runners get nudged); `runner msg read`'s inbox projection picks them up; unread accounting and the delivery gate apply per recipient; `EventFeed` renders message rows natively. This resolves the issue's naming question: neither `channel_said` nor a generalized `human_said` — no signal at all.

2. **One new command, `mission_post_human_message`, beside the signal command — and exposed over MCP.** Mirrors `mission_post_human_signal_impl`'s shape (running-mission guard, `EventLog::open` + append, event returned): payload is `{ text, to: Option<handle> }`; refuses a `to` outside the crew roster and refuses `to: "human"`. Exposed as an MCP tool with the same contract so external actors (Claude posting into a mission) can speak channel instead of only `human_said`-injecting the lead.

3. **`human_said` stays untouched as the immediate-attention exception.** It injects into one runner's stdin now (through the draft-aware gate); channel posts wait for a turn boundary via the nudge machinery. Two verbs, two urgencies — the composer speaks mail, MCP keeps both.

4. **Composer UI: a pinned input at the bottom of the feed tab, primary window only.** Plain textarea (Enter posts, Shift+Enter newline), placeholder "Message the crew — @handle to address one runner". Typing `@` at position 0 opens a roster picker above the box (design frame `ASuQW`): one row per slot — handle, `role · runtime` subtitle — filtered as you type, ↑↓ to move, Enter/Tab selects into an accent chip, Esc dismisses. Only a leading `@` targets: mid-text `@` never opens the picker, and an unrecognized `@word` falls through as plain broadcast text. The secondary (read-mostly) workspace stays read-only — it already suppresses PTY interaction, and the composer follows (resolves the issue's open question).

5. **Feed treats human messages as human-authored.** `isHumanAuthored` (`EventFeed.tsx`) extends to `kind === "message" && from === "human"` so the feed commits to bottom on your own post, and the row styling matches the existing `human_said` message-row treatment.

6. **No preamble changes.** Runners already learn both message verbs; the human reading the channel is implicit in the feed being the log. The crew-prompt guidance about *when* to broadcast belongs in crew `system_prompt_addendum` text (user data), not this impl.

## Goals

- Type into the feed composer, hit Enter → the post appears in the timeline, every runner gets an inbox nudge at its next turn boundary, and `runner msg read` shows it.
- `@reviewer fix the naming` → only @reviewer is nudged; the feed shows the targeted row.
- Claude (over MCP) can post channel messages into a running mission.
- No new routing, signal types, or delivery semantics anywhere.

## Non-Goals

- Changing `human_said`/`human_response`, the ask-card flow, or any injection path.
- Runner-side changes — the CLI, nudge fan-out, and inbox projection ship as-is.
- Mentions-highlighting in feed rows, message editing/deletion, read receipts, or fuzzy search in the picker (it's a prefix filter over 2-3 handles).
- The secondary-window composer.

## Implementation Notes

- `src-tauri/src/commands/mission.rs` — `mission_post_human_message_impl` + command beside `mission_post_human_signal_impl` (~:435); roster lookup for `to` validation via the crew's slots.
- `src-tauri/src/mcp/tools/mission.rs` — MCP tool registration mirroring `mission_post_human_signal`.
- `src-tauri/src/lib.rs` — command registration.
- `src/pages/MissionWorkspace.tsx` — composer mount under the feed (`feedActive`), gated on `!isSecondary`; posts via a new `api.mission.postMessage`.
- `src/components/EventFeed.tsx` — `isHumanAuthored` extension; message-row styling parity for `from === "human"`.
- `src/lib/api.ts` — `mission.postMessage(missionId, text, to?)`.

## Validation

- Rust: command tests — broadcast append shape (`kind=message`, `from=human`, `to=None`), targeted append with roster validation, refusal for unknown handle, `to="human"`, and non-running mission; router test that a `from="human"` broadcast nudges every roster handle and a targeted one nudges only its recipient.
- Frontend (vitest): mention-picker state machine (`@` at position 0 opens, mid-text `@` doesn't, typing filters, Enter/Tab commits a chip, Esc dismisses, unknown `@word` falls through as text); `isHumanAuthored` classification for human message rows.
- Manual: post a broadcast into a live 2-slot mission → both runners nudged at turn boundaries and `runner msg read` shows it; `@handle` post nudges only that runner; post from MCP; composer absent in the secondary window; Enter/Shift+Enter behavior.
- `cargo fmt --check`, `cargo clippy`, `cargo test --workspace`, `pnpm exec tsc --noEmit`, `pnpm run lint`, `pnpm test`.
