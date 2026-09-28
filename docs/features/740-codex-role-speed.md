# 740 — Codex speed in role settings

[Tracking issue #740](https://github.com/yicheng47/runner/issues/740) · P2 · 0.12

## Motivation

Codex Fast mode is a service tier, independent of model and reasoning effort. Today a Runner role needs a raw `-c service_tier=fast` Arg or a global Codex setting to request it. A role should make that choice visible before it starts a chat or crew slot, because Fast mode consumes more credits. Codex's [Speed documentation](https://learn.chatgpt.com/docs/agent-configuration/speed) describes the mode and cost; the installed Codex 0.157.1 reports `fast_mode` enabled.

## Scope

- Add a **Speed** select to Codex role creation and in-place editing: **Inherit** (default), **Standard**, **Fast**. Place it after Model in the create form and after Effort in the edit setup. Show a short hint when Fast is selected: “Fast uses more credits.” Other runtimes do not show the control. Changing a role away from Codex clears the stored choice.
- Show a **Speed** row in the Codex role detail Setup section, below the Model/Effort row. Display the saved choice, including Inherit; Fast retains a visible “more credits” note. The value describes Runner's role choice, not a promise that the model or account will receive Fast service.
- Show **Speed** in the Start Chat modal whenever the selected direct agent or effective role agent is Codex. In Direct mode, Inherit uses Codex config. In Role mode, Inherit uses the role's saved Speed; an explicit Standard or Fast overrides it for that chat. Show the credit note when Fast is effective, and preserve an explicit chat choice on resume. Switching away from Codex clears the modal choice.
- Show **Speed** in a crew slot's override popup when its effective runtime is Codex. Inherit follows the role's saved Speed on the role runtime, or Codex config when the slot switches from another runtime. Standard and Fast override the role for that slot; Fast shows the credit-use note in edit and view. Persist the choice with the slot and the launched session so resume keeps it. Changing the slot or an inheriting role away from Codex clears the slot choice.
- Persist one nullable role setting (`NULL` = Inherit). Existing roles stay Inherit. Create/update/read paths retain the choice without changing model, effort or custom Args.
- At every Codex launch, append an explicit `-c service_tier=default` for Standard or `-c service_tier=fast` for Fast. Inherit appends no service-tier override, allowing the user's Codex config and any manually supplied role Arg to work. Place an explicit choice after role Args so it takes precedence over a manual `service_tier` Arg, including on `resume`. Use the shared spawn path for direct chats and mission slots. A non-Codex effective runtime emits no Speed override.
- No in-terminal `/fast` injection, global Codex config edit, or claimed effective-tier measurement. The [Codex service-tier source](https://github.com/openai/codex/blob/main/codex-rs/protocol/src/config_types.rs) names `default` as the explicit no-tier request; verify the installed CLI accepts it before shipping.

## Implementation phases

1. Add the nullable role field and migration; update role storage and create/update input handling.
2. Add the create/edit select and role detail display in the current Roles page visual language. Keep the new control within the existing form and setup spacing.
3. Add the shared Codex argv override after custom Args, covering fresh and resumed direct and mission sessions. Pass Start Chat choices through direct spawn and persist explicit session choices for resume.

## Verification

- Existing roles load as Inherit. Create, edit, reload and runtime changes preserve or clear Speed as described. Fast's credit note remains visible in edit and detail.
- Start Chat shows Speed for Codex in Direct and Role modes, reflects the role default, resets when the effective agent changes away from Codex, and preserves an explicit choice on resume.
- Crew slot override shows Speed only for an effective Codex runtime, preserves an explicit choice across reload and mission resume, honors role inheritance, and clears when that slot changes away from Codex.
- Test the final argv for Inherit, Standard and Fast; with manual `service_tier` Args; for fresh and resumed direct and mission sessions; and for a non-Codex effective runtime. Confirm `resume <key>` ordering and the first prompt are unchanged.
- Run runner-backend and runner-app tests, workspace Clippy and formatting checks. Inspect the role form and detail in the development app on macOS, and require macOS and Windows CI before the PR handoff.
