use std::cell::RefCell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, point, px, Context, FontWeight, Modifiers, MouseButton, Render, TestAppContext, TextRun,
    VisualTestContext, Window,
};
use pulldown_cmark::Alignment;

use super::code::CompactCodeStates;
use super::draw::{
    compact_code_highlights, compact_highlights, openable_links, with_selection, MarkdownLinkHover,
};
use super::feed::{clear_compact_syntax_theme_cache, SYNTAX_THEME_BUILDS};
use super::layout::fragments;
use super::*;
use crate::surfaces::files::syntax::{SyntaxTheme, CAPTURES};
use crate::theme;

fn deep_files() -> Vec<String> {
    const DEEP: usize = 20_000;
    vec![
        format!("{} deep", ">".repeat(DEEP)),
        format!("{}deep", "- ".repeat(DEEP)),
        format!("{}deep", "1. ".repeat(DEEP)),
        format!("{}deep", "> - ".repeat(DEEP)),
    ]
}

/// Run alone in a process by `a_deeply_nested_file_parses_in_a_small_stack`,
/// since a stack overflow aborts the process it happens in.
#[test]
#[ignore = "run by a_deeply_nested_file_parses_in_a_small_stack"]
fn deeply_nested_files_on_a_small_stack() {
    std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            for file in deep_files() {
                let page = Page::parse(&file, "deep.md");
                let last = page.rows.last().expect("a row");
                assert_eq!(last.text().trim(), "deep", "{}", &file[..8]);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn a_deeply_nested_file_parses_in_a_small_stack() {
    let module = module_path!().split_once("::").unwrap().1;
    let name = format!("{module}::deeply_nested_files_on_a_small_stack");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--ignored", "--nocapture"])
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("1 passed"),
        "{}\n{stdout}\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn nesting_past_the_limit_flattens_and_shows_everything() {
    let quotes = format!("{} deep\n", ">".repeat(100));
    let page = Page::parse(&quotes, "a.md");
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].text(), "deep");
    assert_eq!(page.rows[0].quotes as usize, MAX_NESTING);

    let mut lists = String::new();
    for level in 0..40 {
        lists.push_str(&format!("{}- level {level}\n", "  ".repeat(level)));
    }
    lists.push_str(&format!("{}- sibling\n", "  ".repeat(39)));
    let page = Page::parse(&lists, "a.md");
    let text: Vec<String> = page.rows.iter().map(|row| row.text()).collect();
    for level in 0..40 {
        let found = format!("level {level}");
        assert!(
            text.iter().any(|row| row.contains(&found)),
            "{found} in {text:?}"
        );
    }
    assert!(
        page.rows
            .iter()
            .all(|row| row.indent as usize <= MAX_NESTING),
        "indent stays bounded"
    );
    let last = text.last().unwrap();
    assert!(
        last.contains("sibling"),
        "items past the limit keep their order: {text:?}"
    );
    assert!(
        !text.iter().any(|row| row.contains("level 39sibling")),
        "and stay apart: {text:?}"
    );
}

#[test]
fn a_list_at_the_limit_keeps_its_items_text() {
    // Quote, list and item nest in turn, so the limit falls on each.
    for levels in 1..=MAX_NESTING {
        let file = format!("{}deep", "> - ".repeat(levels));
        let page = Page::parse(&file, "a.md");
        assert_eq!(page.rows.last().unwrap().text().trim(), "deep", "{levels}");
    }
    for levels in 1..=MAX_NESTING {
        let file = format!("{}deep", "- ".repeat(levels));
        let page = Page::parse(&file, "a.md");
        assert_eq!(page.rows.last().unwrap().text().trim(), "deep", "{levels}");
    }
}

#[test]
fn nesting_up_to_the_limit_is_unchanged() {
    let quotes = format!("{} kept\n", ">".repeat(MAX_NESTING));
    assert_eq!(
        Page::parse(&quotes, "a.md").rows[0].quotes as usize,
        MAX_NESTING
    );
    let page = Page::parse("- a\n  - b\n", "a.md");
    assert_eq!(page.rows[1].indent, 2);
    assert_eq!(page.rows[1].marker, Some(Marker::Bullet));
}

#[test]
fn links_open_urls_and_files_inside_the_root_only() {
    let from = "docs/guide/README.md";
    assert_eq!(
        route("https://runner.dev/a?b#c", from),
        LinkTarget::Url("https://runner.dev/a?b#c".into())
    );
    assert_eq!(
        route("mailto:a@b.c", from),
        LinkTarget::Url("mailto:a@b.c".into())
    );
    assert_eq!(route("#install", from), LinkTarget::None);
    assert_eq!(route("javascript:alert(1)", from), LinkTarget::None);
    assert_eq!(route("file:///etc/passwd", from), LinkTarget::None);
    assert_eq!(route("C:/Windows/win.ini", from), LinkTarget::None);
    assert_eq!(route("..\\..\\secret", from), LinkTarget::None);
    assert_eq!(
        route("setup.md", from),
        LinkTarget::File("docs/guide/setup.md".into())
    );
    assert_eq!(
        route("./setup.md#step-2", from),
        LinkTarget::File("docs/guide/setup.md".into())
    );
    assert_eq!(
        route("../../crates/a%20b.rs?plain=1", from),
        LinkTarget::File("crates/a b.rs".into())
    );
    assert_eq!(
        route("/Cargo.toml", from),
        LinkTarget::File("Cargo.toml".into())
    );
    assert_eq!(
        route("../../../outside.md", from),
        LinkTarget::None,
        "leaves the root"
    );
    assert_eq!(route("/../outside.md", from), LinkTarget::None);
    assert_eq!(route("../..", from), LinkTarget::None, "the root itself");
    assert_eq!(
        route("img.png", "README.md"),
        LinkTarget::File("img.png".into())
    );
}

fn page(text: &str) -> Page {
    Page::parse(text, "docs/README.md")
}

#[test]
fn headings_paragraphs_and_inline_styles_parse() {
    let page =
        page("# Runner\n\nA *local* **cockpit** with `code`, ~~old~~ and [a guide](guide.md).\n");
    let [Block::Heading(1, title), Block::Paragraph(text)] = &page.blocks[..] else {
        panic!("{:?}", page.blocks);
    };
    assert_eq!(title.text, "Runner");
    assert_eq!(text.text, "A local cockpit with code, old and a guide.");
    let style_of = |word: &str| {
        let at = text.text.find(word).unwrap();
        text.styles
            .iter()
            .find(|(range, _)| range.contains(&at))
            .unwrap()
            .1
    };
    assert!(style_of("local").emphasis);
    assert!(style_of("cockpit").strong);
    assert!(style_of("code").code);
    assert!(style_of("old").strike);
    assert!(style_of("guide").link);
    assert_eq!(text.links.len(), 1);
    assert_eq!(&text.text[text.links[0].0.clone()], "a guide");
    assert_eq!(&*text.links[0].1, "guide.md");
}

#[test]
fn lists_task_lists_and_nested_blocks_parse() {
    let page =
        page("- one\n- [x] done\n- [ ] todo\n  - nested\n\n3. three\n4. four\n\n> quoted\n\n---\n");
    let Block::List { start: None, items } = &page.blocks[0] else {
        panic!("{:?}", page.blocks);
    };
    assert_eq!(items.len(), 3);
    assert_eq!(items[0].task, None);
    assert_eq!(items[1].task, Some(true));
    assert_eq!(items[2].task, Some(false));
    let Block::Paragraph(text) = &items[1].blocks[0] else {
        panic!("{:?}", items[1].blocks);
    };
    assert_eq!(text.text, "done");
    assert!(
        matches!(items[2].blocks[1], Block::List { .. }),
        "{:?}",
        items[2].blocks
    );
    let Block::List {
        start: Some(3),
        items,
    } = &page.blocks[1]
    else {
        panic!("{:?}", page.blocks);
    };
    assert_eq!(items.len(), 2);
    assert!(
        matches!(&page.blocks[2], Block::Quote(inner) if matches!(inner[0], Block::Paragraph(_)))
    );
    assert!(matches!(page.blocks[3], Block::Rule));
}

#[test]
fn code_blocks_are_highlighted_as_their_language() {
    let page = page("```rust\nfn main() {}\n```\n\n```\nplain\n```\n\n    indented\n");
    let Block::Code(rust) = &page.blocks[0] else {
        panic!("{:?}", page.blocks);
    };
    assert_eq!(rust.document.line(0), "fn main() {}");
    assert_eq!(rust.document.line_count(), 1);
    let spans = rust.highlights.as_ref().expect("highlighted").line(0);
    assert_eq!(
        crate::surfaces::files::syntax::CAPTURES[spans[0].capture as usize],
        "keyword"
    );
    let Block::Code(plain) = &page.blocks[1] else {
        panic!("{:?}", page.blocks);
    };
    assert!(plain.highlights.is_none());
    assert!(matches!(&page.blocks[2], Block::Code(code) if code.document.line(0) == "indented"));
}

#[test]
fn tables_keep_their_alignment_and_cells() {
    let page = page("| Role | Slots |\n| :--- | ---: |\n| **coder** | 1 |\n| reviewer | 2 |\n");
    let Block::Table {
        alignments,
        header,
        rows,
    } = &page.blocks[0]
    else {
        panic!("{:?}", page.blocks);
    };
    assert_eq!(alignments, &[Alignment::Left, Alignment::Right]);
    assert_eq!(
        header
            .iter()
            .map(|cell| cell_text(cell))
            .collect::<Vec<_>>(),
        ["Role", "Slots"]
    );
    assert_eq!(rows.len(), 2);
    let [Piece::Text(coder)] = &rows[0][0][..] else {
        panic!("{:?}", rows[0][0]);
    };
    assert_eq!(coder.text, "coder");
    assert!(coder.styles[0].1.strong);
}

