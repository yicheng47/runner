use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    canvas, div, rems, svg, Bounds, Context, Entity, KeyDownEvent, MouseButton, Pixels, Render,
    ScrollHandle, Window,
};
use runner_core::protocol::runtime::RuntimeCatalogOption;

use crate::theme;
use crate::ui::app_zoom;
use crate::ui::field::TextField;
use crate::ui::menu::{DismissHandler, MenuKey};
use crate::ui::scrollbar::Scrollbar;
use crate::ui::select::{
    option_menu, option_menu_width, OptionMenuStyle, SelectAction, SelectOption, SelectState,
};

/// What the override dot takes at the text's right: the dot and the gaps
/// either side of it.
const MARKER_WIDTH: f32 = 14.;

pub struct ModelField {
    input: Entity<TextField>,
    suggestions: Vec<SelectOption>,
    state: SelectState,
    anchor_bounds: Option<Bounds<Pixels>>,
    disabled: bool,
    /// An amber dot inside the field's right edge: the value overrides
    /// another's.
    marker: bool,
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
            marker: false,
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
        self.sync_padding(cx);
        if self.suggestions.is_empty() {
            self.state.close();
        }
        cx.notify();
    }

    /// Shows or hides the override dot, keeping the text clear of it.
    pub fn set_marker(&mut self, marker: bool, cx: &mut Context<Self>) {
        if self.marker != marker {
            self.marker = marker;
            self.sync_padding(cx);
            cx.notify();
        }
    }

    /// The width the chevron takes at the right edge, or the plain padding.
    fn edge_width(&self) -> f32 {
        if self.suggestions.is_empty() {
            10.
        } else {
            32.
        }
    }

    fn sync_padding(&self, cx: &mut Context<Self>) {
        let padding = self.edge_width() + if self.marker { MARKER_WIDTH } else { 0. };
        self.input.update(cx, |input, input_cx| {
            input.set_right_padding(padding, input_cx)
        });
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
            "enter" if !self.suggestions.is_empty() => Some(MenuKey::Enter),
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
    fn menu_content(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        if !self.state.is_open() {
            return None;
        }
        let current = self.input.read(cx).text().to_owned();
        let field_entity = cx.weak_entity();
        let menu = option_menu(
            &self.suggestions,
            &current,
            self.state.highlighted(),
            OptionMenuStyle::default(),
            &self.menu_scroll,
            self.menu_scrollbar.clone(),
            Rc::new(move |index, _, cx| {
                let _ = field_entity.update(cx, |field, cx| field.choose(index, cx));
            }),
        );
        Some(menu)
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
                click_focus.focus(window, cx);
                click_entity.update(cx, |field, cx| field.toggle(cx));
            })
            .child(self.input.clone())
            .when(self.marker, |field| {
                field.child(
                    div()
                        .debug_selector(|| "MODEL_FIELD_MARK".into())
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .right(rems((self.edge_width() + 4.) / 16.))
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .flex_none()
                                .size(rems(6. / 16.))
                                .rounded_full()
                                .bg(theme::warning()),
                        ),
                )
            })
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
                                .flex_none()
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
            let menu = self.menu_content(window, cx).unwrap();
            let dismiss_entity: Entity<Self> = cx.entity();
            let dismiss: DismissHandler = Rc::new(move |_, cx| {
                dismiss_entity.update(cx, |field, cx| field.close(cx));
            });
            let width = option_menu_width(&self.suggestions, anchor.size.width, app_zoom(window));
            root = root.child(crate::ui::menu::popup_layer_sized(
                anchor, window, width, menu, dismiss,
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

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{px, TestAppContext, VisualTestContext};

    struct FieldHost {
        field: Entity<ModelField>,
    }

    impl Render for FieldHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().p_4().child(
                div()
                    .debug_selector(|| "MODEL_FIELD_HOST".into())
                    .w(px(272.))
                    .child(self.field.clone()),
            )
        }
    }

    fn option(value: &str, label: &str, description: &str) -> RuntimeCatalogOption {
        RuntimeCatalogOption {
            value: value.into(),
            label: label.into(),
            description: Some(description.into()),
            supported_efforts: None,
        }
    }

    /// The override dot takes a slot at the text's right, so a long value
    /// ends before it instead of running underneath.
    #[test]
    fn the_override_marker_reserves_room_at_the_texts_right() {
        fn padding(input: &Entity<TextField>, cx: &VisualTestContext) -> f32 {
            input.read_with(cx, |input, _| input.text_right_padding())
        }
        let mut cx = TestAppContext::single();
        let window = cx.add_window(|window, cx| {
            window.resize(gpui::size(px(1200.), px(900.)));
            let input = cx.new(|cx| TextField::new(cx.focus_handle(), "", "", true));
            let field = cx.new(|cx| ModelField::new(input, &[option("a", "A", "First")], cx));
            FieldHost { field }
        });
        cx.run_until_parked();
        let field = window.read_with(&cx, |host, _| host.field.clone()).unwrap();
        let input = field.read_with(&cx, |field, _| field.input());
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        assert_eq!(padding(&input, &visual), 32.);

        field.update(&mut visual, |field, cx| field.set_marker(true, cx));
        visual.run_until_parked();
        assert_eq!(padding(&input, &visual), 32. + MARKER_WIDTH);
        let host = visual.debug_bounds("MODEL_FIELD_HOST").unwrap();
        let mark = visual.debug_bounds("MODEL_FIELD_MARK").unwrap();
        // The dot sits in the strip the padding reserves, left of the chevron.
        assert!(
            mark.left() >= host.right() - px(32. + MARKER_WIDTH),
            "{mark:?} in {host:?}"
        );
        assert!(
            mark.right() <= host.right() - px(32.),
            "{mark:?} in {host:?}"
        );

        field.update(&mut visual, |field, cx| field.set_suggestions(&[], cx));
        assert_eq!(padding(&input, &visual), 10. + MARKER_WIDTH);
        field.update(&mut visual, |field, cx| field.set_marker(false, cx));
        assert_eq!(padding(&input, &visual), 10.);
    }

    /// The model suggestions open the select's own menu: a centred check,
    /// a menu grown to its descriptions, and a scrollbar inside its frame.
    #[test]
    fn model_suggestions_share_the_select_menu() {
        let mut cx = TestAppContext::single();
        let window = cx.add_window(|window, cx| {
            window.resize(gpui::size(px(1200.), px(900.)));
            let input = cx.new(|cx| TextField::new(cx.focus_handle(), "", "", true));
            let options = [
                option("", "default", "Use the agent's own default model."),
                option(
                    "opus",
                    "Opus 5.5",
                    "Most capable for ambitious, long-running work",
                ),
                option("fable", "Fable 5.1", "For your toughest challenges"),
                option("sonnet", "Sonnet 5", "Most efficient for everyday tasks"),
                option("haiku", "Haiku 4.5", "Fastest for quick answers"),
                option("opus-1m", "Opus 5.5 (1M)", "The long-context variant"),
            ];
            let field = cx.new(|cx| ModelField::new(input, &options, cx));
            FieldHost { field }
        });
        cx.run_until_parked();
        let field = window.read_with(&cx, |host, _| host.field.clone()).unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.run_until_parked();
        field.update(&mut visual, |field, cx| field.toggle(cx));
        visual.run_until_parked();

        let menu = visual.debug_bounds("STYLED_SELECT_MENU").unwrap();
        let row = visual.debug_bounds("STYLED_SELECT_OPTION_0").unwrap();
        let check = visual.debug_bounds("STYLED_SELECT_CHECK").unwrap();
        let track = visual.debug_bounds("STYLED_SELECT_SCROLLBAR").unwrap();
        assert!(
            (check.center().y - row.center().y).abs() <= px(1.),
            "the check is centred in its row: {check:?} {row:?}"
        );
        assert!(
            menu.size.width > px(272.),
            "the menu grows past the field to fit its descriptions: {menu:?}"
        );
        assert!(
            row.size.height < px(60.),
            "a described row stays on two lines: {row:?}"
        );
        assert!(
            track.top() >= menu.top() + px(4.) && track.bottom() <= menu.bottom() - px(4.),
            "the scrollbar's track sits inside the menu's padding: {track:?} in {menu:?}"
        );
        assert!(
            row.right() <= track.left() + px(0.5) && check.right() <= track.left(),
            "six models scroll, so the rows end at the scrollbar's lane: {row:?} {check:?} {track:?}"
        );
    }
}
