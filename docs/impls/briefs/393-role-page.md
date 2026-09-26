# 393 — Role page and role list

Implement [P1 #393](https://github.com/yicheng47/runner/issues/393), a 0.12 release blocker. Jason asked on 2026-09-26 for a claude pair crew mission that ends in an open PR, not a merge.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-393-role-page`, on branch `feat/393-role-page`. The mission's directory is this worktree. Its tip is this brief, on top of main `f99b760` (the design and spec). Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. If main moves and you need it, rebase onto `origin/main`; never merge main into the branch.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions.
- **`docs/features/393-role-page.md`**: the spec. It wins over the issue and over this brief on any detail. The issue body still calls the role list a non-goal; the spec, updated 2026-09-26, puts it in scope.
- **The design**, exported as PNGs in this worktree's ignored `target/design-ref/`: `role-page.png`, `role-page-editing.png`, `role-list.png`. Match their layout, hierarchy, spacing and colours with the existing theme tokens; the sample data is illustrative. Do not delete them, and do not open or edit `.pen` files.
- `crates/runner-app/src/surfaces/roles/`: `detail.rs` (`render_role_detail_body`), `list.rs` (`render_roles_page`, `render_role_card`), `forms.rs` (`open_role_edit`), `edit.rs` (`submit_role_edit`, `render_role_edit_drawer`), `menu.rs`, `mod.rs`, `tests.rs`.
- `crates/runner-app/src/ui/list.rs` (`PaginatedListPage`, `Pager`, the count label), `ui/avatar.rs` (`RoleAvatar`), `chat_icon.rs` (runtime to provider mark), `surfaces/mission_markdown.rs` (`render_markdown`), and `surfaces/crews/slots.rs:320`, where a crew slot's Edit role opens the drawer.

## Deliverable

1. **Role page, profile split** (`role-page.png`). A fixed left column (272 px in the design) and the prompt card filling the right. Left, top to bottom: `RoleAvatar` seeded with the handle at the large size, display name, `@handle`, Chat now (primary) and Edit, the setup rows (Runtime with its mark, Model and Effort side by side, Permissions, Command, Working directory), Crews using this role (small avatar, crew name, LEAD, `as @slot`, chevron; a row opens the crew), then the activity lines (live counts, last seen, created, short id). The Chat now card goes. The caption under the prompt replaces the stale hint: "Used in every chat and crew slot. A crew adds its own conventions; a slot can override runtime, model and effort."
2. **Prompt card.** The prompt renders as markdown through `render_markdown`. The header shows `N lines · X KB`. The body is clamped to a few lines with a fade and a "Show all N lines" toggle that expands and collapses; it is collapsed by default and when another role opens. An empty prompt shows a short empty line, not a blank card.
3. **Edit in place** (`role-page-editing.png`). Edit swaps the page into its form: an EDITING tag after the breadcrumb; the name as a field; the handle read-only with "The handle can't change."; Save (primary) and Cancel in place of Chat now and Edit; an "Unsaved changes" line while dirty; selects for runtime, model, effort and permissions with the permission hint; command (disabled, as today), args, and working directory with browse; crews and activity dimmed. The prompt card becomes a mono editor at full column height, with a Markdown | Preview switch in its header that opens on Markdown; Preview renders the draft. The card border stays neutral. Reuse the drawer's form state, validation, errors and submit (`open_role_edit` and `submit_role_edit` with no slot); do not fork the save path. Cancel, Esc and a dirty form behave as the drawer does today. The role page never opens the drawer, but the drawer stays for the crew slot's Edit role, with its overrides, until #699 replaces it. The list menu's "Edit details" opens the role page in edit mode.
4. **Role list as a table** (`role-list.png`, spec "Role list as a table"). No panel: a rule under the header row and hairlines between rows. Columns: Role (avatar, name, handle), Runtime (mark and name), Model, Effort, Crews, Last active. An unset model or effort reads `default`; zero crews shows `—`. A live role shows its counts in the accent (`2 sessions · 1 mission`, as the card says today); otherwise Last active is `last_started_at` as `Sep 23, 17:41`, or `—` if it never ran. A row opens the role and keeps the cards' focus, Enter and Space behaviour. Hovering a row turns its Chat icon into a Chat button; ⋯ keeps its menu. New subtitle from the spec.
5. **Count and pager** in `PaginatedListPage`: the pager stays pinned at the bottom, centred under its rule, and the count moves from beside search to the left end of the pager row. It reads `9 roles`, and `3 of 9 roles` only while a search is active. This changes the crew list too; that is intended.
6. **Minimum width.** Find the minimum window size and sidebar width, and make both pages fit it with no horizontal scroll: long names, handles, commands and directories truncate, and the table's Role column takes the slack.
7. **Tests** in `surfaces/roles/tests.rs` and `ui/list.rs`: the count text with and without a search; the list cell values (`default`, `—`, live counts against a date); the prompt meta; the clamp collapsed by default and reset on a role change; entering edit mode, Cancel, and Save going through the shared submit; "Edit details" opening edit mode; the crew slot path still opening the drawer. Update assertions the change invalidates. Gate any test import or helper used only by `cfg(unix)` tests with `cfg(unix)`, since Windows clippy fails on unused ones.
8. **Docs, same diff.** If implementation forces a deviation from the spec, update the spec on this branch and say why in the handoff.

Out of scope: the crew page (#699), the crew list beyond the shared count change, the create drawer, backend or data changes, new fields, `.pen` files, README screenshots.

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

- that edit in place saves through the drawer's submit, and a crew slot's Edit role still opens the drawer with its overrides;
- that a plain-text prompt with no markdown still reads as paragraphs, and the clamp never hides the toggle;
- that the count reads right on the role and crew lists, searching and not;
- that nothing in the diff changes the backend or stored data.

Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES`. No extra agents, crews or subagents.

After the clean review, and only then, Jason authorizes:

- **Commit** in focused commits on this branch: imperative subject, scope `ui` or `docs`, no co-author trailers. Keep the brief commit.
- **Push** with `git push -u origin feat/393-role-page`.
- **Open the PR** with `gh pr create --base main`. The body carries `Closes #393`, a summary, test evidence, a manual check for Jason (a role with a prompt of several hundred lines, more than eight roles with some live, an edit saved and one cancelled, a crew slot's Edit role, both pages at the minimum window width), what is unverified, and no agent session links.
- **Watch CI** with `gh pr checks <n> --watch` until nothing is pending, on both macOS and Windows. Fix any failure on the branch, have the reviewer check the fix, and push again.

**Do not merge**, delete the branch or worktree, or cut a nightly or release.

The final handoff goes to everyone through Runner: the PR URL and CI result, changed files, checks with exit codes, any spec deviation, what is untested or unverified, and the reviewer's verdict. Then both slots stand by.
