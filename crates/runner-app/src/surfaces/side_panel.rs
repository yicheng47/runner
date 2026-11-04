//! The side panel's views: what each one declares, the context it renders
//! from, and the header that switches between them.

use std::rc::Rc;

use gpui::{div, prelude::*, rems, svg, AnyElement, App, Div, ElementId, MouseButton};

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

    pub(crate) fn on(&self) -> bool {
        true
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

pub(crate) fn views(surface: PanelSurface) -> impl Iterator<Item = &'static PanelView> {
    VIEWS
        .iter()
        .filter(move |view| view.on() && view.surfaces.contains(&surface))
}

/// The view a panel shows: the one chosen when the surface has it, else the
/// surface's first.
pub(crate) fn shown_view(surface: PanelSurface, chosen: PanelViewId) -> Option<PanelViewId> {
    let mut available = views(surface).map(|view| view.id).peekable();
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
    pub(crate) cwd: Option<String>,
    pub(crate) project: Option<String>,
    pub(crate) remote: bool,
}

pub(crate) fn files_view(context: &PanelContext) -> AnyElement {
    div()
        .when(cfg!(test), |view| {
            view.debug_selector(|| format!("FILES_VIEW {context:?}"))
        })
        .flex_1()
        .min_h(gpui::px(0.))
        .into_any_element()
}

pub(crate) type SelectView = Rc<dyn Fn(PanelViewId, &mut App)>;

/// The header's row: the session's views on the left, the project's beside
/// `collapse` on the right.
pub(crate) fn header_views(
    surface: PanelSurface,
    shown: Option<PanelViewId>,
    select: SelectView,
    collapse: impl IntoElement,
) -> Div {
    let session = views(surface)
        .filter(|view| view.group == PanelGroup::Session)
        .map(|view| {
            let select = select.clone();
            let id = view.id;
            session_view_button(view, shown == Some(id))
                .on_click(move |_, _, cx| select(id, cx))
                .into_any_element()
        })
        .collect::<Vec<_>>();
    let project = views(surface)
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
        views(surface)
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
        assert!(VIEWS.iter().all(PanelView::on));
    }

    #[test]
    fn a_panel_shows_the_chosen_view_or_its_surface_first() {
        use PanelSurface::*;
        use PanelViewId::*;
        assert_eq!(shown_view(Mission, Sessions), Some(Sessions));
        assert_eq!(shown_view(Mission, Files), Some(Files));
        assert_eq!(shown_view(Chat, Sessions), Some(Info));
        assert_eq!(shown_view(Chat, Files), Some(Files));
        assert_eq!(shown_view(Terminal, Info), Some(Files));
    }
}
