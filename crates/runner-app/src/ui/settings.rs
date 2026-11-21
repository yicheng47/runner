use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, App, BoxShadow, CursorStyle, ElementId, FontWeight, Hsla,
    KeyDownEvent, RenderOnce, SharedString, Window,
};

use crate::theme;

#[derive(IntoElement)]
pub struct SettingsHeader {
    title: SharedString,
    subtitle: SharedString,
    action: Option<AnyElement>,
}

pub type PaneHeader = SettingsHeader;

impl SettingsHeader {
    pub fn new(title: impl Into<SharedString>, subtitle: impl Into<SharedString>) -> Self {
        Self {
            title: title.into(),
            subtitle: subtitle.into(),
            action: None,
        }
    }

    pub fn action(mut self, action: impl IntoElement) -> Self {
        self.action = Some(action.into_any_element());
        self
    }
}

impl RenderOnce for SettingsHeader {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(rems(24. / 16.))
                    .child(
                        div()
                            .text_size(theme::text_display())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child(self.title),
                    )
                    .children(self.action.map(|action| div().flex_none().child(action))),
            )
            .child(
                div()
                    .text_size(theme::text_body())
                    .text_color(theme::muted())
                    .child(self.subtitle),
            )
    }
}

#[derive(IntoElement)]
pub struct SettingsCard {
    rows: Vec<AnyElement>,
}

impl SettingsCard {
    pub fn new(rows: impl IntoIterator<Item = AnyElement>) -> Self {
        Self {
            rows: rows.into_iter().collect(),
        }
    }
}

impl RenderOnce for SettingsCard {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(rems(12. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .children(self.rows.into_iter().enumerate().map(|(index, row)| {
                div()
                    .when(index > 0, |row| {
                        row.border_t_1().border_color(theme::border())
                    })
                    .child(row)
            }))
    }
}

#[derive(IntoElement)]
pub struct SettingsRow {
    label: SharedString,
    label_icon: Option<(SharedString, Hsla)>,
    subtitle: Option<SharedString>,
    subtitle_color: Option<Hsla>,
    wrap_control: bool,
    #[cfg(test)]
    debug_prefix: Option<SharedString>,
    control: AnyElement,
}

impl SettingsRow {
    pub fn new(label: impl Into<SharedString>, control: impl IntoElement) -> Self {
        Self {
            label: label.into(),
            label_icon: None,
            subtitle: None,
            subtitle_color: None,
            wrap_control: false,
            #[cfg(test)]
            debug_prefix: None,
            control: control.into_any_element(),
        }
    }

    pub fn label_icon(mut self, path: impl Into<SharedString>, color: Hsla) -> Self {
        self.label_icon = Some((path.into(), color));
        self
    }

    pub fn subtitle(mut self, subtitle: impl Into<SharedString>) -> Self {
        self.subtitle = Some(subtitle.into());
        self
    }

    pub fn subtitle_color(mut self, color: Hsla) -> Self {
        self.subtitle_color = Some(color);
        self
    }

    pub fn wrap_control(mut self) -> Self {
        self.wrap_control = true;
        self
    }

