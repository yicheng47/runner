# 1342 — Unified Markdown Renderer

> Tracking issue: [#1342](https://github.com/yicheng47/runner/issues/1342), a cleanup sub-issue of [#1248](https://github.com/yicheng47/runner/issues/1248) and phase 1 of the Markdown renderer umbrella [#1443](https://github.com/yicheng47/runner/issues/1443). Scheduling and priority live on the issues and in the [roadmap](../roadmap.md). Platforms: macOS, Windows and Linux.
> Status: draft spec. #1342 adds no controls or screens, so it has no Pencil frames; the follow-up controls are drawn with [#1444](https://github.com/yicheng47/runner/issues/1444).
> Background: [Markdown rendering in Runner](../tech/markdown-rendering.md) explains blocks, rows, the two current renderers and how Zed renders Markdown.
> Builds on: [#634](archive/634-file-preview.md) (the Files Markdown preview), [#962](../tests/archive/962-virtualize-mission-feed.md) (the virtualized mission feed) and [#46](https://github.com/yicheng47/runner/issues/46) (single newlines kept in feed messages).

## Motivation

Runner renders Markdown twice:

- `crates/runner-app/src/surfaces/files/markdown.rs` serves the Files preview. It parses with `pulldown-cmark` off the UI thread into a tree of blocks, highlights code with tree-sitter, lays the blocks out as rows that a virtualized list draws, routes links safely, draws local images and caps nesting at `MAX_NESTING` (32).
- `crates/runner-app/src/surfaces/mission_markdown.rs` serves five other places: mission feed messages, question (ask) cards in the feed, the role and crew prompt and its editor preview (`surfaces/profile_page.rs`), the Settings → Skills preview (`surfaces/settings/skills.rs`) and the System prompt in a chat's `Title ▾` Info popover (`surfaces/panes/panel.rs`). It parses line by line by hand into a flat list of blocks.

The hand-written parser misses most of Markdown: nested lists, headings below level 3, task lists, multi-line quotes and CommonMark's rules for emphasis and code. Fixes land in one renderer only: [#1236](https://github.com/yicheng47/runner/issues/1236) made table cells wrap in Files and left the feed's tables alone. The feed also opens a link of any scheme, `file:` included, where Files opens only web and mail links. Every feature planned under #1443 would have to be built twice.

## Two phases

1. **Phase 1, this spec (#1342):** one block model and one renderer for all six places. No new features.
2. **Phase 2, #1443's sub-issues:** code-block controls and block copy ([#1444](https://github.com/yicheng47/runner/issues/1444)), Mermaid diagrams ([#1446](https://github.com/yicheng47/runner/issues/1446)), LaTeX math ([#1447](https://github.com/yicheng47/runner/issues/1447)) and footnotes, alerts and `<details>` ([#1448](https://github.com/yicheng47/runner/issues/1448)). Each is built into the renderer and turned on per look, as Zed does with its `MarkdownOptions`. There is no plugin interface, and nothing users install or configure: Runner stays curated (Jason, 2026-11-24). Whether any of these features gets a setting is designed before phase 2 starts (Jason, 2026-11-24). The starting point is none: each rich block gets its own rendered/source switch, and Settings → Side panel → Files → **Markdown opens as** stays the one global switch.

## Decisions

1. **The Files renderer becomes the shared one.** Its `pulldown-cmark` 0.13 parser, `Block` tree, `Row` layout, tree-sitter highlighting, `MAX_NESTING` guard, 512-byte code fragments and link routing are kept. The feed's `MarkdownBlock`, `FeedMarkdownPiece`, `parse_blocks` and inline scanner are deleted.
2. **Keep rows.** Zed's shared `markdown` crate is the closest precedent: one renderer for its agent panel, hovers and preview, with a few looks and the agent look showing single newlines as line breaks, as #46 does. Zed builds a whole document on every layout, though, and Runner keeps building only the rows on screen, inside a document as well, because a feed message or a Files document can be thousands of lines. #962's feed tests and the Files `DRAWN_CODE_BYTES` test hold this.
3. **Learn from Zed and `gpui-kit`; depend on neither.** Zed's `markdown`, `markdown_preview` and `mermaid_render` crates are GPL-3.0-or-later and Runner is MIT, so their code cannot be copied. `gpui-base` stays out for the reasons in [#771](https://github.com/yicheng47/runner/issues/771).
4. **Frontend only.** Everything stays in `crates/runner-app/`: no change to `runner-core`, `runner-daemon`, `runner-cli`, the client protocol, the database or settings.

## Design

### Module

`crates/runner-app/src/ui/markdown/`, moved from `surfaces/files/markdown.rs`, keeps using Files' `document`, `highlight` and `syntax` modules. A suggested split: parsing (`pulldown-cmark` events to blocks), layout (blocks to rows for a look), drawing (row elements, links, selection highlight), the Files `MarkdownView`, the feed's `FeedMarkdown`, and links (`route`, `image_format`). Both `surfaces/files/markdown.rs` and `surfaces/mission_markdown.rs` are deleted.

### Block model and rows

- **Parsing.** `pulldown-cmark` with tables, task lists and strikethrough, as Files parses today. Footnotes, math and GitHub alerts stay off until #1447 and #1448. `bounded` caps nesting at `MAX_NESTING` on every surface. Raw HTML shows as written.
- **Blocks.** Files' `Block` tree, unchanged: headings of all six levels, paragraphs, standalone images, code, quotes, lists with task boxes, tables, rules and HTML.
- **Rows.** Files' `Row`s, unchanged in Document: one per heading, paragraph, image, rule, table row, line of HTML and line of code, with code lines split every 512 bytes. In Compact, a paragraph splits at each soft break into one row per line.
- **Not in phase 1.** A block's source span, the block each row belongs to and the code fence's language are added by #1444, with the code-block header and block copy that first read them. A field nothing reads fails the dead-code lint in CI, and once both renderers share one parser, adding them there is a small change in one place.

### Two looks

| | Document | Compact |
| --- | --- | --- |
| Used by | Files | Feed messages, question cards, role and crew prompt and editor preview, Skills preview, Info popover System prompt |
| Text | The Files font and size; `⌘+` and `⌘-` zoom | The app's body text, as the feed today |
| Spacing | Files' row spacing, unchanged | The feed's block margins today: 6 px above and below a paragraph or list item, 4 px between list items, 12/6 px around H1, 12/4 px around H2, 8/4 px around H3 to H6, 8/8 px around quotes, code and tables, 12/12 px around rules |
| Single newline | A space | A new row: a line break, as #46 requires |
| Links | `http`, `https` and `mailto` open in the browser; relative paths open files inside the workspace root; anything else does nothing | `http`, `https` and `mailto` open in the browser; anything else does nothing. The underline brightens on hover, as in the feed today |
| Images | Local images drawn | Alt text, with no disk access |
| Code blocks | Highlighted; long lines wrap | Plain text until #1444; long lines wrap |
| Selection colour | The accent at 28% | The terminal palette's selection colour, as the feed today |

### Where each place draws

| Place | Drawn as | Selection |
| --- | --- | --- |
| Files preview | `MarkdownView`: a `gpui::list` over the page's rows | Across the page; `⌘C`, `⌘A`; kept over a live reload when the text is unchanged (`matches_text`, `same_rendered`) |
| Feed messages | `FeedMarkdown`, cached per event; up to 4 rows in each entry of the feed's list, as `PIECES_PER_ROW` packs pieces today | Inside one message |
| Question cards in the feed | All rows in a plain element; cached per card, as today | Inside the card, as for a message |
| Role and crew prompt and editor preview, Skills preview, Info popover System prompt | All rows in a plain element, parsed on render as today, since these texts are short | None |

### Selection and copy in the feed

This answers the issue's open question. Dragging across blocks still works, inside one message, but a position becomes a row and a byte offset in its text, as in Files, instead of a feed-only selection unit. `FeedSelection` keeps its event ID, anchor and head. Two things select differently: a table row selects whole and copies with a tab between cells, where today each cell selects on its own; and a code block selects line by line, where today it is one unit. Copy joins rows with line breaks, without one before the continued piece of a long code line, and reads the cached `FeedMarkdown` instead of parsing the message again, so `selected_plain_text` goes. `Esc` clears the selection, and it survives scrolling the message away and back. A press that turns into a drag never opens a link.

### Intentional changes

In the five compact places only; Files does not change. The pull request lists these:

- Nested lists, headings H4 to H6, task boxes, backslash escapes and CommonMark emphasis: `2 * 3 * 4` stays as written instead of losing its stars to italics.
- A quote over several lines is one quote, each line on its own row, instead of one quote per line.
- `Summary` followed by `---` is a heading, and a line indented four spaces outside a list is a code block.
- Table cells wrap ([#1236](https://github.com/yicheng47/runner/issues/1236)) instead of each column taking at least 100 px and the table scrolling sideways.
- Code blocks wrap long lines instead of scrolling sideways, and are drawn line by line. #1444 adds a toggle back to unwrapped lines.
- `![alt](path)` shows its alt text instead of `!` and a link.
- Only `http`, `https` and `mailto` links open; a `file:` or other link does nothing.
- Selection in a message works by rows, as above.

What stays: in Files, everything, including the look, selection, live reload, scroll position, images, links, zoom and the `Rendered | Source` toggle. In the feed, single newlines, block spacing, the link hover underline, drag selection with `⌘C` and `Esc`, selection that survives scrolling, no parsing again on a warm redraw or a keystroke in the composer, and building only the rows on screen. The profile, Skills and Info previews stay read-only and look as they do now.

### What phase 2 builds on

- **#1444**, code-block controls and block copy: adds each block's source span, the block each row belongs to and the code language; a header row above a code block's first line for the language label, wrap toggle and copy button; Copy (the text as shown) and Copy as Markdown (the source slice), as Zed offers; and tree-sitter highlighting in Compact, which the shared renderer already has.
- **#1446**, Mermaid: evaluate `merman` first (MIT or Apache-2.0, needs Rust 1.95, where Runner is on 1.97.1), which Zed moved to from another crate, with `mermaid-rs-renderer` as the fallback. Render in the background to SVG, recolour it to the theme, cache it by the diagram's text, and show the source while it renders or when it fails.
- **#1447**, math: turns on `ENABLE_MATH`. Zed renders no math, so there is no precedent; inline `$x$` needs an image inside a line of text.
- **#1448**, footnotes, alerts and `<details>`: turns on `ENABLE_FOOTNOTES` and `ENABLE_GFM` and draws what they parse.

### Not in scope

New features, settings, shortcuts or controls; a plugin interface; anything users install or configure; web views or a JavaScript runtime. No `docs/guide/` page describes how the feed formats messages, and Files does not change, so the guide needs no update.

## Implementation steps

1. **Move.** Move the Files renderer to `ui/markdown/` and point Files at it, with no change in behaviour: every Files test passes unchanged. A commit of its own.
2. **Compact and the feed.** Add the Compact look and its row per line; rebuild `FeedMarkdown` on the shared rows; move feed messages and question cards over, with row-based selection and copy from the cache.
3. **The other places.** Move the role and crew prompt, the Skills preview and the Info popover, then delete `surfaces/mission_markdown.rs`.
4. **Tests and the pull request.** Port and add the tests below, and list the intentional changes in the pull request.

## Verification

### Automated

- Every Files Markdown test passes unchanged after the move: parsing, nesting on a 512 KiB stack, `route`, code fragments, selection, live reload and `DRAWN_CODE_BYTES`.
- New Compact tests: a row per line at soft breaks; nested lists; H4 to H6; task boxes; a `---` heading; indented code; `2 * 3 * 4`; a multi-line quote; alt text for images; only `http`, `https` and `mailto` links routed.
- The feed's virtualization tests in `surfaces/mission_workspace/tests.rs` pass with `PARSE_COUNT`, `RENDER_COUNT` and `LINE_COUNT`: the 2,000-message feed, the 2,000-line paragraph and the 2,000-row table. A new one: a 2,000-line code block builds only the lines on screen.
- Feed selection: across a paragraph, a table row (whole, tab-separated) and code lines; copy after scrolling away and back; `Esc`; copy does not change `PARSE_COUNT`; a drag that ends on a link opens nothing.
- The feed's narrow-width wrapping, block spacing and link-hover tests are ported to the shared renderer.
- Gates: `cargo test -p runner-app`, `make clippy`, `cargo fmt --all -- --check` and `git diff --check`, plus a 20-run nextest stress loop for any new test that waits on background work.

### Live checks

On the macOS development app, by `qa` holding `script/qa-lock`:

1. **Feed and question cards** (with `MIS-FEED-03`): post messages with multi-line paragraphs, nested lists, task boxes, an H4, a two-line quote, a wide table, a long code block, and an `https` and a `file:` link, then ask a question with a Markdown prompt. Single newlines stay on their own lines, lists nest, table cells and code lines wrap, the `https` link opens and the `file:` link does nothing, a drag across a paragraph, a table row and code lines copies the expected text and `Esc` clears it, the selection survives scrolling away and back, scrolling and typing stay smooth, and the question card's prompt renders and selects. Reason: feed messages and question cards move to the shared renderer.
2. **Files** (with `WS-FILES-06`): open a Markdown file with headings, nested and task lists, highlighted code, a wide table, a local image, and a relative and a web link. It looks as it did before the change; drag selection, `⌘C`, `⌘A`, `⌘+` and `⌘-`, `Rendered | Source` and both links work; the selection survives an identical rewrite on disk. Reason: the Files renderer moves.
3. **The other compact places** (with `SET-SKILL-01`): a role and a crew prompt preview and editor preview, a Skills preview and the Info popover's System prompt, in dark and light themes. Each renders and reads as before, with the intentional changes above. Reason: these move to the shared renderer.

Selected regression cases: `MIS-FEED-03`, `WS-FILES-06`, `SET-SKILL-01`.
