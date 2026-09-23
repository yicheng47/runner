use gpui::{canvas, deferred, svg, BoxShadow, FontWeight, WindowAppearance, WindowControlArea};
use runner_app::ui::menu::popup_layer;
use runner_backend::model::Runtime;
use runner_backend::usage::{
    AgentUsage, RefreshReason, UnavailableReason, UsageSnapshot, UsageWindow,
};

use crate::app_settings::{clamp_sidebar_width, nudge_zoom};
use crate::toast::ToastTone;
use crate::*;
use runner_app::ui::resize::{resize_strip, resize_strip_inset, ResizeAxis};

pub(crate) const TITLEBAR_DRAG_HEIGHT: f32 = 28.;
#[cfg(target_os = "macos")]
pub(crate) const SIDEBAR_TOGGLE_GLYPH_X: f32 = 94.3;
#[cfg(target_os = "macos")]
pub(crate) const SIDEBAR_TOGGLE_GLYPH_INSET: f32 = 6.3;
const SIDEBAR_TRANSITION_MS: u64 = 200;
// A pass-through of the left edge on the way to another screen is far shorter
// than this; a deliberate rest on it is longer.
const SIDEBAR_PREVIEW_DWELL_MS: u64 = 200;
// Deliberately differs from main's inherited 19.5px line box to align both footer dividers.
const SETTINGS_FOOTER_LINE_HEIGHT: f32 = 18.;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum AppRoute {
    #[default]
    Chat,
    Roles,
    RoleDetail(String),
    Crews,
    CrewEditor(String),
    Mission(String),
    ArchivedChat,
    Settings,
}

impl AppRoute {
    pub(crate) fn terminal_visible(&self) -> bool {
        matches!(self, Self::Chat)
    }
}

fn route_after_mission_archived(
    route: &AppRoute,
    mission_id: &str,
    was_archived: Option<bool>,
    is_archived: bool,
) -> Option<AppRoute> {
    (was_archived == Some(false)
        && is_archived
        && matches!(route, AppRoute::Mission(active) if active == mission_id))
    .then_some(AppRoute::Chat)
}

#[derive(Clone)]
struct SidebarResizeDrag;

impl Render for SidebarResizeDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().w(px(1.)).h(px(1.))
    }
}

fn settings_update_hint_version(
    available: Option<&runner_app::updater::UpdateInfo>,
) -> Option<&str> {
    available.map(|update| update.version())
}

pub(crate) fn usage_installed(core: &AppCore) -> Vec<Runtime> {
    runner_backend::runtime_status::status_list(
        &core.db,
        &core.runtime_shell_env,
        &core.runtime_discovery,
    )
    .map(|status| {
        status
            .runtimes
            .into_iter()
            .filter(|runtime| {
                matches!(runtime.name, Runtime::ClaudeCode | Runtime::Codex)
                    && matches!(
                        runtime.effective_source,
                        Some(
                            runner_backend::runtime_status::RuntimeCommandSource::Detected
                                | runner_backend::runtime_status::RuntimeCommandSource::Override
                        )
                    )
            })
            .map(|runtime| runtime.name)
            .collect()
    })
    .unwrap_or_default()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UsageTone {
    Normal,
    Warning,
    Danger,
}

fn usage_tone(windows: impl IntoIterator<Item = f64>) -> UsageTone {
    let max = windows.into_iter().fold(0.0_f64, f64::max);
    if max >= 100. {
        UsageTone::Danger
    } else if max >= 80. {
        UsageTone::Warning
    } else {
        UsageTone::Normal
    }
}

fn usage_color(tone: UsageTone) -> gpui::Hsla {
    match tone {
        UsageTone::Normal => theme::muted(),
        UsageTone::Warning => theme::warning(),
        UsageTone::Danger => theme::danger(),
    }
}

fn reset_label(
    resets_at: Option<chrono::DateTime<chrono::Utc>>,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let Some(reset) = resets_at else {
        return "reset time unknown".into();
    };
    let seconds = reset.signed_duration_since(now).num_seconds().max(0);
    if seconds < 60 {
        return "resets <1m".into();
    }
    let minutes = (seconds + 59) / 60;
    let days = minutes / 1440;
    let hours = minutes % 1440 / 60;
    let mins = minutes % 60;
    if days > 0 {
        format!("resets {days}d {hours}h")
    } else if hours > 0 {
        format!("resets {hours}h {mins}m")
    } else {
        format!("resets {mins}m")
    }
}

fn unavailable_line(reason: Option<UnavailableReason>, runtime: Runtime) -> &'static str {
    match reason {
        Some(UnavailableReason::SignIn) => "Sign in to Claude Code to see usage.",
        Some(UnavailableReason::KeychainDenied | UnavailableReason::KeychainUnavailable) => {
            "Runner was not allowed to read Claude Code's sign-in from the Keychain."
        }
        Some(UnavailableReason::ClaudeUnreachable) => "Couldn't reach Anthropic.",
        Some(UnavailableReason::CodexNoAnswer) => "Codex didn't answer.",
        Some(UnavailableReason::InvalidResponse) if runtime == Runtime::Codex => {
            "Codex didn't answer."
        }
        None => "Checking…",
        _ => "Couldn't reach Anthropic.",
    }
}

fn usage_tooltip(snapshot: &UsageSnapshot, enabled: &[Runtime]) -> String {
    let mut parts = Vec::new();
    for (runtime, name, usage) in [
        (Runtime::ClaudeCode, "Claude Code", snapshot.claude.as_ref()),
        (Runtime::Codex, "Codex", snapshot.codex.as_ref()),
    ] {
        if !enabled.contains(&runtime) {
            continue;
        }
        let value = usage.and_then(|usage| {
            usage
                .windows
                .iter()
                .max_by(|a, b| a.used_percent.total_cmp(&b.used_percent))
        });
        parts.push(match value {
            Some(window) => format!("{name} {:.0}%", window.used_percent),
            None => format!("{name} unavailable"),
        });
    }
    parts.join(" · ")
}

