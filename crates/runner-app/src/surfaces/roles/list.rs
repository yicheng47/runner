use super::logic::crews_label;
use super::logic::last_active_label;
use super::logic::role_setting_label;
use super::logic::role_table_columns;
use super::logic::runtime_display_name;
use super::logic::RoleColumn;
use super::logic::ROLE_ROW_ACTIONS_WIDTH;
use super::logic::ROLE_ROW_PADDING_X;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, App, Context, CursorStyle, FontWeight, KeyDownEvent,
    MouseButton, SharedString, Window,
};
use runner_app::ui::list::LIST_PAGE_PADDING_X;
use runner_app::ui::{
    Button, ButtonSize, ButtonVariant, EmptyStateCard, IconButton, IconButtonSize,
    PaginatedListPage, RoleAvatar,
};

use crate::chat_icon::ChatIcon;

use super::*;
use crate::list_controls::LIST_QUERY_DEBOUNCE_MS;
use crate::surfaces::*;
use crate::*;

impl NativeRoot {
    pub(crate) fn open_roles(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.remote_window.is_some() {
            self.open_remote_library(
                crate::surfaces::remote_client::RemoteItem::Roles,
                window,
                cx,
            );
            return;
        }

        if self.route != AppRoute::Roles {
            self.role_surfaces.list.reset();
            self.role_surfaces
                .search
                .update(cx, |search, search_cx| search.reset_value("", search_cx));
            self.role_surfaces
                .scroll
                .set_offset(gpui::Point::new(px(0.), px(0.)));
        }
        self.enter_entity_route(AppRoute::Roles, window, cx);
        self.load_role_page(cx);
    }

    pub(crate) fn open_role_detail(
        &mut self,
        handle: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.remote_window.is_some() {
            return;
        }

        self.role_surfaces.prompt_expanded = false;
        self.enter_entity_route(AppRoute::RoleDetail(handle.clone()), window, cx);
        self.load_role_detail(handle, cx);
    }

    pub(crate) fn enter_entity_route(
        &mut self,
        route: AppRoute,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_sidebar_transients(cx);
        self.role_surfaces.context_menu = None;
        self.crew_surfaces.context_menu = None;
        self.set_route(route, cx);
        window.focus(&self.root_focus, cx);
        cx.notify();
    }

    pub(crate) fn set_role_query(&mut self, query: String, cx: &mut Context<Self>) {
        if self.remote_window.is_some() {
            let list = &mut self.role_page_surfaces_mut().list;
            let update = list.set_query(query);
            list.apply_debounced_query(update.generation);
            self.load_role_page(cx);
            return;
        }
        self.set_local_role_query(query, cx);
    }

