# Start missions from the launcher

Tracking issue: [#1097](https://github.com/yicheng47/runner/issues/1097). Design: `runner/desktop/specs/1097-mission-launcher.pen` in the design repo.

## Motivation

Since 0.19.0 chats and terminals start from the in-pane launcher with no form ([#1092](https://github.com/yicheng47/runner/issues/1092)). Missions still opened the New mission dialog, which asked again for the project and folder the launcher already shows. Missions now start from the launcher, and the dialog is gone.

## Behavior

- **Entry points.** ⌘N, the sidebar’s New chat, + on a project or Recents, ⇧⌘M, the launcher’s Mission row, the palette’s New mission and Start mission on a crew page show a workspace launcher page without creating a tab. Mission entry points open the crew step; a crew page picks that crew on the goal step. New chat is selected in sidebar navigation on every step. The sidebar’s New mission entry is gone.
- **Crew step.** A Mission chip in the search field, crews by last use with their pictures and slot handles, and Empty mission on ⇧↵. Esc, ⌫ in an empty field or the chip’s × returns to the default page. A crew with no roles is dimmed with "No roles yet", and picking it opens the crew page. Recent stays agents and roles, so number keys keep starting chats. Typing on the default page also lists matching crews after agents and roles, labelled Crew.
- **Goal step.** The search field becomes the goal: the crew's chip (× back to crews) and the text, which grows to six lines and then scrolls. The start folder stays below it (Change and the project picker work as for chats), then the crew's slots, read-only, with each role and model. ↵ starts, ⇧↵ adds a line, Esc returns to the crews, and an empty goal is allowed. There is no title field.
- **Title.** The launcher names the mission from the goal's first non-empty line, trimmed and cut at 60 characters, else `<crew name> <YYYY-MM-DD>`, else `Mission <YYYY-MM-DD>` for an Empty mission with no goal. The rule exists once, beside the launcher (`launcher_model::mission_title`). `runner mission start` now requires `--title`, since agents always write one, and keeps no fallback. The protocol is unchanged: both callers send `title`.
- **Split panes.** An empty pane inside a real split tab keeps its launcher, pane header and ×; × closes that pane. It has no Mission row or Runner mark. ⇧⌘M and ⌘N from a split show the workspace launcher page without creating a tab.
- **The page.** The 2026-11-01 revised decision supersedes issue Decision 5 and the original empty-tab design. Nothing is saved for an empty launcher: there is no tab, project-tree row, pane header, New label, split menu or pane ×. The Runner mark and "Start something new" (`ui.start_something_new`) remain above the field on every step. Esc on the default page returns to the previous tab/view; selecting a sidebar row leaves the launcher. Starting a chat, terminal or mission creates its tab and selected sidebar row under its project, or Recents. The dashed New row, empty-tab ownership, cleanup and related placement/shortcut expansion are removed.
- **Sidebar +.** The + on a project or Recents shows the launcher page with that scope selected, directly and without a menu or saved tab. Project and folder selection affect the next start; they add no project-tree row.
- **Remote.** On another computer the crew step lists that host's crews and their slots from its catalog, and the Mission row and every mission entry point stay hidden or inert without the host's `catalog` and `startMission` capabilities. Empty mission is available on this computer only; the current remote API requires a crew. Remote slots show their roles without model or effort, because the current catalog does not include slot overrides.

## Out of scope

- Adding a role to a running mission, which keeps its own form (`start_chat/add_role.rs`).
- Agent-named mission titles ([#1100](https://github.com/yicheng47/runner/issues/1100)).

## Guide pages

`getting-started/first-chat.md`, `getting-started/first-mission.md`, `concepts/missions.md`, `concepts/crews.md`, `concepts/projects.md`, `guides/chats-and-windows.md`, `guides/remote-access.md`, `guides/drive-runner-from-agents.md` and `reference/keyboard-shortcuts.md`, with their `zh-CN/` counterparts, plus the generated `reference/cli.md`.
