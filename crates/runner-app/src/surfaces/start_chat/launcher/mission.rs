use super::*;
use crate::surfaces::remote_client::RemoteItem;
use runner_core::companion::Catalog;
use runner_core::protocol::crew::CrewListItem;
use runner_core::protocol::model::SlotWithRole;

pub(super) const SHIFT_ENTER: &str = "⇧↵";

/// The view to return to when leaving the launcher page.
#[derive(Clone)]
pub(crate) enum LauncherOrigin {
    Local {
        route: AppRoute,
        tab: Option<String>,
    },
    Remote(Option<RemoteItem>),
}

struct MissionRequest {
    crew: String,
    title: String,
    goal: Option<String>,
    project: Option<String>,
    cwd: Option<String>,
}

pub(super) fn local_crews(crews: &[CrewListItem]) -> Vec<CrewEntry> {
    crews
        .iter()
        .map(|item| CrewEntry {
            id: item.crew.id.clone(),
            name: item.crew.name.clone(),
            handles: item
                .members
                .iter()
                .map(|member| member.slot_handle.clone())
                .collect(),
            roles: usize::try_from(item.role_count).unwrap_or_default(),
            slots: None,
        })
        .collect()
}

pub(super) fn host_crews(catalog: &Catalog) -> Vec<CrewEntry> {
    catalog
        .crews
        .iter()
        .map(|crew| CrewEntry {
            id: crew.id.clone(),
            name: crew.name.clone(),
            handles: crew.slots.iter().map(|slot| slot.handle.clone()).collect(),
            roles: crew.slots.len(),
            slots: Some(
                crew.slots
                    .iter()
                    .map(|slot| {
                        let role = catalog.roles.iter().find(|role| role.id == slot.role);
                        SlotLine {
                            handle: slot.handle.clone(),
                            role: role.map(|role| role.name.clone()).unwrap_or_default(),
                            // The catalog does not export slot overrides.
                            setup: String::new(),
                        }
                    })
                    .collect(),
            ),
        })
        .collect()
}

fn slot_line(member: &SlotWithRole, runtimes: &[RuntimeChoice]) -> SlotLine {
    let setup = crate::surfaces::crews::slot_setup(member);
    SlotLine {
        handle: member.slot.slot_handle.clone(),
        role: member.role.display_name.clone(),
        setup: setup_text(
            setup
                .model
                .or_else(|| Some(runtime_display_name(runtimes, &setup.runtime))),
            setup.effort,
        ),
    }
}

