# 748 — Link a chat to the missions it starts

> Tracking issue: [#748](https://github.com/yicheng47/runner/issues/748). Priority: P1, milestone 0.19. Platforms: macOS, Windows and Linux.
> Status: draft, rewritten 2026-11-02 after Jason chose reports typed into the chat. The same day he chose the linked group (option E) for the sidebar, the Agents list (option H) for the Active view, and linking by hand. Its details and the remaining decisions need sign-off before implementation; see [Sidebar](#sidebar) and [Decisions to confirm](#decisions-to-confirm).
> Replaces the 2026-10-27 draft, in which the chat heard about its missions only when the person asked (last at `e42ec525`), and the 2026-10-20 side-panel options A–C in the design file; see [Superseded designs](#superseded-designs).
> Related: #686's watch contract ([test record](../tests/archive/686-watch-cli-started-missions.md)), which this retires; [704](https://github.com/yicheng47/runner/issues/704), which first specified `RUNNER_SESSION_ID` and closed unbuilt; [#701](https://github.com/yicheng47/runner/issues/701)'s desktop notifications; [873](https://github.com/yicheng47/runner/blob/2a22742e0b89e1a8711c858c0b11532cb04497f4/docs/features/873-remove-mission-lead.md), which removed the mission lead; [#916](https://github.com/yicheng47/runner/issues/916)'s verbs; [#1108](https://github.com/yicheng47/runner/issues/1108)'s delivery setting for working agents.

## Motivation

A chat is Jason's frontman. He talks with it about anything, and it delegates work by running `runner mission start` and `mission add-role`. By design it is not a member of the missions it starts. Nothing links them: the mission does not know which chat started it, the chat does not show its missions, and nothing tells the chat when one of them needs attention.

The generated skill fills the gap by making the agent watch. Rule 5 requires every external operator to arm `runner mission feed <id> --follow --json` in a background facility that wakes it while idle, and rules 6–7 cover re-arming, expiry and the "automatic watching is unavailable" fallback. That is too heavy:

- **Codex cannot do it.** It exposes no tool that brings a background process's output into an idle conversation, so a Codex chat reports that watching is unavailable and Jason comes back to ask. The chat that started #724's mission did exactly that on 2026-09-28.
- **Claude Code can, at a cost.** Monitor keeps one follower process per mission, every event it surfaces wakes the chat for a model turn, the feed fills the conversation's context, and the skill spends three rules on keeping the watch alive.
- **Since #873 there is no lead** ([#1049](https://github.com/yicheng47/runner/pull/1049), 2026-10-27). The chat that starts a mission is its only coordinator, so the missing link matters more.

Every other orchestrator sends the result back to the delegator. Claude Code's subagents return a summary, a background subagent's result reaches its parent as a notification in a later turn, and an agent team's lead is told when a teammate stops. Claude's Projects is the closest match to the frontman: one coordinating conversation starts a thread per task, each thread reports back to that conversation when it finishes, and an Overview groups the threads by state. Runner's mission matches the inside of that model, with members talking peer to peer in one visible feed, but the mission never reports back to the chat that started it.

## Direction

Jason, 2026-11-02, revising 2026-10-17 and 2026-10-27:

1. **A link, not membership.** When a chat runs `runner mission start`, Runner links the mission to that chat, and the person can link or move a mission by hand. The chat stays outside the mission.
2. **The mission reports; the agent does not watch.** Runner tells the chat when one of its missions needs the person, fails or settles. The agent reports that the mission started and finishes its turn. No follower, no Monitor, no polling.
3. **Reports are typed into the chat, for every runtime.** Runner types one `[runner]` line into the chat's terminal, the way members receive `[inbox]` notices, because typing is the one input every runtime accepts. This reverses the 2026-10-17 decision that Runner never types into a normal chat. Claude Code's own inbox socket, which delivers without typing, can follow later as an opt-in for Claude Code chats.
4. **The person sees it too.** The sidebar shows each chat with its missions, and the existing desktop notifications keep their triggers.
5. **Independent lifecycles.** Archiving the chat leaves its missions running; archiving a mission removes it from the chat's group.

## Scope

### Identity: `RUNNER_SESSION_ID`

Every agent session gets its own session ID in `RUNNER_SESSION_ID` on spawn, resume and fork, as 704 first specified. Today direct chats export only `RUNNER_HANDLE` as a label, and only runtimes with Runner hooks carry `RUNNER_HOOK_SESSION`, which belongs to the hook protocol (`runtimes/helpers.rs::hook_env`). The variable is an address, not a caller: a chat still acts for the person (arch §9.3), and a direct chat stays off the bus (arch §9.4). The session ID is stable for the chat's life, since resume and restart reuse the same row (arch §5.3 and §5.5).

The variable must reach the `runner` the agent's tool shell runs. Mission slots do not rely on the PTY's environment for this: a per-session shim bakes their variables into `runner` (`cli_install::install_session_runner_shim`), because Claude Code's Bash tool has run commands in a shell without them. Phase 1 checks whether Codex and Claude Code pass a direct chat's environment through. If either does not, direct chats get the same kind of shim, keyed by session ID.

### The link

- A mission has at most one **linked chat**: the chat it reports to and is grouped with. It is stored as a nullable `missions.linked_session_id` column with a `missions.report_to_chat` flag, added by one migration. The flag defaults to on. Deleting the chat clears the link; archiving either side keeps it.
- **Starting a mission links it.** The `mission_start` request takes an optional `started_by_session_id`, which the CLI fills from `RUNNER_SESSION_ID`. For a direct agent chat, the daemon links the new mission to that chat and records the chat in the `mission_start` event as the mission's starter, which never changes afterwards. It ignores a terminal pane and a mission slot, and the start succeeds either way. `mission start --no-report` links without reports.
- **The link can be set or moved later**; see [Linking by hand](#linking-by-hand). `mission resume`, `add-role` and other commands leave it unchanged. A mission started in the app, or by an agent outside Runner, starts unlinked.
- The local and companion protocols carry it additively as `linked_chat: {session_id, title, archived, reports}` or null on mission summaries and details, so remote windows show the same link and clients of other versions still parse.

### Linking by hand

- **Drag.** In Projects, drag a mission row onto an agent chat's row. While the mission is over the middle half of the row, the row shows an accent outline and a "Link to <chat>" label, and dropping there links the mission. The top and bottom quarters keep today's reorder markers, the way a project row already accepts a drop into it. Dropping between two rows of a group links the mission to that group's chat at that position, and dragging a linked mission out of its group to a place among the project's rows unlinks it.
- **Menu.** A mission row's context menu has **Link to chat**, listing the agent chats in the mission's project, or in Recents for a mission outside a project, and **Unlink from <chat>** when it is linked. Active has no drag, so it links through this menu only.
- **CLI.** `runner mission link <mission>` from inside a chat links the mission to that chat, and from outside one it takes `--chat <session>`. `--no-report` links without reports. `runner mission unlink <mission>` removes the link.
- **Rules.** Only a direct agent chat that is not archived can be linked, never a terminal pane or a mission slot. The chat and the mission must be in the same project, or both outside one. Linking a mission that already has a chat moves it to the new one. Dropping on a tab that holds several agent chats asks which chat to link.
- **The chat finds out.** Linking from the sidebar, or from another chat's CLI, sends the newly linked chat one report with the mission's current state: `[runner] Mission 01M6DD9 "1025 — Remote chat switch repaint" is now linked to this chat; it is working. Read it with runner mission show 01M6DD9.` Linking from the chat's own CLI sends nothing, since its agent ran the command. Unlinking sends nothing and drops a pending report for that mission.

### Mission state

The daemon derives one state per live mission from what the app's notifications compute today (`crates/runner-app/src/notifications.rs`). It moves into the daemon so reports reach the chat while the app is closed, and the app and remote clients read it from there.

| State | When | Reported to the chat |
| --- | --- | --- |
| **Needs you** | A `human_question` is pending, or a member waits for input or approval. | Yes |
| **Failed** | A member crashed or exited with an error. Other members may still be running. | Yes |
| **Working** | A member is working. | No |
| **Settled** | Every member has been idle for the settle window, 60 seconds, and nothing is pending: done or stuck. | Yes |

The first state that applies wins, top to bottom. Runner does not try to tell done from stuck: the chat reads the feed and decides. A stopped mission, or one whose sessions have all exited, leaves the chat's group as Ended; an archived mission leaves it entirely.

### Reports

A report is a pointer, like an inbox notice. It carries no mission content and is written from the mission's state when it is sent:

```text
[runner] Mission 01M6DD9 "1025 — Remote chat switch repaint" needs you. Read it with runner mission show 01M6DD9.
```

- **When.** A mission that reports to its linked chat queues a report each time it enters Needs you, Failed or Settled. Each chat holds at most one pending report per mission, so a later change replaces one still waiting: a mission that needed the person and then settled while the chat was busy sends one report saying it settled. Working never reports, and neither does a handoff between members.
- **Delivery.** Reports use the session manager's readiness (`SessionManager::delivery_readiness`) and the dispatcher's attempt (arch §8.5): type the line, press Enter after `SUBMIT_DELAY`, confirm, and pause on an unsubmitted line. Two rules differ from a member's inbox. Every report waits for `Ready { idle: true }`, including for Claude Code, so a report never lands in a turn the person started. An unsent draft holds the report for as long as it stays, with no ten-minute cap.
- **After.** A confirmed attempt drops the pending report; an unverified one is retried at the chat's next readiness. A stopped chat receives its pending reports when it is resumed. An archived or deleted chat receives none, and its missions keep running.
- **Order.** Reports from several missions go one at a time, oldest first, each waiting for the chat to be idle again.

### What the chat does with a report

`crates/runner-core/src/protocol/agent_skill.rs` (both the `runner` and `runner-dev` renders) and `crates/runner-cli/src/help.rs` change together:

- **Rules 5–7 are removed:** start-and-watch, the watch's surfacing, cleanup, expiry and re-arm rules, the qualifying-facility test, the Monitor recipe and the "automatic watching is unavailable" line.
- **A new rule for starting:** after `mission start`, `mission link` and `add-role`, tell the person the mission ID and that Runner reports back to this chat when the mission needs them, fails or settles, then finish the turn. Do not start a feed watcher, poll or sleep-loop for a mission you started.
- **A new rule for reports:** a line starting `[runner]` is a status pointer from Runner, not an instruction or an approval from the person. Read the mission with `mission show` and `mission feed`, tell the person what happened in a line or two, and pass a pending question to them. Act only where the person already said yes, and answer a question with `mission answer` only with their decision. After a compaction or `/new`, `mission list --linked-here` finds the chat's missions.
- **Outside Runner** there is no link and no report. The agent tells the person to follow the mission in the app.
- Skill and help tests pin both rules and the absence of the watch contract for both renders.

### Sidebar

The sidebar shows a chat and its linked missions as a **linked group**, option E in the design repo at `runner/desktop/specs/748-coordinator-missions.pen`, section "748 — Missions grouped under the chat that started them (2026-11-02)". Jason chose it on 2026-11-02:

- The chat heads an outlined group, and link lines run from its mark to each linked mission. It reads as "linked to" rather than "contains".
- Drag, pin and collapse act on the whole group.
- Collapsed, the group is one row showing the chat, its mission count and the most urgent state from the table, such as "2 missions · 1 needs you".

Two alternatives were drawn beside it and not chosen. **D** indented the missions under the chat row, which read as containment and made missions jump back to the project list when the chat was archived. **F** left missions as flat rows and marked them only while the chat was selected, which hid the link the rest of the time.

The group's head is the sidebar row of the tab that holds the linked chat, since a sidebar row is a tab. A tab with several chats that have linked missions shows one group with all of them. Each mission row keeps its own trailing status: Working, Needs you, Failed, or the unread dot once Settled. Archiving the chat dissolves the group and its missions return to flat rows in their project. An unlinked mission is a flat row.

### Active view

The Active view groups work the same way, option H in the design file's section "748 — Active view with linked groups (2026-11-02)". Jason chose it on 2026-11-02:

- Chats and missions share one **Agents** heading in creation order, replacing the separate Chats and Missions headings. A chat and its linked missions form the same linked group as in Projects. Terminals keep their own heading.
- Rows still never move: status changes, unread output, renames and restarts keep their place, as #887 decided. Collapsing the Agents or Terminals heading is remembered as before.
- With [#990](https://github.com/yicheng47/runner/issues/990)'s filters, a group shows while its chat or any of its missions matches, and a mission that matches nothing hides inside its group. This replaces #990's statement that the Chats → Missions → Terminals grouping is unchanged.

Two alternatives were drawn beside it and not chosen. **G** kept the three headings with groups under Chats, which put missions under two headings. **I** grouped rows by state (Needs you, Working, Settled), which moved a whole group every time one of its missions changed state.

### Mission back-link

On the mission side, the header carries a "linked to <chat>" caption that opens the chat, and reads "linked to <chat> (archived)" when the chat is archived. The design file draws it, together with a chat receiving two reports.

Desktop notifications keep their current triggers, settings and targets. Clicking one still opens the mission.

### CLI

- `mission start` gains `--no-report`, and its JSON gains `linked_chat`. Plain output, `-q`, stdout and exit codes are unchanged, and `mission start` still returns at once.
- `mission link` and `mission unlink` set and remove the link, as in [Linking by hand](#linking-by-hand).
- `mission show` gains `STARTED BY` and `LINKED TO` lines and the `linked_chat` JSON field, with whether it reports.
- `mission list --linked-here` lists the missions linked to this chat, read from `RUNNER_SESSION_ID`. Outside a Runner chat it exits with a hint.
- `mission feed --follow` stays, for people and scripts that want a live tail.

### Docs

- **Guide:** [Drive Runner from agents](../guide/guides/drive-runner-from-agents.md) loses the external watch contract in "Start and follow a crew" and gains the link, reports, `mission link` and `--no-report`. [Missions](../guide/concepts/missions.md) describes the linked chat and reports. [Chats and windows](../guide/guides/chats-and-windows.md) describes the linked group in Projects, linking by drag and menu, and the Agents and Terminals headings in Active. `make guide-cli` regenerates the [CLI reference](../guide/reference/cli.md).
- **Architecture:** `docs/arch/arch.md` §8.5 adds report delivery beside the inbox dispatcher, §9.4 documents `RUNNER_SESSION_ID` and the link, §9.6 replaces the skill's start-and-watch contract, and §10.1 adds the two columns.
- **Regression:** [MIS-WATCH-01 and MIS-WATCH-02](../tests/regression/missions.md) retire, replaced by rows for the link, a report reaching an idle chat, a report held by a draft, and the agent that does not watch.

## Decisions to confirm

Settled on 2026-11-02: the linked group, option E, in Projects; the Agents list, option H, in Active; and linking by hand, by drag, menu and CLI.

1. **Reports fire on Needs you, Failed and Settled** (recommended), not on Working or member handoffs.
2. **Reports are on by default** for a mission a chat starts, with `--no-report` to turn them off (recommended). The alternative is opt-in with `--report`.
3. **The settle window is 60 seconds** of every member idle with nothing pending.
4. **Only a direct agent chat can be linked**, automatically or by hand. A mission slot that starts another mission links nothing, and a terminal pane cannot be linked.
5. **The group follows the chat** when the chat's pane moves to another tab (recommended), rather than staying with the original tab.
6. **A group whose missions have all settled stays expanded** until the person collapses it (recommended), rather than collapsing on its own.
7. **A chat and a mission link only within one project** (recommended), so a group never pulls a mission out of its project.
8. **Linking by hand sends the chat one report** of the mission's current state (recommended), unless the chat's own CLI made the link.

## Out of scope

- **Claude Code's inbox socket** as a second, non-typing delivery path for Claude Code chats. It can follow as an opt-in if typed reports prove intrusive.
- **States read from the pull request**, such as Ready for review, Landing and CI status from `gh pr view` on the mission's branch. They would sharpen Settled later.
- **A missions view in the chat's right side panel.** The sidebar group and the reports cover it. If one is wanted later, it fits as a plugin side-panel view ([#1072](https://github.com/yicheng47/runner/issues/1072)).
- **#916's verbs**, including `runner done` and `wait`.
- **Codex outside Runner** through its app server.
- **Jason's personal `mission-watch` skill**, and his standing rule that Claude Code arms a watcher after every mission start. Both are updated outside the repo when this ships.

## Superseded designs

**2026-10-27, pull-only.** The chat heard about its missions only when the person asked, and a badge and strip on the chat showed their state. Jason replaced it on 2026-11-02 because the frontman could not react when a crew finished or asked a question. Its text is at `e42ec525`.

**2026-10-20, side panel.** Options A–C showed the chat's missions in its right side panel, with a Catch up chat button that typed one line on request. On 2026-11-02 Jason dropped a missions view in the right side panel altogether: the sidebar group shows each mission's state and the reports bring changes into the chat. The frames were removed from the design file and remain in the design repo's history.

**2026-09-29, watching chat.** The mission's dispatcher typed `[Runner] Mission …` notices into the starting chat with the readiness holds, confirmation and redelivery of arch §8.5. Jason rejected it on 2026-10-17 because the chat is his conversation. This design revives its delivery with stricter holds: idle only, no draft cap, a report only on three states and none for handoffs. Its findings on why the agent cannot watch still hold: Codex has no idle-wake facility, a Stop-hook or `write_stdin` loop keeps the chat's turn open for the whole mission, and the Codex app server cannot reach a chat that Runner hosts. The full text is `docs/features/748-mission-watch-delivery.md` at `9e5ba05e`.

## Implementation phases

1. **Backend link.** `RUNNER_SESSION_ID` for every agent session, with a shim for direct chats if a runtime drops it; the migration with `linked_session_id` and `report_to_chat`; the starter in the `mission_start` event; the link and unlink requests with their validation; `linked_chat` on the local and companion protocols; the `--linked-here` query. Tests: starting from a chat links it, while a terminal pane, a slot and a missing variable link nothing; resume and `add-role` from another chat leave the link unchanged; linking moves a link, refuses an archived chat, a terminal, a slot and another project; deleting the chat clears it; an old database migrates.
2. **Mission state and reports.** The state derivation in the daemon with the settle window, the per-chat pending report, and delivery through readiness and the attempt. Tests: the state precedence; one report per transition and replacement while pending; idle-only for Claude Code; a draft holds without a cap; an unsubmitted line pauses; a stopped chat receives on resume; an archived chat receives nothing; `--no-report`; one report on linking by hand and none from the chat's own CLI.
3. **App.** The linked group in Projects, the Agents list in Active, linking by drag and menu, and the mission's back-link, reading the daemon's state. Tests: grouping in both views, the drop zones on a chat row, the menu's chat list, collapse summary precedence, archive dissolving the group, a mission without a starter, a chat with two missions, creation order in Active, and the #990 filters on a group.
4. **CLI, skill and docs.** `--no-report`, `mission link` and `unlink`, the JSON fields, the `STARTED BY` and `LINKED TO` lines, `--linked-here`, the skill and help rules, the guide, arch and regression rows.
5. **Live checks**, under the standing development-app authorization in `AGENTS.md`; see Verification.

After the design is signed off, the phases are one `codex-trio` mission, since reports and the sidebar need live checks by its QA slot. The change touches the daemon's delivery, the protocol and a migration, so its PR takes `core-review` and `risk-high`.

## Verification

### In-process

- Database, daemon and CLI tests for phases 1 and 2, including `mission show`, `mission link` and `mission list --linked-here` output, JSON and golden files.
- Router and session-manager tests for report delivery, reusing the inbox delivery fixtures.
- App tests for the sidebar group and the back-link.
- Skill and help tests for both renders; `make guide-cli`; workspace clippy.

### Live, macOS

Run in the development app with an inexpensive model, low effort, tiny prompts and fresh folders under `/tmp`.

- [ ] A fresh Codex chat with only the installed skill, asked to start a small mission that asks a yes/no question and then finishes, returns at once, gives the mission ID, starts no follower and ends its turn.
- [ ] When the question is pending, one `[runner] … needs you` line is typed into the idle chat; the chat reads the mission, relays the question and does not answer it. After the answer and the mission's finish, one `… settled` line arrives and the chat summarizes the result.
- [ ] With unsent text in the chat's input, a report waits past ten minutes and arrives only after the text is sent or cleared. A report never arrives during a turn, in Claude Code as well as Codex.
- [ ] Killing one member produces one `… failed` report while the other member keeps running.
- [ ] Two missions from one chat both report, one at a time. A mission started with `--no-report` never reports. A mission started from the app is unlinked and stays a flat row until it is linked by hand.
- [ ] The sidebar shows the chat and its missions as a linked group, with each mission's status, and the collapsed group shows the count and the most urgent state; archiving the chat leaves its missions running as flat rows with an "(archived)" caption, and archiving a mission removes it from the group. In Active, the same group appears under Agents beside chats and unlinked missions, and it keeps its place when a mission's state changes.
- [ ] Dragging an unlinked mission onto a chat row links it and the chat receives one "now linked" report; dragging it out of the group unlinks it; Link to chat in the row's menu does the same in Active; `mission link` from inside the chat links without a report.
- [ ] Claude Code in a Runner chat behaves like Codex and arms no Monitor.

Windows and Linux use the same daemon and surface and stay unverified until checked.
