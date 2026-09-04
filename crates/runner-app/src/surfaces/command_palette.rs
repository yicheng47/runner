use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, point, px, rems, svg, BoxShadow, Context, Entity, FontWeight, KeyDownEvent, MouseButton,
    Render, ScrollHandle, SharedString, Subscription, WeakEntity, Window,
};
use runner_app::ui::TextField;
use runner_backend::model::Runner;
use runner_backend::ops::crew::CrewListItem;
use runner_backend::ops::mission::MissionSummary;
use runner_backend::ops::session::DirectSessionEntry;

use crate::*;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
enum PaletteKind {
    Command,
    Mission,
    Chat,
    Terminal,
    Runner,
    Crew,
    Settings,
}

impl PaletteKind {
    fn label(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Mission => "mission",
            Self::Chat => "chat",
            Self::Terminal => "terminal",
            Self::Runner => "runner",
            Self::Crew => "crew",
            Self::Settings => "settings",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Command => "square-terminal.svg",
            Self::Mission => "flag.svg",
            Self::Chat => "message-square.svg",
            Self::Terminal => "square-terminal.svg",
            Self::Runner => "terminal.svg",
            Self::Crew => "users.svg",
            Self::Settings => "settings.svg",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum PaletteDestination {
    NewTerminal,
    Mission(String),
    Chat(String),
    Runner(String),
    Crew(String),
    Settings,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PaletteItem {
    kind: PaletteKind,
    id: String,
    label: String,
    destination: PaletteDestination,
    search_text: String,
    order: usize,
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
    if runtime == "shell" {
        PaletteKind::Terminal
    } else {
        PaletteKind::Chat
    }
}

fn palette_items(
    missions: &[MissionSummary],
    chats: &[DirectSessionEntry],
    runners: &[Runner],
    crews: &[CrewListItem],
) -> Vec<PaletteItem> {
    let mut items =
        Vec::with_capacity(missions.len() + chats.len() + runners.len() + crews.len() + 2);
    items.push(PaletteItem {
        kind: PaletteKind::Command,
        id: "new-terminal".into(),
        label: "New terminal".into(),
        destination: PaletteDestination::NewTerminal,
        search_text: "new terminal shell drawer".into(),
        order: 0,
    });
    items.extend(
        missions
            .iter()
            .enumerate()
            .map(|(order, summary)| PaletteItem {
                kind: PaletteKind::Mission,
                id: summary.mission.id.clone(),
                label: summary.mission.title.clone(),
                destination: PaletteDestination::Mission(summary.mission.id.clone()),
                search_text: format!("{} {}", summary.mission.title, summary.crew_name)
                    .to_lowercase(),
                order,
            }),
    );
    items.extend(chats.iter().enumerate().map(|(order, chat)| PaletteItem {
        kind: session_palette_kind(&chat.agent_runtime),
        id: chat.session_id.clone(),
        label: chat_label(
            chat.handle.as_deref(),
            &chat.display_name,
            chat.title.as_deref(),
        ),
        destination: PaletteDestination::Chat(chat.session_id.clone()),
        search_text: chat_search_text(
            chat.handle.as_deref(),
            &chat.display_name,
            chat.title.as_deref(),
            chat.cwd.as_deref(),
        ),
        order,
    }));
    items.extend(
        runners
            .iter()
            .enumerate()
            .map(|(order, runner)| PaletteItem {
                kind: PaletteKind::Runner,
                id: runner.id.clone(),
                label: format!("@{}", runner.handle),
                destination: PaletteDestination::Runner(runner.handle.clone()),
                search_text: format!("{} {}", runner.handle, runner.display_name).to_lowercase(),
                order,
            }),
    );
    items.extend(crews.iter().enumerate().map(|(order, crew)| {
        PaletteItem {
            kind: PaletteKind::Crew,
            id: crew.crew.id.clone(),
            label: crew.crew.name.clone(),
            destination: PaletteDestination::Crew(crew.crew.id.clone()),
            search_text: format!(
                "{} {}",
                crew.crew.name,
                crew.crew.purpose.as_deref().unwrap_or_default()
            )
            .to_lowercase(),
            order,
        }
    }));
    items.push(PaletteItem {
        kind: PaletteKind::Settings,
        id: "settings".into(),
        label: "Settings".into(),
        destination: PaletteDestination::Settings,
        search_text: "settings preferences".into(),
        order: 0,
    });
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
    _input_subscription: Subscription,
}

impl CommandPaletteState {
    pub(crate) fn new(
        shell: WeakEntity<NativeRoot>,
        app_store: Entity<AppStore>,
        cx: &mut Context<Self>,
    ) -> Self {
        let palette = cx.entity();
        let input = cx.new(move |input_cx| {
            let key_palette = palette.clone();
            let mut input = TextField::new(input_cx.focus_handle(), "", "Search…", false)
                .text_size(14.)
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
            _input_subscription: input_subscription,
        }
    }

    pub(crate) fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            return;
        }
        self.previous_focus = window.focused(cx);
        let store = self.app_store.read(cx);
        self.items = palette_items(
            &store.missions,
            &store.sessions,
            &store.runners,
            &store.crews,
        );
        self.query.clear();
        self.active_index = 0;
        self.list_scroll.set_offset(point(px(0.), px(0.)));
        self.input
            .update(cx, |input, input_cx| input.reset("", input_cx));
        self.open = true;
        self.input.read(cx).focus_handle().focus(window);
        cx.notify();
    }

    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.open {
            return;
        }
        self.open = false;
        if let Some(focus) = self.previous_focus.take() {
            focus.focus(window);
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
            focus.focus(window);
        }
        let navigated = self.shell.upgrade().is_some_and(|shell| {
            shell.update(cx, |shell, shell_cx| match destination {
                PaletteDestination::NewTerminal => {
                    shell.new_terminal(window, shell_cx);
                    true
                }
                PaletteDestination::Mission(mission_id) => {
                    shell.open_mission(mission_id, window, shell_cx);
                    true
                }
                PaletteDestination::Chat(session_id) => {
                    shell.open_chat_session(&session_id, window, shell_cx)
                }
                PaletteDestination::Runner(handle) => {
                    shell.open_runner_detail(handle, window, shell_cx);
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
            self.input.read(cx).focus_handle().focus(window);
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
                            .child(
                                svg()
                                    .path(item.kind.icon())
                                    .size(rems(14. / 16.))
                                    .flex_none()
                                    .text_color(theme::muted()),
                            )
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_size(rems(13. / 16.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::text())
                                    .child(item.label),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_family("Menlo")
                            .text_size(rems(11. / 16.))
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
                .text_size(rems(12. / 16.))
                .text_color(theme::faint())
                .child(if self.query.trim().is_empty() {
                    "No commands, missions, chats, runners, or crews yet."
                } else {
                    "No matches."
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
                        .font_family("Menlo")
                        .text_size(rems(10. / 16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme::faint())
                        .child("RECENTS")
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
                                    .font_family("Menlo")
                                    .text_size(rems(10. / 16.))
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
            item(PaletteKind::Runner, "runner-1", "runner", 0),
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
                "runner-1",
                "crew-1",
                "settings"
            ]
        );
    }

    #[test]
    fn palette_always_offers_new_terminal_as_a_command() {
        let items = palette_items(&[], &[], &[], &[]);
        assert_eq!(items[0].label, "New terminal");
        assert_eq!(items[0].kind, PaletteKind::Command);
        assert_eq!(items[0].destination, PaletteDestination::NewTerminal);
        assert_eq!(
            filtered_palette_items(&items, "shell")[0].id,
            "new-terminal"
        );
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
}
