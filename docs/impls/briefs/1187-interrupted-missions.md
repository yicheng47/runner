# #1187 — Interrupted missions

Jason requested this mission with the claude trio on 2026-11-05. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1187-interrupted-missions` on `fix/1187-interrupted-missions`, based on `origin/main` at `558c9ebbd9f4ba2a3b8b9d7e04ebc127c1adc91e`. The root checkout stays on main.

## Problem and outcome

Issue [#1187](https://github.com/yicheng47/runner/issues/1187): when runnerd stops (an update, ⌥⌘Q, `runner daemon stop`), launch resume brings mission members back idle with their in-flight turns lost. The mission then settles after 30 s, and its linked chat is told "settled" although nothing finished. The outcome is a new mission state, **Interrupted**, recorded per member in the mission's event log, reported once to the linked chat, and shown in the sidebar, the feed, the member surfaces and `runner mission show` until each interrupted member runs a turn.

The spec, [`docs/features/1187-interrupted-missions.md`](../../features/1187-interrupted-missions.md), is authoritative on behaviour. The design is `runner/desktop/specs/1187-interrupted-missions.pen` in the design repo (`~/repos/yeelinks/yeelinks_design`, pull first): frames `g1uiM1` (option A, the amber pause, chosen), `YoQDv` (variant 1, the feed line without a Continue button, chosen) and `jSj1Z` (report and CLI). It is authoritative on visual detail. Option B, option C and the Continue button are not built. Do not open or edit `.pen` files; ask the operator for a PNG export if you need one.

## Read first

- `crates/runner-daemon/src/ops/mission/report.rs`: `derive`, `PhaseInput`, `Member`, `Reports::apply` and its baseline, `line`.
- `crates/runner-daemon/src/daemon/boot.rs` (startup order), `daemon/resume.rs` and `session/manager/resume.rs` (launch resume), `mark_running_for_resume_on_launch` in `daemon/server.rs`.
- `latest_session_statuses_from_log` in `crates/runner-daemon/src/ops/mission.rs`; signal types in `crates/runner-core/src/model.rs`; `MissionPhase` in `crates/runner-core/src/protocol/mission.rs`.
- `crates/runner-app/src/surfaces/sidebar/link.rs`, `surfaces/mission_workspace/feed.rs`, the member tabs and Sessions card in `mission_workspace`, and `ui/agent_status.rs`.

## Deliverables

1. `MissionPhase::Interrupted`, its derivation and clearing, and the per-member mark, as the spec describes.
2. The boot-time `session_interrupted` marking before launch resume, with exactly one Interrupted report per restart and no premature Settled report.
3. The app surfaces: mission row, collapsed summary, feed warning row, member tab, Sessions card, pane header and its tooltip; the companion overview.
4. `runner mission show` STATE values and `--json` fields; `make guide-cli` if help changes.
5. Older clients decode the new value without failing; the PR says how.
6. Strings in `en.toml` and `zh-hans.toml`; the guide pages the spec names, with their zh-CN copies passing `python3 script/check-guide-translations.py`.

## Validation

Automated: the spec's derivation, boot, serde and sidebar tests; `runner-daemon`, `runner-core` and `runner-app` tests; workspace Clippy with warnings denied; `cargo fmt --all --check`; `git diff --check`; the guide translation check. Use this worktree's own target directory. CI green on macOS, Windows and Linux.

Live checks: the six in the spec's Validation section, run by qa in the development app on macOS on the reviewed candidate. Check 1 needs a real development daemon restart with a member mid-turn. Use an inexpensive model at low effort, and give the member a task long enough to still be running when the daemon restarts.

## Authorization and handoff

Jason approved the claude trio (coder, reviewer, qa) for this mission. The crew may commit, push this branch and open one PR against main, with `core-review`, a risk label picked from the diff, and `needs-live-test` until qa passes every check. Stop at an open PR with green CI, reviewed and live-tested, and report the PR URL, final SHA, changes and validation through Runner.
