# 772 — Start chats, terminals and missions from the keyboard

Implement [P2 #772](https://github.com/yicheng47/runner/issues/772). Jason asked on 2026-10-01 for a codex pair crew mission that ends in an open PR, not a merge.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-772-keyboard-create`, on branch `feat/772-keyboard-create`. The mission's directory is this worktree. Its tip is this brief and the settled spec, on top of main `12c4a65a`. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. If main moves and you need it, rebase onto `origin/main`; never merge main into the branch.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions (a mission lands as one commit).
- **`docs/features/772-keyboard-create.md`**, Proposal and Decisions. It wins over the issue and over this brief on any detail.
- `surfaces/start_chat.rs`: `open_new_tab_modal`, `open_start_chat_modal`, `on_start_chat_key_down`, `render_mode_button`, `start_chat_focus_order`, `new_terminal`, `new_terminal_tab`, and the tests.
- `surfaces/start_mission.rs` (its goal is a textarea), `ui/overlay.rs` (`Modal`), `ui/select.rs` (`on_key_down` ignores modifiers on `enter`), `ui/field.rs`, `ui/button.rs`.
- `keymap.rs`: `entries`, `windows_default` (#725's `cmd-w` case is the pattern), fixed context bindings like `Copy` in `Terminal`, and its default and Windows tests.
- `sidebar/menus.rs` (the create menus), `sidebar/elements.rs` (New chat row shortcut style), `panes.rs` `empty_pane_action_label`, `command_palette.rs` `palette_items`.

The design rests on gpui-pre 0.3.7 dispatching bindings before key-down listeners, with a deeper key context beating a global binding. Prove both in tests.

## Deliverable

1. **Confirm key.** `Modal` gains a way to set a key context on its root. Start a chat uses `StartChat`, Start mission uses `StartMission`; in each, ⌘↵ (Ctrl+Enter on Windows) is bound to a confirm action that runs the existing submit. It works from every control, including a focused select, an open select or model menu (which does not pick its highlighted row), Reset, Browse, the goal textarea and the footer buttons. It does nothing while the form cannot submit, while submitting, or while an IME composes. Plain Enter and Esc keep today's behaviour. These are fixed bindings, not Settings → Keymap entries.
2. **Footer keycaps.** Cancel shows `esc`; Start chat and Start mission show the confirm key formatted by the keymap (Windows text follows `format_combo`). Use the sidebar New chat row's shortcut size and weight, in a colour that reads on each button's own fill. Add the label to `Button` once if it has no slot for it.
3. **Direct | Role.** One tab stop: only the active segment is in `start_chat_focus_order`. ←/→ on it switch mode and keep focus on the switch; Enter and Space unchanged. ⌘1 / ⌘2 (Ctrl+1 / Ctrl+2) in `StartChat` switch to Direct / Role and focus that mode's picker, and do not change the active tab behind the modal. Faint keycaps inside the segments. A mouse switch leaves focus where it is.
4. **Initial focus.** The remembered mode's picker (the role picker when roles exist, the agent picker when agents exist), else Chat name. A preselected role from a pane's New chat opens on the role picker.
5. **New terminal.** A `new-terminal` keymap entry, Global and rebindable: ⌘T, and Ctrl+Shift+T on Windows through `windows_default`. On the chat surface with a focused empty pane, it starts a shell in that pane in the start location a sibling would use; otherwise it opens a terminal tab for the active project as the + menu's New terminal does, switching to the chat surface from other routes. No form.
6. **New mission.** A `new-mission` keymap entry: ⇧⌘M (Ctrl+Shift+M). Opens Start mission for the active project, as the + menu does.
7. **Guards.** ⌘N, ⌘T and ⇧⌘M do nothing while Start a chat or Start mission is open, as ⌘N already does for its own modal.
8. **Discoverability.** Both + menus show each entry's effective binding through `UiMenuItem::shortcut`, nothing when unbound, honouring user overrides. ⌘K gains New chat and New mission commands; its New terminal command is unchanged.
9. **Tests** for each item above, plus: Start a chat opened through `NewTab` with focus outside the modal, then Esc closes it and Enter in Chat name submits (if either fails, fix it first and say so). Update assertions the change invalidates. Gate any import or helper used only by `cfg(unix)` tests with `cfg(unix)`, since Windows clippy fails on unused ones.
10. **Docs, same diff.** If implementation forces a deviation from the spec, update the spec on this branch and say why in the handoff.

Out of scope: backend or CLI changes, type-ahead in pickers, other workspace shortcuts acting behind a modal, the palette's New terminal placement, `.pen` files.

## Validation

Run each of these and report its exit code:

- `cargo test --locked -p runner-app --profile ci --no-fail-fast`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- `cargo fmt --all --check`
- `git diff --check`

Log each to a file and read `$?` from the command itself; never pipe a gate through `tail` or `grep`.

Crews never run the dev app. Do not start, stop, restart or type into Jason's Runner apps, chats or missions, and never open Jason's real Runner database (`~/Library/Application Support/com.wycstudios.runner*`). Jason smoke-tests the UI. Native Windows is unavailable; say what is unverified there.

## Crew handoff and authorization

The coder owns implementation, tests and fixes. The reviewer waits for an explicit Runner handoff, then reviews the whole working-tree diff against the spec and this brief, must-fix findings first with file:line pointers. It checks in particular:

- that ⌘↵ is a binding in the modal contexts, not a key-down listener a focused select or textarea can swallow;
- that ⌘1/⌘2 never reach Select tab behind the modal, and the tab keys still work with no modal open;
- that Windows gets Ctrl+Shift+T, and no existing default or Windows mapping changed;
- that the empty-pane rule for ⌘T matches ⌘N's.

Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES`. No extra agents, crews or subagents.

After the clean review, and only then, Jason authorizes:

- **One commit.** Squash everything on the branch, this brief and the spec edit included, into a single commit on top of `main`: imperative subject naming the change (for example `feat(ui): start chats, terminals and missions from the keyboard`), no co-author trailers.
- **Push** with `git push -u origin feat/772-keyboard-create`.
- **Open the PR** with `gh pr create --base main`. The body carries `Closes #772`, a summary, test evidence, a manual check for Jason (⌘N, ⌘2, pick a role, ⌘↵ with no mouse; ⌘↵ from an open select; ←/→ on the switch; Start mission with a multi-line goal and ⌘↵; ⌘T into an empty pane and into a new tab; ⇧⌘M; the + menu keys; the footer keycaps in both themes), what is unverified, and no agent session links.
- **Watch CI** with `gh pr checks <n> --watch` until nothing is pending, on both macOS and Windows. Fold any fix into the commit with `git commit --amend`, have the reviewer check it, and push with `git push --force-with-lease`.

**Do not merge**, delete the branch or worktree, or cut a nightly or release.

The final handoff goes to everyone through Runner: the PR URL and CI result, changed files, checks with exit codes, any spec deviation, what is untested or unverified, and the reviewer's verdict. Then both slots stand by.
