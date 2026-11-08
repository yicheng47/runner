//! The side panel's surfaces: what each one declares, the context it
//! renders from, and the header that holds the open ones as tabs with a `+`
//! menu of the rest.

use std::cell::Cell;
use std::rc::Rc;

use gpui::{
    canvas, div, prelude::*, px, rems, svg, AnyElement, App, Bounds, Div, ElementId, FocusHandle,
    FontWeight, KeyDownEvent, MouseButton, Pixels, Window,
};
use runner_app::ui::menu::popup_layer;
use runner_app::ui::{IconButton, IconButtonSize};

use crate::app_settings::AppSettings;
use crate::theme;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub(crate) enum PanelViewId {
    #[default]
    Sessions,
    Info,
    Files,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PanelSurface {
    Chat,
    /// A tab holding only terminals.
    Terminal,
    Mission,
}

pub(crate) struct PanelView {
    pub(crate) id: PanelViewId,
    label: &'static str,
    pub(crate) icon: &'static str,
    /// The letter that opens it from the `+` menu or the empty list.
    pub(crate) key: Option<&'static str>,
    surfaces: &'static [PanelSurface],
}

impl PanelView {
    pub(crate) fn label(&self) -> &'static str {
        crate::i18n::text(self.label)
    }

    /// Whether Settings → Side panel turns the view on; a mission's
    /// Sessions always is.
    pub(crate) fn on(&self, settings: &AppSettings) -> bool {
        match self.id {
            PanelViewId::Sessions => true,
            PanelViewId::Info => settings.info_view,
            PanelViewId::Files => settings.files_view,
        }
    }
}

pub(crate) const VIEWS: [PanelView; 3] = [
    PanelView {
        id: PanelViewId::Sessions,
        label: "ui.sessions",
        icon: "users.svg",
        key: None,
        surfaces: &[PanelSurface::Mission],
    },
    PanelView {
        id: PanelViewId::Info,
        label: "ui.info",
        icon: "info.svg",
        key: Some("I"),
        surfaces: &[PanelSurface::Chat, PanelSurface::Mission],
    },
    PanelView {
        id: PanelViewId::Files,
        label: "ui.files",
        icon: "folder.svg",
        key: Some("F"),
        surfaces: &[
            PanelSurface::Chat,
            PanelSurface::Terminal,
            PanelSurface::Mission,
        ],
    },
];