#[test]
fn html_shows_as_text_and_footnotes_and_math_stay_as_written() {
    let page = page("<div align=\"center\">\n  <b>hi</b>\n</div>\n\nInline <kbd>⌘</kbd> key.\n\nA note[^1] and $x$.\n\n[^1]: The note.\n");
    let Block::Html(html) = &page.blocks[0] else {
        panic!("{:?}", page.blocks);
    };
    assert_eq!(html, "<div align=\"center\">\n  <b>hi</b>\n</div>");
    let Block::Paragraph(text) = &page.blocks[1] else {
        panic!("{:?}", page.blocks);
    };
    assert_eq!(text.text, "Inline <kbd>⌘</kbd> key.");
    let Block::Paragraph(text) = &page.blocks[2] else {
        panic!("{:?}", page.blocks);
    };
    assert_eq!(text.text, "A note[^1] and $x$.");
    assert!(matches!(&page.blocks[3], Block::Paragraph(text) if text.text == "[^1]: The note."));
}

#[test]
fn local_images_stand_alone_and_remote_ones_show_their_alt_text() {
    let page = page(
        "Intro\n![Shot](../assets/shot.png)\nafter\n\n![badge](https://img.shields.io/x.svg) [![ci](https://x/ci.svg)](https://x)\n\n![up](../../escape.png)\n",
    );
    let kinds: Vec<_> = page
        .blocks
        .iter()
        .map(|block| match block {
            Block::Paragraph(text) => format!("p:{}", text.text),
            Block::Image(image) => format!("img:{}:{}", image.path, image.alt),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(
        kinds,
        [
            "p:Intro ",
            "img:assets/shot.png:Shot",
            "p: after",
            "p:badge ci",
            "p:up",
        ]
    );
    assert_eq!(page.images(), ["assets/shot.png"]);
}

#[test]
fn images_are_recognised_by_their_extension() {
    assert_eq!(image_format("a/b.PNG"), Some(gpui::ImageFormat::Png));
    assert_eq!(image_format("logo.svg"), Some(gpui::ImageFormat::Svg));
    assert_eq!(image_format("notes.txt"), None);
    assert_eq!(image_format("noext"), None);
}

#[test]
fn loose_task_items_keep_their_boxes() {
    let page = page("- [x] done\n\n  more about it\n\n- [ ] next\n");
    let Block::List { items, .. } = &page.blocks[0] else {
        panic!("{:?}", page.blocks);
    };
    assert_eq!(
        items.iter().map(|item| item.task).collect::<Vec<_>>(),
        [Some(true), Some(false)]
    );
    let texts: Vec<_> = items
        .iter()
        .flat_map(|item| &item.blocks)
        .map(|block| match block {
            Block::Paragraph(text) => text.text.clone(),
            other => format!("{other:?}"),
        })
        .collect();
    assert_eq!(texts, ["done", "more about it", "next"], "no literal [x]");
    let markers: Vec<_> = page.rows.iter().map(|row| row.marker).collect();
    assert_eq!(
        markers,
        [Some(Marker::Task(true)), None, Some(Marker::Task(false))]
    );
}

#[test]
fn linked_and_table_images_stay_images() {
    let page = page(
        "[![Shot](shot.png)](guide.md) after\n\n| Logo | Name |\n| --- | --- |\n| ![Logo](logo.svg) | Runner |\n",
    );
    let Block::Image(image) = &page.blocks[0] else {
        panic!("{:?}", page.blocks);
    };
    assert_eq!(
        image,
        &ImageRef {
            path: "docs/shot.png".into(),
            alt: "Shot".into(),
            link: Some(Arc::from("guide.md")),
        }
    );
    assert!(matches!(&page.blocks[1], Block::Paragraph(text) if text.text == " after"));
    let Block::Table { rows, .. } = &page.blocks[2] else {
        panic!("{:?}", page.blocks);
    };
    assert!(matches!(&rows[0][0][..], [Piece::Image(image)] if image.path == "docs/logo.svg"));
    assert_eq!(page.images(), ["docs/shot.png", "docs/logo.svg"]);
}

#[test]
fn long_blocks_lay_out_as_a_row_each() {
    let mut text = String::from("```\n");
    for line in 0..1000 {
        text.push_str(&format!("line {line}\n"));
    }
    text.push_str("```\n\n");
    for item in 0..50 {
        text.push_str(&format!("- item {item}\n"));
    }
    text.push_str("\n| a |\n| - |\n| 1 |\n| 2 |\n\n> quoted\n>\n> - inside\n");
    let page = page(&text);
    let code = page
        .rows
        .iter()
        .filter(|row| matches!(row.kind, RowKind::Code { .. }))
        .count();
    assert_eq!(code, 1000);
    assert!(matches!(page.rows[0].kind, RowKind::CodeHeader { .. }));
    assert_eq!(page.rows[1000].text(), "line 999");
    let bullets = page
        .rows
        .iter()
        .filter(|row| row.marker == Some(Marker::Bullet))
        .count();
    assert_eq!(bullets, 51, "50 items and the quoted one");
    let table: Vec<_> = page
        .rows
        .iter()
        .filter_map(|row| match &row.kind {
            RowKind::TableRow { head, last, .. } => Some((*head, *last)),
            _ => None,
        })
        .collect();
    assert_eq!(table, [(true, false), (false, false), (false, true)]);
    let quoted: Vec<_> = page.rows.iter().filter(|row| row.quotes == 1).collect();
    assert_eq!(quoted.len(), 2);
    assert_eq!(quoted[1].indent, 1);
}

#[test]
fn a_selection_copies_rows_and_marks_only_its_bytes() {
    let runs = |lens: &[usize]| {
        lens.iter()
            .map(|len| TextRun {
                len: *len,
                font: gpui::font("x"),
                color: gpui::black(),
                background_color: None,
                underline: None,
                strikethrough: None,
            })
            .collect::<Vec<_>>()
    };
    let marked = with_selection(runs(&[4, 6]), 2..7, gpui::white());
    assert_eq!(
        marked
            .iter()
            .map(|run| (run.len, run.background_color.is_some()))
            .collect::<Vec<_>>(),
        [(2, false), (2, true), (3, true), (3, false)]
    );
    assert_eq!(with_selection(runs(&[4]), 0..0, gpui::white()).len(), 1);
}

#[test]
fn a_long_code_line_splits_into_rows_that_copy_back_whole() {
    let line = "é\t".repeat(600) + "end";
    let pieces = fragments(&line);
    assert!(pieces.len() > 1);
    assert!(pieces.iter().all(|piece| piece.len() <= FRAGMENT_BYTES + 4));
    assert_eq!(pieces.first().unwrap().start, 0);
    assert_eq!(pieces.last().unwrap().end, line.len());
    assert!(pieces.windows(2).all(|pair| pair[0].end == pair[1].start));
    let empty = fragments("");
    assert!(
        empty.len() == 1 && empty[0].is_empty(),
        "an empty line is one row"
    );
    // A combining mark stays with its base.
    let marks = format!("{}e\u{301}x", "a".repeat(FRAGMENT_BYTES - 1));
    assert_eq!(fragments(&marks)[0], 0..FRAGMENT_BYTES + 2);

    let page = page(&format!("```\n{line}\nnext\n```\n"));
    let rows = page.rows.len();
    assert_eq!(rows, pieces.len() + 2);
    assert!(matches!(page.rows[0].kind, RowKind::CodeHeader { .. }));
    assert_eq!(page.rows[0].text(), "");
    // Tabs keep their stops across a split: every piece expands as the
    // whole line would.
    let drawn: String = (1..=pieces.len())
        .map(|row| page.rows[row].text())
        .collect();
    assert_eq!(drawn, "é   ".repeat(600) + "end");
    let view_text = page
        .selected_text(
            Position { row: 0, offset: 0 },
            Position {
                row: rows - 1,
                offset: page.rows[rows - 1].text().len(),
            },
        )
        .unwrap();
    assert_eq!(view_text, format!("{drawn}\nnext"));
}

#[test]
fn link_text_on_both_sides_of_a_linked_image_stays_linked() {
    let page = page(
        "[before ![image](image.png) after](guide.md)\n\n| x |\n| - |\n| [a ![i](i.png) b](t.md) |\n",
    );
    let [Block::Paragraph(before), Block::Image(image), Block::Paragraph(after), Block::Table { rows, .. }] =
        &page.blocks[..]
    else {
        panic!("{:?}", page.blocks);
    };
    assert_eq!(before.text, "before ");
    assert_eq!(before.links, [(0..7, Arc::from("guide.md"))]);
    assert_eq!(image.link.as_deref(), Some("guide.md"));
    assert_eq!(after.text, " after");
    assert_eq!(after.links, [(0..6, Arc::from("guide.md"))]);
    let [Piece::Text(a), Piece::Image(i), Piece::Text(b)] = &rows[0][0][..] else {
        panic!("{:?}", rows[0][0]);
    };
    assert_eq!(a.links, [(0..2, Arc::from("t.md"))]);
    assert_eq!(i.link.as_deref(), Some("t.md"));
    assert_eq!(b.links, [(0..2, Arc::from("t.md"))]);
}

// ── Compact look tests (#1342) ─────────────────────────────────────────

#[test]
fn compact_soft_breaks_split_into_one_row_per_line_with_styles_and_links() {
    let source = "First **bold\nsecond** line\n[link\npart](https://example.com)";
    let compact = Page::parse_compact(source);
    assert_eq!(compact.rows.len(), 4);
    assert_eq!(
        compact.rows.iter().map(Row::text).collect::<Vec<_>>(),
        ["First bold", "second line", "link", "part"]
    );
    assert_eq!(compact.rows[0].top, 6.);
    assert_eq!(compact.rows[1].top, 0.);
    assert_eq!(compact.rows[2].top, 0.);
    assert_eq!(compact.rows[3].top, 0.);

    let RowKind::Paragraph(line0) = &compact.rows[0].kind else {
        panic!("{:?}", compact.rows[0]);
    };
    let RowKind::Paragraph(line1) = &compact.rows[1].kind else {
        panic!("{:?}", compact.rows[1]);
    };
    let RowKind::Paragraph(line2) = &compact.rows[2].kind else {
        panic!("{:?}", compact.rows[2]);
    };
    let RowKind::Paragraph(line3) = &compact.rows[3].kind else {
        panic!("{:?}", compact.rows[3]);
    };
    assert!(line0
        .styles
        .iter()
        .any(|(r, s)| &line0.text[r.clone()] == "bold" && s.strong));
    assert!(line1
        .styles
        .iter()
        .any(|(r, s)| &line1.text[r.clone()] == "second" && s.strong));
    assert_eq!(line2.links, [(0..4, Arc::from("https://example.com"))]);
    assert_eq!(line3.links, [(0..4, Arc::from("https://example.com"))]);
    assert!(
        Arc::ptr_eq(&line2.links[0].1, &line3.links[0].1),
        "multi-line link destination should be shared across row slices"
    );

    // Document keeps soft breaks as spaces in a single row.
    let document = page("First line\nsecond line");
    assert_eq!(document.rows.len(), 1);
    assert_eq!(document.rows[0].text(), "First line second line");
}

#[test]
fn compact_multiline_link_shares_one_destination_allocation_across_all_rows() {
    let url = format!("https://example.com/{}", "segment/".repeat(64));
    let label = (0..12)
        .map(|i| format!("line {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let source = format!("[{label}]({url})");
    let compact = Page::parse_compact(&source);
    assert_eq!(compact.rows.len(), 12);
    let RowKind::Paragraph(first) = &compact.rows[0].kind else {
        panic!("{:?}", compact.rows[0]);
    };
    let shared = &first.links[0].1;
    assert_eq!(&**shared, url);
    for (i, row) in compact.rows.iter().enumerate() {
        let RowKind::Paragraph(inline) = &row.kind else {
            panic!("row {i}: {row:?}");
        };
        assert_eq!(inline.links.len(), 1);
        assert!(
            Arc::ptr_eq(shared, &inline.links[0].1),
            "row {i} cloned the link destination instead of sharing it"
        );
        let (ranges, urls) = openable_links(&inline.links);
        assert_eq!(ranges.len(), 1);
        assert_eq!(urls.len(), 1);
        assert!(
            Arc::ptr_eq(shared, &urls[0]),
            "row {i} clickable link preparation cloned the destination instead of sharing Arc<str>"
        );
    }
}

#[test]
fn loose_task_checklist_preserves_every_box_and_paragraph_in_linear_time() {
    let source = (0..2000)
        .map(|i| {
            let box_char = if i % 2 == 0 { 'x' } else { ' ' };
            format!("- [{box_char}] item {i}\n\n  detail {i}\n\n")
        })
        .collect::<String>();
    let compact = Page::parse_compact(&source);
    assert_eq!(compact.rows.len(), 4000);
    assert_eq!(compact.rows[0].marker, Some(Marker::Task(true)));
    assert_eq!(compact.rows[0].text(), "item 0");
    assert_eq!(compact.rows[1].marker, None);
    assert_eq!(compact.rows[1].text(), "detail 0");
    assert_eq!(compact.rows[3998].marker, Some(Marker::Task(false)));
    assert_eq!(compact.rows[3998].text(), "item 1999");
    assert_eq!(compact.rows[3999].text(), "detail 1999");
}

#[test]
fn compact_nested_lists_task_boxes_and_h4_to_h6_parse_and_lay_out() {
    let source = concat!(
        "#### Heading 4\n\n",
        "##### Heading 5\n\n",
        "###### Heading 6\n\n",
        "- top\n",
        "  - child\n",
        "    - grandchild\n",
        "- [x] done\n",
        "- [ ] todo\n",
    );
    let compact = Page::parse_compact(source);
    assert!(matches!(&compact.rows[0].kind, RowKind::Heading(4, i) if i.text == "Heading 4"));
    assert!(matches!(&compact.rows[1].kind, RowKind::Heading(5, i) if i.text == "Heading 5"));
    assert!(matches!(&compact.rows[2].kind, RowKind::Heading(6, i) if i.text == "Heading 6"));
    assert_eq!(compact.rows[0].top, 8.);
    assert_eq!(compact.rows[1].top, 8.);
    assert_eq!(compact.rows[2].top, 8.);

    let list_rows: Vec<_> = compact.rows[3..]
        .iter()
        .map(|row| (row.indent, row.marker, row.top, row.text()))
        .collect();
    assert_eq!(
        list_rows,
        [
            (1, Some(Marker::Bullet), 6., "top".into()),
            (2, Some(Marker::Bullet), 4., "child".into()),
            (3, Some(Marker::Bullet), 4., "grandchild".into()),
            (1, Some(Marker::Task(true)), 4., "done".into()),
            (1, Some(Marker::Task(false)), 4., "todo".into()),
        ]
    );
    assert_eq!(compact.bottom, 6.);
}

#[test]
fn compact_commonmark_rules_setext_heading_indented_code_literal_stars_and_multiline_quote() {
    let source = concat!(
        "Summary\n---\n\n",
        "    let indented = 1;\n\n",
        "2 * 3 * 4 and \\*escaped\\*\n\n",
        "> line one\n> line two\n\n",
        "```rust\nfn main() {}\n```\n",
    );
    let compact = Page::parse_compact(source);
    assert!(matches!(&compact.rows[0].kind, RowKind::Heading(2, i) if i.text == "Summary"));
    assert_eq!(compact.rows[0].top, 12.);

    assert!(matches!(
        &compact.rows[1].kind,
        RowKind::CodeHeader { code } if code.language.is_none()
    ));
    assert_eq!(compact.rows[1].top, 8.);
    assert!(matches!(
        &compact.rows[2].kind,
        RowKind::Code { code, line: 0, .. }
            if code.document.line(0) == "let indented = 1;" && code.highlights().is_none()
    ));
    assert_eq!(compact.rows[2].top, 0.);

    let RowKind::Paragraph(math) = &compact.rows[3].kind else {
        panic!("{:?}", compact.rows[3]);
    };
    assert_eq!(math.text, "2 * 3 * 4 and *escaped*");
    assert!(math.styles.iter().all(|(_, style)| !style.emphasis));
    assert_eq!(compact.rows[3].top, 8.);

    assert_eq!(compact.rows[4].quotes, 1);
    assert_eq!(compact.rows[4].text(), "line one");
    assert_eq!(compact.rows[4].top, 8.);
    assert_eq!(compact.rows[4].inner_top, 0.);
    assert_eq!(compact.rows[5].quotes, 1);
    assert_eq!(compact.rows[5].text(), "line two");
    assert_eq!(compact.rows[5].top, 0.);
    assert_eq!(compact.rows[5].inner_top, 0.);

    assert!(matches!(
        &compact.rows[6].kind,
        RowKind::CodeHeader { code } if code.language.as_deref() == Some("rust")
    ));
    assert_eq!(compact.rows[6].top, 8.);
    assert!(matches!(
        &compact.rows[7].kind,
        RowKind::Code { code, line: 0, .. }
            if code.document.line(0) == "fn main() {}"
                && code.highlights().is_none()
                && code.pending_grammar().is_some()
    ));
    assert_eq!(compact.rows[7].top, 0.);
    assert_eq!(compact.bottom, 8.);
}

#[test]
fn compact_images_render_as_alt_text_without_disk_access() {
    let source = concat!(
        "![Shot](assets/shot.png)\n\n",
        "[![Linked](logo.svg)](https://example.com) and ![Remote](https://img.example/x.png)\n\n",
        "| Icon | Label |\n| --- | --- |\n| ![CellImg](icon.png) | Runner |\n",
    );
    let compact = Page::parse_compact(source);
    assert!(compact.images().is_empty());
    assert!(compact
        .rows
        .iter()
        .all(|row| !matches!(row.kind, RowKind::Image(_))));
    assert_eq!(compact.rows[0].text(), "Shot");
    assert_eq!(compact.rows[1].text(), "Linked and Remote");
    let RowKind::Paragraph(linked) = &compact.rows[1].kind else {
        panic!("{:?}", compact.rows[1]);
    };
    assert_eq!(linked.links, [(0..6, Arc::from("https://example.com"))]);
    assert_eq!(compact.rows[3].text(), "CellImg\tRunner");
}

#[test]
fn compact_links_route_only_http_https_and_mailto() {
    assert_eq!(
        route_web("https://runner.dev/a?b#c"),
        LinkTarget::Url("https://runner.dev/a?b#c".into())
    );
    assert_eq!(
        route_web("  http://example.com  "),
        LinkTarget::Url("http://example.com".into())
    );
    assert_eq!(
        route_web("HTTPS://EXAMPLE.COM"),
        LinkTarget::Url("HTTPS://EXAMPLE.COM".into())
    );
    assert_eq!(
        route_web("mailto:a@b.c"),
        LinkTarget::Url("mailto:a@b.c".into())
    );
    assert_eq!(route_web("file:///etc/hosts"), LinkTarget::None);
    assert_eq!(route_web("javascript:alert(1)"), LinkTarget::None);
    assert_eq!(route_web("data:text/plain,hi"), LinkTarget::None);
    assert_eq!(route_web("C:/Windows/win.ini"), LinkTarget::None);
    assert_eq!(route_web("#heading"), LinkTarget::None);
    assert_eq!(route_web("guide.md"), LinkTarget::None);
    assert_eq!(route_web("/Cargo.toml"), LinkTarget::None);
    assert_eq!(route_web("../secret"), LinkTarget::None);
    assert_eq!(route_web(""), LinkTarget::None);
}

#[test]
fn compact_selection_copies_paragraphs_table_rows_and_code_lines() {
    let long_code = "a".repeat(FRAGMENT_BYTES + 20);
    let source = format!(
        "Intro line\nSecond line\n\n| Key | Val |\n| --- | --- |\n| one | two |\n\n```\n{long_code}\nlast\n```\n"
    );
    let feed = FeedMarkdown::new(&source);
    let rows: Vec<String> = feed.page().rows.iter().map(Row::text).collect();
    // Row 0: Intro line, Row 1: Second line, Row 2: Key\tVal, Row 3: one\ttwo,
    // Row 4: CodeHeader (""), Row 5..=6: long_code split into 2 fragments, Row 7: last.
    assert_eq!(rows.len(), 8);
    assert_eq!(rows[2], "Key\tVal");
    assert_eq!(rows[3], "one\ttwo");
    assert_eq!(rows[4], "");
    assert!(feed.page().rows[6].continues());

    // Reverse selection from row 7 offset 4 back to row 1 offset 7 skips the header row.
    let sel = FeedSelection {
        event_id: "e1".into(),
        anchor: Position { row: 7, offset: 4 },
        head: Position { row: 1, offset: 7 },
    };
    assert_eq!(
        feed.selected_text(&sel).as_deref(),
        Some(format!("line\nKey\tVal\none\ttwo\n{long_code}\nlast").as_str())
    );

    // Selecting any non-empty span inside a table row selects the whole row, tab-separated.
    let table_sel = FeedSelection {
        event_id: "e1".into(),
        anchor: Position { row: 3, offset: 0 },
        head: Position { row: 3, offset: 1 },
    };
    assert_eq!(feed.selected_text(&table_sel).as_deref(), Some("one\ttwo"));

    // Equal anchor and head yields None.
    let empty_sel = FeedSelection {
        event_id: "e1".into(),
        anchor: Position { row: 0, offset: 2 },
        head: Position { row: 0, offset: 2 },
    };
    assert_eq!(feed.selected_text(&empty_sel), None);

    // Distinct endpoints spanning zero bytes across a 512-byte code fragment
    // boundary keep Files' Some("") semantics in Page::selected_text while
    // FeedMarkdown filters empty copies out.
    let frag0_len = feed.page().rows[5].text().len();
    let boundary_start = Position {
        row: 5,
        offset: frag0_len,
    };
    let boundary_end = Position { row: 6, offset: 0 };
    assert_eq!(
        feed.page().selected_text(boundary_start, boundary_end),
        Some(String::new())
    );
    assert_eq!(
        feed.selected_text(&FeedSelection {
            event_id: "e1".into(),
            anchor: boundary_start,
            head: boundary_end,
        }),
        None
    );
}

#[test]
fn compact_selection_across_lists_omits_list_markers_matching_files() {
    let source = "# **Title**\n\n- one `code`\n- [x] done\n2. two\n\nParagraph";
    let feed = FeedMarkdown::new(source);
    assert_eq!(
        feed.page().rows.iter().map(Row::text).collect::<Vec<_>>(),
        ["Title", "one code", "done", "two", "Paragraph"]
    );
    let sel = FeedSelection {
        event_id: "e1".into(),
        anchor: Position { row: 4, offset: 4 },
        head: Position { row: 1, offset: 4 },
    };
    assert_eq!(
        feed.selected_text(&sel).as_deref(),
        Some("code\ndone\ntwo\nPara")
    );
}

#[test]
fn link_hover_returns_when_reentering_a_cached_line() {
    let mut hover = MarkdownLinkHover::default();
    assert!(hover.set_link("line-a", Some(1)));
    assert!(hover.set_line_active("line-a", true));
    assert_eq!(hover.link_for("line-a"), Some(1));

    assert!(hover.set_line_active("line-a", false));
    assert_eq!(hover.link_for("line-a"), None);
    assert!(hover.set_link("line-b", Some(0)));
    assert!(hover.set_line_active("line-b", true));
    assert_eq!(hover.link_for("line-b"), Some(0));

    assert!(hover.set_line_active("line-b", false));
    assert!(hover.set_line_active("line-a", true));
    assert_eq!(hover.link_for("line-a"), Some(1));
}

#[test]
fn compact_inline_markers_recurse_and_code_overrides_outer_weight() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let page = Page::parse_compact("**`runner-app` is the binary**");
    let RowKind::Paragraph(inline) = &page.rows[0].kind else {
        panic!("{:?}", page.rows[0]);
    };
    assert_eq!(inline.text, "runner-app is the binary");
    let highlights = compact_highlights(inline, None, 0..0, theme::raised());
    let code = highlights
        .iter()
        .find(|(range, _)| range.contains(&0))
        .map(|(_, style)| style)
        .expect("inline code style");
    assert_eq!(code.font_weight, Some(FontWeight::NORMAL));
    assert_eq!(code.color, Some(theme::accent()));
    assert_eq!(code.background_color, Some(theme::raised()));
    let bold = highlights
        .iter()
        .find(|(range, _)| range.contains(&12))
        .map(|(_, style)| style)
        .expect("outer bold style");
    assert_eq!(bold.font_weight, Some(FontWeight::SEMIBOLD));
}

#[test]
fn compact_inline_links_nest_in_strong_but_stay_literal_inside_code() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let page = Page::parse_compact("**[Runner](https://example.com)** and `[literal](url)`");
    let RowKind::Paragraph(inline) = &page.rows[0].kind else {
        panic!("{:?}", page.rows[0]);
    };
    assert_eq!(inline.text, "Runner and [literal](url)");
    assert_eq!(inline.links, vec![(0..6, Arc::from("https://example.com"))]);
    let unhovered = compact_highlights(inline, None, 0..0, theme::raised());
    let link = unhovered
        .iter()
        .find(|(range, _)| range.contains(&0))
        .map(|(_, style)| style)
        .expect("link style");
    assert_eq!(link.font_weight, Some(FontWeight::SEMIBOLD));
    assert_eq!(
        link.underline.unwrap().color,
        Some(theme::with_alpha(theme::accent(), 0.4))
    );
    let hovered = compact_highlights(inline, Some(0..6), 0..0, theme::raised());
    let hovered_link = hovered
        .iter()
        .find(|(range, _)| range.contains(&0))
        .map(|(_, style)| style)
        .expect("hovered link style");
    assert_eq!(hovered_link.underline.unwrap().color, Some(theme::accent()));
}

#[test]
fn compact_block_margins_match_the_spec_table() {
    let page = Page::parse_compact(concat!(
        "# H1\n\n",
        "## H2\n\n",
        "### H3\n\n",
        "Para A\n\n",
        "Para B\n\n",
        "- item 1\n- item 2\n\n",
        "---\n\n",
        "> quoted\n\n",
        "```\ncode\n```\n\n",
        "| a |\n| - |\n| 1 |\n",
    ));
    let tops: Vec<f32> = page.rows.iter().map(|row| row.top).collect();
    assert_eq!(
        tops,
        [
            12., // H1: (12, 6)
            12., // H2: max(6, 12) = 12, bottom 4
            8.,  // H3: max(4, 8) = 8, bottom 4
            6.,  // Para A: max(4, 6) = 6, bottom 6
            12., // Para B: Para+Para = 6+6 = 12, bottom 6
            6.,  // item 1: max(6, 6) = 6
            4.,  // item 2: list+list = 4, bottom 6
            12., // Rule: max(6, 12) = 12, bottom 12
            12., // Quote: max(12, 8) = 12, bottom 8
            8.,  // Code header: max(8, 8) = 8
            0.,  // Code line: 0, bottom 8
            8.,  // Table head: max(8, 8) = 8
            0.,  // Table body: 0, bottom 8
        ]
    );
    assert_eq!(page.bottom, 8.);
}

struct MarkdownWrapTest;

struct MarkdownSpacingTest;

struct MarkdownDragTest {
    feed: FeedMarkdown,
    handler: FeedSelectionHandler,
}

struct MarkdownLinkDragTest {
    feed: FeedMarkdown,
    handler: FeedSelectionHandler,
}

impl Render for MarkdownWrapTest {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(280.))
            .debug_selector(|| "COMPACT_WRAP_CONTAINER".into())
            .child(render_compact(
                "wrap-test",
                concat!(
                    "A narrow feed line with **styled text**, `inline code`, and [a link whose label also needs to wrap](https://example.com) before the final words.\n\n",
                    "| Name | Meaning |\n| --- | :---: |\n",
                    "| a long description that cannot fit on one line of a narrow column | `an_unbroken_inline_code_value_far_longer_than_its_column` |\n\n",
                    "```\n",
                    "let very_long_code_identifier_that_must_wrap_inside_a_narrow_compact_feed = 1234567890;\n",
                    "```\n",
                ),
                cx.entity_id(),
                cx,
            ))
    }
}

