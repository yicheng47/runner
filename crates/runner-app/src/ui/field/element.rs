use super::*;

impl IntoElement for TextFieldElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextFieldElement {
    type RequestLayoutState = FieldTextKey;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
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
    ) -> (LayoutId, Self::RequestLayoutState) {
        let field = self.field.read(cx);
        let key = field.text_key(window);
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        if !key.multiline {
            style.size.height = key.line_height.into();
            return (window.request_layout(style, [], cx), key);
        }
        let state = Rc::clone(&field.text_state);
        let measured = key.clone();
        let layout_id =
            window.request_measured_layout(style, move |known, available, window, _| {
                let width = known.width.or(match available.width {
                    AvailableSpace::Definite(width) => Some(width),
                    _ => None,
                });
                match width {
                    Some(width) => {
                        state
                            .borrow_mut()
                            .shape(&measured, Some(width), window)
                            .size
                    }
                    None => size(
                        px(0.),
                        measured.line_height * (measured.text.matches('\n').count() + 1) as f32,
                    ),
                }
            });
        (layout_id, key)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        key: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let field = self.field.read(cx);
        let caret = field.buffer.selection.caret;
        let row_end = field.caret_row_end == Some(caret);
        let scroll_handle = field.scroll_handle.clone();
        let mut state = field.text_state.borrow_mut();
        let caret_top = state
            .shape(key, key.multiline.then_some(bounds.size.width), window)
            .caret_position(caret, row_end)
            .y;
        let mut origin = bounds.origin;
        let mut offset = if key.multiline {
            scroll_handle.offset()
        } else {
            Point::default()
        };
        if key.multiline && std::mem::take(&mut state.reveal_caret) {
            let top = origin.y + caret_top;
            let bottom = top + key.line_height;
            let viewport = scroll_handle.bounds();
            let delta = if top < viewport.top() {
                viewport.top() - top
            } else if bottom > viewport.bottom() {
                viewport.bottom() - bottom
            } else {
                px(0.)
            };
            if delta != px(0.) {
                let max = scroll_handle.max_offset().y;
                let y = (offset.y + delta).clamp(-max, px(0.));
                origin.y += y - offset.y;
                offset.y = y;
                scroll_handle.set_offset(offset);
            }
        }
        state.origin = Some(origin);
        state.scroll_offset = offset;
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _key: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let field = self.field.read(cx);
        let focus_handle = field.focus_handle.clone();
        let focused = focus_handle.is_focused(window);
        let disabled = field.disabled;
        let selection = field.buffer.selection;
        let row_end = field.caret_row_end == Some(selection.caret);
        let marked = field
            .buffer
            .marked
            .as_ref()
            .map(|marked| marked.range.clone());
        let text_state = Rc::clone(&field.text_state);
        let state = text_state.borrow();
        let (Some(shaped), Some(origin)) = (state.shaped.as_ref(), state.origin) else {
            return;
        };
        let content = Bounds::new(origin, bounds.size);
        if !disabled {
            window.handle_input(
                &focus_handle,
                ElementInputHandler::new(content, self.field.clone()),
                cx,
            );
            let field = self.field.clone();
            window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
                if phase == DispatchPhase::Capture && field.read(cx).selecting {
                    field.update(cx, |field, cx| field.on_drag_move(event, window, cx));
                }
            });
        }

        let line_height = shaped.line_height();
        if focused {
            for rect in shaped.range_rects(selection.range(), true) {
                window.paint_quad(fill(
                    rect + origin,
                    theme::with_alpha(theme::accent(), 0.267),
                ));
            }
        }
        let visible = window.content_mask().bounds;
        // A one-line placeholder clips at the content edge, clear of a
        // trailing control such as a suggestions chevron.
        let clip =
            (!shaped.key.multiline && shaped.key.placeholder).then_some(ContentMask { bounds });
        window.with_content_mask(clip, |window| {
            for line in &shaped.lines {
                let top = origin.y + line.top;
                let bottom = top + line_height * line.geometry.rows() as f32;
                if bottom < visible.top() || top > visible.bottom() {
                    continue;
                }
                let _ = line.layout.paint(
                    point(origin.x, top),
                    line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
        });
        if !focused {
            return;
        }
        if let Some(marked) = marked {
            for rect in shaped.range_rects(marked, false) {
                window.paint_quad(fill(
                    Bounds::from_corners(
                        point(rect.left(), rect.bottom() - px(1.)),
                        rect.bottom_right(),
                    ) + origin,
                    theme::accent(),
                ));
            }
        }
        let caret = shaped.caret_position(selection.caret, row_end);
        let caret_height = window.rem_size().min(line_height);
        window.paint_quad(fill(
            Bounds::new(
                origin + caret + point(px(0.), (line_height - caret_height) / 2.),
                size(px(1.), caret_height),
            ),
            theme::accent(),
        ));
    }
}
