use super::*;
use crate::file_links::FileLinkRoute;
use crate::surfaces::files::FilesView;

impl NativeRoot {
    fn showing_archived_chat(&self) -> bool {
        self.remote_client.is_local() && self.route == AppRoute::ArchivedChat
    }

    fn current_chat_panel_target(&self) -> Option<side_panel::ChatPanelTarget> {
        use side_panel::ChatPanelTarget;
        if self.remote_client.is_active() {
            self.remote_client
                .active_chat_panel_target()
                .map(|(computer_id, tab_id)| ChatPanelTarget::Remote {
                    computer_id,
                    tab_id,
                })
        } else if self.showing_archived_chat() {
            self.archived_chat_detail
                .as_ref()
                .map(|detail| ChatPanelTarget::Archived(detail.session_id.clone()))
        } else {
            self.tabs
                .active_tab_id()
                .map(|id| ChatPanelTarget::Local(id.to_owned()))
        }
    }

    pub(crate) fn sync_chat_panel(&mut self, cx: &App) {
        if self.remote_client.is_local() {
            let tab = if self.showing_archived_chat() {
                self.archived_chat_detail
                    .as_ref()
                    .map(|detail| format!("archived:{}", detail.session_id))
            } else {
                self.tabs.active_tab_id().map(str::to_owned)
            };
            let tabs = self.tabs.tabs();
            self.side_panel
                .switch_tab(tab, |id| tabs.iter().any(|tab| tab.id == id));
        }

        let target = self.current_chat_panel_target();
        if self.side_panel.visible_target == target {
            return;
        }
        let open = self.chat_panel_is_open(cx);
        let showing_archived = self.showing_archived_chat();
        let layout = (!showing_archived)
            .then(|| self.front_page_layout())
            .flatten();
        let surface = if showing_archived {
            PanelSurface::Chat
        } else {
            side_panel_surface(self.active_tab_is_terminal(cx))
        };
        let split_columns = if showing_archived {
            1
        } else {
            self.active_chat_split_columns()
        };
        let is_sheet = side_panel::is_narrow(
            self.side_panel.files.available_width(),
            self.settings(cx).chat_panel_width,
            split_columns,
        );
        let visible = open
            && !is_sheet
            && (showing_archived || !start_surface_visible(layout))
            && side_panel_available(self.settings(cx), surface);
        self.side_panel
            .snap_to_target(target, visible, self.remote_client.is_local() && !open);
    }

    pub(super) fn active_chat_split_columns(&self) -> usize {
        if self.showing_archived_chat() || !self.front_page_is_chat() {
            return 1;
        }
        let layout = self.front_page_layout();
        let has_maximized =
            layout.is_some_and(|l| maximized_leaf(&self.maximized_panes, l).is_some());
        side_panel::split_columns_for_layout(layout, has_maximized)
    }

    pub(crate) fn chat_panel_is_open(&self, cx: &App) -> bool {
        let explicit = if self.remote_client.is_active() {
            self.remote_client.chat_panel_explicit_open()
        } else {
            self.side_panel.open
        };
        let surface = if self.showing_archived_chat() {
            PanelSurface::Chat
        } else {
            side_panel_surface(self.active_tab_is_terminal(cx))
        };
        side_panel::resolved_open(
            surface,
            explicit,
            self.side_panel.files.available_width(),
            self.settings(cx).chat_panel_width,
            self.active_chat_split_columns(),
        )
    }

    pub(crate) fn set_chat_panel_open(&mut self, open: bool, cx: &mut Context<Self>) {
        self.sync_chat_panel(cx);
        if self.remote_client.is_active() {
            self.remote_client.set_chat_panel_open(open);
            cx.notify();
            return;
        }
        self.side_panel.set_open(open);
        cx.notify();
    }

    pub(crate) fn chat_panel_contains_focus(&self, window: &Window, cx: &App) -> bool {
        self.side_panel.contains_focus(window, cx)
    }

