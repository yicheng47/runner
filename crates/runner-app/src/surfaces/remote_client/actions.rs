use super::*;
use crate::surfaces::front_page::{
    actions::{Completion, ShellStart, WorkspaceAction},
    FrontPageClient,
};
use runner_core::protocol::ClientResult;

impl RemoteClient {
    pub(crate) fn prepare_action(
        &mut self,
        client: FrontPageClient,
        key: String,
        required: &[&str],
        cx: &mut Context<NativeRoot>,
    ) -> Option<WorkspaceAction> {
        if !client.supports(required) {
            return None;
        }
        let remote = if client.is_remote() {
            let remote = self.workspace.as_mut()?;
            if remote.front_page.target != client.target || !client.valid() {
                return None;
            }
            if key.starts_with("layout:") && remote.actions_pending.contains("archive-sessions") {
                return None;
            }
            if !remote.actions_pending.insert(key.clone()) {
                return None;
            }
            remote.action_error = None;
            cx.notify();
            Some((remote.layouts.clone(), remote.navigation_generation))
        } else {
            None
        };
        Some(WorkspaceAction {
            client,
            key,
            epoch: self.ui.epoch,
            remote,
            pending_archive: None,
        })
    }
    pub(crate) fn complete_action<T>(
        &mut self,
        action: WorkspaceAction,
        result: Result<T, String>,
        remote_completion: impl FnOnce(T) -> Completion,
        cx: &mut Context<NativeRoot>,
    ) -> bool {
        let Some((before, navigation_generation)) = action.remote else {
            return false;
        };
        let Some(current) = self.workspace.as_ref() else {
            return false;
        };
        if self.ui.epoch != action.epoch
            || current.front_page.target != action.client.target
            || !action.client.valid()
        {
            return false;
        }
        let Some(remote) = self.workspace.as_mut() else {
            return false;
        };
        remote.actions_pending.remove(&action.key);
        match result {
            Ok(done) => self.apply_remote_completion(
                remote_completion(done),
                &before,
                navigation_generation,
                cx,
            ),
            Err(error) => {
                if remote.navigation_generation == navigation_generation {
                    remote.action_error = Some(error);
                }
                if let Some(tab) = action.key.strip_prefix("layout:") {
                    remote.closing_layouts.remove(tab);
                    remote.layout_hints.remove(tab);
                    if remote.navigation_generation != navigation_generation {
                        let mut staged = remote.shell_placement.lock().unwrap();
                        if staged.as_ref().is_some_and(|(_, layout)| layout.id == tab) {
                            *staged = None;
                        }
                    }
                }
            }
        }
        true
    }
    pub(crate) fn remote_action(
        &mut self,
        key: String,
        required: &[&str],
        operation: impl FnOnce(FrontPageClient) -> ClientResult<Completion> + Send + 'static,
        cx: &mut Context<NativeRoot>,
    ) {
        if self.workspace.is_none() {
            return;
        }
        let Some(action) = self.prepare_action(
            self.workspace.as_ref().unwrap().front_page.clone(),
            key,
            required,
            cx,
        ) else {
            return;
        };
        let client = action.client.clone();
        let required = required.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let work = cx.background_spawn(async move {
            if !client.supports(&required.iter().map(String::as_str).collect::<Vec<_>>()) {
                return Err("The selected computer changed.".to_owned());
            }
            operation(client).map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |root, cx| {
                WorkspaceAction::complete(root, action, result, |done| done, cx);
            });
        })
        .detach();
        cx.notify();
    }
    pub(crate) fn apply_remote_completion(
        &mut self,
        done: Completion,
        before: &HashMap<String, PaneLayout>,
        navigation_generation: u64,
        cx: &mut Context<NativeRoot>,
    ) {
        let Some(remote) = self.workspace.as_mut() else {
            return;
        };
        let mut activate = done.layout.is_none();
        let mut placement = None;
        for mut layout in done.layout.into_iter().chain(done.layouts) {
            {
                let mut staged = remote.shell_placement.lock().unwrap();
                if staged
                    .as_ref()
                    .is_some_and(|(_, pending)| pending.id == layout.id)
                {
                    *staged = None;
                }
            }
            let current = self
                .app_store
                .read(cx)
                .remote_clients
                .read(cx)
                .overviews
                .get(&remote.computer.id)
                .and_then(|overview| overview.tabs.iter().find(|tab| tab.id == layout.id));
            let superseded = match current {
                Some(tab) => {
                    !crate::surfaces::front_page::model::matches_layout(&layout, tab)
                        && before.get(&layout.id).is_none_or(|previous| {
                            !crate::surfaces::front_page::model::matches_layout(previous, tab)
                        })
                }
                None => before.contains_key(&layout.id),
            };
            if !superseded {
                if let Some(session) = &done.session {
                    if let Some(leaf) = layout
                        .root
                        .leaves()
                        .into_iter()
                        .find(|leaf| leaf.session_id.as_ref() == Some(session))
                    {
                        placement = Some((layout.id.clone(), Some(leaf.id.clone())));
                        activate = true;
                    } else if layout.drawer_shells().contains(session) {
                        placement = Some((layout.id.clone(), None));
                        activate = true;
                    }
                }
                if remote.navigation_generation != navigation_generation {
                    if let Some(previous) = remote.layouts.get(&layout.id) {
                        crate::surfaces::front_page::model::retain_presentation(
                            &mut layout,
                            previous,
                        );
                    }
                }
                remote.layout_hints.insert(layout.id.clone(), layout);
            }
        }
        remote.reopen_sessions.extend(done.reopened_sessions);
        if let Some(session) = done.session {
            remote.reopen_sessions.insert(session.clone());
            if activate && remote.navigation_generation == navigation_generation {
                remote.activate_session = Some(session);
                remote.activate_placement = placement;
            }
        }
        if let Some(tab) = done
            .activate_tab
            .filter(|_| remote.navigation_generation == navigation_generation)
        {
            remote.activate_tab = Some(tab);
        }
    }
}

