use super::*;
use crate::file_links::FileLinkRoute;
use crate::surfaces::files::FilesView;

impl NativeRoot {
    pub(crate) fn sync_chat_panel(&mut self, cx: &App) {
        if !self.remote_client.is_local() {
            return;
        }
        let tab = self.tabs.active_tab_id().map(str::to_owned);
        if self.chat_panel_tab == tab {
            return;
        }
        if let Some(previous) = self
            .chat_panel_tab
            .take()
            .filter(|id| self.tabs.tabs().iter().any(|tab| &tab.id == id))
        {
            self.chat_panels_by_tab.insert(
                previous,
                side_panel::SavedPanel {
                    surfaces: self.chat_surfaces.open.clone(),
                    open: self.chat_panel_open,
                    maximized: self.chat_panel_maximized,
                    widened_tree: self.chat_surfaces.widened_tree(),
                },
            );
        }
        let saved = tab
            .as_ref()
            .and_then(|id| self.chat_panels_by_tab.remove(id));
        if let Some(saved) = saved {
            self.chat_surfaces
                .restore(saved.surfaces, saved.widened_tree);
            self.chat_panel_open = saved.open;
            self.chat_panel_maximized = saved.maximized;
        } else {
            self.chat_surfaces.reset(Default::default());
            self.chat_panel_open = None;
            self.chat_panel_maximized = false;
        }
        self.chat_panel_tab = tab;
        let open = self
            .chat_panel_open
            .unwrap_or(self.settings(cx).chat_panel_open);
        self.chat_panel_open = Some(open);
        let visible = open
            && !start_surface_visible(self.tabs.active())
            && side_panel_available(
                self.settings(cx),
                side_panel_surface(self.active_tab_is_terminal(cx)),
            );
        if !open {
            self.chat_panel_maximized = false;
        }
        self.chat_panel_visibility = SidebarVisibilityTransition::new(visible);
    }

    pub(crate) fn chat_panel_is_open(&self, cx: &App) -> bool {
        if self.remote_client.is_active() {
            return self.remote_client.chat_info_is_open();
        }
        self.chat_panel_open
            .unwrap_or(self.settings(cx).chat_panel_open)
    }

