use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, Context, FontWeight, KeyDownEvent, SharedString, Window,
};
use runner_app::ui::{
    Button, ButtonSize, ButtonVariant, ContextMenu, EmptyStateCard, IconButton, IconButtonSize,
    MenuItem as UiMenuItem, PaginatedListPage, Tooltip,
};
use runner_backend::ops::crew::CrewListItem;

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
            AppRoute::Crews => self.render_crews_page(cx),
            AppRoute::CrewEditor(_) => self.render_crew_editor(window, cx),
            _ => div().into_any_element(),
        }
    }

    fn render_crews_page(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let root = cx.entity();
        let create_root = root.clone();
        let empty_create_root = root.clone();
        let clear_root = root.clone();
        let page_root = root;
        let query = self.crew_surfaces.list.query.clone();
        let cards = self
            .crew_surfaces
            .list
            .items
            .clone()
            .into_iter()
            .map(|crew| self.render_crew_card(crew, cx))
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
                    .child("Search checks names, purposes, goals, system prompts, slot handles, runner handles, and runtimes."),
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
            "A crew is a named group of runners working a goal together. Spin up your first one to get started.",
            Button::new("empty-new-crew", "+ New crew")
                .variant(ButtonVariant::Primary)
                .on_press(move |window, cx| {
                    empty_create_root.update(cx, |this, cx| {
                        this.open_create_crew(window, cx)
                    });
                }),
        );
        PaginatedListPage::new(
            "Crews",
            div().child("Named groups of runners with a shared goal."),
            Button::new("new-crew", "+ New crew")
                .variant(ButtonVariant::Primary)
                .on_press(move |window, cx| {
                    create_root.update(cx, |this, cx| this.open_create_crew(window, cx));
                }),
            "crews",
            empty_state,
            self.crew_surfaces.search.clone(),
            no_matches,
            self.crew_surfaces.list.page,
            self.crew_surfaces.list.page_count(),
            Rc::new(move |page, _, cx| {
                page_root.update(cx, |this, cx| this.set_crew_page(page, cx));
            }),
            div().flex().flex_col().gap_3().children(cards),
            self.crew_surfaces.scroll.clone(),
            self.crew_surfaces.scrollbar.clone(),
        )
        .counts(
            self.crew_surfaces.list.filtered_count,
            self.crew_surfaces.list.total_count,
        )
        .load_state(
            self.crew_surfaces.list.loading,
            self.crew_surfaces.list.loaded,
            self.crew_surfaces.list.error.clone().map(Into::into),
        )
        .into_any_element()
    }

    fn render_crew_card(&self, item: CrewListItem, cx: &mut Context<Self>) -> AnyElement {
        let root = cx.entity();
        let click_root = root.clone();
        let key_root = root.clone();
        let menu_root = root;
        let crew_id = item.crew.id.clone();
        let key_crew_id = crew_id.clone();
        let menu_item = item.clone();
        let count = if item.role_count == 1 {
            "1 runner".to_owned()
        } else {
            format!("{} runners", item.role_count)
        };
        let members = if item.members.is_empty() {
            vec![div()
                .text_size(theme::text_ui())
                .text_color(theme::faint())
                .italic()
                .child("No slots yet.")
                .into_any_element()]
        } else {
            item.members
                .iter()
                .enumerate()
                .map(|(index, member)| {
                    let pill = div()
                        .id(("crew-member-pill", index))
                        .flex()
                        .items_center()
                        .gap(rems(6. / 16.))
                        .rounded_full()
                        .bg(theme::raised())
                        .px(rems(10. / 16.))
                        .py(rems(6. / 16.))
                        .text_size(theme::text_ui())
                        .child(
                            div()
                                .font_family(theme::UI_MONOSPACE_FONT)
                                .font_weight(FontWeight::MEDIUM)
                                .child(format!("@{}", member.slot_handle)),
                        )
                        .child(
                            div()
                                .text_size(theme::text_meta())
                                .text_color(theme::muted())
                                .child(format!("{}-{}", member.runtime, member.role_handle)),
                        )
                        .children(member.lead.then(|| {
                            div()
                                .rounded_sm()
                                .bg(theme::with_alpha(theme::accent(), 0.15))
                                .px_1()
                                .text_size(theme::text_micro())
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme::accent())
                                .child("LEAD")
                        }));
                    if member.lead {
                        Tooltip::new(("crew-member-pill-tooltip", index), "lead slot", pill)
                            .into_any_element()
                    } else {
                        pill.into_any_element()
                    }
                })
                .collect()
        };
        div()
            .id(SharedString::from(format!("crew-card-{}", item.crew.id)))
            .group("crew-card")
            .tab_index(0)
            .flex()
            .flex_col()
            .gap_3()
            .rounded_lg()
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .p_5()
            .cursor_pointer()
            .hover(|card| card.border_color(theme::border_strong()))
            .focus_visible(|card| card.border_color(theme::faint()))
            .on_click(move |_, window, cx| {
                click_root.update(cx, |this, cx| {
                    this.open_crew_editor(crew_id.clone(), window, cx)
                });
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    key_root.update(cx, |this, cx| {
                        this.open_crew_editor(key_crew_id.clone(), window, cx)
                    });
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
                            .flex()
                            .flex_col()
                            .gap(rems(2. / 16.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(theme::text_heading())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(item.crew.name.clone()),
                            )
                            .child(if let Some(purpose) = item.crew.purpose.clone() {
                                div()
                                    .max_h(rems(38. / 16.))
                                    .overflow_hidden()
                                    .text_size(theme::text_ui())
                                    .text_color(theme::muted())
                                    .child(purpose)
                            } else {
                                div()
                                    .text_size(theme::text_ui())
                                    .text_color(theme::faint())
                                    .italic()
                                    .child("No purpose set")
                            }),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(theme::text_ui())
                            .text_color(theme::muted())
                            .child(count)
                            .child(
                                IconButton::new(
                                    SharedString::from(format!("crew-actions-{}", item.crew.id)),
                                    "more-horizontal.svg",
                                )
                                .size(IconButtonSize::Sm)
                                .stop_click_propagation(true)
                                .tooltip("Actions")
                                .on_press(move |window, cx| {
                                    let position = window.mouse_position();
                                    let item = menu_item.clone();
                                    menu_root.update(cx, |this, cx| {
                                        this.open_crew_menu(item, position, window, cx)
                                    });
                                }),
                            ),
                    ),
            )
            .child(div().flex().flex_wrap().gap_2().children(members))
            .into_any_element()
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
        focus.focus(window);
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
