use super::logic::crew_missions;
use super::logic::crew_picture;
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
use runner_core::protocol::crew::CrewListItem;
use runner_core::protocol::mission::MissionSummary;

use crate::chat_icon::ChatIcon;

use super::*;
use crate::list_controls::LIST_QUERY_DEBOUNCE_MS;
use crate::surfaces::*;
use crate::*;

impl NativeRoot {
    pub(crate) fn open_crews(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.remote_window.is_some() {
            self.open_remote_library(
                crate::surfaces::remote_client::RemoteItem::Crews,
                window,
                cx,
            );
            return;
        }

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
        if self.remote_window.is_some() {
            return;
        }

        let editor = &mut self.crew_surfaces.editor;
        editor.conventions_expanded = false;
        editor.missions_expanded = false;
        editor.popup = None;
        self.enter_entity_route(AppRoute::CrewEditor(crew_id.clone()), window, cx);
        self.load_crew_editor(crew_id, cx);
    }

    pub(crate) fn set_crew_query(&mut self, query: String, cx: &mut Context<Self>) {
        if self.remote_window.is_some() {
            let list = &mut self.crew_page_surfaces_mut().list;
            let update = list.set_query(query);
            list.apply_debounced_query(update.generation);
            self.load_crew_page(cx);
            return;
        }
        self.set_local_crew_query(query, cx);
    }

