use super::*;

impl TextField {
    /// The index under a window point, and whether it is the end of a
    /// soft-wrapped row; `None` until the text has been painted.
    pub(super) fn index_for_point(
        &self,
        position: Point<Pixels>,
        under: bool,
        window: &Window,
    ) -> Option<(usize, bool)> {
        self.with_text(window, |text, origin| {
            origin.map(|origin| text.index_for_position(position - origin, under))
        })
        .flatten()
    }

    pub(super) fn move_vertically(
        &mut self,
        up: bool,
        extend: bool,
        vertical_goal: Option<(usize, Pixels)>,
        window: &Window,
    ) {
        let selection = self.buffer.selection;
        let from = if extend || selection.is_empty() {
            selection.caret
        } else if up {
            selection.range().start
        } else {
            selection.range().end
        };
        let row_end = self.caret_row_end == Some(from);
        let goal_x = vertical_goal
            .filter(|(index, _)| *index == from)
            .map(|(_, x)| x);
        let (target, row_end, x) = self
            .with_text(window, |text, _| {
                text.vertical_target(from, row_end, up, goal_x)
            })
            .unwrap_or_else(|| {
                let end = if up { 0 } else { self.buffer.text.len() };
                (end, false, px(0.))
            });
        self.buffer.move_to(target, extend);
        self.caret_row_end = row_end.then_some(target);
        self.vertical_goal = Some((target, x));
        self.reveal_caret();
    }

    pub(super) fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        self.focus_handle.focus(window, cx);
        let (position, row_end) = self
            .index_for_point(event.position, event.click_count >= 2, window)
            .unwrap_or((self.buffer.text.len(), false));
        self.selecting = true;
        self.drag_position = None;
        self.caret_row_end = None;
        self.vertical_goal = None;
        match event.click_count {
            2 => self.buffer.select_word_at(position),
            count if count >= 3 => self.buffer.select_line_at(position),
            _ => {
                self.buffer.move_to(position, event.modifiers.shift);
                self.caret_row_end = row_end.then_some(position);
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// A window-wide mouse move while this field's selection drag is in
    /// progress, so the selection follows the pointer outside the field too.
    pub(super) fn on_drag_move(
        &mut self,
        event: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.pressed_button != Some(MouseButton::Left) {
            self.stop_selecting();
            return;
        }
        self.drag_position = Some(event.position);
        self.select_to_drag_position(window, cx);
        if self.auto_scroll.is_none() && self.auto_scroll_step().is_some() {
            self.auto_scroll = Some(cx.spawn_in(window, async move |field, cx| {
                loop {
                    cx.background_executor().timer(AUTO_SCROLL_TICK).await;
                    let scrolled = field
                        .update_in(cx, |field, window, cx| field.auto_scroll_tick(window, cx))
                        .unwrap_or(false);
                    if !scrolled {
                        break;
                    }
                }
                field.update(cx, |field, _| field.auto_scroll = None).ok();
            }));
        }
    }

    fn select_to_drag_position(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some((index, row_end)) = self
            .drag_position
            .and_then(|position| self.index_for_point(position, false, window))
        else {
            return;
        };
        self.buffer.move_to(index, true);
        self.caret_row_end = row_end.then_some(index);
        self.vertical_goal = None;
        cx.notify();
    }

    /// How far a textarea scrolls per tick toward a drag above or below it.
    fn auto_scroll_step(&self) -> Option<Pixels> {
        if !self.kind.multiline() {
            return None;
        }
        let position = self.drag_position?;
        let line_height = self.text_state.borrow().shaped.as_ref()?.line_height();
        let viewport = self.scroll_handle.bounds();
        let distance = if position.y < viewport.top() {
            viewport.top() - position.y
        } else if position.y > viewport.bottom() {
            viewport.bottom() - position.y
        } else {
            return None;
        };
        let speed = (distance.abs() / 2.).clamp(line_height / 2., line_height * 3.);
        Some(if distance > px(0.) { speed } else { -speed })
    }

    fn auto_scroll_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(step) = self.selecting.then(|| self.auto_scroll_step()).flatten() else {
            return false;
        };
        let offset = self.scroll_handle.offset();
        let max = self.scroll_handle.max_offset().y;
        let y = (offset.y + step).clamp(-max, px(0.));
        if y == offset.y {
            return false;
        }
        self.scroll_handle.set_offset(point(offset.x, y));
        self.select_to_drag_position(window, cx);
        cx.notify();
        true
    }

    fn stop_selecting(&mut self) {
        self.selecting = false;
        self.drag_position = None;
        self.auto_scroll = None;
    }

    pub(super) fn on_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        self.stop_selecting();
    }
}
