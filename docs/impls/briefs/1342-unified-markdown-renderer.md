# #1342 — Unified Markdown renderer (phase 1)

Jason approved the neon trio (Antigravity `gemini-4-neon` coder, Codex `gpt-6.2-astra` reviewer, Codex `gpt-6.2-sol` qa) on 2026-11-24. Work only in `/Users/jason/repos/yicheng47/runner/.worktrees/feat-1342-unified-markdown-renderer` on the existing branch `feat/1342-unified-markdown-renderer` (base `origin/main` `c8cd3b70`, its own `target/`). Draft PR [#1449](https://github.com/yicheng47/runner/pull/1449) is already open on this branch: continue it, do not open another. Never touch the root checkout or sibling worktrees.

## Problem and outcome

Runner renders Markdown twice: `surfaces/files/markdown.rs` (`pulldown-cmark`, a block tree laid out as virtualized rows) for the Files preview, and the hand-written `surfaces/mission_markdown.rs` for feed messages, question cards, the role and crew prompt and editor preview, the Skills preview and the Info popover's System prompt. Outcome: the Files renderer becomes the one block model and renderer for all six places, at `crates/runner-app/src/ui/markdown/`, with a Document look for Files and a Compact look for the rest, and `mission_markdown.rs` deleted.

The spec is authoritative: [`docs/features/1342-unified-markdown-renderer.md`](../../features/1342-unified-markdown-renderer.md). It holds the looks table, where each place draws, feed selection and copy, the intentional changes, the non-goals and the checks. [`docs/tech/markdown-rendering.md`](../../tech/markdown-rendering.md) explains blocks, rows and both renderers. Files is the reference wherever the two renderers disagree, except the Compact rules the spec names.

## Read first

- The spec, then the tech note.
- `surfaces/files/markdown.rs`: `Page::parse`, `Parser::blocks`, `bounded`, `flatten`, `Row::text`, `MarkdownView`, `Draw`.
- `surfaces/mission_markdown.rs`: `FeedMarkdown`, `FeedSelection`, `FeedSelectionHandler`, `MarkdownLinkHover`, `markdown_layout`, `block_margins`, `render_markdown`, `selected_plain_text`.
- `surfaces/mission_workspace/feed.rs`: `cached_feed_markdown`, `render_feed_markdown`, `update_feed_selection`, `copy_feed_selection`, `render_mission_message_row`, `render_mission_ask_card`; plus `state.rs` and `mod.rs` for the cache.
- The other call sites: `profile_page.rs`, `settings/skills.rs`, `panes/panel.rs`.
- Tests: `surfaces/files/tests.rs` and `surfaces/mission_workspace/tests.rs` (`PARSE_COUNT`, `RENDER_COUNT`, `LINE_COUNT`).

## Deliverables

Follow the spec's implementation steps:

1. **Move.** `surfaces/files/markdown.rs` to `ui/markdown/`, Files pointed at it, no behaviour change: every Files test passes unchanged. Its own commit.
2. **Compact, feed and question cards.** The Compact look with the feed's current spacing, selection colour and link hover; a row per line at soft breaks; `FeedMarkdown` rebuilt on the shared rows, packing up to 4 rows per feed list entry; selection by row and byte offset; copy from the cached message, with `selected_plain_text` removed; links limited to `http`, `https` and `mailto`, never opened by a drag; code lines wrapping; images as alt text.
3. **The other places.** Profile, Skills and Info on the shared renderer, read-only as now; delete `surfaces/mission_markdown.rs`.
4. **Tests and the PR.** The spec's automated tests. The PR description lists every intentional change from the spec and the live-check results.

Non-goals, as the spec lists: no source spans, row-to-block index or code-language field (#1444 adds them); no feed syntax highlighting; no footnotes, math or alerts; no new settings, controls or plugin interface; no `docs/guide/` change.

## Validation and handoffs

- **@coder:** `cargo test -p runner-app`, `make clippy`, `cargo fmt --all -- --check`, `git diff --check`. Any new test that waits on background work runs `cargo nextest run -p runner-app --stress-count 20 -E '<filter>'`, and the handoff states the count. Hand `@reviewer` the local commits with a summary and the checks run.
- **@reviewer:** reviews the commits against the spec, especially the #962 virtualization counters, feed selection and copy, link safety and Files parity. The clean review names the live checks and the candidate commit.
- **@qa:** the spec's three live checks on the macOS development app, holding `script/qa-lock`, with the regression cases `MIS-FEED-03`, `WS-FILES-06` and `SET-SKILL-01`. It records pass or fail per check with screenshots in `docs/tests/runs/1342-unified-markdown-renderer-mac.md`. Failures go back to @coder, and the fixed candidate is reviewed again before qa reruns the affected checks.

## Authorization

Approved crew: the neon trio. The crew may commit, push `feat/1342-unified-markdown-renderer` and update draft PR #1449: fold this brief and any review fixes into the commits (the two spec commits may become one docs commit), rewrite the PR description, keep the `frontend-only` and `risk-medium` labels, remove `needs-live-test` once every selected check has passed, and mark the PR ready for review when review is clean, the live checks pass and CI is green. Do not merge.
