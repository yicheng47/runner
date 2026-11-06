use super::*;
use crate::file_links::FileLinkRoute;
use crate::surfaces::files::FilesView;

impl NativeRoot {
    pub(super) fn render_island_panel_toggle(
        &self,
        layout: &PaneLayout,
        pane_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let surface = side_panel_surface(self.active_tab_is_terminal(cx));
        if !self.remote_client.is_local()
            || start_surface_visible(Some(layout))
            || self.settings(cx).chat_panel_open
            || !side_panel_available(self.settings(cx), surface)
            || maximized_leaf(&self.maximized_panes, layout)
                .unwrap_or_else(|| top_right_pane(&layout.root))
                .id
                != pane_id
        {
            return None;
        }
        let root = cx.entity();
        Some(
            div()
                .when(cfg!(test), |toggle| {
                    toggle.debug_selector(|| "CHAT_PANEL_TOGGLE".into())
                })
                .flex_none()
                .flex()
                .items_center()
                .ml_1()
                .gap_1()
                .child(div().w(rems(1. / 16.)).h(rems(1.)).bg(theme::border()))
                .child(
                    IconButton::new("open-chat-panel", "panel-right-hidden.svg")
                        .size(IconButtonSize::Sm)
                        .tooltip(crate::i18n::text("ui.open_side_panel"))
                        .on_press(move |window, cx| {
                            root.update(cx, |this, cx| {
                                this.update_app_settings(cx, true, |settings| {
                                    settings.chat_panel_open = true;
                                    true
                                });
                                this.focus_empty_chat_panel(window, cx);
                                cx.notify();
                            })
                        }),
                )
                .into_any_element(),
        )
    }

    pub(super) fn claude_account_details(
        &self,
        detail: &DirectSessionEntry,
        cx: &App,
    ) -> Option<(String, Option<String>, String)> {
        let store = self.app_store.read(cx);
        if detail.agent_runtime != "claude-code" || !store.claude_accounts.multiple() {
            return None;
        }
        let session = detail.claude_account.as_ref();
        let account = store.claude_accounts.session_account(session);
        let name = account
            .map(|account| account.name.clone())
            .or_else(|| session.map(|session| session.folder.clone()))?;
        let email = account
            .and_then(|account| store.claude_accounts.statuses.get(&account.id))
            .and_then(|status| status.email.clone());
        let folder = session
            .map(|session| session.folder.clone())
            .or_else(|| account.map(|account| account.folder.clone()))?;
        Some((name, email, folder))
    }

    pub(super) fn claude_paused_text(
        &self,
        detail: &DirectSessionEntry,
        cx: &App,
    ) -> Option<String> {
        if detail
            .claude_account
            .as_ref()
            .is_some_and(|account| account.pinned)
        {
            return None;
        }
        let (previous, _, folder) = self.claude_account_details(detail, cx)?;
        let store = self.app_store.read(cx);
        let active = store.claude_accounts.active()?;
        let session = detail.claude_account.as_ref();
        if folder == active.folder && session.is_none_or(|session| session.native == active.native)
        {
            return None;
        }
        Some(crate::i18n::tf(
            "accounts.paused",
            &[("previous", previous), ("active", active.name.clone())],
        ))
    }

    fn chat_panel_context(
        &self,
        detail: &DirectSessionEntry,
        surface: PanelSurface,
        cx: &App,
    ) -> PanelContext {
        let role_dir = || {
            let role_id = detail.role_id.as_deref()?;
            self.app_store
                .read(cx)
                .roles
                .iter()
                .find(|role| role.id == role_id)?
                .working_dir
                .clone()
        };
        PanelContext {
            subject: PanelSubject::Session(detail.session_id.clone()),
            surface,
            cwd: detail.cwd.clone().or_else(role_dir),
            project: detail.project_id.clone(),
            remote: !self.remote_client.is_local(),
        }
    }

    /// Where a ⌘-clicked file link in the terminal of `session`'s pane
    /// opens: the panel's preview, beside the chat the panel follows once
    /// that pane has focus, as the click gives it.
    pub(crate) fn file_link_route(&self, session: &str, cx: &Context<Self>) -> FileLinkRoute {
        let session = session.to_owned();
        FileLinkRoute::new(
            cx.weak_entity(),
            move |this: &mut Self, cx| this.link_files_view(&session, cx),
            Self::show_link_preview,
        )
    }

