# 662 — Split `db.rs` by responsibility

[#662](https://github.com/yicheng47/runner/issues/662), P3, milestone 0.11. A pure refactor: `crates/runner-backend/src/db.rs` is 2,019 lines carrying five responsibilities and their tests. Split it into `src/db/` with no behaviour change. Work in `/Users/jason/repos/yicheng47/runner` on the existing branch `refactor/662-split-db`, cut from `main` at `63b1726`; its first commit carries this brief. Build on it; do not rebase, squash or create another branch.

Read first: this brief; the issue's Scope and Acceptance criteria; `crates/runner-backend/src/db.rs` end to end before moving a line; `docs/arch/arch.md` §7 for what the schema and the seed are; `AGENTS.md` for the conventions.

## Ownership and authorization

The coder owns the split and the checks; the reviewer waits for an explicit Runner handoff, then audits the whole working-tree diff. Because a file split makes `git diff` hard to read, the reviewer's job is one question: **did anything change other than the location of the code?** Iterate through Runner until no must-fix findings remain, and post the final handoff on the Runner feed. No additional crew, nested subagents, new checkout or worktree. Commits, push, PR, merge, and launching or restarting Runner are not authorized; leave the work uncommitted on the branch. `target/debug/runner` is the GUI app on this case-insensitive volume; the CLI is `target/debug/runner-agent-cli`.

Nothing in this mission touches anything outside the repo. Do not touch any real Runner data directory or database; every test already uses `open_in_memory` or a temporary file.

## Deliverable

`crates/runner-backend/src/db.rs` becomes `crates/runner-backend/src/db/` with these files, following the seams that already exist in the file:

- **`mod.rs`** — the module doc comment, `DbPool`, `open_pool`, `open_in_memory`, `build_pool`, `init_connection` (today's lines 1–114), plus the `mod` declarations and whatever `pub use` keeps the crate's `db::*` paths working.
- **`migrations.rs`** — the `MIGRATIONS` registry, `run_migrations`, `run_migrations_up_to`, `backfill_0015_retire_folders`, `backfill_0014_nodes` (today's 116–195 and 450–737).
- **`app_state.rs`** — `SEED_MARKER_KEY`, `LOGIN_SHELL_ENV_LKG_KEY`, `RUNTIME_OVERRIDES_KEY`, `LoginShellEnvLkg`, `ensure_app_state_table`, `app_state_get`, `app_state_set`, `login_shell_env_lkg`, `set_login_shell_env_lkg`, `runtime_overrides`, `set_runtime_override` (today's 197–287).
- **`seed.rs`** — the `SEED_*` constants, `seed_defaults`, `seed_default_crew`, `insert_seed_role`, `insert_seed_slot` (today's 288–449).
- **`tests.rs`** — today's `#[cfg(test)] mod tests` (739–2019), declared from `mod.rs` as `#[cfg(test)] mod tests;`.

`SEED_MARKER_KEY` sits with the other `_app_state` keys today but is read by the seed; give it whatever visibility keeps both callers working rather than duplicating the string.

### The traps

- **`include_str!` is relative to the file it appears in, and every one of these moves a directory deeper.** The 23 migration entries become `../../migrations/…` in `migrations.rs`; the three seed prompts become `../../../../examples/peer-coding/…` in `seed.rs`; inside `tests.rs`, today's line 883 `include_str!("../migrations/0002_persona_only_seeds.sql")` becomes `../../migrations/…` and line 1267's `include_str!("../../../tests/fixtures/system-prompts/architect.md")` becomes `../../../../tests/…`. A wrong path is a compile error, which is the good case; the bad case is a path that resolves to a different file, so check each one resolves to the file it names today.
- **The `MIGRATIONS` array is ordered and the order is the schema.** Moving it must not reorder, renumber or drop an entry. Diff the constant against `main` line by line.
- **The seed constants are data.** `SEED_CREW_ID`, the three role and crew IDs, `SEED_TIMESTAMP` and `SEED_ROLE_ARGS_JSON` are pinned values that existing databases match on; not one character changes.
- **`open_in_memory` is `#[cfg(test)]`** and has 101 callers across the crate; `build_pool`'s `seed` argument is what keeps test databases unseeded. Keep both exactly as they are.
- **`tests.rs` uses `use super::*`**, which after the move resolves to `db` (that is, `mod.rs`), not to the whole old file. Make the test module compile by importing from the new sibling modules, not by widening anything's visibility beyond what the crate already needs. `pub(crate)` stays `pub(crate)`; nothing becomes `pub` to satisfy a test.
- **Comments travel with their code.** Every doc comment and inline comment moves with the item it explains; the module header at the top of today's file belongs in `mod.rs`.

Out of this mission: any change to a migration, the seed data, the schema, `repo/`, or any caller outside `db`; new abstractions; renaming anything; #661's repository test rename; touching `docs/`.

## Verification

`make verify` green, plus `cargo test --locked --workspace --no-fail-fast --profile ci`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo fmt --all --check` and `git diff --check`. The workspace test count must be **identical** to `main`'s (1459 passed, 3 ignored on macOS as of `5620e4d`): a split that loses a test is the failure mode this refactor is most likely to have, so state the before and after counts in the handoff.

Two checks that prove "nothing but the move", both worth running and reporting:

- `git show main:crates/runner-backend/src/db.rs | grep -c ''` against the sum of the new files' line counts, and an explanation of any difference beyond the `mod` declarations, the imports each file now needs, and the `include_str!` path depths.
- `cargo expand -p runner-backend db 2>/dev/null` before and after, if `cargo expand` is available; if it is not, say so rather than installing anything.

## Handoff

Final Runner handoff, posted on the feed: branch and base commit; the five files with their line counts; every `include_str!` path you changed and the file each one resolves to; anything whose visibility you had to change and why; the before and after test counts; checks with results; and the reviewer's explicit no-remaining-must-fix verdict. Leave the work uncommitted.