pub(crate) fn views(
    surface: PanelSurface,
    settings: &AppSettings,
) -> impl Iterator<Item = &'static PanelView> + '_ {
    VIEWS
        .iter()
        .filter(move |view| view.on(settings) && view.surfaces.contains(&surface))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum PanelSubject {
    Session(String),
    Mission(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PanelContext {
    pub(crate) subject: PanelSubject,
    pub(crate) surface: PanelSurface,
    pub(crate) cwd: Option<String>,
    pub(crate) project: Option<String>,
    pub(crate) remote: bool,
}

/// Beside a panel widened by an open file, the chat or the mission with its
/// terminal drawer keeps at least this much.
pub(crate) const MAIN_MIN_WIDTH: f32 = 420.;
/// From this width the mission's Sessions roster places two cards per row.
pub(crate) const ROSTER_TWO_UP_WIDTH: f32 = 560.;

/// A panel widened by an open file: the file's width, whether the tree
/// beside it is on, the tree's width, and the panel's standard width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Widened {
    pub(crate) file: f32,
    pub(crate) tree: bool,
    pub(crate) tree_width: f32,
    pub(crate) standard: f32,
}

impl Widened {
    pub(crate) fn width(self) -> f32 {
        self.file + if self.tree { self.tree_width } else { 0. }
    }

    /// The file's and the tree's widths when the divider between them is
    /// dragged to leave the tree `tree` wide: the panel keeps its width,
    /// both stay within their bounds and the file never narrows below the
    /// standard width.
    pub(crate) fn resize_tree(self, tree: f32) -> (f32, f32) {
        use crate::app_settings::{clamp_file_tree_width, FILE_VIEWER_MAX};
        let total = self.file + self.tree_width;
        let tree = clamp_file_tree_width(tree)
            .max(total - FILE_VIEWER_MAX)
            .min(total - self.standard);
        (total - tree, tree)
    }
}

/// How a panel showing a file of width `file` beside a tree `tree_width`
/// wide fits in `available`, the width it shares with the chat or mission:
/// the file narrows first, never below the panel's `standard` width, and a
/// tree that is turned on always shows (Jason, 2026-11-05).
pub(crate) fn widened(
    available: f32,
    standard: f32,
    file: f32,
    tree_width: f32,
    tree: bool,
) -> Widened {
    let room = available - MAIN_MIN_WIDTH - if tree { tree_width } else { 0. };
    let file = file.max(standard);
    Widened {
        file: room.clamp(standard, file),
        tree,
        tree_width,
        standard,
    }
}

/// The surfaces a panel has opened as tabs, in the order they opened, and
/// the one it shows; kept where the panel keeps its state.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct OpenSurfaces {
    open: Vec<PanelViewId>,
    active: Option<PanelViewId>,
}

impl OpenSurfaces {
    /// Opens `id` as a tab once, and switches to it.
    pub(crate) fn open(&mut self, id: PanelViewId) {
        if id != PanelViewId::Sessions && !self.open.contains(&id) {
            self.open.push(id);
        }
        self.active = Some(id);
    }

    pub(crate) fn select(&mut self, id: PanelViewId) {
        self.active = Some(id);
    }

    /// Closes `id`'s tab; when it was showing, the tab before it shows:
    /// the permanent one of a panel with one, else the previous, else the
    /// next, else none.
    pub(crate) fn close(&mut self, id: PanelViewId, permanent: bool) {
        let Some(index) = self.open.iter().position(|open| *open == id) else {
            return;
        };
        self.open.remove(index);
        if self.active == Some(id) {
            self.active = match index.checked_sub(1) {
                Some(before) => self.open.get(before).copied(),
                None if permanent => None,
                None => self.open.first().copied(),
            };
        }
    }

    /// Closes the tabs of the surfaces Settings turned off.
    pub(crate) fn close_off(&mut self, settings: &AppSettings, permanent: bool) {
        let off = VIEWS
            .iter()
            .filter(|view| !view.on(settings))
            .map(|view| view.id)
            .filter(|id| self.open.contains(id))
            .collect::<Vec<_>>();
        for id in off {
            self.close(id, permanent);
        }
    }

    #[cfg(test)]
    pub(crate) fn ids(&self) -> (&[PanelViewId], Option<PanelViewId>) {
        (&self.open, self.active)
    }
}

/// An inactive tab's panel; an unset open state follows the setting until shown.
#[derive(Clone, Debug, Default)]
pub(crate) struct SavedPanel {
    pub(crate) surfaces: OpenSurfaces,
    pub(crate) open: Option<bool>,
}

/// The surfaces `surface` offers to open: those Settings turns on, a
/// mission's Sessions roster aside.
fn offered(surface: PanelSurface, settings: &AppSettings) -> Vec<&'static PanelView> {
    views(surface, settings)
        .filter(|view| view.id != PanelViewId::Sessions)
        .collect()
}

/// The tab that cannot close: a mission's Sessions, or the one surface on
/// offer.
fn permanent(surface: PanelSurface, settings: &AppSettings) -> Option<PanelViewId> {
    if surface == PanelSurface::Mission {
        return Some(PanelViewId::Sessions);
    }
    match offered(surface, settings).as_slice() {
        [only] => Some(only.id),
        _ => None,
    }
}

/// What the `+` menu and the empty list offer; nothing where a surface's
/// one tab is permanent.
pub(crate) fn menu_views(surface: PanelSurface, settings: &AppSettings) -> Vec<&'static PanelView> {
    let offered = offered(surface, settings);
    if surface != PanelSurface::Mission && offered.len() < 2 {
        return Vec::new();
    }
    offered
}

/// The header's tabs, each with whether it closes: the permanent one
/// first, then the open surfaces on offer.
pub(crate) fn tabs(
    surface: PanelSurface,
    settings: &AppSettings,
    surfaces: &OpenSurfaces,
) -> Vec<(PanelViewId, bool)> {
    let permanent = permanent(surface, settings);
    let offered = offered(surface, settings);
    permanent
        .map(|id| (id, false))
        .into_iter()
        .chain(
            surfaces
                .open
                .iter()
                .filter(|id| Some(**id) != permanent && offered.iter().any(|view| view.id == **id))
                .map(|id| (*id, true)),
        )
        .collect()
}

