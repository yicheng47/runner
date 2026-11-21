# #1409 — release stale Claude Code drafting holds, add fallback cap, and log dropped reports (PR #1412)

Jason approved this `neon trio` mission (`coder`: Antigravity; `reviewer`, `qa`: Codex) on 2026-11-21 to finish, review, live-test, and bring existing draft PR #1412 (`https://github.com/yicheng47/runner/pull/1412`) to merge-ready. This chat operates the mission.

## Starting point and isolation

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-1409-claude-drafting-hold` on existing branch `fix/1409-claude-drafting-hold` (head `f93ba46171267e050e73f8f18bfe42af224dd1c4`, based on `8012ecb7`, targeting `main`). `docs/impls/briefs/1409-claude-drafting-hold.md` is the expected initial untracked brief file; fold it into the PR's fix commit before final push rather than leaving a standalone brief commit.

Never touch the root checkout (stays on `main`, has unrelated uncommitted edits) or sibling worktrees under `.worktrees/` (`fix-1395-remote-project-drag`, `test-flaky-process-probe-tests`). Keep Cargo's `target/` local to this worktree; never share `CARGO_TARGET_DIR`.

PR #1412 already contains the initial implementation commit `f93ba461` as a draft PR with `core-review` and `needs-live-test` (missing a risk label and `reviewed`), and has not been code-reviewed or live-tested yet.

## Read first

1. `AGENTS.md` (Test Scope, Engineering Conventions, Commit And PR Conventions, Crew Missions), `gh issue view 1409`, and `gh pr view 1412`.
2. `crates/runner-terminal/src/input_state.rs` and `crates/runner-terminal/src/input_state/tests.rs` (persisted `last_placeholder_style`, `draft_style`, `is_placeholder_style` dim + fake-inverse-cursor detection, `row_matches_prefix` / `row_matches_nonempty_prefix` NBSP/ASCII matching, unframed `relocate_row`, `recheck_delivery` visibility gate removal and `log_recheck_draft_hold`).
3. `crates/runner-daemon/src/session/state/mod.rs`, `crates/runner-daemon/src/session/manager/mod.rs`, and `crates/runner-daemon/src/session/manager/tests/input.rs` (`report_draft_hold` sharing `OBSERVED_DRAFT_HOLD_CAP`, keeping native `EditorDraft` uncapped).
4. `crates/runner-daemon/src/router/report.rs`, `crates/runner-daemon/src/ops/mission/report.rs`, and their tests (`DropReason`, `report dropped` log line, transient SQLite read error returning `Line::Wait`).
5. `docs/guide/concepts/missions.md`, `docs/guide/zh-CN/concepts/missions.md`, and `docs/tests/regression/missions.md` (`MIS-LINK-03` still specifies an uncapped screen-observed hold past 10 minutes — reconcile per `docs/tests/regression/README.md` if it conflicts with the new contract).

## Scope and deliverables

1. **Audit and tighten implementation (`@coder`):** Inspect commit `f93ba461` and current CI results (`gh pr checks 1412`). Check whether any remaining docs or regression rows (especially `MIS-LINK-03` in `docs/tests/regression/missions.md` and `docs/tech/` if applicable) still state the old uncapped screen-observed report hold and reconcile cleanly. Verify guide translation parity (`python3 script/check-guide-translations.py`), address any edge cases or CI failures, fold this brief into the fix commit locally, run targeted automated verification, and hand the local commit SHA to `@reviewer`. Never run the development app yourself.
2. **Code review (`@reviewer`):** Stay idle until `@coder` hands off a candidate SHA. Review the full diff against #1409 and `AGENTS.md`: check `is_placeholder_style` edge cases (dim suggestion vs real draft, fake inverse cursor on cell 0, persisted placeholder style), `row_matches_nonempty_prefix` / `relocate_row` safety against transcript false matches, `recheck_delivery` hidden-composer clearing and bounded diagnostic logging, `OBSERVED_DRAFT_HOLD_CAP` vs native `EditorDraft`, `DropReason` handling, guide/regression doc accuracy, and no-race test rules. Name any extra live checks for `@qa` in the clean verdict. Do not run the app.
3. **Live QA on macOS (`@qa`, after clean code review):** Stay idle until `@coder` hands over a reviewed candidate SHA. Acquire `script/qa-lock acquire --timeout 540` before building/launching the dev app (`make run` from this worktree). Preserve all pre-existing dev chats and sessions; archive only QA-created test chats, missions, and `/tmp/...` folders; release `qa-lock` cleanly once QA UI exits and artifacts are cleaned up (do not stop the shared dev daemon if pre-existing user sessions are live). Run the selected live checks on the exact reviewed commit with cheap/low-effort Claude Code settings in fresh `/tmp` directories:
   - **Check 1 — Idle Claude Code chat after blank-composer / dim prompt suggestion:** In a linked Claude Code chat after a blank-composer turn (with dim prompt suggestion if reproduced), start a bounded mission and leave the chat idle; verify the `Settled` report delivers and confirms within seconds with no stale `Drafting` hold.
   - **Check 2 — Genuine draft protection and clear release:** Type an unsent draft in the linked Claude Code chat while a mission settles; verify the report stays held as `Drafting` across the 30 s recheck without typing into the draft (and `runnerd.log` logs `input tracker recheck kept drafting` cleanly without repeating identical state lines); then clear the draft and verify delivery releases and confirms.
   - **Check 3 — Held report dropped on state change:** Hold a `Settled` report behind a genuine draft, move the mission back to `Working`, clear the draft, and verify `runnerd.log` logs `report dropped session=... mission=... kind=State reason=state changed (Working)` after `report delivery released` with no stale report delivered.
   - **Check 4 — Any additional check named by `@reviewer`.**
   Record results in `docs/tests/runs/1409-claude-drafting-hold-mac.md` (candidate commit, environment, timestamps, session/mission IDs, log excerpts, screenshots) and update PR #1412's description. Handle `needs-live-test` per `AGENTS.md` (keep if any required check or platform remains unverified; disclose unit-only coverage such as the 10-minute cap honestly).

Non-goals: pending-report UI indicators (#1216); unrelated terminal/router refactors; changing `pi`'s uncapped native `EditorDraft` hold.

## Validation gates

- **Automated (`@coder`):** `cargo test -p runner-terminal`, `cargo test -p runner-daemon`, `make clippy`, `cargo fmt --all --check`, `git diff --check`, and `python3 script/check-guide-translations.py`. If any new daemon-backed or async test is added, stress-loop it 20 times with `cargo nextest run -p <crate> --stress-count 20 -E '<filter>'` and report the count.
- **Rebase note:** Only rebase onto `origin/main` if conflicts exist or overlapping code landed on `main` (currently `origin/main` is only UI/docs ahead of `8012ecb7`).

## Authorization

Approved crew: `neon trio`. After code review and live QA are both clean: `@coder` folds this brief and any review/QA fixes cleanly into the branch history (plus a focused commit for `docs/tests/runs/1409-claude-drafting-hold-mac.md` once written), pushes with `git push --force-with-lease` to `fix/1409-claude-drafting-hold`, confirms `@reviewer` applies `reviewed` and comments the final pushed head SHA on PR #1412, keeps `core-review`, sets exactly one risk label (`risk-medium` or `risk-high` per review), marks PR #1412 ready for review (`gh pr ready 1412`), and drives CI green on all platforms (`gh pr checks 1412`). **Do not merge** — stop once PR #1412 is ready for Jason's review.