    pub(super) fn set_local_role_query(&mut self, query: String, cx: &mut Context<Self>) {
        let update = self.role_surfaces.list.set_query(query);
        if update.load_now {
            self.load_local_role_page(cx);
        }
        cx.spawn(async move |weak, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(LIST_QUERY_DEBOUNCE_MS))
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this
                    .role_surfaces
                    .list
                    .apply_debounced_query(update.generation)
                {
                    this.load_local_role_page(cx);
                }
            });
        })
        .detach();
    }

    pub(crate) fn set_role_page(&mut self, page: usize, cx: &mut Context<Self>) {
        if self.role_page_surfaces_mut().list.set_page(page) {
            self.role_page_surfaces()
                .scroll
                .set_offset(gpui::point(px(0.), px(0.)));
            self.load_role_page(cx);
        }
    }

    pub(crate) fn set_role_page_size(&mut self, size: usize, cx: &mut Context<Self>) {
        if self.remote_window.is_some() {
            if self.role_page_surfaces_mut().list.set_page_size(size) {
                self.role_page_surfaces()
                    .scroll
                    .set_offset(gpui::point(px(0.), px(0.)));
                self.load_role_page(cx);
            }
            return;
        }
        self.set_local_role_page_size(size, cx);
    }

    pub(super) fn set_local_role_page_size(&mut self, size: usize, cx: &mut Context<Self>) {
        if self.role_surfaces.list.set_page_size(size) {
            self.update_app_settings(cx, true, |settings| {
                settings.roles_page_size = size;
                true
            });
            self.role_surfaces
                .scroll
                .set_offset(gpui::point(px(0.), px(0.)));
            self.load_local_role_page(cx);
        }
    }

    pub(crate) fn sync_role_page_size(&mut self, cx: &mut Context<Self>) {
        let size = self.settings(cx).roles_page_size;
        if self.role_surfaces.list.set_page_size(size) {
            self.role_surfaces
                .page_size
                .update(cx, |select, cx| select.set_value(size.to_string(), cx));
            self.role_surfaces
                .scroll
                .set_offset(gpui::point(px(0.), px(0.)));
            if self.route == AppRoute::Roles {
                self.load_local_role_page(cx);
            }
        }
    }

    fn role_page_surfaces(&self) -> &RoleSurfaces {
        self.remote_window
            .as_ref()
            .map(|remote| &remote.role_surfaces)
            .unwrap_or(&self.role_surfaces)
    }
    fn role_page_surfaces_mut(&mut self) -> &mut RoleSurfaces {
        self.remote_window
            .as_mut()
            .map(|remote| &mut remote.role_surfaces)
            .unwrap_or(&mut self.role_surfaces)
    }
    pub(crate) fn load_role_page(&mut self, cx: &mut Context<Self>) {
        let window_epoch = self.client_ui.epoch;
        if let Some(remote) = self.remote_window.as_mut() {
            let Some(catalog) = remote.library_catalog.as_ref() else {
                return;
            };
            let list = &mut remote.role_surfaces.list;
            let request = list.begin_load();
            let query = request.query.trim().to_lowercase();
            let rows = catalog
                .roles
                .iter()
                .filter(|role| {
                    query.is_empty()
                        || role.name.to_lowercase().contains(&query)
                        || role.handle.to_lowercase().contains(&query)
                })
                .map(|role| RoleListRow::Remote {
                    role: role.clone(),
                    crew_count: catalog
                        .crews
                        .iter()
                        .filter(|crew| crew.slots.iter().any(|slot| slot.role == role.id))
                        .count(),
                    target: remote.front_page.target.clone(),
                    epoch: window_epoch,
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
                    total_count: catalog.roles.len() as i64,
                    filtered_count,
                },
            ) {
                self.load_role_page(cx);
            }
            cx.notify();
            return;
        }
        self.load_local_role_page(cx);
    }

    pub(crate) fn load_local_role_page(&mut self, cx: &mut Context<Self>) {
        let request = self.role_surfaces.list.begin_load();
        let request_id = request.request_id;
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            core.role_list_with_activity(
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
                        if this.role_surfaces.list.apply_success(
                            request_id,
                            runner_core::protocol::ListPage {
                                items: page.items.into_iter().map(RoleListRow::Local).collect(),
                                total_count: page.total_count,
                                filtered_count: page.filtered_count,
                            },
                        ) {
                            this.load_local_role_page(cx);
                        }
                    }
                    Err(error) => {
                        this.role_surfaces.list.apply_error(request_id, error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn load_role_detail(&mut self, handle: String, cx: &mut Context<Self>) {
        if self.role_surfaces.detail.handle == handle {
            let detail = &mut self.role_surfaces.detail;
            detail.loading = !detail.loaded;
            detail.error = None;
        } else {
            self.role_surfaces.detail = RoleDetailState {
                handle: handle.clone(),
                loading: true,
                ..Default::default()
            };
        }
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            let requested = handle.clone();
            let result = (|| {
                let role = core.role_get_by_handle(&handle)?;
                let activity = core.role_activity(&role.id)?;
                let crews = core.role_crews_list(&role.id)?;
                Ok::<_, runner_core::protocol::ClientError>((role, activity, crews))
            })();
            result
                .map(|(role, activity, crews)| (requested.clone(), role, activity, crews))
                .map_err(|error| (requested, error.to_string()))
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok((handle, role, activity, crews))
                        if matches!(
                            &this.route,
                            AppRoute::RoleDetail(active) if active == &handle
                        ) =>
                    {
                        this.role_surfaces.detail = RoleDetailState {
                            handle,
                            role: Some(role),
                            activity: Some(activity),
                            crews,
                            loaded: true,
                            loading: false,
                            error: None,
                        };
                    }
                    Ok(_) => {}
                    Err((handle, error))
                        if matches!(
                            &this.route,
                            AppRoute::RoleDetail(active) if active == &handle
                        ) =>
                    {
                        let detail = &mut this.role_surfaces.detail;
                        detail.loading = false;
                        if error.to_lowercase().contains("not found") {
                            detail.loaded = true;
                            detail.role = None;
                            detail.activity = None;
                            detail.crews.clear();
                        }
                        detail.error = Some(error);
                    }
                    Err(_) => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn render_entity_surface(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let route = if self.route == AppRoute::Settings {
            self.settings_return_route.clone()
        } else {
            self.route.clone()
        };
        match route {
            AppRoute::Chat => self.render_active_tab(window, cx),
            AppRoute::Roles => self.render_roles_page(window, cx),
            AppRoute::NewRole | AppRoute::RoleDetail(_) => self.render_role_detail(window, cx),
            AppRoute::Crews | AppRoute::NewCrew | AppRoute::CrewEditor(_) => {
                self.render_crew_surface(window, cx)
            }
            AppRoute::Mission(_) => self.mission_workspace.clone().into_any_element(),
            AppRoute::ArchivedChat => self.render_archived_chat(window, cx),
            AppRoute::Settings => self.render_active_tab(window, cx),
        }
    }

    pub(crate) fn render_roles_page(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = cx.entity();
        let create_root = root.clone();
        let empty_create_root = root.clone();
        let clear_root = root.clone();
        let page_root = root.clone();
        let query = self.role_page_surfaces().list.query.clone();
        let clear_client = self.front_page_client(cx);
        let page_client = clear_client.clone();
        let epoch = self.client_ui.epoch;
        let remote = self.remote_window.is_some();
        let columns = if remote {
            super::logic::role_table_columns_with_activity(self.role_table_width(window, cx), false)
        } else {
            role_table_columns(self.role_table_width(window, cx))
        };
        let items = self.role_page_surfaces().list.items.clone();
        let last = items.len().saturating_sub(1);
        let now = chrono::Local::now();
        let rows = items
            .into_iter()
            .enumerate()
            .map(|(index, item)| self.render_role_row(item, &columns, index < last, &now, cx))
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
                    .text_color(theme::text())
                    .child(crate::i18n::tf(
                        "format.no_roles_match",
                        &[("query", (query).to_string())],
                    )),
            )
            .child(
                div()
                    .text_size(theme::text_ui())
                    .line_height(rems(19. / 16.))
                    .text_color(theme::muted())
                    .child(crate::i18n::text("ui.search_checks_handles_and_names")),
            )
            .child(
                Button::new("clear-role-search", crate::i18n::text("ui.clear_search"))
                    .size(ButtonSize::Sm)
                    .on_press(move |_, cx| {
                        clear_root.update(cx, |this, cx| {
                            if !this.library_view_current(&clear_client, epoch, cx) {
                                return;
                            }
                            this.role_page_surfaces()
                                .search
                                .update(cx, |search, search_cx| search.set_value("", search_cx));
                        });
                    }),
            );
        let empty_state = EmptyStateCard::new(
            svg()
                .flex_none()
                .path("user.svg")
                .size(rems(22. / 16.))
                .text_color(theme::accent()),
            crate::i18n::text("ui.no_roles_yet"),
            crate::i18n::text(if remote {
                "client.roles_read_only"
            } else {
                "ui.a_role_is_a_reusable_cli_agent_claude_code_codex_a_custom_shell_that_crews_pull_in_add_one"
            }),
            div().when(!remote, |entry| {
                entry
                    .when(cfg!(test), |entry| {
                        entry.debug_selector(|| "EMPTY_NEW_ROLE".into())
                    })
                    .child(
                        Button::new(
                            "empty-new-role",
                            crate::i18n::text("ui.new_role_inline_action"),
                        )
                        .variant(ButtonVariant::Primary)
                        .on_press(move |window, cx| {
                            empty_create_root
                                .update(cx, |this, cx| this.open_create_role(window, cx));
                        }),
                    )
            }),
        );
        PaginatedListPage::new(
            crate::i18n::text("ui.roles"),
            div().child(crate::i18n::text(if remote {
                "client.roles_read_only"
            } else {
                "ui.the_setups_your_chats_and_crews_run_on_a_runtime_a_model_an_effort_and_a_brief"
            })),
            div()
                .when(remote, |entry| entry.child(self.render_library_refresh(cx)))
                .when(!remote, |entry| {
                    entry
                        .when(cfg!(test), |entry| {
                            entry.debug_selector(|| "NEW_ROLE".into())
                        })
                        .child(
                            Button::new("new-role", crate::i18n::text("ui.new_role_inline_action"))
                                .variant(ButtonVariant::Primary)
                                .on_press(move |window, cx| {
                                    create_root
                                        .update(cx, |this, cx| this.open_create_role(window, cx));
                                }),
                        )
                }),
            crate::i18n::text("noun.roles"),
            empty_state,
            self.role_page_surfaces().search.clone(),
            no_matches,
            self.role_page_surfaces().list.page,
            self.role_page_surfaces().list.page_count(),
            Rc::new(move |page, _, cx| {
                page_root.update(cx, |this, cx| {
                    if this.library_view_current(&page_client, epoch, cx) {
                        this.set_role_page(page, cx);
                    }
                });
            }),
            div()
                .when(cfg!(test), |table| {
                    table.debug_selector(|| "ROLE_TABLE_ROWS".into())
                })
                .w_full()
                .flex()
                .flex_col()
                .flex_none()
                .children(rows),
            self.role_page_surfaces().scroll.clone(),
            self.role_page_surfaces().scrollbar.clone(),
        )
        .header(role_table_header(&columns))
        .page_size(
            self.role_page_surfaces().page_size.clone(),
            self.role_table_width(window, cx) < 480.,
        )
        .counts(
            self.role_page_surfaces().list.filtered_count,
            self.role_page_surfaces().list.total_count,
            self.role_page_surfaces().list.searching(),
        )
        .load_state(
            self.role_page_surfaces().list.loading || self.app_store.read(cx).daemon_disconnected,
            self.role_page_surfaces().list.loaded,
            self.role_page_surfaces()
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
    fn role_table_width(&self, window: &Window, cx: &App) -> f32 {
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

    fn set_role_row_hovered(&mut self, id: &str, hovered: bool, cx: &mut Context<Self>) {
        let next = if hovered {
            Some(id.to_owned())
        } else if self.role_page_surfaces().hovered_row.as_deref() == Some(id) {
            None
        } else {
            return;
        };
        if self.role_page_surfaces().hovered_row != next {
            self.role_page_surfaces_mut().hovered_row = next;
            cx.notify();
        }
    }

    fn start_list_role_chat(
        &mut self,
        row: RoleListRow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match row {
            RoleListRow::Local(item) if self.remote_window.is_none() => {
                self.start_role_chat(item.role, window, cx)
            }
            RoleListRow::Remote {
                role,
                target,
                epoch,
                ..
            } if self.front_page_client(cx).target == target
                && self.client_ui.epoch == epoch
                && self.remote_window.as_ref().is_some_and(|remote| {
                    matches!(
                        remote.active,
                        Some(crate::surfaces::remote_client::RemoteItem::Roles)
                    )
                }) =>
            {
                self.open_remote_role_chat(role.id, window, cx);
            }
            _ => {}
        }
    }

    fn render_role_row(
        &self,
        item: RoleListRow,
        columns: &[RoleColumn],
        divider: bool,
        now: &chrono::DateTime<chrono::Local>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = cx.entity();
        let open_root = root.clone();
        let key_root = root.clone();
        let hover_root = root.clone();
        let chat_root = root.clone();
        let chat_key_root = root.clone();
        let menu_root = root;
        let role = item.display();
        let local = item.local().is_some();
        let id = role.id.to_owned();
        let hover_id = id.clone();
        let hover_client = self.front_page_client(cx);
        let row_selector = id.clone();
        let chat_selector = id.clone();
        let handle = role.handle.to_owned();
        let open_handle = handle.clone();
        let menu_item = item.local().cloned();
        let chat_role = item.clone();
        let chat_key_role = item.clone();
        let pending = self.role_page_surfaces().chat_pending.as_deref() == Some(id.as_str());
        let can_start = local
            || self
                .front_page_client(cx)
                .supports(&["catalog", "startChat"]);
        let chat_button =
            pending || self.role_page_surfaces().hovered_row.as_deref() == Some(id.as_str());
        let chat = div()
            .id(SharedString::from(format!("role-chat-{id}")))
            .when(cfg!(test), |chat| {
                chat.debug_selector(move || format!("ROLE_ROW_CHAT:{chat_selector}"))
            })
            .tab_index(0)
            .tab_stop(!pending)
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .gap(rems(6. / 16.))
            .h(rems(24. / 16.))
            .rounded(rems(4. / 16.))
            .border_1()
            .when(chat_button, |chat| {
                chat.px_2()
                    .border_color(theme::border_strong())
                    .bg(theme::bg())
            })
            .when(!chat_button, |chat| {
                chat.w(rems(24. / 16.))
                    .border_color(gpui::transparent_black())
            })
            .text_size(theme::text_ui())
            .line_height(rems(1.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme::text())
            .opacity(if pending { 0.6 } else { 1. })
            .cursor(if pending {
                CursorStyle::Arrow
            } else {
                CursorStyle::PointingHand
            })
            .focus_visible(|chat| chat.border_color(theme::border_strong()))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                if !pending {
                    let role = chat_role.clone();
                    chat_root.update(cx, |this, cx| this.start_list_role_chat(role, window, cx));
                }
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if !pending && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    let role = chat_key_role.clone();
                    chat_key_root
                        .update(cx, |this, cx| this.start_list_role_chat(role, window, cx));
                }
            })
            .child(
                // The speech-bubble tail pulls the glyph's optical center down.
                svg()
                    .path("message-square.svg")
                    .size(rems(12. / 16.))
                    .relative()
                    .bottom(rems(1. / 16.))
                    .flex_none()
                    .text_color(if chat_button {
                        theme::accent()
                    } else {
                        theme::faint()
                    }),
            )
            .when(chat_button, |chat| {
                chat.child(if pending {
                    crate::i18n::text("ui.starting")
                } else {
                    crate::i18n::text("ui.chat")
                })
            });
        div()
            .id(SharedString::from(format!("role-row-{id}")))
            .when(cfg!(test), |row| {
                row.debug_selector(move || format!("ROLE_ROW:{row_selector}"))
            })
            .tab_index(0)
            .w_full()
            .flex()
            .items_center()
            .px(rems(ROLE_ROW_PADDING_X / 16.))
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
                        this.set_role_row_hovered(&hover_id, *hovered, cx);
                    }
                });
            })
            .when(local, |row| {
                row.on_click(move |_, window, cx| {
                    open_root.update(cx, |this, cx| {
                        this.open_role_detail(open_handle.clone(), window, cx)
                    });
                })
                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                        cx.stop_propagation();
                        let handle = handle.clone();
                        key_root.update(cx, |this, cx| this.open_role_detail(handle, window, cx));
                    }
                })
            })
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .items_center()
                    .gap(rems(10. / 16.))
                    .pr_3()
                    .child(RoleAvatar::new(role.handle.to_owned(), 26.))
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
                                    .child(role.display_name.to_owned()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_meta())
                                    .text_color(theme::faint())
                                    .child(format!("@{}", role.handle)),
                            ),
                    ),
            )
            .children(
                columns
                    .iter()
                    .map(|column| role_table_cell(*column, &item, now)),
            )
            .child(
                div()
                    .when(cfg!(test), |actions| {
                        actions.debug_selector(|| "ROLE_ROW_ACTIONS".into())
                    })
                    .w(rems(ROLE_ROW_ACTIONS_WIDTH / 16.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap_1()
                    .when(can_start, |actions| actions.child(chat))
                    .children(menu_item.map(|menu_item| {
                        IconButton::new(
                            SharedString::from(format!("role-actions-{id}")),
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
                                    this.open_role_menu(item, position, window, cx);
                                }
                            });
                        })
                    })),
            )
            .into_any_element()
    }
}

