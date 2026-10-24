use runner_core::protocol::model::Runtime;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, point, px, rems, svg, BoxShadow, Context, Entity, FontWeight, KeyDownEvent, MouseButton,
    Render, ScrollHandle, SharedString, Subscription, WeakEntity, Window,
};
use runner_app::ui::TextField;
use runner_core::protocol::command::{RunnerCommandState, RunnerCommandStatus};
use runner_core::protocol::crew::CrewListItem;
use runner_core::protocol::mission::MissionSummary;
use runner_core::protocol::model::Role;
use runner_core::protocol::session::DirectSessionEntry;

use crate::*;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum PaletteKind {
    Command,
    Mission,
    Chat,
    Terminal,
    Role,
    Crew,
    Settings,
}

impl PaletteKind {
    fn label(self) -> &'static str {
        match self {
            Self::Command => crate::i18n::text("noun.command"),
            Self::Mission => crate::i18n::text("noun.mission"),
            Self::Chat => crate::i18n::text("noun.chat"),
            Self::Terminal => crate::i18n::text("noun.terminal"),
            Self::Role => crate::i18n::text("noun.role"),
            Self::Crew => crate::i18n::text("noun.crew"),
            Self::Settings => crate::i18n::text("noun.settings"),
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Command => "square-terminal.svg",
            Self::Mission => "flag.svg",
            Self::Chat => "message-square.svg",
            Self::Terminal => "square-terminal.svg",
            Self::Role => "user.svg",
            Self::Crew => "users.svg",
            Self::Settings => "settings.svg",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PaletteDestination {
    NewTerminal,
    NewChat,
    NewMission,
    InstallRunnerCommand,
    UninstallRunnerCommand,
    Mission(String),
    Chat(String),
    Role(String),
    Crew(String),
    Settings,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PaletteItem {
    kind: PaletteKind,
    runtime: Option<String>,
    live: bool,
    id: String,
    label: String,
    destination: PaletteDestination,
    search_text: String,
    order: usize,
}

impl PaletteItem {
    fn icon(&self) -> ChatIcon {
        self.runtime.as_deref().map_or_else(
            || ChatIcon::generic(self.kind.icon()),
            ChatIcon::for_runtime,
        )
    }
}

fn chat_label(handle: Option<&str>, display_name: &str, title: Option<&str>) -> String {
    title.map(str::to_owned).unwrap_or_else(|| {
        handle
            .map(|handle| format!("@{handle}"))
            .unwrap_or_else(|| display_name.to_owned())
    })
}

fn chat_search_text(
    handle: Option<&str>,
    display_name: &str,
    title: Option<&str>,
    cwd: Option<&str>,
) -> String {
    format!(
        "{} {} {}",
        handle.unwrap_or(display_name),
        title.unwrap_or_default(),
        cwd.unwrap_or_default()
    )
    .to_lowercase()
}

fn session_palette_kind(runtime: &str) -> PaletteKind {
    if Runtime::parse(runtime).is_some_and(Runtime::is_shell) {
        PaletteKind::Terminal
    } else {
        PaletteKind::Chat
    }
}

fn palette_items(
    missions: &[MissionSummary],
    chats: &[DirectSessionEntry],
    roles: &[Role],
    crews: &[CrewListItem],
    command_status: Option<&RunnerCommandStatus>,
) -> Vec<PaletteItem> {
    let mut items =
        Vec::with_capacity(missions.len() + chats.len() + roles.len() + crews.len() + 3);
    items.push(PaletteItem {
        kind: PaletteKind::Command,
        runtime: None,
        live: false,
        id: "new-terminal".into(),
        label: crate::i18n::text("ui.new_terminal").into(),
        destination: PaletteDestination::NewTerminal,
        search_text: format!(
            "{} {}",
            i18n::text("ui.new_terminal_shell_drawer"),
            i18n::text_for(i18n::Locale::English, "ui.new_terminal_shell_drawer")
        ),
        order: 0,
    });
    for (id, label, destination) in [
        (
            "new-chat",
            crate::i18n::text("ui.new_chat"),
            PaletteDestination::NewChat,
        ),
        (
            "new-mission",
            crate::i18n::text("ui.new_mission"),
            PaletteDestination::NewMission,
        ),
    ] {
        items.push(PaletteItem {
            kind: PaletteKind::Command,
            runtime: None,
            live: false,
            id: id.into(),
            label: label.into(),
            destination,
            search_text: label.to_lowercase(),
            order: items.len(),
        });
    }
    match command_status.map(|status| &status.state) {
        Some(RunnerCommandState::Installed | RunnerCommandState::Shadowed) => {
            items.push(PaletteItem {
                kind: PaletteKind::Command,
                runtime: None,
                live: false,
                id: "uninstall-runner-command".into(),
                label: crate::i18n::text("ui.uninstall_runner_command").into(),
                destination: PaletteDestination::UninstallRunnerCommand,
                search_text: format!(
                    "{} {}",
                    i18n::text("ui.uninstall_runner_command_cli_path"),
                    i18n::text_for(
                        i18n::Locale::English,
                        "ui.uninstall_runner_command_cli_path"
                    )
                ),
                order: 1,
            });
        }
        Some(RunnerCommandState::NotInstalled) => {
            items.push(PaletteItem {
                kind: PaletteKind::Command,
                runtime: None,
                live: false,
                id: "install-runner-command".into(),
                label: crate::i18n::text("ui.install_runner_command").into(),
                destination: PaletteDestination::InstallRunnerCommand,
                search_text: format!(
                    "{} {}",
                    i18n::text("ui.install_runner_command_cli_path"),
                    i18n::text_for(i18n::Locale::English, "ui.install_runner_command_cli_path")
                ),
                order: 1,
            });
        }
        _ => {}
    }
    items.extend(missions.iter().enumerate().map(|(order, summary)| {
        PaletteItem {
            kind: PaletteKind::Mission,
            runtime: None,
            live: false,
            id: summary.mission.id.clone(),
            label: summary.mission.title.clone(),
            destination: PaletteDestination::Mission(summary.mission.id.clone()),
            search_text: format!(
                "{} {}",
                summary.mission.title,
                summary.crew_name.as_deref().unwrap_or_default()
            )
            .to_lowercase(),
            order,
        }
    }));
    items.extend(chats.iter().enumerate().map(|(order, chat)| PaletteItem {
        kind: session_palette_kind(&chat.agent_runtime),
        runtime: Some(chat.agent_runtime.clone()),
        live: chat.status == SessionStatus::Running,
        id: chat.session_id.clone(),
        label: chat_label(
            chat.handle.as_deref(),
            &chat.display_name,
            chat.preferred_title(None).as_deref(),
        ),
        destination: PaletteDestination::Chat(chat.session_id.clone()),
        search_text: chat_search_text(
            chat.handle.as_deref(),
            &chat.display_name,
            chat.preferred_title(None).as_deref(),
            chat.cwd.as_deref(),
        ),
        order,
    }));
    items.extend(roles.iter().enumerate().map(|(order, role)| PaletteItem {
        kind: PaletteKind::Role,
        runtime: None,
        live: false,
        id: role.id.clone(),
        label: format!("@{}", role.handle),
        destination: PaletteDestination::Role(role.handle.clone()),
        search_text: format!("{} {}", role.handle, role.display_name).to_lowercase(),
        order,
    }));
    items.extend(crews.iter().enumerate().map(|(order, crew)| PaletteItem {
        kind: PaletteKind::Crew,
        runtime: None,
        live: false,
        id: crew.crew.id.clone(),
        label: crew.crew.name.clone(),
        destination: PaletteDestination::Crew(crew.crew.id.clone()),
        search_text: crew.crew.name.to_lowercase(),
        order,
    }));
    items.push(PaletteItem {
        kind: PaletteKind::Settings,
        runtime: None,
        live: false,
        id: "settings".into(),
        label: crate::i18n::text("ui.settings").into(),
        destination: PaletteDestination::Settings,
        search_text: format!(
            "{} {}",
            crate::i18n::text("ui.settings_preferences"),
            i18n::text_for(i18n::Locale::English, "ui.settings_preferences")
        ),
        order: 0,
    });
    for item in &mut items {
        let id = if item.kind == PaletteKind::Settings {
            "open-settings"
        } else {
            item.id.as_str()
        };
        if matches!(item.kind, PaletteKind::Command | PaletteKind::Settings) {
            if let Some(entry) = keymap::entry(id) {
                item.label = entry.title().to_owned();
                item.search_text = format!(
                    "{} {} {} {} {}",
                    item.search_text,
                    entry.title(),
                    entry.description(),
                    i18n::text_for(i18n::Locale::English, entry.title_key),
                    i18n::text_for(i18n::Locale::English, entry.description_key)
                )
                .to_lowercase();
            }
        }
    }
    items
}

fn filtered_palette_items<'a>(items: &'a [PaletteItem], query: &str) -> Vec<&'a PaletteItem> {
    let query = query.trim().to_lowercase();
    let mut filtered = items
        .iter()
        .filter(|item| query.is_empty() || item.search_text.contains(&query))
        .collect::<Vec<_>>();
    filtered.sort_by_key(|item| (item.kind, item.order));
    filtered
}

pub(crate) struct CommandPaletteState {
    shell: WeakEntity<NativeRoot>,
    app_store: Entity<AppStore>,
    input: Entity<TextField>,
    query: String,
    items: Vec<PaletteItem>,
    active_index: usize,
    list_scroll: ScrollHandle,
    open: bool,
    previous_focus: Option<FocusHandle>,
    remote_data: Option<(
        Option<runner_core::companion::Overview>,
        crate::surfaces::front_page::FrontPageClient,
    )>,
    _input_subscription: Subscription,
}

fn remote_palette_items(
    overview: &runner_core::companion::Overview,
    client: &crate::surfaces::front_page::FrontPageClient,
) -> Vec<PaletteItem> {
    let mut items = palette_items(&[], &[], &[], &[], None)
        .into_iter()
        .filter(|item| match item.destination {
            PaletteDestination::NewChat => client.supports(&["catalog", "startChat"]),
            PaletteDestination::NewTerminal => client.supports(&["startShell", "layout"]),
            _ => false,
        })
        .collect::<Vec<_>>();
    items.extend(
        overview
            .missions
            .iter()
            .enumerate()
            .map(|(order, mission)| PaletteItem {
                kind: PaletteKind::Mission,
                runtime: None,
                live: mission.state == runner_core::companion::MissionState::Running,
                id: mission.id.clone(),
                label: mission.title.clone(),
                destination: PaletteDestination::Mission(mission.id.clone()),
                search_text: mission.title.to_lowercase(),
                order,
            }),
    );
    let members = overview
        .missions
        .iter()
        .flat_map(|mission| mission.members.iter().map(|member| member.session.as_str()))
        .collect::<std::collections::HashSet<_>>();
    items.extend(
        overview
            .sessions
            .iter()
            .filter(|(id, _)| !members.contains(id.as_str()))
            .enumerate()
            .map(|(order, (id, session))| {
                let runtime = if session.kind == runner_core::companion::SessionKind::Shell {
                    "shell"
                } else {
                    session.runtime.as_deref().unwrap_or_default()
                };
                let label = if session.title.is_empty() {
                    crate::surfaces::roles::logic::runtime_display_name(runtime)
                } else {
                    session.title.clone()
                };
                PaletteItem {
                    kind: session_palette_kind(runtime),
                    runtime: Some(runtime.into()),
                    live: !matches!(
                        session.status.status.effective(),
                        runner_core::companion::Status::Stopped
                            | runner_core::companion::Status::Error
                    ),
                    id: id.clone(),
                    destination: PaletteDestination::Chat(id.clone()),
                    search_text: format!(
                        "{} {}",
                        label,
                        session.cwd.as_deref().unwrap_or_default()
                    )
                    .to_lowercase(),
                    label,
                    order,
                }
            }),
    );
    items
}

impl NativeRoot {
    pub(crate) fn refresh_remote_palette(&self, cx: &mut Context<Self>) {
        let data = self.remote_window.as_ref().map(|remote| {
            let overview = self
                .app_store
                .read(cx)
                .remote_clients
                .read(cx)
                .overviews
                .get(&remote.computer.id)
                .cloned();
            (overview, remote.front_page.clone())
        });
        self.command_palette.update(cx, |palette, cx| {
            palette.remote_data = data;
            if palette.open && palette.remote_data.is_some() {
                palette.locale_changed(cx);
            }
        });
    }
}

impl CommandPaletteState {
    pub(crate) fn close_for_target_change(&mut self, cx: &mut Context<Self>) {
        self.open = false;
        self.remote_data = None;
        cx.notify();
    }