fn setup_text(model: Option<String>, effort: Option<String>) -> String {
    [model, effort]
        .into_iter()
        .flatten()
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

fn crew_row(crew: &CrewEntry, label: Option<&str>) -> Row {
    let members = if crew.roles == 0 {
        crate::i18n::text("ui.no_roles_yet").to_owned()
    } else {
        crew.handles.join(", ")
    };
    Row {
        choice: Choice::Crew(crew.id.clone()),
        name: crew.name.clone(),
        detail: match label {
            Some(label) => format!("{label} · {members}"),
            None => members,
        },
        runtime: None,
        avatar: None,
        crew: Some(crew.handles.iter().cloned().map(Into::into).collect()),
        unavailable: false,
        dimmed: crew.roles == 0,
        used_at: None,
        previously_used: false,
    }
}

impl Row {
    pub(super) fn pinned(choice: Choice, name: &str, detail: &str) -> Self {
        Self {
            choice,
            name: name.into(),
            detail: detail.into(),
            runtime: None,
            avatar: None,
            crew: None,
            unavailable: false,
            dimmed: false,
            used_at: None,
            previously_used: false,
        }
    }
}

pub(crate) fn record_launcher_crew(directory: &Path, target: &FrontPageTarget, crew: String) {
    let _writer = RECENT_WRITER.lock().unwrap();
    let mut store = RecentStore::load(directory);
    store.record_crew(&computer_key(target), crew, chrono::Utc::now().timestamp());
    match store.save(directory) {
        Ok(()) => {
            RECENT_REVISION.fetch_add(1, std::sync::atomic::Ordering::Release);
        }
        Err(error) => tracing::warn!(%error, "could not save launcher history"),
    }
}

impl ChatLauncher {
    /// Whether this launcher starts missions: only a tab's own page can,
    /// since a mission takes the whole tab.
    pub(super) fn offers_missions(&self) -> bool {
        self.default_page && self.client.supports(&["catalog", "startMission"])
    }

    pub(super) fn offers_empty_missions(&self) -> bool {
        self.offers_missions() && !self.client.is_remote()
    }

    pub(super) fn crew_rows(&self, cx: &App) -> Vec<Row> {
        let recent = self.recents.crews(&computer_key(&self.client.target));
        let mut rows = model::ranked_crews(&self.crews, recent, self.query.read(cx).text())
            .into_iter()
            .map(|crew| crew_row(crew, None))
            .collect::<Vec<_>>();
        if self.offers_empty_missions() {
            rows.push(Row::pinned(
                Choice::EmptyMission,
                crate::i18n::text("launcher.empty_mission"),
                crate::i18n::text("launcher.empty_mission_detail"),
            ));
        }
        rows
    }

    pub(super) fn typed_crew_rows(&self, query: &str, _cx: &App) -> Vec<Row> {
        let recent = self.recents.crews(&computer_key(&self.client.target));
        model::ranked_crews(&self.crews, recent, query)
            .into_iter()
            .map(|crew| crew_row(crew, Some(crate::i18n::text("launcher.crew"))))
            .collect()
    }

    pub(crate) fn open_step(&mut self, step: Step, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        self.selected = 0;
        self.error = None;
        self.slots = None;
        if !matches!(step, Step::Goal(_)) {
            let placeholder = if step == Step::Crews {
                "launcher.search_crews"
            } else {
                "launcher.search"
            };
            self.query.update(cx, |query, cx| {
                query.reset("", cx);
                query.set_placeholder(crate::i18n::message(placeholder), cx);
            });
        }
        self.step = step;
        self.settle_goal_crew(cx);
        self.focus(window, cx);
        cx.notify();
    }

    /// Shows the goal step's crew once its list is known: its slots, or the
    /// crew step when it has gone or has no roles.
    pub(super) fn settle_goal_crew(&mut self, cx: &mut Context<Self>) {
        let Step::Goal(Some(id)) = &self.step else {
            return;
        };
        if self.loading || self.slots.is_some() {
            return;
        }
        match self.crews.iter().find(|crew| &crew.id == id) {
            Some(crew) if crew.roles > 0 => match crew.slots.clone() {
                Some(slots) => self.slots = Some(slots),
                None => self.load_slots(id.clone(), cx),
            },
            _ => {
                self.step = Step::Crews;
                self.error = Some(crate::i18n::text("launcher.crew_unavailable").into());
                self.query.update(cx, |query, cx| {
                    query.set_placeholder(crate::i18n::message("launcher.search_crews"), cx)
                });
            }
        }
        cx.notify();
    }

    fn load_slots(&mut self, id: String, cx: &mut Context<Self>) {
        let core = self.store.read(cx).client.clone();
        let task = cx.background_spawn(async move {
            let slots = core.slot_list(&id);
            (id, slots)
        });
        cx.spawn(async move |weak, cx| {
            let (id, result) = task.await;
            let _ = weak.update(cx, |this, cx| {
                if this.step != Step::Goal(Some(id)) {
                    return;
                }
                match result {
                    Ok(slots) => {
                        this.slots = Some(
                            slots
                                .iter()
                                .map(|member| slot_line(member, &this.runtimes))
                                .collect(),
                        )
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn leave_crews(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_step(Step::Start, window, cx);
    }

    pub(super) fn leave_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        let origin = self.origin.take();
        let Some(root) = self.root.upgrade() else {
            return;
        };
        window.defer(cx, move |window, cx| {
            root.update(cx, |root, cx| root.leave_launcher_page(origin, window, cx));
        });
    }

    pub(super) fn pick_crew(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(crew) = self.crews.iter().find(|crew| crew.id == id) else {
            return;
        };
        if crew.roles > 0 {
            return self.open_step(Step::Goal(Some(id)), window, cx);
        }
        let Some(root) = self.root.upgrade() else {
            return;
        };
        window.defer(cx, move |window, cx| {
            root.update(cx, |root, cx| {
                if root.remote_client.is_active() {
                    root.open_crews(window, cx);
                } else {
                    root.open_crew_editor(id, window, cx);
                }
            });
        });
    }

    pub(super) fn start_mission(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Step::Goal(crew) = self.step.clone() else {
            return;
        };
        if self.pending.is_some()
            || self.loading
            || !self.client.valid()
            || !self.client.supports(&["catalog", "startMission"])
            || (crew.is_none() && !self.offers_empty_missions())
        {
            return;
        }
        let crew_name = match &crew {
            Some(id) => match self.crews.iter().find(|entry| &entry.id == id) {
                Some(entry) if entry.roles > 0 => Some(entry.name.clone()),
                _ => return,
            },
            None => None,
        };
        let Some(root) = self.root.upgrade() else {
            return;
        };
        let goal = self.goal.read(cx).text().to_owned();
        let request = MissionRequest {
            crew: crew.unwrap_or_default(),
            title: model::mission_title(
                &goal,
                crew_name.as_deref(),
                chrono::Local::now().date_naive(),
            ),
            goal: normalized_value(&goal),
            project: self.scope().project_id().map(str::to_owned),
            cwd: normalized_value(self.cwd.read(cx).text()),
        };
        self.error = None;
        self.pending = Some(Choice::Mission);
        let launcher = cx.entity();
        window.defer(cx, move |window, cx| {
            root.update(cx, |root, cx| {
                root.launch_mission_from_launcher(launcher, request, window, cx)
            });
        });
        cx.notify();
    }

    fn chip(
        &self,
        id: &'static str,
        leading: AnyElement,
        label: String,
        on_remove: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .debug_selector(move || id.to_uppercase())
            .flex_none()
            .h(rems(22. / 16.))
            .pl(rems(6. / 16.))
            .pr(rems(4. / 16.))
            .flex()
            .items_center()
            .gap(rems(6. / 16.))
            .rounded(rems(5. / 16.))
            .border_1()
            .border_color(theme::border_strong())
            .bg(theme::raised())
            .child(leading)
            .child(
                div()
                    .text_size(theme::text_ui())
                    .text_color(theme::text())
                    .child(label),
            )
            .child(
                div()
                    .id(id)
                    .cursor_pointer()
                    .child(
                        svg()
                            .path("close.svg")
                            .size(rems(11. / 16.))
                            .text_color(theme::faint()),
                    )
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .on_click(cx.listener(move |this, _, window, cx| on_remove(this, window, cx))),
            )
            .into_any_element()
    }

    pub(super) fn render_mission_chip(&self, cx: &mut Context<Self>) -> AnyElement {
        self.chip(
            "launcher-mission-chip",
            svg()
                .path("users.svg")
                .size(rems(11. / 16.))
                .text_color(theme::muted())
                .into_any_element(),
            crate::i18n::text("launcher.mission").into(),
            |this, window, cx| this.leave_crews(window, cx),
            cx,
        )
    }

    pub(super) fn render_goal_field(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let crew = match &self.step {
            Step::Goal(Some(id)) => self.crews.iter().find(|crew| &crew.id == id),
            _ => None,
        };
        let leading = match crew {
            Some(crew) => crate::surfaces::crews::crew_picture(
                &crew
                    .handles
                    .iter()
                    .cloned()
                    .map(Into::into)
                    .collect::<Vec<_>>(),
                14.,
            ),
            None => svg()
                .path("plus.svg")
                .size(rems(12. / 16.))
                .text_color(theme::muted())
                .into_any_element(),
        };
        let label = crew.map_or_else(
            || crate::i18n::text("launcher.empty_mission").to_owned(),
            |crew| crew.name.clone(),
        );
        let focused = self.goal.read(cx).focus_handle().is_focused(window);
        div()
            .debug_selector(|| "LAUNCHER_GOAL".into())
            .flex_none()
            .flex()
            .flex_col()
            .gap(rems(8. / 16.))
            .pt(rems(8. / 16.))
            .pb(rems(10. / 16.))
            .px(rems(12. / 16.))
            .rounded(rems(8. / 16.))
            .border_1()
            .border_color(if focused {
                theme::with_alpha(theme::text(), 0.2)
            } else {
                theme::border()
            })
            .bg(theme::panel())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.focus(window, cx)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(rems(8. / 16.))
                    .child(self.chip(
                        "launcher-crew-chip",
                        leading,
                        label,
                        |this, window, cx| this.open_step(Step::Crews, window, cx),
                        cx,
                    ))
                    .child(div().flex_1())
                    .child(
                        div()
                            .flex_none()
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .child(crate::i18n::text("launcher.goal_note")),
                    ),
            )
            .child(self.goal.clone())
            .into_any_element()
    }

    pub(super) fn render_goal_body(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut body = Vec::new();
        if matches!(self.step, Step::Goal(Some(_))) {
            let card = div()
                .debug_selector(|| "LAUNCHER_SLOTS".into())
                .flex()
                .flex_col()
                .p(rems(4. / 16.))
                .rounded(rems(10. / 16.))
                .border_1()
                .border_color(theme::border())
                .bg(theme::panel());
            body.push(match &self.slots {
                None => card
                    .child(div().p_3().child(crate::i18n::text("ui.loading")))
                    .into_any_element(),
                Some(slots) => card
                    .children(slots.iter().map(|slot| {
                        let handle = slot.handle.clone();
                        div()
                            .debug_selector(move || format!("LAUNCHER_SLOT {handle}"))
                            .h(rems(model::COMPACT_ROW / 16.))
                            .px(rems(10. / 16.))
                            .flex()
                            .items_center()
                            .gap(rems(8. / 16.))
                            .child(RoleAvatar::new(slot.handle.clone(), 20.))
                            .child(
                                div()
                                    .flex_none()
                                    .text_size(theme::text_body())
                                    .text_color(theme::text())
                                    .child(slot.handle.clone()),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_size(theme::text_ui())
                                    .text_color(theme::muted())
                                    .child(slot.role.clone()),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .max_w(rems(200. / 16.))
                                    .truncate()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_meta())
                                    .text_color(theme::faint())
                                    .child(slot.setup.clone()),
                            )
                    }))
                    .into_any_element(),
            });
        }
        let starting = self.pending.is_some();
        let launcher = cx.entity();
        body.push(
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(
                    div()
                        .flex_1()
                        .text_size(theme::text_meta())
                        .text_color(theme::faint())
                        .child(format!(
                            "{SHIFT_ENTER} {}",
                            crate::i18n::text("launcher.new_line")
                        )),
                )
                .child(
                    Button::new(
                        "launcher-start-mission",
                        if starting {
                            crate::i18n::text("ui.starting")
                        } else {
                            crate::i18n::text("ui.start_mission")
                        },
                    )
                    .variant(ButtonVariant::Primary)
                    .size(runner_app::ui::ButtonSize::Compact)
                    .shortcut("↵")
                    .loading(starting)
                    .disabled(starting || self.loading)
                    .on_press(move |window, cx| {
                        launcher.update(cx, |launcher, cx| launcher.start_mission(window, cx))
                    }),
                )
                .into_any_element(),
        );
        body
    }
}

impl NativeRoot {
    pub(crate) fn open_mission_launcher(
        &mut self,
        crew: Option<String>,
        scope: ProjectScope,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self
            .front_page_client(cx)
            .supports(&["catalog", "startMission"])
        {
            return;
        }
        let step = crew.map_or(Step::Crews, |id| Step::Goal(Some(id)));
        if self.launcher_page_visible() {
            if self.remote_client.is_active() {
                self.select_remote_item(RemoteItem::Launcher, cx);
            } else {
                self.set_route(AppRoute::Launcher, cx);
            }
            let launcher = self.render_launcher(None, true, window, cx);
            launcher.update(cx, |launcher, cx| {
                launcher.release_page_lock(cx);
                launcher.open_step(step, window, cx);
            });
        } else {
            self.open_launcher_page(scope, step, false, window, cx);
        }
    }

    pub(crate) fn leave_launcher_page(
        &mut self,
        origin: Option<LauncherOrigin>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let origin = origin.or_else(|| {
            (!self.remote_client.is_active() && !self.tabs.tabs().is_empty()).then_some(
                LauncherOrigin::Local {
                    route: AppRoute::Chat,
                    tab: None,
                },
            )
        });
        let Some(origin) = origin else {
            return;
        };
        match origin {
            LauncherOrigin::Local {
                route: AppRoute::Chat,
                tab,
            } => {
                if let Some(tab) = tab
                    .filter(|tab| self.tabs.tabs().iter().any(|layout| &layout.id == tab))
                    .or_else(|| self.recently_viewed_tab_id(cx))
                {
                    self.tabs.activate(&tab);
                } else {
                    self.reset_launcher_page(cx);
                }
                self.set_route(AppRoute::Chat, cx);
                self.sync_active_project_from_active_tab(cx);
                if let Err(error) = self.ensure_active_tab_attached(window, cx) {
                    self.chat_error = Some(error.to_string().into());
                }
                self.focus_active_terminal(window, cx);
            }
            LauncherOrigin::Local {
                route: AppRoute::Settings,
                ..
            } => self.enter_settings_route(None, window, cx),
            LauncherOrigin::Local { route, .. } => self.open_page_route(route, window, cx),
            LauncherOrigin::Remote(Some(item)) => self.select_remote_item(item, cx),
            LauncherOrigin::Remote(None) => {}
        }
        cx.notify();
    }

    /// The most recently viewed tab that no other window has open, else the
    /// first such tab. Showing one that another window has open would give one
    /// of the two the duplicate-chat placeholder, so that is only for when
    /// every tab is open elsewhere, and then the most recent wins.
    pub(crate) fn recently_viewed_tab_id(&self, cx: &App) -> Option<String> {
        let tabs = self.tabs.tabs();
        let viewed = self.runtime_navigation_index.map_or(0, |index| index + 1);
        let recent = self
            .runtime_navigation_history
            .iter()
            .take(viewed)
            .rev()
            .filter_map(|location| match location {
                RuntimeLocation::Chat(session_id) => {
                    tabs.iter().find(|tab| tab.contains_session(session_id))
                }
                _ => None,
            })
            .chain(tabs);
        let others = self
            .core(cx)
            .window_snapshot()
            .unwrap_or_default()
            .into_iter()
            .filter(|entry| entry.label != self.window_label)
            .collect::<Vec<_>>();
        let open_elsewhere = |tab: &PaneLayout| {
            tab.session_ids().into_iter().any(|session| {
                let subject = runner_core::protocol::window::Subject::DirectChat(session);
                others.iter().any(|entry| entry.subjects.contains(&subject))
            })
        };
        let free = recent.clone().find(|tab| !open_elsewhere(tab));
        free.or_else(|| recent.clone().next())
            .map(|tab| tab.id.clone())
    }

    fn launch_mission_from_launcher(
        &mut self,
        launcher: Entity<ChatLauncher>,
        request: MissionRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let release = |launcher: &Entity<ChatLauncher>, cx: &mut App| {
            launcher.update(cx, |launcher, cx| {
                launcher.pending = None;
                cx.notify();
            });
        };
        let client = launcher.read(cx).client.clone();
        if client.target != self.front_page_client(cx).target || !client.valid() {
            return release(&launcher, cx);
        }
        let remote_form = if client.is_remote() {
            match self.remote_client.begin_mission_form() {
                Some((form, navigation, generation)) if form.target == client.target => {
                    Some((navigation, generation))
                }
                _ => return release(&launcher, cx),
            }
        } else {
            None
        };
        let size = if let Some(workspace) = self.remote_client.mission_workspace() {
            let size = workspace
                .read(cx)
                .estimated_mission_terminal_size(window, cx);
            (size.0.max(20), size.1.max(8))
        } else {
            self.estimated_mission_terminal_size(window, cx)
        };
        let directory = self.app_store.read(cx).app_data_dir.clone();
        let scope = ProjectScope::or_root(request.project.clone());
        let target = client.target.clone();
        let recent_target = target.clone();
        let task = cx.background_spawn(async move {
            let crew = request.crew.clone();
            let result = client
                .start_mission(
                    request.crew,
                    request.title,
                    request.goal,
                    request.project,
                    request.cwd,
                    size,
                )
                .map_err(|error| error.to_string());
            if result.is_ok() && !crew.is_empty() {
                record_launcher_crew(&directory, &recent_target, crew);
            }
            result
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |root, window, cx| {
                if let Some((navigation, form)) = remote_form {
                    if !root
                        .remote_client
                        .mission_form_current(&target, navigation, form)
                    {
                        return release(&launcher, cx);
                    }
                }
                match result {
                    Ok(mission) => {
                        if root.front_page_client(cx).target == target {
                            root.reveal_started_scope(&scope, cx);
                        }
                        root.launchers.retain(|_, cached| cached != &launcher);
                        if remote_form.is_some() {
                            root.remote_client.activate_mission(mission);
                            root.sync_remote_state(cx);
                        } else {
                            root.refresh_store(StoreRefreshKind::All, cx);
                            root.open_mission(mission, window, cx);
                        }
                    }
                    Err(error) => {
                        let folder_error = remote_form
                            .and_then(|_| root.remote_client.computer())
                            .map(|computer| computer.name.clone())
                            .filter(|_| error == "Choose an accessible folder on the host.");
                        launcher.update(cx, |launcher, cx| {
                            launcher.pending = None;
                            if let (Some(computer), Some(picker)) =
                                (folder_error, launcher.folder_picker.clone())
                            {
                                picker.update(cx, |picker, cx| {
                                    picker.path_error =
                                        Some(crate::surfaces::remote_client::label(
                                            "client.folder_invalid",
                                            &computer,
                                        ));
                                    cx.notify();
                                });
                            }
                            launcher.error = Some(error);
                            cx.notify();
                        });
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surfaces::start_chat::tests::{modal_harness, project_fixture, ModalHarness};

    fn harness() -> ModalHarness {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        harness.act(|root, _, cx| {
            root.start_chat_modal = None;
            root.set_route(AppRoute::Chat, cx);
        });
        harness
    }

    /// A crew with one codex role per handle; none leaves it without roles.
    fn crew(root: &NativeRoot, name: &str, handles: &[&str], cx: &App) -> String {
        let core = root.core(cx);
        let crew = core
            .crew_create(runner_core::protocol::crew::CreateCrewInput {
                name: name.into(),
                ..Default::default()
            })
            .unwrap();
        for handle in handles {
            let role = core
                .role_create(
                    serde_json::from_value(serde_json::json!({
                        "handle": format!("{}-{handle}", name.split(' ').next().unwrap().to_lowercase()),
                        "display_name": format!("Role {handle}"),
                        "runtime": "codex",
                        "command": "codex",
                    }))
                    .unwrap(),
                )
                .unwrap();
            core.slot_create(
                serde_json::from_value(serde_json::json!({
                    "crew_id": crew.id,
                    "role_id": role.id,
                    "slot_handle": handle,
                }))
                .unwrap(),
            )
            .unwrap();
        }
        crew.id
    }

    fn active_launcher(root: &NativeRoot, cx: &App) -> Entity<ChatLauncher> {
        root.launchers
            .values()
            .find(|launcher| {
                let launcher = launcher.read(cx);
                launcher.pane.is_none()
                    && launcher.client.target == root.front_page_client(cx).target
            })
            .cloned()
            .expect("the launcher page")
    }

    fn step(root: &NativeRoot, cx: &App) -> Step {
        active_launcher(root, cx).read(cx).step.clone()
    }

    fn choices(root: &NativeRoot, cx: &App) -> Vec<Choice> {
        let launcher = active_launcher(root, cx);
        let launcher = launcher.read(cx);
        launcher
            .rows(cx)
            .0
            .into_iter()
            .map(|row| row.choice)
            .collect()
    }

    fn shown(harness: &mut ModalHarness, selector: &'static str) -> bool {
        harness.visual.debug_bounds(selector).is_some()
    }

    #[test]
    fn mission_entry_points_show_a_page_and_dismiss_back_through_steps() {
        let mut harness = harness();
        harness.act(|root, window, cx| root.open_crews(window, cx));
        harness.act(|root, window, cx| root.new_mission_action(&NewMission, window, cx));
        harness.act(|root, _, cx| {
            assert_eq!(root.route, AppRoute::Launcher);
            assert!(root.tabs.tabs().is_empty());
            assert_eq!(step(root, cx), Step::Crews);
            assert_eq!(choices(root, cx).last(), Some(&Choice::EmptyMission));
        });
        assert!(shown(&mut harness, "LAUNCHER_BRAND"));
        assert!(!shown(&mut harness, "SIDEBAR_ROW sidebar-launcher-tab"));
        harness.visual.simulate_input("find a crew");
        harness.visual.simulate_keystrokes("escape");
        harness.visual.run_until_parked();
        harness.act(|root, _, cx| assert_eq!(step(root, cx), Step::Start));
        harness.visual.simulate_keystrokes("escape");
        harness.visual.run_until_parked();
        harness.act(|root, _, _| assert_eq!(root.route, AppRoute::Crews));
    }

    #[test]
    fn new_chat_and_mission_reuse_the_page_and_escape_restores_the_previous_tab() {
        let mut harness = harness();
        let mut previous = String::new();
        harness.act(|root, window, cx| {
            root.test_open_empty_tab(window, cx);
            previous = root.tabs.active_tab_id().unwrap().to_owned();
            root.open_new_tab_modal(&NewTab, window, cx);
        });
        harness.act(|root, window, cx| {
            assert_eq!(root.route, AppRoute::Launcher);
            assert!(root.front_page_layout().is_none());
            assert_eq!(root.tabs.tabs().len(), 1);
            active_launcher(root, cx).update(cx, |launcher, cx| {
                launcher.start(Choice::Mission, false, window, cx)
            });
        });
        harness.visual.simulate_keystrokes("backspace");
        harness.visual.run_until_parked();
        harness.act(|root, _, cx| assert_eq!(step(root, cx), Step::Start));
        harness.visual.simulate_input("a search draft");
        harness.visual.simulate_keystrokes("escape");
        harness.visual.run_until_parked();
        harness.act(|root, _, _| {
            assert_eq!(root.route, AppRoute::Chat);
            assert_eq!(root.tabs.active_tab_id(), Some(previous.as_str()));
            assert_eq!(root.tabs.tabs().len(), 1);
        });
    }

    #[test]
    fn a_split_keeps_its_panes_and_mission_and_new_chat_show_the_launcher_page() {
        let mut harness = harness();
        let mut previous = String::new();
        harness.act(|root, window, cx| {
            root.test_open_empty_tab(window, cx);
            let pane = root.tabs.active().unwrap().focused_pane_id.clone();
            root.split_pane(&pane, SplitOrientation::Row, window, cx);
            let layout = root.tabs.active().unwrap().clone();
            previous = layout.id.clone();
            let view = root.render_launcher(
                Some(PaneKey::new(&layout.id, &layout.focused_pane_id)),
                false,
                window,
                cx,
            );
            assert!(view
                .read(cx)
                .rows(cx)
                .0
                .iter()
                .all(|row| row.choice != Choice::Mission));
        });
        assert!(!shown(&mut harness, "LAUNCHER_BRAND"));
        harness.act(|root, window, cx| root.new_mission_action(&NewMission, window, cx));
        harness.act(|root, _, cx| {
            assert_eq!(step(root, cx), Step::Crews);
            assert!(root.front_page_layout().is_none());
            assert_eq!(root.tabs.tabs().len(), 1);
            assert_eq!(root.tabs.active().unwrap().root.leaves().len(), 2);
        });
        assert!(shown(&mut harness, "LAUNCHER_BRAND"));
        harness.visual.simulate_keystrokes("escape escape");
        harness.visual.run_until_parked();
        harness.act(|root, _, _| {
            assert_eq!(root.tabs.active_tab_id(), Some(previous.as_str()));
            assert_eq!(root.route, AppRoute::Chat);
        });
        harness
            .visual
            .simulate_keystrokes(&keymap::platform_default("cmd-n"));
        harness.visual.run_until_parked();
        harness.act(|root, _, _| {
            assert_eq!(root.route, AppRoute::Launcher);
            assert_eq!(root.tabs.tabs().len(), 1);
        });
    }

    #[test]
    fn project_choices_and_folder_changes_save_no_launcher_tab_on_any_step() {
        let mut harness = harness();
        let projects = project_fixture(&mut harness);
        harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
        for (next, project) in [
            (Step::Start, projects[0].id.as_str()),
            (Step::Crews, projects[1].id.as_str()),
            (Step::Goal(None), ""),
        ] {
            harness.act(|root, window, cx| {
                active_launcher(root, cx).update(cx, |launcher, cx| {
                    launcher.open_step(next, window, cx);
                    launcher.choose_project(project, cx);
                });
                assert!(root.tabs.tabs().is_empty());
                assert!(root
                    .core(cx)
                    .node_list()
                    .unwrap()
                    .iter()
                    .all(|node| node.node_type != runner_core::protocol::node::NodeType::Tab));
            });
            assert!(shown(&mut harness, "LAUNCHER_BRAND"));
            assert!(!shown(&mut harness, "SIDEBAR_ROW sidebar-launcher-tab"));
        }
        harness.act(|root, window, cx| {
            active_launcher(root, cx)
                .update(cx, |launcher, cx| launcher.change_directory(window, cx))
        });
        let folder = PathBuf::from(&projects[0].cwd).join(".worktrees/feature");
        harness
            .visual
            .simulate_path_prompt_response(|_| Some(vec![folder]));
        harness.visual.run_until_parked();
        harness.act(|root, _, cx| {
            assert_eq!(
                active_launcher(root, cx).read(cx).scope(),
                ProjectScope::Project(projects[0].id.clone())
            );
            assert!(root.tabs.tabs().is_empty());
        });
    }

    #[test]
    fn sidebar_selection_leaves_the_page_without_saving_an_empty_tab() {
        let mut harness = harness();
        let mut tab = String::new();
        let mut sidebar = None;
        harness.act(|root, window, cx| {
            let core = root.app_store.read(cx).test_core.clone();
            let conn = core.db.get().unwrap();
            conn.execute(
                "INSERT INTO sessions(id, status, agent_runtime, agent_command) VALUES ('previous-chat', 'stopped', 'codex', 'codex')",
                [],
            )
            .unwrap();
            let layout = PaneLayout::single(Some("previous-chat"), &["previous-chat".into()]);
            tab = runner_daemon::repo::node::create_tab(
                &conn, None, "Previous chat", 0, &layout.serialize().unwrap(),
            )
            .unwrap()
            .id;
            drop(conn);
            root.app_store.update(cx, |store, cx| {
                store.refresh_sessions(cx);
                store.refresh_nodes(cx).unwrap();
            });
            root.reload_tabs(cx).unwrap();
            root.open_new_tab_modal(&NewTab, window, cx);
            sidebar = Some(root.front_page_sidebar());
        });
        harness.visual.update(|window, cx| {
            sidebar
                .unwrap()
                .update(cx, |sidebar, cx| sidebar.select_shortcut_row(1, window, cx));
        });
        harness.visual.run_until_parked();
        harness.act(|root, _, cx| {
            assert_eq!(root.route, AppRoute::Chat);
            assert_eq!(root.tabs.active_tab_id(), Some(tab.as_str()));
            assert_eq!(root.tabs.tabs().len(), 1);
            assert!(!root.launcher_page_visible());
            assert_eq!(
                root.core(cx)
                    .node_list()
                    .unwrap()
                    .iter()
                    .filter(|node| node.node_type == runner_core::protocol::node::NodeType::Tab)
                    .count(),
                1
            );
        });
    }

    #[test]
    fn successful_chat_terminal_and_mission_starts_reveal_their_collapsed_destination() {
        for kind in ["chat", "terminal", "mission"] {
            for in_project in [false, true] {
                let mut harness = harness();
                let projects = project_fixture(&mut harness);
                let scope = ProjectScope::or_root(in_project.then(|| projects[0].id.clone()));
                harness.act(|root, window, cx| {
                    root.app_store.update(cx, |store, _| {
                        store.settings.sidebar_projects_open = false;
                        store.settings.sidebar_chats_open = false;
                        store
                            .settings
                            .sidebar_collapsed_projects
                            .insert(projects[0].id.clone());
                    });
                    root.open_launcher_page(scope.clone(), Step::Start, false, window, cx);
                });
                harness.act(|root, _, cx| {
                    assert!(!root.settings(cx).sidebar_projects_open);
                    assert!(!root.settings(cx).sidebar_chats_open);
                    assert!(root.tabs.tabs().is_empty());
                });
                harness.act(|root, window, cx| match kind {
                    "chat" => {
                        let parent = scope.project_id().map(|id| {
                            root.core(cx).node_list().unwrap().into_iter()
                                .find(|node| node.ref_id.as_deref() == Some(id))
                                .unwrap().id
                        });
                        let core = root.app_store.read(cx).test_core.clone();
                        let conn = core.db.get().unwrap();
                        conn.execute(
                            "INSERT INTO sessions(id, status, agent_runtime, agent_command) VALUES ('started-chat', 'stopped', 'codex', 'codex')",
                            [],
                        ).unwrap();
                        let layout = PaneLayout::single(Some("started-chat"), &["started-chat".into()]);
                        runner_daemon::repo::node::create_tab(
                            &conn, parent.as_deref(), "Started chat", 0, &layout.serialize().unwrap(),
                        ).unwrap();
                        drop(conn);
                        root.finish_local_chat_launch(
                            Some("started-chat".into()),
                            Ok(crate::surfaces::front_page::client::ChatLaunched {
                                session: "started-chat".into(), layout: None,
                            }),
                            window, cx,
                        );
                    }
                    "terminal" => active_launcher(root, cx).update(cx, |launcher, cx| {
                        launcher.start(Choice::Terminal, false, window, cx)
                    }),
                    "mission" => active_launcher(root, cx).update(cx, |launcher, cx| {
                        launcher.open_step(Step::Goal(None), window, cx);
                        launcher.start_mission(window, cx);
                    }),
                    _ => unreachable!(),
                });
                let mut selector = String::new();
                harness.act(|root, _, cx| {
                    let settings = root.settings(cx);
                    if in_project {
                        assert!(settings.sidebar_projects_open, "{kind}");
                        assert!(
                            !settings
                                .sidebar_collapsed_projects
                                .contains(&projects[0].id),
                            "{kind}"
                        );
                        assert!(!settings.sidebar_chats_open);
                    } else {
                        assert!(settings.sidebar_chats_open, "{kind}");
                        assert!(!settings.sidebar_projects_open);
                    }
                    let node = root
                        .core(cx)
                        .node_list()
                        .unwrap()
                        .into_iter()
                        .find(|node| {
                            node.node_type != runner_core::protocol::node::NodeType::Project
                        })
                        .unwrap();
                    selector = if kind == "mission" {
                        format!("SIDEBAR_ROW sidebar-mission-{}", node.ref_id.unwrap())
                    } else {
                        format!("SIDEBAR_ROW sidebar-tab-{}", node.id)
                    };
                });
                assert!(
                    shown(&mut harness, Box::leak(selector.into_boxed_str())),
                    "{kind}, project={in_project}"
                );
                if kind == "terminal" {
                    harness.act(|root, _, cx| {
                        let session = root
                            .tabs
                            .active()
                            .unwrap()
                            .focused_session_id()
                            .unwrap()
                            .to_owned();
                        root.core(cx).session_close(&session).unwrap();
                    });
                }
            }
        }
    }

    struct RefusedMissionStart(runner_daemon::daemon::InProcessTransport);

    impl runner_core::protocol::Transport for RefusedMissionStart {
        fn call(
            &self,
            request: runner_core::protocol::Request,
        ) -> runner_core::protocol::ClientResult<runner_core::protocol::Response> {
            if matches!(
                request,
                runner_core::protocol::Request::mission_start_impl_with_size { .. }
            ) {
                return Ok(
                    runner_core::protocol::Response::mission_start_impl_with_size(Err(
                        runner_core::protocol::ClientError::msg("bounded start failure"),
                    )),
                );
            }
            self.0.call(request)
        }

        fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
            self.0.subscribe()
        }
    }

    #[test]
    fn failed_and_canceled_starts_keep_the_destination_collapsed() {
        for kind in ["chat", "terminal", "mission", "cancel"] {
            for in_project in [false, true] {
                let mut harness = harness();
                let projects = project_fixture(&mut harness);
                let scope = ProjectScope::or_root(in_project.then(|| projects[0].id.clone()));
                let invalid = PathBuf::from(&projects[0].cwd).join("not-a-directory");
                std::fs::write(&invalid, "fixture").unwrap();
                harness.act(|root, window, cx| {
                    root.app_store.update(cx, |store, _| {
                        store.settings.sidebar_projects_open = false;
                        store.settings.sidebar_chats_open = false;
                        store
                            .settings
                            .sidebar_collapsed_projects
                            .insert(projects[0].id.clone());
                    });
                    if kind == "mission" {
                        root.app_store.update(cx, |store, _| {
                            store.client =
                                runner_core::protocol::DaemonClient::new(std::sync::Arc::new(
                                    RefusedMissionStart(runner_daemon::daemon::InProcessTransport(
                                        store.test_core.clone(),
                                    )),
                                ));
                        });
                    }
                    root.open_crews(window, cx);
                    root.open_launcher_page(scope.clone(), Step::Start, false, window, cx);
                    if kind == "chat" {
                        root.finish_local_chat_launch(
                            None,
                            Err(runner_core::protocol::ClientError::msg(
                                "bounded start failure",
                            )),
                            window,
                            cx,
                        );
                    }
                    active_launcher(root, cx).update(cx, |launcher, cx| {
                        launcher.cwd.update(cx, |cwd, cx| {
                            cwd.set_text(invalid.to_string_lossy().into_owned(), cx)
                        });
                        if kind == "terminal" {
                            launcher.start(Choice::Terminal, false, window, cx);
                        } else if kind == "mission" {
                            launcher.open_step(Step::Goal(None), window, cx);
                            launcher.start_mission(window, cx);
                        }
                    });
                });
                if kind == "cancel" {
                    harness.visual.simulate_keystrokes("escape");
                    harness.visual.run_until_parked();
                }
                harness.act(|root, _, cx| {
                    assert!(!root.settings(cx).sidebar_projects_open, "{kind}");
                    assert!(!root.settings(cx).sidebar_chats_open, "{kind}");
                    assert!(root
                        .settings(cx)
                        .sidebar_collapsed_projects
                        .contains(&projects[0].id));
                    assert!(root.tabs.tabs().is_empty());
                    assert!(root.core(cx).node_list().unwrap().iter().all(
                        |node| node.node_type == runner_core::protocol::node::NodeType::Project
                    ));
                    if kind == "cancel" {
                        assert_eq!(root.route, AppRoute::Crews);
                    } else if kind == "chat" {
                        assert!(root.chat_error.is_some());
                    } else {
                        assert!(active_launcher(root, cx).read(cx).error.is_some());
                    }
                });
            }
        }
    }

    #[test]
    fn crew_step_lists_crews_with_roles_and_a_roleless_crew_dimmed_that_opens_its_page() {
        let mut harness = harness();
        let mut ids = (String::new(), String::new());
        harness.act(|root, _, cx| {
            ids = (
                crew(root, "Alpha crew", &["impl", "review"], cx),
                crew(root, "Empty crew", &[], cx),
            )
        });
        let (alpha, empty) = ids;
        harness.act(|root, window, cx| root.open_crews(window, cx));
        harness.act(|root, window, cx| root.new_mission_action(&NewMission, window, cx));
        harness.act(|root, window, cx| {
            let launcher = active_launcher(root, cx);
            let rows = launcher.read(cx).rows(cx).0;
            let row = |id: &str| {
                rows.iter()
                    .find(|row| row.choice == Choice::Crew(id.into()))
                    .unwrap()
            };
            assert!(!row(&alpha).dimmed);
            assert_eq!(row(&alpha).detail, "impl, review");
            assert!(row(&empty).dimmed);
            assert_eq!(row(&empty).detail, "No roles yet");
            assert_eq!(rows.last().unwrap().choice, Choice::EmptyMission);
            launcher.update(cx, |launcher, cx| {
                launcher.start(Choice::Crew(empty.clone()), false, window, cx)
            });
        });
        harness.act(|root, _, _| {
            assert_eq!(root.route, AppRoute::CrewEditor(empty.clone()));
            assert!(
                root.tabs.tabs().is_empty(),
                "the launcher page never saved a tab"
            );
        });
    }

    #[test]
    fn an_empty_mission_starts_from_the_launcher_page_in_the_chosen_project() {
        let mut harness = harness();
        let projects = project_fixture(&mut harness);
        let project = projects[0].clone();
        let mut sidebar = None;
        harness.act(|root, _, _| sidebar = Some(root.front_page_sidebar()));
        let scope = ProjectScope::Project(project.id.clone());
        harness.visual.update(|window, cx| {
            sidebar.clone().unwrap().update(cx, |sidebar, cx| {
                sidebar.open_launcher_in(scope, window, cx)
            })
        });
        harness.visual.run_until_parked();
        harness.act(|root, _, cx| {
            assert!(root.tabs.tabs().is_empty());
            assert_eq!(root.active_project_id(cx), Some(project.id.clone()));
            assert_eq!(step(root, cx), Step::Start);
        });
        harness.act(|root, window, cx| root.new_mission_action(&NewMission, window, cx));
        harness.visual.simulate_keystrokes("shift-enter");
        harness.visual.run_until_parked();
        harness.act(|root, _, cx| assert_eq!(step(root, cx), Step::Goal(None)));
        assert!(shown(&mut harness, "LAUNCHER-CREW-CHIP"));
        assert!(shown(&mut harness, "LAUNCHER_BRAND"));
        assert!(!shown(&mut harness, "LAUNCHER_SLOTS"));
        harness.visual.simulate_keystrokes("enter");
        harness.visual.run_until_parked();
        harness.act(|root, _, cx| {
            let AppRoute::Mission(id) = &root.route else {
                panic!("the mission opened");
            };
            let mission = root.core(cx).mission_get(id).unwrap();
            assert!(mission.crew_id.is_none());
            assert_eq!(
                mission.title,
                format!("Mission {}", chrono::Local::now().format("%Y-%m-%d"))
            );
            assert_eq!(mission.project_id.as_deref(), Some(project.id.as_str()));
            assert_eq!(mission.cwd.as_deref(), Some(project.cwd.as_str()));
            assert!(root.core(cx).session_list(id).unwrap().is_empty());
            assert!(
                root.tabs.tabs().is_empty(),
                "no empty launcher tab was saved"
            );
            assert!(
                root.launchers
                    .values()
                    .all(|launcher| launcher.read(cx).pane.is_none()),
                "no launcher is left on the tab"
            );
        });
        assert!(!shown(&mut harness, "SIDEBAR_ROW sidebar-launcher-tab"));
    }

    #[test]
    fn typing_finds_crews_after_agents_and_the_goal_step_shows_slots_and_steps_back() {
        let mut harness = harness();
        let mut alpha = String::new();
        harness.act(|root, _, cx| alpha = crew(root, "Alpha crew", &["impl", "review"], cx));
        harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
        harness.visual.simulate_input("alpha");
        harness.visual.run_until_parked();
        harness.act(|root, window, cx| {
            let launcher = active_launcher(root, cx);
            let rows = launcher.read(cx).rows(cx).0;
            let crew = rows
                .iter()
                .position(|row| row.choice == Choice::Crew(alpha.clone()))
                .expect("a matching crew");
            assert_eq!(rows[crew].detail, "Crew · impl, review");
            assert!(rows[crew + 1..].iter().all(|row| row.choice.pinned()));
            launcher.update(cx, |launcher, cx| {
                launcher.start(Choice::Crew(alpha.clone()), false, window, cx)
            });
        });
        harness.act(|root, _, cx| assert_eq!(step(root, cx), Step::Goal(Some(alpha.clone()))));
        for selector in [
            "LAUNCHER_GOAL",
            "LAUNCHER_SLOT impl",
            "LAUNCHER_SLOT review",
            "LAUNCHER_BRAND",
        ] {
            assert!(shown(&mut harness, selector), "{selector}");
        }
        harness.visual.simulate_input("First");
        harness.visual.simulate_keystrokes("shift-enter");
        harness.visual.simulate_input("Second");
        harness.visual.run_until_parked();
        harness.act(|root, _, cx| {
            let launcher = active_launcher(root, cx);
            assert_eq!(launcher.read(cx).goal.read(cx).text(), "First\nSecond");
            assert!(launcher.read(cx).pending.is_none());
        });
        harness.visual.simulate_keystrokes("escape");
        harness.visual.run_until_parked();
        harness.act(|root, _, cx| {
            assert_eq!(step(root, cx), Step::Crews, "Esc goes back to the crews");
            assert_eq!(
                active_launcher(root, cx).read(cx).goal.read(cx).text(),
                "First\nSecond",
                "the goal waits for the next crew"
            );
        });
        let chip = harness
            .visual
            .debug_bounds("LAUNCHER-MISSION-CHIP")
            .unwrap();
        harness.visual.simulate_click(
            chip.top_right() + gpui::point(px(-6.), px(10.)),
            Default::default(),
        );
        harness.visual.run_until_parked();
        harness.act(|root, _, cx| {
            assert_eq!(step(root, cx), Step::Start, "the chip's × goes home");
            assert!(root.tabs.tabs().is_empty());
        });
    }
}

#[cfg(test)]
mod remote_tests {
    use super::*;
    use crate::surfaces::remote_client::tests::{
        self as remote_test, action_trace, add_window, choose_host, Fake,
    };
    use gpui::{TestAppContext, VisualTestContext};
    use std::sync::Arc;

    fn catalog() -> serde_json::Value {
        serde_json::json!({"roles":[{"id":"host-role","name":"Host coder","handle":"host-coder","runtime":"codex","model":"host-model","effort":"high","speed":null}],"runtimes":[{"name":"codex","label":"Codex","available":true,"nativeFork":true,"defaultModel":null,"defaultEffort":null,"models":[],"efforts":[],"speeds":[]}],"accounts":[],"activeAccount":"","crews":[{"id":"empty-host-crew","name":"Empty host crew","slots":[]},{"id":"host-crew","name":"Host duo","slots":[{"handle":"lead","role":"host-role","lead":true},{"handle":"reviewer","role":"host-role","lead":false}]}],"projects":[{"id":"host-project","name":"Host project","path":"\\\\nas\\share"}],"recentFolders":[]})
    }

    fn overview() -> runner_core::companion::Overview {
        serde_json::from_value(serde_json::json!({"projects":[{"id":"host-project","name":"Host project"}],"tabs":[{"id":"first","project":null,"name":"First","pinned":false,"panes":{"session":null},"shells":[]}],"missions":[],"sessions":{}})).unwrap()
    }

    fn setup(
        features: Option<&[&str]>,
    ) -> (
        tempfile::TempDir,
        TestAppContext,
        gpui::WindowHandle<NativeRoot>,
        Arc<Fake>,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
        remote_test::enable_missions(&fake);
        if let Some(features) = features {
            fake.set_features(features);
        }
        fake.set_overview(overview());
        let window = add_window(&mut cx, &store, temp.path(), "host-mission-launcher");
        window.update(&mut cx, choose_host).unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |root, _, cx| {
                root.select_remote_item(RemoteItem::Tab("first".into()), cx)
            })
            .unwrap();
        cx.run_until_parked();
        (temp, cx, window, fake)
    }

    fn launcher(
        root: &mut NativeRoot,
        window: &mut Window,
        cx: &mut Context<NativeRoot>,
    ) -> Entity<ChatLauncher> {
        root.render_launcher(None, true, window, cx)
    }

    #[test]
    fn the_crew_step_lists_the_hosts_crews_and_starts_the_mission_there() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let (_temp, mut cx, window, fake) = setup(None);
        fake.queue_result("catalog", Ok(catalog()));
        window
            .update(&mut cx, |root, window, cx| {
                root.new_mission_action(&NewMission, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        let view = window
            .update(&mut cx, |root, window, cx| {
                root.new_mission_action(&NewMission, window, cx);
                let view = launcher(root, window, cx);
                assert!(view
                    .read(cx)
                    .crews
                    .iter()
                    .flat_map(|crew| crew.slots.iter().flatten())
                    .all(|slot| slot.setup.is_empty()));
                let rows = view.read(cx).rows(cx).0;
                let choices = rows
                    .iter()
                    .map(|row| row.choice.clone())
                    .collect::<Vec<_>>();
                assert_eq!(
                    choices,
                    [
                        Choice::Crew("empty-host-crew".into()),
                        Choice::Crew("host-crew".into()),
                    ]
                );
                assert!(rows[0].dimmed);
                assert!(!rows[1].dimmed);
                view.update(cx, |launcher, cx| {
                    launcher.choose_project("host-project", cx);
                    launcher.start(Choice::Crew("host-crew".into()), false, window, cx);
                    launcher.goal.update(cx, |goal, cx| {
                        goal.set_text("\nShip the host change\nthen review", cx)
                    });
                });
                view
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        assert!(visual.debug_bounds("LAUNCHER_SLOT lead").is_some());
        assert!(visual.debug_bounds("LAUNCHER_SLOT reviewer").is_some());
        let mut size = (0, 0);
        window
            .update(&mut visual, |root, window, cx| {
                size = root
                    .remote_client
                    .workspace()
                    .unwrap()
                    .mission_workspace
                    .read(cx)
                    .estimated_mission_terminal_size(window, cx);
                view.update(cx, |launcher, cx| launcher.start_mission(window, cx));
            })
            .unwrap();
        visual.run_until_parked();
        let starts = action_trace(&fake, "startMission");
        assert_eq!(starts.len(), 1);
        let request = &starts[0];
        assert_eq!(request["crew"], "host-crew");
        assert_eq!(request["title"], "Ship the host change");
        assert_eq!(request["goal"], "Ship the host change\nthen review");
        assert_eq!(request["project"], "host-project");
        assert_eq!(request["cwd"], r"\\nas\share");
        assert_eq!(request["cols"], size.0.max(20));
        assert_eq!(request["rows"], size.1.max(8));
        assert!(action_trace(&fake, "archive").is_empty());
        assert!(action_trace(&fake, "layout").is_empty());
        window
            .read_with(&visual, |root, _| {
                let remote = root.remote_client.workspace().unwrap();
                assert_eq!(remote.activate_mission.as_deref(), Some("created-mission"));
            })
            .unwrap();
    }

    #[test]
    fn without_catalog_and_start_mission_no_entry_point_opens_a_launcher() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for features in [vec!["catalog", "layout"], vec!["startMission", "layout"]] {
            let (_temp, mut cx, window, fake) = setup(Some(&features));
            let layouts = fake.request_count("layout");
            window
                .update(&mut cx, |root, window, cx| {
                    let view = launcher(root, window, cx);
                    assert!(view
                        .read(cx)
                        .rows(cx)
                        .0
                        .iter()
                        .all(|row| row.choice != Choice::Mission));
                    root.new_mission_action(&NewMission, window, cx);
                    root.open_mission_launcher(
                        Some("host-crew".into()),
                        ProjectScope::Root,
                        window,
                        cx,
                    );
                    assert!(!root.launcher_page_visible());
                    assert_eq!(view.read(cx).step, Step::Start);
                })
                .unwrap();
            cx.run_until_parked();
            assert_eq!(fake.request_count("layout"), layouts);
            assert_eq!(fake.request_count("startMission"), 0);
        }
    }

    #[test]
    fn remote_empty_missions_are_unavailable_in_rows_keys_and_start_requests() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let (_temp, mut cx, window, fake) = setup(None);
        let view = window
            .update(&mut cx, |root, window, cx| {
                root.new_mission_action(&NewMission, window, cx);
                launcher(root, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.simulate_keystrokes("shift-enter");
        visual.run_until_parked();
        window
            .update(&mut visual, |_, window, cx| {
                view.update(cx, |launcher, cx| {
                    assert_eq!(launcher.step, Step::Crews);
                    assert!(launcher
                        .rows(cx)
                        .0
                        .iter()
                        .all(|row| row.choice != Choice::EmptyMission));
                    launcher.start(Choice::EmptyMission, false, window, cx);
                    assert_eq!(launcher.step, Step::Crews);
                    launcher.open_step(Step::Goal(None), window, cx);
                    launcher.start_mission(window, cx);
                    assert!(launcher.pending.is_none());
                });
            })
            .unwrap();
        visual.run_until_parked();
        assert_eq!(fake.request_count("startMission"), 0);
    }

    #[test]
    fn new_chat_page_sends_only_the_start_request_when_a_chat_is_chosen() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let (_temp, mut cx, window, fake) = setup(None);
        window
            .update(&mut cx, |root, window, cx| {
                root.open_new_tab_modal(&NewTab, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        assert!(action_trace(&fake, "layout").is_empty());
        window
            .update(&mut cx, |root, window, cx| {
                assert!(root.launcher_page_visible());
                assert!(root.front_page_layout().is_none());
                root.front_page_sidebar().update(cx, |sidebar, _| {
                    let settings = sidebar.remote_settings.as_mut().unwrap();
                    settings.sidebar_chats_open = false;
                    settings.sidebar_projects_open = false;
                });
                launcher(root, window, cx).update(cx, |launcher, cx| {
                    launcher.start(
                        Choice::Chat(EntryKey::Agent("codex".into())),
                        false,
                        window,
                        cx,
                    )
                });
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(fake.request_count("startChat"), 1);
        assert!(action_trace(&fake, "layout").is_empty());
        assert!(action_trace(&fake, "archive").is_empty());
        window
            .read_with(&cx, |root, cx| {
                assert_eq!(
                    root.remote_client
                        .workspace()
                        .unwrap()
                        .activate_session
                        .as_deref(),
                    Some("started-session")
                );
                let sidebar = root.front_page_sidebar();
                let settings = sidebar.read(cx).remote_settings.as_ref().unwrap();
                assert!(settings.sidebar_chats_open);
                assert!(!settings.sidebar_projects_open);
            })
            .unwrap();
    }

    /// The row, read with the root free, since the sidebar reads it back.
    fn host_row(
        window: &gpui::WindowHandle<NativeRoot>,
        cx: &TestAppContext,
    ) -> Option<ProjectScope> {
        window
            .read_with(cx, |root, cx| {
                root.front_page_sidebar().read(cx).launcher_row_scope(cx)
            })
            .unwrap()
    }

    fn host_page(
        window: &gpui::WindowHandle<NativeRoot>,
        cx: &mut TestAppContext,
    ) -> (ProjectScope, bool, String, bool) {
        window
            .update(cx, |root, window, cx| {
                let page = launcher(root, window, cx);
                let page = page.read(cx);
                let select = page.project.select();
                (
                    page.scope(),
                    select.read(cx).is_disabled(),
                    page.cwd.read(cx).text().to_owned(),
                    page.loading,
                )
            })
            .unwrap()
    }

    #[test]
    fn new_tab_uses_the_hosts_opening_tab_project_and_picker_close_restores_navigation() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let (_temp, mut cx, window, fake) = setup(None);
        let mut data = overview();
        data.tabs[0].project = Some("host-project".into());
        let mut recent = data.tabs[0].clone();
        recent.id = "recent".into();
        data.tabs.push(recent.clone());
        fake.set_overview(data);
        cx.run_until_parked();
        fake.queue_result("catalog", Ok(catalog()));
        window
            .update(&mut cx, |root, window, cx| {
                root.select_remote_item(RemoteItem::Tab("recent".into()), cx);
                root.select_remote_item(RemoteItem::Tab("first".into()), cx);
                root.open_new_tab_modal(&NewTab, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            host_page(&window, &mut cx),
            (
                ProjectScope::Project("host-project".into()),
                false,
                r"\\nas\share".into(),
                false
            )
        );
        assert_eq!(host_row(&window, &cx), Some(ProjectScope::Root));
        window
            .read_with(&cx, |root, cx| assert_eq!(root.active_project_id(cx), None))
            .unwrap();

        for item in [RemoteItem::Roles, RemoteItem::Crews] {
            fake.queue_result("catalog", Ok(catalog()));
            window
                .update(&mut cx, |root, window, cx| {
                    root.select_remote_item(item, cx);
                    root.open_new_tab_modal(&NewTab, window, cx);
                })
                .unwrap();
            cx.run_until_parked();
            let first = host_page(&window, &mut cx);
            assert_eq!(
                first,
                (
                    ProjectScope::Project("host-project".into()),
                    false,
                    r"\\nas\share".into(),
                    false
                )
            );
            window
                .update(&mut cx, |root, window, cx| {
                    root.open_new_tab_modal(&NewTab, window, cx);
                })
                .unwrap();
            cx.run_until_parked();
            assert_eq!(host_page(&window, &mut cx), first);
        }

        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        for close in ["mouse", "keyboard", "escape"] {
            let picker = visual
                .debug_bounds("STYLED_SELECT_launcher-project")
                .unwrap();
            visual.simulate_click(picker.center(), Default::default());
            visual.run_until_parked();
            match close {
                "mouse" => {
                    let option = visual.debug_bounds("STYLED_SELECT_OPTION_0").unwrap();
                    visual.simulate_click(option.center(), Default::default());
                }
                "keyboard" => visual.simulate_keystrokes("end enter"),
                _ => visual.simulate_keystrokes("escape"),
            }
            visual.run_until_parked();
            let selected = window
                .update(&mut visual, |root, window, cx| {
                    let page = launcher(root, window, cx);
                    assert!(
                        page.read(cx)
                            .query
                            .read(cx)
                            .focus_handle()
                            .is_focused(window),
                        "{close}"
                    );
                    page.read(cx).selected
                })
                .unwrap();
            visual.simulate_keystrokes("down");
            visual.run_until_parked();
            window
                .update(&mut visual, |root, window, cx| {
                    assert_ne!(
                        launcher(root, window, cx).read(cx).selected,
                        selected,
                        "{close}"
                    );
                })
                .unwrap();
        }
        window
            .update(&mut visual, |root, window, cx| {
                launcher(root, window, cx).update(cx, |page, cx| page.choose_project("", cx));
                root.open_new_tab_modal(&NewTab, window, cx);
            })
            .unwrap();
        visual.run_until_parked();
        window
            .update(&mut visual, |root, window, cx| {
                assert_eq!(
                    launcher(root, window, cx).read(cx).scope(),
                    ProjectScope::Project("host-project".into())
                );
                root.select_remote_item(RemoteItem::Tab("first".into()), cx);
            })
            .unwrap();
        let mut no_project = overview();
        no_project.tabs.push(recent.clone());
        fake.set_overview(no_project);
        visual.run_until_parked();
        window
            .update(&mut visual, |root, window, cx| {
                root.open_new_tab_modal(&NewTab, window, cx);
                assert_eq!(
                    launcher(root, window, cx).read(cx).scope(),
                    ProjectScope::Root
                );
            })
            .unwrap();

        let mut remaining = overview();
        remaining.tabs = vec![recent];
        fake.set_overview(remaining);
        visual.run_until_parked();
        window
            .update(&mut visual, |root, window, cx| {
                root.open_new_tab_modal(&NewTab, window, cx);
                assert_eq!(
                    launcher(root, window, cx).read(cx).scope(),
                    ProjectScope::Project("host-project".into())
                );
            })
            .unwrap();
    }

    #[test]
    fn a_locked_page_keeps_its_row_while_the_hosts_catalog_loads_and_ends_on_its_project() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let (_temp, mut cx, window, fake) = setup(None);
        let project = ProjectScope::Project("host-project".into());
        fake.queue_result("catalog", Ok(catalog()));
        window
            .update(&mut cx, |root, window, cx| {
                root.open_sidebar_chat_modal(project.clone(), window, cx);
                assert!(launcher(root, window, cx).read(cx).loading);
            })
            .unwrap();
        assert_eq!(host_row(&window, &cx), Some(project.clone()));
        cx.run_until_parked();
        assert_eq!(host_row(&window, &cx), Some(project.clone()));
        assert_eq!(
            host_page(&window, &mut cx),
            (project.clone(), true, "\\\\nas\\share".to_owned(), false)
        );

        // ⌘N takes the row to Recents and frees the picker.
        window
            .update(&mut cx, |root, window, cx| {
                root.open_new_tab_modal(&NewTab, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(host_row(&window, &cx), Some(ProjectScope::Root));
        assert_eq!(
            host_page(&window, &mut cx),
            (ProjectScope::Root, false, String::new(), false)
        );

        // A project deleted on the host gives a locked page back.
        window
            .update(&mut cx, |root, window, cx| {
                root.open_sidebar_chat_modal(project.clone(), window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert!(host_page(&window, &mut cx).1);
        let mut gone = overview();
        gone.projects.clear();
        fake.set_overview(gone);
        cx.run_until_parked();
        assert_eq!(
            host_page(&window, &mut cx),
            (ProjectScope::Root, false, String::new(), false)
        );
        cx.run_until_parked();
        assert_eq!(host_row(&window, &cx), Some(ProjectScope::Root));
    }

    #[test]
    fn a_mission_entry_frees_a_locked_host_page_and_keeps_its_project() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let (_temp, mut cx, window, fake) = setup(None);
        let project = ProjectScope::Project("host-project".into());
        fake.queue_result("catalog", Ok(catalog()));
        window
            .update(&mut cx, |root, window, cx| {
                root.open_sidebar_chat_modal(project.clone(), window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(host_row(&window, &cx), Some(project.clone()));
        window
            .update(&mut cx, |root, window, cx| {
                root.new_mission_action(&NewMission, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(host_row(&window, &cx), Some(ProjectScope::Root));
        assert_eq!(
            host_page(&window, &mut cx),
            (project, false, "\\\\nas\\share".to_owned(), false)
        );
    }

    #[test]
    fn a_reconnect_keeps_a_locked_page_on_its_row_and_project_until_the_project_is_deleted() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let (_temp, mut cx, window, fake) = setup(None);
        let project = ProjectScope::Project("host-project".into());
        let path = "\\\\nas\\share".to_owned();
        fake.queue_result("catalog", Ok(catalog()));
        window
            .update(&mut cx, |root, window, cx| {
                root.open_sidebar_chat_modal(project.clone(), window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            host_page(&window, &mut cx),
            (project.clone(), true, path.clone(), false)
        );
        let before = window
            .read_with(&cx, |root, cx| root.front_page_client(cx).target)
            .unwrap();

        // The connection's generation moves on, and the page's launcher with it.
        for _ in 0..4 {
            fake.queue_result("catalog", Ok(catalog()));
        }
        window
            .update(&mut cx, |root, _, cx| {
                let clients = root.app_store.read(cx).remote_clients.clone();
                clients.update(cx, |clients, _| clients.invalidate_capabilities("host"));
                root.sync_remote_state(cx);
                assert_ne!(root.front_page_client(cx).target, before);
            })
            .unwrap();
        assert_eq!(host_row(&window, &cx), Some(project.clone()));

        // Rebuilt, it holds the row and the lock before the host's capabilities
        // are back, then while its catalog loads.
        let (scope, locked, _, _) = host_page(&window, &mut cx);
        assert_eq!((scope, locked), (project.clone(), true));
        assert_eq!(host_row(&window, &cx), Some(project.clone()));
        window
            .read_with(&cx, |root, cx| {
                let target = root.front_page_client(cx).target;
                assert!(root
                    .launchers
                    .values()
                    .filter(|launcher| launcher.read(cx).pane.is_none())
                    .all(|launcher| launcher.read(cx).client.target == target));
            })
            .unwrap();
        let mut saw_catalog_loading = false;
        while cx.background_executor.tick() {
            let (scope, locked, _, loading) = host_page(&window, &mut cx);
            assert_eq!((scope, locked), (project.clone(), true));
            assert_eq!(host_row(&window, &cx), Some(project.clone()));
            saw_catalog_loading |= loading;
        }
        assert!(saw_catalog_loading);

        cx.run_until_parked();
        assert_eq!(
            host_page(&window, &mut cx),
            (project.clone(), true, path, false)
        );
        assert_eq!(host_row(&window, &cx), Some(project));

        // A deleted project still gives the page back.
        let mut gone = overview();
        gone.projects.clear();
        fake.set_overview(gone);
        cx.run_until_parked();
        assert_eq!(
            host_page(&window, &mut cx),
            (ProjectScope::Root, false, String::new(), false)
        );
        cx.run_until_parked();
        assert_eq!(host_row(&window, &cx), Some(ProjectScope::Root));
    }

    #[test]
    fn a_reconnect_does_not_bring_an_old_lock_back_into_a_new_page_or_the_empty_state() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let (_temp, mut cx, window, fake) = setup(None);
        let project = ProjectScope::Project("host-project".into());
        for _ in 0..4 {
            fake.queue_result("catalog", Ok(catalog()));
        }
        let reconnect = |window: &gpui::WindowHandle<NativeRoot>, cx: &mut TestAppContext| {
            window
                .update(cx, |root, _, cx| {
                    let clients = root.app_store.read(cx).remote_clients.clone();
                    clients.update(cx, |clients, _| clients.invalidate_capabilities("host"));
                    root.sync_remote_state(cx);
                })
                .unwrap();
        };
        // Left under a project, and replaced while another tab shows.
        window
            .update(&mut cx, |root, window, cx| {
                root.open_sidebar_chat_modal(project.clone(), window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |root, _, cx| {
                root.select_remote_item(RemoteItem::Tab("first".into()), cx)
            })
            .unwrap();
        cx.run_until_parked();
        reconnect(&window, &mut cx);
        window
            .update(&mut cx, |root, window, cx| {
                root.prune_launchers(cx);
                root.open_new_tab_modal(&NewTab, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(host_row(&window, &cx), Some(ProjectScope::Root));
        let (scope, locked, _, _) = host_page(&window, &mut cx);
        assert_eq!((scope, locked), (ProjectScope::Root, false));

        // With no tab or mission left, the empty state starts clean too.
        window
            .update(&mut cx, |root, window, cx| {
                root.open_sidebar_chat_modal(project.clone(), window, cx);
                root.select_remote_item(RemoteItem::Tab("first".into()), cx);
            })
            .unwrap();
        cx.run_until_parked();
        reconnect(&window, &mut cx);
        let mut empty = overview();
        empty.tabs.clear();
        fake.set_overview(empty);
        cx.run_until_parked();
        window
            .read_with(&cx, |root, _| assert!(root.launcher_is_empty_state()))
            .unwrap();
        assert_eq!(host_row(&window, &cx), Some(ProjectScope::Root));
        let (scope, locked, _, _) = host_page(&window, &mut cx);
        assert_eq!((scope, locked), (ProjectScope::Root, false));
    }

    #[test]
    fn a_catalog_without_the_locked_project_leaves_the_page_in_recents_unlocked() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let (_temp, mut cx, window, fake) = setup(None);
        let mut without = catalog();
        without["projects"] = serde_json::json!([]);
        fake.queue_result("catalog", Ok(without));
        window
            .update(&mut cx, |root, window, cx| {
                root.open_sidebar_chat_modal(
                    ProjectScope::Project("host-project".into()),
                    window,
                    cx,
                );
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(
            host_page(&window, &mut cx),
            (ProjectScope::Root, false, String::new(), false)
        );
        assert_eq!(host_row(&window, &cx), Some(ProjectScope::Root));
    }

    #[test]
    fn a_closed_host_tab_lands_on_the_last_viewed_one_and_guards_the_next_close() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let (_temp, mut cx, window, fake) = setup(None);
        let tab = |id: &str| -> runner_core::companion::Tab {
            serde_json::from_value(serde_json::json!({"id":id,"project":null,"name":id,"pinned":false,"panes":{"session":null},"shells":[]})).unwrap()
        };
        let mut three = overview();
        three.tabs.extend([tab("second"), tab("third")]);
        fake.set_overview(three.clone());
        cx.run_until_parked();
        let select = |cx: &mut TestAppContext, item: &str| {
            window
                .update(cx, |root, _, cx| {
                    root.select_remote_item(RemoteItem::Tab(item.into()), cx)
                })
                .unwrap();
            cx.run_until_parked();
        };
        select(&mut cx, "third");
        select(&mut cx, "first");
        window
            .update(&mut cx, |root, _, _| {
                root.remote_client.workspace_mut().unwrap().landed_at = None;
                assert!(!root.tab_close_guarded());
            })
            .unwrap();
        three.tabs.remove(0);
        fake.set_overview(three);
        cx.run_until_parked();
        window
            .read_with(&cx, |root, _| {
                assert!(root.remote_client.active_item() == Some(&RemoteItem::Tab("third".into())));
                assert!(root.tab_close_guarded());
            })
            .unwrap();
        window
            .update(&mut cx, |root, window, cx| {
                crate::surfaces::windowing::close_tab(root, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        assert!(action_trace(&fake, "archive").is_empty());
        window
            .update(&mut cx, |root, window, cx| {
                root.remote_client.workspace_mut().unwrap().landed_at = None;
                crate::surfaces::windowing::close_tab(root, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(action_trace(&fake, "archive").len(), 1);
    }

    #[test]
    fn switching_computers_after_showing_the_page_never_creates_or_archives_a_tab() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let (_temp, mut cx, window, fake) = setup(None);
        window
            .update(&mut cx, |root, window, cx| {
                root.open_new_tab_modal(&NewTab, window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |root, window, cx| {
                root.return_to_this_computer(window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        window.update(&mut cx, choose_host).unwrap();
        cx.run_until_parked();
        assert!(action_trace(&fake, "layout").is_empty());
        assert!(action_trace(&fake, "archive").is_empty());
    }

    #[test]
    fn a_host_crew_page_start_shows_the_launcher_page_on_that_crews_goal_step() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let (_temp, mut cx, window, fake) = setup(None);
        fake.queue_result("catalog", Ok(catalog()));
        window
            .update(&mut cx, |root, window, cx| root.open_crews(window, cx))
            .unwrap();
        cx.run_until_parked();
        fake.queue_result("catalog", Ok(catalog()));
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        let start = visual.debug_bounds("CREW_ROW_START:host-crew").unwrap();
        visual.simulate_click(start.center(), Default::default());
        visual.run_until_parked();
        window
            .read_with(&visual, |root, cx| {
                assert!(matches!(
                    root.remote_client.active_item(),
                    Some(RemoteItem::Launcher)
                ));
                assert!(root.mission_launcher_open(cx));
            })
            .unwrap();
        assert_eq!(action_trace(&fake, "layout").len(), 0);
        assert_eq!(fake.request_count("startMission"), 0);
    }
}
