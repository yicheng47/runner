# 697 — Keyboard shortcut tidy

Implement [P2 #697](https://github.com/yicheng47/runner/issues/697). Jason asked for a codex-crew mission that ends in an open PR, not a merge.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-697-shortcut-tidy`, on branch `feat/697-shortcut-tidy`. Its tip is this brief, on top of main `bef945c`. Do not create another branch or checkout, touch the root checkout, or share another worktree's target directory. If main moves and you need it, rebase onto `origin/main`; never merge main into the branch.

## Read first

- `AGENTS.md`, including Crew Missions.
- **`docs/features/697-shortcut-tidy.md`**: the spec. It wins over the issue and over this brief on any detail. Its design, `design/specs/697-shortcut-tidy.pen`, is Jason's reference; do not open or edit `.pen` files, because the spec describes everything you need.
- `crates/runner-app/src/keymap.rs`: `entries()`, `reserved_entries()`, `windows_default`, `effective_binding`, `find_conflict`, `install_bindings` and its per-entry binding match, and the tests (one asserts `stop-session` formats as "⌘.").
- `crates/runner-app/src/surfaces/settings_page.rs`: `render_shortcuts_settings` (`:1463`), `render_shortcut_row` (the `fixed` branch), search, and the "No shortcuts match" state.
- `crates/runner-app/src/surfaces/chat.rs`: `stop_focused_session` (`:554`), `resume_chats`, `resume_chat`, `active_focused_session_id`.
- `crates/runner-app/src/main.rs` (actions) and `surfaces/app_shell.rs:232` (registration).
- `crates/runner-app/src/surfaces/mission_workspace/`: `view.rs:275` (the `MissionTabPrevious` handler to mirror), `actions.rs` (`act_on_slot`), `rail.rs` and `mod.rs` (`slot_controls`).
- `crates/runner-app/src/surfaces/panes.rs`: `pane_action_items_for` (`:2835`), and the menu handler that dispatches by item index (`0 => stop_chat`).

## Deliverable

1. **Tab row.** Mark the nine `select-tab-N` entries `fixed: true`, with ids and bindings unchanged, and render them as one row: "Go to tab 1–9", "Open a visible sidebar tab or mission by its position.", chip "⌘1–⌘9". The chip has no controls, like other fixed rows. A small `keymap` helper names the group. Losing old per-digit overrides is the spec's deliberate decision; do not migrate or warn.
2. **Fixed card.** The pane renders every rebindable entry in the first card, then a "Fixed" heading ("Runner's built-in keys. They can't be changed.") and a second card with New window, Go to tab 1–9 and Close pane. Move `copy` from `entries()` to `reserved_entries()`: its bindings in `install_bindings` stay, and `find_conflict` must still refuse ⌘C. Search filters both cards; hide a card and its heading when it has no match, and keep "No shortcuts match" for when both are empty. "tab 3", "⌘3" and "go to tab" must all find the tab row.
3. **Keys.** `stop-session` changes its default to ⇧⌘X, and a new `resume-session` ("Resume focused session") defaults to ⇧⌘R, with a `ResumeFocusedSession` action. Update the descriptions as the spec gives them. `windows_default` yields Ctrl+Shift+X and Ctrl+Shift+R.
4. **Routing.** Both keys act on the focused session:
   - **Chat route**: `active_focused_session_id()`. Stop is unchanged. Resume goes through `resume_chat` for the focused pane, which restarts a terminal pane's shell.
   - **Mission view**: `StopFocusedSession` and `ResumeFocusedSession` handlers on `MissionWorkspace` act on `MissionTab::Session(id)` through `act_on_slot`, and do nothing on the Feed tab.
   - Existing guards (transitions, secondary windows, lifecycle busy) make the key a no-op wherever the button would be disabled.
   - No key for mission-level Stop all / Resume, Restart slot or drawer shells.
5. **Bindings shown.**
   - **Pane menu**: the first item is Stop with its binding while the session runs. Once it is stopped or crashed, the item is Resume with its binding, or Restart for a terminal pane, taking the resume path. Fix the index dispatch so item 0 is right in both states.
   - **Rail**: the Stop and Resume slot controls get "Stop · ⇧⌘X" / "Resume · ⇧⌘R" tooltips through `SessionControl`'s `title`.
   - Every label reads its binding through `keymap::effective_binding`.
6. **Tests.** Cover every item in the spec's Verification list, including:
   - the fixed tab entries ignoring a saved override;
   - Copy absent from the page but still conflicting;
   - the search cases across both cards;
   - `pane_action_items_for` for running, stopped agent and stopped shell;
   - both new defaults and their Windows mapping.

   Update existing assertions the change invalidates. Add mission-view routing tests where `mission_workspace/tests.rs` makes it practical, and name in the handoff what could not be unit-tested.
7. **Docs, same diff.** Touch `docs/arch/arch.md` only if it describes something you change. If implementation forces a deviation from the spec, update the spec in this branch and say why in the handoff.

Out of scope: other shortcut changes, the hidden system shortcuts, `.pen` files, README screenshots.

## Validation

Run each of these and report its exit code:

- `cargo test --locked -p runner-app --profile ci --no-fail-fast`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- `cargo fmt --all --check`
- `git diff --check`

Log each to a file and read `$?` from the command itself; never pipe a gate through `tail` or `grep`.

Crews never run the dev app, and do not start, stop, restart or type into Jason's Runner apps, chats or missions; Jason smoke-tests the UI. Do not change agent configuration. Native Windows is unavailable; say what is unverified there, including Ctrl+Shift+X/R reaching the app.

## Crew handoff and authorization

The coder owns implementation, tests and fixes. The reviewer waits for an explicit Runner handoff. Then it reviews the whole working-tree diff against the spec and this brief, must-fix findings first with file:line pointers. It checks in particular:

- that the pane menu's index dispatch can never stop a stopped session or resume a running one;
- that Copy still blocks ⌘C;
- that search hides empty cards.

Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES`. No extra agents, crews or subagents.

After the clean review, and only then, Jason authorizes:

- **Commit** in focused commits on this branch: imperative subject, scope `ui`, `keymap` or `docs`, no co-author trailers. Keep the brief commit.
- **Push** with `git push -u origin feat/697-shortcut-tidy`.
- **Open the PR** with `gh pr create --base main`. The body carries `Closes #697`, a summary, the key changes (the tab row now fixed, Stop ⌘. → ⇧⌘X, Resume ⇧⌘R, Copy hidden), test evidence and unverified platforms, and no agent session links.
- **Watch CI** with `gh pr checks <n> --watch` until nothing is pending. Fix any failure on the branch, have the reviewer check the fix, and push again.

**Do not merge**, delete the branch or worktree, or cut a nightly or release.

The final handoff goes to everyone through Runner: the PR URL and CI result, changed files, checks with exit codes, any spec deviation, what is untested or unverified, and the reviewer's verdict. Then both slots stand by.