    pub(crate) fn new(
        shell: WeakEntity<NativeRoot>,
        app_store: Entity<AppStore>,
        cx: &mut Context<Self>,
    ) -> Self {
        let palette = cx.entity();
        let input = cx.new(move |input_cx| {
            let key_palette = palette.clone();
            let mut input = TextField::new(
                input_cx.focus_handle(),
                "",
                crate::i18n::message("ui.command_palette_search_placeholder"),
                false,
            )
            .text_size(theme::text_title())
            .key_interceptor(Rc::new(move |event, window, cx| {
                key_palette.update(cx, |palette, palette_cx| {
                    palette.on_input_key_down(event, window, palette_cx)
                })
            }));
            input.set_bare(true, input_cx);
            input.set_right_padding(0., input_cx);
            input
        });
        let input_subscription = cx.observe(&input, |this, input, cx| {
            let query = input.read(cx).text().to_owned();
            if query == this.query {
                return;
            }
            this.query = query;
            let filtered_len = filtered_palette_items(&this.items, &this.query).len();
            if filtered_len > 0 && this.active_index >= filtered_len {
                this.active_index = 0;
            }
            cx.notify();
        });
        Self {
            shell,
            app_store,
            input,
            query: String::new(),
            items: Vec::new(),
            active_index: 0,
            list_scroll: ScrollHandle::new(),
            open: false,
            previous_focus: None,
            remote_data: None,
            _input_subscription: input_subscription,
        }
    }

