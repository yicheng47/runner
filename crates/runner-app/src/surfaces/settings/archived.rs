use std::cmp::Reverse;
use std::collections::HashSet;
use std::rc::Rc;

use chrono::{DateTime, Datelike as _, Local, Utc};
use gpui::prelude::*;
use gpui::{
    div, rems, svg, AnyElement, Context, CursorStyle, Entity, FontWeight, KeyDownEvent, Render,
    SharedString, Subscription, WeakEntity, Window,
};
use runner_app::ui::{
    Button, ButtonSize, ButtonVariant, ConfirmDialog, ConfirmDialogState, PaneHeader, SettingsCard,
    TextField,
};
use runner_backend::model::Mission;
use runner_backend::ops::session::DirectSessionEntry;

#[cfg(target_os = "macos")]
use objc2_foundation::{NSDate, NSDateFormatter, NSString};

use crate::app_store::AppStore;
use crate::theme;
use crate::NativeRoot;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ArchivedFilter {
    #[default]
    All,
    Missions,
    Chats,
}

impl ArchivedFilter {
    const ALL: [Self; 3] = [Self::All, Self::Missions, Self::Chats];

    fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Missions => "Missions",
            Self::Chats => "Chats",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum ArchivedKind {
    Mission,
    Chat,
}

#[derive(Clone, Debug)]
struct ArchivedItem {
    kind: ArchivedKind,
    id: String,
    title: String,
    cwd: Option<String>,
    archived_at: DateTime<Utc>,
}

pub(crate) struct ArchivedPane {
    shell: WeakEntity<NativeRoot>,
    app_store: Entity<AppStore>,
    items: Option<Vec<ArchivedItem>>,
    error: Option<String>,
    loading: bool,
    query: String,
    search: Entity<TextField>,
    filter: ArchivedFilter,
    restoring: HashSet<(ArchivedKind, String)>,
    confirm: ConfirmDialogState,
    confirm_overlay: Entity<ArchivedConfirmOverlay>,
    _subscriptions: Vec<Subscription>,
}

impl ArchivedPane {
    pub(crate) fn new(
        shell: WeakEntity<NativeRoot>,
        app_store: Entity<AppStore>,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|input_cx| {
            let mut input = TextField::new(input_cx.focus_handle(), "", "Search archived…", false)
                .text_size(13.);
            input.set_bare(true, input_cx);
            input
        });
        let search_subscription = cx.observe(&search, |this, input, cx| {
            let query = input.read(cx).text().to_owned();
            if this.query != query {
                this.query = query;
                cx.notify();
            }
        });
        let pane = cx.entity();
        let confirm_overlay = cx.new(|overlay_cx| ArchivedConfirmOverlay::new(pane, overlay_cx));
        Self {
            shell,
            app_store,
            items: None,
            error: None,
            loading: false,
            query: String::new(),
            search,
            filter: ArchivedFilter::All,
            restoring: HashSet::new(),
            confirm: ConfirmDialogState::Closed,
            confirm_overlay,
            _subscriptions: vec![search_subscription],
        }
    }

