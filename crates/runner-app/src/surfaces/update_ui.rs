//! The update flow's shared UI: the sidebar icon and its ring, what clicking it does in each state,
//! and the popover that only Ready has.

use std::f32::consts::{PI, TAU};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    canvas, div, point, px, rems, svg, Animation, AnimationExt, AnyElement, App, Bounds, BoxShadow,
    Context, Entity, FocusHandle, FontWeight, Hsla, KeyDownEvent, MouseButton, PathBuilder, Pixels,
    SharedString, Window,
};
use runner_app::ui::menu::popup_layer;
use runner_app::ui::{Button, ButtonSize, ButtonVariant, Tooltip};
use runner_app::updater::{global_updater, release_page_url, UpdateState, Updater};

use crate::app_store::AppStore;
use crate::surfaces::AppRoute;
use crate::{theme, NativeRoot};

const RING_BOX: f32 = 14.;
const RING_RADIUS: f32 = 5.25;
const RING_STROKE: f32 = 1.5;
const ARC_SWEEP: f32 = 0.25;
const POPOVER_WIDTH: f32 = 264.;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum UpdateIcon {
    Ring(f32),
    Arc,
    Restart,
    Download,
    Warning,
}

pub(crate) fn update_icon(state: &UpdateState) -> Option<UpdateIcon> {
    Some(match state {
        UpdateState::UpToDate { .. } => return None,
        UpdateState::Downloading {
            received, total, ..
        } => match ring_fraction(*received, *total) {
            Some(fraction) => UpdateIcon::Ring(fraction),
            None => UpdateIcon::Arc,
        },
        UpdateState::Verifying { .. } | UpdateState::Installing { .. } => UpdateIcon::Arc,
        UpdateState::Ready { .. } => UpdateIcon::Restart,
        UpdateState::Available { .. } => UpdateIcon::Download,
        UpdateState::Failed { info: Some(_), .. } => UpdateIcon::Warning,
        UpdateState::Failed { info: None, .. } => return None,
    })
}

/// The share of a known download that has arrived; `None` when its size is unknown.
pub(crate) fn ring_fraction(received: u64, total: u64) -> Option<f32> {
    (total > 0).then(|| (received as f64 / total as f64).clamp(0., 1.) as f32)
}

fn percent(fraction: f32) -> String {
    format!("{:.0}", fraction * 100.)
}

fn version_args(state: &UpdateState) -> Vec<(&'static str, String)> {
    let version = state.info().map_or("", |info| info.version());
    vec![("version", version.to_string())]
}

pub(crate) fn update_tooltip(state: &UpdateState) -> Option<String> {
    let args = version_args(state);
    let text = |key: &str| crate::i18n::tf(key, &args);
    update_icon(state)?;
    Some(match state {
        UpdateState::UpToDate { .. } => return None,
        UpdateState::Downloading {
            received, total, ..
        } => match ring_fraction(*received, *total) {
            Some(fraction) => crate::i18n::tf(
                "format.update_tooltip_downloading_percent",
                &[
                    ("version", args[0].1.clone()),
                    ("percent", percent(fraction)),
                ],
            ),
            None => text("format.runner_download_title"),
        },
        UpdateState::Verifying { .. } => text("format.update_verifying"),
        UpdateState::Ready { .. } => text("format.runner_is_ready_to_install"),
        UpdateState::Available {
            manual_only: true, ..
        } => text("format.runner_is_available"),
        UpdateState::Available { size: 0, .. } => text("format.update_tooltip_download"),
        UpdateState::Available { size, .. } => crate::i18n::tf(
            "format.update_tooltip_download_with_size",
            &[
                ("version", args[0].1.clone()),
                ("megabytes", megabytes(*size)),
            ],
        ),
        UpdateState::Installing { .. } => text("format.update_installing"),
        UpdateState::Failed { .. } => text("format.runner_couldn_t_be_installed"),
    })
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1}", bytes as f64 / 1_000_000.)
}

/// Every action the Ready popover and the Settings hero can run.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum UpdateAction {
    Check,
    Download,
    Cancel,
    Restart,
    Retry,
    Skip,
    OpenRelease,
    Later,
}

/// What clicking the sidebar icon does. A download needs no confirmation, because nothing installs
/// until Restart to update. While one is under way a click does nothing, so a second click leaves
/// the person in their terminal; the tooltip has the progress.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IconClick {
    StartDownload,
    OpenPopover,
    OpenSettings,
}

pub(crate) fn icon_click(state: &UpdateState) -> Option<IconClick> {
    update_icon(state)?;
    match state {
        UpdateState::Available {
            manual_only: false, ..
        } => Some(IconClick::StartDownload),
        UpdateState::Ready { .. } => Some(IconClick::OpenPopover),
        UpdateState::Available {
            manual_only: true, ..
        }
        | UpdateState::Failed { .. } => Some(IconClick::OpenSettings),
        UpdateState::UpToDate { .. }
        | UpdateState::Downloading { .. }
        | UpdateState::Verifying { .. }
        | UpdateState::Installing { .. } => None,
    }
}

/// The popover's actions, in Tab order.
const POPOVER_ACTIONS: [UpdateAction; 3] = [
    UpdateAction::Restart,
    UpdateAction::OpenRelease,
    UpdateAction::Later,
];

/// What the popover says. Only Ready has one.
#[derive(Debug, Eq, PartialEq)]
pub(crate) struct ReadyPopover {
    pub(crate) title: String,
    pub(crate) working: Option<String>,
}

pub(crate) fn ready_popover(state: &UpdateState, working: usize) -> Option<ReadyPopover> {
    let UpdateState::Ready { .. } = state else {
        return None;
    };
    Some(ReadyPopover {
        title: crate::i18n::tf("format.update_is_ready", &version_args(state)),
        working: runner_app::lifecycle::working_caption(working),
    })
}

/// Runs an action for the Ready popover or the Settings hero, so both reach the same code.
pub(crate) fn run_update_action(
    action: UpdateAction,
    updater: &Entity<Updater>,
    store: &Entity<AppStore>,
    log_dir: &Path,
    cx: &mut App,
) {
    match action {
        UpdateAction::Check => {
            updater.update(cx, |updater, cx| updater.manual_check(cx));
            persist_skipped_version(store, None, cx);
        }
        UpdateAction::Download => updater.update(cx, |updater, cx| updater.request_download(cx)),
        UpdateAction::Cancel => updater.update(cx, |updater, cx| updater.request_cancel(cx)),
        UpdateAction::Restart => {
            updater.update(cx, |updater, cx| updater.request_install(log_dir, cx))
        }
        UpdateAction::Retry => updater.update(cx, |updater, cx| updater.request_retry(log_dir, cx)),
        UpdateAction::Skip => {
            if let Some(version) = updater.update(cx, |updater, cx| updater.skip_update(cx)) {
                persist_skipped_version(store, Some(version), cx);
            }
        }
        UpdateAction::OpenRelease => cx.open_url(release_page_url()),
        UpdateAction::Later => {}
    }
}

fn persist_skipped_version(store: &Entity<AppStore>, version: Option<String>, cx: &mut App) {
    store.update(cx, |store, cx| {
        store.update_settings(
            |settings| {
                if settings.skipped_update_version == version {
                    return false;
                }
                settings.skipped_update_version = version;
                true
            },
            true,
            cx,
        );
    });
}