    pub(crate) fn locale_changed(&mut self, cx: &mut Context<Self>) {
        let selected = filtered_palette_items(&self.items, &self.query)
            .get(self.active_index)
            .map(|item| item.destination.clone());
        let store = self.app_store.read(cx);
        self.items = if let Some((overview, client)) = &self.remote_data {
            overview
                .as_ref()
                .map(|overview| remote_palette_items(overview, client))
                .unwrap_or_default()
        } else {
            palette_items(
                &store.missions,
                &store.sessions,
                &store.roles,
                &store.crews,
                store.runner_command_status(),
            )
        };
        let filtered = filtered_palette_items(&self.items, &self.query);
        self.active_index = selected
            .and_then(|selected| {
                filtered
                    .iter()
                    .position(|item| item.destination == selected)
            })
            .unwrap_or(0);
        cx.notify();
    }

    pub(crate) fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            return;
        }
        self.previous_focus = window.focused(cx);
        let store = self.app_store.read(cx);
        self.items = if let Some((overview, client)) = &self.remote_data {
            overview
                .as_ref()
                .map(|overview| remote_palette_items(overview, client))
                .unwrap_or_default()
        } else {
            palette_items(
                &store.missions,
                &store.sessions,
                &store.roles,
                &store.crews,
                store.runner_command_status(),
            )
        };
        self.query.clear();
        self.active_index = 0;
        self.list_scroll.set_offset(point(px(0.), px(0.)));
        self.input
            .update(cx, |input, input_cx| input.reset("", input_cx));
        self.open = true;
        self.input.read(cx).focus_handle().focus(window, cx);
        cx.notify();
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        if let Some(focus) = self.previous_focus.take() {
            focus.focus(window, cx);
        }
        cx.notify();
    }

    fn on_input_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if event.keystroke.key == "escape" {
            self.dismiss(window, cx);
            return true;
        }
        let filtered_len = filtered_palette_items(&self.items, &self.query).len();
        if filtered_len == 0 {
            return false;
        }
        match event.keystroke.key.as_str() {
            "down" => {
                self.active_index = (self.active_index + 1) % filtered_len;
                self.scroll_active_into_view();
                cx.notify();
                true
            }
            "up" => {
                self.active_index = (self.active_index + filtered_len - 1) % filtered_len;
                self.scroll_active_into_view();
                cx.notify();
                true
            }
            "enter" => {
                self.select(self.active_index, window, cx);
                true
            }
            _ => false,
        }
    }

    fn scroll_active_into_view(&self) {
        let header_offset = usize::from(self.query.trim().is_empty());
        self.list_scroll
            .scroll_to_item(self.active_index + header_offset);
    }

    fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(destination) = filtered_palette_items(&self.items, &self.query)
            .get(index)
            .map(|item| item.destination.clone())
        else {
            return;
        };
        self.open = false;
        let previous_focus = self.previous_focus.take();
        if let Some(focus) = previous_focus.as_ref() {
            focus.focus(window, cx);
        }
        let expected = self
            .remote_data
            .as_ref()
            .map(|(_, client)| client.target.clone())
            .unwrap_or(crate::surfaces::front_page::FrontPageTarget::Local);
        if !self
            .shell
            .upgrade()
            .is_some_and(|shell| shell.read(cx).front_page_client(cx).target == expected)
        {
            cx.notify();
            return;
        }
        if let Some((_, client)) = &self.remote_data {
            let current = self.shell.upgrade().is_some_and(|shell| {
                shell.read(cx).front_page_client(cx).target == client.target && client.valid()
            });
            if !current {
                cx.notify();
                return;
            }
        }
        let navigated = self.shell.upgrade().is_some_and(|shell| {
            shell.update(cx, |shell, shell_cx| match destination {
                PaletteDestination::NewChat => {
                    shell.open_new_tab_modal(&NewTab, window, shell_cx);
                    true
                }
                PaletteDestination::NewMission => {
                    shell.new_mission_action(&NewMission, window, shell_cx);
                    true
                }
                PaletteDestination::NewTerminal => {
                    shell.new_terminal(window, shell_cx);
                    true
                }
                PaletteDestination::InstallRunnerCommand => {
                    shell.install_runner_command(shell_cx);
                    true
                }
                PaletteDestination::UninstallRunnerCommand => {
                    shell.uninstall_runner_command(shell_cx);
                    true
                }
                PaletteDestination::Mission(mission_id) => {
                    shell.open_mission(mission_id, window, shell_cx);
                    true
                }
                PaletteDestination::Chat(session_id) => {
                    shell.open_chat_session(&session_id, window, shell_cx)
                }
                PaletteDestination::Role(handle) => {
                    shell.open_role_detail(handle, window, shell_cx);
                    true
                }
                PaletteDestination::Crew(crew_id) => {
                    shell.open_crew_editor(crew_id, window, shell_cx);
                    true
                }
                PaletteDestination::Settings => {
                    shell.enter_settings_route(None, window, shell_cx);
                    true
                }
            })
        });
        if !navigated {
            self.open = true;
            self.previous_focus = previous_focus;
            self.input.read(cx).focus_handle().focus(window, cx);
        }
        cx.notify();
    }

    fn set_active(&mut self, index: usize, cx: &mut Context<Self>) {
        if self.active_index == index {
            return;
        }
        self.active_index = index;
        cx.notify();
    }
}

