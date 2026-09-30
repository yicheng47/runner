# 756 — Model and effort in the chat side panel

Implement [#756](https://github.com/yicheng47/runner/issues/756). Jason asked on 2026-09-30 for a `codex pair` crew mission that ends in an open PR, not a merge.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-756-chat-side-panel-model-effort`, on branch `feat/756-chat-side-panel-model-effort`, created from `origin/main` at `522920d`. Its tip is this brief. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. A `codex pair` mission for #762 is running in `.worktrees/fix-762-start-chat-model-cache`; treat it as another machine's checkout.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions (a mission lands as one commit).
- The spec, `docs/features/756-chat-side-panel-model-effort.md`. It is the source of truth.
- The design, exported at 2x into `target/design-ref/`: `role-chat-screen.png` (the window with the panel open), `role-chat-panel.png` and `runtime-chat-panel.png`. The panel is 320 px wide at 1x. You cannot read the `.pen` file; these PNGs are the design.
- `crates/runner-app/src/surfaces/panes.rs`: `render_chat_side_panel` and its helpers (`runtime_badge`, `side_panel_label`, `side_panel_key`, `side_panel_value`, `side_panel_row`). The panel reads `active_chat_detail`, loaded through `ops::session::session_get` (`surfaces/chat.rs`).
- UI to reuse: `ui/avatar.rs` `RoleAvatar`; `surfaces/start_chat.rs` `runtime_mark` (the provider mark, on a raised tile from 24 px up); `surfaces/roles/detail.rs` `setup_row`, `setup_value`, `column_text`; `surfaces/roles/logic.rs` `role_setting_label`; `surfaces/profile_page.rs`.
- Backend: `ops/session.rs` `DirectSessionEntry`, `direct_entry_from_repo`, `session_get`; `repo/session.rs` `DirectSessionRow`, `DIRECT_EXTRAS`; `session/manager/mod.rs` `resolve_runtime_override`; `session/manager/spawn.rs`, where a role chat stores `agent_model`/`agent_effort` only when overridden (around line 1674) and resume re-resolves them (around line 2548).

## Deliverables

1. **Effective model and effort on the entry.** Add `agent_model` and `agent_effort` (`Option<String>`) to `DirectSessionEntry`. They must equal what the chat runs on resume:
   - Runtime-only chat: the row's values.
   - Role chat: `resolve_runtime_override(role, row.agent_runtime, row.agent_model, row.agent_effort)`, taking `.effective` or else the current role, which is the resolution resume uses. Do not use a plain `row.or(role)` fallback: an overridden row stores the effective values, including `None`, and a role chat without an override follows later edits to the role.
   - Blank strings become `None`.
   - Filling them on the `session_get` path is enough, as `agent_session_key` already does; the list path may return `None`. Say which paths fill them in the field docs. Do not add one query per row to the list path.
   - Backend tests: a runtime chat with and without values; a role chat without an override (follows the role, including after a role edit); a model-only override; a runtime override to another runtime.
2. **Panel, per the design.**
   - **Identity**, a 40 px leading element. A role chat shows `RoleAvatar::new(handle, 40.)`, then the display name (`text_title`, semibold) over `@handle` (mono, muted). A runtime chat shows the provider mark tile (`runtime_mark(runtime, 40.)`) and the runtime's display name. The uppercase `runtime_badge` goes; delete it if nothing else uses it.
   - **Setup**, under the identity, label over value as on the role page. A role chat shows **Runtime** (the 12 px mark plus the display name), then **Model** and **Effort** side by side in two equal columns. A runtime chat shows only Model and Effort, since its identity already names the runtime.
   - **Runtime and command source.** The Runtime row and the `cmd` row use the entry's effective `agent_runtime` and `agent_command`, not `role.runtime` or `role.command`: a chat with a runtime override runs a different agent from its role.
   - **Values.** An unset value reads `default` in the faint UI font (`role_setting_label`); a set value is monospace in the text color.
   - **Below the divider:** `cmd`, `cwd` and `session_key` with its copy button, otherwise unchanged. The System prompt section is unchanged.
   - **Narrow widths.** The panel goes down to 200 px (`CHAT_PANEL_MIN`). Names, the handle, the runtime name and model values truncate on one line instead of wrapping or overflowing. A truncating text inside a `min_w(0)` column first measures at 0 px and renders `…`; give it an explicit width, as `column_text` does.
   - **Reuse helpers, don't copy them.** If a helper you need is private to another surface, move it to a shared module. `start_chat.rs` is also edited by the running #762 mission, so keep the edit there to the move and its import, and expect to rebase if #762 lands first.
3. **App tests.** Extend the side panel's tests in the existing style:
   - A role chat shows the avatar, the runtime and both values.
   - A runtime chat shows the mark, has no Runtime row, and shows `default` when unset.
   - A runtime-overridden role chat shows the override's runtime and command.
   - Assert on rendered output, not stored state.
4. **Docs.** Do not edit design files. Change the spec only where the implementation had to deviate, and say why in the PR. Leave `README.md` and `README.zh-CN.md` alone unless they describe the panel.

## Boundaries

Crews never run the dev app or drive Jason's Runner for UI checks; Jason smoke-tests. Do not start extra agents, crews or subagents.

## Review, verification and authorization

The coder owns implementation and checks. The reviewer waits for an explicit Runner handoff, then reviews the full branch diff against the spec and the PNGs, must-fix findings first with file:line pointers. Focus on:
- the effective model, effort, runtime and command matching resume;
- the unset rendering;
- truncation at 200 px;
- helpers moved, not duplicated.

Iterate until the reviewer posts `NO REMAINING MUST-FIX ISSUES`.

Run and record exact commands and exit codes:
- `cargo test --locked -p runner-app --profile ci`
- `cargo test --locked -p runner-backend --profile ci`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- the same Clippy with `--features updater`
- `cargo fmt --all --check`
- `git diff --check`

Imports, statics and helpers used only by `#[cfg(unix)]` tests get `#[cfg(unix)]` themselves; macOS Clippy cannot catch this and Windows CI will.

After a clean review, Jason authorizes squashing all work on this branch, this brief included, into one commit on top of current `origin/main`. Its subject names the change, for example `feat(ui): show model and effort in the chat side panel`. Then push `feat/756-chat-side-panel-model-effort` and open a PR against main whose body says `Fixes #756`. If main has moved, rebase; never merge main into the branch. Amend review or CI fixes after the push into the same commit and push with `git push --force-with-lease`. Drive CI green on macOS and Windows. Do not merge, delete the branch or worktree, or cut a nightly or release.

Final Runner handoff: the PR URL, what changed, tests with exit codes, the CI result, the reviewer's verdict, and what Jason should smoke-test. Then stand by. The smoke test:
- a role chat with a model and effort set, on the role or through Start Chat;
- a role chat started with an agent override to Codex;
- a runtime chat with nothing chosen;
- the panel dragged to its narrowest;
- light theme.
