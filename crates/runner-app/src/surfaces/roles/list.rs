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
    PaginatedListPage, RoleAvatar, Tooltip,
};
use runner_backend::ops::role::RoleWithActivity;

use crate::chat_icon::ChatIcon;

use super::*;
use crate::list_controls::LIST_QUERY_DEBOUNCE_MS;
use crate::surfaces::*;
use crate::*;

impl NativeRoot {
    pub(crate) fn open_roles(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
        window.focus(&self.root_focus);
        cx.notify();
    }

    pub(super) fn set_role_query(&mut self, query: String, cx: &mut Context<Self>) {
        let update = self.role_surfaces.list.set_query(query);
        if update.load_now {
            self.load_role_page(cx);
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
                    this.load_role_page(cx);
                }
            });
        })
        .detach();
    }

    fn set_role_page(&mut self, page: usize, cx: &mut Context<Self>) {
        if self.role_surfaces.list.set_page(page) {
            self.load_role_page(cx);
        }
    }

    pub(crate) fn load_role_page(&mut self, cx: &mut Context<Self>) {
        let request = self.role_surfaces.list.begin_load();
        let request_id = request.request_id;
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::role::role_list_with_activity(
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
                        if this.role_surfaces.list.apply_success(request_id, page) {
                            this.load_role_page(cx);
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
                let role = runner_backend::ops::role::role_get_by_handle(&core, &handle)?;
                let activity = runner_backend::ops::role::role_activity(&core, &role.id)?;
                let crews = runner_backend::ops::slot::role_crews_list(&core, &role.id)?;
                Ok::<_, runner_backend::error::Error>((role, activity, crews))
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
            AppRoute::RoleDetail(_) => self.render_role_detail(cx),
            AppRoute::Crews | AppRoute::CrewEditor(_) => self.render_crew_surface(window, cx),
            AppRoute::Mission(_) => self.mission_workspace.clone().into_any_element(),
            AppRoute::ArchivedChat => self.render_archived_chat(window, cx),
            AppRoute::Settings => self.render_active_tab(window, cx),
        }
    }

    fn render_roles_page(&mut self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let root = cx.entity();
        let create_root = root.clone();
        let empty_create_root = root.clone();
        let clear_root = root.clone();
        let page_root = root.clone();
        let query = self.role_surfaces.list.query.clone();
        let columns = role_table_columns(self.role_table_width(window, cx));
        let items = self.role_surfaces.list.items.clone();
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
                    .child(format!("No roles match \"{query}\"")),
            )
            .child(
                div()
                    .text_size(theme::text_ui())
                    .line_height(rems(19. / 16.))
                    .text_color(theme::muted())
                    .child("Search checks handles and names."),
            )
            .child(
                Button::new("clear-role-search", "Clear search")
                    .size(ButtonSize::Sm)
                    .on_press(move |_, cx| {
                        clear_root.update(cx, |this, cx| {
                            this.role_surfaces
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
            "No roles yet",
            "A role is a reusable CLI agent — claude-code, codex, a custom shell — that crews pull in. Add one to start composing crews.",
            Button::new("empty-new-role", "+ New role")
                .variant(ButtonVariant::Primary)
                .on_press(move |window, cx| {
                    empty_create_root.update(cx, |this, cx| {
                        this.open_create_role(window, cx)
                    });
                }),
        );
        PaginatedListPage::new(
            "Roles",
            div().child(
                "The setups your chats and crews run on: a runtime, a model, an effort and a brief.",
            ),
            Button::new("new-role", "+ New role")
                .variant(ButtonVariant::Primary)
                .on_press(move |window, cx| {
                    create_root.update(cx, |this, cx| this.open_create_role(window, cx));
                }),
            "roles",
            empty_state,
            self.role_surfaces.search.clone(),
            no_matches,
            self.role_surfaces.list.page,
            self.role_surfaces.list.page_count(),
            Rc::new(move |page, _, cx| {
                page_root.update(cx, |this, cx| this.set_role_page(page, cx));
            }),
            div()
                .when(cfg!(test), |table| {
                    table.debug_selector(|| "ROLE_TABLE_ROWS".into())
                })
                .w_full()
                .flex()
                .flex_col()
                .children(rows),
            self.role_surfaces.scroll.clone(),
            self.role_surfaces.scrollbar.clone(),
        )
        .header(role_table_header(&columns))
        .counts(
            self.role_surfaces.list.filtered_count,
            self.role_surfaces.list.total_count,
            self.role_surfaces.list.searching(),
        )
        .load_state(
            self.role_surfaces.list.loading,
            self.role_surfaces.list.loaded,
            self.role_surfaces.list.error.clone().map(Into::into),
        )
        .into_any_element()
    }

    /// The table's width in unzoomed pixels: the window less the sidebar and
    /// the list page's side padding.
    fn role_table_width(&self, window: &Window, cx: &App) -> f32 {
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

    fn set_role_row_hovered(&mut self, id: &str, hovered: bool, cx: &mut Context<Self>) {
        let next = if hovered {
            Some(id.to_owned())
        } else if self.role_surfaces.hovered_row.as_deref() == Some(id) {
            None
        } else {
            return;
        };
        if self.role_surfaces.hovered_row != next {
            self.role_surfaces.hovered_row = next;
            cx.notify();
        }
    }

    fn render_role_row(
        &self,
        item: RoleWithActivity,
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
        let id = item.role.id.clone();
        let hover_id = id.clone();
        let handle = item.role.handle.clone();
        let open_handle = handle.clone();
        let menu_item = item.clone();
        let chat_role = item.role.clone();
        let chat_key_role = item.role.clone();
        let pending = self.role_surfaces.chat_pending.as_deref() == Some(id.as_str());
        let chat_button = pending || self.role_surfaces.hovered_row.as_deref() == Some(id.as_str());
        let chat = div()
            .id(SharedString::from(format!("role-chat-{id}")))
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
                    chat_root.update(cx, |this, cx| this.start_role_chat(role, window, cx));
                }
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if !pending && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    let role = chat_key_role.clone();
                    chat_key_root.update(cx, |this, cx| this.start_role_chat(role, window, cx));
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
                chat.child(if pending { "Starting…" } else { "Chat" })
            });
        div()
            .id(SharedString::from(format!("role-row-{id}")))
            .tab_index(0)
            .w_full()
            .flex()
            .items_center()
            .px(rems(ROLE_ROW_PADDING_X / 16.))
            .py(rems(9. / 16.))
            .when(divider, |row| {
                row.border_b_1().border_color(theme::border())
            })
            .cursor_pointer()
            .hover(|row| row.bg(theme::panel()))
            .focus_visible(|row| row.bg(theme::panel()))
            .on_hover(move |hovered, _, cx| {
                hover_root.update(cx, |this, cx| {
                    this.set_role_row_hovered(&hover_id, *hovered, cx)
                });
            })
            .on_click(move |_, window, cx| {
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
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.))
                    .flex()
                    .items_center()
                    .gap(rems(10. / 16.))
                    .pr_3()
                    .child(RoleAvatar::new(item.role.handle.clone(), 26.))
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
                                    .child(item.role.display_name.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_meta())
                                    .text_color(theme::faint())
                                    .child(format!("@{}", item.role.handle)),
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
                    .child(Tooltip::new(
                        SharedString::from(format!("role-chat-tooltip-{id}")),
                        "Start a new chat",
                        chat,
                    ))
                    .child(
                        IconButton::new(
                            SharedString::from(format!("role-actions-{id}")),
                            "more-horizontal.svg",
                        )
                        .size(IconButtonSize::Sm)
                        .stop_click_propagation(true)
                        .tooltip("More actions")
                        .on_press(move |window, cx| {
                            let position = window.mouse_position();
                            let item = menu_item.clone();
                            menu_root.update(cx, |this, cx| {
                                this.open_role_menu(item, position, window, cx)
                            });
                        }),
                    ),
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
        .child(div().flex_1().min_w(px(0.)).child("Role"))
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
    item: &RoleWithActivity,
    now: &chrono::DateTime<chrono::Local>,
) -> AnyElement {
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
            .when(monospace, |value| {
                value.font_family(theme::UI_MONOSPACE_FONT)
            })
            .child(value)
    };
    match column {
        RoleColumn::Runtime => {
            let icon = ChatIcon::for_runtime(&item.role.runtime);
            cell.flex()
                .items_center()
                .gap(rems(6. / 16.))
                .child(
                    svg()
                        .flex_none()
                        .path(icon.path)
                        .size(rems(12. / 16.))
                        .text_color(icon.color(theme::muted(), true)),
                )
                .child(text(
                    runtime_display_name(&item.role.runtime),
                    false,
                    theme::text(),
                ))
                .into_any_element()
        }
        RoleColumn::Model | RoleColumn::Effort => {
            let value = if column == RoleColumn::Model {
                item.role.model.as_deref()
            } else {
                item.role.effort.as_deref()
            };
            let (label, unset) = role_setting_label(value);
            cell.child(text(
                label,
                !unset,
                if unset { theme::faint() } else { theme::text() },
            ))
            .into_any_element()
        }
        RoleColumn::Crews => {
            let crews = item.activity.crew_count;
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
            let (label, live) = last_active_label(&item.activity, now);
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
