use super::logic::crew_missions;
use super::logic::crew_picture;
use super::logic::crew_summary;
use super::logic::crew_table_columns;
use super::logic::last_mission_label;
use super::logic::runtime_counts;
use super::logic::CrewColumn;
use super::logic::CREW_ROW_ACTIONS_WIDTH;
use super::logic::CREW_ROW_PADDING_X;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, App, Context, CursorStyle, FontWeight, KeyDownEvent,
    MouseButton, SharedString, Window,
};
use runner_app::ui::list::LIST_PAGE_PADDING_X;
use runner_app::ui::{
    Button, ButtonSize, ButtonVariant, ContextMenu, EmptyStateCard, IconButton, IconButtonSize,
    MenuItem as UiMenuItem, PaginatedListPage, Tooltip,
};
use runner_backend::model::Mission;
use runner_backend::ops::crew::CrewListItem;

use crate::chat_icon::ChatIcon;

use super::*;
use crate::list_controls::LIST_QUERY_DEBOUNCE_MS;
use crate::surfaces::*;
use crate::*;

impl NativeRoot {
    pub(crate) fn open_crews(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.route != AppRoute::Crews {
            self.crew_surfaces.list.reset();
            self.crew_surfaces
                .search
                .update(cx, |search, search_cx| search.reset_value("", search_cx));
            self.crew_surfaces
                .scroll
                .set_offset(gpui::Point::new(px(0.), px(0.)));
        }
        self.enter_entity_route(AppRoute::Crews, window, cx);
        self.load_crew_page(cx);
    }

    pub(crate) fn open_crew_editor(
        &mut self,
        crew_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = &mut self.crew_surfaces.editor;
        editor.conventions_expanded = false;
        editor.missions_expanded = false;
        editor.popup = None;
        self.enter_entity_route(AppRoute::CrewEditor(crew_id.clone()), window, cx);
        self.load_crew_editor(crew_id, cx);
    }