impl Render for MarkdownSpacingTest {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().child(
            div()
                .w(px(280.))
                .debug_selector(|| "COMPACT_SPACING_CONTAINER".into())
                .child(render_compact(
                    "spacing-test",
                    "First paragraph\n\nSecond paragraph\n\n# Heading",
                    cx.entity_id(),
                    cx,
                )),
        )
    }
}

impl Render for MarkdownDragTest {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().w(px(280.)).child(self.feed.render_all(
            "drag-test",
            cx.entity_id(),
            None,
            theme::raised(),
            Some(&self.handler),
            cx,
        ))
    }
}

impl Render for MarkdownLinkDragTest {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().w(px(320.)).child(self.feed.render_all(
            "link-drag-test",
            cx.entity_id(),
            None,
            theme::raised(),
            Some(&self.handler),
            cx,
        ))
    }
}

#[test]
fn inline_markdown_table_and_code_wrap_inside_a_narrow_feed() {
    let mut cx = TestAppContext::single();
    let window = cx.add_window(|_, _| MarkdownWrapTest);
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(window.into(), &cx);
    let container = window
        .debug_bounds("COMPACT_WRAP_CONTAINER")
        .expect("container bounds");
    let paragraph = window
        .debug_bounds("COMPACT_MD_TEXT wrap-test 0")
        .expect("paragraph bounds");
    assert_eq!(container.size.width, px(280.));
    assert!(paragraph.size.height > px(20.));

    for (cell, piece) in [
        ("COMPACT_MD_CELL 2-0", "COMPACT_MD_CELL_PIECE 2-0-0"),
        ("COMPACT_MD_CELL 2-1", "COMPACT_MD_CELL_PIECE 2-1-0"),
    ] {
        let cell_bounds = window.debug_bounds(cell).expect(cell);
        let piece_bounds = window.debug_bounds(piece).expect(piece);
        assert!(
            piece_bounds.left() >= cell_bounds.left()
                && piece_bounds.right() <= cell_bounds.right(),
            "{piece} is {piece_bounds:?}, outside {cell_bounds:?}"
        );
    }
    let header_cell = window.debug_bounds("COMPACT_MD_CELL 1-0").unwrap();
    let wrapped_cell = window.debug_bounds("COMPACT_MD_CELL 2-0").unwrap();
    assert!(wrapped_cell.size.height > header_cell.size.height);

    let code_line = window
        .debug_bounds("COMPACT_MD_TEXT wrap-test 4")
        .expect("code line bounds");
    assert!(code_line.right() <= container.right());
    assert!(
        code_line.size.height > px(20.),
        "long code line should wrap: {code_line:?}"
    );
}

