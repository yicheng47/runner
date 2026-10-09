use gpui::{
    canvas, deferred, linear_color_stop, linear_gradient, svg, BoxShadow, Div, FontWeight,
    TextAlign, WindowAppearance, WindowControlArea,
};
use runner_app::ui::button::spinner;
use runner_app::ui::menu::popup_layer;
use runner_core::protocol::model::Runtime;
use runner_core::protocol::usage::{
    AgentUsage, RefreshReason, UnavailableReason, UsageSnapshot, UsageWindow,
};

use crate::app_settings::{clamp_sidebar_width, nudge_zoom};
use crate::toast::ToastTone;
use crate::*;
use runner_app::ui::resize::{resize_strip, resize_strip_inset, ResizeAxis};

pub(crate) const TITLEBAR_DRAG_HEIGHT: f32 = 28.;
#[cfg(target_os = "macos")]
pub(crate) const SIDEBAR_TOGGLE_GLYPH_X: f32 = 102.3;
#[cfg(target_os = "macos")]
pub(crate) const SIDEBAR_TOGGLE_GLYPH_INSET: f32 = 6.3;
const SIDEBAR_TRANSITION_MS: u64 = 200;
// A pass-through of the left edge on the way to another screen is far shorter
// than this; a deliberate rest on it is longer.
const SIDEBAR_PREVIEW_DWELL_MS: u64 = 200;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum AppRoute {
    #[default]
    Chat,
    Roles,
    NewRole,
    RoleDetail(String),
    Crews,
    NewCrew,
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

fn app_update_hint_version(available: Option<&runner_app::updater::UpdateInfo>) -> Option<&str> {
    available.map(|update| update.version())
}

pub(crate) fn terminal_style_for(
    settings: &app_settings::AppSettings,
) -> crate::terminal::element::TerminalStyle {
    crate::terminal::element::TerminalStyle {
        palette: app_settings::terminal_palette(settings, theme::active_variant()),
        font: settings.terminal_font_family.font(),
        font_size: settings.terminal_font_size as f32 * settings.app_zoom,
        app_zoom: settings.app_zoom,
    }
}

/// Agents whose npm `latest` is newer than the installed version.
pub(crate) fn agents_with_updates(core: &DaemonClient) -> Vec<Runtime> {
    core.runtime_status_list()
        .ok()
        .map(|status| {
            status
                .runtimes
                .into_iter()
                .filter(|runtime| runtime.available_version.is_some())
                .map(|runtime| runtime.name)
                .collect()
        })
        .unwrap_or_default()
}

/// The Agent settings gear carries a dot while any enabled agent has an
/// update available; a disabled agent's update does not count.
pub(crate) fn update_dot_visible(
    updates: &[Runtime],
    settings: &app_settings::AppSettings,
) -> bool {
    updates.iter().any(|runtime| {
        settings.is_agent_enabled(
            *runtime,
            runner_core::protocol::runtime_metadata::runtime_default_enabled(*runtime),
        )
    })
}

pub(crate) fn usage_installed(core: &DaemonClient) -> Vec<Runtime> {
    core.runtime_status_list()
        .ok()
        .map(|status| {
            status
                .runtimes
                .into_iter()
                .filter(|runtime| {
                    crate::runtime_ui::catalog_capabilities(&[], runtime.name.key()).usage
                        && matches!(
                            runtime.effective_source,
                            Some(
                                runner_core::protocol::RuntimeCommandSource::Detected
                                    | runner_core::protocol::RuntimeCommandSource::Override
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
        return "reset unknown".into();
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
        Some(UnavailableReason::KeychainDenied) => {
            "Runner was not allowed to read Claude Code's sign-in from the Keychain."
        }
        Some(UnavailableReason::KeychainUnavailable) => {
            "Couldn't read Claude Code's sign-in from the Keychain."
        }
        Some(UnavailableReason::ClaudeUnreachable) => "Couldn't reach Anthropic.",
        Some(UnavailableReason::CodexNoAnswer) => "Codex didn't answer.",
        Some(UnavailableReason::AntigravityNoAnswer) => "Antigravity CLI didn't answer.",
        Some(UnavailableReason::InvalidResponse) => {
            crate::runtime_ui::runtime_ui(runtime).usage_invalid
        }
        None => "Checking…",
    }
}

fn weekly_usage_percent(runtime: Runtime, usage: Option<&AgentUsage>) -> Option<f64> {
    let name = crate::runtime_ui::runtime_ui(runtime).usage_week;
    usage?
        .windows
        .iter()
        .find(|window| window.name == name)
        .map(|window| window.used_percent)
}

/// The runtimes the usage footer and popover show: usage-capable, installed and
/// enabled in Settings, whether or not they have run a session (#752), in
/// `Runtime::ALL` order.
fn visible_usage_runtimes(installed: &[Runtime], enabled: &[Runtime]) -> Vec<Runtime> {
    Runtime::ALL
        .into_iter()
        .filter(|runtime| {
            crate::runtime_ui::catalog_capabilities(&[], runtime.key()).usage
                && installed.contains(runtime)
                && enabled.contains(runtime)
        })
        .collect()
}

fn usage_popover_runtimes(installed: &[Runtime], enabled: &[Runtime]) -> Vec<Runtime> {
    let mut runtimes = visible_usage_runtimes(installed, enabled);
    runtimes.sort_by_key(|runtime| crate::runtime_ui::runtime_ui(*runtime).usage_order);
    runtimes
}

fn sidebar_usage_element(
    snapshot: &UsageSnapshot,
    visible: &[Runtime],
    zoom: f32,
) -> gpui::Stateful<Div> {
    let entries: Vec<_> = visible
        .iter()
        .map(|runtime| {
            let usage = snapshot.get(*runtime);
            let percent = weekly_usage_percent(*runtime, usage);
            let value = percent.map_or_else(|| "—".to_owned(), |value| format!("{value:.0}%"));
            let color = percent.map_or(theme::muted(), |value| usage_color(usage_tone([value])));
            div()
                .when(cfg!(test), |entry| {
                    let runtime = *runtime;
                    entry.debug_selector(move || format!("SIDEBAR_USAGE_ENTRY_{}", runtime.key()))
                })
                .flex_none()
                .flex()
                .items_center()
                .gap(px(2. * zoom))
                .child({
                    let icon = ChatIcon::for_runtime(runtime.key());
                    icon.render(px(12. * zoom), icon.color(theme::text(), true), true)
                })
                .child(
                    div()
                        .when(cfg!(test), |value| {
                            let runtime = *runtime;
                            value.debug_selector(move || {
                                format!("SIDEBAR_USAGE_VALUE_{}", runtime.key())
                            })
                        })
                        .text_size(px(11. * zoom))
                        .text_color(color)
                        .child(value),
                )
        })
        .collect();
    div()
        .id("sidebar-usage")
        .when(cfg!(test), |usage| {
            usage.debug_selector(|| "SIDEBAR_USAGE".into())
        })
        .w_full()
        .h(px(32. * zoom))
        .flex()
        .items_center()
        .justify_between()
        .px(px(10. * zoom))
        .rounded(px(6. * zoom))
        .children(entries)
}

fn usage_window_row(
    runtime: Runtime,
    window: &UsageWindow,
    label: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> AnyElement {
    let tone = usage_tone([window.used_percent]);
    let fill = usage_color(tone);
    div()
        .w_full()
        .flex()
        .items_center()
        .gap_2()
        .child(Tooltip::new(
            format!("usage-window-{}-{}", runtime.key(), window.name),
            window.name.clone(),
            div()
                .w(rems(76. / 16.))
                .flex_none()
                .truncate()
                .text_size(theme::text_meta())
                .text_color(theme::text())
                .child(label.to_owned()),
        ))
        .child(
            div()
                .flex_1()
                .h(rems(4. / 16.))
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
                .w(rems(30. / 16.))
                .flex_none()
                .text_align(TextAlign::Right)
                .text_size(theme::text_meta())
                .text_color(fill)
                .child(format!("{:.0}%", window.used_percent)),
        )
        .child(
            div()
                .w(rems(84. / 16.))
                .flex_none()
                .text_align(TextAlign::Right)
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
    refreshing: bool,
    now: chrono::DateTime<chrono::Utc>,
) -> AnyElement {
    let ui = crate::runtime_ui::runtime_ui(runtime);
    let name = ui.usage_title;
    let refresh_spinner_id = ui.usage_refresh_spinner;
    #[cfg(test)]
    crate::catalog_golden::record(
        serde_json::json!({"runtime":runtime,"title":name,"refresh_spinner":refresh_spinner_id}),
    );
    let icon = ChatIcon::for_runtime(runtime.key());
    let mut rows = Vec::new();
    let mut previous_group = None;
    if let Some(usage) = usage {
        for window in &usage.windows {
            let (group, label) = if ui.usage_groups {
                window
                    .name
                    .split_once(" · ")
                    .map_or((None, window.name.as_str()), |(group, label)| {
                        (Some(group), label.strip_suffix(" used").unwrap_or(label))
                    })
            } else {
                (None, window.name.as_str())
            };
            #[cfg(test)]
            crate::catalog_golden::record(
                serde_json::json!({"window":window.name,"group":group,"label":label}),
            );
            if let Some(group) = group.filter(|group| Some(*group) != previous_group) {
                rows.push(
                    div()
                        .text_size(theme::text_meta())
                        .text_color(theme::muted())
                        .child(group.to_owned())
                        .into_any_element(),
                );
            }
            previous_group = group;
            rows.push(usage_window_row(runtime, window, label, now));
        }
    }
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_2()
        .py_3()
        .when(cfg!(test), |section| {
            section.debug_selector(move || format!("USAGE_SECTION_{}", runtime.key()))
        })
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_2()
                .child(icon.render(px(16.), icon.color(theme::text(), true), true))
                .child(
                    div()
                        .text_size(theme::text_title())
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(name),
                )
                .child(div().flex_1())
                .children(
                    (refreshing && usage.is_some())
                        .then(|| spinner(refresh_spinner_id, 12., theme::muted())),
                ),
        )
        .children(rows)
        .children(
            (usage.is_none() && reason.is_none() && refreshing).then(|| {
                let id = ui.usage_checking_spinner;
                #[cfg(test)]
                crate::catalog_golden::record(serde_json::json!({"checking_spinner":id}));
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(theme::text_body())
                    .text_color(theme::muted())
                    .child(spinner(id, 12., theme::muted()))
                    .child("Checking…")
            }),
        )
        .children(
            (usage.is_none() && (reason.is_some() || !refreshing)).then(|| {
                div()
                    .text_size(theme::text_body())
                    .text_color(theme::muted())
                    .child(unavailable_line(reason, runtime))
            }),
        )
        .into_any_element()
}

fn usage_popover_panel(
    max_height: Pixels,
    header: AnyElement,
    sections: Vec<AnyElement>,
) -> AnyElement {
    div()
        .w_full()
        .max_h(max_height)
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
            inset: false,
            offset: point(px(0.), px(4.)),
        }])
        .when(cfg!(test), |panel| {
            #[cfg(test)]
            return crate::theme_snapshot::record_fill("USAGE_POPOVER_PANEL", panel);
            #[cfg(not(test))]
            panel
        })
        .child(div().flex_none().child(header))
        .child(
            div()
                .id("usage-popover-body")
                .min_h(px(0.))
                .flex_shrink(1.)
                .overflow_y_scroll()
                .scrollbar_width(px(0.))
                .when(cfg!(test), |body| {
                    body.debug_selector(|| "USAGE_POPOVER_BODY".into())
                })
                .children(sections),
        )
        .into_any_element()
}

pub(crate) fn alpha(mut color: gpui::Hsla, value: f32) -> gpui::Hsla {
    color.a = value;
    color
}

impl NativeRoot {
    fn render_usage_popover(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let snapshot = self.app_store.read(cx).usage.clone();
        let now = chrono::Utc::now();
        let updated = snapshot
            .runtimes
            .values()
            .filter_map(|entry| entry.value.as_ref())
            .map(|usage| usage.updated_at)
            .max();
        let age = if snapshot.refreshing {
            "Updating…".to_owned()
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
            .when(!snapshot.refreshing, |button| {
                button
                    .cursor_pointer()
                    .hover(|button| button.bg(theme::sidebar_selected()))
                    .on_click(cx.listener(|this, _, _, cx| {
                        let core = this.core(cx).clone();
                        let _ = core.usage_refresh(RefreshReason::Button);
                        cx.notify();
                    }))
            })
            .child(if snapshot.refreshing {
                spinner("usage-refresh-spinner", 14., theme::muted())
            } else {
                svg()
                    .path("refresh-cw.svg")
                    .size(px(14.))
                    .text_color(theme::muted())
                    .into_any_element()
            });
        let update_dot = update_dot_visible(&self.agent_updates, self.settings(cx));
        let owner = cx.weak_entity();
        let agent_settings = Tooltip::new(
            "usage-agent-settings-tooltip",
            if update_dot {
                "Agent settings · Update available"
            } else {
                "Agent settings"
            },
            div()
                .id("usage-agent-settings")
                .debug_selector(|| "USAGE_AGENT_SETTINGS".into())
                .relative()
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_sm()
                .cursor_pointer()
                .hover(|button| button.bg(theme::sidebar_selected()))
                .on_click(move |_, window, cx| {
                    let _ = owner.update(cx, |this, cx| {
                        this.usage_open = false;
                        this.enter_settings_route(Some("agents"), window, cx);
                    });
                })
                .child(
                    svg()
                        .path("settings.svg")
                        .size(px(14.))
                        .text_color(theme::muted()),
                )
                .children(update_dot.then(|| {
                    div()
                        .debug_selector(|| "USAGE_AGENT_SETTINGS_DOT".into())
                        .absolute()
                        .top(px(3.))
                        .right(px(3.))
                        .size(px(6.))
                        .rounded_full()
                        .bg(theme::accent())
                })),
        );
        let sections: Vec<AnyElement> =
            usage_popover_runtimes(&self.usage_installed, &self.settings(cx).model_runtimes())
                .into_iter()
                .map(|runtime| {
                    usage_section(
                        runtime,
                        snapshot.get(runtime),
                        snapshot.error(runtime),
                        snapshot.refreshing,
                        now,
                    )
                })
                .collect();
        let header = div()
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
            .child(refresh)
            .child(agent_settings)
            .into_any_element();
        usage_popover_panel(window.bounds().size.height - px(16.), header, sections)
    }

    pub(crate) fn render_app_shell(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.sync_theme(window, cx);
        runner_app::appearance::configure(self.settings(cx).window_material, cx);
        window.set_background_appearance(
            runner_app::appearance::effective(self.settings(cx).window_material, cx).background(),
        );
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
            .then(|| self.render_start_chat_modal(window, cx));
        let mission_modal = self
            .start_mission_modal
            .is_some()
            .then(|| self.render_start_mission_modal(cx));
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
            self.render_entity_overlays(window, cx)
        } else {
            Vec::new()
        };
        let chrome = div()
            .debug_selector(|| "APP_CHROME".into())
            .relative()
            .size_full()
            .flex()
            .children(sidebar)
            .child(
                runner_app::ui::work_card::work_card(self.sidebar_collapsed)
                    .when(cfg!(test), |column| {
                        column.debug_selector(|| "APP_CONTENT_COLUMN".into())
                    })
                    .relative()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .children(self.render_main_titlebar_drag_area(cx))
                    .children(
                        (!matches!(
                            self.route,
                            AppRoute::Chat
                                | AppRoute::ArchivedChat
                                | AppRoute::Mission(_)
                                | AppRoute::Settings
                        ))
                        .then(|| self.render_daemon_banner(cx))
                        .flatten()
                        .map(|banner| div().flex_none().pt(rems(44. / 16.)).child(banner)),
                    )
                    .child(workspace),
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
            .bg(theme::chrome())
            .text_color(theme::text())
            .children((self.route != AppRoute::Settings).then_some(chrome))
            .children(settings)
            .children(self.render_window_navigation(window, cx))
            .children(modal)
            .children(mission_modal)
            .children(settings_confirm)
            .child(command_palette)
            .children(toast)
            .children(
                self.quit_dialog
                    .clone()
                    .map(|dialog| deferred(dialog).with_priority(200)),
            )
            .children(
                self.agent_update
                    .clone()
                    .map(|dialog| deferred(dialog).with_priority(150)),
            )
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
            .on_action(cx.listener(Self::new_terminal_action))
            .on_action(cx.listener(Self::new_mission_action))
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
        let content = self.cached_region(
            "sidebar",
            gpui::StyleRefinement::default()
                .w(px(full_width))
                .h_full()
                .flex_none(),
            move |root, window, cx| root.render_sidebar_content(full_width, window, cx),
            cx,
        );
        let mut sidebar = div()
            .id("app-sidebar")
            .relative()
            .w(px(width))
            .h_full()
            .flex_none()
            .overflow_hidden()
            .opacity(visibility)
            .bg(if theme::is_glass() {
                gpui::transparent_black()
            } else {
                theme::sidebar()
            })
            .map(|element| {
                #[cfg(test)]
                let element = crate::theme_snapshot::record_fill("APP_SIDEBAR", element);
                element
            })
            .child(content);
        let divider = visible.then_some(width);
        if preview {
            sidebar = sidebar
                .bg(theme::chrome())
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

    fn render_sidebar_brand(&self, update_version: Option<String>, cx: &Context<Self>) -> Div {
        let zoom = self.settings(cx).app_zoom;
        let updater = global_updater(cx);
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
                    .when(cfg!(test), |button| {
                        button.debug_selector(|| "SIDEBAR_UPDATE".into())
                    })
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
                    .border_color(theme::chrome_selected_border())
                    .bg(theme::chrome_hover())
                    .text_color(theme::text())
            })
            .focus_visible(|button| {
                button
                    .border_color(theme::chrome_selected_border())
                    .bg(theme::chrome_hover())
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
        div()
            .when(cfg!(test), |brand| {
                brand.debug_selector(|| "SIDEBAR_HEADER".into())
            })
            .flex_none()
            .px_5()
            .pb_5()
            .pt_1()
            .flex()
            .items_center()
            .gap_2()
            .child(
                svg()
                    .when(cfg!(test), |mark| {
                        mark.debug_selector(|| "SIDEBAR_BRAND_MARK".into())
                    })
                    .flex_none()
                    .path("brand-mark.svg")
                    .size(px(20. * self.settings(cx).app_zoom))
                    .text_color(theme::accent()),
            )
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .when(cfg!(test), |name| {
                                name.debug_selector(|| "SIDEBAR_NAME".into())
                            })
                            .flex_none()
                            .text_size(theme::text_heading())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child("Runner"),
                    )
                    .children(update_hint),
            )
            .child(Tooltip::new(
                "sidebar-search-tooltip",
                keymap::effective_binding("command-palette", &self.settings(cx).keymap_overrides)
                    .map_or_else(
                        || "Search".to_owned(),
                        |combo| format!("Search ({})", keymap::format_combo(&combo)),
                    ),
                search_button,
            ))
    }

    fn render_sidebar_content(
        &mut self,
        full_width: f32,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.sync_sidebar_shortcut_rows(cx);
        let titlebar = self.render_sidebar_titlebar(window, cx);
        let zoom = self.settings(cx).app_zoom;
        let updater = global_updater(cx);
        let update_version =
            app_update_hint_version(updater.read(cx).available()).map(str::to_owned);
        let brand = self.render_sidebar_brand(update_version, cx);
        let enabled = self.settings(cx).model_runtimes();
        let visible_usage_runtimes = visible_usage_runtimes(&self.usage_installed, &enabled);
        let show_usage = !visible_usage_runtimes.is_empty() && !self.sidebar_collapsed;
        let anchor_owner = cx.entity();
        let usage = show_usage.then(|| {
            let snapshot = self.app_store.read(cx).usage.clone();
            let trigger = sidebar_usage_element(&snapshot, &visible_usage_runtimes, zoom)
                .occlude()
                .cursor_pointer()
                .hover(|button| button.bg(theme::chrome_hover()))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.usage_open = !this.usage_open;
                    if this.usage_open {
                        let core = this.core(cx).clone();
                        let _ = core.usage_refresh(RefreshReason::Open);
                        let _ = this.core(cx).runtime_check_updates(false);
                    }
                    cx.notify();
                }));
            div()
                .relative()
                .w(px((visible_usage_runtimes.len() as f32 * 52. + 20.)
                    .min(self.settings(cx).sidebar_width - 56.)
                    * zoom))
                .flex_none()
                .child(trigger)
                .child(
                    canvas(
                        |_, _, _| {},
                        move |bounds, _, _, cx| {
                            anchor_owner.update(cx, |this, _| this.usage_anchor = Some(bounds));
                        },
                    )
                    .absolute()
                    .inset_0(),
                )
        });
        let footer = crate::platform_ui::sidebar_section()
            .when(cfg!(test), |footer| {
                footer.debug_selector(|| "SIDEBAR_FOOTER".into())
            })
            .px_3()
            .child(
                div()
                    .w_full()
                    .h(px(1.))
                    .flex()
                    .children([90., 270.].map(|angle| {
                        div().flex_1().h_full().bg(linear_gradient(
                            angle,
                            linear_color_stop(alpha(theme::chrome_selected_border(), 0.), 0.),
                            linear_color_stop(theme::chrome_selected_border(), 1.),
                        ))
                    })),
            )
            .child(
                div()
                    .when(cfg!(test), |row| {
                        row.debug_selector(|| "SIDEBAR_FOOTER_ROW".into())
                    })
                    .w_full()
                    .pt_2()
                    .flex()
                    .items_center()
                    .children(usage)
                    .child(div().flex_1())
                    .child(
                        div().occlude().cursor_pointer().child(Tooltip::new(
                            "sidebar-settings-tooltip",
                            "Settings",
                            div()
                                .id("open-settings")
                                .group("sidebar-settings")
                                .when(cfg!(test), |button| {
                                    button.debug_selector(|| "SIDEBAR_SETTINGS".into())
                                })
                                .tab_index(0)
                                .flex_none()
                                .size(px(32. * zoom))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(4. * zoom))
                                .cursor_pointer()
                                .hover(|button| button.bg(theme::chrome_hover()))
                                .focus_visible(|button| button.bg(theme::chrome_hover()))
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.enter_settings_route(None, window, cx);
                                }))
                                .on_key_down(cx.listener(
                                    |this, event: &KeyDownEvent, window, cx| {
                                        if matches!(event.keystroke.key.as_str(), "enter" | "space")
                                        {
                                            cx.stop_propagation();
                                            this.enter_settings_route(None, window, cx);
                                        }
                                    },
                                ))
                                .child(
                                    svg()
                                        .path("settings.svg")
                                        .size(px(16. * zoom))
                                        .text_color(theme::faint())
                                        .group_hover("sidebar-settings", |icon| {
                                            icon.text_color(theme::text())
                                        }),
                                ),
                        )),
                    ),
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
            .child(footer)
            .children(
                (self.usage_open && show_usage)
                    .then(|| {
                        self.usage_anchor.map(|anchor| {
                            let owner = cx.entity();
                            popup_layer(
                                anchor,
                                window,
                                px(340. * zoom),
                                self.render_usage_popover(window, cx),
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
        content.into_any_element()
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
            false,
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

    pub(crate) fn render_daemon_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let store = self.app_store.read(cx);
        if !store.daemon_disconnected
            || store.daemon_notice != Some(runner_app::lifecycle::DaemonNotice::Repeated)
        {
            return None;
        }
        let root = cx.weak_entity();
        let log_path = self.log_dir.join("runnerd.log");
        Some(
            runner_app::ui::notice_banner(
                div()
                    .debug_selector(|| "DAEMON_CRASH_MESSAGE".into())
                    .truncate()
                    .child(runner_core::protocol::managed::RESTART_LIMIT_NOTICE),
                runner_app::ui::Tone::Danger,
            )
            .id("daemon-crash-banner")
            .debug_selector(|| "DAEMON_CRASH_BANNER".into())
            .child(
                div()
                    .flex_none()
                    .debug_selector(|| "DAEMON_OPEN_LOG".into())
                    .child(
                        Button::new("daemon-open-log", "Open log")
                            .size(ButtonSize::Sm)
                            .variant(runner_app::ui::ButtonVariant::Ghost)
                            .on_press(move |_, cx| cx.reveal_path(&log_path)),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .debug_selector(|| "DAEMON_TRY_AGAIN".into())
                    .child(
                        Button::new("daemon-try-again", "Try again")
                            .size(ButtonSize::Sm)
                            .radius(8.)
                            .variant(runner_app::ui::ButtonVariant::Secondary)
                            .disabled(self.daemon_retrying)
                            .loading(self.daemon_retrying)
                            .on_press(move |window, cx| {
                                let _ = root.update(cx, |this, cx| this.retry_daemon(window, cx));
                            }),
                    ),
            )
            .into_any_element(),
        )
    }

    fn render_toast(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        self.toasts.active().map(|toast| {
            let zoom = self.settings(cx).app_zoom;
            let icon = toast.tone.icon();
            let icon_color = match toast.tone {
                ToastTone::Info => theme::muted(),
                ToastTone::Success | ToastTone::Restart => theme::accent(),
                ToastTone::Error => theme::danger(),
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
                        .debug_selector(|| "GLOBAL_TOAST".into())
                        .max_w(px(if toast.single_line { 600. } else { 420. } * zoom))
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
                            inset: false,
                        }])
                        .cursor_pointer()
                        .occlude()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, _, cx| {
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
                                .debug_selector(|| "GLOBAL_TOAST_MESSAGE".into())
                                .min_w(px(0.))
                                .whitespace_normal()
                                .when(toast.single_line, |message| message.truncate())
                                .text_size(theme::text_body())
                                .text_color(theme::text())
                                .line_height(px(18. * zoom))
                                .child(SharedString::from(toast.message.clone())),
                        )
                        .child(
                            svg()
                                .debug_selector(|| "GLOBAL_TOAST_CLOSE".into())
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
        if self.app_store.read(cx).daemon_disconnected {
            return;
        }
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

    fn retry_daemon(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.daemon_retrying {
            return;
        }
        self.daemon_retrying = true;
        self.mission_workspace.update(cx, |_, cx| cx.notify());
        let client = self.core(cx).clone();
        let recovery = cx.background_spawn(async move { client.reconnect() });
        cx.spawn_in(window, async move |weak, cx| {
            let result = recovery.await;
            let _ = weak.update_in(cx, |this, _, cx| {
                this.daemon_retrying = false;
                this.mission_workspace.update(cx, |_, cx| cx.notify());
                if let Err(error) = result {
                    tracing::warn!("runnerd manual recovery failed: {error}");
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn terminal_style(&self, cx: &App) -> crate::terminal::element::TerminalStyle {
        terminal_style_for(self.settings(cx))
    }

    pub(crate) fn render_collapsed_titlebar_spacer(&self) -> Option<AnyElement> {
        (self.sidebar_collapsed && !cfg!(windows)).then(|| {
            div()
                .debug_selector(|| "WINDOW_NAVIGATION_SPACE".into())
                .flex_none()
                .w(rems(92. / 16.))
                .h(rems(28. / 16.))
                .into_any_element()
        })
    }

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
        let zoom = self.settings(cx).app_zoom;
        if self.sidebar_collapsed && !cfg!(windows) {
            // The card's scaled inset and unscaled edge already consume part of the window anchor.
            platform_ui::navigation_left(window, zoom) - 8. * zoom - 1.
        } else {
            16. * zoom
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
        if let Err(error) =
            window_state::save(&self.app_store.read(cx).app_data_dir, self.window_state)
        {
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
            AppRoute::NewRole => self.open_create_role(window, cx),
            AppRoute::RoleDetail(handle) => self.open_role_detail(handle, window, cx),
            AppRoute::Crews => self.open_crews(window, cx),
            AppRoute::NewCrew => self.open_create_crew(window, cx),
            AppRoute::CrewEditor(crew_id) => self.open_crew_editor(crew_id, window, cx),
            AppRoute::Mission(mission_id) => self.open_mission(mission_id, window, cx),
            AppRoute::ArchivedChat => {
                self.set_route(AppRoute::ArchivedChat, cx);
                self.chat_focus.focus(window, cx);
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

    pub(crate) fn toggle_window_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.route == AppRoute::Settings {
            return;
        }
        let collapsed = !self.sidebar_collapsed;
        self.set_sidebar_collapsed(collapsed, true, cx);
        self.sidebar_preview_peeking = false;
        if !collapsed {
            self.sidebar_preview_open = false;
            if matches!(self.route, AppRoute::Mission(_)) {
                self.mission_workspace.update(cx, |workspace, cx| {
                    workspace.focus_active_mission_terminal(window, cx);
                });
            } else if self.route == AppRoute::Chat {
                self.focus_active_terminal(window, cx);
            }
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
        self.sync_sidebar_shortcut_rows(cx);
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

    #[cfg(unix)]
    #[test]
    fn work_card_insets_every_route_and_contains_chat_and_mission_columns() {
        use crate::theme_snapshot::{assert_fill, ThemeGuard};
        use gpui::{size, TestAppContext, VisualTestContext};
        let _theme = ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            let conn = store.test_core.db.get().unwrap();
            let mut layout = runner_app::pane_layout::PaneLayout::single(
                Some("direct-agent"),
                &["direct-agent".into()],
            );
            let focused = layout.focused_pane_id.clone();
            layout
                .split(&focused, runner_app::pane_layout::SplitOrientation::Row)
                .unwrap();
            layout.focus_pane(&focused);
            runner_daemon::repo::node::create_tab(&conn, None, "", 0, &layout.serialize().unwrap())
                .unwrap();
            store.replace_nodes(runner_daemon::repo::node::list(&conn).unwrap(), cx);
        });
        for path in [
            "/chats/direct-agent",
            "/missions/test-mission",
            "/roles",
            "/crews",
            "/settings",
        ] {
            let host = cx.add_window(|window, cx| {
                let mut root = NativeRoot::new(
                    format!("card-{path}"),
                    temp.path().join("logs"),
                    Some(path.into()),
                    None,
                    store.clone(),
                    window,
                    cx,
                );
                root.set_sidebar_collapsed(false, false, cx);
                root.sidebar_visibility = SidebarVisibilityTransition::new(true);
                root
            });
            let mut visual = VisualTestContext::from_window(host.into(), &cx);
            visual.simulate_resize(size(px(1440.), px(900.)));
            visual.run_until_parked();
            let (card_selector, chrome_selector) = if path == "/settings" {
                ("WORK_CARD", "SETTINGS_CHROME")
            } else {
                ("APP_CONTENT_COLUMN", "APP_CHROME")
            };
            let card = visual.debug_bounds(card_selector).unwrap();
            let chrome = visual.debug_bounds(chrome_selector).unwrap();
            if path.starts_with("/chats/") {
                assert_fill(&mut visual, "CHAT_TAB_HEADER_CONTENT", None);
                assert_fill(&mut visual, "CHAT_PANEL_HEADER", None);
                assert_fill(&mut visual, "PANE_IDENTITY_LINE", theme::colors().panel);
            } else if path.starts_with("/missions/") {
                assert_fill(&mut visual, "MISSION_HEADER_ROW", None);
                assert_fill(&mut visual, "MISSION_RAIL_HEADER", None);
            }
            assert_eq!(card.top(), chrome.top() + px(8.), "{path}");
            assert_eq!(card.right(), chrome.right() - px(8.), "{path}");
            assert_eq!(card.bottom(), chrome.bottom() - px(8.), "{path}");
            assert_eq!(
                card.left(),
                chrome.left()
                    + px(store.read_with(&visual, |store, _| store.settings.sidebar_width)),
                "{path}"
            );
            for selector in if path.starts_with("/chats/") {
                &["CHAT_TAB_HEADER", "CHAT_SIDE_PANEL"][..]
            } else if path.starts_with("/missions/") {
                &["MISSION_HEADER", "MISSION_RAIL"][..]
            } else {
                &[][..]
            } {
                let child = visual.debug_bounds(selector).unwrap();
                assert!(
                    child.left() > card.left() && child.right() < card.right(),
                    "{path}: {selector} {child:?} in {card:?}"
                );
                assert_eq!(child.top(), card.top() + px(1.), "{path}: {selector}");
                assert!(
                    child.bottom() <= card.bottom() - px(1.),
                    "{path}: {selector}"
                );
                if matches!(*selector, "CHAT_SIDE_PANEL" | "MISSION_RAIL") {
                    assert_eq!(child.bottom(), card.bottom() - px(1.), "{path}: {selector}");
                }
            }
            if path != "/settings" {
                host.update(&mut visual, |root, _, cx| {
                    root.set_sidebar_collapsed(true, false, cx);
                })
                .unwrap();
                visual.run_until_parked();
                // Finish the sidebar's bounded transition before checking its settled layout.
                host.update(&mut visual, |root, _, cx| {
                    root.sidebar_visibility = SidebarVisibilityTransition::new(false);
                    cx.notify();
                })
                .unwrap();
                visual.run_until_parked();
                assert_eq!(
                    visual.debug_bounds("APP_CONTENT_COLUMN").unwrap().left(),
                    px(8.)
                );
            }
            host.update(&mut visual, |_, window, _| window.remove_window())
                .unwrap();
        }
    }

    #[test]
    fn window_navigation_stays_fixed_when_sidebar_animates() {
        use crate::theme_snapshot::ThemeGuard;
        use gpui::{size, Modifiers, TestAppContext, VisualTestContext};
        let _theme = ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            let conn = store.test_core.db.get().unwrap();
            let layout = runner_app::pane_layout::PaneLayout::single(
                Some("direct-agent"),
                &["direct-agent".into()],
            );
            runner_daemon::repo::node::create_tab(&conn, None, "", 0, &layout.serialize().unwrap())
                .unwrap();
            store.replace_nodes(runner_daemon::repo::node::list(&conn).unwrap(), cx);
        });
        for path in [
            "/chats/direct-agent",
            "/missions/test-mission",
            "/roles",
            "/crews",
            "/settings",
        ] {
            let host = cx.add_window(|window, cx| {
                let mut root = NativeRoot::new(
                    format!("navigation-{path}"),
                    temp.path().join("logs"),
                    Some(path.into()),
                    None,
                    store.clone(),
                    window,
                    cx,
                );
                root.set_sidebar_collapsed(false, false, cx);
                root.sidebar_visibility = SidebarVisibilityTransition::new(true);
                root
            });
            let mut visual = VisualTestContext::from_window(host.into(), &cx);
            visual.simulate_resize(size(px(1440.), px(900.)));
            for zoom in [1., 1.5] {
                host.update(&mut visual, |root, window, cx| {
                    root.set_zoom(zoom, window, cx)
                })
                .unwrap();
                visual.run_until_parked();
                let selectors = [
                    "WINDOW_SIDEBAR_TOGGLE",
                    "WINDOW_PREVIOUS_PAGE",
                    "WINDOW_NEXT_PAGE",
                ];
                let original = selectors.map(|selector| visual.debug_bounds(selector).unwrap());
                let controls_center = if cfg!(windows) { 16. } else { 30. } * zoom;
                for bounds in original {
                    assert_eq!(
                        bounds.center().y,
                        px(controls_center),
                        "{path} at {zoom}: {bounds:?}"
                    );
                    assert_eq!(
                        bounds.size,
                        size(px(28. * zoom), px(28. * zoom)),
                        "{path} at {zoom}"
                    );
                }
                if cfg!(windows) {
                    let caption = visual.debug_bounds("WINDOW_CAPTION_BAR").unwrap();
                    let buttons = visual.debug_bounds("WINDOW_CAPTION_CONTROLS").unwrap();
                    assert_eq!(caption.size.height, px(32. * zoom));
                    assert_eq!(buttons.center().y, original[0].center().y);
                    assert!(original[0].top() >= caption.top());
                    assert!(original[0].bottom() <= caption.bottom());
                    let (card_selector, chrome_selector) = if path == "/settings" {
                        ("WORK_CARD", "SETTINGS_CHROME")
                    } else {
                        ("APP_CONTENT_COLUMN", "APP_CHROME")
                    };
                    assert_eq!(
                        visual.debug_bounds(card_selector).unwrap().top(),
                        visual.debug_bounds(chrome_selector).unwrap().top(),
                        "{path} at {zoom}: card starts below the titlebar"
                    );
                }
                let header_selector = if path.starts_with("/chats/") {
                    Some("CHAT_TAB_HEADER_CONTENT")
                } else if path.starts_with("/missions/") {
                    Some("MISSION_HEADER_ROW")
                } else {
                    None
                };
                if let Some(selector) = header_selector {
                    let header = visual.debug_bounds(selector).unwrap();
                    assert!(
                        (header.center().y
                            - original[0].center().y
                            - px(if cfg!(windows) { 38. * zoom } else { 0. }))
                        .abs()
                            <= px(1.),
                        "{path} at {zoom}: {header:?} vs {:?}",
                        original[0]
                    );
                }
                visual.simulate_click(original[0].center(), Modifiers::default());
                visual.run_until_parked();
                host.read_with(&visual, |root, _| {
                    assert_eq!(root.sidebar_collapsed, path != "/settings")
                })
                .unwrap();
                for (selector, before) in selectors.into_iter().zip(original) {
                    assert_eq!(
                        visual.debug_bounds(selector).unwrap(),
                        before,
                        "{path} at {zoom}: collapse moved {selector}"
                    );
                }
                if path != "/settings" {
                    host.update(&mut visual, |root, _, cx| {
                        root.sidebar_visibility = SidebarVisibilityTransition::new(false);
                        cx.notify();
                    })
                    .unwrap();
                    visual.run_until_parked();
                    for (selector, before) in selectors.into_iter().zip(original) {
                        assert_eq!(
                            visual.debug_bounds(selector).unwrap(),
                            before,
                            "{path} at {zoom}: settled collapse moved {selector}"
                        );
                    }
                    if cfg!(windows) {
                        assert!(visual.debug_bounds("WINDOW_NAVIGATION_SPACE").is_none());
                    } else if header_selector.is_some() {
                        let reserved = visual.debug_bounds("WINDOW_NAVIGATION_SPACE").unwrap();
                        assert!(
                            (reserved.left() - original[0].left()).abs() <= px(1.),
                            "{path} at {zoom}: {reserved:?} vs {:?}",
                            original[0]
                        );
                        assert!(
                            reserved.right() >= original[2].right(),
                            "{path} at {zoom}: title overlaps controls"
                        );
                    }
                    visual.simulate_click(original[0].center(), Modifiers::default());
                    visual.run_until_parked();
                    host.read_with(&visual, |root, _| assert!(!root.sidebar_collapsed))
                        .unwrap();
                    for (selector, before) in selectors.into_iter().zip(original) {
                        assert_eq!(
                            visual.debug_bounds(selector).unwrap(),
                            before,
                            "{path} at {zoom}: expand moved {selector}"
                        );
                    }
                    host.update(&mut visual, |root, _, cx| {
                        root.sidebar_visibility = SidebarVisibilityTransition::new(true);
                        cx.notify();
                    })
                    .unwrap();
                    visual.run_until_parked();
                }
            }
            host.update(&mut visual, |_, window, _| window.remove_window())
                .unwrap();
        }
    }

    struct RetryService {
        core: runner_daemon::daemon::InProcessTransport,
        down: std::sync::atomic::AtomicBool,
        error: &'static str,
        fail_once: std::sync::atomic::AtomicBool,
        attempts: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }
    impl runner_core::protocol::Transport for RetryService {
        fn call(
            &self,
            request: runner_core::protocol::Request,
        ) -> Result<runner_core::protocol::Response, runner_core::protocol::ClientError> {
            if self.down.load(std::sync::atomic::Ordering::Acquire) {
                return Err(runner_core::protocol::ClientError::msg(self.error));
            }
            self.core.call(request)
        }
        fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
            self.core.subscribe()
        }
        fn reconnect(&self) -> Result<(), runner_core::protocol::ClientError> {
            use std::sync::atomic::Ordering;
            self.attempts.fetch_add(1, Ordering::AcqRel);
            if self.fail_once.swap(false, Ordering::AcqRel) {
                return Err(runner_core::protocol::ClientError::msg("test start failed"));
            }
            self.down.store(false, Ordering::Release);
            self.core
                .0
                .events
                .emit("daemon/reconnected", &serde_json::Value::Null);
            Ok(())
        }
    }

    #[test]
    fn capped_banner_survives_failed_requests_and_try_again_until_reconnect() {
        use crate::theme_snapshot::ThemeGuard;
        use gpui::{Modifiers, TestAppContext, VisualTestContext};
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let _theme = ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let events = store.read_with(&cx, |store, _| store.test_core.events.clone());
        let window = cx.add_window(|window, cx| {
            NativeRoot::new(
                "retry-notice".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            )
        });
        cx.run_until_parked();
        let attempts = Arc::new(AtomicUsize::new(0));
        store.update(&mut cx, |store, _| {
            store.client = runner_core::protocol::DaemonClient::new(Arc::new(RetryService {
                core: runner_daemon::daemon::InProcessTransport(store.test_core.clone()),
                down: AtomicBool::new(true),
                error: "runnerd connection closed",
                fail_once: AtomicBool::new(true),
                attempts: attempts.clone(),
            }));
            store.live_session_count = 6;
            store.stopped_session_count = 6;
        });
        events.emit(
            "daemon/disconnected",
            &serde_json::json!({"restart_limit":true}),
        );
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        store.read_with(&cx, |store, _| {
            assert_eq!(store.live_session_count, 0);
            assert_eq!(store.stopped_session_count, 6);
        });
        let notice = visual.debug_bounds("DAEMON_CRASH_BANNER").unwrap();
        visual.simulate_click(
            point(notice.left() + px(2.), notice.top() + px(2.)),
            Modifiers::default(),
        );
        cx.run_until_parked();
        assert!(visual.debug_bounds("GLOBAL_TOAST").is_none());
        window
            .update(&mut cx, |root, _, cx| {
                let error = root.core(cx).role_list().unwrap_err().to_string();
                root.error = Some(error.clone());
                root.chat_error = Some(error);
                root.show_toast("Saved", ToastTone::Success, cx);
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        window
            .read_with(&cx, |root, _| assert!(root.toasts.active().is_none()))
            .unwrap();
        for attempt in 1..=2 {
            assert!(visual.debug_bounds("DAEMON_CRASH_BANNER").is_some());
            assert!(visual.debug_bounds("DAEMON_CRASH_MESSAGE").is_some());
            assert!(visual.debug_bounds("DAEMON_OPEN_LOG").is_some());
            assert!(visual.debug_bounds("GLOBAL_TOAST").is_none());
            let retry = visual.debug_bounds("DAEMON_TRY_AGAIN").unwrap();
            visual.simulate_click(retry.center(), Modifiers::default());
            cx.run_until_parked();
            assert_eq!(attempts.load(Ordering::Acquire), attempt);
        }
        assert!(visual.debug_bounds("DAEMON_CRASH_BANNER").is_none());
        store.read_with(&cx, |store, _| {
            assert!(!store.daemon_disconnected && store.daemon_notice.is_none())
        });
        window
            .read_with(&cx, |root, _| {
                assert!(root.chat_error.is_none() && !root.daemon_retrying)
            })
            .unwrap();
    }

    #[test]
    fn failed_automatic_restart_offers_persistent_banner_until_reconnect() {
        use crate::theme_snapshot::ThemeGuard;
        use gpui::{Modifiers, TestAppContext, VisualTestContext};
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let _theme = ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let events = store.read_with(&cx, |store, _| store.test_core.events.clone());
        let window = cx.add_window(|window, cx| {
            NativeRoot::new(
                "failed-automatic-restart".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            )
        });
        cx.run_until_parked();
        let attempts = Arc::new(AtomicUsize::new(0));
        store.update(&mut cx, |store, _| {
            store.client = runner_core::protocol::DaemonClient::new(Arc::new(RetryService {
                core: runner_daemon::daemon::InProcessTransport(store.test_core.clone()),
                down: AtomicBool::new(true),
                error: "runnerd connection closed",
                fail_once: AtomicBool::new(true),
                attempts: attempts.clone(),
            }));
        });
        events.emit("daemon/disconnected", &serde_json::Value::Null);
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        assert!(visual.debug_bounds("DAEMON_CRASH_BANNER").is_none());
        events.emit(
            "daemon/disconnected",
            &serde_json::json!({"restart_limit":true}),
        );
        cx.run_until_parked();
        store.update(&mut cx, |store, cx| {
            assert!(store.client.role_list().is_err());
            store.refresh_sessions(cx);
        });
        cx.run_until_parked();
        for attempt in 1..=2 {
            assert!(visual.debug_bounds("DAEMON_CRASH_BANNER").is_some());
            assert!(visual.debug_bounds("DAEMON_OPEN_LOG").is_some());
            assert!(visual.debug_bounds("GLOBAL_TOAST").is_none());
            let retry = visual.debug_bounds("DAEMON_TRY_AGAIN").unwrap();
            visual.simulate_click(retry.center(), Modifiers::default());
            cx.run_until_parked();
            assert_eq!(attempts.load(Ordering::Acquire), attempt);
        }
        assert!(visual.debug_bounds("DAEMON_CRASH_BANNER").is_none());
        assert!(visual.debug_bounds("GLOBAL_TOAST").is_none());
        store.read_with(&cx, |store, _| {
            assert!(!store.daemon_disconnected);
            assert!(store.daemon_notice.is_none() && store.error.is_none());
        });
    }

    #[test]
    fn capped_service_has_one_notice_on_cached_and_unloaded_entity_lists() {
        use crate::theme_snapshot::ThemeGuard;
        use gpui::{Modifiers, TestAppContext, VisualTestContext};
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let _theme = ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let events = store.read_with(&cx, |store, _| store.test_core.events.clone());
        let mut windows = Vec::new();
        let attempts = Arc::new(AtomicUsize::new(0));
        for cached in [true, false] {
            if !cached {
                store.update(&mut cx, |store, _| {
                    store.client =
                        runner_core::protocol::DaemonClient::new(Arc::new(RetryService {
                            core: runner_daemon::daemon::InProcessTransport(
                                store.test_core.clone(),
                            ),
                            down: AtomicBool::new(true),
                            error: runner_core::protocol::managed::RESTART_LIMIT_NOTICE,
                            fail_once: AtomicBool::new(true),
                            attempts: attempts.clone(),
                        }));
                });
                events.emit(
                    "daemon/disconnected",
                    &serde_json::json!({"restart_limit":true}),
                );
                cx.run_until_parked();
            }
            for path in ["/roles", "/crews"] {
                windows.push(cx.add_window(|window, cx| {
                    NativeRoot::new(
                        format!("list-notice-{cached}-{path}").replace('/', ""),
                        temp.path().join("logs"),
                        Some(path.into()),
                        None,
                        store.clone(),
                        window,
                        cx,
                    )
                }));
            }
            cx.run_until_parked();
        }
        for window in &windows {
            window
                .update(&mut cx, |root, _, cx| {
                    if root.route == AppRoute::Roles {
                        root.load_role_page(cx);
                    } else {
                        root.load_crew_page(cx);
                    }
                })
                .unwrap();
        }
        cx.run_until_parked();
        let mut visuals = windows
            .iter()
            .map(|window| VisualTestContext::from_window((*window).into(), &cx))
            .collect::<Vec<_>>();
        for attempt in 1..=2 {
            for visual in &mut visuals {
                assert!(visual.debug_bounds("DAEMON_CRASH_BANNER").is_some());
                assert!(visual.debug_bounds("DAEMON_OPEN_LOG").is_some());
                assert!(visual.debug_bounds("PAGINATED_LIST_ERROR").is_none());
                assert!(visual.debug_bounds("PAGINATED_LIST_LOAD_ERROR").is_none());
                assert!(visual.debug_bounds("GLOBAL_TOAST").is_none());
            }
            let retry = visuals[0].debug_bounds("DAEMON_TRY_AGAIN").unwrap();
            visuals[0].simulate_click(retry.center(), Modifiers::default());
            cx.run_until_parked();
            assert_eq!(attempts.load(Ordering::Acquire), attempt);
        }
        for visual in &mut visuals {
            assert!(visual.debug_bounds("DAEMON_CRASH_BANNER").is_none());
            assert!(visual.debug_bounds("PAGINATED_LIST_ERROR").is_none());
            assert!(visual.debug_bounds("PAGINATED_LIST_LOAD_ERROR").is_none());
        }
        store.update(&mut cx, |store, _| {
            store.client = runner_core::protocol::DaemonClient::new(Arc::new(RetryService {
                core: runner_daemon::daemon::InProcessTransport(store.test_core.clone()),
                down: AtomicBool::new(true),
                error: "Example page error",
                fail_once: AtomicBool::new(false),
                attempts,
            }));
        });
        for window in &windows {
            window
                .update(&mut cx, |root, _, cx| {
                    if root.route == AppRoute::Roles {
                        root.load_role_page(cx);
                    } else {
                        root.load_crew_page(cx);
                    }
                })
                .unwrap();
        }
        cx.run_until_parked();
        for visual in &mut visuals {
            let banner = visual.debug_bounds("PAGINATED_LIST_ERROR").unwrap();
            let content = visual.debug_bounds("APP_CONTENT_COLUMN").unwrap();
            assert_eq!(banner.size.height, px(41.));
            assert_eq!(banner.left(), content.left() + px(1.));
            assert_eq!(banner.right(), content.right() - px(1.));
            assert_eq!(banner.top(), content.top() + px(45.));
            assert!(visual.debug_bounds("DAEMON_CRASH_BANNER").is_none());
        }
    }

    #[test]
    fn capped_banner_stays_in_each_windows_main_content() {
        use crate::theme_snapshot::ThemeGuard;
        use gpui::{size, TestAppContext, VisualTestContext};
        let _theme = ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, cx| {
            crate::app_store::seed_mixed_working_sessions(store);
            let conn = store.test_core.db.get().unwrap();
            runner_daemon::repo::node::create_tab(
                &conn,
                None,
                "",
                0,
                &runner_app::pane_layout::PaneLayout::single(
                    Some("direct-agent"),
                    &["direct-agent".into()],
                )
                .serialize()
                .unwrap(),
            )
            .unwrap();
            let nodes = runner_daemon::repo::node::list(&conn).unwrap();
            store.replace_nodes(nodes, cx);
        });
        let events = store.read_with(&cx, |store, _| store.test_core.events.clone());
        let chat = cx.add_window(|window, cx| {
            let mut root = NativeRoot::new(
                "main".into(),
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
        let other = cx.add_window(|window, cx| {
            let mut root = NativeRoot::new(
                "banner-other".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.route = AppRoute::Roles;
            root
        });
        let mut chat_visual = VisualTestContext::from_window(chat.into(), &cx);
        let mut other_visual = VisualTestContext::from_window(other.into(), &cx);
        chat_visual.simulate_resize(size(px(1440.), px(900.)));
        other_visual.simulate_resize(size(px(1440.), px(900.)));
        events.emit(
            "daemon/disconnected",
            &serde_json::json!({"restart_limit":true}),
        );
        cx.run_until_parked();
        let banner = chat_visual.debug_bounds("DAEMON_CRASH_BANNER").unwrap();
        let header = chat_visual.debug_bounds("CHAT_TAB_HEADER").unwrap();
        let sidebar = chat_visual.debug_bounds("APP_SIDEBAR").unwrap();
        let panel = chat_visual.debug_bounds("CHAT_SIDE_PANEL").unwrap();
        assert_eq!(banner.top(), header.bottom());
        assert_eq!(banner.left(), sidebar.right() + px(1.));
        assert_eq!(banner.right(), panel.left());
        assert_eq!(banner.size.height, px(41.));
        let text = chat_visual.debug_bounds("DAEMON_CRASH_MESSAGE").unwrap();
        let log = chat_visual.debug_bounds("DAEMON_OPEN_LOG").unwrap();
        let retry = chat_visual.debug_bounds("DAEMON_TRY_AGAIN").unwrap();
        assert!(text.size.height < banner.size.height);
        assert!(text.right() <= log.left() && log.right() <= retry.left());
        assert!(retry.right() <= banner.right());
        assert!(chat_visual.debug_bounds("NOTICE_BANNER_ICON").is_some());
        assert!(chat_visual.debug_bounds("GLOBAL_TOAST").is_none());
        let content = other_visual.debug_bounds("APP_CONTENT_COLUMN").unwrap();
        let other_banner = other_visual.debug_bounds("DAEMON_CRASH_BANNER").unwrap();
        assert_eq!(other_banner.left(), content.left() + px(1.));
        assert_eq!(other_banner.right(), content.right() - px(1.));
        for route in [
            AppRoute::Mission("test-mission".into()),
            AppRoute::Settings,
            AppRoute::ArchivedChat,
        ] {
            other
                .update(&mut cx, |root, _, cx| {
                    root.route = route.clone();
                    cx.notify();
                })
                .unwrap();
            cx.run_until_parked();
            let banner = other_visual.debug_bounds("DAEMON_CRASH_BANNER").unwrap();
            if matches!(route, AppRoute::Mission(_)) {
                let center = other_visual.debug_bounds("MISSION_CONTENT_COLUMN").unwrap();
                let header = other_visual.debug_bounds("MISSION_HEADER").unwrap();
                assert_eq!(banner.top(), header.bottom());
                assert_eq!(banner.left(), center.left());
                assert_eq!(banner.right(), center.right());
            } else {
                assert_eq!(banner.left(), content.left() + px(1.));
                assert_eq!(banner.right(), content.right() - px(1.));
                if route == AppRoute::Settings {
                    let track = other_visual
                        .debug_bounds("SETTINGS_CONTENT_SCROLLBAR_TRACK")
                        .unwrap();
                    assert!(track.top() >= banner.bottom());
                    assert!(track.bottom() <= px(900.));
                }
            }
        }
        events.emit("daemon/reconnected", &serde_json::Value::Null);
        cx.run_until_parked();
        assert!(chat_visual.debug_bounds("DAEMON_CRASH_BANNER").is_none());
        assert!(other_visual.debug_bounds("DAEMON_CRASH_BANNER").is_none());
    }

    #[test]
    fn automatic_crash_recovery_shows_a_short_error_toast_without_actions() {
        use crate::theme_snapshot::ThemeGuard;
        use gpui::{TestAppContext, VisualTestContext};
        let _theme = ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let events = store.read_with(&cx, |store, _| store.test_core.events.clone());
        let window = cx.add_window(|window, cx| {
            NativeRoot::new(
                "crash-toast".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            )
        });
        cx.run_until_parked();
        store.update(&mut cx, |store, _| {
            store.live_session_count = 6;
            store.stopped_session_count = 6;
        });
        events.emit("daemon/disconnected", &serde_json::Value::Null);
        events.emit("daemon/reconnected", &serde_json::Value::Null);
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        assert!(visual.debug_bounds("GLOBAL_TOAST").is_some());
        assert!(
            visual
                .debug_bounds("GLOBAL_TOAST_MESSAGE")
                .unwrap()
                .size
                .height
                <= px(22.)
        );
        assert!(visual.debug_bounds("DAEMON_CRASH_BANNER").is_none());
        assert!(visual.debug_bounds("DAEMON_OPEN_LOG").is_none());
        assert!(visual.debug_bounds("DAEMON_TRY_AGAIN").is_none());
        window
            .read_with(&cx, |root, _| {
                let toast = root.toasts.active().unwrap();
                assert_eq!(
                    toast.message,
                    "Background service restarted · 6 sessions stopped"
                );
                assert_eq!(toast.tone, ToastTone::Error);
                assert_eq!(
                    toast.duration_ms,
                    Some(crate::toast::DEFAULT_TOAST_DURATION_MS)
                );
            })
            .unwrap();
    }

    fn full_weekly_usage_snapshot() -> UsageSnapshot {
        UsageSnapshot {
            runtimes: [Runtime::Codex, Runtime::ClaudeCode, Runtime::Antigravity]
                .into_iter()
                .map(|runtime| {
                    (
                        runtime,
                        runner_core::protocol::usage::RuntimeUsage {
                            value: Some(AgentUsage {
                                windows: vec![UsageWindow {
                                    name: crate::runtime_ui::runtime_ui(runtime).usage_week.into(),
                                    used_percent: 100.,
                                    resets_at: None,
                                }],
                                updated_at: chrono::Utc::now(),
                            }),
                            ..Default::default()
                        },
                    )
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn sidebar_footer_fits_three_weekly_values_and_settings_in_one_row() {
        use crate::theme_snapshot::ThemeGuard;
        use gpui::{size, TestAppContext, VisualTestContext};

        let _theme = ThemeGuard::new();
        for variant in [
            theme::ThemeVariant::Carbon,
            theme::ThemeVariant::RunnerLight,
        ] {
            for width in [
                app_settings::SIDEBAR_MIN,
                app_settings::SIDEBAR_DEFAULT,
                app_settings::SIDEBAR_MAX,
            ] {
                for zoom in [0.6, 1., 1.5, 2.] {
                    let temp = tempfile::tempdir().unwrap();
                    let mut cx = TestAppContext::single();
                    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
                    store.update(&mut cx, |store, _| {
                        store.settings.app_theme = if variant.is_light() {
                            theme::ThemeIntent::Light
                        } else {
                            theme::ThemeIntent::Dark
                        };
                        store.settings.light_app_theme = theme::LightTheme::RunnerLight;
                        store.settings.dark_app_theme = theme::DarkTheme::Runner;
                        store.settings.sidebar_width = width;
                        store.settings.app_zoom = zoom;
                        store.usage = full_weekly_usage_snapshot();
                    });
                    let host = cx.add_window(|window, cx| {
                        let mut root = NativeRoot::new(
                            "sidebar-footer".into(),
                            temp.path().join("logs"),
                            None,
                            None,
                            store.clone(),
                            window,
                            cx,
                        );
                        root.usage_installed =
                            vec![Runtime::Codex, Runtime::ClaudeCode, Runtime::Antigravity];
                        root
                    });
                    let mut visual = VisualTestContext::from_window(host.into(), &cx);
                    visual.simulate_resize(size(px(1200.), px(900.)));
                    visual.run_until_parked();
                    assert_eq!(theme::active_variant(), variant);

                    let footer = visual.debug_bounds("SIDEBAR_FOOTER").unwrap();
                    let usage = visual.debug_bounds("SIDEBAR_USAGE").unwrap();
                    let gear = visual.debug_bounds("SIDEBAR_SETTINGS").unwrap();
                    let row = visual.debug_bounds("SIDEBAR_FOOTER_ROW").unwrap();
                    assert!((usage.size.height - px(32. * zoom)).abs() <= px(1.));
                    assert!((gear.size.width - px(32. * zoom)).abs() <= px(1.));
                    assert_eq!(usage.size.height, gear.size.height);
                    assert_eq!(usage.top(), gear.top());
                    assert!(usage.top() >= row.top() && usage.bottom() <= row.bottom());
                    assert!(gear.top() >= row.top() && gear.bottom() <= row.bottom());
                    assert!(usage.left() >= footer.left());
                    assert!(usage.right() <= gear.left());
                    assert!(gear.right() <= footer.right());
                    assert!(usage.size.width <= px(176. * zoom + 1.));
                    if width >= app_settings::SIDEBAR_DEFAULT {
                        assert!(gear.left() - usage.right() >= px(8. * zoom - 1.));
                    }
                    let entries = [
                        "SIDEBAR_USAGE_ENTRY_codex",
                        "SIDEBAR_USAGE_ENTRY_claude-code",
                        "SIDEBAR_USAGE_ENTRY_antigravity",
                    ]
                    .map(|id| visual.debug_bounds(id).unwrap());
                    assert_eq!(entries[0].left() - usage.left(), px(10. * zoom));
                    for pair in entries.windows(2) {
                        assert!(
                            pair[0].right() <= pair[1].left(),
                            "{width} {zoom}: {pair:?}"
                        );
                    }
                    let last = visual
                        .debug_bounds("SIDEBAR_USAGE_VALUE_antigravity")
                        .unwrap();
                    assert!(
                        last.right() <= usage.right() - px(10. * zoom),
                        "{width} {zoom}: {usage:?} {last:?}"
                    );
                    visual.simulate_mouse_move(usage.center(), None, gpui::Modifiers::default());
                    visual.run_until_parked();
                    assert_eq!(visual.debug_bounds("SIDEBAR_USAGE"), Some(usage));
                    assert_eq!(visual.debug_bounds("SIDEBAR_SETTINGS"), Some(gear));
                    visual.simulate_mouse_move(gear.center(), None, gpui::Modifiers::default());
                    visual.run_until_parked();
                    assert_eq!(visual.debug_bounds("SIDEBAR_USAGE"), Some(usage));
                    assert_eq!(visual.debug_bounds("SIDEBAR_SETTINGS"), Some(gear));
                }
            }
        }
    }

    #[test]
    fn sidebar_footer_opens_usage_and_settings_and_hides_usage_in_collapsed_preview() {
        use crate::theme_snapshot::ThemeGuard;
        use gpui::{size, Modifiers, TestAppContext, VisualTestContext};

        let _theme = ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let host = cx.add_window(|window, cx| {
            let mut root = NativeRoot::new(
                "sidebar-footer-actions".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.usage_installed = vec![Runtime::Codex];
            root
        });
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(1200.), px(900.)));
        visual.run_until_parked();
        let usage = visual.debug_bounds("SIDEBAR_USAGE").unwrap();
        visual.simulate_click(usage.center(), Modifiers::default());
        visual.run_until_parked();
        assert!(host.read_with(&visual, |root, _| root.usage_open).unwrap());
        assert!(visual.debug_bounds("USAGE_POPOVER_PANEL").is_some());
        visual.simulate_click(usage.center(), Modifiers::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("USAGE_POPOVER_PANEL").is_none());
        let gear = visual.debug_bounds("SIDEBAR_SETTINGS").unwrap();
        visual.simulate_click(gear.center(), Modifiers::default());
        visual.run_until_parked();
        assert_eq!(
            host.read_with(&visual, |root, _| root.route.clone())
                .unwrap(),
            AppRoute::Settings
        );
        host.update(&mut visual, |root, _, cx| {
            root.route = AppRoute::Roles;
            root.sidebar_collapsed = true;
            root.sidebar_preview_open = true;
            cx.notify();
        })
        .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("SIDEBAR_USAGE").is_none());
        assert!(visual.debug_bounds("SIDEBAR_SETTINGS").is_some());
        host.update(&mut visual, |root, _, cx| {
            root.sidebar_collapsed = false;
            root.sidebar_preview_open = false;
            root.usage_installed.clear();
            cx.notify();
        })
        .unwrap();
        visual.run_until_parked();
        assert!(visual.debug_bounds("SIDEBAR_USAGE").is_none());
        assert!(visual.debug_bounds("SIDEBAR_SETTINGS").is_some());
    }

    struct SidebarHeaderProbe {
        root: Entity<NativeRoot>,
        width: f32,
        zoom: f32,
        available: bool,
    }

    impl Render for SidebarHeaderProbe {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            window.set_rem_size(px(16. * self.zoom));
            div()
                .w(px(self.width * self.zoom))
                .child(self.root.update(cx, |root, cx| {
                    root.render_sidebar_brand(self.available.then(|| "0.99.0".into()), cx)
                }))
        }
    }

    #[test]
    fn available_app_update_sits_right_after_runner_in_the_sidebar_header() {
        use crate::theme_snapshot::ThemeGuard;
        use gpui::{size, TestAppContext, VisualTestContext};

        let _theme = ThemeGuard::new();
        for width in [app_settings::SIDEBAR_MIN, app_settings::SIDEBAR_DEFAULT] {
            for zoom in [1., 1.5] {
                for available in [false, true] {
                    let temp = tempfile::tempdir().unwrap();
                    let mut cx = TestAppContext::single();
                    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
                    store.update(&mut cx, |store, _| store.settings.app_zoom = zoom);
                    let host = cx.add_window(|window, cx| SidebarHeaderProbe {
                        root: cx.new(|cx| {
                            NativeRoot::new(
                                "sidebar-update".into(),
                                temp.path().join("logs"),
                                None,
                                None,
                                store.clone(),
                                window,
                                cx,
                            )
                        }),
                        width,
                        zoom,
                        available,
                    });
                    let mut visual = VisualTestContext::from_window(host.into(), &cx);
                    visual.simulate_resize(size(px(800.), px(300.)));
                    visual.run_until_parked();
                    let mark = visual.debug_bounds("SIDEBAR_BRAND_MARK").unwrap();
                    assert_eq!(mark.size, size(px(20. * zoom), px(20. * zoom)));
                    let update = visual.debug_bounds("SIDEBAR_UPDATE");
                    assert_eq!(update.is_some(), available);
                    if let Some(update) = update {
                        let header = visual.debug_bounds("SIDEBAR_HEADER").unwrap();
                        let name = visual.debug_bounds("SIDEBAR_NAME").unwrap();
                        assert_eq!(update.left() - name.right(), px(8. * zoom));
                        assert!(update.top() >= header.top());
                        assert!(update.bottom() <= header.bottom());
                        assert!(update.right() <= header.right());
                    }
                }
            }
        }
    }

    struct UsagePopoverProbe {
        zoom: f32,
    }

    impl Render for UsagePopoverProbe {
        fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            window.set_rem_size(px(16. * self.zoom));
            let now = chrono::Utc::now();
            let sections = [
                (Runtime::ClaudeCode, 2),
                (Runtime::Codex, 2),
                (Runtime::Antigravity, 4),
            ]
            .into_iter()
            .map(|(runtime, count)| {
                let usage = AgentUsage {
                    windows: (0..count)
                        .map(|index| UsageWindow {
                            name: if runtime == Runtime::Antigravity {
                                [
                                    "Gemini Models · 5 hours used",
                                    "Gemini Models · Week used",
                                    "Claude and GPT models · 5 hours used",
                                    "Claude and GPT models · Week used",
                                ][index]
                                    .to_owned()
                            } else {
                                format!("Window {index}")
                            },
                            used_percent: 25.,
                            resets_at: Some(now),
                        })
                        .collect(),
                    updated_at: now,
                };
                usage_section(runtime, Some(&usage), None, false, now)
            })
            .collect();
            div().w(px(340. * self.zoom)).child(usage_popover_panel(
                window.bounds().size.height - px(16.),
                div()
                    .h(px(24.))
                    .debug_selector(|| "USAGE_POPOVER_HEADER".into())
                    .child("Usage")
                    .into_any_element(),
                sections,
            ))
        }
    }

    #[test]
    fn usage_popover_scrolls_all_provider_rows_in_short_window() {
        use crate::theme_snapshot::ThemeGuard;
        use gpui::{size, ScrollDelta, ScrollWheelEvent, TestAppContext, VisualTestContext};

        let _theme = ThemeGuard::new();
        theme::set_active_variant(theme::ThemeVariant::Carbon);
        for zoom in [1., 1.5] {
            let mut cx = TestAppContext::single();
            let host = cx.add_window(move |_, _| UsagePopoverProbe { zoom });
            let mut visual = VisualTestContext::from_window(host.into(), &cx);
            visual.simulate_resize(size(px(800.), px(600.)));
            visual.run_until_parked();

            if zoom == 1. {
                let panel = visual.debug_bounds("USAGE_POPOVER_PANEL").unwrap();
                assert!(panel.size.height < px(500.), "{panel:?}");
            }
            visual.simulate_resize(size(px(800.), px(300.)));
            visual.run_until_parked();

            let panel = visual.debug_bounds("USAGE_POPOVER_PANEL").unwrap();
            let body = visual.debug_bounds("USAGE_POPOVER_BODY").unwrap();
            let header = visual.debug_bounds("USAGE_POPOVER_HEADER").unwrap();
            let antigravity = visual.debug_bounds("USAGE_SECTION_antigravity").unwrap();
            assert!(panel.size.height <= px(284.), "{zoom}: {panel:?}");
            assert!(header.bottom() <= body.top(), "{zoom}: {header:?} {body:?}");
            assert!(
                antigravity.bottom() > body.bottom(),
                "{zoom}: {antigravity:?} {body:?}"
            );

            visual.simulate_event(ScrollWheelEvent {
                position: body.center(),
                delta: ScrollDelta::Lines(point(0., -100.)),
                ..Default::default()
            });
            visual.run_until_parked();
            let scrolled = visual.debug_bounds("USAGE_SECTION_antigravity").unwrap();
            assert!(
                scrolled.top() < antigravity.top(),
                "{zoom}: {antigravity:?} {scrolled:?}"
            );
            assert!(
                scrolled.bottom() <= body.bottom(),
                "{zoom}: {scrolled:?} {body:?}"
            );
        }
    }

    #[test]
    fn update_dot_counts_only_enabled_agents() {
        let mut settings = app_settings::AppSettings::default();
        assert!(!update_dot_visible(&[], &settings));
        assert!(update_dot_visible(&[Runtime::Codex], &settings));
        settings.disabled_agents.insert(Runtime::Codex.key().into());
        assert!(!update_dot_visible(&[Runtime::Codex], &settings));
        assert!(update_dot_visible(
            &[Runtime::Codex, Runtime::ClaudeCode],
            &settings
        ));
        let copilot_default =
            runner_core::protocol::runtime_metadata::runtime_default_enabled(Runtime::Copilot);
        assert_eq!(
            update_dot_visible(&[Runtime::Copilot], &settings),
            copilot_default
        );
        settings
            .enabled_agents
            .insert(Runtime::Copilot.key().into());
        assert!(update_dot_visible(&[Runtime::Copilot], &settings));
    }

    #[test]
    fn usage_thresholds_and_reset_labels() {
        use chrono::TimeDelta;
        assert_eq!(usage_tone([79.9]), UsageTone::Normal);
        assert_eq!(usage_tone([5., 80.]), UsageTone::Warning);
        assert_eq!(usage_tone([80., 100.]), UsageTone::Danger);
        let now = chrono::Utc::now();
        assert_eq!(reset_label(None, now), "reset unknown");
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
    fn sidebar_usage_uses_weekly_windows_and_gemini_group() {
        let snapshot = UsageSnapshot {
            runtimes: [
                (
                    Runtime::ClaudeCode,
                    runner_core::protocol::usage::RuntimeUsage {
                        value: Some(AgentUsage {
                            windows: vec![
                                UsageWindow {
                                    name: "5 hours".into(),
                                    used_percent: 91.,
                                    resets_at: None,
                                },
                                UsageWindow {
                                    name: "Week".into(),
                                    used_percent: 26.,
                                    resets_at: None,
                                },
                            ],
                            updated_at: chrono::Utc::now(),
                        }),
                        ..Default::default()
                    },
                ),
                (
                    Runtime::Antigravity,
                    runner_core::protocol::usage::RuntimeUsage {
                        value: Some(AgentUsage {
                            windows: vec![
                                UsageWindow {
                                    name: "Gemini Models · 5 hours used".into(),
                                    used_percent: 90.,
                                    resets_at: None,
                                },
                                UsageWindow {
                                    name: "Gemini Models · Week used".into(),
                                    used_percent: 1.,
                                    resets_at: None,
                                },
                                UsageWindow {
                                    name: "Claude and GPT models · Week used".into(),
                                    used_percent: 70.,
                                    resets_at: None,
                                },
                            ],
                            updated_at: chrono::Utc::now(),
                        }),
                        ..Default::default()
                    },
                ),
            ]
            .into_iter()
            .collect(),
            ..UsageSnapshot::default()
        };
        assert_eq!(
            weekly_usage_percent(Runtime::ClaudeCode, snapshot.get(Runtime::ClaudeCode)),
            Some(26.)
        );
        assert_eq!(
            weekly_usage_percent(Runtime::Antigravity, snapshot.get(Runtime::Antigravity)),
            Some(1.)
        );
        assert_eq!(weekly_usage_percent(Runtime::Codex, None), None);
        assert_eq!(unavailable_line(None, Runtime::Codex), "Checking…");
    }

    #[test]
    fn usage_runtimes_are_installed_enabled_and_usage_capable_in_runtime_order() {
        let all = [Runtime::Antigravity, Runtime::ClaudeCode, Runtime::Codex];
        // An installed, enabled runtime shows without ever having run a session.
        assert_eq!(
            visible_usage_runtimes(&all, &all),
            [Runtime::Codex, Runtime::ClaudeCode, Runtime::Antigravity]
        );
        assert_eq!(
            visible_usage_runtimes(&[Runtime::Antigravity], &all),
            [Runtime::Antigravity]
        );
        // Uninstalled and disabled runtimes do not.
        assert_eq!(
            visible_usage_runtimes(&[Runtime::Codex, Runtime::Antigravity], &all),
            [Runtime::Codex, Runtime::Antigravity]
        );
        assert_eq!(
            visible_usage_runtimes(&all, &[Runtime::ClaudeCode, Runtime::Codex]),
            [Runtime::Codex, Runtime::ClaudeCode]
        );
        // A runtime without usage never shows, even if installed and enabled.
        let with_copilot = [Runtime::Copilot, Runtime::Pi, Runtime::Codex];
        assert_eq!(
            visible_usage_runtimes(&with_copilot, &with_copilot),
            [Runtime::Codex]
        );
        assert!(visible_usage_runtimes(&[], &all).is_empty());
        assert!(visible_usage_runtimes(&all, &[]).is_empty());
        assert!(visible_usage_runtimes(&[Runtime::Copilot], &[Runtime::Copilot]).is_empty());
    }

    #[test]
    fn usage_popover_keeps_provider_order_for_installed_and_enabled_subsets() {
        let order = [Runtime::ClaudeCode, Runtime::Codex, Runtime::Antigravity];
        assert_eq!(usage_popover_runtimes(&Runtime::ALL, &Runtime::ALL), order);
        for installed_mask in 0..8 {
            for enabled_mask in 0..8 {
                let subset = |mask| {
                    order
                        .into_iter()
                        .enumerate()
                        .filter_map(|(index, runtime)| {
                            (mask & (1 << index) != 0).then_some(runtime)
                        })
                        .rev()
                        .collect::<Vec<_>>()
                };
                let installed = subset(installed_mask);
                let enabled = subset(enabled_mask);
                let expected: Vec<_> = order
                    .into_iter()
                    .filter(|runtime| installed.contains(runtime) && enabled.contains(runtime))
                    .collect();
                assert_eq!(
                    usage_popover_runtimes(&installed, &enabled),
                    expected,
                    "installed: {installed:?}, enabled: {enabled:?}"
                );
            }
        }
    }

    #[test]
    fn sidebar_resize_bar_rides_the_divider() {
        use crate::theme_snapshot::ThemeGuard;
        use gpui::{px, size, TestAppContext, VisualTestContext};
        use runner_daemon::{db, session, shell_path};
        use std::sync::{Arc, RwLock};

        let _theme = ThemeGuard::new();
        theme::set_active_variant(theme::ThemeVariant::Carbon);
        let temp = tempfile::tempdir().unwrap();
        let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
        let runtime_discovery =
            Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
        let core = crate::test_support::core(
            Arc::new(db::open_pool(&temp.path().join("runner.db")).unwrap()),
            temp.path().to_owned(),
            session::SessionManager::new(
                runtime_shell_env.clone(),
                runtime_discovery.clone(),
                Arc::new(session::pty_runtime::PtyRuntime::new()),
            ),
            runtime_shell_env,
            runtime_discovery,
        );
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
        let inside = sidebar.right() - bar.left();
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
    fn app_update_hint_shows_available_version() {
        let available = runner_app::updater::UpdateInfo::new("0.6.1");

        assert_eq!(app_update_hint_version(None), None);
        assert_eq!(app_update_hint_version(Some(&available)), Some("0.6.1"));
    }
}

#[test]
fn usage_ui_catalog_golden() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let reasons = [
        None,
        Some(UnavailableReason::SignIn),
        Some(UnavailableReason::KeychainDenied),
        Some(UnavailableReason::KeychainUnavailable),
        Some(UnavailableReason::ClaudeUnreachable),
        Some(UnavailableReason::CodexNoAnswer),
        Some(UnavailableReason::AntigravityNoAnswer),
        Some(UnavailableReason::InvalidResponse),
    ];
    let now = chrono::DateTime::from_timestamp(1770000000, 0).unwrap();
    let usage = AgentUsage {
        updated_at: now,
        windows: [
            "Week",
            "Gemini Models · Week used",
            "Claude and GPT models · 5 hours used",
        ]
        .into_iter()
        .map(|name| UsageWindow {
            name: name.into(),
            used_percent: 27.,
            resets_at: None,
        })
        .collect(),
    };
    let rows: Vec<_> = Runtime::ALL.into_iter().map(|runtime| {
        let sections = crate::catalog_golden::capture(|| {
            let _ = usage_section(runtime,Some(&usage),None,true,now);
            let _ = usage_section(runtime,None,None,true,now);
        });
        serde_json::json!({"runtime":runtime,"unavailable":reasons.map(|reason|unavailable_line(reason,runtime)),"sections":sections,"weekly_percent":weekly_usage_percent(runtime,Some(&usage))})
    }).collect();
    crate::catalog_golden::assert_golden(
        "usage-ui",
        serde_json::json!({"visible":visible_usage_runtimes(&Runtime::ALL,&Runtime::ALL),"runtimes":rows}),
    );
}