    pub(crate) fn set_chat_panel_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.remote_client.is_active() {
            self.remote_client.set_chat_info_open(open);
            return;
        }
        self.sync_chat_panel(cx);
        self.chat_panel_open = Some(open);
        if !open {
            self.chat_panel_maximized = false;
        }
        self.update_app_settings(cx, true, |settings| {
            let changed = settings.chat_panel_open != open;
            settings.chat_panel_open = open;
            changed
        });
    }

    pub(crate) fn chat_panel_contains_focus(&self, window: &Window, cx: &App) -> bool {
        self.chat_panel_focus.contains_focused(window, cx)
            || self.chat_surfaces.menu_focused(window)
    }

    pub(crate) fn focus_chat_panel_shown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let surface = if self.route == AppRoute::ArchivedChat {
            PanelSurface::Chat
        } else {
            side_panel_surface(self.active_tab_is_terminal(cx))
        };
        let settings = self.settings(cx);
        let shown = side_panel::shown(surface, settings, &self.chat_surfaces.open);
        if shown.is_none() {
            self.chat_surfaces.list_focus().focus(window, cx);
            return;
        }
        let detail = if self.route == AppRoute::ArchivedChat {
            self.archived_chat_detail.as_ref()
        } else {
            self.active_chat_detail.as_ref()
        };
        let subject = detail.map(|d| PanelSubject::Session(d.session_id.clone()));
        self.chat_files
            .focus_shown(subject.as_ref(), shown, &self.chat_panel_focus, window, cx);
    }

    pub(crate) fn chat_panel_effectively_maximized(&self, cx: &App) -> bool {
        if !self.chat_panel_maximized
            || !self.remote_client.is_local()
            || !self.chat_panel_is_open(cx)
        {
            return false;
        }
        let surface = if self.route == AppRoute::ArchivedChat {
            PanelSurface::Chat
        } else if !start_surface_visible(self.tabs.active()) && self.front_page_is_chat() {
            side_panel_surface(self.active_tab_is_terminal(cx))
        } else {
            return false;
        };
        side_panel_available(self.settings(cx), surface)
    }

    pub(crate) fn set_chat_panel_maximized(
        &mut self,
        maximized: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_chat_panel(cx);
        let surface = if self.route == AppRoute::ArchivedChat {
            PanelSurface::Chat
        } else {
            side_panel_surface(self.active_tab_is_terminal(cx))
        };
        let allowed = self.remote_client.is_local()
            && (self.route == AppRoute::ArchivedChat
                || (!start_surface_visible(self.tabs.active()) && self.front_page_is_chat()))
            && self.chat_panel_is_open(cx)
            && side_panel_available(self.settings(cx), surface);
        let next = maximized && allowed;
        if self.chat_panel_maximized == next {
            return;
        }
        self.chat_panel_maximized = next;
        if next {
            self.chat_panel_visibility = SidebarVisibilityTransition::new(true);
            if !self.chat_panel_contains_focus(window, cx) {
                self.focus_chat_panel_shown(window, cx);
            }
        }
        cx.notify();
    }

    pub(crate) fn toggle_chat_panel_maximized(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next = !self.chat_panel_maximized;
        self.set_chat_panel_maximized(next, window, cx);
    }

    pub(super) fn active_chat_panel_surface(&self, cx: &App) -> (PanelSurface, bool) {
        let terminal = self.active_tab_is_terminal(cx);
        let surface = side_panel_surface(terminal);
        let settings = self.settings(cx);
        let available = if self.remote_client.is_active() {
            !terminal && settings.show_side_panel && settings.info_view
        } else {
            side_panel_available(settings, surface)
        };
        (surface, available)
    }

    pub(super) fn render_island_panel_toggle(
        &self,
        layout: &PaneLayout,
        pane_id: &str,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if start_surface_visible(Some(layout))
            || self.chat_panel_is_open(cx)
            || !self.active_chat_panel_surface(cx).1
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
                                this.set_chat_panel_open(true, cx);
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

    pub(super) fn chat_panel_context(
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
        let tab = self
            .tabs
            .tabs()
            .iter()
            .find(|tab| tab.contains_session(session))
            .map(|tab| tab.id.clone());
        let session = session.to_owned();
        FileLinkRoute::new(
            cx.weak_entity(),
            move |this: &mut Self, cx| this.link_files_view(&session, cx),
            move |this, cx| {
                if let Some(tab) = tab.as_deref() {
                    this.show_link_preview_in_tab(tab, cx);
                }
            },
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
            && self.chat_panel_is_open(cx)
            && side_panel_available(settings, surface)
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
        if let Some(tab) = self.tabs.active_tab_id().map(str::to_owned) {
            self.show_link_preview_in_tab(&tab, cx);
        }
    }

    fn show_link_preview_in_tab(&mut self, tab: &str, cx: &mut Context<Self>) {
        self.sync_chat_panel(cx);
        if self.chat_panel_tab.as_deref() == Some(tab) {
            self.chat_surfaces.open.open(PanelViewId::Files);
            self.chat_panel_open = Some(true);
        } else {
            let panel = self.chat_panels_by_tab.entry(tab.into()).or_default();
            panel.surfaces.open(PanelViewId::Files);
            panel.open = Some(true);
        }
        self.update_app_settings(cx, true, |settings| {
            let changed = !settings.chat_panel_open;
            settings.chat_panel_open = true;
            changed
        });
        cx.notify();
    }

    /// The chat panel's width, widened once Files or Changes has widened it.
    pub(super) fn chat_panel_layout(
        &self,
        detail: Option<&DirectSessionEntry>,
        cx: &App,
    ) -> (f32, Option<crate::surfaces::side_panel::Widened>) {
        let settings = self.settings(cx);
        let surface = if self.route == AppRoute::ArchivedChat {
            PanelSurface::Chat
        } else {
            side_panel_surface(self.active_tab_is_terminal(cx))
        };
        let subject = detail.map(|detail| PanelSubject::Session(detail.session_id.clone()));
        self.chat_files.panel_layout(
            surface,
            settings,
            &self.chat_surfaces,
            subject.as_ref(),
            settings.chat_panel_width,
            6.,
            self.chat_panel_maximized,
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
        let remote = self.remote_client.is_active() && self.route != AppRoute::ArchivedChat;
        let detail = detail.filter(|_| !remote);
        let maximized = show_panel && !remote && self.chat_panel_maximized;
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
        let maximize_root = cx.entity();
        let req_max_root = cx.entity();
        let surface_root = cx.entity();
        let mut settings = self.settings(cx).clone();
        if remote {
            settings.files_view = false;
            settings.changes_view = false;
        }
        let shown = side_panel::shown(surface, &settings, &self.chat_surfaces.open);
        let handler: side_panel::SurfaceHandler = Rc::new(move |action, window, cx| {
            if remote {
                return;
            }
            surface_root.update(cx, |this, cx| {
                let settings = this.settings(cx).clone();
                this.chat_surfaces
                    .apply(surface, &settings, action, window, cx);
                // A pick removes the empty list, whether made there or from a
                // `+` menu that handed focus back to it: the chat takes the keys.
                if matches!(action, side_panel::SurfaceAction::Open(_))
                    && this.chat_surfaces.list_focus().is_focused(window)
                    && !this.chat_panel_maximized
                {
                    this.focus_active_terminal(window, cx);
                }
                cx.notify();
            })
        });
        let maximize_control = (!remote).then_some(side_panel::MaximizeControl {
            maximized,
            on_toggle: Rc::new(move |window, cx| {
                maximize_root.update(cx, |this, cx| {
                    this.toggle_chat_panel_maximized(window, cx);
                });
            }),
        });
        let on_request_maximize: side_panel::PanelCallback = Rc::new(move |window, cx| {
            req_max_root.update(cx, |this, cx| {
                this.set_chat_panel_maximized(true, window, cx);
            });
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
                .when(maximized, |h| h.rounded_tl(rems(11. / 16.)))
                .rounded_tr(rems(11. / 16.))
                .when(
                    maximized && self.front_page_sidebar_collapsed() && cfg!(target_os = "macos"),
                    |header| {
                        header
                            .pl(px(self.workspace_titlebar_padding(window, cx)))
                            .children(self.render_collapsed_titlebar_spacer())
                    },
                )
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
                    maximize_control,
                    IconButton::new("collapse-chat-panel", "panel-right-open.svg")
                        .tooltip(crate::i18n::text("ui.collapse_side_panel"))
                        .on_press(move |window, cx| {
                            collapse_root.update(cx, |this, cx| {
                                let return_focus = this.chat_panel_contains_focus(window, cx)
                                    || (!remote && this.chat_panel_maximized);
                                this.set_chat_panel_open(false, cx);
                                if return_focus && this.route == AppRoute::Chat {
                                    this.focus_active_terminal(window, cx);
                                }
                                cx.notify();
                            });
                        }),
                )),
            cx,
        );
        let files = match (shown, detail) {
            (Some(PanelViewId::Files), Some(detail)) => {
                self.chat_files.hide_changes(cx);
                Some(self.chat_files.show_with_maximize(
                    &self.chat_panel_context(detail, surface, cx),
                    widened,
                    Some(cx.entity_id()),
                    maximized,
                    Some(on_request_maximize),
                    cx,
                ))
            }
            (Some(PanelViewId::Changes), Some(detail)) => {
                self.chat_files.hide_files(cx);
                Some(self.chat_files.show_changes_with_maximize(
                    &self.chat_panel_context(detail, surface, cx),
                    widened,
                    Some(cx.entity_id()),
                    maximized,
                    Some(on_request_maximize),
                    cx,
                ))
            }
            _ => {
                self.chat_files.hide(cx);
                None
            }
        };
        let content = if shown.is_none() {
            None
        } else if remote {
            self.render_remote_chat_info(cx)
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
            let account = self.claude_account_details(detail, cx);
            let branch_cwd = detail
                .cwd
                .clone()
                .or_else(|| role.as_ref().and_then(|role| role.working_dir.clone()));
            let branch_project = detail.project_id.clone();
            let remote = self.front_page_client(cx).is_remote();
            let branch = crate::surfaces::panes::ensure_info_branch_row(
                &self.app_store,
                branch_cwd.as_deref(),
                branch_project.as_deref(),
                remote,
                cx,
            );
            Some(chat_panel_content(
                detail,
                role.as_ref(),
                width,
                session_key_copy,
                actions,
                account,
                branch,
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
            .when(maximized, |p| p.w_full().flex_1().min_w(px(0.)))
            .when(!maximized, |p| p.w(rems(width / 16.)).flex_none())
            .h_full()
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
                    .when(files.is_none(), |body| body.pb(rems(8. / 16.)))
                    .child(match (files, content) {
                        (Some(files), _) => files,
                        (None, Some(content)) => div()
                            .id("chat-panel-scroll")
                            .when(cfg!(test), |el| {
                                el.debug_selector(|| "CHAT_INFO_SCROLL".into())
                            })
                            .flex_1()
                            .min_h(px(0.))
                            .overflow_y_scroll()
                            .p_5()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, _, window, cx| {
                                    if !this.chat_panel_contains_focus(window, cx) {
                                        this.chat_panel_focus.focus(window, cx);
                                    }
                                }),
                            )
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
            .track_focus(&self.chat_panel_focus)
            .relative()
            .when(maximized, |p| p.flex_1().min_w(px(0.)).w_full())
            .when(!maximized, |p| p.w(rems(visible_width / 16.)).flex_none())
            .h_full()
            .overflow_hidden()
            .when(self.route == AppRoute::Chat, |panel| {
                panel
                    .rounded(rems(12. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::bg())
            })
            .when(
                self.route != AppRoute::Chat && border_on && !maximized,
                |panel| panel.border_l_1().border_color(theme::border()),
            )
            .child(panel)
            .into_any_element()
    }

    fn render_remote_chat_info(&self, cx: &App) -> Option<AnyElement> {
        let computer = self.remote_client.computer()?;
        let focused = self.front_page_layout()?.focused_session_id();
        let (id, _) = self.panel_session_for(focused, cx)?;
        let session = self.remote_client.session(&id, cx)?;
        let branch = remote_info_branch_row(
            &self.app_store,
            &computer.id,
            None,
            Some(&id),
            session.cwd.as_deref(),
            cx,
        );
        Some(remote_chat_panel_content(session, branch))
    }
}