impl Render for CommandPaletteState {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().into_any_element();
        }

        let filtered = filtered_palette_items(&self.items, &self.query)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        if !filtered.is_empty() && self.active_index >= filtered.len() {
            self.active_index = 0;
        }
        let show_recent = self.query.trim().is_empty();
        let rows = filtered
            .into_iter()
            .enumerate()
            .map(|(index, item)| {
                let active = index == self.active_index;
                let hover_palette = cx.entity();
                let click_palette = hover_palette.clone();
                let icon = item.icon();
                div()
                    .id(SharedString::from(format!(
                        "command-palette-{}-{}",
                        item.kind.label(),
                        item.id
                    )))
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .rounded(rems(6. / 16.))
                    .px(rems(10. / 16.))
                    .py_2()
                    .cursor_pointer()
                    .text_color(if active {
                        theme::text()
                    } else {
                        theme::muted()
                    })
                    .when(active, |row| row.bg(theme::raised()))
                    .when(!active, |row| {
                        row.hover(|row| {
                            row.bg(palette_alpha(theme::raised(), 0.6))
                                .text_color(theme::text())
                        })
                    })
                    .on_hover(move |hovered, _, cx| {
                        if *hovered {
                            hover_palette.update(cx, |palette, palette_cx| {
                                palette.set_active(index, palette_cx)
                            });
                        }
                    })
                    .on_click(move |_, window, cx| {
                        click_palette.update(cx, |palette, palette_cx| {
                            palette.select(index, window, palette_cx)
                        });
                    })
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex()
                            .items_center()
                            .gap(rems(10. / 16.))
                            .child(icon.render(
                                rems(14. / 16.),
                                icon.color(theme::muted(), item.live),
                                item.live,
                            ))
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_size(theme::text_body())
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::text())
                                    .child(item.label),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .child(item.kind.label()),
                    )
            })
            .collect::<Vec<_>>();

        let list = if rows.is_empty() {
            div()
                .px_3()
                .py_6()
                .text_center()
                .text_size(theme::text_ui())
                .text_color(theme::faint())
                .child(if self.query.trim().is_empty() {
                    crate::i18n::text("ui.no_commands_missions_chats_roles_or_crews_yet")
                } else {
                    crate::i18n::text("ui.no_matches")
                })
                .into_any_element()
        } else {
            div()
                .id("command-palette-list")
                .max_h(rems(420. / 16.))
                .flex()
                .flex_col()
                .gap(rems(2. / 16.))
                .overflow_y_scroll()
                .track_scroll(&self.list_scroll)
                .px_2()
                .py_2()
                .children(show_recent.then(|| {
                    div()
                        .px(rems(10. / 16.))
                        .py(rems(6. / 16.))
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_caption())
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::faint())
                        .child(crate::i18n::text("heading.recents"))
                }))
                .children(rows)
                .into_any_element()
        };

        div()
            .absolute()
            .inset_0()
            .flex()
            .items_start()
            .justify_center()
            .pt(px(f32::from(window.viewport_size().height) * 0.14))
            .bg(gpui::rgba(0x0000008c))
            .occlude()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.dismiss(window, cx)),
            )
            .child(
                div()
                    .w_full()
                    .max_w(rems(640. / 16.))
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded(rems(12. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .shadow(vec![BoxShadow {
                        color: gpui::rgba(0x00000099).into(),
                        offset: point(px(0.), px(14.)),
                        blur_radius: px(40.),
                        spread_radius: px(0.),
                        inset: false,
                    }])
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(rems(10. / 16.))
                            .border_b_1()
                            .border_color(theme::border())
                            .px(rems(18. / 16.))
                            .py_4()
                            .child(
                                svg()
                                    .path("search.svg")
                                    .size(rems(14. / 16.))
                                    .flex_none()
                                    .text_color(theme::faint()),
                            )
                            .child(div().min_w(px(0.)).flex_1().child(self.input.clone()))
                            .child(
                                div()
                                    .flex_none()
                                    .rounded(rems(4. / 16.))
                                    .bg(theme::bg())
                                    .px(rems(6. / 16.))
                                    .py(px(1.))
                                    .text_size(theme::text_caption())
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::faint())
                                    .child("esc"),
                            ),
                    )
                    .child(list),
            )
            .into_any_element()
    }
}

