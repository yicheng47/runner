# 735 — Start a chat modal redesign

> Tracking issue: [#735](https://github.com/yicheng47/runner/issues/735)
> Priority: P2. Platforms: macOS and Windows.

## Motivation

The redesigned role and crew pages lead with identity and setup: pixel avatars, provider marks, and readable runtime, model, and effort values. Start a chat still presents a dense, generic form. Role mode names choices by `@handle` in a text dropdown, and the selected role's inherited setup is not visible before starting. The Agent field offers an override without showing the role's model, effort, or permissions alongside it. Direct mode uses the same form style and has no visual link to the runtime identity shown elsewhere. The modal should make the chosen chat setup clear at a glance and feel like part of the new Roles and Crews UI.

## Scope

- Design the modal in `design/specs/735-start-chat-modal.pen` using the visual language of the shipped role and crew pages, then implement the approved frames.
- In Role mode, show the chosen role's `RoleAvatar`, display name, and `@handle` in both the picker and selected state. Show its effective setup at a glance: the provider mark, runtime, model and effort. Distinguish inherited values from overrides. Keep the existing ability to switch the agent and set model and effort for that override.
- In Direct mode, show the selected runtime's provider mark and name, with the existing model and effort controls arranged in the same visual language.
- Keep Chat name and Working directory available in both modes, with the effective directory clear when the field is blank. Keep the Direct/Role choice, its remembered mode, role preselection from Chat now, project scope, defaults, and Start chat behavior.
- Preserve loading, empty, disabled, and error states, and keyboard navigation. The modal must scroll vertically and fit Runner's 640 × 480 minimum window without horizontal clipping; long role names and paths truncate or wrap appropriately.
- Presentation only: the existing launch commands and backend behavior stay the same.

## Design

Signed off 2026-09-30 in `design/specs/735-start-chat-modal.pen`, revised the same day after Jason's smoke test of #761: the folded read-only row did not read as editable, and dimmed values read as disabled. Each mode is one card: who the chat is with on top, how it runs underneath, always as controls.

- **Role mode** (`PcFac`). The role card's top row is the role picker: `RoleAvatar` at 40 px, display name, `@handle` and an up-down chevron; its menu lists roles the same way. Under it, the Runtime select, the Model combo and the Effort select in three columns, always editable and filled with the role's values. The field's hint reads "Starts with the role's settings. Change any of them for this chat only."
- **Model changed** (`GdboP`). A changed value carries an amber dot, with `role: <value> ↺ Reset` under the control; Reset restores that one value. Untouched values are unchanged.
- **Agent changed** (`T2swh9`). Runtime shows the new agent with a dot and `role: Claude Code ↺ Reset`; Model and Effort show that agent's own defaults; Speed appears for Codex; a note says the role's model and effort belong to its own agent and don't carry over.
- **Direct mode** (`ouFT7`). The agent card's top row is the agent picker: the provider mark on a 40 px tile, display name and command. Under it, Model, Effort and Speed (Codex only) as controls in one row, filled with the agent's defaults.
- **Both modes.** The Direct | Role switch stays under the header. Chat name carries an "optional" tag and the derived label as its placeholder. Working directory keeps Browse, with a hint naming where a blank field starts: "Blank starts in the role's directory." or "Blank starts in your default directory." Footer: Cancel and Start chat.

Decisions:

- **No permissions value.** Direct chats, whether started from a role or an agent, strip the role's permission flags and run with the agent's own default (feature 596, `strip_permission_flags` in `session/manager/spawn.rs`), so a role's permission mode never applies to a chat. Showing it would be wrong.
- **Nothing is dimmed, nothing is labelled by source.** Every value in a control reads at full contrast, so no field looks disabled. The role's values are only the template the chat starts from, so the controls do not say where a value came from. An override adds the amber dot and the role's value with Reset under the control.
- **No fold, no bar.** The controls are always shown, so there is no collapsed state and no "Use role settings"; each overridden value has its own Reset, as in the crew slot popup.
- **The request does not change.** Untouched controls send no override. Changing only the model or effort on the role's own agent sends the role's runtime as the override plus that value, which `resolve_runtime_override` already treats as keeping the role's other values; the controls show those values, where today's fields show the agent's defaults. Choosing another agent sends it with its own defaults, as today. Speed keeps today's options.
- **States not drawn keep today's copy** in the new layout: no roles yet, detecting agents, no enabled agents with the Settings → Agents link, and the error banner above the form.

Settled in implementation, where the frames were silent:

- **A control showing its starting value is untouched.** Typing the role's own model, or picking its own effort or speed again, sends nothing and shows no dot. The Model control shows the role's model (or the agent's default) as its placeholder at full contrast, so clearing it returns to that value; only a typed model is sent.
- **Another role starts from its own setup.** Choosing a different role clears every override: a model typed for one agent is wrong for another.
- **Effort is changeable without typing a model, except on Antigravity.** Its levels follow the model the chat will run: the one typed, else the role's, else the agent's default model, and every level when none is known. Its starting value is the role's effort on the role's agent, else the agent's default effort, else `default`. The launch gives Antigravity `--effort` only beside a `--model` its catalog pairs with that level (`model_effort_args`), so Antigravity's Effort offers levels only for a typed model or the role's own; its default model in the placeholder is not sent and does not count. Until one is set the control stays disabled rather than showing a level the launch would drop.
- **No choice can undo the role's value.** A role that sets an effort or a speed has no blank Effort or Inherit Speed choice, since the request cannot clear what the role sets. Reset is how to get back.
- **The amber dot sits inside the control, before its chevron, in a slot of its own** on the Runtime, Model, Effort and Speed controls, so a long value ends before it. A text field cannot place it after the typed text. Only Runtime carries it once another agent is chosen; the role's model, effort and speed are not the new agent's to override, and Model and Effort show the agent's own defaults.
- **Each Reset is a tab stop after the controls**, in Runtime, Model, Effort, Speed order, and focus moves to the control it restored because the Reset goes with the override.
- **The Working directory hint follows the role**: a role with no directory of its own starts in "your default directory".

## Non-goals

- Changing how direct chats start or resolve role and runtime defaults.
- Editing roles, creating roles, or changing the Role and Crew pages.
- Changing the mission start flow.

## Implementation phases

1. **Design**: Pencil frames for Role and Direct modes, including selected and empty states, reviewed before code.
2. **Modal**: Update `crates/runner-app/src/surfaces/start_chat.rs` and only the shared UI pieces needed for the approved design.

## Verification

- `runner-app` tests pass, with focused coverage for any changed selection or override behavior; workspace Clippy is clean.
- Manual pass on macOS and Windows: start a preselected role chat, switch roles and modes, override the agent/model/effort, start a direct chat, and check empty runtime/role and launch-error states.
- At 640 × 480, the form remains usable by keyboard and mouse without horizontal scrolling.
