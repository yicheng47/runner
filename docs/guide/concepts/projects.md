# Projects

A project gives a working directory a name and groups its chats and missions in the sidebar. It saves you from choosing the same folder for every launch.

The sidebar defaults to Projects. The list-filter button beside Search switches to Active, which gathers running chats and terminals, missions with live members, and work needing attention across projects; switching views preserves your project organization.

Create a project from the sidebar and choose its directory. New chats and missions started in that project use its directory. You can still supply a different working directory when starting work.

The launcher page starts with the active project, or **No project**, for chats, terminals and missions alike. A project’s **+** shows that page with the project selected; the **+** beside Recents selects **No project**. No tab or project-tree row exists while the launcher is empty: **New chat** is selected in navigation. Starting creates a tab and selected row under the chosen project, or Recents. The project picker and **Change** set the next start’s folder. A real empty pane inside a split has no project/folder control: its chat uses the tab’s project and a neighbouring session’s folder, and its × closes that pane.

Choosing a project fills its folder unless the current folder is already inside it, such as a worktree. **No project** keeps the current folder. Choosing a folder inside another project follows the deepest matching project. A role with its own working directory uses it unless you explicitly choose a folder with **Change**. A mission's members start in the launcher's folder.

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