fn palette_alpha(mut color: gpui::Hsla, alpha: f32) -> gpui::Hsla {
    color.a = alpha;
    color
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: PaletteKind, id: &str, search_text: &str, order: usize) -> PaletteItem {
        PaletteItem {
            kind,
            runtime: None,
            live: false,
            id: id.into(),
            label: id.into(),
            destination: PaletteDestination::Settings,
            search_text: search_text.into(),
            order,
        }
    }

    #[test]
    fn empty_query_groups_kinds_and_preserves_per_kind_recency() {
        let items = vec![
            item(
                PaletteKind::Command,
                "new-terminal",
                "new terminal shell pane",
                0,
            ),
            item(PaletteKind::Crew, "crew-1", "crew", 0),
            item(PaletteKind::Mission, "mission-2", "mission", 1),
            item(PaletteKind::Settings, "settings", "settings preferences", 0),
            item(PaletteKind::Chat, "chat-1", "chat", 0),
            item(PaletteKind::Mission, "mission-1", "mission", 0),
            item(PaletteKind::Role, "role-1", "role", 0),
        ];
        let ids = filtered_palette_items(&items, "")
            .into_iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            [
                "new-terminal",
                "mission-1",
                "mission-2",
                "chat-1",
                "role-1",
                "crew-1",
                "settings"
            ]
        );
    }

    #[test]
    fn palette_always_offers_new_terminal_as_a_command() {
        let items = palette_items(&[], &[], &[], &[], None);
        assert_eq!(items[0].label, "New terminal");
        assert_eq!(items[0].kind, PaletteKind::Command);
        assert_eq!(items[0].destination, PaletteDestination::NewTerminal);
        assert_eq!(
            filtered_palette_items(&items, "shell")[0].id,
            "new-terminal"
        );
    }

    #[test]
    fn palette_offers_chat_and_mission_creation() {
        let items = palette_items(&[], &[], &[], &[], None);
        for (query, destination) in [
            ("new chat", PaletteDestination::NewChat),
            ("new mission", PaletteDestination::NewMission),
        ] {
            let filtered = filtered_palette_items(&items, query);
            assert_eq!(filtered.len(), 1);
            assert_eq!(filtered[0].destination, destination);
            assert_eq!(filtered[0].kind, PaletteKind::Command);
        }
    }

    #[test]
    fn palette_offers_exactly_one_command_install_action_by_state() {
        let not_installed = RunnerCommandStatus::not_installed();
        let items = palette_items(&[], &[], &[], &[], Some(&not_installed));
        assert!(items.iter().any(|item| {
            item.label == "Install runner command"
                && item.destination == PaletteDestination::InstallRunnerCommand
        }));
        assert!(!items
            .iter()
            .any(|item| item.label == "Uninstall runner command"));

        let installed = RunnerCommandStatus {
            state: RunnerCommandState::Installed,
            path: Some("/tmp/runner".into()),
            shadowed_by: None,
        };
        let items = palette_items(&[], &[], &[], &[], Some(&installed));
        assert!(items.iter().any(|item| {
            item.label == "Uninstall runner command"
                && item.destination == PaletteDestination::UninstallRunnerCommand
        }));
        assert!(!items
            .iter()
            .any(|item| item.label == "Install runner command"));

        let foreign = RunnerCommandStatus {
            state: RunnerCommandState::Foreign,
            path: Some("/tmp/runner".into()),
            shadowed_by: None,
        };
        let items = palette_items(&[], &[], &[], &[], Some(&foreign));
        assert!(!items.iter().any(|item| matches!(
            item.destination,
            PaletteDestination::InstallRunnerCommand | PaletteDestination::UninstallRunnerCommand
        )));
    }

    #[test]
    fn query_is_trimmed_case_insensitive_plain_substring_search() {
        let items = vec![
            item(
                PaletteKind::Chat,
                "chat",
                "coder release /users/jason/runner",
                0,
            ),
            item(PaletteKind::Crew, "crew", "peer coding reviews", 0),
        ];
        assert_eq!(filtered_palette_items(&items, "  RUNNER ")[0].id, "chat");
        assert_eq!(filtered_palette_items(&items, "reviews")[0].id, "crew");
        assert!(filtered_palette_items(&items, "fuzzy-gap").is_empty());
    }

    #[test]
    fn chat_label_and_search_match_the_shipped_composition() {
        assert_eq!(chat_label(Some("coder"), "Codex", None), "@coder");
        assert_eq!(
            chat_label(Some("coder"), "Codex", Some("Release prep")),
            "Release prep"
        );
        assert_eq!(chat_label(None, "Shell", None), "Shell");
        assert_eq!(
            chat_search_text(
                Some("Coder"),
                "Codex",
                Some("Release Prep"),
                Some("/Users/Jason/Runner")
            ),
            "coder release prep /users/jason/runner"
        );
    }

    #[test]
    fn shell_sessions_are_terminal_palette_items_not_chats() {
        assert_eq!(session_palette_kind("shell"), PaletteKind::Terminal);
        assert_eq!(session_palette_kind("codex"), PaletteKind::Chat);
    }

    #[test]
    fn chat_results_preserve_runtime_and_liveness_from_the_session() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let mut chat = DirectSessionEntry {
            claude_account: None,
            session_id: "session".into(),
            project_id: None,
            role_id: Some("role".into()),
            handle: Some("coder".into()),
            agent_runtime: "codex".into(),
            agent_command: "codex".into(),
            agent_model: None,
            agent_effort: None,
            display_name: "Codex".into(),
            status: SessionStatus::Running,
            title: Some("Renamed chat".into()),
            live_title: None,
            cwd: None,
            started_at: None,
            stopped_at: None,
            resumable: true,
            native_fork: true,
            forkable: false,
            agent_session_key: None,
            pinned: false,
            archived_at: None,
        };
        for (runtime, path) in [
            ("claude-code", "claude.svg"),
            ("codex", "openai.svg"),
            ("trae", "trae.svg"),
            ("copilot", "copilot.svg"),
            ("pi", "pi.svg"),
            ("antigravity", "antigravity-icon.png"),
            ("shell", "square-terminal.svg"),
            ("unknown", "message-square.svg"),
        ] {
            chat.agent_runtime = runtime.into();
            for status in [
                SessionStatus::Running,
                SessionStatus::Stopped,
                SessionStatus::Crashed,
            ] {
                chat.status = status;
                let items = palette_items(&[], std::slice::from_ref(&chat), &[], &[], None);
                let result = items.iter().find(|item| item.id == "session").unwrap();
                assert_eq!(result.icon().asset_path(), path);
                assert_eq!(result.runtime.as_deref(), Some(runtime));
                assert_eq!(result.live, status == SessionStatus::Running);
                assert_eq!(items[0].icon().asset_path(), "square-terminal.svg");
                assert_eq!(items[0].icon().color(theme::muted(), false), theme::muted());
            }
        }
    }
}

