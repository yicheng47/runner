# 1444 — Code block header

Tracking issue: [#1444](https://github.com/yicheng47/runner/issues/1444), part of the Markdown rendering features umbrella [#1443](https://github.com/yicheng47/runner/issues/1443). Builds on the unified renderer from [#1342](https://github.com/yicheng47/runner/issues/1342) ([spec](./1342-unified-markdown-renderer.md), [explainer](../tech/markdown-rendering.md)).

Design: `runner/desktop/specs/1443-markdown-rendering-features.pen` in the design repo, board `Bka9r` (signed off 2026-11-24).

## Motivation

Copying a code block from an agent's message or a rendered Markdown file means drag-selecting across its lines, a fence's language is not shown anywhere, and long lines always wrap. Feed code blocks are also plain text, while Files highlights the same block as code. Every place that renders Markdown should give a code block the same small set of controls, and the feed should highlight code as Files does.

## Scope

In both looks of the shared renderer at `crates/runner-app/src/ui/markdown/`: the Document look of Files' rendered pages, and the Compact look of feed messages, question cards, the role and crew prompt previews, the Skills preview and the Info popover's System prompt.

- **Header bar.** Every code block, fenced or indented, gets a bar above its first line: the fence's language on the left, always visible, and Wrap and Copy on the right, shown while the pointer is over the bar or any line of the block.
- **Language.** The first word of the fence's info string, as the parser already reads it (`rust` for ` ```rust title="x" `). A block without one, unlabelled or indented, shows the bar with no label.
- **Wrap.** Long lines wrap by default, as today. The Wrap button turns wrapping off for that block: its lines keep their length and the block scrolls sideways as one, with a thin scrollbar under its last line. Pressing it again wraps. The icon is lit while wrapping is on. The choice is per block, held by the view while it is open, and never saved; there is no setting.
- **Copy.** Copies the block's code without the fences, the language, the bar, or the single line ending (`\r\n` or `\n`) before the closing fence, so pasting a copied shell snippet into a terminal does not run it immediately; intentional trailing blank lines before the closing fence keep their line endings (Jason, 2026-11-24). The icon turns into a check and **Copied** for two seconds.
- **Feed highlighting.** The Compact look highlights code with the same tree-sitter grammars and theme colours as Files.
- **Tooltips.** **Wrap lines** and **Copy code**, as other icon buttons in the app show theirs.

Other blocks get no copy button: their text is copied by selecting it, as today (Jason, 2026-11-24). Not in this change: line numbers, collapsing long blocks, opening a block in an editor or saving it to a file, Mermaid's `< >` toggle (#1446) and display math (#1447), which later add their controls to this bar.

## Design

The bar is a row of its own, so virtualization still draws only what is on screen.

- **Rows.** A new row kind for the bar goes before a code block's first line row. The bar takes the block's top border and rounded corners; the first line row loses them, and a divider separates the bar from the lines. Each look keeps its current block colours, padding and code size; the bar is 28 px at the default size in both looks and scales with the viewer's font size in Files.
- **Look.** From the design: the language in the code font at 11 px in the muted text colour; 14 px icons in the secondary text colour; the lit Wrap icon and the **Copied** confirmation in the accent colour.
- **Block identity.** Every row of a block already holds its `Arc<CodeBlock>`, which is enough to find the block a hovered row belongs to and to key its wrap state and copy confirmation. Add the language to `CodeBlock`. Block copy left #1444's scope, so the source spans and general row-to-block index that the #1342 spec expected here are not needed: a field nothing reads fails the dead-code lint.
- **Unwrapped lines.** All rows of an unwrapped block share one horizontal offset, held with its wrap state, so the lines move together; the scrollbar's width comes from the block's widest line in columns, which a monospace font makes exact enough without laying out every line. A horizontal wheel or trackpad gesture over any row of the block scrolls it. Code lines are still split into rows every 512 bytes, so a line longer than that continues on the next row even when unwrapped.
- **Selection.** The bar is not selectable: dragging across a code block selects and copies only its code, and a drag that starts on the bar's buttons selects nothing. `⌘C`, `⌘A` and feed selection behave as now.
- **Feed highlighting.** Compact parses with highlighting on. Parsing a feed message is cached, and that must stay true: highlighting must not add parses, and must not make opening or scrolling a long feed slower. If highlighting a large block where the feed parses it is measurably slow, highlight in the background and show plain text until it is done, as the code viewer does for a large file.
- **Copied timer.** The confirmation's two seconds run on a GPUI timer owned by the view, and tests drive it with the test executor's clock, never with a real sleep.

## Implementation steps

1. **Model and rows.** The language on `CodeBlock`, the header row kind in both looks, and the first line's borders moved to the bar. Every existing Files and feed test passes, with the new row counted where tests count rows.
2. **Copy and Wrap.** Hover per block, the two buttons with tooltips, copy with its confirmation, and unwrapped lines with the shared offset and scrollbar.
3. **Feed highlighting.** Highlighting in Compact, within the cache and virtualization rules above.
4. **Guide and tests.** The guide pages below and the tests in Verification.

## Guide pages

- `docs/guide/guides/chats-and-windows.md` and `docs/guide/zh-CN/guides/chats-and-windows.md`: the rendered Markdown paragraph gains the header bar, Wrap and Copy.
- The missions guide's feed section, in both locales, if it describes how messages render.

## Verification

### Automated

- Parsing: the language is the info string's first word, and unlabelled and indented blocks have none.
- Rows: one bar row before each code block's first line in both looks; the bar's text is empty to selection and copy; a drag across a block copies only its code.
- Copy: the clipboard gets the block's code without fences or the single fence-closing line ending; the confirmation shows, then clears after two seconds of test-executor time.
- Wrap: off draws the block's lines unwrapped with one shared offset; on again wraps them; the state belongs to one block only.
- Virtualization: the feed tests with `PARSE_COUNT`, `RENDER_COUNT` and `LINE_COUNT` pass, including the 2,000-message feed and the 2,000-line code block, which still builds only the lines on screen; highlighting adds no parses.
- Feed highlighting: a `rust` block in Compact carries highlights; a block in an unknown language stays plain.
- Gates: `cargo test -p runner-app`, `make clippy`, `cargo fmt --all -- --check` and `git diff --check`, plus a 20-run nextest stress loop for any new test that waits on background work or a timer.

### Live checks

On the macOS development app, by `qa` holding `script/qa-lock`:

1. **Files** (with `WS-FILES-06`): open a Markdown file with `rust`, `sh`, unlabelled and indented code blocks, one with a line far wider than the panel. Each block has the bar, with its language or none; Wrap and Copy appear when hovering any line; Copy puts exactly the code on the clipboard and shows **Copied** for about two seconds; Wrap off shows the long line unwrapped, scrolling sideways with its block, and Wrap on wraps it again; highlighting is unchanged; a drag across a block copies only its code; `⌘+` and `⌘−` scale the bar with the text; dark and light themes. Reason: the header, Copy and Wrap are new in the Document look.
2. **Feed** (with `MIS-FEED-03`'s scrolling, selection and append steps): in an owned mission, post messages with a `rust` block, a long `sh` line and a 2,000-line block into a long feed. Code is highlighted in the theme's colours; the bar, Copy and Wrap behave as in Files; scrolling and appends stay smooth and pinned; a selection across code still copies the exact text. Reason: the header and highlighting are new in the Compact look and the feed is virtualized.
3. **Other compact places** (with `SET-SKILL-01`'s preview step): a Skills preview and a role prompt preview with a code block show the bar, and Copy works. Reason: they share the Compact look.

Selected regression cases: `WS-FILES-06`, `MIS-FEED-03`, `SET-SKILL-01`.
