# #1141 m3 — Split diff, side-panel maximize, and Files' Diff button

## Starting point

- Issue: [#1141](https://github.com/yicheng47/runner/issues/1141), milestone m3 (third and final pull request to `main` per [Delivery](../../features/1141-git-changes.md#delivery)). m1 merged in [#1314](https://github.com/yicheng47/runner/pull/1314); m2 merged in [#1339](https://github.com/yicheng47/runner/pull/1339).
- Branch: `feat/1141-m3-split-and-diff` (no upstream yet: push with `git push -u origin feat/1141-m3-split-and-diff`, never to `main`).
- Worktree: `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1141-m3-split-and-diff`, with its own Cargo `target/`.
- Base: `origin/main` at `e9209c97`. Fold this brief and the spec's status update into the implementation commits when shaping the PR history.
- Design: `runner/desktop/specs/1141-git-changes.pen` in `yeelinks_design` at `3d9bd0d`, signed off by Jason on 2026-11-15. Focus on boards `Xc2TS` (screen `tv7xO`, Split with the panel maximized), `EqrNQ` (screen `pCdt6`, Files' Diff button), and `B38Ah` (table `TU4G7`, maximize shortcut row).
- Parallel isolation: `.worktrees/feat-1171-onboarding` and `.worktrees/fix-1251-opencode-v2` are active in parallel. Stay strictly inside this worktree, do not touch the root checkout or sibling worktrees, avoid running heavy workspace-wide Cargo suites concurrently with active sibling tests, and serialize live QA via `script/qa-lock acquire --timeout 540`.

## Problem and outcome

m2 shipped the Changes surface with Unified diff rendering, Commits, viewed marks, and keyboard navigation. m3 completes [#1141](https://github.com/yicheng47/runner/issues/1141) in the desktop app (`frontend-only`, zero protocol or daemon changes) by adding side-by-side **Split diff** rendering, **side-panel maximize** (with the 360 px auto-maximize rule so chats keep their 420 px minimum), and the **Diff button** on changed files opened in Files.

The spec [`docs/features/1141-git-changes.md`](../../features/1141-git-changes.md) wins on behaviour; where the design boards and the spec differ, the spec wins.

## Read first

- [`docs/features/1141-git-changes.md`](../../features/1141-git-changes.md) — especially [The diff](../../features/1141-git-changes.md#the-diff), [From Files to the diff](../../features/1141-git-changes.md#from-files-to-the-diff), [Room for Split](../../features/1141-git-changes.md#room-for-split), [Keyboard](../../features/1141-git-changes.md#keyboard), [Language](../../features/1141-git-changes.md#language), [Verification](../../features/1141-git-changes.md#verification), and [Guide pages](../../features/1141-git-changes.md#guide-pages).
- m2 Changes UI: `crates/runner-app/src/surfaces/changes/{mod.rs,model.rs,render.rs,diff.rs,tests.rs}`.
- Files viewer and tab bar: `crates/runner-app/src/surfaces/files/{panel_view.rs,open_file.rs}`, `crates/runner-app/src/surfaces/files/open_file/`.
- Side panel, chat layout, and pane maximize: `crates/runner-app/src/surfaces/side_panel.rs`, `crates/runner-app/src/surfaces/chat/layout.rs`, `crates/runner-app/src/surfaces/panes/{header.rs,pane.rs}`, `crates/runner-app/src/surfaces/mission_workspace/`.
- Settings and localization: `crates/runner-app/src/app_settings.rs`, `crates/runner-app/src/i18n.rs`, `crates/runner-app/i18n/{en,zh-hans}.toml`.

## Deliverables

1. **Unified / Split switch and Split diff renderer:** Add the `Unified` / `Split` selector in the diff header (shared by Changes and Files' in-place diff) and remember the choice in `ui-settings.json` (`AppSettings`). Render Split with the old file on the left and new file on the right, changed lines paired side-by-side with blank filler where one side has no line, synchronized vertical and horizontal virtualized scrolling, off-UI-thread two-version syntax highlighting, CRLF stripping, shared Files font/zoom, cursor tint and keyboard navigation (`↑`/`↓`/`j`/`k`, `[`/`]`, `←`/`→`, `Esc`, selection/copy without gutter, and manual `Mark <file> viewed` footer in Changes only), plus all edge placeholders.
2. **Side-panel maximize and the 360 px rule:** Add the `maximize-2` button (`minimize-2` on Secondary fill labelled **Restore chat** when maximized) to the side panel header across all views (`Changes`, `Files`, `Sessions`, `Info`), ephemeral (not persisted across restarts). Maximizing fills the tab and hides the chat (or mission and its drawer); otherwise `MAIN_MIN_WIDTH` (`420` px) still holds. Bind `toggle-maximize-pane` (`⇧⌘↩` / `Ctrl+Shift+Enter`) to toggle side-panel maximize when focus is in the side panel, keeping pane maximize intact when focus is in chat. When switching to `Split` (or restoring from maximized) where each side would be narrower than `~360` px, auto-maximize the panel on picking `Split`, and on **Restore chat** fall back to rendering `Unified` until wide enough while keeping the remembered choice `Split`.
3. **Files' Diff button:** When an open file tab in `Files` has uncommitted changes, show a **Diff** toggle beside **Open in** in the tab bar. Clicking it swaps the tab in place between the file viewer and its diff under the same diff header (`+`/`−` counts, `Unified`/`Split`, `"Working tree vs HEAD"`, edge placeholders, no Viewed checkbox), keeping state per open tab; when changes disappear, hide the button and return the tab to the file.
4. **Language, spec, and guide docs:** Add all m3 UI strings cleanly to `en.toml` and `zh-hans.toml` (VS Code Simplified Chinese conventions where matching). Update [`docs/features/1141-git-changes.md`](../../features/1141-git-changes.md) Status (`m2 merged in #1339; m3 is next`) and update the m3 sections of `docs/guide/{guides/chats-and-windows.md,reference/keyboard-shortcuts.md}` and their `zh-CN` mirrors.

Out of scope: changing `runnerd` RPCs or protocol types unnecessarily (keep `frontend-only`), line comments (`#1293`), or remote sessions.

## Validation and handoffs

- **Automated:** Add unit/UI tests in `runner-app` covering Split line pairing, remembered `Unified`/`Split` preference, side-panel maximize/restore + focus-scoped `⇧⌘↩`, the `~360` px auto-maximize / `Unified` fallback rule, and Files' `Diff` button visibility, toggle, and change-clear reset. Run `cargo test -p runner-app`, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings`, `--features updater`, `cargo fmt --all -- --check`, `git diff --check`, and `python3 script/check-guide-translations.py`.
- **Live checks (`qa`):** After a clean review (`NO REMAINING MUST-FIX ISSUES`), acquire `script/qa-lock acquire --timeout 540` and test in the macOS dev app (`make run`) inside a `/tmp` fixture repo: Spec check **6** (Split and maximize + 360 px / 420 px + `⇧⌘↩`), Spec check **7** (Files' Diff button + shared Unified/Split), and regression **WS-PANE-07** (panel vs pane maximize coexistence). Record results in `docs/tests/runs/1141-git-changes-mac.md` and the PR description; keep `needs-live-test` until Windows and Linux live checks run.
- **Handoffs:** `coder` sends unpushed candidate SHA to `reviewer`, iterates until clean, then hands the reviewed SHA to `qa`. Any QA fix returns through `reviewer` before `qa` reruns affected checks. Only after both review and macOS QA pass cleanly, fold this brief and spec status into clean commit(s), push `feat/1141-m3-split-and-diff`, and open one PR against `main`.

## Authorization

Jason authorized **`carbon trio`** on 2026-11-15 for `#1141 m3`. Commit (folding this brief and spec update into implementation commits), push `feat/1141-m3-split-and-diff`, open one PR against `main` titled for m3 with `Closes #1141`, labelled `frontend-only`, `risk-medium`, `reviewed`, and `needs-live-test`, and drive CI green. Stop for Jason to review and merge.
