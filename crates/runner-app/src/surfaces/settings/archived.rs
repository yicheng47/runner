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
    Button, ButtonSize, ButtonVariant, ConfirmDialog, ConfirmDialogState, PaneHeader,
    SegmentOption, SegmentedControl, SettingsCard, TextField,
};
use runner_core::protocol::model::Mission;
use runner_core::protocol::session::DirectSessionEntry;

use crate::app_store::{AppStore, StoreRefreshKind};
use crate::chat_icon::ChatIcon;
use crate::surfaces::worktree_restore::{
    classify_restore_failure, render_restore_fallback_banner, RestoreFallbackPrompt,
};
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

    const fn key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Missions => "missions",
            Self::Chats => "chats",
        }
    }

    const fn debug_selector(self) -> &'static str {
        match self {
            Self::All => "ARCHIVED_FILTER_ALL",
            Self::Missions => "ARCHIVED_FILTER_MISSIONS",
            Self::Chats => "ARCHIVED_FILTER_CHATS",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::All => crate::i18n::text("ui.all"),
            Self::Missions => crate::i18n::text("ui.missions"),
            Self::Chats => crate::i18n::text("ui.chats"),
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
    runtime: Option<String>,
    id: String,
    project_id: Option<String>,
    title: String,
    cwd: Option<String>,
    archived_at: DateTime<Utc>,
}

#[derive(Clone, Debug)]
struct ArchivedRestoreFallback {
    item: ArchivedItem,
    prompt: RestoreFallbackPrompt,
}

