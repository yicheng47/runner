use super::*;

#[derive(IntoElement)]
pub struct Label {
    id: SharedString,
    label: SharedString,
    hint: Option<(SharedString, FocusHandle)>,
    focus_target: Option<FocusHandle>,
    emphasized: bool,
}

impl Label {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            hint: None,
            focus_target: None,
            emphasized: false,
        }
    }

    pub fn hint(mut self, hint: impl Into<SharedString>, focus: FocusHandle) -> Self {
        self.hint = Some((hint.into(), focus));
        self
    }

    pub fn focus_target(mut self, focus_target: FocusHandle) -> Self {
        self.focus_target = Some(focus_target);
        self
    }

    pub fn emphasized(mut self, emphasized: bool) -> Self {
        self.emphasized = emphasized;
        self
    }
}

impl RenderOnce for Label {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let hint_id = SharedString::from(format!("field-hint-tooltip-{}", self.id));
        let focus_target = self.focus_target;
        div()
            .id(SharedString::from(format!("field-label-{}", self.id)))
            .flex()
            .items_center()
            .gap(rems(6. / 16.))
            .font_weight(if self.emphasized {
                FontWeight::SEMIBOLD
            } else {
                FontWeight::MEDIUM
            })
            .text_size(theme::text_ui())
            .text_color(if self.emphasized {
                theme::text()
            } else {
                theme::muted()
            })
            .when_some(focus_target, |label, focus| {
                label.cursor(CursorStyle::PointingHand).on_mouse_down(
                    MouseButton::Left,
                    move |_, window, cx| {
                        focus.focus(window, cx);
                    },
                )
            })
            .child(self.label)
            .children(self.hint.map(|(hint, focus)| {
                let hint_focus = focus.clone();
                Tooltip::new(
                    hint_id,
                    hint,
                    div()
                        .track_focus(&focus)
                        .tab_index(0)
                        .tab_stop(true)
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(rems(14. / 16.))
                        .rounded(rems(2. / 16.))
                        .text_color(theme::faint())
                        .focus_visible(|hint| {
                            hint.shadow(vec![BoxShadow {
                                color: theme::faint(),
                                offset: gpui::point(px(0.), px(0.)),
                                blur_radius: px(0.),
                                spread_radius: px(1.),
                                inset: false,
                            }])
                        })
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            hint_focus.focus(window, cx);
                            cx.stop_propagation();
                        })
                        .child(
                            svg()
                                .flex_none()
                                .path("info.svg")
                                .size(rems(14. / 16.))
                                .text_color(theme::faint()),
                        ),
                )
                .focus_handle(focus)
            }))
    }
}

#[derive(IntoElement)]
pub struct FieldError {
    message: SharedString,
}

impl FieldError {
    pub fn new(message: impl Into<SharedString>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl RenderOnce for FieldError {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .text_size(theme::text_ui())
            .text_color(theme::danger())
            .child(self.message)
    }
}

#[derive(IntoElement)]
pub struct Field {
    id: SharedString,
    label: SharedString,
    hint: Option<(SharedString, FocusHandle)>,
    tag: Option<SharedString>,
    subtitle: Option<SharedString>,
    error: Option<SharedString>,
    focus_target: Option<FocusHandle>,
    child: AnyElement,
    emphasized: bool,
}

impl Field {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        child: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            hint: None,
            tag: None,
            subtitle: None,
            error: None,
            focus_target: None,
            child: child.into_any_element(),
            emphasized: false,
        }
    }

    pub fn hint(mut self, hint: impl Into<SharedString>, focus: FocusHandle) -> Self {
        self.hint = Some((hint.into(), focus));
        self
    }

    /// A faint word after the label, such as `optional`.
    pub fn tag(mut self, tag: impl Into<SharedString>) -> Self {
        self.tag = Some(tag.into());
        self
    }

    pub fn focus_target(mut self, focus_target: FocusHandle) -> Self {
        self.focus_target = Some(focus_target);
        self
    }

    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    pub fn error(mut self, error: impl Into<SharedString>) -> Self {
        self.error = Some(error.into());
        self
    }

    pub fn emphasized(mut self, emphasized: bool) -> Self {
        self.emphasized = emphasized;
        self
    }
}

impl RenderOnce for Field {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let mut label = Label::new(self.id, self.label).emphasized(self.emphasized);
        if let Some(focus_target) = self.focus_target {
            label = label.focus_target(focus_target);
        }
        if let Some((hint, focus)) = self.hint {
            label = label.hint(hint, focus);
        }
        div()
            .flex()
            .flex_col()
            .gap(rems(if self.emphasized { 6. / 16. } else { 4. / 16. }))
            .child(match self.tag {
                Some(tag) => div()
                    .flex()
                    .items_center()
                    .gap(rems(6. / 16.))
                    .child(label)
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::faint())
                            .child(tag),
                    )
                    .into_any_element(),
                None => label.into_any_element(),
            })
            .child(self.child)
            .children(self.subtitle.map(|subtitle| {
                div()
                    .text_size(theme::text_meta())
                    .text_color(theme::faint())
                    .child(subtitle)
            }))
            .children(self.error.map(FieldError::new))
    }
}

#[derive(IntoElement)]
pub struct BrowseField {
    input: Entity<TextField>,
    disabled: bool,
    browse_id: ElementId,
    browse_label: SharedString,
    browse_focus: Option<FocusHandle>,
    on_browse: PressHandler,
}

impl BrowseField {
    pub fn new(input: Entity<TextField>, disabled: bool, on_browse: PressHandler) -> Self {
        Self {
            input,
            disabled,
            browse_id: "working-dir-browse".into(),
            browse_label: "Browse".into(),
            browse_focus: None,
            on_browse,
        }
    }

    pub fn browse_id(mut self, browse_id: impl Into<ElementId>) -> Self {
        self.browse_id = browse_id.into();
        self
    }

    pub fn browse_label(mut self, browse_label: impl Into<SharedString>) -> Self {
        self.browse_label = browse_label.into();
        self
    }

    pub fn browse_focus(mut self, browse_focus: FocusHandle) -> Self {
        self.browse_focus = Some(browse_focus);
        self
    }
}

impl RenderOnce for BrowseField {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let browse = Rc::clone(&self.on_browse);
        div()
            .flex()
            .items_center()
            .gap(rems(8. / 16.))
            .child(div().flex_1().min_w(px(0.)).child(self.input))
            .child(
                Button::new(self.browse_id, self.browse_label)
                    .when_some(self.browse_focus, |button, focus| {
                        button.focus_handle(focus)
                    })
                    .disabled(self.disabled)
                    .on_press(move |window, cx| browse(window, cx)),
            )
    }
}
