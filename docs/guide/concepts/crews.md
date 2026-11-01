# Crews

A crew is a reusable team of roles arranged into named slots, with shared team conventions. Starting a crew creates a mission with a terminal for each slot.

The list starts with 10 crews per page. Use **10 / page** in the footer to choose 10, 25 or 50; Runner remembers this separately from the Roles list. Changing the size keeps your search and returns to page 1. Longer pages scroll while the column headings and footer stay visible.

## Slots

Each slot selects a role and gives it a handle, such as `coder` or `reviewer`. These handles are the names members use when sending messages. They must be unique within the crew, even when two slots use the same saved role.

A launchable crew has at least one slot. Every member receives the mission goal; role prompts and team conventions say who acts first and how members hand work over.

The crew page lets you reorder slots and override an individual slot's agent, model, effort or Codex speed. Inherited values come from the role, subject to the selected runtime's capabilities.

## Team conventions

Team conventions are shared instructions appended for every member in a mission. Use them to establish who edits, who reviews, when to hand work over, what checks are required, and whether commits or other external actions are permitted. They do not apply when the role starts a direct chat.

For example, the default pair asks the coder to implement on a feature branch, the reviewer to inspect the working-tree diff, and both to repeat until review is clean. The default conventions leave changes uncommitted unless the goal explicitly authorizes Git actions.

## Crew versus mission

The crew is the reusable starting configuration. The [mission](missions.md) owns the live work, goal, feed and membership. Adding or removing a member from a mission does not edit the saved crew. A mission can also start as an **Empty mission**, then receive members one at a time.

Follow [Build a crew](../guides/build-a-crew.md) to make your own team.
