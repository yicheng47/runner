# 787 — Antigravity mission tools run in Runner's hooks folder

Fix [#787](https://github.com/yicheng47/runner/issues/787). Jason requested this mission on 2026-10-04 as the next 0.12 bug. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-787-antigravity-mission-cwd`, on the existing branch `fix/787-antigravity-mission-cwd`, created from `origin/main` at `c7a9f6f2`. The root checkout stays on main. Do not create another branch or worktree, and do not share a Cargo target directory. Other worktrees under `.worktrees/` belong to other work; treat them as another machine's checkout.

## What is known (not yet reproduced under control)

1. **The failure.** On 2026-10-02 a fresh sole-lead Antigravity mission ran its marker command in `~/Library/Application Support/com.wycstudios.runner-dev/antigravity-hooks` instead of the mission directory. `lsof` showed the agy process cwd was the requested directory, so the process cwd is right and the tool workdir is not. An earlier Antigravity worker mission wrote its marker in the right place.
2. **Runner adds its hooks folder as a workspace.** `antigravity_status_args` (`runner-backend/src/runtimes/antigravity/mod.rs`, about line 22) passes `--add-dir <app data>/antigravity-hooks`, because agy loads `<dir>/.agents/hooks.json` only from a workspace directory. A mission spawn also adds the mission folder through `mission_dir_args` (about line 213). The first turn goes in as `-i <prompt>` (`first_turn_argv`, about line 193).
3. **Added directories are visible to the model.** The #644 probes (`docs/features/archive/644-antigravity-runtime.md`, Research, about lines 54–56) found that an added directory appears in `workspacePaths` and in the agent's workspace list, and that the model wandered into a probe folder. agy's `run_command` appears to take its working directory from the model, so the model may pick a workspace other than the process cwd. This is the leading suspicion, not a finding.
4. **Hooks are not optional.** Removing `--add-dir` alone loses Working, Idle and completion status for every Antigravity session.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions; `docs/tests/full-smoke-test.md` (QA's procedure); issue #787.
- `runner-backend/src/runtimes/antigravity/` (`mod.rs`, `agy_status.rs` for `hooks_dir`, `hooks_available` and the hooks file, `agy_trust.rs`), `runtimes/helpers.rs` (`add_dir`, `prefixed_first_turn`) and the spawn path in `session/manager/spawn.rs` that applies runtime args.
- `docs/features/archive/644-antigravity-runtime.md` (Research and Decisions 5–6), `docs/impls/briefs/747-antigravity-followups.md` (probe boundaries), `docs/arch/runtime-integration.md`.
- `agy --help` (version 1.2.16 on this machine): besides `--add-dir` it has `--project` and `--new-project`.

## Phases

1. **Mechanism (coder, probes only, no code changes).** With bounded `agy` PTY probes in disposable `/private/tmp` directories, find what decides the tool workdir. Use a stand-in hooks folder with its own `.agents/hooks.json`, never Runner's real one. Vary one thing at a time: no `--add-dir`; the hooks folder added; the hooks folder plus a second added folder, as a mission lead gets; the prompt passed with `-i` versus typed after start; and the order of the `--add-dir` arguments. Each probe asks the agent to run one command that prints its working directory. Record `workspacePaths` from a hook payload, the `run_command` arguments from agy's log or transcript, and the directory the command ran in. Run each variant at least three times, because the failure looked intermittent. Then test candidate fixes the same way, for example adding the session cwd as the first workspace, a hooks location the model does not treat as a workspace, or a rules file in the hooks folder. Post the matrix and the cause, with evidence, to Jason through Runner before changing code.
2. **Baseline (QA, in parallel with phase 1).** On the untouched branch, follow the issue's steps in the development app: a sole-lead Antigravity mission and a direct Antigravity chat, each told to append one unique marker line to `$RUNNER_HANDLE.txt` in its working directory and reply ACK. Repeat each three times. Record where each marker landed, the session cwd, and whether status reached Working and then completed. Post the matrix to the coder.
3. **Fix (coder).** Fix the confirmed cause with the smallest per-launch change. Keep hook loading and status working, keep the user's own added directories, and leave the other runtimes' argv unchanged. If the only working fix needs a change to Jason's global agy configuration (for example `~/.gemini/config/hooks.json`), stop and ask Jason through Runner first.
4. **Tests (coder).** Add focused regressions for the new Antigravity argv in direct chats, mission leads, mission workers and resumes. The existing spawn goldens (`session/manager/tests/expectations/`) will change only in Antigravity rows: regenerate them deliberately and list the changed rows in the PR. Do not add a new golden format or file.
5. **Verification (QA).** On the reviewed branch, rerun every baseline row, plus a direct chat whose role passes its own `--add-dir` to a second scratch folder: the marker must land in the working directory, the user's folder must still be listed as a workspace, and status must still go Working then completed. Write `docs/tests/787-antigravity-mission-cwd.md` with dated baseline and fix matrices, the agy version, candidate SHAs and evidence locations, and nothing private. Native Windows is pending for Jason.

## Live-test authorization

The coder may run the bounded `agy` probes in phase 1 with the existing sign-in, inexpensive models, low effort, tiny prompts and disposable directories under `/private/tmp`. QA may build this worktree and run its development app with `make run`, using development data only, and drive it with `runner-dev` by its absolute path. QA may create one test crew and the roles it needs, named with a `qa787-` prefix, and run bounded test missions and chats on them. If a development app is already running when QA starts, ask Jason through Runner instead of quitting it. Do not touch pre-existing chats, missions, crews, roles or agy conversations, the installed Runner app, authentication, global agent configuration, global hooks or permission settings, and do not open Runner's SQLite database. Afterwards, stop and archive QA's test missions and chats, delete the `qa787-` crew and roles, quit the dev app QA started, and keep the evidence directory. The reviewer does not run the app or agy. agy probe processes are test subjects and must not delegate work. No other agents, crews or subagents.

## Review, verification and authorization

The reviewer waits for an explicit Runner handoff. It then reviews the full branch diff against #787, the phase 1 evidence and QA's baseline, posting must-fix findings first with file:line pointers. Its focus: hooks and status still load in every Antigravity launch shape; no other runtime's argv changes; a user's own `--add-dir` survives; resume still finds the conversation. Iterate until the reviewer posts `NO REMAINING MUST-FIX ISSUES`, then run QA's phase 5.

Run these checks and record each exact command with its exit code:
- `cargo test --locked -p runner-backend --profile ci`
- workspace Clippy: `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- the macOS updater Clippy, with `--features updater`
- `cargo fmt --all --check`
- `git diff --check`

Gate imports and helpers used only by `cfg(unix)` tests with `#[cfg(unix)]`; Windows CI keeps failing on them.

After clean review and QA, Jason authorizes the following:
- Squash all work on this branch, this brief included, into one commit on top of current `origin/main`, with a subject that names the fix.
- Push `fix/787-antigravity-mission-cwd` and open a PR against main. Use `Fixes #787` only if QA reproduced the wrong directory and the fix cleared it live. Otherwise use `Refs #787` and say what is left.
- If main has moved, rebase; never merge main into the branch.
- Amend review and CI fixes into the same commit and push with `git push --force-with-lease`.
- Drive CI green on macOS and Windows.
- Do not merge, delete the branch or worktree, or cut a nightly or release.

Final Runner handoff: the PR URL, the mechanism with evidence, what changed, tests and exit codes, QA's matrices, the CI result, the reviewer's verdict, and what Jason should still check (native Windows). Then stand by.