#[cfg(all(test, debug_assertions))]
mod localization_tests {
    use super::*;

    #[test]
    fn palette_refresh_preserves_drafts_and_destination_and_clamps_a_query_with_no_matches() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        i18n::testing::with_locale(i18n::Locale::English, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = gpui::TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            let window = cx.add_window(|window, cx| {
                NativeRoot::new(
                    "palette-locale".into(),
                    temp.path().join("logs"),
                    None,
                    None,
                    store,
                    window,
                    cx,
                )
            });
            let palette = window
                .read_with(&cx, |root, _| root.command_palette.clone())
                .unwrap();
            palette.update(&mut cx, |palette, cx| {
                palette.locale_changed(cx);
                assert_eq!(
                    filtered_palette_items(&palette.items, "preferences")[0].id,
                    "settings"
                )
            });
            i18n::set_active_locale(i18n::Locale::SimplifiedChinese);
            palette.update(&mut cx, |palette, cx| {
                palette.locale_changed(cx);
                assert_eq!(
                    filtered_palette_items(&palette.items, "preferences")[0].id,
                    "settings"
                );
                palette.query = "新建".into();
                palette.active_index = 2;
                palette
                    .input
                    .update(cx, |field, cx| field.set_text("新建", cx));
                assert_eq!(
                    filtered_palette_items(&palette.items, &palette.query)[2].id,
                    "new-mission"
                );
            });
            i18n::set_active_locale(i18n::Locale::English);
            palette.update(&mut cx, |palette, cx| {
                palette.locale_changed(cx);
                assert_eq!(
                    filtered_palette_items(&palette.items, "preferences")[0].id,
                    "settings"
                );
                assert_eq!(palette.query, "新建");
                assert_eq!(palette.input.read(cx).text(), "新建");
                assert!(filtered_palette_items(&palette.items, &palette.query).is_empty());
                assert_eq!(palette.active_index, 0);
                palette.query.clear();
                palette.input.update(cx, |field, cx| field.reset("", cx));
                palette.active_index = filtered_palette_items(&palette.items, "")
                    .iter()
                    .position(|item| item.id == "new-mission")
                    .unwrap();
            });
            i18n::set_active_locale(i18n::Locale::SimplifiedChinese);
            palette.update(&mut cx, |palette, cx| {
                palette.locale_changed(cx);
                assert_eq!(
                    filtered_palette_items(&palette.items, "")[palette.active_index].id,
                    "new-mission"
                );
                assert_eq!(palette.input.read(cx).text(), "");
            });
        });
    }

    #[test]
    fn translated_commands_search_by_label_description_and_original_aliases() {
        i18n::testing::with_locale(i18n::Locale::SimplifiedChinese, || {
            let items = palette_items(&[], &[], &[], &[], None);
            assert_eq!(filtered_palette_items(&items, "新建对话")[0].id, "new-chat");
            assert_eq!(
                filtered_palette_items(&items, "当前空窗格")[0].id,
                "new-terminal"
            );
            assert_eq!(filtered_palette_items(&items, "new chat")[0].id, "new-chat");
            assert_eq!(
                filtered_palette_items(&items, "shell")[0].id,
                "new-terminal"
            );
            assert_eq!(
                items
                    .iter()
                    .find(|item| item.id == "settings")
                    .unwrap()
                    .label,
                "打开设置"
            );
            assert_eq!(PaletteKind::Mission.label(), "任务");
        });
    }
}

