# 562 — missions as containers: program record

Implementation record for [feature 562](../../../features/archive/562-mission-spawn.md) ([#562](https://github.com/yicheng47/runner/issues/562), "delegate work to other agents", milestone 0.16). The spec says *what*; this directory says *how, in what order, and what has landed*. This file is the condensed state and the decisions that bind every mission. [plan.md](plan.md) has each phase in detail, and [impl_log.md](impl_log.md) is the dated log. Briefs go in [`../briefs/`](../../briefs/) as `562-m{n}-{slug}.md`. The design is `runner/desktop/specs/archive/562-mission-spawn.pen` on `main`, drawn phase by phase.

## Status (2026-10-15)

| Mission | Scope | Crew | State |
| --- | --- | --- | --- |
| m1 | Phase 1: each mission keeps its roster in `roster.json` (fixes #863) | — | Merged 2026-10-11 as [#868](https://github.com/yicheng47/runner/pull/868), shipped in 0.15.2 |
| m2 | Each mission session independent of the others (Phase 2 decision 9, split out) | codex trio | Merged 2026-10-12 as [#874](https://github.com/yicheng47/runner/pull/874); QA passed five macOS checks, Windows unverified |
| — | [#878](https://github.com/yicheng47/runner/issues/878): a session records its whole launch definition at spawn, so role edits never change it | codex trio | Merged 2026-10-13 as [#881](https://github.com/yicheng47/runner/pull/881) after live QA on macOS; it also fixed the feed's raw card for a stopped mission. Windows unverified |
| m3 | Phase 2: add and remove roles while a mission runs, CLI and app | codex trio | Merged 2026-10-13 as [#889](https://github.com/yicheng47/runner/pull/889); ten selected macOS/Codex QA checks passed, post-#888 recheck waived at Jason’s call, native Windows unverified |
| m4 | Phase 3: start a mission empty; roles join one at a time through add-role; with #849's Start mission redesign | codex trio | Merged 2026-10-15 as [#906](https://github.com/yicheng47/runner/pull/906) (`caa7b72b`), closing #849, in six commits: the five planned steps and a startup-delivery fix Jason's live test called for. CI passed on macOS and Windows. Jason confirmed startup delivery and archiving live and accepted the remaining QA gaps; the [test record](../../../tests/archive/562-start-empty.md) keeps every matrix, including the final basic regression (1 Passed, 2 Failed, 1 Blocked). Brief [`562-m4-start-empty.md`](https://github.com/yicheng47/runner/blob/v0.16.2/docs/impls/briefs/562-m4-start-empty.md) |

Related issue: [#873](https://github.com/yicheng47/runner/issues/873) makes the mission lead optional; Phase 3 builds its no-lead paths, since an empty mission has no lead.

## Decisions that bind

- **A mission owns its roster** (Jason, 2026-10-11). The database holds the definitions that are listed and searched (roles, crews, slots); a mission's roster is its own state, in `roster.json` beside its event log, and a crew edit never changes a mission that has started.
- **Each phase is designed first and ships its GUI with its CLI** (Jason, 2026-10-12). Phase 2 and Phase 3 each draw their frames on `main`, stop for sign-off, and land daemon, CLI and app work in one pull request.
- **Roles and crews are templates; each layer copies at creation** (Jason, 2026-10-12). A mission copies its crew into its roster at start, and a session records its whole launch definition at spawn (#878). Resume replays a session exactly; Restart takes up role edits. The spec's "What is copied, and when" has the rules.
- **Each mission session is independent** (Jason, 2026-10-12). One session's exit or failed spawn never stops another; shipped in m2.
- **Nothing assumes a mission has a lead** (2026-10-12). New code reads the lead from a slot's flag, so #873 stays a local change (plan, Phase 2 decision 14).
- **Missions run on the codex trio** (Jason, 2026-10-12). `qa` runs the mission regression checks in the development app on macOS; Windows is Jason's.
- **A mission can start empty** (Jason, 2026-10-15). A start names no roles; roles join one at a time through add-role, and a mission started empty has no lead. Start-from-roles is dropped.
- **The database never deletes** (Jason, 2026-10-15). The app removes or unbinds every dependent row in code, as it has since `da4333ac`; Phase 3's migration turns every `ON DELETE CASCADE` into `RESTRICT`, so the foreign keys only refuse.
- **The coordinator can sit outside the mission.** A chat that starts a mission drives it through the `runner` CLI; crews stay as templates for repeatable shapes.

## Open

- Native Windows verification of m3 ([#889](https://github.com/yicheng47/runner/pull/889)). The [test record](../../../tests/archive/562-add-remove-roles.md) preserves the selected macOS matrix, failed fixture attempts, recheck waiver, evidence and cleanup; CI is tracked on the PR.
- m4's unverified checks: resuming a conversation that existed before migration 0028 (accepted on count, link and column evidence, not run) and native Windows. A nightly from `caa7b72b` was cut on 2026-10-15 for Windows and daily use before the release. On 2026-10-15 that nightly migrated the production database on Jason's Mac from 27 to 28 with `PRAGMA foreign_key_check` empty, every mission's crew and sessions still linked, and its saved sessions resumed at launch; Jason checked the nightly on Windows before 0.16.2 shipped.
- m4's follow-ups: mission delivery reliability ([#907](https://github.com/yicheng47/runner/issues/907)), an intermittent Codex exit when several sessions resume at `runnerd` start ([#918](https://github.com/yicheng47/runner/issues/918), cause not established), and the CLI help goldens' positional cap, which dropped `ask` and `call` help coverage ([#915](https://github.com/yicheng47/runner/issues/915)).
- #562 closed on 2026-10-16. The lead's verbs (`spawn`, `ps`, `wait`, `stop`, `done`), crash notices to the lead and outside seats continue in [#916](https://github.com/yicheng47/runner/issues/916); deleting a role that unarchived missions use is [#917](https://github.com/yicheng47/runner/issues/917).
- m1's light Windows check, at the next update of Jason's PC with existing missions ([record](../../../tests/archive/562-mission-roster.md)).
- The release that ships #881 and m3 gates on a Windows PC pass: back up the database, upgrade over 0.15.3 with real data, the suite's smoke cases, and the Windows checks deferred from m1, m2, #881 and m3.