    #[cfg(test)]
    pub fn debug_prefix(mut self, prefix: impl Into<SharedString>) -> Self {
        self.debug_prefix = Some(prefix.into());
        self
    }
}

impl RenderOnce for SettingsRow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        #[cfg(test)]
        let label_prefix = self.debug_prefix.clone();
        #[cfg(test)]
        let subtitle_prefix = self.debug_prefix.clone();
        #[cfg(test)]
        let control_prefix = self.debug_prefix;
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(rems(24. / 16.))
            .when(self.wrap_control, |row| {
                row.flex_wrap()
                    .gap_y(rems(12. / 16.))
                    .debug_selector(|| "SETTINGS_WRAPPED_ROW".into())
            })
            .px_4()
            .py_3()
            .child(
                div()
                    .min_w(px(0.))
                    .when(self.wrap_control, |text| {
                        text.flex_1()
                            .flex_basis(rems(200. / 16.))
                            .debug_selector(|| "SETTINGS_WRAPPED_ROW_TEXT".into())
                    })
                    .flex()
                    .flex_col()
                    .gap(rems(2. / 16.))
                    .child({
                        let label = div()
                            .when_some(
                                {
                                    #[cfg(test)]
                                    {
                                        label_prefix
                                    }
                                    #[cfg(not(test))]
                                    {
                                        None::<SharedString>
                                    }
                                },
                                |label, prefix| {
                                    label.debug_selector(move || format!("{prefix}_LABEL"))
                                },
                            )
                            .text_size(theme::text_body())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::text());
                        match self.label_icon {
                            Some((path, color)) => label
                                .flex()
                                .items_center()
                                .gap(rems(6. / 16.))
                                .child(
                                    svg()
                                        .flex_none()
                                        .path(path)
                                        .size(rems(13. / 16.))
                                        .text_color(color),
                                )
                                .child(self.label),
                            None => label.child(self.label),
                        }
                    })
                    .children(self.subtitle.map(|subtitle| {
                        div()
                            .when(self.wrap_control, |description| {
                                description
                                    .debug_selector(|| "SETTINGS_WRAPPED_ROW_DESCRIPTION".into())
                            })
                            .when_some(
                                {
                                    #[cfg(test)]
                                    {
                                        subtitle_prefix
                                    }
                                    #[cfg(not(test))]
                                    {
                                        None::<SharedString>
                                    }
                                },
                                |el, prefix| {
                                    el.debug_selector(move || format!("{prefix}_SUBTITLE"))
                                },
                            )
                            .text_size(theme::text_meta())
                            .text_color(self.subtitle_color.unwrap_or_else(theme::muted))
                            .child(subtitle)
                    })),
            )
            .child(
                div()
                    .when_some(
                        {
                            #[cfg(test)]
                            {
                                control_prefix
                            }
                            #[cfg(not(test))]
                            {
                                None::<SharedString>
                            }
                        },
                        |el, prefix| el.debug_selector(move || format!("{prefix}_CONTROL")),
                    )
                    .flex_none()
                    .child(self.control),
            )
    }
}

pub type StepHandler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Stepper {
    id: ElementId,
    value_width: f32,
    value: AnyElement,
    decrement_disabled: bool,
    increment_disabled: bool,
    on_decrement: StepHandler,
    on_increment: StepHandler,
}

impl Stepper {
    pub fn new(
        id: impl Into<ElementId>,
        value_width: f32,
        value: impl IntoElement,
        on_decrement: StepHandler,
        on_increment: StepHandler,
    ) -> Self {
        Self {
            id: id.into(),
            value_width,
            value: value.into_any_element(),
            decrement_disabled: false,
            increment_disabled: false,
            on_decrement,
            on_increment,
        }
    }

    pub fn decrement_disabled(mut self, disabled: bool) -> Self {
        self.decrement_disabled = disabled;
        self
    }

    pub fn increment_disabled(mut self, disabled: bool) -> Self {
        self.increment_disabled = disabled;
        self
    }
}

impl RenderOnce for Stepper {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .h(rems(30. / 16.))
            .items_center()
            .rounded(rems(6. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg())
            .child(stepper_button(
                (self.id.clone(), "decrement"),
                "minus.svg",
                self.decrement_disabled,
                self.on_decrement,
            ))
            .child(
                div()
                    .w(rems(self.value_width / 16.))
                    .h(rems(30. / 16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .border_l_1()
                    .border_r_1()
                    .border_color(theme::border())
                    .child(self.value),
            )
            .child(stepper_button(
                (self.id, "increment"),
                "plus.svg",
                self.increment_disabled,
                self.on_increment,
            ))
    }
}

fn stepper_button(
    id: impl Into<ElementId>,
    icon: &'static str,
    disabled: bool,
    handler: StepHandler,
) -> AnyElement {
    const HOVER_GROUP: &str = "settings-stepper-button";

    let key_handler = Rc::clone(&handler);
    let icon = svg()
        .flex_none()
        .path(icon)
        .size(rems(14. / 16.))
        .text_color(theme::faint())
        .when(!disabled, |icon| {
            icon.group_hover(HOVER_GROUP, |icon| icon.text_color(theme::text()))
        });
    let mut button = div()
        .id(id)
        .when(!disabled, |button| button.group(HOVER_GROUP))
        .tab_index(0)
        .tab_stop(!disabled)
        .size(rems(30. / 16.))
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme::faint())
        .opacity(if disabled { 0.4 } else { 1. })
        .cursor(if disabled {
            CursorStyle::OperationNotAllowed
        } else {
            CursorStyle::PointingHand
        })
        .focus_visible(|button| {
            button.shadow(vec![BoxShadow {
                color: theme::border_strong(),
                offset: gpui::point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(2.),
                inset: false,
            }])
        })
        .child(icon);
    if !disabled {
        button = button
            .on_click(move |_, window, cx| handler(window, cx))
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    key_handler(window, cx);
                }
            });
    }
    button.into_any_element()
}

