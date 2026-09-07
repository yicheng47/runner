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

pub(crate) fn update_hint_tooltip(
    version: &str,
    state: &runner_app::updater::UpdateState,
) -> String {
    if matches!(state, runner_app::updater::UpdateState::Ready { .. }) {
        format!("Runner {version} is ready to install")
    } else {
        format!("Runner {version} is available")
    }
}

pub(crate) fn activate_update_hint(_updater: &Entity<Updater>, window: &mut Window, cx: &mut App) {
    window.dispatch_action(
        Box::new(crate::surfaces::update_dialog::OpenUpdateDialog),
        cx,
    );
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

    struct UpdateTriggerTestView {
        focus: FocusHandle,
        updater: Entity<Updater>,
        opened: bool,
    }

    impl Render for UpdateTriggerTestView {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .track_focus(&self.focus)
                .on_action(cx.listener(
                    |this, _: &crate::surfaces::update_dialog::OpenUpdateDialog, _, _| {
                        this.opened = true;
                    },
                ))
        }
    }

    #[test]
    fn update_indicator_requests_the_dialog_without_opening_a_browser() {
        let mut cx = gpui::TestAppContext::single();
        let window = cx.add_window(|window, cx| {
            let focus = cx.focus_handle();
            focus.focus(window);
            UpdateTriggerTestView {
                focus,
                updater: cx.new(|cx| Updater::new(false, PathBuf::new(), cx)),
                opened: false,
            }
        });
        cx.run_until_parked();
        window
            .update(&mut cx, |view, window, cx| {
                activate_update_hint(&view.updater, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |view, _, cx| {
                assert!(view.opened);
                assert!(!view.updater.read(cx).is_checking());
            })
            .unwrap();
        assert!(cx.opened_url().is_none());
    }

    #[test]
    fn update_indicator_visibility_and_tooltip_follow_state() {
        use runner_app::updater::{UpdateInfo, UpdateState, UpdateStep};
        let info = UpdateInfo::new("0.8.2");
        for state in [
            UpdateState::UpToDate { checking: false },
            UpdateState::Downloading {
                received: 1,
                total: 2,
            },
            UpdateState::Available {
                info: info.clone(),
                installer_url: "url".into(),
                sig_url: None,
            },
            UpdateState::Ready {
                info: info.clone(),
                path: PathBuf::from("verified.exe"),
            },
            UpdateState::Failed {
                step: UpdateStep::Verify,
                message: "test".into(),
                info: Some(info),
            },
        ] {
            let ready = matches!(state, UpdateState::Ready { .. });
            assert_eq!(
                state.available().is_some(),
                !matches!(
                    state,
                    UpdateState::UpToDate { .. } | UpdateState::Downloading { .. }
                )
            );
            assert_eq!(
                update_hint_tooltip("0.8.2", &state),
                if ready {
                    "Runner 0.8.2 is ready to install"
                } else {
                    "Runner 0.8.2 is available"
                }
            );
        }
    }
}
