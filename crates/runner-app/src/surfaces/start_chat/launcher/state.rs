use super::*;

pub(in crate::surfaces::start_chat) enum TerminalStart {
    NoLauncher,
    Blocked,
    Ready(Entity<ChatLauncher>, Box<LaunchRequest>),
}

/// Cached `ChatLauncher` views and launcher-page bookkeeping owned by `NativeRoot`.
#[derive(Default)]
pub(crate) struct LauncherState {
    entries: HashMap<String, Entity<ChatLauncher>>,
}

impl LauncherState {
    fn cache_key(target: &FrontPageTarget, pane: Option<&PaneKey>) -> String {
        format!("{target:?}:{pane:?}")
    }

    pub(crate) fn page(&self, target: &FrontPageTarget) -> Option<&Entity<ChatLauncher>> {
        self.entries.get(&Self::cache_key(target, None))
    }

    pub(crate) fn page_has_update_retry(&self, target: &FrontPageTarget, cx: &App) -> bool {
        self.page(target)
            .is_some_and(|launcher| launcher.read(cx).has_update_retry())
    }

    pub(crate) fn page_origin(
        &self,
        target: &FrontPageTarget,
        page_visible: bool,
        cx: &App,
    ) -> Option<LauncherOrigin> {
        self.page(target)
            .filter(|_| page_visible)
            .and_then(|page| page.read(cx).origin.clone())
    }

    pub(super) fn get(
        &self,
        target: &FrontPageTarget,
        pane: Option<&PaneKey>,
    ) -> Option<&Entity<ChatLauncher>> {
        self.entries.get(&Self::cache_key(target, pane))
    }

    pub(super) fn insert(
        &mut self,
        target: &FrontPageTarget,
        pane: Option<&PaneKey>,
        launcher: Entity<ChatLauncher>,
    ) {
        self.entries.insert(Self::cache_key(target, pane), launcher);
    }

    pub(crate) fn remove(&mut self, launcher: &Entity<ChatLauncher>) {
        self.entries.retain(|_, cached| cached != launcher);
    }

    /// The project the launcher page's row sits under, Recents when `None`.
    /// Until a reconnect's new launcher is built, the one it replaces still
    /// says, so the row does not move in between.
    pub(crate) fn page_project(&self, target: &FrontPageTarget, cx: &App) -> Option<String> {
        let computer = computer_key(target);
        let launcher = self.page(target).or_else(|| {
            self.entries.values().find(|launcher| {
                let launcher = launcher.read(cx);
                launcher.pane.is_none() && computer_key(&launcher.client.target) == computer
            })
        })?;
        launcher.read(cx).page_project.clone()
    }

    /// Removes the page launcher a reconnect left behind, whose client is no
    /// longer valid, and returns the project it was locked to.
    pub(super) fn take_replaced_page_project(
        &mut self,
        target: &FrontPageTarget,
        cx: &App,
    ) -> Option<String> {
        let mut project = None;
        self.entries.retain(|_, launcher| {
            let launcher = launcher.read(cx);
            let replaced = launcher.pane.is_none()
                && &launcher.client.target != target
                && computer_key(&launcher.client.target) == computer_key(target);
            if replaced {
                project = launcher.page_project.clone();
            }
            !replaced
        });
        project
    }

    /// Forgets the page launcher, which is cached between visits and could hold
    /// an older project, lock and origin. The next page starts fresh.
    pub(crate) fn reset_page(&mut self, target: &FrontPageTarget, cx: &App) {
        let computer = computer_key(target);
        let mut waiting = Vec::new();
        self.entries.retain(|_, entity| {
            let launcher = entity.read(cx);
            if launcher.pane.is_some() || computer_key(&launcher.client.target) != computer {
                return true;
            }
            if launcher.update_retry.is_some() {
                waiting.push((
                    format!("update-wait:{:?}", entity.entity_id()),
                    entity.clone(),
                ));
            }
            false
        });
        self.entries.extend(waiting);
    }

    pub(super) fn bind_reserved_pane(
        &mut self,
        launcher: &Entity<ChatLauncher>,
        pane: PaneKey,
        scope: &ProjectScope,
        cx: &mut App,
    ) {
        self.remove(launcher);
        let target = launcher.update(cx, |view, _| {
            view.pane = Some(pane.clone());
            view.initial_project = scope.project_id().map(str::to_owned);
            view.client.target.clone()
        });
        self.insert(&target, Some(&pane), launcher.clone());
    }

