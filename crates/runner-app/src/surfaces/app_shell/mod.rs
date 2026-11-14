mod actions;
mod notices;
mod mission_usage;
mod sidebar;
mod usage;
pub(crate) use sidebar::{sidebar_footer_contents, SidebarFooterControl};
mod usage_timeline;

pub(crate) use mission_usage::MissionUsageState;
pub(crate) use usage::*;

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

mod settings_button;

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
    Launcher,
    Roles,
    NewRole,
    RoleDetail(String),
    Crews,
    NewCrew,
    CrewEditor(String),
    Mission(String),
    Usage,
    MissionControl,
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

pub(crate) fn terminal_style_for(
    settings: &app_settings::AppSettings,
    cx: &App,
) -> crate::terminal::element::TerminalStyle {
    crate::terminal::element::TerminalStyle {
        palette: app_settings::terminal_palette(settings, theme::active_variant()),
        font: settings.terminal_font_family.font(cx),
        font_size: settings.terminal_font_size as f32 * settings.app_zoom,
        app_zoom: settings.app_zoom,
        line_height_factor: settings.terminal_line_height,
        copy_on_select: settings.terminal_copy_on_select,
    }
}

impl NativeRoot {
    pub(crate) fn render_app_shell(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.sync_theme(window, cx);
        runner_app::appearance::configure(self.settings(cx).window_material, cx);
        #[cfg(target_os = "linux")]
        let background = platform_ui::window_background(window.window_decorations());
        #[cfg(not(target_os = "linux"))]
        let background =
            runner_app::appearance::effective(self.settings(cx).window_material, cx).background();
        window.set_background_appearance(background);
        window.set_rem_size(px(16. * self.settings(cx).app_zoom));
        if let Some(error) = self.error.take() {
            self.show_toast(error, ToastTone::Error, cx);
        }

        let workspace = if self.remote_client.is_active() {
            self.render_remote_workspace(window, cx)
        } else {
            self.render_entity_surface(window, cx)
        };
        let (sidebar, sidebar_divider) = self.render_app_sidebar(window, cx);
        let sidebar_resize =
            sidebar_divider.map(|divider| self.render_sidebar_resize_handle(divider, cx));
        let preview_trigger = self.render_sidebar_preview_trigger(cx);
        let modal = self
            .start_chat_modal
            .is_some()
            .then(|| self.render_start_chat_modal(window, cx));
        let terminal_close_confirm = (self.remote_client.is_active()
            || matches!(self.route, AppRoute::Chat | AppRoute::Mission(_)))
        .then_some(self.terminal_close_confirm.as_ref())
        .flatten()
        .map(|_| self.render_terminal_close_confirm(cx));
        let fork_confirm = (self.remote_client.is_active() || self.route == AppRoute::Chat)
            .then_some(self.fork_confirm.as_ref())
            .flatten()
            .map(|_| self.render_fork_confirm(cx));
        let sidebar_overlays = if self.route != AppRoute::Settings || self.remote_client.is_active()
        {
            self.render_sidebar_overlays(cx)
        } else {
            Vec::new()
        };
        let entity_overlays = match self.remote_client.mission_workspace() {
            Some(workspace) => {
                let fallback = (!self.create_modal_open()
                    && self.fork_confirm.is_none()
                    && self.terminal_close_confirm.is_none()
                    && self.remote_client.ui.pair.is_none()
                    && !self.remote_client.ui.picker_open)
                    .then(|| self.root_focus.clone());
                workspace.update(cx, |workspace, cx| {
                    workspace.restore_remote_dialog_focus(fallback.as_ref(), window, cx);
                    workspace.render_mission_overlays(cx)
                })
            }
            _ => {
                if self.route != AppRoute::Settings {
                    self.render_entity_overlays(window, cx)
                } else {
                    Vec::new()
                }
            }
        };
        let chrome = div()
            .debug_selector(|| "APP_CHROME".into())
            .relative()
            .size_full()
            .flex()
            .children(sidebar)
            .child(
                (if self.front_page_has_islands() {
                    runner_app::ui::work_card::island_area(self.front_page_sidebar_collapsed())
                } else {
                    runner_app::ui::work_card::work_card(self.front_page_sidebar_collapsed())
                })
                .map(|column| {
                    #[cfg(test)]
                    let column = crate::theme_snapshot::record_fill("APP_CONTENT_COLUMN", column);
                    column
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
                            | AppRoute::Launcher
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
            .children(terminal_close_confirm)
            .children(fork_confirm)
            .children(sidebar_overlays)
            .children(entity_overlays);
        let settings = (self.remote_client.is_local() && self.route == AppRoute::Settings)
            .then(|| self.render_settings_takeover(window, cx));
        let settings_confirm = self
            .remote_client
            .is_local()
            .then(|| self.settings_confirm_overlay(cx))
            .flatten()
            .map(|overlay| deferred(overlay).with_priority(100));
        let computer_pair = self
            .remote_client
            .render_computer_pair(cx)
            .map(|dialog| deferred(dialog).with_priority(110));
        let computer_picker = self
            .remote_client
            .render_computer_picker(window, cx)
            .map(|picker| deferred(picker).with_priority(80));
        let command_palette = Some(deferred(self.command_palette.clone()).with_priority(50));
        let toast_priority = if self.route == AppRoute::Settings && self.remote_pin_dialog_open() {
            101
        } else {
            3
        };
        let toast = self
            .render_toast(cx)
            .map(|toast| deferred(toast).with_priority(toast_priority));
        let modifier_sidebar = self.front_page_sidebar();
        let key_sidebar = self.front_page_sidebar();
        let usage_root = cx.entity();

        div()
            .relative()
            .key_context("Root")
            .size_full()
            .overflow_hidden()
            .track_focus(&self.root_focus)
            .font(crate::app_settings::app_font())
            .bg(theme::chrome())
            .map(|root| {
                #[cfg(target_os = "linux")]
                let root = platform_ui::round_window_corners(root, window);
                root
            })
            .text_color(theme::text())
            .children(
                (self.remote_client.is_active() || self.route != AppRoute::Settings)
                    .then_some(chrome),
            )
            .children(settings)
            .children(self.render_window_top_margin_drag_area(cx))
            .children(self.render_window_navigation(window, cx))
            .children(modal)
            .children(settings_confirm)
            .children(command_palette)
            .children(computer_picker)
            .children(computer_pair)
            .children(toast)
            .children(self.render_background_tasks(cx))
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
            .map(|root| self.decorate_window(root, window, cx))
            .on_modifiers_changed(move |event, window, cx| {
                modifier_sidebar.update(cx, |sidebar, sidebar_cx| {
                    sidebar.handle_shortcut_modifiers_changed(event.modifiers, window, sidebar_cx);
                });
            })
            .capture_key_down(move |event, window, cx| {
                if event.keystroke.key == "escape" {
                    usage_root.update(cx, |this, cx| {
                        if this.usage_open {
                            this.usage_open = false;
                            cx.stop_propagation();
                            cx.notify();
                        } else {
                            this.close_unfocused_update_popover(window, cx);
                        }
                    });
                }
                key_sidebar.update(cx, |sidebar, sidebar_cx| {
                    sidebar.handle_shortcut_key_pressed(sidebar_cx);
                });
            })
            .on_drag_move::<SidebarResizeDrag>(cx.listener(
                |this, event: &DragMoveEvent<SidebarResizeDrag>, _, cx| {
                    let width = (f32::from(event.event.position.x - event.bounds.left())
                        - this.sidebar_border_offset(cx))
                        / this.settings(cx).app_zoom;
                    let width = clamp_sidebar_width(width);
                    // A drag suppresses hover, so the bar needs the drag's own state to stay lit.
                    if !this.sidebar_resizing {
                        this.sidebar_resizing = true;
                        cx.notify();
                    }
                    if this.remote_client.is_active() {
                        this.remote_client.set_sidebar_width(width, cx);
                        return;
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
                if this.remote_client.is_local() {
                    this.save_settings(cx);
                }
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
            .map(|root| {
                root.on_action(cx.listener(Self::open_new_tab_modal))
                    .on_action(cx.listener(Self::new_terminal_action))
                    .on_action(cx.listener(Self::new_mission_action))
                    .on_action(cx.listener(Self::split_pane_right))
                    .on_action(cx.listener(Self::split_pane_down))
                    .on_action(cx.listener(Self::toggle_maximize_pane_action))
                    .on_action(cx.listener(Self::focus_previous_chat_pane))
                    .on_action(cx.listener(Self::focus_next_chat_pane))
                    .on_action(cx.listener(Self::stop_focused_session))
                    .on_action(cx.listener(Self::resume_focused_session))
                    .on_action(cx.listener(Self::toggle_terminal_drawer_action))
            })
            .on_action(cx.listener(Self::toggle_sidebar))
            .on_action(cx.listener(Self::switch_sidebar_view))
            .on_action(cx.listener(Self::open_command_palette))
            .on_action(cx.listener(Self::search_files))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::navigate_previous_page))
            .on_action(cx.listener(Self::navigate_next_page))
            .on_action(cx.listener(Self::zoom_in))
            .on_action(cx.listener(Self::zoom_out))
            .on_action(cx.listener(Self::zoom_reset))
            .on_action(cx.listener(Self::toggle_fullscreen))
            .map(|root| {
                root.on_action(cx.listener(Self::select_tab_1))
                    .on_action(cx.listener(Self::select_tab_2))
                    .on_action(cx.listener(Self::select_tab_3))
                    .on_action(cx.listener(Self::select_tab_4))
                    .on_action(cx.listener(Self::select_tab_5))
                    .on_action(cx.listener(Self::select_tab_6))
                    .on_action(cx.listener(Self::select_tab_7))
                    .on_action(cx.listener(Self::select_tab_8))
                    .on_action(cx.listener(Self::select_tab_9))
            })
            .into_any_element()
    }

    pub(crate) fn render_titlebar_drag_area(
        &self,
        id: &'static str,
        area: gpui::Div,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let area = area
            .id(id)
            .window_control_area(WindowControlArea::Drag)
            .capture_any_mouse_down(|event, _, _| platform_ui::capture_titlebar_press(event))
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
                cx.listener(|this, event, window, cx| {
                    this.titlebar_drag_armed = true;
                    platform_ui::titlebar_mouse_down(event, window, cx);
                }),
            );
        #[cfg(target_os = "linux")]
        let area = area
            .on_mouse_move(cx.listener(|this, _, window, _| {
                if this.titlebar_drag_armed {
                    this.titlebar_drag_armed = false;
                    window.start_window_move();
                }
            }))
            .on_click(|event, window, cx| {
                if event.click_count() == 2 && platform_ui::titlebar_double_click(2) {
                    cx.stop_propagation();
                    window.titlebar_double_click();
                }
            });
        area
    }
}

#[cfg(test)]
mod tests;

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
        credits: None,
        updated_at: now,
        windows: [
            "Week",
            "5 hours",
            "Opus · week",
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
