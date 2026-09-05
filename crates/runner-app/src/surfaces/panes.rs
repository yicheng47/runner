//! Chat-surface rendering: the active tab, layout picker, and pane tree.
use super::*;
use crate::*;
use gpui::{svg, FontWeight};
use runner_app::ui::{
    ButtonVariant, Modal, OverlayWidth, SessionControlVariant, SessionOverlay, SessionOverlayKind,
    Tooltip,
};

use crate::surfaces::chat_lifecycle::{
    ended_subtitle, resolve_pane_overlay, shell_exited_subtitle, PaneOverlayState, TransitionKind,
};
use crate::surfaces::sidebar::{direct_chat_display_status, DirectChatDisplayStatus};

const CHAT_PANEL_TRANSITION_MS: u64 = 200;

pub(crate) type TerminalDrawerSessionCallback = Rc<dyn Fn(String, &mut Window, &mut App)>;
pub(crate) type TerminalDrawerActionCallback = Rc<dyn Fn(&mut Window, &mut App)>;

pub(crate) struct TerminalDrawerCallbacks {
    pub activate: TerminalDrawerSessionCallback,
    pub close: TerminalDrawerSessionCallback,
    pub add: TerminalDrawerActionCallback,
    pub hide: TerminalDrawerActionCallback,
}

pub(crate) fn render_terminal_drawer_strip(
    id_prefix: &str,
    shells: &[String],
    active_id: Option<&str>,
    labels: &[String],
    callbacks: TerminalDrawerCallbacks,
) -> AnyElement {
    let TerminalDrawerCallbacks {
        activate,
        close,
        add,
        hide,
    } = callbacks;
    let chips = shells
        .iter()
        .enumerate()
        .map(|(index, session_id)| {
            let active = active_id == Some(session_id.as_str());
            let label = labels.get(index).cloned().unwrap_or_else(|| "shell".into());
            let activate = Rc::clone(&activate);
            let close = Rc::clone(&close);
            let activate_id = session_id.clone();
            let close_id = session_id.clone();
            div()
                .id(SharedString::from(format!(
                    "{id_prefix}-drawer-chip-{session_id}"
                )))
                .flex_none()
                .h(rems(24. / 16.))
                .max_w(rems(180. / 16.))
                .px_2()
                .flex()
                .items_center()
                .gap(rems(6. / 16.))
                .rounded(rems(5. / 16.))
                .bg(if active {
                    theme::raised()
                } else {
                    gpui::transparent_black()
                })
                .text_color(if active {
                    theme::text()
                } else {
                    theme::muted()
                })
                .cursor_pointer()
                .hover(|chip| chip.bg(theme::raised()))
                .child(
                    svg()
                        .path("terminal.svg")
                        .size(rems(12. / 16.))
                        .flex_none()
                        .text_color(theme::muted()),
                )
                .child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(rems(12. / 16.))
                        .child(label),
                )
                .child(
                    IconButton::new(
                        SharedString::from(format!("{id_prefix}-close-drawer-shell-{session_id}")),
                        "close.svg",
                    )
                    .size(IconButtonSize::Xs)
                    .stop_click_propagation(true)
                    .tooltip("Close terminal")
                    .on_press(move |window, cx| close(close_id.clone(), window, cx)),
                )
                .on_click(move |_, window, cx| activate(activate_id.clone(), window, cx))
        })
        .collect::<Vec<_>>();
    let add_id = SharedString::from(format!("{id_prefix}-add-drawer-shell"));
    let hide_id = SharedString::from(format!("{id_prefix}-hide-terminal-drawer"));
    div()
        .flex_none()
        .h(rems(32. / 16.))
        .w_full()
        .pl(rems(10. / 16.))
        .pr(rems(8. / 16.))
        .flex()
        .items_center()
        .gap(rems(6. / 16.))
        .border_b_1()
        .border_color(theme::border())
        .bg(theme::panel())
        .children(chips)
        .child(
            IconButton::new(add_id, "plus.svg")
                .size(IconButtonSize::Sm)
                .tooltip("New terminal")
                .on_press(move |window, cx| add(window, cx)),
        )
        .child(div().flex_1())
        .child(
            IconButton::new(hide_id, "chevron-down.svg")
                .size(IconButtonSize::Sm)
                .tooltip("Hide terminal drawer")
                .on_press(move |window, cx| hide(window, cx)),
        )
        .into_any_element()
}

