use gpui::TextRun;
use pulldown_cmark::Alignment;

use super::draw::with_selection;
use super::layout::fragments;
use super::*;

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
    assert_eq!(text.links[0].1, "guide.md");
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
            link: Some("guide.md".into()),
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
    assert_eq!(page.rows[999].text(), "line 999");
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
    assert_eq!(rows, pieces.len() + 1);
    // Tabs keep their stops across a split: every piece expands as the
    // whole line would.
    let drawn: String = (0..pieces.len()).map(|row| page.rows[row].text()).collect();
    assert_eq!(drawn, "é   ".repeat(600) + "end");
    let view_text = {
        let mut copied = String::new();
        for (index, row) in page.rows.iter().enumerate() {
            if index > 0 && !row.continues() {
                copied.push('\n');
            }
            copied.push_str(&row.text());
        }
        copied
    };
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
    assert_eq!(before.links, [(0..7, "guide.md".to_owned())]);
    assert_eq!(image.link.as_deref(), Some("guide.md"));
    assert_eq!(after.text, " after");
    assert_eq!(after.links, [(0..6, "guide.md".to_owned())]);
    let [Piece::Text(a), Piece::Image(i), Piece::Text(b)] = &rows[0][0][..] else {
        panic!("{:?}", rows[0][0]);
    };
    assert_eq!(a.links, [(0..2, "t.md".to_owned())]);
    assert_eq!(i.link.as_deref(), Some("t.md"));
    assert_eq!(b.links, [(0..2, "t.md".to_owned())]);
}
