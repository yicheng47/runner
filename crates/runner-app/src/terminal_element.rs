//! Custom GPUI element painting the `alacritty_terminal` grid: cell
//! backgrounds as quads, glyphs as shaped lines, cursor on top.
//!
//! Alignment strategy: same-style ASCII spans are shaped as one run
//! (monospace advance == cell width, so columns stay true); anything
//! else — wide CJK, emoji, box drawing — is shaped per cell and
//! painted at its own column origin, so a fallback font's advance can
//! never skew the grid (the xterm garble class this spike exists to
//! kill). gpui caches shaped lines, so per-cell shaping of repetitive
//! glyphs stays cheap.

use std::sync::Arc;

use alacritty_terminal::index::Point as GridPoint;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::point_to_viewport;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor, Rgb};
use gpui::{
    fill, font, outline, point, px, relative, size, App, Bounds, ContentMask, Element,
    ElementInputHandler, Entity, FocusHandle, Font, GlobalElementId, Hsla, InspectorElementId,
    IntoElement, LayoutId, Pixels, Point, ShapedLine, SharedString, Style, TextRun, UnderlineStyle,
    Window,
};

use runner_app::terminal_ime::TerminalInput;
use runner_app::terminal_resize::{
    size_push_verdict, terminal_grid_size, SizePushVerdict, TerminalGridSize,
};
use runner_terminal::palette;
use runner_terminal::terminal::TerminalSession;

pub const FONT_FAMILY: &str = "Menlo";
pub const FONT_SIZE: f32 = 13.0;
pub const LINE_HEIGHT_FACTOR: f32 = 1.4;

fn to_hsla(rgb: Rgb, alpha: f32) -> Hsla {
    let mut rgba = gpui::rgb(((rgb.r as u32) << 16) | ((rgb.g as u32) << 8) | rgb.b as u32);
    rgba.a = alpha;
    rgba.into()
}

pub struct TerminalElement {
    session: Arc<TerminalSession>,
    input: Entity<TerminalInput>,
    focus_handle: FocusHandle,
    resize_owner: bool,
}

impl TerminalElement {
    pub fn new(
        session: Arc<TerminalSession>,
        input: Entity<TerminalInput>,
        focus_handle: FocusHandle,
        resize_owner: bool,
    ) -> Self {
        Self {
            session,
            input,
            focus_handle,
            resize_owner,
        }
    }
}

pub struct GridPrepaint {
    backgrounds: Vec<(Bounds<Pixels>, Hsla)>,
    lines: Vec<(Point<Pixels>, ShapedLine)>,
    cursor: Option<(Bounds<Pixels>, CursorShape)>,
    cursor_cell: Option<Bounds<Pixels>>,
    marked_text: Option<(Bounds<Pixels>, ShapedLine, Hsla)>,
    cell_width: Pixels,
    line_height: Pixels,
}

struct StyledSpan {
    col: usize,
    text: String,
    runs: Vec<TextRun>,
    /// True only while every cell in the span is narrow ASCII; only
    /// such spans may be extended (their byte len == column count and
    /// the monospace advance is exact).
    ascii_only: bool,
}

