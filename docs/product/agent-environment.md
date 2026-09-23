# Runner as an environment for agents

> Direction, 2026-09-23, from Jason after a month of daily use. Not yet folded into [`vision.md`](./vision.md); that happens once [562](../features/562-mission-spawn.md) ships and the vocabulary changes with it.

## The shift

Until now the person has driven Runner: they build a crew, start a mission, and the agents take part in it. From here the main caller of Runner is an agent. A Claude Code session starts a mission, spawns a Codex worker into it, waits for the result and stops the worker, without the person clicking anything. Runner becomes the place where agents from different providers spawn, find and coordinate with each other, and the app is the environment they live in.

Most of the groundwork is already done. The 0.11.0 CLI made agents first-class callers: one binary, the caller identified by a roster handle, the coordinator verbs `spawn`, `ps`, `wait`, `stop` and `done` reserved, and MCP removed so there is one way in. This doc names the direction so later decisions can be checked against it.

## What stays: the app is where the person watches

Agents being the main callers does not make the app a thin viewer. The windows, tabs, panes and the real terminal stay core to the product. Every session an agent spawns is a real PTY that the person can open in a pane, read, type into and take over, exactly like one they started themselves. Humans and agents share the same sessions; they just get to them differently (the person through the UI, agents through the CLI).

This is the second ceiling in [`vision.md`](./vision.md) §1 applied: the more agents that spawn without the person, the more the person relies on the terminal layout and tab management to follow them. Work on the terminal and on tabs keeps its priority.

## Why Runner and not the vendors

Claude Code's subagents spawn Claude and Codex's spawn Codex. No vendor has a reason to let its agent hand work to a competitor's as an equal. Being neutral about providers is the one position the vendors will not take, and it is Runner's position already ([`vision.md`](./vision.md) §2).

## What the environment provides

| Need | How Runner meets it | State |
| --- | --- | --- |
| A place to work together | The mission as a container that owns its roster, seeded from a crew or a single role | [#562](https://github.com/yicheng47/runner/issues/562), 0.12 |
| Identity | Roster handles; outside agents take a seat | 648 decisions shipped; seats in #562 |
| Coordination | The mission bus: signals, messages, the inbox | Shipped |
| Handing off directly | `runner session send` into an open chat, with no bus | [#704](https://github.com/yicheng47/runner/issues/704), 0.12 |
| Living past the app | Sessions that survive quitting the app; today they die with it ([`vision.md`](./vision.md) §4.2) | [#645](https://github.com/yicheng47/runner/issues/645), 0.13 |
| Limits | `MISSION_SPAWN_CAP` (8) and one level of spawning: workers ask the lead and do not spawn | In #562 |
| Reaching the person | `ask_lead`, then `ask_human`; desktop notifications | Shipped; [#701](https://github.com/yicheng47/runner/issues/701) |
| Keeping track | Activity / Needs you, plan usage, token ledger | [#552](https://github.com/yicheng47/runner/issues/552), [#706](https://github.com/yicheng47/runner/issues/706), [#630](https://github.com/yicheng47/runner/issues/630) |

The limits matter more as agents do more of the spawning. Once agents can start agents, a runaway fan-out is the first failure to expect, and the cap plus the one-level rule are what stop it.

## What it does not provide

The non-goals in [`vision.md`](./vision.md) §6 stand. Runner does not create worktrees, show file trees, render diffs or sandbox anything. The agent brings its own tools, and a crew that needs its own checkout makes one from its brief. Runner provides the place, the identities, the channels, the limits and the way to reach the person, and nothing more.

## The test

The direction holds if this works end to end without the person touching anything:

1. A Claude Code chat, open in a tab, starts a one-role mission or takes a seat in one.
2. It spawns a Codex worker with a task.
3. The worker's session appears in the app, where the person can watch it but is not interrupted.
4. The chat waits, reads the worker's report from its inbox, and stops the worker or closes the mission.

Steps 1, 2 and 4 are #562 (role-seeded missions, outside seats, `spawn`, `wait`, `done`). Step 3 is an open design question.

## Open questions

1. **Where does a session an agent spawned show up?** In a new tab, in the mission's tab as a pane, or kept in the sidebar until the person opens it. It must not take focus from what the person is doing. This belongs in #562's design phase, before Phase 4.
2. **Does a direct chat get a way to spawn workers without a mission?** #704 says no: direct chats stay off the bus, and a chat that needs workers starts a mission. Revisit only if that step turns out to be friction in real use.

## Roadmap implications

- **0.12 is this direction's first release.** #562 and #704 are the headline, unchanged. The role and crew pages still gate 0.12.0: roles are what agents spawn, so the role page is where the person decides what an agent can pick, and #562 turns crews into templates.
- **#645 is the next headline after 0.12.** Agents cannot live in an environment that closes when the app quits.
- **Update `vision.md` when #562 ships.** Three changes: "Mission: one live run of a roster, seeded from a crew or a role"; the caller can be an agent; and sessions outlive the app once #645 lands.
