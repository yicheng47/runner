# 850 — feed composer @mention works anywhere in the draft

Fix [#850](https://github.com/yicheng47/runner/issues/850). Jason requested this mission on 2026-10-09. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-850-composer-mid-text-mention`, on the existing branch `fix/850-composer-mid-text-mention`, created from `origin/main` at `fe441f4c`. The root checkout stays on main. Do not create another branch or worktree, and do not share a Cargo target directory.

## The bug

In a mission's feed composer, the `@` roster picker opens only when `@` is the first character of the draft. After any text has been typed, `@` opens nothing, so a role cannot be mentioned once the message has started. `leading_mention_query` (`crates/runner-app/src/surfaces/mission_composer.rs` about line 29) uses `draft.strip_prefix('@')`, and `mention_query`, `mention_options`, `update_draft` and `key_down` all go through it. `ComposerState` has no caret position, so the query cannot follow the caret. Picking a handle goes through `select_target` (about line 66), which returns `ComposerState::default()` and clears the draft; the view then resets the input to empty (`select_mission_composer_target` in `crates/runner-app/src/surfaces/mission_workspace/composer.rs` about line 269, and `on_mission_composer_key_down` about line 238).

## Behavior to build

Jason decided this on 2026-10-09; he may adjust it at PR review.

1. **The query follows the caret.** The picker opens when the caret sits at the end of an `@query` token whose `@` is at the start of the draft or right after whitespace, and `query` has no whitespace (it may be empty). The roster filter, Up/Down, Enter/Tab, Space on an exact match, and Escape dismissal work the same mid-text as at the start. An `@` inside a word (`me@host`) does not open it.
2. **A leading pick works as today, without losing text.** When the token starts at offset 0, picking sets the recipient chip and removes the token, as now, but keeps any text after it.
3. **A mid-text pick completes the mention in place.** When the token starts after other text, picking replaces `@query` with `@handle ` in the draft, keeps the text before and after it, and puts the caret after the inserted space. If no recipient chip is set, the pick also sets the chip to that handle, so `ask @reviewer to check the diff` is addressed to the reviewer and still reads as a sentence. If a chip is already set, the pick only completes the text and leaves the chip unchanged.
4. **Unchanged:** one recipient at most (several recipients arrive with #826, do not build them), Backspace on an empty draft clears the chip, Enter posts `ComposerPost { text, to }` with the chip as `to`, Shift+Enter, and posting while a post is in flight or in a secondary window stays blocked.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions.
- Issue #850, and decision 1 of `docs/features/826-targeted-messaging.md` for context only: #826 changes the daemon's addressing later; do not touch the daemon, the CLI or the event model here.
- `crates/runner-app/src/surfaces/mission_composer.rs`: the pure composer state machine and its tests (`picker_and_send_match_the_react_composer_contract`, `escape_and_target_backspace_match_the_react_composer_contract`).
- `crates/runner-app/src/surfaces/mission_workspace/composer.rs`: rendering, picker clicks, key handling, and how the draft and the `TextField` stay in sync.
- `crates/runner-app/src/ui/field/text_field.rs` and `buffer.rs`: `TextField` has `reset` and `set_text` but no public caret accessor. Add the smallest API the composer needs (for example the caret offset and a way to replace a range and place the caret), following the buffer's existing selection and undo handling, and keep it generic to `TextField`.
- `crates/runner-app/src/surfaces/mission_workspace/tests.rs` around line 154 for how the workspace tests drive the composer input.

## Deliverables

1. The behavior above, with the caret position carried into the composer state machine so it stays a pure, unit-tested function of draft, caret and key.
2. Unit tests in `mission_composer.rs` for: a mid-text `@` opening the picker; `me@host` not opening it; filtering by the partial query; a mid-text pick completing `@handle ` with text before and after preserved and the caret after the space; a mid-text pick setting the chip when none is set and leaving it when one is; a leading pick keeping trailing text; Escape dismissing a mid-text query; the caret moved away from the token closing the picker. Keep the existing tests passing unchanged unless a test pinned the draft-wiping behavior, and say so in the handoff if one did.
3. If the `TextField` gains API, a test for it under `crates/runner-app/src/ui/field/tests`.

Keep the change scoped to this. No design, styling or layout changes to the composer or the picker, no daemon or CLI changes, no README change.

## Boundaries

No visual or live check is needed for this mission (Jason's call). Crews never run the dev app, and do not run `runner-dev` or `runner` commands that start, stop or kill sessions or the daemon. Do not start extra agents, crews or subagents.

## Review, verification and authorization

The coder owns implementation and checks. The reviewer waits for an explicit Runner handoff, then reviews the full branch diff against #850 and this brief with must-fix findings first and file:line pointers. Focus on: caret and byte/char offsets with non-ASCII text (Pinyin, emoji) never splitting a character; the IME marked-text state not being clobbered by a programmatic replace; undo after a pick restoring sensible text; no draft text lost on any pick; the picker state staying consistent when the caret moves by mouse or arrow keys; and no behavior change outside the composer. Iterate until the reviewer posts `NO REMAINING MUST-FIX ISSUES`.

Run `cargo test --locked -p runner-app --profile ci`, workspace Clippy with warnings denied (`cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`), macOS updater Clippy (`cargo clippy --locked --workspace --all-targets --profile ci --features updater -- -D warnings`), `cargo fmt --all --check`, and `git diff --check`. Record the exact commands and exit codes, and never pipe a gate through `tail` or `head` in a way that hides its exit status. Windows CI has repeatedly failed on imports or helpers used only by `cfg(unix)` tests; gate any such import or helper with `#[cfg(unix)]`.

After a clean review, Jason authorizes squashing all work on this branch, this brief included, into one commit on top of current `origin/main` with a subject that names the fix (for example `fix(ui): open the feed composer's mention picker anywhere in the draft`), pushing `fix/850-composer-mid-text-mention`, and opening a PR against main whose body says `Fixes #850`. If main has moved, rebase; never merge main into the branch. Review or CI fixes after the push are amended into the same commit and pushed with `git push --force-with-lease`. Drive CI green on macOS and Windows. Do not merge, delete the branch or worktree, or cut a nightly or release. Final Runner handoff: PR URL, what changed, tests and exit codes, CI result, the reviewer's verdict, and what Jason should try in the composer (type `please check `, then `@rev`, Tab: the draft reads `please check @reviewer ` and the chip shows `@reviewer`; `@` at the start still makes a chip). Then stand by.
