//! Chat-surface rendering: the active tab, layout picker, and pane tree.
use super::*;

impl NativeRoot {
    pub(crate) fn render_active_tab(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(layout) = self.tabs.active().cloned() else {
            return div()
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .justify_center()
                .text_color(theme::muted())
                .child(if self.sessions.is_empty() {
                    "No direct chats yet — press ⌘T"
                } else {
                    "No active tab"
                })
                .into_any_element();
        };
        let label = self.tab_label(&layout);
        let preset = layout.preset;
        let pane_tree = self.render_pane_node(&layout.root, &layout, window, cx);
        let picker = self
            .layout_picker_open
            .then(|| self.render_layout_picker(preset, cx));

        div()
            .relative()
            .flex_1()
            .min_w(px(0.))
            .h_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_none()
                    .h(px(WORKSPACE_HEADER_HEIGHT))
                    .px_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .border_b_1()
                    .border_color(theme::border())
                    .child(
                        div()
                            .min_w(px(0.))
                            .text_sm()
                            .text_color(theme::text())
                            .child(label),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .id("layout-picker-toggle")
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(if self.layout_picker_open {
                                        theme::accent()
                                    } else {
                                        theme::border()
                                    })
                                    .cursor_pointer()
                                    .text_xs()
                                    .text_color(theme::text())
                                    .hover(|button| button.bg(theme::border()))
                                    .child("Layout")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.layout_picker_open = !this.layout_picker_open;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id("new-tab-header")
                                    .px_2()
                                    .py_1()
                                    .rounded_md()
                                    .cursor_pointer()
                                    .text_xs()
                                    .text_color(theme::muted())
                                    .hover(|button| button.bg(theme::border()))
                                    .child("New tab  ⌘T")
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.begin_new_tab(&NewTab, window, cx);
                                    })),
                            ),
                    ),
            )
            .child(div().flex_1().min_h(px(0.)).p_1().child(pane_tree))
            .children(picker)
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.finish_split_resize()),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.finish_split_resize()),
            )
            .into_any_element()
    }

    pub(crate) fn render_layout_picker(
        &self,
        active: PresetKind,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .absolute()
            .top(px(WORKSPACE_HEADER_HEIGHT + 4.))
            .right(px(12.))
            .w(px(244.))
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(theme::border())
            .bg(theme::composer_bg())
            .shadow_lg()
            .child(
                div()
                    .pb_2()
                    .text_xs()
                    .text_color(theme::muted())
                    .child("LAYOUT"),
            )
            .child(
                div().flex().flex_wrap().gap_2().children(
                    PresetKind::ALL
                        .into_iter()
                        .enumerate()
                        .map(|(index, preset)| {
                            div()
                                .id(("layout-preset", index))
                                .w(px(104.))
                                .px_2()
                                .py_2()
                                .rounded_md()
                                .border_1()
                                .border_color(if preset == active {
                                    theme::accent()
                                } else {
                                    theme::border()
                                })
                                .cursor_pointer()
                                .text_xs()
                                .text_color(if preset == active {
                                    theme::accent()
                                } else {
                                    theme::text()
                                })
                                .hover(|tile| tile.bg(theme::border()))
                                .child(preset.label())
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.pick_preset(preset, window, cx);
                                }))
                        }),
                ),
            )
            .child(
                div()
                    .pt_2()
                    .text_xs()
                    .text_color(theme::muted())
                    .child("Drag pane dividers to resize"),
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
                            .w(px(5.))
                            .h_full()
                            .cursor(CursorStyle::ResizeLeftRight)
                    })
                    .when(orientation == SplitOrientation::Column, |gutter| {
                        gutter.h(px(5.)).w_full().cursor(CursorStyle::ResizeUpDown)
                    })
                    .bg(theme::border())
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
                    .on_drop(cx.listener(|this, _: &SplitResizeDrag, _, _| {
                        this.finish_split_resize();
                    }))
                    .into_any_element()
            }
        }
    }

    pub(crate) fn render_pane(
        &mut self,
        leaf: &PaneLeaf,
        layout: &PaneLayout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let focused = layout.focused_pane_id == leaf.id;
        let grouped = layout.root.leaves().len() > 1;
        let pane_id = leaf.id.clone();
        let pane_id_for_focus = pane_id.clone();
        let header = grouped.then(|| {
            let label = leaf
                .session_id
                .as_deref()
                .and_then(|session_id| self.session_entry(session_id))
                .map(session_label)
                .unwrap_or_else(|| "Empty pane".into());
            div()
                .flex_none()
                .h(px(PANE_HEADER_HEIGHT))
                .px_3()
                .flex()
                .items_center()
                .border_b_1()
                .border_color(theme::border())
                .bg(theme::composer_bg())
                .child(
                    div()
                        .mr_2()
                        .text_xs()
                        .text_color(if focused {
                            theme::accent()
                        } else {
                            theme::muted()
                        })
                        .child("●"),
                )
                .child(
                    div()
                        .min_w(px(0.))
                        .text_sm()
                        .text_color(theme::text())
                        .child(label),
                )
        });

        let body: AnyElement = if let Some(session_id) = leaf.session_id.as_deref() {
            let status = self.session_entry(session_id).map(|entry| entry.status);
            if let Some(chat) = self.attached.get(session_id) {
                let terminal = Arc::clone(&chat.terminal);
                let composer = chat.composer.clone();
                let terminal_focus = chat.terminal_focus.clone();
                let terminal_focused = terminal_focus.is_focused(window);
                let key_session_id = session_id.to_owned();
                let scroll_session_id = session_id.to_owned();
                let paste_session_id = session_id.to_owned();
                let click_session_id = session_id.to_owned();
                let click_pane_id = pane_id.clone();
                let content = div().flex_1().min_h(px(0.)).flex().flex_col().child(
                    div()
                        .id(SharedString::from(format!("terminal-{session_id}")))
                        .key_context("Terminal")
                        .track_focus(&terminal_focus)
                        .flex_1()
                        .min_h(px(0.))
                        .p_2()
                        .on_key_down(cx.listener(move |this, event, window, cx| {
                            this.on_key_down(&key_session_id, event, window, cx);
                        }))
                        .on_scroll_wheel(cx.listener(move |this, event, window, cx| {
                            this.on_scroll(&scroll_session_id, event, window, cx);
                        }))
                        .on_action(cx.listener(move |this, action, window, cx| {
                            this.on_paste(&paste_session_id, action, window, cx);
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _, window, cx| {
                                this.focus_terminal(&click_pane_id, &click_session_id, window, cx);
                            }),
                        )
                        .child(TerminalElement::new(terminal, terminal_focused)),
                );
                if status == Some(SessionStatus::Running) {
                    content.child(composer).into_any_element()
                } else {
                    let resume_pane_id = pane_id.clone();
                    let resume_session_id = session_id.to_owned();
                    let status_label = match status {
                        Some(SessionStatus::Crashed) => "Chat crashed",
                        _ => "Chat stopped",
                    };
                    content
                        .child(
                            div()
                                .flex_none()
                                .px_3()
                                .py_2()
                                .flex()
                                .items_center()
                                .justify_between()
                                .border_t_1()
                                .border_color(theme::border())
                                .bg(theme::composer_bg())
                                .text_sm()
                                .text_color(theme::muted())
                                .child(status_label)
                                .child(
                                    div()
                                        .id(SharedString::from(format!("resume-chat-{session_id}")))
                                        .px_2()
                                        .py_1()
                                        .rounded_md()
                                        .border_1()
                                        .border_color(theme::border())
                                        .cursor_pointer()
                                        .text_sm()
                                        .text_color(theme::accent())
                                        .hover(|button| button.bg(theme::border()))
                                        .child("Resume")
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.resume_chat(
                                                &resume_pane_id,
                                                &resume_session_id,
                                                window,
                                                cx,
                                            );
                                        })),
                                ),
                        )
                        .into_any_element()
                }
            } else if status != Some(SessionStatus::Running) {
                let resume_pane_id = pane_id.clone();
                let resume_session_id = session_id.to_owned();
                let status_label = match status {
                    Some(SessionStatus::Crashed) => "Chat crashed",
                    _ => "Chat stopped",
                };
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_color(theme::muted())
                    .child(status_label)
                    .child(
                        div()
                            .id(SharedString::from(format!("resume-chat-{session_id}")))
                            .px_3()
                            .py_2()
                            .rounded_md()
                            .border_1()
                            .border_color(theme::border())
                            .cursor_pointer()
                            .text_sm()
                            .text_color(theme::accent())
                            .hover(|button| button.bg(theme::border()))
                            .child("Resume")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.resume_chat(&resume_pane_id, &resume_session_id, window, cx);
                            })),
                    )
                    .into_any_element()
            } else {
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_color(theme::muted())
                    .child("Unable to attach chat")
                    .into_any_element()
            }
        } else {
            let new_chat_pane_id = pane_id.clone();
            div()
                .flex_1()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .id(SharedString::from(format!("new-chat-{pane_id}")))
                        .px_3()
                        .py_2()
                        .rounded_md()
                        .border_1()
                        .border_color(theme::border())
                        .cursor_pointer()
                        .text_sm()
                        .text_color(theme::accent())
                        .hover(|button| button.bg(theme::border()))
                        .child("New chat")
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.begin_pane_chat(&new_chat_pane_id, cx);
                        })),
                )
                .into_any_element()
        };

        div()
            .id(SharedString::from(format!("pane-{pane_id}")))
            .size_full()
            .min_w(px(0.))
            .min_h(px(0.))
            .flex()
            .flex_col()
            .border_1()
            .border_color(if focused {
                theme::accent()
            } else {
                theme::border()
            })
            .children(header)
            .child(body)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _, cx| {
                    this.focus_pane(&pane_id_for_focus, cx);
                }),
            )
            .into_any_element()
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
