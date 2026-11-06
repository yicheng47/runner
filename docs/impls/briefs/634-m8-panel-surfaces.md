# 634 m8 — side panel surfaces with a + menu

## Starting point

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-634-m8-panel-surfaces`, on branch `feat/634-m8-panel-surfaces`, cut from the umbrella `origin/feat/634-file-preview` at `6dac9713` (m1–m7 merged, rebased onto `main` `324dbacb`). This brief is the branch's tip. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. Follow the umbrella by rebase, never by merge.

## Problem and outcome

Option H, from m7, puts every view of the surface in the panel header as an icon button. Each new view (Changes in #1141, later others) adds an icon, and the header gets crowded. Jason chose a model after t3code on 2026-11-05: the header holds only the surfaces you opened, as tabs, and a `+` lists every surface with a one-key shortcut. Settings → Side panel → Panel follows.

## Design

Board "634 — Side panel surfaces · after t3code (2026-11-05)" (`cwksG`) in `runner/desktop/specs/634-file-preview.pen` in `yeelinks_design`, signed off by Jason on 2026-11-05: A (`du8Q4`), B (`GfSLO`, with the `+` menu `r2y8b`), C (`h2gjjR`) and D (`jdKtZ`). PNG exports are in this worktree's `target/design-ref/`:

- `chat-empty-surface-list.png` and `panel-empty-closeup.png`: a chat with nothing open;
- `chat-files-open-plus-menu.png`, `header-chat-closeup.png` and `plus-menu-closeup.png`: Files open, `+` menu open;
- `mission-sessions-permanent.png` and `header-mission-closeup.png`: a mission;
- `settings-panel-surfaces.png` and `settings-surfaces-card-closeup.png`: Settings.

Where the design and this brief disagree on visual detail, the design wins; on behaviour, this brief wins.

## Decisions (Jason, 2026-11-05, and the operator under his mandate)

1. **The header holds the open surfaces as tabs.** Each is the surface's icon and name, 28 px high; the active one is raised, semibold and has a ×. After the tabs comes a `+` (24 px, plus icon), then a flex gap, then collapse. Option H's title and icon row are removed, and so is its single-view rule.
2. **Each surface opens once.** Picking one that is open switches to its tab. Files stays one tab with one folder, holding any number of file tabs in its own row as today. Its tree, widening, width rules and colours are unchanged.
3. **The `+` menu** lists the surfaces this surface offers that Settings turns on, in the order Info, Files, each with its icon, name, a check mark when open, and its key in a keycap: `I` for Info, `F` for Files. Clicking a row, or pressing its key while the menu has focus, opens or switches to it and closes the menu. Esc closes it. The keys act only while the menu or the empty list has focus, never globally.
4. **The empty panel.** When a chat's panel has no surface open, it shows "Open a surface" over the same rows and keys, with the hint "Press a surface's letter to open it", and the header shows only `+` and collapse. The list takes focus when the panel opens empty.
5. **Closing.** × on a surface tab closes it and shows the surface before it, or the empty list when it was the last.
6. **A mission's Sessions roster is permanent** (Jason): always the first tab, with no ×, and never in the `+` menu, so a mission never shows the empty list. `+` adds Info and Files.
7. **A surface with one surface on offer**, such as a terminal-only tab (Files only) or a chat with Info turned off, shows that surface's tab permanently, with no × and no `+`.
8. **What opens.** A new chat's panel opens the surface Settings names (decision 10), Info by default as today. Opening a file, from the tree, a Markdown link or a ⌘-clicked terminal link (m7), opens the Files tab if it is closed and switches to it. The open surfaces and the active one are kept per chat and per mission where the chosen view is kept today (`chat_panel_view`, `rail_view`), with the same lifetime; no runnerd, protocol or storage change.
9. **Changes is not shown yet** (operator): the design draws it dimmed as "Coming with #1141", but a row for a view that does not exist is dead UI; #1141 adds it. No dimmed-row state is built until a surface needs it.
10. **Settings → Side panel → Panel.** The Views card becomes Surfaces: the header preview shows the new header (Info open, `+`, collapse); the Info and Files rows keep their switches, now meaning "offered in the `+` menu" (turning one off closes its open tabs), and show their key; a new row "A new chat's panel opens" is a select of Info (default), Files and Nothing (the surface list). The note under it says a mission always shows Sessions as its first tab, which cannot be closed. Show the side panel is unchanged, and a chat with every surface off still has no panel. Strings in English and Simplified Chinese.

Non-goals: Changes and any new surface, several Files tabs or folders, global surface shortcuts, drag-reordering tabs. If a question arises that these decisions, the spec and the design do not settle, ask through Runner.

## Read first

- `side_panel.rs` (`PanelViewId`, `VIEWS`, `views`, `shown_view`, `header_views`), `panes/header.rs` and `panes/mod.rs` (the chat panel and `chat_panel_view`), `mission_workspace/rail.rs` and `state.rs` (`rail_view`), `settings/page/side_panel.rs`, `app_settings.rs`, `files/links.rs` (m7's link routing).
- [The spec](../../features/634-file-preview.md), whose header paragraph (line 16) describes option H, [the plan](../634-file-preview/plan.md) and [the README](../634-file-preview/README.md): update all three to these decisions, add m8 to the plan and the README's table, and add the decisions under the README's binding decisions.
- The guide: [Chats and windows](../../guide/guides/chats-and-windows.md) (the side panel), [Missions](../../guide/concepts/missions.md) and the Settings reference, in both locales.

## Validation

Unit tests for the header for a chat, a mission and a terminal-only tab; the `+` menu's rows, check marks, keys and Esc; the empty list and its focus; opening, switching and closing, including the last surface and the permanent ones; a file open or ⌘-click opening a closed Files tab; each Settings value, the switches closing tabs, and "A new chat's panel opens" in a new chat; snapshots of the header, menu, empty list and Settings card in both themes and both languages. Run `runner-app` tests, workspace clippy with the CI profile and `--features runner-app/updater`, fmt and `script/check-guide-translations.py`, logged with exit codes.

Live checks, by `qa` on macOS in the dark theme, on the reviewed candidate:

1. A new chat's panel opens on Info; closing every surface shows "Open a surface", and `I` and `F` open Info and Files from it.
2. `+` lists Info and Files with check marks on the open ones; picking an open one switches to it, a closed one opens it, and Esc closes the menu.
3. With Files closed, clicking a file link in a terminal and opening a file from Info's folder both open the Files tab; several files keep their tabs inside Files.
4. A mission shows Sessions first with no × and not in `+`; `+` adds Info and Files, and closing them returns to Sessions.
5. A terminal-only tab shows Files with no × and no `+`.
6. Settings → Side panel → Panel shows the Surfaces card as drawn, in English and Chinese; turning Files off removes it from `+` and closes its tab; each "A new chat's panel opens" value applies to a new chat.

## Handoffs

The coder implements, tests and fixes, then hands the candidate to the reviewer through Runner. The reviewer reviews the whole diff against these decisions, the design and the spec, must-fix first with file:line pointers, and checks in particular that m7's link routing, the widened panel's width rules and the mission roster's two-up layout still work, that no stale reference to option H remains in code, strings or docs, and that old settings files still load. After a clean review, the coder hands the reviewed commit to `qa`, who runs the live checks above with `script/qa-lock` and reports pass or fail per check with screenshots; a failure goes back to the coder and the fix is reviewed again before qa reruns the affected checks.

## Authorization

Approved crew: the claude duo with the `qa-claude` role as `qa`. Once review is clean, the crew may commit, push this branch and open a PR against `feat/634-file-preview` titled `feat(ui): side panel surfaces with a + menu (#634 m8)`, labelled `risk-medium`, `needs-live-test` and `frontend-only` if nothing outside `runner-app` changes, and drives CI green on every job. qa records its results in the PR description and removes `needs-live-test` once every check passes. The operator merges it into the umbrella; the crew does not merge.
