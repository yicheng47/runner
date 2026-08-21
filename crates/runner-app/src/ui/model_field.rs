use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    canvas, div, px, rems, svg, Bounds, Context, Entity, KeyDownEvent, MouseButton, Pixels, Render,
    ScrollHandle, Window,
};
use runner_backend::ops::runtime::RuntimeCatalogOption;

use crate::theme;
use crate::ui::field::TextField;
use crate::ui::menu::{popup_layer, DismissHandler, MenuKey};
use crate::ui::scrollbar::Scrollbar;
use crate::ui::select::{SelectAction, SelectOption, SelectState};

pub struct ModelField {
    input: Entity<TextField>,
    suggestions: Vec<SelectOption>,
    state: SelectState,
    anchor_bounds: Option<Bounds<Pixels>>,
    disabled: bool,
    menu_scroll: ScrollHandle,
    menu_scrollbar: Entity<Scrollbar>,
}

impl ModelField {
    pub fn new(
        input: Entity<TextField>,
        suggestions: &[RuntimeCatalogOption],
        cx: &mut Context<Self>,
    ) -> Self {
        let suggestions = model_options(suggestions);
        input.update(cx, |input, input_cx| {
            input.set_placeholder("default", input_cx);
            input.set_placeholder_as_value(true, input_cx);
            input.set_hover_border(true, input_cx);
            input.set_disabled_cursor_not_allowed(true, input_cx);
            input.set_right_padding(if suggestions.is_empty() { 10. } else { 32. }, input_cx);
        });
        let menu_scroll = ScrollHandle::new();
        let owner = cx.entity_id();
        let menu_scrollbar = cx.new(|_| Scrollbar::app(menu_scroll.clone(), owner));
        Self {
            input,
            suggestions,
            state: SelectState::default(),
            anchor_bounds: None,
            disabled: false,
            menu_scroll,
            menu_scrollbar,
        }
    }

    pub fn input(&self) -> Entity<TextField> {
        self.input.clone()
    }

    pub fn set_suggestions(
        &mut self,
        suggestions: &[RuntimeCatalogOption],
        cx: &mut Context<Self>,
    ) {
        self.suggestions = model_options(suggestions);
        self.input.update(cx, |input, input_cx| {
            input.set_right_padding(
                if self.suggestions.is_empty() {
                    10.
                } else {
                    32.
                },
                input_cx,
            )
        });
        if self.suggestions.is_empty() {
            self.state.close();
        }
        cx.notify();
    }

    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        if disabled {
            self.state.close();
        }
        self.input
            .update(cx, |input, input_cx| input.set_disabled(disabled, input_cx));
        cx.notify();
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.state.close();
        cx.notify();
    }

    fn selected_index(&self, cx: &Context<Self>) -> usize {
        let value = self.input.read(cx).text();
        self.suggestions
            .iter()
            .position(|option| option.value == value)
            .unwrap_or(0)
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.disabled || self.suggestions.is_empty() {
            return;
        }
        self.state
            .toggle(&self.suggestions, self.selected_index(cx));
        cx.notify();
    }

    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(option) = self.suggestions.get(index) else {
            return;
        };
        let value = option.value.clone();
        self.input
            .update(cx, |input, input_cx| input.reset(value, input_cx));
        self.state.close();
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || self.input.read(cx).is_composing() {
            return;
        }
        let key = match event.keystroke.key.as_str() {
            "up" => Some(MenuKey::Up),
            "down" => Some(MenuKey::Down),
            "home" if self.state.is_open() => Some(MenuKey::Home),
            "end" if self.state.is_open() => Some(MenuKey::End),
            "enter" if self.state.is_open() => Some(MenuKey::Enter),
            "escape" if self.state.is_open() => Some(MenuKey::Escape),
            _ => None,
        };
        let Some(key) = key else { return };
        cx.stop_propagation();
        let selected = self.selected_index(cx);
        if let SelectAction::Changed(index) =
            self.state.handle_key(key, &self.suggestions, selected)
        {
            self.choose(index, cx);
        } else {
            cx.notify();
        }
    }
}