pub(crate) type ShellPlacement = Arc<std::sync::Mutex<Option<(String, PaneLayout)>>>;

pub(crate) struct ShellPreparation {
    pub(crate) staged: Option<ShellPlacement>,
    pub(crate) pending: Option<(String, PaneLayout)>,
}
impl RemoteClient {
    pub(crate) fn begin_named_action(&mut self, key: &str) -> bool {
        self.workspace
            .as_mut()
            .is_none_or(|remote| remote.actions_pending.insert(key.to_owned()))
    }
    pub(crate) fn finish_named_action(&mut self, key: &str) {
        if let Some(remote) = self.workspace.as_mut() {
            remote.actions_pending.remove(key);
        }
    }
    pub(crate) fn layout_pending(&self, id: &str) -> bool {
        self.workspace.as_ref().is_some_and(|remote| {
            remote.layout_hints.contains_key(id)
                || remote.actions_pending.contains("archive-sessions")
                || remote.actions_pending.contains(&format!("layout:{id}"))
        })
    }
    pub(crate) fn prepare_layout(
        &mut self,
        draft: &PaneLayout,
        cx: &mut Context<NativeRoot>,
    ) -> Option<WorkspaceAction> {
        if self.layout_pending(&draft.id) {
            return None;
        }
        let client = self.workspace.as_ref()?.front_page.clone();
        let action =
            self.prepare_action(client, format!("layout:{}", draft.id), &["layout"], cx)?;
        self.workspace
            .as_mut()?
            .layout_hints
            .insert(draft.id.clone(), draft.clone());
        Some(action)
    }
    pub(crate) fn prepare_shell_input(&self, input: &mut ShellStart) -> Option<ShellPreparation> {
        let staged = self
            .workspace
            .as_ref()
            .map(|remote| remote.shell_placement.clone());
        let pending = staged
            .as_ref()
            .and_then(|staged| staged.lock().unwrap().clone());
        if let Some(remote) = self.workspace.as_ref() {
            if remote.actions_pending.contains("archive-sessions")
                || remote
                    .actions_pending
                    .iter()
                    .any(|key| key.starts_with("layout:"))
            {
                return None;
            }
            if let Some((_, draft)) = &pending {
                input.draft = draft.clone();
            }
            if remote.layout_hints.contains_key(&input.draft.id) {
                return None;
            }
            input.project = input.draft.parent_id.clone();
            input.size = (input.size.0.max(20), input.size.1.max(8));
        }
        Some(ShellPreparation { staged, pending })
    }
    pub(crate) fn archive_layouts(&self, ids: &[String]) -> Option<Vec<PaneLayout>> {
        let layouts = self
            .workspace
            .as_ref()
            .map(|remote| {
                remote
                    .layouts
                    .values()
                    .filter(|layout| layout.all_session_ids().iter().any(|id| ids.contains(id)))
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if let Some(remote) = self.workspace.as_ref() {
            if layouts.iter().any(|layout| {
                remote.layout_hints.contains_key(&layout.id)
                    || remote
                        .actions_pending
                        .contains(&format!("layout:{}", layout.id))
            }) {
                return None;
            }
        }
        Some(layouts)
    }
    pub(crate) fn close_layout(
        &self,
        session: &str,
        pane: Option<&str>,
        drawer: bool,
        cx: &App,
    ) -> Option<(Option<PaneLayout>, bool)> {
        let mut draft = self.layout()?.clone();
        if self
            .workspace
            .as_ref()
            .is_some_and(|remote| remote.layout_hints.contains_key(&draft.id))
        {
            return None;
        }
        let shell = self.session(session, cx)?.agent_runtime == "shell";
        if drawer {
            draft.remove_drawer_shell(session);
        } else if let Some(pane) = pane {
            draft.close_pane(pane);
            draft.remove_session(session);
        } else {
            draft.remove_session(session);
        }
        Some((Some(draft), shell))
    }
}
