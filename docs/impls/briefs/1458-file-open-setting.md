# #1458 — Where ⌘-clicked files open

Jason approved the neon trio (Antigravity `gemini-4-neon` coder, Codex `gpt-6.2-astra` reviewer, Codex `gpt-6.2-sol` qa) on 2026-11-27. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1458-file-open-setting` on the branch `feat/1458-file-open-setting` (rebased onto `origin/main` `851f10c8`, its own `target/`). Never touch the root checkout or sibling worktrees. `origin/feat/1458-file-open-setting` still holds an earlier draft of the spec, so the first push uses `git push --force-with-lease`.

## Problem and outcome

⌘-click (Ctrl-click on Windows and Linux) on a file path in a terminal opens it in the side panel only when the panel and Files are on, the session is local and the path is inside the Files root. Everything else goes to the editor saved by the viewer's **Open in** chevron, which defaults to **Default app**, and nothing in Settings shows or sets that choice. Outcome: a **File links** section in Settings → General sets where ⌘-click opens files, **Runner preview** by default. Local files outside the Files root open in the panel too. Each file type is routed to something that can show it. Changes is dimmed when there is no git checkout. The cannot-preview placeholder says why.

The spec is authoritative: [`docs/features/1458-file-open-setting.md`](../../features/1458-file-open-setting.md). It holds the scope, the routing table, the design notes, the implementation steps, the automated tests and the live checks. The signed-off design (options A2, B1, C1 and D1) is `runner/desktop/specs/1458-file-open-setting.pen` in the design repo, boards `mlals`, `Z12TDP`, `Xayo4`, `yKQSi` and `ffz9p`. The spec states everything the build needs from it, and the spec wins on detail.

## Read first

- The spec.
- `file_links.rs`: `FileLinkTarget` (the clicked path, line and column; the new setting is `FileLinkOpenIn`, not this), `open`, `link_tooltip_content`, `link_tooltip`, `open_file_link`.
- `surfaces/files/links.rs`: `relative_in`, `open_terminal_link`, `link_previews`.
- `app_settings.rs`: `FileLinkEditor`, `file_link_editor` and their defaults, and `app_settings_tests.rs`.
- `surfaces/files/open_file/open_in.rs`: the **Open in** chevron that writes `file_link_editor`.
- `surfaces/settings/page/general.rs`: `render_general_settings`.
- `surfaces/files/open_file.rs` (`IGNORED_REFRESH`), `open_file/read.rs` (`files_read`, `if_unchanged`), `open_file/render.rs` (`render_placeholder`).
- `surfaces/side_panel.rs` (`activity_bar`) and `surfaces/changes/render.rs` (`CHANGES_NOT_GIT`).
- `crates/runner-app/i18n/`, `docs/guide-authoring.md`, and the guide pages the spec lists.

## Deliverables

Follow the spec's implementation steps, with each commit building and passing its tests:

1. **Setting and decision.** Add `file_link_open_in` with its default and the upgrade path. Add one pure decision function with its tests. Move the click and tooltip paths onto it, including the fallback when the panel is off.
2. **Settings UI.** The File links section in General; the Editor menu, shared with **Open in**; the info line shown when the panel is off. Both locales.
3. **Outside tabs.** The absolute-path tab key, reading through the file's folder, reloading, the full folder in the subbar, and no Diff.
4. **Changes and placeholders.** The disabled Changes icon and its tooltip, plus the new binary and over-10 MB placeholder copy.
5. **Guide and tests.** The guide pages in both locales and the spec's automated tests.

Not in scope, as the spec lists: remote sessions, Diff for files outside the root, resolving shortened paths (#1459), and side-panel shortcuts (#1482). No change to the client protocol or to `runnerd`: outside files are read with the existing `files_read`.

## Validation and handoffs

- **@coder:** run `cargo test -p runner-app`, `make clippy`, `cargo fmt --all -- --check`, `git diff --check` and `python3 script/check-guide-translations.py`. Any new test that waits on background work, a watcher or a timer also runs `cargo nextest run -p runner-app --stress-count 20 -E '<filter>'`, and the handoff states the count. Hand `@reviewer` the local commits with a summary and the checks run.
- **@reviewer:** review the commits against the spec. Pay particular attention to:
  - the click handler, the tooltip and the Settings info line all using the one decision function;
  - a path inside the root never opening a second, outside tab;
  - the settings upgrade keeping `file_link_editor`;
  - root checks, size checks and reads staying off the UI thread;
  - outside tabs reloading without a watcher;
  - **Open in** and the Editor row staying in sync;
  - remote sessions behaving exactly as before.

  The clean review names the live checks and the candidate commit.
- **@qa:** run the spec's live checks 1–7 on the macOS development app, holding `script/qa-lock`, together with the regression cases the spec selects (`SET-PANEL-03`, `WS-FILES-03`, `WS-FILES-04`, `WS-CHANGES-08`). Record pass or fail per check, with screenshots, in `docs/tests/runs/1458-file-open-setting-mac.md`. Check 8 (Windows, with `TERM-LINK-02`) needs a Windows machine: if none is reachable, record it as Blocked, not passed. Failures go back to @coder, and @reviewer reviews the fixed candidate before qa reruns the affected checks. Restore any development setting a check changes.

## Authorization

Approved crew: the neon trio. The crew may commit, push `feat/1458-file-open-setting` and open a pull request against `main` titled `feat(files): choose where ⌘-clicked files open, preview by default`. The pull request closes #1458 and carries the labels `risk-medium`, `frontend-only` and `needs-live-test`. Fold this brief and the spec into the commits. Remove `needs-live-test` only once every selected check has passed; a Blocked Windows check keeps it on. Drive CI green on all platforms. Do not merge.