#[cfg(test)]
mod remote_isolation_tests {
    use super::*;
    use crate::surfaces::remote_client::tests::{
        add_window, choose_host, enable_front_page, fixture,
    };
    use gpui::TestAppContext;

    #[test]
    fn missing_remote_overview_never_exposes_local_items_or_installs_the_cli() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, clients) = fixture(&mut cx, temp.path());
        enable_front_page(&fake);
        let window = add_window(&mut cx, &store, temp.path(), "palette-isolation");
        window.update(&mut cx, choose_host).unwrap();
        cx.run_until_parked();
        let palette = window
            .read_with(&cx, |root, _| root.command_palette.clone())
            .unwrap();
        window
            .update(&mut cx, |root, window, cx| {
                root.refresh_remote_palette(cx);
                palette.update(cx, |palette, cx| palette.open(window, cx));
            })
            .unwrap();
        assert!(palette.read_with(&cx, |palette, _| !palette.items.is_empty()));
        clients.update(&mut cx, |clients, _| {
            clients.overviews.remove("host");
        });
        window
            .update(&mut cx, |root, window, cx| {
                root.refresh_remote_palette(cx);
                palette.update(cx, |palette, cx| {
                    assert!(palette.remote_data.is_some());
                    assert!(palette.items.is_empty());
                    palette.dismiss(window, cx);
                    palette.open(window, cx);
                    assert!(palette.items.is_empty());
                });
            })
            .unwrap();
        let local_calls = fake.local_call_count();
        window
            .update(&mut cx, |root, _, cx| {
                root.install_runner_command(cx);
                root.uninstall_runner_command(cx);
            })
            .unwrap();
        let mut visual = gpui::VisualTestContext::from_window(window.into(), &cx);
        visual.update(|window, cx| {
            palette.update(cx, |palette, cx| {
                // A callback retained from the local target must also be refused.
                palette.remote_data = None;
                palette.items = palette_items(&[], &[], &[], &[], None);
                palette.items[0].destination = PaletteDestination::InstallRunnerCommand;
                let index = palette
                    .items
                    .iter()
                    .position(|item| item.destination == PaletteDestination::InstallRunnerCommand)
                    .unwrap();
                palette.select(index, window, cx);
            });
        });
        visual.run_until_parked();
        assert_eq!(fake.local_call_count(), local_calls);
        window
            .read_with(&visual, |root, _| {
                assert!(root.remote_window.is_some());
            })
            .unwrap();
    }
}
