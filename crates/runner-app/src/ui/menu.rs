use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    anchored, canvas, deferred, div, point, px, rems, svg, AnchoredPositionMode, AnyElement, App,
    Bounds, Context, Corner, ElementId, Entity, FocusHandle, FontWeight, KeyDownEvent, MouseButton,
    Pixels, Render, ScrollHandle, SharedString, Window,
};

use crate::theme;
use crate::ui::app_zoom;
use crate::ui::button::{IconButton, IconButtonSize};
use crate::ui::scrollbar::Scrollbar;

pub type MenuHandler = Rc<dyn Fn(usize, &mut Window, &mut App)>;
pub type DismissHandler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MenuItem {
    pub label: SharedString,
    pub description: Option<SharedString>,
    pub icon: Option<SharedString>,
    pub destructive: bool,
    pub disabled: bool,
    pub separator_before: bool,
}

impl MenuItem {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            description: None,
            icon: None,
            destructive: false,
            disabled: false,
            separator_before: false,
        }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn icon(mut self, icon: impl Into<SharedString>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn destructive(mut self, destructive: bool) -> Self {
        self.destructive = destructive;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn separator_before(mut self, separator_before: bool) -> Self {
        self.separator_before = separator_before;
        self
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuKey {
    Up,
    Down,
    Home,
    End,
    Enter,
    Escape,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MenuAction {
    None,
    Activate(usize),
    Close,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MenuState {
    pub open: bool,
    pub highlighted: usize,
}

impl MenuState {
    pub fn open(&mut self, items: &[MenuItem], preferred: usize) {
        self.open = true;
        self.highlighted = enabled_at_or_after(items, preferred)
            .or_else(|| enabled_at_or_before(items, preferred))
            .unwrap_or(0);
    }

    pub fn close(&mut self) {
        self.open = false;
    }

    pub fn handle_key(&mut self, key: MenuKey, items: &[MenuItem]) -> MenuAction {
        if !self.open {
            match key {
                MenuKey::Enter | MenuKey::Down => self.open(items, 0),
                MenuKey::Up => self.open(items, items.len().saturating_sub(1)),
                _ => {}
            }
            return MenuAction::None;
        }
        match key {
            MenuKey::Up => {
                self.highlighted =
                    previous_enabled(items, self.highlighted).unwrap_or(self.highlighted);
                MenuAction::None
            }
            MenuKey::Down => {
                self.highlighted =
                    next_enabled(items, self.highlighted).unwrap_or(self.highlighted);
                MenuAction::None
            }
            MenuKey::Home => {
                self.highlighted = enabled_at_or_after(items, 0).unwrap_or(self.highlighted);
                MenuAction::None
            }
            MenuKey::End => {
                self.highlighted = enabled_at_or_before(items, items.len().saturating_sub(1))
                    .unwrap_or(self.highlighted);
                MenuAction::None
            }
            MenuKey::Enter => {
                let index = self.highlighted;
                if items.get(index).is_some_and(|item| !item.disabled) {
                    self.close();
                    MenuAction::Activate(index)
                } else {
                    MenuAction::None
                }
            }
            MenuKey::Escape => {
                self.close();
                MenuAction::Close
            }
        }
    }
}

fn enabled_at_or_after(items: &[MenuItem], index: usize) -> Option<usize> {
    items
        .iter()
        .enumerate()
        .skip(index.min(items.len()))
        .find_map(|(index, item)| (!item.disabled).then_some(index))
}

fn enabled_at_or_before(items: &[MenuItem], index: usize) -> Option<usize> {
    items
        .iter()
        .enumerate()
        .take(index.saturating_add(1))
        .rev()
        .find_map(|(index, item)| (!item.disabled).then_some(index))
}

fn next_enabled(items: &[MenuItem], current: usize) -> Option<usize> {
    enabled_at_or_after(items, current.saturating_add(1)).or_else(|| enabled_at_or_after(items, 0))
}

fn previous_enabled(items: &[MenuItem], current: usize) -> Option<usize> {
    current
        .checked_sub(1)
        .and_then(|index| enabled_at_or_before(items, index))
        .or_else(|| enabled_at_or_before(items, items.len().saturating_sub(1)))
}

pub fn popup_layer(
    anchor: Bounds<Pixels>,
    window: &Window,
    width: Pixels,
    menu: AnyElement,
    on_dismiss: DismissHandler,
) -> AnyElement {
    let viewport = window.viewport_size();
    let zoom = app_zoom(window);
    let gap = px(4. * zoom);
    let edge = px(8. * zoom);
    let estimated_height = px(280. * zoom);
    let dismiss = Rc::clone(&on_dismiss);
    let dismiss_right = Rc::clone(&on_dismiss);
    let width = width.min(viewport.width - edge * 2.);
    let left = anchor.left().min(viewport.width - edge - width).max(edge);
    let space_below = viewport.height - anchor.bottom();
    let flip = space_below < estimated_height && anchor.top() > space_below;
    let top = if flip {
        anchor.top() - gap
    } else {
        anchor.bottom() + gap
    };
    deferred(
        div()
            .child(
                anchored().position(point(px(0.), px(0.))).child(
                    div()
                        .w(viewport.width)
                        .h(viewport.height)
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            dismiss(window, cx);
                        })
                        .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                            dismiss_right(window, cx);
                        }),
                ),
            )
            .child(
                anchored()
                    .position(point(left, top))
                    .anchor(if flip {
                        Corner::BottomLeft
                    } else {
                        Corner::TopLeft
                    })
                    .position_mode(AnchoredPositionMode::Window)
                    .snap_to_window_with_margin(edge)
                    .child(
                        div()
                            .w(width)
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                            .child(menu),
                    ),
            ),
    )
    .with_priority(8)
    .into_any_element()
}

pub struct PopoverMenu {
    id: ElementId,
    focus_handle: FocusHandle,
    items: Vec<MenuItem>,
    state: MenuState,
    anchor_bounds: Option<Bounds<Pixels>>,
    min_width: Pixels,
    trigger_size: IconButtonSize,
    trigger_icon: SharedString,
    trigger_tooltip: Option<SharedString>,
    menu_scroll: ScrollHandle,
    menu_scrollbar: Entity<Scrollbar>,
    on_activate: MenuHandler,
}

impl PopoverMenu {
    pub fn new(
        id: impl Into<ElementId>,
        focus_handle: FocusHandle,
        items: Vec<MenuItem>,
        on_activate: MenuHandler,
        cx: &mut Context<Self>,
    ) -> Self {
        let menu_scroll = ScrollHandle::new();
        let owner = cx.entity_id();
        let menu_scrollbar = cx.new(|_| Scrollbar::app(menu_scroll.clone(), owner));
        Self {
            id: id.into(),
            focus_handle,
            items,
            state: MenuState::default(),
            anchor_bounds: None,
            min_width: px(160.),
            trigger_size: IconButtonSize::Md,
            trigger_icon: "more-horizontal.svg".into(),
            trigger_tooltip: Some("More actions".into()),
            menu_scroll,
            menu_scrollbar,
            on_activate,
        }
    }

    pub fn min_width(mut self, min_width: Pixels) -> Self {
        self.min_width = min_width;
        self
    }

    pub fn trigger_size(mut self, trigger_size: IconButtonSize) -> Self {
        self.trigger_size = trigger_size;
        self
    }

    pub fn trigger_icon(mut self, trigger_icon: impl Into<SharedString>) -> Self {
        self.trigger_icon = trigger_icon.into();
        self
    }

    pub fn trigger_tooltip(mut self, trigger_tooltip: impl Into<SharedString>) -> Self {
        self.trigger_tooltip = Some(trigger_tooltip.into());
        self
    }

    pub fn without_trigger_tooltip(mut self) -> Self {
        self.trigger_tooltip = None;
        self
    }

    pub fn set_items(&mut self, items: Vec<MenuItem>, cx: &mut Context<Self>) {
        self.items = items;
        if self.items.is_empty() {
            self.state.close();
        } else {
            self.state.highlighted = self.state.highlighted.min(self.items.len() - 1);
        }
        cx.notify();
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.state.close();
        cx.notify();
    }

    fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.items.is_empty() {
            return;
        }
        if self.state.open {
            self.state.close();
        } else {
            self.state.open(&self.items, 0);
        }
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = match event.keystroke.key.as_str() {
            "up" => Some(MenuKey::Up),
            "down" => Some(MenuKey::Down),
            "home" if self.state.open => Some(MenuKey::Home),
            "end" if self.state.open => Some(MenuKey::End),
            "enter" | "space" => Some(MenuKey::Enter),
            "escape" if self.state.open => Some(MenuKey::Escape),
            _ => None,
        };
        let Some(key) = key else { return };
        cx.stop_propagation();
        if let MenuAction::Activate(index) = self.state.handle_key(key, &self.items) {
            (self.on_activate)(index, window, cx);
        }
        cx.notify();
    }
}

