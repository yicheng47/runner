# 606 — Mission 1: one liveness rule for rail glyphs, and a folder that opens

Jason requested a Codex crew mission on 2026-09-16. Work in `/Users/jason/repos/yicheng47/runner` on the existing branch `feat/606-rail-glyphs`, created from `main` at `1f647cc` with this brief as its only commit. Build on it; do not create another branch, rebase, or squash.

Read first: this brief; `docs/features/archive/606-rail-glyph-liveness.md` (binding; the design frame `Spec — Rail glyphs (606) · v1`, `Xk8NM`, on `design/runner.pen` shows the before and after rails and the folder pair, and Jason signed off the lucide `folder` / `folder-open` outline pair on 2026-09-16); `docs/features/593-provider-chat-icons.md` for the provider-mark rule this generalizes; then the code: `crates/runner-app/src/surfaces/sidebar/elements.rs` (`sidebar_icon`), `sidebar/rows_render.rs` (the project header at its two `folder-code.svg` sites, normal and inline-rename; the mission row at its two `flag.svg` sites; the tab leaf through `sidebar_tab_icon`; the generic site near line 758), `sidebar/mod.rs` near line 176, `sidebar/menus.rs` (`sidebar_tab_icon`), `chat_icon.rs` (`ChatIcon::color`, the rule provider marks already follow), `assets.rs` (`FOLDER_CODE`, `FOLDER_OPEN`, the registry), and `sidebar/tests.rs`.

## Ownership and authorization

The coder owns implementation and checks; the reviewer waits for an explicit Runner handoff, then audits the whole working-tree diff against `1f647cc`. Iterate through Runner until no must-fix findings remain. No additional crew, nested subagents, new checkout, or worktree. Stop at a clean working-tree review with a smoke checklist for Jason. Commits, push, PR, merge and restarting the development app are not authorized; leave the work uncommitted on top of `1f647cc`. Do not touch `design/runner.pen`.

## Deliverable

**One liveness rule for every head-of-row glyph.** In `sidebar_icon(icon, live)` a generic icon (one without a tint) draws in `theme::text()` at opacity 1.0 when `live` and 0.45 when not, through `theme::with_alpha`, which is exactly what `ChatIcon::color` already does for provider marks; provider marks keep their branch and their tints unchanged. No head-of-row glyph reaches `theme::accent()` or `theme::muted()` any more. Every caller keeps passing the same `live` it passes today: project folder, mission flag, shell and layout tab glyphs, the generic site. Selection, hover, the semibold label and the chevron are untouched. The accent stays at the tail of the row for state: the Working spinner, the ready dot, the unread pill, the attention badge and the drag-and-drop indicators. Verify those by reading them; do not change them.

**A folder that opens.** `assets.rs` gains lucide `folder` as `folder.svg`: the `FOLDER_CODE` outline path without its two `<>` chevron paths, same 2 px stroke; `folder-open.svg` already exists. The project header, in both the normal row and the inline-rename row, picks `folder.svg` when `collapsed` and `folder-open.svg` when expanded, beside the chevron that already flips on the same flag. Remove `folder-code.svg` and `FOLDER_CODE` once nothing in the crate references them; grep the whole crate, menus and the command palette included, before deleting.

**Tests.** The colour table in `chat_icon.rs` or beside `sidebar_icon`: a generic icon resolves to `theme::text()` at 1.0 live and 0.45 not live in both theme variants, and a provider mark still returns its tint. `sidebar_tab_icon` paths unchanged. A project-header test for the asset by expansion state, in the shape of the existing header tests in `sidebar/tests.rs`. Any assets registry test that enumerates icon names.

**Docs.** None beyond the spec: tick the phase 2 items in `docs/features/archive/606-rail-glyph-liveness.md` as they land and note anything the code forced you to decide differently.

Out of this mission: any change to provider marks or their tints, the chevron, row metrics, the tail-state glyphs and their colours, the drag-and-drop indicators, platform window chrome, and the Pencil file.

## Verification

`cargo test --locked -p runner-app`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `cargo fmt --all -- --check`, and `git diff --check`. `theme::text()` resolves per variant, so there is no per-theme code; the tests cover both variants through `theme::set_active_variant` the way `chat_icon.rs` already does.

## Handoff

Final Runner handoff: branch and base commit, the diff summary, checks with results, Jason's manual smoke checklist (macOS and Windows: a project with a running session and one without, a shell tab, a mission row and a chat, in both themes; collapse and expand a project; drag a row and confirm the accent drop indicator is unchanged), and the reviewer's explicit no-remaining-must-fix verdict. Leave the work uncommitted.
