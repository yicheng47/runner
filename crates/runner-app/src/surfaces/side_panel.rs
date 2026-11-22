//! The right side panel's tools, Orca-style 40 px icon activity bar, and
//! docked-vs-overlay-sheet geometry (#1403, #1331).

use std::rc::Rc;

use gpui::{
    div, prelude::*, px, rems, svg, AnyElement, App, BoxShadow, Div, ElementId, MouseButton, Window,
};
use runner_app::ui::{ButtonVariant, IconButton, Tooltip};
use runner_core::pane_layout::{PaneLayout, PaneNode, SplitOrientation};

use crate::app_settings::{AppSettings, DiffViewMode};
use crate::theme;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
pub(crate) enum PanelViewId {
    #[default]
    Sessions,
    Files,
    Changes,
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

    /// Whether Settings → Side panel enables the view; a mission's Sessions
    /// roster is always enabled while the side panel is on.
    pub(crate) fn on(&self, settings: &AppSettings) -> bool {
        match self.id {
            PanelViewId::Sessions => true,
            PanelViewId::Files => settings.files_view,
            PanelViewId::Changes => settings.changes_view,
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
        id: PanelViewId::Files,
        label: "ui.files",
        icon: "files.svg",
        surfaces: &[
            PanelSurface::Chat,
            PanelSurface::Terminal,
            PanelSurface::Mission,
        ],
    },
    PanelView {
        id: PanelViewId::Changes,
        label: "ui.changes",
        icon: "git-compare.svg",
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

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
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
    /// Split-pane "Following <pane> · <branch/folder>" indicator when panes in
    /// the same tab work in different folders or worktrees.
    pub(crate) following: Option<String>,
}

/// Beside a docked panel on a single-pane chat or mission, the main workspace
/// keeps at least this much width.
pub(crate) const MAIN_MIN_WIDTH: f32 = 420.;
/// Minimum width preserved per horizontal column in a split chat tab.
pub(crate) const SPLIT_COL_MIN_WIDTH: f32 = 340.;
/// Minimum backdrop margin kept visible to the left of an overlay sheet.
pub(crate) const SHEET_MIN_SCRIM_WIDTH: f32 = 48.;
/// From this width the mission's Sessions roster places two cards per row.
pub(crate) const ROSTER_TWO_UP_WIDTH: f32 = 560.;
/// Each side of a Split diff takes at least this much unmaximized; narrower
/// panels maximize on picking Split and fall back to Unified on Restore chat.
pub(crate) const SPLIT_SIDE_MIN_WIDTH: f32 = 360.;
pub(crate) const SPLIT_MIN_DIFF_WIDTH: f32 = SPLIT_SIDE_MIN_WIDTH * 2.;

pub(crate) fn effective_diff_mode(
    preferred: DiffViewMode,
    diff_width: f32,
    panel_maximized: bool,
) -> DiffViewMode {
    if preferred == DiffViewMode::Split && (panel_maximized || diff_width >= SPLIT_MIN_DIFF_WIDTH) {
        DiffViewMode::Split
    } else {
        DiffViewMode::Unified
    }
}

pub(crate) fn min_main_width(split_columns: usize) -> f32 {
    if split_columns <= 1 {
        MAIN_MIN_WIDTH
    } else {
        split_columns as f32 * SPLIT_COL_MIN_WIDTH
    }
}

pub(crate) fn node_split_columns(node: &PaneNode) -> usize {
    match node {
        PaneNode::Leaf(_) => 1,
        PaneNode::Split(split) => match split.orientation {
            SplitOrientation::Row => node_split_columns(&split.a) + node_split_columns(&split.b),
            SplitOrientation::Column => {
                node_split_columns(&split.a).max(node_split_columns(&split.b))
            }
        },
    }
}

pub(crate) fn split_columns_for_layout(layout: Option<&PaneLayout>, has_maximized: bool) -> usize {
    if has_maximized {
        return 1;
    }
    layout.map_or(1, |layout| node_split_columns(&layout.root))
}

/// Whether a window with `available` workspace width is too narrow to dock a
/// compact `compact_width` side panel beside `split_columns` main columns.
pub(crate) fn is_narrow(available: f32, compact_width: f32, split_columns: usize) -> bool {
    available > 0. && (available - compact_width) < min_main_width(split_columns)
}

/// Default open state for a tab or mission whose user has not explicitly opened
/// or collapsed the panel yet: chats and terminals start collapsed; missions
/// start open (on Sessions) on wide windows and collapsed on narrow ones.
pub(crate) fn default_open(
    surface: PanelSurface,
    available: f32,
    compact_width: f32,
    split_columns: usize,
) -> bool {
    match surface {
        PanelSurface::Chat | PanelSurface::Terminal => false,
        PanelSurface::Mission => !is_narrow(available, compact_width, split_columns),
    }
}

pub(crate) fn resolved_open(
    surface: PanelSurface,
    explicit: Option<bool>,
    available: f32,
    compact_width: f32,
    split_columns: usize,
) -> bool {
    explicit.unwrap_or_else(|| default_open(surface, available, compact_width, split_columns))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PanelPresentation {
    Docked,
    Sheet,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PanelLayout {
    pub(crate) width: f32,
    pub(crate) widened: Option<Widened>,
    pub(crate) presentation: PanelPresentation,
    /// Docked width reserved in the flex row when `presentation == Sheet`:
    /// `0` on narrow windows (terminal stays full width, zero SIGWINCH) and
    /// `standard` when widening overflowed an already-docked compact rail
    /// (split panes keep their docked width, zero SIGWINCH).
    pub(crate) reserved_docked_width: f32,
}

impl PanelLayout {
    pub(crate) fn is_sheet(self) -> bool {
        self.presentation == PanelPresentation::Sheet
    }
}

/// A panel widened by an open file or diff: the viewer's width, whether the
/// tree/list column beside it is on, that column's width, and the compact
/// width.
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

/// How a docked panel showing a file of width `file` beside a tree `tree_width`
/// wide fits in `available` while keeping `min_main` for the workspace.
pub(crate) fn widened_for(
    available: f32,
    min_main: f32,
    standard: f32,
    file: f32,
    tree_width: f32,
    tree: bool,
) -> Widened {
    let room = available - min_main - if tree { tree_width } else { 0. };
    let file = file.max(standard);
    Widened {
        file: room.clamp(standard, file),
        tree,
        tree_width,
        standard,
    }
}

#[cfg(test)]
pub(crate) fn widened(
    available: f32,
    standard: f32,
    file: f32,
    tree_width: f32,
    tree: bool,
) -> Widened {
    widened_for(available, MAIN_MIN_WIDTH, standard, file, tree_width, tree)
}

/// Computes the panel's width, widened geometry, and Docked vs Sheet
/// presentation for a workspace of `available` width (already minus any docked
/// gap, or `0` before first measurement).
#[allow(clippy::too_many_arguments)]
pub(crate) fn compute_panel_layout(
    available: f32,
    split_columns: usize,
    standard: f32,
    file: f32,
    tree_width: f32,
    widened_tree: Option<bool>,
    maximized: bool,
    maximized_full: f32,
) -> PanelLayout {
    if maximized {
        let widened = widened_tree.map(|tree| {
            let file_width = (maximized_full - if tree { tree_width } else { 0. }).max(standard);
            Widened {
                file: file_width,
                tree,
                tree_width,
                standard,
            }
        });
        return PanelLayout {
            width: maximized_full,
            widened,
            presentation: PanelPresentation::Docked,
            reserved_docked_width: maximized_full,
        };
    }

    let min_main = min_main_width(split_columns);
    let narrow = is_narrow(available, standard, split_columns);
    let max_sheet = if available > 0. {
        (available - SHEET_MIN_SCRIM_WIDTH).max(standard.min(available))
    } else {
        standard
    };

    let Some(tree) = widened_tree else {
        if narrow {
            let width = standard.min(max_sheet);
            return PanelLayout {
                width,
                widened: None,
                presentation: PanelPresentation::Sheet,
                reserved_docked_width: 0.,
            };
        }
        return PanelLayout {
            width: standard,
            widened: None,
            presentation: PanelPresentation::Docked,
            reserved_docked_width: standard,
        };
    };

    let desired_file = file.max(standard);
    let desired_tree = if tree { tree_width } else { 0. };
    // Splits promote to an overlay sheet whenever docking the requested widened
    // width would drop any split column below 340 px; single-pane views narrow
    // smoothly down to `standard + tree` before promoting (#1331, #1403).
    let docked_threshold = if split_columns > 1 {
        desired_file + desired_tree
    } else {
        standard + desired_tree
    };

    if available > 0. && (narrow || (available - docked_threshold) < min_main) {
        let (sheet_tree, sheet_file) = if tree && max_sheet < standard + tree_width {
            (
                false,
                desired_file.min(max_sheet).max(standard.min(max_sheet)),
            )
        } else {
            let room = max_sheet - if tree { tree_width } else { 0. };
            (tree, desired_file.min(room).max(standard.min(room)))
        };
        let layout = Widened {
            file: sheet_file,
            tree: sheet_tree,
            tree_width,
            standard,
        };
        return PanelLayout {
            width: layout.width(),
            widened: Some(layout),
            presentation: PanelPresentation::Sheet,
            reserved_docked_width: if narrow { 0. } else { standard },
        };
    }

    let layout = if available > 0. {
        widened_for(
            available,
            min_main,
            standard,
            desired_file,
            tree_width,
            tree,
        )
    } else {
        Widened {
            file: desired_file,
            tree,
            tree_width,
            standard,
        }
    };
    PanelLayout {
        width: layout.width(),
        widened: Some(layout),
        presentation: PanelPresentation::Docked,
        reserved_docked_width: layout.width(),
    }
}

/// The active mode selected in a panel's activity bar; defaults to the first
/// enabled view (`Files` on chats/terminals, `Sessions` on missions).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PanelSelection {
    active: Option<PanelViewId>,
}

impl PanelSelection {
    #[cfg(test)]
    pub(crate) fn new(id: PanelViewId) -> Self {
        Self { active: Some(id) }
    }

    pub(crate) fn select(&mut self, id: PanelViewId) {
        self.active = Some(id);
    }

    #[cfg(test)]
    pub(crate) fn selected(&self) -> Option<PanelViewId> {
        self.active
    }

    #[cfg(test)]
    pub(crate) fn effective(
        &self,
        surface: PanelSurface,
        settings: &AppSettings,
    ) -> Option<PanelViewId> {
        shown(surface, settings, self)
    }
}

/// An inactive tab's or mission's panel state; `open: None` follows the
/// surface's default (collapsed on chats/terminals; open on wide missions,
/// collapsed on narrow ones) until explicitly opened or collapsed.
#[derive(Clone, Debug, Default)]
pub(crate) struct SavedPanel {
    pub(crate) selection: PanelSelection,
    pub(crate) open: Option<bool>,
    pub(crate) maximized: bool,
}

/// The view a panel shows: the selected mode if still enabled on `surface`,
/// otherwise the first enabled view (`Files` on chats/terminals, `Sessions` on
/// missions). `None` only when Settings disabled every tool for this surface.
pub(crate) fn shown(
    surface: PanelSurface,
    settings: &AppSettings,
    selection: &PanelSelection,
) -> Option<PanelViewId> {
    let mut enabled = views(surface, settings);
    let first = enabled.next()?.id;
    if let Some(active) = selection.active {
        if active == first || enabled.any(|view| view.id == active) {
            return Some(active);
        }
    }
    Some(first)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SurfaceAction {
    Select(PanelViewId),
}

pub(crate) type SurfaceHandler = Rc<dyn Fn(SurfaceAction, &mut Window, &mut App)>;

/// Per-panel activity-bar state.
#[derive(Default)]
pub(crate) struct PanelSurfaces {
    pub(crate) selection: PanelSelection,
}

impl PanelSurfaces {
    pub(crate) fn new(selection: PanelSelection) -> Self {
        Self { selection }
    }

    pub(crate) fn reset(&mut self, selection: PanelSelection) {
        self.selection = selection;
    }

    pub(crate) fn restore(&mut self, selection: PanelSelection) {
        self.selection = selection;
    }

    pub(crate) fn apply(&mut self, action: SurfaceAction) {
        match action {
            SurfaceAction::Select(id) => self.selection.select(id),
        }
    }

    #[cfg(test)]
    pub(crate) fn select(&mut self, id: PanelViewId) {
        self.selection.select(id);
    }
}

pub(crate) type PanelCallback = Rc<dyn Fn(&mut Window, &mut App)>;
pub(crate) type PanelMaximizeCallback = Rc<dyn Fn(bool, &mut Window, &mut App)>;

pub(crate) struct MaximizeControl {
    pub(crate) maximized: bool,
    pub(crate) on_toggle: PanelCallback,
}

/// Orca-style 40 px top icon activity bar: fixed icon buttons (`Sessions`,
/// `Files`, `Changes` with amber dirty dot), active raised tile + 2 px bottom
/// accent indicator, optional `Maximize` (shown only when widened or already
/// maximized), and `Collapse`.
pub(crate) fn activity_bar(
    surface: PanelSurface,
    settings: &AppSettings,
    selection: &PanelSelection,
    dirty_changes: bool,
    handler: SurfaceHandler,
    maximize: Option<MaximizeControl>,
    collapse: impl IntoElement,
) -> Div {
    let active = shown(surface, settings, selection);
    let buttons = views(surface, settings)
        .map(|view| {
            let dot = view.id == PanelViewId::Changes && dirty_changes;
            activity_button(view, active == Some(view.id), dot, handler.clone())
        })
        .collect::<Vec<_>>();
    let maximize = maximize.map(|control| {
        let maximized = control.maximized;
        let on_toggle = control.on_toggle;
        let label = crate::i18n::text(if maximized {
            "ui.restore_chat"
        } else {
            "ui.maximize_side_panel"
        });
        let tooltip =
            crate::keymap::effective_binding("toggle-maximize-pane", &settings.keymap_overrides)
                .map_or_else(
                    || label.to_owned(),
                    |combo| format!("{label} · {}", crate::keymap::format_combo(&combo)),
                );
        div()
            .flex_none()
            .debug_selector(move || {
                if maximized {
                    "SIDE_PANEL_MAXIMIZE active".into()
                } else {
                    "SIDE_PANEL_MAXIMIZE".into()
                }
            })
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                IconButton::new(
                    "side-panel-maximize",
                    if maximized {
                        "minimize-2.svg"
                    } else {
                        "maximize-2.svg"
                    },
                )
                .when(maximized, |button| button.variant(ButtonVariant::Secondary))
                .tooltip(tooltip)
                .on_press(move |window, cx| on_toggle(window, cx)),
            )
    });
    div()
        .flex_1()
        .min_w(px(0.))
        .h_full()
        .flex()
        .items_center()
        .gap(rems(4. / 16.))
        .child(
            div()
                .h_full()
                .flex()
                .items_center()
                .gap(rems(4. / 16.))
                .children(buttons),
        )
        .child(div().flex_1().min_w(px(0.)).h_full())
        .children(maximize)
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

/// One activity-bar icon button: 28×28 tile (`theme::raised()` when active),
/// optional amber dirty dot on Changes, and a 2 px bottom `theme::accent()` bar
/// flush with the 40 px header divider when active.
fn activity_button(
    view: &'static PanelView,
    active: bool,
    dirty_dot: bool,
    handler: SurfaceHandler,
) -> AnyElement {
    let id = view.id;
    let ink = if active {
        theme::text()
    } else {
        theme::muted()
    };
    let tile = stop_titlebar_drag(div().id(view_element_id("activity", view)))
        .relative()
        .size(rems(28. / 16.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .cursor_pointer()
        .when(active, |el| el.bg(theme::raised()))
        .when(!active, |el| {
            el.hover(|el| el.bg(theme::with_alpha(theme::raised(), 0.6)))
        })
        .map(|el| record("PANEL_ACTIVITY", view, el))
        .on_click(move |_, window, cx| handler(SurfaceAction::Select(id), window, cx))
        .child(
            svg()
                .flex_none()
                .path(view.icon)
                .size(rems(15. / 16.))
                .text_color(ink),
        )
        .when(dirty_dot, |el| {
            el.child(
                div()
                    .when(cfg!(test), |dot| {
                        dot.debug_selector(|| "PANEL_CHANGES_DIRTY_DOT".into())
                    })
                    .absolute()
                    .top(rems(4. / 16.))
                    .right(rems(4. / 16.))
                    .size(rems(6. / 16.))
                    .rounded_full()
                    .bg(theme::warning()),
            )
        });

    let wrapped = div()
        .relative()
        .h_full()
        .flex()
        .items_center()
        .when(cfg!(test) && active, |el| {
            el.debug_selector(move || format!("PANEL_ACTIVITY {id:?} active"))
        })
        .child(Tooltip::new(
            view_element_id("activity-tip", view),
            view.label(),
            tile,
        ))
        .when(active, |el| {
            el.child(
                div()
                    .when(cfg!(test), |bar| {
                        bar.debug_selector(move || format!("PANEL_ACTIVITY_BAR {id:?}"))
                    })
                    .absolute()
                    .bottom_0()
                    .left(rems(4. / 16.))
                    .right(rems(4. / 16.))
                    .h(rems(2. / 16.))
                    .rounded_full()
                    .bg(theme::accent()),
            )
        });
    wrapped.into_any_element()
}

/// Elevated overlay sheet backdrop + right-anchored sheet wrapper for narrow
/// windows and split-widening overflow (#1331). Clicking the scrim dismisses
/// the sheet. `progress` (`0.0..=1.0`) drives the smooth slide-in/out and
/// scrim fade without changing the reserved terminal width behind it.
pub(crate) fn overlay_sheet(
    id: &'static str,
    panel: AnyElement,
    width: f32,
    progress: f32,
    on_dismiss: PanelCallback,
) -> AnyElement {
    let progress = progress.clamp(0., 1.);
    let scrim_alpha = if theme::active_variant().is_light() {
        0.16
    } else {
        0.38
    } * progress;
    let slide_offset = -width * (1. - progress);
    div()
        .id(id)
        .when(cfg!(test), |el| {
            el.debug_selector(|| "PANEL_OVERLAY_SHEET".into())
        })
        .absolute()
        .inset_0()
        .overflow_hidden()
        .flex()
        .justify_end()
        .bg(theme::with_alpha(theme::scrim(), scrim_alpha))
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            on_dismiss(window, cx);
        })
        .child(
            div()
                .h_full()
                .mr(rems(slide_offset / 16.))
                .flex()
                .flex_none()
                .shadow(vec![BoxShadow {
                    color: theme::with_alpha(theme::scrim(), 0.45 * progress),
                    offset: gpui::point(px(-8.), px(0.)),
                    blur_radius: px(28.),
                    spread_radius: px(0.),
                    inset: false,
                }])
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(panel),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use runner_core::pane_layout::{PaneLeaf, PaneSplit};

    fn ids_with(settings: &AppSettings, surface: PanelSurface) -> Vec<PanelViewId> {
        views(surface, settings).map(|view| view.id).collect()
    }

    fn leaf(id: &str) -> PaneNode {
        PaneNode::Leaf(PaneLeaf {
            id: id.into(),
            session_id: None,
        })
    }

    fn split(orientation: SplitOrientation, a: PaneNode, b: PaneNode) -> PaneNode {
        PaneNode::Split(PaneSplit {
            id: "s".into(),
            orientation,
            sizes: [50., 50.],
            a: Box::new(a),
            b: Box::new(b),
        })
    }

    #[test]
    fn activity_bar_views_match_surface_and_settings() {
        use PanelViewId::*;
        let defaults = AppSettings::default();
        assert_eq!(ids_with(&defaults, PanelSurface::Chat), [Files, Changes]);
        assert_eq!(
            ids_with(&defaults, PanelSurface::Terminal),
            [Files, Changes]
        );
        assert_eq!(
            ids_with(&defaults, PanelSurface::Mission),
            [Sessions, Files, Changes]
        );

        let files_off = AppSettings {
            files_view: false,
            ..defaults.clone()
        };
        assert_eq!(ids_with(&files_off, PanelSurface::Chat), [Changes]);
        assert_eq!(
            ids_with(&files_off, PanelSurface::Mission),
            [Sessions, Changes]
        );

        let both_off = AppSettings {
            files_view: false,
            changes_view: false,
            ..defaults
        };
        assert!(ids_with(&both_off, PanelSurface::Chat).is_empty());
        assert!(ids_with(&both_off, PanelSurface::Terminal).is_empty());
        assert_eq!(ids_with(&both_off, PanelSurface::Mission), [Sessions]);
    }

    #[test]
    fn default_shown_view_and_selection_fallback_on_settings_change() {
        use PanelViewId::*;
        let defaults = AppSettings::default();
        let mut selection = PanelSelection::default();

        assert_eq!(
            shown(PanelSurface::Chat, &defaults, &selection),
            Some(Files)
        );
        assert_eq!(
            shown(PanelSurface::Terminal, &defaults, &selection),
            Some(Files)
        );
        assert_eq!(
            shown(PanelSurface::Mission, &defaults, &selection),
            Some(Sessions)
        );

        selection.select(Changes);
        assert_eq!(
            shown(PanelSurface::Chat, &defaults, &selection),
            Some(Changes)
        );
        assert_eq!(
            shown(PanelSurface::Mission, &defaults, &selection),
            Some(Changes)
        );

        let changes_off = AppSettings {
            changes_view: false,
            ..defaults.clone()
        };
        assert_eq!(
            shown(PanelSurface::Chat, &changes_off, &selection),
            Some(Files)
        );
        assert_eq!(
            shown(PanelSurface::Mission, &changes_off, &selection),
            Some(Sessions)
        );

        let both_off = AppSettings {
            files_view: false,
            changes_view: false,
            ..defaults
        };
        assert_eq!(shown(PanelSurface::Chat, &both_off, &selection), None);
        assert_eq!(
            shown(PanelSurface::Mission, &both_off, &selection),
            Some(Sessions)
        );
    }

    #[test]
    fn split_columns_counts_horizontal_columns_and_ignores_vertical_rows() {
        assert_eq!(node_split_columns(&leaf("p1")), 1);
        let two_cols = split(SplitOrientation::Row, leaf("p1"), leaf("p2"));
        assert_eq!(node_split_columns(&two_cols), 2);
        let two_rows = split(SplitOrientation::Column, leaf("p1"), leaf("p2"));
        assert_eq!(node_split_columns(&two_rows), 1);
        let mixed = split(
            SplitOrientation::Column,
            split(SplitOrientation::Row, leaf("p1"), leaf("p2")),
            leaf("p3"),
        );
        assert_eq!(node_split_columns(&mixed), 2);
        let three_cols = split(
            SplitOrientation::Row,
            leaf("p1"),
            split(SplitOrientation::Row, leaf("p2"), leaf("p3")),
        );
        assert_eq!(node_split_columns(&three_cols), 3);
    }

    #[test]
    fn narrow_and_split_layout_math_follows_420_single_and_340_per_split_column() {
        assert_eq!(min_main_width(1), 420.);
        assert_eq!(min_main_width(2), 680.);
        assert_eq!(min_main_width(3), 1020.);

        // Chats and terminals start collapsed regardless of window width;
        // missions start open on wide windows (>= 720 available) and collapsed when narrow.
        assert!(!default_open(PanelSurface::Chat, 720., 300., 1));
        assert!(!default_open(PanelSurface::Terminal, 720., 300., 1));
        assert!(!default_open(PanelSurface::Chat, 1440., 300., 1));
        assert!(!default_open(PanelSurface::Terminal, 1440., 300., 1));
        assert!(resolved_open(PanelSurface::Chat, Some(true), 720., 300., 1));
        assert!(resolved_open(
            PanelSurface::Terminal,
            Some(true),
            719.,
            300.,
            1
        ));
        assert!(!resolved_open(
            PanelSurface::Mission,
            Some(false),
            1440.,
            300.,
            1
        ));

        // Single-pane: 720 available => 720 - 300 = 420 >= 420 (wide; missions open docked).
        assert!(default_open(PanelSurface::Mission, 720., 300., 1));
        assert!(!is_narrow(720., 300., 1));
        let wide_compact = compute_panel_layout(720., 1, 300., 440., 240., None, false, 720.);
        assert_eq!(wide_compact.presentation, PanelPresentation::Docked);
        assert_eq!(wide_compact.width, 300.);

        // Single-pane: 719 available => 719 - 300 = 419 < 420 (narrow, default collapsed, sheet).
        assert!(!default_open(PanelSurface::Mission, 719., 300., 1));
        assert!(!default_open(PanelSurface::Chat, 719., 300., 1));
        assert!(is_narrow(719., 300., 1));
        let narrow_compact = compute_panel_layout(719., 1, 300., 440., 240., None, false, 719.);
        assert_eq!(narrow_compact.presentation, PanelPresentation::Sheet);
        assert_eq!(narrow_compact.width, 300.);
        assert_eq!(narrow_compact.reserved_docked_width, 0.);

        // Single-pane on 1440 px screen (available = 1194): widened 680 fits docked (1194 - 680 = 514 >= 420).
        let wide_file = compute_panel_layout(1194., 1, 300., 440., 240., Some(true), false, 1194.);
        assert_eq!(wide_file.presentation, PanelPresentation::Docked);
        assert_eq!(wide_file.width, 680.);

        // 2-column split on 1440 px screen (available = 1194, min_main = 680):
        // Chat starts collapsed; once opened, compact 300 docks (1194 - 300 = 894 >= 680).
        assert!(!default_open(PanelSurface::Chat, 1194., 300., 2));
        assert!(!is_narrow(1194., 300., 2));
        let split_compact = compute_panel_layout(1194., 2, 300., 440., 240., None, false, 1194.);
        assert_eq!(split_compact.presentation, PanelPresentation::Docked);
        assert_eq!(split_compact.width, 300.);

        // Widening a file (680) would leave 514 < 680 => auto-sheets, reserving 300 docked so
        // split terminals never SIGWINCH.
        let split_file = compute_panel_layout(1194., 2, 300., 440., 240., Some(true), false, 1194.);
        assert_eq!(split_file.presentation, PanelPresentation::Sheet);
        assert_eq!(split_file.width, 680.);
        assert_eq!(split_file.reserved_docked_width, 300.);

        // Intermediate split band: 1300 px available with 2 columns (min_main = 680) and default
        // 680 px Files viewer => 1300 - 680 = 620 < 680 => must promote to Sheet rather than
        // docking at 620 and resizing both split terminals (#1331, #1403).
        let split_mid_files =
            compute_panel_layout(1300., 2, 300., 440., 240., Some(true), false, 1300.);
        assert_eq!(split_mid_files.presentation, PanelPresentation::Sheet);
        assert_eq!(split_mid_files.width, 680.);
        assert_eq!(split_mid_files.reserved_docked_width, 300.);

        // Same for signed-off ~740 px Changes diff (500 + 240) at 1400 px available:
        // 1400 - 740 = 660 < 680 => promotes to Sheet; at 1420 px => docks at 740 px.
        let split_mid_changes =
            compute_panel_layout(1400., 2, 300., 500., 240., Some(true), false, 1400.);
        assert_eq!(split_mid_changes.presentation, PanelPresentation::Sheet);
        assert_eq!(split_mid_changes.width, 740.);
        assert_eq!(split_mid_changes.reserved_docked_width, 300.);
        let split_wide_changes =
            compute_panel_layout(1420., 2, 300., 500., 240., Some(true), false, 1420.);
        assert_eq!(split_wide_changes.presentation, PanelPresentation::Docked);
        assert_eq!(split_wide_changes.width, 740.);

        // Very narrow window (500 px available): widened sheet drops the 240 px tree/list column
        // (`widened.tree == false`) so the viewer/diff gets full sheet width.
        let tiny_sheet = compute_panel_layout(500., 1, 300., 500., 240., Some(true), false, 500.);
        assert_eq!(tiny_sheet.presentation, PanelPresentation::Sheet);
        assert!(!tiny_sheet.widened.unwrap().tree);
        assert_eq!(tiny_sheet.reserved_docked_width, 0.);

        // Closing the file snaps back to docked 300; widening window to 1360 docks the file.
        let split_wide_file =
            compute_panel_layout(1360., 2, 300., 440., 240., Some(true), false, 1360.);
        assert_eq!(split_wide_file.presentation, PanelPresentation::Docked);
        assert_eq!(split_wide_file.width, 680.);
    }

    #[test]
    fn a_widened_panel_narrows_the_file_first_and_keeps_the_tree() {
        let fit = |available| widened(available, 300., 440., 240., true);
        let full = fit(2000.);
        assert_eq!((full.file, full.tree, full.width()), (440., true, 680.));

        let tight = fit(1000.);
        assert!(tight.tree);
        assert_eq!(tight.file, 340.);
        assert_eq!(tight.width() + MAIN_MIN_WIDTH, 1000.);

        let floor = fit(600.);
        assert_eq!((floor.file, floor.tree, floor.width()), (300., true, 540.));

        let off = widened(800., 300., 440., 240., false);
        assert_eq!((off.file, off.tree, off.width()), (380., false, 380.));
        assert_eq!(widened(5000., 480., 440., 240., true).file, 480.);
    }

    #[test]
    fn resizing_the_tree_divider_keeps_the_panel_width() {
        let panel = widened(2000., 300., 440., 240., true);
        assert_eq!(panel.resize_tree(280.), (400., 280.));
        assert_eq!(panel.resize_tree(100.), (520., 160.));
        assert_eq!(panel.resize_tree(500.), (300., 380.));
        assert_eq!(panel.resize_tree(600.), (300., 380.));

        let wide = widened(3000., 300., 1000., 240., true);
        assert_eq!(wide.resize_tree(160.), (1080., 160.));

        let widest = widened(3000., 300., 1200., 240., true);
        assert_eq!(widest.resize_tree(160.), (1200., 240.));
        let wider = widened(3000., 300., 1100., 300., true);
        assert_eq!(wider.resize_tree(160.), (1200., 200.));
    }
}