#[test]
fn markdown_layout_collapses_adjacent_block_margins() {
    let mut cx = TestAppContext::single();
    let window = cx.add_window(|_, _| MarkdownSpacingTest);
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(window.into(), &cx);
    let first = window.debug_bounds("COMPACT_MD_ROW 0").unwrap();
    let second = window.debug_bounds("COMPACT_MD_ROW 1").unwrap();
    let heading = window.debug_bounds("COMPACT_MD_ROW 2").unwrap();
    let body = window.debug_bounds("COMPACT_SPACING_CONTAINER").unwrap();

    assert_eq!(
        second.origin.y - first.origin.y - first.size.height,
        px(12.)
    );
    assert_eq!(
        heading.origin.y - second.origin.y - second.size.height,
        px(12.)
    );
    assert_eq!(
        body.origin.y + body.size.height - heading.origin.y - heading.size.height,
        px(6.)
    );
}

#[test]
fn markdown_drag_maps_pointer_positions_to_feed_offsets() {
    let events = Rc::new(RefCell::new(Vec::new()));
    let recorded = events.clone();
    let handler = FeedSelectionHandler::new(move |phase, event_id, position, _, _| {
        recorded
            .borrow_mut()
            .push((phase, event_id.to_owned(), position));
    });
    let mut cx = TestAppContext::single();
    let window = cx.add_window(move |_, _| MarkdownDragTest {
        feed: FeedMarkdown::new("Select this sentence"),
        handler,
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(window.into(), &cx);
    let line = window.debug_bounds("COMPACT_MD_TEXT drag-test 0").unwrap();
    let start = point(
        line.origin.x + px(1.),
        line.origin.y + line.size.height / 2.,
    );
    let end = point(
        line.origin.x + line.size.width.min(px(80.)),
        line.origin.y + line.size.height / 2.,
    );
    window.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());

    let events = events.borrow();
    assert_eq!(
        events[0],
        (
            FeedSelectionPhase::Begin,
            "drag-test".into(),
            Position { row: 0, offset: 0 }
        )
    );
    assert!(events.iter().any(|(phase, event_id, position)| {
        *phase == FeedSelectionPhase::Extend
            && event_id == "drag-test"
            && position.row == 0
            && position.offset > 0
    }));
    assert_eq!(events.last().unwrap().0, FeedSelectionPhase::End);
}

#[test]
fn compact_drag_over_link_opens_nothing_and_click_opens_only_web_links() {
    let handler = FeedSelectionHandler::new(|_, _, _, _, _| {});
    let mut cx = TestAppContext::single();
    let window = cx.add_window(move |_, _| MarkdownLinkDragTest {
        feed: FeedMarkdown::new(concat!(
            "[open the web link](https://example.com)\n",
            "[ignore local file](file:///etc/hosts)\n",
            "[second web link](https://second.example)",
        )),
        handler,
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(window.into(), &cx);
    let web = window
        .debug_bounds("COMPACT_MD_TEXT link-drag-test 0")
        .unwrap();
    let file = window
        .debug_bounds("COMPACT_MD_TEXT link-drag-test 1")
        .unwrap();
    let second = window
        .debug_bounds("COMPACT_MD_TEXT link-drag-test 2")
        .unwrap();

    // Dragging across the web link must not open it.
    let from = point(web.left() + px(2.), web.center().y);
    let to = point(web.left() + px(70.), web.center().y);
    window.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_move(to, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    window.run_until_parked();
    assert_eq!(window.cx.opened_url(), None);

    // Secondary-button press/drag on the same link must not open it.
    window.simulate_mouse_down(from, MouseButton::Right, Modifiers::default());
    window.simulate_mouse_move(to, MouseButton::Right, Modifiers::default());
    window.simulate_mouse_up(to, MouseButton::Right, Modifiers::default());
    window.run_until_parked();
    assert_eq!(window.cx.opened_url(), None);

    // A displaced primary release without an intermediate mouse-move must not open it.
    window.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    window.run_until_parked();
    assert_eq!(window.cx.opened_url(), None);

    // Clicking the file: link does nothing.
    window.simulate_click(
        point(file.left() + px(20.), file.center().y),
        Modifiers::default(),
    );
    window.run_until_parked();
    assert_eq!(window.cx.opened_url(), None);

    // Clicking the https: link opens it.
    window.simulate_click(
        point(web.left() + px(20.), web.center().y),
        Modifiers::default(),
    );
    window.run_until_parked();
    assert_eq!(
        window.cx.opened_url().as_deref(),
        Some("https://example.com")
    );

    // After a successful primary click, a fresh secondary-button drag or
    // displaced primary release on another link still opens nothing.
    let second_from = point(second.left() + px(2.), second.center().y);
    let second_to = point(second.left() + px(70.), second.center().y);
    window.simulate_mouse_down(second_from, MouseButton::Right, Modifiers::default());
    window.simulate_mouse_move(second_to, MouseButton::Right, Modifiers::default());
    window.simulate_mouse_up(second_to, MouseButton::Right, Modifiers::default());
    window.run_until_parked();
    assert_eq!(
        window.cx.opened_url().as_deref(),
        Some("https://example.com")
    );

    window.simulate_mouse_down(second_from, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_up(second_to, MouseButton::Left, Modifiers::default());
    window.run_until_parked();
    assert_eq!(
        window.cx.opened_url().as_deref(),
        Some("https://example.com")
    );
}

#[test]
fn document_secondary_drag_and_displaced_release_do_not_open_links() {
    let opened = Rc::new(RefCell::new(Vec::<LinkTarget>::new()));
    let recorded = opened.clone();
    let mut cx = TestAppContext::single();
    let window = cx.add_window(move |_, cx| {
        let parsed = Arc::new(Page::parse(
            "[first web link](https://example.com)\n\n[second web link](https://second.example)\n",
            "docs/README.md",
        ));
        let mut view = MarkdownView::new(parsed, "docs/README.md".into(), Default::default(), cx);
        view.set_open_link(Rc::new(move |target, _, _| {
            recorded.borrow_mut().push(target);
        }));
        view
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(window.into(), &cx);
    let first = window.debug_bounds("FILE_MD_TEXT 0").unwrap();
    let second = window.debug_bounds("FILE_MD_TEXT 1").unwrap();

    // Primary click opens the first link.
    window.simulate_click(
        point(first.left() + px(20.), first.center().y),
        Modifiers::default(),
    );
    window.run_until_parked();
    assert_eq!(
        *opened.borrow(),
        vec![LinkTarget::Url("https://example.com".into())]
    );

    // Secondary-button drag and displaced primary release on the second link open nothing.
    let from = point(second.left() + px(2.), second.center().y);
    let to = point(second.left() + px(70.), second.center().y);
    window.simulate_mouse_down(from, MouseButton::Right, Modifiers::default());
    window.simulate_mouse_move(to, MouseButton::Right, Modifiers::default());
    window.simulate_mouse_up(to, MouseButton::Right, Modifiers::default());
    window.run_until_parked();
    assert_eq!(opened.borrow().len(), 1);

    window.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    window.run_until_parked();
    assert_eq!(opened.borrow().len(), 1);
}

struct ReadonlyExcursionTest;

impl Render for ReadonlyExcursionTest {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(320.))
            .flex()
            .flex_col()
            .child(render_compact(
                "readonly-link",
                "[open the link](https://example.com/readonly)",
                cx.entity_id(),
                cx,
            ))
            .child(
                div()
                    .h(px(40.))
                    .debug_selector(|| "ADJACENT_CONTROL".into())
                    .child("Adjacent control"),
            )
    }
}

#[test]
fn readonly_compact_link_out_and_back_excursion_does_not_open_link() {
    let mut cx = TestAppContext::single();
    let window = cx.add_window(|_, _| ReadonlyExcursionTest);
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(window.into(), &cx);
    let link = window
        .debug_bounds("COMPACT_MD_TEXT readonly-link 0")
        .unwrap();
    let adjacent = window.debug_bounds("ADJACENT_CONTROL").unwrap();

    // Press on the link, move outside the preview into an adjacent control
    // (producing zero in-bounds displaced moves), then return to the exact
    // press origin and release: must not open the URL.
    let origin = point(link.left() + px(20.), link.center().y);
    let outside = adjacent.center();
    window.simulate_mouse_down(origin, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_move(outside, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_move(origin, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_up(origin, MouseButton::Left, Modifiers::default());
    window.run_until_parked();
    assert_eq!(window.cx.opened_url(), None);

    // A clean primary click at the same point still opens the link.
    window.simulate_click(origin, Modifiers::default());
    window.run_until_parked();
    assert_eq!(
        window.cx.opened_url().as_deref(),
        Some("https://example.com/readonly")
    );
}

struct DocumentExcursionTest {
    view: gpui::Entity<MarkdownView>,
}

impl Render for DocumentExcursionTest {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(320.))
            .flex()
            .flex_col()
            .child(div().h(px(80.)).child(self.view.clone()))
            .child(
                div()
                    .h(px(40.))
                    .debug_selector(|| "DOC_ADJACENT_CONTROL".into())
                    .child("Adjacent control"),
            )
    }
}

#[test]
fn document_link_out_and_back_excursion_does_not_open_link() {
    let opened = Rc::new(RefCell::new(Vec::<LinkTarget>::new()));
    let recorded = opened.clone();
    let mut cx = TestAppContext::single();
    let window = cx.add_window(move |_, cx| {
        let parsed = Arc::new(Page::parse(
            "[open the doc link](https://example.com/doc)\n",
            "docs/README.md",
        ));
        let view = cx.new(|cx| {
            let mut view =
                MarkdownView::new(parsed, "docs/README.md".into(), Default::default(), cx);
            view.set_open_link(Rc::new(move |target, _, _| {
                recorded.borrow_mut().push(target);
            }));
            view
        });
        DocumentExcursionTest { view }
    });
    cx.run_until_parked();
    let mut window = VisualTestContext::from_window(window.into(), &cx);
    let link = window.debug_bounds("FILE_MD_TEXT 0").unwrap();
    let adjacent = window.debug_bounds("DOC_ADJACENT_CONTROL").unwrap();

    let origin = point(link.left() + px(20.), link.center().y);
    let outside = adjacent.center();
    window.simulate_mouse_down(origin, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_move(outside, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_move(origin, MouseButton::Left, Modifiers::default());
    window.simulate_mouse_up(origin, MouseButton::Left, Modifiers::default());
    window.run_until_parked();
    assert!(opened.borrow().is_empty());

    window.simulate_click(origin, Modifiers::default());
    window.run_until_parked();
    assert_eq!(
        *opened.borrow(),
        vec![LinkTarget::Url("https://example.com/doc".into())]
    );
}

// ── Code block header, Copy, Wrap & Feed highlighting (#1444) ──────────

#[test]
fn code_block_language_extracts_first_info_word_and_omits_unlabelled_and_indented() {
    let source = concat!(
        "```rust title=\"x\"\nfn a() {}\n```\n\n",
        "```  sh,copy\necho hi\n```\n\n",
        "```ts{linenos}\nconst x = 1;\n```\n\n",
        "```\nunlabelled\n```\n\n",
        "```   \nspaces\n```\n\n",
        "    indented\n",
    );
    let expected = [Some("rust"), Some("sh"), Some("ts"), None, None, None];
    for parsed in [page(source), Page::parse_compact(source)] {
        let langs: Vec<Option<&str>> = parsed
            .blocks
            .iter()
            .map(|block| match block {
                Block::Code(code) => code.language.as_deref(),
                other => panic!("expected code block, got {other:?}"),
            })
            .collect();
        assert_eq!(langs, expected);

        let headers: Vec<Option<&str>> = parsed
            .rows
            .iter()
            .filter_map(|row| match &row.kind {
                RowKind::CodeHeader { code } => Some(code.language.as_deref()),
                _ => None,
            })
            .collect();
        assert_eq!(headers, expected);
    }

    // A language-only edit must mark the rendered page changed so live reload updates the bar.
    let before = page("```\nplain\n```\n");
    let after = page("```txt\nplain\n```\n");
    assert!(before.matches_text(&after));
    assert!(!before.same_rendered(&after));
}

#[test]
fn code_header_row_precedes_code_and_stays_empty_to_selection_and_copy() {
    let source = "Before\n\n```rust\nlet a = 1;\nlet b = 2;\n```\n\nAfter\n";
    for parsed in [page(source), Page::parse_compact(source)] {
        assert_eq!(parsed.rows.len(), 5);
        assert_eq!(parsed.rows[0].text(), "Before");
        assert!(matches!(
            &parsed.rows[1].kind,
            RowKind::CodeHeader { code } if code.language.as_deref() == Some("rust")
        ));
        assert_eq!(parsed.rows[1].text(), "");
        assert_eq!(parsed.rows[2].text(), "let a = 1;");
        assert_eq!(parsed.rows[3].text(), "let b = 2;");
        assert_eq!(parsed.rows[4].text(), "After");

        // Selection spanning the whole document omits the header bar cleanly.
        assert_eq!(
            parsed.selected_text(
                Position { row: 0, offset: 0 },
                Position { row: 4, offset: 5 }
            ),
            Some("Before\nlet a = 1;\nlet b = 2;\nAfter".into())
        );
        // Select-all starting directly on a code header adds no leading newline.
        assert_eq!(
            parsed.selected_text(
                Position { row: 1, offset: 0 },
                Position { row: 3, offset: 10 }
            ),
            Some("let a = 1;\nlet b = 2;".into())
        );
    }
}

#[test]
fn compact_code_highlighting_styles_known_languages_and_keeps_unknown_plain() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    theme::set_active_variant(theme::ThemeVariant::Carbon);
    let syntax = SyntaxTheme::for_palette(&runner_terminal::palette::RUNNER);
    let keyword_idx = CAPTURES.iter().position(|name| *name == "keyword").unwrap() as u16;
    let keyword_color = syntax.color(keyword_idx).expect("keyword colour");

    HIGHLIGHT_COUNT.set(0);
    let feed = FeedMarkdown::new("```rust\nfn main() {}\n```\n\n```notalang\nfn plain() {}\n```\n");
    assert_eq!(
        HIGHLIGHT_COUNT.get(),
        0,
        "compact parsing defers tree-sitter highlighting until drawn"
    );
    let Block::Code(rust) = &feed.page().blocks[0] else {
        panic!("{:?}", feed.page().blocks);
    };
    let Block::Code(unknown) = &feed.page().blocks[1] else {
        panic!("{:?}", feed.page().blocks);
    };
    let rust = Arc::clone(rust);
    let unknown = Arc::clone(unknown);
    assert!(rust.highlights().is_none());
    assert!(unknown.highlights().is_none());

    let mut cx = TestAppContext::single();
    let window = cx.add_window(move |_, _| CompactAsyncHighlightTest { feed });
    cx.run_until_parked();
    let _visual = VisualTestContext::from_window(window.into(), &cx);
    assert_eq!(
        HIGHLIGHT_COUNT.get(),
        1,
        "only the recognised language runs tree-sitter when drawn"
    );

    let spans = rust
        .highlights()
        .expect("rust highlighted in compact after draw")
        .line(0);
    assert_eq!(CAPTURES[spans[0].capture as usize], "keyword");
    let (_, fragment_spans) = rust.fragment(0, 0..rust.document.line(0).len());
    let styled = compact_code_highlights(&fragment_spans, Some(&syntax), 0..0, theme::raised());
    assert!(
        styled
            .iter()
            .any(|(range, style)| *range == (0..2) && style.color == Some(keyword_color)),
        "expected keyword highlight on 'fn', got {styled:?}"
    );
    // Selection background overlays without losing the token's syntax colour.
    let selected = compact_code_highlights(&fragment_spans, Some(&syntax), 0..5, theme::raised());
    assert!(
        selected.iter().any(|(range, style)| *range == (0..2)
            && style.color == Some(keyword_color)
            && style.background_color == Some(theme::raised())),
        "expected selected keyword style, got {selected:?}"
    );

    assert!(unknown.highlights().is_none());
    let (_, unknown_spans) = unknown.fragment(0, 0..unknown.document.line(0).len());
    assert!(
        compact_code_highlights(&unknown_spans, Some(&syntax), 0..0, theme::raised()).is_empty()
    );
}

struct CompactAsyncHighlightTest {
    feed: FeedMarkdown,
}

impl Render for CompactAsyncHighlightTest {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().w(px(360.)).child(self.feed.render_all(
            "async-hl",
            cx.entity_id(),
            None,
            theme::raised(),
            None,
            cx,
        ))
    }
}

#[test]
fn large_compact_rust_block_highlights_in_background_without_extra_parses() {
    let mut source = String::from("```rust\n");
    for i in 0..300 {
        source.push_str(&format!("let value_{i} = {i};\n"));
    }
    source.push_str("```\n");
    assert!(source.len() > 4 * 1024);

    PARSE_COUNT.set(0);
    HIGHLIGHT_COUNT.set(0);
    let feed = FeedMarkdown::new(&source);
    assert_eq!(PARSE_COUNT.get(), 1);
    assert_eq!(HIGHLIGHT_COUNT.get(), 0);
    let Block::Code(code) = &feed.page().blocks[0] else {
        panic!("code block");
    };
    let code = Arc::clone(code);
    assert!(
        code.highlights().is_none(),
        "compact block defers highlighting off the UI thread"
    );

    let mut cx = TestAppContext::single();
    let window = cx.add_window(move |_, _| CompactAsyncHighlightTest { feed });
    cx.run_until_parked();
    let _visual = VisualTestContext::from_window(window.into(), &cx);
    assert_eq!(PARSE_COUNT.get(), 1, "highlighting must not re-parse");
    assert_eq!(HIGHLIGHT_COUNT.get(), 1, "block highlights exactly once");
    let spans = code
        .highlights()
        .expect("background highlight completed")
        .line(0);
    assert_eq!(CAPTURES[spans[0].capture as usize], "keyword");
}

#[test]
fn document_code_block_hover_copy_timer_and_drag_exclusion() {
    use std::time::Duration;

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let mut cx = TestAppContext::single();
    let raw_code = "fn main() {\n    println!(\"hi\");\n}";
    let source = format!("```rust title=\"demo\"\n{raw_code}\n```\n\n```\nplain\n```\n");
    let window = cx.add_window(move |_, cx| {
        let parsed = Arc::new(Page::parse(&source, "docs/README.md"));
        MarkdownView::new(parsed, "docs/README.md".into(), Default::default(), cx)
    });
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);

    let first_code = window
        .read_with(&visual, |view, _| {
            let Block::Code(code) = &view.page().blocks[0] else {
                panic!("first block");
            };
            Arc::clone(code)
        })
        .unwrap();

    // Header bars are visible; labelled block shows its language, unlabelled has no lang node.
    assert!(visual.debug_bounds("FILE_MD_CODE_HEADER 0").is_some());
    assert!(visual.debug_bounds("FILE_MD_CODE_LANG 0").is_some());
    assert!(visual.debug_bounds("FILE_MD_CODE_HEADER 4").is_some());
    assert!(visual.debug_bounds("FILE_MD_CODE_LANG 4").is_none());
    // Controls stay hidden until hovered.
    assert!(visual.debug_bounds("FILE_MD_CODE_WRAP 0").is_none());
    assert!(visual.debug_bounds("FILE_MD_CODE_COPY 0").is_none());

    // Hovering a code line reveals the bar's Wrap and Copy buttons.
    let line0 = visual.debug_bounds("FILE_MD_TEXT 1").unwrap();
    visual.simulate_mouse_move(line0.center(), None, Modifiers::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("FILE_MD_CODE_WRAP 0").is_some());
    let copy_bounds = visual.debug_bounds("FILE_MD_CODE_COPY 0").unwrap();

    // Dragging starting on the Copy button selects nothing.
    let drag_start = copy_bounds.center();
    let drag_end = line0.center();
    visual.simulate_mouse_down(drag_start, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(drag_end, Some(MouseButton::Left), Modifiers::default());
    visual.simulate_mouse_up(drag_end, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    window
        .read_with(&visual, |view, _| {
            assert_eq!(view.selected_text(), None);
        })
        .unwrap();

    // Moving onto the header keeps the controls visible; clicking Copy copies raw code.
    let header = visual.debug_bounds("FILE_MD_CODE_HEADER 0").unwrap();
    visual.simulate_mouse_move(header.center(), None, Modifiers::default());
    visual.run_until_parked();
    let copy_bounds = visual.debug_bounds("FILE_MD_CODE_COPY 0").unwrap();
    visual.simulate_click(copy_bounds.center(), Modifiers::default());
    visual.run_until_parked();
    assert_eq!(
        visual
            .read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some(raw_code)
    );
    assert!(visual.debug_bounds("FILE_MD_CODE_COPIED 0").is_some());
    assert!(window
        .read_with(&visual, |view, _| view.is_code_copied(&first_code))
        .unwrap());

    // Copied stays visible for 2 seconds of test-executor time, then clears.
    visual.executor().advance_clock(Duration::from_millis(1999));
    visual.run_until_parked();
    assert!(visual.debug_bounds("FILE_MD_CODE_COPIED 0").is_some());
    visual.executor().advance_clock(Duration::from_millis(1));
    visual.run_until_parked();
    assert!(visual.debug_bounds("FILE_MD_CODE_COPIED 0").is_none());
    assert!(!window
        .read_with(&visual, |view, _| view.is_code_copied(&first_code))
        .unwrap());
}

struct CompactCodeInteractiveTest {
    feed: FeedMarkdown,
    handler: FeedSelectionHandler,
    readonly_md: &'static str,
}

impl Render for CompactCodeInteractiveTest {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(280.))
            .flex()
            .flex_col()
            .child(self.feed.render_all(
                "feed-code",
                cx.entity_id(),
                None,
                theme::raised(),
                Some(&self.handler),
                cx,
            ))
            .child(render_compact(
                "readonly-code",
                self.readonly_md,
                cx.entity_id(),
                cx,
            ))
    }
}

#[test]
fn compact_code_block_hover_copy_timer_and_wrap_per_block() {
    use std::time::Duration;

    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let long_a = "let very_long_first_code_block_line_that_exceeds_the_narrow_viewport_width_by_far = 1234567890;";
    let long_b = "let second_long_line_in_first_block_moves_together_with_line_one_when_scrolled = 9876543210;";
    let other_long =
        "let independent_second_code_block_line_that_must_stay_wrapped_when_first_unwraps = 42;";
    let feed_md = format!("```rust\n{long_a}\n{long_b}\n```\n\n```sh\n{other_long}\n```\n");
    let readonly_md = "```sh\necho from-preview\n```\n";
    let selection_events = Rc::new(RefCell::new(Vec::new()));
    let recorded = Rc::clone(&selection_events);
    let handler = FeedSelectionHandler::new(move |phase, doc, pos, _, _| {
        recorded.borrow_mut().push((phase, doc.to_owned(), pos));
    });

    let mut cx = TestAppContext::single();
    let window = cx.add_window(move |_, _| CompactCodeInteractiveTest {
        feed: FeedMarkdown::new(&feed_md),
        handler,
        readonly_md,
    });
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);

    // Block 0: header=row 0, line0=row 1, line1=row 2.
    // Block 1: header=row 3, line0=row 4.
    let wrapped_h_block0 = visual
        .debug_bounds("COMPACT_MD_TEXT feed-code 1")
        .unwrap()
        .size
        .height;
    let wrapped_h_block1 = visual
        .debug_bounds("COMPACT_MD_TEXT feed-code 4")
        .unwrap()
        .size
        .height;
    assert!(wrapped_h_block0 > px(24.), "block 0 starts wrapped");
    assert!(wrapped_h_block1 > px(24.), "block 1 starts wrapped");
    assert!(visual
        .debug_bounds("COMPACT_MD_CODE_SCROLLBAR feed-code 2")
        .is_none());

    // Hovering line 1 of block 0 reveals Wrap & Copy on block 0 only.
    let line0 = visual.debug_bounds("COMPACT_MD_TEXT feed-code 1").unwrap();
    visual.simulate_mouse_move(line0.center(), None, Modifiers::default());
    visual.run_until_parked();
    let wrap_btn = visual
        .debug_bounds("COMPACT_MD_CODE_WRAP feed-code 0")
        .expect("block 0 wrap button");
    assert!(visual
        .debug_bounds("COMPACT_MD_CODE_WRAP feed-code 3")
        .is_none());

    // Dragging starting on the Wrap button never begins a feed selection.
    visual.simulate_mouse_down(wrap_btn.center(), MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_move(
        line0.center(),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    visual.simulate_mouse_up(line0.center(), MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    assert!(
        !selection_events
            .borrow()
            .iter()
            .any(|(phase, ..)| *phase == FeedSelectionPhase::Begin),
        "dragging from Wrap button must not start selection: {:?}",
        selection_events.borrow()
    );

    // Clicking Wrap turns wrapping off on block 0 only and shows its horizontal scrollbar.
    let wrap_btn = visual
        .debug_bounds("COMPACT_MD_CODE_WRAP feed-code 0")
        .unwrap();
    visual.simulate_click(wrap_btn.center(), Modifiers::default());
    visual.run_until_parked();
    let unwrapped_line0 = visual.debug_bounds("COMPACT_MD_TEXT feed-code 1").unwrap();
    let unwrapped_line1 = visual.debug_bounds("COMPACT_MD_TEXT feed-code 2").unwrap();
    assert!(
        unwrapped_line0.size.height < wrapped_h_block0,
        "block 0 unwrapped: {:?} vs {:?}",
        unwrapped_line0.size.height,
        wrapped_h_block0
    );
    assert_eq!(
        visual
            .debug_bounds("COMPACT_MD_TEXT feed-code 4")
            .unwrap()
            .size
            .height,
        wrapped_h_block1,
        "block 1 stays wrapped"
    );
    assert!(visual
        .debug_bounds("COMPACT_MD_CODE_SCROLLBAR feed-code 2")
        .is_some());

    // Horizontal wheel over line 0 shifts both lines of block 0 together.
    let header0 = visual
        .debug_bounds("COMPACT_MD_CODE_HEADER feed-code 0")
        .unwrap();
    let before_left0 = unwrapped_line0.left();
    let before_left1 = unwrapped_line1.left();
    assert_eq!(before_left0, before_left1);
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: point(header0.center().x, unwrapped_line0.center().y),
        delta: gpui::ScrollDelta::Pixels(point(px(-80.), px(0.))),
        ..Default::default()
    });
    visual.run_until_parked();
    let scrolled_line0 = visual.debug_bounds("COMPACT_MD_TEXT feed-code 1").unwrap();
    let scrolled_line1 = visual.debug_bounds("COMPACT_MD_TEXT feed-code 2").unwrap();
    assert!(
        scrolled_line0.left() < before_left0,
        "line 0 scrolled left: {:?} vs {:?}",
        scrolled_line0.left(),
        before_left0
    );
    assert_eq!(
        scrolled_line0.left(),
        scrolled_line1.left(),
        "both lines share one horizontal offset"
    );

    // Clicking Wrap again re-wraps block 0 and hides its scrollbar.
    let header0 = visual
        .debug_bounds("COMPACT_MD_CODE_HEADER feed-code 0")
        .unwrap();
    visual.simulate_mouse_move(header0.center(), None, Modifiers::default());
    visual.run_until_parked();
    let wrap_btn = visual
        .debug_bounds("COMPACT_MD_CODE_WRAP feed-code 0")
        .unwrap();
    visual.simulate_click(wrap_btn.center(), Modifiers::default());
    visual.run_until_parked();
    assert_eq!(
        visual
            .debug_bounds("COMPACT_MD_TEXT feed-code 1")
            .unwrap()
            .size
            .height,
        wrapped_h_block0
    );
    assert!(visual
        .debug_bounds("COMPACT_MD_CODE_SCROLLBAR feed-code 2")
        .is_none());

    // Copy in `render_compact` (Skills / role prompt previews) copies raw code and clears after 2s.
    let preview_hdr = visual
        .debug_bounds("COMPACT_MD_CODE_HEADER readonly-code 0")
        .unwrap();
    visual.simulate_mouse_move(preview_hdr.center(), None, Modifiers::default());
    visual.run_until_parked();
    let copy_btn = visual
        .debug_bounds("COMPACT_MD_CODE_COPY readonly-code 0")
        .unwrap();
    visual.simulate_click(copy_btn.center(), Modifiers::default());
    visual.run_until_parked();
    assert_eq!(
        visual
            .read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("echo from-preview")
    );
    assert!(visual
        .debug_bounds("COMPACT_MD_CODE_COPIED readonly-code 0")
        .is_some());
    visual.executor().advance_clock(Duration::from_millis(1999));
    visual.run_until_parked();
    assert!(visual
        .debug_bounds("COMPACT_MD_CODE_COPIED readonly-code 0")
        .is_some());
    visual.executor().advance_clock(Duration::from_millis(1));
    visual.run_until_parked();
    assert!(visual
        .debug_bounds("COMPACT_MD_CODE_COPIED readonly-code 0")
        .is_none());
}

#[test]
fn document_code_block_wrap_toggle_shared_offset_scrollbar_and_block_isolation() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let long0 =
        "let document_block_zero_line_zero_is_much_wider_than_a_narrow_markdown_panel = 100;";
    let long1 =
        "let document_block_zero_line_one_scrolls_horizontally_in_lockstep_with_line_zero = 200;";
    let other = "let document_block_one_remains_wrapped_independently_of_block_zero_state = 300;";
    let source = format!("```rust\n{long0}\n{long1}\n```\n\n```rust\n{other}\n```\n");

    let mut cx = TestAppContext::single();
    let window = cx.add_window(move |_, cx| {
        let parsed = Arc::new(Page::parse(&source, "docs/WRAP.md"));
        MarkdownView::new(parsed, "docs/WRAP.md".into(), Default::default(), cx)
    });
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.simulate_resize(gpui::size(px(320.), px(600.)));
    visual.run_until_parked();

    let (block0, block1) = window
        .read_with(&visual, |view, _| {
            let Block::Code(b0) = &view.page().blocks[0] else {
                panic!("block 0");
            };
            let Block::Code(b1) = &view.page().blocks[1] else {
                panic!("block 1");
            };
            (Arc::clone(b0), Arc::clone(b1))
        })
        .unwrap();

    // Row 0: header0, Row 1: long0, Row 2: long1, Row 3: header1, Row 4: other.
    let wrapped_h0 = visual.debug_bounds("FILE_MD_TEXT 1").unwrap().size.height;
    let wrapped_h1 = visual.debug_bounds("FILE_MD_TEXT 4").unwrap().size.height;
    assert!(wrapped_h0 > px(26.), "block 0 wraps by default");
    assert!(wrapped_h1 > px(26.), "block 1 wraps by default");

    // Hover and click Wrap on block 0.
    let header0 = visual.debug_bounds("FILE_MD_CODE_HEADER 0").unwrap();
    visual.simulate_mouse_move(header0.center(), None, Modifiers::default());
    visual.run_until_parked();
    let wrap_btn = visual.debug_bounds("FILE_MD_CODE_WRAP 0").unwrap();
    visual.simulate_click(wrap_btn.center(), Modifiers::default());
    visual.run_until_parked();

    window
        .read_with(&visual, |view, _| {
            assert!(view.is_code_unwrapped(&block0));
            assert!(!view.is_code_unwrapped(&block1));
        })
        .unwrap();
    let line0 = visual.debug_bounds("FILE_MD_TEXT 1").unwrap();
    let line1 = visual.debug_bounds("FILE_MD_TEXT 2").unwrap();
    assert!(line0.size.height < wrapped_h0);
    assert_eq!(
        visual.debug_bounds("FILE_MD_TEXT 4").unwrap().size.height,
        wrapped_h1
    );
    let scrollbar = visual
        .debug_bounds("FILE_MD_CODE_SCROLLBAR 2")
        .expect("horizontal scrollbar under last line");

    // Wheel scroll over line 1 shifts both lines together.
    let before_left = line0.left();
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: point(header0.center().x, line1.center().y),
        delta: gpui::ScrollDelta::Pixels(point(px(-90.), px(0.))),
        ..Default::default()
    });
    visual.run_until_parked();
    let after_scroll = window
        .read_with(&visual, |view, _| view.code_scroll_x(&block0))
        .unwrap();
    assert!(after_scroll > 0., "horizontal wheel scrolled block 0");
    let shifted0 = visual.debug_bounds("FILE_MD_TEXT 1").unwrap();
    let shifted1 = visual.debug_bounds("FILE_MD_TEXT 2").unwrap();
    assert!(shifted0.left() < before_left);
    assert_eq!(shifted0.left(), shifted1.left());

    // Clicking near the right end of the scrollbar track jumps further right without selecting.
    let track_right = point(scrollbar.right() - px(4.), scrollbar.center().y);
    visual.simulate_mouse_down(track_right, MouseButton::Left, Modifiers::default());
    visual.simulate_mouse_up(track_right, MouseButton::Left, Modifiers::default());
    visual.run_until_parked();
    let jumped_scroll = window
        .read_with(&visual, |view, _| view.code_scroll_x(&block0))
        .unwrap();
    assert!(jumped_scroll > after_scroll);
    assert_eq!(
        window
            .read_with(&visual, |view, _| view.selected_text())
            .unwrap(),
        None
    );

    // Clicking Wrap again re-wraps block 0 and hides the scrollbar.
    let header0 = visual.debug_bounds("FILE_MD_CODE_HEADER 0").unwrap();
    visual.simulate_mouse_move(header0.center(), None, Modifiers::default());
    visual.run_until_parked();
    let wrap_btn = visual.debug_bounds("FILE_MD_CODE_WRAP 0").unwrap();
    visual.simulate_click(wrap_btn.center(), Modifiers::default());
    visual.run_until_parked();
    assert!(!window
        .read_with(&visual, |view, _| view.is_code_unwrapped(&block0))
        .unwrap());
    assert_eq!(
        visual.debug_bounds("FILE_MD_TEXT 1").unwrap().size.height,
        wrapped_h0
    );
    assert!(visual.debug_bounds("FILE_MD_CODE_SCROLLBAR 2").is_none());
}

struct CompactPreviewLifecycleTest {
    show_preview: bool,
    markdown: String,
    redraws: usize,
}

impl Render for CompactPreviewLifecycleTest {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(280.))
            .flex()
            .flex_col()
            .when(self.show_preview, |el| {
                el.child(render_compact(
                    "lifecycle-preview",
                    &self.markdown,
                    cx.entity_id(),
                    cx,
                ))
            })
    }
}

