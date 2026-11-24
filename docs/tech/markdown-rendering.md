# Markdown Rendering in Runner

How a Markdown message or file becomes text on screen, why Runner has two renderers that do it differently, how Zed does the same job, and what [#1342](https://github.com/yicheng47/runner/issues/1342) and [#1443](https://github.com/yicheng47/runner/issues/1443) change. Written against Runner's `main` at `c8cd3b70` (2026-11-24), `pulldown-cmark` 0.13.6 and Zed's `main` at `4aa53044` (2026-11-23). It names files and functions, not line numbers.

No background is assumed beyond knowing what Markdown looks like. The [glossary](#glossary) at the end defines each term the note uses.

## The mental model

Every Markdown text Runner shows is its own small document: each feed message, each question card's prompt, each role's prompt, each `.md` file in Files. Rendering one is three jobs done in order:

```text
 source text            parse              lay out              draw
 ───────────           ───────▶ blocks    ────────▶ rows       ──────▶ pixels

 # Plan                 Heading("Plan")     row 0: Plan          GPUI text elements,
                                                                 one per row
 - run tests            List                row 1: • run tests
 - ship                   Item("run tests") row 2: • ship
                          Item("ship")
```

1. **Parse.** Read the characters and decide what they are: a heading, a paragraph, a list, a table, a code block, and inside the text, which words are bold, italic, code or links. This is the hard part, because Markdown is full of edge cases.
2. **Lay out.** Turn that structure into rows to draw top to bottom, with spacing, indentation, bullets and table columns. GPUI, Runner's UI framework, has no Markdown widget, so Runner writes this step itself.
3. **Draw.** Each row becomes GPUI elements: a `StyledText`, which is a string plus *text runs* saying which byte ranges are bold, coloured or underlined; an `InteractiveText` when the row has clickable links; an `img` for an image.

### Blocks and rows

Two words carry most of this note, and they are different things.

The **block model** is what the Markdown *is*: the parser's output, a tree of blocks such as headings, paragraphs, lists, quotes, tables and code blocks, with styled text inside them. Unifying Runner's renderers means unifying on one block model.

**Rows** are how a block is cut up for *drawing*: horizontal strips, the smallest pieces the screen builds. They are not a second model but a layout step after the blocks, and one block often becomes several rows:

| Block | Rows |
| --- | --- |
| Heading, rule, image | 1 |
| Paragraph | 1 in Files; 1 per line in the feed |
| List | 1 per item |
| Table | 1 per table row |
| Code block | 1 per code line, with long lines split every 512 bytes |
| Quote | The rows of the blocks inside it, drawn with a quote bar |

Rows exist for one reason, [explained below](#why-rows-drawing-only-whats-on-screen): so the app builds only what is on screen, even inside one huge block.

## Parsing: CommonMark and GFM

The original Markdown of 2004 was described in a blog post rather than defined, so tools disagreed about anything unusual. [CommonMark](https://spec.commonmark.org/) is the precise specification that settled those cases, with more than 600 numbered examples. [GitHub Flavored Markdown](https://github.github.com/gfm/) (GFM) adds tables, task lists (`- [x] done`), strikethrough (`~~old~~`) and bare links; GitHub also renders footnotes (`[^1]`) and alerts (`> [!NOTE]`). Agents write Markdown the way GitHub shows it, so a renderer that follows CommonMark and GFM shows what the agent meant.

A few of the cases the specification settles, which matter later in this note:

- `Summary` followed by `---` on the next line is a level-2 heading, not a paragraph and a horizontal rule.
- A line indented four spaces, outside a list, is a code block.
- A list item indented under another list item is a nested list.
- `2 * 3 * 4` stays as written: a `*` with a space on both sides cannot start italics.
- A single newline inside a paragraph is a *soft break*, which a standard renderer shows as a space, so hard-wrapped text reflows. A line ending in two spaces or a backslash is a *hard break*, a real line break.

That last rule is wrong for chat. People and agents press Enter once and expect a new line, so [#46](https://github.com/yicheng47/runner/issues/46) made the mission feed show every single newline as a line break. Files keeps the standard rule, because a Markdown file is a document. Zed makes the same split, as [below](#how-zed-does-it).

## pulldown-cmark

[`pulldown-cmark`](https://docs.rs/pulldown-cmark/0.13.6) is the CommonMark parser most Rust tools use, including rustdoc, mdBook and Zed. It is a *pull parser*: rather than building a tree, it hands its caller a stream of events, and the caller builds whatever structure it needs.

```text
source:  # Plan

         Run **tests**
         then ship

events:  Start(Heading H1)  Text("Plan")  End(Heading)
         Start(Paragraph)
           Text("Run ")  Start(Strong)  Text("tests")  End(Strong)
           SoftBreak
           Text("then ship")
         End(Paragraph)
```

Options switch the GFM extensions on. Files enables `ENABLE_TABLES`, `ENABLE_TASKLISTS` and `ENABLE_STRIKETHROUGH` today. Version 0.13 also has `ENABLE_FOOTNOTES`, `ENABLE_MATH` and `ENABLE_GFM` (alerts), which [#1447](https://github.com/yicheng47/runner/issues/1447) and [#1448](https://github.com/yicheng47/runner/issues/1448) turn on.

`Parser::into_offset_iter()` gives every event the byte range of the source it came from: its *source span*. With spans, "copy this block as Markdown" is a slice of the original text, exact to the author's spacing and markers, instead of Markdown rebuilt from the parsed structure. Runner does not record spans yet; [#1444](https://github.com/yicheng47/runner/issues/1444) adds them for block copy.

## Runner's two renderers today

Both renderers already have blocks and rows, but the feed's are much simpler:

| | Files (`files/markdown.rs`) | Feed and the other compact places (`mission_markdown.rs`) |
| --- | --- | --- |
| Blocks | `Block`: a real tree. A list holds items, items hold blocks, quotes nest | `MarkdownBlock`: a flat list. A list item stands alone with no list around it, a quote is one line, and nothing nests |
| Rows | `Row`, made by `flatten`: one per heading, paragraph, list item, table row, image, rule and code line | `FeedMarkdownPiece`: one per paragraph line and table row, but a whole code block, list item or quote is one piece. The feed packs 4 pieces into each entry of its list |

### Files: `surfaces/files/markdown.rs`

The Files viewer's `.md` preview, built for [#634](../features/archive/634-file-preview.md).

- **Parse.** `Page::parse` runs `pulldown-cmark` off the UI thread when the file is read, alongside loading its images. `Parser::blocks` turns the events into `Block`s. `bounded` caps quotes and lists at `MAX_NESTING` (32) levels, because a small file nested thousands deep would otherwise overflow the stack and crash the app.
- **Code.** A fenced block's language picks a tree-sitter grammar (`highlight::grammar_named`), and its code is highlighted. The language itself is not kept afterwards.
- **Lay out.** `flatten` turns blocks into `Row`s, with a long code line split every 512 bytes (`FRAGMENT_BYTES`).
- **Draw.** `MarkdownView` puts the rows in a `gpui::list`, which builds only the rows on screen. Fonts and size follow the Files settings, so `⌘+` and `⌘-` scale it.
- **Select and copy.** A position is a row and a byte offset in it. Dragging selects across rows; a table row or an image selects whole. `⌘C` copies from the rows and `⌘A` selects everything.
- **Links.** `route` opens `http`, `https` and `mailto` in the browser and relative paths as files inside the workspace root; anything else does nothing. A press that turns into a drag opens nothing.
- **Live reload.** When the file changes on disk, `matches_text` and `same_rendered` keep the scroll position and selection if the text is unchanged.

### Everywhere else: `surfaces/mission_markdown.rs`

A smaller renderer written by hand, used in five places:

| Place | Code | Draws only the rows on screen? | Selectable? |
| --- | --- | --- | --- |
| Mission feed messages | `mission_workspace/feed.rs` | yes | yes |
| A question card in the feed | `feed.rs`, `render_mission_ask_card` | no | yes |
| Role and crew prompt, and its editor preview | `profile_page.rs` | no | no |
| Settings → Skills document preview | `settings/skills.rs` | no | no |
| A chat's `Title ▾` Info popover, System prompt | `panes/panel.rs` | no | no |

- **Parse.** `parse_blocks` reads the text line by line, trims each line, and matches it against a short list of shapes: a ```` ``` ```` fence, a table (a line with `|` followed by a `---` separator), a blank line, a rule, a `#` to `###` heading, a `> ` quote, a `- ` or `* ` bullet, or a `1. ` number. Any other line joins the current paragraph. Inside a line, `append_inline_segment` looks for the next `**`, `*`, `` ` `` or `~~` pair, or a `[label](url)` link, and styles what is between.
- **Lay out and draw.** `FeedMarkdown::new` caches each feed message by its event and splits it into pieces. Code blocks are plain, never wrap, and scroll sideways. The other four places parse again on every render and draw everything, which is fine for a short prompt.
- **Select and copy.** Selection stays inside one message. A position is a *selection unit* and a byte offset, where a unit is a paragraph line, a heading, a list item, a quote, a whole code block or one table cell. `⌘C` parses the message again to find the text (`selected_plain_text`), and `Esc` clears the selection.
- **Links.** A click opens any URL with `cx.open_url`, whatever its scheme, and a drag that ends on a link still opens it.

### Where they disagree

The same Markdown, shown by each renderer today:

| Markdown | Feed, profile, Skills, Info | Files (CommonMark) |
| --- | --- | --- |
| `  - nested` under a bullet | a second bullet at the same level | a nested bullet |
| `#### Notes` | the text `#### Notes` | a level-4 heading |
| `- [x] done` | a bullet reading `[x] done` | a checked box |
| `2 * 3 * 4` | the stars vanish and the 3 turns italic | as written |
| two lines starting `> ` | two separate quotes | one quote |
| `Summary`, then `---` | a paragraph and a rule | a level-2 heading |
| a line indented four spaces | a paragraph | a code block |
| a wide table | 100 px or more per column, scrolls sideways | cells wrap ([#1236](https://github.com/yicheng47/runner/issues/1236)) |
| a 2,000-line code block | drawn whole as one piece | one row per line, drawn as it scrolls in |
| a link to `file:///etc/hosts` | opens | does nothing |
| one newline inside a paragraph | a line break (#46) | a space |

Every fix and feature so far has had to land twice or leave one side behind, which is why #1342 merges them before #1443 adds anything new.

## Why rows: drawing only what's on screen

Before GPUI can draw text it *shapes* it: it finds each character's glyph in the font, places the glyphs, and wraps lines to the available width. The cost grows with the amount of text. A long feed or a large file cannot be shaped in full on every frame, so Runner draws only what is on screen. A virtualized list (`gpui::list`) asks for one entry at a time as entries scroll into view, and forgets entries that scroll out.

The feed skips work at two levels:

1. **Messages.** Messages scrolled out of view are not built at all. [#962](https://github.com/yicheng47/runner/issues/962) added this after a long feed stalled the mission view.
2. **Rows inside a message.** Only the visible rows of a message are built.

The second level matters for one big message. Say an agent posts a 2,000-line log in a code block:

```text
          ┌─ screen ─┐
row  0    │          │   built
...       │          │
row 40    │          │   built
          └──────────┘
row 41..1999             not built: off screen
```

Without rows, the message is one unit: while any part of it is on screen, all 2,000 lines are laid out again on every redraw, and the feed redraws on every scroll step and every new event. With rows, only the 40 or so lines on screen are built. Files works the same way, so a 5,000-line README builds one screenful at a time. Tests hold both to it: `DRAWN_CODE_BYTES` in `surfaces/files/tests.rs`, and `long_feed_multiline_paragraph_and_table_build_only_visible_lines_and_cells` with the `PARSE_COUNT`, `RENDER_COUNT` and `LINE_COUNT` counters in `surfaces/mission_workspace/tests.rs`.

Rows cost two things. Anything with a border or background, like a code block's box, is drawn in strips, so each row knows whether it is the first or last of its block to draw the box's top or bottom. And a selection is a row and a position in it, instead of one position in the whole text.

## How Zed does it

Zed is the closest precedent: a GPUI app with one shared Markdown renderer, `crates/markdown`, used across the editor, including the agent panel, editor hovers, the git views, diagnostics and the Markdown preview. The preview, `crates/markdown_preview`, is a tab that follows the editor, syncs scrolling, searches and zooms; the drawing is all `markdown`'s.

| | Zed | Runner after #1342 |
| --- | --- | --- |
| Looks | `MarkdownFont::{Agent, Editor, Preview}` | Compact and Document |
| A single newline | A line break in the Agent look (`soft_break_as_hard_break`) and the git commit view; a space elsewhere | A line break in Compact; a space in Document |
| What the parse keeps | `pulldown-cmark`'s flat list of events, each with its source span, plus where each top-level block starts | A tree of blocks, laid out as rows |
| Parsing and code highlighting | In the background | In the background for Files; per message, cached, for the feed |
| Long documents | `MarkdownElement` walks every event and builds the whole document on each layout; the agent thread skips only whole messages off screen | Builds only the rows on screen, inside a message too |
| Selection and copy | Positions in the source text. **Copy** gives the text as shown; **Copy as Markdown** gives the source, with half-selected markers closed | A row and a position in it; Copy as Markdown can come with #1444 |
| Extras | Footnotes, GitHub alerts, heading anchors, YAML front matter as a table, some inline HTML through `html5ever`; no `<details>` | Footnotes, alerts and `<details>` in #1448 |
| Math | Off on purpose: a test lists `ENABLE_MATH` as unwanted | #1447 |
| Adding features | No plugin interface. Mermaid, HTML and front matter are built in and switched on per surface by flags in `MarkdownOptions` | The same: built in, switched on per look |

Zed renders Mermaid with the [`merman`](https://crates.io/crates/merman) crate (0.9.3), a Rust port of Mermaid, after moving off an earlier crate. In the background it turns the diagram's text into an SVG, recolours it to the theme, and has GPUI turn the SVG into an image, cached by the diagram's text; the old image stays up while a new one renders. Each diagram has a Diagram/Code switch, zoom on scroll and a copy button. About 1,700 of `crates/mermaid_render`'s 3,500 lines recolour and clean up the SVG: matching the theme is the real cost.

Runner takes Zed's shape: one renderer, a few looks, chat-style line breaks, built-in features instead of plugins. It differs on purpose in keeping rows. It can learn from Zed's design but not copy its code: Zed's `markdown`, `markdown_preview` and `mermaid_render` crates are GPL-3.0-or-later and Runner is MIT. `merman` itself is MIT or Apache-2.0.

## Phase 1: one renderer (#1342)

#1342 replaces both renderers with one at `crates/runner-app/src/ui/markdown/`, grown from the Files renderer. Every place parses with `pulldown-cmark` into the same block model. The feed's `MarkdownBlock` and `FeedMarkdownPiece` are deleted, and the Files `Block` and `Row` serve everyone. Two looks sit on top:

- **Document**, for Files: the Files font and size, a soft break shown as a space. Files looks and behaves as it does now.
- **Compact**, for the five other places: the app's body text, the feed's spacing, and a soft break that starts a new row, so #46 holds and a paragraph's lines are still built one at a time.

For people reading the feed and the other compact places, the left column of the table above becomes the right one, except the line breaks: nested lists, all heading levels, task boxes, wrapped tables and CommonMark's rules for emphasis, quotes, `---` headings and indented code. Code blocks wrap long lines and are drawn line by line; #1444 adds a toggle back to unwrapped lines. Links follow one rule: `http`, `https` and `mailto` open, other schemes do nothing, and a drag never opens a link.

Selection in a feed message moves to rows, as in Files: a code block selects line by line, and a table row selects whole and copies with tabs between its cells. `⌘C` reads the cached message instead of parsing it again.

What stays: the feed's line breaks, spacing, link hover underline, drag selection, `⌘C` and `Esc`, and smooth scrolling and typing; Files' look, selection, links, images, font scaling, `Rendered | Source` toggle and live reload. Feed code blocks stay unhighlighted until #1444.

The full list, the tests and the live checks are in the [#1342 spec](../features/1342-unified-markdown-renderer.md).

## Phase 2: built-in block renderers (#1443)

Phase 2 teaches the shared renderer new kinds of blocks, each built into Runner and turned on per look, as Zed does. There is no plugin interface: adding one means a module, the library it needs in `Cargo.toml`, and a switch in the looks that show it, and it ships in the next release. People do not install renderers or plug in their own: Runner stays curated (Jason, 2026-11-24). "Plugin" names a pattern in the code, not a system anyone can extend. Whether a feature gets a setting is designed before phase 2 starts; the starting point is none, with a rendered/source switch on each rich block and Settings → Side panel → Files → **Markdown opens as** as the one global switch. Rendering stays native, with no embedded web view or JavaScript runtime ([#1443](https://github.com/yicheng47/runner/issues/1443), [#1260](https://github.com/yicheng47/runner/issues/1260)).

A slow block, such as a diagram, works in two steps, so the screen never waits:

1. **Prepare, off the UI thread.** A ```` ```mermaid ```` fence goes to the Mermaid library, which produces an SVG or an error.
2. **Draw, on the UI thread.** The SVG as an image row, or the source text with a notice while it is still being prepared or when it failed.

```text
a mermaid fence ─▶ background: Mermaid library ─▶ SVG ─▶ GPUI img row
                                └─ error ─────────────▶ source text + notice
```

| Issue | Adds | How |
| --- | --- | --- |
| [#1444](https://github.com/yicheng47/runner/issues/1444) | Code-block copy, wrap toggle and language label; feed syntax highlighting; copying a whole block | Records each block's source span, the block each row belongs to and the code language, then adds a header row above each code block |
| [#1446](https://github.com/yicheng47/runner/issues/1446) | Mermaid diagrams | Evaluate `merman`, Zed's choice, first, with `mermaid-rs-renderer` as the fallback |
| [#1447](https://github.com/yicheng47/runner/issues/1447) | LaTeX math | Display math (`$$…$$`) as a block; inline `$x$` also needs an image inside a line of text. Zed has no precedent. Candidates: `ratex-gpui` or Typst's math layout |
| [#1448](https://github.com/yicheng47/runner/issues/1448) | Footnotes, GitHub alerts, `<details>` | `pulldown-cmark` already parses footnotes and alerts once its options are on; the renderer draws them |

## Glossary

- **Block / inline.** Blocks stack vertically: headings, paragraphs, lists, quotes, code blocks, tables, rules. Inline content is styling inside a block's text: bold, italics, code spans, links, images.
- **Block model.** The parser's output: the tree of blocks a document is made of. What the Markdown *is*.
- **CommonMark / GFM.** The precise Markdown specification, and GitHub's extensions to it.
- **Event / pull parser.** A parser that hands out a stream of start, text and end events instead of a finished tree.
- **Look.** A presentation of the same block model: Document for Files, Compact everywhere else.
- **Row.** A horizontal strip cut from a block for drawing: a heading, a paragraph or one of its lines, a list item, a table row, a code line. What the screen builds.
- **Shaping.** Turning characters into placed glyphs, wrapped to a width: the expensive part of drawing text.
- **Soft break / hard break.** A single newline inside a paragraph, shown as a space in Document and as a line break in Compact; and an explicit line break.
- **Source span.** The byte range of the original text a block came from.
- **Text run.** A byte range of a string with one style: font, weight, colour, underline.
- **Tree-sitter.** The parsing library Files uses to highlight code by language.
- **Virtualization.** Building only what is on screen, so the cost of drawing does not grow with the length of the text.

## Reading path

1. The `pulldown-cmark` 0.13.6 types `Event`, `Tag` and `Options`, in `~/.cargo/registry/src/*/pulldown-cmark-0.13.6/src/lib.rs`.
2. `crates/runner-app/src/surfaces/files/markdown.rs`: `Page::parse`, `Parser::blocks`, `bounded`, `flatten`, `MarkdownView::render`, then `Draw::selectable` for selection and links.
3. `crates/runner-app/src/surfaces/mission_markdown.rs`: `parse_blocks`, `append_inline_segment`, `FeedMarkdown::new` and `selected_plain_text`.
4. `crates/runner-app/src/surfaces/mission_workspace/feed.rs`: `cached_feed_markdown`, `render_mission_message_row` and `copy_feed_selection`.
5. In a Zed checkout, `crates/markdown/src/parser.rs` for `PARSE_OPTIONS` and the event list; `crates/markdown/src/markdown.rs` for `MarkdownOptions`, `MarkdownStyle::themed` and `MarkdownElement`'s `request_layout`; and `crates/mermaid_render/src/mermaid_render.rs` for the Mermaid pipeline.
6. The phase 1 spec, [`1342-unified-markdown-renderer.md`](../features/1342-unified-markdown-renderer.md).