fn usage_window_row(window: &UsageWindow, now: chrono::DateTime<chrono::Utc>) -> AnyElement {
    let tone = usage_tone([window.used_percent]);
    let fill = usage_color(tone);
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(theme::text_body())
                        .text_color(theme::text())
                        .child(window.name.clone()),
                )
                .child(
                    div()
                        .text_size(theme::text_body())
                        .text_color(fill)
                        .child(format!("{:.0}%", window.used_percent)),
                ),
        )
        .child(
            div()
                .w_full()
                .h(px(5.))
                .rounded_full()
                .bg(theme::border())
                .child(
                    div()
                        .w(relative((window.used_percent / 100.) as f32))
                        .h_full()
                        .rounded_full()
                        .bg(fill),
                ),
        )
        .child(
            div()
                .text_size(theme::text_meta())
                .text_color(theme::muted())
                .child(reset_label(window.resets_at, now)),
        )
        .into_any_element()
}

fn usage_section(
    runtime: Runtime,
    usage: Option<&AgentUsage>,
    reason: Option<UnavailableReason>,
    now: chrono::DateTime<chrono::Utc>,
) -> AnyElement {
    let (name, mark) = match runtime {
        Runtime::ClaudeCode => ("Claude Code", "claude.svg"),
        _ => ("Codex", "openai.svg"),
    };
    let rows: Vec<AnyElement> = usage
        .map(|usage| {
            usage
                .windows
                .iter()
                .map(|window| usage_window_row(window, now))
                .collect()
        })
        .unwrap_or_default();
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_3()
        .py_3()
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(svg().path(mark).size(px(16.)).text_color(theme::text()))
                .child(
                    div()
                        .text_size(theme::text_title())
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(name),
                ),
        )
        .children(rows)
        .children(usage.is_none().then(|| {
            div()
                .text_size(theme::text_body())
                .text_color(theme::muted())
                .child(unavailable_line(reason, runtime))
        }))
        .into_any_element()
}

pub(crate) fn alpha(mut color: gpui::Hsla, value: f32) -> gpui::Hsla {
    color.a = value;
    color
}

