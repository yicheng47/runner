# 437 — Windows nightly: log

Dated record for the Windows nightly program ([README](README.md), [plan](plan.md)). Newest entries at the bottom; keep entries short: what landed, deviations, carries, blockers. The `nightly` skill's `windows stamp` step appends to the Windows nightlies table.

## Current state (update with each entry)

- **Landed on `nightly-windows`**: Phase 0 compile gate ([#482](https://github.com/yicheng47/runner/pull/482), `9a213e5`), Phase 1 it opens on the PC ([#483](https://github.com/yicheng47/runner/pull/483), `952767c`), Windows title row fix for [#484](https://github.com/yicheng47/runner/issues/484) ([#485](https://github.com/yicheng47/runner/pull/485), `2a28ada`).
- **Current nightly**: `Runner-Nightly-0.7.5.20260905.1323-x64.zip` from `2a28ada`, run 33968763863.
- **Next**: Jason's PC pass on the 1323 zip against the Phase 1 checklist plus the #484 checks; file failures as `bug:` issues against #437 and fix in `fix/437-…` PRs into `nightly-windows`. Then the Phase 2 brief (Job Object lifecycle, one-tier stop, sweep change, both `runner` shims, MCP pipe end to end).
- **Open**: an unsigned Inno Setup installer for the friend (asked 2026-09-05, undecided); the router logs a `mission_warning` "human_response references unknown question_id" on every answer from the app's ask dialog (unfiled, answers still reach the crew).

## Windows nightlies

| Stamp (UTC) | Zip | Source | Run | Carries |
| --- | --- | --- | --- | --- |
| 20260905.1004 | [Runner-Nightly-0.7.5.20260905.1004-x64.zip](https://github.com/yicheng47/runner/releases/download/nightly-win/Runner-Nightly-0.7.5.20260905.1004-x64.zip) (18.8 MB) | `952767c` | [33959658297](https://github.com/yicheng47/runner/actions/runs/33959658297) | First cut: Phases 0 and 1. OS-drawn title bar. |
| 20260905.1323 | [Runner-Nightly-0.7.5.20260905.1323-x64.zip](https://github.com/yicheng47/runner/releases/download/nightly-win/Runner-Nightly-0.7.5.20260905.1323-x64.zip) (18.8 MB) | `2a28ada` | [33968763863](https://github.com/yicheng47/runner/actions/runs/33968763863) | #484 fix: OS bar hidden, caption buttons in the header. |

## 2026-09-04 — spec

#437 reopened as a nightly-only pre-release (the 2026-08-27 won't-do commit `c49b575` never reached `main`). Spec written from a measured compile gap: ~17 errors on `main` `49543b2` with `cargo check --target x86_64-pc-windows-msvc` from macOS, the whole GPUI app type-checking against gpui-ce 0.3.3's Windows backend. Decisions taken in the spec: native spawn (never WSL), Job Objects instead of signal escalation, named pipe for MCP, Windows Terminal keymap conventions, a separate public pre-release tag with the macOS `nightly` staying a draft.

## 2026-09-05 — plan, Phase 0, branch model

Plan written on baseline `08e2bf7` (v0.7.5) as four PR-sized phases; two spec amendments (`Ctrl+Shift+<letter>` for app chords, no Windows orphan-sweep kill) and the PC-is-a-run-box rule.

**Phase 0** — mission `01M1R2VA2TGX2GPYSA3H61J48G` on codex-crew, PR mode. Seven commits in the requested order: `runner_core::app_paths` and all 20 `HOME` reads; `ipc` (unix socket + named pipe) threaded through the MCP server and CLI; `session/process/{mod,unix,windows}.rs` with unix bodies moved verbatim and Windows stubs; `compose_path` / `find_executable` on `split_paths` + `PATHEXT`; `cfg(unix)` on 20 shell-spawning tests; the non-required `Rust / Windows` CI job; `vision.md`. First PR to prove a Windows compile under MSVC. Carry for Phase 2: `portable-pty` 0.9.0 `WinChildKiller::kill` has an inverted success check around `TerminateProcess`.

**Branch model** — after the review, Jason moved the work off `main`: `nightly-windows` created from `main` `7cf7dd1`, checked out in `../runner-windows`, #482 retargeted to it, `ci.yaml` swapped the dead `gpui-nightly` trigger for `nightly-windows` (`8369826`), plan amended (`7b172b6`). #482 landed as `9a213e5` by local `--no-ff` merge because `gh pr merge` refuses workflow-file PRs on this token.

## 2026-09-05 — Phase 1, first nightly

Mission `01M1RD61YNDQE4PSQYAXX95X1F`, PR mode against `nightly-windows`. Four commits: the `nightly.yml` Windows job and `platform` dispatch input with a public pre-release; native window chrome; Windows shortcuts and text editing; the inherited-environment wording. Review round 1 found four defects in the brief by reading gpui-ce source, ruled the same way by the human in the feed and by Jason from the app dialog: `titlebar: None` hides the bar on Windows (`unwrap_or(true)`), `QuitMode::Explicit` leaves a headless process off macOS, `hide_other_apps` is `unimplemented!()`, and a mechanical `cmd-[` → `ctrl-[` steals ESC from terminals. Two more corrections: the release tag became `nightly-win` (a tag named after the branch makes pushes ambiguous), and `ui/field.rs` tested the Win key for Ctrl+A/C/X/V. Landed as `952767c` ([#483](https://github.com/yicheng47/runner/pull/483)); first Windows nightly cut at 10:04 UTC. The YAML anchor and PowerShell packaging were unproven until that run.

**PC pass (Jason)**: the app opens. The OS title bar sat above Runner's 44px header and the sidebar toggle was alone on the second row → [#484](https://github.com/yicheng47/runner/issues/484). Jason also asked why there is no installer; the answer is the spec's non-goal, and an unsigned Inno Setup installer was recommended as the next Windows task if the friend is the audience.

## 2026-09-05 — #484 title row fix, second nightly

Mission `01M1RJD9Z2GVFM8698J7AKJ65T`, branch `fix/437-windows-titlebar`, [#485](https://github.com/yicheng47/runner/pull/485), five commits, three review rounds. Jason chose the toggle at the left of the title row (iconless) over "beside the logo". Review findings that changed the brief: caption buttons must render above every overlay (the brief said under the modals; a modal without outside-dismiss would have trapped the window), `SessionControl` needed the same mouse-down stop-propagation as `Button` (Stop/Resume inside a Drag hitbox became a window drag), the header inset is gated on the side panel and mission rail being closed, and the Runners/Crews list "New" buttons ran under the caption strip. Landed as `2a28ada`; second nightly cut at 13:23 UTC. Missions stopped after landing.

**Housekeeping the same day**: this directory created (plan moved from `docs/impls/437-windows-nightly.md`, README and this log added); the `nightly` skill's Windows paths repointed; fork-chat (feature 60, shipped in #460 on 2026-09-01) archived.
