use super::*;

/// A glyph's source index and its box on its visual row.
#[derive(Clone, Copy, Debug)]
struct GlyphBox {
    index: usize,
    row: usize,
    left: Pixels,
    right: Pixels,
}

/// A grapheme boundary of a line and where it shows.
#[derive(Clone, Copy, Debug)]
struct Stop {
    index: usize,
    /// Where it shows as the start of the text after it.
    after: Spot,
    /// Where it shows as the end of the text before it, which is the end of
    /// the previous row where a soft wrap breaks the line.
    before: Spot,
}

/// A line's glyphs where they are painted, split into rows at the same
/// glyphs as the painter splits them, and where each boundary shows. It
/// lays text out left to right; glyphs out of source order, as right-to-left
/// runs shape, stay in bounds and map only to grapheme boundaries.
#[derive(Debug)]
pub(super) struct LineGeometry {
    /// The right end of each visual row.
    row_ends: Vec<Pixels>,
    /// In visual order.
    glyphs: Vec<GlyphBox>,
    /// In source order.
    stops: Vec<Stop>,
}

impl LineGeometry {
    /// `glyphs` are each glyph's source index and x in the unwrapped line,
    /// in visual order; `wraps` are the glyphs that start a new row.
    pub(super) fn new(
        source: &str,
        glyphs: &[(usize, Pixels)],
        wraps: &[usize],
        width: Pixels,
    ) -> Self {
        let mut wraps = wraps.iter().peekable();
        let mut row_ends = vec![px(0.)];
        let mut row_x = px(0.);
        let mut boxes = Vec::with_capacity(glyphs.len());
        for (ordinal, &(index, x)) in glyphs.iter().enumerate() {
            if wraps.next_if(|wrap| **wrap == ordinal).is_some() {
                row_ends.push(px(0.));
                row_x = x;
            }
            let left = x - row_x;
            let right =
                (glyphs.get(ordinal + 1).map_or(width, |(_, next)| *next) - row_x).max(left);
            let row = row_ends.len() - 1;
            row_ends[row] = row_ends[row].max(right);
            boxes.push(GlyphBox {
                index,
                row,
                left,
                right,
            });
        }
        let mut by_index = (0..boxes.len()).collect::<Vec<_>>();
        by_index.sort_by_key(|ordinal| (boxes[*ordinal].index, *ordinal));
        let stops = source
            .grapheme_indices(true)
            .map(|(index, _)| index)
            .chain([source.len()])
            .map(|index| {
                let next = by_index.partition_point(|ordinal| boxes[*ordinal].index < index);
                let after = by_index.get(next).map(|ordinal| {
                    let glyph = boxes[*ordinal];
                    Spot {
                        row: glyph.row,
                        x: glyph.left,
                    }
                });
                let before = next.checked_sub(1).map(|previous| {
                    let glyph = boxes[by_index[previous]];
                    Spot {
                        row: glyph.row,
                        x: glyph.right,
                    }
                });
                Stop {
                    index,
                    after: after.or(before).unwrap_or_default(),
                    before: before.or(after).unwrap_or_default(),
                }
            })
            .collect();
        Self {
            row_ends,
            glyphs: boxes,
            stops,
        }
    }

    fn len(&self) -> usize {
        self.stops.last().map_or(0, |stop| stop.index)
    }

    pub(super) fn rows(&self) -> usize {
        self.row_ends.len()
    }

    /// Where `index` shows: after the text before it with `upstream`, else
    /// before the text after it.
    pub(super) fn spot(&self, index: usize, upstream: bool) -> Spot {
        let stop = self
            .stops
            .partition_point(|stop| stop.index < index)
            .min(self.stops.len() - 1);
        let stop = self.stops[stop];
        if upstream {
            stop.before
        } else {
            stop.after
        }
    }