impl Render for PopoverMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let zoom = app_zoom(window);
        let entity = cx.entity();
        let click_entity = entity.clone();
        let trigger = IconButton::new("popover-trigger", self.trigger_icon.clone())
            .size(self.trigger_size)
            .focus_handle(self.focus_handle.clone())
            .keyboard_activation(false)
            .on_press(move |_, cx| {
                click_entity.update(cx, |menu, cx| menu.toggle(cx));
            });
        let trigger = match self.trigger_tooltip.clone() {
            Some(tooltip) => trigger.tooltip(tooltip),
            None => trigger,
        };
        let mut root = div()
            .id(self.id.clone())
            .relative()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .child(trigger)
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, _, cx| {
                        entity.update(cx, |menu, _| menu.anchor_bounds = Some(bounds));
                    },
                )
                .absolute()
                .inset_0(),
            );

        if let (true, Some(anchor)) = (self.state.open, self.anchor_bounds) {
            let menu_entity = cx.entity();
            let rows = self.items.iter().cloned().enumerate().map(|(index, item)| {
                let active = self.state.highlighted == index;
                let click_entity = menu_entity.clone();
                let foreground = if item.destructive {
                    theme::danger()
                } else if item.disabled {
                    theme::faint()
                } else {
                    theme::text()
                };
                let separator_before = item.separator_before;
                let row = div()
                    .id(("popover-item", index))
                    .w_full()
                    .px(rems(10. / 16.))
                    .py(rems(if item.description.is_some() {
                        8. / 16.
                    } else {
                        6. / 16.
                    }))
                    .flex()
                    .items_center()
                    .gap(rems(10. / 16.))
                    .rounded(rems(4. / 16.))
                    .opacity(if item.disabled { 0.5 } else { 1. })
                    .when(active, |row| row.bg(theme::border()))
                    .when(!item.disabled, |row| {
                        row.cursor_pointer().hover(|row| row.bg(theme::border()))
                    })
                    .children(item.icon.map(|icon| {
                        svg()
                            .path(icon)
                            .size(rems(14. / 16.))
                            .text_color(foreground)
                    }))
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(rems(13. / 16.))
                                    .font_weight(FontWeight::NORMAL)
                                    .text_color(foreground)
                                    .child(item.label),
                            )
                            .children(item.description.map(|description| {
                                div()
                                    .text_size(rems(11. / 16.))
                                    .text_color(theme::faint())
                                    .child(description)
                            })),
                    )
                    .when(!item.disabled, |row| {
                        row.on_click(move |_, window, cx| {
                            click_entity.update(cx, |menu, cx| {
                                menu.state.close();
                                (menu.on_activate)(index, window, cx);
                                cx.notify();
                            });
                        })
                    });
                div()
                    .w_full()
                    .when(separator_before, |wrapper| {
                        wrapper.child(
                            div()
                                .mx_1()
                                .py_1()
                                .child(div().h(px(1.)).w_full().bg(theme::border())),
                        )
                    })
                    .child(row)
            });
            let menu = div()
                .id("popover-menu-items")
                .relative()
                .max_h(rems(280. / 16.))
                .overflow_hidden()
                .rounded(rems(8. / 16.))
                .border_1()
                .border_color(theme::border())
                .bg(theme::raised())
                .shadow_2xl()
                .child(
                    div()
                        .id("popover-menu-scroll")
                        .max_h(rems(280. / 16.))
                        .overflow_y_scroll()
                        .scrollbar_width(px(0.))
                        .track_scroll(&self.menu_scroll)
                        .flex()
                        .flex_col()
                        .gap(rems(1. / 16.))
                        .p(rems(6. / 16.))
                        .children(rows),
                )
                .child(self.menu_scrollbar.clone())
                .into_any_element();
            let dismiss_entity: Entity<Self> = cx.entity();
            let dismiss: DismissHandler = Rc::new(move |_, cx| {
                dismiss_entity.update(cx, |menu, cx| menu.close(cx));
            });
            root = root.child(popup_layer(
                anchor,
                window,
                anchor.size.width.max(self.min_width * zoom),
                menu,
                dismiss,
            ));
        }
        root
    }
}