/// Runner → Check for Updates: Settings → Updates reports the result.
pub(crate) fn check_for_updates_from_menu(
    root: &mut NativeRoot,
    window: &mut Window,
    cx: &mut Context<NativeRoot>,
) {
    root.open_this_computer_settings(Some("updates"), window, cx);
    let updater = global_updater(cx);
    let store = root.app_store.clone();
    let log_dir = root.log_dir.clone();
    run_update_action(UpdateAction::Check, &updater, &store, &log_dir, cx);
}

/// Splits a clockwise arc from `start` (in turns, 0 at the top) into arcs SVG can express.
fn arc_ends(start: f32, sweep: f32) -> Vec<f32> {
    let sweep = sweep.clamp(0., 1.);
    if sweep <= 0.5 {
        vec![start + sweep]
    } else {
        vec![start + sweep / 2., start + sweep]
    }
}

fn ring_point(center: gpui::Point<Pixels>, radius: f32, turns: f32) -> gpui::Point<Pixels> {
    let angle = turns * TAU - PI / 2.;
    point(
        center.x + px(radius * angle.cos()),
        center.y + px(radius * angle.sin()),
    )
}

fn paint_arc(
    center: gpui::Point<Pixels>,
    radius: f32,
    stroke: f32,
    start: f32,
    sweep: f32,
    color: Hsla,
    window: &mut Window,
) {
    let mut builder = PathBuilder::stroke(px(stroke));
    builder.move_to(ring_point(center, radius, start));
    for end in arc_ends(start, sweep) {
        builder.arc_to(
            point(px(radius), px(radius)),
            px(0.),
            false,
            true,
            ring_point(center, radius, end),
        );
    }
    if let Ok(path) = builder.build() {
        window.paint_path(path, color);
    }
}

fn paint_ring(bounds: Bounds<Pixels>, start: f32, sweep: f32, window: &mut Window) {
    let scale = f32::from(bounds.size.width) / RING_BOX;
    let device = window.scale_factor();
    let snap = |value: Pixels| px((f32::from(value) * device).round() / device);
    let center = bounds.center();
    let center = point(snap(center.x), snap(center.y));
    let (radius, stroke) = (RING_RADIUS * scale, RING_STROKE * scale);
    paint_arc(center, radius, stroke, 0., 1., theme::faint(), window);
    if sweep > 0. {
        paint_arc(
            center,
            radius,
            stroke,
            start,
            sweep,
            theme::accent(),
            window,
        );
    }
}

fn ring(start: f32, sweep: f32) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| paint_ring(bounds, start, sweep, window),
    )
    .size_full()
}

fn icon_box() -> gpui::Div {
    div()
        .relative()
        .flex_none()
        .size(rems(RING_BOX / 16.))
        .debug_selector(|| "UPDATE_ICON".into())
}

fn icon_glyph(path: &'static str, color: Hsla) -> impl IntoElement {
    svg()
        .path(path)
        .flex_none()
        .size(rems(RING_BOX / 16.))
        .text_color(color)
}

/// The icon in its 14 px box, so nothing shifts between states.
pub(crate) fn render_update_icon(icon: UpdateIcon, animate: bool) -> AnyElement {
    match icon {
        UpdateIcon::Ring(fraction) => icon_box().child(ring(0., fraction)).into_any_element(),
        // A rotating arc redraws every frame, so it stops while its window can't be seen.
        UpdateIcon::Arc if animate => icon_box()
            .with_animation(
                "update-arc",
                Animation::new(Duration::from_millis(1100)).repeat(),
                |icon, delta| icon.child(ring(delta, ARC_SWEEP)),
            )
            .into_any_element(),
        UpdateIcon::Arc => icon_box().child(ring(0., ARC_SWEEP)).into_any_element(),
        UpdateIcon::Restart => icon_box()
            .child(icon_glyph("rotate-cw.svg", theme::accent()))
            .into_any_element(),
        UpdateIcon::Download => icon_box()
            .child(icon_glyph("circle-arrow-down.svg", theme::accent()))
            .into_any_element(),
        UpdateIcon::Warning => icon_box()
            .child(icon_glyph("triangle-alert.svg", theme::warning()))
            .into_any_element(),
    }
}

pub(crate) fn working_note(caption: String) -> impl IntoElement {
    div()
        .debug_selector(|| "UPDATE_WORKING_NOTE".into())
        .flex()
        .flex_none()
        .items_center()
        .gap(rems(7. / 16.))
        .child(
            div()
                .debug_selector(|| "UPDATE_WORKING_DOT".into())
                .flex_none()
                .size(rems(6. / 16.))
                .rounded_full()
                .bg(theme::warning()),
        )
        .child(
            div()
                .text_size(theme::text_ui())
                .text_color(theme::muted())
                .child(caption),
        )
}

/// One window's popover: only the window where a download was started opens it by itself.
pub(crate) struct UpdatePopoverState {
    pub(crate) open: bool,
    pub(crate) anchor: Option<Bounds<Pixels>>,
    focus: FocusHandle,
    icon_focus: FocusHandle,
    action_focus: [FocusHandle; POPOVER_ACTIONS.len()],
    previous_focus: Option<FocusHandle>,
    download_started_here: bool,
    tooltip: Arc<Mutex<SharedString>>,
}

impl UpdatePopoverState {
    pub(crate) fn new(cx: &mut App) -> Self {
        Self {
            open: false,
            anchor: None,
            focus: cx.focus_handle(),
            icon_focus: cx.focus_handle().tab_index(0).tab_stop(true),
            action_focus: std::array::from_fn(|_| cx.focus_handle()),
            previous_focus: None,
            download_started_here: false,
            tooltip: Arc::default(),
        }
    }

    fn has_focus(&self, window: &Window) -> bool {
        self.focus.is_focused(window)
            || self
                .action_focus
                .iter()
                .any(|focus| focus.is_focused(window))
    }

    /// For when what's on screen is changing anyway, and focus is moving with it.
    pub(crate) fn close_without_focus(&mut self) {
        self.open = false;
        self.previous_focus = None;
    }
}

/// Tab's next stop among the popover's actions. The window's tab order runs through the whole app,
/// so the popover cycles through its own actions instead.
fn next_action(current: Option<usize>, count: usize, backward: bool) -> usize {
    match (current, backward) {
        (None, false) => 0,
        (None, true) => count - 1,
        (Some(index), false) => (index + 1) % count,
        (Some(index), true) => (index + count - 1) % count,
    }
}

/// What the sidebar should show: nothing when up to date or when the version was skipped.
pub(crate) fn visible_update_state(updater: &Updater) -> Option<UpdateState> {
    (!updater.is_skipped() && update_icon(updater.state()).is_some())
        .then(|| updater.state().clone())
}

impl NativeRoot {
    fn handle_update_icon_click(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.remote_client.is_local() {
            return;
        }
        let state = global_updater(cx).read(cx).state().clone();
        match icon_click(&state) {
            Some(IconClick::StartDownload) => {
                let updater = global_updater(cx);
                let store = self.app_store.clone();
                let log_dir = self.log_dir.clone();
                run_update_action(UpdateAction::Download, &updater, &store, &log_dir, cx);
                self.mark_update_download_started(cx);
            }
            Some(IconClick::OpenPopover) => self.toggle_update_popover(window, cx),
            Some(IconClick::OpenSettings) => self.enter_settings_route(Some("updates"), window, cx),
            None => {}
        }
    }

