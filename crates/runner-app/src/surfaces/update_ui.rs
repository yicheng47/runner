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
    ) -> Option<impl IntoElement + use<>> {
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
mod tests;