pub struct ContextMenu {
    id: ElementId,
    focus_handle: FocusHandle,
    position: gpui::Point<Pixels>,
    width: Pixels,
    items: Vec<MenuItem>,
    state: MenuState,
    on_activate: MenuHandler,
    on_dismiss: DismissHandler,
}

impl ContextMenu {
    pub fn new(
        id: impl Into<ElementId>,
        focus_handle: FocusHandle,
        position: gpui::Point<Pixels>,
        items: Vec<MenuItem>,
        on_activate: MenuHandler,
        on_dismiss: DismissHandler,
    ) -> Self {
        let mut state = MenuState::default();
        state.open(&items, 0);
        Self {
            id: id.into(),
            focus_handle,
            position,
            width: px(160.),
            items,
            state,
            on_activate,
            on_dismiss,
        }
    }

    pub fn width(mut self, width: Pixels) -> Self {
        self.width = width;
        self
    }

    pub fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.state.close();
        (self.on_dismiss)(window, cx);
        cx.notify();
    }

    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.items.get(index).is_none_or(|item| item.disabled) {
            return;
        }
        self.state.close();
        (self.on_dismiss)(window, cx);
        (self.on_activate)(index, window, cx);
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = match event.keystroke.key.as_str() {
            "up" => Some(MenuKey::Up),
            "down" => Some(MenuKey::Down),
            "home" => Some(MenuKey::Home),
            "end" => Some(MenuKey::End),
            "enter" | "space" => Some(MenuKey::Enter),
            "escape" => Some(MenuKey::Escape),
            _ => None,
        };
        let Some(key) = key else { return };
        cx.stop_propagation();
        match self.state.handle_key(key, &self.items) {
            MenuAction::Activate(index) => self.activate(index, window, cx),
            MenuAction::Close => self.dismiss(window, cx),
            MenuAction::None => cx.notify(),
        }
    }
}

