# 735 — Start a chat modal redesign

Implement [P2 #735](https://github.com/yicheng47/runner/issues/735). Jason asked on 2026-09-30 for a claude pair crew mission that ends in an open PR, not a merge.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-735-start-chat-modal`, on branch `feat/735-start-chat-modal`. The mission's directory is this worktree. Its tip is this brief, on top of main `d485284` (the design and spec). Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. If main moves and you need it, rebase onto `origin/main`; never merge main into the branch.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions (a mission lands as one commit).
- **`docs/features/735-start-chat-modal.md`**, especially its Design section and decisions. It wins over the issue and over this brief on any detail.
- **The design**, exported as PNGs in this worktree's ignored `target/design-ref/`: `role-mode.png`, `role-mode-model-override.png`, `role-mode-agent-override.png`, `direct-mode.png`. Match their layout, hierarchy, spacing and colours with the existing theme tokens; the sample data is illustrative. Do not delete them, and do not open or edit `.pen` files.
- `crates/runner-app/src/surfaces/start_chat.rs`: `open_start_chat_modal`, `select_start_chat_choice`, the `sync_*` helpers, `render_start_chat_modal`, `start_chat_focus_order`, `build_start_request`, and the tests.
- What the new card borrows, to share rather than copy: `surfaces/crews/popup.rs` (`override_row`, `inherited_label`, `runtime_note`, the amber dot, `role: <value>` and Reset), `surfaces/roles/detail.rs` (the setup values), `ui/avatar.rs` (`RoleAvatar`), `chat_icon.rs` (`ChatIcon::for_runtime`), `ui/select.rs` (`StyledSelect`, `SelectOption`, `option_menu`), `ui/model_field.rs`, `ui/field.rs`, `ui/overlay.rs`.
- Backend, read only: `resolve_runtime_override` in `session/manager/mod.rs`, and `strip_permission_flags` in `spawn.rs`, which is why the modal shows no permissions.

## Deliverable

1. **Role card** (`role-mode.png`). The top row is the role picker: `RoleAvatar` at 40 px seeded with the handle, display name, `@handle` in mono, and an up-down chevron. Its menu rows show the avatar, display name and `@handle`, and keep `StyledSelect`'s keyboard behaviour. If `SelectOption` needs a leading avatar or provider mark, add it once in `ui/select.rs` for both pickers. Under the picker, read-only Runtime (mark and name), Model and Effort from the role, a role without a model or effort reading `default` dimmed as on the role page; a Codex role adds Speed. The bar "Run on another agent, model or effort" toggles the overrides.
2. **Overrides open** (`role-mode-model-override.png`, `role-mode-agent-override.png`). Runtime select (marks), Model combo and Effort select in the same three columns. On the role's own agent, an untouched field shows the role's value dimmed as `role (xhigh)`; a changed one gets the amber dot and `role: <value>` under it. On another agent, Runtime gets the dot and `role: <runtime>`, Model and Effort show that agent's defaults dimmed, Speed appears when the agent is Codex, and the note says the role's model and effort don't carry over. The bar reads "Overrides apply to this chat only" with "Use role settings", which clears every override, and a chevron that folds. Folding keeps the overrides; folded, the row shows the effective values with an amber dot on each overridden one.
3. **Request mapping, unchanged in shape.** Overrides untouched: `runtime: None`, no model or effort. Model or effort changed on the role's agent: `runtime: Some(role.runtime)` plus the changed values, which `resolve_runtime_override` already resolves by keeping the role's other values. Another agent: `runtime: Some(that)` with its own defaults, as today. Speed as today. Direct mode unchanged. No backend change.
4. **Agent card** (`direct-mode.png`). The top row is the agent picker: the provider mark on a 40 px tile, display name and the command in mono; menu rows with marks. Under it Model, Effort and Speed (Codex only) in one row: Effort about 160 px, Speed about 104 px, Model fills.
5. **Both modes.** The Direct | Role switch stays. Chat name loses its hint line and gains an "optional" tag after the label, placeholder as today. Working directory keeps Browse; the hint reads "Blank starts in the role's directory." in Role mode and "Blank starts in your default directory." in Direct. Footer unchanged. The undrawn states keep today's copy inside the new layout: no roles yet, detecting agents, no enabled agents with its Settings → Agents link, the error banner, and disabled controls while submitting.
6. **Keyboard.** Tab order: close, Direct, Role, the picker, the override bar (Enter or Space toggles), the open controls, Use role settings when shown, Chat name, Working directory, Browse, Cancel, Start chat. Esc still closes.
7. **Narrow windows.** At 640 × 480 the modal keeps its width, scrolls vertically, and nothing clips sideways; long names, handles and paths truncate.
8. **Tests** in `start_chat.rs` beside the existing ones: the three request mappings above; folding keeps overrides and Use role settings clears them; the dimmed role placeholders on the role's agent against the agent defaults on another agent; Speed visibility for Codex roles, Codex overrides and Direct Codex; the focus order including the bar; a headless layout check at 640 px wide (`debug_selector` plus `VisualTestContext`, as the existing modal test does). Update assertions the change invalidates. Gate any test import or helper used only by `cfg(unix)` tests with `cfg(unix)`, since Windows clippy fails on unused ones.
9. **Docs, same diff.** If implementation forces a deviation from the spec, update the spec on this branch and say why in the handoff.

Out of scope: backend or CLI changes, the Role and Crew pages beyond shared helpers, the mission start flow, `.pen` files, README screenshots.

## Validation

Run each of these and report its exit code:

- `cargo test --locked -p runner-app --profile ci --no-fail-fast`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- `cargo fmt --all --check`
- `git diff --check`

Log each to a file and read `$?` from the command itself; never pipe a gate through `tail` or `grep`.

Crews never run the dev app. Do not start, stop, restart or type into Jason's Runner apps, chats or missions, and never open Jason's real Runner database (`~/Library/Application Support/com.wycstudios.runner*`). Jason smoke-tests the UI. Native Windows is unavailable; say what is unverified there.

## Crew handoff and authorization

The coder owns implementation, tests and fixes. The reviewer waits for an explicit Runner handoff, then reviews the whole working-tree diff against the spec, the PNGs and this brief, must-fix findings first with file:line pointers. It checks in particular:

- that a role chat with untouched overrides sends no override, and the other two mappings match item 3;
- that dimmed `role (…)` values appear only on the role's own agent;
- that no permissions value appears anywhere in the modal;
- that Direct mode starts exactly as before;
- the tab order and the 640 × 480 layout.

Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES`. No extra agents, crews or subagents.

After the clean review, and only then, Jason authorizes:

- **One commit.** Squash everything on the branch, this brief included, into a single commit on top of `main`: imperative subject naming the change (for example `feat(ui): redesign the start a chat modal`), no co-author trailers.
- **Push** with `git push -u origin feat/735-start-chat-modal`.
- **Open the PR** with `gh pr create --base main`. The body carries `Closes #735`, a summary, test evidence, a manual check for Jason (a role chat started untouched, with a model-only override, with a Codex override; Use role settings and folding; a Direct chat on Claude Code and on Codex with Speed; no roles; no agents; tab through the whole form; the modal at 640 × 480), what is unverified, and no agent session links.
- **Watch CI** with `gh pr checks <n> --watch` until nothing is pending, on both macOS and Windows. Fold any fix into the commit with `git commit --amend`, have the reviewer check it, and push with `git push --force-with-lease`.

**Do not merge**, delete the branch or worktree, or cut a nightly or release.

The final handoff goes to everyone through Runner: the PR URL and CI result, changed files, checks with exit codes, any spec deviation, what is untested or unverified, and the reviewer's verdict. Then both slots stand by.
