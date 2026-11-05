use super::*;

impl NativeRoot {
    /// Split menus and pane sizes are keyed by tab *and* pane, because a pane
    /// id such as `p1` exists in every tab; drop everything the active tab
    /// does not have so a switch cannot inherit the other tab's measurement
    /// or leave its menu hanging open.
    pub(super) fn prune_pane_state(&mut self, layout: &PaneLayout) {
        let panes = layout
            .root
            .leaves()
            .into_iter()
            .map(|leaf| leaf.id.as_str())
            .collect::<HashSet<_>>();
        let tab_id = layout.id.as_str();
        let region_keys = panes
            .iter()
            .map(|pane| format!("pane:{tab_id}:{pane}"))
            .collect::<HashSet<_>>();
        self.render_regions
            .retain(|key, _| !key.starts_with("pane:") || region_keys.contains(key));
        self.split_menus
            .retain(|key, _| key.tab_id == tab_id && panes.contains(key.pane_id.as_str()));
        self.front_page_pane_bounds_mut()
            .retain(|key, _| key.tab_id == tab_id && panes.contains(key.pane_id.as_str()));
    }

    /// The split menu for one pane. The header and an identity line differ
    /// only in trigger size, so each keeps its own entity rather than
    /// resizing a shared one when a tab is split or collapsed.
    pub(super) fn split_menu(
        &mut self,
        surface: SplitMenuSurface,
        tab_id: &str,
        pane_id: &str,
        cx: &mut Context<Self>,
    ) -> Entity<PopoverMenu> {
        let key = PaneKey::new(tab_id, pane_id);
        let menu = self
            .split_menus
            .entry(SplitMenuKey {
                surface,
                tab_id: key.tab_id.clone(),
                pane_id: key.pane_id.clone(),
            })
            .or_insert_with(|| {
                let root = cx.entity();
                let target = pane_id.to_owned();
                let id = SharedString::from(format!("split-menu-{}-{pane_id}", surface.key()));
                cx.new(move |menu_cx| {
                    let menu = PopoverMenu::new(
                        id,
                        menu_cx.focus_handle(),
                        Vec::new(),
                        Rc::new(move |index, window, cx| {
                            let pane_id = target.clone();
                            root.update(cx, |this, cx| match index {
                                0 => this.split_pane(&pane_id, SplitOrientation::Row, window, cx),
                                1 => {
                                    this.split_pane(&pane_id, SplitOrientation::Column, window, cx)
                                }
                                _ => {}
                            });
                        }),
                        menu_cx,
                    )
                    .min_width(px(196.))
                    .trigger_icon("columns-2.svg")
                    .without_trigger_tooltip();
                    match surface {
                        SplitMenuSurface::Pane => menu.trigger_size(IconButtonSize::Sm),
                    }
                })
            })
            .clone();
        let settings = self.settings(cx);
        let zoom = settings.app_zoom;
        let overrides = settings.keymap_overrides.clone();
        let items = split_menu_items(
            self.split_pane_bounds(tab_id, pane_id, cx),
            zoom,
            &overrides,
        );
        menu.update(cx, |menu, menu_cx| menu.set_items(items, menu_cx));
        menu
    }

    /// The size a split of `pane_id` is judged against. A maximized pane is
    /// measured filling the tab, but it splits in its restored place, so its
    /// share of the split layout is what counts.
    pub(crate) fn split_pane_bounds(
        &self,
        tab_id: &str,
        pane_id: &str,
        cx: &App,
    ) -> Option<Size<Pixels>> {
        let measured = self
            .front_page_pane_bounds()
            .get(&PaneKey::new(tab_id, pane_id))
            .copied()?;
        let Some(layout) = self
            .front_page_layout()
            .filter(|layout| layout.id == tab_id)
            .filter(|layout| {
                maximized_leaf(&self.maximized_panes, layout).is_some_and(|leaf| leaf.id == pane_id)
            })
        else {
            return Some(measured);
        };
        Some(restored_pane_size(
            measured,
            layout,
            pane_id,
            self.settings(cx).app_zoom,
        ))
    }