impl Render for ModelField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.state.is_open();
        let has_suggestions = !self.suggestions.is_empty();
        let entity = cx.entity();
        let click_entity = entity.clone();
        let click_focus = self.input.read(cx).focus_handle();
        let disabled = self.disabled;
        let mut root = div()
            .id("model-field")
            .relative()
            .w_full()
            .on_key_down(cx.listener(Self::on_key_down))
            .capture_any_mouse_down(move |event, window, cx| {
                if disabled || event.button != MouseButton::Left {
                    return;
                }
                click_focus.focus(window);
                click_entity.update(cx, |field, cx| field.toggle(cx));
            })
            .child(self.input.clone())
            .when(has_suggestions, |field| {
                field.child(
                    div()
                        .absolute()
                        .right(rems(2. / 16.))
                        .top(rems(2. / 16.))
                        .size(rems(32. / 16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(theme::faint())
                        .when(!self.disabled, |toggle| toggle.cursor_pointer())
                        .child(
                            svg()
                                .path(if open {
                                    "chevron-up.svg"
                                } else {
                                    "chevron-down.svg"
                                })
                                .size(rems(14. / 16.))
                                .text_color(theme::faint()),
                        ),
                )
            })
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, _, cx| {
                        entity.update(cx, |field, _| field.anchor_bounds = Some(bounds));
                    },
                )
                .absolute()
                .inset_0(),
            );

        if let (true, Some(anchor)) = (open, self.anchor_bounds) {
            let current = self.input.read(cx).text().to_owned();
            let field_entity = cx.entity();
            let rows = self
                .suggestions
                .iter()
                .cloned()
                .enumerate()
                .map(|(index, option)| {
                    let active = option.value == current;
                    let highlighted = self.state.highlighted() == index;
                    let foreground = if active {
                        theme::text()
                    } else {
                        theme::muted()
                    };
                    let click_entity = field_entity.clone();
                    div()
                        .id(("model-option", index))
                        .w_full()
                        .px_3()
                        .py_2()
                        .flex()
                        .flex_col()
                        .gap(rems(2. / 16.))
                        .cursor_pointer()
                        .when(active || highlighted, |row| row.bg(theme::raised()))
                        .hover(|row| row.bg(theme::raised()))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .justify_between()
                                .gap_2()
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(rems(14. / 16.))
                                        .font_weight(gpui::FontWeight::MEDIUM)
                                        .text_color(foreground)
                                        .child(option.label),
                                )
                                .when(active, |label| {
                                    label.child(
                                        svg()
                                            .path("check.svg")
                                            .size(rems(14. / 16.))
                                            .text_color(theme::accent()),
                                    )
                                }),
                        )
                        .children(option.description.map(|description| {
                            div()
                                .text_size(rems(11. / 16.))
                                .text_color(theme::faint())
                                .child(description)
                        }))
                        .on_click(move |_, _, cx| {
                            click_entity.update(cx, |field, cx| field.choose(index, cx));
                        })
                });
            let menu = div()
                .id("model-field-options")
                .relative()
                .max_h(rems(260. / 16.))
                .overflow_hidden()
                .rounded(rems(4. / 16.))
                .border_1()
                .border_color(theme::border_strong())
                .bg(theme::panel())
                .shadow_xl()
                .child(
                    div()
                        .id("model-field-scroll")
                        .max_h(rems(260. / 16.))
                        .overflow_y_scroll()
                        .scrollbar_width(px(0.))
                        .track_scroll(&self.menu_scroll)
                        .py_1()
                        .children(rows),
                )
                .child(self.menu_scrollbar.clone())
                .into_any_element();
            let dismiss_entity: Entity<Self> = cx.entity();
            let dismiss: DismissHandler = Rc::new(move |_, cx| {
                dismiss_entity.update(cx, |field, cx| field.close(cx));
            });
            root = root.child(popup_layer(
                anchor,
                window,
                anchor.size.width,
                menu,
                dismiss,
            ));
        }
        root
    }
}

fn model_options(options: &[RuntimeCatalogOption]) -> Vec<SelectOption> {
    options
        .iter()
        .map(|option| {
            let mut mapped = SelectOption::new(option.value.clone(), option.label.clone());
            if let Some(description) = &option.description {
                mapped = mapped.description(description.clone());
            }
            mapped
        })
        .collect()
}
