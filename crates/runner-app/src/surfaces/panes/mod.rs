//! Chat-surface rendering: the active tab, its split menus, and the pane tree.
mod dialogs;
mod drawer;
mod header;
mod pane;
mod panel;
mod start;
mod tab;
mod tree;

pub(crate) use drawer::*;
use panel::*;
use start::*;

use super::*;

use crate::*;

use gpui::{canvas, quad, svg, BorderStyle, Div, FontWeight, Pixels, Point, Size};

use runner_app::ui::resize::{resize_strip, resize_strip_inset, ResizeAxis};

use runner_app::ui::{ButtonVariant, SessionOverlay, SessionOverlayKind};

use runner_core::protocol::model::Runtime;

use crate::chat_icon::runtime_mark;

use crate::surfaces::chat_lifecycle::{
    ended_subtitle, resolve_pane_overlay, PaneOverlayState, TransitionKind,
};

use crate::surfaces::front_page::model::SessionDisplay;

use crate::surfaces::profile_page::{
    clamped_markdown, column_text, prompt_meta, setup_row, setup_value, ClickHandler,
};

use crate::surfaces::roles::logic::{role_setting_label, runtime_display_name};

use runner_app::ui::RoleAvatar;

use crate::surfaces::side_panel::{self, PanelContext, PanelSubject, PanelSurface, PanelViewId};

const CHAT_PANEL_TRANSITION_MS: u64 = 200;

pub(crate) const UNFOCUSED_PANE_OPACITY: f32 = 0.7;

/// The smallest pane a split may leave behind, at 1× zoom.
pub(crate) const MIN_SPLIT_PANE_WIDTH: f32 = 240.;

pub(crate) const MIN_SPLIT_PANE_HEIGHT: f32 = 160.;

const TOO_SMALL_TO_SPLIT: &str = "ui.too_small_to_split";

impl NativeRoot {
    pub(crate) fn reset_chat_panel_prompt(&mut self, selected: Option<String>) {
        if self.chat_panel_prompt_session != selected {
            self.chat_panel_prompt_session = selected;
            self.chat_panel_prompt_expanded = false;
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) enum SplitMenuSurface {
    Pane,
}

impl SplitMenuSurface {
    fn key(self) -> &'static str {
        match self {
            Self::Pane => "pane",
        }
    }
}

/// One pane, named the only way a pane id is unique: inside its tab.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct PaneKey {
    pub(crate) tab_id: String,
    pub(crate) pane_id: String,
}