impl NativeRoot {
    fn render_usage_popover(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let snapshot = self.core(cx).usage.snapshot();
        let now = chrono::Utc::now();
        let updated = snapshot
            .claude
            .iter()
            .chain(snapshot.codex.iter())
            .map(|usage| usage.updated_at)
            .max();
        let age = if snapshot.refreshing && updated.is_none() {
            "Refreshing…".to_owned()
        } else if let Some(updated) = updated {
            let minutes = now.signed_duration_since(updated).num_minutes().max(0);
            if minutes == 0 {
                "updated just now".into()
            } else {
                format!("updated {minutes}m ago")
            }
        } else {
            "Not updated yet".into()
        };
        let refresh = div()
            .id("refresh-usage")
            .size(px(24.))
            .flex()
            .items_center()
            .justify_center()
            .rounded_sm()
            .cursor_pointer()
            .hover(|button| button.bg(theme::sidebar_selected()))
            .on_click(cx.listener(|this, _, _, cx| {
                let core = this.core(cx).clone();
                core.usage
                    .request_refresh(core.clone(), RefreshReason::Button);
                cx.notify();
            }))
            .child(
                svg()
                    .path("refresh-cw.svg")
                    .size(px(14.))
                    .text_color(theme::muted()),
            );
        let sections: Vec<AnyElement> = [Runtime::ClaudeCode, Runtime::Codex]
            .into_iter()
            .filter(|runtime| {
                self.usage_installed.contains(runtime)
                    && self.settings(cx).model_runtimes().contains(runtime)
            })
            .map(|runtime| match runtime {
                Runtime::ClaudeCode => usage_section(
                    runtime,
                    snapshot.claude.as_ref(),
                    snapshot.claude_error,
                    now,
                ),
                _ => usage_section(runtime, snapshot.codex.as_ref(), snapshot.codex_error, now),
            })
            .collect();
        div()
            .w_full()
            .px_4()
            .pt_3()
            .pb_2()
            .flex()
            .flex_col()
            .rounded(px(8.))
            .border_1()
            .border_color(theme::border_strong())
            .bg(theme::panel())
            .shadow(vec![BoxShadow {
                color: gpui::black().opacity(0.25),
                blur_radius: px(16.),
                spread_radius: px(0.),
                offset: point(px(0.), px(4.)),
            }])
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_size(theme::text_lead())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Usage"),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(theme::text_meta())
                            .text_color(theme::muted())
                            .child(age),
                    )
                    .child(refresh),
            )
            .children(sections)
            .child(
                div()
                    .w_full()
                    .border_t_1()
                    .border_color(theme::border())
                    .pt_2()
                    .child(
                        div()
                            .id("usage-agent-settings")
                            .w_full()
                            .py_2()
                            .rounded_sm()
                            .cursor_pointer()
                            .text_size(theme::text_body())
                            .text_color(theme::muted())
                            .hover(|row| {
                                row.bg(theme::sidebar_selected()).text_color(theme::text())
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.usage_open = false;
                                this.enter_settings_route(Some("agents"), window, cx);
                            }))
                            .child("Agent settings…"),
                    ),
            )
            .into_any_element()
    }

    pub(crate) fn render_app_shell(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.sync_theme(window, cx);
        self.sync_sidebar_shortcut_rows(cx);
        window.set_rem_size(px(16. * self.settings(cx).app_zoom));
        if let Some(error) = self.error.take() {
            self.show_toast(error, ToastTone::Error, cx);
        }

        let workspace = self.render_entity_surface(window, cx);
        let (sidebar, sidebar_divider) = self.render_app_sidebar(window, cx);
        let sidebar_resize =
            sidebar_divider.map(|divider| self.render_sidebar_resize_handle(divider, cx));
        let preview_trigger = self.render_sidebar_preview_trigger(cx);
        let modal = self
            .start_chat_modal
            .is_some()
            .then(|| self.render_start_chat_modal(cx));
        let chat_rename_modal = (self.route == AppRoute::Chat)
            .then_some(self.chat_rename_modal.as_ref())
            .flatten()
            .map(|_| self.render_chat_rename_modal(cx));
        let terminal_close_confirm = matches!(self.route, AppRoute::Chat | AppRoute::Mission(_))
            .then_some(self.terminal_close_confirm.as_ref())
            .flatten()
            .map(|_| self.render_terminal_close_confirm(cx));
        let fork_confirm = (self.route == AppRoute::Chat)
            .then_some(self.fork_confirm.as_ref())
            .flatten()
            .map(|_| self.render_fork_confirm(cx));
        let sidebar_overlays = if self.route != AppRoute::Settings {
            self.render_sidebar_overlays(cx)
        } else {
            Vec::new()
        };
        let entity_overlays = if self.route != AppRoute::Settings {
            self.render_entity_overlays(cx)
        } else {
            Vec::new()
        };
        let chrome = div()
            .relative()
            .size_full()
            .flex()
            .children(sidebar)
            .child(
                div()
                    .when(cfg!(test), |column| {
                        column.debug_selector(|| "APP_CONTENT_COLUMN".into())
                    })
                    .relative()
                    .flex_1()
                    .min_w(px(0.))
                    .h_full()
                    .flex()
                    .flex_col()
                    .children(self.render_main_titlebar_drag_area(cx))
                    .child(workspace)
                    .children(self.render_entity_sidebar_toggle(window, cx)),
            )
            .children(sidebar_resize)
            .children(preview_trigger)
            .children(chat_rename_modal)
            .children(terminal_close_confirm)
            .children(fork_confirm)
            .children(sidebar_overlays)
            .children(entity_overlays);
        let settings =
            (self.route == AppRoute::Settings).then(|| self.render_settings_takeover(window, cx));
        let settings_confirm = self
            .settings_confirm_overlay(cx)
            .map(|overlay| deferred(overlay).with_priority(100));
        let command_palette = deferred(self.command_palette.clone()).with_priority(50);
        let toast = self
            .render_toast(cx)
            .map(|toast| deferred(toast).with_priority(3));
        let modifier_sidebar = self.sidebar.clone();
        let key_sidebar = self.sidebar.clone();
        let usage_root = cx.entity();

        div()
            .relative()
            .key_context("Root")
            .size_full()
            .overflow_hidden()
            .track_focus(&self.root_focus)
            .font(crate::app_settings::app_font())
            .bg(theme::bg())
            .text_color(theme::text())
            .child(chrome)
            .children(settings)
            .children(modal)
            .children(settings_confirm)
            .child(command_palette)
            .children(toast)
            .map(|root| {
                // Stays inside the content root so the scrim ends at the Windows title bar.
                #[cfg(windows)]
                let root = root
                    .children(self.update_dialog.clone().map(deferred))
                    .on_action(cx.listener(Self::open_update_dialog));
                root
            })
            .map(|root| self.decorate_window(root, window, cx))
            .on_modifiers_changed(move |event, window, cx| {
                modifier_sidebar.update(cx, |sidebar, sidebar_cx| {
                    sidebar.handle_shortcut_modifiers_changed(event.modifiers, window, sidebar_cx);
                });
            })
            .capture_key_down(move |event, _, cx| {
                if event.keystroke.key == "escape" {
                    usage_root.update(cx, |this, cx| {
                        if this.usage_open {
                            this.usage_open = false;
                            cx.stop_propagation();
                            cx.notify();
                        }
                    });
                }
                key_sidebar.update(cx, |sidebar, sidebar_cx| {
                    sidebar.handle_shortcut_key_pressed(sidebar_cx);
                });
            })
            .on_drag_move::<SidebarResizeDrag>(cx.listener(
                |this, event: &DragMoveEvent<SidebarResizeDrag>, _, cx| {
                    let width = f32::from(event.event.position.x - event.bounds.left())
                        / this.settings(cx).app_zoom;
                    let width = clamp_sidebar_width(width);
                    // A drag suppresses hover, so the bar needs the drag's own state to stay lit.
                    if !this.sidebar_resizing {
                        this.sidebar_resizing = true;
                        cx.notify();
                    }
                    this.update_app_settings(cx, false, |settings| {
                        if settings.sidebar_width == width {
                            return false;
                        }
                        settings.sidebar_width = width;
                        true
                    });
                },
            ))
            .on_drop(cx.listener(|this, _: &SidebarResizeDrag, _, cx| {
                this.finish_sidebar_resize(cx);
                this.save_settings(cx);
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.finish_sidebar_resize(cx);
                    this.clear_sidebar_drag("root-up", cx);
                    this.clear_crew_slot_drag(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.finish_sidebar_resize(cx);
                    this.clear_sidebar_drag("root-up-out", cx);
                    this.clear_crew_slot_drag(cx);
                }),
            )
            .on_action(cx.listener(Self::open_new_tab_modal))
            .on_action(cx.listener(Self::split_pane_right))
            .on_action(cx.listener(Self::split_pane_down))
            .on_action(cx.listener(Self::focus_previous_chat_pane))
            .on_action(cx.listener(Self::focus_next_chat_pane))
            .on_action(cx.listener(Self::stop_focused_session))
            .on_action(cx.listener(Self::resume_focused_session))
            .on_action(cx.listener(Self::toggle_terminal_drawer_action))
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::open_command_palette))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::navigate_previous_page))
            .on_action(cx.listener(Self::navigate_next_page))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::zoom_reset))
            .on_action(cx.listener(Self::toggle_fullscreen))
            .on_action(cx.listener(Self::select_tab_1))
            .on_action(cx.listener(Self::select_tab_2))
            .on_action(cx.listener(Self::select_tab_3))
            .on_action(cx.listener(Self::select_tab_4))
            .on_action(cx.listener(Self::select_tab_5))
            .on_action(cx.listener(Self::select_tab_6))
            .on_action(cx.listener(Self::select_tab_7))
            .on_action(cx.listener(Self::select_tab_8))
            .on_action(cx.listener(Self::select_tab_9))
            .into_any_element()
    }

    fn render_app_sidebar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Option<AnyElement>, Option<f32>) {
        let visible = !self.sidebar_collapsed || self.sidebar_preview_open;
        let visibility_target = if visible { 1. } else { 0. };
        let (visibility, animating) = self.sidebar_visibility.animate_to(
            visibility_target,
            Instant::now(),
            Duration::from_millis(SIDEBAR_TRANSITION_MS),
        );
        if animating {
            window.request_animation_frame();
        }
        let show_panel = visible || animating;
        let full_width = self.settings(cx).sidebar_width * self.settings(cx).app_zoom;
        let width = full_width * visibility;
        if !show_panel {
            return (
                Some(
                    div()
                        .id("app-sidebar")
                        .relative()
                        .w(px(width))
                        .h_full()
                        .flex_none()
                        .overflow_hidden()
                        .into_any_element(),
                ),
                None,
            );
        }
        let preview =
            self.sidebar_collapsed && (self.sidebar_preview_open || self.sidebar_preview_peeking);
        let titlebar = self.render_sidebar_titlebar(window, cx);
        let search_button = div()
            .id("sidebar-search")
            .group("sidebar-search")
            .tab_index(0)
            .flex_none()
            .size(px(24. * self.settings(cx).app_zoom))
            .flex()
            .items_center()
            .justify_center()
            .rounded_sm()
            .border_1()
            .border_color(gpui::transparent_black())
            .cursor_pointer()
            .text_color(theme::muted())
            .hover(|button| {
                button
                    .border_color(theme::sidebar_selected_border())
                    .bg(alpha(theme::sidebar_selected(), 0.4))
                    .text_color(theme::text())
            })
            .focus_visible(|button| {
                button
                    .border_color(theme::sidebar_selected_border())
                    .bg(alpha(theme::sidebar_selected(), 0.4))
                    .text_color(theme::text())
            })
            .on_click(cx.listener(|this, _, window, cx| {
                this.show_command_palette(window, cx);
            }))
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    this.show_command_palette(window, cx);
                }
            }))
            .child(
                svg()
                    .path("search.svg")
                    .size(px(14. * self.settings(cx).app_zoom))
                    .text_color(theme::muted())
                    .group_hover("sidebar-search", |icon| icon.text_color(theme::text())),
            );
        let brand = div()
            .flex_none()
            .px_5()
            .pb_5()
            .pt_1()
            .flex()
            .items_center()
            .gap_2()
            .child(
                svg()
                    .flex_none()
                    .path("brand-mark.svg")
                    .w(px(32. * self.settings(cx).app_zoom))
                    .h(px(32. * self.settings(cx).app_zoom))
                    .text_color(theme::accent()),
            )
            .child(
                div()
                    .flex_1()
                    .text_size(theme::text_heading())
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .child("Runner"),
            )
            .child(Tooltip::new(
                "sidebar-search-tooltip",
                keymap::effective_binding("command-palette", &self.settings(cx).keymap_overrides)
                    .map_or_else(
                        || "Search".to_owned(),
                        |combo| format!("Search ({})", keymap::format_combo(&combo)),
                    ),
                search_button,
            ));
        let updater = global_updater(cx);
        let zoom = self.settings(cx).app_zoom;
        let update_version =
            settings_update_hint_version(updater.read(cx).available()).map(str::to_owned);
        let update_hint = update_version.map(|version| {
            let click_updater = updater.clone();
            #[cfg(not(windows))]
            let tooltip = crate::platform_ui::update_hint_tooltip(&version);
            #[cfg(windows)]
            let tooltip =
                crate::platform_ui::update_hint_tooltip(&version, updater.read(cx).state());
            Tooltip::new(
                "sidebar-update-tooltip",
                tooltip,
                div()
                    .id("sidebar-update")
                    .flex_none()
                    .size(rems(32. / 16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .cursor_pointer()
                    .hover(|button| {
                        button
                            .border_color(alpha(theme::accent(), 0.4))
                            .bg(alpha(theme::accent(), 0.1))
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(move |_, _window, cx| {
                        cx.stop_propagation();
                        #[cfg(not(windows))]
                        crate::platform_ui::activate_update_hint(&click_updater, cx);
                        #[cfg(windows)]
                        crate::platform_ui::activate_update_hint(&click_updater, _window, cx);
                    })
                    .child(
                        svg()
                            .path("circle-arrow-down.svg")
                            .flex_none()
                            .size(px(14. * zoom))
                            .text_color(theme::accent()),
                    ),
            )
        });
        let enabled = self.settings(cx).model_runtimes();
        let show_usage = self
            .usage_installed
            .iter()
            .any(|runtime| enabled.contains(runtime))
            && !self.sidebar_collapsed;
        let usage_hint = show_usage.then(|| {
            let snapshot = self.core(cx).usage.snapshot();
            let visible: Vec<_> = self
                .usage_installed
                .iter()
                .copied()
                .filter(|runtime| enabled.contains(runtime))
                .collect();
            let tone = usage_tone(
                visible
                    .iter()
                    .filter_map(|runtime| match runtime {
                        Runtime::ClaudeCode => snapshot.claude.as_ref(),
                        Runtime::Codex => snapshot.codex.as_ref(),
                        _ => None,
                    })
                    .flat_map(|agent| agent.windows.iter().map(|window| window.used_percent)),
            );
            let color = usage_color(tone);
            let trigger = Tooltip::new(
                "sidebar-usage-tooltip",
                usage_tooltip(&snapshot, &visible),
                div()
                    .id("sidebar-usage")
                    .flex_none()
                    .size(rems(32. / 16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_sm()
                    .border_1()
                    .border_color(gpui::transparent_black())
                    .cursor_pointer()
                    .when(self.usage_open, |button| {
                        button.bg(theme::sidebar_selected())
                    })
                    .hover(|button| button.bg(alpha(theme::sidebar_selected(), 0.5)))
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.usage_open = !this.usage_open;
                        if this.usage_open {
                            let core = this.core(cx).clone();
                            core.usage
                                .request_refresh(core.clone(), RefreshReason::Open);
                        }
                        cx.notify();
                    }))
                    .child(
                        svg()
                            .path("gauge.svg")
                            .size(px(14. * zoom))
                            .text_color(color),
                    ),
            );
            div().flex_none().size(rems(32. / 16.)).child(trigger)
        });
        let anchor_owner = cx.entity();
        let settings_button = crate::platform_ui::sidebar_section()
            .relative()
            .px_3()
            .pt_2()
            .border_t_1()
            .border_color(theme::sidebar_selected_border())
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .id("open-settings")
                            .group("sidebar-settings")
                            .min_w(px(0.))
                            .flex_1()
                            .px(rems(10. / 16.))
                            .py_2()
                            .flex()
                            .items_center()
                            .gap(rems(10. / 16.))
                            .rounded_sm()
                            .border_1()
                            .border_color(gpui::transparent_black())
                            .cursor_pointer()
                            .text_color(theme::muted())
                            .line_height(px(SETTINGS_FOOTER_LINE_HEIGHT * zoom))
                            .hover(|button| {
                                button
                                    .border_color(theme::sidebar_selected_border())
                                    .bg(alpha(theme::sidebar_selected(), 0.4))
                                    .text_color(theme::text())
                            })
                            .child(
                                svg()
                                    .path("settings.svg")
                                    .w(px(14. * zoom))
                                    .h(px(14. * zoom))
                                    .flex_none()
                                    .text_color(theme::muted())
                                    .group_hover("sidebar-settings", |icon| {
                                        icon.text_color(theme::text())
                                    }),
                            )
                            .child(div().text_size(px(13. * zoom)).child("Settings"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.enter_settings_route(None, window, cx);
                            })),
                    )
                    .children(update_hint)
                    .children(usage_hint),
            )
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, _, cx| {
                        anchor_owner.update(cx, |this, _| this.usage_anchor = Some(bounds));
                    },
                )
                .absolute()
                .inset_0(),
            );
        // The content keeps its full width while the wrapper animates, so the
        // transition clips instead of squashing every row; a squashed row
        // shrinks its icons to zero and gpui refuses to paint them (#512).
        let content = div()
            .w(px(full_width))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .pb_3()
            .children(titlebar)
            .child(brand)
            .child(self.sidebar.clone())
            .child(settings_button)
            .children(
                (self.usage_open && show_usage)
                    .then(|| {
                        self.usage_anchor.map(|anchor| {
                            let owner = cx.entity();
                            popup_layer(
                                anchor,
                                window,
                                px(340. * zoom),
                                self.render_usage_popover(cx),
                                Rc::new(move |_, cx| {
                                    owner.update(cx, |this, cx| {
                                        this.usage_open = false;
                                        cx.notify();
                                    });
                                }),
                            )
                        })
                    })
                    .flatten(),
            );
        let mut sidebar = div()
            .id("app-sidebar")
            .relative()
            .w(px(width))
            .h_full()
            .flex_none()
            .overflow_hidden()
            .opacity(visibility)
            .bg(theme::sidebar())
            .map(|element| {
                #[cfg(test)]
                let element = crate::theme_snapshot::record_fill("APP_SIDEBAR", element);
                element
            })
            .border_r_1()
            .border_color(theme::border())
            .child(content);
        // The 1px right border is the divider; the handle centres on it.
        let divider = visible.then_some(width - 0.5);
        if preview {
            sidebar = sidebar
                .absolute()
                .left_0()
                .top_0()
                .rounded_tr(px(12. * self.settings(cx).app_zoom))
                .rounded_br(px(12. * self.settings(cx).app_zoom))
                .shadow_2xl()
                .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    if !*hovered && this.sidebar_collapsed {
                        this.sidebar_preview_open = false;
                        cx.notify();
                    }
                }));
            return (
                Some(deferred(sidebar).with_priority(1).into_any_element()),
                divider,
            );
        }
        (Some(sidebar.into_any_element()), divider)
    }

    fn finish_sidebar_resize(&mut self, cx: &mut Context<Self>) {
        if !self.sidebar_resizing {
            return;
        }
        self.sidebar_resizing = false;
        cx.notify();
    }

    pub(crate) fn render_sidebar_resize_handle(&self, divider: f32, cx: &App) -> AnyElement {
        let zoom = self.settings(cx).app_zoom;
        resize_strip(
            "sidebar-resize",
            ResizeAxis::Columns,
            self.sidebar_resizing,
            zoom,
            None,
        )
        .id("sidebar-resize")
        .map(|handle| {
            #[cfg(windows)]
            let handle = handle.occlude();
            handle
        })
        .absolute()
        .top_0()
        .left(px(divider - resize_strip_inset(zoom)))
        .on_drag(
            SidebarResizeDrag,
            |drag: &SidebarResizeDrag, _, _, cx: &mut App| cx.new(|_| drag.clone()),
        )
        .into_any_element()
    }

    fn render_sidebar_preview_trigger(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        (self.sidebar_collapsed && self.route != AppRoute::Settings).then(|| {
            div()
                .id("sidebar-preview-trigger")
                .absolute()
                .left_0()
                .top_0()
                .w(px(16. * self.settings(cx).app_zoom))
                .h_full()
                .on_hover(cx.listener(|this, hovered: &bool, _, _| {
                    this.sidebar_preview_trigger_hovered = *hovered;
                    if !*hovered {
                        this.cancel_sidebar_preview_dwell();
                    }
                }))
                .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                    // A drag that reaches the edge, such as a text selection,
                    // must not open the preview.
                    if event.pressed_button.is_some() {
                        this.cancel_sidebar_preview_dwell();
                    } else {
                        this.arm_sidebar_preview_dwell(cx);
                    }
                }))
                .into_any_element()
        })
    }

    fn cancel_sidebar_preview_dwell(&mut self) {
        self.sidebar_preview_dwell = 0;
    }

    fn arm_sidebar_preview_dwell(&mut self, cx: &mut Context<Self>) {
        if self.sidebar_preview_dwell != 0 || self.sidebar_preview_open {
            return;
        }
        let generation = self.sidebar_preview_dwell_generation.wrapping_add(1).max(1);
        self.sidebar_preview_dwell_generation = generation;
        self.sidebar_preview_dwell = generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(SIDEBAR_PREVIEW_DWELL_MS))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.sidebar_preview_dwell != generation {
                    return;
                }
                this.sidebar_preview_dwell = 0;
                if this.sidebar_preview_trigger_hovered && this.sidebar_collapsed {
                    this.sidebar_preview_open = true;
                    this.sidebar_preview_peeking = true;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn render_titlebar_drag_area(
        &self,
        id: &'static str,
        area: gpui::Div,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let area = area
            .id(id)
            .window_control_area(WindowControlArea::Drag)
            .on_mouse_down_out(cx.listener(|this, _, _, _| {
                this.titlebar_drag_armed = false;
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.titlebar_drag_armed = false;
                }),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.titlebar_drag_armed = true;
                }),
            );
        #[cfg(target_os = "macos")]
        let area = area
            .on_mouse_move(cx.listener(|this, _, window, _| {
                if this.titlebar_drag_armed {
                    this.titlebar_drag_armed = false;
                    window.start_window_move();
                }
            }))
            .on_click(|event, window, cx| {
                if event.click_count() == 2 {
                    cx.stop_propagation();
                    window.titlebar_double_click();
                }
            });
        area
    }

    fn render_toast(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.toasts.active().map(|toast| {
            let zoom = self.settings(cx).app_zoom;
            let (icon, icon_color) = match toast.tone {
                ToastTone::Info => ("info.svg", theme::muted()),
                ToastTone::Success => ("circle-check.svg", theme::accent()),
                ToastTone::Error => ("circle-x.svg", theme::danger()),
            };
            div()
                .absolute()
                .top(px((TITLEBAR_DRAG_HEIGHT + 12.) * zoom))
                .left(px(16. * zoom))
                .right(px(16. * zoom))
                .flex()
                .justify_center()
                .child(
                    div()
                        .id("global-toast")
                        .max_w(px(420. * zoom))
                        .pl(px(14. * zoom))
                        .pr(px(12. * zoom))
                        .py(px(10. * zoom))
                        .flex()
                        .items_center()
                        .gap(px(10. * zoom))
                        .rounded(px(10. * zoom))
                        .border_1()
                        .border_color(theme::border())
                        .bg(theme::panel())
                        .shadow(vec![BoxShadow {
                            color: theme::scrim(),
                            offset: point(px(0.), px(8. * zoom)),
                            blur_radius: px(24. * zoom),
                            spread_radius: px(0.),
                        }])
                        .cursor_pointer()
                        .occlude()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(|this, _, _, cx| {
                            cx.stop_propagation();
                            this.toasts.dismiss();
                            cx.notify();
                        }))
                        .child(
                            svg()
                                .flex_none()
                                .path(icon)
                                .w(px(16. * zoom))
                                .h(px(16. * zoom))
                                .text_color(icon_color),
                        )
                        .child(
                            div()
                                .min_w(px(0.))
                                .whitespace_normal()
                                .text_size(theme::text_body())
                                .text_color(theme::text())
                                .line_height(px(18. * zoom))
                                .child(SharedString::from(toast.message.clone())),
                        )
                        .child(
                            svg()
                                .flex_none()
                                .ml(px(6. * zoom))
                                .path("close.svg")
                                .w(px(14. * zoom))
                                .h(px(14. * zoom))
                                .text_color(theme::faint())
                                .hover(|close| close.text_color(theme::text())),
                        ),
                )
                .into_any_element()
        })
    }

    pub(crate) fn show_toast(
        &mut self,
        message: impl Into<String>,
        tone: ToastTone,
        cx: &mut Context<Self>,
    ) {
        let id = self.toasts.show(message, tone);
        let duration_ms = self.toasts.active().and_then(|toast| toast.duration_ms);
        cx.notify();
        let Some(duration_ms) = duration_ms else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(duration_ms))
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.toasts.expire(id) {
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(crate) fn terminal_style(&self, cx: &App) -> crate::terminal::element::TerminalStyle {
        crate::terminal::element::TerminalStyle {
            palette: app_settings::terminal_palette(self.settings(cx), theme::active_variant()),
            font: self.settings(cx).terminal_font_family.font(),
            font_size: self.settings(cx).terminal_font_size as f32 * self.settings(cx).app_zoom,
            app_zoom: self.settings(cx).app_zoom,
        }
    }

    /// Chat panes and the mission workspace carry the open-sidebar cluster in
    /// their own 44 px header rows; the entity pages have no header row, so the
    /// shell pins the same cluster into a row of that height for them, level
    /// with the traffic lights, painted after the page so nothing covers it.
    fn render_entity_sidebar_toggle(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !matches!(
            self.route,
            AppRoute::Roles | AppRoute::RoleDetail(_) | AppRoute::Crews | AppRoute::CrewEditor(_)
        ) {
            return None;
        }
        let button = self.render_open_sidebar_button(cx)?;
        Some(
            div()
                .when(cfg!(test), |toggle| {
                    toggle.debug_selector(|| "ENTITY_SIDEBAR_TOGGLE".into())
                })
                .absolute()
                .top_0()
                .left(px(self.workspace_titlebar_padding(window, cx)))
                .h(gpui::rems(runner_app::ui::WORKSPACE_HEADER_HEIGHT / 16.))
                .flex()
                .items_center()
                .child(button)
                .into_any_element(),
        )
    }

    /// Whether the window's previous / next page arrows are enabled. The
    /// arrows live in the macOS title-bar clusters; Windows chrome computes
    /// its own.
    #[cfg(target_os = "macos")]
    pub(crate) fn page_navigation_state(&self) -> (bool, bool) {
        let in_settings = self.route == AppRoute::Settings;
        let can_go_back =
            !in_settings && self.runtime_navigation_index.is_some_and(|index| index > 0);
        let can_go_forward = !in_settings
            && self
                .runtime_navigation_index
                .is_some_and(|index| index + 1 < self.runtime_navigation_history.len());
        (can_go_back, can_go_forward)
    }

    pub(crate) fn workspace_titlebar_padding(&self, window: &Window, cx: &App) -> f32 {
        #[cfg(target_os = "macos")]
        {
            if self.sidebar_collapsed && !window.is_fullscreen() {
                SIDEBAR_TOGGLE_GLYPH_X - SIDEBAR_TOGGLE_GLYPH_INSET * self.settings(cx).app_zoom
            } else {
                16. * self.settings(cx).app_zoom
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = window;
            16. * self.settings(cx).app_zoom
        }
    }

    pub(crate) fn sync_theme(&self, window: &Window, cx: &mut Context<Self>) {
        let system_is_light = matches!(
            window.appearance(),
            WindowAppearance::Light | WindowAppearance::VibrantLight
        );
        let variant = theme::resolve_variant(
            self.settings(cx).app_theme,
            system_is_light,
            self.settings(cx).light_app_theme,
            self.settings(cx).dark_app_theme,
        );
        let previous = theme::active_variant();
        if previous != variant {
            theme::set_active_variant(variant);
            self.app_store
                .update(cx, |store, cx| store.theme_changed(previous, cx));
        }
        self.apply_terminal_palette(cx);
    }

    pub(crate) fn apply_terminal_palette(&self, cx: &App) {
        self.app_store
            .read(cx)
            .bridge
            .set_palette(app_settings::terminal_palette(
                self.settings(cx),
                theme::active_variant(),
            ));
    }

    pub(crate) fn save_settings(&self, cx: &App) {
        self.app_store.read(cx).save_settings();
    }

    pub(crate) fn update_app_settings(
        &self,
        cx: &mut Context<Self>,
        persist: bool,
        update: impl FnOnce(&mut AppSettings) -> bool,
    ) -> bool {
        self.app_store.update(cx, |store, store_cx| {
            store.update_settings(update, persist, store_cx)
        })
    }

    pub(crate) fn set_sidebar_collapsed(
        &mut self,
        collapsed: bool,
        persist: bool,
        cx: &mut Context<Self>,
    ) {
        let changed = self.sidebar_collapsed != collapsed;
        self.sidebar_collapsed = collapsed;
        if collapsed {
            self.usage_open = false;
        }
        self.mission_workspace
            .update(cx, |workspace, workspace_cx| {
                workspace.set_sidebar_collapsed(collapsed, workspace_cx)
            });
        self.update_app_settings(cx, persist, |settings| {
            if settings.sidebar_collapsed == collapsed {
                return false;
            }
            settings.sidebar_collapsed = collapsed;
            true
        });
        if changed {
            cx.notify();
        }
    }

    pub(crate) fn note_window_state(&mut self, window: &Window) {
        self.window_state = window_state::snapshot(window, Some(self.window_state));
    }

    pub(crate) fn schedule_window_state_checkpoint(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.note_window_state(window);
        self.window_state_save_generation = self.window_state_save_generation.wrapping_add(1);
        let generation = self.window_state_save_generation;
        cx.spawn_in(window, async move |weak, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(WINDOW_STATE_SAVE_DELAY_MS))
                .await;
            let _ = weak.update_in(cx, |this, _, cx| {
                if this.window_state_save_generation == generation {
                    checkpoint_window_layout_deferred(cx);
                }
            });
        })
        .detach();
    }

    pub(crate) fn save_main_window_state(&mut self, window: &Window, cx: &App) {
        if self.window_label != "main" {
            return;
        }
        self.note_window_state(window);
        if let Err(error) = window_state::save(&self.core(cx).app_data_dir, self.window_state) {
            eprintln!("Runner window-state save failed: {error:#}");
        }
    }

    pub(crate) fn leave_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.shortcut_recording_active() {
            self.finish_shortcut_recording(window, cx);
        }
        self.pause_updates_pane(cx);
        self.save_settings(cx);
        match self.settings_return_route.clone() {
            AppRoute::Chat | AppRoute::Settings => {
                self.set_route(AppRoute::Chat, cx);
                match self.ensure_active_tab_attached(window, cx) {
                    Ok(()) => {
                        self.mark_active_tab_viewed(window, cx);
                        self.focus_active_terminal(window, cx);
                    }
                    Err(error) => self.chat_error = Some(error.to_string()),
                }
                cx.notify();
            }
            AppRoute::Roles => self.open_roles(window, cx),
            AppRoute::RoleDetail(handle) => self.open_role_detail(handle, window, cx),
            AppRoute::Crews => self.open_crews(window, cx),
            AppRoute::CrewEditor(crew_id) => self.open_crew_editor(crew_id, window, cx),
            AppRoute::Mission(mission_id) => self.open_mission(mission_id, window, cx),
            AppRoute::ArchivedChat => {
                self.set_route(AppRoute::ArchivedChat, cx);
                self.chat_focus.focus(window);
                cx.notify();
            }
        }
    }

    pub(crate) fn leave_archived_mission(
        &mut self,
        mission_id: &str,
        was_archived: Option<bool>,
        is_archived: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(route) =
            route_after_mission_archived(&self.route, mission_id, was_archived, is_archived)
        else {
            return;
        };
        self.set_route(route, cx);
        match self.ensure_active_tab_attached(window, cx) {
            Ok(()) => {
                self.mark_active_tab_viewed(window, cx);
                self.focus_active_terminal(window, cx);
            }
            Err(error) => self.chat_error = Some(error.to_string()),
        }
        cx.notify();
    }

    fn toggle_sidebar(&mut self, _: &ToggleSidebar, _: &mut Window, cx: &mut Context<Self>) {
        if self.route == AppRoute::Settings {
            return;
        }
        let collapsed = !self.sidebar_collapsed;
        self.set_sidebar_collapsed(collapsed, true, cx);
        if !collapsed {
            self.sidebar_preview_open = false;
        }
        self.sidebar_preview_peeking = false;
        cx.notify();
    }

    fn toggle_terminal_drawer_action(
        &mut self,
        _: &ToggleTerminalDrawer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_terminal_drawer(window, cx);
    }

    fn select_sidebar_shortcut(&mut self, index: u8, window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar.update(cx, |sidebar, sidebar_cx| {
            sidebar.select_shortcut_row(index, window, sidebar_cx)
        });
    }

    fn select_tab_1(&mut self, _: &SelectTab1, window: &mut Window, cx: &mut Context<Self>) {
        self.select_sidebar_shortcut(1, window, cx);
    }

    fn select_tab_2(&mut self, _: &SelectTab2, window: &mut Window, cx: &mut Context<Self>) {
        self.select_sidebar_shortcut(2, window, cx);
    }

    fn select_tab_3(&mut self, _: &SelectTab3, window: &mut Window, cx: &mut Context<Self>) {
        self.select_sidebar_shortcut(3, window, cx);
    }

    fn select_tab_4(&mut self, _: &SelectTab4, window: &mut Window, cx: &mut Context<Self>) {
        self.select_sidebar_shortcut(4, window, cx);
    }

    fn select_tab_5(&mut self, _: &SelectTab5, window: &mut Window, cx: &mut Context<Self>) {
        self.select_sidebar_shortcut(5, window, cx);
    }

    fn select_tab_6(&mut self, _: &SelectTab6, window: &mut Window, cx: &mut Context<Self>) {
        self.select_sidebar_shortcut(6, window, cx);
    }

    fn select_tab_7(&mut self, _: &SelectTab7, window: &mut Window, cx: &mut Context<Self>) {
        self.select_sidebar_shortcut(7, window, cx);
    }

    fn select_tab_8(&mut self, _: &SelectTab8, window: &mut Window, cx: &mut Context<Self>) {
        self.select_sidebar_shortcut(8, window, cx);
    }

    fn select_tab_9(&mut self, _: &SelectTab9, window: &mut Window, cx: &mut Context<Self>) {
        self.select_sidebar_shortcut(9, window, cx);
    }

    fn show_command_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.route == AppRoute::Settings {
            return;
        }
        self.command_palette
            .update(cx, |palette, palette_cx| palette.open(window, palette_cx));
    }

    fn open_command_palette(
        &mut self,
        _: &CommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_command_palette(window, cx);
    }

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        if self.route == AppRoute::Settings {
            return;
        }
        self.enter_settings_route(None, window, cx);
    }

    fn navigate_previous_page(
        &mut self,
        _: &NavigatePreviousPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate_runtime_page(-1, window, cx);
    }

    fn navigate_next_page(
        &mut self,
        _: &NavigateNextPage,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate_runtime_page(1, window, cx);
    }

    fn zoom_in(&mut self, _: &ZoomIn, window: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(nudge_zoom(self.settings(cx).app_zoom, 1), window, cx);
    }

    fn zoom_out(&mut self, _: &ZoomOut, window: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(nudge_zoom(self.settings(cx).app_zoom, -1), window, cx);
    }

    fn zoom_reset(&mut self, _: &ZoomReset, window: &mut Window, cx: &mut Context<Self>) {
        self.set_zoom(1., window, cx);
    }

    pub(crate) fn set_zoom(&mut self, zoom: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.update_app_settings(cx, true, |settings| {
            if settings.app_zoom == zoom {
                return false;
            }
            settings.app_zoom = zoom;
            true
        });
        window.set_rem_size(px(16. * zoom));
        mac_chrome::sync_traffic_lights(window, zoom);
        cx.notify();
    }

    fn toggle_fullscreen(
        &mut self,
        _: &ToggleFullscreen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.toggle_fullscreen();
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_thresholds_and_reset_labels() {
        use chrono::TimeDelta;
        assert_eq!(usage_tone([79.9]), UsageTone::Normal);
        assert_eq!(usage_tone([5., 80.]), UsageTone::Warning);
        assert_eq!(usage_tone([80., 100.]), UsageTone::Danger);
        let now = chrono::Utc::now();
        assert_eq!(
            reset_label(Some(now + TimeDelta::minutes(190)), now),
            "resets 3h 10m"
        );
        assert_eq!(
            reset_label(Some(now + TimeDelta::hours(164)), now),
            "resets 6d 20h"
        );
        assert_eq!(
            reset_label(Some(now + TimeDelta::seconds(30)), now),
            "resets <1m"
        );
    }

    #[test]
    fn usage_tooltip_explains_highest_window_and_pending_is_not_an_error() {
        let snapshot = UsageSnapshot {
            claude: Some(AgentUsage {
                windows: vec![
                    UsageWindow {
                        name: "5 hours".into(),
                        used_percent: 4.,
                        resets_at: None,
                    },
                    UsageWindow {
                        name: "Week".into(),
                        used_percent: 85.,
                        resets_at: None,
                    },
                ],
                updated_at: chrono::Utc::now(),
            }),
            ..UsageSnapshot::default()
        };
        assert_eq!(
            usage_tooltip(&snapshot, &[Runtime::ClaudeCode]),
            "Claude Code 85%"
        );
        assert_eq!(unavailable_line(None, Runtime::Codex), "Checking…");
    }

    #[test]
    fn sidebar_resize_bar_rides_the_divider() {
        use crate::theme_snapshot::ThemeGuard;
        use gpui::{px, size, TestAppContext, VisualTestContext};
        use runner_backend::{db, event_bus, events, mcp, router, session, shell_path, windows};
        use std::sync::{Arc, Mutex, RwLock};

        let _theme = ThemeGuard::new();
        theme::set_active_variant(theme::ThemeVariant::Carbon);
        let temp = tempfile::tempdir().unwrap();
        let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
        let runtime_discovery =
            Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
        let core = AppCore {
            db: Arc::new(db::open_pool(&temp.path().join("runner.db")).unwrap()),
            app_data_dir: temp.path().to_owned(),
            sessions: session::SessionManager::new(
                runtime_shell_env.clone(),
                runtime_discovery.clone(),
                Arc::new(session::pty_runtime::PtyRuntime::new()),
            ),
            runtime_shell_env,
            runtime_discovery,
            usage: Arc::new(runner_backend::usage::UsageService::default()),
            buses: event_bus::BusRegistry::new(),
            routers: router::RouterRegistry::new(),
            mission_grid_hint: Arc::new(Mutex::new(None)),
            mcp: Arc::new(mcp::McpHandle::new()),
            windows: Arc::new(windows::WindowRegistry::new()),
            events: events::EventChannel::new(),
            session_event_observer: Default::default(),
            app_version: "0.0.0-test".into(),
        };
        let mut cx = TestAppContext::single();
        let store = cx.new(|cx| {
            AppStore::new(
                core.clone(),
                None,
                None,
                temp.path().join("settings.json"),
                AppSettings::default(),
                None,
                cx,
            )
        });
        cx.update(|cx| {
            cx.set_global(crate::GlobalAppStore(store.clone()));
            cx.set_global(crate::WindowLayoutCheckpoint::default());
            #[cfg(not(windows))]
            let updater = cx.new(|cx| crate::Updater::new(false, cx));
            #[cfg(windows)]
            let updater = cx.new(|cx| crate::Updater::new(false, temp.path().join("updates"), cx));
            cx.set_global(crate::GlobalUpdater(updater));
        });
        let host = cx.add_window(|window, cx| {
            NativeRoot::new(
                "sidebar-resize".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            )
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(1200.), px(900.)));
        visual.run_until_parked();
        let sidebar = visual.debug_bounds("APP_SIDEBAR").expect("sidebar");
        let bar = visual
            .debug_bounds("sidebar-resize-bar")
            .expect("resize bar");
        // The divider is the sidebar's own 1px right border.
        let inside = (sidebar.right() - px(1.)) - bar.left();
        let outside = bar.right() - sidebar.right();
        assert_eq!(
            bar.size.width,
            px(runner_app::ui::resize::RESIZE_BAR),
            "{bar:?}"
        );
        assert!(
            inside > px(0.) && (inside - outside).abs() <= px(0.01),
            "the bar must ride the divider, not sit beside it: {inside:?} inside against {outside:?} outside, bar {bar:?}, sidebar {sidebar:?}"
        );
    }

    #[test]
    fn archived_mission_only_leaves_its_open_route() {
        assert_eq!(
            route_after_mission_archived(
                &AppRoute::Mission("mission-1".into()),
                "mission-1",
                Some(false),
                true,
            ),
            Some(AppRoute::Chat)
        );
        for route in [
            AppRoute::Mission("mission-2".into()),
            AppRoute::Roles,
            AppRoute::Settings,
            AppRoute::Chat,
        ] {
            assert_eq!(
                route_after_mission_archived(&route, "mission-1", Some(false), true),
                None
            );
        }
        let open_archived = AppRoute::Mission("mission-1".into());
        assert_eq!(
            route_after_mission_archived(&open_archived, "mission-1", Some(true), true),
            None
        );
        assert_eq!(
            route_after_mission_archived(&open_archived, "mission-1", None, true),
            None
        );
        assert_eq!(
            route_after_mission_archived(&open_archived, "mission-1", Some(false), false),
            None
        );
    }

    #[test]
    fn settings_update_hint_shows_available_version() {
        let available = runner_app::updater::UpdateInfo::new("0.6.1");

        assert_eq!(settings_update_hint_version(None), None);
        assert_eq!(
            settings_update_hint_version(Some(&available)),
            Some("0.6.1")
        );
    }
}