    fn toggle_update_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.update_popover.open {
            self.close_update_popover(window, cx);
            return;
        }
        self.update_popover.previous_focus = window.focused(cx);
        self.update_popover.open = true;
        self.update_popover.focus.clone().focus(window, cx);
        cx.notify();
    }

    pub(crate) fn close_update_popover(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.update_popover.open = false;
        let previous = self.update_popover.previous_focus.take();
        // Focus that has already moved elsewhere stays there.
        if self.update_popover.has_focus(window) {
            match previous {
                Some(focus) => focus.focus(window, cx),
                None => window.blur(cx),
            }
        }
        cx.notify();
    }

    /// The command palette's way in: the popover at Ready, and Settings otherwise. The palette's
    /// list is from when it opened, so the update may have moved on by now.
    pub(crate) fn show_update_from_keyboard(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ready = matches!(
            visible_update_state(global_updater(cx).read(cx)),
            Some(UpdateState::Ready { .. })
        );
        if !ready || self.route == AppRoute::Settings || self.front_page_sidebar_collapsed() {
            self.enter_settings_route(Some("updates"), window, cx);
            return;
        }
        if !self.update_popover.has_focus(window) {
            self.update_popover.previous_focus = window.focused(cx);
            self.update_popover.focus.clone().focus(window, cx);
        }
        self.update_popover.open = true;
        cx.notify();
    }

    /// Escape for a popover that opened by itself: it closes, and the key still reaches whatever
    /// has focus, so an Escape meant for the terminal isn't lost.
    pub(crate) fn close_unfocused_update_popover(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.update_popover.open && !self.update_popover.has_focus(window) {
            self.close_update_popover(window, cx);
        }
    }

    fn run_update_popover_action(
        &mut self,
        action: UpdateAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let updater = global_updater(cx);
        let store = self.app_store.clone();
        let log_dir = self.log_dir.clone();
        run_update_action(action, &updater, &store, &log_dir, cx);
        if matches!(action, UpdateAction::Restart | UpdateAction::Later) {
            self.close_update_popover(window, cx);
        }
    }

    /// Records that this window started the download, for the Ready popover when it finishes.
    pub(crate) fn mark_update_download_started(&mut self, cx: &mut Context<Self>) {
        self.update_popover.download_started_here = matches!(
            global_updater(cx).read(cx).state(),
            UpdateState::Downloading { .. } | UpdateState::Verifying { .. }
        );
    }

    pub(crate) fn handle_updater_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let updater = global_updater(cx);
        let updater = updater.read(cx);
        let state = updater.state().clone();
        let ready = matches!(
            visible_update_state(updater),
            Some(UpdateState::Ready { .. })
        );
        if self.update_popover.open && !ready {
            self.close_update_popover(window, cx);
        }
        if self.update_popover.download_started_here {
            match state {
                UpdateState::Downloading { .. } | UpdateState::Verifying { .. } => {}
                UpdateState::Ready { .. } => {
                    self.update_popover.download_started_here = false;
                    self.open_ready_update_popover(window, cx);
                }
                _ => self.update_popover.download_started_here = false,
            }
        }
        cx.notify();
    }

    /// Doesn't take focus, so keys keep going to the terminal.
    fn open_ready_update_popover(&mut self, window: &Window, cx: &App) {
        if self.update_popover.open {
            return;
        }
        // Settings shows the same Ready state, and a collapsed sidebar has nothing to anchor to.
        if self.route != AppRoute::Settings
            && !self.front_page_sidebar_collapsed()
            && self.remote_client.is_local()
        {
            self.update_popover.open = true;
            // For a click on one of its actions, which moves focus into it.
            self.update_popover.previous_focus = window.focused(cx);
        }
    }

    pub(crate) fn render_update_hint(
        &self,
        state: &UpdateState,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<impl IntoElement> {
        let icon = update_icon(state)?;
        let tooltip = update_tooltip(state)?;
        *self
            .update_popover
            .tooltip
            .lock()
            .expect("update tooltip lock") = tooltip.into();
        let shared_tooltip = self.update_popover.tooltip.clone();
        let click_root = cx.weak_entity();
        let anchor_root = cx.entity();
        Some(Tooltip::refreshing(
            "sidebar-update-tooltip",
            move || shared_tooltip.lock().expect("update tooltip lock").clone(),
            Duration::from_millis(250),
            div()
                .id("sidebar-update")
                .when(cfg!(test), |button| {
                    button.debug_selector(|| "SIDEBAR_UPDATE".into())
                })
                .track_focus(&self.update_popover.icon_focus)
                .relative()
                .flex_none()
                .size(rems(24. / 16.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .border_1()
                .border_color(gpui::transparent_black())
                .cursor_pointer()
                .hover(|button| {
                    button
                        .border_color(theme::with_alpha(theme::accent(), 0.4))
                        .bg(theme::with_alpha(theme::accent(), 0.1))
                })
                .focus_visible(|button| {
                    button.border_color(theme::with_alpha(theme::accent(), 0.4))
                })
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                // GPUI also clicks a focused element when Enter or Space is released, so a key
                // handler here would act twice.
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    let _ =
                        click_root.update(cx, |root, cx| root.handle_update_icon_click(window, cx));
                })
                .child(render_update_icon(icon, window.is_window_active()))
                .child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, _, cx| {
                            anchor_root
                                .update(cx, |root, _| root.update_popover.anchor = Some(bounds));
                        },
                    )
                    .absolute()
                    .inset_0(),
                ),
        ))
    }

    pub(crate) fn render_update_popover(
        &self,
        window: &Window,
        cx: &Context<Self>,
    ) -> Option<AnyElement> {
        if !self.update_popover.open || !self.remote_client.is_local() {
            return None;
        }
        let anchor = self.update_popover.anchor?;
        let updater = global_updater(cx);
        let state = visible_update_state(updater.read(cx))?;
        let working = self.app_store.read(cx).working_agent_count();
        let content = ready_popover(&state, working)?;
        let zoom = self.settings(cx).app_zoom;
        let dismiss = cx.weak_entity();
        // Room below the icon, so a long version scrolls instead of leaving a small window.
        let max_height =
            (window.viewport_size().height - anchor.bottom() - px(12. * zoom)).max(px(120. * zoom));
        Some(popup_layer(
            anchor,
            window,
            px(POPOVER_WIDTH * zoom),
            self.render_update_popover_panel(content, max_height, cx),
            std::rc::Rc::new(move |window, cx| {
                let _ = dismiss.update(cx, |root, cx| root.close_update_popover(window, cx));
            }),
        ))
    }

    fn render_update_popover_panel(
        &self,
        content: ReadyPopover,
        max_height: Pixels,
        cx: &Context<Self>,
    ) -> AnyElement {
        let root = cx.weak_entity();
        let close_root = root.clone();
        let restart_root = root.clone();
        let [restart_focus, whats_new_focus, later_focus] = &self.update_popover.action_focus;
        let actions = self.update_popover.action_focus.clone();
        let text_action =
            move |id: &'static str, label: &'static str, action, focus: &FocusHandle| {
                let root = root.clone();
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .track_focus(focus)
                    .flex()
                    .items_center()
                    .gap_1()
                    .cursor_pointer()
                    .hover(|link| link.text_color(theme::text()))
                    .focus_visible(|link| link.text_color(theme::text()))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    // GPUI also clicks a focused element when Enter or Space is released.
                    .on_click(move |_, window, cx| {
                        let _ = root.update(cx, |root, cx| {
                            root.run_update_popover_action(action, window, cx)
                        });
                    })
                    .child(label)
            };
        div()
            .id("update-popover")
            .debug_selector(|| "UPDATE_POPOVER".into())
            .w_full()
            .max_h(max_height)
            .overflow_y_scroll()
            .track_focus(&self.update_popover.focus)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => {
                        cx.stop_propagation();
                        let _ =
                            close_root.update(cx, |root, cx| root.close_update_popover(window, cx));
                    }
                    "tab" => {
                        cx.stop_propagation();
                        let current = actions.iter().position(|focus| focus.is_focused(window));
                        let backward = event.keystroke.modifiers.shift;
                        actions[next_action(current, actions.len(), backward)].focus(window, cx);
                    }
                    _ => {}
                }
            })
            .flex()
            .flex_col()
            .gap(rems(10. / 16.))
            .px(rems(14. / 16.))
            .py(rems(12. / 16.))
            .rounded(rems(10. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .shadow(vec![BoxShadow {
                color: gpui::black().opacity(0.25),
                blur_radius: px(16.),
                spread_radius: px(0.),
                inset: false,
                offset: point(px(0.), px(4.)),
            }])
            .child(
                div()
                    .w_full()
                    .whitespace_normal()
                    .text_size(theme::text_body())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .debug_selector(|| "UPDATE_POPOVER_TITLE".into())
                    .child(content.title),
            )
            .children(content.working.map(working_note))
            .child(
                Button::new(
                    "update-popover-restart",
                    crate::i18n::text("ui.restart_to_update"),
                )
                .focus_handle(restart_focus.clone())
                .variant(ButtonVariant::Primary)
                .size(ButtonSize::Sm)
                .full_width(true)
                .radius(7.)
                .icon("rotate-cw.svg")
                .debug_selector("UPDATE_POPOVER_RESTART")
                .on_press(move |window, cx| {
                    let _ = restart_root.update(cx, |root, cx| {
                        root.run_update_popover_action(UpdateAction::Restart, window, cx)
                    });
                }),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_1()
                    .text_size(theme::text_ui())
                    .text_color(theme::muted())
                    .child(
                        text_action(
                            "update-popover-whats-new",
                            crate::i18n::text("ui.whats_new"),
                            UpdateAction::OpenRelease,
                            whats_new_focus,
                        )
                        .font_weight(FontWeight::MEDIUM)
                        .child(
                            svg()
                                .path("arrow-up-right.svg")
                                .flex_none()
                                .size(rems(12. / 16.))
                                .text_color(theme::muted()),
                        ),
                    )
                    .child(div().flex_1())
                    .child(text_action(
                        "update-popover-later",
                        crate::i18n::text("ui.later"),
                        UpdateAction::Later,
                        later_focus,
                    )),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use runner_app::updater::{UpdateInfo, UpdateStep};

    use super::*;

    fn info() -> UpdateInfo {
        UpdateInfo::new("0.19.5")
    }

    fn downloading(received: u64, total: u64) -> UpdateState {
        UpdateState::Downloading {
            info: info(),
            received,
            total,
        }
    }

    fn available(size: u64, manual_only: bool) -> UpdateState {
        UpdateState::Available {
            info: info(),
            size,
            manual_only,
        }
    }

    fn ready() -> UpdateState {
        UpdateState::Ready {
            info: info(),
            size: 40_400_000,
        }
    }

    fn failed() -> UpdateState {
        UpdateState::Failed {
            step: UpdateStep::Download,
            message: "The network connection was lost.".into(),
            info: Some(info()),
        }
    }

    #[test]
    fn tab_cycles_through_the_actions_in_both_directions() {
        assert_eq!(next_action(None, 3, false), 0);
        assert_eq!(next_action(None, 3, true), 2);
        assert_eq!(next_action(Some(0), 3, false), 1);
        assert_eq!(next_action(Some(2), 3, false), 0);
        assert_eq!(next_action(Some(0), 3, true), 2);
        assert_eq!(next_action(Some(1), 3, true), 0);
    }

    #[test]
    fn ring_fraction_is_clamped_and_unknown_sizes_have_none() {
        assert_eq!(ring_fraction(0, 100), Some(0.));
        assert_eq!(ring_fraction(45, 100), Some(0.45));
        assert_eq!(ring_fraction(500, 100), Some(1.));
        assert_eq!(ring_fraction(45, 0), None);
    }

    #[test]
    fn long_arcs_split_so_svg_can_draw_them() {
        assert_eq!(arc_ends(0., 0.25), [0.25]);
        assert_eq!(arc_ends(0.5, 0.5), [1.0]);
        assert_eq!(arc_ends(0., 0.8), [0.4, 0.8]);
        assert_eq!(arc_ends(0., 1.), [0.5, 1.]);
        assert_eq!(arc_ends(0., 7.), [0.5, 1.]);
    }

    #[test]
    fn ring_starts_at_the_top_and_runs_clockwise() {
        let center = point(px(10.), px(10.));
        let top = ring_point(center, 5., 0.);
        assert!((f32::from(top.x) - 10.).abs() < 0.01 && (f32::from(top.y) - 5.).abs() < 0.01);
        let right = ring_point(center, 5., 0.25);
        assert!((f32::from(right.x) - 15.).abs() < 0.01 && (f32::from(right.y) - 10.).abs() < 0.01);
    }

    #[test]
    fn each_state_has_its_icon_tooltip_and_click() {
        let cases = [
            (
                downloading(18_180_000, 40_400_000),
                UpdateIcon::Ring(0.45),
                "Downloading Runner 0.19.5 · 45%",
                None,
            ),
            (
                downloading(18_180_000, 0),
                UpdateIcon::Arc,
                "Downloading Runner 0.19.5",
                None,
            ),
            (
                UpdateState::Verifying { info: info() },
                UpdateIcon::Arc,
                "Verifying Runner 0.19.5",
                None,
            ),
            (
                ready(),
                UpdateIcon::Restart,
                "Runner 0.19.5 is ready to install",
                Some(IconClick::OpenPopover),
            ),
            (
                available(40_400_000, false),
                UpdateIcon::Download,
                "Download Runner 0.19.5 · 40.4 MB",
                Some(IconClick::StartDownload),
            ),
            (
                available(0, false),
                UpdateIcon::Download,
                "Download Runner 0.19.5",
                Some(IconClick::StartDownload),
            ),
            (
                available(40_400_000, true),
                UpdateIcon::Download,
                "Runner 0.19.5 is available",
                Some(IconClick::OpenSettings),
            ),
            (
                UpdateState::Installing { info: info() },
                UpdateIcon::Arc,
                "Installing Runner 0.19.5",
                None,
            ),
            (
                failed(),
                UpdateIcon::Warning,
                "Runner 0.19.5 couldn't be installed",
                Some(IconClick::OpenSettings),
            ),
        ];
        for (state, icon, tooltip, click) in cases {
            assert_eq!(update_icon(&state), Some(icon), "{state:?}");
            assert_eq!(update_tooltip(&state).as_deref(), Some(tooltip));
            assert_eq!(icon_click(&state), click, "{state:?}");
        }
    }

    #[test]
    fn up_to_date_and_a_failed_check_with_no_version_have_no_icon() {
        for state in [
            UpdateState::UpToDate { checking: false },
            UpdateState::UpToDate { checking: true },
            UpdateState::Failed {
                step: UpdateStep::Check,
                message: "offline".into(),
                info: None,
            },
        ] {
            assert_eq!(update_icon(&state), None);
            assert_eq!(update_tooltip(&state), None);
            assert_eq!(icon_click(&state), None);
            assert_eq!(ready_popover(&state, 3), None);
        }
    }

    #[test]
    fn only_ready_has_a_popover_and_it_carries_the_working_note() {
        for state in [
            downloading(1, 2),
            UpdateState::Verifying { info: info() },
            available(1, false),
            available(1, true),
            UpdateState::Installing { info: info() },
            failed(),
        ] {
            assert_eq!(ready_popover(&state, 2), None, "{state:?}");
        }
        assert_eq!(
            ready_popover(&ready(), 0),
            Some(ReadyPopover {
                title: "Runner 0.19.5 is ready".into(),
                working: None,
            })
        );
        assert_eq!(
            ready_popover(&ready(), 2).unwrap().working.as_deref(),
            Some("2 agents working")
        );
    }

    mod in_a_window {
        use gpui::{
            px, size, InputEvent as _, Modifiers, TestAppContext, VisualTestContext, WindowHandle,
        };

        use super::*;
        use crate::theme_snapshot::ThemeGuard;

        struct Opened {
            handle: WindowHandle<NativeRoot>,
            visual: VisualTestContext,
            store: Entity<AppStore>,
            _temp: tempfile::TempDir,
            _cx: TestAppContext,
        }

        fn open_window(
            cx: &mut TestAppContext,
            store: &Entity<AppStore>,
            logs: std::path::PathBuf,
        ) -> WindowHandle<NativeRoot> {
            cx.add_window(|window, cx| {
                NativeRoot::new(
                    "update-ui".into(),
                    logs,
                    None,
                    None,
                    store.clone(),
                    window,
                    cx,
                )
            })
        }

        fn opened() -> Opened {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            let handle = open_window(&mut cx, &store, temp.path().join("logs"));
            let visual = VisualTestContext::from_window(handle.into(), &cx);
            visual.simulate_resize(size(px(1200.), px(900.)));
            visual.run_until_parked();
            Opened {
                handle,
                visual,
                store,
                _temp: temp,
                _cx: cx,
            }
        }

        fn set_state(cx: &mut VisualTestContext, state: UpdateState) {
            cx.update(|_, cx| {
                global_updater(cx).update(cx, |updater, cx| updater.set_state_for_test(state, cx))
            });
            cx.run_until_parked();
        }

        fn set_preview(visual: &mut VisualTestContext, state: UpdateState) {
            visual.update(|_, cx| {
                global_updater(cx).update(cx, |updater, cx| updater.set_preview_for_test(state, cx))
            });
            visual.run_until_parked();
        }

        fn state(visual: &mut VisualTestContext) -> UpdateState {
            visual.update(|_, cx| global_updater(cx).read(cx).state().clone())
        }

        fn is_open(handle: WindowHandle<NativeRoot>, cx: &VisualTestContext) -> bool {
            handle
                .read_with(cx, |root, _| root.update_popover.open)
                .unwrap()
        }

        fn route(handle: WindowHandle<NativeRoot>, cx: &VisualTestContext) -> AppRoute {
            handle.read_with(cx, |root, _| root.route.clone()).unwrap()
        }

        fn focused(
            handle: WindowHandle<NativeRoot>,
            visual: &mut VisualTestContext,
        ) -> FocusHandle {
            handle
                .update(visual, |_, window, cx| {
                    window.focused(cx).expect("the window focuses something")
                })
                .unwrap()
        }

        fn is_focused(
            handle: WindowHandle<NativeRoot>,
            visual: &mut VisualTestContext,
            focus: &FocusHandle,
        ) -> bool {
            handle
                .update(visual, |_, window, _| focus.is_focused(window))
                .unwrap()
        }

        fn click_icon(visual: &mut VisualTestContext) {
            let icon = visual.debug_bounds("SIDEBAR_UPDATE").unwrap();
            visual.simulate_click(icon.center(), Modifiers::default());
            visual.run_until_parked();
        }

        fn click(visual: &mut VisualTestContext, selector: &'static str) {
            let bounds = visual.debug_bounds(selector).unwrap();
            visual.simulate_click(bounds.center(), Modifiers::default());
            visual.run_until_parked();
        }

        /// A whole key press: `simulate_keystrokes` alone sends only the key-down, and GPUI clicks
        /// the focused element on the key-up.
        fn press(visual: &mut VisualTestContext, key: &str) {
            visual.simulate_keystrokes(key);
            visual.run_until_parked();
            let keystroke = gpui::Keystroke::parse(key).unwrap();
            visual.update(|window, cx| {
                window.dispatch_event(gpui::KeyUpEvent { keystroke }.to_platform_input(), cx);
            });
            visual.run_until_parked();
        }

        #[test]
        fn clicking_the_available_icon_starts_one_download_and_changes_the_icon_at_once() {
            let _theme = ThemeGuard::new();
            let Opened {
                handle, mut visual, ..
            } = opened();
            set_preview(&mut visual, available(40_400_000, false));
            let before = route(handle, &visual);
            click_icon(&mut visual);
            assert!(matches!(
                state(&mut visual),
                UpdateState::Downloading { received: 0, .. }
            ));
            assert!(!is_open(handle, &visual));
            assert_eq!(route(handle, &visual), before);
        }

        #[test]
        fn one_key_press_on_the_icon_starts_the_download_and_doesnt_go_on_to_settings() {
            let _theme = ThemeGuard::new();
            for key in ["enter", "space"] {
                let Opened {
                    handle, mut visual, ..
                } = opened();
                set_preview(&mut visual, available(40_400_000, false));
                let before = route(handle, &visual);
                handle
                    .update(&mut visual, |root, window, cx| {
                        root.update_popover.icon_focus.clone().focus(window, cx)
                    })
                    .unwrap();
                visual.run_until_parked();
                press(&mut visual, key);
                assert!(
                    matches!(state(&mut visual), UpdateState::Downloading { .. }),
                    "{key}"
                );
                assert_eq!(route(handle, &visual), before, "{key}");
            }
        }

        #[test]
        fn a_double_click_at_available_starts_one_download_and_stays_where_it_is() {
            let _theme = ThemeGuard::new();
            let Opened {
                handle, mut visual, ..
            } = opened();
            let before = route(handle, &visual);
            let focus = focused(handle, &mut visual);
            set_preview(&mut visual, available(40_400_000, false));

            click_icon(&mut visual);
            // The first click's download has moved on, so a second one would show from zero.
            visual.executor().advance_clock(Duration::from_millis(250));
            visual.run_until_parked();
            let progress = state(&mut visual);
            assert!(matches!(
                progress,
                UpdateState::Downloading { received, .. } if received > 0
            ));

            let position = visual.debug_bounds("SIDEBAR_UPDATE").unwrap().center();
            visual.simulate_event(gpui::MouseDownEvent {
                button: MouseButton::Left,
                position,
                modifiers: Modifiers::default(),
                click_count: 2,
                first_mouse: false,
            });
            visual.simulate_event(gpui::MouseUpEvent {
                button: MouseButton::Left,
                position,
                modifiers: Modifiers::default(),
                click_count: 2,
            });
            visual.run_until_parked();

            assert_eq!(state(&mut visual), progress);
            assert_eq!(route(handle, &visual), before);
            assert!(!is_open(handle, &visual));
            assert!(is_focused(handle, &mut visual, &focus));
        }

        #[test]
        fn clicking_the_icon_does_nothing_while_an_update_is_under_way() {
            let _theme = ThemeGuard::new();
            for update in [
                downloading(18_180_000, 40_400_000),
                downloading(18_180_000, 0),
                UpdateState::Verifying { info: info() },
                UpdateState::Installing { info: info() },
            ] {
                let Opened {
                    handle, mut visual, ..
                } = opened();
                let before = route(handle, &visual);
                // Whatever the window itself rendered and focused, as a terminal would be.
                let focus = focused(handle, &mut visual);
                set_state(&mut visual, update.clone());

                click_icon(&mut visual);

                assert_eq!(route(handle, &visual), before, "{update:?}");
                assert_eq!(state(&mut visual), update);
                assert!(!is_open(handle, &visual), "{update:?}");
                assert!(is_focused(handle, &mut visual, &focus), "{update:?}");
                assert!(visual.debug_bounds("UPDATE_HERO_STATUS").is_none());
                assert!(visual.debug_bounds("UPDATE_POPOVER").is_none());
                // It stays an ordinary icon, with its tooltip.
                assert!(visual.debug_bounds("SIDEBAR_UPDATE").is_some());
            }
        }

        #[test]
        fn failed_and_manual_download_open_settings_from_the_icon() {
            let _theme = ThemeGuard::new();
            for update in [failed(), available(1, true)] {
                let Opened {
                    handle, mut visual, ..
                } = opened();
                set_state(&mut visual, update.clone());
                click_icon(&mut visual);
                assert_eq!(route(handle, &visual), AppRoute::Settings, "{update:?}");
                assert!(!is_open(handle, &visual), "{update:?}");
                assert_eq!(state(&mut visual), update);
                assert!(visual.debug_bounds("UPDATE_HERO_STATUS").is_some());
                assert!(visual.debug_bounds("UPDATE_POPOVER").is_none());
            }
        }

        #[test]
        fn the_ready_icon_opens_the_popover_and_escape_returns_focus() {
            let _theme = ThemeGuard::new();
            let Opened {
                handle, mut visual, ..
            } = opened();
            set_state(&mut visual, ready());
            let previous = focused(handle, &mut visual);
            click_icon(&mut visual);
            assert!(is_open(handle, &visual));
            let icon = visual.debug_bounds("SIDEBAR_UPDATE").unwrap();
            let popover = visual.debug_bounds("UPDATE_POPOVER").unwrap();
            assert!(popover.top() >= icon.bottom());
            assert_eq!(popover.size.width, px(POPOVER_WIDTH));
            assert!(!is_focused(handle, &mut visual, &previous));

            visual.simulate_keystrokes("escape");
            visual.run_until_parked();
            assert!(!is_open(handle, &visual));
            assert!(visual.debug_bounds("UPDATE_POPOVER").is_none());
            assert!(is_focused(handle, &mut visual, &previous));
        }

        #[test]
        fn a_click_outside_closes_the_popover_and_later_keeps_the_icon() {
            let _theme = ThemeGuard::new();
            let Opened {
                handle, mut visual, ..
            } = opened();
            set_state(&mut visual, ready());
            click_icon(&mut visual);
            assert!(is_open(handle, &visual));
            visual.simulate_click(gpui::point(px(900.), px(600.)), Modifiers::default());
            visual.run_until_parked();
            assert!(!is_open(handle, &visual));

            click_icon(&mut visual);
            click(&mut visual, "update-popover-later");
            assert!(!is_open(handle, &visual));
            assert!(visual.debug_bounds("SIDEBAR_UPDATE").is_some());
            assert!(matches!(state(&mut visual), UpdateState::Ready { .. }));
        }

        #[test]
        fn restart_closes_the_popover_and_the_icon_shows_installing() {
            let _theme = ThemeGuard::new();
            let Opened {
                handle, mut visual, ..
            } = opened();
            set_preview(&mut visual, ready());
            let previous = focused(handle, &mut visual);
            click_icon(&mut visual);
            click(&mut visual, "UPDATE_POPOVER_RESTART");
            assert!(!is_open(handle, &visual));
            assert!(matches!(state(&mut visual), UpdateState::Installing { .. }));
            assert!(visual.debug_bounds("SIDEBAR_UPDATE").is_some());
            assert!(is_focused(handle, &mut visual, &previous));
        }

        #[test]
        fn the_popover_closes_when_the_state_leaves_ready_and_returns_focus() {
            let _theme = ThemeGuard::new();
            let Opened {
                handle, mut visual, ..
            } = opened();
            set_state(&mut visual, ready());
            let previous = focused(handle, &mut visual);
            click_icon(&mut visual);
            visual.simulate_keystrokes("tab");
            visual.run_until_parked();
            set_state(&mut visual, failed());
            assert!(!is_open(handle, &visual));
            assert!(is_focused(handle, &mut visual, &previous));
        }

        #[test]
        fn a_download_started_from_the_icon_opens_ready_only_in_that_window_without_focus() {
            let _theme = ThemeGuard::new();
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            let started = open_window(&mut cx, &store, temp.path().join("logs"));
            let other = open_window(&mut cx, &store, temp.path().join("logs"));
            let mut visual = VisualTestContext::from_window(started.into(), &cx);
            visual.simulate_resize(size(px(1200.), px(900.)));
            visual.run_until_parked();
            let focus = focused(started, &mut visual);

            set_preview(&mut visual, available(40_400_000, false));
            click_icon(&mut visual);
            assert!(matches!(
                state(&mut visual),
                UpdateState::Downloading { .. }
            ));
            set_state(&mut visual, UpdateState::Verifying { info: info() });
            assert!(!is_open(started, &visual));
            set_state(&mut visual, ready());
            assert!(is_open(started, &visual));
            assert!(!is_open(other, &visual));
            assert!(visual.debug_bounds("UPDATE_POPOVER").is_some());
            assert!(is_focused(started, &mut visual, &focus));

            // Only once per download.
            started
                .update(&mut visual, |root, window, cx| {
                    root.close_update_popover(window, cx)
                })
                .unwrap();
            set_state(&mut visual, available(1, false));
            set_state(&mut visual, ready());
            assert!(!is_open(started, &visual));
        }

        #[test]
        fn an_automatic_download_only_changes_the_icon() {
            let _theme = ThemeGuard::new();
            let Opened {
                handle, mut visual, ..
            } = opened();
            set_state(&mut visual, downloading(1, 2));
            set_state(&mut visual, ready());
            assert!(!is_open(handle, &visual));
            assert!(visual.debug_bounds("SIDEBAR_UPDATE").is_some());
        }

        #[test]
        fn a_download_started_from_the_hero_opens_ready_once_the_person_has_left_settings() {
            let _theme = ThemeGuard::new();
            let Opened {
                handle, mut visual, ..
            } = opened();
            set_preview(&mut visual, available(40_400_000, false));
            let before = route(handle, &visual);
            handle
                .update(&mut visual, |root, window, cx| {
                    root.enter_settings_route(Some("updates"), window, cx)
                })
                .unwrap();
            visual.run_until_parked();
            click(&mut visual, "UPDATE_HERO_Download");
            assert!(matches!(
                state(&mut visual),
                UpdateState::Downloading { .. }
            ));
            handle
                .update(&mut visual, |root, _, cx| root.set_route(before, cx))
                .unwrap();
            visual.run_until_parked();
            set_state(&mut visual, ready());
            assert!(is_open(handle, &visual));

            // Still on Settings, the hero shows Ready and nothing opens.
            let Opened {
                handle, mut visual, ..
            } = opened();
            set_preview(&mut visual, available(40_400_000, false));
            handle
                .update(&mut visual, |root, window, cx| {
                    root.enter_settings_route(Some("updates"), window, cx)
                })
                .unwrap();
            visual.run_until_parked();
            click(&mut visual, "UPDATE_HERO_Download");
            set_state(&mut visual, ready());
            assert!(!is_open(handle, &visual));
            assert!(visual.debug_bounds("UPDATE_HERO_Restart").is_some());
        }

        #[test]
        fn a_cancelled_or_failed_download_does_not_open_ready_later() {
            let _theme = ThemeGuard::new();
            let Opened {
                handle, mut visual, ..
            } = opened();
            for end in [available(1, false), failed()] {
                set_state(&mut visual, downloading(1, 2));
                handle
                    .update(&mut visual, |root, _, cx| {
                        root.mark_update_download_started(cx)
                    })
                    .unwrap();
                set_state(&mut visual, end);
                set_state(&mut visual, ready());
                assert!(!is_open(handle, &visual));
            }
        }

        #[test]
        fn the_hero_carries_every_action_that_left_the_popover() {
            let _theme = ThemeGuard::new();
            let Opened {
                handle,
                mut visual,
                store,
                ..
            } = opened();
            set_preview(&mut visual, available(40_400_000, false));
            handle
                .update(&mut visual, |root, window, cx| {
                    root.enter_settings_route(Some("updates"), window, cx)
                })
                .unwrap();
            visual.run_until_parked();

            click(&mut visual, "UPDATE_HERO_Download");
            assert!(matches!(
                state(&mut visual),
                UpdateState::Downloading { .. }
            ));
            click(&mut visual, "UPDATE_HERO_Cancel");
            assert!(matches!(state(&mut visual), UpdateState::Available { .. }));
            assert_eq!(
                visual.update(|_, cx| store.read(cx).settings.skipped_update_version.clone()),
                None
            );

            set_preview(&mut visual, failed());
            assert!(visual.debug_bounds("UPDATE_HERO_OpenRelease").is_some());
            click(&mut visual, "UPDATE_HERO_Retry");
            assert!(matches!(
                state(&mut visual),
                UpdateState::Downloading { .. }
            ));

            set_preview(&mut visual, available(40_400_000, true));
            click(&mut visual, "UPDATE_HERO_OpenRelease");
            assert_eq!(visual.cx.opened_url().as_deref(), Some(release_page_url()));
            assert!(visual.debug_bounds("UPDATE_HERO_Download").is_none());
        }

        #[test]
        fn one_whole_key_press_runs_each_popover_action_once() {
            let _theme = ThemeGuard::new();
            let Opened {
                handle, mut visual, ..
            } = opened();
            set_preview(&mut visual, ready());
            click_icon(&mut visual);

            // What's new opens the page and leaves the popover; Later closes it.
            visual.simulate_keystrokes("tab tab");
            visual.run_until_parked();
            press(&mut visual, "enter");
            assert_eq!(visual.cx.opened_url().as_deref(), Some(release_page_url()));
            assert!(is_open(handle, &visual));
            visual.simulate_keystrokes("tab");
            visual.run_until_parked();
            press(&mut visual, "enter");
            assert!(!is_open(handle, &visual));
            assert!(matches!(state(&mut visual), UpdateState::Ready { .. }));

            click_icon(&mut visual);
            visual.simulate_keystrokes("tab");
            visual.run_until_parked();
            press(&mut visual, "enter");
            assert!(!is_open(handle, &visual));
            assert!(matches!(state(&mut visual), UpdateState::Installing { .. }));
        }

        #[test]
        fn the_keyboard_opens_the_popover_from_the_palette_and_tabs_through_it() {
            let _theme = ThemeGuard::new();
            let Opened {
                handle, mut visual, ..
            } = opened();
            set_state(&mut visual, ready());
            let previous = focused(handle, &mut visual);
            handle
                .update(&mut visual, |root, window, cx| {
                    root.command_palette
                        .update(cx, |palette, cx| palette.open(window, cx))
                })
                .unwrap();
            visual.run_until_parked();
            visual.simulate_input("show runner");
            visual.run_until_parked();
            visual.simulate_keystrokes("enter");
            visual.run_until_parked();
            assert!(is_open(handle, &visual));
            assert!(visual.debug_bounds("UPDATE_POPOVER").is_some());
            assert!(!is_focused(handle, &mut visual, &previous));

            // Restart to update, What's new, then Later, which closes it and returns focus.
            visual.simulate_keystrokes("tab tab tab");
            visual.run_until_parked();
            press(&mut visual, "enter");
            assert!(!is_open(handle, &visual));
            assert!(visual.cx.opened_url().is_none());
            assert!(is_focused(handle, &mut visual, &previous));
        }

        #[test]
        fn the_palette_command_opens_settings_in_every_state_but_ready() {
            let _theme = ThemeGuard::new();
            for update in [
                available(1, false),
                available(1, true),
                downloading(1, 2),
                UpdateState::Verifying { info: info() },
                UpdateState::Installing { info: info() },
                failed(),
            ] {
                let Opened {
                    handle, mut visual, ..
                } = opened();
                set_state(&mut visual, update.clone());
                handle
                    .update(&mut visual, |root, window, cx| {
                        root.show_update_from_keyboard(window, cx);
                        assert_eq!(root.route, AppRoute::Settings, "{update:?}");
                        assert!(!root.update_popover.open, "{update:?}");
                    })
                    .unwrap();
                visual.run_until_parked();
                // It never starts the download, unlike the icon.
                assert_eq!(state(&mut visual), update);
                assert!(visual.debug_bounds("UPDATE_HERO_STATUS").is_some());
            }
        }

        #[test]
        fn tab_wraps_through_the_actions_in_both_directions() {
            let _theme = ThemeGuard::new();
            let Opened {
                handle, mut visual, ..
            } = opened();
            set_state(&mut visual, ready());
            click_icon(&mut visual);

            let on = |visual: &mut VisualTestContext, index: usize| {
                handle
                    .update(visual, |root, window, _| {
                        root.update_popover.action_focus[index].is_focused(window)
                    })
                    .unwrap()
            };
            visual.simulate_keystrokes("tab");
            visual.run_until_parked();
            assert!(on(&mut visual, 0));
            visual.simulate_keystrokes("shift-tab");
            visual.run_until_parked();
            assert!(on(&mut visual, 2));
            visual.simulate_keystrokes("tab");
            visual.run_until_parked();
            assert!(on(&mut visual, 0));
            visual.simulate_keystrokes("tab tab");
            visual.run_until_parked();
            assert!(on(&mut visual, 2));
            assert!(is_open(handle, &visual));
        }

        #[test]
        fn clicking_later_on_a_popover_that_opened_by_itself_keeps_terminal_focus() {
            let _theme = ThemeGuard::new();
            let Opened {
                handle, mut visual, ..
            } = opened();
            set_state(&mut visual, downloading(1, 2));
            handle
                .update(&mut visual, |root, _, cx| {
                    root.mark_update_download_started(cx)
                })
                .unwrap();
            let focus = focused(handle, &mut visual);
            set_state(&mut visual, ready());
            assert!(is_open(handle, &visual));
            click(&mut visual, "update-popover-later");
            assert!(!is_open(handle, &visual));
            assert!(is_focused(handle, &mut visual, &focus));
        }

        #[test]
        fn a_stale_palette_command_opens_settings_when_the_state_has_moved_on() {
            let _theme = ThemeGuard::new();
            let Opened {
                handle, mut visual, ..
            } = opened();
            set_state(&mut visual, ready());
            handle
                .update(&mut visual, |root, window, cx| {
                    root.command_palette
                        .update(cx, |palette, cx| palette.open(window, cx))
                })
                .unwrap();
            visual.run_until_parked();
            visual.simulate_input("show runner");
            visual.run_until_parked();
            set_state(&mut visual, UpdateState::UpToDate { checking: false });
            visual.simulate_keystrokes("enter");
            visual.run_until_parked();
            handle
                .read_with(&visual, |root, _| {
                    assert!(!root.update_popover.open);
                    assert_eq!(root.route, AppRoute::Settings);
                })
                .unwrap();
        }

        #[test]
        fn without_an_icon_the_palette_command_opens_settings_updates() {
            let _theme = ThemeGuard::new();
            let Opened {
                handle, mut visual, ..
            } = opened();
            set_state(&mut visual, ready());
            handle
                .update(&mut visual, |root, window, cx| {
                    root.enter_settings_route(None, window, cx);
                    root.show_update_from_keyboard(window, cx);
                    assert_eq!(root.route, AppRoute::Settings);
                    assert!(!root.update_popover.open);
                })
                .unwrap();
            visual.run_until_parked();
            assert!(visual.debug_bounds("UPDATE_HERO_STATUS").is_some());
        }

        #[test]
        fn escape_closes_a_popover_that_opened_by_itself_and_leaves_focus_alone() {
            let _theme = ThemeGuard::new();
            let Opened {
                handle, mut visual, ..
            } = opened();
            set_state(&mut visual, downloading(1, 2));
            // Whatever the window itself rendered and focused, as a terminal would be.
            let focus = handle
                .update(&mut visual, |root, window, cx| {
                    root.mark_update_download_started(cx);
                    window.focused(cx).expect("the window focuses something")
                })
                .unwrap();
            set_state(&mut visual, ready());
            assert!(is_open(handle, &visual));

            visual.simulate_keystrokes("escape");
            visual.run_until_parked();
            assert!(!is_open(handle, &visual));
            assert!(is_focused(handle, &mut visual, &focus));
        }

        struct EscapeProbe {
            root: Entity<NativeRoot>,
            focus: FocusHandle,
            keys_received: usize,
        }

        impl gpui::Render for EscapeProbe {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                let root = self.root.clone();
                div()
                    .size_full()
                    .capture_key_down(move |_, window, cx| {
                        root.update(cx, |root, cx| {
                            root.close_unfocused_update_popover(window, cx)
                        });
                    })
                    .child(div().size_full().track_focus(&self.focus).on_key_down(
                        cx.listener(|this, _: &KeyDownEvent, _, _| this.keys_received += 1),
                    ))
            }
        }

        #[test]
        fn that_escape_still_reaches_whatever_has_focus() {
            let _theme = ThemeGuard::new();
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            let host = cx.add_window(|window, cx| {
                let focus = cx.focus_handle();
                EscapeProbe {
                    root: cx.new(|cx| {
                        NativeRoot::new(
                            "update-ui-escape".into(),
                            temp.path().join("logs"),
                            None,
                            None,
                            store.clone(),
                            window,
                            cx,
                        )
                    }),
                    focus,
                    keys_received: 0,
                }
            });
            let mut visual = VisualTestContext::from_window(host.into(), &cx);
            visual.run_until_parked();
            host.update(&mut visual, |probe, window, cx| {
                probe.focus.focus(window, cx);
                probe.root.update(cx, |root, cx| {
                    root.open_ready_update_popover(window, cx);
                    assert!(root.update_popover.open);
                });
            })
            .unwrap();

            visual.simulate_keystrokes("escape");
            visual.run_until_parked();
            host.read_with(&visual, |probe, cx| {
                assert!(!probe.root.read(cx).update_popover.open);
                assert_eq!(probe.keys_received, 1);
            })
            .unwrap();
        }

        #[test]
        fn the_popover_fits_small_windows_and_wraps_a_long_version_at_each_zoom() {
            let _theme = ThemeGuard::new();
            for zoom in [1., 1.3, 1.5] {
                let temp = tempfile::tempdir().unwrap();
                let mut cx = TestAppContext::single();
                let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
                store.update(&mut cx, |store, _| store.settings.app_zoom = zoom);
                let handle = open_window(&mut cx, &store, temp.path().join("logs"));
                let mut visual = VisualTestContext::from_window(handle.into(), &cx);
                let viewport = size(px(700.), px(500.));
                visual.simulate_resize(viewport);
                visual.update(|_, cx| {
                    store.update(cx, |store, _| {
                        crate::app_store::seed_mixed_working_sessions(store)
                    })
                });
                let long = UpdateInfo::new("preview-nightly.abc1234abc1234abc1234.20261104.0100");
                set_state(
                    &mut visual,
                    UpdateState::Ready {
                        info: long,
                        size: 40_400_000,
                    },
                );
                click_icon(&mut visual);
                let popover = visual.debug_bounds("UPDATE_POPOVER").unwrap();
                let title = visual.debug_bounds("UPDATE_POPOVER_TITLE").unwrap();
                let restart = visual.debug_bounds("UPDATE_POPOVER_RESTART").unwrap();
                assert!((popover.size.width - px(POPOVER_WIDTH * zoom)).abs() < px(1.));
                assert!(popover.right() <= viewport.width);
                assert!(popover.bottom() <= viewport.height);
                assert!(title.left() >= popover.left() && title.right() <= popover.right());
                assert!(restart.bottom() <= popover.bottom());
            }
        }

        #[test]
        fn the_ready_popover_shows_working_agents_and_clears_when_they_finish() {
            let _theme = ThemeGuard::new();
            let Opened {
                mut visual, store, ..
            } = opened();
            visual.update(|_, cx| {
                store.update(cx, |store, _| {
                    crate::app_store::seed_mixed_working_sessions(store)
                })
            });
            set_state(&mut visual, ready());
            click_icon(&mut visual);
            assert!(visual.debug_bounds("UPDATE_WORKING_NOTE").is_some());
            assert!(visual.debug_bounds("UPDATE_WORKING_DOT").is_some());
            assert_eq!(
                visual.update(|_, cx| store.read(cx).working_agent_count()),
                2
            );
            visual.update(|_, cx| {
                store.update(cx, |store, cx| {
                    store.session_statuses.clear();
                    cx.notify();
                })
            });
            visual.run_until_parked();
            assert!(visual.debug_bounds("UPDATE_WORKING_NOTE").is_none());
        }

        #[test]
        fn skip_in_settings_hides_the_icon_persists_and_a_manual_check_brings_it_back() {
            let _theme = ThemeGuard::new();
            let Opened {
                handle,
                mut visual,
                store,
                ..
            } = opened();
            set_state(&mut visual, available(1, false));
            handle
                .update(&mut visual, |root, window, cx| {
                    root.enter_settings_route(Some("updates"), window, cx)
                })
                .unwrap();
            visual.run_until_parked();
            click(&mut visual, "UPDATE_HERO_Skip");
            assert_eq!(
                visual.update(|_, cx| store.read(cx).settings.skipped_update_version.clone()),
                Some("0.19.5".to_owned())
            );
            assert!(visual.update(|_, cx| global_updater(cx).read(cx).is_skipped()));
            assert_eq!(
                visual.update(|_, cx| visible_update_state(global_updater(cx).read(cx))),
                None
            );

            handle
                .update(&mut visual, |root, window, cx| {
                    check_for_updates_from_menu(root, window, cx);
                    assert_eq!(root.route, AppRoute::Settings);
                })
                .unwrap();
            visual.run_until_parked();
            assert!(visual.debug_bounds("UPDATE_HERO_STATUS").is_some());
            assert_eq!(
                visual.update(|_, cx| store.read(cx).settings.skipped_update_version.clone()),
                None
            );
            assert!(visual.update(|_, cx| !global_updater(cx).read(cx).is_skipped()));
        }
    }
}