impl PaneKey {
    pub(crate) fn new(tab_id: &str, pane_id: &str) -> Self {
        Self {
            tab_id: tab_id.to_owned(),
            pane_id: pane_id.to_owned(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct SplitMenuKey {
    pub(crate) surface: SplitMenuSurface,
    pub(crate) tab_id: String,
    pub(crate) pane_id: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PaneCloseBehavior {
    LayoutOnly,
    CloseTerminal,
    ArchiveChat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PaneRenameKey {
    Submit,
    Cancel,
}

fn pane_rename_key(key: &str) -> Option<PaneRenameKey> {
    match key {
        "enter" => Some(PaneRenameKey::Submit),
        "escape" => Some(PaneRenameKey::Cancel),
        _ => None,
    }
}

fn split_tab_archiving(layout: &PaneLayout, archiving: impl Fn(&str) -> bool) -> bool {
    let session_ids = layout.session_ids();
    layout.root.leaves().len() > 1
        && !session_ids.is_empty()
        && session_ids.iter().all(|session_id| archiving(session_id))
}

fn start_surface_visible(layout: Option<&PaneLayout>) -> bool {
    layout.is_none_or(|layout| {
        let leaves = layout.root.leaves();
        leaves.len() == 1 && leaves[0].session_id.is_none()
    })
}

pub(crate) fn terminal_drawer_tooltip(open: bool, overrides: &keymap::KeymapOverrides) -> String {
    let label = if open {
        crate::i18n::text("ui.hide_terminal_drawer")
    } else {
        crate::i18n::text("ui.show_terminal_drawer")
    };
    keymap::effective_binding("toggle-terminal-drawer", overrides).map_or_else(
        || label.to_owned(),
        |combo| format!("{label} · {}", keymap::format_combo(&combo)),
    )
}

/// A split leaves two panes where one stood, so each side needs half of the
/// floor. `None` bounds mean the pane has not been laid out yet — allow it
/// rather than refuse on a measurement we do not have.
fn split_allowed(pane: Option<Size<Pixels>>, orientation: SplitOrientation, zoom: f32) -> bool {
    let Some(pane) = pane else {
        return true;
    };
    match orientation {
        SplitOrientation::Row => f32::from(pane.width) >= 2. * MIN_SPLIT_PANE_WIDTH * zoom,
        SplitOrientation::Column => f32::from(pane.height) >= 2. * MIN_SPLIT_PANE_HEIGHT * zoom,
    }
}

fn drop_side(pane: Bounds<Pixels>, pointer: Point<Pixels>) -> DropSide {
    [
        (DropSide::Left, (pointer.x - pane.left()) / pane.size.width),
        (DropSide::Up, (pointer.y - pane.top()) / pane.size.height),
        (
            DropSide::Right,
            (pane.right() - pointer.x) / pane.size.width,
        ),
        (
            DropSide::Down,
            (pane.bottom() - pointer.y) / pane.size.height,
        ),
    ]
    .into_iter()
    .min_by(|(_, a), (_, b)| a.total_cmp(b))
    .expect("the four sides are a non-empty array")
    .0
}

fn drop_preview(mut pane: Bounds<Pixels>, side: DropSide) -> Bounds<Pixels> {
    match side {
        DropSide::Left | DropSide::Right => {
            pane.size.width /= 2.;
            if side == DropSide::Right {
                pane.origin.x += pane.size.width;
            }
        }
        DropSide::Up | DropSide::Down => {
            pane.size.height /= 2.;
            if side == DropSide::Down {
                pane.origin.y += pane.size.height;
            }
        }
    }
    pane
}

pub(crate) fn drop_allowed(pane: Option<Size<Pixels>>, side: DropSide, zoom: f32) -> bool {
    split_allowed(pane, side.orientation(), zoom)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SplitDecision {
    Allowed,
    /// Why the split cannot happen — menu tooltip and error-banner copy alike.
    Blocked(&'static str),
}

/// The one place a split request is judged. Both routes — the menu item and
/// `⌘D` / `⇧⌘D` — go through it, so the size floor cannot be walked around
/// by reaching for the keyboard.
pub(crate) fn split_decision(
    pane: Option<Size<Pixels>>,
    orientation: SplitOrientation,
    zoom: f32,
) -> SplitDecision {
    if split_allowed(pane, orientation, zoom) {
        SplitDecision::Allowed
    } else {
        SplitDecision::Blocked(crate::i18n::text(TOO_SMALL_TO_SPLIT))
    }
}

fn split_menu_items(
    pane: Option<Size<Pixels>>,
    zoom: f32,
    overrides: &keymap::KeymapOverrides,
) -> Vec<UiMenuItem> {
    [
        (
            crate::i18n::message("ui.split_right"),
            "split-pane-right",
            SplitOrientation::Row,
        ),
        (
            crate::i18n::message("ui.split_down"),
            "split-pane-down",
            SplitOrientation::Column,
        ),
    ]
    .into_iter()
    .map(|(label, binding, orientation)| {
        let blocked = match split_decision(pane, orientation, zoom) {
            SplitDecision::Allowed => None,
            SplitDecision::Blocked(reason) => Some(reason),
        };
        let mut item = UiMenuItem::new(label).disabled(blocked.is_some());
        if let Some(combo) = keymap::effective_binding(binding, overrides) {
            item = item.shortcut(keymap::format_combo(&combo));
        }
        if blocked.is_some() {
            item = item.tooltip(crate::i18n::message(TOO_SMALL_TO_SPLIT));
        }
        item
    })
    .collect()
}

fn pane_body_opacity(grouped: bool, focused: bool) -> f32 {
    if grouped && !focused {
        UNFOCUSED_PANE_OPACITY
    } else {
        1.
    }
}

fn pane_identity_icon(runtime: Option<&str>) -> ChatIcon {
    runtime.map_or_else(|| ChatIcon::generic("plus.svg"), ChatIcon::for_runtime)
}

fn starting_overlay_label(runtime: &str, fork_materializing: bool) -> Option<&'static str> {
    if Runtime::parse(runtime).is_some_and(Runtime::is_shell) {
        Some(crate::i18n::text("ui.starting_terminal"))
    } else if fork_materializing {
        Some(crate::i18n::text("ui.forking_chat"))
    } else {
        None
    }
}

fn side_panel_surface(terminal_tab: bool) -> PanelSurface {
    if terminal_tab {
        PanelSurface::Terminal
    } else {
        PanelSurface::Chat
    }
}

/// Whether `surface`'s panel can show at all: Settings → Side panel shows
/// the panel and turns on at least one of its views.
fn side_panel_available(settings: &AppSettings, surface: PanelSurface) -> bool {
    settings.show_side_panel && side_panel::views(surface, settings).next().is_some()
}

fn side_panel_open(settings: &AppSettings, surface: PanelSurface) -> bool {
    settings.chat_panel_open && side_panel_available(settings, surface)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HeaderForkState {
    Enabled,
    Disabled(Option<&'static str>),
    Hidden,
}

fn header_fork_state(
    focused: Option<&DirectSessionEntry>,
    focused_secondary: bool,
) -> HeaderForkState {
    let Some(entry) = focused
        .filter(|entry| !Runtime::parse(&entry.agent_runtime).is_some_and(Runtime::is_shell))
    else {
        return HeaderForkState::Hidden;
    };
    if focused_secondary {
        return HeaderForkState::Hidden;
    }
    if !entry.native_fork {
        HeaderForkState::Disabled(None)
    } else if !entry.forkable {
        HeaderForkState::Disabled(Some(crate::i18n::text("ui.no_session_key_captured_yet")))
    } else {
        HeaderForkState::Enabled
    }
}

pub(crate) fn pane_close_behavior(runtime: Option<&str>) -> PaneCloseBehavior {
    match runtime {
        None => PaneCloseBehavior::LayoutOnly,
        Some(runtime) if Runtime::parse(runtime).is_some_and(Runtime::is_shell) => {
            PaneCloseBehavior::CloseTerminal
        }
        Some(_) => PaneCloseBehavior::ArchiveChat,
    }
}

fn archive_chat_confirm_body(label: &str, running: bool) -> String {
    if running {
        crate::i18n::tf(
            "format.is_still_running_archiving_stops_it_you_can_restore_the_chat_from_settings_arc",
            &[("label", (label).to_string())],
        )
    } else {
        crate::i18n::tf(
            "format.you_can_restore_from_settings_archived",
            &[("label", (label).to_string())],
        )
    }
}

fn pane_lifecycle_action(
    runtime: &str,
    running: bool,
    status: runner_app::ui::agent_status::StatusKind,
    disabled: bool,
    secondary: bool,
) -> Option<SessionControlKind> {
    use runner_app::ui::agent_status::StatusKind;
    if runtime == "shell"
        || disabled
        || secondary
        || matches!(status, StatusKind::Starting | StatusKind::Resuming)
    {
        None
    } else if running {
        Some(SessionControlKind::Stop)
    } else {
        Some(SessionControlKind::Resume)
    }
}

fn top_right_pane(node: &PaneNode) -> &PaneLeaf {
    match node {
        PaneNode::Leaf(leaf) => leaf,
        PaneNode::Split(split) => top_right_pane(match split.orientation {
            SplitOrientation::Row => &split.b,
            SplitOrientation::Column => &split.a,
        }),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PaneMenuAction {
    Rename,
    Fork,
    Pin,
}

/// The title menu's actions: the sidebar row's chat operations minus
/// Archive, which the pane's × already does. A terminal cannot be forked,
/// and a secondary pane leaves forking to the window that controls it.
fn pane_menu_actions(
    runtime: &str,
    secondary: bool,
    pinnable: bool,
    supports: impl Fn(&[&str]) -> bool,
) -> Vec<PaneMenuAction> {
    let mut actions = Vec::new();
    if supports(&["rename"]) {
        actions.push(PaneMenuAction::Rename);
    }
    if runtime != "shell" && !secondary && supports(&["catalog", "fork"]) {
        actions.push(PaneMenuAction::Fork);
    }
    if pinnable && supports(&["pin"]) {
        actions.push(PaneMenuAction::Pin);
    }
    actions
}

fn pane_menu_items(
    actions: &[PaneMenuAction],
    pinned: bool,
    disabled: bool,
    fork_disabled: bool,
    fork_reason: Option<&'static str>,
) -> Vec<UiMenuItem> {
    actions
        .iter()
        .map(|action| match action {
            PaneMenuAction::Rename => {
                UiMenuItem::new(crate::i18n::message("ui.rename_chat_action"))
                    .icon("pencil.svg")
                    .disabled(disabled)
            }
            PaneMenuAction::Fork => {
                let item = UiMenuItem::new(crate::i18n::message("ui.fork_chat_action"))
                    .icon("git-fork.svg")
                    .disabled(disabled || fork_disabled);
                match fork_reason {
                    Some(reason) => item.tooltip(reason),
                    None => item,
                }
            }
            PaneMenuAction::Pin => UiMenuItem::new(if pinned {
                crate::i18n::message("ui.unpin")
            } else {
                crate::i18n::message("ui.pin")
            })
            .icon(if pinned { "pin-off.svg" } else { "pin.svg" })
            .disabled(disabled),
        })
        .collect()
}

/// The pane a tab shows alone. Maximize lasts only while that pane is in a
/// split and holds the tab's focus, so anything that moves the focus to
/// another pane, closes the pane or collapses the split shows the split again.
pub(crate) fn maximized_leaf<'a>(
    maximized: &HashMap<String, String>,
    layout: &'a PaneLayout,
) -> Option<&'a PaneLeaf> {
    let pane_id = maximized.get(&layout.id)?;
    let leaves = layout.root.leaves();
    (leaves.len() > 1 && layout.focused_pane_id == *pane_id)
        .then(|| leaves.into_iter().find(|leaf| leaf.id == *pane_id))
        .flatten()
}

/// Maximizes `pane_id`, which the caller has focused, or restores the split
/// when it already is maximized. Returns whether the pane is maximized now.
pub(crate) fn toggle_maximized(
    maximized: &mut HashMap<String, String>,
    layout: &PaneLayout,
    pane_id: &str,
) -> bool {
    if maximized_leaf(maximized, layout).is_some_and(|leaf| leaf.id == pane_id) {
        maximized.remove(&layout.id);
        return false;
    }
    maximized.insert(layout.id.clone(), pane_id.to_owned());
    if maximized_leaf(maximized, layout).is_none() {
        maximized.remove(&layout.id);
        return false;
    }
    true
}

/// A maximized pane's measured size once its split is restored, from the
/// size measured while it fills the tab. Like the pane tree, each split first
/// gives its resize strip a fixed width and shares the rest by its sizes. The
/// measurement excludes the island's 1 px border, which every pane keeps.
fn restored_pane_size(
    filled: Size<Pixels>,
    layout: &PaneLayout,
    pane_id: &str,
    zoom: f32,
) -> Size<Pixels> {
    fn descend(
        node: &PaneNode,
        size: Size<Pixels>,
        pane_id: &str,
        grab: Pixels,
    ) -> Option<Size<Pixels>> {
        match node {
            PaneNode::Leaf(leaf) => (leaf.id == pane_id).then_some(size),
            PaneNode::Split(split) => {
                let total = split.sizes[0] + split.sizes[1];
                let share = |weight: f32| match split.orientation {
                    SplitOrientation::Row => Size {
                        width: (size.width - grab).max(px(0.)) * (weight / total),
                        height: size.height,
                    },
                    SplitOrientation::Column => Size {
                        width: size.width,
                        height: (size.height - grab).max(px(0.)) * (weight / total),
                    },
                };
                descend(&split.a, share(split.sizes[0]), pane_id, grab)
                    .or_else(|| descend(&split.b, share(split.sizes[1]), pane_id, grab))
            }
        }
    }
    let border = px(2.);
    let outer = Size {
        width: filled.width + border,
        height: filled.height + border,
    };
    descend(
        &layout.root,
        outer,
        pane_id,
        px(runner_app::ui::resize::RESIZE_GRAB * zoom),
    )
    .map_or(filled, |restored| Size {
        width: (restored.width - border).max(px(0.)),
        height: (restored.height - border).max(px(0.)),
    })
}

/// Whether a pane of `layout` shows `session_id`; a pane hidden behind a
/// maximized neighbour does not, though it keeps running.
pub(crate) fn pane_shows_session(
    maximized: &HashMap<String, String>,
    layout: &PaneLayout,
    session_id: &str,
) -> bool {
    match maximized_leaf(maximized, layout) {
        Some(leaf) => leaf.session_id.as_deref() == Some(session_id),
        None => layout.contains_session(session_id),
    }
}

pub(crate) fn prune_maximized(maximized: &mut HashMap<String, String>, layout: &PaneLayout) {
    if maximized.contains_key(&layout.id) && maximized_leaf(maximized, layout).is_none() {
        maximized.remove(&layout.id);
    }
}

pub(crate) fn pane_fractions(node: &PaneNode, pane_id: &str) -> Option<(f32, f32)> {
    match node {
        PaneNode::Leaf(leaf) => (leaf.id == pane_id).then_some((1., 1.)),
        PaneNode::Split(split) => {
            let orientation = split.orientation;
            let first = split.sizes[0] / 100.;
            let second = split.sizes[1] / 100.;
            pane_fractions(&split.a, pane_id)
                .map(|(width, height)| match orientation {
                    SplitOrientation::Row => (width * first, height),
                    SplitOrientation::Column => (width, height * first),
                })
                .or_else(|| {
                    pane_fractions(&split.b, pane_id).map(|(width, height)| match orientation {
                        SplitOrientation::Row => (width * second, height),
                        SplitOrientation::Column => (width, height * second),
                    })
                })
        }
    }
}

pub(crate) fn adjacent_pane_index(
    current: usize,
    pane_count: usize,
    direction: isize,
) -> Option<usize> {
    if pane_count < 2 || current >= pane_count {
        return None;
    }
    Some((current as isize + direction).rem_euclid(pane_count as isize) as usize)
}

#[cfg(test)]
mod tests {
    use super::{
        adjacent_pane_index, archive_chat_confirm_body, drop_allowed, drop_preview, drop_side,
        header_fork_state, maximized_leaf, pane_body_opacity, pane_close_behavior,
        pane_identity_icon, pane_menu_actions, pane_menu_items, pane_rename_key, prune_maximized,
        side_panel_available, side_panel_open, side_panel_surface, split_allowed, split_decision,
        split_menu_items, split_tab_archiving, starting_overlay_label, terminal_drawer_tooltip,
        toggle_maximized, HeaderForkState, PaneCloseBehavior, PaneRenameKey, PanelSurface,
        SplitDecision, MIN_SPLIT_PANE_HEIGHT, MIN_SPLIT_PANE_WIDTH, TOO_SMALL_TO_SPLIT,
        UNFOCUSED_PANE_OPACITY,
    };
    use crate::app_settings::AppSettings;
    use crate::keymap;
    use gpui::{point, px, size, AppContext, Bounds};
    use runner_app::pane_layout::{DropSide, PaneLayout, SplitOrientation};
    use runner_core::protocol::model::SessionStatus;
    use runner_core::protocol::session::DirectSessionEntry;

    fn direct_session(runtime: &str, native_fork: bool, forkable: bool) -> DirectSessionEntry {
        DirectSessionEntry {
            claude_account: None,
            session_id: format!("{runtime}-session"),
            project_id: None,
            role_id: None,
            handle: None,
            agent_runtime: runtime.into(),
            agent_command: runtime.into(),
            agent_model: None,
            agent_effort: None,
            display_name: runtime.into(),
            status: SessionStatus::Running,
            title: None,
            live_title: None,
            cwd: None,
            started_at: None,
            stopped_at: None,
            resumable: forkable,
            native_fork,
            forkable,
            agent_session_key: forkable.then(|| "key".into()),
            pinned: false,
            archived_at: None,
        }
    }

    #[test]
    fn island_lifecycle_controls_follow_lifecycle_and_never_control_shells() {
        use runner_app::ui::{agent_status::StatusKind::*, SessionControlKind};
        for status in [
            Working,
            Idle,
            Interrupted,
            Approval,
            Answer,
            NeedsYou,
            Error,
            ResponseFailed,
            Unavailable,
        ] {
            assert_eq!(
                super::pane_lifecycle_action("codex", true, status, false, false),
                Some(SessionControlKind::Stop)
            );
            assert_eq!(
                super::pane_lifecycle_action("shell", true, status, false, false),
                None
            );
            assert_eq!(
                super::pane_lifecycle_action("codex", true, status, true, false),
                None
            );
            assert_eq!(
                super::pane_lifecycle_action("codex", true, status, false, true),
                None
            );
        }
        for status in [Starting, Resuming] {
            assert_eq!(
                super::pane_lifecycle_action("codex", true, status, false, false),
                None
            );
        }
        assert_eq!(
            super::pane_lifecycle_action("codex", false, Stopped, false, false),
            Some(SessionControlKind::Resume)
        );
        assert_eq!(
            super::pane_lifecycle_action("shell", false, Stopped, false, false),
            None
        );
    }

    #[test]
    fn panel_toggle_follows_the_top_right_corner_across_nested_splits() {
        let mut layout = PaneLayout::single(Some("chat"), &[]);
        let left = layout.focused_pane_id.clone();
        layout.split(&left, SplitOrientation::Row).unwrap();
        let right = layout.focused_pane_id.clone();
        assert_eq!(super::top_right_pane(&layout.root).id, right);
        layout.split(&right, SplitOrientation::Column).unwrap();
        assert_eq!(super::top_right_pane(&layout.root).id, right);
        layout.split(&right, SplitOrientation::Row).unwrap();
        assert_eq!(
            super::top_right_pane(&layout.root).id,
            layout.focused_pane_id
        );
        layout.focus_pane(&left);
        assert_ne!(super::top_right_pane(&layout.root).id, left);
    }

    #[test]
    fn dragging_an_empty_pane_grip_preserves_its_id_and_sibling_chat() {
        use gpui::{Modifiers, MouseButton, TestAppContext, VisualTestContext};
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            let mut layout = PaneLayout::single(Some("direct-agent"), &[]);
            layout.split("p1", SplitOrientation::Row).unwrap();
            {
                let conn = store.test_core.db.get().unwrap();
                conn.execute("UPDATE sessions SET status = 'stopped'", [])
                    .unwrap();
                runner_daemon::repo::node::create_tab(
                    &conn,
                    None,
                    "",
                    0,
                    &layout.serialize().unwrap(),
                )
                .unwrap();
            }
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        });
        let window = cx.add_window(|window, cx| {
            let mut root = crate::NativeRoot::new(
                "empty-pane-drag".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.apply_tab_rows(cx);
            assert!(root.tabs.activate_session("direct-agent"));
            root.route = crate::AppRoute::Chat;
            root.sync_active_chat_detail(cx);
            root
        });
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.simulate_resize(size(px(1600.), px(900.)));
        visual.run_until_parked();
        #[cfg(target_os = "macos")]
        {
            let margin = visual.debug_bounds("WINDOW_TOP_MARGIN_DRAG").unwrap();
            let islands = visual.debug_bounds("APP_CONTENT_COLUMN").unwrap();
            let at = point(islands.center().x, px(4.));
            assert!(margin.contains(&at));
            crate::platform_ui::take_titlebar_actions();
            visual.simulate_mouse_down(at, MouseButton::Left, Modifiers::default());
            assert_eq!(
                crate::platform_ui::take_titlebar_actions(),
                [crate::platform_ui::TitlebarAction::Move]
            );
            visual.simulate_mouse_up(at, MouseButton::Left, Modifiers::default());
            visual.run_until_parked();
        }
        #[cfg(target_os = "macos")]
        {
            let label = window
                .read_with(&visual, |root, cx| {
                    root.front_page_session("direct-agent", cx)
                        .unwrap()
                        .label(None)
                })
                .unwrap();
            let title = visual
                .debug_bounds(Box::leak(format!("PANE_TITLE:{label}").into_boxed_str()))
                .unwrap();
            crate::platform_ui::take_titlebar_actions();
            visual.simulate_event(gpui::MouseDownEvent {
                position: title.center(),
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
                click_count: 2,
                first_mouse: false,
            });
            assert!(
                crate::platform_ui::take_titlebar_actions().is_empty(),
                "title rename must not request a window move or zoom"
            );
            visual.simulate_event(gpui::MouseUpEvent {
                position: title.center(),
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
                click_count: 2,
            });
            visual.run_until_parked();
            window
                .update(&mut visual, |root, _, cx| {
                    assert!(root.pane_rename.is_some());
                    root.pane_rename = None;
                    cx.notify();
                })
                .unwrap();
            visual.run_until_parked();
        }
        let mark = visual.debug_bounds("PANE_DRAG_GRIP:p3").unwrap();
        let target = visual.debug_bounds("PANE_ISLAND:p1").unwrap();
        let destination = point(target.center().x, target.top() + px(10.));
        #[cfg(target_os = "macos")]
        crate::platform_ui::take_titlebar_actions();
        visual.simulate_mouse_down(mark.center(), MouseButton::Left, Modifiers::default());
        #[cfg(target_os = "macos")]
        assert!(
            crate::platform_ui::take_titlebar_actions().is_empty(),
            "pane grip does not request native window movement"
        );
        window
            .read_with(&visual, |root, _| assert!(!root.titlebar_drag_armed))
            .unwrap();
        visual.simulate_mouse_move(
            mark.center() + point(px(12.), px(12.)),
            Some(MouseButton::Left),
            Modifiers::default(),
        );
        visual.simulate_mouse_move(destination, Some(MouseButton::Left), Modifiers::default());
        visual.simulate_mouse_up(destination, MouseButton::Left, Modifiers::default());
        visual.run_until_parked();
        window
            .read_with(&visual, |root, _| {
                let layout = root.tabs.active().unwrap();
                assert_eq!(
                    layout
                        .root
                        .leaves()
                        .iter()
                        .map(|leaf| leaf.id.as_str())
                        .collect::<Vec<_>>(),
                    ["p3", "p1"]
                );
                assert!(layout.root.leaves()[0].session_id.is_none());
                assert_eq!(layout.session_ids(), ["direct-agent"]);
                assert_eq!(layout.focused_pane_id, "p3");
            })
            .unwrap();
    }

    #[test]
    fn split_header_keeps_its_grip_and_actions_within_narrow_zoomed_panes() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            let conn = store.test_core.db.get().unwrap();
            conn.execute("UPDATE sessions SET status='stopped', agent_runtime='claude-code', agent_command='claude', title='Claude Code' WHERE id='direct-agent'", []).unwrap();
            let mut layout = PaneLayout::single(Some("direct-agent"), &[]);
            layout.split("p1", SplitOrientation::Row).unwrap();
            runner_daemon::repo::node::create_tab(&conn, None, "", 0, &layout.serialize().unwrap()).unwrap();
            drop(conn);
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        });
        let window = cx.add_window(|window, cx| {
            let mut root = crate::NativeRoot::new(
                "split-grip-budget".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.tabs.activate_session("direct-agent");
            root.route = crate::AppRoute::Chat;
            root
        });
        let mut visual = gpui::VisualTestContext::from_window(window.into(), &cx);
        let chrome_width = store.read_with(&cx, |store, _| {
            store.settings.sidebar_width + store.settings.chat_panel_width + 26.
        });
        for zoom in [1., 1.3] {
            window
                .update(&mut visual, |root, window, cx| {
                    root.set_zoom(zoom, window, cx)
                })
                .unwrap();
            for width in [240., 360.] {
                visual.simulate_resize(size(
                    px((2. * width + chrome_width) * zoom),
                    px(800. * zoom),
                ));
                visual.run_until_parked();
                let header = visual.debug_bounds("PANE_HEADER:direct-agent").unwrap();
                let grip = visual.debug_bounds("PANE_DRAG_GRIP:p1").unwrap();
                let mark = visual.debug_bounds("PANE_RUNTIME_MARK:p1").unwrap();
                let resume = visual.debug_bounds("PANE_LIFECYCLE:Resume chat").unwrap();
                assert!((grip.size.width - px(14. * zoom)).abs() < px(1.));
                assert!((grip.left() - header.left() - px(8. * zoom)).abs() < px(1.));
                assert!((mark.left() - grip.right() - px(4. * zoom)).abs() < px(1.));
                assert!(grip.left() >= header.left() && resume.right() <= header.right());
                assert!(mark.right() < resume.left());
            }
        }
    }

    #[test]
    fn a_maximized_pane_fills_the_tab_and_its_toggle_restores_the_split() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let other = store.update(&mut cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            store.settings.chat_panel_open = false;
            let conn = store.test_core.db.get().unwrap();
            let mut layout = PaneLayout::single(Some("direct-agent"), &[]);
            let other = layout.split("p1", SplitOrientation::Row).unwrap();
            runner_daemon::repo::node::create_tab(&conn, None, "", 0, &layout.serialize().unwrap())
                .unwrap();
            drop(conn);
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
            other
        });
        let other_island: &str = Box::leak(format!("PANE_ISLAND:{other}").into_boxed_str());
        let other_toggle: &str = Box::leak(format!("PANE_MAXIMIZE:{other}").into_boxed_str());
        let window = cx.add_window(|window, cx| {
            let mut root = crate::NativeRoot::new(
                "maximize-pane".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.tabs.activate_session("direct-agent");
            root.route = crate::AppRoute::Chat;
            root
        });
        let mut visual = gpui::VisualTestContext::from_window(window.into(), &cx);
        visual.simulate_resize(size(px(1600.), px(800.)));
        visual.run_until_parked();
        let split_left = visual.debug_bounds("PANE_ISLAND:p1").unwrap();
        let split_right = visual.debug_bounds(other_island).unwrap();
        assert!(visual.debug_bounds("PANE_MAXIMIZE:p1").is_some());
        assert!(visual.debug_bounds(other_toggle).is_some());
        let persisted = window
            .read_with(&visual, |root, _| {
                root.tabs.active().unwrap().serialize().unwrap()
            })
            .unwrap();

        let toggle = visual.debug_bounds("PANE_MAXIMIZE:p1").unwrap();
        visual.simulate_click(toggle.center(), Default::default());
        visual.run_until_parked();
        let maximized = visual.debug_bounds("PANE_ISLAND:p1").unwrap();
        assert!(visual.debug_bounds(other_island).is_none());
        assert_eq!(maximized.left(), split_left.left());
        assert_eq!(maximized.right(), split_right.right());
        window
            .read_with(&visual, |root, _| {
                assert_eq!(root.tabs.active().unwrap().serialize().unwrap(), persisted);
            })
            .unwrap();

        let toggle = visual.debug_bounds("PANE_MAXIMIZE:p1").unwrap();
        visual.simulate_click(toggle.center(), Default::default());
        visual.run_until_parked();
        assert_eq!(visual.debug_bounds("PANE_ISLAND:p1"), Some(split_left));
        assert_eq!(visual.debug_bounds(other_island), Some(split_right));

        window
            .update(&mut visual, |root, window, cx| {
                root.toggle_maximize_pane(&other, window, cx);
                assert_eq!(root.tabs.active().unwrap().focused_pane_id, other);
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("PANE_ISLAND:p1").is_none());
        window
            .update(&mut visual, |root, window, cx| {
                root.focus_next_chat_pane(&crate::FocusNextPane, window, cx);
            })
            .unwrap();
        visual.run_until_parked();
        assert_eq!(visual.debug_bounds("PANE_ISLAND:p1"), Some(split_left));
        assert_eq!(visual.debug_bounds(other_island), Some(split_right));
        window
            .read_with(&visual, |root, _| {
                assert_eq!(root.tabs.active().unwrap().focused_pane_id, "p1");
                assert!(root.maximized_panes.is_empty());
            })
            .unwrap();

        window
            .update(&mut visual, |root, window, cx| {
                root.toggle_maximize_pane("p1", window, cx);
                root.split_pane("p1", SplitOrientation::Column, window, cx);
                assert!(root.maximized_panes.is_empty());
            })
            .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds(other_island).is_some());
    }

    #[test]
    fn a_maximized_pane_splits_by_its_restored_size() {
        let filled = size(px(958.), px(698.));
        let mut layout = PaneLayout::single(Some("chat"), &[]);
        let left = layout.focused_pane_id.clone();
        let right = layout.split(&left, SplitOrientation::Row).unwrap();
        let restored = super::restored_pane_size(filled, &layout, &left, 1.);
        assert_eq!(
            restored,
            size(px(475.), px(698.)),
            "neither the gutter nor the borders are shared"
        );
        assert!(matches!(
            split_decision(Some(filled), SplitOrientation::Row, 1.),
            SplitDecision::Allowed
        ));
        assert!(matches!(
            split_decision(Some(restored), SplitOrientation::Row, 1.),
            SplitDecision::Blocked(_)
        ));
        assert!(matches!(
            split_decision(Some(restored), SplitOrientation::Column, 1.),
            SplitDecision::Allowed
        ));

        let lower = layout.split(&right, SplitOrientation::Column).unwrap();
        assert_eq!(
            super::restored_pane_size(filled, &layout, &lower, 1.5),
            size(px((960. - 9.) / 2. - 2.), px((700. - 9.) / 2. - 2.)),
            "nested splits each take their zoomed gutter"
        );
    }

    #[test]
    fn title_clicks_select_their_pane_and_maximize_respects_split_floors_and_disconnects() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let right = store.update(&mut cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            store.settings.chat_panel_open = false;
            let conn = store.test_core.db.get().unwrap();
            conn.execute_batch("UPDATE sessions SET title='Left chat' WHERE id='direct-agent'; UPDATE sessions SET title='Right shell' WHERE id='busy-shell';").unwrap();
            let mut layout = PaneLayout::single(Some("direct-agent"), &[]);
            let right = layout.split("p1", SplitOrientation::Row).unwrap();
            layout.assign_session(&right, "busy-shell").unwrap();
            assert!(layout.focus_pane("p1"));
            runner_daemon::repo::node::create_tab(&conn, None, "", 0, &layout.serialize().unwrap()).unwrap();
            drop(conn);
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
            right
        });
        let window = cx.add_window(|window, cx| {
            let mut root = crate::NativeRoot::new(
                "title-focus".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.tabs.activate_session("direct-agent");
            root.route = crate::AppRoute::Chat;
            root
        });
        let mut visual = gpui::VisualTestContext::from_window(window.into(), &cx);
        visual.simulate_resize(size(px(1600.), px(800.)));
        visual.run_until_parked();
        let focused = |visual: &mut gpui::VisualTestContext| {
            window
                .read_with(visual, |root, _| {
                    root.tabs.active().unwrap().focused_pane_id.clone()
                })
                .unwrap()
        };
        assert_eq!(focused(&mut visual), "p1");

        let title = visual.debug_bounds("PANE_TITLE:Right shell").unwrap();
        visual.simulate_click(title.center(), Default::default());
        visual.run_until_parked();
        assert_eq!(
            focused(&mut visual),
            right,
            "a title click selects its pane"
        );
        assert!(visual.debug_bounds("POPOVER_MENU_PANEL").is_some());
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        assert!(visual.debug_bounds("POPOVER_MENU_PANEL").is_none());
        assert_eq!(focused(&mut visual), right);
        window
            .update(&mut visual, |root, window, cx| {
                root.toggle_maximize_pane_action(&crate::ToggleMaximizePane, window, cx)
            })
            .unwrap();
        visual.run_until_parked();
        window
            .read_with(&visual, |root, _| {
                assert_eq!(root.maximized_panes.values().collect::<Vec<_>>(), [&right]);
            })
            .unwrap();
        assert!(visual.debug_bounds("PANE_ISLAND:p1").is_none());

        store.update(&mut visual, |store, cx| {
            store.daemon_disconnected = true;
            store.revisions.error = store.revisions.error.wrapping_add(1);
            cx.notify();
        });
        visual.run_until_parked();
        window
            .read_with(&visual, |root, _| assert!(root.maximized_panes.is_empty()))
            .unwrap();
        store.update(&mut visual, |store, cx| {
            store.daemon_disconnected = false;
            store.revisions.error = store.revisions.error.wrapping_add(1);
            cx.notify();
        });

        // At the floor's edge: side by side each pane is a few pixels short
        // of a split, though it is wide enough while it fills the tab, and
        // maximize must reach the same decision as the split view.
        let split_width = |visual: &mut gpui::VisualTestContext| {
            let left = visual.debug_bounds("PANE_ISLAND:p1").unwrap();
            let right: &str = Box::leak(format!("PANE_ISLAND:{right}").into_boxed_str());
            let right = visual.debug_bounds(right).unwrap();
            (left, right.right() - left.left())
        };
        let (_, tab_width) = split_width(&mut visual);
        let chrome = f32::from(px(1600.) - tab_width);
        for zoom in [1., 1.3] {
            window
                .update(&mut visual, |root, window, cx| {
                    root.set_zoom(zoom, window, cx)
                })
                .unwrap();
            visual.simulate_resize(size(
                px(chrome * zoom + 2. * MIN_SPLIT_PANE_WIDTH * zoom * 2. + 3.),
                px(800.),
            ));
            visual.run_until_parked();
            let split = window
                .read_with(&visual, |root, _| {
                    let tab = root.tabs.active().unwrap().id.clone();
                    root.front_page_pane_bounds()[&super::PaneKey::new(&tab, "p1")]
                })
                .unwrap();
            assert!(
                split.width < px(2. * MIN_SPLIT_PANE_WIDTH * zoom),
                "{split:?}"
            );
            assert!(matches!(
                split_decision(Some(split), SplitOrientation::Row, zoom),
                SplitDecision::Blocked(_)
            ));
            window
                .update(&mut visual, |root, window, cx| {
                    root.toggle_maximize_pane("p1", window, cx)
                })
                .unwrap();
            visual.run_until_parked();
            let filled = visual.debug_bounds("PANE_ISLAND:p1").unwrap();
            assert!(matches!(
                split_decision(Some(filled.size), SplitOrientation::Row, zoom),
                SplitDecision::Allowed
            ));
            window
                .update(&mut visual, |root, window, cx| {
                    let tab = root.tabs.active().unwrap().id.clone();
                    let restored = root.split_pane_bounds(&tab, "p1", cx).unwrap();
                    assert!(
                        (restored.width - split.width).abs() < px(0.5)
                            && (restored.height - split.height).abs() < px(0.5),
                        "zoom {zoom}: estimated {restored:?}, measured {split:?}"
                    );
                    assert!(matches!(
                        split_decision(Some(restored), SplitOrientation::Row, zoom),
                        SplitDecision::Blocked(_)
                    ));
                    root.split_pane("p1", SplitOrientation::Row, window, cx);
                    assert_eq!(root.tabs.active().unwrap().root.leaves().len(), 2);
                    assert!(
                        !root.maximized_panes.is_empty(),
                        "a refused split stays maximized"
                    );
                    root.toggle_maximize_pane("p1", window, cx);
                })
                .unwrap();
            visual.run_until_parked();
        }
        window
            .update(&mut visual, |root, window, cx| {
                root.set_zoom(1., window, cx);
                root.toggle_maximize_pane("p1", window, cx);
            })
            .unwrap();

        visual.simulate_resize(size(px(1600.), px(800.)));
        visual.run_until_parked();
        window
            .update(&mut visual, |root, window, cx| {
                root.split_pane("p1", SplitOrientation::Row, window, cx);
                assert_eq!(root.tabs.active().unwrap().root.leaves().len(), 3);
                assert!(root.maximized_panes.is_empty());
            })
            .unwrap();
    }

    #[test]
    fn clicking_away_from_an_inline_rename_saves_it() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            store.settings.chat_panel_open = false;
            let conn = store.test_core.db.get().unwrap();
            conn.execute_batch("UPDATE sessions SET title='Before' WHERE id='direct-agent'; UPDATE sessions SET title='Right' WHERE id='busy-shell';").unwrap();
            let mut layout = PaneLayout::single(Some("direct-agent"), &[]);
            let right = layout.split("p1", SplitOrientation::Row).unwrap();
            layout.assign_session(&right, "busy-shell").unwrap();
            assert!(layout.focus_pane("p1"));
            runner_daemon::repo::node::create_tab(&conn, None, "", 0, &layout.serialize().unwrap())
                .unwrap();
            drop(conn);
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        });
        let window = cx.add_window(|window, cx| {
            let mut root = crate::NativeRoot::new(
                "rename-blur".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.tabs.activate_session("direct-agent");
            root.route = crate::AppRoute::Chat;
            root
        });
        let mut visual = gpui::VisualTestContext::from_window(window.into(), &cx);
        visual.simulate_resize(size(px(1600.), px(800.)));
        visual.update(|window, _| window.activate_window());
        visual.run_until_parked();
        let input = window
            .update(&mut visual, |root, window, cx| {
                root.begin_pane_rename(
                    "direct-agent".into(),
                    "Before".into(),
                    "codex".into(),
                    window,
                    cx,
                );
                root.pane_rename.as_ref().unwrap().input.clone()
            })
            .unwrap();
        input.update(&mut visual, |field, cx| field.set_text("After", cx));
        visual.run_until_parked();
        let other = visual.debug_bounds("PANE_TITLE:Right").unwrap();
        visual.simulate_click(other.center(), Default::default());
        visual.run_until_parked();
        window
            .read_with(&visual, |root, cx| {
                assert!(root.pane_rename.is_none(), "losing focus ends the rename");
                assert_eq!(
                    root.front_page_session("direct-agent", cx)
                        .and_then(|entry| entry.title),
                    Some("After".into())
                );
            })
            .unwrap();
        visual.simulate_keystrokes("enter");
        visual.run_until_parked();
        window
            .read_with(&visual, |root, _| {
                assert_eq!(
                    root.pane_rename
                        .as_ref()
                        .map(|rename| rename.session_id.as_str()),
                    Some("busy-shell"),
                    "the clicked title's menu keeps the keyboard"
                );
            })
            .unwrap();
    }

    #[test]
    fn pane_title_menu_labels_fit_one_line() {
        use crate::i18n::{testing::with_locale, Language, Locale};
        use gpui::{Focusable, TestAppContext, VisualTestContext};
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for locale in [Locale::English, Locale::SimplifiedChinese] {
            with_locale(locale, || {
                for zoom in [1., 1.5] {
                    for running in [true, false] {
                        let temp = tempfile::tempdir().unwrap();
                        let mut cx = TestAppContext::single();
                        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
                        store.update(&mut cx, |store, cx| {
                            crate::app_store::seed_mixed_working_sessions(store);
                            store.settings.app_zoom = zoom;
                            store.settings.language = if locale == Locale::English { Language::English } else { Language::SimplifiedChinese };
                            if !running {
                                store.test_core.db.get().unwrap().execute("UPDATE sessions SET status = 'stopped' WHERE id = 'direct-agent'", []).unwrap();
                            }
                            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
                            store.sync_locale(cx);
                        });
                        let window = cx.add_window(|window, cx| {
                            let mut root = crate::NativeRoot::new(
                                "pane-menu-fit".into(),
                                temp.path().join("logs"),
                                None,
                                None,
                                store.clone(),
                                window,
                                cx,
                            );
                            root.apply_tab_rows(cx);
                            assert!(root.tabs.activate_session("direct-agent"));
                            root.route = crate::AppRoute::Chat;
                            root.sync_active_chat_detail(cx);
                            root
                        });
                        let mut visual = VisualTestContext::from_window(window.into(), &cx);
                        visual.simulate_resize(size(px(1799.), px(1128.)));
                        visual.run_until_parked();
                        let menu = window
                            .read_with(&visual, |root, _| {
                                root.pane_action_menus["direct-agent"].clone()
                            })
                            .unwrap();
                        visual.update(|window, cx| {
                            menu.read(cx).focus_handle(cx).focus(window, cx);
                        });
                        visual.simulate_keystrokes("down");
                        visual.run_until_parked();
                        let panel = visual.debug_bounds("POPOVER_MENU_PANEL").unwrap();
                        assert!(
                            panel.right() <= px(1799.),
                            "menu must stay inside the window: {panel:?}"
                        );
                        for (label, key) in [
                            ("POPOVER_LABEL_0", "ui.rename_chat_action"),
                            ("POPOVER_LABEL_1", "ui.fork_chat_action"),
                            ("POPOVER_LABEL_2", "ui.pin"),
                        ] {
                            let bounds = visual.debug_bounds(label).unwrap();
                            let full_text_width = visual.update(|window, _| {
                                let text = crate::i18n::text(key);
                                let style = window.text_style();
                                window
                                    .text_system()
                                    .shape_line(
                                        text.into(),
                                        px(13. * zoom),
                                        &[style.to_run(text.len())],
                                        None,
                                    )
                                    .width
                            });
                            assert!(
                                bounds.size.height <= px(18. * zoom + 1.),
                                "{locale:?}, zoom={zoom}, running={running}: {label} {bounds:?}"
                            );
                            assert!(
                                bounds.size.width >= full_text_width,
                                "full menu label must fit: {bounds:?}"
                            );
                        }
                    }
                }
            });
        }
    }

    #[test]
    fn island_status_details_truncate_inside_the_header_without_displacing_controls() {
        use gpui::{TestAppContext, VisualTestContext};
        use runner_core::protocol::status::{
            Activity, AgentObservation, AgentStatus, HumanInteraction, Lifecycle,
            ProgramStatusEvidence, WaitReason, WorkDetail,
        };
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for approval in [true, false] {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            store.update(&mut cx, |store, cx| {
                crate::app_store::seed_mixed_working_sessions(store);
                store.test_core.db.get().unwrap().execute("UPDATE sessions SET title='Header detail fixture' WHERE id='direct-agent'", []).unwrap();
                store.settings.chat_panel_open = false;
                store.settings.sidebar_collapsed = false;
                store.refresh(crate::app_store::StoreRefreshKind::All, cx);
                store.session_statuses.insert("direct-agent".into(), AgentStatus {
                    lifecycle: Lifecycle::Running,
                    observation: AgentObservation {
                        activity: Activity::Working,
                        detail: (!approval).then_some(WorkDetail::CompactingContext),
                        program_status: Some(ProgramStatusEvidence {
                            message: approval.then(|| "Allow this command with a very long program status message? ".repeat(40)),
                        }),
                        interactions: if approval { vec![HumanInteraction {
                            id: "approval".into(), reason: WaitReason::Approval, owners: vec![], since: chrono::Utc::now().timestamp_millis(),
                        }] } else { vec![] },
                        ..Default::default()
                    },
                    ..Default::default()
                });
            });
            let reported = store.read_with(&cx, |store, _| {
                store.session_statuses["direct-agent"].clone()
            });
            let sidebar_width = store.read_with(&cx, |store, _| store.settings.sidebar_width);
            let host = cx.add_window(|window, cx| {
                let mut root = crate::NativeRoot::new(
                    "status-detail-fit".into(),
                    temp.path().join("logs"),
                    None,
                    None,
                    store.clone(),
                    window,
                    cx,
                );
                root.apply_tab_rows(cx);
                assert!(root.tabs.activate_session("direct-agent"));
                root.route = crate::AppRoute::Chat;
                root.sync_active_chat_detail(cx);
                root
            });
            let mut visual = VisualTestContext::from_window(host.into(), &cx);
            for zoom in [1., 1.3] {
                host.update(&mut visual, |root, window, cx| {
                    root.set_zoom(zoom, window, cx)
                })
                .unwrap();
                for pane_width in if approval { [410., 600.] } else { [570., 800.] } {
                    visual.simulate_resize(size(
                        px((pane_width + sidebar_width + 14.) * zoom),
                        px(900.),
                    ));
                    visual.run_until_parked();
                    host.update(&mut visual, |_, _, cx| {
                        store.update(cx, |store, _| {
                            store
                                .session_statuses
                                .insert("direct-agent".into(), reported.clone());
                        });
                        cx.notify();
                    })
                    .unwrap();
                    visual.run_until_parked();
                    let pane = visual.debug_bounds("PANE_ISLAND:p1").unwrap();
                    let header = visual.debug_bounds("PANE_HEADER:direct-agent").unwrap();
                    let status = visual.debug_bounds("ISLAND_STATUS_INDICATOR").unwrap();
                    let state = host
                        .read_with(&visual, |root, cx| {
                            root.front_page_status("direct-agent", cx).unwrap()
                        })
                        .unwrap();
                    assert_eq!(
                        state.kind,
                        if approval {
                            runner_app::ui::agent_status::StatusKind::Approval
                        } else {
                            runner_app::ui::agent_status::StatusKind::Working
                        }
                    );
                    let detail = visual.debug_bounds("ISLAND_STATUS_DETAIL");
                    let actions = visual.debug_bounds("PANE_HEADER_ACTIONS").unwrap();
                    if let Some(detail) = detail {
                        assert!((status.size.width - px(180. * zoom)).abs() < px(0.5));
                        assert!(detail.size.width > px(0.) && detail.right() <= status.right());
                    }
                    assert_eq!(detail.is_some(), pane_width >= 570.);
                    assert!(status.left() >= header.left() && status.right() <= actions.left());
                    let lifecycle = visual.debug_bounds("PANE_LIFECYCLE:Stop chat").unwrap();
                    assert!(
                        lifecycle.left() >= status.right() && lifecycle.right() <= actions.left(), "pane={pane_width}, status={status:?}, action={lifecycle:?}, controls={actions:?}, identity={:?}", visual.debug_bounds("ISLAND_HEADER_IDENTITY")
                    );
                    assert!(
                        (actions.left() - lifecycle.right() - px(8. * zoom + 1.)).abs() < px(0.5)
                    );
                    assert!(actions.right() <= header.right() && header.right() < pane.right());
                    host.update(&mut visual, |_, _, cx| {
                        store.update(cx, |store, _| {
                            store.session_statuses.insert(
                                "direct-agent".into(),
                                AgentStatus {
                                    lifecycle: Lifecycle::Running,
                                    observation: AgentObservation {
                                        activity: Activity::Ready,
                                        ..Default::default()
                                    },
                                    ..Default::default()
                                },
                            );
                        });
                        cx.notify();
                    })
                    .unwrap();
                    visual.run_until_parked();
                    assert!(visual.debug_bounds("ISLAND_STATUS_DETAIL").is_none());
                    assert_eq!(
                        visual.debug_bounds("PANE_LIFECYCLE:Stop chat"),
                        Some(lifecycle),
                        "pane={pane_width}, zoom={zoom}"
                    );
                }
            }
        }
    }

    #[test]
    fn status_grows_leftward_and_the_lifecycle_slot_never_moves() {
        use gpui::{TestAppContext, VisualTestContext};
        use runner_core::protocol::status::{
            Activity, AgentObservation, AgentStatus, HumanInteraction, Lifecycle, WaitReason,
        };
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let status = |activity, approval: bool| AgentStatus {
            lifecycle: Lifecycle::Running,
            observation: AgentObservation {
                activity,
                interactions: if approval {
                    vec![HumanInteraction {
                        id: "approval".into(),
                        reason: WaitReason::Approval,
                        owners: vec![],
                        since: chrono::Utc::now().timestamp_millis(),
                    }]
                } else {
                    vec![]
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            store
                .test_core
                .db
                .get()
                .unwrap()
                .execute(
                    "UPDATE sessions SET title='Slot fixture' WHERE id='direct-agent'",
                    [],
                )
                .unwrap();
            store.settings.chat_panel_open = false;
            store.settings.sidebar_collapsed = false;
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
        });
        let sidebar_width = store.read_with(&cx, |store, _| store.settings.sidebar_width);
        let host = cx.add_window(|window, cx| {
            let mut root = crate::NativeRoot::new(
                "lifecycle-slot".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.apply_tab_rows(cx);
            assert!(root.tabs.activate_session("direct-agent"));
            root.route = crate::AppRoute::Chat;
            root.sync_active_chat_detail(cx);
            root
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        for zoom in [1., 1.3] {
            host.update(&mut visual, |root, window, cx| {
                root.chat_transitions.remove("direct-agent");
                root.set_zoom(zoom, window, cx)
            })
            .unwrap();
            visual.simulate_resize(size(px((700. + sidebar_width + 14.) * zoom), px(900.)));
            let mut slot_at = None;
            for (activity, approval) in [
                (Activity::Working, false),
                (Activity::Ready, false),
                (Activity::Working, true),
            ] {
                host.update(&mut visual, |_, _, cx| {
                    store.update(cx, |store, _| {
                        store
                            .session_statuses
                            .insert("direct-agent".into(), status(activity, approval));
                    });
                    cx.notify();
                })
                .unwrap();
                visual.run_until_parked();
                assert!(
                    visual.debug_bounds("PANE_MAXIMIZE:p1").is_none(),
                    "a single pane has no maximize toggle"
                );
                let title = visual.debug_bounds("PANE_TITLE:Slot fixture").unwrap();
                let indicator = visual.debug_bounds("ISLAND_STATUS_INDICATOR").unwrap();
                let slot = visual.debug_bounds("PANE_LIFECYCLE_SLOT").unwrap();
                let stop = visual.debug_bounds("PANE_LIFECYCLE:Stop chat").unwrap();
                assert_eq!(stop, slot);
                assert!(title.right() < indicator.left());
                assert!(
                    (slot.left() - indicator.right() - px(8. * zoom)).abs() < px(1.),
                    "{activity:?}/{approval}: status {indicator:?}, slot {slot:?}"
                );
                assert_eq!(
                    *slot_at.get_or_insert(slot),
                    slot,
                    "{activity:?}/{approval}"
                );
            }
            host.update(&mut visual, |root, _, cx| {
                root.chat_transitions.insert(
                    "direct-agent".into(),
                    crate::ChatTransition {
                        kind: crate::surfaces::chat_lifecycle::TransitionKind::Starting,
                        started_at: std::time::Instant::now(),
                        baseline_seq: 0,
                        generation: 0,
                    },
                );
                cx.notify();
            })
            .unwrap();
            visual.run_until_parked();
            assert!(visual.debug_bounds("PANE_LIFECYCLE:Stop chat").is_none());
            let slot = visual.debug_bounds("PANE_LIFECYCLE_SLOT").unwrap();
            assert_eq!(Some(slot), slot_at, "Starting keeps the button's place");
            let indicator = visual.debug_bounds("ISLAND_STATUS_INDICATOR").unwrap();
            assert!((slot.left() - indicator.right() - px(8. * zoom)).abs() < px(1.));
        }
    }

    #[test]
    fn terminal_drawer_header_has_no_fill_of_its_own() {
        use crate::theme;
        use crate::theme_snapshot::{assert_fill, ThemeGuard};
        use gpui::prelude::*;
        use gpui::{div, App, Context, Render, TestAppContext, VisualTestContext, Window};
        use std::rc::Rc;

        struct DrawerHeader;
        impl Render for DrawerHeader {
            fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
                div()
                    .size_full()
                    .bg(theme::bg())
                    .child(super::render_terminal_drawer_strip(
                        "probe",
                        &[],
                        None,
                        &[],
                        super::TerminalDrawerCallbacks {
                            can_add: true,
                            can_close: true,
                            activate: Rc::new(|_, _: &mut Window, _: &mut App| {}),
                            close: Rc::new(|_, _, _| {}),
                            add: Rc::new(|_, _| {}),
                            hide: Rc::new(|_, _| {}),
                        },
                    ))
            }
        }
        let _theme = ThemeGuard::new();
        let mut cx = TestAppContext::single();
        let host = cx.add_window(|_, _| DrawerHeader);
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.run_until_parked();
        assert_fill(&mut visual, "DRAWER_HEADER", None);
    }

    #[test]
    fn side_panel_keeps_the_last_focused_chat_while_a_split_terminal_has_focus() {
        use crate::{AppRoute, NativeRoot};
        use gpui::{TestAppContext, VisualTestContext};

        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            let core = store.test_core.clone();
            let conn = core.db.get().unwrap();
            conn.execute_batch(
                "INSERT INTO sessions (id, role_id, mission_id, slot_id, status, agent_runtime, agent_command)
                 VALUES ('second-agent', 'test-shell-role', NULL, NULL, 'stopped', 'codex', 'codex');
                 UPDATE sessions SET status = 'stopped' WHERE mission_id IS NULL;",
            )
            .unwrap();
            let mut layout = PaneLayout::single(Some("direct-agent"), &[]);
            let second = layout.split("p1", SplitOrientation::Row).unwrap();
            layout.assign_session(&second, "second-agent").unwrap();
            let shell = layout.split(&second, SplitOrientation::Row).unwrap();
            layout.assign_session(&shell, "busy-shell").unwrap();
            runner_daemon::repo::node::create_tab(&conn, None, "", 0, &layout.serialize().unwrap())
                .unwrap();
            store.replace_nodes(runner_daemon::repo::node::list(&conn).unwrap(), cx);
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
            store.settings.chat_panel_open = true;
        });
        let host = cx.add_window(|window, cx| {
            let mut root = NativeRoot::new(
                "side-panel-terminal-split".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.apply_tab_rows(cx);
            assert!(root.tabs.activate_session("direct-agent"));
            root.route = AppRoute::Chat;
            root.sync_active_chat_detail(cx);
            root
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(1600.), px(900.)));
        let focus = |session_id: &str, visual: &mut VisualTestContext| {
            host.update(visual, |root, _, cx| {
                assert!(root.tabs.active_mut().unwrap().focus_session(session_id));
                cx.notify();
            })
            .unwrap();
            visual.run_until_parked();
            host.read_with(visual, |root, _| {
                (
                    root.chat_panel_visibility.target,
                    root.active_chat_detail
                        .as_ref()
                        .map(|detail| detail.session_id.clone()),
                )
            })
            .unwrap()
        };

        for (focused, described) in [
            ("direct-agent", "direct-agent"),
            ("busy-shell", "direct-agent"),
            ("second-agent", "second-agent"),
            ("busy-shell", "second-agent"),
        ] {
            assert_eq!(
                focus(focused, &mut visual),
                (1., Some(described.to_owned())),
                "focused {focused}"
            );
            assert!(visual.debug_bounds("CHAT_SIDE_PANEL").is_some());
            assert!(visual.debug_bounds("PANEL_VIEW Info").is_some());
            assert!(visual.debug_bounds("PANEL_VIEW Files").is_some());
        }

        store.update(&mut visual, |store, _| {
            store.settings.chat_panel_open = false
        });
        assert_eq!(focus("busy-shell", &mut visual).0, 0.);
        assert!(visual.debug_bounds("CHAT_PANEL_TOGGLE").is_some());
    }

    fn panel_root(
        session: &'static str,
    ) -> (
        tempfile::TempDir,
        gpui::TestAppContext,
        gpui::Entity<crate::app_store::AppStore>,
        gpui::WindowHandle<crate::NativeRoot>,
        gpui::VisualTestContext,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            store
                .test_core
                .db
                .get()
                .unwrap()
                .execute_batch(
                    "UPDATE sessions SET status = 'stopped' WHERE mission_id IS NULL;
                     UPDATE sessions SET cwd = '/tmp/panel-' || id WHERE mission_id IS NULL;",
                )
                .unwrap();
            store.refresh(crate::app_store::StoreRefreshKind::All, cx);
            store.settings.chat_panel_open = true;
        });
        let host = cx.add_window(|window, cx| {
            let mut root = crate::NativeRoot::new(
                "side-panel-views".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.apply_tab_rows(cx);
            assert!(root.tabs.activate_session(session));
            root.route = crate::AppRoute::Chat;
            root.sync_active_chat_detail(cx);
            root
        });
        let visual = gpui::VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(1600.), px(900.)));
        visual.run_until_parked();
        (temp, cx, store, host, visual)
    }

    fn set_panel_theme(
        store: &gpui::Entity<crate::app_store::AppStore>,
        visual: &mut gpui::VisualTestContext,
        intent: crate::theme::ThemeIntent,
    ) {
        store.update(visual, |store, cx| {
            store.update_settings(
                |settings| {
                    settings.app_theme = intent;
                    true
                },
                false,
                cx,
            );
        });
        visual.run_until_parked();
        visual.refresh().unwrap();
        visual.run_until_parked();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn double_clicking_the_panel_collapse_control_does_not_zoom_the_window() {
        use crate::platform_ui::{take_titlebar_actions, TitlebarAction};
        use gpui::{point, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent};

        let (_temp, _cx, store, host, mut visual) = panel_root("direct-agent");
        let panel = visual.debug_bounds("CHAT_SIDE_PANEL").unwrap();
        let info = visual.debug_bounds("PANEL_VIEW Info").unwrap();
        let collapse = point(panel.right() - px(22.), info.center().y);
        let click = |visual: &mut gpui::VisualTestContext, click_count| {
            visual.simulate_event(MouseDownEvent {
                position: collapse,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
                click_count,
                first_mouse: false,
            });
            visual.simulate_event(MouseUpEvent {
                position: collapse,
                button: MouseButton::Left,
                modifiers: Modifiers::default(),
                click_count,
            });
            visual.run_until_parked();
        };
        take_titlebar_actions();
        click(&mut visual, 1);
        assert!(!store.read_with(&visual, |store, _| store.settings.chat_panel_open));
        assert!(take_titlebar_actions().is_empty());
        host.update(&mut visual, |root, _, cx| {
            // Hold the collapse halfway, where a real double-click's second
            // press lands while the panel is still shrinking.
            root.chat_panel_visibility.start = 0.5;
            root.chat_panel_visibility.target = 0.;
            root.chat_panel_visibility.started_at =
                Some(std::time::Instant::now() + std::time::Duration::from_secs(3600));
            cx.notify();
        })
        .unwrap();
        visual.run_until_parked();
        click(&mut visual, 2);
        assert_eq!(
            take_titlebar_actions(),
            [TitlebarAction::Move],
            "the second click lands on the uncovered drag area as a fresh press"
        );
        click(&mut visual, 1);
        click(&mut visual, 2);
        assert_eq!(
            take_titlebar_actions(),
            [TitlebarAction::Move, TitlebarAction::Zoom],
            "a double-click on the drag area itself still zooms"
        );
    }

    #[test]
    fn chat_panel_header_puts_info_left_and_files_right_in_both_themes() {
        use crate::surfaces::side_panel::{PanelContext, PanelSubject, PanelViewId};
        use crate::theme::{self, ThemeIntent};
        use crate::theme_snapshot::{assert_border, assert_fill_color};

        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let (_temp, _cx, store, host, mut visual) = panel_root("direct-agent");
        let panel = visual.debug_bounds("CHAT_SIDE_PANEL").unwrap();
        let info = visual.debug_bounds("PANEL_VIEW Info").unwrap();
        let files = visual.debug_bounds("PANEL_VIEW Files").unwrap();
        assert!(visual.debug_bounds("PANEL_VIEW Sessions").is_none());
        assert!(info.left() - panel.left() <= px(20.), "{info:?} {panel:?}");
        assert!(info.right() < files.left());
        assert_eq!(info.center().y, files.center().y);
        assert!(
            panel.right() - files.right() >= px(36.) && panel.right() - files.right() <= px(56.),
            "the divider and collapse control follow Files: {files:?} {panel:?}"
        );
        assert!(visual.debug_bounds("CHAT_PANEL_META codex").is_some());

        let selected = || Some(theme::with_alpha(theme::sidebar_selected(), 0.6));
        for (intent, variant) in [
            (ThemeIntent::Dark, theme::ThemeVariant::Carbon),
            (ThemeIntent::Light, theme::ThemeVariant::RunnerLight),
        ] {
            set_panel_theme(&store, &mut visual, intent);
            assert_eq!(theme::active_variant(), variant);
            assert_fill_color(&mut visual, "PANEL_VIEW Info", selected());
            assert_fill_color(&mut visual, "PANEL_VIEW Files", None);
            assert_border(&mut visual, "PANEL_VIEW Files", theme::border());
        }

        #[cfg(target_os = "macos")]
        {
            use crate::platform_ui::{take_titlebar_actions, TitlebarAction};
            let gap = gpui::point((info.right() + files.left()) / 2., info.center().y);
            take_titlebar_actions();
            visual.simulate_mouse_down(gap, gpui::MouseButton::Left, gpui::Modifiers::default());
            visual.simulate_mouse_up(gap, gpui::MouseButton::Left, gpui::Modifiers::default());
            assert_eq!(take_titlebar_actions(), [TitlebarAction::Move]);
            for view in [info, files] {
                for click_count in [1, 2] {
                    visual.simulate_event(gpui::MouseDownEvent {
                        button: gpui::MouseButton::Left,
                        position: view.center(),
                        modifiers: gpui::Modifiers::default(),
                        click_count,
                        first_mouse: false,
                    });
                    visual.simulate_event(gpui::MouseUpEvent {
                        button: gpui::MouseButton::Left,
                        position: view.center(),
                        modifiers: gpui::Modifiers::default(),
                        click_count,
                    });
                    visual.run_until_parked();
                    assert!(
                        take_titlebar_actions().is_empty(),
                        "{view:?} x{click_count}"
                    );
                }
            }
        }

        visual.simulate_click(files.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        assert_eq!(
            host.read_with(&visual, |root, _| root.chat_panel_view)
                .unwrap(),
            PanelViewId::Files
        );
        let context = PanelContext {
            subject: PanelSubject::Session("direct-agent".into()),
            surface: crate::surfaces::side_panel::PanelSurface::Chat,
            cwd: Some("/tmp/panel-direct-agent".into()),
            project: None,
            remote: false,
        };
        assert!(visual
            .debug_bounds(format!("FILES_VIEW {context:?}").leak())
            .is_some());
        assert!(visual.debug_bounds("CHAT_PANEL_META codex").is_none());
        for intent in [ThemeIntent::Dark, ThemeIntent::Light] {
            set_panel_theme(&store, &mut visual, intent);
            assert_fill_color(&mut visual, "PANEL_VIEW Info", None);
            assert_fill_color(&mut visual, "PANEL_VIEW Files", selected());
            assert_border(&mut visual, "PANEL_VIEW Files", theme::border());
        }

        let info = visual.debug_bounds("PANEL_VIEW Info").unwrap();
        visual.simulate_click(info.center(), gpui::Modifiers::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("CHAT_PANEL_META codex").is_some());
    }

    #[test]
    fn terminal_tab_panel_holds_files_alone_and_shares_the_chat_panel_state() {
        use crate::surfaces::side_panel::{PanelContext, PanelSubject, PanelViewId};
        use crate::theme::{self, ThemeIntent};
        use crate::theme_snapshot::{assert_border, assert_fill_color};

        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let (_temp, _cx, store, host, mut visual) = panel_root("busy-shell");
        let panel = visual.debug_bounds("CHAT_SIDE_PANEL").unwrap();
        let width = store.read_with(&visual, |store, _| store.settings.chat_panel_width);
        assert!((panel.size.width - px(width)).abs() <= px(1.), "{panel:?}");
        assert!(visual.debug_bounds("CHAT_PANEL_GUTTER").is_some());
        assert!(visual.debug_bounds("PANEL_VIEW Info").is_none());
        assert!(visual.debug_bounds("PANEL_VIEW Sessions").is_none());
        let files = visual.debug_bounds("PANEL_VIEW Files").unwrap();
        assert!(panel.right() - files.right() <= px(56.));
        let context = PanelContext {
            subject: PanelSubject::Session("busy-shell".into()),
            surface: crate::surfaces::side_panel::PanelSurface::Terminal,
            cwd: Some("/tmp/panel-busy-shell".into()),
            project: None,
            remote: false,
        };
        assert!(visual
            .debug_bounds(format!("FILES_VIEW {context:?}").leak())
            .is_some());
        assert_eq!(
            host.read_with(&visual, |root, _| root.chat_panel_view)
                .unwrap(),
            PanelViewId::Info,
            "showing Files alone keeps the chat's chosen view"
        );
        #[cfg(target_os = "macos")]
        {
            crate::platform_ui::take_titlebar_actions();
            visual.simulate_click(files.center(), gpui::Modifiers::default());
            visual.run_until_parked();
            assert!(crate::platform_ui::take_titlebar_actions().is_empty());
        }
        for intent in [ThemeIntent::Dark, ThemeIntent::Light] {
            set_panel_theme(&store, &mut visual, intent);
            assert_fill_color(
                &mut visual,
                "PANEL_VIEW Files",
                Some(theme::with_alpha(theme::sidebar_selected(), 0.6)),
            );
            assert_border(&mut visual, "PANEL_VIEW Files", theme::border());
        }

        host.update(&mut visual, |root, _, cx| {
            root.update_app_settings(cx, false, |settings| {
                settings.chat_panel_width = 400.;
                true
            });
        })
        .unwrap();
        visual.run_until_parked();
        let panel = visual.debug_bounds("CHAT_SIDE_PANEL").unwrap();
        assert!((panel.size.width - px(400.)).abs() <= px(1.), "{panel:?}");

        host.update(&mut visual, |root, _, cx| {
            root.update_app_settings(cx, false, |settings| {
                settings.chat_panel_open = false;
                true
            });
        })
        .unwrap();
        visual.run_until_parked();
        assert_eq!(
            host.read_with(&visual, |root, _| root.chat_panel_visibility.target)
                .unwrap(),
            0.
        );
        assert!(visual.debug_bounds("CHAT_PANEL_TOGGLE").is_some());
    }

    #[test]
    fn settings_remove_the_chat_panel_and_its_toggle() {
        let (_temp, _cx, _store, host, mut visual) = panel_root("direct-agent");
        let set = |visual: &mut gpui::VisualTestContext, update: fn(&mut AppSettings)| {
            host.update(visual, |root, _, cx| {
                root.update_app_settings(cx, false, |settings| {
                    update(settings);
                    true
                });
            })
            .unwrap();
            visual.run_until_parked();
            host.read_with(visual, |root, _| root.chat_panel_visibility.target)
                .unwrap()
        };
        assert!(visual.debug_bounds("CHAT_SIDE_PANEL").is_some());

        // Show the side panel off: the open panel closes and no toggle
        // offers it back, collapsed or not.
        assert_eq!(
            set(&mut visual, |settings| settings.show_side_panel = false),
            0.
        );
        assert!(visual.debug_bounds("CHAT_PANEL_TOGGLE").is_none());
        set(&mut visual, |settings| settings.chat_panel_open = false);
        assert!(visual.debug_bounds("CHAT_PANEL_TOGGLE").is_none());
        set(&mut visual, |settings| settings.show_side_panel = true);
        assert!(visual.debug_bounds("CHAT_PANEL_TOGGLE").is_some());

        // Every view off: the chat has no panel and no toggle.
        set(&mut visual, |settings| {
            settings.info_view = false;
            settings.files_view = false;
        });
        assert!(visual.debug_bounds("CHAT_PANEL_TOGGLE").is_none());
        assert_eq!(
            set(&mut visual, |settings| settings.chat_panel_open = true),
            0.
        );

        // One view back: the panel opens on it.
        assert_eq!(set(&mut visual, |settings| settings.files_view = true), 1.);
        assert!(visual.debug_bounds("PANEL_VIEW Files").is_some());
        assert!(visual.debug_bounds("PANEL_VIEW Info").is_none());
    }

    #[test]
    fn cached_chat_header_spans_the_chat_column_with_or_without_the_side_panel() {
        use crate::{AppRoute, NativeRoot};
        use gpui::{TestAppContext, VisualTestContext};

        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for panel_open in [false, true] {
            for zoom in [1., 1.5] {
                let temp = tempfile::tempdir().unwrap();
                let mut cx = TestAppContext::single();
                let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
                store.update(&mut cx, |store, cx| {
                    crate::app_store::seed_mixed_working_sessions(store);
                    store
                        .test_core
                        .db
                        .get()
                        .unwrap()
                        .execute(
                            "UPDATE sessions SET status = 'stopped' WHERE mission_id IS NULL",
                            [],
                        )
                        .unwrap();
                    store.refresh(crate::app_store::StoreRefreshKind::All, cx);
                    store.settings.chat_panel_open = panel_open;
                    store.settings.app_zoom = zoom;
                });
                let host = cx.add_window(|window, cx| {
                    let mut root = NativeRoot::new(
                        "chat-header-width".into(),
                        temp.path().join("logs"),
                        None,
                        None,
                        store.clone(),
                        window,
                        cx,
                    );
                    root.apply_tab_rows(cx);
                    assert!(root.tabs.activate_session("direct-agent"));
                    root.route = AppRoute::Chat;
                    root.sync_active_chat_detail(cx);
                    root
                });
                let mut visual = VisualTestContext::from_window(host.into(), &cx);
                for width in [1000., 1600.] {
                    visual.simulate_resize(size(px(width), px(900.)));
                    visual.run_until_parked();
                    let column = visual.debug_bounds("PANE_ISLAND:p1").unwrap();
                    let header = visual.debug_bounds("PANE_HEADER:direct-agent").unwrap();
                    assert!((header.left() - column.left() - px(zoom)).abs() <= px(1.));
                    assert!(
                        (header.right() - column.right() + px(zoom)).abs() <= px(1.),
                        "panel={panel_open}, zoom={zoom}, width={width}"
                    );
                    if panel_open {
                        let panel = visual.debug_bounds("CHAT_SIDE_PANEL").unwrap();
                        let gutter = visual.debug_bounds("CHAT_PANEL_GUTTER").unwrap();
                        assert!((panel.left() - column.right() - px(6. * zoom)).abs() <= px(0.5));
                        assert!((gutter.center().x - panel.left()).abs() <= px(0.5));
                        assert_eq!(gutter.size.width, px(6. * zoom));
                    }
                }
            }
        }
    }

    fn panel_role() -> runner_core::protocol::model::Role {
        runner_core::protocol::model::Role {
            id: "role-architect".into(),
            handle: "architect".into(),
            display_name: "Architect".into(),
            runtime: "claude-code".into(),
            command: "claude".into(),
            args: Vec::new(),
            working_dir: None,
            system_prompt: None,
            env: Default::default(),
            model: Some("fable".into()),
            effort: Some("xhigh".into()),
            codex_speed: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    struct ChatPanel {
        detail: DirectSessionEntry,
        role: Option<runner_core::protocol::model::Role>,
        width: f32,
        copy: gpui::Entity<runner_app::ui::CopyValueButton>,
    }

    impl gpui::Render for ChatPanel {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            use gpui::prelude::*;
            gpui::div()
                .w(gpui::rems(self.width / 16.))
                .p_5()
                .child(super::chat_panel_content(
                    &self.detail,
                    self.role.as_ref(),
                    self.width,
                    self.copy.clone(),
                    super::ChatPanelActions {
                        open_role: self.role.as_ref().map(|_| {
                            std::rc::Rc::new(|_: &mut gpui::Window, _: &mut gpui::App| {})
                                as super::ClickHandler
                        }),
                        toggle_prompt: std::rc::Rc::new(|_, _| {}),
                        expanded: false,
                        view: cx.entity_id(),
                    },
                    self.detail.claude_account.as_ref().map(|account| {
                        (
                            "Work".into(),
                            Some("fixture@example.test".into()),
                            account.folder.clone(),
                        )
                    }),
                    cx,
                ))
        }
    }

    fn panel_harness(
        detail: DirectSessionEntry,
        role: Option<runner_core::protocol::model::Role>,
        width: f32,
    ) -> (gpui::TestAppContext, gpui::VisualTestContext) {
        let mut cx = gpui::TestAppContext::single();
        let window = cx.add_window(move |_, cx| ChatPanel {
            copy: cx.new(|cx| {
                runner_app::ui::CopyValueButton::new(cx.focus_handle(), None, "Copy session_key")
            }),
            detail,
            role,
            width,
        });
        let visual = gpui::VisualTestContext::from_window(window.into(), &cx);
        visual.simulate_resize(size(px(500.), px(800.)));
        visual.run_until_parked();
        (cx, visual)
    }

    #[test]
    fn claude_header_preserves_account_spacing_and_controls_in_narrow_panes() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx,|store,cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            {
                let conn = store.test_core.db.get().unwrap();
                conn.execute("UPDATE sessions SET status='stopped', agent_runtime='claude-code', agent_command='claude', title='Claude Code' WHERE id='direct-agent'",[]).unwrap();
                let accounts = serde_json::json!({"accounts":[{"id":"native","name":"Personal","folder":"/tmp/native","native":true},{"id":"work","name":"Work","folder":"/tmp/work","native":false}],"main_id":"native","active_id":"work"});
                conn.execute("INSERT INTO _app_state(key,value) VALUES('claude_accounts',?1)",[accounts.to_string()]).unwrap();
            }
            store.settings.chat_panel_open = true;
            store.settings.keymap_overrides.insert("resume-session".into(), Some(crate::keymap::KeyCombo { meta: false, ctrl: false, alt: false, shift: false, code: "F8".into(), label: None, shift_optional: false }));
            store.refresh(crate::app_store::StoreRefreshKind::All,cx);
        });
        let sidebar_width = store.read_with(&cx, |store, _| store.settings.sidebar_width);
        let panel_width = store.read_with(&cx, |store, _| store.settings.chat_panel_width);
        let host = cx.add_window(|window, cx| {
            let mut root = crate::NativeRoot::new(
                "account-header".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.apply_tab_rows(cx);
            assert!(root.tabs.activate_session("direct-agent"));
            root.route = crate::AppRoute::Chat;
            root.sync_active_chat_detail(cx);
            root
        });
        let mut visual = gpui::VisualTestContext::from_window(host.into(), &cx);
        for zoom in [1., 1.3] {
            host.update(&mut visual, |root, window, cx| {
                root.set_zoom(zoom, window, cx)
            })
            .unwrap();
            for width in [240., 360., 640.] {
                visual.simulate_resize(size(
                    px((width + sidebar_width + panel_width + 20.) * zoom),
                    px(900.),
                ));
                visual.run_until_parked();
                let header = visual.debug_bounds("PANE_HEADER:direct-agent").unwrap();
                let chip = visual.debug_bounds("CHAT_ACCOUNT_CHIP");
                let name = visual.debug_bounds("PANE_TITLE:Claude Code").unwrap();
                if let Some(chip) = chip {
                    assert!(chip.top() >= header.top() && chip.bottom() <= header.bottom());
                    assert!((chip.left() - name.right() - px(8. * zoom)).abs() < px(1.));
                }
                assert_eq!(chip.is_some(), width >= 360.);
                let indicator = visual.debug_bounds("ISLAND_STATUS_INDICATOR").unwrap();
                let button = visual.debug_bounds("PANE_LIFECYCLE:Resume chat").unwrap();
                let actions = visual.debug_bounds("PANE_HEADER_ACTIONS").unwrap();
                assert!(indicator.left() >= chip.map_or(name.right(), |chip| chip.right()));
                assert!(
                    button.right() <= actions.left(),
                    "{width}: {button:?}, {actions:?}, name={name:?}, chip={chip:?}, status={indicator:?}, identity={:?}", visual.debug_bounds("ISLAND_HEADER_IDENTITY")
                );
                assert!(actions.right() <= header.right());
                assert_eq!(visual.debug_bounds("ISLAND_STATUS_LABEL").is_some(), width >= 360., "width={width}, name={name:?}, chip={chip:?}, header={header:?}, actions={actions:?}, status={indicator:?}");
                if width == 360. {
                    assert!(
                        name.size.width >= px(24. * zoom) && name.size.width < px(70. * zoom),
                        "{name:?}"
                    );
                }
                assert!(visual
                    .debug_bounds("CHAT_STOPPED_TITLE:Chat stopped")
                    .is_some());
                assert!(visual.debug_bounds("BUTTON_SHORTCUT:F8").is_some());
            }
        }
        host.update(&mut visual, |root, _, cx| {
            root.update_app_settings(cx, false, |settings| {
                settings.chat_panel_open = false;
                settings
                    .keymap_overrides
                    .insert("resume-session".into(), None);
                true
            });
            root.chat_panel_visibility =
                crate::surfaces::root::SidebarVisibilityTransition::new(false);
        })
        .unwrap();
        visual.simulate_resize(size(px((240. + sidebar_width + 14.) * 1.3), px(900.)));
        visual.run_until_parked();
        let actions = visual.debug_bounds("PANE_HEADER_ACTIONS").unwrap();
        let lifecycle = visual.debug_bounds("PANE_LIFECYCLE:Resume chat").unwrap();
        assert!(lifecycle.right() <= actions.left(), "lifecycle={lifecycle:?}, actions={actions:?}, identity={:?}, chip={:?}, name={:?}, status={:?}", visual.debug_bounds("ISLAND_HEADER_IDENTITY"), visual.debug_bounds("CHAT_ACCOUNT_CHIP"), visual.debug_bounds("PANE_TITLE:Claude Code"), visual.debug_bounds("ISLAND_STATUS_INDICATOR"));
        assert!(visual.debug_bounds("ISLAND_STATUS_LABEL").is_none());
        assert!(visual.debug_bounds("BUTTON_SHORTCUT:F8").is_none());
    }

    #[test]
    fn account_is_beside_runtime_and_config_is_in_the_metadata_rows() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let mut detail = direct_session("claude-code", true, true);
        detail.claude_account = Some(runner_core::protocol::ClaudeSessionAccount {
            account_id: Some("work".into()),
            folder: "/tmp/work".into(),
            native: false,
            pinned: false,
            origin_folder: "/tmp/native".into(),
            permission_mode: "default".into(),
        });
        for role in [None, Some(panel_role())] {
            let (_cx, mut visual) = panel_harness(detail.clone(), role, 320.);
            let runtime = visual.debug_bounds("SETUP_ROW RUNTIME").unwrap();
            let account = visual.debug_bounds("SETUP_ROW Account").unwrap();
            assert!((runtime.top() - account.top()).abs() <= px(1.));
            assert!(account.left() > runtime.left());
            let config = visual.debug_bounds("CHAT_PANEL_META /tmp/work").unwrap();
            assert!(config.top() > account.bottom());
        }
    }

    #[test]
    fn panel_prompt_only_shows_nonblank_text_and_only_long_prompts_have_a_toggle() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for prompt in ["", " \n ", "# Short\n\nPrompt", &"line\n".repeat(60)] {
            let mut role = panel_role();
            role.system_prompt = Some(prompt.into());
            let (_cx, mut visual) = panel_harness(
                direct_session("codex", true, true),
                Some(role),
                crate::app_settings::CHAT_PANEL_MIN,
            );
            let link = visual.debug_bounds("CHAT_PANEL_OPEN_ROLE").unwrap();
            let label = visual.debug_bounds("CHAT_PANEL_LABEL Role").unwrap();
            assert!((label.center().y - link.center().y).abs() <= gpui::px(1.));
            assert!(link.right() <= gpui::px(crate::app_settings::CHAT_PANEL_MIN));
            if !prompt.trim().is_empty() {
                let label = visual
                    .debug_bounds("CHAT_PANEL_LABEL System prompt")
                    .unwrap();
                let meta = visual.debug_bounds("CHAT_PANEL_PROMPT_META").unwrap();
                assert!((label.center().y - meta.center().y).abs() <= gpui::px(1.));
                assert!(meta.right() <= gpui::px(crate::app_settings::CHAT_PANEL_MIN));
            }
            assert_eq!(
                visual.debug_bounds("CHAT_PANEL_PROMPT_TEXT").is_some(),
                !prompt.trim().is_empty()
            );
            assert_eq!(
                visual.debug_bounds("CHAT_PANEL_PROMPT_TOGGLE").is_some(),
                prompt.lines().count() > crate::surfaces::profile_page::PREVIEW_LINES
            );
        }
    }

