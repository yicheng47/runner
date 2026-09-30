# 768 + 731 — New role and crew pages, role side panel

Implement [P2 #768](https://github.com/yicheng47/runner/issues/768) and the rest of [P2 #731](https://github.com/yicheng47/runner/issues/731). Jason asked on 2026-09-30 for a codex pair crew mission that ends in one open PR, not a merge.

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-768-731-role-crew-ui`, on branch `feat/768-731-role-crew-ui`. The mission's directory is this worktree. Its tip is this brief, on top of main `29beeb9` (the designs and specs). Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory. If main moves and you need it, rebase onto `origin/main`; never merge main into the branch.

## Read first

- `AGENTS.md`, including Worktrees and Crew Missions (a mission lands as one commit).
- **`docs/features/768-new-role-crew-pages.md`** and **`docs/features/731-role-side-panel.md`**. They win over the issues and over this brief on any detail.
- **The designs**, as PNGs in this worktree's ignored `target/design-ref/`: `new-role-page.png`, `new-crew-page.png`, `side-panel-open-role.png` and its crop `side-panel-crop.png`. Match layout, hierarchy, spacing and colours with the existing theme tokens; sample data is illustrative. Do not delete them, and do not open or edit `.pen` files.
- #768: `surfaces/roles/` (`create.rs`, `forms.rs`, `detail.rs` `render_role_edit_page`, `edit.rs`, `list.rs`, `delete.rs`, `mod.rs`), `surfaces/crews/` (`create.rs`, `editor.rs`, `editor_sections.rs`, `slots.rs`, `list.rs`, `add_slot.rs` `create_role_from_add_slot`, `overlays.rs`), `surfaces/app_shell.rs` (`AppRoute`, history), `surfaces/profile_page.rs`, `ui/avatar.rs`.
- #731: `surfaces/panes.rs` (`render_chat_side_panel`, `chat_panel_content`, `side_panel_label`), `profile_page.rs` (`clamped_markdown`, `prompt_meta`, `prompt_preview`), and how `roles/detail.rs` `render_prompt_card` uses them.

## Deliverable

1. **Creating routes** (#768). New routes for the creating pages (for example `AppRoute::NewRole`, `AppRoute::NewCrew`), so the sidebar highlights role or crew and back and forward work. Breadcrumb `Roles › New role` / `Crews › New crew` with a **NEW** tag in `editing_tag`'s style.
2. **New role page** (`new-role-page.png`). The role page's in-place edit layout in a creating state, sharing `render_role_edit_page`'s code rather than copying it: live `RoleAvatar` from the typed handle (empty tile before one is typed), the large Display name field, an editable `@handle` field with the spec's hint and today's handle error in its place, **Create role** (plus icon) and Cancel, the full setup including **Effort**, which create never sent before, and Speed for Codex, the system prompt editor card and caption. No crews list, activity lines or unsaved note. Create lands on the new role's page in view mode; Cancel returns to where New role was opened.
3. **New crew page** (`new-crew-page.png`). The crew page's edit layout in a creating state: empty tile with a `users` glyph, Name field, "No slots yet", **Create crew** and Cancel, "Slots · 0" with Add slot disabled over the spec's two lines, the Team conventions editor (sent as `system_prompt_addendum`). No Missions card or created lines. Create lands on the crew's page in view mode, where Add slot works.
4. **Entry points and removal** (#768). The list buttons, the lists' empty states and Add slot's Create role link open the creating pages. Delete both create modals, their overlay hooks, and form state the pages do not reuse. Keep today's validation, disabled, error and IME-safe Enter behaviour, and a top-to-bottom tab order.
5. **Side panel** (#731, `side-panel-crop.png`). An "Open role ↗" text link at the right end of the ROLE label row, styled like the crew page's "+ Add slot", calling `open_role_detail`; none on runtime chats. The System prompt label row gains `prompt_meta` at its right. The prompt renders through `clamped_markdown` with Show all / Show less. Its fade currently ends in `theme::panel()`; the panel's box is `theme::bg()`, so give the helper the background to fade into without changing the role and crew pages. The panel keeps its own expanded flag, reset when the selected chat changes. No Permissions row.
6. **Narrow windows.** The pages hold at 640 × 480 like the edit pages; the panel holds at its minimum width, long values truncating.
7. **Tests** beside the existing ones: each entry point opens its creating page; the creating states render the spec's pieces and hide the others; Create sends effort and lands on view mode; Cancel returns; the modals are gone; the panel's link opens the role page and is absent on runtime chats; the prompt clamps and toggles, and its flag resets on a new chat. Update assertions the change invalidates. Gate any test import or helper used only by `cfg(unix)` tests with `cfg(unix)`, since Windows clippy fails on unused ones.
8. **Docs, same diff.** If implementation forces a deviation from a spec, update that spec on this branch and say why in the handoff.

Out of scope: backend or CLI changes beyond sending `effort`; adding slots before a crew exists; the list pages, Start a chat and the mission workspace; `.pen` files; README screenshots.

## Validation

Run each and report its exit code:

- `cargo test --locked -p runner-app --profile ci --no-fail-fast`
- `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`
- `cargo fmt --all --check`
- `git diff --check`

Log each to a file and read `$?` from the command itself; never pipe a gate through `tail` or `grep`.

Crews never run the dev app. Do not start, stop or type into Jason's Runner apps, chats or missions, and never open his real Runner database (`~/Library/Application Support/com.wycstudios.runner*`). Jason smoke-tests the UI. Native Windows is unavailable; say what is unverified there.

## Crew handoff and authorization

The coder owns implementation, tests and fixes. The reviewer waits for an explicit Runner handoff, then reviews the whole working-tree diff against both specs, the PNGs and this brief, must-fix findings first with file:line pointers. It checks in particular: no create modal remains reachable; the creating and edit states share one render path; Create role sends effort; back and forward across the new routes; the panel link, the clamp's fade colour, and the flag reset. Iterate through Runner until the reviewer posts `NO REMAINING MUST-FIX ISSUES`. No extra agents, crews or subagents.

After the clean review, and only then, Jason authorizes:

- **One commit.** Squash everything on the branch, this brief included, into a single commit on top of `main`, imperative subject (for example `feat(ui): create roles and crews on their pages; role side panel prompt`), no co-author trailers.
- **Push** with `git push -u origin feat/768-731-role-crew-ui`.
- **Open the PR** with `gh pr create --base main`. The body carries `Closes #768` and `Closes #731`, a summary, test evidence, a manual check for Jason (the spec docs' manual passes), what is unverified, and no agent session links.
- **Watch CI** with `gh pr checks <n> --watch` until nothing is pending, on macOS and Windows. Fold any fix into the commit with `git commit --amend`, have the reviewer check it, and push with `git push --force-with-lease`.

**Do not merge without Jason's explicit request**, clean up local branches or worktrees without explicit authorization, or cut a nightly or release. An authorized merge includes GitHub's configured automatic deletion of the merged remote branch; no separate confirmation is needed.

On 2026-10-01, Jason confirmed all smoke tests passed and requested the merge. He clarified that the remote branch does not need to be retained and asked for this authorization wording to be corrected in the same PR.

The final handoff goes to everyone through Runner: the PR URL and CI result, changed files, checks with exit codes, any spec deviation, what is unverified, and the reviewer's verdict. Then both slots stand by.