    pub(crate) fn focus_chat_panel_shown(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let surface = if self.showing_archived_chat() {
            PanelSurface::Chat
        } else {
            side_panel_surface(self.active_tab_is_terminal(cx))
        };
        let settings = self.settings(cx);
        let selection = self.active_chat_panel_selection();
        let shown = side_panel::shown(surface, settings, &selection);
        let detail = if self.showing_archived_chat() {
            self.archived_chat_detail.as_ref()
        } else {
            self.active_chat_detail.as_ref()
        };
        let subject = detail.map(|d| PanelSubject::Session(d.session_id.clone()));
        self.side_panel
            .focus_shown(subject.as_ref(), shown, window, cx);
    }

    pub(crate) fn chat_panel_effectively_maximized(&self, cx: &App) -> bool {
        if !self.side_panel.maximized
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
        if !self.side_panel.set_maximized(next) {
            return;
        }
        if next && !self.chat_panel_contains_focus(window, cx) {
            self.focus_chat_panel_shown(window, cx);
        }
        cx.notify();
    }

    pub(crate) fn toggle_chat_panel_maximized(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let next = !self.side_panel.maximized;
        self.set_chat_panel_maximized(next, window, cx);
    }

    pub(crate) fn active_chat_panel_surface(&self, cx: &App) -> (PanelSurface, bool) {
        let terminal = self.active_tab_is_terminal(cx);
        let surface = side_panel_surface(terminal);
        let settings = self.settings(cx);
        let available = side_panel_available(settings, surface);
        (surface, available)
    }

