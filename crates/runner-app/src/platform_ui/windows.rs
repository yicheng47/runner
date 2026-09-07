use gpui::{deferred, svg, Div, WindowControlArea};

use crate::surfaces::app_shell::alpha;
use crate::*;
use runner_app::ui::CAPTION_BUTTON_WIDTH;

const TITLEBAR_HEIGHT: f32 = 32.;

pub(crate) const PRIMARY_MODIFIER: &str = "Ctrl";
pub(crate) const SHORTCUT_REQUIREMENTS: &str =
    "Shortcuts must include Ctrl, Alt, or Win. Function keys can be used alone.";

pub(crate) fn primary_modifier_held(modifiers: gpui::Modifiers) -> bool {
    modifiers.control
}

pub(crate) fn other_shortcut_modifiers_held(modifiers: gpui::Modifiers) -> bool {
    modifiers.platform || modifiers.alt || modifiers.shift || modifiers.function
}

pub(crate) fn sidebar_row_wrapper(id: SharedString) -> gpui::Stateful<Div> {
    div().id(id).relative().w_full().flex().flex_col()
}

pub(crate) fn sidebar_section() -> Div {
    div().flex_none().w_full().flex().flex_col()
}

pub(crate) fn update_hint_tooltip(version: &str) -> String {
    format!("Runner {version} is available — open downloads")
}

pub(crate) fn activate_update_hint(_updater: &Entity<Updater>, cx: &mut App) {
    cx.open_url(runner_app::updater::windows_download_url());
}

pub(crate) fn finish_window_close(window: &mut Window) -> bool {
    // Let GPUI own HWND destruction instead of also running Windows' default WM_CLOSE handler.
    window.remove_window();
    false
}

impl NativeRoot {
    pub(crate) fn decorate_window(
        &self,
        root: Div,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Div {
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::bg())
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(TITLEBAR_HEIGHT * self.settings(cx).app_zoom))
                    .flex_none()
                    .child(deferred(self.render_windows_titlebar(window, cx)).with_priority(200)),
            )
            .child(div().relative().w_full().flex_1().min_h(px(0.)).child(root))
    }

    pub(crate) fn render_main_titlebar_drag_area(
        &self,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }

    pub(crate) fn render_sidebar_titlebar(
        &self,
        _window: &Window,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        None
    }

    pub(crate) fn render_open_sidebar_button(&self, _cx: &mut Context<Self>) -> Option<AnyElement> {
        None
    }

    fn render_windows_titlebar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let zoom = self.settings(cx).app_zoom;
        let sidebar_root = cx.entity();
        let back_root = cx.entity();
        let forward_root = cx.entity();
        let in_settings = self.route == AppRoute::Settings;
        let can_go_back =
            !in_settings && self.runtime_navigation_index.is_some_and(|index| index > 0);
        let can_go_forward = !in_settings
            && self
                .runtime_navigation_index
                .is_some_and(|index| index + 1 < self.runtime_navigation_history.len());
        let caption_width = if window.is_fullscreen() {
            0.
        } else {
            3. * CAPTION_BUTTON_WIDTH * zoom
        };
        div()
            .relative()
            .size_full()
            .flex()
            .items_center()
            .bg(theme::sidebar())
            .font(crate::app_settings::app_font())
            .child(
                div()
                    .occlude()
                    .flex_none()
                    .h_full()
                    .flex()
                    .items_center()
                    .pl_2()
                    .gap_1()
                    .child(
                        IconButton::new(
                            "sidebar-toggle",
                            if self.sidebar_collapsed {
                                "panel-left-hidden.svg"
                            } else {
                                "panel-left-open.svg"
                            },
                        )
                        .tooltip("Toggle sidebar")
                        .disabled(in_settings)
                        .on_press(move |_, cx| {
                            sidebar_root.update(cx, |this, cx| {
                                this.set_sidebar_collapsed(!this.sidebar_collapsed, true, cx);
                                this.sidebar_preview_open = false;
                                this.sidebar_preview_peeking = false;
                                cx.notify();
                            });
                        }),
                    )
                    .child(
                        IconButton::new("window-previous-page", "chevron-left.svg")
                            .tooltip("Previous page")
                            .disabled(!can_go_back)
                            .on_press(move |window, cx| {
                                back_root.update(cx, |this, cx| {
                                    this.navigate_runtime_page(-1, window, cx)
                                });
                            }),
                    )
                    .child(
                        IconButton::new("window-next-page", "chevron-right.svg")
                            .tooltip("Next page")
                            .disabled(!can_go_forward)
                            .on_press(move |window, cx| {
                                forward_root.update(cx, |this, cx| {
                                    this.navigate_runtime_page(1, window, cx)
                                });
                            }),
                    ),
            )
            .child(
                self.render_titlebar_drag_area(
                    "windows-titlebar-drag",
                    div()
                        .flex_1()
                        .h_full()
                        .mr(px(caption_width))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_size(px(12. * zoom))
                        .text_color(theme::muted())
                        .child("Runner"),
                    cx,
                ),
            )
            .children(self.render_caption_buttons(window, cx))
            .into_any_element()
    }

    pub(crate) fn render_caption_buttons(&self, window: &Window, cx: &App) -> Option<AnyElement> {
        if window.is_fullscreen() {
            return None;
        }
        let zoom = self.settings(cx).app_zoom;
        let maximize_icon = if window.is_maximized() {
            "copy.svg"
        } else {
            "square.svg"
        };
        Some(
            div()
                .absolute()
                .top_0()
                .right_0()
                .h(px(TITLEBAR_HEIGHT * zoom))
                .flex()
                .children(
                    [
                        ("caption-minimize", WindowControlArea::Min, "minus.svg"),
                        ("caption-maximize", WindowControlArea::Max, maximize_icon),
                        ("caption-close", WindowControlArea::Close, "close.svg"),
                    ]
                    .into_iter()
                    .map(|(id, area, icon)| {
                        let close = matches!(area, WindowControlArea::Close);
                        div()
                            .id(id)
                            .group(id)
                            .w(px(CAPTION_BUTTON_WIDTH * zoom))
                            .h_full()
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .occlude()
                            .window_control_area(area)
                            .hover(move |button| {
                                button.bg(if close {
                                    gpui::rgb(0xc42b1c).into()
                                } else {
                                    alpha(theme::sidebar_selected(), 0.6)
                                })
                            })
                            .child(
                                svg()
                                    .path(icon)
                                    .size(px(10. * zoom))
                                    .text_color(theme::muted())
                                    .group_hover(id, move |icon| {
                                        icon.text_color(if close {
                                            gpui::rgb(0xffffff).into()
                                        } else {
                                            theme::text()
                                        })
                                    }),
                            )
                    }),
                )
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_indicator_opens_windows_downloads() {
        let cx = gpui::TestAppContext::single();
        cx.update(|cx| {
            let updater = cx.new(|cx| Updater::new(false, PathBuf::new(), cx));
            activate_update_hint(&updater, cx);
            assert!(!updater.read(cx).is_checking());
        });
        assert_eq!(
            cx.opened_url().as_deref(),
            Some(runner_app::updater::windows_download_url())
        );
    }
}
