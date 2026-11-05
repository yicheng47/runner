//! The side panel's views: what each one declares, the context it renders
//! from, and the header that switches between them.

use std::rc::Rc;

use gpui::{div, prelude::*, rems, svg, App, Div, ElementId, FontWeight, MouseButton};
use runner_app::ui::Tooltip;

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
        surfaces: &[PanelSurface::Mission],
    },
    PanelView {
        id: PanelViewId::Info,
        label: "ui.info",
        icon: "info.svg",
        surfaces: &[PanelSurface::Chat, PanelSurface::Mission],
    },
    PanelView {
        id: PanelViewId::Files,
        label: "ui.files",
        icon: "folder.svg",
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

/// The view a panel shows: the one chosen when the surface has it, else the
/// surface's first.
pub(crate) fn shown_view(
    surface: PanelSurface,
    settings: &AppSettings,
    chosen: PanelViewId,
) -> Option<PanelViewId> {
    let mut available = views(surface, settings).map(|view| view.id).peekable();
    let first = available.peek().copied();
    available.find(|id| *id == chosen).or(first)
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

pub(crate) type SelectView = Rc<dyn Fn(PanelViewId, &mut App)>;

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

/// The header's row (option H, Jason, 2026-11-05): the open view's name on
/// the left; on the right the surface's views as bare icon buttons, the open
/// one raised, then `collapse`. A surface with one view shows no buttons.
pub(crate) fn header_views(
    surface: PanelSurface,
    settings: &AppSettings,
    shown: Option<PanelViewId>,
    select: SelectView,
    collapse: impl IntoElement,
) -> Div {
    let available = views(surface, settings).collect::<Vec<_>>();
    let title = available.iter().find(|view| shown == Some(view.id));
    let buttons = (available.len() > 1).then(|| {
        available
            .iter()
            .map(|view| {
                let select = select.clone();
                let id = view.id;
                Tooltip::new(
                    ElementId::Name(format!("side-panel-view-tooltip-{id:?}").into()),
                    view.label(),
                    view_button(view, shown == Some(id)).on_click(move |_, _, cx| select(id, cx)),
                )
                .into_any_element()
            })
            .collect::<Vec<_>>()
    });
    div()
        .flex_1()
        .min_w(gpui::px(0.))
        .flex()
        .items_center()
        .gap_2()
        .child(
            div()
                .flex_1()
                .min_w(gpui::px(0.))
                .truncate()
                .text_size(rems(13. / 16.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme::text())
                .map(|element| match title {
                    Some(view) => record_title(view, element).child(view.label()),
                    None => element,
                }),
        )
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(rems(4. / 16.))
                .children(buttons.into_iter().flatten())
                .child(collapse),
        )
}

fn view_element_id(view: &PanelView) -> ElementId {
    ElementId::Name(format!("side-panel-view-{:?}", view.id).into())
}

fn stop_titlebar_drag(button: gpui::Stateful<Div>) -> gpui::Stateful<Div> {
    button.on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

#[cfg_attr(not(test), allow(unused_variables))]
fn record(view: &PanelView, button: gpui::Stateful<Div>) -> gpui::Stateful<Div> {
    #[cfg(test)]
    let button = {
        let id = match view.id {
            PanelViewId::Sessions => "PANEL_VIEW Sessions",
            PanelViewId::Info => "PANEL_VIEW Info",
            PanelViewId::Files => "PANEL_VIEW Files",
        };
        crate::theme_snapshot::record_fill(id, button)
    };
    button
}

#[cfg_attr(not(test), allow(unused_variables))]
fn record_ink(view: &PanelView, ink: Div) -> Div {
    #[cfg(test)]
    let ink = {
        let id = match view.id {
            PanelViewId::Sessions => "PANEL_VIEW_INK Sessions",
            PanelViewId::Info => "PANEL_VIEW_INK Info",
            PanelViewId::Files => "PANEL_VIEW_INK Files",
        };
        crate::theme_snapshot::record_text(id, ink)
    };
    ink
}

#[cfg_attr(not(test), allow(unused_variables))]
fn record_title(view: &PanelView, title: Div) -> Div {
    #[cfg(test)]
    let title = {
        let id = match view.id {
            PanelViewId::Sessions => "SIDE_PANEL_TITLE Sessions",
            PanelViewId::Info => "SIDE_PANEL_TITLE Info",
            PanelViewId::Files => "SIDE_PANEL_TITLE Files",
        };
        crate::theme_snapshot::record_text(id, title)
    };
    title
}

/// A view's 24 px icon button: raised with a bright icon while open.
fn view_button(view: &PanelView, active: bool) -> gpui::Stateful<Div> {
    let group = format!("side-panel-view-{:?}", view.id);
    let ink = if active {
        theme::text()
    } else {
        theme::faint()
    };
    stop_titlebar_drag(div().id(view_element_id(view)))
        .group(group.clone())
        .flex_none()
        .size(rems(24. / 16.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .cursor_pointer()
        .when(active, |button| button.bg(theme::raised()))
        .hover(|button| button.bg(theme::raised()))
        .map(|button| record(view, button))
        .child(
            div()
                .text_color(ink)
                .map(|element| record_ink(view, element))
                .child(
                    svg()
                        .flex_none()
                        .path(view.icon)
                        .size(rems(13. / 16.))
                        .text_color(ink)
                        .group_hover(group, |icon| icon.text_color(theme::text())),
                ),
        )
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
        assert_eq!(shown_view(Chat, &off, Info), None);
        assert_eq!(shown_view(Mission, &off, Files), Some(Sessions));
        let files_only = AppSettings {
            info_view: false,
            ..AppSettings::default()
        };
        assert_eq!(shown_view(Chat, &files_only, Info), Some(Files));
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

    #[test]
    fn a_panel_shows_the_chosen_view_or_its_surface_first() {
        use PanelSurface::*;
        use PanelViewId::*;
        let settings = AppSettings::default();
        assert_eq!(shown_view(Mission, &settings, Sessions), Some(Sessions));
        assert_eq!(shown_view(Mission, &settings, Files), Some(Files));
        assert_eq!(shown_view(Chat, &settings, Sessions), Some(Info));
        assert_eq!(shown_view(Chat, &settings, Files), Some(Files));
        assert_eq!(shown_view(Terminal, &settings, Info), Some(Files));
    }
}