    pub(super) fn set_crew_query(&mut self, query: String, cx: &mut Context<Self>) {
        let update = self.crew_surfaces.list.set_query(query);
        if update.load_now {
            self.load_crew_page(cx);
        }
        cx.spawn(async move |weak, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(LIST_QUERY_DEBOUNCE_MS))
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this
                    .crew_surfaces
                    .list
                    .apply_debounced_query(update.generation)
                {
                    this.load_crew_page(cx);
                }
            });
        })
        .detach();
    }

    fn set_crew_page(&mut self, page: usize, cx: &mut Context<Self>) {
        if self.crew_surfaces.list.set_page(page) {
            self.load_crew_page(cx);
        }
    }

    pub(crate) fn load_crew_page(&mut self, cx: &mut Context<Self>) {
        let request = self.crew_surfaces.list.begin_load();
        let request_id = request.request_id;
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::crew::crew_list(
                &core,
                request.page as i64,
                request.page_size as i64,
                &request.query,
            )
            .map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok(page) => {
                        if this.crew_surfaces.list.apply_success(request_id, page) {
                            this.load_crew_page(cx);
                        }
                    }
                    Err(error) => {
                        this.crew_surfaces.list.apply_error(request_id, error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn render_crew_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        match self.route {
            AppRoute::Crews => self.render_crews_page(window, cx),
            AppRoute::NewCrew | AppRoute::CrewEditor(_) => self.render_crew_editor(window, cx),
            _ => div().into_any_element(),
        }
    }

    fn render_crews_page(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let root = cx.entity();
        let create_root = root.clone();
        let empty_create_root = root.clone();
        let clear_root = root.clone();
        let page_root = root;
        let query = self.crew_surfaces.list.query.clone();
        let columns = crew_table_columns(self.crew_table_width(window, cx));
        let items = self.crew_surfaces.list.items.clone();
        let last = items.len().saturating_sub(1);
        let now = chrono::Local::now();
        let rows = items
            .into_iter()
            .enumerate()
            .map(|(index, crew)| self.render_crew_row(crew, &columns, index < last, &now, cx))
            .collect::<Vec<_>>();
        let no_matches = div()
            .w_full()
            .flex()
            .flex_col()
            .items_center()
            .gap_3()
            .rounded_lg()
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .px_8()
            .py(rems(56. / 16.))
            .text_center()
            .child(
                svg()
                    .flex_none()
                    .path("search-x.svg")
                    .size(rems(20. / 16.))
                    .text_color(theme::faint()),
            )
            .child(
                div()
                    .text_size(theme::text_title())
                    .font_weight(FontWeight::MEDIUM)
                    .child(format!("No crews match \"{query}\"")),
            )
            .child(
                div()
                    .max_w(rems(480. / 16.))
                    .text_size(theme::text_ui())
                    .line_height(rems(19. / 16.))
                    .text_color(theme::muted())
                    .child("Search checks crew names, team conventions, slot and role handles, and runtimes."),
            )
            .child(
                Button::new("clear-crew-search", "Clear search")
                    .size(ButtonSize::Sm)
                    .on_press(move |_, cx| {
                        clear_root.update(cx, |this, cx| {
                            this.crew_surfaces.search.update(cx, |search, search_cx| {
                                search.set_value("", search_cx)
                            });
                        });
                    }),
            );
        let empty_state = EmptyStateCard::new(
            svg()
                .flex_none()
                .path("users.svg")
                .size(rems(22. / 16.))
                .text_color(theme::accent()),
            "No crews yet",
            "A crew is a team of roles: slots, one lead, and the conventions they share. Make one to start missions from it.",
            div().when(cfg!(test), |entry| entry.debug_selector(|| "EMPTY_NEW_CREW".into())).child(Button::new("empty-new-crew", "+ New crew")
                .variant(ButtonVariant::Primary)
                .on_press(move |window, cx| {
                    empty_create_root.update(cx, |this, cx| {
                        this.open_create_crew(window, cx)
                    });
                })),
        );
        PaginatedListPage::new(
            "Crews",
            div().child(
                "Teams of roles you start missions from: slots, one lead, and the conventions they share.",
            ),
            div().when(cfg!(test), |entry| entry.debug_selector(|| "NEW_CREW".into())).child(Button::new("new-crew", "+ New crew")
                .variant(ButtonVariant::Primary)
                .on_press(move |window, cx| {
                    create_root.update(cx, |this, cx| this.open_create_crew(window, cx));
                })),
            "crews",
            empty_state,
            self.crew_surfaces.search.clone(),
            no_matches,
            self.crew_surfaces.list.page,
            self.crew_surfaces.list.page_count(),
            Rc::new(move |page, _, cx| {
                page_root.update(cx, |this, cx| this.set_crew_page(page, cx));
            }),
            div()
                .when(cfg!(test), |table| {
                    table.debug_selector(|| "CREW_TABLE_ROWS".into())
                })
                .w_full()
                .flex()
                .flex_col()
                .children(rows),
            self.crew_surfaces.scroll.clone(),
            self.crew_surfaces.scrollbar.clone(),
        )
        .header(crew_table_header(&columns))
        .counts(
            self.crew_surfaces.list.filtered_count,
            self.crew_surfaces.list.total_count,
            self.crew_surfaces.list.searching(),
        )
        .load_state(
            self.crew_surfaces.list.loading,
            self.crew_surfaces.list.loaded,
            self.crew_surfaces.list.error.clone().map(Into::into),
        )
        .into_any_element()
    }

    /// The table's width in unzoomed pixels: the window less the sidebar and
    /// the list page's side padding.
    fn crew_table_width(&self, window: &Window, cx: &App) -> f32 {
        let settings = self.settings(cx);
        let sidebar = if self.sidebar_collapsed {
            0.
        } else {
            settings.sidebar_width
        };
        f32::from(window.viewport_size().width) / settings.app_zoom
            - sidebar
            - 2. * LIST_PAGE_PADDING_X
    }

    fn set_crew_row_hovered(&mut self, id: &str, hovered: bool, cx: &mut Context<Self>) {
        let next = if hovered {
            Some(id.to_owned())
        } else if self.crew_surfaces.hovered_row.as_deref() == Some(id) {
            None
        } else {
            return;
        };
        if self.crew_surfaces.hovered_row != next {
            self.crew_surfaces.hovered_row = next;
            cx.notify();
        }
    }

    fn render_crew_row(
        &self,
        item: CrewListItem,
        columns: &[CrewColumn],
        divider: bool,
        now: &chrono::DateTime<chrono::Local>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = cx.entity();
        let open_root = root.clone();
        let key_root = root.clone();
        let hover_root = root.clone();
        let start_root = root.clone();
        let start_key_root = root.clone();
        let menu_root = root;
        let id = item.crew.id.clone();
        let hover_id = id.clone();
        let open_id = id.clone();
        let key_id = id.clone();
        let start_id = id.clone();
        let start_key_id = id.clone();
        let menu_item = item.clone();
        let store = self.app_store.read(cx);
        let missions = crew_missions(&store.missions, &id);
        let handles = item
            .members
            .iter()
            .map(|member| SharedString::from(member.slot_handle.clone()))
            .collect::<Vec<_>>();
        let lead = item
            .members
            .iter()
            .find(|member| member.lead)
            .map(|member| member.slot_handle.as_str());
        let startable = !item.members.is_empty();
        let start_button =
            startable && self.crew_surfaces.hovered_row.as_deref() == Some(id.as_str());
        let start = div()
            .id(SharedString::from(format!("crew-start-{id}")))
            .when(cfg!(test), |start| {
                start.debug_selector(|| "CREW_ROW_START".into())
            })
            .tab_index(0)
            .tab_stop(startable)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .gap(rems(6. / 16.))
            .h(rems(24. / 16.))
            .rounded(rems(4. / 16.))
            .border_1()
            .when(start_button, |start| {
                start
                    .px_2()
                    .border_color(theme::border_strong())
                    .bg(theme::bg())
            })
            .when(!start_button, |start| {
                start
                    .w(rems(24. / 16.))
                    .border_color(gpui::transparent_black())
            })
            .text_size(theme::text_ui())
            .line_height(rems(1.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme::text())
            .opacity(if startable { 1. } else { 0.4 })
            .cursor(if startable {
                CursorStyle::PointingHand
            } else {
                CursorStyle::Arrow
            })
            .focus_visible(|start| start.border_color(theme::border_strong()))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                if startable {
                    let crew_id = start_id.clone();
                    start_root.update(cx, |this, cx| this.start_crew_mission(crew_id, window, cx));
                }
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if startable && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    let crew_id = start_key_id.clone();
                    start_key_root
                        .update(cx, |this, cx| this.start_crew_mission(crew_id, window, cx));
                }
            })
            .child(
                svg()
                    .path("play.svg")
                    .size(rems(12. / 16.))
                    .flex_none()
                    .text_color(if start_button {
                        theme::accent()
                    } else {
                        theme::faint()
                    }),
            )
            .when(start_button, |start| start.child("Start mission"));
        div()
            .id(SharedString::from(format!("crew-row-{id}")))
            .tab_index(0)
            .w_full()
            .flex()
            .items_center()
            .px(rems(CREW_ROW_PADDING_X / 16.))
            .py(rems(9. / 16.))
            .when(divider, |row| {
                row.border_b_1().border_color(theme::border())
            })
            .cursor_pointer()
            .hover(|row| row.bg(theme::panel()))
            .focus_visible(|row| row.bg(theme::panel()))
            .on_hover(move |hovered, _, cx| {
                hover_root.update(cx, |this, cx| {
                    this.set_crew_row_hovered(&hover_id, *hovered, cx)
                });
            })
            .on_click(move |_, window, cx| {
                let crew_id = open_id.clone();
                open_root.update(cx, |this, cx| this.open_crew_editor(crew_id, window, cx));
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    let crew_id = key_id.clone();
                    key_root.update(cx, |this, cx| this.open_crew_editor(crew_id, window, cx));
                }
            })
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .items_center()
                    .gap(rems(12. / 16.))
                    .pr_3()
                    .child(
                        div()
                            .w(rems(LIST_PICTURE_SIZE / 16.))
                            .flex_none()
                            .child(crew_picture(&handles, LIST_PICTURE_SIZE)),
                    )
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .truncate()
                                    .text_size(theme::text_body())
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::text())
                                    .child(item.crew.name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_meta())
                                    .text_color(theme::faint())
                                    .child(crew_summary(item.members.len(), lead)),
                            ),
                    ),
            )
            .children(
                columns
                    .iter()
                    .map(|column| crew_table_cell(*column, &item, &missions, now)),
            )
            .child(
                div()
                    .when(cfg!(test), |actions| {
                        actions.debug_selector(|| "CREW_ROW_ACTIONS".into())
                    })
                    .w(rems(CREW_ROW_ACTIONS_WIDTH / 16.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    // The hovered button names itself; only an empty crew's
                    // disabled one needs a word.
                    .child(if startable {
                        start.into_any_element()
                    } else {
                        Tooltip::new(
                            SharedString::from(format!("crew-start-tooltip-{id}")),
                            "Add a slot before starting a mission",
                            start,
                        )
                        .into_any_element()
                    })
                    .child(
                        IconButton::new(
                            SharedString::from(format!("crew-actions-{id}")),
                            "more-horizontal.svg",
                        )
                        .size(IconButtonSize::Sm)
                        .stop_click_propagation(true)
                        .tooltip("More actions")
                        .on_press(move |window, cx| {
                            let position = window.mouse_position();
                            let item = menu_item.clone();
                            menu_root.update(cx, |this, cx| {
                                this.open_crew_menu(item, position, window, cx)
                            });
                        }),
                    ),
            )
            .into_any_element()
    }

    fn start_crew_mission(&mut self, crew_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.open_start_mission_modal(
            Some(crew_id),
            runner_backend::ops::project::ProjectScope::Root,
            window,
            cx,
        );
    }

    fn open_crew_menu(
        &mut self,
        item: CrewListItem,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let actions = [
            CrewMenuAction::Open(item.crew.id.clone()),
            CrewMenuAction::Delete {
                id: item.crew.id,
                name: item.crew.name,
            },
        ];
        let items = vec![
            UiMenuItem::new("Open"),
            UiMenuItem::new("Delete")
                .icon("trash.svg")
                .destructive(true),
        ];
        let root = cx.entity();
        let dismiss_root = root.clone();
        let menu = cx.new(move |menu_cx| {
            let action_root = root;
            ContextMenu::new(
                "crew-context-menu",
                menu_cx.focus_handle(),
                position,
                items,
                Rc::new(move |index, window, cx| {
                    if let Some(action) = actions.get(index).cloned() {
                        action_root.update(cx, |this, cx| {
                            this.handle_crew_menu_action(action, window, cx)
                        });
                    }
                }),
                Rc::new(move |_, cx| {
                    dismiss_root.update(cx, |this, cx| {
                        this.crew_surfaces.context_menu = None;
                        cx.notify();
                    });
                }),
            )
            .width(px(160.))
        });
        let focus = menu.read(cx).focus_handle();
        self.crew_surfaces.context_menu = Some(menu);
        focus.focus(window, cx);
        cx.notify();
    }

    fn handle_crew_menu_action(
        &mut self,
        action: CrewMenuAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            CrewMenuAction::Open(id) => self.open_crew_editor(id, window, cx),
            CrewMenuAction::Delete { id, name } => {
                self.crew_surfaces.delete_confirm = Some(CrewDeleteConfirm { id, name });
                cx.notify();
            }
        }
    }
}