impl Element for TerminalElement {
    type RequestLayoutState = ();
    type PrepaintState = GridPrepaint;

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> GridPrepaint {
        let text_system = window.text_system();
        let base_font = font(FONT_FAMILY);
        let font_size = px(FONT_SIZE);
        let font_id = text_system.resolve_font(&base_font);
        let cell_width = text_system
            .em_advance(font_id, font_size)
            .unwrap_or(px(FONT_SIZE * 0.6));
        let line_height = px((FONT_SIZE * LINE_HEIGHT_FACTOR).round());

        let measured = terminal_grid_size(
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
            f32::from(cell_width),
            f32::from(line_height),
        );
        let (last_cols, last_rows) = self.session.size();
        match size_push_verdict(
            measured,
            TerminalGridSize {
                cols: last_cols,
                rows: last_rows,
            },
            self.resize_owner,
        ) {
            SizePushVerdict::Push(size) => self.session.resize(size.cols, size.rows),
            SizePushVerdict::Unchanged | SizePushVerdict::SuppressedNonOwner => {}
            SizePushVerdict::SuppressedUnplaced => {
                return GridPrepaint {
                    backgrounds: Vec::new(),
                    lines: Vec::new(),
                    cursor: None,
                    cursor_cell: None,
                    marked_text: None,
                    cell_width,
                    line_height,
                };
            }
        }
        let (cols, rows) = self.session.size();

        let base = palette::base_palette();
        let mut backgrounds: Vec<(Bounds<Pixels>, Hsla)> = Vec::new();
        let mut spans: Vec<(usize, StyledSpan)> = Vec::new();
        let mut cursor = None;
        let mut cursor_cell = None;
        let marked_foreground;
        let marked_background;

        {
            let term = self.session.term.lock();
            let content = term.renderable_content();
            let display_offset = content.display_offset;
            let overrides = content.colors;
            marked_foreground =
                palette::resolve(Color::Named(NamedColor::Foreground), overrides, &base);
            marked_background =
                palette::resolve(Color::Named(NamedColor::Background), overrides, &base);

            for indexed in content.display_iter {
                let cell = &indexed.cell;
                if cell
                    .flags
                    .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }
                let Some(vp) = point_to_viewport(display_offset, indexed.point) else {
                    continue;
                };
                let (row, col) = (vp.line, vp.column.0);
                if row >= rows as usize || col >= cols as usize {
                    continue;
                }

                let mut fg = palette::resolve(cell.fg, overrides, &base);
                let mut bg = palette::resolve(cell.bg, overrides, &base);
                if cell.flags.contains(Flags::INVERSE) {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if cell.flags.contains(Flags::DIM) {
                    fg = fg * 0.66;
                }

                let wide = cell.flags.contains(Flags::WIDE_CHAR);
                let cell_cols = if wide { 2usize } else { 1 };

                if bg != palette::BACKGROUND {
                    let quad_bounds = Bounds::new(
                        point(
                            bounds.left() + cell_width * col as f32,
                            bounds.top() + line_height * row as f32,
                        ),
                        size(cell_width * cell_cols as f32, line_height),
                    );
                    // Merge with the previous quad when contiguous and
                    // same color, to keep quad count sane.
                    match backgrounds.last_mut() {
                        Some((last, color))
                            if *color == to_hsla(bg, 1.)
                                && last.top() == quad_bounds.top()
                                && last.right() == quad_bounds.left() =>
                        {
                            last.size.width += quad_bounds.size.width;
                        }
                        _ => backgrounds.push((quad_bounds, to_hsla(bg, 1.))),
                    }
                }

                if cell.flags.contains(Flags::HIDDEN)
                    || (cell.c == ' ' && cell.zerowidth().is_none())
                {
                    continue;
                }

                let mut cell_font: Font = base_font.clone();
                if cell.flags.contains(Flags::BOLD) {
                    cell_font = cell_font.bold();
                }
                if cell.flags.contains(Flags::ITALIC) {
                    cell_font = cell_font.italic();
                }
                let underline = if cell.flags.intersects(Flags::ALL_UNDERLINES) {
                    Some(gpui::UnderlineStyle {
                        color: Some(to_hsla(
                            cell.underline_color()
                                .map(|c| palette::resolve(c, overrides, &base))
                                .unwrap_or(fg),
                            1.,
                        )),
                        thickness: px(1.),
                        wavy: cell.flags.contains(Flags::UNDERCURL),
                    })
                } else {
                    None
                };
                let strikethrough = if cell.flags.contains(Flags::STRIKEOUT) {
                    Some(gpui::StrikethroughStyle {
                        color: Some(to_hsla(fg, 1.)),
                        thickness: px(1.),
                    })
                } else {
                    None
                };

                let mut text = String::new();
                text.push(cell.c);
                if let Some(zw) = cell.zerowidth() {
                    text.extend(zw.iter());
                }
                let run = TextRun {
                    len: text.len(),
                    font: cell_font,
                    color: to_hsla(fg, 1.),
                    background_color: None,
                    underline,
                    strikethrough,
                };

                // Extend the open span only when BOTH the span so far
                // and this cell are simple ASCII — an ASCII cell after
                // a non-ASCII narrow glyph (box drawing, braille)
                // must start its own span, or it inherits the
                // fallback font's advance and skews off the grid.
                let simple = !wide && cell.c.is_ascii() && cell.zerowidth().is_none();
                let extended = simple
                    && match spans.last_mut() {
                        Some((last_row, span))
                            if *last_row == row
                                && span.ascii_only
                                && span.col + span.text.len() == col =>
                        {
                            match span.runs.last_mut() {
                                Some(last_run)
                                    if last_run.font == run.font
                                        && last_run.color == run.color
                                        && last_run.underline == run.underline
                                        && last_run.strikethrough == run.strikethrough =>
                                {
                                    last_run.len += run.len;
                                }
                                _ => span.runs.push(run.clone()),
                            }
                            span.text.push(cell.c);
                            true
                        }
                        _ => false,
                    };
                if !extended {
                    spans.push((
                        row,
                        StyledSpan {
                            col,
                            text,
                            runs: vec![run],
                            ascii_only: simple,
                        },
                    ));
                }
            }

            let rc = content.cursor;
            if let Some(vp) = point_to_viewport(
                display_offset,
                GridPoint::new(rc.point.line, rc.point.column),
            ) {
                if vp.line < rows as usize && vp.column.0 < cols as usize {
                    let origin = point(
                        bounds.left() + cell_width * vp.column.0 as f32,
                        bounds.top() + line_height * vp.line as f32,
                    );
                    let bounds = Bounds::new(origin, size(cell_width, line_height));
                    cursor_cell = Some(bounds);
                    if rc.shape != CursorShape::Hidden {
                        cursor = Some((bounds, rc.shape));
                    }
                }
            }
        }

        let lines = spans
            .into_iter()
            .map(|(row, span)| {
                let shaped = window.text_system().shape_line(
                    SharedString::from(span.text),
                    font_size,
                    &span.runs,
                    None,
                );
                (
                    point(
                        bounds.left() + cell_width * span.col as f32,
                        bounds.top() + line_height * row as f32,
                    ),
                    shaped,
                )
            })
            .collect();

        let marked_text =
            self.input
                .read(cx)
                .marked_text()
                .zip(cursor_cell)
                .map(|(text, cursor_bounds)| {
                    let color = to_hsla(marked_foreground, 1.);
                    let line = window.text_system().shape_line(
                        SharedString::from(text.to_owned()),
                        font_size,
                        &[TextRun {
                            len: text.len(),
                            font: base_font,
                            color,
                            background_color: None,
                            underline: Some(UnderlineStyle {
                                color: Some(color),
                                thickness: px(1.),
                                wavy: false,
                            }),
                            strikethrough: None,
                        }],
                        None,
                    );
                    (cursor_bounds, line, to_hsla(marked_background, 1.))
                });

        GridPrepaint {
            backgrounds,
            lines,
            cursor,
            cursor_cell,
            marked_text,
            cell_width,
            line_height,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        prepaint: &mut GridPrepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        for (quad_bounds, color) in &prepaint.backgrounds {
            window.paint_quad(fill(*quad_bounds, *color));
        }
        for (origin, line) in &prepaint.lines {
            let _ = line.paint(*origin, prepaint.line_height, window, cx);
        }
        if let Some((cursor_bounds, line, background)) = &prepaint.marked_text {
            window.with_content_mask(Some(ContentMask { bounds }), |window| {
                window.paint_quad(fill(
                    Bounds::new(
                        cursor_bounds.origin,
                        size(line.width.max(prepaint.cell_width), prepaint.line_height),
                    ),
                    *background,
                ));
                let _ = line.paint(cursor_bounds.origin, prepaint.line_height, window, cx);
            });
        } else if let Some((cursor_bounds, shape)) = prepaint.cursor {
            let focused = self.focus_handle.is_focused(window);
            let color = to_hsla(palette::CURSOR, if focused { 0.55 } else { 0.3 });
            match shape {
                CursorShape::Block if focused => {
                    window.paint_quad(fill(cursor_bounds, color));
                }
                CursorShape::Block | CursorShape::HollowBlock => {
                    window.paint_quad(outline(cursor_bounds, color, Default::default()));
                }
                CursorShape::Underline => {
                    let mut b = cursor_bounds;
                    b.origin.y = b.bottom() - px(2.);
                    b.size.height = px(2.);
                    window.paint_quad(fill(b, color));
                }
                CursorShape::Beam => {
                    let mut b = cursor_bounds;
                    b.size.width = px(2.);
                    window.paint_quad(fill(b, color));
                }
                CursorShape::Hidden => {}
            }
        }
        let input_bounds = prepaint.cursor_cell.unwrap_or_else(|| {
            Bounds::new(
                bounds.origin,
                size(prepaint.cell_width, prepaint.line_height),
            )
        });
        window.handle_input(
            &self.focus_handle,
            ElementInputHandler::new(input_bounds, self.input.clone()),
            cx,
        );
    }
}

impl IntoElement for TerminalElement {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}
