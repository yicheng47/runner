//! Chat-surface rendering: the active tab, layout picker, and pane tree.
use super::*;
use crate::*;
use gpui::{svg, FontWeight};
use runner_app::ui::{
    ButtonVariant, Modal, OverlayWidth, SessionControlVariant, SessionOverlay, SessionOverlayKind,
    Tooltip,
};

use crate::surfaces::chat_lifecycle::{
    ended_subtitle, resolve_pane_overlay, PaneOverlayState, TransitionKind,
};
use crate::surfaces::sidebar::{direct_chat_display_status, DirectChatDisplayStatus};

const CHAT_PANEL_TRANSITION_MS: u64 = 200;

impl NativeRoot {
    pub(crate) fn render_active_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.ensure_active_chat_detail(cx);
        let Some(layout) = self.tabs.active().cloned() else {
            return div()
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme::muted())
                .child(if self.app_store.read(cx).sessions.is_empty() {
                    "No direct chats yet — press ⌘T"
                } else {
                    "No active tab"
                })
                .into_any_element();
        };
        let preset = layout.preset;
        let session_ids = layout.session_ids();
        let grouped = layout.root.leaves().len() > 1;
        let label = if grouped {
            self.tab_label(&layout, cx)
        } else {
            session_ids
                .first()
                .and_then(|session_id| self.session_entry(session_id, cx))
                .map(session_label)
                .unwrap_or_else(|| "Empty tab".into())
        };
        let lifecycle_busy = session_ids
            .iter()
            .any(|session_id| self.session_lifecycle_disabled(session_id, cx));
        self.configure_chat_action_menu(&layout, lifecycle_busy, cx);
        let pane_tree = self.render_pane_node(&layout.root, &layout, window, cx);
        let picker = self
            .layout_picker_open
            .then(|| self.render_layout_picker(preset, cx));
        let sidebar_toggle = self.render_open_sidebar_button(cx);
        let sidebar_divider = self.settings(cx).sidebar_collapsed.then(|| {
            div()
                .mx_1()
                .h(rems(20. / 16.))
                .w(rems(1. / 16.))
                .flex_none()
                .bg(theme::border())
        });
        let root = cx.entity();
        let layout_root = root.clone();
        let panel_root = root.clone();
        let control = self.render_topbar_session_control(&layout, window, cx);
        let title_group = div()
            .flex_1()
            .min_w(px(0.))
            .flex()
            .items_center()
            .gap_3()
            .child(
                svg()
                    .path(if grouped {
                        "square-split-horizontal.svg"
                    } else {
                        "terminal.svg"
                    })
                    .size(rems(15. / 16.))
                    .flex_none()
                    .text_color(theme::accent()),
            )
            .child(
                div()
                    .min_w(px(0.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .relative()
                            .top(rems(1. / 16.))
                            .min_w(px(0.))
                            .truncate()
                            .text_size(rems(13. / 16.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::text())
                            .child(label),
                    )
                    .children((!session_ids.is_empty()).then(|| self.chat_action_menu.clone()))
                    .children(control),
            );
        let header = div()
            .flex_none()
            .h(rems(WORKSPACE_HEADER_HEIGHT / 16.))
            .pl(px(self.workspace_titlebar_padding(window, cx)))
            .pr_2()
            .flex()
            .items_center()
            .border_b_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .items_center()
                    .gap_2()
                    .children(sidebar_toggle)
                    .children(sidebar_divider)
                    .child(title_group)
                    .child(
                        div()
                            .ml_auto()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                IconButton::new(
                                    "layout-picker-toggle",
                                    "square-split-horizontal.svg",
                                )
                                .focus_handle(self.layout_picker_focus.clone())
                                .variant(if self.layout_picker_open {
                                    ButtonVariant::Secondary
                                } else {
                                    ButtonVariant::Ghost
                                })
                                .tooltip("Layout")
                                .on_press(move |_, cx| {
                                    layout_root.update(cx, |this, cx| {
                                        this.layout_picker_open = !this.layout_picker_open;
                                        cx.notify();
                                    });
                                }),
                            )
                            .when(!self.settings(cx).chat_panel_open, |actions| {
                                actions.child(
                                    IconButton::new("open-chat-panel", "panel-right-hollow.svg")
                                        .tooltip("Open side panel")
                                        .on_press(move |_, cx| {
                                            panel_root.update(cx, |this, cx| {
                                                this.update_app_settings(cx, true, |settings| {
                                                    settings.chat_panel_open = true;
                                                    true
                                                });
                                                cx.notify();
                                            });
                                        }),
                                )
                            }),
                    ),
            );

        let error_banner = self.chat_error.clone().map(|error| {
            div()
                .mx_8()
                .mt_4()
                .rounded(rems(4. / 16.))
                .border_1()
                .border_color(theme::with_alpha(theme::danger(), 0.4))
                .bg(theme::with_alpha(theme::danger(), 0.1))
                .px_3()
                .py_2()
                .text_size(rems(14. / 16.))
                .text_color(theme::danger())
                .child(error)
        });
        let warning_root = root.clone();
        let warning_banner = self.chat_warning.clone().map(|warning| {
            div()
                .mx_8()
                .mt_4()
                .flex()
                .items_start()
                .justify_between()
                .gap_3()
                .rounded(rems(4. / 16.))
                .border_1()
                .border_color(theme::with_alpha(theme::warning(), 0.4))
                .bg(theme::with_alpha(theme::warning(), 0.1))
                .px_3()
                .py_2()
                .text_size(rems(14. / 16.))
                .text_color(theme::warning())
                .child(warning)
                .child(
                    div()
                        .id("dismiss-chat-warning")
                        .cursor_pointer()
                        .text_size(rems(12. / 16.))
                        .text_color(theme::with_alpha(theme::warning(), 0.8))
                        .hover(|button| button.text_color(theme::warning()))
                        .child("Dismiss")
                        .on_click(move |_, _, cx| {
                            warning_root.update(cx, |this, cx| {
                                this.chat_warning = None;
                                cx.notify();
                            });
                        }),
                )
        });

        let chat_column = div()
            .relative()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.layout_picker_open && event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    this.layout_picker_open = false;
                    this.focus_active_terminal(window, cx);
                    cx.notify();
                }
            }))
            .child(self.render_titlebar_drag_area("chat-titlebar-drag", header, cx))
            .children(error_banner)
            .children(warning_banner)
            .child(div().relative().flex_1().min_h(px(0.)).child(pane_tree))
            .children(picker);
        let panel_open = self.settings(cx).chat_panel_open;
        let (panel_visibility, panel_animating) = self.chat_panel_visibility.animate_to(
            if panel_open { 1. } else { 0. },
            Instant::now(),
            Duration::from_millis(CHAT_PANEL_TRANSITION_MS),
        );
        if panel_animating {
            window.request_animation_frame();
        }
        let side_panel = self.render_chat_side_panel(
            panel_visibility,
            panel_open || panel_animating,
            panel_open && !panel_animating,
            cx,
        );

        div()
            .relative()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .child(chat_column)
            .child(side_panel)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.layout_picker_open {
                        this.layout_picker_open = false;
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_split_resize(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.finish_split_resize(cx)),
            )
            .on_drag_move::<ChatPanelResizeDrag>(cx.listener(
                |this, event: &DragMoveEvent<ChatPanelResizeDrag>, _, cx| {
                    let width = f32::from(event.bounds.right() - event.event.position.x)
                        / this.settings(cx).app_zoom;
                    let width = app_settings::clamp_chat_panel_width(width);
                    this.update_app_settings(cx, false, |settings| {
                        if settings.chat_panel_width == width {
                            return false;
                        }
                        settings.chat_panel_width = width;
                        true
                    });
                },
            ))
            .on_drop(cx.listener(|this, _: &ChatPanelResizeDrag, _, cx| {
                this.save_settings(cx);
            }))
            .into_any_element()
    }

    fn configure_chat_action_menu(
        &mut self,
        layout: &PaneLayout,
        lifecycle_busy: bool,
        cx: &mut Context<Self>,
    ) {
        let session_ids = layout.session_ids();
        let grouped = layout.root.leaves().len() > 1;
        let mut actions = Vec::new();
        let mut items = Vec::new();
        if grouped && !session_ids.is_empty() {
            items.push(
                UiMenuItem::new("Rename group")
                    .icon("square-pen.svg")
                    .disabled(lifecycle_busy),
            );
            actions.push(ChatMenuAction::RenameTab {
                tab_id: layout.id.clone(),
                current: layout.name.clone().unwrap_or_default(),
            });
            items.push(
                UiMenuItem::new("Archive all")
                    .icon("archive.svg")
                    .destructive(true)
                    .disabled(lifecycle_busy),
            );
            actions.push(ChatMenuAction::Archive(session_ids));
        } else if let Some(session_id) = session_ids.first() {
            if let Some(entry) = self.session_entry(session_id, cx) {
                let current = session_label(entry);
                items.push(
                    UiMenuItem::new(if entry.pinned { "Unpin" } else { "Pin" })
                        .icon(if entry.pinned {
                            "pin-off.svg"
                        } else {
                            "pin.svg"
                        })
                        .disabled(lifecycle_busy),
                );
                actions.push(ChatMenuAction::TogglePin {
                    session_id: session_id.clone(),
                    pinned: entry.pinned,
                });
                items.push(
                    UiMenuItem::new("Rename")
                        .icon("square-pen.svg")
                        .disabled(lifecycle_busy),
                );
                actions.push(ChatMenuAction::RenameSession {
                    session_id: session_id.clone(),
                    current,
                });
                items.push(
                    UiMenuItem::new("Archive")
                        .icon("archive.svg")
                        .destructive(true)
                        .disabled(lifecycle_busy),
                );
                actions.push(ChatMenuAction::Archive(vec![session_id.clone()]));
            }
        }
        self.chat_menu_actions = actions;
        self.chat_action_menu
            .update(cx, |menu, menu_cx| menu.set_items(items, menu_cx));
    }

    fn render_topbar_session_control(
        &self,
        layout: &PaneLayout,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let session_ids = layout.session_ids();
        if session_ids.is_empty() {
            return None;
        }
        let grouped = layout.root.leaves().len() > 1;
        let lifecycle_busy = session_ids
            .iter()
            .any(|session_id| self.session_lifecycle_disabled(session_id, cx));
        let any_running = session_ids.iter().any(|session_id| {
            self.session_entry(session_id, cx)
                .is_some_and(|entry| entry.status == SessionStatus::Running)
        });
        let any_resuming = session_ids.iter().any(|session_id| {
            self.chat_transitions
                .get(session_id)
                .is_some_and(|transition| transition.kind == TransitionKind::Resuming)
        });
        let root = cx.entity();
        if grouped {
            if any_running {
                let targets = session_ids;
                Some(
                    SessionControl::new("stop-all-chats", SessionControlKind::Stop)
                        .variant(SessionControlVariant::Header)
                        .label("Stop all")
                        .title("Stop all chats")
                        .lifecycle_disabled(lifecycle_busy)
                        .on_press(move |window, cx| {
                            root.update(cx, |this, cx| {
                                this.stop_chats(targets.clone(), window, cx)
                            });
                        })
                        .into_any_element(),
                )
            } else if any_resuming {
                Some(
                    SessionControl::new("resume-all-chats", SessionControlKind::Resuming)
                        .variant(SessionControlVariant::Header)
                        .into_any_element(),
                )
            } else {
                let targets = session_ids;
                Some(
                    SessionControl::new("resume-all-chats", SessionControlKind::Resume)
                        .variant(SessionControlVariant::Header)
                        .label("Resume all")
                        .title("Resume all chats")
                        .lifecycle_disabled(lifecycle_busy)
                        .on_press(move |window, cx| {
                            root.update(cx, |this, cx| {
                                this.resume_chats(targets.clone(), window, cx)
                            });
                        })
                        .into_any_element(),
                )
            }
        } else {
            let session_id = session_ids[0].clone();
            let status = self
                .session_entry(&session_id, cx)
                .map(|entry| entry.status)?;
            if any_resuming {
                Some(
                    SessionControl::new("resume-chat-header", SessionControlKind::Resuming)
                        .variant(SessionControlVariant::Header)
                        .into_any_element(),
                )
            } else if status == SessionStatus::Running {
                Some(
                    SessionControl::new("stop-chat-header", SessionControlKind::Stop)
                        .variant(SessionControlVariant::Header)
                        .title("Stop chat")
                        .lifecycle_disabled(lifecycle_busy)
                        .on_press(move |window, cx| {
                            root.update(cx, |this, cx| this.stop_chat(&session_id, window, cx));
                        })
                        .into_any_element(),
                )
            } else {
                let pane_id = layout.focused_pane_id.clone();
                Some(
                    SessionControl::new("resume-chat-header", SessionControlKind::Resume)
                        .variant(SessionControlVariant::Header)
                        .title("Resume chat")
                        .lifecycle_disabled(lifecycle_busy)
                        .on_press(move |window, cx| {
                            root.update(cx, |this, cx| {
                                this.resume_chat(&pane_id, &session_id, window, cx)
                            });
                        })
                        .into_any_element(),
                )
            }
        }
    }

    fn render_chat_side_panel(
        &self,
        visibility: f32,
        show_panel: bool,
        border_on: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let width = self.settings(cx).chat_panel_width;
        let visible_width = width * visibility;
        if !show_panel {
            return div()
                .id("chat-side-panel")
                .relative()
                .w(rems(visible_width / 16.))
                .h_full()
                .flex_none()
                .overflow_hidden()
                .into_any_element();
        }
        let detail = self.active_chat_detail.as_ref();
        let runner = detail
            .and_then(|detail| detail.runner_id.as_deref())
            .and_then(|runner_id| {
                self.app_store
                    .read(cx)
                    .runners
                    .iter()
                    .find(|runner| runner.id == runner_id)
                    .cloned()
            });
        let collapse_root = cx.entity();
        let header = self.render_titlebar_drag_area(
            "chat-panel-titlebar-drag",
            div()
                .flex_none()
                .h(rems(WORKSPACE_HEADER_HEIGHT / 16.))
                .pl_4()
                .pr_2()
                .flex()
                .items_center()
                .justify_end()
                .border_b_1()
                .border_color(theme::border())
                .child(
                    IconButton::new("collapse-chat-panel", "panel-right-filled.svg")
                        .tooltip("Collapse side panel")
                        .on_press(move |_, cx| {
                            collapse_root.update(cx, |this, cx| {
                                this.update_app_settings(cx, true, |settings| {
                                    settings.chat_panel_open = false;
                                    true
                                });
                                cx.notify();
                            });
                        }),
                ),
            cx,
        );
        let content = if let Some(detail) = detail {
            let (
                section_label,
                identity,
                identity_monospace,
                badge,
                description,
                command,
                cwd,
                system_prompt,
            ) = if let Some(runner) = runner.as_ref() {
                (
                    "Runner",
                    format!("@{}", runner.handle),
                    true,
                    runner.runtime.clone(),
                    (!runner.display_name.is_empty()).then(|| runner.display_name.clone()),
                    runner.command.clone(),
                    runner.working_dir.clone(),
                    runner.system_prompt.clone(),
                )
            } else {
                (
                    "Runtime",
                    detail.display_name.clone(),
                    false,
                    detail.agent_runtime.clone(),
                    None,
                    detail.agent_command.clone(),
                    detail.cwd.clone(),
                    None,
                )
            };
            div()
                .flex()
                .flex_col()
                .gap(rems(18. / 16.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(rems(10. / 16.))
                        .child(side_panel_label(section_label))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(rems(10. / 16.))
                                .rounded_lg()
                                .border_1()
                                .border_color(theme::border_strong())
                                .bg(theme::bg())
                                .p(rems(14. / 16.))
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .child(
                                            div()
                                                .when(identity_monospace, |identity| {
                                                    identity.font_family("Menlo")
                                                })
                                                .text_size(rems(14. / 16.))
                                                .font_weight(FontWeight::SEMIBOLD)
                                                .text_color(theme::text())
                                                .child(identity),
                                        )
                                        .child(runtime_badge(badge)),
                                )
                                .children(description.map(|description| {
                                    div()
                                        .text_size(rems(12. / 16.))
                                        .text_color(theme::muted())
                                        .child(description)
                                }))
                                .child(div().h(rems(1. / 16.)).w_full().bg(theme::border()))
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .gap(rems(6. / 16.))
                                        .child(side_panel_row("cmd", side_panel_value(command)))
                                        .children(cwd.map(|cwd| {
                                            side_panel_row("cwd", side_panel_value(cwd))
                                        }))
                                        .child(side_panel_row(
                                            "session_key",
                                            div()
                                                .flex_1()
                                                .min_w(px(0.))
                                                .flex()
                                                .items_start()
                                                .gap(rems(6. / 16.))
                                                .child(
                                                    div()
                                                        .flex_1()
                                                        .min_w(px(0.))
                                                        .font_family("Menlo")
                                                        .text_color(theme::muted())
                                                        .child(
                                                            detail
                                                                .agent_session_key
                                                                .clone()
                                                                .unwrap_or_else(|| "NULL".into()),
                                                        ),
                                                )
                                                .child(self.session_key_copy.clone()),
                                        )),
                                ),
                        ),
                )
                .children(system_prompt.map(|prompt| {
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(side_panel_label("System prompt"))
                        .child(
                            div()
                                .rounded_md()
                                .border_1()
                                .border_color(theme::border_strong())
                                .bg(theme::bg())
                                .p_3()
                                .text_size(rems(12. / 16.))
                                .line_height(rems(20. / 16.))
                                .text_color(theme::muted())
                                .child(prompt),
                        )
                }))
                .into_any_element()
        } else {
            div()
                .text_xs()
                .text_color(theme::faint())
                .child("Loading chat…")
                .into_any_element()
        };
        let drag = ChatPanelResizeDrag;
        let panel = div()
            .relative()
            .w(rems(width / 16.))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(theme::panel())
            .child(header)
            .child(
                div()
                    .id("chat-panel-scroll")
                    .flex_1()
                    .min_h(px(0.))
                    .overflow_y_scroll()
                    .p_5()
                    .child(content),
            )
            .child(
                div()
                    .id("chat-panel-resize")
                    .absolute()
                    .left_0()
                    .top_0()
                    .h_full()
                    .w(rems(4. / 16.))
                    .cursor(CursorStyle::ResizeLeftRight)
                    .hover(|strip| strip.bg(theme::with_alpha(theme::accent(), 0.4)))
                    .on_drag(drag, |drag: &ChatPanelResizeDrag, _, _, cx: &mut App| {
                        cx.new(|_| drag.clone())
                    }),
            );
        div()
            .id("chat-side-panel")
            .relative()
            .w(rems(visible_width / 16.))
            .h_full()
            .flex_none()
            .overflow_hidden()
            .bg(theme::panel())
            .when(border_on, |panel| {
                panel.border_l_1().border_color(theme::border())
            })
            .child(panel)
            .into_any_element()
    }

    pub(crate) fn render_chat_rename_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let modal = self.chat_rename_modal.as_ref().expect("chat rename modal");
        let is_group = matches!(modal.target, ChatRenameTarget::Tab { .. });
        let submitting = modal.submitting;
        let valid = is_group || !modal.input.read(cx).text().trim().is_empty();
        let root = cx.entity();
        let close_root = root.clone();
        let cancel_root = root.clone();
        let submit_root = root.clone();
        let dismiss_root = root;
        let title = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(2. / 16.))
                    .child(
                        div()
                            .text_size(rems(1.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(if is_group {
                                "Rename group"
                            } else {
                                "Rename chat"
                            }),
                    )
                    .children(is_group.then(|| {
                        div()
                            .text_size(rems(12. / 16.))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::muted())
                            .child("Leave blank to derive the name from its chats.")
                    })),
            )
            .child(
                IconButton::new("close-chat-rename", "close.svg")
                    .focus_handle(modal.close_focus.clone())
                    .tooltip("Close rename")
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        close_root.update(cx, |this, cx| this.close_chat_rename(window, cx));
                    }),
            );
        let body = div()
            .flex()
            .flex_col()
            .gap_3()
            .on_key_down(cx.listener(Self::on_chat_rename_key_down))
            .children(modal.error.clone().map(|error| {
                div()
                    .rounded_sm()
                    .border_1()
                    .border_color(theme::with_alpha(theme::danger(), 0.4))
                    .bg(theme::with_alpha(theme::danger(), 0.1))
                    .px_3()
                    .py_2()
                    .text_xs()
                    .text_color(theme::danger())
                    .child(error)
            }))
            .child(
                runner_app::ui::Field::new("chat-rename-name", "Name", modal.input.clone())
                    .focus_target(modal.input.read(cx).focus_handle())
                    .emphasized(true),
            );
        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("cancel-chat-rename", "Cancel")
                    .focus_handle(modal.cancel_focus.clone())
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        cancel_root.update(cx, |this, cx| this.close_chat_rename(window, cx));
                    }),
            )
            .child(
                Button::new(
                    "submit-chat-rename",
                    if submitting { "Saving…" } else { "Save" },
                )
                .focus_handle(modal.submit_focus.clone())
                .variant(ButtonVariant::Primary)
                .disabled(submitting || !valid)
                .on_press(move |window, cx| {
                    submit_root.update(cx, |this, cx| this.submit_chat_rename(window, cx));
                }),
            );
        Modal::new(
            title,
            body,
            Rc::new(move |window, cx| {
                dismiss_root.update(cx, |this, cx| this.close_chat_rename(window, cx));
            }),
        )
        .width(OverlayWidth::Custom(448.))
        .busy(submitting)
        .focus_order(if submitting {
            Vec::new()
        } else {
            vec![
                modal.input.read(cx).focus_handle(),
                modal.cancel_focus.clone(),
                modal.submit_focus.clone(),
                modal.close_focus.clone(),
            ]
        })
        .footer(footer)
        .into_any_element()
    }

    pub(crate) fn render_layout_picker(
        &self,
        active: PresetKind,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let rows = [
            ("1", vec![PresetKind::Single]),
            ("2", vec![PresetKind::Cols2, PresetKind::Rows2]),
            (
                "3",
                vec![PresetKind::Main2, PresetKind::Cols3, PresetKind::Rows3],
            ),
        ];
        let root = cx.entity();
        let picker_right = if self.settings(cx).chat_panel_open {
            8.
        } else {
            44.
        };
        div()
            .absolute()
            .id("layout-picker-popup")
            .top(rems((WORKSPACE_HEADER_HEIGHT + 6.) / 16.))
            .right(rems(picker_right / 16.))
            .w(rems(236. / 16.))
            .p(rems(14. / 16.))
            .flex()
            .flex_col()
            .gap(rems(14. / 16.))
            .rounded_lg()
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .shadow_lg()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .font_family("Menlo")
                    .text_size(rems(10. / 16.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::faint())
                    .child("LAYOUT"),
            )
            .children(rows.into_iter().map(|(count, presets)| {
                let root = root.clone();
                div()
                    .flex()
                    .items_center()
                    .gap(rems(10. / 16.))
                    .child(
                        div()
                            .w(rems(8. / 16.))
                            .font_family("Menlo")
                            .text_size(rems(11. / 16.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::muted())
                            .child(count),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .children(presets.into_iter().map(move |preset| {
                                let tile_root = root.clone();
                                let tile = div()
                                    .id(SharedString::from(format!(
                                        "layout-preset-{}",
                                        preset.label()
                                    )))
                                    .w(rems(56. / 16.))
                                    .h(rems(40. / 16.))
                                    .p(rems(4. / 16.))
                                    .rounded(rems(5. / 16.))
                                    .border_1()
                                    .border_color(if preset == active {
                                        theme::accent()
                                    } else {
                                        theme::border_strong()
                                    })
                                    .bg(theme::bg())
                                    .cursor_pointer()
                                    .hover(|tile| tile.border_color(theme::faint()))
                                    .child(preset_diagram(preset, preset == active))
                                    .on_click(move |_, window, cx| {
                                        tile_root.update(cx, |this, cx| {
                                            this.pick_preset(preset, window, cx)
                                        });
                                    });
                                Tooltip::new(
                                    SharedString::from(format!(
                                        "layout-preset-tooltip-{}",
                                        preset.label()
                                    )),
                                    preset.label(),
                                    tile,
                                )
                            })),
                    )
            }))
            .child(div().h(rems(1. / 16.)).w_full().bg(theme::border()))
            .child(
                div()
                    .text_size(rems(10. / 16.))
                    .text_color(theme::faint())
                    .child("Layout is remembered across restarts"),
            )
            .into_any_element()
    }

    pub(crate) fn render_pane_node(
        &mut self,
        node: &PaneNode,
        layout: &PaneLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match node {
            PaneNode::Leaf(leaf) => self.render_pane(leaf, layout, window, cx),
            PaneNode::Split(split) => {
                let a = self.render_pane_node(&split.a, layout, window, cx);
                let b = self.render_pane_node(&split.b, layout, window, cx);
                let first = split.sizes[0] / 100.;
                let second = split.sizes[1] / 100.;
                let split_id = split.id.clone();
                let orientation = split.orientation;
                let drag = SplitResizeDrag {
                    split_id: split.id.clone(),
                    orientation,
                };
                let a_wrap = div()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .when(orientation == SplitOrientation::Row, |pane| {
                        pane.w(relative(first)).h_full()
                    })
                    .when(orientation == SplitOrientation::Column, |pane| {
                        pane.h(relative(first)).w_full()
                    })
                    .child(a);
                let b_wrap = div()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .when(orientation == SplitOrientation::Row, |pane| {
                        pane.w(relative(second)).h_full()
                    })
                    .when(orientation == SplitOrientation::Column, |pane| {
                        pane.h(relative(second)).w_full()
                    })
                    .child(b);
                let gutter = div()
                    .id(SharedString::from(format!("gutter-{}", split.id)))
                    .flex_none()
                    .when(orientation == SplitOrientation::Row, |gutter| {
                        gutter
                            .w(rems(5. / 16.))
                            .h_full()
                            .cursor(CursorStyle::ResizeLeftRight)
                    })
                    .when(orientation == SplitOrientation::Column, |gutter| {
                        gutter
                            .h(rems(5. / 16.))
                            .w_full()
                            .cursor(CursorStyle::ResizeUpDown)
                    })
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .when(orientation == SplitOrientation::Row, |line| {
                                line.w(rems(1. / 16.)).h_full()
                            })
                            .when(orientation == SplitOrientation::Column, |line| {
                                line.h(rems(1. / 16.)).w_full()
                            })
                            .bg(theme::border_strong()),
                    )
                    .on_drag(drag, |drag: &SplitResizeDrag, _, _, cx: &mut App| {
                        cx.new(|_| drag.clone())
                    });
                div()
                    .id(SharedString::from(format!("split-{}", split.id)))
                    .size_full()
                    .flex()
                    .when(orientation == SplitOrientation::Column, |container| {
                        container.flex_col()
                    })
                    .child(a_wrap)
                    .child(gutter)
                    .child(b_wrap)
                    .on_drag_move::<SplitResizeDrag>(cx.listener(
                        move |this, event: &DragMoveEvent<SplitResizeDrag>, _, cx| {
                            let drag = event.drag(cx);
                            if drag.split_id == split_id {
                                this.resize_split(&split_id, drag.orientation, event, cx);
                            }
                        },
                    ))
                    .on_drop(cx.listener(|this, _: &SplitResizeDrag, _, cx| {
                        this.finish_split_resize(cx);
                    }))
                    .into_any_element()
            }
        }
    }

    fn pane_action_menu(
        &mut self,
        session_id: &str,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> Entity<PopoverMenu> {
        let menu = self
            .pane_action_menus
            .entry(session_id.to_owned())
            .or_insert_with(|| {
                let root = cx.entity();
                let target = session_id.to_owned();
                cx.new(move |menu_cx| {
                    let action_root = root.clone();
                    let action_target = target.clone();
                    PopoverMenu::new(
                        SharedString::from(format!("pane-actions-{target}")),
                        menu_cx.focus_handle(),
                        Vec::new(),
                        Rc::new(move |index, window, cx| {
                            if index == 0 {
                                let session_id = action_target.clone();
                                action_root.update(cx, |this, cx| {
                                    this.archive_chat_sessions(vec![session_id], window, cx)
                                });
                            }
                        }),
                        menu_cx,
                    )
                    .min_width(px(144.))
                    .trigger_size(IconButtonSize::Sm)
                    .trigger_icon("more-horizontal.svg")
                    .trigger_tooltip("Pane actions")
                })
            })
            .clone();
        menu.update(cx, |menu, menu_cx| {
            menu.set_items(
                vec![UiMenuItem::new("Archive")
                    .icon("archive.svg")
                    .destructive(true)
                    .disabled(disabled)],
                menu_cx,
            )
        });
        menu
    }

    pub(crate) fn render_pane(
        &mut self,
        leaf: &PaneLeaf,
        layout: &PaneLayout,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let focused = layout.focused_pane_id == leaf.id;
        let grouped = layout.root.leaves().len() > 1;
        let pane_id = leaf.id.clone();
        let pane_id_for_focus = pane_id.clone();
        let entry = leaf
            .session_id
            .as_deref()
            .and_then(|session_id| self.session_entry(session_id, cx))
            .cloned();
        let pane_session_for_focus = entry.as_ref().map(|entry| entry.session_id.clone());
        let close_root = cx.entity();
        let header = grouped.then(|| {
            let close_pane_id = pane_id.clone();
            let label = entry
                .as_ref()
                .map(session_label)
                .unwrap_or_else(|| "Empty pane".into());
            let status = entry.as_ref().map(|entry| {
                direct_chat_display_status(
                    entry,
                    self.app_store
                        .read(cx)
                        .session_activity
                        .get(&entry.session_id),
                )
            });
            let controls = entry.as_ref().map(|entry| {
                let session_id = entry.session_id.clone();
                let transition = self.chat_transitions.get(&session_id).map(|item| item.kind);
                let disabled = self.session_lifecycle_disabled(&session_id, cx);
                let control_root = cx.entity();
                let lifecycle = if transition == Some(TransitionKind::Resuming) {
                    SessionControl::new(
                        SharedString::from(format!("pane-resuming-{session_id}")),
                        SessionControlKind::Resuming,
                    )
                } else if entry.status == SessionStatus::Running {
                    let stop_id = session_id.clone();
                    SessionControl::new(
                        SharedString::from(format!("pane-stop-{session_id}")),
                        SessionControlKind::Stop,
                    )
                    .title("Stop this chat")
                    .lifecycle_disabled(disabled)
                    .on_press(move |window, cx| {
                        control_root.update(cx, |this, cx| this.stop_chat(&stop_id, window, cx));
                    })
                } else {
                    let resume_id = session_id.clone();
                    let resume_pane = pane_id.clone();
                    SessionControl::new(
                        SharedString::from(format!("pane-resume-{session_id}")),
                        SessionControlKind::Resume,
                    )
                    .title("Resume this chat")
                    .lifecycle_disabled(disabled)
                    .on_press(move |window, cx| {
                        control_root.update(cx, |this, cx| {
                            this.resume_chat(&resume_pane, &resume_id, window, cx)
                        });
                    })
                };
                div()
                    .ml_auto()
                    .flex()
                    .items_center()
                    .gap_1()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(lifecycle)
                    .child(self.pane_action_menu(&session_id, disabled, cx))
            });
            div()
                .flex_none()
                .h(rems(PANE_HEADER_HEIGHT / 16.))
                .px(rems(14. / 16.))
                .flex()
                .items_center()
                .gap_2()
                .border_b_1()
                .border_color(theme::border())
                .bg(theme::panel())
                .child(
                    svg()
                        .path("terminal.svg")
                        .size(rems(13. / 16.))
                        .flex_none()
                        .mr_1()
                        .text_color(if focused {
                            theme::accent()
                        } else {
                            theme::faint()
                        }),
                )
                .child(
                    div()
                        .relative()
                        .top(rems(1. / 16.))
                        .min_w(px(0.))
                        .truncate()
                        .text_size(rems(13. / 16.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if focused {
                            theme::text()
                        } else {
                            theme::muted()
                        })
                        .child(label),
                )
                .child(chat_badge("Chat"))
                .children(status.map(render_pane_status))
                .children(controls)
                .when(entry.is_none(), |header| {
                    header.child(
                        div()
                            .ml_auto()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(
                                IconButton::new(
                                    SharedString::from(format!("close-pane-{close_pane_id}")),
                                    "close.svg",
                                )
                                .size(IconButtonSize::Sm)
                                .tooltip("Close pane")
                                .on_press(move |window, cx| {
                                    close_root.update(cx, |this, cx| {
                                        this.close_pane(&close_pane_id, window, cx);
                                    });
                                }),
                            ),
                    )
                })
        });

        let body: AnyElement = if let Some(entry) = entry.as_ref() {
            let session_id = entry.session_id.clone();
            let transition = self.chat_transitions.get(&session_id).map(|item| item.kind);
            let overlay = resolve_pane_overlay(
                self.sidebar_archiving_session(&session_id, cx),
                transition,
                entry.status,
                entry.resumable,
                self.session_exit_codes.get(&session_id).copied().flatten(),
            );
            let interactive = self.session_is_interactive(&session_id, cx);
            let scrollable = self.route == AppRoute::Chat
                && transition.is_none()
                && !self.sidebar_archiving_session(&session_id, cx);
            let terminal_style = self.terminal_style(cx);
            let terminal_background =
                crate::terminal::element::to_hsla(terminal_style.palette.background, 1.);
            let terminal_surface = if let Some(chat) = self.attached.get(&session_id) {
                let terminal = Arc::clone(&chat.terminal);
                let terminal_scrollbar = chat.terminal_scrollbar.clone();
                let terminal_input = chat.terminal_input.clone();
                let terminal_focus = chat.terminal_focus.clone();
                let resize_owner = scrollable && layout.is_resize_owner(&pane_id, &session_id);
                let key_session_id = session_id.clone();
                let scroll_session_id = session_id.clone();
                let paste_session_id = session_id.clone();
                div()
                    .id(SharedString::from(format!("terminal-{session_id}")))
                    .absolute()
                    .inset_0()
                    .key_context("Terminal")
                    .track_focus(&terminal_focus)
                    .flex()
                    .py_3()
                    .pl_3()
                    .pr_1()
                    .bg(terminal_background)
                    .opacity(match overlay {
                        PaneOverlayState::Starting | PaneOverlayState::Resuming => 0.,
                        PaneOverlayState::Ended { .. } => 0.45,
                        PaneOverlayState::Archiving | PaneOverlayState::None => 1.,
                    })
                    .when(interactive, |surface| {
                        surface
                            .on_key_down(cx.listener(move |this, event, window, cx| {
                                this.on_key_down(&key_session_id, event, window, cx);
                            }))
                            .on_action(cx.listener(move |this, action, window, cx| {
                                this.on_paste(&paste_session_id, action, window, cx);
                            }))
                    })
                    .when(scrollable, |surface| {
                        surface.on_scroll_wheel(cx.listener(move |this, event, window, cx| {
                            this.on_scroll(&scroll_session_id, event, window, cx);
                        }))
                    })
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w(px(0.))
                            .min_h(px(0.))
                            .child(TerminalElement::new(
                                terminal,
                                terminal_input,
                                terminal_focus,
                                resize_owner,
                                terminal_style,
                            ))
                            .child(terminal_scrollbar),
                    )
                    .into_any_element()
            } else {
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(terminal_background)
                    .text_size(rems(12. / 16.))
                    .text_color(theme::faint())
                    .child("Unable to attach chat")
                    .into_any_element()
            };
            let overlay_element = match overlay {
                PaneOverlayState::Archiving => Some(
                    SessionOverlay::transition(
                        format!("archiving-{session_id}"),
                        SessionOverlayKind::Archiving,
                    )
                    .into_any_element(),
                ),
                PaneOverlayState::Resuming => Some(
                    SessionOverlay::transition(
                        format!("resuming-{session_id}"),
                        SessionOverlayKind::Resuming,
                    )
                    .into_any_element(),
                ),
                PaneOverlayState::Starting => Some(
                    SessionOverlay::transition(
                        format!("starting-{session_id}"),
                        SessionOverlayKind::Starting,
                    )
                    .into_any_element(),
                ),
                PaneOverlayState::Ended {
                    status,
                    resumable,
                    exit_code,
                } => {
                    let resume_root = cx.entity();
                    let archive_root = resume_root.clone();
                    let resume_id = session_id.clone();
                    let archive_id = session_id.clone();
                    let resume_pane = pane_id.clone();
                    Some(
                        SessionOverlay::ended(
                            format!("ended-{session_id}"),
                            ended_subtitle(status, resumable, exit_code),
                            move |window, cx| {
                                resume_root.update(cx, |this, cx| {
                                    this.resume_chat(&resume_pane, &resume_id, window, cx)
                                });
                            },
                            move |window, cx| {
                                archive_root.update(cx, |this, cx| {
                                    this.archive_chat_sessions(vec![archive_id.clone()], window, cx)
                                });
                            },
                        )
                        .into_any_element(),
                    )
                }
                PaneOverlayState::None => None,
            };
            div()
                .relative()
                .flex_1()
                .min_h(px(0.))
                .overflow_hidden()
                .bg(terminal_background)
                .child(terminal_surface)
                .children(overlay_element)
                .into_any_element()
        } else {
            let new_chat_pane_id = pane_id.clone();
            let root = cx.entity();
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(rems(14. / 16.))
                .p_4()
                .child(
                    svg()
                        .path("square-pen.svg")
                        .size(rems(22. / 16.))
                        .text_color(theme::faint()),
                )
                .child(
                    div()
                        .text_size(rems(13. / 16.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::muted())
                        .child("No chat in this pane"),
                )
                .child(
                    Button::new(
                        SharedString::from(format!("new-chat-{pane_id}")),
                        "New chat",
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Primary)
                    .on_press(move |window, cx| {
                        root.update(cx, |this, cx| {
                            this.open_pane_chat_modal(&new_chat_pane_id, window, cx)
                        });
                    }),
                )
                .into_any_element()
        };

        div()
            .id(SharedString::from(format!("pane-{pane_id}")))
            .relative()
            .size_full()
            .min_w(px(0.))
            .min_h(px(0.))
            .flex()
            .flex_col()
            .overflow_hidden()
            .when(grouped, |pane| {
                pane.border_1().border_color(if focused {
                    theme::accent()
                } else {
                    gpui::transparent_black()
                })
            })
            .children(header)
            .child(body)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, window, cx| {
                    if let Some(session_id) = pane_session_for_focus.as_deref() {
                        this.focus_terminal(&pane_id_for_focus, session_id, window, cx);
                    } else {
                        this.focus_pane(&pane_id_for_focus, cx);
                    }
                }),
            )
            .into_any_element()
    }
}

