# 725 — ⌘W closes the tab, never the window

Fix [P2 #725](https://github.com/yicheng47/runner/issues/725). Jason asked on 2026-10-01 for a codex pair crew mission that ends in one open PR, not a merge.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/fix-725-close-tab`, on branch `fix/725-close-tab`. The mission's directory is this worktree; its tip is this brief on top of main `fccfe648`. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. If main moves and you need it, rebase onto `origin/main`; never merge main into the branch.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions (a mission lands as one commit).
- **Issue #725** (`gh issue view 725`). Its Expected behavior is the spec and wins over this brief on any detail; its Relevant code has line references checked on 2026-10-01. One correction to it: `request_close_chat_pane` already widens to an "Archive tab?" prompt through `archive_targets_for_chats` when the tab has drawer shells, so archiving a single-pane chat always deletes its tab.
- `main.rs` `close_target` and `close_window_or_pane`, the menu at `main.rs:1500`; `keymap.rs` entries `close-pane` and `system-close-window`, `windows_default`, the conflict exception at `keymap.rs:555`, and the binding block near `keymap.rs:952`; `surfaces/chat.rs` `request_close_chat_pane`, `request_close_terminal_tab`, `close_terminal_tab`; `surfaces/sidebar/archive.rs`; `pane_layout.rs` `PaneTabs`; `runner-backend/src/ops/node.rs` and `repo/node.rs`.

## Keys

| Action | macOS | Windows |
|---|---|---|
| Close tab | ⌘W | Ctrl+Shift+W |
| Close window | ⇧⌘W | Alt+F4 |

On Windows, Ctrl+W must reach the focused pane again (delete-word in shells, Claude Code and Codex).

## Deliverable

1. **Close tab action.** Today's `CloseWindowOrPane` becomes the close-tab action; rename it to say so. Order: a focused chat or mission drawer hides; a split closes its focused pane (unchanged); a single-pane tab closes (2–4); otherwise nothing happens. That covers the empty chat surface and the role, crew, settings, archived-chat and mission pages. It never closes the window.
2. **Single-pane chat tab.** Call `request_close_chat_pane` as the split path does. Cancel leaves everything open; Archive archives the chat and the backend deletes the tab.
3. **Single-pane terminal tab.** Call `request_close_terminal_tab`, as the sidebar × does: it closes at once when the shell is idle and asks first when a foreground process is running.
4. **Empty pane.** When the tab still has drawer shells, use the existing archive-all prompt for those shells; closing them deletes the tab in the backend. With no sessions left at all, delete the tab row directly. That needs a new backend op (for example `ops::node::node_tab_delete`) that refuses non-tab nodes and emits the layout change like `node_tab_upsert` does.
5. **Next tab.** After a tab closes through 2–4, the tab below it in the sidebar's order becomes active, or the one above it if it was last. `select_shortcut_row` numbers those same rows for ⌘1–9. Missions are skipped. If no chat tab is left, the chat surface's empty state shows. Today `PaneTabs::replace_rows` falls back to the first tab: choose the neighbour before the reload and activate it after, since the close finishes asynchronously.
6. **Close window action.** A new action bound to ⇧⌘W on macOS and to Alt+F4 on Windows. It closes the window from any route through the same path as the window's close button (`prepare_window_close`, then remove), so the layout is saved and chats keep running. Alt+F4 needs an explicit binding: without one, a focused terminal pane encodes it and stops propagation, so Windows never sees it.
7. **Menu and keymap.** The menu shows **Close Tab** (⌘W) and **Close Window** (⇧⌘W). Keep the `close-pane` id so stored overrides still match; retitle it "Close tab", update its description and scope, and default it to `cmd-w` on macOS and `ctrl-shift-w` on Windows. `system-close-window` defaults to `shift-cmd-w` and `alt-f4`. Both stay fixed. Bind both from their entries' platform defaults rather than a hard-coded `cmd-w`, and drop the `keymap.rs:555` exception once the two no longer share a key. Settings → Keymap shows both with the right keys on each platform.
8. **Tests**, beside the existing ones. Rewrite `cmd_w_hides_focused_drawers_and_only_closes_a_split_chat_pane` for the new targets, including a no-op on non-chat routes and on an empty chat surface. Also cover: the single-pane chat, terminal and empty-pane dispatch; neighbour selection after a tab closes, both middle and last; the new backend op (deletes a tab, refuses a mission or project node); and the keymap defaults on both platforms through the existing Windows-mapping tests (no `ctrl-w` default left on Windows, `alt-f4` present). Gate any test import or helper used only by `cfg(unix)` tests with `cfg(unix)`, since Windows clippy fails on unused ones.

Out of scope: #772's shortcuts; ⌘W on non-chat routes doing anything beyond drawers; changes to the archive or terminal prompts themselves; `.pen` files; README.

## Validation

Run each and report its exit code:

- `cargo test --locked -p runner-app --profile ci --no-fail-fast`
- `cargo test --locked -p runner-backend --profile ci --no-fail-fast`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- `cargo fmt --all --check`
- `git diff --check`

Log each to a file and read `$?` from the command itself; never pipe a gate through `tail` or `grep`.

Crews never run the dev app. Do not start, stop or type into Jason's Runner apps, chats or missions, and never open his real Runner database (`~/Library/Application Support/com.wycstudios.runner*`). Jason smoke-tests the UI. Native Windows is unavailable; say what is unverified there (Alt+F4 reaching the window, Ctrl+W reaching a pane).

## Crew handoff and authorization

The coder owns implementation, tests and fixes. The reviewer waits for an explicit Runner handoff, then reviews the whole working-tree diff against #725 and this brief, must-fix findings first with file:line pointers. It checks in particular: no route reaches a window close through the close-tab key; Ctrl+W has no binding on Windows; Alt+F4 wins over a focused terminal; the neighbour is chosen before the async close reloads tabs; the new backend op cannot delete a non-tab node. Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES`. No extra agents, crews or subagents.

After the clean review, and only then, Jason authorizes:

- **One commit.** Squash everything on the branch, this brief included, into a single commit on top of `main` with an imperative subject (for example `fix(ui): close the tab on ⌘W and the window on ⇧⌘W`) and no co-author trailers.
- **Push** with `git push -u origin fix/725-close-tab`.
- **Open the PR** with `gh pr create --base main`. The body carries `Closes #725`, a summary, test evidence, a manual check for Jason on macOS and Windows (each row of the issue's Expected behavior), what is unverified, and no agent session links.
- **Watch CI** with `gh pr checks <n> --watch` until nothing is pending, on macOS and Windows. Fold any fix into the commit with `git commit --amend`, have the reviewer check it, and push with `git push --force-with-lease`.

**Do not merge without Jason's explicit request**, clean up local branches or worktrees without explicit authorization, or cut a nightly or release. An authorized merge includes GitHub's configured automatic deletion of the merged remote branch; no separate confirmation is needed.

The final handoff goes to everyone through Runner: the PR URL and CI result, changed files, checks with exit codes, any deviation from #725, what is unverified, and the reviewer's verdict. Then both slots stand by.