    pub(crate) fn notify_all(&self, cx: &mut App) {
        for launcher in self.entries.values() {
            launcher.update(cx, |_, cx| cx.notify());
        }
    }

    pub(crate) fn apply_remote_runtimes(
        &self,
        target: &FrontPageTarget,
        runtimes: &[runner_core::companion::CatalogRuntime],
        cx: &mut App,
    ) {
        for launcher in self.entries.values() {
            if &launcher.read(cx).client.target == target {
                launcher.update(cx, |launcher, cx| {
                    launcher.apply_remote_runtimes(runtimes, cx);
                });
            }
        }
    }

    pub(crate) fn refresh_local(&self, store: &Entity<AppStore>, cx: &mut App) {
        let (runtimes, roles, default, overrides, error) = {
            let store = store.read(cx);
            let (runtimes, _, error) = load_selectable_runtimes(&store.client, &store.settings);
            let runtimes = runtimes
                .into_iter()
                .map(Into::into)
                .collect::<Vec<RuntimeChoice>>();
            let roles = store
                .roles
                .clone()
                .into_iter()
                .map(Into::into)
                .collect::<Vec<RoleChoice>>();
            (
                runtimes,
                roles,
                store.settings.default_runtime.clone(),
                store.settings.keymap_overrides.clone(),
                error,
            )
        };
        for launcher in self.entries.values() {
            launcher.update(cx, |launcher, cx| {
                launcher.apply_local_refresh(
                    &runtimes,
                    &roles,
                    &default,
                    &overrides,
                    error.as_ref(),
                    cx,
                );
            });
        }
    }

    pub(crate) fn mark_update_ready(
        &self,
        runtime: Runtime,
        target: &FrontPageTarget,
        cx: &mut App,
    ) {
        for launcher in self.entries.values() {
            launcher.update(cx, |view, _| {
                view.mark_update_ready(runtime, target);
            });
        }
    }

    pub(super) fn snapshot(&self) -> Vec<Entity<ChatLauncher>> {
        self.entries.values().cloned().collect()
    }

    pub(super) fn take_ready_launch(
        launcher: &Entity<ChatLauncher>,
        current_target: &FrontPageTarget,
        store: &Entity<AppStore>,
        remote_client: &crate::surfaces::remote_client::RemoteClient,
        cx: &mut App,
    ) -> Option<UpdateRetry> {
        let view = launcher.read(cx);
        let retry = view.update_retry.as_ref().filter(|retry| retry.ready)?;
        if !view.client.valid()
            || (view.client.is_remote() && &view.client.target != current_target)
        {
            return None;
        }
        if let UpdateAction::Chat(LaunchRequest {
            pane: Some(pane), ..
        }) = &retry.action
        {
            let blocked = if view.client.is_remote() {
                Self::tab_pending(current_target, &pane.tab_id, store, remote_client, cx)
            } else {
                let key = format!("{}:{}", computer_key(&view.client.target), pane.tab_id);
                store
                    .read(cx)
                    .launcher_tabs_pending
                    .lock()
                    .unwrap()
                    .contains_key(&key)
            };
            if blocked {
                return None;
            }
        }
        launcher.update(cx, |view, _| view.update_retry.take())
    }

    pub(crate) fn prune(
        &mut self,
        target: &FrontPageTarget,
        active_layout: Option<&PaneLayout>,
        tabs: &TabSet,
        remote_client: &crate::surfaces::remote_client::RemoteClient,
        cx: &App,
    ) {
        self.entries.retain(|_, entity| {
            let launcher = entity.read(cx);
            if launcher.update_retry.is_some() && launcher.client.valid() {
                return true;
            }
            if &launcher.client.target != target {
                // A reconnect's new launcher takes the page over when it is built.
                return launcher.pane.is_none()
                    && computer_key(&launcher.client.target) == computer_key(target);
            }
            if launcher.launched_session.as_ref().is_some_and(|session| {
                active_layout.is_some_and(|layout| layout.contains_session(session))
            }) {
                return false;
            }
            let Some(pane) = &launcher.pane else {
                return true;
            };
            let layout = if launcher.client.is_remote() {
                remote_client.tab_layout(&pane.tab_id)
            } else {
                tabs.tabs().iter().find(|layout| layout.id == pane.tab_id)
            };
            layout.is_none_or(|layout| {
                layout
                    .root
                    .leaves()
                    .iter()
                    .find(|leaf| leaf.id == pane.pane_id)
                    .is_none_or(|leaf| leaf.session_id.is_none())
            })
        });
    }

