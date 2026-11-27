use super::*;

impl ChangesView {
    pub(super) fn render_branch_header(
        &self,
        state: &RootChanges,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let viewing_other = is_viewing_other_checkout(&state.root);
        let worktrees_slice = state
            .commits
            .as_ref()
            .map(|c| c.worktrees.as_slice())
            .filter(|w| !w.is_empty())
            .or_else(|| state.root.git.as_ref().map(|g| g.worktrees.as_slice()))
            .unwrap_or(&[]);
        let multiple_checkouts = live_worktrees(worktrees_slice).len() > 1 || viewing_other;

        let (branch_name, commit_count) = match &state.commits {
            Some(commits) => (
                branch_header_label(commits),
                commits.base.is_some().then(|| {
                    if viewing_other {
                        commits.total.to_string()
                    } else {
                        crate::i18n::tn("changes.commits", commits.total)
                    }
                }),
            ),
            None => (
                state
                    .root
                    .git
                    .as_ref()
                    .and_then(|g| g.branch.clone())
                    .unwrap_or_else(|| state.root.name.clone()),
                None,
            ),
        };

        let removed_notice = self.removed_checkout_notice.as_ref().map(|name| {
            let notice_key = match self.context.surface {
                PanelSurface::Chat => "changes.checkout_removed_chat",
                PanelSurface::Terminal => "changes.checkout_removed_terminal",
                PanelSurface::Mission => "changes.checkout_removed_mission",
            };
            div()
                .when(cfg!(test), |el| {
                    el.debug_selector(|| "CHANGES_REMOVED_CHECKOUT_NOTICE".into())
                })
                .px(px(10.))
                .pb(px(6.))
                .flex()
                .items_start()
                .gap(px(6.))
                .child(
                    svg()
                        .flex_none()
                        .mt(px(2.))
                        .size(px(12.))
                        .path("triangle-alert.svg")
                        .text_color(theme::warning()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(theme::text_meta())
                        .line_height(relative(1.4))
                        .text_color(theme::warning())
                        .child(crate::i18n::tf(notice_key, &[("name", name.clone())])),
                )
        });

        if !multiple_checkouts {
            let plain_row = div()
                .when(cfg!(test), |el| {
                    el.debug_selector(|| "CHANGES_BRANCH_HEADER".into())
                })
                .flex_none()
                .h(px(36.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    svg()
                        .flex_none()
                        .size(px(13.))
                        .path("git-branch.svg")
                        .text_color(theme::muted()),
                )
                .child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(theme::text_ui())
                        .text_color(theme::text())
                        .child(branch_name),
                )
                .child(div().flex_1())
                .children(commit_count.map(|count| {
                    div()
                        .flex_none()
                        .text_size(theme::text_meta())
                        .text_color(theme::faint())
                        .child(count)
                }));
            return div()
                .flex_none()
                .flex()
                .flex_col()
                .border_b_1()
                .border_color(theme::border())
                .child(plain_row)
                .children(removed_notice)
                .into_any_element();
        }

        let is_main = state
            .root
            .git
            .as_ref()
            .is_some_and(|g| g.worktree == WorktreeKind::Main);
        let kind_icon = if is_main {
            "house.svg"
        } else {
            "folder-git-2.svg"
        };
        let kind_selector = if is_main {
            "CHANGES_CHECKOUT_KIND_MAIN"
        } else {
            "CHANGES_CHECKOUT_KIND_LINKED"
        };
        let picker_open = self.checkout_picker.is_some();
        let button_focused = self.checkout_button_focus.is_focused(window);
        let anchor = self.checkout_picker_anchor.clone();
        let branch_button = div()
            .id("changes-checkout-button")
            .when(cfg!(test), |el| {
                el.debug_selector(|| "CHANGES_CHECKOUT_BUTTON".into())
            })
            .track_focus(&self.checkout_button_focus)
            .tab_index(0)
            .tab_stop(true)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let m = event.keystroke.modifiers;
                if m.platform || m.control || m.alt {
                    return;
                }
                if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                    cx.stop_propagation();
                    this.toggle_checkout_picker(window, cx);
                }
            }))
            .relative()
            .min_w(px(0.))
            .flex_shrink(1.)
            .py(px(2.))
            .pl(px(4.))
            .pr(px(5.))
            .flex()
            .items_center()
            .gap(px(4.))
            .rounded(px(5.))
            .border_1()
            .border_color(if picker_open || button_focused {
                theme::accent()
            } else {
                gpui::transparent_black()
            })
            .when(picker_open, |el| {
                el.bg(theme::with_alpha(theme::sidebar_selected(), 0.6))
            })
            .hover(|el| el.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| this.toggle_checkout_picker(window, cx)))
            .child(
                canvas(move |bounds, _, _| anchor.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .inset_0(),
            )
            .child(
                div()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(theme::text_ui())
                    .text_color(theme::text())
                    .child(branch_name),
            )
            .child(
                svg()
                    .flex_none()
                    .size(px(11.))
                    .path("chevron-down.svg")
                    .text_color(theme::muted()),
            );

        let return_tip_key = match self.context.surface {
            PanelSurface::Chat => "changes.back_to_chat_checkout",
            PanelSurface::Terminal => "changes.back_to_terminal_checkout",
            PanelSurface::Mission => "changes.back_to_mission_checkout",
        };
        let return_focused = self.checkout_return_focus.is_focused(window);
        let return_button = viewing_other.then(|| {
            div()
                .id("changes-checkout-return")
                .when(cfg!(test), |el| {
                    el.debug_selector(|| "CHANGES_CHECKOUT_RETURN".into())
                })
                .track_focus(&self.checkout_return_focus)
                .tab_index(0)
                .tab_stop(true)
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    let m = event.keystroke.modifiers;
                    if m.platform || m.control || m.alt {
                        return;
                    }
                    if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                        cx.stop_propagation();
                        this.set_root(None, cx);
                        this.checkout_button_focus.focus(window, cx);
                    }
                }))
                .flex_none()
                .size(px(22.))
                .rounded(px(4.))
                .border_1()
                .border_color(if return_focused {
                    theme::accent()
                } else {
                    gpui::transparent_black()
                })
                .flex()
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|el| el.bg(theme::with_alpha(theme::raised(), 0.7)))
                .tooltip(move |_, cx| tooltip_view(crate::i18n::t(return_tip_key), cx))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(|this, _, _, cx| this.set_root(None, cx)))
                .child(
                    svg()
                        .size(px(13.))
                        .path("undo-2.svg")
                        .text_color(theme::muted()),
                )
        });

        let row = div()
            .when(cfg!(test), |el| {
                el.debug_selector(move || {
                    if viewing_other {
                        "CHANGES_BRANCH_HEADER other".into()
                    } else {
                        "CHANGES_BRANCH_HEADER".into()
                    }
                })
            })
            .flex_none()
            .h(px(36.))
            .px(px(10.))
            .flex()
            .items_center()
            .gap(px(5.))
            .when(viewing_other, |el| el.bg(theme::sidebar_selected()))
            .child(
                div()
                    .when(cfg!(test), move |el| {
                        el.debug_selector(move || kind_selector.into())
                    })
                    .flex_none()
                    .flex()
                    .items_center()
                    .child(
                        svg()
                            .size(px(13.))
                            .path(kind_icon)
                            .text_color(theme::muted()),
                    ),
            )
            .child(branch_button)
            .child(div().flex_1())
            .children(commit_count.map(|count| {
                div()
                    .when(cfg!(test), {
                        let c = count.clone();
                        move |el| el.debug_selector(move || format!("CHANGES_BRANCH_COUNT {c}"))
                    })
                    .flex_none()
                    .text_size(theme::text_meta())
                    .text_color(theme::faint())
                    .child(count)
            }))
            .children(return_button);

        div()
            .flex_none()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(theme::border())
            .child(row)
            .children(removed_notice)
            .into_any_element()
    }

    pub(super) fn render_base_row(
        &self,
        state: &RootChanges,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (chip_text, is_custom) =
            base_chip_label(state.chosen_base.as_deref(), state.commits.as_ref());
        let picker_open = self.base_picker.is_some();
        let chip_focused = self.base_chip_focus.is_focused(window);
        let anchor = self.base_picker_anchor.clone();
        let chip = div()
            .id("changes-base-chip")
            .when(cfg!(test), {
                let text = chip_text.clone();
                move |el| el.debug_selector(move || format!("CHANGES_BASE_CHIP {text}"))
            })
            .track_focus(&self.base_chip_focus)
            .tab_index(0)
            .tab_stop(true)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let m = event.keystroke.modifiers;
                if m.platform || m.control || m.alt {
                    return;
                }
                if event.keystroke.key == "enter" || event.keystroke.key == "space" {
                    cx.stop_propagation();
                    this.toggle_base_picker(window, cx);
                }
            }))
            .relative()
            .min_w(px(0.))
            .flex_shrink(1.)
            .py(px(2.))
            .pl(px(8.))
            .pr(px(6.))
            .flex()
            .items_center()
            .gap(px(4.))
            .rounded(px(5.))
            .border_1()
            .border_color(if picker_open || chip_focused {
                theme::accent()
            } else {
                theme::border()
            })
            .bg(theme::raised())
            .hover(|el| el.bg(theme::with_alpha(theme::sidebar_selected(), 0.6)))
            .cursor_pointer()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(|this, _, window, cx| this.toggle_base_picker(window, cx)))
            .child(
                canvas(move |bounds, _, _| anchor.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .inset_0(),
            )
            .child(
                div()
                    .min_w(px(0.))
                    .truncate()
                    .text_size(theme::text_meta())
                    .text_color(if is_custom {
                        theme::text()
                    } else {
                        theme::muted()
                    })
                    .child(chip_text),
            )
            .child(
                svg()
                    .flex_none()
                    .size(px(11.))
                    .path("chevron-down.svg")
                    .text_color(theme::muted()),
            );

        let missing_notice = state.missing_base_notice.as_ref().map(|missing| {
            div()
                .when(cfg!(test), |el| {
                    el.debug_selector(|| "CHANGES_MISSING_BASE_NOTICE".into())
                })
                .px(px(10.))
                .pb(px(6.))
                .flex()
                .items_start()
                .gap(px(6.))
                .child(
                    svg()
                        .flex_none()
                        .mt(px(2.))
                        .size(px(12.))
                        .path("triangle-alert.svg")
                        .text_color(theme::warning()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .text_size(theme::text_meta())
                        .line_height(relative(1.4))
                        .text_color(theme::warning())
                        .child(crate::i18n::tf(
                            "changes.base_missing_notice",
                            &[("base", display_base_ref(missing).to_owned())],
                        )),
                )
        });

        div()
            .when(cfg!(test), |el| {
                el.debug_selector(|| "CHANGES_BASE_ROW".into())
            })
            .flex_none()
            .flex()
            .flex_col()
            .border_b_1()
            .border_color(theme::border())
            .child(
                div()
                    .h(px(30.))
                    .px(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(
                        div()
                            .flex_none()
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .child(crate::i18n::text("changes.base")),
                    )
                    .child(div().flex_1())
                    .child(chip),
            )
            .children(missing_notice)
            .into_any_element()
    }

    pub(super) fn render_base_picker(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let picker = self.base_picker.as_ref()?;
        let visible = filter_base_picker_items(&picker.items, &picker.query);
        let entity = cx.entity();
        let content = crate::surfaces::worktree_pickers::base_picker_card(
            crate::surfaces::worktree_pickers::PickerListView {
                id_prefix: "changes-base-picker-item",
                selector_prefix: "CHANGES_BASE_PICKER_ITEM",
                title: crate::i18n::text("changes.compare_against").into(),
                field: picker.field.clone(),
                visible: &visible,
                highlighted: picker.highlighted,
                scroll: &picker.scroll,
                reveal: &picker.reveal,
            },
            &picker.items,
            Rc::new(move |base_ref, window, cx| {
                entity.update(cx, |this, cx| this.pick_base(base_ref, window, cx));
            }),
            "CHANGES_BASE_PICKER",
            "changes-base-picker-list",
            "CHANGES_BASE_PICKER_LIST",
        );

        let root = cx.entity();
        Some(popup_layer_right_aligned(
            self.base_picker_anchor.get(),
            window,
            px(BASE_PICKER_WIDTH),
            content,
            Rc::new(move |window, cx| {
                root.update(cx, |this, cx| this.close_base_picker(window, cx));
            }),
        ))
    }

    pub(super) fn render_checkout_picker(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let picker = self.checkout_picker.as_ref()?;
        let visible = filter_checkout_picker_items(&picker.items, &picker.query);
        let entity = cx.entity();
        let content = crate::surfaces::worktree_pickers::checkout_picker_card(
            crate::surfaces::worktree_pickers::PickerListView {
                id_prefix: "changes-checkout-picker-item",
                selector_prefix: "CHANGES_CHECKOUT_PICKER_ITEM",
                title: crate::i18n::text("changes.show_changes_in").into(),
                field: picker.field.clone(),
                visible: &visible,
                highlighted: picker.highlighted,
                scroll: &picker.scroll,
                reveal: &picker.reveal,
            },
            &picker.items,
            false,
            None,
            Rc::new(move |pick_path, window, cx| {
                entity.update(cx, |this, cx| this.pick_checkout(pick_path, window, cx));
            }),
            "CHANGES_CHECKOUT_PICKER",
            "changes-checkout-picker-list",
            "CHANGES_CHECKOUT_PICKER_LIST",
        );

        let root = cx.entity();
        Some(popup_layer_right_aligned(
            self.checkout_picker_anchor.get(),
            window,
            px(CHECKOUT_PICKER_WIDTH),
            content,
            Rc::new(move |window, cx| {
                root.update(cx, |this, cx| this.close_checkout_picker(window, cx));
            }),
        ))
    }
}
