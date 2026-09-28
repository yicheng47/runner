# 729 — Remove unused crew purpose and goal

Implement [#729](https://github.com/yicheng47/runner/issues/729). Jason requested a codex pair mission on 2026-09-29. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/refactor-729-drop-crew-purpose-goal` on the existing branch `refactor/729-drop-crew-purpose-goal`, based on latest fetched `origin/main` at `2dd05b6`. Read `AGENTS.md` and the issue before editing. Other missions have their own worktrees; do not edit them or the root checkout, create another checkout, or share a Cargo target directory.

## Outcome and decisions

#699 / PR #727 already removed these fields from the app and mission prompt behavior. Finish removing the write-only crew fields across SQLite, Rust models/repositories/operations, socket schemas and CLI. Crew conventions (`system_prompt_addendum`) remain the crew's prose; mission goals remain per-mission. Do not migrate purpose/goal text into conventions or alter the prompt model.

The issue's migration number is stale: `0024_codex_speed.sql` already exists and must remain unchanged. Add the next unused migration, currently `0025`, that drops `crews.purpose` and `crews.goal`, following `0008_drop_crews_orchestrator_policy.sql` and `src/db/migrations.rs`. Recheck the sequence if main moves. Do not rewrite historical migrations.

CLI decision: remove `--purpose` and `--goal` from `crew create` and `crew update` outright. Passing either becomes the ordinary Clap usage error (exit 2), before any socket request or mutation. No warning-only compatibility layer. The mission start/goal flags and other unrelated goal concepts are unaffected.

## Implementation inventory

- `crates/runner-backend/src/model.rs`: `Crew` serialization and all construction sites.
- `src/repo/crew.rs`: row type, column lists, mappings, insert/update SQL and tests. Keep SQL behind the existing repository layer.
- `src/ops/crew.rs`: create/update inputs, goal validation and obsolete tests; update all call sites and fixtures, including mission test seeds. Audit the active crew tool/socket schemas in `src/mcp/tools/crew.rs` and their callers so their JSON contracts no longer advertise or emit either field.
- `crates/runner-cli/src/command.rs`: argument structs, JSON builders, help and tests. Current crew show output already omits the values; confirm JSON output and accepted input match the new model.
- `docs/arch/arch.md`: crew description, CLI reference, and schema. Correct active documentation that describes the removed fields as current behavior. Do not rewrite historical briefs/specs merely to erase history. If a README changes, update both languages.
- Record a clear downgrade note in durable project documentation and prominently in the PR: after this migration, older Runner versions that select the removed columns cannot use the same database. A database backed up before migration is needed to downgrade; do not claim the drop is reversible. No release/version bump in this mission.

## Verification

Use temporary/in-memory databases only; do not run the new migration against Jason's installed or development app data. Add a meaningful upgrade regression from the pre-drop schema with populated crew purpose/goal and real retained data. Verify conventions, identity/timestamps, slots and mission relationships survive; the two columns are absent; existing crew create/update/list/show behavior works; reopening/rerunning migrations succeeds. Also cover a fresh database.

Test CLI rejection of both removed flags on create and update, normal create/update and JSON output, and continued support for mission goals. Update obsolete fixtures rather than weakening assertions. Search active sources for residual crew purpose/goal use, distinguishing historical SQL and per-mission goals.

Run backend and CLI tests, workspace Clippy with warnings denied, formatting and `git diff --check`. Run runner-app tests when its changed construction sites or behavior warrant it; both platform CI jobs must pass on the final commit. Use the repo's ci profile where appropriate and record exact commands and exit codes.

## Crew workflow and authorization

The coder owns implementation, verification and fixes. The reviewer waits for an explicit Runner handoff, then reviews the complete branch and working-tree diff against this brief and the issue. Focus on migration data preservation, all SQL/model/schema consumers, the CLI error contract, and retaining mission goals/conventions. Iterate via Runner until the reviewer reports `NO REMAINING MUST-FIX ISSUES`. Do not start extra agents, subagents or crews.

After clean review and passing local checks, Jason authorizes squashing all branch work including this brief into one focused commit on top of current main, pushing `refactor/729-drop-crew-purpose-goal`, and opening a PR against main with `Closes #729`, the CLI removal decision, downgrade note, and test evidence. Fetch/rebase onto `origin/main` if it advances; never merge main into the branch. Amend subsequent review/CI fixes into the same commit and push with `--force-with-lease`. Drive macOS and Windows CI green. Do not merge, delete the branch/worktree, or cut a nightly/release. Final Runner handoff includes the PR URL, changed behavior, migration number, tests/exit codes, CI result and review verdict. Then stand by.
