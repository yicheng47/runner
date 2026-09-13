use super::logic::plural;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, Context, CursorStyle, FontWeight, KeyDownEvent, MouseButton,
    SharedString, Window,
};
use runner_app::ui::{
    Button, ButtonSize, ButtonVariant, EmptyStateCard, IconButton, IconButtonSize,
    PaginatedListPage, Tooltip,
};
use runner_backend::ops::runner::RunnerWithActivity;

use super::*;
use crate::list_controls::LIST_QUERY_DEBOUNCE_MS;
use crate::surfaces::*;
use crate::*;

impl NativeRoot {
    pub(crate) fn open_runners(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.route != AppRoute::Runners {
            self.runner_surfaces.list.reset();
            self.runner_surfaces
                .search
                .update(cx, |search, search_cx| search.reset_value("", search_cx));
            self.runner_surfaces
                .scroll
                .set_offset(gpui::Point::new(px(0.), px(0.)));
        }
        self.enter_entity_route(AppRoute::Runners, window, cx);
        self.load_runner_page(cx);
    }

    pub(crate) fn open_runner_detail(
        &mut self,
        handle: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.enter_entity_route(AppRoute::RunnerDetail(handle.clone()), window, cx);
        self.load_runner_detail(handle, cx);
    }

    pub(crate) fn enter_entity_route(
        &mut self,
        route: AppRoute,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dismiss_sidebar_transients(cx);
        self.runner_surfaces.context_menu = None;
        self.crew_surfaces.context_menu = None;
        self.set_route(route, cx);
        window.focus(&self.root_focus);
        cx.notify();
    }

