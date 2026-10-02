#[cfg(test)]
use runner_backend::model::Runtime;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    canvas, div, px, rems, rgb, svg, AlignSelf, AnyElement, App, Bounds, Context, ElementId,
    Entity, FocusHandle, FontWeight, KeyDownEvent, Pixels, Render, ScrollHandle, SharedString,
    Window,
};
use runner_backend::ops::runtime::RuntimeCatalogEntry;

use crate::theme;
use crate::ui::app_zoom;
use crate::ui::menu::{
    popup_layer_sized, DismissHandler, MenuItem, MenuKey, MenuState, PopupWidth,
};
use crate::ui::scrollbar::{app_scrollbar_gutter, Scrollbar};

pub type SelectHandler = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// A mark drawn before an option's label, at the size the row asks for: a
/// role's avatar or a provider's mark. Options compare by the key, which must
/// name everything the mark draws.
#[derive(Clone)]
pub struct SelectLeading {
    key: SharedString,
    draw: Rc<dyn Fn(f32) -> AnyElement>,
}

impl SelectLeading {
    pub fn new(key: impl Into<SharedString>, draw: impl Fn(f32) -> AnyElement + 'static) -> Self {
        Self {
            key: key.into(),
            draw: Rc::new(draw),
        }
    }

    fn draw(&self, size: f32) -> AnyElement {
        (self.draw)(size)
    }
}

impl PartialEq for SelectLeading {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}

impl Eq for SelectLeading {}