fn chat_badge(label: impl Into<SharedString>) -> AnyElement {
    let label = label.into();
    div()
        .flex_none()
        .rounded(rems(3. / 16.))
        .bg(theme::border_strong())
        .px_2()
        .py(rems(1. / 16.))
        .font_weight(FontWeight::BOLD)
        .text_size(rems(9. / 16.))
        .text_color(theme::muted())
        .child(label.to_uppercase())
        .into_any_element()
}

fn runtime_badge(label: impl Into<SharedString>) -> AnyElement {
    let label = label.into();
    div()
        .flex_none()
        .rounded(rems(3. / 16.))
        .bg(theme::border_strong())
        .px(rems(6. / 16.))
        .py(rems(1. / 16.))
        .font_weight(FontWeight::BOLD)
        .text_size(rems(9. / 16.))
        .text_color(theme::muted())
        .child(label.to_uppercase())
        .into_any_element()
}

fn render_pane_status(status: DirectChatDisplayStatus) -> AnyElement {
    let (label, color) = match status {
        DirectChatDisplayStatus::Busy => ("busy", theme::accent()),
        DirectChatDisplayStatus::Idle => ("idle", theme::with_alpha(theme::accent(), 0.35)),
        DirectChatDisplayStatus::Stopped => ("stopped", theme::faint()),
        DirectChatDisplayStatus::Crashed => ("crashed", theme::danger()),
    };
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(rems(6. / 16.))
        .child(div().size(rems(6. / 16.)).rounded_full().bg(color))
        .child(
            div()
                .text_size(rems(11. / 16.))
                .text_color(theme::muted())
                .child(label),
        )
        .into_any_element()
}

