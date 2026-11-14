# Projects

A project gives a working directory a name and groups its chats and missions in the sidebar. It saves you from choosing the same folder for every launch.

## Mission Control

**Mission Control** in the sidebar reads a project's local `issues/` tickets and
`.crew/triage/` evidence alongside Runner's existing mission state. It is a
compact, horizontally scrolling board: all seven derived lanes remain visible,
including empty ones, and cards emphasize their lane, ticket details and any
pending question before terse work and source evidence. It is read-only: cards
link to their source evidence and to a linked mission's existing workspace, but
it cannot create or edit tickets, triage, questions, branches or worktrees.
Missing, malformed or stale local sources show scoped warnings while the
remaining evidence stays available.

The sidebar defaults to Projects. The list-filter button beside Search switches to Active, which gathers running chats and terminals, missions with live members, and work needing attention across projects; switching views preserves your project organization.

Create a project from the sidebar and choose its directory. New chats and missions started in that project use its directory. You can still supply a different working directory when starting work.

**⌘N** / **Ctrl+N** and **New tab** start the launcher page with the recent tab’s project in an editable picker and that project’s folder, for chats, terminals and missions alike. While the page already shows, they use the tab the page opened from, or the most recently viewed tab if that tab is gone. A tab in Recents, or no tab, gives **No project**; the **+** beside Recents always starts with **No project**. The dashed row stays in Recents, without selecting or expanding the preselected project in the sidebar. Remote computers use their own tabs and projects. **⇧⌘M** / **Ctrl+Shift+M** and starting from a crew start it with the active project. A project’s **+** shows that page locked to the project, expanding the project if it is collapsed: the picker is disabled, and a folder chosen with **Change** stays filed under the project. If that project is deleted while the page is open, the page goes back to **No project**, unlocked. While the launcher shows, a dashed **New tab** row marks it, under the project whose **+** opened it, otherwise in Recents. It stays there whatever the picker or **Change** then do, and what you start files under the project the picker shows. The row is not a saved tab: starting replaces it with the new tab’s row, and leaving the launcher removes it. A real empty pane inside a split has no project/folder control: its chat uses the tab’s project and a neighbouring session’s folder, and its × closes that pane.

Choosing a project fills its folder unless the current folder is already inside it, such as a worktree. **No project** keeps the current folder. On a page that is not locked, choosing a folder inside another project follows the deepest matching project. A role with its own working directory uses it unless you explicitly choose a folder with **Change**. A mission's members start in the launcher's folder. After choosing a project or dismissing its menu with Esc, focus returns to the launcher’s search so ↑/↓ move through the rows.

The CLI can create the same association:

```sh
runner project create "My app" --path /absolute/path/to/my-app
runner chat start --runtime codex --project "My app"
runner mission start --crew "Pair coding crew" --project "My app" --title "README review" --goal "Review the README for accuracy."
```

On Windows, use an absolute Windows path such as `C:\Users\you\src\my-app`. Quote paths containing spaces.

## Directory and grouping

From a shell, a chat or mission started inside a registered project's directory is associated with that project automatically. `--project` explicitly selects a project and its directory; `--cwd` supplies a launch directory instead. These two flags cannot be combined. The CLI's directory defaults to the shell's current directory, so check where you are before starting a mission.

A project is a directory association, not a sandbox or an isolated Git checkout. Several sessions can edit the same files. To work in a separate worktree, create that worktree first and launch with its path.

Moving an existing mission into another project changes its sidebar grouping; it does not move repository files or change a running process's directory.

## Removing a project

Deleting a project archives its members; it does not delete the directory on disk. The CLI refuses deletion with running members unless `--force` is supplied. Stop and review the affected work before deleting the grouping.

See [CLI reference](../reference/cli.md#runner-project) for project commands.
