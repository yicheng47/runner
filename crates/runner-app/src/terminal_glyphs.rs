use gpui::{fill, point, px, size, Bounds, ContentMask, Hsla, PathBuilder, Pixels, Point, Window};

#[path = "terminal_glyph_data.rs"]
mod data;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Draw {
    Fill,
    Stroke,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scale {
    Cell,
    Char,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Octant {
    x: u8,
    y: u8,
    width: u8,
    height: u8,
}

impl Octant {
    pub(super) const fn new(x: u8, y: u8, width: u8, height: u8) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Part {
    Octants(&'static [Octant]),
    Pattern {
        alpha: f32,
        clip: &'static str,
    },
    Path {
        data: &'static str,
        draw: Draw,
        stroke_width: f32,
        scale: Scale,
        left_padding: f32,
        right_padding: f32,
        snap: bool,
    },
    DynamicPath {
        base: &'static str,
        x: &'static str,
        y: &'static str,
        draw: Draw,
        stroke_width: f32,
        scale: Scale,
    },
    NegativePath {
        data: &'static str,
        draw: Draw,
        scale: Scale,
    },
}

#[derive(Clone, Copy)]
enum Glyph {
    Parts(&'static [Part]),
    Braille(u8),
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Rect {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
}

impl Rect {
    fn width(self) -> f32 {
        self.right - self.left
    }

    fn height(self) -> f32 {
        self.bottom - self.top
    }

    fn to_bounds(self) -> Bounds<Pixels> {
        Bounds::new(
            point(px(self.left), px(self.top)),
            size(px(self.width()), px(self.height())),
        )
    }
}

pub(crate) struct ProceduralCell {
    glyph: Glyph,
    bounds: Bounds<Pixels>,
    foreground: Hsla,
    background: Hsla,
    font_size: Pixels,
}

impl ProceduralCell {
    pub(crate) fn new(
        character: char,
        bounds: Bounds<Pixels>,
        foreground: Hsla,
        background: Hsla,
        font_size: Pixels,
    ) -> Option<Self> {
        Some(Self {
            glyph: glyph(character)?,
            bounds,
            foreground,
            background,
            font_size,
        })
    }

    pub(crate) fn paint(&self, window: &mut Window) {
        if matches!(self.glyph, Glyph::Braille(0)) {
            return;
        }
        let scale_factor = window.scale_factor();
        window.with_content_mask(
            Some(ContentMask {
                bounds: self.bounds,
            }),
            |window| match self.glyph {
                Glyph::Parts(parts) => {
                    for part in parts {
                        paint_part(*part, self, scale_factor, window);
                    }
                }
                Glyph::Braille(pattern) => {
                    paint_braille(pattern, Rect::from(self.bounds), self.foreground, window)
                }
            },
        );
    }
}

impl From<Bounds<Pixels>> for Rect {
    fn from(bounds: Bounds<Pixels>) -> Self {
        Self {
            left: f32::from(bounds.left()),
            top: f32::from(bounds.top()),
            right: f32::from(bounds.right()),
            bottom: f32::from(bounds.bottom()),
        }
    }
}

fn glyph(character: char) -> Option<Glyph> {
    if character < '\u{2500}' {
        return None;
    }
    if ('\u{2800}'..='\u{28ff}').contains(&character) {
        return Some(Glyph::Braille((character as u32 - 0x2800) as u8));
    }
    data::table_parts(character).map(Glyph::Parts)
}

pub(crate) fn snapped_cell_bounds(
    terminal_bounds: Bounds<Pixels>,
    cell_width: Pixels,
    line_height: Pixels,
    row: usize,
    column: usize,
    scale_factor: f32,
) -> Bounds<Pixels> {
    let origin_x = f32::from(terminal_bounds.left());
    let origin_y = f32::from(terminal_bounds.top());
    let cell_width = f32::from(cell_width);
    let line_height = f32::from(line_height);
    Rect {
        left: snap(origin_x + cell_width * column as f32, scale_factor),
        top: snap(origin_y + line_height * row as f32, scale_factor),
        right: snap(origin_x + cell_width * (column + 1) as f32, scale_factor),
        bottom: snap(origin_y + line_height * (row + 1) as f32, scale_factor),
    }
    .to_bounds()
}

fn snap(value: f32, scale_factor: f32) -> f32 {
    (value * scale_factor).round() / scale_factor
}

fn paint_part(part: Part, cell: &ProceduralCell, scale_factor: f32, window: &mut Window) {
    let cell_area = Rect::from(cell.bounds);
    match part {
        Part::Octants(rects) => {
            for rect in rects {
                window.paint_quad(fill(
                    octant_bounds(cell_area, *rect, scale_factor).to_bounds(),
                    cell.foreground,
                ));
            }
        }
        Part::Pattern { alpha, clip } => {
            // Preserve xterm's pattern coverage as flat alpha instead of a device-pixel dither,
            // which keeps the intended shade without introducing moire under app zoom.
            let mut color = cell.foreground;
            color.a *= alpha;
            if clip.is_empty() {
                window.paint_quad(fill(cell.bounds, color));
            } else {
                paint_path(
                    clip,
                    None,
                    Draw::Fill,
                    0.,
                    cell_area,
                    0.,
                    0.,
                    false,
                    color,
                    scale_factor,
                    window,
                );
            }
        }
        Part::Path {
            data,
            draw,
            stroke_width,
            scale,
            left_padding,
            right_padding,
            snap,
        } => {
            let area = scale_area(cell_area, scale, f32::from(cell.font_size), scale_factor);
            let width = resolved_stroke_width(draw, stroke_width, cell.font_size);
            let padding_unit = f32::from(cell.font_size) / 24.;
            paint_path(
                data,
                None,
                draw,
                width,
                area,
                left_padding * padding_unit,
                right_padding * padding_unit,
                snap,
                cell.foreground,
                scale_factor,
                window,
            );
        }
        Part::DynamicPath {
            base,
            x,
            y,
            draw,
            stroke_width,
            scale,
        } => {
            let area = scale_area(cell_area, scale, f32::from(cell.font_size), scale_factor);
            let width = resolved_stroke_width(draw, stroke_width, cell.font_size);
            paint_path(
                base,
                Some((x, y)),
                draw,
                width,
                area,
                0.,
                0.,
                true,
                cell.foreground,
                scale_factor,
                window,
            );
        }
        Part::NegativePath { data, draw, scale } => {
            window.paint_quad(fill(cell.bounds, cell.foreground));
            let area = scale_area(cell_area, scale, f32::from(cell.font_size), scale_factor);
            // GPUI strokes use butt caps, so open cutouts end half a stroke earlier than xterm's
            // square-capped negative paths.
            let width = if draw == Draw::Stroke { 1. } else { 0. };
            paint_path(
                data,
                None,
                draw,
                width,
                area,
                0.,
                0.,
                false,
                cell.background,
                scale_factor,
                window,
            );
        }
    }
}

fn resolved_stroke_width(draw: Draw, stroke_width: f32, font_size: Pixels) -> f32 {
    if draw == Draw::Fill {
        0.
    } else if stroke_width > 0. {
        stroke_width
    } else {
        f32::from(font_size) / 12.
    }
}

fn scale_area(cell: Rect, scale: Scale, font_size: f32, scale_factor: f32) -> Rect {
    if scale == Scale::Cell {
        return cell;
    }
    // GPUI exposes the em advance and font size but not xterm's measured character line box, so
    // CHAR shapes use the full cell width and a centered font-size-tall approximation.
    let char_height = font_size.min(cell.height());
    let center = (cell.top + cell.bottom) / 2.;
    Rect {
        left: cell.left,
        right: cell.right,
        top: snap(center - char_height / 2., scale_factor),
        bottom: snap(center + char_height / 2., scale_factor),
    }
}

fn octant_bounds(cell: Rect, octant: Octant, scale_factor: f32) -> Rect {
    Rect {
        left: snap(
            cell.left + cell.width() * octant.x as f32 / 8.,
            scale_factor,
        ),
        top: snap(
            cell.top + cell.height() * octant.y as f32 / 8.,
            scale_factor,
        ),
        right: snap(
            cell.left + cell.width() * (octant.x + octant.width) as f32 / 8.,
            scale_factor,
        ),
        bottom: snap(
            cell.top + cell.height() * (octant.y + octant.height) as f32 / 8.,
            scale_factor,
        ),
    }
}

fn paint_braille(pattern: u8, area: Rect, color: Hsla, window: &mut Window) {
    const POSITIONS: [(u8, u8); 8] = [
        (1, 0),
        (1, 2),
        (1, 4),
        (5, 0),
        (5, 2),
        (5, 4),
        (1, 6),
        (5, 6),
    ];
    let x_eighth = area.width() / 8.;
    let padding_y = area.height() * 0.1;
    let y_eighth = area.height() * 0.8 / 8.;
    let radius = x_eighth.min(y_eighth);
    for (bit, (x, y)) in POSITIONS.into_iter().enumerate() {
        if pattern & (1 << bit) == 0 {
            continue;
        }
        let center_x = area.left + (x + 1) as f32 * x_eighth;
        let center_y = area.top + padding_y + (y + 1) as f32 * y_eighth;
        window.paint_quad(
            fill(
                Bounds::new(
                    point(px(center_x - radius), px(center_y - radius)),
                    size(px(radius * 2.), px(radius * 2.)),
                ),
                color,
            )
            .corner_radii(px(radius)),
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_path(
    data: &'static str,
    dynamic: Option<(&'static str, &'static str)>,
    draw: Draw,
    stroke_width: f32,
    area: Rect,
    left_padding: f32,
    right_padding: f32,
    snap_coordinates: bool,
    color: Hsla,
    scale_factor: f32,
    window: &mut Window,
) {
    let transform = PathTransform {
        area,
        left_padding,
        right_padding,
        snap_coordinates,
        scale_factor,
    };
    if draw == Draw::Stroke
        && try_paint_axis_aligned_stroke(
            data,
            dynamic,
            area,
            transform,
            stroke_width,
            color,
            window,
        )
    {
        return;
    }
    let mut builder = match draw {
        Draw::Fill => PathBuilder::fill(),
        Draw::Stroke => PathBuilder::stroke(px(stroke_width)),
    };
    let mut state = PathState::default();
    for_each_resolved_token(data, dynamic, area, |command, values| {
        emit_command(&mut builder, &mut state, command, values, transform);
    });
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn for_each_resolved_token(
    data: &'static str,
    dynamic: Option<(&'static str, &'static str)>,
    area: Rect,
    mut callback: impl FnMut(u8, &[f32]),
) {
    let mut base_tokens = data.split_ascii_whitespace();
    let mut x_tokens = dynamic.map(|(x, _)| x.split_ascii_whitespace());
    let mut y_tokens = dynamic.map(|(_, y)| y.split_ascii_whitespace());
    let xp = 0.15;
    let yp = if area.height() == 0. {
        0.
    } else {
        0.15 * area.width() / area.height()
    };

    for token in &mut base_tokens {
        let mut values = [0.; 7];
        let (command, count) = parse_token(token, &mut values);
        if let (Some(x_tokens), Some(y_tokens)) = (&mut x_tokens, &mut y_tokens) {
            let x_token = x_tokens.next().expect("xterm dynamic path shape drift");
            let y_token = y_tokens.next().expect("xterm dynamic path shape drift");
            let mut x_values = [0.; 7];
            let mut y_values = [0.; 7];
            let (x_command, x_count) = parse_token(x_token, &mut x_values);
            let (y_command, y_count) = parse_token(y_token, &mut y_values);
            debug_assert_eq!((command, count), (x_command, x_count));
            debug_assert_eq!((command, count), (y_command, y_count));
            for index in 0..count {
                let base = values[index];
                values[index] =
                    base + xp * (x_values[index] - base) + yp * (y_values[index] - base);
            }
        }
        callback(command, &values[..count]);
    }
    debug_assert!(x_tokens
        .as_mut()
        .is_none_or(|tokens| tokens.next().is_none()));
    debug_assert!(y_tokens
        .as_mut()
        .is_none_or(|tokens| tokens.next().is_none()));
}

fn try_paint_axis_aligned_stroke(
    data: &'static str,
    dynamic: Option<(&'static str, &'static str)>,
    area: Rect,
    transform: PathTransform,
    stroke_width: f32,
    color: Hsla,
    window: &mut Window,
) -> bool {
    let mut current = point(px(0.), px(0.));
    let mut valid = true;
    for_each_resolved_token(data, dynamic, area, |command, values| match command {
        b'M' => current = point(transform.x(values[0]), transform.y(values[1])),
        b'L' => {
            let to = point(transform.x(values[0]), transform.y(values[1]));
            valid &= current.x == to.x || current.y == to.y;
            current = to;
        }
        b'H' => current.x = transform.x(values[0]),
        b'V' => current.y = transform.y(values[0]),
        _ => valid = false,
    });
    if !valid {
        return false;
    }

    current = point(px(0.), px(0.));
    let mut has_drawn_segment = false;
    for_each_resolved_token(data, dynamic, area, |command, values| match command {
        b'M' => {
            current = point(transform.x(values[0]), transform.y(values[1]));
            has_drawn_segment = false;
        }
        b'L' => {
            let to = point(transform.x(values[0]), transform.y(values[1]));
            paint_axis_step(
                &mut current,
                to,
                &mut has_drawn_segment,
                stroke_width,
                color,
                window,
            );
        }
        b'H' => {
            let to = point(transform.x(values[0]), current.y);
            paint_axis_step(
                &mut current,
                to,
                &mut has_drawn_segment,
                stroke_width,
                color,
                window,
            );
        }
        b'V' => {
            let to = point(current.x, transform.y(values[0]));
            paint_axis_step(
                &mut current,
                to,
                &mut has_drawn_segment,
                stroke_width,
                color,
                window,
            );
        }
        _ => unreachable!(),
    });
    true
}

fn paint_axis_step(
    current: &mut Point<Pixels>,
    to: Point<Pixels>,
    has_drawn_segment: &mut bool,
    stroke_width: f32,
    color: Hsla,
    window: &mut Window,
) {
    if *has_drawn_segment {
        window.paint_quad(fill(miter_join_bounds(*current, stroke_width), color));
    }
    paint_axis_segment(*current, to, stroke_width, color, window);
    *has_drawn_segment = *current != to;
    *current = to;
}

fn miter_join_bounds(center: Point<Pixels>, stroke_width: f32) -> Bounds<Pixels> {
    let half = px(stroke_width / 2.);
    Bounds::from_corners(
        point(center.x - half, center.y - half),
        point(center.x + half, center.y + half),
    )
}

fn paint_axis_segment(
    from: Point<Pixels>,
    to: Point<Pixels>,
    stroke_width: f32,
    color: Hsla,
    window: &mut Window,
) {
    let half = px(stroke_width / 2.);
    let bounds = if from.y == to.y {
        Bounds::from_corners(
            point(from.x.min(to.x), from.y - half),
            point(from.x.max(to.x), from.y + half),
        )
    } else {
        Bounds::from_corners(
            point(from.x - half, from.y.min(to.y)),
            point(from.x + half, from.y.max(to.y)),
        )
    };
    window.paint_quad(fill(bounds, color));
}

fn parse_token(token: &str, values: &mut [f32; 7]) -> (u8, usize) {
    let command = token.as_bytes()[0];
    if command == b'Z' {
        return (command, 0);
    }
    let mut count = 0;
    for value in token[1..].split(',') {
        values[count] = value
            .parse()
            .unwrap_or_else(|_| panic!("valid static xterm path number {value:?} in {token:?}"));
        count += 1;
    }
    (command, count)
}

#[derive(Clone, Copy)]
struct PathTransform {
    area: Rect,
    left_padding: f32,
    right_padding: f32,
    snap_coordinates: bool,
    scale_factor: f32,
}

impl PathTransform {
    fn x(self, value: f32) -> Pixels {
        let width = self.area.width() - self.left_padding - self.right_padding;
        let relative = self.axis(value, width);
        px(self.area.left + self.left_padding + relative)
    }

    fn y(self, value: f32) -> Pixels {
        px(self.area.top + self.axis(value, self.area.height()))
    }

    fn axis(self, value: f32, extent: f32) -> f32 {
        let value = value * extent;
        if !self.snap_coordinates || value == 0. {
            return value;
        }
        let device_value = value * self.scale_factor;
        ((device_value + 0.5).round() - 0.5).clamp(0., extent * self.scale_factor)
            / self.scale_factor
    }
}

#[derive(Clone, Copy)]
struct PathState {
    current: Point<Pixels>,
    quadratic_control: Point<Pixels>,
    last_command: u8,
}

impl Default for PathState {
    fn default() -> Self {
        Self {
            current: point(px(0.), px(0.)),
            quadratic_control: point(px(0.), px(0.)),
            last_command: 0,
        }
    }
}

fn emit_command(
    builder: &mut PathBuilder,
    state: &mut PathState,
    command: u8,
    values: &[f32],
    transform: PathTransform,
) {
    let xy = |x: f32, y: f32| point(transform.x(x), transform.y(y));
    match command {
        b'M' => {
            let to = xy(values[0], values[1]);
            builder.move_to(to);
            state.current = to;
        }
        b'L' => {
            let to = xy(values[0], values[1]);
            builder.line_to(to);
            state.current = to;
        }
        b'H' => {
            let to = point(transform.x(values[0]), state.current.y);
            builder.line_to(to);
            state.current = to;
        }
        b'V' => {
            let to = point(state.current.x, transform.y(values[0]));
            builder.line_to(to);
            state.current = to;
        }
        b'Q' => {
            let control = xy(values[0], values[1]);
            let to = xy(values[2], values[3]);
            builder.curve_to(to, control);
            state.current = to;
            state.quadratic_control = control;
        }
        b'T' => {
            let control = if matches!(state.last_command, b'Q' | b'T') {
                point(
                    state.current.x * 2. - state.quadratic_control.x,
                    state.current.y * 2. - state.quadratic_control.y,
                )
            } else {
                state.current
            };
            let to = xy(values[0], values[1]);
            builder.curve_to(to, control);
            state.current = to;
            state.quadratic_control = control;
        }
        b'C' => {
            let control_a = xy(values[0], values[1]);
            let control_b = xy(values[2], values[3]);
            let to = xy(values[4], values[5]);
            builder.cubic_bezier_to(to, control_a, control_b);
            state.current = to;
        }
        b'A' => {
            let to = xy(values[5], values[6]);
            builder.arc_to(
                point(
                    px(values[0] * transform.area.width()),
                    px(values[1] * transform.area.height()),
                ),
                px(values[2]),
                values[3] != 0.,
                values[4] != 0.,
                to,
            );
            state.current = to;
        }
        b'Z' => builder.close(),
        _ => panic!("unsupported static xterm path command {command}"),
    }
    state.last_command = command;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_bounds(left: f32, top: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(left), px(top)), size(px(width), px(height)))
    }

    #[test]
    fn coverage_matches_installed_xterm_table() {
        assert_eq!(data::TABLE_GLYPH_COUNT, 522);
        assert_eq!(data::TABLE_CODEPOINTS.len(), data::TABLE_GLYPH_COUNT);
        assert!(data::TABLE_CODEPOINTS
            .iter()
            .copied()
            .all(|character| glyph(character).is_some()));
        assert!((0x2500..=0x259f).all(|cp| glyph(char::from_u32(cp).unwrap()).is_some()));
        assert!((0x2800..=0x28ff).all(|cp| glyph(char::from_u32(cp).unwrap()).is_some()));
        assert!(glyph('\u{e0a4}').is_none());
        assert!(glyph('\u{1fb93}').is_none());
        assert!(glyph('A').is_none());
    }

    #[test]
    fn mapping_preserves_weights_curves_blocks_and_shades() {
        assert!(matches!(
            glyph('─'),
            Some(Glyph::Parts(&[Part::Path {
                draw: Draw::Stroke,
                stroke_width: 1.,
                ..
            }]))
        ));
        assert!(matches!(
            glyph('━'),
            Some(Glyph::Parts(&[Part::Path {
                draw: Draw::Stroke,
                stroke_width: 3.,
                ..
            }]))
        ));
        assert!(matches!(
            glyph('╭'),
            Some(Glyph::Parts(&[Part::DynamicPath { .. }]))
        ));
        assert!(matches!(
            glyph('═'),
            Some(Glyph::Parts(&[Part::DynamicPath {
                draw: Draw::Stroke,
                stroke_width: 1.,
                ..
            }]))
        ));
        assert!(matches!(
            glyph('╔'),
            Some(Glyph::Parts(&[Part::DynamicPath { .. }]))
        ));
        assert!(matches!(
            glyph('█'),
            Some(Glyph::Parts(&[Part::Octants(&[Octant {
                x: 0,
                y: 0,
                width: 8,
                height: 8
            }])]))
        ));
        assert!(matches!(
            glyph('░'),
            Some(Glyph::Parts(&[Part::Pattern { alpha: 0.25, .. }]))
        ));
        assert_eq!(resolved_stroke_width(Draw::Stroke, 1., px(14.)), 1.);
        assert_eq!(resolved_stroke_width(Draw::Stroke, 3., px(14.)), 3.);
    }

    #[test]
    fn axis_stroke_miter_fills_the_shared_corner() {
        let bounds = miter_join_bounds(point(px(12.), px(18.)), 3.);
        assert_eq!(bounds.left(), px(10.5));
        assert_eq!(bounds.top(), px(16.5));
        assert_eq!(bounds.right(), px(13.5));
        assert_eq!(bounds.bottom(), px(19.5));
    }

    #[test]
    fn full_blocks_tile_at_every_scale_and_zoom() {
        for scale_factor in [1., 2., 1.25] {
            for (cell_width, line_height) in [(8.4, 18.), (9.73, 21.4), (11.125, 24.75)] {
                let terminal = test_bounds(0.3, 0.2, 400., 300.);
                let first = Rect::from(snapped_cell_bounds(
                    terminal,
                    px(cell_width),
                    px(line_height),
                    2,
                    3,
                    scale_factor,
                ));
                let right = Rect::from(snapped_cell_bounds(
                    terminal,
                    px(cell_width),
                    px(line_height),
                    2,
                    4,
                    scale_factor,
                ));
                let below = Rect::from(snapped_cell_bounds(
                    terminal,
                    px(cell_width),
                    px(line_height),
                    3,
                    3,
                    scale_factor,
                ));
                let full = Octant::new(0, 0, 8, 8);
                let first_fill = octant_bounds(first, full, scale_factor);
                let right_fill = octant_bounds(right, full, scale_factor);
                let below_fill = octant_bounds(below, full, scale_factor);
                assert_eq!(first_fill.right, right_fill.left);
                assert_eq!(first_fill.bottom, below_fill.top);
            }
        }
    }

    #[test]
    fn powerline_fills_join_their_cell_edges_at_every_scale() {
        for scale_factor in [1., 2., 1.25] {
            let cell = Rect::from(snapped_cell_bounds(
                test_bounds(0.3, 0.2, 400., 300.),
                px(9.73),
                px(21.4),
                2,
                3,
                scale_factor,
            ));
            for (character, expected_x) in [('\u{e0b0}', cell.left), ('\u{e0b2}', cell.right)] {
                let Some(Glyph::Parts(parts)) = glyph(character) else {
                    panic!("powerline separator missing");
                };
                let Part::Path {
                    data,
                    left_padding,
                    right_padding,
                    ..
                } = parts[0]
                else {
                    panic!("powerline separator is not a path");
                };
                let padding_unit = 14. / 24.;
                let transform = PathTransform {
                    area: cell,
                    left_padding: left_padding * padding_unit,
                    right_padding: right_padding * padding_unit,
                    snap_coordinates: false,
                    scale_factor,
                };
                let mut edge_points = Vec::new();
                for_each_resolved_token(data, None, cell, |command, values| {
                    if matches!(command, b'M' | b'L') {
                        let point = (transform.x(values[0]), transform.y(values[1]));
                        if point.0 == px(expected_x) {
                            edge_points.push(point);
                        }
                    }
                });
                assert!(edge_points.contains(&(px(expected_x), px(cell.top))));
                assert!(edge_points.contains(&(px(expected_x), px(cell.bottom))));
            }
        }
    }

    #[test]
    fn braille_uses_xterm_bit_order() {
        assert!(matches!(
            glyph('\u{2801}'),
            Some(Glyph::Braille(0b0000_0001))
        ));
        assert!(matches!(
            glyph('\u{28ff}'),
            Some(Glyph::Braille(0b1111_1111))
        ));
    }

    #[test]
    fn every_static_path_is_parseable_with_supported_arities() {
        let assert_parseable =
            |character: char, data: &'static str, dynamic: Option<(&'static str, &'static str)>| {
                let mut command_count = 0;
                for_each_resolved_token(
                    data,
                    dynamic,
                    Rect {
                        left: 0.,
                        top: 0.,
                        right: 9.,
                        bottom: 18.,
                    },
                    |command, values| {
                        let expected = match command {
                            b'M' | b'L' | b'T' => 2,
                            b'H' | b'V' => 1,
                            b'Q' => 4,
                            b'C' => 6,
                            b'A' => 7,
                            b'Z' => 0,
                            _ => panic!(
                                "unsupported command in U+{:04X}: {}",
                                character as u32, command as char
                            ),
                        };
                        assert_eq!(
                            values.len(),
                            expected,
                            "wrong arity in U+{:04X} command {}",
                            character as u32,
                            command as char
                        );
                        command_count += 1;
                    },
                );
                assert!(
                    command_count > 0,
                    "empty path in U+{:04X}",
                    character as u32
                );
            };

        for character in data::TABLE_CODEPOINTS {
            let Some(Glyph::Parts(parts)) = glyph(character) else {
                unreachable!();
            };
            for part in parts {
                match part {
                    Part::Path { data, .. } | Part::NegativePath { data, .. } => {
                        assert_parseable(character, data, None)
                    }
                    Part::DynamicPath { base, x, y, .. } => {
                        assert_parseable(character, base, Some((x, y)))
                    }
                    Part::Pattern { clip, .. } if !clip.is_empty() => {
                        assert_parseable(character, clip, None)
                    }
                    Part::Octants(_) | Part::Pattern { .. } => {}
                }
            }
        }
    }
}