impl ArchivedItem {
    fn icon(&self) -> ChatIcon {
        self.runtime
            .as_deref()
            .map_or_else(|| ChatIcon::generic("rocket.svg"), ChatIcon::for_runtime)
    }
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
    restore_fallback: Option<ArchivedRestoreFallback>,
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
            let mut input = TextField::new(
                input_cx.focus_handle(),
                "",
                crate::i18n::message("ui.search_archived"),
                false,
            )
            .text_size(theme::text_body());
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
            restore_fallback: None,
            confirm: ConfirmDialogState::Closed,
            confirm_overlay,
            _subscriptions: vec![search_subscription],
        }
    }

    pub(crate) fn confirm_overlay(&self) -> Entity<ArchivedConfirmOverlay> {
        self.confirm_overlay.clone()
    }

    pub(crate) fn confirm_open(&self) -> bool {
        self.confirm.is_open()
    }

    pub(crate) fn is_composing(&self, cx: &gpui::App) -> bool {
        self.search.read(cx).is_composing()
    }

    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.loading || !self.restoring.is_empty() || self.confirm.is_submitting() {
            return;
        }
        self.loading = true;
        self.items = None;
        self.error = None;
        self.restore_fallback = None;
        let core = self.app_store.read(cx).client.clone();
        let task = cx.background_spawn(async move {
            let missions = core
                .mission_list_archived(None)
                .map_err(|error| error.to_string())?;
            let chats = core
                .session_list_archived()
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
                        if shell.remote_client.is_active() {
                            shell.return_to_this_computer(window, shell_cx);
                        }
                        shell.open_mission(item.id, window, shell_cx)
                    });
                }
            });
            return;
        }
        let core = self.app_store.read(cx).client.clone();
        let session_id = item.id;
        let shell = self.shell.clone();
        let task = cx.background_spawn(async move {
            core.session_get(&session_id)
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
                                if shell.remote_client.is_active() {
                                    shell.return_to_this_computer(window, shell_cx);
                                }
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
        self.restore_with_options(item, false, cx);
    }

    fn restore_with_options(
        &mut self,
        item: ArchivedItem,
        restore_in_main: bool,
        cx: &mut Context<Self>,
    ) {
        let key = (item.kind, item.id.clone());
        if !self.restoring.insert(key.clone()) {
            return;
        }
        if let Some(items) = self.items.as_mut() {
            items.retain(|row| (row.kind, row.id.as_str()) != (key.0, key.1.as_str()));
        }
        self.error = None;
        self.restore_fallback = None;
        let core = self.app_store.read(cx).client.clone();
        let task_item = item.clone();
        let task = cx.background_spawn(async move {
            let result = match task_item.kind {
                ArchivedKind::Mission => core
                    .mission_unarchive(task_item.id.clone(), restore_in_main)
                    .map(|_| ()),
                ArchivedKind::Chat => core.session_unarchive(&task_item.id, restore_in_main),
            };
            match result {
                Ok(()) => Ok(()),
                Err(error) => {
                    let raw = error.to_string();
                    let fallback = (!restore_in_main)
                        .then(|| {
                            classify_restore_failure(
                                &core,
                                task_item.cwd.as_deref(),
                                task_item.project_id.as_deref(),
                                &raw,
                            )
                        })
                        .flatten();
                    Err((raw, fallback))
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
                        this.app_store.update(cx, |store, cx| {
                            store.refresh(StoreRefreshKind::All, cx);
                            store.sync_worktree_checkouts(cx);
                        });
                    }
                    Err((error, fallback)) => {
                        if let Some(prompt) = fallback {
                            this.restore_fallback = Some(ArchivedRestoreFallback {
                                item: item.clone(),
                                prompt,
                            });
                            this.error = None;
                        } else {
                            this.error = Some(error);
                        }
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
        self.restore_fallback = None;
        let core = self.app_store.read(cx).client.clone();
        let task = cx.background_spawn(async move {
            let mut failed = Vec::new();
            let mut first_error = None;
            for item in items {
                let result = match item.kind {
                    ArchivedKind::Mission => core.mission_delete(&item.id),
                    ArchivedKind::Chat => core.session_delete(&item.id),
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
        SegmentedControl::new(ArchivedFilter::ALL.into_iter().map(|filter| {
            let pane = pane.clone();
            SegmentOption::new(
                format!("archived-filter-{}", filter.key()),
                filter.label(),
                self.filter == filter,
                move |_, cx| {
                    pane.update(cx, |this, pane_cx| this.set_filter(filter, pane_cx));
                },
            )
            .debug_selector(filter.debug_selector())
        }))
        .debug_selector("ARCHIVED_FILTER_SEGMENTED")
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
        let selector_id = item.id.clone();
        let restoring = self.restoring.contains(&(item.kind, item.id.clone()));
        let cwd = item.cwd.as_deref().map(cwd_basename);
        div()
            .id(SharedString::from(format!(
                "archived-{}-{}",
                item.kind.key(),
                item.id
            )))
            .debug_selector(move || format!("ARCHIVED_ROW:{selector_id}"))
            .tab_index(0)
            .flex()
            .items_center()
            .gap_3()
            .px_4()
            .py_3()
            .cursor(CursorStyle::PointingHand)
            .hover(|row| row.bg(theme::with_alpha(theme::raised(), 0.4)))
            .focus(|row| row.bg(theme::with_alpha(theme::raised(), 0.4)))
            .child(item.icon().render(
                rems(14. / 16.),
                item.icon().color(theme::faint(), false),
                false,
            ))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .truncate()
                    .text_size(theme::text_body())
                    .font_weight(FontWeight::MEDIUM)
                    .child(item.title),
            )
            .children(cwd.map(|cwd| {
                div()
                    .max_w(rems(180. / 16.))
                    .flex_none()
                    .truncate()
                    .font_family(theme::UI_MONOSPACE_FONT)
                    .text_size(theme::text_meta())
                    .text_color(theme::faint())
                    .child(cwd)
            }))
            .child(
                div()
                    .flex_none()
                    .text_size(theme::text_meta())
                    .text_color(theme::faint())
                    .child(format_timestamp(item.archived_at, Local::now())),
            )
            .child(
                Button::new(
                    SharedString::from(format!("restore-{}-{}", item.kind.key(), item.id)),
                    crate::i18n::text("ui.restore"),
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
                            .flex_none()
                            .path("archive.svg")
                            .size(rems(20. / 16.))
                            .text_color(theme::faint()),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_size(theme::text_body())
                            .font_weight(FontWeight::MEDIUM)
                            .child(crate::i18n::text("ui.nothing_archived_yet")),
                    )
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::muted())
                            .child(
                                crate::i18n::text("ui.archive_a_chat_or_mission_from_the_sidebar_and_it_will_land_here"),
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
                    .text_size(theme::text_ui())
                    .text_color(theme::faint())
                    .child(crate::i18n::text("ui.no_archived_items_match"))
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
                    crate::i18n::text("ui.archived"),
                    crate::i18n::text(
                        "ui.everything_you_ve_archived_restore_anytime_or_delete_permanently",
                    ),
                )
                .action(
                    Button::new("archived-delete-all", crate::i18n::text("ui.delete_all"))
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
                                    .flex_none()
                                    .path("search.svg")
                                    .size(rems(14. / 16.))
                                    .text_color(theme::faint()),
                            )
                            .child(div().min_w_0().flex_1().child(self.search.clone())),
                    )
                    .child(self.render_filters(cx)),
            )
            .children(
                self.error
                    .clone()
                    .filter(|_| !self.app_store.read(cx).daemon_disconnected)
                    .map(|error| {
                        runner_app::ui::notice_banner(error, runner_app::ui::Tone::Danger)
                    }),
            )
            .children(
                self.restore_fallback
                    .as_ref()
                    .filter(|_| !self.app_store.read(cx).daemon_disconnected)
                    .map(|fallback| {
                        let restore_pane = cx.entity();
                        let dismiss_pane = cx.entity();
                        let busy = self
                            .restoring
                            .contains(&(fallback.item.kind, fallback.item.id.clone()));
                        div()
                            .rounded(rems(8. / 16.))
                            .overflow_hidden()
                            .border_1()
                            .border_color(gpui::rgb(0x4a2226))
                            .child(render_restore_fallback_banner(
                                "archived",
                                &fallback.prompt,
                                busy,
                                move |_, cx| {
                                    restore_pane.update(cx, |this, cx| {
                                        if let Some(fb) = this.restore_fallback.take() {
                                            this.restore_with_options(fb.item, true, cx);
                                        }
                                    });
                                },
                                move |_, cx| {
                                    dismiss_pane.update(cx, |this, cx| {
                                        this.restore_fallback = None;
                                        cx.notify();
                                    });
                                },
                            ))
                    }),
            )
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
            crate::i18n::text("ui.delete_all_archived_items"),
            crate::i18n::tnf("format.this_permanently_deletes_all_archived_including_mission_event_logs_this_can_t", count, &[]),
            crate::i18n::text("ui.delete_all"),
            crate::i18n::text("ui.deleting"),
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
    if let Some(title) = chat.preferred_title(None) {
        return title;
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
                runtime: None,
                id: mission.id.clone(),
                project_id: mission.project_id.clone(),
                title: mission.title.clone(),
                cwd: mission.cwd.clone(),
                archived_at,
            })
        })
        .chain(chats.iter().filter_map(|chat| {
            chat.archived_at.map(|archived_at| ArchivedItem {
                kind: ArchivedKind::Chat,
                runtime: Some(chat.agent_runtime.clone()),
                id: chat.session_id.clone(),
                project_id: chat.project_id.clone(),
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

fn localized_timestamp(timestamp: DateTime<Local>, time_only: bool) -> String {
    timestamp
        .format(crate::i18n::text(if time_only {
            "date.time24"
        } else {
            "date.short"
        }))
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use runner_core::protocol::model::{MissionStatus, SessionStatus};

    fn mission(id: &str, title: &str, cwd: &str, archived_at: &str) -> Mission {
        Mission {
            id: id.into(),
            crew_id: Some("crew".into()),
            project_id: None,
            title: title.into(),
            status: MissionStatus::Completed,
            goal_override: None,
            cwd: Some(cwd.into()),
            started_at: "2026-08-01T00:00:00Z".parse().unwrap(),
            stopped_at: None,
            pinned_at: None,
            archived_at: Some(archived_at.parse().unwrap()),
            linked_chat: None,
        }
    }

    fn chat(id: &str, title: Option<&str>, cwd: &str, archived_at: &str) -> DirectSessionEntry {
        DirectSessionEntry {
            claude_account: None,
            session_id: id.into(),
            project_id: None,
            role_id: None,
            handle: Some("coder".into()),
            agent_runtime: "codex".into(),
            agent_command: "codex".into(),
            agent_model: None,
            agent_effort: None,
            display_name: "Codex".into(),
            status: SessionStatus::Stopped,
            title: title.map(str::to_owned),
            live_title: None,
            cwd: Some(cwd.into()),
            started_at: Some("2026-08-01T00:00:00Z".parse().unwrap()),
            stopped_at: None,
            resumable: true,
            native_fork: true,
            forkable: false,
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
    fn archived_chat_marks_keep_the_runtime_and_draw_neutral() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let mut chat = chat(
            "c1",
            Some("Renamed chat"),
            "/work/chat",
            "2026-08-21T01:00:00Z",
        );
        chat.role_id = Some("role".into());
        for (runtime, path) in [
            ("claude-code", "claude.svg"),
            ("codex", "openai.svg"),
            ("trae", "trae.svg"),
            ("copilot", "copilot.svg"),
            ("pi", "pi.svg"),
            ("antigravity", "antigravity-icon.png"),
        ] {
            chat.agent_runtime = runtime.into();
            let items = merge_archived_items(&[], std::slice::from_ref(&chat));
            assert_eq!(items[0].runtime.as_deref(), Some(runtime));
            assert_eq!(items[0].icon().asset_path(), path);
            assert_eq!(
                items[0].icon().color(theme::faint(), false),
                theme::with_alpha(theme::text(), 0.45)
            );
        }
        chat.agent_runtime = "unknown".into();
        let items = merge_archived_items(
            &[mission(
                "m1",
                "Mission",
                "/work/mission",
                "2026-08-20T01:00:00Z",
            )],
            &[chat],
        );
        assert_eq!(items[0].icon().asset_path(), "message-square.svg");
        assert_eq!(items[1].icon().asset_path(), "rocket.svg");
        assert_eq!(items[1].icon().color(theme::faint(), false), theme::faint());
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
    fn archived_timestamps_follow_the_display_language() {
        let date = "2026-10-16T13:45:00Z"
            .parse::<DateTime<Utc>>()
            .unwrap()
            .with_timezone(&Local);
        crate::i18n::testing::with_locale(crate::i18n::Locale::English, || {
            assert_eq!(localized_timestamp(date, false), "Oct 16")
        });
        crate::i18n::testing::with_locale(crate::i18n::Locale::SimplifiedChinese, || {
            assert_eq!(localized_timestamp(date, false), "10月16日")
        });
        let now = Local::now();
        assert!(!format_timestamp(now.with_timezone(&Utc), now).is_empty());
    }

    struct ArchivedHost(Entity<ArchivedPane>);

    impl Render for ArchivedHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .p_6()
                .bg(theme::bg())
                .child(self.0.clone())
        }
    }

    fn git_cmd(dir: &std::path::Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git command");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    #[test]
    fn archived_restore_fallback_handles_gone_and_ambiguous_branches() {
        let mut cx = gpui::TestAppContext::single();
        let cx = &mut cx;
        let temp = tempfile::Builder::new()
            .prefix("1213-m2-arch-restore-")
            .tempdir()
            .unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join("apps/web")).unwrap();
        git_cmd(&repo, &["init", "-b", "main"]);
        git_cmd(&repo, &["config", "user.name", "Runner Test"]);
        git_cmd(&repo, &["config", "user.email", "test@runner.local"]);
        std::fs::write(repo.join("apps/web/index.ts"), "export {};\n").unwrap();
        git_cmd(&repo, &["add", "."]);
        git_cmd(&repo, &["commit", "-m", "initial"]);

        let wt_gone = repo.join(".worktrees/gone-branch");
        let wt_ambig = repo.join(".worktrees/feat-dup");
        git_cmd(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "feat/gone-branch",
                wt_gone.to_str().unwrap(),
            ],
        );
        git_cmd(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "feat/dup",
                wt_ambig.to_str().unwrap(),
            ],
        );
        git_cmd(&repo, &["branch", "feat-dup", "main"]);

        std::fs::remove_dir_all(&wt_gone).unwrap();
        std::fs::remove_dir_all(&wt_ambig).unwrap();
        git_cmd(&repo, &["worktree", "prune"]);
        git_cmd(&repo, &["branch", "-D", "feat/gone-branch"]);

        let store = crate::app_store::test_lifecycle_store(cx, temp.path());
        let core = store.read_with(cx, |store, _| store.test_core.clone());
        let project = runner_daemon::repo::project::create(
            &core.db.get().unwrap(),
            "RestoreRepo",
            &repo.to_string_lossy(),
        )
        .unwrap();
        {
            let conn = core.db.get().unwrap();
            let gone_sub = wt_gone.join("apps/web");
            conn.execute_batch(
                "INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES ('crew', 'Crew', '2026-11-17T00:00:00Z', '2026-11-17T00:00:00Z');",
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions(id,title,status,agent_runtime,agent_command,project_id,cwd,archived_at)
                 VALUES ('c-gone','Gone chat','stopped','claude-code','claude',?1,?2,'2026-11-17T00:01:00Z')",
                [project.id.as_str(), gone_sub.to_string_lossy().as_ref()],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO missions(id,crew_id,project_id,title,status,cwd,started_at,archived_at)
                 VALUES ('m-ambig','crew',?1,'Ambiguous mission','completed',?2,'2026-11-17T00:00:00Z','2026-11-17T00:02:00Z')",
                [project.id.as_str(), wt_ambig.to_string_lossy().as_ref()],
            )
            .unwrap();
        }

        let shell = cx.add_window(|window, cx| {
            NativeRoot::new(
                "arch-restore".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            )
        });
        let shell_weak = shell.entity(cx).unwrap().downgrade();
        let pane_win = cx.add_window(|_, cx| {
            let pane = cx.new(|cx| ArchivedPane::new(shell_weak, store.clone(), cx));
            pane.update(cx, |pane, cx| pane.refresh(cx));
            ArchivedHost(pane)
        });
        let pane = pane_win.read_with(cx, |host, _| host.0.clone()).unwrap();
        let mut visual = gpui::VisualTestContext::from_window(pane_win.into(), cx);

        fn wait_until(
            visual: &gpui::VisualTestContext,
            label: &str,
            mut pred: impl FnMut() -> bool,
        ) {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            loop {
                visual.run_until_parked();
                if pred() {
                    return;
                }
                assert!(
                    std::time::Instant::now() < deadline,
                    "timed out waiting for {label}"
                );
                std::thread::yield_now();
            }
        }

        wait_until(&visual, "archived items loaded", || {
            pane.read_with(&visual, |p, _| {
                p.items.as_ref().is_some_and(|i| i.len() == 2)
            })
        });

        // 1. Gone branch on archived Claude chat: offers Restore in main checkout + history warning.
        let gone_item = pane.read_with(&visual, |p, _| {
            p.items
                .as_ref()
                .unwrap()
                .iter()
                .find(|item| item.id == "c-gone")
                .unwrap()
                .clone()
        });
        pane.update(&mut visual, |p, cx| p.restore(gone_item, cx));
        wait_until(&visual, "gone fallback banner", || {
            pane.read_with(&visual, |p, _| p.restore_fallback.is_some())
        });
        let gone_msg = pane.read_with(&visual, |p, _| {
            p.restore_fallback
                .as_ref()
                .unwrap()
                .prompt
                .message
                .resolve()
        });
        assert!(gone_msg.contains("no longer exists"), "{gone_msg}");
        assert!(
            gone_msg.to_lowercase().contains("without its history"),
            "{gone_msg}"
        );
        visual.run_until_parked();
        assert!(visual.debug_bounds("RESTORE_FALLBACK_BANNER").is_some());
        assert!(visual.debug_bounds("RESTORE_IN_MAIN_BUTTON").is_some());

        pane.update(&mut visual, |p, cx| {
            let fb = p.restore_fallback.take().unwrap();
            p.restore_with_options(fb.item, true, cx);
        });
        wait_until(&visual, "c-gone restored into main/apps/web", || {
            let row = runner_daemon::repo::session::get_row(&core.db.get().unwrap(), "c-gone")
                .unwrap()
                .unwrap();
            row.archived_at.is_none()
        });
        let restored_chat =
            runner_daemon::repo::session::get_row(&core.db.get().unwrap(), "c-gone")
                .unwrap()
                .unwrap();
        assert_eq!(
            runner_core::app_paths::normalize_checkout_path(std::path::Path::new(
                restored_chat.cwd.as_deref().unwrap()
            )),
            runner_core::app_paths::normalize_checkout_path(&repo.join("apps/web"))
        );

        // 2. Ambiguous branch on archived mission: lists candidate branches and restores to main root.
        let ambig_item = pane.read_with(&visual, |p, _| {
            p.items
                .as_ref()
                .unwrap()
                .iter()
                .find(|item| item.id == "m-ambig")
                .unwrap()
                .clone()
        });
        pane.update(&mut visual, |p, cx| p.restore(ambig_item, cx));
        wait_until(&visual, "ambiguous fallback banner", || {
            pane.read_with(&visual, |p, _| p.restore_fallback.is_some())
        });
        let ambig_msg = pane.read_with(&visual, |p, _| {
            p.restore_fallback
                .as_ref()
                .unwrap()
                .prompt
                .message
                .resolve()
        });
        assert!(ambig_msg.contains("feat/dup"), "{ambig_msg}");
        assert!(ambig_msg.contains("feat-dup"), "{ambig_msg}");
        assert!(
            ambig_msg.to_lowercase().contains("without its history"),
            "{ambig_msg}"
        );

        pane.update(&mut visual, |p, cx| {
            let fb = p.restore_fallback.take().unwrap();
            p.restore_with_options(fb.item, true, cx);
        });
        wait_until(&visual, "m-ambig restored into main root", || {
            let mission = runner_daemon::repo::mission::get(&core.db.get().unwrap(), "m-ambig")
                .unwrap()
                .unwrap();
            mission.archived_at.is_none()
        });
        let restored_mission =
            runner_daemon::repo::mission::get(&core.db.get().unwrap(), "m-ambig")
                .unwrap()
                .unwrap();
        assert_eq!(
            runner_core::app_paths::normalize_checkout_path(std::path::Path::new(
                restored_mission.cwd.as_deref().unwrap()
            )),
            runner_core::app_paths::normalize_checkout_path(&repo)
        );
    }

    #[test]
    fn archived_filter_uses_shared_segmented_control_and_updates_filtered_rows() {
        use crate::theme_snapshot::{assert_border, assert_fill_color, assert_text, ThemeGuard};
        use crate::{AppRoute, SettingsPane};
        use gpui::{px, size, Modifiers, TestAppContext, VisualTestContext};

        let _theme = ThemeGuard::new();
        theme::set_active_variant(theme::ThemeVariant::RunnerLight);
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let host = cx.add_window(|window, cx| {
            let mut root = NativeRoot::new(
                "arch-filter".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            let shell_weak = cx.entity().downgrade();
            let pane = cx.new(|cx| {
                let mut pane = ArchivedPane::new(shell_weak, store.clone(), cx);
                pane.items = Some(merge_archived_items(
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
                ));
                pane
            });
            root.route = AppRoute::Settings;
            root.settings_page.pane = SettingsPane::Archived;
            root.settings_page.archived = Some(pane);
            root
        });
        let pane = host
            .read_with(&cx, |root, _| root.settings_page.archived.clone().unwrap())
            .unwrap();
        let mut visual = VisualTestContext::from_window(host.into(), &cx);
        visual.simulate_resize(size(px(1200.), px(800.)));
        visual.run_until_parked();

        assert_fill_color(&mut visual, "ARCHIVED_FILTER_SEGMENTED", Some(theme::bg()));
        assert_border(
            &mut visual,
            "ARCHIVED_FILTER_SEGMENTED",
            theme::border_strong(),
        );
        assert_fill_color(
            &mut visual,
            "ARCHIVED_FILTER_ALL",
            Some(theme::seg_selected()),
        );
        assert_border(
            &mut visual,
            "ARCHIVED_FILTER_ALL",
            theme::seg_selected_border(),
        );
        assert_text(&mut visual, "ARCHIVED_FILTER_ALL", theme::text());
        assert_fill_color(
            &mut visual,
            "ARCHIVED_FILTER_MISSIONS",
            Some(gpui::transparent_black()),
        );
        assert_text(&mut visual, "ARCHIVED_FILTER_MISSIONS", theme::muted());
        assert!(visual.debug_bounds("ARCHIVED_ROW:m1").is_some());
        assert!(visual.debug_bounds("ARCHIVED_ROW:c1").is_some());

        let missions = visual.debug_bounds("ARCHIVED_FILTER_MISSIONS").unwrap();
        visual.simulate_click(missions.center(), Modifiers::default());
        visual.run_until_parked();
        assert_eq!(
            pane.read_with(&visual, |p, _| p.filter),
            ArchivedFilter::Missions
        );
        assert_fill_color(
            &mut visual,
            "ARCHIVED_FILTER_MISSIONS",
            Some(theme::seg_selected()),
        );
        assert_fill_color(
            &mut visual,
            "ARCHIVED_FILTER_ALL",
            Some(gpui::transparent_black()),
        );
        assert!(visual.debug_bounds("ARCHIVED_ROW:m1").is_some());
        assert!(visual.debug_bounds("ARCHIVED_ROW:c1").is_none());

        // Ctrl+Tab does not traverse out of Missions; real Tab moves focus to Chats.
        let missions_focus = host
            .update(&mut visual, |_, window, cx| window.focused(cx).unwrap())
            .unwrap();
        visual.simulate_keystrokes("ctrl-tab");
        visual.run_until_parked();
        assert_eq!(
            host.update(&mut visual, |_, window, cx| window.focused(cx))
                .unwrap(),
            Some(missions_focus.clone())
        );

        visual.simulate_keystrokes("tab");
        visual.run_until_parked();
        let chats_focus = host
            .update(&mut visual, |_, window, cx| window.focused(cx).unwrap())
            .unwrap();
        assert_ne!(chats_focus, missions_focus);

        // Shift+Space is ignored; plain Space activates Chats.
        visual.simulate_keystrokes("shift-space");
        visual.run_until_parked();
        assert_eq!(
            pane.read_with(&visual, |p, _| p.filter),
            ArchivedFilter::Missions
        );

        visual.simulate_keystrokes("space");
        visual.run_until_parked();
        assert_eq!(
            pane.read_with(&visual, |p, _| p.filter),
            ArchivedFilter::Chats
        );
        assert_fill_color(
            &mut visual,
            "ARCHIVED_FILTER_CHATS",
            Some(theme::seg_selected()),
        );
        assert!(visual.debug_bounds("ARCHIVED_ROW:m1").is_none());
        assert!(visual.debug_bounds("ARCHIVED_ROW:c1").is_some());

        // Shift-Tab moves focus back to Missions and Enter activates it.
        visual.simulate_keystrokes("shift-tab");
        visual.run_until_parked();
        assert_eq!(
            host.update(&mut visual, |_, window, cx| window.focused(cx))
                .unwrap(),
            Some(missions_focus)
        );
        visual.simulate_keystrokes("enter");
        visual.run_until_parked();
        assert_eq!(
            pane.read_with(&visual, |p, _| p.filter),
            ArchivedFilter::Missions
        );
    }
}
