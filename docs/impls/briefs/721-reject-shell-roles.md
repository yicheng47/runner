# 721 — Roles cannot use the shell runtime

Fix [P1 #721](https://github.com/yicheng47/runner/issues/721). Jason asked on 2026-09-26 for a claude pair crew mission that ends in an open PR, not a merge.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-721-reject-shell-roles`, on branch `fix/721-reject-shell-roles`. The mission's directory is this worktree. Its tip is this brief, on top of `main` `5d9242b`. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. If main moves and you need it, rebase onto `origin/main`; never merge main into the branch.

## The bug

A role can be stored with `runtime = "shell"`. A role is an agent runtime plus a brief, so a shell role has no model, effort, permission modes or system prompt and cannot take part in a mission. `Runtime::Shell` exists for plain-terminal sessions, which build their role in memory (`ops/session.rs::session_start_shell_in` via `runtime_direct_role`) and never store one. Slot overrides already refuse it: `ops/slot.rs::validate_runtime_override` accepts only runtimes that have a `router::runtime::runtime_definition`. Role create and update do not, and since #648 `runner role create --runtime shell` maps `shell` to `/bin/zsh` (`cmd.exe` on Windows).

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions.
- Issue #721.
- `crates/runner-backend/src/ops/role.rs` (`create`, `update`), `ops/slot.rs` (`validate_runtime_override`), `router/runtime.rs` (`runtime_definition`, `runtime_definitions`).
- `crates/runner-cli/src/command.rs` (`runtime_command`) and the `role create` / `role update` argument help.
- `crates/runner-backend/src/mcp/tools/role.rs`, the socket tools, which call `ops::role::create` / `update`.
- `crates/runner-app/src/surfaces/roles/` (`create.rs`, `edit.rs`, `logic.rs`), the app's role forms.

## The rule

1. **One check, shared.** Role create, role update and slot overrides use the same rule and the same error: `unknown runtime '<name>' — valid runtimes: codex, claude-code, …` (the list comes from `runtime_definitions()`, as today). Factor it out of `validate_runtime_override` into one place both call; do not write a second copy.
2. **Create** rejects any runtime without a runtime definition, `shell` included.
3. **Update** rejects a patch whose `runtime` is set to a runtime without a definition, whether or not it changed. A patch that omits `runtime` passes, so an existing shell role can still be renamed or deleted from the CLI. In the app, the edit form always sends the runtime, so an existing shell role saves only once an agent runtime is picked; check that the backend error shows in the form rather than failing silently, and that the runtime select for such a role offers the agent runtimes to switch to.
4. **CLI.** `runtime_command` drops `shell`; its usage error lists only the agent runtimes. Check `role update --runtime` too, and any help text, `help agents` output or embedded skill text that lists `shell` as a role runtime.
5. **No migration.** Existing shell roles stay readable, listable and deletable. Plain-terminal sessions (New terminal, splits, `runner` shell starts) keep working unchanged.

## Deliverable

1. The shared check and its use in `ops::role::create` and `update`.
2. The CLI change.
3. **Test fixtures.** Backend tests that store shell roles to avoid launching an agent must move to a real runtime: `ops/role.rs:543`, `:752`, `:1184`, `:1281`; `ops/slot.rs:549`; `ops/mission.rs:1678`. Grep the whole workspace for others (`Runtime::Shell` and `"shell"` in role inputs, including `session/manager/tests/`, `db/tests.rs` and `runner-app` tests). Where a test needs a role that spawns no agent, use whatever the existing tests use for that (a real runtime with a harmless command such as `/bin/sh` or `true`); do not weaken the check for tests. A test that inserts a shell row straight through `repo::role` to model legacy data is fine and should stay.
4. **New tests.** Backend: create with `Shell` fails with the shared error; update setting `Shell` fails; update omitting the runtime on an existing shell row (inserted through `repo::role`) succeeds and keeps it `shell`; the slot override error text is unchanged. CLI: `role create --runtime shell` is a usage error that does not list `shell`. Gate any test import or helper used only by `cfg(unix)` tests with `cfg(unix)`, since Windows clippy fails on unused ones.

Out of scope: deleting or migrating existing shell roles, changing plain-terminal sessions, and any role list or role page UI change beyond surfacing the error.

## Validation

Run each of these and report its exit code:

- `cargo test --locked -p runner-backend --profile ci --no-fail-fast`
- `cargo test --locked -p runner-app --profile ci --no-fail-fast`
- `cargo test --locked -p runner-cli --profile ci`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- `cargo fmt --all --check`
- `git diff --check`

Log each to a file and read `$?` from the command itself; never pipe a gate through `tail` or `grep`.

Crews never run the dev app. Do not start, stop, restart or type into Jason's Runner apps, chats or missions, do not run `runner role create` against a live app, and never open Jason's real Runner database (`~/Library/Application Support/com.wycstudios.runner*`): every test uses a temp database. Jason smoke-tests the fix himself.

## Crew handoff and authorization

The coder owns the implementation, tests and fixes. The reviewer waits for an explicit Runner handoff, then reviews the whole working-tree diff against the issue and this brief, with must-fix findings first and file:line pointers. It checks in particular:

- that create, update and slot overrides share one check and one error text;
- that no remaining create or update path can store a shell role (CLI, socket tools, app forms);
- that plain-terminal sessions and existing shell rows are untouched, and no test was made to pass by loosening the check.

Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES`. No extra agents, crews or subagents.

After the clean review, and only then, Jason authorizes:

- **Commit** in focused commits on this branch: imperative subject, scopes such as `session`, `cli` or `ui`, no co-author trailers. Keep the brief commit.
- **Push** with `git push -u origin fix/721-reject-shell-roles`.
- **Open the PR** with `gh pr create --base main`. The body carries `Closes #721`, a summary, test evidence, a manual check for Jason (`runner role create smoke-shell --runtime shell` is refused with the runtime list; the existing `@smoke648-a` still lists, and saving it in the app requires picking an agent runtime; New terminal still opens a shell), and no agent session links.
- **Watch CI** with `gh pr checks <n> --watch` until nothing is pending, on both macOS and Windows. Fix any failure on the branch, have the reviewer check the fix, and push again.

**Do not merge**, delete the branch or worktree, or cut a nightly or release.

The final handoff goes to everyone through Runner. It carries the PR URL and CI result, changed files, checks with exit codes, what is untested or unverified, and the reviewer's verdict. Then both slots stand by.