    #[test]
    fn role_chat_panel_renders_avatar_runtime_model_and_effort() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let mut detail = direct_session("claude-code", true, true);
        detail.agent_model = Some("fable".into());
        detail.agent_effort = Some("xhigh".into());
        let (_cx, mut visual) = panel_harness(detail, Some(panel_role()), 320.);
        let avatar = visual.debug_bounds("CHAT_PANEL_AVATAR architect").unwrap();
        assert_eq!(avatar.size, size(px(40.), px(40.)));
        for text in [
            "COLUMN_TEXT Architect",
            "COLUMN_TEXT @architect",
            "COLUMN_TEXT Claude Code",
            "COLUMN_TEXT fable",
            "COLUMN_TEXT xhigh",
        ] {
            assert!(
                visual.debug_bounds(text).is_some(),
                "missing rendered {text}"
            );
        }
        assert!(visual.debug_bounds("SETUP_ROW RUNTIME").is_some());
        let model = visual.debug_bounds("SETUP_ROW MODEL").unwrap();
        let effort = visual.debug_bounds("SETUP_ROW EFFORT").unwrap();
        assert_eq!(model.top(), effort.top());
        assert_eq!(model.size.width, effort.size.width);
    }

    #[test]
    fn runtime_chat_panel_renders_mark_and_defaults_without_runtime_row() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for variant in [
            crate::theme::ThemeVariant::Carbon,
            crate::theme::ThemeVariant::RunnerLight,
        ] {
            crate::theme::set_active_variant(variant);
            for (model, effort, default_row) in [
                (None, None, "SETUP_ROW EFFORT"),
                (None, Some("xhigh"), "SETUP_ROW MODEL"),
                (Some("gpt-5.6-sol"), None, "SETUP_ROW EFFORT"),
            ] {
                let mut detail = direct_session("codex", true, true);
                detail.agent_model = model.map(str::to_owned);
                detail.agent_effort = effort.map(str::to_owned);
                let (_cx, mut visual) = panel_harness(detail, None, 320.);
                assert!(visual.debug_bounds("CHAT_PANEL_OPEN_ROLE").is_none());
                assert!(visual.debug_bounds("CHAT_PANEL_PROMPT_TEXT").is_none());
                let mark = visual.debug_bounds("CHAT_PANEL_MARK codex").unwrap();
                assert_eq!(mark.size, size(px(40.), px(40.)));
                assert!(visual.debug_bounds("COLUMN_TEXT Codex").is_some());
                assert!(visual.debug_bounds("SETUP_ROW RUNTIME").is_none());
                let model = visual.debug_bounds("SETUP_ROW MODEL").unwrap();
                let effort = visual.debug_bounds("SETUP_ROW EFFORT").unwrap();
                let row = visual.debug_bounds(default_row).unwrap();
                let default = visual.debug_bounds("COLUMN_TEXT default").unwrap();
                assert_eq!(model.top(), effort.top());
                assert_eq!(model.size, effort.size);
                assert!(default.top() >= row.top() && default.bottom() <= row.bottom());
            }
        }
    }

    #[test]
    fn role_chat_panel_renders_effective_runtime_and_command_override() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let mut detail = direct_session("codex", true, true);
        detail.agent_command = "codex-override".into();
        detail.agent_model = Some("gpt-5.6-sol".into());
        detail.agent_effort = Some("high".into());
        let (_cx, mut visual) = panel_harness(detail, Some(panel_role()), 320.);
        assert!(visual.debug_bounds("COLUMN_TEXT Codex").is_some());
        assert!(visual.debug_bounds("COLUMN_TEXT Claude Code").is_none());
        assert!(visual
            .debug_bounds("CHAT_PANEL_META codex-override")
            .is_some());
        assert!(visual.debug_bounds("CHAT_PANEL_META claude").is_none());
        assert!(visual.debug_bounds("COLUMN_TEXT gpt-5.6-sol").is_some());
        assert!(visual.debug_bounds("COLUMN_TEXT high").is_some());
    }

    #[test]
    fn role_chat_panel_renders_chat_cwd_with_legacy_role_fallback() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for (chat_cwd, role_cwd, expected) in [
            (Some("/chat"), Some("/role"), "CHAT_PANEL_META /chat"),
            (Some("/chat"), None, "CHAT_PANEL_META /chat"),
            (None, Some("/role"), "CHAT_PANEL_META /role"),
        ] {
            let mut detail = direct_session("claude-code", true, true);
            detail.cwd = chat_cwd.map(str::to_owned);
            let mut role = panel_role();
            role.working_dir = role_cwd.map(str::to_owned);
            let (_cx, mut visual) = panel_harness(detail, Some(role), 320.);
            assert!(
                visual.debug_bounds(expected).is_some(),
                "missing rendered {expected}"
            );
            if chat_cwd.is_some() {
                assert!(visual.debug_bounds("CHAT_PANEL_META /role").is_none());
            }
        }
    }

    #[test]
    fn chat_panel_identity_and_setup_fit_at_minimum_width() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let mut role = panel_role();
        role.display_name = "A very long architect display name that must truncate".into();
        role.handle = "a-very-long-architect-handle".into();
        let mut detail = direct_session("antigravity", false, false);
        detail.agent_model = Some("a-very-long-model-name-that-must-truncate".into());
        detail.agent_effort = Some("xhigh".into());
        let (_cx, mut visual) =
            panel_harness(detail, Some(role), crate::app_settings::CHAT_PANEL_MIN);
        for text in [
            "COLUMN_TEXT A very long architect display name that must truncate",
            "COLUMN_TEXT @a-very-long-architect-handle",
            "COLUMN_TEXT Antigravity CLI",
            "COLUMN_TEXT a-very-long-model-name-that-must-truncate",
            "COLUMN_TEXT xhigh",
        ] {
            let bounds = visual
                .debug_bounds(text)
                .unwrap_or_else(|| panic!("missing rendered {text}"));
            assert!(bounds.size.width > px(0.), "{text}: {bounds:?}");
            assert!(
                bounds.left() >= px(35.) && bounds.right() <= px(165.),
                "{text}: {bounds:?}"
            );
            assert!(bounds.size.height <= px(24.), "{text} wrapped: {bounds:?}");
        }
    }

    #[test]
    fn split_tab_shows_one_archiving_pill_only_when_every_pane_archives() {
        let single = PaneLayout::single(Some("a"), &["a".into()]);
        assert!(!split_tab_archiving(&single, |_| true));

        let mut split = PaneLayout::single(Some("a"), &["a".into()]);
        let second = split.split("p1", SplitOrientation::Row).unwrap();
        assert!(split_tab_archiving(&split, |_| true));

        split.assign_session(&second, "b").unwrap();
        assert!(split_tab_archiving(&split, |_| true));
        assert!(!split_tab_archiving(&split, |session_id| session_id == "a"));
        assert!(!split_tab_archiving(&split, |_| false));
    }

    #[test]
    fn adjacent_pane_index_wraps_in_both_directions() {
        assert_eq!(adjacent_pane_index(0, 3, -1), Some(2));
        assert_eq!(adjacent_pane_index(2, 3, 1), Some(0));
        assert_eq!(adjacent_pane_index(1, 3, 1), Some(2));
        assert_eq!(adjacent_pane_index(0, 1, 1), None);
        assert_eq!(adjacent_pane_index(3, 3, -1), None);
    }

    #[test]
    fn drop_side_uses_normalized_edges_and_breaks_diagonal_ties_left_then_up() {
        let pane = Bounds::new(point(px(100.), px(200.)), size(px(800.), px(400.)));
        for (x, y, side) in [
            (0.1, 0.5, DropSide::Left),
            (0.9, 0.5, DropSide::Right),
            (0.5, 0.1, DropSide::Up),
            (0.5, 0.9, DropSide::Down),
            (0.375, 0.5, DropSide::Left),
            (0.25, 0.25, DropSide::Left),
            (0.25, 0.75, DropSide::Left),
            (0.75, 0.25, DropSide::Up),
            (0.75, 0.75, DropSide::Right),
            (0.5, 0.5, DropSide::Left),
            (0., 0., DropSide::Left),
            (1., 0., DropSide::Up),
            (0., 1., DropSide::Left),
            (1., 1., DropSide::Right),
        ] {
            assert_eq!(
                drop_side(pane, pane.origin + point(px(800. * x), px(400. * y))),
                side,
                "({x}, {y})"
            );
        }
    }

    #[test]
    fn drop_preview_is_the_destination_half_in_pane_coordinates() {
        let pane = Bounds::new(point(px(100.), px(200.)), size(px(800.), px(400.)));
        for (side, x, y, width, height) in [
            (DropSide::Left, 100., 200., 400., 400.),
            (DropSide::Right, 500., 200., 400., 400.),
            (DropSide::Up, 100., 200., 800., 200.),
            (DropSide::Down, 100., 400., 800., 200.),
        ] {
            assert_eq!(
                drop_preview(pane, side),
                Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
            );
        }
    }

    #[test]
    fn drop_allowed_respects_both_split_axes_at_each_zoom() {
        for zoom in [1., 1.5] {
            for side in [DropSide::Left, DropSide::Right] {
                assert!(drop_allowed(
                    Some(size(px(480. * zoom), px(160. * zoom))),
                    side,
                    zoom
                ));
                assert!(!drop_allowed(
                    Some(size(px(480. * zoom - 1.), px(160. * zoom))),
                    side,
                    zoom
                ));
            }
            for side in [DropSide::Up, DropSide::Down] {
                assert!(drop_allowed(
                    Some(size(px(240. * zoom), px(320. * zoom))),
                    side,
                    zoom
                ));
                assert!(!drop_allowed(
                    Some(size(px(240. * zoom), px(320. * zoom - 1.))),
                    side,
                    zoom
                ));
            }
        }
    }

    #[test]
    fn header_fork_state_uses_capability_key_and_focused_pane_kind() {
        for runtime in ["claude-code", "codex", "pi"] {
            let entry = direct_session(runtime, true, true);
            assert_eq!(
                header_fork_state(Some(&entry), false),
                HeaderForkState::Enabled
            );
        }

        for runtime in ["trae", "copilot", "antigravity"] {
            let entry = direct_session(runtime, false, false);
            assert_eq!(
                header_fork_state(Some(&entry), false),
                HeaderForkState::Disabled(None)
            );
        }
        let waiting = direct_session("codex", true, false);
        assert_eq!(
            header_fork_state(Some(&waiting), false),
            HeaderForkState::Disabled(Some("No session key captured yet"))
        );
        let shell = direct_session("shell", false, false);
        assert_eq!(
            header_fork_state(Some(&shell), false),
            HeaderForkState::Hidden
        );
        assert_eq!(header_fork_state(None, false), HeaderForkState::Hidden);
        assert_eq!(
            header_fork_state(Some(&waiting), true),
            HeaderForkState::Hidden
        );
    }

    #[test]
    fn starting_overlay_names_fork_materialization_before_normal_startup() {
        assert_eq!(
            starting_overlay_label("claude-code", true),
            Some("Forking chat…")
        );
        assert_eq!(starting_overlay_label("codex", false), None);
        assert_eq!(
            starting_overlay_label("shell", false),
            Some("Starting terminal…")
        );
    }

    #[test]
    fn pane_identity_branches_for_chat_terminal_and_empty_panes() {
        assert_eq!(pane_identity_icon(Some("codex")).asset_path(), "openai.svg");
        assert_eq!(
            pane_identity_icon(Some("claude-code")).asset_path(),
            "claude.svg"
        );
        assert_eq!(pane_identity_icon(Some("trae")).asset_path(), "trae.svg");
        assert_eq!(
            pane_identity_icon(Some("copilot")).asset_path(),
            "copilot.svg"
        );
        assert_eq!(pane_identity_icon(Some("pi")).asset_path(), "pi.svg");
        assert_eq!(
            pane_identity_icon(Some("antigravity")).asset_path(),
            "antigravity-icon.png"
        );
        assert_eq!(
            pane_identity_icon(Some("unknown")).asset_path(),
            "message-square.svg"
        );

        assert_eq!(
            pane_identity_icon(Some("shell")).asset_path(),
            "square-terminal.svg"
        );
        assert_eq!(side_panel_surface(true), PanelSurface::Terminal);
        assert_eq!(side_panel_surface(false), PanelSurface::Chat);
        let open = AppSettings::default();
        let collapsed = AppSettings {
            chat_panel_open: false,
            ..AppSettings::default()
        };
        assert!(side_panel_open(&open, PanelSurface::Terminal));
        assert!(side_panel_open(&open, PanelSurface::Chat));
        assert!(!side_panel_open(&collapsed, PanelSurface::Terminal));
        assert!(side_panel_available(&collapsed, PanelSurface::Terminal));

        // Settings → Side panel → Show the side panel off: no panel and no
        // toggle anywhere, the open state and the switches kept.
        let hidden = AppSettings {
            show_side_panel: false,
            ..AppSettings::default()
        };
        for surface in [PanelSurface::Chat, PanelSurface::Terminal] {
            assert!(!side_panel_open(&hidden, surface));
            assert!(!side_panel_available(&hidden, surface));
        }
        assert!(hidden.chat_panel_open && hidden.info_view && hidden.files_view);
        assert!(!hidden.chat_panel_shown() && !hidden.mission_rail_shown());

        // Every view a surface has turned off: that surface has no panel.
        let files_off = AppSettings {
            files_view: false,
            ..AppSettings::default()
        };
        assert!(side_panel_open(&files_off, PanelSurface::Chat));
        assert!(!side_panel_available(&files_off, PanelSurface::Terminal));
        let all_off = AppSettings {
            info_view: false,
            ..files_off
        };
        assert!(!side_panel_available(&all_off, PanelSurface::Chat));

        assert_eq!(pane_identity_icon(None).asset_path(), "plus.svg");
        assert_eq!(pane_close_behavior(None), PaneCloseBehavior::LayoutOnly);
    }

    #[test]
    fn only_unfocused_panes_in_a_split_are_faded() {
        assert_eq!(UNFOCUSED_PANE_OPACITY, 0.7);
        assert_eq!(pane_body_opacity(true, false), UNFOCUSED_PANE_OPACITY);
        assert_eq!(pane_body_opacity(true, true), 1.);
        assert_eq!(pane_body_opacity(false, false), 1.);
    }

    #[test]
    fn title_menu_lists_rename_fork_and_pin_and_never_stop_or_archive() {
        use super::PaneMenuAction::{Fork, Pin, Rename};
        let every = |_: &[&str]| true;
        assert_eq!(
            pane_menu_actions("codex", false, true, every),
            [Rename, Fork, Pin]
        );
        assert_eq!(
            pane_menu_actions("shell", false, true, every),
            [Rename, Pin]
        );
        assert_eq!(
            pane_menu_actions("codex", true, true, every),
            [Rename, Pin],
            "a secondary pane leaves forking to the window that controls it"
        );
        assert_eq!(
            pane_menu_actions("codex", false, false, every),
            [Rename, Fork]
        );

        let only = |offered: &'static [&'static str]| {
            move |features: &[&str]| features.iter().all(|feature| offered.contains(feature))
        };
        assert_eq!(
            pane_menu_actions("codex", false, true, only(&["rename"])),
            [Rename]
        );
        assert_eq!(pane_menu_actions("codex", false, true, only(&["fork"])), []);
        assert_eq!(
            pane_menu_actions("codex", false, true, only(&["catalog", "fork", "pin"])),
            [Fork, Pin]
        );
        assert_eq!(
            pane_menu_actions("shell", false, true, only(&["catalog", "fork"])),
            []
        );

        let labels = |items: &[crate::UiMenuItem]| {
            items
                .iter()
                .map(|item| item.label.to_string())
                .collect::<Vec<_>>()
        };
        let chat = pane_menu_items(&[Rename, Fork, Pin], false, false, false, None);
        assert_eq!(labels(&chat), ["Rename…", "Fork chat", "Pin"]);
        assert_eq!(
            chat.iter()
                .map(|item| item.icon.as_deref())
                .collect::<Vec<_>>(),
            [Some("pencil.svg"), Some("git-fork.svg"), Some("pin.svg")]
        );
        assert!(chat.iter().all(|item| !item.disabled
            && !item.destructive
            && item.shortcut.is_none()
            && !item.separator_before));

        let pinned = pane_menu_items(&[Rename, Pin], true, false, false, None);
        assert_eq!(labels(&pinned), ["Rename…", "Unpin"]);
        assert_eq!(pinned[1].icon.as_deref(), Some("pin-off.svg"));

        let waiting = pane_menu_items(
            &[Rename, Fork, Pin],
            false,
            false,
            true,
            Some("No session key captured yet"),
        );
        assert!(!waiting[0].disabled && waiting[1].disabled && !waiting[2].disabled);
        assert_eq!(
            waiting[1].tooltip.as_deref(),
            Some("No session key captured yet")
        );

        let secondary = pane_menu_items(&[Rename, Pin], false, true, false, None);
        assert!(secondary.iter().all(|item| item.disabled));
    }

    #[test]
    fn maximize_holds_the_focused_pane_and_ends_on_navigation_split_close_or_single_pane() {
        use std::collections::HashMap;
        let mut layout = PaneLayout::single(Some("chat"), &[]);
        let left = layout.focused_pane_id.clone();
        let mut maximized = HashMap::new();
        assert!(!toggle_maximized(&mut maximized, &layout, &left));
        assert!(
            maximized.is_empty(),
            "a single pane has nothing to maximize"
        );

        let right = layout.split(&left, SplitOrientation::Row).unwrap();
        let runner_app::pane_layout::PaneNode::Split(split) = &layout.root else {
            unreachable!("the tab was just split");
        };
        let split_id = split.id.clone();
        assert!(layout.set_split_sizes(&split_id, [70., 30.]));
        let before = layout.serialize().unwrap();
        assert!(layout.focus_pane(&left));
        assert!(toggle_maximized(&mut maximized, &layout, &left));
        assert_eq!(maximized_leaf(&maximized, &layout).unwrap().id, left);
        assert_eq!(
            layout.serialize().unwrap(),
            before,
            "maximize is view state and never reaches the persisted layout"
        );

        assert!(!toggle_maximized(&mut maximized, &layout, &left));
        assert!(maximized_leaf(&maximized, &layout).is_none());
        assert!(maximized.is_empty());

        assert!(toggle_maximized(&mut maximized, &layout, &left));
        assert!(layout.focus_pane(&right));
        assert!(
            maximized_leaf(&maximized, &layout).is_none(),
            "focusing a hidden pane restores the split"
        );
        prune_maximized(&mut maximized, &layout);
        assert!(maximized.is_empty());
        assert!(layout.focus_pane(&left));
        assert!(
            maximized_leaf(&maximized, &layout).is_none(),
            "returning to the pane does not maximize it again"
        );

        assert!(toggle_maximized(&mut maximized, &layout, &left));
        assert!(layout.close_pane(&left));
        assert!(maximized_leaf(&maximized, &layout).is_none());
        prune_maximized(&mut maximized, &layout);
        assert!(
            maximized.is_empty(),
            "a tab reduced to one pane drops the state"
        );

        let mut layout = PaneLayout::single(Some("chat"), &[]);
        let first = layout.focused_pane_id.clone();
        let second = layout.split(&first, SplitOrientation::Row).unwrap();
        let third = layout.split(&second, SplitOrientation::Column).unwrap();
        assert!(layout.focus_pane(&second));
        assert!(toggle_maximized(&mut maximized, &layout, &second));
        assert!(layout.close_pane(&second));
        prune_maximized(&mut maximized, &layout);
        assert!(
            maximized.is_empty(),
            "closing the maximized pane leaves the rest unmaximized"
        );
        assert_eq!(layout.root.leaves().len(), 2);

        assert!(layout.focus_pane(&third));
        assert!(toggle_maximized(&mut maximized, &layout, &third));
        let mut other = PaneLayout::single(Some("other"), &[]);
        other.id = "other-tab".into();
        let other_left = other.focused_pane_id.clone();
        other.split(&other_left, SplitOrientation::Row).unwrap();
        prune_maximized(&mut maximized, &other);
        assert_eq!(
            maximized_leaf(&maximized, &layout).unwrap().id,
            third,
            "each tab keeps its own maximized pane"
        );
        assert!(maximized_leaf(&maximized, &other).is_none());
    }

    #[test]
    fn pane_close_archives_chats_closes_terminals_and_drops_empty_panes() {
        assert_eq!(pane_close_behavior(None), PaneCloseBehavior::LayoutOnly);
        assert_eq!(
            pane_close_behavior(Some("shell")),
            PaneCloseBehavior::CloseTerminal
        );
        assert_eq!(
            pane_close_behavior(Some("codex")),
            PaneCloseBehavior::ArchiveChat
        );
        assert_eq!(
            pane_close_behavior(Some("claude-code")),
            PaneCloseBehavior::ArchiveChat
        );
    }

    #[test]
    fn archive_chat_confirm_body_warns_only_while_the_agent_runs() {
        assert_eq!(
            archive_chat_confirm_body("@coder", true),
            "@coder is still running. Archiving stops it. You can restore the chat from Settings → Archived."
        );
        assert_eq!(
            archive_chat_confirm_body("@coder", false),
            "You can restore @coder from Settings → Archived."
        );
        assert_eq!(
            archive_chat_confirm_body("this chat", false),
            "You can restore this chat from Settings → Archived."
        );
    }

    #[test]
    fn start_surface_badges_follow_default_rebound_and_unbound_bindings() {
        let mut overrides = keymap::KeymapOverrides::new();
        for (binding, mac, windows) in [
            ("new-chat", "⌘N", "Ctrl+N"),
            ("new-terminal", "⌘T", "Ctrl+Shift+T"),
            ("new-mission", "⇧⌘M", "Ctrl+Shift+M"),
        ] {
            let expected = if cfg!(any(windows, target_os = "linux")) {
                windows
            } else {
                mac
            };
            assert_eq!(
                super::start_action_shortcut(binding, &overrides).as_deref(),
                Some(expected)
            );
            let mut rebound = keymap::entry(binding).unwrap().default.clone();
            rebound.meta = false;
            rebound.ctrl = true;
            rebound.shift = false;
            rebound.code = "KeyJ".into();
            rebound.label = None;
            overrides.insert(binding.into(), Some(rebound));
            let expected = if cfg!(any(windows, target_os = "linux")) {
                "Ctrl+J"
            } else {
                "⌃J"
            };
            assert_eq!(
                super::start_action_shortcut(binding, &overrides).as_deref(),
                Some(expected)
            );
            overrides.insert(binding.into(), None);
            assert_eq!(super::start_action_shortcut(binding, &overrides), None);
        }
    }

    #[test]
    fn start_surface_is_shown_for_no_tab_or_a_single_empty_pane() {
        assert!(super::start_surface_visible(None));
        let mut layout = PaneLayout::single(None, &[]);
        assert!(super::start_surface_visible(Some(&layout)));
        layout.split("p1", SplitOrientation::Row).unwrap();
        assert!(!super::start_surface_visible(Some(&layout)));
        let filled = PaneLayout::single(Some("chat"), &[]);
        assert!(!super::start_surface_visible(Some(&filled)));
    }

    #[cfg(unix)]
    #[test]
    fn start_surface_and_split_pane_variants_render_in_their_own_contexts() {
        use crate::{AppRoute, NativeRoot};
        use gpui::{TestAppContext, VisualTestContext};
        use runner_app::pane_layout::TabSet;
        use runner_core::protocol::node::{NodeRow, NodeType};

        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for variant in [
            "no-tab",
            "no-tab-error",
            "single-empty",
            "single-empty-error",
            "single-empty-drawer",
            "split-empty",
            "filled",
        ] {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            store.update(&mut cx, |store, cx| {
                crate::app_store::seed_mixed_working_sessions(store);
                store
                    .test_core
                    .db
                    .get()
                    .unwrap()
                    .execute("UPDATE sessions SET status = 'stopped'", [])
                    .unwrap();
                store.refresh(crate::app_store::StoreRefreshKind::All, cx);
            });
            let host = cx.add_window(|window, cx| {
                let mut root = NativeRoot::new(
                    "surface-variants".into(),
                    temp.path().join("logs"),
                    None,
                    None,
                    store.clone(),
                    window,
                    cx,
                );
                let mut layout =
                    PaneLayout::single((variant == "filled").then_some("direct-agent"), &[]);
                if variant == "split-empty" {
                    layout.assign_session("p1", "direct-agent").unwrap();
                    layout.split("p1", SplitOrientation::Row).unwrap();
                }
                if variant == "single-empty-drawer" {
                    layout.add_drawer_shell("busy-shell".into());
                }
                let rows = if variant.starts_with("no-tab") {
                    Vec::new()
                } else {
                    vec![NodeRow {
                        id: layout.id.clone(),
                        parent_id: None,
                        position: 0,
                        node_type: NodeType::Tab,
                        name: None,
                        ref_id: None,
                        layout: Some(layout.serialize().unwrap()),
                        pinned_position: None,
                        last_completed_at: None,
                        last_viewed_at: None,
                        created_at: String::new(),
                    }]
                };
                root.tabs = TabSet::from_rows(&rows);
                root.route = AppRoute::Chat;
                if variant.ends_with("error") {
                    root.chat_error = Some("Shell launch failed".into());
                }
                root
            });
            let mut visual = VisualTestContext::from_window(host.into(), &cx);
            visual.simulate_resize(size(px(1440.), px(900.)));
            visual.run_until_parked();
            assert_eq!(
                visual.debug_bounds("PANE_DRAG_GRIP:p1").is_some(),
                variant == "split-empty",
                "{variant}"
            );
            assert_eq!(
                visual.debug_bounds("START_SURFACE").is_some(),
                variant.starts_with("no-tab") || variant.starts_with("single-empty"),
                "{variant}"
            );
            assert_eq!(
                visual.debug_bounds("EMPTY_PANE_START").is_some(),
                variant == "split-empty",
                "{variant}"
            );
            assert_eq!(
                visual.debug_bounds("CHAT_ERROR_BANNER").is_some(),
                variant.ends_with("error"),
                "{variant}"
            );
            assert!(
                visual.debug_bounds("CHAT_TERMINAL_DRAWER").is_none(),
                "{variant}"
            );
            if let Some(surface) = visual.debug_bounds("START_SURFACE") {
                assert!(surface.size.height > px(0.), "{variant}");
                assert!(
                    visual.debug_bounds("CHAT_TAB_HEADER").is_none(),
                    "{variant}"
                );
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn start_surface_column_stays_440_pixels_wide_and_centred() {
        use crate::{AppRoute, NativeRoot};
        use gpui::{TestAppContext, VisualTestContext};
        use runner_app::pane_layout::TabSet;

        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let host = cx.add_window(|window, cx| {
            let mut root = NativeRoot::new(
                "start-surface-width".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.tabs = TabSet::default();
            root.route = AppRoute::Chat;
            root
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        for width in [900., 1440.] {
            visual.simulate_resize(size(px(width), px(900.)));
            visual.run_until_parked();
            let surface = visual.debug_bounds("START_SURFACE").unwrap();
            let column = visual.debug_bounds("START_SURFACE_COLUMN").unwrap();
            assert_eq!(column.size.width, px(440.), "window width {width}");
            assert_eq!(
                column.center().x,
                surface.center().x,
                "window width {width}"
            );
        }
    }

    #[test]
    fn terminal_drawer_tooltip_tracks_rebound_and_unbound_shortcuts() {
        let mut overrides = keymap::KeymapOverrides::new();
        #[cfg(target_os = "macos")]
        assert_eq!(
            terminal_drawer_tooltip(false, &overrides),
            "Show terminal drawer · ⌥F12"
        );
        #[cfg(any(windows, target_os = "linux"))]
        assert_eq!(
            terminal_drawer_tooltip(false, &overrides),
            "Show terminal drawer · Alt+F12"
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            terminal_drawer_tooltip(true, &overrides),
            "Hide terminal drawer · ⌥F12"
        );
        #[cfg(any(windows, target_os = "linux"))]
        assert_eq!(
            terminal_drawer_tooltip(true, &overrides),
            "Hide terminal drawer · Alt+F12"
        );

        let mut rebound = keymap::entry("toggle-terminal-drawer")
            .unwrap()
            .default
            .clone();
        rebound.alt = false;
        rebound.ctrl = true;
        rebound.code = "Backquote".into();
        overrides.insert("toggle-terminal-drawer".into(), Some(rebound));
        #[cfg(target_os = "macos")]
        assert_eq!(
            terminal_drawer_tooltip(false, &overrides),
            "Show terminal drawer · ⌃`"
        );
        #[cfg(any(windows, target_os = "linux"))]
        assert_eq!(
            terminal_drawer_tooltip(false, &overrides),
            "Show terminal drawer · Ctrl+`"
        );

        overrides.insert("toggle-terminal-drawer".into(), None);
        assert_eq!(
            terminal_drawer_tooltip(false, &overrides),
            "Show terminal drawer"
        );
    }

    #[test]
    fn a_split_needs_room_for_both_halves_and_the_floor_scales_with_zoom() {
        let wide = size(
            px(2. * MIN_SPLIT_PANE_WIDTH),
            px(2. * MIN_SPLIT_PANE_HEIGHT),
        );
        assert!(split_allowed(Some(wide), SplitOrientation::Row, 1.));
        assert!(split_allowed(Some(wide), SplitOrientation::Column, 1.));

        let narrow = size(
            px(2. * MIN_SPLIT_PANE_WIDTH - 1.),
            px(2. * MIN_SPLIT_PANE_HEIGHT),
        );
        assert!(!split_allowed(Some(narrow), SplitOrientation::Row, 1.));
        assert!(split_allowed(Some(narrow), SplitOrientation::Column, 1.));

        let short = size(
            px(2. * MIN_SPLIT_PANE_WIDTH),
            px(2. * MIN_SPLIT_PANE_HEIGHT - 1.),
        );
        assert!(split_allowed(Some(short), SplitOrientation::Row, 1.));
        assert!(!split_allowed(Some(short), SplitOrientation::Column, 1.));

        // The same pixels are too small once the app is zoomed in.
        assert!(!split_allowed(Some(wide), SplitOrientation::Row, 1.5));
        assert!(!split_allowed(Some(wide), SplitOrientation::Column, 1.5));
        let zoomed = size(
            px(3. * MIN_SPLIT_PANE_WIDTH),
            px(3. * MIN_SPLIT_PANE_HEIGHT),
        );
        assert!(split_allowed(Some(zoomed), SplitOrientation::Row, 1.5));
        assert!(split_allowed(Some(zoomed), SplitOrientation::Column, 1.5));

        // An unmeasured pane is not disabled on a size we do not have.
        assert!(split_allowed(None, SplitOrientation::Row, 1.));
    }

    // `split_pane` is the single gate both routes pass through — the menu
    // item and `⌘D` / `⇧⌘D` alike — so what it decides is what is pinned here.
    #[test]
    fn chat_and_terminal_splits_meet_the_same_floor_on_every_route() {
        for zoom in [1., 1.5] {
            let roomy = size(
                px(2. * MIN_SPLIT_PANE_WIDTH * zoom),
                px(2. * MIN_SPLIT_PANE_HEIGHT * zoom),
            );
            let narrow = size(roomy.width - px(1.), roomy.height);
            let short = size(roomy.width, roomy.height - px(1.));
            for orientation in [SplitOrientation::Row, SplitOrientation::Column] {
                assert_eq!(
                    split_decision(Some(roomy), orientation, zoom),
                    SplitDecision::Allowed
                );
                assert_eq!(
                    split_decision(None, orientation, zoom),
                    SplitDecision::Allowed
                );
            }
            assert_eq!(
                split_decision(Some(narrow), SplitOrientation::Row, zoom),
                SplitDecision::Blocked(crate::i18n::text(TOO_SMALL_TO_SPLIT))
            );
            assert_eq!(
                split_decision(Some(narrow), SplitOrientation::Column, zoom),
                SplitDecision::Allowed
            );
            assert_eq!(
                split_decision(Some(short), SplitOrientation::Column, zoom),
                SplitDecision::Blocked(crate::i18n::text(TOO_SMALL_TO_SPLIT))
            );
            assert_eq!(
                split_decision(Some(short), SplitOrientation::Row, zoom),
                SplitDecision::Allowed
            );
        }
    }

    #[test]
    fn a_disabled_menu_item_carries_the_reason_the_action_path_would_report() {
        let narrow = size(px(300.), px(800.));
        let items = split_menu_items(Some(narrow), 1., &keymap::KeymapOverrides::new());
        let SplitDecision::Blocked(reason) =
            split_decision(Some(narrow), SplitOrientation::Row, 1.)
        else {
            panic!("a 300 px pane cannot split to the right");
        };

        assert!(items[0].disabled);
        assert_eq!(
            items[0].tooltip.as_ref().map(|tip| tip.as_ref()),
            Some(reason)
        );
    }

    #[test]
    fn the_split_menu_offers_two_directions_with_live_pills_and_says_why_it_is_off() {
        let mut overrides = keymap::KeymapOverrides::new();
        let roomy = size(px(1200.), px(800.));
        let items = split_menu_items(Some(roomy), 1., &overrides);
        assert_eq!(
            items
                .iter()
                .map(|item| item.label.as_ref())
                .collect::<Vec<_>>(),
            ["Split Right", "Split Down"]
        );
        assert!(items.iter().all(|item| !item.disabled));
        assert!(items.iter().all(|item| item.tooltip.is_none()));
        #[cfg(target_os = "macos")]
        assert_eq!(
            items
                .iter()
                .map(|item| item.shortcut.as_ref().map(|pill| pill.as_ref()))
                .collect::<Vec<_>>(),
            [Some("⌘D"), Some("⇧⌘D")]
        );

        overrides.insert("split-pane-right".into(), None);
        let unbound = split_menu_items(Some(roomy), 1., &overrides);
        assert_eq!(unbound[0].shortcut, None);
        assert!(unbound[1].shortcut.is_some());

        let cramped = size(px(300.), px(800.));
        let gated = split_menu_items(Some(cramped), 1., &keymap::KeymapOverrides::new());
        assert!(gated[0].disabled);
        assert_eq!(
            gated[0].tooltip.as_ref().map(|tip| tip.as_ref()),
            Some(crate::i18n::text(TOO_SMALL_TO_SPLIT))
        );
        assert!(!gated[1].disabled);
        assert_eq!(gated[1].tooltip, None);
    }

    #[test]
    fn pane_rename_keys_submit_or_revert_inline_edits() {
        assert_eq!(pane_rename_key("enter"), Some(PaneRenameKey::Submit));
        assert_eq!(pane_rename_key("escape"), Some(PaneRenameKey::Cancel));
        assert_eq!(pane_rename_key("tab"), None);
    }

    #[test]
    fn dropped_files_focus_the_pane_they_land_on() {
        use crate::{AppRoute, NativeRoot};
        use gpui::{ExternalPaths, FileDropEvent, TestAppContext, VisualTestContext};
        use std::sync::Arc;

        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let target_pane = store.update(&mut cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            let core = store.test_core.clone();
            let events: Arc<dyn runner_daemon::session::manager::SessionEvents> =
                Arc::new(core.session_events());
            for id in ["busy-shell", "direct-agent"] {
                core.sessions
                    .prepare_unlisted_terminal(id, (80, 24), &core.db, &events)
                    .unwrap();
                store.bridge.attach(id).unwrap();
            }
            let mut layout = PaneLayout::single(Some("busy-shell"), &[]);
            let target_pane = layout.split("p1", SplitOrientation::Row).unwrap();
            layout.assign_session(&target_pane, "direct-agent").unwrap();
            assert!(layout.focus_pane("p1"));
            let conn = core.db.get().unwrap();
            runner_daemon::repo::node::create_tab(&conn, None, "", 0, &layout.serialize().unwrap())
                .unwrap();
            store.replace_nodes(runner_daemon::repo::node::list(&conn).unwrap(), cx);
            target_pane
        });
        let host = cx.add_window(|window, cx| {
            let mut root = NativeRoot::new(
                "file-drop-test".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.apply_tab_rows(cx);
            assert!(root.tabs.activate_session("busy-shell"));
            root.route = AppRoute::Chat;
            root.sync_active_chat_detail(cx);
            root.ensure_active_tab_attached(window, cx).unwrap();
            root
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(1440.), px(900.)));
        visual.run_until_parked();
        host.update(&mut cx, |root, _, _| {
            assert_eq!(root.tabs.active().unwrap().focused_pane_id, "p1");
        })
        .unwrap();

        let target = visual
            .debug_bounds("TERMINAL_FILE_DROP direct-agent")
            .expect("an interactive terminal is a drop target");
        visual.simulate_event(FileDropEvent::Entered {
            position: target.center(),
            paths: ExternalPaths(["/tmp/two words.png"].into_iter().map(Into::into).collect()),
        });
        visual.simulate_event(FileDropEvent::Submit {
            position: target.center(),
        });
        visual.run_until_parked();

        host.update(&mut cx, |root, window, _| {
            assert_eq!(root.tabs.active().unwrap().focused_pane_id, target_pane);
            assert!(root.attached["direct-agent"]
                .terminal_focus
                .is_focused(window));
            // No process backs the test session, so the daemon refuses the
            // write; the refusal names the terminal the paths were sent to.
            assert_eq!(
                root.chat_error,
                Some("session not found: direct-agent".into())
            );
        })
        .unwrap();
    }
}