impl Render for ContextMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.state.open {
            return div().into_any_element();
        }
        let zoom = app_zoom(window);
        let width = self.width * zoom;
        let entity = cx.entity();
        let rows = self.items.iter().cloned().enumerate().map(|(index, item)| {
            let active = self.state.highlighted == index;
            let click_entity = entity.clone();
            let foreground = if item.destructive {
                theme::danger()
            } else if item.disabled {
                theme::faint()
            } else {
                theme::text()
            };
            let separator_before = item.separator_before;
            let row = div()
                .id(("context-menu-item", index))
                .w_full()
                .px(rems(10. / 16.))
                .py(rems(if item.description.is_some() {
                    8. / 16.
                } else {
                    6. / 16.
                }))
                .flex()
                .items_center()
                .gap(rems(10. / 16.))
                .rounded(rems(4. / 16.))
                .opacity(if item.disabled { 0.5 } else { 1. })
                .when(active, |row| row.bg(theme::border()))
                .when(!item.disabled, |row| {
                    row.cursor_pointer().hover(|row| row.bg(theme::border()))
                })
                .children(item.icon.map(|icon| {
                    svg()
                        .path(icon)
                        .size(rems(14. / 16.))
                        .text_color(foreground)
                }))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(rems(13. / 16.))
                                .font_weight(FontWeight::NORMAL)
                                .text_color(foreground)
                                .child(item.label),
                        )
                        .children(item.description.map(|description| {
                            div()
                                .text_size(rems(11. / 16.))
                                .text_color(theme::faint())
                                .child(description)
                        })),
                )
                .when(!item.disabled, |row| {
                    row.on_click(move |_, window, cx| {
                        click_entity.update(cx, |menu, cx| menu.activate(index, window, cx));
                    })
                });
            div()
                .w_full()
                .when(separator_before, |wrapper| {
                    wrapper.child(
                        div()
                            .mx_1()
                            .py_1()
                            .child(div().h(px(1.)).w_full().bg(theme::border())),
                    )
                })
                .child(row)
        });
        let menu = div()
            .id(self.id.clone())
            .track_focus(&self.focus_handle)
            .tab_index(0)
            .w(width)
            .flex()
            .flex_col()
            .gap(rems(1. / 16.))
            .p(rems(6. / 16.))
            .rounded(rems(8. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::raised())
            .shadow_2xl()
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
            .children(rows)
            .into_any_element();
        let dismiss_entity = cx.entity();
        let dismiss: DismissHandler = Rc::new(move |window, cx| {
            dismiss_entity.update(cx, |menu, cx| menu.dismiss(window, cx));
        });
        context_menu_layer(self.position, window, width, menu, dismiss)
    }
}

