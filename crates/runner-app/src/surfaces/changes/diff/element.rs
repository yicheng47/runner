use super::*;

pub(super) struct UnifiedDiffElement {
    pub(super) view: Entity<UnifiedDiffView>,
}

pub(super) struct DiffPrepaint {
    /// Where text is selected; the wheel also scrolls over the lanes.
    text_hitbox: Hitbox,
    scroll_hitbox: Hitbox,
    metrics: Metrics,
    scroll: Point<Pixels>,
    focused: bool,
    selection_side: DiffSide,
    rows: ShapedRows,
    selections: Vec<Bounds<Pixels>>,
}

impl Element for UnifiedDiffElement {
    type RequestLayoutState = ();
    type PrepaintState = DiffPrepaint;

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
    ) -> DiffPrepaint {
        let (font, font_size) = CodeView::font(cx);
        let text_system = window.text_system();
        let font_id = text_system.resolve_font(&font);
        let char_width = text_system
            .em_advance(font_id, font_size)
            .unwrap_or(font_size * 0.6);
        let line_height = (font_size * crate::app_settings::TERMINAL_LINE_HEIGHT_DEFAULT).round();
        let num_font_size = (font_size * 0.9).max(px(10.));

        let (mode, old_digits, new_digits, focused, selection_side) = {
            let v = self.view.read(cx);
            (
                v.mode,
                v.old_digits,
                v.new_digits,
                v.focus.is_focused(window),
                v.selection_side,
            )
        };
        let old_col_w = char_width * old_digits as f32;
        let new_col_w = char_width * new_digits as f32;

        let old_num_right = px(GUTTER_PAD_LEFT) + old_col_w;
        let (new_num_right, sign_left, text_left) = match mode {
            DiffViewMode::Unified => {
                let new_num_right = old_num_right + px(COL_GAP) + new_col_w;
                let sign_left = new_num_right + px(COL_GAP);
                let text_left = sign_left + px(SIGN_WIDTH + TEXT_GAP);
                (new_num_right, sign_left, text_left)
            }
            DiffViewMode::Split => {
                let sign_left = old_num_right + px(COL_GAP);
                let text_left = sign_left + px(SIGN_WIDTH + TEXT_GAP);
                (px(0.), sign_left, text_left)
            }
        };

        let left_col_width = ((bounds.size.width - px(1.)) * 0.5).floor().max(px(0.));
        let right_col_left = left_col_width + px(1.);
        let right_col_width = (bounds.size.width - right_col_left).max(px(0.));
        let right_num_right = px(GUTTER_PAD_LEFT) + new_col_w;
        let right_sign_left = right_num_right + px(COL_GAP);
        let right_text_left = right_sign_left + px(SIGN_WIDTH + TEXT_GAP);

        let mut metrics = Metrics {
            mode,
            bounds,
            line_height,
            char_width,
            old_num_right,
            new_num_right,
            sign_left,
            text_left,
            left_col_width,
            right_col_left,
            right_col_width,
            right_num_right,
            right_sign_left,
            right_text_left,
            lanes: ScrollbarLanes::default(),
        };
        let view = self.view.read(cx);
        metrics.lanes = ScrollbarLanes::measure(
            view.content_height(&metrics),
            bounds.size.height,
            app_scrollbar_gutter().to_pixels(window.rem_size()),
            |right| {
                let lanes = ScrollbarLanes {
                    right,
                    bottom: px(0.),
                };
                view.overflow_x(&Metrics { lanes, ..metrics })
            },
        );
        let viewport = metrics.viewport();
        let scroll_hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        let text_hitbox = window.insert_hitbox(viewport, HitboxBehavior::Normal);

        self.view.update(cx, |view, cx| {
            let previous = view.metrics.replace(Some(metrics));
            // The bars and the footer are laid out from the last frame's lanes.
            if previous.map(|metrics| metrics.lanes) != Some(metrics.lanes) {
                cx.notify();
            }
            view.apply_reveal(&metrics);
            view.clamp_scroll(&metrics);
            view.measure_scrollbars(&metrics);
        });

        let view = self.view.read(cx);
        let scroll = view.scroll;
        let top = bounds.top() + px(TOP_PAD) - scroll.y;
        let first = (f32::from(scroll.y - px(TOP_PAD)) / f32::from(line_height))
            .floor()
            .max(0.) as usize;
        let shown_lines =
            (f32::from(viewport.size.height) / f32::from(line_height)).ceil() as usize + 1;
        let last = (first + shown_lines).min(view.active_row_count());

        let visible = metrics.shape_viewport_width();
        let from = (f32::from(scroll.x) / f32::from(char_width)).floor() as usize;
        let to = from + (f32::from(visible) / f32::from(char_width)).ceil() as usize + 2;

        let light = theme::active_variant().is_light();
        let add_bg = theme::with_alpha(theme::accent(), if light { 0.12 } else { 0.10 });
        let del_bg = theme::with_alpha(theme::danger(), if light { 0.12 } else { 0.13 });
        let hunk_bg = theme::panel();
        let filler_bg = theme::panel();
        let number_color = theme::faint();
        let text_color = theme::text();

        let syntax = (view.old_highlights.is_some() || view.new_highlights.is_some())
            .then(|| SyntaxTheme::current(cx));
        let old_style = RowStyle {
            font: &font,
            font_size,
            char_width,
            color: text_color,
            syntax: view.old_highlights.as_deref().zip(syntax.as_ref()),
        };
        let new_style = RowStyle {
            font: &font,
            font_size,
            char_width,
            color: text_color,
            syntax: view.new_highlights.as_deref().zip(syntax.as_ref()),
        };

        let rows = match mode {
            DiffViewMode::Unified => {
                let mut shaped_rows = Vec::with_capacity(last.saturating_sub(first));
                for row_index in first..last {
                    let cursor = row_index == view.cursor_row;
                    match &view.rows[row_index] {
                        DiffRow::Hunk { header } => {
                            let hunk_line = Some(shape_plain(
                                header.clone(),
                                &font,
                                num_font_size,
                                number_color,
                                window,
                            ));
                            shaped_rows.push(ShapedUnifiedRow {
                                row_index,
                                bg: Some(hunk_bg),
                                cursor,
                                old_num: None,
                                new_num: None,
                                sign: None,
                                segments: Vec::new(),
                                hunk_line,
                                #[cfg(test)]
                                painted: Vec::new(),
                            });
                        }
                        DiffRow::Context { old_line, new_line } => {
                            let shaped = shape_doc_line(
                                &view.new_doc,
                                *new_line,
                                (from, to),
                                &new_style,
                                window,
                            );
                            let old_num = Some(shape_plain(
                                (*old_line + 1).to_string().into(),
                                &font,
                                num_font_size,
                                number_color,
                                window,
                            ));
                            let new_num = Some(shape_plain(
                                (*new_line + 1).to_string().into(),
                                &font,
                                num_font_size,
                                number_color,
                                window,
                            ));
                            shaped_rows.push(ShapedUnifiedRow {
                                row_index,
                                bg: None,
                                cursor,
                                old_num,
                                new_num,
                                sign: None,
                                segments: shaped.segments,
                                hunk_line: None,
                                #[cfg(test)]
                                painted: shaped.painted,
                            });
                        }
                        DiffRow::Removed { old_line } => {
                            let shaped = shape_doc_line(
                                &view.old_doc,
                                *old_line,
                                (from, to),
                                &old_style,
                                window,
                            );
                            let old_num = Some(shape_plain(
                                (*old_line + 1).to_string().into(),
                                &font,
                                num_font_size,
                                number_color,
                                window,
                            ));
                            let sign = Some(shape_plain(
                                "−".into(),
                                &font,
                                font_size,
                                theme::danger(),
                                window,
                            ));
                            shaped_rows.push(ShapedUnifiedRow {
                                row_index,
                                bg: Some(del_bg),
                                cursor,
                                old_num,
                                new_num: None,
                                sign,
                                segments: shaped.segments,
                                hunk_line: None,
                                #[cfg(test)]
                                painted: shaped.painted,
                            });
                        }
                        DiffRow::Added { new_line } => {
                            let shaped = shape_doc_line(
                                &view.new_doc,
                                *new_line,
                                (from, to),
                                &new_style,
                                window,
                            );
                            let new_num = Some(shape_plain(
                                (*new_line + 1).to_string().into(),
                                &font,
                                num_font_size,
                                number_color,
                                window,
                            ));
                            let sign = Some(shape_plain(
                                "+".into(),
                                &font,
                                font_size,
                                theme::accent(),
                                window,
                            ));
                            shaped_rows.push(ShapedUnifiedRow {
                                row_index,
                                bg: Some(add_bg),
                                cursor,
                                old_num: None,
                                new_num,
                                sign,
                                segments: shaped.segments,
                                hunk_line: None,
                                #[cfg(test)]
                                painted: shaped.painted,
                            });
                        }
                    }
                }
                #[cfg(test)]
                view.painted.replace(
                    shaped_rows
                        .iter()
                        .map(|r| (r.row_index, r.painted.clone()))
                        .collect(),
                );
                ShapedRows::Unified(shaped_rows)
            }
            DiffViewMode::Split => {
                let mut shaped_rows = Vec::with_capacity(last.saturating_sub(first));
                #[cfg(test)]
                let mut test_painted = std::collections::BTreeMap::new();
                for row_index in first..last {
                    let cursor = row_index == view.cursor_row;
                    match &view.split_rows[row_index] {
                        SplitRow::Hunk { header } => {
                            let hunk_line = shape_plain(
                                header.clone(),
                                &font,
                                num_font_size,
                                number_color,
                                window,
                            );
                            #[cfg(test)]
                            test_painted.insert(row_index, Vec::new());
                            shaped_rows.push(ShapedSplitRow {
                                row_index,
                                cursor,
                                kind: ShapedSplitRowKind::Hunk(hunk_line),
                            });
                        }
                        SplitRow::Pair { left, right } => {
                            let left_shaped = shape_split_cell(
                                *left,
                                &view.old_doc,
                                &old_style,
                                (from, to),
                                &font,
                                font_size,
                                num_font_size,
                                number_color,
                                add_bg,
                                del_bg,
                                filler_bg,
                                window,
                            );
                            let right_shaped = shape_split_cell(
                                *right,
                                &view.new_doc,
                                &new_style,
                                (from, to),
                                &font,
                                font_size,
                                num_font_size,
                                number_color,
                                add_bg,
                                del_bg,
                                filler_bg,
                                window,
                            );
                            #[cfg(test)]
                            {
                                let mut row_tokens = left_shaped.painted.clone();
                                row_tokens.extend(right_shaped.painted.iter().cloned());
                                test_painted.insert(row_index, row_tokens);
                            }
                            shaped_rows.push(ShapedSplitRow {
                                row_index,
                                cursor,
                                kind: ShapedSplitRowKind::Pair {
                                    left: left_shaped,
                                    right: right_shaped,
                                },
                            });
                        }
                    }
                }
                #[cfg(test)]
                view.painted.replace(test_painted);
                ShapedRows::Split(shaped_rows)
            }
        };

        let text_x = metrics.side_text_left_abs(selection_side) - scroll.x;
        let row_top = |r: usize| top + line_height * r as f32;
        let mut selections = Vec::new();
        if let Some((anchor, head)) = view.selection.filter(|(a, b)| a != b || view.selects_all) {
            let (start, end) = (anchor.min(head), anchor.max(head));
            let vis_start = start.row.max(first);
            let vis_end = end.row.min(last.saturating_sub(1));
            if vis_start <= vis_end && last > first {
                for row_index in vis_start..=vis_end {
                    let Some((doc, doc_line)) =
                        view.row_doc_and_line_for_side(row_index, selection_side)
                    else {
                        continue;
                    };
                    let line_str = doc.line(doc_line);
                    let s_off = if row_index == start.row {
                        start.offset.min(line_str.len())
                    } else {
                        0
                    };
                    let e_off = if row_index == end.row {
                        end.offset.min(line_str.len())
                    } else {
                        line_str.len()
                    };
                    let left = text_x + char_width * doc.column_at(doc_line, s_off) as f32;
                    let right = text_x + char_width * doc.column_at(doc_line, e_off) as f32;
                    let mut sel_bounds = Bounds::new(
                        point(left, row_top(row_index)),
                        size((right - left).max(px(0.)), line_height),
                    );
                    if row_index != end.row {
                        sel_bounds.size.width += char_width / 2.;
                    }
                    selections.push(sel_bounds);
                }
            }
        }

        DiffPrepaint {
            text_hitbox,
            scroll_hitbox,
            metrics,
            scroll,
            focused,
            selection_side,
            rows,
            selections,
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _state: &mut (),
        prepaint: &mut DiffPrepaint,
        window: &mut Window,
        cx: &mut App,
    ) {
        let metrics = prepaint.metrics;
        let viewport = metrics.viewport();
        // Row colours run under the right lane to the edge.
        let row_area = Bounds::new(bounds.origin, size(bounds.size.width, viewport.size.height));
        window.set_cursor_style(CursorStyle::IBeam, &prepaint.text_hitbox);
        let top = bounds.top() + px(TOP_PAD) - prepaint.scroll.y;
        let cursor_tint =
            theme::with_alpha(theme::info(), if prepaint.focused { 0.14 } else { 0.06 });
        let selection_color = theme::with_alpha(theme::accent(), 0.28);

        match &prepaint.rows {
            ShapedRows::Unified(rows) => {
                window.with_content_mask(Some(ContentMask { bounds: row_area }), |window| {
                    for row in rows {
                        let y = top + metrics.line_height * row.row_index as f32;
                        let row_bounds = Bounds::new(
                            point(bounds.left(), y),
                            size(bounds.size.width, metrics.line_height),
                        );
                        if let Some(bg) = row.bg {
                            window.paint_quad(fill(row_bounds, bg));
                        }
                        if row.cursor {
                            window.paint_quad(fill(row_bounds, cursor_tint));
                            if prepaint.focused {
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(bounds.left(), y),
                                        size(px(2.), metrics.line_height),
                                    ),
                                    theme::info(),
                                ));
                            }
                        }
                    }
                });

                let text_bounds = Bounds::new(
                    point(bounds.left() + metrics.text_left, bounds.top()),
                    size(metrics.scroll_viewport_width(), viewport.size.height),
                );
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: text_bounds,
                    }),
                    |window| {
                        for sel in &prepaint.selections {
                            window.paint_quad(fill(*sel, selection_color));
                        }
                        let text_x = bounds.left() + metrics.text_left - prepaint.scroll.x;
                        for row in rows {
                            let y = top + metrics.line_height * row.row_index as f32;
                            for (column, shaped) in &row.segments {
                                let origin = point(text_x + metrics.char_width * *column as f32, y);
                                let _ = shaped.paint(
                                    origin,
                                    metrics.line_height,
                                    TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                );
                            }
                        }
                    },
                );

                window.with_content_mask(Some(ContentMask { bounds: viewport }), |window| {
                    for row in rows {
                        let y = top + metrics.line_height * row.row_index as f32;
                        if let Some(hunk) = &row.hunk_line {
                            let _ = hunk.paint(
                                point(bounds.left() + px(10.) - prepaint.scroll.x.min(px(0.)), y),
                                metrics.line_height,
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                            continue;
                        }
                        if let Some(old_num) = &row.old_num {
                            let x = bounds.left() + metrics.old_num_right - old_num.width;
                            let _ = old_num.paint(
                                point(x, y),
                                metrics.line_height,
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                        if let Some(new_num) = &row.new_num {
                            let x = bounds.left() + metrics.new_num_right - new_num.width;
                            let _ = new_num.paint(
                                point(x, y),
                                metrics.line_height,
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                        if let Some(sign) = &row.sign {
                            let x = bounds.left() + metrics.sign_left;
                            let _ = sign.paint(
                                point(x, y),
                                metrics.line_height,
                                TextAlign::Left,
                                None,
                                window,
                                cx,
                            );
                        }
                    }
                });
            }
            ShapedRows::Split(rows) => {
                let hunk_bg = theme::panel();
                let border_color = theme::border();

                // 1. Backgrounds, center divider on code rows, cursor tint, and hunk text.
                window.with_content_mask(Some(ContentMask { bounds: row_area }), |window| {
                    for row in rows {
                        let y = top + metrics.line_height * row.row_index as f32;
                        let row_bounds = Bounds::new(
                            point(bounds.left(), y),
                            size(bounds.size.width, metrics.line_height),
                        );
                        match &row.kind {
                            ShapedSplitRowKind::Hunk(hunk) => {
                                window.paint_quad(fill(row_bounds, hunk_bg));
                                if row.cursor {
                                    window.paint_quad(fill(row_bounds, cursor_tint));
                                    if prepaint.focused {
                                        window.paint_quad(fill(
                                            Bounds::new(
                                                point(bounds.left(), y),
                                                size(px(2.), metrics.line_height),
                                            ),
                                            theme::info(),
                                        ));
                                    }
                                }
                                window.with_content_mask(
                                    Some(ContentMask { bounds: viewport }),
                                    |window| {
                                        let _ = hunk.paint(
                                            point(
                                                bounds.left() + px(10.)
                                                    - prepaint.scroll.x.min(px(0.)),
                                                y,
                                            ),
                                            metrics.line_height,
                                            TextAlign::Left,
                                            None,
                                            window,
                                            cx,
                                        );
                                    },
                                );
                            }
                            ShapedSplitRowKind::Pair { left, right } => {
                                if let Some(bg) = left.bg {
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(bounds.left(), y),
                                            size(metrics.left_col_width, metrics.line_height),
                                        ),
                                        bg,
                                    ));
                                }
                                if let Some(bg) = right.bg {
                                    window.paint_quad(fill(
                                        Bounds::new(
                                            point(bounds.left() + metrics.right_col_left, y),
                                            size(metrics.right_col_width, metrics.line_height),
                                        ),
                                        bg,
                                    ));
                                }
                                window.paint_quad(fill(
                                    Bounds::new(
                                        point(bounds.left() + metrics.left_col_width, y),
                                        size(px(1.), metrics.line_height),
                                    ),
                                    border_color,
                                ));
                                if row.cursor {
                                    window.paint_quad(fill(row_bounds, cursor_tint));
                                    if prepaint.focused {
                                        window.paint_quad(fill(
                                            Bounds::new(
                                                point(bounds.left(), y),
                                                size(px(2.), metrics.line_height),
                                            ),
                                            theme::info(),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                });

                // 2. Left column text & selection (clipped to left text area).
                let left_text_bounds = Bounds::new(
                    point(bounds.left() + metrics.text_left, bounds.top()),
                    size(metrics.left_text_width(), viewport.size.height),
                );
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: left_text_bounds,
                    }),
                    |window| {
                        if prepaint.selection_side == DiffSide::Left {
                            for sel in &prepaint.selections {
                                window.paint_quad(fill(*sel, selection_color));
                            }
                        }
                        let left_text_x = bounds.left() + metrics.text_left - prepaint.scroll.x;
                        for row in rows {
                            if let ShapedSplitRowKind::Pair { left, .. } = &row.kind {
                                let y = top + metrics.line_height * row.row_index as f32;
                                for (column, shaped) in &left.segments {
                                    let origin =
                                        point(left_text_x + metrics.char_width * *column as f32, y);
                                    let _ = shaped.paint(
                                        origin,
                                        metrics.line_height,
                                        TextAlign::Left,
                                        None,
                                        window,
                                        cx,
                                    );
                                }
                            }
                        }
                    },
                );

                // 3. Right column text & selection (clipped to right text area).
                let right_text_bounds = Bounds::new(
                    point(
                        bounds.left() + metrics.right_col_left + metrics.right_text_left,
                        bounds.top(),
                    ),
                    size(metrics.right_text_width(), viewport.size.height),
                );
                window.with_content_mask(
                    Some(ContentMask {
                        bounds: right_text_bounds,
                    }),
                    |window| {
                        if prepaint.selection_side == DiffSide::Right {
                            for sel in &prepaint.selections {
                                window.paint_quad(fill(*sel, selection_color));
                            }
                        }
                        let right_text_x =
                            bounds.left() + metrics.right_col_left + metrics.right_text_left
                                - prepaint.scroll.x;
                        for row in rows {
                            if let ShapedSplitRowKind::Pair { right, .. } = &row.kind {
                                let y = top + metrics.line_height * row.row_index as f32;
                                for (column, shaped) in &right.segments {
                                    let origin = point(
                                        right_text_x + metrics.char_width * *column as f32,
                                        y,
                                    );
                                    let _ = shaped.paint(
                                        origin,
                                        metrics.line_height,
                                        TextAlign::Left,
                                        None,
                                        window,
                                        cx,
                                    );
                                }
                            }
                        }
                    },
                );

                // 4. Left & right gutters (line numbers and −/+ signs).
                window.with_content_mask(Some(ContentMask { bounds: viewport }), |window| {
                    for row in rows {
                        if let ShapedSplitRowKind::Pair { left, right } = &row.kind {
                            let y = top + metrics.line_height * row.row_index as f32;
                            if let Some(num) = &left.num {
                                let x = bounds.left() + metrics.old_num_right - num.width;
                                let _ = num.paint(
                                    point(x, y),
                                    metrics.line_height,
                                    TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                );
                            }
                            if let Some(sign) = &left.sign {
                                let x = bounds.left() + metrics.sign_left;
                                let _ = sign.paint(
                                    point(x, y),
                                    metrics.line_height,
                                    TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                );
                            }
                            if let Some(num) = &right.num {
                                let x = bounds.left()
                                    + metrics.right_col_left
                                    + metrics.right_num_right
                                    - num.width;
                                let _ = num.paint(
                                    point(x, y),
                                    metrics.line_height,
                                    TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                );
                            }
                            if let Some(sign) = &right.sign {
                                let x = bounds.left()
                                    + metrics.right_col_left
                                    + metrics.right_sign_left;
                                let _ = sign.paint(
                                    point(x, y),
                                    metrics.line_height,
                                    TextAlign::Left,
                                    None,
                                    window,
                                    cx,
                                );
                            }
                        }
                    }
                });
            }
        }

        register_mouse(
            self.view.clone(),
            prepaint.text_hitbox.clone(),
            prepaint.scroll_hitbox.clone(),
            metrics,
            window,
        );
    }
}

fn position_at(
    view: &UnifiedDiffView,
    metrics: &Metrics,
    at: Point<Pixels>,
    side: DiffSide,
) -> DiffPos {
    let count = view.active_row_count();
    if count == 0 {
        return DiffPos::default();
    }
    let y = at.y - metrics.bounds.top() - px(TOP_PAD) + view.scroll.y;
    let row_f = (f32::from(y) / f32::from(metrics.line_height)).floor();
    if row_f < 0. {
        return DiffPos { row: 0, offset: 0 };
    }
    let row = (row_f as usize).min(count - 1);
    let Some((doc, doc_line)) = view.row_doc_and_line_for_side(row, side) else {
        return DiffPos { row, offset: 0 };
    };
    let x = at.x - metrics.side_text_left_abs(side) + view.scroll.x;
    let column = f32::from(x.max(px(0.))) / f32::from(metrics.char_width);
    DiffPos {
        row,
        offset: doc.offset_near(doc_line, column),
    }
}

fn register_mouse(
    view: Entity<UnifiedDiffView>,
    text_hitbox: Hitbox,
    scroll_hitbox: Hitbox,
    metrics: Metrics,
    window: &mut Window,
) {
    let scroll_view = view.clone();
    window.on_mouse_event(move |event: &ScrollEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble || !scroll_hitbox.should_handle_scroll(window) {
            return;
        }
        let mut delta = event.delta.pixel_delta(metrics.line_height);
        if event.modifiers.shift && delta.x == px(0.) {
            delta = point(delta.y, px(0.));
        }
        scroll_view.update(cx, |view, cx| view.scroll_by(delta, cx));
        cx.stop_propagation();
    });

    let down_view = view.clone();
    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
        if phase != DispatchPhase::Bubble
            || event.button != MouseButton::Left
            || !text_hitbox.is_hovered(window)
        {
            return;
        }
        let mut focus_cb = None;
        down_view.update(cx, |view, cx| {
            view.focus.focus(window, cx);
            focus_cb = view.on_focus_in.clone();
            let keep_side = event.modifiers.shift && view.selection.is_some();
            if metrics.mode == DiffViewMode::Split && !keep_side {
                view.selection_side = metrics.side_at_x(event.position.x);
            }
            let side = view.selection_side;
            let at = position_at(view, &metrics, event.position, side);
            view.cursor_row = at.row;
            view.selects_all = false;
            match event.click_count {
                2 => {
                    if let Some(line) = view.row_line_text(at.row) {
                        let (start, end) = word_at(line, at.offset);
                        view.selection = Some((
                            DiffPos {
                                row: at.row,
                                offset: start,
                            },
                            DiffPos {
                                row: at.row,
                                offset: end,
                            },
                        ));
                    }
                }
                count if count >= 3 => {
                    let end = view.row_line_text(at.row).map_or(0, str::len);
                    view.selection = Some((
                        DiffPos {
                            row: at.row,
                            offset: 0,
                        },
                        DiffPos {
                            row: at.row,
                            offset: end,
                        },
                    ));
                }
                _ => {
                    let anchor = match view.selection {
                        Some((anchor, _)) if event.modifiers.shift => anchor,
                        _ => at,
                    };
                    view.selection = Some((anchor, at));
                    view.dragging = true;
                }
            }
            cx.notify();
        });
        if let Some(cb) = focus_cb {
            cb(window, cx);
        }
    });

    let move_view = view.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
        if phase != DispatchPhase::Capture
            || event.pressed_button != Some(MouseButton::Left)
            || !move_view.read(cx).dragging
        {
            return;
        }
        move_view.update(cx, |view, cx| {
            let bounds = metrics.viewport();
            let side = view.selection_side;
            let mut delta = Point::default();
            if event.position.y < bounds.top() {
                delta.y = bounds.top() - event.position.y;
            } else if event.position.y > bounds.bottom() {
                delta.y = bounds.bottom() - event.position.y;
            }
            let text_left = metrics.side_text_left_abs(side);
            let side_right = metrics.side_right_abs(side);
            if event.position.x < text_left {
                delta.x = text_left - event.position.x;
            } else if event.position.x > side_right {
                delta.x = side_right - event.position.x;
            }
            if delta != Point::default() {
                view.scroll_by(delta, cx);
            }
            let head = position_at(view, &metrics, event.position, side);
            if let Some((anchor, _)) = view.selection {
                view.selection = Some((anchor, head));
            }
            view.cursor_row = head.row;
            cx.notify();
        });
    });

    let up_view = view;
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
        if phase != DispatchPhase::Capture || event.button != MouseButton::Left {
            return;
        }
        up_view.update(cx, |view, _| view.dragging = false);
    });
}
