# Missions

A mission holds agents working on one goal, their terminals, and a shared record of messages and questions. Start from a saved crew, or create an empty mission and add workers as needed.

## Goal and membership

For a crew mission, Runner starts a session per slot and delivers the goal to the lead. Members receive their role instructions, team conventions and coordination instructions. The mission captures its own roster, so the crew remains a reusable template.

With **No crew**, the mission starts without sessions or a lead. Add a role from the mission and give it a task. Each joins as a worker and receives the shared mission context. The person controlling the mission can add or remove workers while it runs; a crew lead cannot be removed. Removing a worker stops its session but keeps the mission's history.

The mission's working directory is shared by its members when supplied. Runner does not make branches, worktrees or copies of your repository. Set up any isolation you need before launch and describe it in the goal.

## Feed and terminals

The feed records messages, questions, answers and lifecycle events in a persistent event log. A member can address another member by handle, post to the crew, or ask the person for a decision. Runner delivers coordination messages to the relevant agents; their own terminals retain the detailed work and output.

Answer a question through its feed card. For an ordinary follow-up, use the message composer. Type `@` at the start and pick a member to add a recipient chip; repeat to choose several. Click a chip to remove it, or press Backspace in an empty draft to remove the last chip. With no chips or leading mentions, the message broadcasts to the crew. Only its recipients receive inbox notifications. Mentions inside the text remain text. You can also [use the CLI](../guides/drive-runner-from-agents.md) to read the feed, send a message or answer a pending question.

## Permissions

Mission launches use the runtime's broad-access mode, generally **Bypass**; pi uses a trusted workspace. A role's more restrictive direct-chat permission setting does not turn a mission into an approval-by-approval workflow. Write clear limits in the goal, and choose the working directory accordingly.

## Stop, resume and archive

- **Stop a slot** to halt just that agent. **Resume** continues its saved conversation; **Restart** starts that mission slot with a fresh conversation.
- **Stop the mission** to halt its members. **Resume** resumes stopped sessions.
- **Archive** stops active members and removes the mission from active lists. Its feed remains available under **Settings → Archived**. Restore it before resuming work.

While a mission is paused, its unread-response green dot is hidden in the sidebar and collapsed groups. Unread responses remain saved, and the dot returns after resuming if any remain unread. Viewing a role's session still marks that session's response as read while paused. Error and approval indicators keep their usual behavior.

A “done” message or an idle agent does not automatically mean all sessions have exited. Stopping and archiving never revert repository changes.

Closing Runner can leave missions working in its background service. **Settings → General → When Runner quits** controls that choice. See [Sessions](sessions.md) for relaunch and recovery behavior.
