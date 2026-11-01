# #1097 — Start missions from the launcher

Jason requested a `codex trio` mission on 2026-11-01 to continue [#1097](https://github.com/yicheng47/runner/issues/1097). Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1097-mission-launcher` on `feat/1097-mission-launcher`, which tracks `origin/feat/1097-mission-launcher` and is based on `origin/main` at `b59ddf2a7db5bf615953f68ae076dea74ed42c06`. The root checkout stays on `main`. Use this worktree's own `target/`; do not set a shared `CARGO_TARGET_DIR`.

## Current state

An earlier `claude duo` mission pushed `27f1dc64 wip(ui): start missions from the launcher`. It removes `start_mission.rs`, adds the launcher's crew and goal steps (`start_chat/launcher/mission.rs`), the title rule (`launcher_model::mission_title`), the sidebar's New row and direct +, the required `--title`, the guide pages and the feature spec. Its own message says workspace clippy and `make guide-cli-check` have not run, and it has had no review and no live checks. Treat it as an unreviewed candidate: audit it against the spec, finish the gaps and fix what is wrong. Do not restart from scratch.

## Spec and design

The issue's Scope and Decisions (2026-11-01) and `docs/features/1097-mission-launcher.md` on this branch are the behavior spec. The design is `runner/desktop/specs/1097-mission-launcher.pen` in `~/repos/yeelinks/yeelinks_design`: pull it, open it with `open -a Pen <file>`, read it through the Pencil MCP and never edit it. Frames: `h2a6w` empty new tab, `akLO6` crew step, `n1MhHr` a crew found by typing, `B3XUF` goal step, `ZwEPr` split pane, `gdV31` entry-point notes, `TtpxH` and `dtJPK` the launcher's tab in the sidebar. The issue wins on behavior, the design on visual detail; ask Jason through Runner when they conflict.

Read first: `git show 27f1dc64 --stat`, the spec, `start_chat/launcher.rs`, `start_chat/launcher/mission.rs`, `launcher_model.rs`, `panes/start.rs`, `sidebar/` and `front_page/client.rs` (`start_mission`). The daemon still trims and rejects an empty title; leave it.

## Deliverables

Verify each against the WIP and finish it:

1. **Entry points.** ⇧⌘M, the launcher's Mission row and the palette's New mission reuse the focused empty launcher, else open a launcher tab, on the crew step. Start mission on a crew page (editor and list) opens a launcher tab on that crew's goal step. The sidebar's New mission entry is gone.
2. **Crew step.** Mission chip, crews by last use with pictures and roles, Empty mission on ⇧↵. Esc, ⌫ in an empty field or the chip's × removes a tab an entry point opened, with its row, and returns to where you were; a reused launcher returns to its default page. A crew with no roles is dimmed with "No roles yet" and opens the crew page. Recent stays agents and roles. Typing on the default page lists matching crews after agents and roles, labelled Crew.
3. **Goal step.** The crew's chip (× back to crews) and multi-line goal text, six lines then scrolling; the start folder (Change and project picker as for chats); the crew's slots, read-only, with role and model. ↵ starts, ⇧↵ adds a line, an empty goal is allowed, no title field.
4. **Title.** The rule exists once, beside the launcher: the goal's first non-empty line, trimmed and cut at 60 characters, else `<crew name> <YYYY-MM-DD>`, else `Mission <YYYY-MM-DD>`. `runner mission start` requires `--title` with no fallback; every example that starts a mission gains one (CLI help and agents guide, then `make guide-cli`; READMEs; guide pages, with `zh-CN`). The protocol does not change.
5. **Split panes.** No Mission row; ⇧⌘M from a split opens a new tab on the crew step.
6. **The tab.** A full-tab launcher shows the Runner mark and "Start something new" (`ui.start_something_new`) in the secondary text colour on every step; split panes show no mark. An empty launcher tab, chats included, gets a selected dashed "New" row with the app-window icon on top of its project, or Recents; starting replaces it in place, and switching away from an empty launcher tab removes it. The sidebar's + on a project and on Recents opens a launcher tab directly, with no menu.
7. **Remote.** The crew step lists the host's crews; the Mission row and every mission entry point stay hidden or inert without `catalog` and `startMission`.
8. **Removal.** `start_mission.rs`, its modal state, its unused i18n strings (`en.toml` and `zh-hans.toml`) and its tests are gone; new strings exist in both files. `rg -i "start_mission_modal|StartMissionModal" crates` is empty.
9. **Docs.** The spec's guide pages, with `zh-CN`, describe the launcher flow; the spec matches what ships.

Out of scope: the add-role form (`start_chat/add_role.rs`), agent-named titles (#1100), daemon or protocol changes.

## Validation

Before running checks, list each changed behavior and the check covering it. Automated: unit tests for the title rule (first line, blank leading lines, 60-character cut on a multi-byte string, no goal, Empty mission); `runner-cli` tests and goldens for the required `--title`; `runner-app` tests for the crew step, goal step, entry points, split-pane Mission row, the mark on every step, the sidebar's New row and +, removing an empty tab on Esc, on start and on switching away, remote gating and the start request sent. Then `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo fmt --all --check`, `make guide-cli-check` and `git diff --check`.

Live checks, run by `qa` in the macOS development app under [Test Scope](../../../AGENTS.md#test-scope) after a clean code review:

- **L1 Entry points:** ⇧⌘M on an empty tab and on a busy tab, the Mission row, the palette's New mission, Start mission from the crew editor and list.
- **L2 Crew step:** chip and crew list, a roleless crew, crews found by typing, Esc/⌫/× on an entry-point tab and on a reused launcher, number keys still start chats.
- **L3 Goal and start:** multi-line growth and scroll, folder Change, read-only slots; start an Empty mission with no goal (title `Mission <date>`) and one bounded crew mission with a tiny goal (title from its first line, members receive the goal); stop and archive both.
- **L4 Split pane:** no Mission row; ⇧⌘M opens a new tab on the crew step.
- **L5 Tab and sidebar:** the mark on all four steps and none in a split; the dashed New row, replaced on chat and mission start, removed on switching away; + on a project and on Recents.
- **L6 Remote:** Blocked unless a second host is available; the automated gating tests stand in. Windows is covered by CI, not natively.

## Roles and handoffs

The crew's loop applies: `coder` → `reviewer` on a local unpushed commit by SHA → `qa` with L1–L6 plus any check `reviewer` names. Fixes return through `reviewer` as deltas.

## Authorization

Jason authorized this `codex trio` mission (coder, reviewer, qa) with implementation, validation, local commits on this branch and `qa`'s bounded live checks in the development app. After review and QA are clean: reshape the history so the WIP commit and both brief commits fold into implementation commits whose subjects name the change, push with `git push --force-with-lease` (never plain force), open one PR against `main` that closes #1097, and drive the required macOS and Windows CI green. Fold review and CI fixes into the commits they correct. Rebase onto `origin/main` only when AGENTS.md requires it; never merge `main` into the branch. No tool-specific co-author trailers. The PR carries exactly one risk label per AGENTS.md, keeps `core-review` if the workflow adds it, and lists each live check with its verdict, Blocked ones included.

Stop at an open PR with green CI and report through Runner with the PR URL, final SHA, changes and validation results. Do not merge, release, cut a nightly, archive the mission, delete branches or remove worktrees. No other agents, missions or internal subagents beyond `qa`'s test chats and missions. When waiting for a teammate or Jason, finish the turn; do not poll.
