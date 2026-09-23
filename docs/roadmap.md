# Runner roadmap

Snapshot as of 2026-09-23. The live source is the [GitHub milestones page](https://github.com/yicheng47/runner/milestones); this file mirrors it so the state of the project is readable from the repo without a browser. Update it when an issue changes milestone, a release is cut, or a mission lands, and move the date.

## Where the project is

- **Latest release:** [0.11.3](https://github.com/yicheng47/runner/releases/tag/v0.11.3) on 2026-09-22: Codex settles to Idle from its terminal title when hooks are not reporting, pi resumes conversations from relocated session directories, and tidier Not installed cards. [0.11.2](https://github.com/yicheng47/runner/releases/tag/v0.11.2) the day before split Settings → Agents into Installed and Not installed and fixed Codex sessions stuck on Working. The nightly feed builds from `main`.
- **Landed on `main` since 0.11.3:** deterministic backend process tests ([#694](https://github.com/yicheng47/runner/issues/694), [#696](https://github.com/yicheng47/runner/pull/696)) agents watching the missions they start through the CLI ([#686](https://github.com/yicheng47/runner/issues/686), [#698](https://github.com/yicheng47/runner/pull/698)), the keyboard shortcut tidy with fixed shortcuts that can be turned off and stop and resume of the focused session ([#697](https://github.com/yicheng47/runner/issues/697), [#702](https://github.com/yicheng47/runner/pull/702)), and the tab archive label and single Archiving pill ([#700](https://github.com/yicheng47/runner/issues/700), [#705](https://github.com/yicheng47/runner/pull/705)).
- **In flight:** no open pull requests. The two 0.12 headline specs are under review before implementation: missions as containers ([`features/562-mission-spawn.md`](./features/562-mission-spawn.md)) and session-to-session prompts ([`features/704-session-send.md`](./features/704-session-send.md)), specced together as the coordination layer and the terminal layer beside it; the role page ([#393](https://github.com/yicheng47/runner/issues/393)) is next after it, then the crew page ([#699](https://github.com/yicheng47/runner/issues/699)).

## Releases

| Release | Content | Issues |
| --- | --- | --- |
| 0.11.0 | Shipped 2026-09-20: pi runtime, and the general `runner` CLI with its agent skill as Runner's external control surface; the MCP integration removed | #539, #648 |
| 0.11.1 | Shipped 2026-09-21: session status, Windows agent discovery, splitter feedback, mission accents, project inference, and default crew naming | #659, #670, #672, #673, #653, #680, #676 |
| 0.11.2 | Shipped 2026-09-21: Settings → Agents Installed / Not installed split, Codex stuck on Working at an untouched prompt | #617, #687 |
| 0.11.3 | Shipped 2026-09-22: title-aware Codex status fallback, pi resume with custom session directories, Not installed card layout | #688, #666, #695 |
| [0.11.x](https://github.com/yicheng47/runner/milestone/1) | Agent maintenance, the Antigravity runtime, and focused follow-through; #686, #694, #697 and #700 have landed for the next patch; plan usage (#706) moved in from 0.12 | #644, #533, #703, #706 |
| [0.12](https://github.com/yicheng47/runner/milestone/2) | Headline: missions as containers (#562) and session-to-session prompts (#704). Also the role and crew pages, navigation, desktop notifications, and the terminal black-pane fix | #562, #704, #619, #393, #699, #577, #559, #701, #647 |
| [0.13](https://github.com/yicheng47/runner/milestone/3) | Session hosting and visibility: local/remote hosts, Activity, shell process status | #645, #552, #586 |
| [0.14](https://github.com/yicheng47/runner/milestone/4) | Localization and terminal follow-through: live shell cwd, 简体中文 | #575, #565 |

A minor is a change to the model or a new surface; a patch is fixes and follow-through. Patch releases have carried features before (0.8.4 to 0.8.8), which is fine for small ones, but #562 migrates every mission's roster and gets its own minor.

## Open work by release

There are 21 open issues: 18 assigned to the four milestones and three deliberately unscheduled. Milestone placement is the release track, not a promise that every issue gates its first release. The `release-blocker` label marks the issues that gate the milestone's .0 release; everything else in the milestone may ship in a patch. Only [#393](https://github.com/yicheng47/runner/issues/393) (role page) and [#699](https://github.com/yicheng47/runner/issues/699) (crew page) gate 0.12.0. [#562](https://github.com/yicheng47/runner/issues/562) (missions as containers) and [#704](https://github.com/yicheng47/runner/issues/704) (session-to-session prompts) stay the release's headline, but Jason narrowed the gate to the two page redesigns on 2026-09-23, the same day he had marked all four. Priority labels are unchanged by the scheduling pass, except [#562](https://github.com/yicheng47/runner/issues/562), raised from P3 to P1 on 2026-09-22 because it changes how crews work, and [#704](https://github.com/yicheng47/runner/issues/704), filed P2 and raised to P1 on 2026-09-23 when Jason made the two 0.12's headline.

| Release | Issue | Reason and ordering |
| --- | --- | --- |
| 0.11.x | [#644](https://github.com/yicheng47/runner/issues/644) Antigravity CLI runtime | P1; moved from the backlog on 2026-09-22 once Jason could use the CLI |
| 0.11.x | [#533](https://github.com/yicheng47/runner/issues/533) update agent CLIs from Settings | Builds on the Installed cards that shipped in 0.11.2 (#617) |
| 0.11.x | [#706](https://github.com/yicheng47/runner/issues/706) plan usage for Claude Code and Codex | Moved from 0.12 on 2026-09-23 because Jason wants to use it; a usage icon in the Settings row and a popover, also where #533's update signal shows |
| 0.11.x | [#703](https://github.com/yicheng47/runner/issues/703) one monospace font in the app | Found in #697: 20 places ask for Menlo; move them to the bundled JetBrains Mono, key hints to Inter |
| 0.12 | [#562](https://github.com/yicheng47/runner/issues/562) missions as containers | P1, headline; settle the model and migration/lifecycle contracts before the dependent UI |
| 0.12 | [#704](https://github.com/yicheng47/runner/issues/704) send a prompt from one session to another | P1, headline; the terminal layer beside #562, after Orca's split between `terminal send` and orchestration; no bus, no new tables |
| 0.12 | [#619](https://github.com/yicheng47/runner/issues/619) deletion integrity in application code | Coordinate ownership and deletion rules with #562; do not carry crew cascade assumptions into mission slots |
| 0.12 | [#393](https://github.com/yicheng47/runner/issues/393) role page redesign | **Release blocker**; the first of the two page redesigns, split from the crew page on 2026-09-22 |
| 0.12 | [#699](https://github.com/yicheng47/runner/issues/699) crew page redesign | **Release blocker**; after #393, against #562's settled roster model |
| 0.12 | [#577](https://github.com/yicheng47/runner/issues/577) per-role skills and MCP picks | Role setup and spawn behavior; coordinate with #393's layout |
| 0.12 | [#559](https://github.com/yicheng47/runner/issues/559) command palette | Expose the settled mission and role actions through the keyboard |
| 0.12 | [#701](https://github.com/yicheng47/runner/issues/701) desktop notifications | A popup Runner draws itself, after Zed's agent notification; app-side, no backend changes |
| 0.12 | [#647](https://github.com/yicheng47/runner/issues/647) terminal goes black after sidebar toggle | Moved from 0.11 at Jason's request because of the change's size |
| 0.13 | [#645](https://github.com/yicheng47/runner/issues/645) session host | Local host first, then ssh remotes and the Windows host |
| 0.13 | [#552](https://github.com/yicheng47/runner/issues/552) Activity / Needs you view | Consolidate the status of sessions across missions and hosts |
| 0.13 | [#586](https://github.com/yicheng47/runner/issues/586) shell process status | Keep process observation on the host that owns the PTY |
| 0.14 | [#575](https://github.com/yicheng47/runner/issues/575) inherit live shell cwd | Split behavior; reported cwd remains distinct from spawn cwd |
| 0.14 | [#565](https://github.com/yicheng47/runner/issues/565) i18n, 简体中文 first | Extract and translate after the mission and role/crew surfaces settle |

## Decisions that shape the next releases

- **Runner is not an agent development environment.** Worktree isolation ([#403](https://github.com/yicheng47/runner/issues/403)) and the project tree with a read-only diff viewer ([#634](https://github.com/yicheng47/runner/issues/634)) were closed as not planned on 2026-09-22, which leaves 0.14 with localization and the live shell cwd. Checkouts, file trees and diffs belong to git, the agents and the editor; Runner stays what sits between agents.
- **The README is the front door.** The landing page ([#468](https://github.com/yicheng47/runner/issues/468)) was closed as not planned the same day: visitors arrive on GitHub, and the README already explains roles, crews and missions in both languages.
- **One `runner` binary, two modes.** Inside a mission the commands are unchanged and `--mission` defaults to the caller's own; outside, `--mission` is a flag. No second control binary, and no scoping by packaging: a mission agent's shell runs anything on `PATH`. A limit, if ever wanted, is an authorization check on the app side of `mcp.sock`.
- **A caller is the person at the app or a roster handle, never a location.** `human` on the bus means only the person at the app UI. Inside a mission the handle comes from `RUNNER_HANDLE`; outside, `--as <handle>` names a seat the caller holds, and no handle means the person at a terminal. An outside agent that drives a mission takes a seat first. The socket's post and signal tools carry a `from` handle validated against the roster. Seats without a Runner-spawned session are #562's.
- **Runner's MCP integration was removed for 0.11.0, the release that ships the CLI.** Two ways in at once would have been confusing. The `runner-mcp` bridge, the registrations Runner wrote, and the pinned row in Settings → MCP are gone; the socket stays as the CLI's implementation-detail transport, and Settings → MCP remains the catalog of the user's own servers.
- **#648 and #562 stay separate.** 648 is a surface over tools that exist and ships in 0.11.0; 562 changes the mission model and ships as 0.12. 648 reserves the coordinator verbs (`spawn`, `ps`, `wait`, `stop`, `done`); a `peek` at a worker's terminal is not supported, messages are the contract, and #562 owns the seats and the verbs.
- **pi has no MCP client, by design** (its README: "No MCP. Build CLI tools with READMEs, or build an extension that adds MCP support"). Runner writes no pi MCP entry, the Settings MCP pane has no pi column, and the Skills pane lists pi's two personal roots read-only. Crews coordinate through the bundled `runner` CLI, so nothing is missing, and from 0.11.0 no runtime uses MCP to reach Runner.

## Backlog

These deferrals were explicitly preserved with Jason on 2026-09-21; #644 left the list for 0.11 on 2026-09-22. They have no release date or milestone; they are scheduled by setting a milestone, not by editing this file.

- **P2:** [#592](https://github.com/yicheng47/runner/issues/592) OpenCode runtime; [#630](https://github.com/yicheng47/runner/issues/630) token ledger and per-subscription quota.
- **P3:** [#582](https://github.com/yicheng47/runner/issues/582) split the files that outgrew the 478 audit; time it around feature work on the affected files.
