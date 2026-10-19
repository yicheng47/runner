use std::collections::{BTreeMap, BTreeSet, VecDeque};

use futures::StreamExt;

use gpui::{App, AppContext, Context, Entity, Global, Subscription, SystemNotification, Task};
use runner_app::ui::agent_status::{StatusKind, StatusPresentation};
use runner_core::protocol::mission::MissionActivityState;
use runner_core::protocol::model::Event;
use runner_core::protocol::status::{AgentStatus, Lifecycle, WaitReason};
use runner_core::Runtime;

use crate::surfaces::mission_workspace::MissionTab;

use crate::app_settings::NotificationSettings;
use crate::app_store::{AppStore, StoreRevisions};
use crate::{global_app_store, AppRoute, NativeRoot};

mod permission;
#[cfg(not(target_os = "linux"))]
pub(crate) use permission::settings_url;
pub(crate) use permission::Permission;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reason {
    Input(WaitReason),
    Error(Option<i32>),
    Failed,
    Finished,
    Question,
    MissionIdle,
}

impl Reason {
    fn label(self) -> String {
        let kind = match self {
            Self::Input(WaitReason::Approval) => StatusKind::Approval,
            Self::Input(WaitReason::Answer) => StatusKind::Answer,
            Self::Input(WaitReason::Unknown) => StatusKind::NeedsYou,
            Self::Error(code) => {
                return code.map_or_else(
                    || crate::i18n::text("ui.error").into(),
                    |code| {
                        crate::i18n::tf("format.error_exit_code", &[("code", (code).to_string())])
                    },
                )
            }
            Self::Failed => StatusKind::ResponseFailed,
            Self::Finished => return crate::i18n::text("ui.finished").into(),
            Self::Question => return crate::i18n::text("ui.asked_you").into(),
            Self::MissionIdle => return crate::i18n::text("ui.every_crew_member_is_idle").into(),
        };
        StatusPresentation {
            kind,
            estimated: false,
            exit_code: None,
            outcome: None,
            waiting_since: None,
            detail: None,
        }
        .label()
        .into()
    }