/// The crew picture in a list row.
const LIST_PICTURE_SIZE: f32 = 25.;

fn crew_table_header(columns: &[CrewColumn]) -> AnyElement {
    div()
        .when(cfg!(test), |header| {
            header.debug_selector(|| "CREW_TABLE_HEADER".into())
        })
        .w_full()
        .flex()
        .items_center()
        .px(rems(CREW_ROW_PADDING_X / 16.))
        .pb_2()
        .border_b_1()
        .border_color(theme::border_strong())
        .text_size(theme::text_meta())
        .text_color(theme::faint())
        .child(div().flex_1().min_w(px(0.)).child("Crew"))
        .children(columns.iter().map(|column| {
            div()
                .w(rems(column.width() / 16.))
                .flex_none()
                .child(column.label())
        }))
        .child(div().w(rems(CREW_ROW_ACTIONS_WIDTH / 16.)).flex_none())
        .into_any_element()
}

fn crew_table_cell(
    column: CrewColumn,
    item: &CrewListItem,
    missions: &[&Mission],
    now: &chrono::DateTime<chrono::Local>,
) -> AnyElement {
    let cell = div()
        .w(rems(column.width() / 16.))
        .flex_none()
        .min_w(px(0.))
        .pr_3()
        .text_size(theme::text_ui());
    match column {
        CrewColumn::Runtimes => cell
            .flex()
            .items_center()
            .gap_3()
            .overflow_hidden()
            .children(
                runtime_counts(&item.members)
                    .into_iter()
                    .map(|(runtime, count)| {
                        let icon = ChatIcon::for_runtime(&runtime);
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(icon.render(
                                rems(12. / 16.),
                                icon.color(theme::muted(), true),
                                true,
                            ))
                            .child(div().text_color(theme::muted()).child(count.to_string()))
                    }),
            )
            .into_any_element(),
        CrewColumn::Missions => cell
            .text_color(if missions.is_empty() {
                theme::faint()
            } else {
                theme::text()
            })
            .child(if missions.is_empty() {
                "—".to_owned()
            } else {
                missions.len().to_string()
            })
            .into_any_element(),
        CrewColumn::LastMission => {
            let (label, live) = last_mission_label(missions, now);
            cell.flex()
                .items_center()
                .gap(rems(6. / 16.))
                .children(live.then(|| {
                    div()
                        .flex_none()
                        .size(rems(6. / 16.))
                        .rounded_full()
                        .bg(theme::accent())
                }))
                .child(
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .text_color(if live {
                            theme::accent()
                        } else {
                            theme::faint()
                        })
                        .child(label),
                )
                .into_any_element()
        }
    }
}
