# 661 — Rename repository mapping contract tests

Issue: https://github.com/yicheng47/runner/issues/661. Milestone 0.11, P3. This is naming-only cleanup of permanent tests, with no product or storage behavior change.

## Worktree and scope

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-661-row-mapping-contract-tests`, on the existing branch `fix/661-row-mapping-contract-tests`, based on `origin/main` at `7d5c88005267d9ed30073b5fb4d4273571964c78`. Read `AGENTS.md` and this brief first. The root checkout and other worktrees contain unrelated live work. Do not change branches, rebase, or create another checkout. Keep this worktree's own target directory; do not set a shared `CARGO_TARGET_DIR`.

In `crates/runner-backend/src/repo/mod.rs`, rename `spike_tests` to `row_mapping_contract_tests`, `SpikeRow` to `MappingRow`, `SPIKE_COLUMNS` to `MAPPING_COLUMNS`, and the in-memory `spike` fixture table to `mapping_contract`, including every SQL reference to that fixture. Rewrite the stale introductory spike wording to describe permanent row-mapping contracts. In `crates/runner-backend/src/repo/serde.rs`, update the `spike-proven (repo::spike_tests)` reference to match.

Preserve every test, assertion, fixture column, serialized value, and storage-format guarantee: bool, enum, timestamp, JSON TEXT, NULL, round trip, legacy reads, and partial updates. Do not change production behavior, schema migrations, test coverage, dependencies, or unrelated historical documentation. No new tests are needed. Apart from this brief, the expected diff is the two source files above.

## Verification

- `CARGO_BUILD_JOBS=4 cargo test --locked -p runner-backend --lib --profile ci`.
- `cargo fmt --all --check`.
- `git diff --check`.
- Search the two files for `spike`, `Spike`, and `SPIKE`; no stale references should remain. Confirm the test inventory and assertions are unchanged apart from the enclosing module name.

Another mission is active, so keep Cargo at four jobs and do not launch or restart Runner, open any real Runner database, or run broad workspace builds for this naming change.

## Crew and completion

Coder implements and verifies, then explicitly requests reviewer review through Runner on the working-tree diff. Reviewer waits for that handoff, checks the entire diff for naming consistency and unchanged assertions/storage guarantees, and sends findings to coder. Iterate until the reviewer reports no remaining must-fix issues.

The planning brief is committed by the coordinator under the repository's mission convention. Implementation authorization stops at a clean, reviewed working-tree diff: no implementation commits, push, PR, merge, or branch deletion. No additional crew or nested subagents. Report blockers through Runner rather than guessing past the scope.

Final handoff on the mission feed: branch and base, changed files and names, each verification command and result, test inventory comparison, reviewer's verdict, and confirmation that implementation remains uncommitted.
