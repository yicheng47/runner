# Runner roadmap

Snapshot as of 2026-09-21. The live source is the [GitHub milestones page](https://github.com/yicheng47/runner/milestones); this file mirrors it so the state of the project is readable from the repo without a browser. Update it when an issue changes milestone, a release is cut, or a mission lands, and move the date.

## Where the project is

- **Latest release:** [0.11.1](https://github.com/yicheng47/runner/releases/tag/v0.11.1) on 2026-09-21: session-start and resume status fixes, Windows PATH refresh, splitter drag feedback, live mission flag accents, and project membership inferred from the working directory. The nightly feed builds from `main`.
- **Landed on `main` since 0.11.1:** the Settings → Agents split design and its documentation; implementation is still in flight.
- **In flight:** [#617](https://github.com/yicheng47/runner/issues/617), the Settings → Agents split, in its existing coder/reviewer mission. [#668](https://github.com/yicheng47/runner/issues/668), [#651](https://github.com/yicheng47/runner/issues/651), and [#653](https://github.com/yicheng47/runner/issues/653) are closed. The missions-as-containers plan ([`features/562-mission-spawn.md`](./features/562-mission-spawn.md)) is under review before implementation.

## Releases

| Release | Content | Issues |
| --- | --- | --- |
| 0.11.0 | Shipped 2026-09-20: pi runtime, and the general `runner` CLI with its agent skill as Runner's external control surface; the MCP integration removed | #539, #648 |
| 0.11.1 | Shipped 2026-09-21: session status, Windows agent discovery, splitter feedback, mission accents, project inference, and default crew naming | #659, #670, #672, #673, #653, #680, #676 |
| [0.11.x](https://github.com/yicheng47/runner/milestone/1) | Agent maintenance, mission monitoring, and focused follow-through | #617, #533, #666, #661, #686 |
| [0.12](https://github.com/yicheng47/runner/milestone/2) | Mission ownership and delegation, role/crew workflows, navigation, and the terminal black-pane fix | #562, #619, #393, #577, #559, #468, #647 |
| [0.13](https://github.com/yicheng47/runner/milestone/3) | Session hosting and visibility: local/remote hosts, Activity, shell process status | #645, #552, #586 |
| [0.14](https://github.com/yicheng47/runner/milestone/4) | Workspace features and localization: worktrees, project tree/diff, live shell cwd, 简体中文 | #403, #634, #575, #565 |

A minor is a change to the model or a new surface; a patch is fixes and follow-through. Patch releases have carried features before (0.8.4 to 0.8.8), which is fine for small ones, but #562 migrates every mission's roster and gets its own minor.

## Open work by release

There are 23 open issues: 19 assigned to the four milestones and four deliberately unscheduled. Milestone placement is the release track, not a promise that every issue gates its first release. The `release-blocker` label marks the issues that gate the milestone's .0 release; everything else in the milestone may ship in a patch. [#393](https://github.com/yicheng47/runner/issues/393) explicitly gates 0.12.0. Priority labels are unchanged by the scheduling pass; the subsequently filed [#686](https://github.com/yicheng47/runner/issues/686) is P1.

| Release | Issue | Reason and ordering |
| --- | --- | --- |
| 0.11.x | [#617](https://github.com/yicheng47/runner/issues/617) Installed / Not installed agent sections | [PR #684](https://github.com/yicheng47/runner/pull/684) open; per-card default-agent UI follow-up in progress |
| 0.11.x | [#533](https://github.com/yicheng47/runner/issues/533) update agent CLIs from Settings | Builds on #617's Installed cards; separate follow-through |
| 0.11.x | [#666](https://github.com/yicheng47/runner/issues/666) pi resume with custom environment paths | Follow-through on the pi runtime |
| 0.11.x | [#661](https://github.com/yicheng47/runner/issues/661) repository mapping-test naming | Small maintenance change |
| 0.11.x | [#686](https://github.com/yicheng47/runner/issues/686) watch CLI-started missions by default | P1; reuse the existing CLI follower and slow its 500 ms polling cadence |
| 0.12 | [#562](https://github.com/yicheng47/runner/issues/562) missions as containers | Settle the model and migration/lifecycle contracts before the dependent UI |
| 0.12 | [#619](https://github.com/yicheng47/runner/issues/619) deletion integrity in application code | Coordinate ownership and deletion rules with #562; do not carry crew cascade assumptions into mission slots |
| 0.12 | [#393](https://github.com/yicheng47/runner/issues/393) role and crew page redesign | **Release blocker**; implement against #562's settled model |
| 0.12 | [#577](https://github.com/yicheng47/runner/issues/577) per-role skills and MCP picks | Role setup and spawn behavior; coordinate with #393's layout |
| 0.12 | [#559](https://github.com/yicheng47/runner/issues/559) command palette | Expose the settled mission and role actions through the keyboard |
| 0.12 | [#468](https://github.com/yicheng47/runner/issues/468) landing page | Copy and screenshots should explain the new mission model |
| 0.12 | [#647](https://github.com/yicheng47/runner/issues/647) terminal goes black after sidebar toggle | Moved from 0.11 at Jason's request because of the change's size |
| 0.13 | [#645](https://github.com/yicheng47/runner/issues/645) session host | Local host first, then ssh remotes and the Windows host |
| 0.13 | [#552](https://github.com/yicheng47/runner/issues/552) Activity / Needs you view | Consolidate the status of sessions across missions and hosts |
| 0.13 | [#586](https://github.com/yicheng47/runner/issues/586) shell process status | Keep process observation on the host that owns the PTY |
| 0.14 | [#403](https://github.com/yicheng47/runner/issues/403) git worktrees under projects | Workspace ownership and isolated working directories |
| 0.14 | [#634](https://github.com/yicheng47/runner/issues/634) project tree and read-only diff/file viewer | Phases 1 and 2; remote file browsing remains out of scope |
| 0.14 | [#575](https://github.com/yicheng47/runner/issues/575) inherit live shell cwd | Workspace and split behavior; reported cwd remains distinct from spawn cwd |
| 0.14 | [#565](https://github.com/yicheng47/runner/issues/565) i18n, 简体中文 first | Extract and translate after the mission and role/crew surfaces settle |

## Decisions that shape the next releases

- **One `runner` binary, two modes.** Inside a mission the commands are unchanged and `--mission` defaults to the caller's own; outside, `--mission` is a flag. No second control binary, and no scoping by packaging: a mission agent's shell runs anything on `PATH`. A limit, if ever wanted, is an authorization check on the app side of `mcp.sock`.
- **A caller is the person at the app or a roster handle, never a location.** `human` on the bus means only the person at the app UI. Inside a mission the handle comes from `RUNNER_HANDLE`; outside, `--as <handle>` names a seat the caller holds, and no handle means the person at a terminal. An outside agent that drives a mission takes a seat first. The socket's post and signal tools carry a `from` handle validated against the roster. Seats without a Runner-spawned session are #562's.
- **Runner's MCP integration was removed for 0.11.0, the release that ships the CLI.** Two ways in at once would have been confusing. The `runner-mcp` bridge, the registrations Runner wrote, and the pinned row in Settings → MCP are gone; the socket stays as the CLI's implementation-detail transport, and Settings → MCP remains the catalog of the user's own servers.
- **#648 and #562 stay separate.** 648 is a surface over tools that exist and ships in 0.11.0; 562 changes the mission model and ships as 0.12. 648 reserves the coordinator verbs (`spawn`, `ps`, `wait`, `stop`, `done`); a `peek` at a worker's terminal is not supported, messages are the contract, and #562 owns the seats and the verbs.
- **pi has no MCP client, by design** (its README: "No MCP. Build CLI tools with READMEs, or build an extension that adds MCP support"). Runner writes no pi MCP entry, the Settings MCP pane has no pi column, and the Skills pane lists pi's two personal roots read-only. Crews coordinate through the bundled `runner` CLI, so nothing is missing, and from 0.11.0 no runtime uses MCP to reach Runner.

## Backlog

These four deferrals were explicitly preserved with Jason on 2026-09-21. They have no release date or milestone; they are scheduled by setting a milestone, not by editing this file.

- **P2:** [#592](https://github.com/yicheng47/runner/issues/592) OpenCode runtime; [#644](https://github.com/yicheng47/runner/issues/644) Antigravity CLI runtime; [#630](https://github.com/yicheng47/runner/issues/630) token ledger and per-subscription quota.
- **P3:** [#582](https://github.com/yicheng47/runner/issues/582) split the files that outgrew the 478 audit; time it around feature work on the affected files.