fn context_menu_layer(
    position: gpui::Point<Pixels>,
    window: &Window,
    width: Pixels,
    menu: AnyElement,
    on_dismiss: DismissHandler,
) -> AnyElement {
    let viewport = window.viewport_size();
    let zoom = app_zoom(window);
    let edge = px(4. * zoom);
    let left = position.x.min(viewport.width - width - edge).max(edge);
    let top = position.y.min(viewport.height - edge).max(edge);
    let dismiss_right = Rc::clone(&on_dismiss);
    deferred(
        div()
            .child(
                anchored().position(point(px(0.), px(0.))).child(
                    div()
                        .w(viewport.width)
                        .h(viewport.height)
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            on_dismiss(window, cx)
                        })
                        .on_mouse_down(MouseButton::Right, move |_, window, cx| {
                            dismiss_right(window, cx)
                        }),
                ),
            )
            .child(
                anchored()
                    .position(point(left, top))
                    .anchor(Corner::TopLeft)
                    .position_mode(AnchoredPositionMode::Window)
                    .snap_to_window_with_margin(edge)
                    .child(menu),
            ),
    )
    .with_priority(8)
    .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items() -> Vec<MenuItem> {
        vec![
            MenuItem::new("Open"),
            MenuItem::new("Unavailable").disabled(true),
            MenuItem::new("Delete").destructive(true),
        ]
    }

    #[test]
    fn keyboard_navigation_skips_disabled_items_and_wraps() {
        let items = items();
        let mut state = MenuState::default();
        state.open(&items, 0);
        assert_eq!(state.highlighted, 0);

        assert_eq!(state.handle_key(MenuKey::Down, &items), MenuAction::None);
        assert_eq!(state.highlighted, 2);
        state.handle_key(MenuKey::Down, &items);
        assert_eq!(state.highlighted, 0);
        state.handle_key(MenuKey::Up, &items);
        assert_eq!(state.highlighted, 2);
        assert_eq!(
            state.handle_key(MenuKey::Enter, &items),
            MenuAction::Activate(2)
        );
        assert!(!state.open);
    }

    #[test]
    fn escape_closes_without_activation() {
        let items = items();
        let mut state = MenuState::default();
        state.open(&items, 0);
        assert_eq!(state.handle_key(MenuKey::Escape, &items), MenuAction::Close);
        assert!(!state.open);
    }

    #[test]
    fn arrow_keys_open_at_the_nearest_edge() {
        let items = items();
        let mut state = MenuState::default();
        assert_eq!(state.handle_key(MenuKey::Up, &items), MenuAction::None);
        assert!(state.open);
        assert_eq!(state.highlighted, 2);

        state.close();
        assert_eq!(state.handle_key(MenuKey::Down, &items), MenuAction::None);
        assert_eq!(state.highlighted, 0);
    }

    #[test]
    fn enter_opens_before_it_activates() {
        let items = items();
        let mut state = MenuState::default();
        assert_eq!(state.handle_key(MenuKey::Enter, &items), MenuAction::None);
        assert!(state.open);
        assert_eq!(state.highlighted, 0);
        assert_eq!(
            state.handle_key(MenuKey::Enter, &items),
            MenuAction::Activate(0)
        );
    }
}