    /// The panel's Files view while its preview can take a link: the panel
    /// and Files on, and the chat local.
    fn link_files_view(
        &mut self,
        session: &str,
        cx: &mut Context<Self>,
    ) -> Option<Entity<FilesView>> {
        let settings = self.settings(cx);
        if !settings.show_side_panel || !settings.files_view || !self.remote_client.is_local() {
            return None;
        }
        let (followed, _) = self.panel_session_for(Some(session), cx)?;
        let detail = self
            .active_chat_detail
            .clone()
            .filter(|detail| detail.session_id == followed)
            .or_else(|| {
                self.app_store
                    .read(cx)
                    .session_details
                    .get(&followed)
                    .cloned()
            })?;
        let surface = side_panel_surface(self.active_tab_is_terminal(cx));
        let context = self.chat_panel_context(&detail, surface, cx);
        Some(self.chat_files.link_view(&context, cx))
    }

    /// The chat panel's empty list takes focus when the panel opens on it.
    pub(crate) fn focus_empty_chat_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let surface = side_panel_surface(self.active_tab_is_terminal(cx));
        let settings = self.settings(cx);
        if self.remote_client.is_local()
            && self.route == AppRoute::Chat
            && side_panel_open(settings, surface)
            && side_panel::shown(surface, settings, &self.chat_surfaces.open).is_none()
        {
            self.chat_surfaces.list_focus().focus(window, cx);
        }
    }

    /// ⇧⌘O: opens the panel on Files, as a link does, and its finder. With no
    /// view to search the key goes on, so a terminal program gets it.
    pub(crate) fn search_files_in_chat(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = self
            .active_focused_session_id()
            .and_then(|session| self.link_files_view(&session, cx));
        let Some(view) = view else {
            cx.propagate();
            return;
        };
        self.show_link_preview(cx);
        view.update(cx, |view, cx| view.toggle_finder(window, cx));
    }

    /// A link opened in the preview: the panel opens Files, or switches to it.
    fn show_link_preview(&mut self, cx: &mut Context<Self>) {
        if side_panel_surface(self.active_tab_is_terminal(cx)) == PanelSurface::Chat {
            self.chat_surfaces.open.open(PanelViewId::Files);
        }
        if !self.settings(cx).chat_panel_open {
            self.update_app_settings(cx, true, |settings| {
                settings.chat_panel_open = true;
                true
            });
        }
        cx.notify();
    }

    /// The chat panel's width, widened while its Files view has a file open.
    pub(super) fn chat_panel_layout(
        &self,
        detail: Option<&DirectSessionEntry>,
        cx: &App,
    ) -> (f32, Option<crate::surfaces::side_panel::Widened>) {
        let settings = self.settings(cx);
        let subject = detail
            .filter(|_| settings.files_view)
            .map(|detail| PanelSubject::Session(detail.session_id.clone()));
        let surface = side_panel_surface(self.active_tab_is_terminal(cx));
        self.chat_files.layout(
            subject.as_ref(),
            crate::surfaces::files::FilesSurface::of(surface, settings, &self.chat_surfaces.open),
            settings.chat_panel_width,
            settings.file_viewer_width,
            settings.file_tree_width,
            6.,
            cx,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_chat_side_panel(
        &self,
        detail: Option<&DirectSessionEntry>,
        surface: PanelSurface,
        session_key_copy: Entity<CopyValueButton>,
        (width, widened): (f32, Option<crate::surfaces::side_panel::Widened>),
        visibility: f32,
        show_panel: bool,
        border_on: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        #[cfg(test)]
        crate::render_counts::count_render("side-panel", cx);
        let visible_width = width * visibility;
        if !show_panel {
            self.chat_files.hide(cx);
            return div()
                .id("chat-side-panel")
                .relative()
                .w(rems(visible_width / 16.))
                .h_full()
                .flex_none()
                .overflow_hidden()
                .into_any_element();
        }
        let role = detail
            .and_then(|detail| detail.role_id.as_deref())
            .and_then(|role_id| {
                self.app_store
                    .read(cx)
                    .roles
                    .iter()
                    .find(|role| role.id == role_id)
                    .cloned()
            });
        let collapse_root = cx.entity();
        let surface_root = cx.entity();
        let settings = self.settings(cx).clone();
        let shown = side_panel::shown(surface, &settings, &self.chat_surfaces.open);
        let handler: side_panel::SurfaceHandler = Rc::new(move |action, window, cx| {
            surface_root.update(cx, |this, cx| {
                let settings = this.settings(cx).clone();
                this.chat_surfaces
                    .apply(surface, &settings, action, window, cx);
                // A pick removes the empty list, whether made there or from a
                // `+` menu that handed focus back to it: the chat takes the keys.
                if matches!(action, side_panel::SurfaceAction::Open(_))
                    && this.chat_surfaces.list_focus().is_focused(window)
                {
                    this.focus_active_terminal(window, cx);
                }
                cx.notify();
            })
        });
        let header = self.render_titlebar_drag_area(
            "chat-panel-titlebar-drag",
            div()
                .flex_none()
                .h(rems(
                    if self.route == AppRoute::Chat {
                        40.
                    } else {
                        WORKSPACE_HEADER_HEIGHT
                    } / 16.,
                ))
                .pl_2()
                .pr_2()
                .flex()
                .items_center()
                .border_b_1()
                .border_color(theme::border())
                .rounded_tr(rems(11. / 16.))
                .map(|element| {
                    #[cfg(test)]
                    let element = crate::theme_snapshot::record_fill("CHAT_PANEL_HEADER", element);
                    element
                })
                .child(side_panel::header_tabs(
                    surface,
                    &settings,
                    &self.chat_surfaces.open,
                    (&self.chat_surfaces, window),
                    handler.clone(),
                    IconButton::new("collapse-chat-panel", "panel-right-open.svg")
                        .tooltip(crate::i18n::text("ui.collapse_side_panel"))
                        .on_press(move |_, cx| {
                            collapse_root.update(cx, |this, cx| {
                                this.update_app_settings(cx, true, |settings| {
                                    settings.chat_panel_open = false;
                                    true
                                });
                                cx.notify();
                            });
                        }),
                )),
            cx,
        );
        let files = detail
            .filter(|_| shown == Some(PanelViewId::Files))
            .map(|detail| {
                self.chat_files.show_with(
                    &self.chat_panel_context(detail, surface, cx),
                    widened,
                    Some(cx.entity_id()),
                    cx,
                )
            });
        if files.is_none() {
            self.chat_files.hide(cx);
        }
        let content = if shown.is_none() {
            None
        } else if let Some(detail) = detail {
            let open_root = cx.entity();
            let toggle_root = cx.entity();
            let actions = ChatPanelActions {
                open_role: role.as_ref().map(|role| {
                    let handle = role.handle.clone();
                    Rc::new(move |window: &mut Window, cx: &mut App| {
                        open_root.update(cx, |this, cx| {
                            this.open_role_detail(handle.clone(), window, cx)
                        });
                    }) as ClickHandler
                }),
                toggle_prompt: Rc::new(move |_, cx| {
                    toggle_root.update(cx, |this, cx| {
                        this.chat_panel_prompt_expanded = !this.chat_panel_prompt_expanded;
                        cx.notify();
                    });
                }),
                expanded: self.chat_panel_prompt_expanded,
                view: cx.entity_id(),
            };
            Some(chat_panel_content(
                detail,
                role.as_ref(),
                width,
                session_key_copy,
                actions,
                self.claude_account_details(detail, cx),
                cx,
            ))
        } else {
            Some(
                div()
                    .text_size(theme::text_ui())
                    .text_color(theme::faint())
                    .child(crate::i18n::text("ui.loading_chat"))
                    .into_any_element(),
            )
        };
        let panel = div()
            .relative()
            .w(rems(width / 16.))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.))
                    .flex()
                    .flex_col()
                    .pb(rems(8. / 16.))
                    .child(match (files, content) {
                        (Some(files), _) => files,
                        (None, Some(content)) => div()
                            .id("chat-panel-scroll")
                            .flex_1()
                            .min_h(px(0.))
                            .overflow_y_scroll()
                            .p_5()
                            .child(content)
                            .into_any_element(),
                        (None, None) => {
                            side_panel::empty_list(surface, &settings, &self.chat_surfaces, handler)
                        }
                    }),
            );
        div()
            .id("chat-side-panel")
            .debug_selector(|| "CHAT_SIDE_PANEL".into())
            .relative()
            .w(rems(visible_width / 16.))
            .h_full()
            .flex_none()
            .overflow_hidden()
            .when(self.route == AppRoute::Chat, |panel| {
                panel
                    .rounded(rems(12. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::bg())
            })
            .when(self.route != AppRoute::Chat && border_on, |panel| {
                panel.border_l_1().border_color(theme::border())
            })
            .child(panel)
            .into_any_element()
    }
}
