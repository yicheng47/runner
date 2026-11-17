# #1213 m1 — Git worktrees in runnerd and the CLI

## Starting point

- Issue: [#1213](https://github.com/yicheng47/runner/issues/1213), milestone 0.20. This is m1 of two pull requests to `main`; m2 (the app) follows in its own mission.
- Branch: `feat/1213-worktrees`, already pushed. Draft PR [#1350](https://github.com/yicheng47/runner/pull/1350) holds the spec commits; finish m1 on this PR rather than opening another.
- Worktree: `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1213-worktrees`, with its own Cargo `target/`. Base: `origin/main` at `d768d233`.
- Design: `runner/desktop/specs/1213-worktrees.pen` in `yeelinks_design`, revision `685b8c9`, board `A0AUx`. m1 has no screens; the design matters only for the strings m2 will show, which m1's requests must be able to produce.
- Stay inside this worktree, run heavy workspace suites one at a time, and use a `/tmp/1213-m1-*` prefix for every scratch folder and fixture.

## Problem and outcome

Runner's crews make worktrees by hand for every mission, and nothing in Runner can create one. m1 adds the `runnerd` side: a worktree service, an optional `worktree: { branch, base? }` on chat, terminal and mission starts resolved as one create-and-start request, git-aware project membership, removal and restore rules, and the `runner worktree` CLI with `--worktree` on `chat start` and `mission start`. The spec [`docs/features/1213-worktrees.md`](../../features/1213-worktrees.md) is the contract; where this brief and the spec differ, the spec wins.

Runner stores nothing about worktrees (decision 9): **no migration, no table, no column**. Which checkout a session is in and its branch are derived from its stored cwd and git. Removal is offered for linked worktrees under the main checkout's `.worktrees/`. Restore finds the branch from git's `prunable` entry or the flattened folder name. runnerd's automatic resume at startup never recreates a missing worktree (decision 10).

## Read first

- The spec: Principle, Model (runnerd), Cleanup at archive, Restore and resume after removal, CLI, Remote, Non-goals, Risks, Verification, Delivery, and decisions 2, 3, 6, 9 and 10.
- Membership: `crates/runner-daemon/src/ops/project.rs` (`resolve_scope`, `resolve_cwd`), `crates/runner-daemon/src/repo/project.rs` (`find_for_path`).
- Git: `crates/runner-daemon/src/files/git.rs` (the `git worktree list --porcelain` parsing), `crates/runner-daemon/src/files/mod.rs` (`worktrees`), `crates/runner-daemon/src/files/changes.rs` (`changes_branches`, `resolve_base` / `auto_base`, which the refs list and base reuse).
- Starts and resume: `crates/runner-daemon/src/ops/session.rs`, the mission start and add-role paths under `crates/runner-daemon/src/ops/`, `crates/runner-daemon/src/daemon/boot.rs` (resume on launch), `crates/runner-daemon/src/runtimes/claude_code/mod.rs` (why the cwd must be kept), and `seed_project_trust`.
- Protocol: `crates/runner-core/src/protocol/{api.rs,files.rs,changes.rs,tests.rs}`.
- CLI: `crates/runner-cli/src/command/{chat.rs,mission.rs,cli.rs}`.

## Deliverables

1. **Worktree service in `runnerd`:** name check (`git check-ref-format --branch`, existing branch, already checked out), the path `<main checkout>/.worktrees/<branch with "/" replaced by "-">`, the one-time `/.worktrees/` line in the common `info/exclude`, create in the spec's resolution order with stale `prunable` registrations cleared first, creates serialized per repository, base `origin/HEAD` falling back to the main checkout's `HEAD` with no fetch, and the refs list through `changes_branches`.
2. **Create and start:** the optional `worktree` field on chat, terminal and mission starts, idempotent for retries; a failed add starts nothing; a spawn failure after the add keeps the worktree. Every new protocol field and request optional with serde defaults, so today's starts are unchanged.
3. **Git-aware membership:** `ProjectScope::Infer` keeps path containment first and falls back to git for a cwd in a linked worktree whose main checkout is a project's folder. The project is still decided at start and stored.
4. **Removal and restore:** `git worktree remove` without `--force`, only under `.worktrees/`, refused while an unarchived session's cwd is inside, git's reason returned when git refuses, branches never deleted. Explicit restore, resume and add-role recreate a missing worktree at the same path (branch from the `prunable` entry, else the unique flattened-name match, else the Restore-in-main-checkout fallback); the automatic resume in `daemon/boot.rs` leaves such a session stopped. A request that tells m2 a checkout's branch, kind and missing state for a cwd, resolving a missing branch as restore does.
5. **CLI:** `runner worktree create|list|remove` and `--worktree <branch> [--base REF]` on `chat start` and `mission start`, all with `--json`; regenerate the reference with `make guide-cli`.
6. **Docs:** the `AGENTS.md` crew flow (`runner worktree create`, commit the brief, then `runner mission start --worktree` or `--cwd`); the guide pages the spec names for the CLI and project membership ([Drive Runner from agents](../../guide/guides/drive-runner-from-agents.md), [Projects](../../guide/concepts/projects.md)), with their `zh-CN` mirrors; mark the spec's Status as m1 implemented in this PR.

Out of scope: everything m2 owns in the spec's Delivery table (launcher, sidebar, Info, archive dialog, restore UI, the remaining guide pages) and everything in the spec's Non-goals.

## Validation and handoffs

- **Automated:** the spec's Automated list in full, in fixture repositories built with the git CLI. Run `cargo test -p runner-daemon`, `cargo test -p runner-core`, `cargo test -p runner-cli`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` (and `--features updater` on macOS), `cargo fmt --all -- --check`, `git diff --check`, and `python3 script/check-guide-translations.py`. Tests that spawn git or watchers follow the no-race rule in `AGENTS.md`, and the coder runs them 20 times before handoff and states the count.
- **Live checks (`qa`):** after a clean review, the spec's m1-1 to m1-4 on macOS in the dev app (`make run`) with the spec's `/tmp` fixture under the `/tmp/1213-m1-` prefix, plus the selected workspace regression cases. Record them in `docs/tests/runs/1213-worktrees-mac.md` and in the PR description. A native Windows run is a recorded gap.
- **Handoffs:** `coder` sends the unpushed candidate SHA to `reviewer` and iterates until the review says no must-fix issues remain; `reviewer` then hands the reviewed SHA to `qa`. A QA failure goes back to `coder` and through `reviewer` again before `qa` reruns the affected checks.
- **Long commands:** an Antigravity turn that ends while a command still runs can show as idle (#1336). The coder waits for each `cargo` run to finish before ending its turn and reports the result.

## Authorization

Jason authorized the **hydrogen trio** on 2026-11-17 for #1213 m1. Commit, folding this brief and the spec commits into the implementation commits, and push `feat/1213-worktrees` with `git push --force-with-lease`. Update draft PR #1350's description for m1, mark it ready for review, and label it `core-review`, `risk-high`, `reviewed` (after the clean review) and `needs-live-test` (until every selected live check passes). Drive CI green, then stop for Jason to review and merge.