    pub(crate) fn render_pane_node(
        &mut self,
        node: &PaneNode,
        layout: &PaneLayout,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_card_pane_node(node, layout, cx)
    }

    pub(super) fn render_card_pane_node(
        &mut self,
        node: &PaneNode,
        layout: &PaneLayout,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match node {
            PaneNode::Leaf(leaf) => {
                let key = format!("pane:{}:{}", layout.id, leaf.id);
                let leaf = leaf.clone();
                let layout = layout.clone();
                self.cached_region(
                    key,
                    gpui::StyleRefinement::default().size_full(),
                    move |root, window, cx| root.render_pane(&leaf, &layout, window, cx),
                    cx,
                )
            }
            PaneNode::Split(split) => {
                let a = self.render_card_pane_node(&split.a, layout, cx);
                let b = self.render_card_pane_node(&split.b, layout, cx);
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
                        pane.flex_basis(px(0.)).flex_grow(first).h_full()
                    })
                    .when(orientation == SplitOrientation::Column, |pane| {
                        pane.flex_basis(px(0.)).flex_grow(first).w_full()
                    })
                    .child(a);
                let b_wrap = div()
                    .min_w(px(0.))
                    .min_h(px(0.))
                    .when(orientation == SplitOrientation::Row, |pane| {
                        pane.flex_basis(px(0.)).flex_grow(second).h_full()
                    })
                    .when(orientation == SplitOrientation::Column, |pane| {
                        pane.flex_basis(px(0.)).flex_grow(second).w_full()
                    })
                    .child(b);
                let gutter = resize_strip(
                    "pane-gutter",
                    match orientation {
                        SplitOrientation::Row => ResizeAxis::Columns,
                        SplitOrientation::Column => ResizeAxis::Rows,
                    },
                    self.front_page_resizing_split() == Some(split.id.as_str()),
                    self.settings(cx).app_zoom,
                    None,
                    false,
                )
                .id(SharedString::from(format!("gutter-{}", split.id)))
                .when(self.front_page_client(cx).supports(&["layout"]), |gutter| {
                    gutter.on_drag(drag, |drag: &SplitResizeDrag, _, _, cx: &mut App| {
                        cx.new(|_| drag.clone())
                    })
                })
                .when(
                    !self.front_page_client(cx).supports(&["layout"]),
                    |gutter| gutter.cursor(CursorStyle::Arrow),
                );
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

    /// The title's chat menu, or `None` when the client offers none of its
    /// actions and the title is plain text.
    pub(super) fn pane_title_menu(
        &mut self,
        entry: &SessionDisplay,
        tab_id: &str,
        label: SharedString,
        color: gpui::Hsla,
        disabled: bool,
        cx: &mut Context<Self>,
    ) -> Option<Entity<PopoverMenu>> {
        let session_id = &entry.session_id;
        let secondary =
            self.remote_client.is_local() && self.cached_chat_secondary_state(session_id).secondary;
        let pinned = self.front_page_sidebar().read(cx).tab_pinned(tab_id, cx);
        let client = self.front_page_client(cx);
        let actions = pane_menu_actions(
            &entry.agent_runtime,
            secondary,
            pinned.is_some(),
            |features| client.supports(features),
        );
        if actions.is_empty() {
            return None;
        }
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
                        SharedString::from(format!("pane-title-{target}")),
                        menu_cx.focus_handle(),
                        Vec::new(),
                        Rc::new(move |index, window, cx| {
                            let session_id = action_target.clone();
                            action_root.update(cx, |this, cx| {
                                this.activate_pane_menu_action(&session_id, index, window, cx)
                            });
                        }),
                        menu_cx,
                    )
                    .min_width(px(196.))
                    .single_line_labels()
                    .on_trigger_press({
                        let root = root.clone();
                        let target = target.clone();
                        move |window, cx| {
                            root.update(cx, |this, cx| {
                                this.focus_session_pane(&target, window, cx)
                            });
                        }
                    })
                    .on_trigger_double_click(move |window, cx| {
                        root.update(cx, |this, cx| {
                            this.begin_session_pane_rename(&target, window, cx)
                        });
                    })
                })
            })
            .clone();
        let fork = if let Some(local) = entry.local.as_ref() {
            header_fork_state(Some(local), secondary)
        } else if !entry.native_fork {
            HeaderForkState::Disabled(None)
        } else if !entry.forkable {
            HeaderForkState::Disabled(Some(crate::i18n::text("ui.no_session_key_captured_yet")))
        } else {
            HeaderForkState::Enabled
        };
        let pending = self
            .fork_confirm
            .as_ref()
            .is_some_and(|confirm| confirm.pending && confirm.session_id == *session_id)
            || super::super::chat::fork_in_progress(&self.forking_sessions, session_id);
        let items = pane_menu_items(
            &actions,
            pinned.unwrap_or(false),
            disabled,
            pending || !matches!(fork, HeaderForkState::Enabled),
            match fork {
                HeaderForkState::Disabled(reason) => reason,
                HeaderForkState::Enabled | HeaderForkState::Hidden => None,
            },
        );
        menu.update(cx, |menu, menu_cx| {
            menu.set_items(items, menu_cx);
            menu.set_trigger_label(
                runner_app::ui::MenuTriggerLabel { text: label, color },
                menu_cx,
            );
        });
        Some(menu)
    }

    fn activate_pane_menu_action(
        &mut self,
        session_id: &str,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.front_page_session(session_id, cx) else {
            return;
        };
        let Some(tab_id) = self
            .front_page_layout()
            .filter(|layout| layout.session_ids().iter().any(|id| id == session_id))
            .map(|layout| layout.id.clone())
        else {
            return;
        };
        let secondary =
            self.remote_client.is_local() && self.cached_chat_secondary_state(session_id).secondary;
        let pinned = self.front_page_sidebar().read(cx).tab_pinned(&tab_id, cx);
        let client = self.front_page_client(cx);
        let actions = pane_menu_actions(
            &entry.agent_runtime,
            secondary,
            pinned.is_some(),
            |features| client.supports(features),
        );
        match actions.get(index) {
            Some(PaneMenuAction::Rename) => self.begin_session_pane_rename(session_id, window, cx),
            Some(PaneMenuAction::Fork) => self.fork_chat(session_id, window, cx),
            Some(PaneMenuAction::Pin) => {
                // A remote pin updates this root again, so it runs after this update ends.
                let pinned = pinned.unwrap_or(false);
                let sidebar = self.front_page_sidebar();
                window.defer(cx, move |window, cx| {
                    sidebar.update(cx, |sidebar, sidebar_cx| {
                        sidebar.toggle_tab_pin(tab_id, pinned, window, sidebar_cx)
                    });
                });
            }
            None => {}
        }
    }

    /// Selects the pane showing `session_id` in the active tab, as a click
    /// anywhere else on that pane does.
    fn focus_session_pane(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(pane_id) = self.front_page_layout().and_then(|layout| {
            layout
                .root
                .leaves()
                .into_iter()
                .find(|leaf| leaf.session_id.as_deref() == Some(session_id))
                .map(|leaf| leaf.id.clone())
        }) else {
            return;
        };
        self.focus_terminal(&pane_id, session_id, window, cx);
    }

    fn begin_session_pane_rename(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(entry) = self.front_page_session(session_id, cx) else {
            return;
        };
        self.begin_pane_rename(
            entry.session_id.clone(),
            entry.title.clone().unwrap_or_default(),
            entry.default_label(),
            window,
            cx,
        );
    }

    pub(super) fn activate_primary_pane_action(
        &mut self,
        session_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.session_lifecycle_disabled(session_id, cx)
            || self.chat_secondary_state(session_id, cx).secondary
        {
            return;
        }
        let Some(status) = self
            .front_page_session(session_id, cx)
            .map(|entry| entry.status)
        else {
            return;
        };
        if status == SessionStatus::Running {
            self.stop_chat(session_id, window, cx);
        } else {
            self.resume_chats(vec![session_id.to_owned()], window, cx);
        }
    }
}
