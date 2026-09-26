# 718 — Recents starts stay out of projects

Fix [P1 #718](https://github.com/yicheng47/runner/issues/718). Jason asked on 2026-09-24 for a claude pair crew mission that ends in an open PR, not a merge.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-718-recents-project-inference`, on branch `fix/718-recents-project-inference`. The mission's directory is this worktree. Its tip is this brief, on top of `main` `041cc23`. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. Other missions may be building in their own worktrees; ignore them. If main moves and you need it, rebase onto `origin/main`; never merge main into the branch.

## The bug

After a project is created from a folder, New chat or New terminal in the sidebar's Recents section puts the session under that project's node whenever its working directory is the project's folder or inside it. The Recents menu passes no project (`sidebar/menus.rs:489`, handled at `:204` and `:222`), and every backend start path reads "no project" as "infer it from the directory": `ops/project.rs::resolve_cwd` falls back to `repo/project.rs::find_for_path`, called from `ops/session.rs` (`resolve_direct_start`, the runtime-direct start, `session_start_shell`) and `ops/mission.rs`. The working directory comes from the default working directory or the role's own (`start_chat.rs::terminal_start_location`, `terminal_working_dir`).

The inference came from #682 (closes #680, commit `078a8e3`) for callers that give only a directory: `runner mission start` and `runner chat` run from a worktree land under its project. That stays. The app's Recents path was collateral.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions.
- Issue #718 and PR #682 (`gh pr view 682`), whose tests pin the inference that must keep working.
- `crates/runner-backend/src/ops/project.rs`, `repo/project.rs`, `ops/session.rs`, `ops/mission.rs`, `ops/node.rs`, and the socket tools in `crates/runner-backend/src/mcp/tools/` that start sessions and missions.
- `crates/runner-app/src/surfaces/sidebar/menus.rs`, `surfaces/start_chat.rs`, and every other app path that starts a chat, terminal, split or mission.

## The rule

A start from inside the app follows the place it was started from, never the directory. From a project's node or menu, it belongs to that project; from Recents, it belongs to no project, whatever its working directory. A caller that passes only a directory (the CLI, the socket tools) keeps inferring the project exactly as today, and an explicit project keeps winning.

## Deliverable

1. **Inventory first.** List every app code path that starts a session or mission without a project today: at least Recents New chat, New terminal and New mission, ⌘N, the command palette, and splits and new terminals that follow a pane's live directory (#575). For each, say where it is started from and which project it should get under the rule; a split or new terminal from a pane takes that pane's project. Post the list to the reviewer through Runner before changing code. The reviewer checks it is complete, then you implement.
2. **Make "no project" explicit** in the backend start paths, so the app can say "infer", "no project" or "this project" and the resolver honors it. Pick the smallest clear shape (an enum such as infer / root / project is one option) and keep one place that decides membership. The CLI and socket tools keep their current behavior; their arguments and help text do not change.
3. **Route every app path from the inventory through it** with the project the rule gives. No app start relies on directory inference.
4. **Tests.** Backend: an explicit "no project" start whose directory is inside a project stays at the root, for a direct chat, a runtime chat, a shell and a mission, and its sidebar node is a root node. The #682 inference tests keep passing unchanged. App: the Recents menu actions start with "no project", and a project's menu actions with that project. Gate any test import or helper used only by `cfg(unix)` tests with `cfg(unix)`, since Windows clippy fails on unused ones.

Out of scope: moving existing sessions or missions between projects, changing how the CLI or socket tools infer, a setting to turn inference off, and any sidebar layout change.

## Validation

Run each of these and report its exit code:

- `cargo test --locked -p runner-backend --profile ci --no-fail-fast`
- `cargo test --locked -p runner-app --profile ci --no-fail-fast`
- `cargo test --locked -p runner-cli --profile ci`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- `cargo fmt --all --check`
- `git diff --check`

Log each to a file and read `$?` from the command itself; never pipe a gate through `tail` or `grep`.

Crews never run the dev app. Do not start, stop, restart or type into Jason's Runner apps, chats or missions, and never open Jason's real Runner database (`~/Library/Application Support/com.wycstudios.runner*`): every test uses a temp database. Jason smoke-tests the fix himself.

## Crew handoff and authorization

The coder owns the inventory, implementation, tests and fixes. The reviewer waits for an explicit Runner handoff: first the inventory, then the implementation. It reviews the whole working-tree diff against the issue and this brief, with must-fix findings first and file:line pointers. It checks in particular:

- that the inventory covers every app start path, and each now gets the project the rule gives;
- that the CLI and socket tools still infer from a directory, with #682's tests untouched;
- that membership is decided in one place, and an explicit "no project" can never be overridden by the directory.

Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES`. No extra agents, crews or subagents.

After the clean review, and only then, Jason authorizes:

- **Commit** in focused commits on this branch: imperative subject, scopes such as `session`, `ui` or `cli`, no co-author trailers. Keep the brief commit.
- **Push** with `git push -u origin fix/718-recents-project-inference`.
- **Open the PR** with `gh pr create --base main`. The body carries `Closes #718`, a summary, the inventory table (path, started from, project after the fix), test evidence, a manual check for Jason (create a project from a folder, set the default working directory inside it, start a chat and a terminal from Recents and from the project's node, and run `runner chat` from inside the folder), and no agent session links.
- **Watch CI** with `gh pr checks <n> --watch` until nothing is pending, on both macOS and Windows. Fix any failure on the branch, have the reviewer check the fix, and push again.

**Do not merge**, delete the branch or worktree, or cut a nightly or release.

The final handoff goes to everyone through Runner. It carries the PR URL and CI result, changed files, checks with exit codes, what is untested or unverified, and the reviewer's verdict. Then both slots stand by.