    fn enabled(self, settings: &NotificationSettings, mission: bool) -> bool {
        match (mission, self) {
            (_, Self::MissionIdle) => settings.missions.idle,
            (_, Self::Question) => settings.missions.question,
            (false, Self::Input(_)) => settings.chats.input,
            (false, Self::Finished) => settings.chats.finished,
            (false, Self::Error(_) | Self::Failed) => settings.chats.failed,
            (true, Self::Input(_)) => settings.missions.input,
            (true, Self::Finished) => false,
            (true, Self::Error(_) | Self::Failed) => settings.missions.failed,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Target {
    Chat(String),
    Slot { mission: String, session: String },
    Question { mission: String },
    MissionIdle { mission: String },
}

#[derive(Clone)]
struct Posted {
    target: Target,
    reason: Reason,
}

#[derive(Default)]
struct Visible {
    app_active: bool,
    chats: Vec<String>,
    mission: Option<(String, MissionTab)>,
}

fn target_visible(target: &Target, visible: &Visible) -> bool {
    match target {
        Target::Chat(id) => visible.chats.contains(id),
        Target::Slot { mission, session } => visible
            .mission
            .as_ref()
            .is_some_and(|(id, tab)| id == mission && tab == &MissionTab::Session(session.clone())),
        Target::MissionIdle { mission } => visible
            .mission
            .as_ref()
            .is_some_and(|(id, _)| id == mission),
        Target::Question { mission } => visible
            .mission
            .as_ref()
            .is_some_and(|(id, tab)| id == mission && tab == &MissionTab::Feed),
    }
}

fn detect_status_triggers(
    previous: Option<&BTreeMap<String, AgentStatus>>,
    next: &BTreeMap<String, AgentStatus>,
    mission_sessions: &BTreeSet<String>,
) -> Vec<(String, Reason)> {
    let Some(previous) = previous else {
        return Vec::new();
    };
    next.iter()
        .filter_map(|(id, status)| {
            let default = AgentStatus::default();
            let old = previous.get(id).unwrap_or(&default);
            let wait = status.observation.interactions.iter().find(|wait| {
                !old.observation
                    .interactions
                    .iter()
                    .any(|old| old.id == wait.id)
            });
            let reason = if let Some(wait) = wait {
                Reason::Input(wait.reason)
            } else if status.lifecycle == Lifecycle::Error && old.lifecycle != Lifecycle::Error {
                Reason::Error(status.exit_code)
            } else if status.failed_since.is_some() && old.failed_since.is_none() {
                Reason::Failed
            } else if !mission_sessions.contains(id)
                && status.unread_since.is_some()
                && old.unread_since.is_none()
            {
                Reason::Finished
            } else {
                return None;
            };
            Some((id.clone(), reason))
        })
        .collect()
}

fn notification_content(
    tag: &str,
    title: String,
    project: Option<&str>,
    reason: Reason,
) -> SystemNotification {
    let label = reason.label();
    SystemNotification {
        tag: tag.to_owned().into(),
        title: title.into(),
        body: project
            .map_or(label.clone(), |project| format!("{label} · {project}"))
            .into(),
        actions: Vec::new(),
    }
}

fn mission_content(tag: &str, handle: &str, mission: &str, reason: Reason) -> SystemNotification {
    SystemNotification {
        tag: tag.to_owned().into(),
        title: format!("@{handle} · {mission}").into(),
        body: reason.label().into(),
        actions: Vec::new(),
    }
}

fn question_content(
    tag: &str,
    asker: Option<&str>,
    mission: &str,
    prompt: &str,
) -> SystemNotification {
    SystemNotification {
        tag: tag.to_owned().into(),
        title: asker
            .map_or_else(
                || {
                    crate::i18n::tf(
                        "format.a_crew_member_asked_you",
                        &[("mission", (mission).to_string())],
                    )
                },
                |asker| {
                    crate::i18n::tf(
                        "format.asked_you",
                        &[
                            ("asker", (asker).to_string()),
                            ("mission", (mission).to_string()),
                        ],
                    )
                },
            )
            .into(),
        body: prompt.to_owned().into(),
        actions: Vec::new(),
    }
}

fn status_snapshot(store: &AppStore) -> BTreeMap<String, AgentStatus> {
    let mut statuses = BTreeMap::new();
    for mission in &store.missions {
        statuses.extend(mission.session_statuses.iter().cloned());
    }
    statuses.extend(
        store
            .session_statuses
            .iter()
            .map(|(id, status)| (id.clone(), status.clone())),
    );
    statuses.retain(|id, _| session_target(store, id).is_some());
    statuses
}

fn mission_snapshot(store: &AppStore) -> BTreeMap<String, Option<MissionActivityState>> {
    store
        .missions
        .iter()
        .filter(|summary| summary.mission.archived_at.is_none())
        .map(|summary| (summary.mission.id.clone(), summary.activity))
        .collect()
}

fn detect_mission_idle(
    previous: &BTreeMap<String, Option<MissionActivityState>>,
    next: &BTreeMap<String, Option<MissionActivityState>>,
) -> Vec<String> {
    next.iter()
        .filter(|(id, activity)| {
            **activity == Some(MissionActivityState::Idle)
                && previous.get(*id) == Some(&Some(MissionActivityState::Busy))
        })
        .map(|(id, _)| id.clone())
        .collect()
}

fn mission_idle_content(mission: &str, title: &str) -> SystemNotification {
    SystemNotification {
        tag: format!("mission:{mission}:idle").into(),
        title: title.to_owned().into(),
        body: Reason::MissionIdle.label().into(),
        actions: Vec::new(),
    }
}

fn session_target(store: &AppStore, id: &str) -> Option<Target> {
    if store
        .session_details
        .get(id)
        .is_some_and(|entry| entry.archived_at.is_none())
    {
        return Some(Target::Chat(id.into()));
    }
    store
        .missions
        .iter()
        .find(|mission| {
            mission.mission.archived_at.is_none()
                && mission
                    .session_statuses
                    .iter()
                    .any(|(session, _)| session == id)
        })
        .map(|mission| Target::Slot {
            mission: mission.mission.id.clone(),
            session: id.into(),
        })
}

fn stale_tags(
    posted: &BTreeMap<String, Posted>,
    statuses: &BTreeMap<String, AgentStatus>,
    missions: &BTreeMap<String, Option<MissionActivityState>>,
    settings: &NotificationSettings,
    visible: &Visible,
    exists: impl Fn(&Target) -> bool,
) -> Vec<String> {
    posted
        .iter()
        .filter_map(|(tag, posted)| {
            let id = match &posted.target {
                Target::Chat(id) => Some(id),
                Target::Slot { session, .. } => Some(session),
                Target::Question { .. } | Target::MissionIdle { .. } => None,
            };
            let holds = if let Target::MissionIdle { mission } = &posted.target {
                missions.get(mission) == Some(&Some(MissionActivityState::Idle))
            } else {
                id.is_none_or(|id| {
                    statuses.get(id).is_some_and(|status| match posted.reason {
                        Reason::Input(_) => !status.observation.interactions.is_empty(),
                        Reason::Finished => status.unread_since.is_some(),
                        Reason::Failed => status.failed_since.is_some(),
                        Reason::Error(_) => status.lifecycle == Lifecycle::Error,
                        Reason::Question | Reason::MissionIdle => true,
                    })
                })
            };
            (!exists(&posted.target)
                || !holds
                || !posted
                    .reason
                    .enabled(settings, !matches!(posted.target, Target::Chat(_)))
                || target_visible(&posted.target, visible))
            .then(|| tag.clone())
        })
        .collect()
}

fn visible(cx: &mut App) -> Visible {
    for window in cx.windows() {
        let Some(window) = window.downcast::<NativeRoot>() else {
            continue;
        };
        if let Ok(Some(visible)) = window.update(cx, |root, window, cx| {
            window.is_window_active().then(|| Visible {
                app_active: root.remote_window.is_none(),
                chats: if root.remote_window.is_none() && root.route == AppRoute::Chat {
                    root.tabs
                        .active()
                        .map(|tab| tab.session_ids())
                        .unwrap_or_default()
                } else {
                    Vec::new()
                },
                mission: match &root.route {
                    AppRoute::Mission(id) if root.remote_window.is_none() => root
                        .mission_workspace
                        .read(cx)
                        .notification_tab(id)
                        .map(|tab| (id.clone(), tab.clone())),
                    _ => None,
                },
            })
        }) {
            return visible;
        }
    }
    Visible::default()
}

struct NotificationController {
    store: Entity<AppStore>,
    revisions: StoreRevisions,
    previous: Option<BTreeMap<String, AgentStatus>>,
    previous_missions: BTreeMap<String, Option<MissionActivityState>>,
    posted: BTreeMap<String, Posted>,
    askers: BTreeMap<String, String>,
    ask_order: VecDeque<String>,
    pending_questions: BTreeMap<String, Event>,
    permission: Permission,
    _subscription: Subscription,
    _pairing_subscription: Subscription,
    _listener: Task<()>,
    _events: Task<()>,
}

struct GlobalNotifications(Entity<NotificationController>);
impl Global for GlobalNotifications {}

fn pairing_content(pairing: &crate::app_store::RemotePairing) -> SystemNotification {
    SystemNotification {
        tag: format!("remote-paired:{}", pairing.fingerprint).into(),
        title: crate::i18n::tf("remote.paired_title", &[("name", pairing.name.clone())]).into(),
        body: crate::i18n::tf("remote.paired_body", &[("name", pairing.name.clone())]).into(),
        actions: Vec::new(),
    }
}

impl NotificationController {
    fn paired(&mut self, pairing: &crate::app_store::RemotePairing, cx: &mut Context<Self>) {
        let notification = pairing_content(pairing);
        #[cfg(target_os = "linux")]
        let system_notification = true;
        #[cfg(not(target_os = "linux"))]
        let system_notification = self.permission == Permission::Allowed;
        if system_notification {
            cx.show_system_notification(notification);
        }
        // Linux cannot report delivery permission, so keep the pairing visible in-app too.
        if !system_notification || cfg!(target_os = "linux") {
            let window = cx
                .active_window()
                .and_then(|window| window.downcast::<NativeRoot>())
                .or_else(|| {
                    cx.windows()
                        .into_iter()
                        .find_map(|window| window.downcast::<NativeRoot>())
                });
            if let Some(window) = window {
                let _ = window.update(cx, |root, _, cx| {
                    root.show_toast(
                        crate::i18n::messagef(
                            "remote.paired_body",
                            &[("name", pairing.name.clone())],
                        ),
                        crate::toast::ToastTone::Success,
                        cx,
                    );
                    root.toasts.remote_pairing();
                });
            }
        }
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let store = self.store.read(cx);
        let revisions = store.revisions;
        if self.revisions == revisions {
            return;
        }
        let statuses_changed = self.revisions.activity != revisions.activity
            || self.revisions.missions != revisions.missions;
        let relevant = statuses_changed
            || self.revisions.sessions != revisions.sessions
            || self.revisions.settings != revisions.settings;
        self.revisions = revisions;
        if !relevant {
            return;
        }
        let (triggers, idle_missions) = if statuses_changed {
            let next = status_snapshot(store);
            let mission_sessions = store
                .missions
                .iter()
                .flat_map(|mission| mission.session_statuses.iter().map(|(id, _)| id.clone()))
                .collect();
            let triggers = detect_status_triggers(self.previous.as_ref(), &next, &mission_sessions);
            self.previous = Some(next);
            let missions = mission_snapshot(store);
            let idle_missions = detect_mission_idle(&self.previous_missions, &missions);
            self.previous_missions = missions;
            (triggers, idle_missions)
        } else {
            (Vec::new(), Vec::new())
        };
        let visible = visible(cx);
        self.withdraw(&visible, cx);
        for (id, reason) in triggers {
            let store = self.store.read(cx);
            let Some(target) = session_target(store, &id) else {
                continue;
            };
            if visible.app_active
                || !reason.enabled(
                    &store.settings.notifications,
                    !matches!(target, Target::Chat(_)),
                )
            {
                continue;
            }
            let notification = match &target {
                Target::Chat(_) => {
                    let entry = &store.session_details[&id];
                    if Runtime::parse(&entry.agent_runtime).is_some_and(Runtime::is_shell) {
                        continue;
                    }
                    let title = crate::surfaces::session_label_live(
                        entry,
                        store
                            .bridge
                            .session(&id)
                            .map(|terminal| terminal.title())
                            .as_deref(),
                    );
                    let project = entry
                        .project_id
                        .as_ref()
                        .and_then(|id| store.projects.iter().find(|project| &project.id == id))
                        .map(|project| project.name.as_str());
                    notification_content(&id, title, project, reason)
                }
                Target::Slot { mission, .. } => {
                    let Ok(rows) = store.client.session_list(mission) else {
                        continue;
                    };
                    let Some(row) = rows.iter().find(|row| row.session.id == id) else {
                        continue;
                    };
                    if Runtime::parse(&row.runtime).is_some_and(Runtime::is_shell) {
                        continue;
                    }
                    let title = &store
                        .missions
                        .iter()
                        .find(|summary| &summary.mission.id == mission)
                        .unwrap()
                        .mission
                        .title;
                    mission_content(&id, &row.handle, title, reason)
                }
                Target::Question { .. } | Target::MissionIdle { .. } => continue,
            };
            cx.show_system_notification(notification);
            self.posted.insert(id, Posted { target, reason });
        }
        for mission_id in idle_missions {
            let store = self.store.read(cx);
            if visible.app_active
                || !Reason::MissionIdle.enabled(&store.settings.notifications, true)
            {
                continue;
            }
            let mission = &store
                .missions
                .iter()
                .find(|summary| summary.mission.id == mission_id)
                .unwrap()
                .mission;
            let notification = mission_idle_content(&mission_id, &mission.title);
            self.posted.insert(
                notification.tag.to_string(),
                Posted {
                    target: Target::MissionIdle {
                        mission: mission_id,
                    },
                    reason: Reason::MissionIdle,
                },
            );
            cx.show_system_notification(notification);
        }
        for event in std::mem::take(&mut self.pending_questions).into_values() {
            self.event(event, cx);
        }
    }

    fn withdraw(&mut self, visible: &Visible, cx: &mut Context<Self>) {
        let store = self.store.read(cx);
        let stale = stale_tags(
            &self.posted,
            &status_snapshot(store),
            &mission_snapshot(store),
            &store.settings.notifications,
            visible,
            |target| match target {
                Target::Chat(id) => store
                    .session_details
                    .get(id)
                    .is_some_and(|entry| entry.archived_at.is_none()),
                Target::Slot { mission, session } => store.missions.iter().any(|summary| {
                    &summary.mission.id == mission
                        && summary.mission.archived_at.is_none()
                        && summary.session_statuses.iter().any(|(id, _)| id == session)
                }),
                Target::Question { mission } | Target::MissionIdle { mission } => {
                    store.missions.iter().any(|summary| {
                        &summary.mission.id == mission && summary.mission.archived_at.is_none()
                    })
                }
            },
        );
        for tag in stale {
            cx.dismiss_system_notification(&tag);
            self.posted.remove(&tag);
        }
    }

    fn event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event.signal_type.as_ref().map(|kind| kind.as_str()) {
            Some("ask_human") => {
                if !self.askers.contains_key(&event.id) {
                    self.ask_order.push_back(event.id.clone());
                }
                self.askers.insert(event.id, event.from);
                while self.ask_order.len() > 256 {
                    if let Some(id) = self.ask_order.pop_front() {
                        self.askers.remove(&id);
                    }
                }
            }
            Some("human_response") => {
                if let Some(id) = event.payload["question_id"].as_str() {
                    self.pending_questions.remove(id);
                    let tag = format!("question:{id}");
                    if self.posted.remove(&tag).is_some() {
                        cx.dismiss_system_notification(&tag);
                    }
                }
            }
            Some("human_question") => {
                let target = Target::Question {
                    mission: event.mission_id.clone(),
                };
                if visible(cx).app_active {
                    return;
                }
                let store = self.store.read(cx);
                if !Reason::Question.enabled(&store.settings.notifications, true) {
                    return;
                }
                let Some(mission) = store
                    .missions
                    .iter()
                    .find(|summary| summary.mission.id == event.mission_id)
                else {
                    self.pending_questions.insert(event.id.clone(), event);
                    return;
                };
                if mission.mission.archived_at.is_some() {
                    return;
                }
                let asker = event.payload["triggered_by"]
                    .as_str()
                    .and_then(|id| self.askers.get(id))
                    .map(String::as_str);
                let tag = format!("question:{}", event.id);
                cx.show_system_notification(question_content(
                    &tag,
                    asker,
                    &mission.mission.title,
                    event.payload["prompt"].as_str().unwrap_or_default(),
                ));
                self.posted.insert(
                    tag,
                    Posted {
                        target,
                        reason: Reason::Question,
                    },
                );
            }
            _ => {}
        }
    }
}

pub(crate) fn install(cx: &mut App) {
    let store = global_app_store(cx);
    let mut events = store.read(cx).client.subscribe();
    let controller = cx.new(|cx: &mut Context<NotificationController>| {
        let (tx, mut rx) = futures::channel::mpsc::unbounded();
        let listener = cx.background_spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) if event.name == "event/appended" => {
                        let Ok(event) =
                            serde_json::from_value::<Event>(event.payload["event"].clone())
                        else {
                            continue;
                        };
                        if matches!(
                            event.signal_type.as_ref().map(|kind| kind.as_str()),
                            Some("ask_human" | "human_question" | "human_response")
                        ) && tx.unbounded_send(event).is_err()
                        {
                            break;
                        }
                    }
                    Ok(_) | Err(runner_core::protocol::EventError::Lagged(_)) => {}
                    Err(runner_core::protocol::EventError::Closed) => break,
                }
            }
        });
        let event_task = cx.spawn(async move |weak, cx| {
            while let Some(event) = rx.next().await {
                if weak.update(cx, |this, cx| this.event(event, cx)).is_err() {
                    break;
                }
            }
        });
        let snapshot = store.read(cx);
        let statuses = status_snapshot(snapshot);
        NotificationController {
            revisions: snapshot.revisions,
            previous: Some(statuses),
            previous_missions: mission_snapshot(snapshot),
            posted: BTreeMap::new(),
            askers: BTreeMap::new(),
            ask_order: VecDeque::new(),
            pending_questions: BTreeMap::new(),
            permission: Permission::Unknown,
            _subscription: cx.observe(&store, |this, _, cx| this.refresh(cx)),
            _pairing_subscription: cx.subscribe(
                &store,
                |this, _, pairing: &crate::app_store::RemotePairing, cx| this.paired(pairing, cx),
            ),
            _listener: listener,
            _events: event_task,
            store,
        }
    });
    cx.set_global(GlobalNotifications(controller));
    refresh_permission(cx);
    request_permission(cx);
    cx.on_system_notification_response(|response, cx| open(response.tag.as_ref(), cx));
}