fn side_panel_label(label: &'static str) -> AnyElement {
    div()
        .font_weight(FontWeight::SEMIBOLD)
        .text_size(rems(10. / 16.))
        .text_color(theme::faint())
        .child(label.to_uppercase())
        .into_any_element()
}

fn side_panel_key(label: &'static str) -> AnyElement {
    div()
        .w(rems(72. / 16.))
        .flex_none()
        .text_color(theme::faint())
        .child(label)
        .into_any_element()
}

fn side_panel_value(value: String) -> AnyElement {
    div()
        .flex_1()
        .min_w(px(0.))
        .font_family("Menlo")
        .text_color(theme::muted())
        .child(value)
        .into_any_element()
}

fn side_panel_row(label: &'static str, value: impl IntoElement) -> AnyElement {
    div()
        .flex()
        .items_start()
        .gap_3()
        .text_size(rems(11. / 16.))
        .child(side_panel_key(label))
        .child(value)
        .into_any_element()
}

fn preset_diagram(preset: PresetKind, active: bool) -> AnyElement {
    let pane = || {
        div()
            .flex_1()
            .min_w(px(0.))
            .min_h(px(0.))
            .rounded(rems(2. / 16.))
            .bg(if active {
                theme::with_alpha(theme::accent(), 0.15)
            } else {
                theme::sidebar_selected()
            })
    };
    match preset {
        PresetKind::Single => div().size_full().flex().child(pane()).into_any_element(),
        PresetKind::Cols2 => div()
            .size_full()
            .flex()
            .gap(rems(3. / 16.))
            .child(pane())
            .child(pane())
            .into_any_element(),
        PresetKind::Rows2 => div()
            .size_full()
            .flex()
            .flex_col()
            .gap(rems(3. / 16.))
            .child(pane())
            .child(pane())
            .into_any_element(),
        PresetKind::Main2 => div()
            .size_full()
            .flex()
            .gap(rems(3. / 16.))
            .child(pane())
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap(rems(3. / 16.))
                    .child(pane())
                    .child(pane()),
            )
            .into_any_element(),
        PresetKind::Cols3 => div()
            .size_full()
            .flex()
            .gap(rems(3. / 16.))
            .child(pane())
            .child(pane())
            .child(pane())
            .into_any_element(),
        PresetKind::Rows3 => div()
            .size_full()
            .flex()
            .flex_col()
            .gap(rems(3. / 16.))
            .child(pane())
            .child(pane())
            .child(pane())
            .into_any_element(),
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
    use super::adjacent_pane_index;

    #[test]
    fn adjacent_pane_index_wraps_in_both_directions() {
        assert_eq!(adjacent_pane_index(0, 3, -1), Some(2));
        assert_eq!(adjacent_pane_index(2, 3, 1), Some(0));
        assert_eq!(adjacent_pane_index(1, 3, 1), Some(2));
        assert_eq!(adjacent_pane_index(0, 1, 1), None);
        assert_eq!(adjacent_pane_index(3, 3, -1), None);
    }
}
