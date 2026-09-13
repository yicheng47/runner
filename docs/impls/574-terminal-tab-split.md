# 574 — Terminal tab split: Split Right / Split Down spawn a shell in place

Tracking issue: [#574](https://github.com/yicheng47/runner/issues/574). Spec: [574](../features/574-terminal-tab-split.md). Feature, P2. Design: `design/runner.pen` frame `Spec — Terminal tab split (574) · v1` (`TbPja`). Branch: **`feat/574-terminal-tab-split` already exists and is checked out** — it carries this brief; work on it, do not create another.

## What ships

A terminal tab — every filled pane a shell — splits like a chat tab: the same split icon (header while single-pane, identity line once grouped), Split Right `⌘D` / Split Down `⇧⌘D`, the 50 / 50 tree, the 240 × 160 floor with **Too small to split**. The new pane is never empty: a shell spawns in it at once and takes focus, in the cwd the split-from pane's shell was spawned with. A chat tab's split still makes the chat stub; the header's drawer icon stays hidden on terminal tabs. New terminal with a terminal tab active is Split Right through the same gate. Live cwd (OSC 7) is #575 and stays out.

## Where the code is

- `crates/runner-app/src/surfaces/chat.rs:1127` `active_tab_is_terminal_only` — true only for a **single-pane** tab holding one shell; a two-shell tab is not "terminal-only" today, so its identity-line split icons already show and `⌘D` there leaves an empty pane offering New chat, a mixed tab by accident. `:1724` `split_pane`: the gate `:1741` (`NotSplittable` return `:1748`), `layout.split`, then `persist_active_tab` → `reload_tabs` → `ensure_active_tab_attached` → `remember_active_runner`, `mark_active_tab_viewed`, `focus_active_terminal`; `:1781` / `:1790` the shortcut handlers, `:1799` `split_focused_pane`.
- `crates/runner-app/src/surfaces/panes.rs:398` header `split_action = (!grouped && !terminal_only)`; `:404` `drawer_action = (!terminal_only)`; `:1327` `split_menu`; `:2510` `split_allowed`; `:2573` `split_decision(terminal_only, pane, orientation, zoom)` with the `NotSplittable` arm `:2580`, mapped to no item in `split_menu_items` `:2602`; test `:3248` `every_split_route_meets_the_same_floor_and_terminal_only_tabs_offer_none`.
- `crates/runner-app/src/surfaces/start_chat.rs:67` `new_terminal_target` (Tab when the active tab is terminal-only); `:252` `new_terminal` — the Tab arm calls `prepare_new_pane` `:283` (fills an empty pane, else splits the focused pane right with no floor) then `spawn_terminal_in_pane`; `:447` `terminal_start_location` (the focused session's `cwd`, else the project cwd, else the default); `:478` `spawn_terminal_in_pane(pane_id, original, project_id, cwd, …)` — `session_start_shell`, assign, persist, reload, activate, attach, the Starting transition, and on error it closes the shell and restores `original`; test `:2073`. `DirectSessionEntry.cwd` is `runner-backend/src/ops/session.rs:243`.
- Docs: `docs/arch/arch.md:199` ("Splits hold chats only…"), `docs/tests/64-terminal-as-pane-option-smoke.md:23` and §4, `docs/features/archive/570-split-panes-redesign.md:32`.

## Fix shape

1. **Tab kind.** Replace `active_tab_is_terminal_only` with `active_tab_is_terminal(cx)`: at least one session and every session a `Runtime::Shell`; empty panes ignored, pane count irrelevant. `new_terminal_target` and the header's `drawer_action` use it. A legacy tab that holds a chat splits as a chat tab.
2. **Gate.** `split_decision` loses `terminal_only` and `NotSplittable`; `split_menu_items` and both `split_pane` routes follow. `split_action` becomes `(!grouped)`, so a single-shell tab's header shows the icon. The existing test is rewritten: a terminal tab is judged by size like a chat tab.
3. **Split spawns.** In `split_pane`, after `layout.split` returns the new pane id, when the tab is a terminal tab: keep a clone of the pre-split layout as `original`, take `project_id` and `cwd` from the split-from pane's session entry (fall back to `terminal_start_location`), and call `spawn_terminal_in_pane(new_pane_id, original, project_id, cwd, window, cx)` — it persists the split together with the shell and rolls back on failure. Otherwise the empty-stub path as today, unchanged.
4. **New terminal.** In `new_terminal`'s Tab arm an empty pane is still filled; with none, call `split_pane(focused_pane_id, Row, …)` instead of `prepare_new_pane`, so the floor applies and the shell spawns through step 3. Delete `prepare_new_pane` if nothing else calls it.
5. **Docs.** `arch.md:199`: the "chats only" sentence becomes the tab-kind rule. Smoke test `:23` rewritten (the icon shows, `⌘D` gives a shell) and one §4 line. The 570 archived spec's "A split holds chats only" bullet gets "Superseded by [574](../574-terminal-tab-split.md)".

## Rules of the road

- No backend change. No mixing: a chat tab never spawns a shell into a split, a terminal tab never shows the New chat stub. No live cwd, no drawer change, no settings.
- Stage by path, never `git add -A`. The tree carries nothing else at launch; if it does, ask.
- Do not launch the app (`make run`); Jason smoke-tests. Verify with `cargo test -p runner-app`, `make clippy` (also `--features updater`), `make fmt`.
- Mission authorization: after the reviewer reports clean, PR mode is authorized — commit on this branch, push, open the PR titled `feat(ui): terminal tabs split like Ghostty — a shell spawns in the new pane (574)`, drive CI green (`gh pr checks <n> --watch`; required check `Rust / macOS`). Do not merge: Jason merges after his own check. No worktrees, no extra checkouts, no extra agents.

## Tests

- `active_tab_is_terminal` (or its pure core): one shell, two shells, shell + empty → true; one chat, chat + shell, empty only → false.
- `split_decision` without the terminal arm: allowed and blocked by size on both axes, and at zoom 1.5.
- `new_terminal_target`: the existing cases plus a two-shell tab → Tab.
- `cargo test -p runner-app` green.

## Jason's smoke test (after landing)

1. A single terminal tab: the header shows the split icon; Split Right and `⌘D` give a second shell on the right, focused, `pwd` matching, no stub; `⇧⌘D` the same below. Both identity lines: terminal glyph, `⋯`, grip, split icon, `×`, no status dot.
2. Keep splitting until **Too small to split**; widen the window and it re-enables.
3. `⌘K` → New terminal with the terminal tab active splits right through the floor; with an empty pane it fills that pane.
4. A chat tab: a split still gives the New chat stub and its drawer icon still shows; the terminal tab has no drawer icon.
5. Quit and relaunch with three shells: shape and cwds come back; drag-reorder works between them.