    pub(super) fn set_local_crew_query(&mut self, query: String, cx: &mut Context<Self>) {
        let update = self.crew_surfaces.list.set_query(query);
        if update.load_now {
            self.load_local_crew_page(cx);
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
                    this.load_local_crew_page(cx);
                }
            });
        })
        .detach();
    }

    pub(crate) fn set_crew_page(&mut self, page: usize, cx: &mut Context<Self>) {
        if self.crew_page_surfaces_mut().list.set_page(page) {
            self.crew_page_surfaces()
                .scroll
                .set_offset(gpui::point(px(0.), px(0.)));
            self.load_crew_page(cx);
        }
    }

    pub(crate) fn set_crew_page_size(&mut self, size: usize, cx: &mut Context<Self>) {
        if self.remote_window.is_some() {
            if self.crew_page_surfaces_mut().list.set_page_size(size) {
                self.crew_page_surfaces()
                    .scroll
                    .set_offset(gpui::point(px(0.), px(0.)));
                self.load_crew_page(cx);
            }
            return;
        }
        self.set_local_crew_page_size(size, cx);
    }

    pub(super) fn set_local_crew_page_size(&mut self, size: usize, cx: &mut Context<Self>) {
        if self.crew_surfaces.list.set_page_size(size) {
            self.update_app_settings(cx, true, |settings| {
                settings.crews_page_size = size;
                true
            });
            self.crew_surfaces
                .scroll
                .set_offset(gpui::point(px(0.), px(0.)));
            self.load_local_crew_page(cx);
        }
    }

    pub(crate) fn sync_crew_page_size(&mut self, cx: &mut Context<Self>) {
        let size = self.settings(cx).crews_page_size;
        if self.crew_surfaces.list.set_page_size(size) {
            self.crew_surfaces
                .page_size
                .update(cx, |select, cx| select.set_value(size.to_string(), cx));
            self.crew_surfaces
                .scroll
                .set_offset(gpui::point(px(0.), px(0.)));
            if self.route == AppRoute::Crews {
                self.load_local_crew_page(cx);
            }
        }
    }

    fn crew_page_surfaces(&self) -> &CrewSurfaces {
        self.remote_window
            .as_ref()
            .map(|remote| &remote.crew_surfaces)
            .unwrap_or(&self.crew_surfaces)
    }
    fn crew_page_surfaces_mut(&mut self) -> &mut CrewSurfaces {
        self.remote_window
            .as_mut()
            .map(|remote| &mut remote.crew_surfaces)
            .unwrap_or(&mut self.crew_surfaces)
    }
    pub(crate) fn load_crew_page(&mut self, cx: &mut Context<Self>) {
        let window_epoch = self.client_ui.epoch;
        if let Some(remote) = self.remote_window.as_mut() {
            let Some(catalog) = remote.library_catalog.as_ref() else {
                return;
            };
            let list = &mut remote.crew_surfaces.list;
            let request = list.begin_load();
            let query = request.query.trim().to_lowercase();
            let rows = catalog
                .crews
                .iter()
                .filter(|crew| {
                    query.is_empty()
                        || crew.name.to_lowercase().contains(&query)
                        || crew.slots.iter().any(|slot| {
                            slot.handle.to_lowercase().contains(&query)
                                || catalog
                                    .roles
                                    .iter()
                                    .find(|role| role.id == slot.role)
                                    .is_some_and(|role| role.handle.to_lowercase().contains(&query))
                        })
                })
                .map(|crew| {
                    let mut runtimes: Vec<(String, usize)> = Vec::new();
                    for slot in &crew.slots {
                        if let Some(role) = catalog.roles.iter().find(|role| role.id == slot.role) {
                            if let Some((_, count)) = runtimes
                                .iter_mut()
                                .find(|(runtime, _)| runtime == &role.runtime)
                            {
                                *count += 1;
                            } else {
                                runtimes.push((role.runtime.clone(), 1));
                            }
                        }
                    }
                    CrewListRow::Remote {
                        role_handles: crew
                            .slots
                            .iter()
                            .map(|slot| {
                                catalog
                                    .roles
                                    .iter()
                                    .find(|role| role.id == slot.role)
                                    .map(|role| role.handle.clone())
                            })
                            .collect(),
                        crew: crew.clone(),
                        runtimes,
                        target: remote.front_page.target.clone(),
                        epoch: window_epoch,
                    }
                })
                .collect::<Vec<_>>();
            let filtered_count = rows.len() as i64;
            let items = rows
                .into_iter()
                .skip((request.page - 1) * request.page_size)
                .take(request.page_size)
                .collect();
            if list.apply_success(
                request.request_id,
                runner_core::protocol::ListPage {
                    items,
                    total_count: catalog.crews.len() as i64,
                    filtered_count,
                },
            ) {
                self.load_crew_page(cx);
            }
            cx.notify();
            return;
        }
        self.load_local_crew_page(cx);
    }

    pub(crate) fn load_local_crew_page(&mut self, cx: &mut Context<Self>) {
        let request = self.crew_surfaces.list.begin_load();
        let request_id = request.request_id;
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            core.crew_list(
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
                        if this.crew_surfaces.list.apply_success(
                            request_id,
                            runner_core::protocol::ListPage {
                                items: page.items.into_iter().map(CrewListRow::Local).collect(),
                                total_count: page.total_count,
                                filtered_count: page.filtered_count,
                            },
                        ) {
                            this.load_local_crew_page(cx);
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

    pub(crate) fn render_crews_page(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = cx.entity();
        let create_root = root.clone();
        let empty_create_root = root.clone();
        let clear_root = root.clone();
        let page_root = root;
        let query = self.crew_page_surfaces().list.query.clone();
        let clear_client = self.front_page_client(cx);
        let page_client = clear_client.clone();
        let epoch = self.client_ui.epoch;
        let remote = self.remote_window.is_some();
        let columns = if remote {
            super::logic::crew_table_columns_with_history(self.crew_table_width(window, cx), false)
        } else {
            crew_table_columns(self.crew_table_width(window, cx))
        };
        let items = self.crew_page_surfaces().list.items.clone();
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
                    .child(crate::i18n::tf("format.no_crews_match", &[("query", (query).to_string())])),
            )
            .child(
                div()
                    .max_w(rems(480. / 16.))
                    .text_size(theme::text_ui())
                    .line_height(rems(19. / 16.))
                    .text_color(theme::muted())
                    .child(crate::i18n::text(if remote { "client.crews_search" } else { "ui.search_checks_crew_names_team_conventions_slot_and_role_handles_and_runtimes" })),
            )
            .child(
                Button::new("clear-crew-search", crate::i18n::text("ui.clear_search"))
                    .size(ButtonSize::Sm)
                    .on_press(move |_, cx| {
                        clear_root.update(cx, |this, cx| {
                            if !this.library_view_current(&clear_client, epoch, cx) { return; }
                            this.crew_page_surfaces().search.update(cx, |search, search_cx| {
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
            crate::i18n::text("ui.no_crews_yet"),
            crate::i18n::text(if remote {
                "client.crews_read_only"
            } else {
                "ui.a_crew_is_a_team_of_roles_slots_one_lead_and_the_conventions_they_share_make_one_to_start"
            }),
            div().when(!remote, |entry| {
                entry
                    .when(cfg!(test), |entry| {
                        entry.debug_selector(|| "EMPTY_NEW_CREW".into())
                    })
                    .child(
                        Button::new(
                            "empty-new-crew",
                            crate::i18n::text("ui.new_crew_inline_action"),
                        )
                        .variant(ButtonVariant::Primary)
                        .on_press(move |window, cx| {
                            empty_create_root
                                .update(cx, |this, cx| this.open_create_crew(window, cx));
                        }),
                    )
            }),
        );
        PaginatedListPage::new(
            crate::i18n::text("ui.crews"),
            div().child(
                crate::i18n::text(if remote { "client.crews_read_only" } else { "ui.teams_of_roles_you_start_missions_from_slots_one_lead_and_the_conventions_they_share" }),
            ),
            div().when(remote, |entry| entry.child(self.render_library_refresh(cx))).when(!remote, |entry| entry.when(cfg!(test), |entry| entry.debug_selector(|| "NEW_CREW".into())).child(Button::new("new-crew", crate::i18n::text("ui.new_crew_inline_action"))
                .variant(ButtonVariant::Primary)
                .on_press(move |window, cx| {
                    create_root.update(cx, |this, cx| this.open_create_crew(window, cx));
                }))),
            crate::i18n::text("noun.crews"),
            empty_state,
            self.crew_page_surfaces().search.clone(),
            no_matches,
            self.crew_page_surfaces().list.page,
            self.crew_page_surfaces().list.page_count(),
            Rc::new(move |page, _, cx| {
                page_root.update(cx, |this, cx| if this.library_view_current(&page_client, epoch, cx) { this.set_crew_page(page, cx); });
            }),
            div()
                .when(cfg!(test), |table| {
                    table.debug_selector(|| "CREW_TABLE_ROWS".into())
                })
                .w_full()
                .flex()
                .flex_col()
                .flex_none()
                .children(rows),
            self.crew_page_surfaces().scroll.clone(),
            self.crew_page_surfaces().scrollbar.clone(),
        )
        .header(crew_table_header(&columns))
        .page_size(self.crew_page_surfaces().page_size.clone(), self.crew_table_width(window, cx) < 480.)
        .counts(
            self.crew_page_surfaces().list.filtered_count,
            self.crew_page_surfaces().list.total_count,
            self.crew_page_surfaces().list.searching(),
        )
        .load_state(
            self.crew_page_surfaces().list.loading || self.app_store.read(cx).daemon_disconnected,
            self.crew_page_surfaces().list.loaded,
            self.crew_page_surfaces()
                .list
                .error
                .clone()
                .filter(|_| !self.app_store.read(cx).daemon_disconnected)
                .map(Into::into),
        )
        .into_any_element()
    }

    /// The table's width in unzoomed pixels: the window less the sidebar and
    /// the list page's side padding.
    fn crew_table_width(&self, window: &Window, cx: &App) -> f32 {
        let settings = self.settings(cx);
        let sidebar = if let Some(remote) = &self.remote_window {
            if remote.sidebar_collapsed {
                0.
            } else {
                remote.sidebar_width
            }
        } else if self.sidebar_collapsed {
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
        } else if self.crew_page_surfaces().hovered_row.as_deref() == Some(id) {
            None
        } else {
            return;
        };
        if self.crew_page_surfaces().hovered_row != next {
            self.crew_page_surfaces_mut().hovered_row = next;
            cx.notify();
        }
    }

    fn render_crew_row(
        &self,
        item: CrewListRow,
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
        let local = item.local().is_some();
        let id = item.id().to_owned();
        let hover_id = id.clone();
        let hover_client = self.front_page_client(cx);
        let row_selector = id.clone();
        let start_selector = id.clone();
        let open_id = id.clone();
        let key_id = id.clone();
        let menu_item = item.local().cloned();
        let store = self.app_store.read(cx);
        let missions = if local {
            crew_missions(&store.missions, &id)
        } else {
            Vec::new()
        };
        let handles = item.handles();
        let can_start = local
            || self
                .front_page_client(cx)
                .supports(&["catalog", "startMission"]);
        let startable = !handles.is_empty();
        let start_item = item.clone();
        let start_key_item = item.clone();
        let start_button =
            startable && self.crew_page_surfaces().hovered_row.as_deref() == Some(id.as_str());
        let start = div()
            .id(SharedString::from(format!("crew-start-{id}")))
            .when(cfg!(test), |start| {
                start.debug_selector(move || format!("CREW_ROW_START:{start_selector}"))
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
                    let row = start_item.clone();
                    start_root.update(cx, |this, cx| this.start_list_crew_mission(row, window, cx));
                }
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if startable && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    let row = start_key_item.clone();
                    start_key_root
                        .update(cx, |this, cx| this.start_list_crew_mission(row, window, cx));
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
            .when(start_button, |start| {
                start.child(crate::i18n::text("ui.start_mission"))
            });
        div()
            .id(SharedString::from(format!("crew-row-{id}")))
            .when(cfg!(test), |row| {
                row.debug_selector(move || format!("CREW_ROW:{row_selector}"))
            })
            .tab_index(0)
            .w_full()
            .flex()
            .items_center()
            .px(rems(CREW_ROW_PADDING_X / 16.))
            .py(rems(9. / 16.))
            .when(divider, |row| {
                row.border_b_1().border_color(theme::border())
            })
            .when(local, |row| row.cursor_pointer())
            .tab_stop(local)
            .hover(|row| row.bg(theme::panel()))
            .focus_visible(|row| row.bg(theme::panel()))
            .on_hover(move |hovered, _, cx| {
                hover_root.update(cx, |this, cx| {
                    if this.front_page_client(cx).target == hover_client.target
                        && (!hover_client.is_remote() || hover_client.valid())
                    {
                        this.set_crew_row_hovered(&hover_id, *hovered, cx);
                    }
                });
            })
            .when(local, |row| {
                row.on_click(move |_, window, cx| {
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
                                    .child(item.name().to_owned()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_meta())
                                    .text_color(theme::faint())
                                    .child(item.summary()),
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
                    .when(can_start, |actions| {
                        actions.child(if startable {
                            start.into_any_element()
                        } else {
                            Tooltip::new(
                                SharedString::from(format!("crew-start-tooltip-{id}")),
                                crate::i18n::text("ui.add_a_slot_before_starting_a_mission"),
                                start,
                            )
                            .into_any_element()
                        })
                    })
                    .children(menu_item.map(|menu_item| {
                        IconButton::new(
                            SharedString::from(format!("crew-actions-{id}")),
                            "more-horizontal.svg",
                        )
                        .size(IconButtonSize::Sm)
                        .stop_click_propagation(true)
                        .tooltip(crate::i18n::text("ui.more_actions"))
                        .on_press(move |window, cx| {
                            let position = window.mouse_position();
                            let item = menu_item.clone();
                            menu_root.update(cx, |this, cx| {
                                if this.remote_window.is_none() {
                                    this.open_crew_menu(item, position, window, cx);
                                }
                            });
                        })
                    })),
            )
            .into_any_element()
    }

    fn start_list_crew_mission(
        &mut self,
        row: CrewListRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match row {
            CrewListRow::Local(item) if self.remote_window.is_none() => {
                self.start_crew_mission(item.crew.id, window, cx)
            }
            CrewListRow::Remote {
                crew,
                target,
                epoch,
                ..
            } if self.front_page_client(cx).target == target
                && self.client_ui.epoch == epoch
                && self.remote_window.as_ref().is_some_and(|remote| {
                    matches!(
                        remote.active,
                        Some(crate::surfaces::remote_client::RemoteItem::Crews)
                    )
                }) =>
            {
                self.open_start_mission_modal(
                    Some(crew.id),
                    runner_core::protocol::project::ProjectScope::Root,
                    window,
                    cx,
                );
            }
            _ => {}
        }
    }

    fn start_crew_mission(&mut self, crew_id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.open_start_mission_modal(
            Some(crew_id),
            runner_core::protocol::project::ProjectScope::Root,
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
            UiMenuItem::new(crate::i18n::message("ui.open")),
            UiMenuItem::new(crate::i18n::message("ui.delete"))
                .icon("trash.svg")
                .destructive(true),
        ];
        let root = cx.entity();
        let dismiss_root = root.clone();
        let return_focus = window.focused(cx);
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
            .return_focus(return_focus)
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
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .child(crate::i18n::text("ui.crew")),
        )
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
    item: &CrewListRow,
    missions: &[&MissionSummary],
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
                match item {
                    CrewListRow::Local(item) => runtime_counts(&item.members),
                    CrewListRow::Remote { runtimes, .. } => runtimes.clone(),
                }
                .into_iter()
                .map(|(runtime, count)| {
                    let icon = ChatIcon::for_runtime(&runtime);
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(icon.render(rems(12. / 16.), icon.color(theme::muted(), true), true))
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
