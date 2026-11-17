# #1213 m1 — macOS live QA

## Candidate and environment

2026-11-17, approximately 02:57–03:09 UTC (10:57–11:09 CST). **Passed for the selected macOS feature scope.** Native Windows execution is an unavailable-platform gap, as allowed by the m1 brief; this is not a full regression pass or m2 validation.

- Repository: `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1213-worktrees`.
- Branch: `feat/1213-worktrees`.
- Reviewed and tested commit: `02ef5ae8d1d2d6b2e5fdf67fa5572fedd109c731`; base: `d768d23344a6c6f1717a0342b3c8dd56dc5ae68a`.
- Working tree was clean before building. Both staged and unstaged binary diff hashes were SHA-256 `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`. HEAD and product source stayed unchanged through QA; this record is added afterwards.
- macOS 26.6.2, build 25G83, arm64; Git 2.54.0 (Apple Git-157), Cargo/rustc 1.97.1; Codex CLI 0.169.0; Claude Code 2.1.326 installed. Live agent fixtures used Codex `gpt-6-luna`, low effort, and `/bin/zsh` for the terminal.
- Authorization: Jason's hydrogen trio m1 mission, its reviewed-candidate handoff, and standing development-app testing authorization. QA changed no product source, tests, expectations, authentication, or permission settings and made no commits or pushes.

## Identity, isolation and method

Evidence root: `/private/tmp/1213-m1-4zicq0yg` (canonical form of `/tmp/1213-m1-4zicq0yg`). `candidate.json`, `build.json`, `plan.md`, `ledger.json`, and `cleanup.json` hold the detailed identity, plan, fixture IDs and cleanup ledger. Screenshots and command results are under its `evidence/` directory; these local artifacts are retained rather than committed.

Acquired `script/qa-lock` before building. Ran `make run` on the frozen candidate, stopped that owned bare executable, and copied byte-identical `Runner` and `runner-agent-cli` binaries into `Runner 1213 QA.app`, bundle ID `com.wycstudios.runner.qa1213`, under the evidence root. App SHA-256: `81b5fd5d08e6c8ea63959ff10c3b1e4845d8a998f53944df25d64b429cd5d96b`; CLI SHA-256: `3fc2014efb1dbdabe41ae493c39b1532880a58f8976f86ae9aef72c24a34e286`. The development installation's `runner` and `runnerd` hashes match the copied CLI. `status.json` and `status-after-restart.json` report `0.20.0 (dev)`, the development socket `com.wycstudios.runner-dev/runnerd.sock`, and outside-mission identity. Process evidence identifies the copied app executable and its development daemon.

Product validation calls used the absolute development CLI path through the recorded `qa_cli.py` helper, with `RUNNER_SESSION_CLI`, `RUNNER_CREW_ID`, `RUNNER_MISSION_ID`, `RUNNER_HANDLE`, and `RUNNER_EVENT_LOG` removed. Candidate app launches also removed those variables and `NO_COLOR`. Installed Runner supplied implementation-mission coordination. No SQLite database was opened.

Native computer use selected the distinct wrapper, opened/dismissed New tab as a harmless control check, clicked owned sidebar rows and launch controls, typed into their actual terminals, and captured screenshots. CLI state corroborates session cwd, project, running status and conversation identity; git corroborates registrations, branch retention and excludes. CLI success alone was not used as evidence of rendering or terminal input.

Fixture: a git-initialized `main/` with a committed README, `outside/` created with `git worktree add -b feat/outside ../outside`, `main/nested/`, and non-git `unfiled/`. Registered `QA1213-main`, then `QA1213-outside`, and later `QA1213-nested`. All live agents used the owned `qa1213` role and single-slot `QA1213-crew`, with bounded prompts. The plan was recorded before execution.

## Result matrix

Evidence filenames below are relative to the evidence root's `evidence/` directory. JSON command records contain exact arguments, exit status, stdout and stderr.