    fn active_chat_panel_selection(&self) -> side_panel::PanelSelection {
        if self.remote_client.is_active() {
            self.remote_client.chat_panel_selection()
        } else {
            self.side_panel.surfaces.selection
        }
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
                                this.focus_chat_panel_shown(window, cx);
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

    fn session_effective_cwd(&self, session_id: &str, cx: &App) -> Option<String> {
        if self.remote_client.is_active() {
            return self
                .remote_client
                .session(session_id, cx)
                .and_then(|session| session.cwd);
        }
        let detail = self
            .active_chat_detail
            .as_ref()
            .filter(|d| d.session_id == session_id)
            .or_else(|| self.app_store.read(cx).session_details.get(session_id))
            .or_else(|| self.session_entry(session_id, cx))?;
        detail.cwd.clone().or_else(|| {
            let role_id = detail.role_id.as_deref()?;
            self.app_store
                .read(cx)
                .roles
                .iter()
                .find(|role| role.id == role_id)?
                .working_dir
                .clone()
        })
    }

    /// Quiet `Following <pane> · <branch/folder>` cue when a split tab holds
    /// panes working in different folders or worktrees (#1403).
    pub(super) fn split_following_badge(&self, followed_id: &str, cx: &App) -> Option<String> {
        if self.showing_archived_chat() || !self.front_page_is_chat() {
            return None;
        }
        let layout = self.front_page_layout()?;
        let leaves = layout.root.leaves();
        if leaves.len() <= 1 {
            return None;
        }
        let mut distinct_cwds = HashSet::new();
        for leaf in &leaves {
            if let Some(session_id) = leaf.session_id.as_deref()
                && let Some(cwd) = self.session_effective_cwd(session_id, cx)
            {
                let trimmed = cwd.trim().trim_end_matches(['/', '\\']).to_owned();
                if !trimmed.is_empty() {
                    distinct_cwds.insert(trimmed);
                }
            }
        }
        if distinct_cwds.len() <= 1 {
            return None;
        }
        let pane_label = self
            .front_page_session(followed_id, cx)
            .map(|entry| {
                entry.label(
                    self.remote_client
                        .is_local()
                        .then(|| self.attached_title(followed_id, cx))
                        .flatten()
                        .as_deref(),
                )
            })
            .unwrap_or_else(|| followed_id.to_owned());
        let cwd = self.session_effective_cwd(followed_id, cx);
        let branch = if self.remote_client.is_active() {
            self.remote_client.computer().and_then(|computer| {
                remote_info_branch_row(
                    &self.app_store,
                    &computer.id,
                    None,
                    Some(followed_id),
                    cwd.as_deref(),
                    cx,
                )
            })
        } else {
            let project_id = self
                .session_entry(followed_id, cx)
                .and_then(|entry| entry.project_id.as_deref());
            cwd.as_deref().and_then(|cwd| {
                self.app_store
                    .read(cx)
                    .worktree_checkout_for(cwd, project_id)
                    .and_then(|checkout| info_branch_row_from_checkout(checkout, None))
            })
        };
        let target = branch
            .map(|b| b.branch)
            .or_else(|| {
                cwd.as_deref().and_then(|cwd| {
                    crate::surfaces::remote_client::model::host_path_leaf(cwd)
                        .map(str::to_owned)
                        .or_else(|| {
                            std::path::Path::new(cwd)
                                .file_name()
                                .map(|name| name.to_string_lossy().into_owned())
                        })
                })
            })
            .filter(|s| !s.is_empty())?;
        Some(crate::i18n::tf(
            "side_panel.following",
            &[("pane", pane_label), ("target", target)],
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
            following: self.split_following_badge(&detail.session_id, cx),
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
        Some(self.side_panel.files.link_view(&context, cx))
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
        self.side_panel.open_files_in_tab(tab);
        cx.notify();
    }

    /// The chat panel's width, widened state, and Docked vs Sheet presentation.
    pub(super) fn chat_panel_layout(
        &self,
        detail: Option<&DirectSessionEntry>,
        split_columns: usize,
        cx: &App,
    ) -> side_panel::PanelLayout {
        let settings = self.settings(cx);
        let surface = if self.showing_archived_chat() {
            PanelSurface::Chat
        } else {
            side_panel_surface(self.active_tab_is_terminal(cx))
        };
        let subject = detail.map(|detail| PanelSubject::Session(detail.session_id.clone()));
        let selection = self.active_chat_panel_selection();
        self.side_panel.files.panel_layout(
            surface,
            settings,
            &selection,
            subject.as_ref(),
            settings.chat_panel_width,
            split_columns,
            self.side_panel.maximized && self.remote_client.is_local(),
            cx,
        )
    }

    pub(super) fn session_key_copy_for(
        &mut self,
        session_id: &str,
        value: Option<String>,
        cx: &mut Context<Self>,
    ) -> Entity<CopyValueButton> {
        let copy = self
            .pane_session_key_copies
            .entry(session_id.to_owned())
            .or_insert_with(|| {
                let value = value.clone();
                cx.new(|copy_cx| {
                    CopyValueButton::new(
                        copy_cx.focus_handle(),
                        value,
                        crate::i18n::message("ui.copy_session_key"),
                    )
                })
            })
            .clone();
        copy.update(cx, |btn, btn_cx| btn.set_value(value, btn_cx));
        copy
    }

    pub(super) fn worktree_copy_for(
        &mut self,
        session_id: &str,
        path: Option<String>,
        cx: &mut Context<Self>,
    ) -> Option<Entity<CopyValueButton>> {
        let path = path?;
        let copy = self
            .pane_worktree_copies
            .entry(session_id.to_owned())
            .or_insert_with(|| {
                let path = path.clone();
                cx.new(|copy_cx| {
                    CopyValueButton::new(
                        copy_cx.focus_handle(),
                        Some(path),
                        crate::i18n::message("info.copy_worktree_path"),
                    )
                })
            })
            .clone();
        copy.update(cx, |btn, btn_cx| btn.set_value(Some(path), btn_cx));
        Some(copy)
    }

    pub(super) fn render_pane_info_card_for_session(
        &mut self,
        session_id: &str,
        menu: gpui::WeakEntity<PopoverMenu>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let shortcut_hint = crate::keymap::effective_binding(
            "toggle-session-info",
            &self.settings(cx).keymap_overrides,
        )
        .map(|combo| crate::keymap::format_combo(&combo));

        if self.remote_client.is_active() && self.route != AppRoute::ArchivedChat {
            let Some(session) = self.remote_client.session(session_id, cx) else {
                return div().into_any_element();
            };
            let branch = self.remote_client.computer().and_then(|computer| {
                remote_info_branch_row(
                    &self.app_store,
                    &computer.id,
                    None,
                    Some(session_id),
                    session.cwd.as_deref(),
                    cx,
                )
            });
            let location = resolve_info_location_rows(session.cwd.as_deref(), branch.as_ref());
            let worktree_copy = location
                .worktree
                .as_ref()
                .and_then(|wt| wt.path.clone())
                .and_then(|path| self.worktree_copy_for(session_id, Some(path), cx));
            return render_s2_remote_chat_info_card(session, branch, worktree_copy, shortcut_hint);
        }

        let detail = self
            .archived_chat_detail
            .as_ref()
            .filter(|d| self.route == AppRoute::ArchivedChat && d.session_id == session_id)
            .or_else(|| {
                self.active_chat_detail
                    .as_ref()
                    .filter(|d| d.session_id == session_id)
            })
            .or_else(|| self.app_store.read(cx).session_details.get(session_id))
            .or_else(|| self.session_entry(session_id, cx))
            .cloned();
        let Some(detail) = detail else {
            return div()
                .p(rems(12. / 16.))
                .text_size(theme::text_ui())
                .text_color(theme::faint())
                .child(crate::i18n::text("ui.loading_chat"))
                .into_any_element();
        };
        let role = detail.role_id.as_deref().and_then(|role_id| {
            self.app_store
                .read(cx)
                .roles
                .iter()
                .find(|role| role.id == role_id)
                .cloned()
        });
        let branch_cwd = detail
            .cwd
            .clone()
            .or_else(|| role.as_ref().and_then(|role| role.working_dir.clone()));
        let branch = ensure_info_branch_row(
            &self.app_store,
            branch_cwd.as_deref(),
            detail.project_id.as_deref(),
            false,
            cx,
        );
        let location = resolve_info_location_rows(branch_cwd.as_deref(), branch.as_ref());
        let session_key_copy =
            self.session_key_copy_for(&detail.session_id, detail.agent_session_key.clone(), cx);
        let worktree_copy = location
            .worktree
            .as_ref()
            .and_then(|wt| wt.path.clone())
            .and_then(|path| self.worktree_copy_for(&detail.session_id, Some(path), cx));
        let account = self.claude_account_details(&detail, cx);

        let open_root = cx.entity();
        let close_menu = menu.clone();
        let toggle_root = cx.entity();
        let toggle_menu = menu;
        let toggle_session = detail.session_id.clone();
        let expanded = self.side_panel.info_prompt_expanded(&detail.session_id);
        let actions = ChatInfoPopoverActions {
            open_role: role.as_ref().map(|role| {
                let handle = role.handle.clone();
                Rc::new(move |window: &mut Window, cx: &mut App| {
                    let _ = close_menu.update(cx, |menu, menu_cx| menu.close(menu_cx));
                    open_root.update(cx, |this, cx| {
                        this.open_role_detail(handle.clone(), window, cx)
                    });
                }) as ClickHandler
            }),
            toggle_prompt: Rc::new(move |_, cx| {
                let id = toggle_session.clone();
                toggle_root.update(cx, |this, cx| {
                    this.side_panel.toggle_info_prompt_expanded(id);
                    cx.notify();
                });
                let _ = toggle_menu.update(cx, |_, menu_cx| menu_cx.notify());
            }),
            expanded,
            view: cx.entity_id(),
            shortcut_hint,
        };
        render_s2_chat_info_card(
            &detail,
            role.as_ref(),
            session_key_copy,
            worktree_copy,
            actions,
            account,
            branch,
            cx,
        )
    }

    pub(crate) fn toggle_focused_pane_info_popover(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(session_id) = self
            .front_page_layout()
            .and_then(PaneLayout::focused_session_id)
            .map(str::to_owned)
        else {
            cx.propagate();
            return;
        };
        let is_shell = self
            .front_page_session(&session_id, cx)
            .is_some_and(|entry| entry.agent_runtime == "shell");
        if is_shell {
            cx.propagate();
            return;
        }
        let Some(menu) = self.pane_action_menus.get(&session_id).cloned() else {
            cx.propagate();
            return;
        };
        menu.update(cx, |menu, menu_cx| menu.toggle_open(window, menu_cx));
    }

    pub(crate) fn toggle_archived_chat_info_popover(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((_, menu)) = self.archived_title_menu.as_ref().cloned() else {
            cx.propagate();
            return;
        };
        menu.update(cx, |menu, menu_cx| menu.toggle_open(window, menu_cx));
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn render_chat_side_panel(
        &self,
        detail: Option<&DirectSessionEntry>,
        surface: PanelSurface,
        panel_layout: side_panel::PanelLayout,
        animated_width: f32,
        visibility: f32,
        show_panel: bool,
        border_on: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        #[cfg(test)]
        crate::render_counts::count_render("side-panel", cx);
        let remote = self.remote_client.is_active();
        let detail = detail.filter(|_| !remote);
        let width = if panel_layout.is_sheet() {
            panel_layout.width
        } else {
            animated_width
        };
        let widened = panel_layout.widened;
        let sheet = panel_layout.is_sheet();
        let maximized = show_panel && !remote && self.side_panel.maximized;
        let visible_width = if !show_panel {
            0.
        } else if sheet {
            width
        } else {
            width * visibility
        };
        if !show_panel {
            self.side_panel.files.hide(cx);
            return div()
                .id("chat-side-panel")
                .relative()
                .w(rems(visible_width / 16.))
                .h_full()
                .flex_none()
                .overflow_hidden()
                .into_any_element();
        }
        let collapse_root = cx.entity();
        let maximize_root = cx.entity();
        let req_max_root = cx.entity();
        let surface_root = cx.entity();
        let settings = self.settings(cx).clone();
        let zoom = settings.app_zoom;
        let selection = self.active_chat_panel_selection();
        let shown = side_panel::shown(surface, &settings, &selection);
        let handler: side_panel::SurfaceHandler = Rc::new(move |action, _window, cx| {
            surface_root.update(cx, |this, cx| {
                match action {
                    side_panel::SurfaceAction::Select(id) => {
                        if this.remote_client.is_active() {
                            this.remote_client.select_chat_panel_mode(id);
                        } else {
                            this.side_panel.surfaces.apply(action);
                        }
                    }
                }
                cx.notify();
            })
        });
        // Maximize is shown only when widened on an open file/diff or already maximized (#1403).
        let maximize_control =
            (!remote && (widened.is_some() || maximized)).then_some(side_panel::MaximizeControl {
                maximized,
                on_toggle: Rc::new(move |window, cx| {
                    maximize_root.update(cx, |this, cx| {
                        this.toggle_chat_panel_maximized(window, cx);
                    });
                }),
            });
        let on_request_maximize: side_panel::PanelMaximizeCallback =
            Rc::new(move |maximize, window, cx| {
                req_max_root.update(cx, |this, cx| {
                    this.set_chat_panel_maximized(maximize, window, cx);
                });
            });

        let panel_context = if remote {
            let focused = self
                .front_page_layout()
                .and_then(PaneLayout::focused_session_id);
            self.panel_session_for(focused, cx)
                .map(|(id, _)| PanelContext {
                    following: self.split_following_badge(&id, cx),
                    subject: PanelSubject::Session(id),
                    surface,
                    cwd: None,
                    project: None,
                    remote: true,
                })
        } else {
            detail.map(|detail| self.chat_panel_context(detail, surface, cx))
        };

        let chat_card = !self.showing_archived_chat() && self.front_page_is_chat();
        let macos_titlebar_inset = (maximized
            && self.front_page_sidebar_collapsed()
            && cfg!(target_os = "macos"))
        .then(|| {
            (
                self.workspace_titlebar_padding(window, cx),
                self.render_collapsed_titlebar_spacer(),
            )
        });
        let titlebar_weak = cx.weak_entity();
        let wrap_titlebar: crate::surfaces::files::TitlebarWrapper = Rc::new(move |id, area, _| {
            let out_weak = titlebar_weak.clone();
            let up_weak = titlebar_weak.clone();
            let down_weak = titlebar_weak.clone();
            #[cfg(target_os = "linux")]
            let move_weak = titlebar_weak.clone();
            let area = area
                .id(id)
                .window_control_area(gpui::WindowControlArea::Drag)
                .capture_any_mouse_down(|event, _, _| platform_ui::capture_titlebar_press(event))
                .on_mouse_down_out(move |_, _, cx| {
                    let _ = out_weak.update(cx, |this, _| this.titlebar_drag_armed = false);
                })
                .on_mouse_up(MouseButton::Left, move |_, _, cx| {
                    let _ = up_weak.update(cx, |this, _| this.titlebar_drag_armed = false);
                })
                .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                    let _ = down_weak.update(cx, |this, cx| {
                        this.titlebar_drag_armed = true;
                        platform_ui::titlebar_mouse_down(event, window, cx);
                    });
                });
            #[cfg(target_os = "linux")]
            let area = area
                .on_mouse_move(move |_, window, cx| {
                    let _ = move_weak.update(cx, |this, _| {
                        if this.titlebar_drag_armed {
                            this.titlebar_drag_armed = false;
                            window.start_window_move();
                        }
                    });
                })
                .on_click(|event, window, cx| {
                    if event.click_count() == 2 && platform_ui::titlebar_double_click(2) {
                        cx.stop_propagation();
                        window.titlebar_double_click();
                    }
                });
            area.into_any_element()
        });
        let make_collapse_button: Rc<dyn Fn(&mut App) -> AnyElement> = Rc::new(move |_| {
            let collapse_root = collapse_root.clone();
            IconButton::new("collapse-chat-panel", "panel-right-open.svg")
                .tooltip(crate::i18n::text("ui.collapse_side_panel"))
                .on_press(move |window, cx| {
                    collapse_root.update(cx, |this, cx| {
                        let return_focus = this.chat_panel_contains_focus(window, cx)
                            || (!remote && this.side_panel.maximized);
                        this.set_chat_panel_open(false, cx);
                        if return_focus && this.route == AppRoute::Chat {
                            this.focus_active_terminal(window, cx);
                        }
                        cx.notify();
                    });
                })
                .into_any_element()
        });
        let loading_fallback = Some(
            div()
                .flex_1()
                .p_5()
                .text_size(theme::text_ui())
                .text_color(theme::faint())
                .child(crate::i18n::text("ui.loading_chat"))
                .into_any_element(),
        );
        let owner = cx.entity_id();
        let content = self.side_panel.files.render_panel(
            crate::surfaces::files::SidePanelRenderProps {
                surface,
                settings,
                selection,
                shown,
                context: panel_context,
                widened,
                maximized,
                remote,
                owner,
                header_selector: "CHAT_PANEL_HEADER",
                titlebar_id: "chat-panel-titlebar-drag",
                round_header_tl_when_compact: false,
                macos_titlebar_inset,
                wrap_titlebar,
                on_surface_action: handler,
                maximize_control,
                on_request_maximize,
                make_collapse_button,
                sessions_body: None,
                sessions_focus: None,
                loading_fallback,
            },
            cx,
        );
        let border = if chat_card || sheet {
            2.
        } else if border_on {
            1.
        } else {
            0.
        };
        let panel = div()
            .relative()
            .when(maximized, |p| p.w_full().flex_1().min_w(px(0.)))
            .when(!maximized, |p| {
                if sheet || visibility >= 1. {
                    p.w_full()
                } else {
                    p.w(px(width * zoom - border)).flex_none()
                }
            })
            .h_full()
            .flex()
            .flex_col()
            .overflow_hidden()
            .child(content);
        div()
            .id("chat-side-panel")
            .debug_selector(|| "CHAT_SIDE_PANEL".into())
            .track_focus(&self.side_panel.focus)
            .relative()
            .when(maximized, |p| p.flex_1().min_w(px(0.)).w_full())
            .when(!maximized, |p| p.w(rems(visible_width / 16.)).flex_none())
            .h_full()
            .overflow_hidden()
            .when(chat_card || sheet, |panel| {
                panel
                    .rounded(rems(12. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::bg())
            })
            .when(!chat_card && !sheet && border_on && !maximized, |panel| {
                panel.border_l_1().border_color(theme::border())
            })
            .child(panel)
            .into_any_element()
    }
}