    /// The index at `x` on a row: the boundary closest to it, or with
    /// `under`, the start of the grapheme it falls on. Also whether it shows
    /// there as the end of the text before it.
    pub(super) fn index_at(&self, row: usize, x: Pixels, under: bool) -> (usize, bool) {
        if !under {
            let mut closest: Option<(Pixels, usize, bool)> = None;
            for stop in &self.stops {
                for (spot, upstream) in [(stop.after, false), (stop.before, true)] {
                    if spot.row != row || (upstream && stop.before == stop.after) {
                        continue;
                    }
                    let distance = (spot.x - x).abs();
                    if closest.is_none_or(|(closest, ..)| distance < closest) {
                        closest = Some((distance, stop.index, upstream));
                    }
                }
            }
            if let Some((_, index, upstream)) = closest {
                return (index, upstream);
            }
        }
        // The grapheme holding the glyph nearest `x`; also where no boundary
        // shows on the row, as when a wrap falls inside a grapheme.
        let distance = |glyph: &GlyphBox| {
            if x < glyph.left {
                glyph.left - x
            } else if x >= glyph.right {
                x - glyph.right
            } else {
                px(0.)
            }
        };
        let index = self
            .glyphs
            .iter()
            .filter(|glyph| glyph.row == row)
            .min_by(|a, b| {
                distance(a)
                    .partial_cmp(&distance(b))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map_or(0, |glyph| glyph.index);
        let stop = self
            .stops
            .partition_point(|stop| stop.index <= index)
            .max(1)
            - 1;
        (self.stops[stop].index, false)
    }

    /// The spans a range covers on each row, as `(row, left, right)`, with a
    /// block of `newline_width` at the end of the last row for a selected
    /// line break.
    pub(super) fn spans(
        &self,
        range: Range<usize>,
        newline_width: Option<Pixels>,
    ) -> Vec<(usize, Pixels, Pixels)> {
        let mut boxes = self
            .glyphs
            .iter()
            .filter(|glyph| range.contains(&glyph.index))
            .map(|glyph| (glyph.row, glyph.left, glyph.right))
            .collect::<Vec<_>>();
        if let Some(width) = newline_width {
            let row = self.rows() - 1;
            boxes.push((row, self.row_ends[row], self.row_ends[row] + width));
        }
        boxes.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        });
        let mut spans: Vec<(usize, Pixels, Pixels)> = Vec::with_capacity(boxes.len());
        for (row, left, right) in boxes {
            match spans.last_mut() {
                Some(span) if span.0 == row && left <= span.2 => span.2 = span.2.max(right),
                _ => spans.push((row, left, right)),
            }
        }
        spans
    }
}

impl FieldLine {
    fn new(source: &str, start: usize, top: Pixels, layout: WrappedLine) -> Self {
        let unwrapped = &layout.unwrapped_layout;
        let mut run_starts = Vec::with_capacity(unwrapped.runs.len());
        let mut glyphs = Vec::new();
        for run in &unwrapped.runs {
            run_starts.push(glyphs.len());
            glyphs.extend(
                run.glyphs
                    .iter()
                    .map(|glyph| (glyph.index, glyph.position.x)),
            );
        }
        let wraps = layout
            .wrap_boundaries()
            .iter()
            .map(|boundary| run_starts[boundary.run_ix] + boundary.glyph_ix)
            .collect::<Vec<_>>();
        let geometry = LineGeometry::new(source, &glyphs, &wraps, unwrapped.width);
        Self {
            start,
            top,
            layout,
            geometry,
        }
    }

    fn end(&self) -> usize {
        self.start + self.geometry.len()
    }

    fn row_top(&self, row: usize, line_height: Pixels) -> Pixels {
        self.top + line_height * row as f32
    }
}

