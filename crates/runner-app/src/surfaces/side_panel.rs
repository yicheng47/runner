//! The side panel's views: what each one declares, the context it renders
//! from, and the header that switches between them.

use std::rc::Rc;

use gpui::{div, prelude::*, rems, svg, App, Div, ElementId, MouseButton};

use crate::app_settings::AppSettings;
use crate::theme;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub(crate) enum PanelViewId {
    #[default]
    Sessions,
    Info,
    Files,
}

/// The session's own views sit on the header's left as icons; the project's
/// sit on its right as labelled buttons.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PanelGroup {
    Session,
    Project,
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
    pub(crate) group: PanelGroup,
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
        group: PanelGroup::Session,
        surfaces: &[PanelSurface::Mission],
    },
    PanelView {
        id: PanelViewId::Info,
        label: "ui.info",
        icon: "info.svg",
        group: PanelGroup::Session,
        surfaces: &[PanelSurface::Chat, PanelSurface::Mission],
    },
    PanelView {
        id: PanelViewId::Files,
        label: "ui.files",
        icon: "folder.svg",
        group: PanelGroup::Project,
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
/// beside it fits, the tree's width, and the panel's standard width.
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
/// the tree, when turned on, hides first, then the file narrows, never below
/// the panel's `standard` width.
pub(crate) fn widened(
    available: f32,
    standard: f32,
    file: f32,
    tree_width: f32,
    tree: bool,
) -> Widened {
    let room = available - MAIN_MIN_WIDTH;
    let file = file.max(standard);
    if tree && file + tree_width <= room {
        return Widened {
            file,
            tree: true,
            tree_width,
            standard,
        };
    }
    Widened {
        file: room.clamp(standard, file),
        tree: false,
        tree_width,
        standard,
    }
}

/// The header's row: the session's views on the left, the project's beside
/// `collapse` on the right.
pub(crate) fn header_views(
    surface: PanelSurface,
    settings: &AppSettings,
    shown: Option<PanelViewId>,
    select: SelectView,
    collapse: impl IntoElement,
) -> Div {
    let session = views(surface, settings)
        .filter(|view| view.group == PanelGroup::Session)
        .map(|view| {
            let select = select.clone();
            let id = view.id;
            session_view_button(view, shown == Some(id))
                .on_click(move |_, _, cx| select(id, cx))
                .into_any_element()
        })
        .collect::<Vec<_>>();
    let project = views(surface, settings)
        .filter(|view| view.group == PanelGroup::Project)
        .map(|view| {
            let select = select.clone();
            let id = view.id;
            project_view_button(view, shown == Some(id))
                .on_click(move |_, _, cx| select(id, cx))
                .into_any_element()
        })
        .collect::<Vec<_>>();
    let divider = (!project.is_empty()).then(|| {
        div()
            .flex_none()
            .w(rems(1. / 16.))
            .h(rems(1.))
            .bg(theme::border())
    });
    div()
        .flex_1()
        .min_w(gpui::px(0.))
        .flex()
        .items_center()
        .child(div().flex().items_center().gap_1().children(session))
        .child(div().flex_1())
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_2()
                .children(project)
                .children(divider)
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
        crate::theme_snapshot::record_fill(id, crate::theme_snapshot::record_border(id, button))
    };
    button
}

