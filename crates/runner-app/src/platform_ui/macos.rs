use gpui::{svg, Div};

use crate::surfaces::app_shell::{alpha, TITLEBAR_DRAG_HEIGHT};
use crate::surfaces::app_shell::{SIDEBAR_TOGGLE_GLYPH_INSET, SIDEBAR_TOGGLE_GLYPH_X};
use crate::*;

const TITLEBAR_HEIGHT: f32 = 44.;

pub(crate) const PRIMARY_MODIFIER: &str = "⌘";
pub(crate) const SHORTCUT_REQUIREMENTS: &str =
    "Shortcuts must include ⌘, Control, or Option. Function keys can be used alone.";

pub(crate) fn primary_modifier_held(modifiers: gpui::Modifiers) -> bool {
    modifiers.platform
}

pub(crate) fn other_shortcut_modifiers_held(modifiers: gpui::Modifiers) -> bool {
    modifiers.control || modifiers.alt || modifiers.shift || modifiers.function
}

pub(crate) fn sidebar_row_wrapper(id: SharedString) -> gpui::Stateful<Div> {
    div().id(id).relative()
}

pub(crate) fn sidebar_section() -> Div {
    div().flex_none()
}

pub(crate) fn update_hint_tooltip(version: &str) -> String {
    format!("Runner {version} is ready to install")
}

pub(crate) fn activate_update_hint(updater: &Entity<Updater>, cx: &mut App) {
    updater.read(cx).check_for_updates();
}

pub(crate) fn finish_window_close(_window: &mut Window) -> bool {
    true
}

impl NativeRoot {
    pub(crate) fn decorate_window(
        &self,
        root: Div,
        _window: &Window,
        _cx: &mut Context<Self>,
    ) -> Div {
        root
    }

    pub(crate) fn render_main_titlebar_drag_area(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        Some(
            self.render_titlebar_drag_area(
                "main-titlebar-drag",
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(TITLEBAR_DRAG_HEIGHT * self.settings(cx).app_zoom)),
                cx,
            )
            .into_any_element(),
        )
    }

    pub(crate) fn render_sidebar_titlebar(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let fullscreen = window.is_fullscreen();
        let titlebar_padding = if fullscreen {
            8. * self.settings(cx).app_zoom
        } else {
            SIDEBAR_TOGGLE_GLYPH_X - SIDEBAR_TOGGLE_GLYPH_INSET * self.settings(cx).app_zoom
        };
        let panel_path = if self.sidebar_collapsed {
            "panel-left-hidden.svg"
        } else {
            "panel-left-open.svg"
        };
        let (previous_page, next_page) = self.render_page_navigation_buttons(cx);
        let titlebar = self.render_titlebar_drag_area(
            "sidebar-titlebar-drag",
            div()
                .flex_none()
                .h(px(TITLEBAR_HEIGHT * self.settings(cx).app_zoom))
                .pl(px(titlebar_padding))
                .pr_3()
                .flex()
                .items_center()
                .child(
                    Self::titlebar_control_cluster("sidebar-titlebar-controls")
                        .child(
                            div()
                                .id("sidebar-toggle")
                                .group("sidebar-toggle")
                                .flex_none()
                                .w(px(28. * self.settings(cx).app_zoom))
                                .h(px(28. * self.settings(cx).app_zoom))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_sm()
                                .cursor_pointer()
                                .text_color(theme::muted())
                                .hover(|button| {
                                    button
                                        .bg(alpha(theme::sidebar_selected(), 0.6))
                                        .text_color(theme::text())
                                })
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .child(
                                    svg()
                                        .path(panel_path)
                                        .w(px(14. * self.settings(cx).app_zoom))
                                        .h(px(14. * self.settings(cx).app_zoom))
                                        .flex_none()
                                        .text_color(theme::muted())
                                        .group_hover("sidebar-toggle", |icon| {
                                            icon.text_color(theme::text())
                                        }),
                                )
                                .on_click(cx.listener(|this, _, _, cx| {
                                    cx.stop_propagation();
                                    if this.sidebar_collapsed {
                                        this.set_sidebar_collapsed(false, true, cx);
                                        this.sidebar_preview_open = false;
                                        this.sidebar_preview_peeking = false;
                                    } else {
                                        this.set_sidebar_collapsed(true, true, cx);
                                        this.sidebar_preview_peeking = false;
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(previous_page)
                        .child(next_page),
                ),
            cx,
        );
        Some(titlebar.into_any_element())
    }

    /// Previous/Next page arrows in the Windows order and treatment, driven by
    /// the same per-window page history. Both disable in Settings and at the
    /// history boundaries. No tooltips on macOS, by decision.
    fn render_page_navigation_buttons(&self, cx: &mut Context<Self>) -> (IconButton, IconButton) {
        let in_settings = self.route == AppRoute::Settings;
        let can_go_back =
            !in_settings && self.runtime_navigation_index.is_some_and(|index| index > 0);
        let can_go_forward = !in_settings
            && self
                .runtime_navigation_index
                .is_some_and(|index| index + 1 < self.runtime_navigation_history.len());
        let back_root = cx.entity();
        let forward_root = cx.entity();
        (
            IconButton::new("window-previous-page", "chevron-left.svg")
                .disabled(!can_go_back)
                .on_press(move |window, cx| {
                    back_root.update(cx, |this, cx| this.navigate_runtime_page(-1, window, cx));
                }),
            IconButton::new("window-next-page", "chevron-right.svg")
                .disabled(!can_go_forward)
                .on_press(move |window, cx| {
                    forward_root.update(cx, |this, cx| this.navigate_runtime_page(1, window, cx));
                }),
        )
    }

    /// The leading titlebar cluster. A press anywhere in it, including on a
    /// disabled arrow, must neither arm a window drag nor count as a
    /// titlebar double-click.
    fn titlebar_control_cluster(id: &'static str) -> gpui::Stateful<Div> {
        div()
            .id(id)
            .flex_none()
            .flex()
            .items_center()
            .gap_1()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(|_, _, cx| cx.stop_propagation())
    }

    pub(crate) fn render_open_sidebar_button(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.sidebar_collapsed {
            return None;
        }
        let (previous_page, next_page) = self.render_page_navigation_buttons(cx);
        let open_sidebar = div()
            .id("open-sidebar")
            .group("open-sidebar")
            .flex_none()
            .w(px(28. * self.settings(cx).app_zoom))
            .h(px(28. * self.settings(cx).app_zoom))
            .flex()
            .items_center()
            .justify_center()
            .rounded_sm()
            .cursor_pointer()
            .text_color(theme::muted())
            .hover(|button| button.bg(theme::raised()).text_color(theme::text()))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                svg()
                    .path("panel-left-hidden.svg")
                    .w(px(14. * self.settings(cx).app_zoom))
                    .h(px(14. * self.settings(cx).app_zoom))
                    .flex_none()
                    .text_color(theme::muted())
                    .group_hover("open-sidebar", |icon| icon.text_color(theme::text())),
            )
            .on_click(cx.listener(|this, _, window, cx| {
                cx.stop_propagation();
                this.set_sidebar_collapsed(false, true, cx);
                this.sidebar_preview_open = false;
                this.sidebar_preview_peeking = false;
                this.focus_active_terminal(window, cx);
                cx.notify();
            }));
        Some(
            Self::titlebar_control_cluster("workspace-titlebar-controls")
                .child(open_sidebar)
                .child(previous_page)
                .child(next_page)
                .into_any_element(),
        )
    }
}
