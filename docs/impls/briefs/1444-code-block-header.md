# #1444 — Code block header

Jason approved the neon trio (Antigravity `gemini-4-neon` coder, Codex `gpt-6.2-astra` reviewer, Codex `gpt-6.2-sol` qa) on 2026-11-24. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1444-code-block-header` on the branch `feat/1444-code-block-header` (base `origin/main` `a084538b`, its own `target/`). Never touch the root checkout or sibling worktrees.

## Problem and outcome

Code blocks in rendered Markdown have no controls: copying one means drag-selecting its lines, its language is not shown, long lines always wrap, and feed code blocks are plain text while Files highlights them. Outcome: in both looks of the shared renderer at `crates/runner-app/src/ui/markdown/`, every code block gets a header bar with its language, a Wrap toggle and a Copy button, and the Compact look highlights code as Files does.

The spec is authoritative: [`docs/features/1444-code-block-header.md`](../../features/1444-code-block-header.md). It holds the scope, the design rules for rows, hover, unwrapped lines, selection and feed highlighting, the implementation steps and the checks. The signed-off design is board `Bka9r` in `runner/desktop/specs/1443-markdown-rendering-features.pen` of the design repo; the spec states everything the build needs from it. [`docs/tech/markdown-rendering.md`](../../tech/markdown-rendering.md) explains blocks, rows and the two looks.

## Read first

- The spec, then the tech note's sections on blocks, rows and looks.
- `ui/markdown/mod.rs`: `CodeBlock`, `Block`, `Page::same_rendered`.
- `ui/markdown/parse.rs`: `ParseConfig`, `parse_document_blocks`, `parse_compact_blocks`, the `Tag::CodeBlock` arm.
- `ui/markdown/layout.rs`: `RowKind::Code`, `Row::text`, `Row::continues`, `fragments`, `flatten_blocks`.
- `ui/markdown/draw.rs`: `code_line`, the selection handlers, `document_selectable`, `compact_plain_line`.
- `ui/markdown/view.rs` and `ui/markdown/feed.rs`: `MarkdownView`, `DRAWN_CODE_BYTES`, `FeedMarkdown`.
- `surfaces/files/highlight.rs`, and how the code viewer highlights a large file later.
- `surfaces/mission_workspace/feed.rs` with its tests (`PARSE_COUNT`, `RENDER_COUNT`, `LINE_COUNT`), and `ui/markdown/tests.rs`.
- `docs/guide/guides/chats-and-windows.md` and its zh-CN twin; `docs/guide-authoring.md` for the locale rules.

## Deliverables

Follow the spec's implementation steps:

1. **Model and rows.** The language on `CodeBlock`, the header row kind in both looks, borders moved to the bar; every existing test passes.
2. **Copy and Wrap.** Hover per block, the buttons with tooltips, copy with its two-second confirmation, unwrapped lines with one shared offset and a scrollbar.
3. **Feed highlighting.** Highlighting in Compact without extra parses or a slower feed.
4. **Guide and tests.** The spec's guide pages in both locales (`python3 script/check-guide-translations.py`), and its automated tests.

Not in scope, as the spec lists: copy buttons on other blocks, source spans or a general row-to-block index, line numbers, collapsing, open in editor, save to file, Mermaid and math controls, and any setting.

## Validation and handoffs

- **@coder:** `cargo test -p runner-app`, `make clippy`, `cargo fmt --all -- --check`, `git diff --check`. Any new test that waits on background work or a timer runs `cargo nextest run -p runner-app --stress-count 20 -E '<filter>'`, and the handoff states the count. Hand `@reviewer` the local commits with a summary and the checks run.
- **@reviewer:** reviews the commits against the spec, especially virtualization and the #962 counters, feed highlighting cost, selection excluding the bar, the shared horizontal offset and the timer tests. The clean review names the live checks and the candidate commit.
- **@qa:** the spec's three live checks on the macOS development app, holding `script/qa-lock`, with the regression cases `WS-FILES-06`, `MIS-FEED-03` and `SET-SKILL-01` as the spec selects them. It records pass or fail per check with screenshots in `docs/tests/runs/1444-code-block-header-mac.md`. Failures go back to @coder, and the fixed candidate is reviewed again before qa reruns the affected checks.

## Authorization

Approved crew: the neon trio. The crew may commit, push `feat/1444-code-block-header` and open a pull request against `main` titled `feat(markdown): code block header with language, wrap and copy` that closes #1444, with the labels `frontend-only`, `risk-medium` and `needs-live-test`. Fold this brief and the spec into the commits. Remove `needs-live-test` once every selected check has passed, and drive CI green on both platforms. Do not merge.
