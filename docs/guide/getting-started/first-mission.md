# Your first mission

Run the default pair-coding crew on a small task: the coder implements it, the reviewer checks the diff, and their conversation appears in one shared feed.

## Prepare the crew and folder

On a fresh installation, Runner seeds **Pair coding crew**, with **@coder** as lead and **@reviewer** as the second slot. Both use Codex, so first install Codex, sign in, and confirm it works in a [chat](first-chat.md). Open **Crews**, select **Pair coding crew**, and check both slots. Existing installations with roles or crews may not receive the seed; [Build a crew](../guides/build-a-crew.md) explains how to create the same pair or choose different agents.

Choose a small repository or a disposable project for the first task. Runner starts the crew in the directory you give it. It does not create an isolated copy or Git worktree for you; all members can see and change that directory.

> [!WARNING]
> Mission agents run with broad access: supported runtimes use Bypass permissions, while pi uses a trusted workspace. Agents can edit files and run commands without the approvals you may see in a direct chat. Choose a folder and goal that you are ready to let the crew work on.

## Start a mission

1. Press **⇧⌘M** / **Ctrl+Shift+M**, or start a mission from the crew's page.
2. Select **Pair coding crew**. The dialog shows its roster, lead and agent settings.
3. Give it a title such as “Explain the project.”
4. Enter a concrete **Goal**, for example:

   ```text
   Create a short PROJECT-TOUR.md explaining the top-level folders in this repository. Inspect the files before writing. The reviewer should check every claim against the repository and send corrections to the coder. Make no code changes, commits, pushes or pull requests. Leave the final diff for me to inspect.
   ```

5. Set **Working directory** to the repository, then click **Start mission** or press **⌘Enter** / **Ctrl+Enter**.

The lead receives the goal. The other slot gets its role and team conventions and waits for work from the lead. An optional blank goal is useful for setting up a mission first, but a first task is easier to follow with the goal written at launch.

## Follow the work

The **Feed** shows messages and coordination events. Open a slot's terminal to see its full agent interface; return to the feed to follow the team. **⌘[** / **Ctrl+[** and **⌘]** / **Ctrl+]** cycle through the feed and open session tabs.

Use the mission's message composer to give the crew a correction or new constraint. Start with `@` and choose a member to add a recipient chip; repeat for several members. Remove a chip by clicking it. A message with no chips or leading mentions broadcasts to everyone; addressed messages wake only their recipients. If an agent asks you a question, its question card shows the available choices; select an answer there. Ordinary agent approval prompts, when present, appear in the agent terminal rather than as a feed question.

The seeded conventions ask the coder and reviewer to repeat their review until no must-fix findings remain. A message saying the work is done does not itself stop the processes. Inspect the final diff and the reported checks yourself.

## Stop and archive

Use the mission's **Stop** control when you are finished. **Resume** brings stopped members back to their conversations. You can also stop or resume one slot through its own controls without stopping the others.

Archive the mission when you want it out of the active sidebar. Archived missions are available in **Settings → Archived**, and restoring one is separate from resuming it. The CLI equivalents are:

```sh
runner mission list
runner mission stop <mission-id>
runner mission archive <mission-id>
```

Replace `<mission-id>` with the ID from the list. Stopping or archiving a mission does not undo the crew's file changes.

Next: [how missions work](../concepts/missions.md), or [build your own crew](../guides/build-a-crew.md).
