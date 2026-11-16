# 1345 — Changes: choose the checkout and the base

> Tracking issue: [#1345](https://github.com/yicheng47/runner/issues/1345). Scheduling and priority live on the issue and in the [roadmap](../roadmap.md). Platforms: macOS, Windows and Linux.
> Status: implemented in [#1348](https://github.com/yicheng47/runner/pull/1348); design and decisions agreed by Jason on 2026-11-16.
> Design: `runner/desktop/specs/1345-changes-checkout-and-base.pen` in `yeelinks_design`, revision `003d626`. Board `PxxiI` ("1345 — Changes: checkout and base"): the default panel (`InitT`), a chosen base with the picker open (`dzu8A`), the Base row states (`VCgPS`), the checkout picker open (`ebb17`), viewing another checkout (`KHUOM`) and the branch row states (`VH7ml`). Components: `cmp/BasePicker` (`Bqvfh`) and `cmp/CheckoutPicker` (`gFJbt`).
> Follows: [#1141](./archive/1141-git-changes.md), the Changes view, whose list, diff, requests and watcher this extends. Related: [#1213](https://github.com/yicheng47/runner/issues/1213) (starting chats and missions in their own worktree).

## Motivation

Changes shows the chat's own checkout against a base it guesses. Two ways of working meet there:

1. **One checkout, switching branches.** Changes follows `HEAD`, so `git switch` already updates it. What is missing is the base: a branch cut from another feature, release or umbrella branch is compared with `origin/main`.
2. **Several worktrees, one per agent.** Each chat or mission has its own worktree. Files' header can already show another worktree, but Changes gives no sign that there is more than one, so from the root chat you cannot see what the m4 coder has changed without opening its chat or going through Files.

The base guess, `resolve_base` in `crates/runner-daemon/src/files/changes.rs` (`origin/HEAD`, then `origin/main`, `origin/master`, then a local `main` or `master`), is wrong for umbrella programs (a mission branch cut from `feat/645-runnerd` lists every umbrella commit, and All changes shows the whole umbrella), stacked and release branches, and forks or remotes not named `origin`.

## Scope

### The Base row

- A **Base** row sits directly under the branch row in the Changes list: the label "Base" and, right-aligned, a chip naming the base with a chevron. It is shown whenever Changes shows a git checkout, for one checkout or many.
- **Auto** is the default, and its chip reads "Auto · origin/main", naming what [#1141's resolution](./archive/1141-git-changes.md#commits-and-what-is-compared) chose. With no base resolved it reads "Auto · none". A chosen base shows its ref, `feat/645-runnerd`, in the primary text colour.
- Clicking the chip, or Enter on it, opens the picker titled **Compare against**: a filter field, then Auto with what it resolves to, then the local branches, then one section per remote (`ORIGIN`, `UPSTREAM`) with that remote's branches, the current choice checked. The checkout's own branch and its upstream are left out. Typing filters every section and hides Auto unless it matches; ↑↓ and Enter pick, Esc closes. Picking closes the picker and refreshes the list.
- The base drives everything that depends on it: the commits since base, the commit count in the branch row, and All changes, the diff from the merge base of `HEAD` and the chosen ref. Uncommitted changes and a single commit do not depend on it. The rule for showing All changes is #1141's: at least one commit since the base.
- **Remembered** per checkout and branch (Jason, 2026-11-16): an app setting `changes_bases` in `ui-settings.json`, mapping the checkout's top path and branch name to the chosen ref. Picking Auto removes the entry. A detached `HEAD` keys by the checkout alone. A branch's base therefore follows it across sessions and restarts, and another branch in the same checkout keeps its own.
- **A base that no longer exists**, as after an umbrella merges and its branch is deleted, falls back to Auto: the chip reads "Auto · origin/main", and an amber line under the row says "feat/645-runnerd no longer exists. Using Auto." until the picker is next opened or the base changes. The saved entry is removed.
- **No umbrella default** (Jason, 2026-11-16): a mission's worktree starts on Auto like any other checkout; Runner records nothing about which branch a worktree was cut from.

### The checkout picker

- With **one checkout**, the branch row stays as #1141 shipped it: the branch icon, the branch, the commit count, plain text.
- With **more than one checkout** (`git worktree list`), the row becomes a picker: a kind icon (`house` for the main checkout, `folder-git-2` for a linked worktree), then the branch as a button with a chevron, then the commit count.
- The picker, titled **Show changes in**, lists the repository's checkouts only, the main checkout first, then the linked worktrees by branch. Each row shows the branch, then its short commit, its uncommitted file count (amber when not zero, "clean" when zero) and its path shortened from the left (`…/feat-645-m4-host-api`), and on the right the session working in it: "this chat" or "this mission" for the panel's own, otherwise "chat", "terminal", "mission · <slot>", or "N sessions" when several share it. The current one is checked. A filter field matches branches and paths; ↑↓, Enter and Esc work as in the Base picker.
- **Picking one sets the same root as the checkout picker in Files' header** (`files_set_root`), so Files and Changes always show the same checkout, and it is remembered per chat, terminal or mission as Files' root is today, across tab switches and restarts (Jason, 2026-11-16, "per panel": a chat's panel, a terminal-only tab's panel, a mission's rail). The chat or mission does not move.
- **Viewing another checkout**, the branch row takes the selection tint and ends with a return button, `undo-2`, tooltip "Back to this chat's checkout" ("this mission's" in a rail), which sets the root back to the session's own (`files_set_root` with no path). The commit count shortens to the number alone in that state so the button fits.
- **A viewed worktree that is removed** falls back to the session's own checkout, as Files already does, and the row shows an amber line, "feat/645-m4-host-api was removed. Showing this chat's checkout.", until the picker is next opened.
- Files' header picker keeps its own list (Choose folder…, worktrees, projects, recent folders); its worktree rows gain the same uncommitted count and session label so both pickers describe a checkout the same way.
- Changes stays read-only: no branch switching and no worktree creation (#1213 creates worktrees).

### The diff header

Changes' diff header drops #1141's comparison label ("feat/1141-git-changes vs origin/main", "Working tree vs HEAD", "a1f3c9e vs parent"). The list beside it names all three parts, the branch in its row, the base in the Base row and the target in the selected Commits row, and the list cannot be hidden. The label also truncated in narrow headers. The header ends with the Unified and Split switch. Files' Diff button view keeps its "Working tree vs HEAD" label, since no list sits beside it.

### Files and Changes reach the panel's bottom

In the chat side panel and the mission rail, Files and Changes stop 8 px above the island's bottom edge: the tree column, its left border and the file or diff column end early, and a band of the island background shows under them. The panel body keeps a bottom padding added for the Info surface before Files and Changes existed, `.pb(rems(8. / 16.))` in `render_chat_side_panel` (`crates/runner-app/src/surfaces/panes/header.rs`) and in the mission rail (`crates/runner-app/src/surfaces/mission_workspace/rail.rs`). The padding stays for Info and the empty surface list; Files and Changes fill to the bottom, clipped by the island's rounded corners, maximized or not.

### Where it runs

`runnerd` does the git work, as for #1141. Request changes:

| Request | Change |
|---|---|
| `changes_commits(root, base)` | Takes an optional base ref. `None` resolves as today. A given ref must name a local or remote-tracking branch (`refs/heads/…` or `refs/remotes/…`, checked with `rev-parse --verify`); otherwise the request resolves Auto and reports the miss. `ChangesCommits` gains `auto_base: Option<String>`, what Auto resolves to, and `base_missing: bool`; `base` stays the ref actually used. |
| `changes_files(root, target, base)` and `changes_diff(root, target, path, base)` | Take the same optional base, used only by the `branch` target to find the merge base. |
| `changes_branches(root)` (new, slow) | The checkout's local branches and each remote's branches, by `git for-each-ref --format` over `refs/heads` and `refs/remotes`, without `HEAD` symrefs, as short names grouped by remote. |
| `files_worktrees(root)` | `WorktreeEntry` gains `uncommitted: Option<usize>`, from `git status --porcelain=v2 -z` in each worktree, run in parallel and reported as `None` for a worktree whose status fails or is prunable. |

Viewed marks are unchanged: a mark made in All changes records the merge base's version, so marks made against one base do not match against another, as #1141 already says for a moving base. The fallback for a missing base is reported in data (`base_missing`), never detected from an error message. Session labels in the checkout picker come from the app's own sessions and missions, matched by working directory inside each worktree's path; `runnerd` does not change for them.

New strings go through `crates/runner-app/i18n/` in `en.toml` and `zh-hans.toml`, with #1141's terminology. Branch names, refs, commit ids and paths stay as git gives them. The comparison label's keys used only by Changes' diff header are removed; `changes.working_tree_vs_head` and `changes.working_tree_unborn` stay for Files' Diff view.

## Non-goals

Comparing two arbitrary refs, commit ranges, and switching or creating branches or worktrees from Changes. Tags as a base. Defaulting a mission's base to its umbrella. Remote sessions, as #1141.

## Risks

- **Core review.** The request changes touch the client protocol, so the pull request takes `core-review`. Every new parameter and field is optional with a serde default, so an app and a daemon one build apart keep working with Auto.
- **Worktree status cost.** Counting uncommitted files runs `git status` in every worktree each time a picker opens. It runs only when a picker opens, in parallel, and the rows render before the counts arrive.
- **Root shared with Files.** Picking a checkout in Changes re-roots Files too. That is intended, since both describe one checkout, and the return button is always one click away.

## Delivery

One pull request from `feat/1345-changes-checkout-and-base`, built by the claude trio (Jason's default for a designed feature that needs live checks), its brief in `docs/impls/briefs/`. Labels: `core-review` (protocol), `risk-medium`, `needs-live-test`.

## Verification

### Automated

- **Base resolution with a chosen ref,** in fixture repositories built with the git CLI: a local branch, a remote-tracking branch, an `upstream` remote, a deleted ref (Auto with `base_missing`), a tag or a commit id (refused, Auto with `base_missing`), and `None` matching #1141's cases unchanged.
- **All changes against a chosen base:** in a worktree cut from an umbrella two commits ahead of `main`, with the umbrella as base, `changes_files(branch)` matches `git diff --numstat $(git merge-base HEAD umbrella) HEAD` and `changes_commits` lists only the worktree's own commits.
- **`changes_branches`:** local and remote branches grouped by remote, without `origin/HEAD`, in a repository with two remotes.
- **`files_worktrees` counts:** a clean worktree, one with modified and untracked files, and a prunable one.
- **App:** the settings key for a branch and for a detached `HEAD`; Auto removes the entry; the missing-base notice clears when the picker opens; the branch row is plain with one checkout and a picker with two; session labels for the panel's own session, one chat, a mission slot and several sessions; the Base and checkout pickers' filtering and keys.
- **Protocol samples** for the new fields and request, as `crates/runner-core/src/protocol/tests.rs` requires.

### Live checks

The claude trio's `qa` runs these on macOS, in a fixture repository under `/tmp` with `main`, an umbrella branch two commits ahead of it, and a worktree cut from the umbrella with three commits of its own and one uncommitted edit, plus a second worktree with a running bounded test chat:

1. **Base row default:** the worktree's Changes shows "Auto · origin/main", and Commits lists all five commits. Reason: new row, unchanged default.
2. **Picking a base:** the picker lists Auto, the local and remote branches; picking the umbrella leaves three commits and an All changes of their files only, and the choice survives closing the panel and restarting the development app. Reason: the base drives the list, and it is remembered.
3. **Deleted base:** deleting the umbrella branch shows the amber notice and falls back to Auto. Reason: fallback.
4. **One checkout:** a repository with no worktrees shows the plain branch row. Reason: unchanged single-checkout look.
5. **Checkout picker:** from the main checkout's chat, the picker lists the main checkout and both worktrees with their commits, uncommitted counts and session labels; picking a worktree shows its changes in Changes and in Files, with the tint and the return button; the return button goes back. Reason: new picker sharing Files' root.
6. **Removed worktree:** removing the viewed worktree returns to the chat's own checkout with the amber notice. Reason: fallback.
7. **Diff header:** Changes' diff header ends with Unified and Split and no comparison label; Files' Diff view still reads "Working tree vs HEAD". Reason: removed label.
8. **Panel bottom:** in a chat panel and a mission rail, Files' tree column and Changes' list reach the island's bottom edge inside its rounded corner, in dark and light, maximized and not; Info keeps its bottom space. Reason: layout fix.

Windows and Linux are not selected: the change adds no platform code, and git runs through the same paths #1141 validated. Selected regression cases: [WS-FILES-05](../tests/regression/workspace.md) (panel width, which the rows must fit) and #1141's live checks 3 and 5 (Commits and viewed marks, which now take a base).

## Guide pages

[Chats and windows](../guide/guides/chats-and-windows.md) ("Review changes in the side panel": the Base row and its picker, the checkout picker and its return button; the Files section's picker rows gain the uncommitted count and session label) and [Missions](../guide/concepts/missions.md) (the same in a mission's rail).

## Decisions settled with this spec

Agreed by Jason on 2026-11-16.

1. **Placement:** the base gets its own row under the branch row, chosen over a control in the diff header, a two-line branch header and an icon button (board history in `yeelinks_design` before `697073d`).
2. **The checkout picker** joins this issue: the branch row picks among the repository's checkouts when there is more than one, modelled on the worktree picker in Zed's title bar but without its separate branch switcher, since Changes is read-only.
3. **The comparison label** leaves Changes' diff header.
4. **Storage:** the base in app settings keyed by checkout and branch; the checkout through Files' existing per-session root.
5. **No umbrella default** for mission worktrees.
6. **Per panel:** the checkout is chosen per chat, terminal-only tab or mission rail, as Files' root is.
7. **Milestone 0.19**, with the panel-bottom fix folded in.