impl FieldText {
    pub(super) fn new(key: FieldTextKey, wrap_width: Option<Pixels>, window: &Window) -> Self {
        let text_system = window.text_system();
        let run = TextRun {
            len: key.text.len(),
            font: key.font.clone(),
            color: key.color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let mut shaped = text_system
            .shape_text(key.text.clone(), key.font_size, &[run], wrap_width, None)
            .unwrap_or_default()
            .into_iter();
        let mut lines = Vec::new();
        let mut start = 0;
        let mut top = px(0.);
        let mut width = px(0.);
        for source in key.text.split('\n') {
            let line = FieldLine::new(source, start, top, shaped.next().unwrap_or_default());
            start += source.len() + 1;
            top += key.line_height * line.geometry.rows() as f32;
            width = width.max(line.layout.width());
            lines.push(line);
        }
        let font_id = text_system.resolve_font(&key.font);
        let newline_width = text_system
            .advance(font_id, key.font_size, ' ')
            .map_or(key.font_size / 2., |advance| advance.width);
        Self {
            len: key.text.len(),
            size: size(wrap_width.unwrap_or(width), top),
            key,
            wrap_width,
            lines,
            newline_width,
        }
    }

    pub(super) fn line_height(&self) -> Pixels {
        self.key.line_height
    }

    fn line_index(&self, index: usize) -> usize {
        self.lines
            .partition_point(|line| line.start <= index)
            .max(1)
            - 1
    }

    /// The top left of the caret at `index`, relative to the text's origin.
    pub(super) fn caret_position(&self, index: usize, upstream: bool) -> Point<Pixels> {
        if self.key.placeholder {
            return Point::default();
        }
        let line = &self.lines[self.line_index(index)];
        let spot = line
            .geometry
            .spot(index.saturating_sub(line.start), upstream);
        point(spot.x, line.row_top(spot.row, self.line_height()))
    }

    /// The index at a point relative to the text's origin, and whether it
    /// shows there as the end of the text before it. Above the text is its
    /// start, below it its end; a single-line field reads only `x`.
    pub(super) fn index_for_position(&self, position: Point<Pixels>, under: bool) -> (usize, bool) {
        if self.key.placeholder {
            return (0, false);
        }
        let (line, row) = if self.key.multiline {
            if position.y < px(0.) {
                return (0, false);
            }
            let line = &self.lines[self
                .lines
                .partition_point(|line| line.top <= position.y)
                .max(1)
                - 1];
            let row = ((position.y - line.top) / self.line_height()).floor() as usize;
            if row >= line.geometry.rows() {
                return (self.len, false);
            }
            (line, row)
        } else {
            (&self.lines[0], 0)
        };
        let (index, upstream) = line.geometry.index_at(row, position.x, under);
        (line.start + index, upstream)
    }

    /// Where Up or Down from `index` lands, keeping `goal_x` (or the caret's
    /// own x), and whether it shows as the end of the text before it. From
    /// the first row Up goes to the start, from the last Down to the end.
    pub(super) fn vertical_target(
        &self,
        index: usize,
        upstream: bool,
        up: bool,
        goal_x: Option<Pixels>,
    ) -> (usize, bool, Pixels) {
        if self.key.placeholder {
            return (0, false, px(0.));
        }
        let line_ix = self.line_index(index);
        let line = &self.lines[line_ix];
        let spot = line
            .geometry
            .spot(index.saturating_sub(line.start), upstream);
        let x = goal_x.unwrap_or(spot.x);
        let target = if up {
            if spot.row > 0 {
                Some((line_ix, spot.row - 1))
            } else {
                line_ix
                    .checked_sub(1)
                    .map(|previous| (previous, self.lines[previous].geometry.rows() - 1))
            }
        } else if spot.row + 1 < line.geometry.rows() {
            Some((line_ix, spot.row + 1))
        } else {
            (line_ix + 1 < self.lines.len()).then_some((line_ix + 1, 0))
        };
        match target {
            Some((line_ix, row)) => {
                let line = &self.lines[line_ix];
                let (index, upstream) = line.geometry.index_at(row, x, false);
                (line.start + index, upstream, x)
            }
            None => (if up { 0 } else { self.len }, false, x),
        }
    }

    /// The rectangles a range covers, one per run of glyphs on a visual row,
    /// relative to the text's origin. With `newline_blocks`, a selected line
    /// break adds a block at the end of its line, so empty lines show as
    /// selected.
    pub(super) fn range_rects(
        &self,
        range: Range<usize>,
        newline_blocks: bool,
    ) -> Vec<Bounds<Pixels>> {
        let mut rects = Vec::new();
        if self.key.placeholder || range.is_empty() {
            return rects;
        }
        let line_height = self.line_height();
        for (line_ix, line) in self
            .lines
            .iter()
            .enumerate()
            .skip(self.line_index(range.start))
        {
            if line.start >= range.end {
                break;
            }
            let local = range.start.saturating_sub(line.start)..range.end - line.start;
            let newline =
                newline_blocks && range.end > line.end() && line_ix + 1 < self.lines.len();
            for (row, left, right) in line
                .geometry
                .spans(local, newline.then_some(self.newline_width))
            {
                let top = line.row_top(row, line_height);
                rects.push(Bounds::from_corners(
                    point(left, top),
                    point(right, top + line_height),
                ));
            }
        }
        rects
    }

    /// The bounds of a range's first span, or of the caret for an empty
    /// range.
    pub(super) fn range_bounds(&self, range: Range<usize>, upstream: bool) -> Bounds<Pixels> {
        self.range_rects(range.clone(), false)
            .into_iter()
            .next()
            .unwrap_or_else(|| {
                Bounds::new(
                    self.caret_position(range.start, upstream),
                    size(px(0.), self.line_height()),
                )
            })
    }
}

impl FieldTextState {
    pub(super) fn shape(
        &mut self,
        key: &FieldTextKey,
        wrap_width: Option<Pixels>,
        window: &Window,
    ) -> &FieldText {
        let reusable = self.shaped.as_ref().is_some_and(|shaped| {
            shaped.key == *key
                && match (shaped.wrap_width, wrap_width) {
                    // Layout rounds to device pixels, so a shape measured at
                    // the unrounded width is the one the bounds carry.
                    (Some(shaped), Some(width)) => (shaped - width).abs() < px(1.),
                    (shaped, width) => shaped == width,
                }
        });
        if !reusable {
            self.shaped = Some(FieldText::new(key.clone(), wrap_width, window));
        }
        self.shaped.as_ref().expect("shaped above")
    }
}