    pub(super) fn set_runner_query(&mut self, query: String, cx: &mut Context<Self>) {
        let update = self.runner_surfaces.list.set_query(query);
        if update.load_now {
            self.load_runner_page(cx);
        }
        cx.spawn(async move |weak, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(LIST_QUERY_DEBOUNCE_MS))
                .await;
            let _ = weak.update(cx, |this, cx| {
                if this
                    .runner_surfaces
                    .list
                    .apply_debounced_query(update.generation)
                {
                    this.load_runner_page(cx);
                }
            });
        })
        .detach();
    }

    fn set_runner_page(&mut self, page: usize, cx: &mut Context<Self>) {
        if self.runner_surfaces.list.set_page(page) {
            self.load_runner_page(cx);
        }
    }

    pub(crate) fn load_runner_page(&mut self, cx: &mut Context<Self>) {
        let request = self.runner_surfaces.list.begin_load();
        let request_id = request.request_id;
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::runner::runner_list_with_activity(
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
                        if this.runner_surfaces.list.apply_success(request_id, page) {
                            this.load_runner_page(cx);
                        }
                    }
                    Err(error) => {
                        this.runner_surfaces.list.apply_error(request_id, error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn load_runner_detail(&mut self, handle: String, cx: &mut Context<Self>) {
        if self.runner_surfaces.detail.handle == handle {
            let detail = &mut self.runner_surfaces.detail;
            detail.loading = !detail.loaded;
            detail.error = None;
        } else {
            self.runner_surfaces.detail = RunnerDetailState {
                handle: handle.clone(),
                loading: true,
                ..Default::default()
            };
        }
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            let requested = handle.clone();
            let result = (|| {
                let runner = runner_backend::ops::runner::runner_get_by_handle(&core, &handle)?;
                let activity = runner_backend::ops::runner::runner_activity(&core, &runner.id)?;
                let crews = runner_backend::ops::slot::runner_crews_list(&core, &runner.id)?;
                Ok::<_, runner_backend::error::Error>((runner, activity, crews))
            })();
            result
                .map(|(runner, activity, crews)| (requested.clone(), runner, activity, crews))
                .map_err(|error| (requested, error.to_string()))
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok((handle, runner, activity, crews))
                        if matches!(
                            &this.route,
                            AppRoute::RunnerDetail(active) if active == &handle
                        ) =>
                    {
                        this.runner_surfaces.detail = RunnerDetailState {
                            handle,
                            runner: Some(runner),
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
                            AppRoute::RunnerDetail(active) if active == &handle
                        ) =>
                    {
                        let detail = &mut this.runner_surfaces.detail;
                        detail.loading = false;
                        if error.to_lowercase().contains("not found") {
                            detail.loaded = true;
                            detail.runner = None;
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
            AppRoute::Runners => self.render_runners_page(cx),
            AppRoute::RunnerDetail(_) => self.render_runner_detail(cx),
            AppRoute::Crews | AppRoute::CrewEditor(_) => self.render_crew_surface(window, cx),
            AppRoute::Mission(_) => self.mission_workspace.clone().into_any_element(),
            AppRoute::ArchivedChat => self.render_archived_chat(window, cx),
            AppRoute::Settings => self.render_active_tab(window, cx),
        }
    }

    fn render_runners_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let root = cx.entity();
        let create_root = root.clone();
        let empty_create_root = root.clone();
        let clear_root = root.clone();
        let page_root = root.clone();
        let query = self.runner_surfaces.list.query.clone();
        let cards = self
            .runner_surfaces
            .list
            .items
            .clone()
            .into_iter()
            .map(|item| self.render_runner_card(item, cx))
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
                    .child(format!("No runners match \"{query}\"")),
            )
            .child(
                div()
                    .text_size(theme::text_ui())
                    .line_height(rems(19. / 16.))
                    .text_color(theme::muted())
                    .child("Search checks handles and names."),
            )
            .child(
                Button::new("clear-runner-search", "Clear search")
                    .size(ButtonSize::Sm)
                    .on_press(move |_, cx| {
                        clear_root.update(cx, |this, cx| {
                            this.runner_surfaces
                                .search
                                .update(cx, |search, search_cx| search.set_value("", search_cx));
                        });
                    }),
            );
        let empty_state = EmptyStateCard::new(
            svg()
                .flex_none()
                .path("terminal.svg")
                .size(rems(22. / 16.))
                .text_color(theme::accent()),
            "No runners yet",
            "A runner is a reusable CLI agent — claude-code, codex, a custom shell — that crews pull in. Add one to start composing crews.",
            Button::new("empty-new-runner", "+ New runner")
                .variant(ButtonVariant::Primary)
                .on_press(move |window, cx| {
                    empty_create_root.update(cx, |this, cx| {
                        this.open_create_runner(window, cx)
                    });
                }),
        );
        PaginatedListPage::new(
            "Runners",
            div().child("Reusable CLI agents — pick one for a crew slot or chat directly."),
            Button::new("new-runner", "+ New runner")
                .variant(ButtonVariant::Primary)
                .on_press(move |window, cx| {
                    create_root.update(cx, |this, cx| this.open_create_runner(window, cx));
                }),
            "runners",
            empty_state,
            self.runner_surfaces.search.clone(),
            no_matches,
            self.runner_surfaces.list.page,
            self.runner_surfaces.list.page_count(),
            Rc::new(move |page, _, cx| {
                page_root.update(cx, |this, cx| this.set_runner_page(page, cx));
            }),
            div().flex().flex_col().gap_3().children(cards),
            self.runner_surfaces.scroll.clone(),
            self.runner_surfaces.scrollbar.clone(),
        )
        .counts(
            self.runner_surfaces.list.filtered_count,
            self.runner_surfaces.list.total_count,
        )
        .load_state(
            self.runner_surfaces.list.loading,
            self.runner_surfaces.list.loaded,
            self.runner_surfaces.list.error.clone().map(Into::into),
        )
        .into_any_element()
    }

    fn render_runner_card(&self, item: RunnerWithActivity, cx: &mut Context<Self>) -> AnyElement {
        let root = cx.entity();
        let open_root = root.clone();
        let key_root = root.clone();
        let chat_root = root.clone();
        let chat_key_root = root.clone();
        let menu_root = root;
        let handle = item.runner.handle.clone();
        let open_handle = handle.clone();
        let menu_item = item.clone();
        let chat_runner = item.runner.clone();
        let chat_key_runner = item.runner.clone();
        let sessions_label = plural(item.activity.active_sessions, "session", "sessions");
        let missions_label = plural(item.activity.active_missions, "mission", "missions");
        let crews_label = if item.activity.crew_count == 1 {
            "in 1 crew".to_owned()
        } else {
            format!("in {} crews", item.activity.crew_count)
        };
        let live = item.activity.active_sessions > 0 || item.activity.active_missions > 0;
        let pending = self.runner_surfaces.chat_pending.as_deref() == Some(item.runner.id.as_str());
        let command = if item.runner.args.is_empty() {
            item.runner.command.clone()
        } else {
            format!("{} {}", item.runner.command, item.runner.args.join(" "))
        };
        div()
            .id(SharedString::from(format!(
                "runner-card-{}",
                item.runner.id
            )))
            .group("runner-card")
            .tab_index(0)
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .rounded_lg()
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .p_4()
            .cursor_pointer()
            .hover(|card| card.border_color(theme::border_strong()))
            .focus_visible(|card| card.border_color(theme::faint()))
            .on_click(move |_, window, cx| {
                open_root.update(cx, |this, cx| {
                    this.open_runner_detail(open_handle.clone(), window, cx)
                });
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    let handle = handle.clone();
                    key_root.update(cx, |this, cx| this.open_runner_detail(handle, window, cx));
                }
            })
            .child(
                div()
                    .flex()
                    .items_start()
                    .justify_between()
                    .gap_4()
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex_1()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .font_family(theme::UI_MONOSPACE_FONT)
                                            .text_size(theme::text_heading())
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme::text())
                                            .child(format!("@{}", item.runner.handle)),
                                    )
                                    .child(
                                        div()
                                            .font_family(theme::UI_MONOSPACE_FONT)
                                            .text_size(theme::text_meta())
                                            .text_color(theme::faint())
                                            .child(item.runner.runtime.clone()),
                                    )
                                    .child(Tooltip::new(
                                        SharedString::from(format!(
                                            "runner-chat-tooltip-{}",
                                            item.runner.id
                                        )),
                                        "Start a new chat",
                                        div()
                                            .id(SharedString::from(format!(
                                                "runner-chat-{}",
                                                item.runner.id
                                            )))
                                            .tab_index(0)
                                            .tab_stop(!pending)
                                            .ml_1()
                                            .flex()
                                            .items_center()
                                            .gap(rems(6. / 16.))
                                            .rounded_sm()
                                            .px(rems(6. / 16.))
                                            .py(rems(2. / 16.))
                                            .text_size(theme::text_meta())
                                            .line_height(rems(1.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme::accent())
                                            .opacity(if pending { 0.6 } else { 1. })
                                            .cursor(if pending {
                                                CursorStyle::Arrow
                                            } else {
                                                CursorStyle::PointingHand
                                            })
                                            .when(!pending, |button| {
                                                button.hover(|button| {
                                                    button
                                                        .bg(theme::with_alpha(theme::accent(), 0.1))
                                                })
                                            })
                                            .focus_visible(|button| {
                                                button.bg(theme::with_alpha(theme::accent(), 0.1))
                                            })
                                            .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                                cx.stop_propagation()
                                            })
                                            .on_click(move |_, window, cx| {
                                                cx.stop_propagation();
                                                if !pending {
                                                    let runner = chat_runner.clone();
                                                    chat_root.update(cx, |this, cx| {
                                                        this.start_runner_chat(runner, window, cx)
                                                    });
                                                }
                                            })
                                            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                                if !pending
                                                    && matches!(
                                                        event.keystroke.key.as_str(),
                                                        "enter" | "space"
                                                    )
                                                {
                                                    cx.stop_propagation();
                                                    let runner = chat_key_runner.clone();
                                                    chat_key_root.update(cx, |this, cx| {
                                                        this.start_runner_chat(runner, window, cx)
                                                    });
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
                                                    .text_color(theme::accent()),
                                            )
                                            .child(div().h(rems(1.)).flex().items_center().child(
                                                if pending { "Starting…" } else { "Chat" },
                                            )),
                                    )),
                            )
                            .child(
                                div()
                                    .mt_1()
                                    .max_h(rems(38. / 16.))
                                    .overflow_hidden()
                                    .text_size(theme::text_ui())
                                    .text_color(theme::muted())
                                    .child(item.runner.display_name.clone()),
                            )
                            .child(
                                div()
                                    .mt(rems(6. / 16.))
                                    .truncate()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_meta())
                                    .text_color(theme::faint())
                                    .child(format!("$ {command}")),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(theme::text_ui())
                            .child(
                                div()
                                    .text_color(if live {
                                        theme::accent()
                                    } else {
                                        theme::faint()
                                    })
                                    .child(if live {
                                        if item.activity.active_missions > 0 {
                                            format!("{sessions_label} · {missions_label}")
                                        } else {
                                            sessions_label
                                        }
                                    } else {
                                        crews_label
                                    }),
                            )
                            .child(
                                IconButton::new(
                                    SharedString::from(format!(
                                        "runner-actions-{}",
                                        item.runner.id
                                    )),
                                    "more-horizontal.svg",
                                )
                                .size(IconButtonSize::Sm)
                                .stop_click_propagation(true)
                                .tooltip("More actions")
                                .on_press(move |window, cx| {
                                    let position = window.mouse_position();
                                    let item = menu_item.clone();
                                    menu_root.update(cx, |this, cx| {
                                        this.open_runner_menu(item, position, window, cx)
                                    });
                                }),
                            ),
                    ),
            )
            .into_any_element()
    }
}