/// The surface a panel shows: the active tab, else the first; `None` for
/// the empty list.
pub(crate) fn shown(
    surface: PanelSurface,
    settings: &AppSettings,
    surfaces: &OpenSurfaces,
) -> Option<PanelViewId> {
    let tabs = tabs(surface, settings, surfaces);
    surfaces
        .active
        .filter(|active| tabs.iter().any(|(id, _)| id == active))
        .or_else(|| tabs.first().map(|(id, _)| *id))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SurfaceAction {
    Select(PanelViewId),
    Open(PanelViewId),
    Close(PanelViewId),
    ToggleMenu,
    DismissMenu,
    HighlightMenu(usize),
    HighlightList(usize),
    FocusList,
}

pub(crate) type SurfaceHandler = Rc<dyn Fn(SurfaceAction, &mut Window, &mut App)>;

/// A panel's surfaces and its `+` menu: whether it is open, the rows the
/// keyboard highlights there and in the empty list, and their focus.
pub(crate) struct PanelSurfaces {
    pub(crate) open: OpenSurfaces,
    menu_open: bool,
    menu_highlighted: usize,
    list_highlighted: usize,
    menu_focus: FocusHandle,
    list_focus: FocusHandle,
    return_focus: Option<FocusHandle>,
    anchor: Rc<Cell<Option<Bounds<Pixels>>>>,
}

impl PanelSurfaces {
    pub(crate) fn new(open: OpenSurfaces, cx: &mut App) -> Self {
        Self {
            open,
            menu_open: false,
            menu_highlighted: 0,
            list_highlighted: 0,
            menu_focus: cx.focus_handle(),
            list_focus: cx.focus_handle(),
            return_focus: None,
            anchor: Rc::default(),
        }
    }

    /// Starts over on `open`, with the menu closed.
    pub(crate) fn reset(&mut self, open: OpenSurfaces) {
        self.open = open;
        self.menu_open = false;
        self.return_focus = None;
        self.list_highlighted = 0;
    }

    pub(crate) fn list_focus(&self) -> &FocusHandle {
        &self.list_focus
    }

    pub(crate) fn apply(
        &mut self,
        surface: PanelSurface,
        settings: &AppSettings,
        action: SurfaceAction,
        window: &mut Window,
        cx: &mut App,
    ) {
        match action {
            SurfaceAction::Select(id) => self.open.select(id),
            SurfaceAction::Open(id) => {
                self.close_menu(window, cx);
                self.open.open(id);
            }
            SurfaceAction::Close(id) => {
                self.open.close(id, surface == PanelSurface::Mission);
                if shown(surface, settings, &self.open).is_none() {
                    self.list_highlighted = 0;
                    self.list_focus.focus(window, cx);
                }
            }
            SurfaceAction::ToggleMenu if self.menu_open => self.close_menu(window, cx),
            SurfaceAction::ToggleMenu => {
                self.return_focus = window.focused(cx);
                self.menu_open = true;
                self.menu_highlighted = 0;
                self.menu_focus.focus(window, cx);
            }
            SurfaceAction::DismissMenu => self.close_menu(window, cx),
            SurfaceAction::HighlightMenu(index) => self.menu_highlighted = index,
            SurfaceAction::HighlightList(index) => self.list_highlighted = index,
            SurfaceAction::FocusList => self.list_focus.focus(window, cx),
        }
    }

    fn close_menu(&mut self, window: &mut Window, cx: &mut App) {
        if !self.menu_open {
            return;
        }
        self.menu_open = false;
        if let Some(focus) = self.return_focus.take() {
            if self.menu_focus.is_focused(window) {
                focus.focus(window, cx);
            }
        }
    }
}

/// What a key pressed in the menu or the empty list does: a surface's
/// letter opens it, the arrows move the highlight, Enter opens the
/// highlighted row and Esc closes the menu.
fn surface_key(
    event: &KeyDownEvent,
    rows: &[&'static PanelView],
    highlighted: usize,
    menu: bool,
) -> Option<SurfaceAction> {
    let keystroke = &event.keystroke;
    let modifiers = keystroke.modifiers;
    if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
        return None;
    }
    let highlight = |index| {
        if menu {
            SurfaceAction::HighlightMenu(index)
        } else {
            SurfaceAction::HighlightList(index)
        }
    };
    let last = rows.len().checked_sub(1)?;
    match keystroke.key.as_str() {
        "escape" if menu => Some(SurfaceAction::DismissMenu),
        "down" => Some(highlight(if highlighted >= last {
            0
        } else {
            highlighted + 1
        })),
        "up" => Some(highlight(
            highlighted.checked_sub(1).unwrap_or(last).min(last),
        )),
        "enter" | "space" => rows
            .get(highlighted)
            .map(|view| SurfaceAction::Open(view.id)),
        key => rows
            .iter()
            .find(|view| {
                view.key
                    .is_some_and(|letter| letter.eq_ignore_ascii_case(key))
            })
            .map(|view| SurfaceAction::Open(view.id)),
    }
}

/// The header (Jason, 2026-11-05, after t3code): the open surfaces as tabs,
/// the shown one raised with a × where it closes, then `+` for the surface
/// menu, then `collapse` on the right.
pub(crate) fn header_tabs(
    surface: PanelSurface,
    settings: &AppSettings,
    surfaces: &OpenSurfaces,
    (menu, window): (&PanelSurfaces, &Window),
    handler: SurfaceHandler,
    collapse: impl IntoElement,
) -> Div {
    let shown = shown(surface, settings, surfaces);
    let rows = menu_views(surface, settings);
    let tabs = tabs(surface, settings, surfaces)
        .into_iter()
        .filter_map(|(id, closable)| {
            let view = VIEWS.iter().find(|view| view.id == id)?;
            Some(surface_tab(view, shown == Some(id), closable, handler.clone()).into_any_element())
        })
        .collect::<Vec<_>>();
    let add =
        (!rows.is_empty()).then(|| {
            let toggle = handler.clone();
            let anchor = menu.anchor.clone();
            div()
                .relative()
                .flex_none()
                .debug_selector(|| "PANEL_ADD_SURFACE".into())
                .child(
                    IconButton::new("side-panel-add-surface", "plus.svg")
                        .size(IconButtonSize::Sm)
                        .tooltip(crate::i18n::text("ui.open_a_surface"))
                        .on_press(move |window, cx| toggle(SurfaceAction::ToggleMenu, window, cx)),
                )
                .child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, _, _| anchor.set(Some(bounds)),
                    )
                    .absolute()
                    .inset_0(),
                )
                .children(menu.anchor.get().filter(|_| menu.menu_open).map(|anchor| {
                    surface_menu(&rows, surfaces, menu, anchor, handler.clone(), window)
                }))
        });
    // Tabs shrink, their names first, so `+` and collapse always show.
    div()
        .flex_1()
        .min_w(px(0.))
        .flex()
        .items_center()
        .gap(rems(4. / 16.))
        .child(
            div()
                .min_w(px(0.))
                .overflow_hidden()
                .flex()
                .items_center()
                .gap(rems(4. / 16.))
                .children(tabs),
        )
        .children(add)
        .child(div().flex_1().min_w(px(0.)).h_full())
        .child(
            div()
                .flex_none()
                .debug_selector(|| "PANEL_COLLAPSE".into())
                .child(collapse),
        )
}

