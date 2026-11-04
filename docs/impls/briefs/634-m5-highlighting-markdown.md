# 634 m5 — highlighting and Markdown

## Starting point

Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-634-m5-highlighting-markdown`, on branch `feat/634-m5-highlighting-markdown`. Do not create another branch or checkout, touch the root checkout or another worktree, or share another worktree's target directory.

The branch is stacked: it is cut from m4's reviewed head `55b2ee76` (PR #1173, `feat/634-m4-file-viewer`), whose live QA is waiting for the screen, not on the umbrella `feat/634-file-preview` (`bf77e138`). This brief is the branch's tip. When the operator tells you m4 has merged, rebase onto the umbrella with `git rebase --onto origin/feat/634-file-preview <m4 commit on this branch>` so only m5's commits remain, and push with `--force-with-lease`. If m4's commit changes before then (a QA fix), the operator tells you its new SHA and you rebase onto it the same way. Never merge.

## Problem and outcome

m4 draws every file as plain text and every Markdown file as its source. This mission highlights code with tree-sitter in the terminal palette's colours and renders Markdown with a `</>` toggle to the highlighted source. Settings, including the Syntax highlighting switch and Rendered or Source, is m6's: m5 behaves as those settings' defaults (highlighting on, Markdown rendered).

## Read first

- [The plan](../634-file-preview/plan.md), Phase 3, m5: the authoritative scope, including the spike and its 10 MB / 2 minute threshold. The [spec](../../features/634-file-preview.md), Syntax highlighting and Read-only file viewer, wins on behaviour.
- The design, exported as PNGs in this worktree's ignored `target/design-ref/` from `runner/desktop/specs/634-file-preview.pen` (`yeelinks_design` `bde8051`). They win on visual detail; match them with theme tokens, sample data is illustrative; do not delete them or open `.pen` files.
  - `markdown-preview.png` (board `n3y8HZ`): rendered Markdown with its margins, headings, lists, code block and inline code, links, and the `</>` button in the tab strip.
  - `tree-on-the-right.png` and `tree-and-viewer-states.png`: highlighted code in the viewer.
- m4's code view, find and reload (`crates/runner-app/src/` Files viewer modules on this branch), the terminal palettes and their light/dark choice in Settings → Appearance, and `mission_markdown.rs`, which stays as it is.
- Reference only, no dependency: Zed's `language` crate (highlight map, capture-name fallback) and its `markdown` crate.

## Decisions already made

- **Markdown beyond the spec's list** (the README's open question, decided by the operator under Jason's mandate, 2026-11-05): render GFM tables and task lists (checkboxes shown, read-only), and show inline and block HTML as literal text. Footnotes, math and anything else stay as text. The operator records this in the program README when m4 merges.
- **The spike's threshold**: if the cheaper option is over about 10 MB of app size or 2 minutes on a CI job, ask through Runner with the numbers; do not ask otherwise.
- Remote, the real pointer (drags, resize, hover-only states) and the light theme are deferred for live checks (Jason, 2026-11-04). The code still supports light; unit snapshots cover both themes as the plan says.

## Deliverables

The plan's m5 items 1–4: the spike (first commit or a measured note, results with numbers in `docs/impls/634-file-preview/impl_log.md`), highlighting off the UI thread cached per line for the virtualized view and recomputed on reload, the capture-name colour maps (hand-tuned for Runner, Catppuccin Mocha and Rosé Pine Dawn, derived for any other palette), and rendered Markdown with the `</>` source toggle, local images through `files_read`, browser links, and relative links to project files opening in a new tab. Plus:

5. **Licences**: each bundled grammar keeps its licence, shipped as m3 shipped the icon themes' licences.
6. **Guide**: highlighting and Markdown preview in [Chats and windows](../../guide/guides/chats-and-windows.md), both locales.

Non-goals: Settings, language servers, editing, any daemon or protocol change. If a design question arises that the plan, spec and PNGs do not settle, ask through Runner.

## Validation

Unit tests as the plan's m5 Checks list: language detection, capture fallback, the derived map, a 10 MB file highlighted without work on the UI thread and only visible lines styled, each Markdown block (including tables and task lists, and HTML as text) from snapshots in both themes, link routing, and find still matching over highlighted text. `runner-app` tests, workspace clippy with the CI profile (and `--features runner-app/updater`), fmt and the guide translation check, logged with exit codes. CI must be green on macOS, Windows and both Linux jobs, since grammars compile C on each.

Live checks are required and run by `qa` after the clean review, on macOS, dark theme only: Rust, TypeScript and TOML highlighted; a README with a table, a task list, images and relative links, the relative link opening the file in a new tab; the `</>` toggle both ways; a large file scrolling without stalls; and find over highlighted code. Light theme, the real pointer and remote are deferred, not run.

## Handoffs

The coder implements, tests and fixes. The reviewer waits for the coder's Runner handoff, then reviews the whole of m5's diff (not m4's commit) against the plan, spec, PNGs and this brief, must-fix first with file:line pointers, checking in particular: no parse or highlight on the UI thread, per-line caching with the virtualized view, the capture fallback, the grammar build on Windows and Linux, licences, and Markdown link routing staying inside the root.

## Authorization

Approved crew: the claude duo, with the codex `qa` role added by the operator after the clean review. Once review is clean the crew may commit and push this branch. It opens the PR against `feat/634-file-preview` only after the operator says m4 has merged and the branch is rebased; until then it reports the reviewed head through Runner and waits. The PR is titled `feat(ui): highlighting and Markdown preview (#634 m5)`, labelled `risk-medium` and `needs-live-test`, plus `frontend-only` if nothing outside `runner-app` changes, and CI is driven green on every job. The operator merges it into the umbrella; the crew does not merge.