impl NativeRoot {
    pub(crate) fn render_archived_chat(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(detail) = self.archived_chat_detail.as_ref() else {
            return div().into_any_element();
        };
        let label = session_label(detail);
        let back_label = if detail.handle.is_some() {
            "Back to runner"
        } else {
            "Back to runners"
        };
        let handle = detail.handle.clone();
        let root = cx.entity();
        let panel_root = root.clone();
        let sidebar_toggle = self.render_open_sidebar_button(cx);
        let sidebar_divider = self.sidebar_collapsed.then(|| {
            div()
                .mx_1()
                .h(rems(20. / 16.))
                .w(rems(1. / 16.))
                .flex_none()
                .bg(theme::border())
        });
        let header = div()
            .flex_none()
            .h(rems(WORKSPACE_HEADER_HEIGHT / 16.))
            .pl(px(self.workspace_titlebar_padding(window, cx)))
            .pr_2()
            .map(|header| {
                #[cfg(windows)]
                let header = header
                    .pr(px(8. * self.settings(cx).app_zoom
                        + self.chat_header_caption_inset(window, cx)));
                header
            })
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .children(sidebar_toggle)
            .children(sidebar_divider)
            .child(
                svg()
                    .path("terminal.svg")
                    .size(rems(15. / 16.))
                    .flex_none()
                    .text_color(theme::accent()),
            )
            .child(
                div()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(rems(13. / 16.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(label),
            )
            .child(
                Button::new("archived-chat-back", back_label)
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .on_press(move |window, cx| {
                        let handle = handle.clone();
                        root.update(cx, |this, root_cx| {
                            this.archived_chat_detail = None;
                            if let Some(handle) = handle {
                                this.open_runner_detail(handle, window, root_cx);
                            } else {
                                this.open_runners(window, root_cx);
                            }
                        });
                    }),
            )
            .child(div().min_w(px(0.)).flex_1())
            .when(!self.settings(cx).chat_panel_open, |header| {
                header.child(
                    IconButton::new("open-archived-chat-panel", "panel-right-hidden.svg")
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
            });
        let chat_column = div()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .child(self.render_titlebar_drag_area("archived-chat-titlebar-drag", header, cx))
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .max_w(rems(448. / 16.))
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_2()
                            .rounded(rems(4. / 16.))
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::raised())
                            .px_6()
                            .py_5()
                            .text_center()
                            .child(
                                div()
                                    .text_size(rems(13. / 16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Session ended — terminal closed"),
                            )
                            .child(
                                div()
                                    .text_size(rems(12. / 16.))
                                    .text_color(theme::muted())
                                    .child("This chat was archived. The PTY is gone and the workspace is read-only."),
                            ),
                    ),
            );
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
            self.archived_chat_detail.as_ref(),
            self.archived_session_key_copy.clone(),
            panel_visibility,
            panel_open || panel_animating,
            panel_open && !panel_animating,
            #[cfg(windows)]
            window,
            cx,
        );
        div()
            .track_focus(&self.chat_focus)
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .child(chat_column)
            .child(side_panel)
            .into_any_element()
    }

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
                    "No direct chats yet — press ⌘N"
                } else {
                    "No active tab"
                })
                .into_any_element();
        };
        let preset = layout.preset;
        let session_ids = layout.session_ids();
        let grouped = pane_identity_visible(layout.root.leaves().len());
        let focused_session_id = layout.focused_session_id().map(str::to_owned);
        let focused_entry = focused_session_id
            .as_deref()
            .and_then(|session_id| self.session_entry(session_id, cx))
            .cloned();
        let focused_shell = focused_entry
            .as_ref()
            .is_some_and(|entry| entry.agent_runtime == "shell");
        let terminal_only = self.active_tab_is_terminal_only(cx);
        let focused_secondary = focused_session_id
            .as_deref()
            .is_some_and(|session_id| self.cached_chat_secondary_state(session_id).secondary);
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
            .filter(|session_id| {
                self.session_entry(session_id, cx)
                    .is_some_and(|entry| entry.agent_runtime != "shell")
            })
            .any(|session_id| self.session_lifecycle_disabled(session_id, cx));
        self.configure_chat_action_menu(&layout, lifecycle_busy, cx);
        let pane_tree = self.render_pane_node(&layout.root, &layout, window, cx);
        let picker = self.layout_picker_open.then(|| {
            self.render_layout_picker(
                preset,
                self.settings(cx).chat_panel_open && !focused_shell,
                cx,
            )
        });
        let sidebar_toggle = self.render_open_sidebar_button(cx);
        let root = cx.entity();
        let fork_root = root.clone();
        let layout_root = root.clone();
        let drawer_root = root.clone();
        let panel_root = root.clone();
        let control = (!focused_secondary)
            .then(|| self.render_topbar_session_control(&layout, window, cx))
            .flatten();
        let fork_pending = focused_session_id.as_deref().is_some_and(|session_id| {
            self.fork_confirm
                .as_ref()
                .is_some_and(|confirm| confirm.pending && confirm.session_id == session_id)
                || super::chat::fork_in_progress(&self.forking_sessions, session_id)
        });
        let fork_action = focused_session_id.clone().and_then(|session_id| {
            let (disabled, tooltip) =
                match header_fork_state(focused_entry.as_ref(), focused_secondary) {
                    HeaderForkState::Enabled if fork_pending => (true, None),
                    HeaderForkState::Enabled => (false, Some("Fork chat into a new tab")),
                    HeaderForkState::Disabled(_) if fork_pending => (true, None),
                    HeaderForkState::Disabled(caption) => (true, Some(caption)),
                    HeaderForkState::Hidden => return None,
                };
            let mut button = IconButton::new("fork-chat", "git-fork.svg")
                .disabled(disabled)
                .on_press(move |window, cx| {
                    fork_root.update(cx, |this, cx| this.fork_chat(&session_id, window, cx));
                });
            if let Some(tooltip) = tooltip {
                button = button.tooltip(tooltip);
            }
            Some(button.into_any_element())
        });
        let title_actions = (!session_ids.is_empty() && !focused_secondary)
            .then(|| self.chat_action_menu.clone().into_any_element())
            .into_iter()
            .chain(control)
            .chain(fork_action);
        let keymap_overrides = self.settings(cx).keymap_overrides.clone();
        let split_tooltip = split_panes_tooltip(&keymap_overrides);
        let layout_action = IconButton::new("layout-picker-toggle", "square-split-horizontal.svg")
            .focus_handle(self.layout_picker_focus.clone())
            .variant(if self.layout_picker_open {
                ButtonVariant::Secondary
            } else {
                ButtonVariant::Ghost
            })
            .tooltip(split_tooltip)
            .on_press(move |_, cx| {
                layout_root.update(cx, |this, cx| {
                    this.layout_picker_open = !this.layout_picker_open;
                    cx.notify();
                });
            })
            .into_any_element();
        let drawer_action = (!terminal_only).then(|| {
            let open = layout.drawer_open();
            let tooltip = terminal_drawer_tooltip(open, &keymap_overrides);
            IconButton::new(
                "terminal-drawer-toggle",
                if open {
                    "panel-bottom-open.svg"
                } else {
                    "panel-bottom-hidden.svg"
                },
            )
            .tooltip(tooltip)
            .on_press(move |window, cx| {
                drawer_root.update(cx, |this, cx| this.toggle_terminal_drawer(window, cx));
            })
            .into_any_element()
        });
        let panel_action = (!side_panel_open(self.settings(cx).chat_panel_open, focused_shell)
            && !focused_shell)
            .then(|| {
                IconButton::new("open-chat-panel", "panel-right-hidden.svg")
                    .tooltip("Open side panel")
                    .on_press(move |_, cx| {
                        panel_root.update(cx, |this, cx| {
                            this.update_app_settings(cx, true, |settings| {
                                settings.chat_panel_open = true;
                                true
                            });
                            cx.notify();
                        });
                    })
                    .into_any_element()
            });
        let header = WorkspaceHeader::new(
            px(self.workspace_titlebar_padding(window, cx)),
            if grouped {
                "square-split-horizontal.svg"
            } else if focused_shell {
                "square-terminal.svg"
            } else {
                "terminal.svg"
            },
            label,
        )
        .sidebar_toggle(sidebar_toggle)
        .title_actions(title_actions)
        .trailing_actions(
            std::iter::once(layout_action)
                .chain(drawer_action)
                .chain(panel_action),
        )
        .into_div();
        #[cfg(windows)]
        let header = header.pr(px(
            8. * self.settings(cx).app_zoom + self.chat_header_caption_inset(window, cx)
        ));

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

        let drawer = layout
            .drawer_open()
            .then(|| self.render_terminal_drawer(&layout, window, cx));
        let drawer_height = layout.drawer_height();
        let drawer_drag = DrawerResizeDrag;
        let drawer_resizing = self.drawer_resizing;
        let tab_body = div()
            .id("chat-tab-body")
            .relative()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .child(div().relative().flex_1().min_h(px(0.)).child(pane_tree))
            .children(drawer.map(|drawer| {
                div()
                    .flex_none()
                    .h(rems(drawer_height / 16.))
                    .min_h(rems(MIN_DRAWER_HEIGHT / 16.))
                    .max_h(rems(MAX_DRAWER_HEIGHT / 16.))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("terminal-drawer-resize")
                            .flex_none()
                            .h(rems(5. / 16.))
                            .w_full()
                            .flex()
                            .items_center()
                            .cursor(CursorStyle::ResizeUpDown)
                            .child(
                                div()
                                    .h(rems(if drawer_resizing { 2. / 16. } else { 1. / 16. }))
                                    .w_full()
                                    .bg(theme::border_strong()),
                            )
                            .on_drag(
                                drawer_drag,
                                |drag: &DrawerResizeDrag, _, _, cx: &mut App| {
                                    cx.new(|_| drag.clone())
                                },
                            ),
                    )
                    .child(drawer)
            }))
            .on_drag_move::<DrawerResizeDrag>(cx.listener(
                |this, event: &DragMoveEvent<DrawerResizeDrag>, _, cx| {
                    this.resize_terminal_drawer(event, cx);
                },
            ))
            .on_drop(cx.listener(|this, _: &DrawerResizeDrag, _, cx| {
                this.finish_terminal_drawer_resize(cx);
            }));
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
            .child(tab_body)
            .children(picker);
        let panel_open = side_panel_open(self.settings(cx).chat_panel_open, focused_shell);
        let (panel_visibility, panel_animating) = self.chat_panel_visibility.animate_to(
            if panel_open { 1. } else { 0. },
            Instant::now(),
            Duration::from_millis(CHAT_PANEL_TRANSITION_MS),
        );
        if panel_animating {
            window.request_animation_frame();
        }
        let side_panel = self.render_chat_side_panel(
            self.active_chat_detail.as_ref(),
            self.session_key_copy.clone(),
            panel_visibility,
            panel_open || panel_animating,
            panel_open && !panel_animating,
            #[cfg(windows)]
            window,
            cx,
        );

        div()
            .relative()
            .when(grouped, |surface| surface.key_context("ChatSplit"))
            .track_focus(&self.chat_focus)
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
                cx.listener(|this, _, _, cx| {
                    this.finish_split_resize(cx);
                    this.finish_terminal_drawer_resize(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.finish_split_resize(cx);
                    this.finish_terminal_drawer_resize(cx);
                }),
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
        let all_session_ids = layout.all_session_ids();
        let chat_session_ids = session_ids
            .iter()
            .filter(|session_id| {
                self.session_entry(session_id, cx)
                    .is_some_and(|entry| entry.agent_runtime != "shell")
            })
            .cloned()
            .collect::<Vec<_>>();
        let grouped = pane_identity_visible(layout.root.leaves().len());
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
            if !chat_session_ids.is_empty() {
                items.push(
                    UiMenuItem::new("Archive all")
                        .icon("archive.svg")
                        .destructive(true)
                        .disabled(lifecycle_busy),
                );
                actions.push(ChatMenuAction::ArchiveAll(all_session_ids));
            }
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
                if entry.agent_runtime != "shell" {
                    let archive_all = !layout.drawer_shells().is_empty();
                    items.push(
                        UiMenuItem::new(if archive_all {
                            "Archive all"
                        } else {
                            "Archive"
                        })
                        .icon("archive.svg")
                        .destructive(true)
                        .disabled(lifecycle_busy),
                    );
                    actions.push(if archive_all {
                        ChatMenuAction::ArchiveAll(all_session_ids)
                    } else {
                        ChatMenuAction::Archive(vec![session_id.clone()])
                    });
                }
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
                        .title("Stop all sessions")
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
                        .title("Resume all sessions")
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
            let shell = self
                .session_entry(&session_id, cx)
                .is_some_and(|entry| entry.agent_runtime == "shell");
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
                        .title(if shell { "Stop terminal" } else { "Stop chat" })
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
                        .title(if shell {
                            "Restart terminal"
                        } else {
                            "Resume chat"
                        })
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

    #[cfg(windows)]
    fn chat_header_caption_inset(&self, window: &Window, cx: &App) -> f32 {
        let visibility = self.chat_panel_visibility.value_at(
            Instant::now(),
            Duration::from_millis(CHAT_PANEL_TRANSITION_MS),
        );
        (self.caption_inset(window, cx)
            - self.settings(cx).chat_panel_width * self.settings(cx).app_zoom * visibility)
            .max(0.)
    }

    #[cfg_attr(windows, allow(clippy::too_many_arguments))]
    fn render_chat_side_panel(
        &self,
        detail: Option<&DirectSessionEntry>,
        session_key_copy: Entity<CopyValueButton>,
        visibility: f32,
        show_panel: bool,
        border_on: bool,
        #[cfg(windows)] window: &Window,
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
                .map(|header| {
                    #[cfg(windows)]
                    let header = header.pr(px(
                        8. * self.settings(cx).app_zoom + self.caption_inset(window, cx)
                    ));
                    header
                })
                .flex()
                .items_center()
                .justify_end()
                .border_b_1()
                .border_color(theme::border())
                .child(
                    IconButton::new("collapse-chat-panel", "panel-right-open.svg")
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
                                                .child(session_key_copy),
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
                    .map(|handle| {
                        #[cfg(windows)]
                        let handle = handle.occlude();
                        handle
                    })
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

    pub(crate) fn render_terminal_close_confirm(&self, cx: &mut Context<Self>) -> AnyElement {
        let (title, body, confirm_label, pending_label) = match self
            .terminal_close_confirm
            .as_ref()
            .map(|confirm| &confirm.target)
        {
            Some(
                TerminalCloseTarget::Tab { .. }
                | TerminalCloseTarget::Drawer { .. }
                | TerminalCloseTarget::MissionDrawer { .. },
            ) => (
                "Close terminal?",
                "A foreground process is still running. Closing this terminal will stop it."
                    .to_owned(),
                "Close terminal",
                "Closing…",
            ),
            Some(TerminalCloseTarget::ArchiveAll {
                confirmation_body, ..
            }) => (
                "Archive all?",
                confirmation_body.clone(),
                "Archive all",
                "Archiving…",
            ),
            _ => (
                "Close terminal?",
                "A foreground process is still running. Closing this pane will stop it.".to_owned(),
                "Close terminal",
                "Closing…",
            ),
        };
        let root = cx.entity();
        let confirm_root = root.clone();
        ConfirmDialog::new(
            title,
            body,
            confirm_label,
            pending_label,
            false,
            Rc::new(move |window, cx| {
                confirm_root.update(cx, |this, cx| this.confirm_terminal_close(window, cx));
            }),
            Rc::new(move |_, cx| {
                root.update(cx, |this, cx| this.cancel_terminal_close(cx));
            }),
        )
        .into_any_element()
    }

    pub(crate) fn render_fork_confirm(&self, cx: &mut Context<Self>) -> AnyElement {
        let confirm = self.fork_confirm.as_ref().expect("fork confirm is open");
        let label = self
            .session_entry(&confirm.session_id, cx)
            .map(session_label)
            .unwrap_or_else(|| "this chat".into());
        let body = format!(
            "Start a new chat from {label} with its full conversation history. The fork opens in a new tab; the original chat is not changed."
        );
        let busy = confirm.pending;
        let root = cx.entity();
        let confirm_root = root.clone();
        ConfirmDialog::new(
            "Fork chat?",
            body,
            "Fork",
            "Forking…",
            busy,
            Rc::new(move |window, cx| {
                confirm_root.update(cx, |this, cx| this.confirm_fork_chat(window, cx));
            }),
            Rc::new(move |_, cx| {
                root.update(cx, |this, cx| this.cancel_fork_chat(cx));
            }),
        )
        .icon("git-fork.svg")
        .variant(ButtonVariant::Primary)
        .into_any_element()
    }

    pub(crate) fn render_chat_rename_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let modal = self.chat_rename_modal.as_ref().expect("chat rename modal");
        let is_group = matches!(modal.target, ChatRenameTarget::Tab { .. });
        let is_shell = match &modal.target {
            ChatRenameTarget::Session { session_id, .. } => self
                .session_entry(session_id, cx)
                .is_some_and(|entry| entry.agent_runtime == "shell"),
            ChatRenameTarget::Tab { .. } => false,
        };
        let submitting = modal.submitting;
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
                            } else if is_shell {
                                "Rename terminal"
                            } else {
                                "Rename chat"
                            }),
                    )
                    .child(
                        div()
                            .text_size(rems(12. / 16.))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::muted())
                            .child(if is_group {
                                "Leave blank to derive the name from its panes."
                            } else {
                                "Leave blank to restore the default name."
                            }),
                    ),
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
                .disabled(submitting)
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
        panel_open: bool,
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
        let picker_right = if panel_open { 8. } else { 44. };
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
        entry: &DirectSessionEntry,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> Entity<PopoverMenu> {
        let session_id = &entry.session_id;
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
                            let session_id = action_target.clone();
                            action_root.update(cx, |this, cx| match index {
                                0 => this.stop_chat(&session_id, window, cx),
                                1 => {
                                    if let Some(entry) =
                                        this.session_entry(&session_id, cx).cloned()
                                    {
                                        this.begin_pane_rename(
                                            session_id,
                                            session_label(&entry),
                                            default_session_label(&entry),
                                            window,
                                            cx,
                                        );
                                    }
                                }
                                2 if this
                                    .session_entry(&session_id, cx)
                                    .is_some_and(|entry| entry.agent_runtime != "shell") =>
                                {
                                    this.archive_chat_sessions(vec![session_id], window, cx)
                                }
                                _ => {}
                            });
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
        let stop_shortcut =
            keymap::effective_binding("stop-session", &self.settings(cx).keymap_overrides)
                .map(|combo| keymap::format_combo(&combo));
        menu.update(cx, |menu, menu_cx| {
            menu.set_items(pane_action_items(entry, disabled, stop_shortcut), menu_cx)
        });
        menu
    }

    pub(crate) fn render_terminal_drawer(
        &mut self,
        layout: &PaneLayout,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let active_id = layout.active_drawer_shell().map(str::to_owned);
        let labels = layout
            .drawer_shells()
            .iter()
            .map(|session_id| {
                self.session_entry(session_id, cx)
                    .map(default_session_label)
                    .unwrap_or_else(|| "shell".into())
            })
            .collect::<Vec<_>>();
        let activate_root = cx.entity();
        let close_root = activate_root.clone();
        let add_root = activate_root.clone();
        let hide_root = activate_root.clone();
        let strip = render_terminal_drawer_strip(
            "chat",
            layout.drawer_shells(),
            active_id.as_deref(),
            &labels,
            TerminalDrawerCallbacks {
                activate: Rc::new(move |session_id, window, cx| {
                    activate_root.update(cx, |this, cx| {
                        this.activate_terminal_drawer_shell(&session_id, window, cx)
                    });
                }),
                close: Rc::new(move |session_id, window, cx| {
                    close_root.update(cx, |this, cx| {
                        this.request_close_drawer_shell(&session_id, window, cx)
                    });
                }),
                add: Rc::new(move |window, cx| {
                    add_root.update(cx, |this, cx| this.add_terminal_drawer_shell(window, cx));
                }),
                hide: Rc::new(move |window, cx| {
                    hide_root.update(cx, |this, cx| this.hide_terminal_drawer(window, cx));
                }),
            },
        );

        let body = active_id
            .as_deref()
            .and_then(|session_id| self.render_drawer_terminal(session_id, cx))
            .unwrap_or_else(|| {
                div()
                    .relative()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(rems(12. / 16.))
                    .text_color(theme::faint())
                    .child("Terminal unavailable")
                    .into_any_element()
            });
        div()
            .id("terminal-drawer")
            .track_focus(&self.drawer_focus)
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(strip)
            .child(body)
            .into_any_element()
    }

    fn render_drawer_terminal(
        &mut self,
        session_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let entry = self.session_entry(session_id, cx)?.clone();
        let session_id = entry.session_id.clone();
        let transition = self.chat_transitions.get(&session_id).map(|item| item.kind);
        let overlay = resolve_pane_overlay(
            false,
            transition,
            entry.status,
            entry.resumable,
            self.session_exit_codes.get(&session_id).copied().flatten(),
        );
        let interactive = self.cached_session_is_interactive(&session_id, cx);
        let scrollable = self.route == AppRoute::Chat && transition.is_none();
        let terminal_style = self.terminal_style(cx);
        let terminal_background =
            crate::terminal::element::to_hsla(terminal_style.palette.background, 1.);
        let terminal_surface = if let Some(chat) = self.attached.get(&session_id) {
            let terminal = Arc::clone(&chat.terminal);
            let terminal_interaction = chat.terminal_interaction.clone();
            let terminal_scrollbar = chat.terminal_scrollbar.clone();
            let terminal_input = chat.terminal_input.clone();
            let terminal_focus = chat.terminal_focus.clone();
            let resize_owner = scrollable;
            let key_session_id = session_id.clone();
            let copy_session_id = session_id.clone();
            let scroll_session_id = session_id.clone();
            let paste_session_id = session_id.clone();
            div()
                .id(SharedString::from(format!("drawer-terminal-{session_id}")))
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
                .on_action(cx.listener(move |this, action: &Copy, window, cx| {
                    this.on_terminal_copy(&copy_session_id, action, window, cx);
                }))
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
                            terminal_interaction,
                            terminal_input,
                            terminal_focus,
                            interactive,
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
                .bg(terminal_background)
                .text_size(rems(12. / 16.))
                .text_color(theme::faint())
                .when(matches!(overlay, PaneOverlayState::None), |surface| {
                    surface
                        .flex()
                        .items_center()
                        .justify_center()
                        .child("Attaching terminal…")
                })
                .into_any_element()
        };
        let overlay_element = match overlay {
            PaneOverlayState::Resuming => Some(
                SessionOverlay::transition(
                    format!("drawer-resuming-{session_id}"),
                    SessionOverlayKind::Resuming,
                )
                .label("Restarting terminal…")
                .into_any_element(),
            ),
            PaneOverlayState::Starting => Some(
                SessionOverlay::transition(
                    format!("drawer-starting-{session_id}"),
                    SessionOverlayKind::Starting,
                )
                .label("Starting terminal…")
                .into_any_element(),
            ),
            PaneOverlayState::Ended { exit_code, .. } => {
                let restart_root = cx.entity();
                let close_root = restart_root.clone();
                let restart_id = session_id.clone();
                let close_id = session_id.clone();
                Some(
                    SessionOverlay::shell_exited(
                        format!("drawer-ended-{session_id}"),
                        shell_exited_subtitle(
                            exit_code,
                            &default_session_label(&entry),
                            entry.cwd.as_deref(),
                            runner_backend::app_paths::home_dir()
                                .as_deref()
                                .and_then(|home| home.to_str()),
                        ),
                        move |window, cx| {
                            restart_root.update(cx, |this, cx| {
                                this.resume_drawer_shell(&restart_id, window, cx)
                            });
                        },
                        move |window, cx| {
                            close_root.update(cx, |this, cx| {
                                this.close_drawer_shell(&close_id, window, cx)
                            });
                        },
                    )
                    .into_any_element(),
                )
            }
            PaneOverlayState::Archiving | PaneOverlayState::None => None,
        };
        let focus_id = session_id.clone();
        Some(
            div()
                .relative()
                .flex_1()
                .min_h(px(0.))
                .overflow_hidden()
                .bg(terminal_background)
                .child(terminal_surface)
                .children(overlay_element)
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _, window, cx| {
                        this.focus_drawer_terminal(&focus_id, window, cx);
                    }),
                )
                .into_any_element(),
        )
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
        let secondary = entry
            .as_ref()
            .map(|entry| self.cached_chat_secondary_state(&entry.session_id));
        let pane_session_for_focus = entry.as_ref().map(|entry| entry.session_id.clone());
        let close_root = cx.entity();
        let header = grouped.then(|| {
            let close_pane_id = pane_id.clone();
            let close_behavior =
                pane_close_behavior(entry.as_ref().map(|entry| entry.agent_runtime.as_str()));
            let close_session_id = entry.as_ref().map(|entry| entry.session_id.clone());
            let rename_input = entry.as_ref().and_then(|entry| {
                self.pane_rename
                    .as_ref()
                    .filter(|rename| rename.session_id == entry.session_id)
                    .map(|rename| rename.input.clone())
            });
            let identity = if let Some(entry) = entry.as_ref() {
                let session_id = entry.session_id.clone();
                let label = session_label(entry);
                let placeholder = default_session_label(entry);
                let status = pane_identity_shows_status(&entry.agent_runtime).then(|| {
                    direct_chat_display_status(
                        entry,
                        self.app_store
                            .read(cx)
                            .session_activity
                            .get(&entry.session_id),
                    )
                });
                let disabled = self.session_lifecycle_disabled(&session_id, cx)
                    || secondary.as_ref().is_some_and(|state| state.secondary);
                let menu = rename_input
                    .is_none()
                    .then(|| self.pane_action_menu(entry, disabled, cx));
                let name = if let Some(input) = rename_input {
                    div()
                        .ml_2()
                        .w(rems(144. / 16.))
                        .min_w(rems(64. / 16.))
                        .child(input)
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                            match pane_rename_key(&event.keystroke.key) {
                                Some(PaneRenameKey::Submit) => {
                                    cx.stop_propagation();
                                    this.submit_pane_rename(window, cx);
                                }
                                Some(PaneRenameKey::Cancel) => {
                                    cx.stop_propagation();
                                    this.cancel_pane_rename(window, cx);
                                }
                                None => {}
                            }
                        }))
                        .into_any_element()
                } else {
                    let rename_root = cx.entity();
                    div()
                        .id(SharedString::from(format!("pane-name-{session_id}")))
                        .ml_2()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(rems(12. / 16.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(if focused {
                            theme::text()
                        } else {
                            theme::muted()
                        })
                        .child(label.clone())
                        .on_click(move |event, window, cx| {
                            if event.click_count() != 2 {
                                return;
                            }
                            cx.stop_propagation();
                            let session_id = session_id.clone();
                            let label = label.clone();
                            let placeholder = placeholder.clone();
                            rename_root.update(cx, |this, cx| {
                                this.begin_pane_rename(session_id, label, placeholder, window, cx)
                            });
                        })
                        .into_any_element()
                };
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .flex()
                    .items_center()
                    .child(
                        svg()
                            .path(pane_identity_icon(Some(&entry.agent_runtime)))
                            .size(rems(12. / 16.))
                            .flex_none()
                            .text_color(if focused {
                                theme::accent()
                            } else {
                                theme::faint()
                            }),
                    )
                    .child(name)
                    .children(status.map(render_pane_header_status))
                    .children(menu)
                    .into_any_element()
            } else {
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .flex()
                    .items_center()
                    .child(
                        svg()
                            .path(pane_identity_icon(None))
                            .size(rems(12. / 16.))
                            .flex_none()
                            .text_color(theme::faint()),
                    )
                    .child(
                        div()
                            .ml_2()
                            .text_size(rems(12. / 16.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::faint())
                            .child("Empty"),
                    )
                    .into_any_element()
            };
            div()
                .flex_none()
                .h(rems(PANE_HEADER_HEIGHT / 16.))
                .px(rems(8. / 16.))
                .flex()
                .items_center()
                .border_b_1()
                .border_color(theme::border())
                .bg(theme::panel())
                .child(identity)
                .child(
                    div()
                        .ml_2()
                        .flex_none()
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .child(
                            IconButton::new(
                                SharedString::from(format!("close-pane-{close_pane_id}")),
                                "close.svg",
                            )
                            .size(IconButtonSize::Sm)
                            .tooltip("Close pane")
                            .on_press(move |window, cx| {
                                let pane_id = close_pane_id.clone();
                                let session_id = close_session_id.clone();
                                close_root.update(cx, |this, cx| {
                                    match (close_behavior, session_id) {
                                        (PaneCloseBehavior::CloseTerminal, Some(session_id)) => {
                                            this.request_close_terminal_pane(
                                                &pane_id,
                                                &session_id,
                                                window,
                                                cx,
                                            );
                                        }
                                        (PaneCloseBehavior::LayoutOnly, _) => {
                                            this.close_pane(&pane_id, window, cx);
                                        }
                                        (PaneCloseBehavior::CloseTerminal, None) => {}
                                    }
                                });
                            }),
                        ),
                )
        });

        let body: AnyElement = if let Some(entry) = entry.as_ref() {
            let session_id = entry.session_id.clone();
            let secondary_state = secondary.clone().unwrap_or_default();
            let fork_materializing =
                super::chat::fork_materializing(&self.forking_sessions, &session_id);
            let transition = if fork_materializing {
                Some(TransitionKind::Starting)
            } else {
                self.chat_transitions.get(&session_id).map(|item| item.kind)
            };
            let overlay = resolve_pane_overlay(
                self.sidebar_archiving_session(&session_id, cx),
                transition,
                entry.status,
                entry.resumable,
                self.session_exit_codes.get(&session_id).copied().flatten(),
            );
            let interactive = self.cached_session_is_interactive(&session_id, cx);
            let scrollable = self.route == AppRoute::Chat
                && transition.is_none()
                && !self.sidebar_archiving_session(&session_id, cx);
            let terminal_style = self.terminal_style(cx);
            let terminal_background =
                crate::terminal::element::to_hsla(terminal_style.palette.background, 1.);
            let terminal_surface = if secondary_state.secondary {
                div()
                    .absolute()
                    .inset_0()
                    .bg(terminal_background)
                    .into_any_element()
            } else if let Some(chat) = self.attached.get(&session_id) {
                let terminal = Arc::clone(&chat.terminal);
                let terminal_interaction = chat.terminal_interaction.clone();
                let terminal_scrollbar = chat.terminal_scrollbar.clone();
                let terminal_input = chat.terminal_input.clone();
                let terminal_focus = chat.terminal_focus.clone();
                let resize_owner = scrollable && layout.is_resize_owner(&pane_id, &session_id);
                let key_session_id = session_id.clone();
                let copy_session_id = session_id.clone();
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
                    .on_action(cx.listener(move |this, action: &Copy, window, cx| {
                        this.on_terminal_copy(&copy_session_id, action, window, cx);
                    }))
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
                                terminal_interaction,
                                terminal_input,
                                terminal_focus,
                                interactive,
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
                    .bg(terminal_background)
                    .text_size(rems(12. / 16.))
                    .text_color(theme::faint())
                    .when(matches!(overlay, PaneOverlayState::None), |surface| {
                        surface
                            .flex()
                            .items_center()
                            .justify_center()
                            .child("Attaching terminal…")
                    })
                    .into_any_element()
            };
            let overlay_element = if secondary_state.secondary
                && !self.dismissed_duplicate_chats.contains(&session_id)
            {
                let focus_label = secondary_state.primary_label.clone();
                let stay_root = cx.entity();
                let stay_session_id = session_id.clone();
                Some(
                    DuplicateSubjectOverlay::new(
                        SharedString::from(format!("duplicate-chat-{session_id}")),
                        DuplicateSubjectKind::Chat,
                        focus_label.is_some(),
                        move |_, cx| {
                            if let Some(label) = focus_label.as_deref() {
                                focus_other_window(label, cx);
                            }
                        },
                        move |_, cx| {
                            stay_root.update(cx, |this, cx| {
                                this.dismiss_duplicate_chat(&stay_session_id, cx)
                            });
                        },
                    )
                    .into_any_element(),
                )
            } else {
                match overlay {
                    PaneOverlayState::Archiving => Some(
                        SessionOverlay::transition(
                            format!("archiving-{session_id}"),
                            SessionOverlayKind::Archiving,
                        )
                        .into_any_element(),
                    ),
                    PaneOverlayState::Resuming => Some(if entry.agent_runtime == "shell" {
                        SessionOverlay::transition(
                            format!("resuming-{session_id}"),
                            SessionOverlayKind::Resuming,
                        )
                        .label("Restarting terminal…")
                        .into_any_element()
                    } else {
                        SessionOverlay::transition(
                            format!("resuming-{session_id}"),
                            SessionOverlayKind::Resuming,
                        )
                        .into_any_element()
                    }),
                    PaneOverlayState::Starting => {
                        let overlay = SessionOverlay::transition(
                            format!("starting-{session_id}"),
                            SessionOverlayKind::Starting,
                        );
                        Some(
                            match starting_overlay_label(&entry.agent_runtime, fork_materializing) {
                                Some(label) => overlay.label(label).into_any_element(),
                                None => overlay.into_any_element(),
                            },
                        )
                    }
                    PaneOverlayState::Ended {
                        status,
                        resumable,
                        exit_code,
                    } => {
                        if entry.agent_runtime == "shell" {
                            let restart_root = cx.entity();
                            let close_root = restart_root.clone();
                            let restart_id = session_id.clone();
                            let close_id = session_id.clone();
                            let restart_pane = pane_id.clone();
                            let close_pane = pane_id.clone();
                            Some(
                                SessionOverlay::shell_exited(
                                    format!("ended-{session_id}"),
                                    shell_exited_subtitle(
                                        exit_code,
                                        &default_session_label(entry),
                                        entry.cwd.as_deref(),
                                        runner_backend::app_paths::home_dir()
                                            .as_deref()
                                            .and_then(|home| home.to_str()),
                                    ),
                                    move |window, cx| {
                                        restart_root.update(cx, |this, cx| {
                                            this.resume_chat(&restart_pane, &restart_id, window, cx)
                                        });
                                    },
                                    move |window, cx| {
                                        close_root.update(cx, |this, cx| {
                                            this.close_terminal_pane(
                                                &close_pane,
                                                &close_id,
                                                window,
                                                cx,
                                            )
                                        });
                                    },
                                )
                                .into_any_element(),
                            )
                        } else {
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
                                            this.archive_chat_sessions(
                                                vec![archive_id.clone()],
                                                window,
                                                cx,
                                            )
                                        });
                                    },
                                )
                                .into_any_element(),
                            )
                        }
                    }
                    PaneOverlayState::None => None,
                }
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
            let new_chat_root = cx.entity();
            let new_chat_label = empty_pane_action_label(&self.settings(cx).keymap_overrides);
            div()
                .flex_1()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(rems(10. / 16.))
                .p_4()
                .child(
                    div()
                        .text_size(rems(13. / 16.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::muted())
                        .child("No session in this pane"),
                )
                .child(
                    Button::new(
                        SharedString::from(format!("new-chat-{pane_id}")),
                        new_chat_label,
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Primary)
                    .on_press(move |window, cx| {
                        new_chat_root.update(cx, |this, cx| {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PaneCloseBehavior {
    LayoutOnly,
    CloseTerminal,
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

fn pane_identity_visible(pane_count: usize) -> bool {
    pane_count > 1
}

fn empty_pane_action_label(overrides: &keymap::KeymapOverrides) -> String {
    keymap::effective_binding("new-chat", overrides).map_or_else(
        || "New chat".to_owned(),
        |combo| format!("{}  New chat", keymap::format_combo(&combo)),
    )
}

pub(crate) fn terminal_drawer_tooltip(open: bool, overrides: &keymap::KeymapOverrides) -> String {
    let label = if open {
        "Hide terminal drawer"
    } else {
        "Show terminal drawer"
    };
    keymap::effective_binding("toggle-terminal-drawer", overrides).map_or_else(
        || label.to_owned(),
        |combo| format!("{label} · {}", keymap::format_combo(&combo)),
    )
}

fn split_panes_tooltip(overrides: &keymap::KeymapOverrides) -> String {
    let shortcuts = ["split-pane-right", "split-pane-down"]
        .into_iter()
        .filter_map(|id| keymap::effective_binding(id, overrides))
        .map(|combo| keymap::format_combo(&combo))
        .collect::<Vec<_>>();
    if shortcuts.is_empty() {
        "Split panes".to_owned()
    } else {
        format!("Split panes · {}", shortcuts.join(" / "))
    }
}

fn pane_identity_icon(runtime: Option<&str>) -> &'static str {
    match runtime {
        Some("shell") => "square-terminal.svg",
        Some(_) => "terminal.svg",
        None => "square-dashed.svg",
    }
}

fn pane_identity_shows_status(runtime: &str) -> bool {
    runtime != "shell"
}

fn starting_overlay_label(runtime: &str, fork_materializing: bool) -> Option<&'static str> {
    if runtime == "shell" {
        Some("Starting terminal…")
    } else if fork_materializing {
        Some("Forking chat…")
    } else {
        None
    }
}

fn side_panel_open(setting_open: bool, focused_shell: bool) -> bool {
    setting_open && !focused_shell
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HeaderForkState {
    Enabled,
    Disabled(&'static str),
    Hidden,
}

fn header_fork_state(
    focused: Option<&DirectSessionEntry>,
    focused_secondary: bool,
) -> HeaderForkState {
    let Some(entry) = focused.filter(|entry| entry.agent_runtime != "shell") else {
        return HeaderForkState::Hidden;
    };
    if focused_secondary {
        return HeaderForkState::Hidden;
    }
    if !entry.native_fork {
        HeaderForkState::Disabled("Forking needs claude-code or codex")
    } else if !entry.forkable {
        HeaderForkState::Disabled("No session key captured yet")
    } else {
        HeaderForkState::Enabled
    }
}

fn pane_close_behavior(runtime: Option<&str>) -> PaneCloseBehavior {
    if runtime == Some("shell") {
        PaneCloseBehavior::CloseTerminal
    } else {
        PaneCloseBehavior::LayoutOnly
    }
}

fn pane_action_items(
    entry: &DirectSessionEntry,
    disabled: bool,
    stop_shortcut: Option<String>,
) -> Vec<UiMenuItem> {
    pane_action_items_for(
        &entry.agent_runtime,
        entry.status == SessionStatus::Running,
        disabled,
        stop_shortcut,
    )
}

fn pane_action_items_for(
    runtime: &str,
    running: bool,
    disabled: bool,
    stop_shortcut: Option<String>,
) -> Vec<UiMenuItem> {
    let stop = UiMenuItem::new("Stop")
        .icon("square.svg")
        .disabled(disabled || !running);
    let stop = match stop_shortcut {
        Some(shortcut) => stop.shortcut(shortcut),
        None => stop,
    };
    let mut items = vec![
        stop,
        UiMenuItem::new("Rename…")
            .icon("pencil.svg")
            .disabled(disabled),
    ];
    if runtime != "shell" {
        items.push(
            UiMenuItem::new("Archive chat")
                .icon("archive.svg")
                .separator_before(true)
                .destructive(true)
                .disabled(disabled),
        );
    }
    items
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

fn render_pane_header_status(status: DirectChatDisplayStatus) -> AnyElement {
    let color = match status {
        DirectChatDisplayStatus::Busy => theme::warning(),
        DirectChatDisplayStatus::Idle => theme::accent(),
        DirectChatDisplayStatus::Stopped => theme::faint(),
        DirectChatDisplayStatus::Crashed => theme::danger(),
    };
    div()
        .ml_2()
        .flex_none()
        .size(rems(5. / 16.))
        .rounded_full()
        .bg(color)
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
    use super::{
        adjacent_pane_index, empty_pane_action_label, header_fork_state, pane_action_items_for,
        pane_close_behavior, pane_identity_icon, pane_identity_shows_status, pane_identity_visible,
        pane_rename_key, side_panel_open, split_panes_tooltip, starting_overlay_label,
        terminal_drawer_tooltip, HeaderForkState, PaneCloseBehavior, PaneRenameKey,
    };
    use crate::keymap;
    use runner_backend::model::SessionStatus;
    use runner_backend::ops::session::DirectSessionEntry;

    fn direct_session(runtime: &str, native_fork: bool, forkable: bool) -> DirectSessionEntry {
        DirectSessionEntry {
            session_id: format!("{runtime}-session"),
            project_id: None,
            runner_id: None,
            handle: None,
            agent_runtime: runtime.into(),
            agent_command: runtime.into(),
            display_name: runtime.into(),
            status: SessionStatus::Running,
            title: None,
            cwd: None,
            started_at: None,
            stopped_at: None,
            resumable: forkable,
            native_fork,
            forkable,
            agent_session_key: forkable.then(|| "key".into()),
            pinned: false,
            archived_at: None,
        }
    }

    #[test]
    fn adjacent_pane_index_wraps_in_both_directions() {
        assert_eq!(adjacent_pane_index(0, 3, -1), Some(2));
        assert_eq!(adjacent_pane_index(2, 3, 1), Some(0));
        assert_eq!(adjacent_pane_index(1, 3, 1), Some(2));
        assert_eq!(adjacent_pane_index(0, 1, 1), None);
        assert_eq!(adjacent_pane_index(3, 3, -1), None);
    }

    #[test]
    fn header_fork_state_uses_capability_key_and_focused_pane_kind() {
        for runtime in ["claude-code", "codex"] {
            let entry = direct_session(runtime, true, true);
            assert_eq!(
                header_fork_state(Some(&entry), false),
                HeaderForkState::Enabled
            );
        }

        let trae = direct_session("trae", false, false);
        assert_eq!(
            header_fork_state(Some(&trae), false),
            HeaderForkState::Disabled("Forking needs claude-code or codex")
        );
        let waiting = direct_session("codex", true, false);
        assert_eq!(
            header_fork_state(Some(&waiting), false),
            HeaderForkState::Disabled("No session key captured yet")
        );
        let shell = direct_session("shell", false, false);
        assert_eq!(
            header_fork_state(Some(&shell), false),
            HeaderForkState::Hidden
        );
        assert_eq!(header_fork_state(None, false), HeaderForkState::Hidden);
        assert_eq!(
            header_fork_state(Some(&waiting), true),
            HeaderForkState::Hidden
        );
    }

    #[test]
    fn starting_overlay_names_fork_materialization_before_normal_startup() {
        assert_eq!(
            starting_overlay_label("claude-code", true),
            Some("Forking chat…")
        );
        assert_eq!(starting_overlay_label("codex", false), None);
        assert_eq!(
            starting_overlay_label("shell", false),
            Some("Starting terminal…")
        );
    }

    #[test]
    fn pane_identity_branches_for_chat_terminal_and_empty_panes() {
        assert_eq!(pane_identity_icon(Some("codex")), "terminal.svg");
        assert!(pane_identity_shows_status("codex"));

        assert_eq!(pane_identity_icon(Some("shell")), "square-terminal.svg");
        assert!(!pane_identity_shows_status("shell"));
        assert!(!side_panel_open(true, true));

        assert_eq!(pane_identity_icon(None), "square-dashed.svg");
        assert_eq!(pane_close_behavior(None), PaneCloseBehavior::LayoutOnly);
    }

    #[test]
    fn pane_actions_never_offer_close_and_terminals_cannot_be_archived() {
        let chat = pane_action_items_for("codex", true, false, Some("⌘.".to_owned()));
        assert_eq!(
            chat.iter()
                .map(|item| item.label.as_ref())
                .collect::<Vec<_>>(),
            ["Stop", "Rename…", "Archive chat"]
        );
        assert_eq!(
            chat[0].shortcut.as_ref().map(|shortcut| shortcut.as_ref()),
            Some("⌘.")
        );
        assert!(chat[2].separator_before);
        assert!(chat.iter().all(|item| item.label.as_ref() != "Close pane"));

        let terminal = pane_action_items_for("shell", true, false, Some("⌘.".to_owned()));
        assert_eq!(
            terminal
                .iter()
                .map(|item| item.label.as_ref())
                .collect::<Vec<_>>(),
            ["Stop", "Rename…"]
        );
        assert_eq!(
            pane_close_behavior(Some("shell")),
            PaneCloseBehavior::CloseTerminal
        );
        assert_eq!(
            pane_close_behavior(Some("codex")),
            PaneCloseBehavior::LayoutOnly
        );
    }

    #[test]
    fn pane_identity_renders_only_for_split_tabs() {
        assert!(!pane_identity_visible(1));
        assert!(pane_identity_visible(2));
        assert!(pane_identity_visible(3));
    }

    #[test]
    fn empty_pane_offers_only_the_chat_entry_point() {
        let mut overrides = keymap::KeymapOverrides::new();
        assert_eq!(empty_pane_action_label(&overrides), "⌘N  New chat");

        let mut rebound = keymap::entry("new-chat").unwrap().default.clone();
        rebound.meta = false;
        rebound.ctrl = true;
        overrides.insert("new-chat".into(), Some(rebound));
        assert_eq!(empty_pane_action_label(&overrides), "⌃N  New chat");

        overrides.insert("new-chat".into(), None);
        assert_eq!(empty_pane_action_label(&overrides), "New chat");
    }

    #[test]
    fn terminal_drawer_tooltip_tracks_rebound_and_unbound_shortcuts() {
        let mut overrides = keymap::KeymapOverrides::new();
        assert_eq!(
            terminal_drawer_tooltip(false, &overrides),
            "Show terminal drawer · ⌥F12"
        );
        assert_eq!(
            terminal_drawer_tooltip(true, &overrides),
            "Hide terminal drawer · ⌥F12"
        );

        let mut rebound = keymap::entry("toggle-terminal-drawer")
            .unwrap()
            .default
            .clone();
        rebound.alt = false;
        rebound.ctrl = true;
        rebound.code = "Backquote".into();
        overrides.insert("toggle-terminal-drawer".into(), Some(rebound));
        assert_eq!(
            terminal_drawer_tooltip(false, &overrides),
            "Show terminal drawer · ⌃`"
        );

        overrides.insert("toggle-terminal-drawer".into(), None);
        assert_eq!(
            terminal_drawer_tooltip(false, &overrides),
            "Show terminal drawer"
        );
    }

    #[test]
    fn split_panes_tooltip_tracks_rebound_and_unbound_shortcuts() {
        let mut overrides = keymap::KeymapOverrides::new();
        assert_eq!(split_panes_tooltip(&overrides), "Split panes · ⌘D / ⇧⌘D");

        let mut rebound = keymap::entry("split-pane-right").unwrap().default.clone();
        rebound.meta = false;
        rebound.ctrl = true;
        overrides.insert("split-pane-right".into(), Some(rebound));
        overrides.insert("split-pane-down".into(), None);
        assert_eq!(split_panes_tooltip(&overrides), "Split panes · ⌃D");

        overrides.insert("split-pane-right".into(), None);
        assert_eq!(split_panes_tooltip(&overrides), "Split panes");
    }

    #[test]
    fn pane_rename_keys_submit_or_revert_inline_edits() {
        assert_eq!(pane_rename_key("enter"), Some(PaneRenameKey::Submit));
        assert_eq!(pane_rename_key("escape"), Some(PaneRenameKey::Cancel));
        assert_eq!(pane_rename_key("tab"), None);
    }
}