| Check | Result | Observed behavior and evidence |
| --- | --- | --- |
| m1-1 — starts without worktrees | Passed | Ordinary chat, native launcher terminal and one-slot crew mission ran in `main/`. Chat and mission terminal output showed that cwd. Following the controlled app/daemon restart, both agents retained their conversation keys and the shell restarted in the same cwd; native input worked in all three. `ordinary-chat-result.png`, `ordinary-terminal-pwd.png`, `ordinary-mission.png`, `controlled-chat-after-restart.png`, `controlled-terminal-after-restart.png`, `controlled-mission-after-restart.png`, `controlled-restart-*.json`, and `processes-controlled.txt`. The resumed mission ran `pwd` and replied `QA1213-RESUMED`. |
| m1-2 — git-aware membership | Passed | The first chat in `outside/` was stored under `QA1213-main` and appeared there in Projects and Active. Registering `outside/` as its own project made the next chat belong to `QA1213-outside`; the earlier chat kept its stored main-project membership. `outside-chat.json`, `outside-own-project-chat.json`, `outside-projects.png`, `projects-precedence.png`, `active-main-managed-external.png`, and `active-external-own-unfiled.png`. |
| m1-3 — CLI create and start | Passed | Chat `--worktree feat/x` created `main/.worktrees/feat-x`; mission `--worktree feat/y` created `main/.worktrees/feat-y`. Repeating each start succeeded and reused its checkout. Sessions and mission members belonged to the main fixture project. Native chat and mission terminals printed the managed cwd. `managed-chat.json`, `managed-chat-repeat.json`, `managed-mission.json`, `managed-mission-repeat.json`, `managed-chat-pwd.png`, `managed-mission.png`, `worktree-list.json`, and `state-*.json`. |
| m1-4 — CLI removal | Passed | Removing `feat/x` exited 1 with its unarchived occupant identified. After archiving both chats using that checkout, removal exited 0, the folder was absent, and `refs/heads/feat/x` survived. Removing `outside/` exited 1 with the managed-location restriction. `remove-occupied.json`, `remove-archived.json`, `remove-external.json`, `branch-preserved.txt`, and `git-worktrees-final.txt`. |
| WS-CREATE-13 — selected project/folder behavior | Passed | Project selection supplied its root; Change selected a nested folder and an outside worktree. On mission goal setup, choosing `main/nested/` after registering its project selected the deepest project, `QA1213-nested`. No project preserved the chosen folder in both launcher and mission setup. `launcher-nested-role.png`, `launcher-outside-project.png`, `launcher-no-project.png`, `mission-goal-project-folder.png`, `mission-goal-nested-project.png`, and `mission-goal-no-project-preserves-folder.png`. |
| WS-CREATE-17 — selected project-locked/Recents starts | Passed | Main project's + opened its selected dashed row with the main project picker disabled; changing its folder to `main/nested/` and starting the bounded role filed the chat under main. Cmd-N opened an editable picker; explicitly choosing No project and starting with cwd `main/` created a Recents chat with null project ID. `launcher-project-locked.png`, `launcher-nested-role.png`, `launcher-unfiled-result.png`, `sessions-all.json`, and `sessions-before-cleanup.json`. |
| WS-ACTIVE-03 — selected location rows | Passed | In the existing dark theme, root rows showed `QA1213-main`, managed rows showed the project plus `feat-x`/`feat-y`, external rows showed the main project plus external location, the worktree's own project showed its own name, and unfiled rows showed their compact cwd. `active-main-managed-external.png`, `active-external-own-unfiled.png`, and `active-after-restart.png`. |
| WS-ACTIVE-04 — selected grouping/persistence | Passed | Owned chats and missions grouped in Agents, the owned shell in Terminals; switching Projects/Active preserved project membership. Active persisted across the controlled restart. Archiving owned chats/missions and stopping the terminal removed their rows. `active-before-restart.png`, `active-after-restart.png`, `projects-precedence.png`, `cleanup-owned-projects-empty.png`, and `cleanup-*.json`. |
| Native Windows | Blocked — recorded gap | No native Windows machine was used. Windows CI is separate automated evidence, not a substitute for native UI validation. The m1 brief explicitly allows this gap. |

The selected workspace portions exercise membership consumers. This run does not claim the full workspace rows' unrelated dragging, renaming, zoom/theme permutations, keyboard customization, split-neighbor behavior or complete lifecycle matrices. The m2 checkout control, archive/restore UI and other new screens are outside m1.

## Findings and retained attempts

No product failure was established in the selected macOS checks.

1. The first terminal opened at an oh-my-zsh update prompt. It consumed the first `p` of `pwd`, leaving `wd`, which failed; the following marker printed. Preserved `ordinary-terminal.png`; reran `pwd` after the prompt cleared (`ordinary-terminal-pwd.png`). After the controlled restart, explicitly declined the shell update before entering `pwd`; no global shell setting was changed.
2. The first restart attempt used native Cmd-Q, but its wrapper process survived. Launching another copy produced `window connection closed or replaced` during terminal selection. This was harness interference: process inventory proved two run-owned wrapper PIDs. Stopped both, stopped the development daemon, launched one wrapper, and verified one wrapper/daemon pair before repeating the restart checks. `restart-interference.json` and `processes-controlled.txt` retain the diagnosis; only the controlled rerun is used for the final restart verdict. The interrupted in-flight mission command from teardown remains in its conversation; a fresh post-restart prompt completed successfully.
3. Discovery attempts to call `tool_list` and `node_list` were refused (`tool not found`); the former was invoked before using the environment-clearing helper and is excluded from product evidence. The supported public session list and native UI supplied the needed evidence instead. The terminal archive CLI correctly refused with “terminals close rather than archive”; stopping the owned shell closed its row.

Automated gates are owned by coder. Reviewer's clean handoff reported that coder passed the required crate tests, workspace Clippy including macOS updater, formatting, translation and diff checks, and 20/20 worktree stability repetitions. QA did not rerun those suites and does not present that report as live evidence.

## Cleanup

Stopped and archived the three owned test missions and eight owned agent chats; stopped the one owned terminal, which closed its row. The CLI ledger includes every created ID. Stopped all owned wrapper copies and the normal development daemon, unregistered the wrapper from LaunchServices, and verified no process running a fixture agent, the wrapper or that daemon survived. An unrelated pre-existing private-HOME development daemon was left alone. Released `script/qa-lock` after cleanup.

Captured Codex configuration before launch. Removed only the two trust entries created for this run (`main/` and `unfiled/`); parsed comparison proves all other project entries and non-project settings stayed unchanged during cleanup. `trust-cleanup.json` and the private `trust-cleanup.diff` retain that evidence. No authentication material is included in this record.

Retained evidence, conversation history, fixture projects, the owned role/crew, surviving fixture worktrees and all branches. Only the `feat/x` checkout was removed, as explicitly required by m1-4; its branch survives. No project, role, crew or branch was deleted.