fn role_table_header(columns: &[RoleColumn]) -> AnyElement {
    div()
        .when(cfg!(test), |header| {
            header.debug_selector(|| "ROLE_TABLE_HEADER".into())
        })
        .w_full()
        .flex()
        .items_center()
        .px(rems(ROLE_ROW_PADDING_X / 16.))
        .pb_2()
        .border_b_1()
        .border_color(theme::border_strong())
        .text_size(theme::text_meta())
        .text_color(theme::faint())
        .child(
            div()
                .flex_1()
                .min_w(px(0.))
                .child(crate::i18n::text("ui.role")),
        )
        .children(columns.iter().map(|column| {
            div()
                .w(rems(column.width() / 16.))
                .flex_none()
                .child(column.label())
        }))
        .child(div().w(rems(ROLE_ROW_ACTIONS_WIDTH / 16.)).flex_none())
        .into_any_element()
}

fn role_table_cell(
    column: RoleColumn,
    item: &RoleListRow,
    now: &chrono::DateTime<chrono::Local>,
) -> AnyElement {
    let role = item.display();
    let cell = div()
        .w(rems(column.width() / 16.))
        .flex_none()
        .min_w(px(0.))
        .pr_3()
        .text_size(theme::text_ui());
    let text = |value: String, monospace: bool, color: gpui::Hsla| {
        div()
            .min_w(px(0.))
            .truncate()
            .text_color(color)
            .when(cfg!(test), |text| {
                let value = value.clone();
                text.debug_selector(move || format!("ROLE_TABLE_CELL {column:?} {value}"))
            })
            .when(monospace, |value| {
                value.font_family(theme::UI_MONOSPACE_FONT)
            })
            .child(value)
    };
    match column {
        RoleColumn::Runtime => {
            let icon = ChatIcon::for_runtime(role.runtime);
            cell.flex()
                .items_center()
                .gap(rems(6. / 16.))
                .child(icon.render(rems(12. / 16.), icon.color(theme::muted(), true), true))
                .child(text(
                    runtime_display_name(role.runtime),
                    false,
                    theme::text(),
                ))
                .into_any_element()
        }
        RoleColumn::Model | RoleColumn::Effort => {
            let (label, unset) = if column == RoleColumn::Model {
                role_setting_label(role.model)
            } else {
                super::logic::role_effort_label(role.effort)
            };
            cell.child(text(
                label,
                !unset,
                if unset { theme::faint() } else { theme::text() },
            ))
            .into_any_element()
        }
        RoleColumn::Crews => {
            let crews = match item {
                RoleListRow::Local(item) => item.activity.crew_count,
                RoleListRow::Remote { crew_count, .. } => *crew_count as i64,
            };
            cell.child(text(
                crews_label(crews),
                false,
                if crews > 0 {
                    theme::text()
                } else {
                    theme::faint()
                },
            ))
            .into_any_element()
        }
        RoleColumn::LastActive => {
            let (label, live) = last_active_label(&item.local().unwrap().activity, now);
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
                .child(text(
                    label,
                    false,
                    if live {
                        theme::accent()
                    } else {
                        theme::faint()
                    },
                ))
                .into_any_element()
        }
    }
}
