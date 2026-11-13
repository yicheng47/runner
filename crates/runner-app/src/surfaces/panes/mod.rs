//! Chat-surface rendering: the active tab, its split menus, and the pane tree.
mod dialogs;
mod drawer;
mod header;
mod pane;
mod panel;
mod start;
mod tab;
mod tree;

#[cfg(test)]
pub(crate) use dialogs::{single_chat_with_missions_confirm_copy, tab_with_missions_confirm_copy};
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
mod tests;