fn session_view_button(view: &PanelView, active: bool) -> gpui::Stateful<Div> {
    let group = format!("side-panel-view-{:?}", view.id);
    stop_titlebar_drag(div().id(view_element_id(view)))
        .group(group.clone())
        .size(rems(28. / 16.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_sm()
        .cursor_pointer()
        .text_color(if active {
            theme::text()
        } else {
            theme::muted()
        })
        .when(active, |button| {
            button.bg(theme::with_alpha(theme::sidebar_selected(), 0.6))
        })
        .hover(|button| {
            button
                .bg(theme::with_alpha(theme::sidebar_selected(), 0.6))
                .text_color(theme::text())
        })
        .map(|button| record(view, button))
        .child(
            svg()
                .flex_none()
                .path(view.icon)
                .size(rems(14. / 16.))
                .text_color(if active {
                    theme::text()
                } else {
                    theme::muted()
                })
                .group_hover(group, |icon| icon.text_color(theme::text())),
        )
}

fn project_view_button(view: &PanelView, active: bool) -> gpui::Stateful<Div> {
    let group = format!("side-panel-view-{:?}", view.id);
    let label = view.label();
    let ink = if active {
        theme::text()
    } else {
        theme::muted()
    };
    stop_titlebar_drag(div().id(view_element_id(view)))
        .group(group.clone())
        .flex_none()
        .h(rems(28. / 16.))
        .px(rems(10. / 16.))
        .flex()
        .items_center()
        .gap(rems(6. / 16.))
        .rounded_md()
        .border_1()
        .border_color(theme::border())
        .cursor_pointer()
        .text_size(theme::text_ui())
        .text_color(ink)
        .when(active, |button| {
            button.bg(theme::with_alpha(theme::sidebar_selected(), 0.6))
        })
        .hover(|button| {
            button
                .bg(theme::with_alpha(theme::sidebar_selected(), 0.6))
                .text_color(theme::text())
        })
        .map(|button| record(view, button))
        .child(
            svg()
                .flex_none()
                .path(view.icon)
                .size(rems(14. / 16.))
                .text_color(ink)
                .group_hover(group, |icon| icon.text_color(theme::text())),
        )
        .child(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(surface: PanelSurface, group: PanelGroup) -> Vec<PanelViewId> {
        ids_with(&AppSettings::default(), surface, group)
    }

    fn ids_with(
        settings: &AppSettings,
        surface: PanelSurface,
        group: PanelGroup,
    ) -> Vec<PanelViewId> {
        views(surface, settings)
            .filter(|view| view.group == group)
            .map(|view| view.id)
            .collect()
    }

    #[test]
    fn views_group_by_session_and_project_and_filter_by_surface() {
        use PanelGroup::*;
        use PanelSurface::*;
        use PanelViewId::*;
        assert_eq!(ids(Mission, Session), [Sessions, Info]);
        assert_eq!(ids(Mission, Project), [Files]);
        assert_eq!(ids(Chat, Session), [Info]);
        assert_eq!(ids(Chat, Project), [Files]);
        assert_eq!(ids(Terminal, Session), []);
        assert_eq!(ids(Terminal, Project), [Files]);
        assert!(VIEWS.iter().all(|view| view.on(&AppSettings::default())));
    }

    #[test]
    fn settings_turn_info_and_files_off_but_never_sessions() {
        use PanelGroup::*;
        use PanelSurface::*;
        use PanelViewId::*;
        let off = AppSettings {
            info_view: false,
            files_view: false,
            ..AppSettings::default()
        };
        assert_eq!(ids_with(&off, Mission, Session), [Sessions]);
        assert_eq!(ids_with(&off, Mission, Project), []);
        assert_eq!(views(Chat, &off).count(), 0);
        assert_eq!(views(Terminal, &off).count(), 0);
        assert_eq!(shown_view(Chat, &off, Info), None);
        assert_eq!(shown_view(Mission, &off, Files), Some(Sessions));
        let files_only = AppSettings {
            info_view: false,
            ..AppSettings::default()
        };
        assert_eq!(shown_view(Chat, &files_only, Info), Some(Files));
        assert_eq!(ids_with(&files_only, Mission, Session), [Sessions]);
    }

    #[test]
    fn a_widened_panel_hides_the_tree_first_then_narrows_the_file() {
        let fit = |available| widened(available, 320., 440., 240., true);
        let laid = |file, tree| Widened {
            file,
            tree,
            tree_width: 240.,
            standard: 320.,
        };
        // 420 + 440 + 240: everything fits.
        assert_eq!(fit(1100.), laid(440., true));
        assert_eq!(fit(1100.).width(), 680.);
        // One pixel short: the tree hides and the file keeps its width.
        assert_eq!(fit(1099.), laid(440., false));
        // Then the file narrows, the mission keeping its 420.
        assert_eq!(fit(800.).file, 380.);
        assert_eq!(fit(800.).width() + MAIN_MIN_WIDTH, 800.);
        // Never below the standard panel width.
        assert_eq!(fit(600.).file, 320.);
        assert_eq!(fit(200.).file, 320.);
        // A tree turned off stays off; a standard width wider than the file wins.
        assert!(!widened(5000., 320., 440., 240., false).tree);
        assert_eq!(widened(5000., 480., 440., 240., true).file, 480.);
    }

    #[test]
    fn a_wider_tree_widens_the_panel_and_hides_first_in_the_same_order() {
        let fit = |available| widened(available, 288., 440., 360., true);
        // 420 + 440 + 360: the saved tree width fits beside the file.
        assert!(fit(1220.).tree);
        assert_eq!(fit(1220.).width(), 800.);
        // One pixel short: the tree hides first, the file keeps its width.
        assert!(!fit(1219.).tree);
        assert_eq!(fit(1219.).width(), 440.);
        // Then the file narrows to the mission's 420, never below standard.
        assert_eq!(fit(800.).width(), 380.);
        assert_eq!(fit(600.).width(), 288.);
        // A narrower tree fits where the wider one did not.
        assert!(widened(1100., 288., 440., 160., true).tree);
        assert!(!widened(1100., 288., 440., 360., true).tree);
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
