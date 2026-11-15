# Build a crew

Create reusable roles, put them into a crew, and write a clear handoff between them. A coder and a reviewer are enough for a first team.

## Create the roles

Open **Roles** and create two roles. Choose an installed, signed-in agent for each. You can use one runtime for both or mix providers; each needs its own working installation.

| Role | Suggested instructions |
| --- | --- |
| Coder, handle `coder` | Implement the requested change, match local patterns, run relevant checks, and fix review findings. Stay within the goal. |
| Reviewer, handle `reviewer` | Read the brief and working-tree diff. Report must-fix defects with file and line references, or state that the review is clean. Do not edit files. |

If these handles already exist, use the saved roles or choose different handles. Keep task-specific requirements out of the role prompt so it also makes sense in a direct chat.

## Assemble the crew

1. Open **Crews**, create a crew and give it a name.
2. Add the coder and reviewer roles. Give their slots distinct handles.
3. Review the runtime, model and effort for each slot. Leave values inherited unless this crew needs an override. If you change the runtime, recheck model and effort for that runtime.
4. Add team conventions, for example:

   ```text
   The coder implements; the reviewer waits for a handoff. Before editing, the coder checks repository instructions and existing changes. The coder sends the reviewer a summary, changed files and checks run. The reviewer checks the working-tree diff without editing. Repeat until the review is clean. Leave all changes uncommitted unless the mission goal explicitly authorizes commits or a pull request. Ask the person if a decision changes scope.
   ```

Team conventions apply to every member in a mission. Role prompts describe responsibilities; conventions describe how those responsibilities fit together.

## Give the crew a goal

Start the mission with a specific outcome, the working directory, checks to run, and any permission to commit, push or publish. For example: “Add a regression test for the reported parsing bug, fix it, and have the reviewer verify both. Leave the changes uncommitted.”

Runner does not isolate working directories automatically. If the team should work in a Git worktree, create it before launch and select its path. Avoid assigning overlapping edits to several workers without a clear owner.

## Use the examples

### Starter pair-coding crew

Fresh installations no longer seed a crew blindly before any agent is installed. Instead, first-run **Setup** offers **Pair coding**, a two-role loop that pairs two distinct eligible runtimes when possible (preferring your default and signed-in agents) or falls back to one runtime for both slots. Switch either slot to another installed agent at any time in **Crews**, or reopen **Open setup** from **Settings → Agents** when no crews exist.

| Role | Responsibility | System prompt |
| --- | --- | --- |
| **@coder** | Creates the task branch, implements the change, runs checks, hands the diff to the reviewer and fixes findings. | [coder.md](https://github.com/yicheng47/runner/blob/main/examples/pair-coding/coder.md) |
| **@reviewer** | Reviews the working-tree diff and reports must-fix issues with file and line references, without editing code. | [reviewer.md](https://github.com/yicheng47/runner/blob/main/examples/pair-coding/reviewer.md) |

The [team conventions](https://github.com/yicheng47/runner/blob/main/examples/pair-coding/team-conventions.md) require a task branch and a clean review before committing. Changes stay uncommitted unless the mission goal explicitly authorizes Git actions; merging always requires the person's instruction.

### More crews

The repository includes ready-to-copy prompts and conventions:

- [Pair coding](https://github.com/yicheng47/runner/tree/main/examples/pair-coding): the seeded coder/reviewer loop.
- [Codex trio](https://github.com/yicheng47/runner/tree/main/examples/codex-trio): coder, reviewer and QA; QA builds the reviewed revision and tests it in the live app.
- [Development crew](https://github.com/yicheng47/runner/tree/main/examples/dev-crew): architect, implementer and reviewer.
- [Documentation crew](https://github.com/yicheng47/runner/tree/main/examples/docs-crew): architect, writers and editor.
- [Tic-tac-toe](https://github.com/yicheng47/runner/tree/main/examples/tic-tac-toe): two players and a referee.
- [Werewolf](https://github.com/yicheng47/runner/tree/main/examples/werewolf): six players and a moderator.
- [Tomb raid](https://github.com/yicheng47/runner/tree/main/examples/tomb-raid): a four-person heist crew run by a game master.

Copy the role prompts into roles and the conventions into the crew. The example goals are starting points to replace with your task. The links above show the current examples on `main`; use the repository's tag selector for examples matching an older release.

See [Your first mission](../getting-started/first-mission.md) to launch and follow the team.
