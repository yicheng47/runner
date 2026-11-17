# Your first mission

Run the default pair-coding crew on a small task: the coder implements it, the reviewer checks the diff, and their conversation appears in one shared feed.

## Prepare the crew and folder

On a fresh installation, **Setup** offers to create the **Pair coding** starter crew, with **@coder** and **@reviewer** as its slots, pairing two distinct eligible agents when available (or using your available agent for both). Finish **Setup**, or open **Crews** to create a crew or inspect **Pair coding**; if no crews exist yet, **Mission** in the launcher shows **No crews yet** with **Create a crew** and **Open setup**. [Build a crew](../guides/build-a-crew.md) explains how to configure slots or create your own roles from scratch.

Choose a small repository or a disposable project for the first task. Runner starts the crew in the folder and checkout you pick: stay on **Main checkout**, pick an existing worktree, or choose **New worktree…** in the launcher’s **Checkout** control to isolate the mission in a linked worktree under `.worktrees/`. All members inside that mission still share the chosen directory and can see and edit each other's changes.

> [!WARNING]
> Mission agents run with broad access: supported runtimes use Bypass permissions, while pi uses a trusted workspace. Agents can edit files and run commands without the approvals you may see in a direct chat. Choose a folder and goal that you are ready to let the crew work on.

## Start a mission

1. Press **⇧⌘M** / **Ctrl+Shift+M**, or choose **Mission** on the launcher page. The page shows its crew step without creating a tab: a **Mission** chip in the search field and your crews, most recently used first. **Start mission** on a crew’s page skips to the next step with that crew picked.
2. Pick **Pair coding**. The search field becomes the goal, with the crew's chip; its slots are listed below with each role and model.
3. Type a concrete goal, for example:

   ```text
   Create a short PROJECT-TOUR.md explaining the top-level folders in this repository. Inspect the files before writing. The reviewer should check every claim against the repository and send corrections to the coder. Make no code changes, commits, pushes or pull requests. Leave the final diff for me to inspect.
   ```

   **⇧Enter** / **Shift+Enter** adds a line; the field grows to six lines, then scrolls.

4. Check the project, folder and checkout below the goal. The project picker, **Change** and **Checkout** work as they do for a chat; choose **No project** to list the mission under Recents.
5. Press **Enter** or click **Start mission**. Runner creates the mission tab and selects its row under the chosen project, or Recents.

Every member receives the goal, its role and the team conventions. The coder starts implementation; the reviewer waits for the coder’s handoff as the conventions require. The mission's title is the goal's first non-empty line, up to 60 characters, or the crew's name and the date when there is no goal; rename it later from the mission's menu. A blank goal is useful for setting up a mission first, but a first task is easier to follow with the goal written at launch.

On the crew step, Esc, Backspace in an empty field or the chip’s × returns to the default launcher page. On the goal step, Esc, empty-field Backspace or the crew chip’s × goes back to the crews. Esc on the default page returns to your previous tab/view. The page has no pane header or pane-close button. A crew with no roles is dimmed with “No roles yet”; picking it opens its page.

## Follow the work

The **Feed** shows messages and coordination events. Open a slot's terminal to see its full agent interface; return to the feed to follow the team. **⌘[** / **Ctrl+[** and **⌘]** / **Ctrl+]** cycle through the feed and open session tabs.

Use the mission's message composer to give the crew a correction or new constraint. Start with `@` and choose a member to add a recipient chip; repeat for several members. Remove a chip by clicking it. A message with no chips or leading mentions broadcasts to everyone; addressed messages wake only their recipients. If an agent asks you a question, its question card shows the available choices; select an answer there. Ordinary agent approval prompts, when present, appear in the agent terminal rather than as a feed question.

The seeded conventions ask the coder and reviewer to repeat their review until no must-fix findings remain. A message saying the work is done does not itself stop the processes. Inspect the final diff and the reported checks yourself.

## Stop and archive

Use the mission's **Stop** control when you are finished. **Resume** brings stopped members back to their conversations. You can also stop or resume one slot through its own controls without stopping the others.

Archive the mission when you want it out of the active sidebar. When archiving empties a Runner-created `.worktrees/` folder, the confirmation offers to remove the clean folder while keeping its branch. Archived missions are available in **Settings → Archived**, and restoring one reopens it with its members stopped (or offers **Restore in main checkout** if a missing worktree’s branch is gone or ambiguous). Use **Resume** afterward to continue their conversations. The CLI equivalents are:

```sh
runner mission list
runner mission stop <mission-id>
runner mission archive <mission-id>
```

Replace `<mission-id>` with the ID from the list. Stopping or archiving a mission does not undo the crew's file changes.

Next: [how missions work](../concepts/missions.md), or [build your own crew](../guides/build-a-crew.md).