    pub(crate) fn is_visible(
        &self,
        launcher: &Entity<ChatLauncher>,
        current_target: &FrontPageTarget,
        front_page_is_chat: bool,
        page_visible: bool,
        active_layout: Option<&PaneLayout>,
        cx: &App,
    ) -> bool {
        let view = launcher.read(cx);
        if &view.client.target != current_target {
            return false;
        }
        if let Some(pane) = &view.pane {
            front_page_is_chat
                && active_layout.is_some_and(|layout| {
                    layout.id == pane.tab_id && layout.focused_pane_id == pane.pane_id
                })
        } else {
            page_visible && self.page(current_target) == Some(launcher)
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn focus_visible(
        &self,
        current_target: &FrontPageTarget,
        front_page_is_chat: bool,
        create_modal_open: bool,
        page_visible: bool,
        active_layout: Option<&PaneLayout>,
        root_focus: &FocusHandle,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if !front_page_is_chat || create_modal_open {
            return false;
        }
        let pane = active_layout.map(|layout| PaneKey::new(&layout.id, &layout.focused_pane_id));
        if active_layout.is_some_and(|layout| layout.focused_session_id().is_some()) {
            return false;
        }
        if let Some(launcher) = self
            .get(current_target, pane.as_ref())
            .filter(|launcher| {
                self.is_visible(
                    launcher,
                    current_target,
                    front_page_is_chat,
                    page_visible,
                    active_layout,
                    cx,
                )
            })
            .cloned()
        {
            if launcher.read(cx).update_retry.is_some() {
                root_focus.focus(window, cx);
            } else {
                launcher.update(cx, |launcher, cx| launcher.focus(window, cx));
            }
            return true;
        }
        false
    }

    pub(crate) fn start_pending(
        &self,
        current_target: &FrontPageTarget,
        active_layout: Option<&PaneLayout>,
        cx: &App,
    ) -> bool {
        self.entries.values().any(|launcher| {
            let launcher = launcher.read(cx);
            &launcher.client.target == current_target
                && launcher.pending.is_some()
                && launcher.pane.as_ref().is_none_or(|pane| {
                    active_layout.is_some_and(|layout| {
                        layout.id == pane.tab_id
                            && layout
                                .root
                                .leaves()
                                .iter()
                                .any(|leaf| leaf.id == pane.pane_id && leaf.session_id.is_none())
                    })
                })
        })
    }

    pub(in crate::surfaces::start_chat) fn begin_terminal_start(
        &self,
        current_target: &FrontPageTarget,
        front_page_is_chat: bool,
        active_layout: Option<&PaneLayout>,
        cx: &mut App,
    ) -> TerminalStart {
        if !front_page_is_chat {
            return TerminalStart::NoLauncher;
        }
        let pane = active_layout.map(|layout| PaneKey::new(&layout.id, &layout.focused_pane_id));
        if active_layout.is_some_and(|layout| layout.focused_session_id().is_some()) {
            return TerminalStart::NoLauncher;
        }
        let launcher = self
            .entries
            .values()
            .find(|launcher| {
                let launcher = launcher.read(cx);
                &launcher.client.target == current_target && launcher.pane == pane
            })
            .cloned();
        let Some(launcher) = launcher else {
            return TerminalStart::NoLauncher;
        };
        match launcher.update(cx, |launcher, cx| launcher.begin_terminal_start(cx)) {
            Some(request) => TerminalStart::Ready(launcher, Box::new(request)),
            None => TerminalStart::Blocked,
        }
    }

    pub(crate) fn tab_pending(
        target: &FrontPageTarget,
        tab: &str,
        store: &Entity<AppStore>,
        remote_client: &crate::surfaces::remote_client::RemoteClient,
        cx: &App,
    ) -> bool {
        let key = format!("{}:{tab}", computer_key(target));
        let pending = store.read(cx).launcher_tabs_pending.clone();
        let mut pending = pending.lock().unwrap();
        if let (FrontPageTarget::Remote { computer, .. }, Some(Some(before))) =
            (target, pending.get(&key))
        {
            let clients = store.read(cx).remote_clients.read(cx);
            if clients.overviews.get(computer).is_some_and(|overview| {
                overview
                    .tabs
                    .iter()
                    .find(|candidate| candidate.id == tab)
                    .is_none_or(|tab| {
                        !crate::surfaces::front_page::model::matches_layout(before, tab)
                    })
            }) {
                pending.remove(&key);
            }
        }
        pending.contains_key(&key) || remote_client.layout_pending(tab)
    }

    pub(crate) fn session_pending(
        target: &FrontPageTarget,
        session: &str,
        tabs: &TabSet,
        store: &Entity<AppStore>,
        remote_client: &crate::surfaces::remote_client::RemoteClient,
        cx: &App,
    ) -> bool {
        let layout = if remote_client.is_active() {
            remote_client.session_tab_layout(session)
        } else {
            tabs.tabs()
                .iter()
                .find(|layout| layout.contains_session(session))
        };
        layout.is_some_and(|layout| Self::tab_pending(target, &layout.id, store, remote_client, cx))
    }

    pub(super) fn begin_tab_start(
        displayed_target: &FrontPageTarget,
        client_target: &FrontPageTarget,
        layout: &PaneLayout,
        store: &Entity<AppStore>,
        remote_client: &crate::surfaces::remote_client::RemoteClient,
        cx: &App,
    ) -> Option<TabStart> {
        if Self::tab_pending(displayed_target, &layout.id, store, remote_client, cx) {
            return None;
        }
        let key = format!("{}:{}", computer_key(client_target), layout.id);
        TabStart::begin(
            store.read(cx).launcher_tabs_pending.clone(),
            key,
            layout.clone(),
        )
    }

    /// With no tab or page behind it the launcher is the window's empty state,
    /// and there is nothing to close it back to. A page with no recorded origin
    /// is that state after a tab arrived from another window, and closes to it.
    pub(crate) fn page_closable(
        &self,
        target: &FrontPageTarget,
        page_visible: bool,
        remote_active: bool,
        has_tabs: bool,
        cx: &App,
    ) -> bool {
        page_visible
            && self
                .page(target)
                .is_some_and(|launcher| match &launcher.read(cx).origin {
                    Some(mission::LauncherOrigin::Local {
                        route: AppRoute::Chat,
                        ..
                    }) => has_tabs,
                    None => !remote_active && has_tabs,
                    Some(mission::LauncherOrigin::Remote(None)) => false,
                    Some(_) => true,
                })
    }

    pub(crate) fn close_page(
        &self,
        target: &FrontPageTarget,
        page_visible: bool,
        remote_active: bool,
        has_tabs: bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        if !self.page_closable(target, page_visible, remote_active, has_tabs, cx) {
            return;
        }
        if let Some(launcher) = self.page(target).cloned() {
            launcher.update(cx, |launcher, cx| launcher.leave_page(window, cx));
        }
    }

    pub(super) fn effective_default_page(
        pane: Option<&PaneKey>,
        default_page: bool,
        active_layout: Option<&PaneLayout>,
    ) -> bool {
        pane.and_then(|pane| active_layout.filter(|layout| layout.id == pane.tab_id))
            .map_or(default_page, |layout| {
                let leaves = layout.root.leaves();
                leaves.len() == 1 && leaves[0].session_id.is_none()
            })
    }

    pub(super) fn context_changed(
        launcher: &Entity<ChatLauncher>,
        pane: Option<&PaneKey>,
        default_page: bool,
        active_tab_project_id: impl FnOnce() -> Option<String>,
        cx: &App,
    ) -> bool {
        let view = launcher.read(cx);
        view.default_page != default_page
            || (pane.is_some() && !default_page && view.initial_project != active_tab_project_id())
    }

    pub(super) fn replace_with_carry_over(
        &mut self,
        target: &FrontPageTarget,
        pane: Option<&PaneKey>,
        previous: &Entity<ChatLauncher>,
        next: Entity<ChatLauncher>,
        cx: &mut App,
    ) -> Entity<ChatLauncher> {
        let (query, directory) = {
            let previous = previous.read(cx);
            (
                previous.query.read(cx).text().to_owned(),
                previous
                    .changed_directory
                    .then(|| previous.cwd.read(cx).text().to_owned()),
            )
        };
        next.update(cx, |launcher, cx| {
            launcher.carry_over_draft(query, directory, cx);
        });
        self.insert(target, pane, next.clone());
        next
    }

    pub(super) fn reconcile_cached(
        launcher: &Entity<ChatLauncher>,
        target: &FrontPageTarget,
        store: &Entity<AppStore>,
        computer: Option<runner_core::remote::Computer>,
        window: &mut Window,
        cx: &mut Context<NativeRoot>,
    ) {
        if launcher
            .read(cx)
            .update_retry
            .as_ref()
            .is_some_and(|retry| retry.ready)
        {
            cx.defer_in(window, |root, window, cx| {
                root.drain_ready_launches(window, cx)
            });
        }
        let (overrides, revisions) = {
            let store = store.read(cx);
            (store.settings.keymap_overrides.clone(), store.revisions)
        };
        let catalog_revisions = (revisions.roles, revisions.projects, revisions.crews);
        let catalogs = (!launcher.read(cx).client.is_remote()
            && launcher.read(cx).catalog_revisions != catalog_revisions)
            .then(|| {
                let store = store.read(cx);
                LocalCatalogSnapshot {
                    roles: store
                        .roles
                        .clone()
                        .into_iter()
                        .map(Into::into)
                        .collect::<Vec<RoleChoice>>(),
                    projects: ProjectChoice::local(&store.projects),
                    crews: mission::local_crews(&store.crews),
                    revisions: catalog_revisions,
                }
            });
        let recent_revision = RECENT_REVISION.load(std::sync::atomic::Ordering::Acquire);
        let project_deleted = launcher
            .read(cx)
            .page_project
            .as_deref()
            .is_some_and(|project| front_page_project_deleted(target, store, project, cx));
        launcher.update(cx, |launcher, cx| {
            launcher.reconcile(
                recent_revision,
                catalogs,
                project_deleted,
                overrides,
                computer,
                window,
                cx,
            );
        });
    }

    #[cfg(test)]
    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[cfg(test)]
    pub(crate) fn values(&self) -> impl Iterator<Item = &Entity<ChatLauncher>> {
        self.entries.values()
    }

    #[cfg(test)]
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&String, &Entity<ChatLauncher>)> {
        self.entries.iter()
    }

    /// Whether a mission entry point reached the crew or goal step.
    #[cfg(test)]
    pub(crate) fn any_mission_open(&self, cx: &App) -> bool {
        self.entries
            .values()
            .any(|launcher| launcher.read(cx).step != Step::Start)
    }

    #[cfg(test)]
    pub(crate) fn any_search_focused(&self, window: &Window, cx: &App) -> bool {
        self.entries.values().any(|launcher| {
            launcher
                .read(cx)
                .field()
                .read(cx)
                .focus_handle()
                .is_focused(window)
        })
    }
}

fn front_page_project_deleted(
    target: &FrontPageTarget,
    store: &Entity<AppStore>,
    project: &str,
    cx: &App,
) -> bool {
    match target {
        FrontPageTarget::Local => !store.read(cx).projects.iter().any(|row| row.id == project),
        FrontPageTarget::Remote { computer, .. } => store
            .read(cx)
            .remote_clients
            .read(cx)
            .overviews
            .get(computer)
            .is_some_and(|overview| !overview.projects.iter().any(|row| row.id == project)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_default_page_tracks_single_empty_leaf_for_pane_launchers() {
        let mut single = PaneLayout::single(None, &[]);
        single.id = "tab-1".into();
        let pane = PaneKey::new("tab-1", &single.focused_pane_id);
        assert!(LauncherState::effective_default_page(
            Some(&pane),
            false,
            Some(&single)
        ));

        let mut occupied = PaneLayout::single(Some("s1"), &["s1".into()]);
        occupied.id = "tab-1".into();
        assert!(!LauncherState::effective_default_page(
            Some(&pane),
            true,
            Some(&occupied)
        ));

        let mut split = PaneLayout::single(None, &[]);
        split.id = "tab-1".into();
        split.split("p1", SplitOrientation::Row).unwrap();
        assert!(!LauncherState::effective_default_page(
            Some(&pane),
            true,
            Some(&split)
        ));

        assert!(LauncherState::effective_default_page(
            None,
            true,
            Some(&split)
        ));
        assert!(!LauncherState::effective_default_page(None, false, None));
    }

    #[test]
    fn cache_key_separates_page_and_pane_targets() {
        let pane = PaneKey::new("tab-1", "p1");
        let local_page = LauncherState::cache_key(&FrontPageTarget::Local, None);
        let local_pane = LauncherState::cache_key(&FrontPageTarget::Local, Some(&pane));
        let remote_page = LauncherState::cache_key(
            &FrontPageTarget::Remote {
                computer: "host".into(),
                fingerprint: "fp".into(),
                generation: 1,
            },
            None,
        );
        assert_ne!(local_page, local_pane);
        assert_ne!(local_page, remote_page);
    }
}