impl std::fmt::Debug for SelectLeading {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SelectLeading").field(&self.key).finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectOption {
    pub value: String,
    pub label: SharedString,
    pub description: Option<SharedString>,
    pub danger: bool,
    pub disabled: bool,
    pub swatch: Option<u32>,
    pub leading: Option<SelectLeading>,
    /// The trigger carries an amber dot while this option is chosen: a value
    /// that overrides another's.
    pub marked: bool,
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
            leading: None,
            marked: false,
        }
    }

    pub fn leading(mut self, leading: SelectLeading) -> Self {
        self.leading = Some(leading);
        self
    }

    pub fn marked(mut self, marked: bool) -> Self {
        self.marked = marked;
        self
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
    picker: bool,
    full_width: bool,
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
            picker: false,
            full_width: false,
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

    /// How far the open menu can scroll; zero when every option is in view.
    pub fn menu_scroll_range(&self) -> Pixels {
        self.menu_scroll.max_offset().y
    }

    pub fn min_menu_width(mut self, width: Pixels) -> Self {
        self.min_menu_width = width;
        self
    }

    pub fn detailed(mut self, detailed: bool) -> Self {
        self.detailed = detailed;
        self
    }

    /// Fills the width of whatever holds the select, which then sizes it.
    pub fn full_width(mut self, full_width: bool) -> Self {
        self.full_width = full_width;
        self
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// The option the select currently shows, if it holds one.
    pub fn selected(&self) -> Option<&SelectOption> {
        self.options.get(self.selected_index())
    }

    /// Draws the trigger as the head of a card that supplies the border: the
    /// chosen option's mark at 40 px, its label and its description in
    /// monospace, and an up-down chevron. The menu keeps the trigger's width.
    pub fn picker(mut self, picker: bool) -> Self {
        self.picker = picker;
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
        if self.state.is_open() {
            self.menu_scroll.scroll_to_item(self.state.highlighted());
        }
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
            if self.state.is_open() {
                self.menu_scroll.scroll_to_item(self.state.highlighted());
            }
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
        let stacked = self.detailed || self.picker;
        let description = stacked
            .then(|| selected.and_then(|option| option.description.clone()))
            .flatten();
        let swatch = selected.and_then(|option| option.swatch);
        let leading = selected.and_then(|option| option.leading.clone());
        let marked = selected.is_some_and(|option| option.marked);
        let picker = self.picker;
        let open = self.state.is_open();
        let entity = cx.entity();
        let click_entity = entity.clone();
        let click_focus = self.focus_handle.clone();
        let height = if picker {
            60.
        } else if self.detailed {
            52.
        } else {
            34.
        };
        let mut root = div()
            .id(self.id.clone())
            .relative()
            .when(self.picker || self.full_width, |root| root.w_full())
            .when(!(self.picker || self.full_width), |root| {
                root.w(rems(f32::from(self.width) / 16.))
            })
            .on_key_down(cx.listener(Self::on_key_down))
            .child(
                div()
                    .id("styled-select-trigger")
                    .debug_selector(|| "STYLED_SELECT_TRIGGER".into())
                    .track_focus(&self.focus_handle.clone().tab_stop(!self.disabled))
                    .tab_index(0)
                    .tab_stop(!self.disabled)
                    .w_full()
                    .h(rems(height / 16.))
                    .px(rems(if stacked { 12. / 16. } else { 10. / 16. }))
                    .flex()
                    .items_center()
                    .gap(rems(if picker { 12. / 16. } else { 8. / 16. }))
                    .when(picker, |trigger| {
                        trigger
                            .rounded_tl(rems(7. / 16.))
                            .rounded_tr(rems(7. / 16.))
                    })
                    .when(!picker, |trigger| {
                        trigger
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
                    })
                    .opacity(if self.disabled { 0.6 } else { 1. })
                    .when(!self.disabled, |trigger| {
                        trigger
                            .cursor_pointer()
                            .hover(move |trigger| {
                                if picker {
                                    trigger.bg(theme::raised())
                                } else {
                                    trigger.border_color(theme::faint())
                                }
                            })
                            .on_click(move |event, window, cx| {
                                // Key-down already handles activation; GPUI emits another click on key-up.
                                if matches!(event, gpui::ClickEvent::Keyboard(_)) {
                                    return;
                                }
                                click_focus.focus(window, cx);
                                click_entity.update(cx, |select, cx| select.toggle(cx));
                            })
                    })
                    .focus_visible(move |style| {
                        if picker {
                            style.bg(theme::raised())
                        } else {
                            style.border_color(theme::faint())
                        }
                    })
                    .children(swatch.map(|color| {
                        div()
                            .size(rems(12. / 16.))
                            .flex_none()
                            .rounded(rems(2. / 16.))
                            .bg(rgb(color))
                    }))
                    .children(leading.map(|leading| leading.draw(if picker { 40. } else { 13. })))
                    .child(
                        div()
                            .debug_selector(|| "STYLED_SELECT_TEXT".into())
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .justify_center()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(if picker {
                                        theme::text_lead()
                                    } else if self.detailed {
                                        theme::text_body()
                                    } else {
                                        theme::text_title()
                                    })
                                    .font_weight(if stacked {
                                        FontWeight::SEMIBOLD
                                    } else {
                                        FontWeight::NORMAL
                                    })
                                    .text_color(theme::text())
                                    .when(self.monospace, |label| {
                                        label.font_family(theme::UI_MONOSPACE_FONT)
                                    })
                                    .child(label),
                            )
                            .children(description.map(|description| {
                                div()
                                    .truncate()
                                    .when(picker, |line| {
                                        line.font_family(theme::UI_MONOSPACE_FONT)
                                            .text_size(theme::text_ui())
                                    })
                                    .when(!picker, |line| line.text_size(theme::text_meta()))
                                    .text_color(theme::muted())
                                    .child(description)
                            })),
                    )
                    .children(marked.then(|| {
                        // Its own slot in the row: the label ends before it.
                        div()
                            .debug_selector(|| "STYLED_SELECT_MARK".into())
                            .flex_none()
                            .size(rems(6. / 16.))
                            .rounded_full()
                            .bg(theme::warning())
                    }))
                    .child(
                        svg()
                            .path(if picker {
                                "chevrons-up-down.svg"
                            } else if open {
                                "chevron-up.svg"
                            } else {
                                "chevron-down.svg"
                            })
                            .size(rems(if picker { 1. } else { 14. / 16. }))
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
            let menu = option_menu(
                &self.options,
                &self.value,
                self.state.highlighted(),
                OptionMenuStyle {
                    detailed: self.detailed || self.picker,
                    monospace: self.monospace,
                    runtime_style: self.runtime_style,
                    mono_description: self.picker,
                },
                &self.menu_scroll,
                self.menu_scrollbar.clone(),
                Rc::new(move |index, window, cx| {
                    select_entity.update(cx, |select, cx| select.choose(index, window, cx));
                }),
            );
            let dismiss_entity: Entity<Self> = cx.entity();
            let dismiss: DismissHandler = Rc::new(move |_, cx| {
                dismiss_entity.update(cx, |select, cx| select.close(cx));
            });
            let width = if self.picker {
                PopupWidth::Fixed(anchor.size.width)
            } else {
                option_menu_width(
                    &self.options,
                    anchor.size.width.max(self.min_menu_width * zoom),
                    zoom,
                )
            };
            root = root.child(popup_layer_sized(anchor, window, width, menu, dismiss));
        }
        root
    }
}

/// How an option menu draws its rows.
#[derive(Clone, Copy, Default)]
pub(crate) struct OptionMenuStyle {
    /// Roomier rows with inset corners and no check, for rich pickers.
    pub detailed: bool,
    pub monospace: bool,
    /// Every row reads at full contrast, as a runtime list does.
    pub runtime_style: bool,
    /// Descriptions in monospace, for a list of commands or handles.
    pub mono_description: bool,
}

pub(crate) type OptionHandler = Rc<dyn Fn(usize, &mut Window, &mut App)>;

/// The option list a select or the model field opens: each row's label with
/// its description under it and a check centred on the chosen row, in a
/// scroll list whose bar sits inside the menu's padding.
pub(crate) fn option_menu(
    options: &[SelectOption],
    value: &str,
    highlighted: usize,
    style: OptionMenuStyle,
    scroll: &ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    on_choose: OptionHandler,
) -> AnyElement {
    let rows = options.iter().cloned().enumerate().map(|(index, option)| {
        let active = option.value == value;
        let highlighted = highlighted == index;
        let stacked = style.detailed || option.description.is_some();
        let foreground = if option.disabled {
            theme::faint()
        } else if option.danger {
            theme::with_alpha(theme::danger(), if active { 1. } else { 0.8 })
        } else if active || style.detailed || style.runtime_style {
            theme::text()
        } else {
            theme::muted()
        };
        let on_choose = Rc::clone(&on_choose);
        div()
            .id(("select-option", index))
            .debug_selector(|| format!("STYLED_SELECT_OPTION_{index}"))
            .w_full()
            .px(rems(if style.detailed { 10. / 16. } else { 12. / 16. }))
            .py_2()
            .flex()
            .items_center()
            .gap_2()
            .when(stacked && option.leading.is_none(), |row| row.items_start())
            .when(style.detailed, |row| row.rounded(rems(4. / 16.)))
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
            .children(
                option
                    .leading
                    .as_ref()
                    .map(|leading| leading.draw(if style.detailed { 28. } else { 13. })),
            )
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
                            .text_size(if style.detailed {
                                theme::text_body()
                            } else {
                                theme::text_title()
                            })
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(foreground)
                            .when(style.monospace, |label| {
                                label.font_family(theme::UI_MONOSPACE_FONT)
                            })
                            .child(option.label),
                    )
                    .children(option.description.map(|description| {
                        div()
                            .text_size(theme::text_meta())
                            .when(style.mono_description, |line| {
                                line.font_family(theme::UI_MONOSPACE_FONT)
                            })
                            .text_color(if style.detailed {
                                theme::muted()
                            } else {
                                theme::faint()
                            })
                            .child(description)
                    })),
            )
            .when(active && !style.detailed, |row| {
                row.child(
                    svg()
                        .debug_selector(|| "STYLED_SELECT_CHECK".into())
                        .path("check.svg")
                        .size(rems(14. / 16.))
                        .flex_none()
                        .map(|mut check| {
                            // Stacked rows top-align their items; the check still
                            // sits on the row's centre line.
                            check.style().align_self = Some(AlignSelf::Center);
                            check
                        })
                        .text_color(if option.danger {
                            theme::danger()
                        } else {
                            theme::accent()
                        }),
                )
            })
            .when(!option.disabled, |row| {
                row.on_click(move |_, window, cx| on_choose(index, window, cx))
            })
    });
    // A list that scrolls gives its scrollbar a lane of its own, so no row,
    // highlight or check sits under the thumb. Whether it scrolls is known
    // once it is laid out; the canvas below renders once more if that changed.
    let scrolls = menu_scrolls(scroll);
    let measured = scroll.clone();
    div()
        .id("styled-select-options")
        .debug_selector(|| "STYLED_SELECT_MENU".into())
        .relative()
        .max_h(rems(260. / 16.))
        .overflow_hidden()
        .rounded(rems(if style.detailed { 6. / 16. } else { 4. / 16. }))
        .border_1()
        .border_color(if style.detailed {
            theme::border()
        } else {
            theme::border_strong()
        })
        .bg(theme::panel())
        .shadow_xl()
        .child(
            div()
                .id("styled-select-scroll")
                .debug_selector(|| "STYLED_SELECT_SCROLL".into())
                .max_h(rems(260. / 16.))
                .overflow_y_scroll()
                .scrollbar_width(px(0.))
                .track_scroll(scroll)
                .when(style.detailed, |menu| menu.p_1())
                .when(!style.detailed, |menu| menu.py_1())
                .when(scrolls, |menu| menu.pr(app_scrollbar_gutter()))
                .children(rows),
        )
        .child(
            div()
                .debug_selector(|| "STYLED_SELECT_SCROLLBAR".into())
                .absolute()
                // Inset by the list's padding, clear of the rounded top and
                // bottom.
                .top(rems(4. / 16.))
                .bottom(rems(4. / 16.))
                .right_0()
                .w(app_scrollbar_gutter())
                .child(scrollbar),
        )
        .child(
            canvas(
                move |_, window, cx| {
                    if menu_scrolls(&measured) != scrolls {
                        // The first layout initializes the viewport needed by scroll_to_item.
                        measured.scroll_to_item(highlighted);
                        rerender_after_draw(window, cx);
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_0(),
        )
        .into_any_element()
}

/// Whether the list has anything to scroll, past sub-pixel rounding.
fn menu_scrolls(scroll: &ScrollHandle) -> bool {
    scroll.max_offset().y >= px(1.)
}

/// Renders the view being drawn once more, for an element whose look depends
/// on its own layout. Mid-draw `Window::refresh` is ignored, so this notifies
/// the view once the draw's effects flush.
pub(crate) fn rerender_after_draw(window: &Window, cx: &mut App) {
    let view = window.current_view();
    cx.defer(move |cx| cx.notify(view));
}

/// Options with descriptions read best on one line each, so their menu grows
/// to fit them instead of wrapping inside the trigger's width.
pub(crate) fn option_menu_width(options: &[SelectOption], min: Pixels, zoom: f32) -> PopupWidth {
    if options.iter().any(|option| option.description.is_some()) {
        PopupWidth::Fit {
            min,
            max: px(480.) * zoom,
        }
    } else {
        PopupWidth::Fixed(min)
    }
}

pub type RuntimeSelect = StyledSelect;

pub fn runtime_select_options(catalog: &[RuntimeCatalogEntry]) -> Vec<SelectOption> {
    catalog
        .iter()
        .filter(|runtime| runtime.available)
        .map(|runtime| {
            SelectOption::new(runtime.name.key(), runtime.display_name.clone())
                .description(runtime.description.clone())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{KeyUpEvent, Keystroke, Modifiers, TestAppContext, VisualTestContext};

    struct SelectHost {
        select: Entity<StyledSelect>,
    }

    impl Render for SelectHost {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().p_4().child(self.select.clone())
        }
    }

    fn press_key(visual: &mut VisualTestContext, key: &str) {
        visual.simulate_keystrokes(key);
        visual.simulate_event(KeyUpEvent {
            keystroke: Keystroke::parse(key).unwrap(),
        });
        visual.run_until_parked();
    }

    #[test]
    fn enter_and_space_open_and_choose_once_after_key_release() {
        for key in ["enter", "space"] {
            let mut cx = TestAppContext::single();
            let choices = Rc::new(std::cell::RefCell::new(Vec::new()));
            let window = cx.add_window(|window, cx| {
                let focus_handle = cx.focus_handle();
                focus_handle.focus(window, cx);
                let select = cx.new(|cx| {
                    let choices = choices.clone();
                    StyledSelect::new(
                        "select",
                        focus_handle,
                        "a",
                        options(),
                        Rc::new(move |value, _, _| choices.borrow_mut().push(value)),
                        cx,
                    )
                });
                SelectHost { select }
            });
            cx.run_until_parked();
            let select = window
                .read_with(&cx, |host, _| host.select.clone())
                .unwrap();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);

            press_key(&mut visual, key);
            assert!(
                select.read_with(&visual, |select, _| select.state.is_open()),
                "{key}"
            );
            assert!(choices.borrow().is_empty());

            press_key(&mut visual, "down");
            press_key(&mut visual, key);
            assert!(
                !select.read_with(&visual, |select, _| select.state.is_open()),
                "{key}"
            );
            assert_eq!(
                select.read_with(&visual, |select, _| select.value().to_owned()),
                "c"
            );
            assert_eq!(&*choices.borrow(), &["c"]);

            press_key(&mut visual, key);
            assert!(select.read_with(&visual, |select, _| select.state.is_open()));
            press_key(&mut visual, "escape");
            assert!(!select.read_with(&visual, |select, _| select.state.is_open()));
            assert_eq!(&*choices.borrow(), &["c"]);
        }
    }

    #[test]
    fn keyboard_navigation_keeps_highlighted_options_in_view() {
        for (detailed, rem) in [(false, 16.), (true, 20.8)] {
            let mut cx = TestAppContext::single();
            let window = cx.add_window(|window, cx| {
                window.resize(gpui::size(px(1200.), px(900.)));
                window.set_rem_size(px(rem));
                let focus_handle = cx.focus_handle();
                focus_handle.focus(window, cx);
                let select = cx.new(|cx| {
                    StyledSelect::new(
                        "select",
                        focus_handle,
                        "0",
                        (0..20)
                            .map(|index| {
                                SelectOption::new(index.to_string(), format!("Option {index}"))
                                    .disabled(index == 10)
                            })
                            .collect(),
                        Rc::new(|_, _, _| {}),
                        cx,
                    )
                    .detailed(detailed)
                });
                SelectHost { select }
            });
            cx.run_until_parked();
            let select = window
                .read_with(&cx, |host, _| host.select.clone())
                .unwrap();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            press_key(&mut visual, "down");
            assert!(select.read_with(&visual, |select, _| select.menu_scroll_range() > px(0.)));

            for key in std::iter::repeat_n("down", 19).chain(["down", "up", "home", "end"]) {
                press_key(&mut visual, key);
                let highlighted = select.read_with(&visual, |select, _| select.state.highlighted());
                assert_ne!(highlighted, 10, "disabled options are skipped");
                let row = visual
                    .debug_bounds(Box::leak(
                        format!("STYLED_SELECT_OPTION_{highlighted}").into_boxed_str(),
                    ))
                    .unwrap();
                let viewport = visual.debug_bounds("STYLED_SELECT_SCROLL").unwrap();
                assert!(row.top() >= viewport.top() - px(1.) && row.bottom() <= viewport.bottom() + px(1.), "{detailed}/{rem}/{key}: highlighted row {highlighted} {row:?} outside {viewport:?}");
            }
            assert!(select.read_with(&visual, |select, _| select.menu_scroll.offset().y < px(0.)));
            assert_eq!(
                select.read_with(&visual, |select, _| select.value().to_owned()),
                "0"
            );
        }
    }

    #[test]
    fn opening_a_select_scrolls_to_its_current_selection() {
        for mouse in [false, true] {
            let mut cx = TestAppContext::single();
            let window = cx.add_window(|window, cx| {
                let focus_handle = cx.focus_handle();
                focus_handle.focus(window, cx);
                let select = cx.new(|cx| {
                    StyledSelect::new(
                        "select",
                        focus_handle,
                        "19",
                        (0..20)
                            .map(|index| {
                                SelectOption::new(index.to_string(), format!("Option {index}"))
                            })
                            .collect(),
                        Rc::new(|_, _, _| {}),
                        cx,
                    )
                });
                SelectHost { select }
            });
            cx.run_until_parked();
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            if mouse {
                let trigger = visual.debug_bounds("STYLED_SELECT_TRIGGER").unwrap();
                visual.simulate_click(trigger.center(), Modifiers::default());
                visual.run_until_parked();
            } else {
                press_key(&mut visual, "enter");
            }
            let row = visual.debug_bounds("STYLED_SELECT_OPTION_19").unwrap();
            let viewport = visual.debug_bounds("STYLED_SELECT_SCROLL").unwrap();
            assert!(
                row.top() >= viewport.top() - px(1.) && row.bottom() <= viewport.bottom() + px(1.),
                "mouse={mouse}: selected row {row:?} outside {viewport:?}"
            );
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

    #[test]
    fn described_options_fit_on_one_line_with_a_centered_check_and_no_scrollbar() {
        let mut cx = TestAppContext::single();
        let window = cx.add_window(|window, cx| {
            window.resize(gpui::size(px(1200.), px(900.)));
            let focus_handle = cx.focus_handle();
            let select = cx.new(|cx| {
                StyledSelect::new(
                    "select",
                    focus_handle,
                    "bypass",
                    vec![
                        SelectOption::new("bypass", "Bypass")
                            .description("Default. No prompts; codex gets full access."),
                        SelectOption::new("auto", "Auto").description(
                            "The runtime's classifier decides; may stall on a prompt.",
                        ),
                        SelectOption::new("role", "Role default")
                            .description("Whatever each role carries."),
                    ],
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
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        for rem in [16., 20.8] {
            window
                .update(&mut visual, |_, window, _| {
                    window.set_rem_size(px(rem));
                    window.refresh();
                })
                .unwrap();
            visual.run_until_parked();
            let trigger = visual.debug_bounds("STYLED_SELECT_TRIGGER").unwrap();
            visual.simulate_click(trigger.center(), Modifiers::default());
            visual.run_until_parked();

            let menu = visual.debug_bounds("STYLED_SELECT_MENU").unwrap();
            let row = visual.debug_bounds("STYLED_SELECT_OPTION_0").unwrap();
            let check = visual.debug_bounds("STYLED_SELECT_CHECK").unwrap();
            assert!(
                menu.size.width > px(240. * rem / 16.) && menu.size.width <= px(480. * rem / 16.),
                "{rem}: the menu grows to fit the descriptions: {menu:?}"
            );
            assert!(
                row.size.height < px(60. * rem / 16.),
                "{rem}: a described row stays on two lines: {row:?}"
            );
            assert!(
                (check.center().y - row.center().y).abs() <= px(1.),
                "{rem}: the check is centered in its row: {check:?} {row:?}"
            );
            let max_offset = select.read_with(&visual, |select, _| select.menu_scroll.max_offset());
            assert_eq!(max_offset.y, px(0.), "{rem}: three options never scroll");
            assert!(
                row.right() >= menu.right() - px(2.),
                "{rem}: with nothing to scroll no scrollbar lane is kept: {row:?} in {menu:?}"
            );

            visual.simulate_click(trigger.center(), Modifiers::default());
            visual.run_until_parked();
        }
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
            name: Runtime::Codex,
            capabilities: runner_backend::ops::runtime::RuntimeCatalogEntry::for_runtime(
                Runtime::Codex,
            )
            .map(|entry| entry.capabilities)
            .unwrap_or_default(),
            display_name: "Codex".into(),
            command: "codex".into(),
            native_fork: true,
            description: "OpenAI Codex CLI".into(),
            install_url: String::new(),
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
        let mut copilot = catalog[0].clone();
        copilot.name = runner_backend::model::Runtime::Copilot;
        copilot.display_name = "GitHub Copilot CLI".into();
        copilot.command = "copilot".into();
        catalog.push(copilot);
        let mut pi = catalog[0].clone();
        pi.name = runner_backend::model::Runtime::Pi;
        pi.display_name = "pi".into();
        pi.command = "pi".into();
        catalog.push(pi);
        let mut agy = catalog[0].clone();
        agy.name = runner_backend::model::Runtime::Antigravity;
        agy.display_name = "Antigravity CLI".into();
        agy.command = "agy".into();
        catalog.push(agy);
        assert_eq!(
            runtime_select_options(&catalog)
                .iter()
                .map(|entry| entry.value.as_str())
                .collect::<Vec<_>>(),
            ["codex", "copilot", "pi", "antigravity"]
        );
    }
}
