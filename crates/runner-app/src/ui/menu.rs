use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    anchored, canvas, deferred, div, point, px, svg, AnchoredPositionMode, AnyElement, App, Bounds,
    Context, Corner, ElementId, Entity, FocusHandle, FontWeight, KeyDownEvent, MouseButton, Pixels,
    Render, ScrollHandle, SharedString, Size, Window,
};

use crate::theme;
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
    viewport: Size<Pixels>,
    width: Pixels,
    menu: AnyElement,
    on_dismiss: DismissHandler,
) -> AnyElement {
    const GAP: f32 = 4.;
    const EDGE: f32 = 8.;
    const ESTIMATED_HEIGHT: f32 = 280.;
    let dismiss = Rc::clone(&on_dismiss);
    let width = width.min(viewport.width - px(EDGE * 2.));
    let left = anchor
        .left()
        .min(viewport.width - px(EDGE) - width)
        .max(px(EDGE));
    let space_below = viewport.height - anchor.bottom();
    let flip = space_below < px(ESTIMATED_HEIGHT) && anchor.top() > space_below;
    let top = if flip {
        anchor.top() - px(GAP)
    } else {
        anchor.bottom() + px(GAP)
    };
    deferred(
        div()
            .child(anchored().position(point(px(0.), px(0.))).child(
                div().w(viewport.width).h(viewport.height).on_mouse_down(
                    MouseButton::Left,
                    move |_, window, cx| {
                        dismiss(window, cx);
                    },
                ),
            ))
            .child(
                anchored()
                    .position(point(left, top))
                    .anchor(if flip {
                        Corner::BottomLeft
                    } else {
                        Corner::TopLeft
                    })
                    .position_mode(AnchoredPositionMode::Window)
                    .snap_to_window_with_margin(px(8.))
                    .child(
                        div()
                            .w(width)
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
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
        let entity = cx.entity();
        let click_entity = entity.clone();
        let mut root = div()
            .id(self.id.clone())
            .relative()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .child(
                IconButton::new("popover-trigger", "more-horizontal.svg")
                    .size(self.trigger_size)
                    .focus_handle(self.focus_handle.clone())
                    .tooltip("More actions")
                    .keyboard_activation(false)
                    .on_press(move |_, cx| {
                        click_entity.update(cx, |menu, cx| menu.toggle(cx));
                    }),
            )
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
                    .px(px(10.))
                    .py(px(if item.description.is_some() { 8. } else { 6. }))
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .rounded(px(4.))
                    .opacity(if item.disabled { 0.5 } else { 1. })
                    .when(active, |row| row.bg(theme::border()))
                    .when(!item.disabled, |row| {
                        row.cursor_pointer().hover(|row| row.bg(theme::border()))
                    })
                    .children(
                        item.icon
                            .map(|icon| svg().path(icon).size(px(14.)).text_color(foreground)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .font_weight(FontWeight::NORMAL)
                                    .text_color(foreground)
                                    .child(item.label),
                            )
                            .children(item.description.map(|description| {
                                div()
                                    .text_size(px(11.))
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
                .max_h(px(280.))
                .overflow_hidden()
                .rounded(px(8.))
                .border_1()
                .border_color(theme::border())
                .bg(theme::raised())
                .shadow_2xl()
                .child(
                    div()
                        .id("popover-menu-scroll")
                        .max_h(px(280.))
                        .overflow_y_scroll()
                        .scrollbar_width(px(0.))
                        .track_scroll(&self.menu_scroll)
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .p(px(6.))
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
                window.viewport_size(),
                anchor.size.width.max(self.min_width),
                menu,
                dismiss,
            ));
        }
        root
    }
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
