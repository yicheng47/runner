use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    canvas, div, px, rems, rgb, svg, App, Bounds, Context, ElementId, Entity, FocusHandle,
    FontWeight, KeyDownEvent, Pixels, Render, ScrollHandle, SharedString, Window,
};
use runner_backend::ops::runtime::RuntimeCatalogEntry;

use crate::theme;
use crate::ui::app_zoom;
use crate::ui::menu::{popup_layer, DismissHandler, MenuItem, MenuKey, MenuState};
use crate::ui::scrollbar::Scrollbar;

pub type SelectHandler = Rc<dyn Fn(String, &mut Window, &mut App)>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectOption {
    pub value: String,
    pub label: SharedString,
    pub description: Option<SharedString>,
    pub danger: bool,
    pub disabled: bool,
    pub swatch: Option<u32>,
}

impl SelectOption {
    pub fn new(value: impl Into<String>, label: impl Into<SharedString>) -> Self {
        Self {
            value: value.into(),
            label: label.into(),
            description: None,
            danger: false,
            disabled: false,
            swatch: None,
        }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn danger(mut self, danger: bool) -> Self {
        self.danger = danger;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn swatch(mut self, color: u32) -> Self {
        self.swatch = Some(color);
        self
    }

    fn as_menu_item(&self) -> MenuItem {
        let mut item = MenuItem::new(self.label.clone())
            .destructive(self.danger)
            .disabled(self.disabled);
        if let Some(description) = &self.description {
            item = item.description(description.clone());
        }
        item
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectAction {
    None,
    Changed(usize),
    Closed,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SelectState {
    menu: MenuState,
}

impl SelectState {
    pub fn is_open(&self) -> bool {
        self.menu.open
    }

    pub fn highlighted(&self) -> usize {
        self.menu.highlighted
    }

    pub fn open(&mut self, options: &[SelectOption], selected: usize) {
        self.menu.open(&menu_items(options), selected);
    }

    pub fn close(&mut self) {
        self.menu.close();
    }

    pub fn toggle(&mut self, options: &[SelectOption], selected: usize) {
        if self.is_open() {
            self.close();
        } else {
            self.open(options, selected);
        }
    }

    pub fn handle_key(
        &mut self,
        key: MenuKey,
        options: &[SelectOption],
        selected: usize,
    ) -> SelectAction {
        if !self.is_open() && matches!(key, MenuKey::Enter | MenuKey::Down | MenuKey::Up) {
            self.open(options, selected);
            if key == MenuKey::Up {
                self.menu.handle_key(MenuKey::Up, &menu_items(options));
            }
            return SelectAction::None;
        }
        match self.menu.handle_key(key, &menu_items(options)) {
            crate::ui::menu::MenuAction::Activate(index) => SelectAction::Changed(index),
            crate::ui::menu::MenuAction::Close => SelectAction::Closed,
            crate::ui::menu::MenuAction::None => SelectAction::None,
        }
    }
}

fn menu_items(options: &[SelectOption]) -> Vec<MenuItem> {
    options.iter().map(SelectOption::as_menu_item).collect()
}

pub struct StyledSelect {
    id: ElementId,
    focus_handle: FocusHandle,
    options: Vec<SelectOption>,
    value: String,
    placeholder: SharedString,
    state: SelectState,
    anchor_bounds: Option<Bounds<Pixels>>,
    width: Pixels,
    min_menu_width: Pixels,
    detailed: bool,
    runtime_style: bool,
    monospace: bool,
    disabled: bool,
    error: Option<SharedString>,
    menu_scroll: ScrollHandle,
    menu_scrollbar: Entity<Scrollbar>,
    on_change: SelectHandler,
}

impl StyledSelect {
    pub fn new(
        id: impl Into<ElementId>,
        focus_handle: FocusHandle,
        value: impl Into<String>,
        options: Vec<SelectOption>,
        on_change: SelectHandler,
        cx: &mut Context<Self>,
    ) -> Self {
        let menu_scroll = ScrollHandle::new();
        let owner = cx.entity_id();
        let menu_scrollbar = cx.new(|_| Scrollbar::app(menu_scroll.clone(), owner));
        Self {
            id: id.into(),
            focus_handle,
            options,
            value: value.into(),
            placeholder: "Select…".into(),
            state: SelectState::default(),
            anchor_bounds: None,
            width: px(160.),
            min_menu_width: px(240.),
            detailed: false,
            runtime_style: false,
            monospace: false,
            disabled: false,
            error: None,
            menu_scroll,
            menu_scrollbar,
            on_change,
        }
    }

    pub fn runtime(
        id: impl Into<ElementId>,
        focus_handle: FocusHandle,
        value: impl Into<String>,
        catalog: &[RuntimeCatalogEntry],
        on_change: SelectHandler,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new(
            id,
            focus_handle,
            value,
            runtime_select_options(catalog),
            on_change,
            cx,
        )
        .runtime_style(true)
        .monospace(true)
        .min_menu_width(px(0.))
        .placeholder("No agents available")
    }

    pub fn width(mut self, width: Pixels) -> Self {
        self.width = width;
        self
    }

    pub fn min_menu_width(mut self, width: Pixels) -> Self {
        self.min_menu_width = width;
        self
    }

    pub fn detailed(mut self, detailed: bool) -> Self {
        self.detailed = detailed;
        self
    }

    fn runtime_style(mut self, runtime_style: bool) -> Self {
        self.runtime_style = runtime_style;
        self
    }

    pub fn monospace(mut self, monospace: bool) -> Self {
        self.monospace = monospace;
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    pub fn set_value(&mut self, value: impl Into<String>, cx: &mut Context<Self>) {
        self.value = value.into();
        cx.notify();
    }

    pub fn set_options(&mut self, options: Vec<SelectOption>, cx: &mut Context<Self>) {
        if self.options == options {
            return;
        }
        self.options = options;
        if self.options.is_empty() {
            self.state.close();
        }
        cx.notify();
    }

    pub fn set_disabled(&mut self, disabled: bool, cx: &mut Context<Self>) {
        self.disabled = disabled;
        if disabled {
            self.state.close();
        }
        cx.notify();
    }

    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    pub fn set_error(&mut self, error: Option<SharedString>, cx: &mut Context<Self>) {
        self.error = error;
        cx.notify();
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.state.close();
        cx.notify();
    }

    fn selected_index(&self) -> usize {
        self.options
            .iter()
            .position(|option| option.value == self.value)
            .unwrap_or(0)
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.disabled || self.options.is_empty() {
            return;
        }
        self.state.toggle(
            &self.options,
            self.selected_index().min(self.options.len() - 1),
        );
        cx.notify();
    }

    fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(option) = self.options.get(index) else {
            return;
        };
        if option.disabled {
            return;
        }
        self.value = option.value.clone();
        self.state.close();
        (self.on_change)(self.value.clone(), window, cx);
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        let key = match event.keystroke.key.as_str() {
            "up" => Some(MenuKey::Up),
            "down" => Some(MenuKey::Down),
            "home" if self.state.is_open() => Some(MenuKey::Home),
            "end" if self.state.is_open() => Some(MenuKey::End),
            "enter" | "space" => Some(MenuKey::Enter),
            "escape" if self.state.is_open() => Some(MenuKey::Escape),
            _ => None,
        };
        let Some(key) = key else { return };
        cx.stop_propagation();
        let selected = self.selected_index();
        if let SelectAction::Changed(index) = self.state.handle_key(key, &self.options, selected) {
            self.choose(index, window, cx);
        } else {
            cx.notify();
        }
    }
}

impl Render for StyledSelect {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let zoom = app_zoom(window);
        let selected = self.options.get(self.selected_index());
        let label = selected
            .map(|option| option.label.clone())
            .unwrap_or_else(|| self.placeholder.clone());
        let description = self
            .detailed
            .then(|| selected.and_then(|option| option.description.clone()))
            .flatten();
        let swatch = selected.and_then(|option| option.swatch);
        let open = self.state.is_open();
        let entity = cx.entity();
        let click_entity = entity.clone();
        let click_focus = self.focus_handle.clone();
        let height = if self.detailed { 52. } else { 34. };
        let mut root = div()
            .id(self.id.clone())
            .relative()
            .w(rems(f32::from(self.width) / 16.))
            .track_focus(&self.focus_handle)
            .tab_index(0)
            .tab_stop(!self.disabled)
            .on_key_down(cx.listener(Self::on_key_down))
            .child(
                div()
                    .id("styled-select-trigger")
                    .debug_selector(|| "STYLED_SELECT_TRIGGER".into())
                    .track_focus(&self.focus_handle)
                    .w_full()
                    .h(rems(height / 16.))
                    .px(rems(if self.detailed { 12. / 16. } else { 10. / 16. }))
                    .flex()
                    .items_center()
                    .gap_2()
                    .rounded(rems(if self.detailed { 6. / 16. } else { 4. / 16. }))
                    .border_1()
                    .border_color(if self.error.is_some() {
                        theme::danger()
                    } else if open {
                        theme::faint()
                    } else if self.detailed {
                        theme::border()
                    } else {
                        theme::border_strong()
                    })
                    .bg(theme::bg())
                    .opacity(if self.disabled { 0.6 } else { 1. })
                    .when(!self.disabled, |trigger| {
                        trigger
                            .cursor_pointer()
                            .hover(|trigger| trigger.border_color(theme::faint()))
                            .on_click(move |_, window, cx| {
                                click_focus.focus(window);
                                click_entity.update(cx, |select, cx| select.toggle(cx));
                            })
                    })
                    .focus_visible(|style| style.border_color(theme::faint()))
                    .children(swatch.map(|color| {
                        div()
                            .size(rems(12. / 16.))
                            .flex_none()
                            .rounded(rems(2. / 16.))
                            .bg(rgb(color))
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .justify_center()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(rems(if self.detailed {
                                        13. / 16.
                                    } else {
                                        14. / 16.
                                    }))
                                    .font_weight(if self.detailed {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(theme::text())
                                    .when(self.monospace, |label| {
                                        label.font_family("JetBrains Mono")
                                    })
                                    .child(label),
                            )
                            .children(description.map(|description| {
                                div()
                                    .truncate()
                                    .text_size(rems(11. / 16.))
                                    .text_color(theme::muted())
                                    .child(description)
                            })),
                    )
                    .child(
                        svg()
                            .path(if open {
                                "chevron-up.svg"
                            } else {
                                "chevron-down.svg"
                            })
                            .size(rems(14. / 16.))
                            .flex_none()
                            .text_color(theme::faint()),
                    ),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, _, cx| {
                        entity.update(cx, |select, _| select.anchor_bounds = Some(bounds));
                    },
                )
                .absolute()
                .inset_0(),
            );

        if let (true, Some(anchor)) = (open, self.anchor_bounds) {
            let select_entity = cx.entity();
            let rows = self
                .options
                .iter()
                .cloned()
                .enumerate()
                .map(|(index, option)| {
                    let active = option.value == self.value;
                    let highlighted = self.state.highlighted() == index;
                    let stacked = self.detailed || option.description.is_some();
                    let foreground = if option.disabled {
                        theme::faint()
                    } else if option.danger {
                        theme::with_alpha(theme::danger(), if active { 1. } else { 0.8 })
                    } else if active || self.detailed || self.runtime_style {
                        theme::text()
                    } else {
                        theme::muted()
                    };
                    let click_entity = select_entity.clone();
                    div()
                        .id(("select-option", index))
                        .w_full()
                        .px(rems(if self.detailed { 10. / 16. } else { 12. / 16. }))
                        .py_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .when(stacked, |row| row.items_start())
                        .when(self.detailed, |row| row.rounded(rems(4. / 16.)))
                        .opacity(if option.disabled { 0.5 } else { 1. })
                        .when(active || highlighted, |row| {
                            row.bg(if option.danger {
                                theme::with_alpha(theme::danger(), 0.1)
                            } else {
                                theme::raised()
                            })
                        })
                        .when(!option.disabled, |row| {
                            row.cursor_pointer().hover(|row| {
                                if option.danger {
                                    row.bg(theme::with_alpha(theme::danger(), 0.1))
                                } else {
                                    row.bg(theme::raised())
                                }
                            })
                        })
                        .children(option.swatch.map(|color| {
                            div()
                                .size(rems(12. / 16.))
                                .flex_none()
                                .rounded(rems(2. / 16.))
                                .bg(rgb(color))
                                .when(stacked, |swatch| swatch.mt(rems(2. / 16.)))
                        }))
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.))
                                .flex()
                                .flex_col()
                                .gap(rems(2. / 16.))
                                .child(
                                    div()
                                        .truncate()
                                        .text_size(rems(if self.detailed {
                                            13. / 16.
                                        } else {
                                            14. / 16.
                                        }))
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(foreground)
                                        .when(self.monospace, |label| {
                                            label.font_family("JetBrains Mono")
                                        })
                                        .child(option.label),
                                )
                                .children(option.description.map(|description| {
                                    div()
                                        .text_size(rems(11. / 16.))
                                        .text_color(if self.detailed {
                                            theme::muted()
                                        } else {
                                            theme::faint()
                                        })
                                        .child(description)
                                })),
                        )
                        .when(active && !self.detailed, |row| {
                            row.child(
                                svg()
                                    .path("check.svg")
                                    .size(rems(14. / 16.))
                                    .flex_none()
                                    .text_color(if option.danger {
                                        theme::danger()
                                    } else {
                                        theme::accent()
                                    }),
                            )
                        })
                        .when(!option.disabled, |row| {
                            row.on_click(move |_, window, cx| {
                                click_entity
                                    .update(cx, |select, cx| select.choose(index, window, cx));
                            })
                        })
                });
            let menu = div()
                .id("styled-select-options")
                .relative()
                .max_h(rems(260. / 16.))
                .overflow_hidden()
                .rounded(rems(if self.detailed { 6. / 16. } else { 4. / 16. }))
                .border_1()
                .border_color(if self.detailed {
                    theme::border()
                } else {
                    theme::border_strong()
                })
                .bg(theme::panel())
                .shadow_xl()
                .child(
                    div()
                        .id("styled-select-scroll")
                        .max_h(rems(260. / 16.))
                        .overflow_y_scroll()
                        .scrollbar_width(px(0.))
                        .track_scroll(&self.menu_scroll)
                        .when(self.detailed, |menu| menu.p_1())
                        .when(!self.detailed, |menu| menu.py_1())
                        .children(rows),
                )
                .child(self.menu_scrollbar.clone())
                .into_any_element();
            let dismiss_entity: Entity<Self> = cx.entity();
            let dismiss: DismissHandler = Rc::new(move |_, cx| {
                dismiss_entity.update(cx, |select, cx| select.close(cx));
            });
            root = root.child(popup_layer(
                anchor,
                window,
                anchor.size.width.max(self.min_menu_width * zoom),
                menu,
                dismiss,
            ));
        }
        root
    }
}

pub type RuntimeSelect = StyledSelect;

pub fn runtime_select_options(catalog: &[RuntimeCatalogEntry]) -> Vec<SelectOption> {
    catalog
        .iter()
        .filter(|runtime| runtime.available)
        .map(|runtime| {
            SelectOption::new(runtime.name.clone(), runtime.display_name.clone())
                .description(runtime.description.clone())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, TestAppContext, VisualTestContext};

    struct SelectHost {
        select: Entity<StyledSelect>,
    }

    impl Render for SelectHost {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().p_4().child(self.select.clone())
        }
    }

    #[test]
    fn clicking_the_trigger_of_an_open_select_closes_it() {
        let mut cx = TestAppContext::single();
        let window = cx.add_window(|_, cx| {
            let focus_handle = cx.focus_handle();
            let select = cx.new(|cx| {
                StyledSelect::new(
                    "select",
                    focus_handle,
                    "a",
                    options(),
                    Rc::new(|_, _, _| {}),
                    cx,
                )
            });
            SelectHost { select }
        });
        cx.run_until_parked();
        let select = window
            .read_with(&cx, |host, _| host.select.clone())
            .unwrap();
        let mut window = VisualTestContext::from_window(window.into(), &cx);
        let trigger = window
            .debug_bounds("STYLED_SELECT_TRIGGER")
            .expect("trigger bounds");

        window.simulate_click(trigger.center(), Modifiers::default());
        window.run_until_parked();
        assert!(select.read_with(&window, |select, _| select.state.is_open()));

        window.simulate_click(trigger.center(), Modifiers::default());
        window.run_until_parked();
        assert!(!select.read_with(&window, |select, _| select.state.is_open()));
    }

    fn options() -> Vec<SelectOption> {
        vec![
            SelectOption::new("a", "A"),
            SelectOption::new("b", "B").disabled(true),
            SelectOption::new("c", "C"),
        ]
    }

    #[test]
    fn keyboard_state_opens_on_the_selection_and_skips_disabled_options() {
        let options = options();
        let mut state = SelectState::default();
        state.open(&options, 0);
        assert_eq!(state.highlighted(), 0);

        assert_eq!(
            state.handle_key(MenuKey::Down, &options, 0),
            SelectAction::None
        );
        assert_eq!(state.highlighted(), 2);
        assert_eq!(
            state.handle_key(MenuKey::Enter, &options, 0),
            SelectAction::Changed(2)
        );
        assert!(!state.is_open());
    }

    #[test]
    fn keyboard_state_closes_on_escape_without_changing() {
        let options = options();
        let mut state = SelectState::default();
        state.open(&options, 0);
        assert_eq!(
            state.handle_key(MenuKey::Escape, &options, 0),
            SelectAction::Closed
        );
        assert!(!state.is_open());
    }

    #[test]
    fn runtime_select_hides_unavailable_catalog_entries() {
        let mut catalog = vec![RuntimeCatalogEntry {
            name: "codex".into(),
            display_name: "Codex".into(),
            command: "codex".into(),
            native_fork: true,
            description: "OpenAI Codex CLI".into(),
            default_enabled: true,
            available: false,
            default_model: None,
            default_effort: None,
            models: Vec::new(),
            efforts: Vec::new(),
        }];
        assert!(runtime_select_options(&catalog).is_empty());
        catalog[0].available = true;
        assert_eq!(runtime_select_options(&catalog)[0].value, "codex");
    }
}
