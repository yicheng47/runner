# 619 — Referential integrity in application code

Implement [P2 #619](https://github.com/yicheng47/runner/issues/619), milestone 0.12. Jason asked for a claude pair crew mission on 2026-09-23 that ends in an open PR, not a merge; he reviews it the next day.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-619-delete-integrity`, on branch `feat/619-delete-integrity`. The mission's directory is this worktree. Its tip is this brief, on top of `main` `19c1238`. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. Two other missions are building in their own worktrees; ignore them. If main moves and you need it, rebase onto `origin/main`; never merge main into the branch.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions.
- **The issue body** is the spec: Motivation, Scope, Out of scope and Verification are binding. It was filed on 2026-09-16, and the code has moved since then. Crews and roles now refuse deletion in some states, and the tree lives in `nodes`. Today's code and schema win over the issue's examples. The issue's intent does not change: every delete owns its consequences in code, and behavior stays exactly as it is.
- `crates/runner-backend/migrations/` (`REFERENCES` appears in 0001, 0007, 0009, 0011, 0014 and 0021) and `crates/runner-backend/src/db/mod.rs`, which opens connections with `PRAGMA foreign_keys = ON`.
- `crates/runner-backend/src/repo/` (`role.rs`, `crew.rs`, `slot.rs`, `mission.rs`, `session.rs`, `session_attention.rs`, `project.rs`, `node.rs`) and `crates/runner-backend/src/ops/` (`role.rs`, `crew.rs`, `slot.rs`, `mission.rs`, `session.rs`, `project.rs`, `node.rs`, `window.rs`): every delete path and its existing tests.
- `docs/features/562-mission-spawn.md`, for context only. #562 will change who owns slots and missions. Do not build for it; keep each entity's delete in one place so #562 can change it there.

## Deliverable

1. **Inventory first.** Build the foreign-key list from a freshly migrated database (`pragma_foreign_key_list` for every table), not from reading the SQL. Build the delete-path list from the code: every `DELETE` statement and every function that removes a row that others reference. Post both lists to the reviewer through Runner before changing code, with each foreign key's action (CASCADE, SET NULL, RESTRICT, or none) and the delete path that relies on it. The reviewer checks that the inventory is complete, then you implement.
2. **One owning function per entity** (role, crew, slot, mission, session, project, node, and any others the inventory finds). Inside one transaction, it removes or annuls every dependent explicitly, in the order the schema implies today, and refuses the RESTRICT cases before writing, with the error the UI already gets. Every caller goes through it. No path depends on a cascade or SET NULL.
3. **Live sessions first.** Where role, crew or mission deletion kills live sessions today, that ordering stays: the kill happens before the transaction. If it cannot move into the repo layer, because the repo has no session manager, keep it in the one ops function that calls the owning delete, and say so in the handoff.
4. **The schema keeps its constraints, and the pragma stays on.** Do not add a migration, and do not change the constraints.
5. **Tests, per delete path**, on a connection with `PRAGMA foreign_keys = OFF`: dependents gone or nulled, unrelated rows untouched, RESTRICT cases refused and nothing written. Add one test on a real database, with the pragma on, that runs every delete and asserts `PRAGMA foreign_key_check` is empty. Existing tests keep passing unchanged. If one relied on a cascade, say which and why it still passes.
6. **Docs, same diff**: a short "Deletion" section in `docs/arch/arch.md` listing each entity's delete, what it removes or nulls, and what it refuses. It replaces the schema as the place a reader learns the rules.

Out of scope: any user-visible change (what can be deleted, error text, confirmation dialogs), soft deletes, removing or changing constraints, migrations, and #562's model.

## Validation

Run each of these and report its exit code:

- `cargo test --locked -p runner-backend --profile ci --no-fail-fast`
- `cargo test --locked -p runner-app --profile ci --no-fail-fast`
- `cargo test --locked -p runner-cli --profile ci`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- `cargo fmt --all --check`
- `git diff --check`

Log each to a file and read `$?` from the command itself; never pipe a gate through `tail` or `grep`. Report the backend test count before and after: it should grow by the new delete tests only.

Crews never run the dev app. Do not start, stop, restart or type into Jason's Runner apps, chats or missions, and never open Jason's real Runner database (`~/Library/Application Support/com.wycstudios.runner*`): every test and every inventory query uses a temp database.

## Crew handoff and authorization

The coder owns the inventory, implementation, tests and fixes. The reviewer waits for an explicit Runner handoff: first the inventory, then the implementation. It reviews the whole working-tree diff against the issue and this brief, with must-fix findings first and file:line pointers. It checks in particular:

- that no delete path still depends on a CASCADE or SET NULL, and that the pragma-off tests would fail if a dependent step were removed;
- that each owning delete is one transaction, and a refused RESTRICT case writes nothing;
- that no user-visible behavior changed, including the existing refusals for crews and roles;
- that every caller, in ops, the app, the CLI and the socket tools, now goes through the owning function.

Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES`. No extra agents, crews or subagents.

After the clean review, and only then, Jason authorizes:

- **Commit** in focused commits on this branch: imperative subject, scope `db`, `repo`, `ops` or `docs`, no co-author trailers. Keep the brief commit.
- **Push** with `git push -u origin feat/619-delete-integrity`.
- **Open the PR** with `gh pr create --base main`. The body carries `Closes #619`, a summary, the inventory table, test evidence with the before and after test counts, any issue example that no longer matched the code, and no agent session links.
- **Watch CI** with `gh pr checks <n> --watch` until nothing is pending. Fix any failure on the branch, have the reviewer check the fix, and push again.

**Do not merge**, delete the branch or worktree, or cut a nightly or release.

The final handoff goes to everyone through Runner. It carries the PR URL and CI result, changed files, checks with exit codes, what is untested or unverified, and the reviewer's verdict. Then both slots stand by.
