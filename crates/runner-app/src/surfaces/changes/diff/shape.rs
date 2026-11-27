use super::*;

pub(super) struct ShapedUnifiedRow {
    pub(super) row_index: usize,
    pub(super) bg: Option<Hsla>,
    pub(super) cursor: bool,
    pub(super) old_num: Option<ShapedLine>,
    pub(super) new_num: Option<ShapedLine>,
    pub(super) sign: Option<ShapedLine>,
    pub(super) segments: Vec<(usize, ShapedLine)>,
    pub(super) hunk_line: Option<ShapedLine>,
    #[cfg(test)]
    pub(super) painted: Vec<(String, Hsla)>,
}

pub(super) struct ShapedSplitHalf {
    pub(super) bg: Option<Hsla>,
    pub(super) num: Option<ShapedLine>,
    pub(super) sign: Option<ShapedLine>,
    pub(super) segments: Vec<(usize, ShapedLine)>,
    #[cfg(test)]
    pub(super) painted: Vec<(String, Hsla)>,
}

#[allow(clippy::large_enum_variant)]
pub(super) enum ShapedSplitRowKind {
    Hunk(ShapedLine),
    Pair {
        left: ShapedSplitHalf,
        right: ShapedSplitHalf,
    },
}

pub(super) struct ShapedSplitRow {
    pub(super) row_index: usize,
    pub(super) cursor: bool,
    pub(super) kind: ShapedSplitRowKind,
}

pub(super) enum ShapedRows {
    Unified(Vec<ShapedUnifiedRow>),
    Split(Vec<ShapedSplitRow>),
}

pub(super) struct RowStyle<'a> {
    pub(super) font: &'a Font,
    pub(super) font_size: Pixels,
    pub(super) char_width: Pixels,
    pub(super) color: Hsla,
    pub(super) syntax: Option<(&'a Highlights, &'a SyntaxTheme)>,
}

pub(super) struct ShapedDocLine {
    pub(super) segments: Vec<(usize, ShapedLine)>,
    #[cfg(test)]
    pub(super) painted: Vec<(String, Hsla)>,
}

pub(super) fn shape_doc_line(
    document: &Document,
    line: usize,
    (from, to): (usize, usize),
    style: &RowStyle,
    window: &Window,
) -> ShapedDocLine {
    let spans = style
        .syntax
        .map_or(&[][..], |(highlights, _)| highlights.line(line));
    #[cfg(test)]
    let mut painted = Vec::new();
    let segments = document
        .segments(line, from, to, spans)
        .into_iter()
        .map(|segment| {
            let runs = segment
                .runs
                .iter()
                .map(|&(len, capture)| TextRun {
                    len,
                    font: style.font.clone(),
                    color: capture
                        .zip(style.syntax)
                        .and_then(|(capture, (_, theme))| theme.color(capture))
                        .unwrap_or(style.color),
                    background_color: None,
                    underline: None,
                    strikethrough: None,
                })
                .collect::<Vec<_>>();
            #[cfg(test)]
            {
                let mut at = 0;
                for run in &runs {
                    painted.push((segment.text[at..at + run.len].to_owned(), run.color));
                    at += run.len;
                }
            }
            let text: SharedString = segment.text.into();
            let shaped = window.text_system().shape_line(
                text,
                style.font_size,
                &runs,
                Some(style.char_width * segment.cells as f32),
            );
            (segment.column, shaped)
        })
        .collect();
    ShapedDocLine {
        segments,
        #[cfg(test)]
        painted,
    }
}

pub(super) fn shape_plain(
    text: SharedString,
    font: &Font,
    size: Pixels,
    color: Hsla,
    window: &Window,
) -> ShapedLine {
    let len = text.len();
    window.text_system().shape_line(
        text,
        size,
        &[TextRun {
            len,
            font: font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }],
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn shape_split_cell(
    cell: SplitCell,
    doc: &Document,
    style: &RowStyle,
    (from, to): (usize, usize),
    font: &Font,
    font_size: Pixels,
    num_font_size: Pixels,
    number_color: Hsla,
    add_bg: Hsla,
    del_bg: Hsla,
    filler_bg: Hsla,
    window: &Window,
) -> ShapedSplitHalf {
    match cell {
        SplitCell::Empty => ShapedSplitHalf {
            bg: Some(filler_bg),
            num: None,
            sign: None,
            segments: Vec::new(),
            #[cfg(test)]
            painted: Vec::new(),
        },
        SplitCell::Context { line } => {
            let shaped = shape_doc_line(doc, line, (from, to), style, window);
            let num = Some(shape_plain(
                (line + 1).to_string().into(),
                font,
                num_font_size,
                number_color,
                window,
            ));
            ShapedSplitHalf {
                bg: None,
                num,
                sign: None,
                segments: shaped.segments,
                #[cfg(test)]
                painted: shaped.painted,
            }
        }
        SplitCell::Removed { line } => {
            let shaped = shape_doc_line(doc, line, (from, to), style, window);
            let num = Some(shape_plain(
                (line + 1).to_string().into(),
                font,
                num_font_size,
                number_color,
                window,
            ));
            let sign = Some(shape_plain(
                "−".into(),
                font,
                font_size,
                theme::danger(),
                window,
            ));
            ShapedSplitHalf {
                bg: Some(del_bg),
                num,
                sign,
                segments: shaped.segments,
                #[cfg(test)]
                painted: shaped.painted,
            }
        }
        SplitCell::Added { line } => {
            let shaped = shape_doc_line(doc, line, (from, to), style, window);
            let num = Some(shape_plain(
                (line + 1).to_string().into(),
                font,
                num_font_size,
                number_color,
                window,
            ));
            let sign = Some(shape_plain(
                "+".into(),
                font,
                font_size,
                theme::accent(),
                window,
            ));
            ShapedSplitHalf {
                bg: Some(add_bg),
                num,
                sign,
                segments: shaped.segments,
                #[cfg(test)]
                painted: shaped.painted,
            }
        }
    }
}