pub type SegmentHandler = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct SegmentOption {
    id: SharedString,
    label: SharedString,
    icon: Option<SharedString>,
    selected: bool,
    debug_selector: Option<&'static str>,
    on_select: SegmentHandler,
}

impl SegmentOption {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        selected: bool,
        on_select: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            selected,
            debug_selector: None,
            on_select: Rc::new(on_select),
        }
    }

    pub fn icon(mut self, path: impl Into<SharedString>) -> Self {
        self.icon = Some(path.into());
        self
    }

    pub fn debug_selector(mut self, selector: &'static str) -> Self {
        self.debug_selector = Some(selector);
        self
    }
}

#[derive(IntoElement)]
pub struct SegmentedControl {
    options: Vec<SegmentOption>,
    debug_selector: Option<&'static str>,
}

impl SegmentedControl {
    pub fn new(options: impl IntoIterator<Item = SegmentOption>) -> Self {
        Self {
            options: options.into_iter().collect(),
            debug_selector: None,
        }
    }

    pub fn debug_selector(mut self, selector: &'static str) -> Self {
        self.debug_selector = Some(selector);
        self
    }
}

fn is_plain_activation_key(event: &KeyDownEvent) -> bool {
    !event.is_held
        && !event.keystroke.modifiers.modified()
        && matches!(event.keystroke.key.as_str(), "enter" | "space")
}