pub(crate) fn permission(cx: &App) -> Permission {
    cx.try_global::<GlobalNotifications>()
        .map_or(Permission::Unknown, |global| global.0.read(cx).permission)
}

/// Asks the system again, since the user can change the permission in
/// System Settings at any time. Called when a Runner window comes to the
/// front, and after the launch request is answered.
pub(crate) fn refresh_permission(cx: &mut App) {
    let Some(controller) = cx
        .try_global::<GlobalNotifications>()
        .map(|global| global.0.clone())
    else {
        return;
    };
    let answer = permission::query();
    controller.update(cx, |_, cx| {
        cx.spawn(async move |this, cx| {
            let Ok(permission) = answer.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.permission != permission {
                    this.permission = permission;
                    cx.refresh_windows();
                }
            });
        })
        .detach();
    });
}

fn request_permission(cx: &mut App) {
    let answered = permission::request();
    cx.spawn(async move |cx| {
        if answered.await.is_ok() {
            cx.update(refresh_permission);
        }
    })
    .detach();
}

pub(crate) fn view_changed(cx: &mut App) {
    cx.defer(|cx| {
        let Some(controller) = cx
            .try_global::<GlobalNotifications>()
            .map(|global| global.0.clone())
        else {
            return;
        };
        let visible = visible(cx);
        controller.update(cx, |this, cx| this.withdraw(&visible, cx));
    });
}