    pub(crate) fn confirm_overlay(&self) -> Entity<ArchivedConfirmOverlay> {
        self.confirm_overlay.clone()
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading || !self.restoring.is_empty() || self.confirm.is_submitting() {
            return;
        }
        self.loading = true;
        self.items = None;
        self.error = None;
        let core = self.app_store.read(cx).core.clone();
        let task = cx.background_spawn(async move {
            let missions = runner_backend::ops::mission::mission_list_archived(&core, None)
                .map_err(|error| error.to_string())?;
            let chats = runner_backend::ops::session::session_list_archived(&core)
                .map_err(|error| error.to_string())?;
            Ok::<_, String>(merge_archived_items(&missions, &chats))
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.loading = false;
                match result {
                    Ok(items) => {
                        this.items = Some(items);
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn filtered_items(&self) -> Vec<ArchivedItem> {
        self.items
            .as_deref()
            .map(|items| filter_archived_items(items, self.filter, &self.query))
            .unwrap_or_default()
    }

    fn set_filter(&mut self, filter: ArchivedFilter, cx: &mut Context<Self>) {
        if self.filter != filter {
            self.filter = filter;
            cx.notify();
        }
    }

    fn open_item(&mut self, item: ArchivedItem, window: &mut Window, cx: &mut Context<Self>) {
        if item.kind == ArchivedKind::Mission {
            let shell = self.shell.clone();
            window.defer(cx, move |window, cx| {
                if let Some(shell) = shell.upgrade() {
                    shell.update(cx, |shell, shell_cx| {
                        shell.open_mission(item.id, window, shell_cx)
                    });
                }
            });
            return;
        }
        let core = self.app_store.read(cx).core.clone();
        let session_id = item.id;
        let shell = self.shell.clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::session::session_get(&core, &session_id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("session not found: {session_id}"))
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| match result {
                Ok(chat) => {
                    window.defer(cx, move |window, cx| {
                        if let Some(shell) = shell.upgrade() {
                            shell.update(cx, |shell, shell_cx| {
                                shell.open_archived_chat(chat, window, shell_cx)
                            });
                        }
                    });
                }
                Err(error) => {
                    this.error = Some(error);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn restore(&mut self, item: ArchivedItem, cx: &mut Context<Self>) {
        let key = (item.kind, item.id.clone());
        if !self.restoring.insert(key.clone()) {
            return;
        }
        if let Some(items) = self.items.as_mut() {
            items.retain(|row| (row.kind, row.id.as_str()) != (key.0, key.1.as_str()));
        }
        self.error = None;
        let core = self.app_store.read(cx).core.clone();
        let task_item = item.clone();
        let task = cx.background_spawn(async move {
            match task_item.kind {
                ArchivedKind::Mission => {
                    runner_backend::ops::mission::mission_unarchive(&core, task_item.id)
                        .await
                        .map(|_| ())
                        .map_err(|error| error.to_string())
                }
                ArchivedKind::Chat => {
                    runner_backend::ops::session::session_unarchive(&core, &task_item.id)
                        .map_err(|error| error.to_string())
                }
            }
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.restoring.remove(&key);
                match result {
                    Ok(()) => {
                        if let Some(items) = this.items.as_mut() {
                            items.retain(|row| {
                                (row.kind, row.id.as_str()) != (item.kind, item.id.as_str())
                            });
                        }
                    }
                    Err(error) => {
                        this.error = Some(error);
                        let items = this.items.get_or_insert_default();
                        items.retain(|row| {
                            (row.kind, row.id.as_str()) != (item.kind, item.id.as_str())
                        });
                        items.push(item);
                        items.sort_by_key(|item| Reverse(item.archived_at));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn open_confirm(&mut self, cx: &mut Context<Self>) {
        if self.items.as_ref().is_some_and(|items| !items.is_empty()) {
            self.confirm.open();
            cx.notify();
        }
    }

    fn cancel_confirm(&mut self, cx: &mut Context<Self>) {
        let before = self.confirm;
        self.confirm.cancel();
        if self.confirm != before {
            cx.notify();
        }
    }

    fn delete_all(&mut self, cx: &mut Context<Self>) {
        if !self.confirm.submit() {
            return;
        }
        let Some(items) = self.items.clone().filter(|items| !items.is_empty()) else {
            self.confirm.finish();
            cx.notify();
            return;
        };
        self.error = None;
        let core = self.app_store.read(cx).core.clone();
        let task = cx.background_spawn(async move {
            let mut failed = Vec::new();
            let mut first_error = None;
            for item in items {
                let result = match item.kind {
                    ArchivedKind::Mission => {
                        runner_backend::ops::mission::mission_delete(&core, &item.id)
                    }
                    ArchivedKind::Chat => {
                        runner_backend::ops::session::session_delete(&core, &item.id)
                    }
                };
                if let Err(error) = result {
                    first_error.get_or_insert_with(|| error.to_string());
                    failed.push(item);
                }
            }
            (failed, first_error)
        });
        cx.spawn(async move |weak, cx| {
            let (failed, error) = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.items = Some(failed);
                this.error = error;
                this.confirm.finish();
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn render_filters(&self, cx: &mut Context<Self>) -> AnyElement {
        let pane = cx.entity();
        div()
            .flex()
            .flex_none()
            .items_center()
            .gap(rems(2. / 16.))
            .rounded(rems(6. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg())
            .p(rems(2. / 16.))
            .children(ArchivedFilter::ALL.into_iter().map(|filter| {
                let active = self.filter == filter;
                let pane = pane.clone();
                div()
                    .id(SharedString::from(format!(
                        "archived-filter-{}",
                        filter.label()
                    )))
                    .tab_index(0)
                    .cursor_pointer()
                    .rounded(rems(4. / 16.))
                    .bg(if active {
                        theme::raised()
                    } else {
                        gpui::transparent_black()
                    })
                    .px(rems(10. / 16.))
                    .py(rems(5. / 16.))
                    .text_size(rems(12. / 16.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(if active {
                        theme::text()
                    } else {
                        theme::muted()
                    })
                    .hover(|cell| cell.text_color(theme::text()))
                    .child(filter.label())
                    .on_click(move |_, _, cx| {
                        pane.update(cx, |this, pane_cx| this.set_filter(filter, pane_cx));
                    })
            }))
            .into_any_element()
    }

    fn render_row(&self, item: ArchivedItem, cx: &mut Context<Self>) -> AnyElement {
        let pane = cx.entity();
        let open_pane = pane.clone();
        let key_pane = pane.clone();
        let restore_pane = pane;
        let open_item = item.clone();
        let key_item = item.clone();
        let restore_item = item.clone();
        let restoring = self.restoring.contains(&(item.kind, item.id.clone()));
        let cwd = item.cwd.as_deref().map(cwd_basename);
        div()
            .id(SharedString::from(format!(
                "archived-{}-{}",
                item.kind.key(),
                item.id
            )))
            .tab_index(0)
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .py_3()
            .cursor(CursorStyle::PointingHand)
            .hover(|row| row.bg(theme::with_alpha(theme::raised(), 0.4)))
            .focus(|row| row.bg(theme::with_alpha(theme::raised(), 0.4)))
            .child(
                svg()
                    .path(match item.kind {
                        ArchivedKind::Mission => "rocket.svg",
                        ArchivedKind::Chat => "message-square.svg",
                    })
                    .size(rems(14. / 16.))
                    .flex_none()
                    .text_color(theme::faint()),
            )
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_size(rems(13. / 16.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(item.title),
            )
            .children(cwd.map(|cwd| {
                div()
                    .max_w(rems(180. / 16.))
                    .flex_none()
                    .truncate()
                    .font_family("JetBrains Mono")
                    .text_size(rems(11. / 16.))
                    .text_color(theme::faint())
                    .child(cwd)
            }))
            .child(
                div()
                    .flex_none()
                    .text_size(rems(11. / 16.))
                    .text_color(theme::faint())
                    .child(format_timestamp(item.archived_at, Local::now())),
            )
            .child(
                Button::new(
                    SharedString::from(format!("restore-{}-{}", item.kind.key(), item.id)),
                    "Restore",
                )
                .size(ButtonSize::Sm)
                .variant(ButtonVariant::Secondary)
                .disabled(restoring)
                .on_press(move |_, cx| {
                    restore_pane.update(cx, |this, pane_cx| {
                        this.restore(restore_item.clone(), pane_cx)
                    });
                }),
            )
            .on_click(move |_, window, cx| {
                open_pane.update(cx, |this, pane_cx| {
                    this.open_item(open_item.clone(), window, pane_cx)
                });
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "enter" {
                    cx.stop_propagation();
                    key_pane.update(cx, |this, pane_cx| {
                        this.open_item(key_item.clone(), window, pane_cx)
                    });
                }
            })
            .into_any_element()
    }
}

impl ArchivedKind {
    fn key(self) -> &'static str {
        match self {
            Self::Mission => "mission",
            Self::Chat => "chat",
        }
    }
}

impl Render for ArchivedPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pane = cx.entity();
        let count = self.items.as_ref().map_or(0, Vec::len);
        let rows = self.filtered_items();
        let content = if self.items.is_none() {
            None
        } else if count == 0 {
            Some(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(rems(6. / 16.))
                    .rounded(rems(12. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .px_6()
                    .py(rems(48. / 16.))
                    .text_center()
                    .child(
                        svg()
                            .path("archive.svg")
                            .size(rems(20. / 16.))
                            .text_color(theme::faint()),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_size(rems(13. / 16.))
                            .font_weight(FontWeight::MEDIUM)
                            .child("Nothing archived yet"),
                    )
                    .child(
                        div()
                            .text_size(rems(12. / 16.))
                            .text_color(theme::muted())
                            .child(
                                "Archive a chat or mission from the sidebar and it will land here.",
                            ),
                    )
                    .into_any_element(),
            )
        } else if rows.is_empty() {
            Some(
                SettingsCard::new(vec![div()
                    .px_4()
                    .py_6()
                    .text_center()
                    .text_size(rems(12. / 16.))
                    .text_color(theme::faint())
                    .child("No archived items match.")
                    .into_any_element()])
                .into_any_element(),
            )
        } else {
            Some(
                SettingsCard::new(
                    rows.into_iter()
                        .map(|item| self.render_row(item, cx))
                        .collect::<Vec<_>>(),
                )
                .into_any_element(),
            )
        };
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(
                PaneHeader::new(
                    "Archived chats & missions",
                    "Everything you've archived — restore anytime, or delete permanently.",
                )
                .action(
                    Button::new("archived-delete-all", "Delete all")
                        .icon("trash.svg")
                        .size(ButtonSize::Md)
                        .variant(ButtonVariant::Danger)
                        .disabled(
                            self.confirm.is_submitting() || self.items.is_none() || count == 0,
                        )
                        .on_press(move |_, cx| {
                            pane.update(cx, |this, pane_cx| this.open_confirm(pane_cx));
                        }),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .h_8()
                            .min_w_0()
                            .flex_1()
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded(rems(6. / 16.))
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::bg())
                            .px(rems(10. / 16.))
                            .child(
                                svg()
                                    .path("search.svg")
                                    .size(rems(14. / 16.))
                                    .text_color(theme::faint()),
                            )
                            .child(div().min_w_0().flex_1().child(self.search.clone())),
                    )
                    .child(self.render_filters(cx)),
            )
            .children(self.error.clone().map(|error| {
                div()
                    .rounded(rems(12. / 16.))
                    .border_1()
                    .border_color(theme::with_alpha(theme::danger(), 0.3))
                    .bg(theme::with_alpha(theme::danger(), 0.1))
                    .px_4()
                    .py_3()
                    .text_size(rems(12. / 16.))
                    .text_color(theme::danger())
                    .child(error)
            }))
            .children(content)
    }
}

pub(crate) struct ArchivedConfirmOverlay {
    pane: WeakEntity<ArchivedPane>,
    _subscription: Subscription,
}

impl ArchivedConfirmOverlay {
    fn new(pane: Entity<ArchivedPane>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&pane, |_, _, cx| cx.notify());
        Self {
            pane: pane.downgrade(),
            _subscription: subscription,
        }
    }
}

impl Render for ArchivedConfirmOverlay {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(pane) = self.pane.upgrade() else {
            return div().into_any_element();
        };
        let (open, count, busy) = {
            let pane_state = pane.read(cx);
            (
                pane_state.confirm.is_open(),
                pane_state.items.as_ref().map_or(0, Vec::len),
                pane_state.confirm.is_submitting(),
            )
        };
        if !open {
            return div().into_any_element();
        }
        let cancel_pane = pane.clone();
        let confirm_pane = pane;
        ConfirmDialog::new(
            "Delete all archived items?",
            format!(
                "This permanently deletes all {count} archived {}, including mission event logs. This can't be undone.",
                if count == 1 { "item" } else { "items" }
            ),
            "Delete all",
            "Deleting…",
            busy,
            Rc::new(move |_, cx| {
                confirm_pane.update(cx, |pane, pane_cx| pane.delete_all(pane_cx));
            }),
            Rc::new(move |_, cx| {
                cancel_pane.update(cx, |pane, pane_cx| pane.cancel_confirm(pane_cx));
            }),
        )
        .into_any_element()
    }
}

fn chat_title(chat: &DirectSessionEntry) -> String {
    if let Some(title) = chat.title.as_ref().filter(|title| !title.is_empty()) {
        return title.clone();
    }
    let timestamp = chat
        .started_at
        .or(chat.stopped_at)
        .map(|timestamp| format_timestamp(timestamp, Local::now()))
        .unwrap_or_else(|| "session".into());
    chat.handle.as_ref().map_or_else(
        || format!("{} · {timestamp}", chat.display_name),
        |handle| format!("@{handle} · {timestamp}"),
    )
}

fn merge_archived_items(missions: &[Mission], chats: &[DirectSessionEntry]) -> Vec<ArchivedItem> {
    let mut items = missions
        .iter()
        .filter_map(|mission| {
            mission.archived_at.map(|archived_at| ArchivedItem {
                kind: ArchivedKind::Mission,
                id: mission.id.clone(),
                title: mission.title.clone(),
                cwd: mission.cwd.clone(),
                archived_at,
            })
        })
        .chain(chats.iter().filter_map(|chat| {
            chat.archived_at.map(|archived_at| ArchivedItem {
                kind: ArchivedKind::Chat,
                id: chat.session_id.clone(),
                title: chat_title(chat),
                cwd: chat.cwd.clone(),
                archived_at,
            })
        }))
        .collect::<Vec<_>>();
    items.sort_by_key(|item| Reverse(item.archived_at));
    items
}

fn filter_archived_items(
    items: &[ArchivedItem],
    filter: ArchivedFilter,
    query: &str,
) -> Vec<ArchivedItem> {
    let query = query.trim().to_lowercase();
    items
        .iter()
        .filter(|item| match filter {
            ArchivedFilter::All => true,
            ArchivedFilter::Missions => item.kind == ArchivedKind::Mission,
            ArchivedFilter::Chats => item.kind == ArchivedKind::Chat,
        })
        .filter(|item| {
            query.is_empty()
                || item.title.to_lowercase().contains(&query)
                || item
                    .cwd
                    .as_deref()
                    .unwrap_or_default()
                    .to_lowercase()
                    .contains(&query)
        })
        .cloned()
        .collect()
}

fn cwd_basename(cwd: &str) -> String {
    cwd.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(cwd)
        .to_owned()
}

fn format_timestamp(timestamp: DateTime<Utc>, now: DateTime<Local>) -> String {
    let timestamp = timestamp.with_timezone(&Local);
    let same_day = timestamp.year() == now.year()
        && timestamp.month() == now.month()
        && timestamp.day() == now.day();
    localized_timestamp(timestamp, same_day)
}

fn localized_timestamp_template(time_only: bool) -> &'static str {
    if time_only {
        "jm"
    } else {
        "MMM d"
    }
}

#[cfg(target_os = "macos")]
fn localized_timestamp(timestamp: DateTime<Local>, time_only: bool) -> String {
    let date = NSDate::dateWithTimeIntervalSince1970(timestamp.timestamp_millis() as f64 / 1000.);
    let formatter = NSDateFormatter::new();
    let template = NSString::from_str(localized_timestamp_template(time_only));
    formatter.setLocalizedDateFormatFromTemplate(&template);
    formatter.stringFromDate(&date).to_string()
}

#[cfg(not(target_os = "macos"))]
fn localized_timestamp(timestamp: DateTime<Local>, time_only: bool) -> String {
    if time_only {
        timestamp.format("%H:%M").to_string()
    } else {
        timestamp.format("%b %-d").to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runner_backend::model::{MissionStatus, SessionStatus};

    fn mission(id: &str, title: &str, cwd: &str, archived_at: &str) -> Mission {
        Mission {
            id: id.into(),
            crew_id: "crew".into(),
            project_id: None,
            title: title.into(),
            status: MissionStatus::Completed,
            goal_override: None,
            cwd: Some(cwd.into()),
            started_at: "2026-08-01T00:00:00Z".parse().unwrap(),
            stopped_at: None,
            pinned_at: None,
            archived_at: Some(archived_at.parse().unwrap()),
        }
    }

    fn chat(id: &str, title: Option<&str>, cwd: &str, archived_at: &str) -> DirectSessionEntry {
        DirectSessionEntry {
            session_id: id.into(),
            project_id: None,
            runner_id: None,
            handle: Some("coder".into()),
            agent_runtime: "codex".into(),
            agent_command: "codex".into(),
            display_name: "Codex".into(),
            status: SessionStatus::Stopped,
            title: title.map(str::to_owned),
            cwd: Some(cwd.into()),
            started_at: Some("2026-08-01T00:00:00Z".parse().unwrap()),
            stopped_at: None,
            resumable: true,
            agent_session_key: None,
            pinned: false,
            archived_at: Some(archived_at.parse().unwrap()),
        }
    }

    #[test]
    fn merges_missions_and_chats_by_archive_recency() {
        let items = merge_archived_items(
            &[mission(
                "m1",
                "Mission",
                "/work/mission",
                "2026-08-20T01:00:00Z",
            )],
            &[chat(
                "c1",
                Some("Chat"),
                "/work/chat",
                "2026-08-21T01:00:00Z",
            )],
        );
        assert_eq!(
            items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            vec!["c1", "m1"]
        );
    }

    #[test]
    fn filters_by_kind_title_and_cwd() {
        let items = merge_archived_items(
            &[mission(
                "m1",
                "Release",
                "/work/runner",
                "2026-08-20T01:00:00Z",
            )],
            &[chat(
                "c1",
                Some("Investigate"),
                "/work/quill",
                "2026-08-21T01:00:00Z",
            )],
        );
        assert_eq!(
            filter_archived_items(&items, ArchivedFilter::Missions, "release").len(),
            1
        );
        assert_eq!(
            filter_archived_items(&items, ArchivedFilter::Chats, "quill").len(),
            1
        );
        assert!(filter_archived_items(&items, ArchivedFilter::Missions, "quill").is_empty());
    }

    #[test]
    fn untitled_chat_uses_sidebar_style_label() {
        let chat = chat("c1", None, "/work", "2026-08-21T01:00:00Z");
        assert!(chat_title(&chat).starts_with("@coder · "));
    }

    #[test]
    fn archived_timestamps_use_the_platform_formatter() {
        assert_eq!(localized_timestamp_template(true), "jm");
        assert_eq!(localized_timestamp_template(false), "MMM d");
        let now = Local::now();
        assert!(!format_timestamp(now.with_timezone(&Utc), now).is_empty());
    }
}
