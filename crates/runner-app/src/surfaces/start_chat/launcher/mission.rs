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

#[derive(Clone)]
pub(super) struct MissionRequest {
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
                root.launch_mission_from_launcher(launcher, request, false, window, cx)
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

    pub(super) fn launch_mission_from_launcher(
        &mut self,
        launcher: Entity<ChatLauncher>,
        request: MissionRequest,
        background: bool,
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
        if !client.valid()
            || (client.target != self.front_page_client(cx).target
                && (!background || client.is_remote()))
        {
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
        let size = match self.remote_client.mission_workspace() {
            Some(workspace) => {
                let size = workspace
                    .read(cx)
                    .estimated_mission_terminal_size(window, cx);
                (size.0.max(20), size.1.max(8))
            }
            _ => self.estimated_mission_terminal_size(window, cx),
        };
        let directory = self.app_store.read(cx).app_data_dir.clone();
        let scope = ProjectScope::or_root(request.project.clone());
        let target = client.target.clone();
        let recent_target = target.clone();
        let retry_request = request.clone();
        let task = cx.background_spawn(async move {
            let crew = request.crew.clone();
            let result = client.start_mission(
                request.crew,
                request.title,
                request.goal,
                request.project,
                request.cwd,
                size,
            );
            let gate = result
                .as_ref()
                .err()
                .and_then(|error| error.version_gate.clone());
            let result = result.map_err(|error| error.to_string());
            if result.is_ok() && !crew.is_empty() {
                record_launcher_crew(&directory, &recent_target, crew);
            }
            (result, gate)
        });
        cx.spawn_in(window, async move |weak, cx| {
            let (result, gate) = task.await;
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
                        let reveal = !background || root.update_launcher_visible(&launcher, cx);
                        if reveal && root.front_page_client(cx).target == target {
                            root.reveal_started_scope(&scope, cx);
                        }
                        root.launchers.retain(|_, cached| cached != &launcher);
                        if !reveal {
                            if remote_form.is_none() {
                                root.refresh_store(StoreRefreshKind::All, cx);
                            }
                        } else if remote_form.is_some() {
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
                            if let Some(gate) = gate {
                                launcher.update_retry = Some(UpdateRetry {
                                    runtime: gate.runtime,
                                    gate: Some(gate),
                                    action: UpdateAction::Mission(retry_request),
                                    requested: false,
                                    ready: false,
                                });
                            }
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
mod tests;

#[cfg(test)]
mod remote_tests;