pub(crate) fn dismiss_all(cx: &mut App) {
    let Some(controller) = cx
        .try_global::<GlobalNotifications>()
        .map(|global| global.0.clone())
    else {
        return;
    };
    controller.update(cx, |this, cx| {
        for (tag, _) in std::mem::take(&mut this.posted) {
            cx.dismiss_system_notification(&tag);
        }
    });
}

pub(crate) fn open(tag: &str, cx: &mut App) {
    if tag.starts_with("remote-paired:") {
        if !cx
            .windows()
            .iter()
            .any(|window| window.downcast::<NativeRoot>().is_some())
        {
            crate::handle_reopen(cx);
        }
        let window = cx
            .active_window()
            .and_then(|window| window.downcast::<NativeRoot>())
            .or_else(|| {
                cx.windows()
                    .into_iter()
                    .find_map(|window| window.downcast::<NativeRoot>())
            });
        if let Some(window) = window {
            let _ = window.update(cx, |root, window, cx| {
                window.activate_window();
                root.enter_settings_route(Some("remote"), window, cx);
            });
        }
        cx.activate(true);
        return;
    }
    let target = cx
        .try_global::<GlobalNotifications>()
        .and_then(|global| global.0.read(cx).posted.get(tag))
        .map(|posted| posted.target.clone())
        .or_else(|| session_target(global_app_store(cx).read(cx), tag))
        .or_else(|| {
            tag.strip_prefix("mission:")
                .and_then(|tag| tag.strip_suffix(":idle"))
                .filter(|id| {
                    global_app_store(cx)
                        .read(cx)
                        .missions
                        .iter()
                        .any(|summary| {
                            summary.mission.id == *id && summary.mission.archived_at.is_none()
                        })
                })
                .map(|id| Target::MissionIdle { mission: id.into() })
        });
    let mut windows = cx
        .windows()
        .into_iter()
        .filter_map(|window| window.downcast::<NativeRoot>())
        .collect::<Vec<_>>();
    if windows.is_empty() {
        crate::handle_reopen(cx);
        windows = cx
            .windows()
            .into_iter()
            .filter_map(|window| window.downcast::<NativeRoot>())
            .collect();
    }
    let entries = global_app_store(cx).read(cx).window_entries.clone();
    let displaying = target.as_ref().and_then(|target| {
        windows
            .iter()
            .copied()
            .filter(|window| {
                window.read(cx).is_ok_and(|root| {
                    root.remote_window.is_none()
                        && match target {
                            Target::Chat(id) => {
                                root.route == AppRoute::Chat
                                    && root
                                        .tabs
                                        .active()
                                        .is_some_and(|tab| tab.session_ids().contains(id))
                            }
                            Target::Slot { mission, .. }
                            | Target::Question { mission }
                            | Target::MissionIdle { mission } => {
                                root.route == AppRoute::Mission(mission.clone())
                            }
                        }
                })
            })
            .max_by_key(|window| {
                window
                    .read(cx)
                    .ok()
                    .and_then(|root| {
                        entries
                            .iter()
                            .find(|entry| entry.label == root.window_label)
                    })
                    .map(|entry| entry.focused_at)
            })
    });
    let holding = target.as_ref().and_then(|target| {
        windows.iter().copied().find(|window| {
            window.read(cx).is_ok_and(|root| match target {
                Target::Chat(id) => root
                    .tabs
                    .tabs()
                    .iter()
                    .any(|tab| tab.session_ids().contains(id)),
                _ => false,
            })
        })
    });
    let window = displaying
        .or(holding)
        .or_else(|| {
            if matches!(target, Some(Target::Chat(_))) {
                windows.iter().copied().find(|window| {
                    window
                        .read(cx)
                        .is_ok_and(|root| root.window_label == "main")
                })
            } else {
                cx.active_window()
                    .and_then(|window| window.downcast::<NativeRoot>())
            }
        })
        .or_else(|| windows.first().copied());
    if let Some(window) = window {
        let _ = window.update(cx, |root, window, cx| {
            window.activate_window();
            if target.is_some() && root.remote_window.is_some() {
                root.remote_window = None;
                root.client_ui.picker_open = false;
                cx.notify();
            }
            match target {
                Some(Target::Chat(id)) => {
                    if !root.open_chat_session(&id, window, cx) {
                        match root.reload_tabs(cx) {
                            Ok(()) => {
                                root.open_chat_session(&id, window, cx);
                            }
                            Err(error) => root.chat_error = Some(error.to_string().into()),
                        }
                    }
                }
                Some(Target::Slot { mission, session }) => {
                    root.update_app_settings(cx, true, |settings| {
                        settings
                            .last_mission_terminal_ids
                            .insert(mission.clone(), session.clone());
                        true
                    });
                    root.open_mission(mission.clone(), window, cx);
                    root.mission_workspace.update(cx, |workspace, cx| {
                        workspace.focus_status_session(&mission, &session, window, cx)
                    });
                }
                Some(Target::Question { mission } | Target::MissionIdle { mission }) => {
                    root.update_app_settings(cx, true, |settings| {
                        settings.last_mission_terminal_ids.remove(&mission);
                        true
                    });
                    root.open_mission(mission.clone(), window, cx);
                    root.mission_workspace.update(cx, |workspace, cx| {
                        workspace.focus_feed(&mission, window, cx)
                    });
                }
                None => {}
            }
        });
    }
    cx.activate(true);
    view_changed(cx);
}

#[cfg(test)]
mod tests;