impl RenderOnce for SegmentedControl {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let track = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(rems(2. / 16.))
            .p(rems(3. / 16.))
            .rounded(rems(8. / 16.))
            .border_1()
            .border_color(theme::border_strong())
            .bg(theme::bg())
            .children(self.options.into_iter().map(|option| {
                let selected = option.selected;
                let foreground = if selected {
                    theme::text()
                } else {
                    theme::muted()
                };
                let hover_group = option.id.clone();
                let click = Rc::clone(&option.on_select);
                let key_select = option.on_select;
                let mut item = div()
                    .id(option.id)
                    .when(!selected, |item| item.group(hover_group.clone()))
                    .tab_index(0)
                    .flex()
                    .items_center()
                    .gap(rems(6. / 16.))
                    .px(rems(12. / 16.))
                    .py(rems(5. / 16.))
                    .rounded(rems(6. / 16.))
                    .border_1()
                    .border_color(if selected {
                        theme::seg_selected_border()
                    } else {
                        gpui::transparent_black()
                    })
                    .bg(if selected {
                        theme::seg_selected()
                    } else {
                        gpui::transparent_black()
                    })
                    .when(selected, |item| {
                        item.shadow(vec![BoxShadow {
                            color: gpui::rgba(0x0000001f).into(),
                            offset: gpui::point(px(0.), px(1.)),
                            blur_radius: px(2.),
                            spread_radius: px(0.),
                            inset: false,
                        }])
                    })
                    .text_size(theme::text_ui())
                    .font_weight(if selected {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::MEDIUM
                    })
                    .text_color(foreground)
                    .cursor(CursorStyle::PointingHand)
                    .when(!selected, |item| {
                        item.hover(|item| item.text_color(theme::text()))
                    })
                    .focus_visible(move |style| {
                        let style = style.shadow(crate::ui::focus_ring_with_offset(
                            theme::bg(),
                            theme::accent(),
                            px(1.),
                            px(2.),
                        ));
                        if selected {
                            style
                        } else {
                            // Paint opaque track ground inside the focus ring so the shadow does
                            // not flood an unselected segment's label.
                            style.bg(theme::bg()).text_color(theme::text())
                        }
                    })
                    .children(option.icon.map(|path| {
                        svg()
                            .path(path)
                            .size(rems(13. / 16.))
                            .flex_none()
                            .text_color(foreground)
                            .when(!selected, |icon| {
                                icon.group_hover(hover_group, |icon| icon.text_color(theme::text()))
                            })
                    }))
                    .child(option.label)
                    .on_click(move |event, window, cx| {
                        if !event.is_keyboard() {
                            click(window, cx);
                        }
                    })
                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                        if is_plain_activation_key(event) {
                            cx.stop_propagation();
                            key_select(window, cx);
                        }
                    });
                if let Some(selector) = option.debug_selector {
                    #[cfg(test)]
                    {
                        item = crate::theme_snapshot::record_fill(selector, item);
                        item = crate::theme_snapshot::note_border(selector, item);
                        item = crate::theme_snapshot::note_text(selector, item);
                    }
                    #[cfg(not(test))]
                    {
                        item = item.debug_selector(move || selector.into());
                    }
                }
                item
            }));
        if let Some(selector) = self.debug_selector {
            #[cfg(test)]
            {
                let track = crate::theme_snapshot::record_fill(selector, track);
                crate::theme_snapshot::note_border(selector, track)
            }
            #[cfg(not(test))]
            {
                track.debug_selector(move || selector.into())
            }
        } else {
            track
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme_snapshot::{assert_border, assert_fill_color, assert_text, ThemeGuard};
    use gpui::{
        size, Context, KeyUpEvent, Keystroke, Modifiers, Render, TestAppContext, VisualTestContext,
    };

    struct SegmentedHost {
        selected: usize,
        activations: Vec<usize>,
        bubbled_keys: Vec<String>,
    }

    impl Render for SegmentedHost {
        fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let host = cx.entity();
            let options = [
                ("auto", "Auto", Some("monitor.svg"), "SEG_OPT_AUTO"),
                ("light", "Light", Some("sun.svg"), "SEG_OPT_LIGHT"),
                ("dark", "Dark", Some("moon.svg"), "SEG_OPT_DARK"),
            ]
            .into_iter()
            .enumerate()
            .map(|(index, (key, label, icon, selector))| {
                let host = host.clone();
                let mut opt = SegmentOption::new(
                    format!("test-seg-{key}"),
                    label,
                    self.selected == index,
                    move |_, cx| {
                        host.update(cx, |this, cx| {
                            this.selected = index;
                            this.activations.push(index);
                            cx.notify();
                        });
                    },
                )
                .debug_selector(selector);
                if let Some(path) = icon {
                    opt = opt.icon(path);
                }
                opt
            });
            div()
                .size_full()
                .p_4()
                .bg(theme::panel())
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, _| {
                    this.bubbled_keys.push(event.keystroke.unparse());
                }))
                .child(SegmentedControl::new(options).debug_selector("SEG_TRACK"))
        }
    }

    #[test]
    fn segmented_control_paints_theme_tokens_and_avoids_vertical_shift() {
        let _theme = ThemeGuard::new();
        for variant in [
            theme::ThemeVariant::RunnerLight,
            theme::ThemeVariant::Carbon,
            theme::ThemeVariant::GruvboxLight,
            theme::ThemeVariant::TokyoNight,
        ] {
            theme::set_active_variant(variant);
            let mut cx = TestAppContext::single();
            let host = cx.add_window(|_, _| SegmentedHost {
                selected: 1,
                activations: Vec::new(),
                bubbled_keys: Vec::new(),
            });
            let mut visual = VisualTestContext::from_window(host.into(), &cx);
            visual.simulate_resize(size(px(400.), px(120.)));
            visual.run_until_parked();

            assert_fill_color(&mut visual, "SEG_TRACK", Some(theme::bg()));
            assert_border(&mut visual, "SEG_TRACK", theme::border_strong());
            assert_fill_color(&mut visual, "SEG_OPT_LIGHT", Some(theme::seg_selected()));
            assert_border(&mut visual, "SEG_OPT_LIGHT", theme::seg_selected_border());
            assert_text(&mut visual, "SEG_OPT_LIGHT", theme::text());
            assert_fill_color(&mut visual, "SEG_OPT_AUTO", Some(gpui::transparent_black()));
            assert_border(&mut visual, "SEG_OPT_AUTO", gpui::transparent_black());
            assert_text(&mut visual, "SEG_OPT_AUTO", theme::muted());

            let auto = visual.debug_bounds("SEG_OPT_AUTO").unwrap();
            let light = visual.debug_bounds("SEG_OPT_LIGHT").unwrap();
            let dark = visual.debug_bounds("SEG_OPT_DARK").unwrap();
            assert_eq!(auto.size.height, light.size.height, "{variant:?}");
            assert_eq!(dark.size.height, light.size.height, "{variant:?}");
        }
    }

    #[test]
    fn segmented_control_supports_mouse_and_plain_keyboard_activation_without_double_firing() {
        let _theme = ThemeGuard::new();
        theme::set_active_variant(theme::ThemeVariant::RunnerLight);
        let mut cx = TestAppContext::single();
        let host = cx.add_window(|_, _| SegmentedHost {
            selected: 0,
            activations: Vec::new(),
            bubbled_keys: Vec::new(),
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(400.), px(120.)));
        visual.run_until_parked();

        let light = visual.debug_bounds("SEG_OPT_LIGHT").unwrap();
        visual.simulate_click(light.center(), Modifiers::default());
        visual.run_until_parked();
        host.read_with(&visual, |host, _| {
            assert_eq!(host.selected, 1);
            assert_eq!(host.activations, vec![1]);
        })
        .unwrap();

        // Tab to Dark and verify modified chords (Ctrl/Shift + Space/Enter) neither activate nor
        // consume the key event, while plain Space activates once.
        host.update(&mut visual, |_, window, cx| {
            window.focus_next(cx);
        })
        .unwrap();
        visual.run_until_parked();
        for chord in ["ctrl-space", "shift-space", "shift-enter"] {
            visual.simulate_keystrokes(chord);
            visual.run_until_parked();
        }
        host.read_with(&visual, |host, _| {
            assert_eq!(host.selected, 1);
            assert_eq!(host.activations, vec![1]);
            assert_eq!(host.bubbled_keys.len(), 3);
        })
        .unwrap();

        visual.simulate_keystrokes("space");
        visual.simulate_event(KeyUpEvent {
            keystroke: Keystroke::parse("space").unwrap(),
        });
        visual.run_until_parked();
        host.read_with(&visual, |host, _| {
            assert_eq!(host.selected, 2);
            assert_eq!(host.activations, vec![1, 2]);
            assert_eq!(host.bubbled_keys.len(), 3);
        })
        .unwrap();
        assert_fill_color(&mut visual, "SEG_OPT_DARK", Some(theme::seg_selected()));
        assert_fill_color(
            &mut visual,
            "SEG_OPT_LIGHT",
            Some(gpui::transparent_black()),
        );

        // Shift-Tab back to Light and activate with Enter once.
        host.update(&mut visual, |_, window, cx| {
            window.focus_prev(cx);
        })
        .unwrap();
        visual.run_until_parked();
        visual.simulate_keystrokes("enter");
        visual.simulate_event(KeyUpEvent {
            keystroke: Keystroke::parse("enter").unwrap(),
        });
        visual.run_until_parked();
        host.read_with(&visual, |host, _| {
            assert_eq!(host.selected, 1);
            assert_eq!(host.activations, vec![1, 2, 1]);
            assert_eq!(host.bubbled_keys.len(), 3);
        })
        .unwrap();
    }
}