fn view_element_id(prefix: &str, view: &PanelView) -> ElementId {
    ElementId::Name(format!("side-panel-{prefix}-{:?}", view.id).into())
}

fn stop_titlebar_drag(button: gpui::Stateful<Div>) -> gpui::Stateful<Div> {
    button.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

#[cfg_attr(not(test), allow(unused_variables))]
fn record(
    prefix: &'static str,
    view: &PanelView,
    element: gpui::Stateful<Div>,
) -> gpui::Stateful<Div> {
    #[cfg(test)]
    let element = {
        let id: &'static str = format!("{prefix} {:?}", view.id).leak();
        crate::theme_snapshot::record_fill(id, element)
    };
    element
}

#[cfg_attr(not(test), allow(unused_variables))]
fn record_ink(prefix: &'static str, view: &PanelView, ink: Div) -> Div {
    #[cfg(test)]
    let ink = {
        let id: &'static str = format!("{prefix} {:?}", view.id).leak();
        crate::theme_snapshot::record_text(id, ink)
    };
    ink
}

const TAB_PADDING: f32 = 8.;
const TAB_GAP: f32 = 6.;

/// A surface's 28 px tab: its icon and name, raised, semibold and with a ×
/// while shown, the × only where it closes.
fn surface_tab(
    view: &PanelView,
    active: bool,
    closable: bool,
    handler: SurfaceHandler,
) -> gpui::Stateful<Div> {
    let id = view.id;
    let ink = if active {
        theme::text()
    } else {
        theme::muted()
    };
    let select = handler.clone();
    // Short of room the names truncate; the shown tab keeps its icon and ×.
    let floor = match (active, closable) {
        (true, true) => TAB_PADDING * 2. + 14. + TAB_GAP * 2. + 16.,
        (true, false) => TAB_PADDING * 2. + 14.,
        (false, _) => 0.,
    };
    stop_titlebar_drag(div().id(view_element_id("tab", view)))
        .flex_shrink(1.)
        .min_w(rems(floor / 16.))
        .overflow_hidden()
        .h(rems(28. / 16.))
        .px(rems(TAB_PADDING / 16.))
        .flex()
        .items_center()
        .gap(rems(TAB_GAP / 16.))
        .rounded_md()
        .cursor_pointer()
        .when(active, |tab| tab.bg(theme::raised()))
        .when(!active, |tab| {
            tab.hover(|tab| tab.bg(theme::with_alpha(theme::raised(), 0.6)))
        })
        .map(|tab| record("PANEL_TAB", view, tab))
        .on_click(move |_, window, cx| select(SurfaceAction::Select(id), window, cx))
        .child(
            svg()
                .flex_none()
                .path(view.icon)
                .size(rems(14. / 16.))
                .text_color(ink),
        )
        .child(
            div()
                .min_w(px(0.))
                .truncate()
                .text_size(rems(13. / 16.))
                .when(active, |label| label.font_weight(FontWeight::SEMIBOLD))
                .text_color(ink)
                .map(|label| record_ink("PANEL_TAB_INK", view, label))
                .child(view.label()),
        )
        .when(active && closable, |tab| {
            tab.child(
                div()
                    .id(view_element_id("tab-close", view))
                    .debug_selector(move || format!("PANEL_TAB_CLOSE {id:?}"))
                    .flex_none()
                    .size(rems(1.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .cursor_pointer()
                    .hover(|close| close.bg(theme::with_alpha(theme::text(), 0.08)))
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        handler(SurfaceAction::Close(id), window, cx);
                    })
                    .child(
                        svg()
                            .path("close.svg")
                            .size(rems(12. / 16.))
                            .text_color(theme::muted()),
                    ),
            )
        })
}

/// A surface's letter in a keycap.
pub(crate) fn keycap(letter: &'static str) -> Div {
    div()
        .flex_none()
        .size(rems(20. / 16.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(rems(4. / 16.))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg())
        .font_family(theme::UI_MONOSPACE_FONT)
        .text_size(rems(11. / 16.))
        .text_color(theme::muted())
        .child(letter)
}

/// One row of the menu or the empty list: the surface's icon and name, a
/// check mark when it is open (the menu only), and its key.
fn surface_row(
    prefix: &'static str,
    view: &'static PanelView,
    index: usize,
    highlighted: bool,
    checked: Option<bool>,
    handler: SurfaceHandler,
    menu: bool,
) -> gpui::Stateful<Div> {
    let id = view.id;
    let hover = handler.clone();
    div()
        .id(view_element_id(prefix, view))
        .w_full()
        .h(rems(32. / 16.))
        .px(rems(10. / 16.))
        .flex()
        .items_center()
        .gap(rems(10. / 16.))
        .rounded(rems(6. / 16.))
        .cursor_pointer()
        .when(highlighted, |row| row.bg(theme::sidebar_selected()))
        .map(|row| record(prefix, view, row))
        .on_hover(move |hovered, window, cx| {
            if *hovered {
                hover(
                    if menu {
                        SurfaceAction::HighlightMenu(index)
                    } else {
                        SurfaceAction::HighlightList(index)
                    },
                    window,
                    cx,
                );
            }
        })
        .on_click(move |_, window, cx| handler(SurfaceAction::Open(id), window, cx))
        .child(
            svg()
                .flex_none()
                .path(view.icon)
                .size(rems(14. / 16.))
                .text_color(theme::muted()),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .truncate()
                .text_size(theme::text_body())
                .text_color(theme::text())
                .child(view.label()),
        )
        .children(checked.filter(|checked| *checked).map(|_| {
            svg()
                .flex_none()
                .debug_selector(move || format!("PANEL_MENU_CHECK {id:?}"))
                .path("check.svg")
                .size(rems(14. / 16.))
                .text_color(theme::muted())
        }))
        .children(view.key.map(keycap))
}

fn surface_menu(
    rows: &[&'static PanelView],
    surfaces: &OpenSurfaces,
    menu: &PanelSurfaces,
    anchor: Bounds<Pixels>,
    handler: SurfaceHandler,
    window: &Window,
) -> AnyElement {
    let key_rows = rows.to_vec();
    let highlighted = menu.menu_highlighted.min(rows.len().saturating_sub(1));
    let keys = handler.clone();
    let content = div()
        .id("side-panel-surface-menu")
        .track_focus(&menu.menu_focus)
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if let Some(action) = surface_key(event, &key_rows, highlighted, true) {
                cx.stop_propagation();
                keys(action, window, cx);
            }
        })
        .rounded(rems(8. / 16.))
        .border_1()
        .border_color(theme::border())
        .bg(theme::raised())
        .shadow_2xl()
        .p(rems(6. / 16.))
        .flex()
        .flex_col()
        .gap(rems(1. / 16.))
        .map(|menu| {
            #[cfg(test)]
            let menu = crate::theme_snapshot::record_fill("PANEL_MENU", menu);
            menu
        })
        .children(rows.iter().enumerate().map(|(index, view)| {
            surface_row(
                "PANEL_MENU_ROW",
                view,
                index,
                index == highlighted,
                Some(surfaces.open.contains(&view.id)),
                handler.clone(),
                true,
            )
        }))
        .into_any_element();
    let dismiss = handler;
    popup_layer(
        anchor,
        window,
        px(MENU_WIDTH) * (window.rem_size() / px(16.)),
        content,
        Rc::new(move |window, cx| dismiss(SurfaceAction::DismissMenu, window, cx)),
    )
}

const MENU_WIDTH: f32 = 240.;

/// A chat's panel with nothing open: "Open a surface" over the surfaces on
/// offer and their keys, which act while the list has focus.
pub(crate) fn empty_list(
    surface: PanelSurface,
    settings: &AppSettings,
    surfaces: &PanelSurfaces,
    handler: SurfaceHandler,
) -> AnyElement {
    let rows = menu_views(surface, settings);
    let highlighted = surfaces.list_highlighted.min(rows.len().saturating_sub(1));
    let key_rows = rows.clone();
    let keys = handler.clone();
    let focus = handler.clone();
    div()
        .id("side-panel-empty")
        .debug_selector(|| "PANEL_EMPTY".into())
        .track_focus(&surfaces.list_focus)
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if let Some(action) = surface_key(event, &key_rows, highlighted, false) {
                cx.stop_propagation();
                keys(action, window, cx);
            }
        })
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            focus(SurfaceAction::FocusList, window, cx)
        })
        .flex_1()
        .min_h(px(0.))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .px_5()
        .child(
            div()
                .w_full()
                .max_w(rems(260. / 16.))
                .flex()
                .flex_col()
                .items_center()
                .gap(rems(4. / 16.))
                .child(
                    div()
                        .pb(rems(12. / 16.))
                        .text_size(rems(15. / 16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::text())
                        .child(crate::i18n::text("ui.open_a_surface")),
                )
                .children(rows.iter().enumerate().map(|(index, view)| {
                    surface_row(
                        "PANEL_EMPTY_ROW",
                        view,
                        index,
                        index == highlighted,
                        None,
                        handler.clone(),
                        false,
                    )
                }))
                .child(
                    div()
                        .pt(rems(1.))
                        .text_size(theme::text_ui())
                        .text_color(theme::faint())
                        .child(crate::i18n::text("ui.surface_letter_hint")),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(surface: PanelSurface) -> Vec<PanelViewId> {
        ids_with(&AppSettings::default(), surface)
    }

    fn ids_with(settings: &AppSettings, surface: PanelSurface) -> Vec<PanelViewId> {
        views(surface, settings).map(|view| view.id).collect()
    }

    #[test]
    fn views_keep_one_order_and_filter_by_surface() {
        use PanelSurface::*;
        use PanelViewId::*;
        assert_eq!(ids(Mission), [Sessions, Info, Files]);
        assert_eq!(ids(Chat), [Info, Files]);
        assert_eq!(ids(Terminal), [Files]);
        assert!(VIEWS.iter().all(|view| view.on(&AppSettings::default())));
    }

    #[test]
    fn settings_turn_info_and_files_off_but_never_sessions() {
        use PanelSurface::*;
        use PanelViewId::*;
        let off = AppSettings {
            info_view: false,
            files_view: false,
            ..AppSettings::default()
        };
        assert_eq!(ids_with(&off, Mission), [Sessions]);
        assert_eq!(views(Chat, &off).count(), 0);
        assert_eq!(views(Terminal, &off).count(), 0);
        let files_only = AppSettings {
            info_view: false,
            ..AppSettings::default()
        };
        assert_eq!(ids_with(&files_only, Mission), [Sessions, Files]);
    }

    #[test]
    fn a_widened_panel_narrows_the_file_first_and_keeps_the_tree() {
        let fit = |available| widened(available, 320., 440., 240., true);
        let laid = |file| Widened {
            file,
            tree: true,
            tree_width: 240.,
            standard: 320.,
        };
        // 420 + 440 + 240: everything fits.
        assert_eq!(fit(1100.), laid(440.));
        assert_eq!(fit(1100.).width(), 680.);
        // Short of room, the file narrows and the tree stays.
        assert_eq!(fit(1099.), laid(439.));
        assert_eq!(fit(1000.).width() + MAIN_MIN_WIDTH, 1000.);
        // The file stops at the standard width; the tree still shows.
        assert_eq!(fit(980.), laid(320.));
        assert_eq!(fit(600.), laid(320.));
        // A tree turned off gives its room to the file.
        let off = widened(800., 320., 440., 240., false);
        assert!(!off.tree);
        assert_eq!(off.file, 380.);
        assert_eq!(widened(5000., 480., 440., 240., true).file, 480.);
    }

    #[test]
    fn a_wider_tree_narrows_the_file_sooner_and_still_shows() {
        let fit = |available| widened(available, 288., 440., 360., true);
        // 420 + 440 + 360: the saved tree width fits beside the file.
        assert_eq!(fit(1220.).width(), 800.);
        // Short of room, the file narrows to the standard width, the tree stays.
        assert!(fit(1219.).tree);
        assert_eq!(fit(1219.).file, 439.);
        assert_eq!(fit(1000.).file, 288.);
        assert_eq!(fit(800.).width(), 648.);
        // A narrower tree leaves the file more room.
        assert_eq!(widened(1100., 288., 440., 160., true).file, 440.);
        assert_eq!(widened(1100., 288., 440., 360., true).file, 320.);
    }

    #[test]
    fn dragging_the_tree_divider_trades_width_with_the_file() {
        let panel = widened(2000., 320., 440., 240., true);
        // Toward the file: the tree widens, the file narrows, the panel holds.
        assert_eq!(panel.resize_tree(300.), (380., 300.));
        // The file stops at the standard width.
        assert_eq!(panel.resize_tree(400.), (320., 360.));
        // Toward the edge: the tree stops at its minimum.
        assert_eq!(panel.resize_tree(100.), (520., 160.));
        // A file wide enough lets the tree reach its maximum, no further.
        let wide = widened(3000., 320., 1000., 240., true);
        assert_eq!(wide.resize_tree(900.), (760., 480.));
        // A file at its maximum gives the tree no less than the panel holds.
        let widest = widened(3000., 320., 1200., 240., true);
        assert_eq!(widest.resize_tree(160.), (1200., 240.));
        let wider = widened(3000., 320., 1100., 300., true);
        assert_eq!(wider.resize_tree(160.), (1200., 200.));
    }

    fn open(ids: &[PanelViewId], active: Option<PanelViewId>) -> OpenSurfaces {
        OpenSurfaces {
            open: ids.to_vec(),
            active,
        }
    }

    fn menu_ids(surface: PanelSurface, settings: &AppSettings) -> Vec<PanelViewId> {
        menu_views(surface, settings)
            .iter()
            .map(|view| view.id)
            .collect()
    }

    #[test]
    fn a_chat_holds_its_open_surfaces_as_closable_tabs_and_offers_the_rest() {
        use PanelSurface::*;
        use PanelViewId::*;
        let settings = AppSettings::default();
        assert_eq!(menu_ids(Chat, &settings), [Info, Files]);
        let both = open(&[Info, Files], Some(Files));
        assert_eq!(tabs(Chat, &settings, &both), [(Info, true), (Files, true)]);
        assert_eq!(shown(Chat, &settings, &both), Some(Files));
        let none = OpenSurfaces::default();
        assert!(tabs(Chat, &settings, &none).is_empty());
        assert_eq!(shown(Chat, &settings, &none), None, "the empty list");
        assert_eq!(
            VIEWS.iter().map(|view| view.key).collect::<Vec<_>>(),
            [None, Some("I"), Some("F")]
        );
    }

    #[test]
    fn a_mission_keeps_sessions_first_and_never_offers_it() {
        use PanelSurface::*;
        use PanelViewId::*;
        let settings = AppSettings::default();
        assert_eq!(menu_ids(Mission, &settings), [Info, Files]);
        let none = OpenSurfaces::default();
        assert_eq!(tabs(Mission, &settings, &none), [(Sessions, false)]);
        assert_eq!(shown(Mission, &settings, &none), Some(Sessions));
        let files = open(&[Files], Some(Files));
        assert_eq!(
            tabs(Mission, &settings, &files),
            [(Sessions, false), (Files, true)]
        );
        // A mission with one surface on offer still has `+` for it.
        let files_only = AppSettings {
            info_view: false,
            ..AppSettings::default()
        };
        assert_eq!(menu_ids(Mission, &files_only), [Files]);
        let off = AppSettings {
            info_view: false,
            files_view: false,
            ..AppSettings::default()
        };
        assert!(menu_ids(Mission, &off).is_empty());
        assert_eq!(tabs(Mission, &off, &files), [(Sessions, false)]);
        assert_eq!(shown(Mission, &off, &files), Some(Sessions));
    }

    #[test]
    fn one_surface_on_offer_is_a_permanent_tab_with_no_menu() {
        use PanelSurface::*;
        use PanelViewId::*;
        let settings = AppSettings::default();
        let info = open(&[Info], Some(Info));
        assert!(menu_ids(Terminal, &settings).is_empty());
        assert_eq!(tabs(Terminal, &settings, &info), [(Files, false)]);
        assert_eq!(shown(Terminal, &settings, &info), Some(Files));
        let files_only = AppSettings {
            info_view: false,
            ..AppSettings::default()
        };
        assert!(menu_ids(Chat, &files_only).is_empty());
        let none = OpenSurfaces::default();
        assert_eq!(tabs(Chat, &files_only, &none), [(Files, false)]);
        assert_eq!(shown(Chat, &files_only, &none), Some(Files));
        let info_only = AppSettings {
            files_view: false,
            ..AppSettings::default()
        };
        assert_eq!(tabs(Chat, &info_only, &none), [(Info, false)]);
    }

    #[test]
    fn opening_switches_to_a_surface_open_once_and_closing_shows_the_one_before() {
        use PanelViewId::*;
        let mut surfaces = OpenSurfaces::default();
        surfaces.open(Info);
        surfaces.open(Files);
        assert_eq!(surfaces.ids(), (&[Info, Files][..], Some(Files)));
        surfaces.open(Info);
        assert_eq!(surfaces.ids(), (&[Info, Files][..], Some(Info)), "once");
        surfaces.open(Sessions);
        assert_eq!(surfaces.ids(), (&[Info, Files][..], Some(Sessions)));

        // The surface before it, the next when it was first, then none.
        surfaces.select(Files);
        surfaces.close(Files, false);
        assert_eq!(surfaces.ids(), (&[Info][..], Some(Info)));
        surfaces.open(Files);
        surfaces.select(Info);
        surfaces.close(Info, false);
        assert_eq!(surfaces.ids(), (&[Files][..], Some(Files)));
        surfaces.close(Files, false);
        assert_eq!(surfaces.ids(), (&[][..], None));
        // A closed tab that was not showing leaves the shown one.
        let mut both = open(&[Info, Files], Some(Files));
        both.close(Info, false);
        assert_eq!(both.ids(), (&[Files][..], Some(Files)));
        // In a mission the tab before the first is Sessions.
        let mut mission = open(&[Info, Files], Some(Info));
        mission.close(Info, true);
        assert_eq!(mission.ids(), (&[Files][..], None));
        assert_eq!(
            shown(PanelSurface::Mission, &AppSettings::default(), &mission),
            Some(Sessions)
        );
    }

    #[test]
    fn settings_turning_a_surface_off_close_its_tab() {
        use PanelViewId::*;
        let mut surfaces = open(&[Info, Files], Some(Files));
        let files_off = AppSettings {
            files_view: false,
            ..AppSettings::default()
        };
        surfaces.close_off(&files_off, false);
        assert_eq!(surfaces.ids(), (&[Info][..], Some(Info)));
        surfaces.close_off(&AppSettings::default(), false);
        assert_eq!(surfaces.ids(), (&[Info][..], Some(Info)));
    }

    #[test]
    fn a_surface_key_opens_moves_or_closes_only_without_modifiers() {
        let rows = menu_views(PanelSurface::Chat, &AppSettings::default());
        let key = |key: &str, highlighted, menu| {
            let event = KeyDownEvent {
                keystroke: gpui::Keystroke::parse(key).unwrap(),
                is_held: false,
                prefer_character_input: false,
            };
            surface_key(&event, &rows, highlighted, menu)
        };
        use PanelViewId::*;
        use SurfaceAction::*;
        assert_eq!(key("i", 0, true), Some(Open(Info)));
        assert_eq!(key("f", 0, false), Some(Open(Files)));
        assert_eq!(key("shift-f", 0, true), Some(Open(Files)));
        assert_eq!(key("c", 0, true), None, "no Changes yet");
        assert_eq!(key("cmd-f", 0, true), None);
        assert_eq!(key("ctrl-f", 0, false), None);
        assert_eq!(key("escape", 0, true), Some(DismissMenu));
        assert_eq!(key("escape", 0, false), None);
        assert_eq!(key("down", 0, true), Some(HighlightMenu(1)));
        assert_eq!(key("down", 1, false), Some(HighlightList(0)));
        assert_eq!(key("up", 0, true), Some(HighlightMenu(1)));
        assert_eq!(key("enter", 1, true), Some(Open(Files)));
    }
}
