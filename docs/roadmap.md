# Runner roadmap

Snapshot as of 2026-09-20. The live source is the [GitHub milestones page](https://github.com/yicheng47/runner/milestones); this file mirrors it so the state of the project is readable from the repo without a browser. Update it when an issue changes milestone, a release is cut, or a mission lands, and move the date.

## Where the project is

- **Latest release:** 0.10.0 on 2026-09-17. Hook-based status on Windows, status detail in the label (`Working · Using tools`, `Idle · Interrupted`), the Runners menu renamed to Roles, a new icon. The nightly feed builds from `main`.
- **Landed on `main` since 0.10.0:** the pi runtime and its hook-based status ([#646](https://github.com/yicheng47/runner/pull/646), [#649](https://github.com/yicheng47/runner/pull/649)); missions 1 through 4 of the general `runner` CLI ([#650](https://github.com/yicheng47/runner/pull/650), [#652](https://github.com/yicheng47/runner/pull/652), [#664](https://github.com/yicheng47/runner/pull/664), [#654](https://github.com/yicheng47/runner/pull/654)), whose third mission removed Runner's MCP integration; the CLI crate moved to `crates/runner-cli` in [#660](https://github.com/yicheng47/runner/pull/660); and the removal's Windows test fix landed in [#665](https://github.com/yicheng47/runner/pull/665).
- **In flight:** #648 mission 5, which wraps both READMEs and the roadmap and adds pi's real-runtime smoke test; the remaining nightly gate legs; and the JASONPC session that finishes the Windows checks for #539 and #648. The missions-as-containers spec ([`features/562-mission-spawn.md`](./features/562-mission-spawn.md)) is written and waits for 0.12.

## Releases

| Release | Content | Issues |
| --- | --- | --- |
| 0.11.0 | pi runtime, and the general `runner` CLI with its agent skill as Runner's external control surface | #539, #648 |
| 0.11.x | The rest of the 0.11 milestone as it lands, plus fixes | #647, #651, #653, #617, #592 |
| 0.12 | Missions as containers: the mission owns its roster, role-seeded missions, the lead or an outside agent spawns, lists, waits on and stops slots through the CLI | #562 |
| 0.13 | Session host: sessions outlive the app, then remote hosts over ssh, then a Windows host so a JASONPC slot can be a crew's tester | #645 |
| 0.14 | Git worktrees under projects, and the project tree with git status and a read-only diff viewer | #403, #634 |

A minor is a change to the model or a new surface; a patch is fixes and follow-through. Patch releases have carried features before (0.8.4 to 0.8.8), which is fine for small ones, but #562 migrates every mission's roster and gets its own minor.

## 0.11 in detail

| Issue | Priority | State on 2026-09-20 |
| --- | --- | --- |
| [#539](https://github.com/yicheng47/runner/issues/539) pi runtime | P1 | Missions 1 and 2 merged; `pi_runtime_smoke.rs` is in flight with #648 mission 5, while the fixture recording, JASONPC pass and archive stay with Jason and the driver |
| [#648](https://github.com/yicheng47/runner/issues/648) general `runner` CLI | P1 | Missions 1 through 4 merged; mission 5's README, roadmap and pi-smoke work is in flight, with the remaining nightly gate legs and JASONPC session still to run |
| [#647](https://github.com/yicheng47/runner/issues/647) terminal tab goes black after toggling the sidebar | P1 | Open; not started |
| [#651](https://github.com/yicheng47/runner/issues/651) position the READMEs as a cockpit for agent crews | P2 | Open; copy drafted, implementation and the hero reshoot not started |
| [#653](https://github.com/yicheng47/runner/issues/653) a live mission's flag draws in the accent | P2 | Open; not started |
| [#658](https://github.com/yicheng47/runner/issues/658) move the runner CLI to `crates/runner-cli` | P2 | Closed; merged in [#660](https://github.com/yicheng47/runner/pull/660) |
| [#617](https://github.com/yicheng47/runner/issues/617) split Settings → Agents into Installed and Not installed | P2 | Open; not started |
| [#592](https://github.com/yicheng47/runner/issues/592) OpenCode runtime | P2 | Open; probed, spec not written, implementation not started |

The `release-blocker` label marks the issues that gate the milestone's .0 release; everything else in the milestone may ship in a patch.

The milestone description also names the token ledger ([#630](https://github.com/yicheng47/runner/issues/630)) and Antigravity CLI ([#644](https://github.com/yicheng47/runner/issues/644)); neither has a milestone set, so both stay in the backlog, with Antigravity waiting until the binary is installed somewhere to probe.

## Decisions that shape the next releases

- **One `runner` binary, two modes.** Inside a mission the commands are unchanged and `--mission` defaults to the caller's own; outside, `--mission` is a flag. No second control binary, and no scoping by packaging: a mission agent's shell runs anything on `PATH`. A limit, if ever wanted, is an authorization check on the app side of `mcp.sock`.
- **A caller is the person at the app or a roster handle, never a location.** `human` on the bus means only the person at the app UI. Inside a mission the handle comes from `RUNNER_HANDLE`; outside, `--as <handle>` names a seat the caller holds, and no handle means the person at a terminal. An outside agent that drives a mission takes a seat first. The socket's post and signal tools carry a `from` handle validated against the roster. Seats without a Runner-spawned session are #562's.
- **Runner's MCP integration was removed for 0.11.0, the release that ships the CLI.** Two ways in at once would have been confusing. The `runner-mcp` bridge, the registrations Runner wrote, and the pinned row in Settings → MCP are gone; the socket stays as the CLI's implementation-detail transport, and Settings → MCP remains the catalog of the user's own servers.
- **#648 and #562 stay separate.** 648 is a surface over tools that exist and ships in 0.11.0; 562 changes the mission model and ships as 0.12. 648 reserves the coordinator verbs (`spawn`, `ps`, `wait`, `stop`, `done`); a `peek` at a worker's terminal is not supported, messages are the contract, and #562 owns the seats and the verbs.
- **pi has no MCP client, by design** (its README: "No MCP. Build CLI tools with READMEs, or build an extension that adds MCP support"). Runner writes no pi MCP entry, the Settings MCP pane has no pi column, and the Skills pane lists pi's two personal roots read-only. Crews coordinate through the bundled `runner` CLI, so nothing is missing, and from 0.11.0 no runtime uses MCP to reach Runner.

## Backlog

Open issues with no milestone, ordered by priority label. They are scheduled by setting a milestone, not by editing this file.

- **P1:** [#393](https://github.com/yicheng47/runner/issues/393) redesign the role and crew pages.
- **P2:** [#630](https://github.com/yicheng47/runner/issues/630) token ledger and per-subscription quota; [#644](https://github.com/yicheng47/runner/issues/644) Antigravity CLI runtime; [#552](https://github.com/yicheng47/runner/issues/552) activity view; [#559](https://github.com/yicheng47/runner/issues/559) command palette; [#619](https://github.com/yicheng47/runner/issues/619) referential integrity in application code; [#565](https://github.com/yicheng47/runner/issues/565) i18n, 简体中文 first; [#586](https://github.com/yicheng47/runner/issues/586) shell status with process detection; [#468](https://github.com/yicheng47/runner/issues/468) landing page; [#577](https://github.com/yicheng47/runner/issues/577) per-role skills and MCP picks; [#533](https://github.com/yicheng47/runner/issues/533) update agent CLIs from Settings.
- **P3:** [#582](https://github.com/yicheng47/runner/issues/582) split the files that outgrew the 478 audit; [#575](https://github.com/yicheng47/runner/issues/575) splits and new terminals inherit the shell's working directory.