#[test]
fn compact_preview_teardown_and_reopen_resets_wrap_and_prunes_state() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let long = "let preview_code_line_long_enough_to_wrap_inside_a_narrow_preview_card = 42;";
    let markdown = format!("```rust\n{long}\n```\n");

    let mut cx = TestAppContext::single();
    let window = cx.add_window(move |_, _| CompactPreviewLifecycleTest {
        show_preview: true,
        markdown,
        redraws: 0,
    });
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);

    let wrapped_h = visual
        .debug_bounds("COMPACT_MD_TEXT lifecycle-preview 1")
        .unwrap()
        .size
        .height;
    assert!(wrapped_h > px(24.), "starts wrapped");

    // Hover and turn Wrap off.
    let header = visual
        .debug_bounds("COMPACT_MD_CODE_HEADER lifecycle-preview 0")
        .unwrap();
    visual.simulate_mouse_move(header.center(), None, Modifiers::default());
    visual.run_until_parked();
    let wrap_btn = visual
        .debug_bounds("COMPACT_MD_CODE_WRAP lifecycle-preview 0")
        .unwrap();
    visual.simulate_click(wrap_btn.center(), Modifiers::default());
    visual.run_until_parked();
    let unwrapped_h = visual
        .debug_bounds("COMPACT_MD_TEXT lifecycle-preview 1")
        .unwrap()
        .size
        .height;
    assert!(unwrapped_h < wrapped_h, "unwrapped on click");

    // Unrelated redraws while mounted preserve the Wrap choice.
    window
        .update(&mut visual, |view, _, cx| {
            view.redraws += 1;
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(
        visual
            .debug_bounds("COMPACT_MD_TEXT lifecycle-preview 1")
            .unwrap()
            .size
            .height,
        unwrapped_h,
        "redraw preserves unwrapped state while preview stays open"
    );

    // Closing the preview drops its element state; reopening starts wrapped again
    // and prunes the closed preview's state from CompactCodeStates.
    visual.simulate_mouse_move(point(px(0.), px(500.)), None, Modifiers::default());
    window
        .update(&mut visual, |view, _, cx| {
            view.show_preview = false;
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual
        .debug_bounds("COMPACT_MD_CODE_HEADER lifecycle-preview 0")
        .is_none());

    window
        .update(&mut visual, |view, _, cx| {
            view.show_preview = true;
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(
        visual
            .debug_bounds("COMPACT_MD_TEXT lifecycle-preview 1")
            .unwrap()
            .size
            .height,
        wrapped_h,
        "reopening a closed preview resets Wrap to default"
    );
    assert!(visual
        .debug_bounds("COMPACT_MD_CODE_SCROLLBAR lifecycle-preview 1")
        .is_none());
    visual.read(|cx| {
        assert!(
            cx.try_global::<CompactCodeStates>()
                .is_none_or(CompactCodeStates::is_empty),
            "closed preview leaves no state in CompactCodeStates"
        );
    });
}

struct CompactSyntaxThemeReuseTest {
    prose: FeedMarkdown,
    code_a: FeedMarkdown,
    code_b: FeedMarkdown,
    show_code: bool,
}

impl Render for CompactSyntaxThemeReuseTest {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(360.))
            .flex()
            .flex_col()
            .child(self.prose.render_all(
                "prose-only",
                cx.entity_id(),
                None,
                theme::raised(),
                None,
                cx,
            ))
            .when(self.show_code, |el| {
                el.child(self.code_a.render_all(
                    "code-a",
                    cx.entity_id(),
                    None,
                    theme::raised(),
                    None,
                    cx,
                ))
                .child(self.code_b.render_all(
                    "code-b",
                    cx.entity_id(),
                    None,
                    theme::raised(),
                    None,
                    cx,
                ))
            })
    }
}

#[test]
fn compact_syntax_theme_skips_prose_and_reuses_palette_for_code_chunks() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let mut cx = TestAppContext::single();
    clear_compact_syntax_theme_cache();
    let window = cx.add_window(|_, _| CompactSyntaxThemeReuseTest {
        prose: FeedMarkdown::new("Just a paragraph.\n\n- And a list item"),
        code_a: FeedMarkdown::new("```rust\nlet a = 1;\n```"),
        code_b: FeedMarkdown::new("```rust\nlet b = 2;\n```"),
        show_code: false,
    });
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    assert_eq!(
        SYNTAX_THEME_BUILDS.get(),
        0,
        "prose chunks never build a SyntaxTheme"
    );

    window
        .update(&mut visual, |view, _, cx| {
            view.show_code = true;
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(
        SYNTAX_THEME_BUILDS.get(),
        1,
        "first highlighted draw builds SyntaxTheme once and shares it across both code chunks"
    );

    SYNTAX_THEME_BUILDS.set(0);
    window.update(&mut visual, |_, _, cx| cx.notify()).unwrap();
    visual.run_until_parked();
    assert_eq!(
        SYNTAX_THEME_BUILDS.get(),
        0,
        "redrawing highlighted code reuses the cached SyntaxTheme"
    );
}
