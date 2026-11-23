# #1432 — Move large inline test modules into sibling test files

Jason approved a neon duo on 2026-11-23: `coder` (`impl-agy` on Antigravity `gemini-4-neon`, xhigh) and `reviewer` (codex `gpt-6.2-astra`, xhigh). Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/refactor-1432-inline-test-modules` on `refactor/1432-inline-test-modules` (base `origin/main` `1c891b45`, its own `target/`). Other worktrees under `.worktrees/` belong to other work; never inspect or edit them.

## Problem and outcome

Issue #1432, a sub-issue of the #1248 cleanup tracker. Ten production files carry inline test modules of 1,000+ lines, so the production code can't be read on its own. Outcome: each of these files keeps only its production code and declares its tests as an out-of-line module, with test bodies unchanged and the same tests running. The unreferenced `pane_status_indicator` is deleted. No behavior change.

| File | Production lines | Inline test lines |
| --- | --- | --- |
| `crates/runner-app/src/app_store.rs` | 1,796 | 2,317 |
| `crates/runner-daemon/src/runtimes/antigravity/usage.rs` | 1,308 | 1,829 |
| `crates/runner-app/src/surfaces/start_chat/setup.rs` | 1,609 | 1,549 |
| `crates/runner-daemon/src/daemon/server.rs` | 1,456 | 1,374 |
| `crates/runner-app/src/ui/agent_status.rs` | 649 | 1,332 |
| `crates/runner-daemon/src/ops/runtime.rs` | 698 | 1,294 |
| `crates/runner-daemon/src/ops/mcp.rs` | 927 | 1,197 |
| `crates/runner-app/src/app_settings.rs` | 823 | 1,042 |
| `crates/runner-daemon/src/usage.rs` | 680 | 1,011 |
| `crates/runner-app/src/surfaces/chat/close.rs` | 667 | 1,010 |

## Read first

- Issue #1432.
- `crates/runner-daemon/src/files/changes.rs` (end of file) and `changes_tests.rs`: the `#[cfg(test)] #[path = "changes_tests.rs"] mod tests;` layout already used in the repo.
- Each target file's `#[cfg(test)]` items. Several have more than one inline module or `#[cfg(test)]` helpers outside the test module; only the large test module is in scope.

## Deliverables

1. **Move each large inline test module out of line.** Replace `#[cfg(test)] mod tests { … }` with a declaration, and move the body verbatim into the new file:
   - `app_store.rs` already has an `app_store/` directory: use `#[cfg(test)] mod tests;` with `app_store/tests.rs`.
   - Every other file has no module directory (and `runtimes/antigravity/tests.rs` already belongs to `antigravity/mod.rs`): use `#[cfg(test)] #[path = "<stem>_tests.rs"] mod tests;` with a sibling `<stem>_tests.rs`, for example `ops/runtime_tests.rs`.
   - Keep the module name, attributes and contents, including `use super::*;`, since the module stays a child of the same parent. Move `#[cfg(test)]` imports or helpers at file scope into the test file only when the tests are their sole user; otherwise leave them.
   - Small additional inline modules (under ~200 lines) stay where they are.
2. **Delete `pane_status_indicator`** (`crates/runner-app/src/ui/agent_status.rs`), which has no references, along with any import or helper that only it used.
3. **Commits.** One commit per moved file, each building and passing on its own, plus the deletion; fold this brief into the first commit. A test-file diff that is a pure move shows up cleanly under `git show --color-moved=dimmed-zebra`.

Non-goals: changing any test's content, renaming tests, splitting production code, moving other files' tests, or editing docs beyond this brief.

## Validation

- **Same tests:** before the first move, record the test list for `runner-app` and `runner-daemon` (`cargo test -p <crate> -- --list 2>/dev/null | grep ': test$' | sort`). After the last commit, the lists must be identical. Put the counts in the PR description.
- **Automated:** `cargo test -p runner-app`, `cargo test -p runner-daemon`, `make clippy`, `cargo fmt --all -- --check` and `git diff --check`. No new tests are added, so no stress loop is needed.
- **Live checks:** none expected: the change moves test code and deletes an unreferenced function. `@reviewer` confirms this from the diff in the clean review.
- **Review:** `@reviewer` checks that every moved block is a pure move (`--color-moved`), that the test lists match, and that nothing outside the ten files and the deletion changed.

## Authorization

Approved crew: the neon duo above. Push and open a PR against `main` with `Fixes #1432` and the labels `no-behavior-change` and `risk-low`; if the core-review workflow adds `core-review` for `daemon/server.rs`, leave it. `@reviewer` adds `reviewed` on the final head. Do not merge.
