use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, Context, CursorStyle, DragMoveEvent, Entity, FocusHandle,
    FontWeight, HighlightStyle, KeyDownEvent, ScrollHandle, SharedString, StyledText, Subscription,
    Window,
};
use runner_app::ui::{
    Button, ButtonSize, ButtonVariant, ConfirmDialog, ContextMenu, EmptyStateCard, Field,
    IconButton, IconButtonSize, MenuItem as UiMenuItem, Modal, ModelField, OverlayWidth,
    PaginatedListPage, RuntimeBadge, Scrollbar, SearchInput, SelectOption, StyledSelect, TextField,
    Tooltip,
};
use runner_backend::model::{Crew, SlotWithRunner};
use runner_backend::ops::crew::{CreateCrewInput, CrewListItem, UpdateCrewInput};
use runner_backend::ops::runner::RunnerWithActivity;
use runner_backend::ops::runtime::{RuntimeCatalogEntry, RuntimeCatalogOption};

use super::*;
use crate::list_controls::{ListControls, LIST_QUERY_DEBOUNCE_MS};
use crate::*;

const FORM_WIDTH: f32 = 576.;
const FIELD_WIDTH: f32 = 528.;

#[derive(Clone)]
struct SlotDrag {
    slot_id: String,
    label: String,
}

impl Render for SlotDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(rems(220. / 16.))
            .px_3()
            .py_2()
            .rounded_sm()
            .border_1()
            .border_color(theme::accent())
            .bg(theme::panel())
            .shadow_lg()
            .font_family("JetBrains Mono")
            .text_size(rems(13. / 16.))
            .text_color(theme::text())
            .child(self.label.clone())
    }
}

#[derive(Default)]
struct CrewEditorState {
    crew_id: String,
    crew: Option<Crew>,
    slots: Vec<SlotWithRunner>,
    loaded: bool,
    loading: bool,
    error: Option<String>,
    name: Option<Entity<TextField>>,
    _name_subscription: Option<Subscription>,
    original_name: String,
    name_changed: bool,
    name_dirty: bool,
    name_empty: bool,
    goal_edit: Option<Entity<TextField>>,
    conventions_edit: Option<Entity<TextField>>,
    saving_name: bool,
    saving_goal: bool,
    saving_conventions: bool,
    reordering: bool,
    dragged_slot_id: Option<String>,
    drop_target: Option<usize>,
}

struct CreateCrewForm {
    name: Entity<TextField>,
    purpose: Entity<TextField>,
    goal: Entity<TextField>,
    purpose_hint_focus: FocusHandle,
    goal_hint_focus: FocusHandle,
    close_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    submitting: bool,
    error: Option<String>,
}

struct AddSlotForm {
    crew_id: String,
    crew_name: String,
    existing_handles: HashSet<String>,
    runners: Vec<RunnerWithActivity>,
    runtimes: Vec<RuntimeCatalogEntry>,
    query: Entity<TextField>,
    last_synced_query: String,
    selected_runner_id: Option<String>,
    slot_handle: Entity<TextField>,
    runtime_override: String,
    model_override: Entity<TextField>,
    model_field: Entity<ModelField>,
    runtime_select: Entity<StyledSelect>,
    scroll: ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    slot_handle_hint_focus: FocusHandle,
    runtime_hint_focus: FocusHandle,
    model_hint_focus: FocusHandle,
    close_focus: FocusHandle,
    cancel_focus: FocusHandle,
    submit_focus: FocusHandle,
    slot_handle_empty: bool,
    slot_handle_error: Option<String>,
    loading: bool,
    submitting: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone)]
enum CrewMenuAction {
    Open(String),
    Delete { id: String, name: String },
}

#[derive(Clone)]
enum SlotMenuAction {
    SetLead(String),
    Edit(SlotWithRunner),
    Remove(SlotWithRunner),
}

struct CrewDeleteConfirm {
    id: String,
    name: String,
}

struct SlotRemoveConfirm {
    slot: SlotWithRunner,
}

pub(crate) struct CrewSurfaces {
    list: ListControls<CrewListItem>,
    search: Entity<SearchInput>,
    scroll: ScrollHandle,
    scrollbar: Entity<Scrollbar>,
    editor: CrewEditorState,
    create: Option<CreateCrewForm>,
    add_slot: Option<AddSlotForm>,
    pub(crate) context_menu: Option<Entity<ContextMenu>>,
    delete_confirm: Option<CrewDeleteConfirm>,
    delete_busy: bool,
    slot_remove_confirm: Option<SlotRemoveConfirm>,
    slot_remove_busy: bool,
}

impl CrewSurfaces {
    pub(crate) fn new(root: Entity<NativeRoot>, cx: &mut Context<NativeRoot>) -> Self {
        let search = cx.new(move |search_cx| {
            SearchInput::new(
                "",
                "Search crews",
                "Search crews…",
                Rc::new(move |query, cx| {
                    root.update(cx, |this, cx| this.set_crew_query(query, cx));
                }),
                search_cx,
            )
        });
        let scroll = ScrollHandle::new();
        let owner = cx.entity_id();
        let scrollbar = cx.new(|_| Scrollbar::app(scroll.clone(), owner));
        Self {
            list: ListControls::default(),
            search,
            scroll,
            scrollbar,
            editor: CrewEditorState::default(),
            create: None,
            add_slot: None,
            context_menu: None,
            delete_confirm: None,
            delete_busy: false,
            slot_remove_confirm: None,
            slot_remove_busy: false,
        }
    }
}

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

    fn set_crew_query(&mut self, query: String, cx: &mut Context<Self>) {
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
                    .path("search-x.svg")
                    .size(rems(20. / 16.))
                    .text_color(theme::faint()),
            )
            .child(
                div()
                    .text_size(rems(14. / 16.))
                    .font_weight(FontWeight::MEDIUM)
                    .child(format!("No crews match \"{query}\"")),
            )
            .child(
                div()
                    .max_w(rems(480. / 16.))
                    .text_size(rems(12. / 16.))
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
        let count = if item.runner_count == 1 {
            "1 runner".to_owned()
        } else {
            format!("{} runners", item.runner_count)
        };
        let members = if item.members.is_empty() {
            vec![div()
                .text_size(rems(12. / 16.))
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
                        .text_size(rems(12. / 16.))
                        .child(
                            div()
                                .font_family("JetBrains Mono")
                                .font_weight(FontWeight::MEDIUM)
                                .child(format!("@{}", member.slot_handle)),
                        )
                        .child(
                            div()
                                .text_size(rems(11. / 16.))
                                .text_color(theme::muted())
                                .child(format!("{}-{}", member.runtime, member.runner_handle)),
                        )
                        .children(member.lead.then(|| {
                            div()
                                .rounded_sm()
                                .bg(theme::with_alpha(theme::accent(), 0.15))
                                .px_1()
                                .text_size(rems(9. / 16.))
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
                                    .text_size(rems(1.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(item.crew.name.clone()),
                            )
                            .child(if let Some(purpose) = item.crew.purpose.clone() {
                                div()
                                    .max_h(rems(38. / 16.))
                                    .overflow_hidden()
                                    .text_size(rems(12. / 16.))
                                    .text_color(theme::muted())
                                    .child(purpose)
                            } else {
                                div()
                                    .text_size(rems(12. / 16.))
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
                            .text_size(rems(12. / 16.))
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

    pub(crate) fn load_crew_editor(&mut self, crew_id: String, cx: &mut Context<Self>) {
        if self.crew_surfaces.editor.crew_id == crew_id {
            let editor = &mut self.crew_surfaces.editor;
            editor.loading = !editor.loaded;
            editor.error = None;
        } else {
            self.crew_surfaces.editor = CrewEditorState {
                crew_id: crew_id.clone(),
                loading: true,
                ..Default::default()
            };
        }
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            let requested = crew_id.clone();
            let result = (|| {
                let crew = runner_backend::ops::crew::crew_get(&core, &crew_id)?;
                let slots = runner_backend::ops::slot::slot_list(&core, &crew_id)?;
                Ok::<_, runner_backend::error::Error>((crew, slots))
            })();
            result
                .map(|(crew, slots)| (requested.clone(), crew, slots))
                .map_err(|error| (requested, error.to_string()))
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok((crew_id, crew, slots))
                        if matches!(
                            &this.route,
                            AppRoute::CrewEditor(active) if active == &crew_id
                        ) =>
                    {
                        let active_crew_id = crew_id.clone();
                        let crew_name = crew.name.clone();
                        let existing_handles = slots
                            .iter()
                            .map(|slot| slot.slot.slot_handle.clone())
                            .collect::<HashSet<_>>();
                        if this.crew_surfaces.editor.name.is_none() {
                            let name = cx.new(|input_cx| {
                                TextField::new(
                                    input_cx.focus_handle(),
                                    crew.name.clone(),
                                    "",
                                    false,
                                )
                                .text_size(14.)
                            });
                            let subscription = cx.observe(&name, move |this, input, cx| {
                                let value = input.read(cx).text().to_owned();
                                let editor = &mut this.crew_surfaces.editor;
                                let (changed, dirty, empty) =
                                    crew_name_state(&value, &editor.original_name);
                                if editor.name_changed != changed
                                    || editor.name_dirty != dirty
                                    || editor.name_empty != empty
                                {
                                    editor.name_changed = changed;
                                    editor.name_dirty = dirty;
                                    editor.name_empty = empty;
                                    cx.notify();
                                }
                            });
                            let editor = &mut this.crew_surfaces.editor;
                            editor.name = Some(name);
                            editor._name_subscription = Some(subscription);
                        }
                        let name = this
                            .crew_surfaces
                            .editor
                            .name
                            .as_ref()
                            .cloned()
                            .expect("crew editor name field");
                        let current_name = name.read(cx).text().to_owned();
                        match crew_name_refresh(&current_name, &crew.name, name.read(cx).edited()) {
                            CrewNameRefresh::MarkClean => {
                                name.update(cx, |input, _| input.mark_clean());
                            }
                            CrewNameRefresh::Reset => {
                                name.update(cx, |input, input_cx| {
                                    input.reset(crew.name.clone(), input_cx)
                                });
                            }
                            CrewNameRefresh::Preserve => {}
                        }
                        let current_name = name.read(cx).text().to_owned();
                        let (name_changed, name_dirty, name_empty) =
                            crew_name_state(&current_name, &crew.name);
                        let editor = &mut this.crew_surfaces.editor;
                        editor.crew_id = crew_id;
                        editor.original_name = crew.name.clone();
                        editor.crew = Some(crew);
                        editor.slots = slots;
                        editor.loaded = true;
                        editor.loading = false;
                        editor.error = None;
                        editor.name_changed = name_changed;
                        editor.name_dirty = name_dirty;
                        editor.name_empty = name_empty;
                        if let Some(form) = this
                            .crew_surfaces
                            .add_slot
                            .as_mut()
                            .filter(|form| form.crew_id == active_crew_id)
                        {
                            form.crew_name = crew_name;
                            form.existing_handles = existing_handles;
                            if !form.slot_handle.read(cx).edited() {
                                let suggestion = selected_add_slot_runner(form)
                                    .map(|runner| {
                                        suggest_slot_handle(
                                            &runner.runner.handle,
                                            &form.existing_handles,
                                        )
                                    })
                                    .unwrap_or_default();
                                if form.slot_handle.read(cx).text() != suggestion {
                                    form.slot_handle.update(cx, |input, input_cx| {
                                        input.reset(suggestion, input_cx)
                                    });
                                }
                            } else {
                                let handle = form.slot_handle.read(cx).text();
                                form.slot_handle_empty = handle.is_empty();
                                form.slot_handle_error =
                                    slot_handle_error(handle, &form.existing_handles);
                            }
                        }
                    }
                    Ok(_) => {}
                    Err((crew_id, error))
                        if matches!(
                            &this.route,
                            AppRoute::CrewEditor(active) if active == &crew_id
                        ) =>
                    {
                        let editor = &mut this.crew_surfaces.editor;
                        editor.loading = false;
                        if error.to_lowercase().contains("not found") {
                            editor.crew = None;
                            editor.slots.clear();
                            editor.name = None;
                            editor._name_subscription = None;
                            editor.original_name.clear();
                            editor.goal_edit = None;
                            editor.conventions_edit = None;
                            editor.loaded = true;
                        }
                        editor.error = Some(error);
                    }
                    Err(_) => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_crew_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        #[cfg(not(windows))]
        let _ = window;
        let editor = &self.crew_surfaces.editor;
        let crew = editor.crew.clone();
        let slots = editor.slots.clone();
        let name = editor.name.clone();
        let name_changed = editor.name_changed;
        let name_dirty = editor.name_dirty;
        let name_empty = editor.name_empty;
        let root = cx.entity();
        let back_root = root.clone();
        let back_key_root = root.clone();
        let save_name_root = root.clone();
        let start_mission_root = root.clone();
        let start_mission_crew_id = editor.crew_id.clone();
        let add_slot_root = root.clone();
        let header = div()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .border_b_1()
            .border_color(theme::border())
            .bg(theme::panel())
            .px_8()
            .pb_4()
            .pt(rems(36. / 16.))
            .map(|header| {
                #[cfg(windows)]
                let header = header.pr(px(
                    32. * self.settings(cx).app_zoom + self.caption_inset(window, cx)
                ));
                header
            })
            .on_key_down(cx.listener(Self::on_crew_name_key_down))
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .id("crew-editor-back")
                            .tab_index(0)
                            .flex_none()
                            .cursor_pointer()
                            .text_size(rems(14. / 16.))
                            .text_color(theme::muted())
                            .hover(|text| text.text_color(theme::text()))
                            .focus_visible(|text| text.text_color(theme::text()).underline())
                            .on_click(move |_, window, cx| {
                                back_root.update(cx, |this, cx| this.open_crews(window, cx));
                            })
                            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                    cx.stop_propagation();
                                    back_key_root
                                        .update(cx, |this, cx| this.open_crews(window, cx));
                                }
                            })
                            .child("‹ Crews"),
                    )
                    .child(div().text_color(theme::border_strong()).child("›"))
                    .child(if let Some(name) = name.clone() {
                        div()
                            .w_full()
                            .max_w(rems(384. / 16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(name)
                            .into_any_element()
                    } else {
                        div()
                            .text_size(rems(14. / 16.))
                            .text_color(theme::faint())
                            .child("…")
                            .into_any_element()
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(if name_changed || editor.saving_name {
                        Button::new(
                            "save-crew-name",
                            if editor.saving_name {
                                "Saving..."
                            } else {
                                "Save"
                            },
                        )
                        .disabled(editor.saving_name || !name_dirty)
                        .tooltip(if name_empty {
                            "Crew name cannot be empty"
                        } else if name_dirty {
                            "Save crew name"
                        } else {
                            "No persisted change after trimming"
                        })
                        .on_press(move |_, cx| {
                            save_name_root.update(cx, |this, cx| this.save_crew_name(cx));
                        })
                        .into_any_element()
                    } else {
                        Tooltip::new(
                            "crew-name-saved-tooltip",
                            "Crew name is saved. Slot changes save immediately.",
                            div()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_sm()
                                .border_1()
                                .border_color(theme::border())
                                .bg(theme::raised())
                                .px_3()
                                .py(rems(6. / 16.))
                                .text_size(rems(14. / 16.))
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme::faint())
                                .child("Saved"),
                        )
                        .into_any_element()
                    })
                    .child(
                        Button::new("start-crew-mission", "Start mission")
                            .variant(ButtonVariant::Primary)
                            .tooltip(if slots.is_empty() {
                                "Add at least one slot before starting a mission"
                            } else {
                                "Start a mission with this crew"
                            })
                            .disabled(slots.is_empty())
                            .on_press(move |window, cx| {
                                start_mission_root.update(cx, |this, cx| {
                                    this.open_start_mission_modal(
                                        Some(start_mission_crew_id.clone()),
                                        None,
                                        window,
                                        cx,
                                    )
                                });
                            }),
                    ),
            );
        let content = if editor.loading {
            div()
                .p_8()
                .text_size(rems(14. / 16.))
                .text_color(theme::muted())
                .child("Loading…")
                .into_any_element()
        } else if !editor.loaded {
            div()
                .m_8()
                .child(error_panel(
                    editor
                        .error
                        .clone()
                        .unwrap_or_else(|| "Failed to load crew.".into()),
                ))
                .into_any_element()
        } else if crew.is_none() {
            div()
                .p_8()
                .text_size(rems(14. / 16.))
                .text_color(theme::danger())
                .child("Crew not found.")
                .into_any_element()
        } else {
            let crew = match crew {
                Some(crew) => crew,
                None => unreachable!("crew presence checked above"),
            };
            let sections = div()
                .w_full()
                .min_w(px(0.))
                .flex()
                .flex_col()
                .gap_8()
                .children(editor.error.clone().map(error_panel))
                .child(
                    div()
                        .w_full()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .gap(rems(6. / 16.))
                        .child(section_label("Purpose"))
                        .child(if let Some(purpose) = crew.purpose.clone() {
                            div()
                                .w_full()
                                .min_w(px(0.))
                                .whitespace_normal()
                                .text_size(rems(14. / 16.))
                                .line_height(rems(20. / 16.))
                                .text_color(theme::text())
                                .child(purpose)
                        } else {
                            div()
                                .text_size(rems(14. / 16.))
                                .text_color(theme::faint())
                                .italic()
                                .child("No purpose set.")
                        }),
                )
                .child(self.render_crew_goal_section(&crew, cx))
                .child(self.render_crew_conventions_section(&crew, cx))
                .child(
                    div()
                        .w_full()
                        .min_w(px(0.))
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(
                            div()
                                .w_full()
                                .min_w(px(0.))
                                .flex()
                                .items_end()
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
                                                .text_size(rems(20. / 16.))
                                                .font_weight(FontWeight::BOLD)
                                                .child("Slots"),
                                        )
                                        .child(
                                            div()
                                                .w_full()
                                                .min_w(px(0.))
                                                .whitespace_normal()
                                                .text_size(rems(12. / 16.))
                                                .line_height(rems(1.))
                                                .text_color(theme::muted())
                                                .child(slot_section_description()),
                                        ),
                                )
                                .child(
                                    div().flex_none().child(
                                        Button::new("add-crew-slot", "+ Add slot")
                                            .variant(ButtonVariant::Primary)
                                            .on_press(move |window, cx| {
                                                add_slot_root.update(cx, |this, cx| {
                                                    this.open_add_slot(window, cx)
                                                });
                                            }),
                                    ),
                                ),
                        )
                        .child(self.render_slot_list(slots, cx)),
                );
            div()
                .mx_auto()
                .w_full()
                .min_w(px(0.))
                .max_w(rems(896. / 16.))
                .px_8()
                .py_8()
                .child(sections)
                .into_any_element()
        };
        div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .id("crew-editor-scroll")
                    .min_h(px(0.))
                    .flex_1()
                    .overflow_y_scroll()
                    .child(content),
            )
            .into_any_element()
    }

    fn on_crew_name_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = &self.crew_surfaces.editor;
        let Some(name) = editor.name.as_ref() else {
            return;
        };
        if !name.read(cx).focus_handle().is_focused(window) {
            return;
        }
        match event.keystroke.key.as_str() {
            "enter" if !name.read(cx).is_composing() => {
                cx.stop_propagation();
                self.save_crew_name(cx);
            }
            "escape" => {
                cx.stop_propagation();
                if let Some(value) = self
                    .crew_surfaces
                    .editor
                    .crew
                    .as_ref()
                    .map(|crew| crew.name.clone())
                {
                    name.update(cx, |field, field_cx| field.reset(value, field_cx));
                }
                window.focus(&self.root_focus);
                cx.notify();
            }
            _ => {}
        }
    }

    fn save_crew_name(&mut self, cx: &mut Context<Self>) {
        let editor = &mut self.crew_surfaces.editor;
        let (Some(crew), Some(name)) = (editor.crew.as_ref(), editor.name.as_ref()) else {
            return;
        };
        let next = name.read(cx).text().trim().to_owned();
        if next.is_empty() || next == crew.name || editor.saving_name {
            return;
        }
        editor.saving_name = true;
        let crew_id = crew.id.clone();
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            let result = runner_backend::ops::crew::crew_update(
                &core,
                &crew_id,
                UpdateCrewInput {
                    name: Some(next),
                    ..Default::default()
                },
            )
            .map(|_| ())
            .map_err(|error| error.to_string());
            (crew_id, result)
        });
        self.finish_crew_update(task, cx);
        cx.notify();
    }

    fn render_crew_goal_section(&self, crew: &Crew, cx: &mut Context<Self>) -> AnyElement {
        let editor = &self.crew_surfaces.editor;
        let root = cx.entity();
        let existing_edit_root = root.clone();
        let add_edit_root = root.clone();
        let save_root = root.clone();
        let cancel_root = root;
        div()
            .w_full()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap(rems(6. / 16.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(section_label("Default goal"))
                    .children(
                        (editor.goal_edit.is_none() && crew.goal.is_some()).then(|| {
                            text_action("edit-crew-goal", "Edit", move |window, cx| {
                                existing_edit_root
                                    .update(cx, |this, cx| this.start_crew_goal_edit(window, cx));
                            })
                        }),
                    ),
            )
            .child(if let Some(input) = editor.goal_edit.clone() {
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(input)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new(
                                    "save-crew-goal",
                                    if editor.saving_goal {
                                        "Saving..."
                                    } else {
                                        "Save"
                                    },
                                )
                                .disabled(editor.saving_goal)
                                .on_press(move |_, cx| {
                                    save_root.update(cx, |this, cx| this.save_crew_goal(cx));
                                }),
                            )
                            .child(
                                Button::new("cancel-crew-goal", "Cancel")
                                    .disabled(editor.saving_goal)
                                    .on_press(move |_, cx| {
                                        cancel_root
                                            .update(cx, |this, cx| this.cancel_crew_goal_edit(cx));
                                    }),
                            ),
                    )
                    .into_any_element()
            } else if let Some(goal) = crew.goal.clone() {
                div()
                    .w_full()
                    .min_w(px(0.))
                    .whitespace_normal()
                    .text_size(rems(14. / 16.))
                    .line_height(rems(20. / 16.))
                    .text_color(theme::text())
                    .child(goal)
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(text_action(
                        "add-crew-goal",
                        "+ Add default goal",
                        move |window, cx| {
                            add_edit_root
                                .update(cx, |this, cx| this.start_crew_goal_edit(window, cx));
                        },
                    ))
                    .child(
                        div()
                            .text_size(rems(12. / 16.))
                            .text_color(theme::faint())
                            .child("Pre-fills the Start Mission goal. Optional."),
                    )
                    .into_any_element()
            })
            .into_any_element()
    }

    fn render_crew_conventions_section(&self, crew: &Crew, cx: &mut Context<Self>) -> AnyElement {
        let editor = &self.crew_surfaces.editor;
        let root = cx.entity();
        let existing_edit_root = root.clone();
        let add_edit_root = root.clone();
        let save_root = root.clone();
        let cancel_root = root;
        div()
            .w_full()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap(rems(6. / 16.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(section_label("Team conventions"))
                    .children(
                        (editor.conventions_edit.is_none()
                            && crew.system_prompt_addendum.is_some())
                        .then(|| {
                            text_action("edit-crew-conventions", "Edit", move |window, cx| {
                                existing_edit_root.update(cx, |this, cx| {
                                    this.start_crew_conventions_edit(window, cx)
                                });
                            })
                        }),
                    ),
            )
            .child(if let Some(input) = editor.conventions_edit.clone() {
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(input)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                Button::new(
                                    "save-crew-conventions",
                                    if editor.saving_conventions {
                                        "Saving..."
                                    } else {
                                        "Save"
                                    },
                                )
                                .disabled(editor.saving_conventions)
                                .on_press(move |_, cx| {
                                    save_root.update(cx, |this, cx| {
                                        this.save_crew_conventions(cx)
                                    });
                                }),
                            )
                            .child(
                                Button::new("cancel-crew-conventions", "Cancel")
                                    .disabled(editor.saving_conventions)
                                    .on_press(move |_, cx| {
                                        cancel_root.update(cx, |this, cx| {
                                            this.cancel_crew_conventions_edit(cx)
                                        });
                                    }),
                            ),
                    )
                    .into_any_element()
            } else if let Some(conventions) = crew.system_prompt_addendum.clone() {
                div()
                    .w_full()
                    .min_w(px(0.))
                    .whitespace_normal()
                    .text_size(rems(14. / 16.))
                    .line_height(rems(20. / 16.))
                    .text_color(theme::text())
                    .child(conventions)
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(text_action(
                        "add-crew-conventions",
                        "+ Add team conventions",
                        move |window, cx| {
                            add_edit_root.update(cx, |this, cx| {
                                this.start_crew_conventions_edit(window, cx)
                            });
                        },
                    ))
                    .child(
                        div()
                            .text_size(rems(12. / 16.))
                            .text_color(theme::faint())
                            .child("Optional team-level guidance applied to all mission spawns. Leave blank for crews that need no team-level layer."),
                    )
                    .into_any_element()
            })
            .into_any_element()
    }

    fn start_crew_goal_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self
            .crew_surfaces
            .editor
            .crew
            .as_ref()
            .and_then(|crew| crew.goal.clone())
            .unwrap_or_default();
        let input = cx.new(|input_cx| {
            TextField::textarea(
                input_cx.focus_handle(),
                value,
                "Pre-fills the Start Mission goal.",
                4,
                false,
            )
            .auto_grow(12)
        });
        let focus = input.read(cx).focus_handle();
        self.crew_surfaces.editor.goal_edit = Some(input);
        focus.focus(window);
        cx.notify();
    }

    fn cancel_crew_goal_edit(&mut self, cx: &mut Context<Self>) {
        if !self.crew_surfaces.editor.saving_goal {
            self.crew_surfaces.editor.goal_edit = None;
            cx.notify();
        }
    }

    fn save_crew_goal(&mut self, cx: &mut Context<Self>) {
        let editor = &mut self.crew_surfaces.editor;
        let (Some(crew), Some(input)) = (editor.crew.as_ref(), editor.goal_edit.as_ref()) else {
            return;
        };
        if editor.saving_goal {
            return;
        }
        editor.saving_goal = true;
        let crew_id = crew.id.clone();
        let value = trimmed_option(input.read(cx).text());
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            let result = runner_backend::ops::crew::crew_update(
                &core,
                &crew_id,
                UpdateCrewInput {
                    goal: Some(value),
                    ..Default::default()
                },
            )
            .map(|_| ())
            .map_err(|error| error.to_string());
            (crew_id, result)
        });
        self.finish_crew_update(task, cx);
        cx.notify();
    }

    fn start_crew_conventions_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self
            .crew_surfaces
            .editor
            .crew
            .as_ref()
            .and_then(|crew| crew.system_prompt_addendum.clone())
            .unwrap_or_default();
        let input = cx.new(|input_cx| {
            TextField::textarea(
                input_cx.focus_handle(),
                value,
                "Optional team-level guidance applied to all mission spawns.",
                6,
                true,
            )
            .auto_grow(24)
        });
        let focus = input.read(cx).focus_handle();
        self.crew_surfaces.editor.conventions_edit = Some(input);
        focus.focus(window);
        cx.notify();
    }

    fn cancel_crew_conventions_edit(&mut self, cx: &mut Context<Self>) {
        if !self.crew_surfaces.editor.saving_conventions {
            self.crew_surfaces.editor.conventions_edit = None;
            cx.notify();
        }
    }

    fn save_crew_conventions(&mut self, cx: &mut Context<Self>) {
        let editor = &mut self.crew_surfaces.editor;
        let (Some(crew), Some(input)) = (editor.crew.as_ref(), editor.conventions_edit.as_ref())
        else {
            return;
        };
        if editor.saving_conventions {
            return;
        }
        editor.saving_conventions = true;
        let crew_id = crew.id.clone();
        let value = trimmed_option(input.read(cx).text());
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            let result = runner_backend::ops::crew::crew_update(
                &core,
                &crew_id,
                UpdateCrewInput {
                    system_prompt_addendum: Some(value),
                    ..Default::default()
                },
            )
            .map(|_| ())
            .map_err(|error| error.to_string());
            (crew_id, result)
        });
        self.finish_crew_update(task, cx);
        cx.notify();
    }

    fn open_create_crew(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.crew_surfaces.create.is_some() {
            return;
        }
        let name = cx
            .new(|input_cx| TextField::new(input_cx.focus_handle(), "", "runners-feature", false));
        let purpose = cx.new(|input_cx| {
            TextField::textarea(
                input_cx.focus_handle(),
                "",
                "What does this crew exist to do?",
                2,
                false,
            )
        });
        let goal = cx.new(|input_cx| {
            TextField::textarea(
                input_cx.focus_handle(),
                "",
                "Pre-fills the Start Mission goal.",
                3,
                false,
            )
        });
        let focus = name.read(cx).focus_handle();
        self.crew_surfaces.create = Some(CreateCrewForm {
            name,
            purpose,
            goal,
            purpose_hint_focus: cx.focus_handle(),
            goal_hint_focus: cx.focus_handle(),
            close_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            submit_focus: cx.focus_handle(),
            submitting: false,
            error: None,
        });
        focus.focus(window);
        cx.notify();
    }

    fn close_create_crew(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .crew_surfaces
            .create
            .as_ref()
            .is_some_and(|form| form.submitting)
        {
            return;
        }
        self.crew_surfaces.create = None;
        window.focus(&self.root_focus);
        cx.notify();
    }

    fn on_create_crew_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "enter"
            && self.crew_surfaces.create.as_ref().is_some_and(|form| {
                let multiline_focused = [&form.purpose, &form.goal]
                    .into_iter()
                    .any(|field| field.read(cx).focus_handle().is_focused(window));
                !multiline_focused && !create_crew_form_is_composing(form, cx)
            })
        {
            cx.stop_propagation();
            self.submit_create_crew(window, cx);
        }
    }

    fn submit_create_crew(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.crew_surfaces.create.as_mut() else {
            return;
        };
        let name = form.name.read(cx).text().trim().to_owned();
        if name.is_empty() {
            form.error = Some("Name is required".into());
            cx.notify();
            return;
        }
        if form.submitting {
            return;
        }
        form.submitting = true;
        form.error = None;
        let input = CreateCrewInput {
            name,
            purpose: trimmed_option(form.purpose.read(cx).text()),
            goal: trimmed_option(form.goal.read(cx).text()),
            system_prompt_addendum: None,
        };
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::crew::crew_create(&core, input).map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                match result {
                    Ok(crew) => {
                        this.crew_surfaces.create = None;
                        this.load_crew_page(cx);
                        this.open_crew_editor(crew.id, window, cx);
                    }
                    Err(error) => {
                        if let Some(form) = this.crew_surfaces.create.as_mut() {
                            form.submitting = false;
                            form.error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn render_create_crew_modal(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let form = self
            .crew_surfaces
            .create
            .as_ref()
            .expect("create crew form");
        let submitting = form.submitting;
        let can_submit = !submitting;
        let root = cx.entity();
        let close_root = root.clone();
        let cancel_root = root.clone();
        let submit_root = root.clone();
        let title = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(2. / 16.))
                    .child(
                        div()
                            .text_size(rems(1.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("New crew"),
                    )
                    .child(
                        div()
                            .text_size(rems(12. / 16.))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::muted())
                            .child("Group of runners that work missions together."),
                    ),
            )
            .child(
                IconButton::new("close-create-crew", "close.svg")
                    .focus_handle(form.close_focus.clone())
                    .tooltip("Close new crew")
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        close_root.update(cx, |this, cx| this.close_create_crew(window, cx));
                    }),
            );
        let body = div()
            .flex()
            .flex_col()
            .gap_4()
            .on_key_down(cx.listener(Self::on_create_crew_key_down))
            .children(form.error.clone().map(error_banner))
            .child(
                Field::new("crew-name", "Name", form.name.clone())
                    .focus_target(form.name.read(cx).focus_handle()),
            )
            .child(
                Field::new("crew-purpose", "Purpose", form.purpose.clone())
                    .focus_target(form.purpose.read(cx).focus_handle())
                    .hint("optional", form.purpose_hint_focus.clone()),
            )
            .child(
                Field::new("crew-goal", "Default goal", form.goal.clone())
                    .focus_target(form.goal.read(cx).focus_handle())
                    .hint("optional", form.goal_hint_focus.clone()),
            );
        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("cancel-create-crew", "Cancel")
                    .focus_handle(form.cancel_focus.clone())
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        cancel_root.update(cx, |this, cx| this.close_create_crew(window, cx));
                    }),
            )
            .child(
                Button::new(
                    "submit-create-crew",
                    if submitting {
                        "Creating…"
                    } else {
                        "Create crew"
                    },
                )
                .focus_handle(form.submit_focus.clone())
                .variant(ButtonVariant::Primary)
                .disabled(!can_submit)
                .on_press(move |window, cx| {
                    submit_root.update(cx, |this, cx| this.submit_create_crew(window, cx));
                }),
            );
        let modal_root = root;
        Modal::new(
            title,
            body,
            Rc::new(move |window, cx| {
                modal_root.update(cx, |this, cx| this.close_create_crew(window, cx));
            }),
        )
        .width(OverlayWidth::Md)
        .busy(submitting)
        .focus_order(if submitting {
            Vec::new()
        } else {
            vec![
                form.close_focus.clone(),
                form.name.read(cx).focus_handle(),
                form.purpose_hint_focus.clone(),
                form.purpose.read(cx).focus_handle(),
                form.goal_hint_focus.clone(),
                form.goal.read(cx).focus_handle(),
                form.cancel_focus.clone(),
                form.submit_focus.clone(),
            ]
        })
        .footer(footer)
        .into_any_element()
    }

    fn render_crew_delete_confirm(&self, cx: &mut Context<Self>) -> AnyElement {
        let confirm = self
            .crew_surfaces
            .delete_confirm
            .as_ref()
            .expect("crew delete confirm");
        let root = cx.entity();
        let confirm_root = root.clone();
        let cancel_root = root;
        ConfirmDialog::new(
            format!("Delete crew \"{}\" permanently?", confirm.name),
            "This removes all its slots and deletes its archived missions and session history. Crews with non-archived missions cannot be deleted until those missions are archived.",
            "Delete crew",
            "Deleting…",
            self.crew_surfaces.delete_busy,
            Rc::new(move |_, cx| {
                confirm_root.update(cx, |this, cx| this.advance_crew_delete(cx));
            }),
            Rc::new(move |_, cx| {
                cancel_root.update(cx, |this, cx| {
                    if !this.crew_surfaces.delete_busy {
                        this.crew_surfaces.delete_confirm = None;
                        cx.notify();
                    }
                });
            }),
        )
        .into_any_element()
    }

    fn open_add_slot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.crew_surfaces.add_slot.is_some() {
            return;
        }
        let Some(crew) = self.crew_surfaces.editor.crew.clone() else {
            return;
        };
        let existing_handles = self
            .crew_surfaces
            .editor
            .slots
            .iter()
            .map(|slot| slot.slot.slot_handle.clone())
            .collect::<HashSet<_>>();
        let runtimes =
            runner_backend::ops::runtime::runtime_catalog(self.core(cx)).unwrap_or_default();
        let query = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), "", "Search runners...", false).text_size(13.)
        });
        let slot_handle = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), "", "architect", true).text_size(14.)
        });
        slot_handle.update(cx, |input, input_cx| input.set_bare(true, input_cx));
        let model_override = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), "", "default", true).placeholder_as_value(true)
        });
        let model_field = cx.new(|model_cx| ModelField::new(model_override.clone(), &[], model_cx));
        let runtime_root = cx.entity();
        let runtime_select = cx.new(|select_cx| {
            StyledSelect::new(
                "add-slot-runtime",
                select_cx.focus_handle(),
                "",
                add_slot_runtime_options(&runtimes, None),
                Rc::new(move |value, _, cx| {
                    runtime_root.update(cx, |this, cx| this.select_add_slot_runtime(value, cx));
                }),
                select_cx,
            )
            .width(px(FIELD_WIDTH))
            .min_menu_width(px(FIELD_WIDTH))
        });
        let scroll = ScrollHandle::new();
        let owner = cx.entity_id();
        let scrollbar = cx.new(|_| Scrollbar::app(scroll.clone(), owner));
        let mut subscriptions = Vec::new();
        subscriptions.push(cx.observe(&query, |this, _, cx| {
            this.sync_add_slot_filter(cx);
        }));
        subscriptions.push(cx.observe(&slot_handle, |this, input, cx| {
            let text = input.read(cx).text().to_owned();
            let lowercase = text.to_lowercase();
            if text != lowercase {
                input.update(cx, |input, input_cx| input.set_text(lowercase, input_cx));
                return;
            }
            let Some(form) = this.crew_surfaces.add_slot.as_mut() else {
                return;
            };
            let empty = text.is_empty();
            let error = slot_handle_error(&text, &form.existing_handles);
            if form.slot_handle_empty != empty || form.slot_handle_error != error {
                form.slot_handle_empty = empty;
                form.slot_handle_error = error;
                cx.notify();
            }
        }));
        let focus = query.read(cx).focus_handle();
        self.crew_surfaces.add_slot = Some(AddSlotForm {
            crew_id: crew.id,
            crew_name: crew.name,
            existing_handles,
            runners: Vec::new(),
            runtimes,
            query,
            last_synced_query: String::new(),
            selected_runner_id: None,
            slot_handle,
            runtime_override: String::new(),
            model_override,
            model_field,
            runtime_select,
            scroll,
            scrollbar,
            slot_handle_hint_focus: cx.focus_handle(),
            runtime_hint_focus: cx.focus_handle(),
            model_hint_focus: cx.focus_handle(),
            close_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            submit_focus: cx.focus_handle(),
            slot_handle_empty: true,
            slot_handle_error: None,
            loading: true,
            submitting: false,
            error: None,
            _subscriptions: subscriptions,
        });
        focus.focus(window);
        self.load_add_slot_runners(cx);
        cx.notify();
    }

    fn load_add_slot_runners(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.crew_surfaces.add_slot.as_ref() else {
            return;
        };
        let crew_id = form.crew_id.clone();
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::runner::runner_list_with_activity(&core, 1, 1_000_000, "")
                .map(|page| page.items)
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                let Some(form) = this.crew_surfaces.add_slot.as_mut() else {
                    return;
                };
                if form.crew_id != crew_id {
                    return;
                }
                form.loading = false;
                match result {
                    Ok(runners) => {
                        form.runners = runners;
                        let query = form.query.read(cx).text().trim().to_lowercase();
                        form.last_synced_query = query.clone();
                        form.selected_runner_id = form
                            .runners
                            .iter()
                            .find(|runner| runner_matches(runner, &query))
                            .map(|runner| runner.runner.id.clone());
                        if !form.slot_handle.read(cx).edited() {
                            if let Some(runner) = selected_add_slot_runner(form) {
                                let suggestion = suggest_slot_handle(
                                    &runner.runner.handle,
                                    &form.existing_handles,
                                );
                                if form.slot_handle.read(cx).text() != suggestion {
                                    form.slot_handle.update(cx, |input, input_cx| {
                                        input.reset(suggestion, input_cx)
                                    });
                                }
                            }
                        }
                        let options = add_slot_runtime_options(
                            &form.runtimes,
                            selected_add_slot_runner(form),
                        );
                        form.runtime_select.update(cx, |select, select_cx| {
                            select.set_options(options, select_cx)
                        });
                    }
                    Err(error) => form.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn sync_add_slot_filter(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.crew_surfaces.add_slot.as_mut() else {
            return;
        };
        let query = form.query.read(cx).text().trim().to_lowercase();
        let query_changed = query != form.last_synced_query;
        if query_changed {
            form.last_synced_query = query.clone();
        }
        let selected_visible = form.selected_runner_id.as_ref().is_some_and(|selected| {
            form.runners
                .iter()
                .any(|runner| runner.runner.id == *selected && runner_matches(runner, &query))
        });
        if !query_changed && selected_visible {
            return;
        }
        let mut selection_changed = false;
        if !selected_visible {
            let next = form
                .runners
                .iter()
                .find(|runner| runner_matches(runner, &query))
                .map(|runner| runner.runner.id.clone());
            selection_changed = next != form.selected_runner_id;
            form.selected_runner_id = next;
            if selection_changed && !form.slot_handle.read(cx).edited() {
                let suggestion = selected_add_slot_runner(form)
                    .map(|runner| {
                        suggest_slot_handle(&runner.runner.handle, &form.existing_handles)
                    })
                    .unwrap_or_default();
                if form.slot_handle.read(cx).text() != suggestion {
                    form.slot_handle
                        .update(cx, |input, input_cx| input.reset(suggestion, input_cx));
                }
            }
            if selection_changed {
                let options =
                    add_slot_runtime_options(&form.runtimes, selected_add_slot_runner(form));
                form.runtime_select.update(cx, |select, select_cx| {
                    select.set_options(options, select_cx)
                });
            }
        }
        if query_changed || selection_changed {
            cx.notify();
        }
    }

    fn select_add_slot_runner(&mut self, runner_id: String, cx: &mut Context<Self>) {
        let Some(form) = self.crew_surfaces.add_slot.as_mut() else {
            return;
        };
        form.selected_runner_id = Some(runner_id);
        let suggestion = selected_add_slot_runner(form)
            .map(|runner| suggest_slot_handle(&runner.runner.handle, &form.existing_handles))
            .unwrap_or_default();
        form.slot_handle
            .update(cx, |input, input_cx| input.reset(suggestion, input_cx));
        let options = add_slot_runtime_options(&form.runtimes, selected_add_slot_runner(form));
        form.runtime_select.update(cx, |select, select_cx| {
            select.set_options(options, select_cx)
        });
        cx.notify();
    }

    fn select_add_slot_runtime(&mut self, value: String, cx: &mut Context<Self>) {
        let Some(form) = self.crew_surfaces.add_slot.as_mut() else {
            return;
        };
        form.runtime_override = value.clone();
        form.model_override
            .update(cx, |input, input_cx| input.reset("", input_cx));
        form.model_field.update(cx, |field, field_cx| {
            field.set_suggestions(runtime_models(&form.runtimes, &value), field_cx)
        });
        cx.notify();
    }

    fn close_add_slot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .crew_surfaces
            .add_slot
            .as_ref()
            .is_some_and(|form| form.submitting)
        {
            return;
        }
        self.crew_surfaces.add_slot = None;
        window.focus(&self.root_focus);
        cx.notify();
    }

    fn on_add_slot_key_down(
        &mut self,
        event: &KeyDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "enter"
            && self
                .crew_surfaces
                .add_slot
                .as_ref()
                .is_some_and(|form| !add_slot_form_is_composing(form, cx))
        {
            cx.stop_propagation();
            self.submit_add_slot(cx);
        }
    }

    fn create_runner_from_add_slot(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .crew_surfaces
            .add_slot
            .as_ref()
            .is_some_and(|form| form.submitting)
        {
            return;
        }
        self.crew_surfaces.add_slot = None;
        self.open_runners(window, cx);
        self.open_create_runner(window, cx);
    }

    fn submit_add_slot(&mut self, cx: &mut Context<Self>) {
        let Some(form) = self.crew_surfaces.add_slot.as_mut() else {
            return;
        };
        let handle = form.slot_handle.read(cx).text().to_owned();
        if !add_slot_can_submit(form) {
            return;
        }
        let runner_id = form.selected_runner_id.clone().expect("validated runner");
        form.submitting = true;
        form.error = None;
        let crew_id = form.crew_id.clone();
        let input = runner_backend::ops::slot::CreateSlotInput {
            crew_id: crew_id.clone(),
            runner_id,
            slot_handle: handle,
            runtime_override: trimmed_option(&form.runtime_override),
            model_override: (!form.runtime_override.is_empty())
                .then(|| trimmed_option(form.model_override.read(cx).text()))
                .flatten(),
        };
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::slot::slot_create(&core, input).map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok(_) => {
                        this.crew_surfaces.add_slot = None;
                        this.load_crew_editor(crew_id, cx);
                        this.load_crew_page(cx);
                        this.load_runner_page(cx);
                    }
                    Err(error) => {
                        if let Some(form) = this.crew_surfaces.add_slot.as_mut() {
                            form.submitting = false;
                            form.error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn render_add_slot_modal(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let form = self.crew_surfaces.add_slot.as_ref().expect("add slot form");
        let query = form.query.read(cx).text().trim().to_lowercase();
        let filtered = form
            .runners
            .iter()
            .filter(|runner| runner_matches(runner, &query))
            .cloned()
            .collect::<Vec<_>>();
        let submitting = form.submitting;
        let can_submit = add_slot_can_submit(form);
        let handle_error = form.slot_handle_error.clone();
        let root = cx.entity();
        let close_root = root.clone();
        let cancel_root = root.clone();
        let submit_root = root.clone();
        let create_root = root.clone();
        let create_key_root = root.clone();
        let title = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(2. / 16.))
                    .child(
                        div()
                            .text_size(rems(1.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Add slot"),
                    )
                    .child(
                        div()
                            .text_size(rems(12. / 16.))
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::muted())
                            .child(format!("crew: {}", form.crew_name)),
                    ),
            )
            .child(
                IconButton::new("close-add-slot", "close.svg")
                    .focus_handle(form.close_focus.clone())
                    .tooltip("Close add slot")
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        close_root.update(cx, |this, cx| this.close_add_slot(window, cx));
                    }),
            );
        let runner_rows = if form.loading {
            vec![div()
                .px_3()
                .py_3()
                .text_size(rems(12. / 16.))
                .text_color(theme::faint())
                .child("Loading runners...")
                .into_any_element()]
        } else if filtered.is_empty() {
            vec![div()
                .px_3()
                .py_3()
                .text_size(rems(12. / 16.))
                .text_color(theme::faint())
                .child(if form.runners.is_empty() {
                    "No runners yet. Create one first, then add it here."
                } else {
                    "No runners match this search."
                })
                .into_any_element()]
        } else {
            filtered
                .into_iter()
                .enumerate()
                .map(|(index, runner)| {
                    let selected =
                        form.selected_runner_id.as_deref() == Some(runner.runner.id.as_str());
                    let runner_id = runner.runner.id.clone();
                    let key_runner_id = runner_id.clone();
                    let select_root = cx.entity();
                    let key_root = select_root.clone();
                    div()
                        .id(("add-slot-runner", index))
                        .tab_index(0)
                        .w_full()
                        .flex()
                        .items_center()
                        .gap_3()
                        .border_b_1()
                        .border_color(theme::border())
                        .px_3()
                        .py(rems(10. / 16.))
                        .cursor_pointer()
                        .when(selected, |row| row.bg(theme::raised()))
                        .hover(|row| row.bg(theme::raised()))
                        .child(
                            div()
                                .w(rems(160. / 16.))
                                .truncate()
                                .font_family("JetBrains Mono")
                                .text_size(rems(13. / 16.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::accent())
                                .child(format!("@{}", runner.runner.handle)),
                        )
                        .child(
                            div()
                                .w_20()
                                .truncate()
                                .text_size(rems(11. / 16.))
                                .text_color(theme::muted())
                                .child(runner.runner.runtime.clone()),
                        )
                        .child(
                            div()
                                .min_w(px(0.))
                                .flex_1()
                                .truncate()
                                .text_size(rems(12. / 16.))
                                .text_color(theme::muted())
                                .child(format!(
                                    "{} · {}",
                                    crew_usage_label(&runner),
                                    runner_activity_label(&runner)
                                )),
                        )
                        .on_click(move |_, _, cx| {
                            select_root.update(cx, |this, cx| {
                                this.select_add_slot_runner(runner_id.clone(), cx)
                            });
                        })
                        .on_key_down(move |event: &KeyDownEvent, _, cx| {
                            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                cx.stop_propagation();
                                key_root.update(cx, |this, cx| {
                                    this.select_add_slot_runner(key_runner_id.clone(), cx)
                                });
                            }
                        })
                        .into_any_element()
                })
                .collect()
        };
        let handle_input = div()
            .w_full()
            .flex()
            .items_center()
            .rounded_sm()
            .border_1()
            .border_color(if handle_error.is_some() {
                theme::danger()
            } else {
                theme::border_strong()
            })
            .bg(theme::bg())
            .px(rems(10. / 16.))
            .py(rems(6. / 16.))
            .text_size(rems(14. / 16.))
            .child(
                div()
                    .pr_1()
                    .font_family("JetBrains Mono")
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::faint())
                    .child("@"),
            )
            .child(div().min_w(px(0.)).flex_1().child(form.slot_handle.clone()));
        let body = div()
            .flex()
            .flex_col()
            .gap_5()
            .on_key_down(cx.listener(Self::on_add_slot_key_down))
            .children(form.error.clone().map(error_banner))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(6. / 16.))
                    .child(
                        div()
                            .text_size(rems(12. / 16.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Runner"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .rounded(rems(6. / 16.))
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::bg())
                            .px_3()
                            .py_2()
                            .child(div().min_w(px(0.)).flex_1().child(form.query.clone()))
                            .child(
                                svg()
                                    .path("chevron-down.svg")
                                    .size(rems(14. / 16.))
                                    .text_color(theme::faint()),
                            ),
                    )
                    .child(
                        div()
                            .overflow_hidden()
                            .rounded(rems(6. / 16.))
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::panel())
                            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                            .child(
                                div()
                                    .id("add-slot-create-runner")
                                    .tab_index(0)
                                    .cursor_pointer()
                                    .border_b_1()
                                    .border_color(theme::border())
                                    .px_3()
                                    .py(rems(10. / 16.))
                                    .text_size(rems(13. / 16.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::accent())
                                    .hover(|row| row.bg(theme::raised()))
                                    .focus_visible(|row| row.bg(theme::raised()))
                                    .on_click(move |_, window, cx| {
                                        create_root.update(cx, |this, cx| {
                                            this.create_runner_from_add_slot(window, cx)
                                        });
                                    })
                                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                        if matches!(
                                            event.keystroke.key.as_str(),
                                            "enter" | "space"
                                        ) {
                                            cx.stop_propagation();
                                            create_key_root.update(cx, |this, cx| {
                                                this.create_runner_from_add_slot(window, cx)
                                            });
                                        }
                                    })
                                    .child("+ Create new runner..."),
                            )
                            .child(
                                div()
                                    .id("add-slot-runner-scroll")
                                    .max_h(rems(224. / 16.))
                                    .overflow_y_scroll()
                                    .children(runner_rows),
                            ),
                    ),
            )
            .child(
                Field::new("add-slot-handle", "Slot handle", handle_input)
                    .focus_target(form.slot_handle.read(cx).focus_handle())
                    .hint(
                        "in-crew identity used by mission events and stdin routing",
                        form.slot_handle_hint_focus.clone(),
                    )
                    .when_some(handle_error, |field, error| field.error(error)),
            )
            .child(
                Field::new("add-slot-runtime", "Runtime", form.runtime_select.clone())
                    .focus_target(form.runtime_select.read(cx).focus_handle())
                    .hint(
                        "engine this slot runs — overriding keeps the runner's persona but uses the runtime's default command and flags",
                        form.runtime_hint_focus.clone(),
                    ),
            )
            .children((!form.runtime_override.is_empty()
                && form.selected_runner_id.is_some())
            .then(|| {
                Field::new("add-slot-model", "Model", form.model_field.clone())
                    .focus_target(form.model_override.read(cx).focus_handle())
                    .hint(
                        "optional · default uses the selected runtime's own model",
                        form.model_hint_focus.clone(),
                    )
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .opacity(0.7)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_3()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div()
                                            .text_size(rems(12. / 16.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child("System prompt override"),
                                    )
                                    .child(
                                        div()
                                            .rounded_sm()
                                            .bg(theme::raised())
                                            .px(rems(6. / 16.))
                                            .py(rems(2. / 16.))
                                            .text_size(rems(10. / 16.))
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme::faint())
                                            .child("V0.X"),
                                    ),
                            )
                            .child(Tooltip::new(
                                "add-slot-system-prompt-tooltip",
                                "Per-slot prompt overrides land in v0.x",
                                div()
                                    .w_8()
                                    .h(rems(18. / 16.))
                                    .flex()
                                    .items_center()
                                    .rounded_full()
                                    .bg(theme::raised())
                                    .p(rems(2. / 16.))
                                    .child(
                                        div()
                                            .size(rems(14. / 16.))
                                            .rounded_full()
                                            .bg(theme::panel()),
                                    ),
                            )),
                    )
                    .child(
                        div()
                            .text_size(rems(11. / 16.))
                            .text_color(theme::muted())
                            .child("Uses the selected runner's default prompt. Per-slot overrides are not editable in the MVP."),
                    ),
            );
        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("cancel-add-slot", "Cancel")
                    .focus_handle(form.cancel_focus.clone())
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        cancel_root.update(cx, |this, cx| this.close_add_slot(window, cx));
                    }),
            )
            .child(
                Button::new(
                    "submit-add-slot",
                    if submitting { "Adding..." } else { "Add slot" },
                )
                .focus_handle(form.submit_focus.clone())
                .variant(ButtonVariant::Primary)
                .disabled(!can_submit)
                .on_press(move |_, cx| {
                    submit_root.update(cx, |this, cx| this.submit_add_slot(cx));
                }),
            );
        let modal_root = root;
        Modal::new(
            title,
            body,
            Rc::new(move |window, cx| {
                modal_root.update(cx, |this, cx| this.close_add_slot(window, cx));
            }),
        )
        .width(OverlayWidth::Custom(FORM_WIDTH))
        .busy(submitting)
        .focus_order(add_slot_focus_order(form, cx))
        .scrollbar(form.scroll.clone(), form.scrollbar.clone())
        .footer(footer)
        .into_any_element()
    }

    fn render_slot_list(
        &mut self,
        slots: Vec<SlotWithRunner>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if slots.is_empty() {
            return div()
                .rounded_lg()
                .border_1()
                .border_dashed()
                .border_color(theme::border_strong())
                .bg(theme::with_alpha(theme::panel(), 0.4))
                .px_5()
                .py_8()
                .text_center()
                .child(
                    div()
                        .text_size(rems(14. / 16.))
                        .text_color(theme::text())
                        .child("No slots yet."),
                )
                .child(
                    div()
                        .mt_1()
                        .text_size(rems(12. / 16.))
                        .text_color(theme::faint())
                        .child("Use + Add slot above — the first slot auto-assigns as LEAD."),
                )
                .into_any_element();
        }
        let total = slots.len();
        div()
            .w_full()
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap_2()
            .children(
                slots
                    .into_iter()
                    .enumerate()
                    .map(|(index, slot)| self.render_slot_row(slot, index, total, cx)),
            )
            .into_any_element()
    }

    fn render_slot_row(
        &self,
        slot: SlotWithRunner,
        index: usize,
        total: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let effective_runtime = slot
            .slot
            .runtime_override
            .as_deref()
            .unwrap_or(&slot.runner.runtime)
            .to_owned();
        let runtime_overridden =
            slot.slot.runtime_override.is_some() && effective_runtime != slot.runner.runtime;
        let summary = slot_command_summary(&slot);
        let draggable = total > 1 && !self.crew_surfaces.editor.reordering;
        let active_drop = self.crew_surfaces.editor.drop_target == Some(index)
            && self.crew_surfaces.editor.dragged_slot_id.as_deref() != Some(slot.slot.id.as_str());
        let drag_handle = div()
            .flex_none()
            .text_size(rems(14. / 16.))
            .text_color(theme::faint())
            .opacity(if draggable { 1. } else { 0.4 })
            .cursor(if draggable {
                CursorStyle::OpenHand
            } else {
                CursorStyle::Arrow
            })
            .child("⋮⋮");
        let drag_handle = if draggable {
            Tooltip::new(
                SharedString::from(format!("crew-slot-drag-tooltip-{}", slot.slot.id)),
                "Drag to reorder",
                drag_handle,
            )
            .into_any_element()
        } else {
            drag_handle.into_any_element()
        };
        let menu_slot = slot.clone();
        let menu_root = cx.entity();
        let mut row = div()
            .id(SharedString::from(format!("crew-slot-{}", slot.slot.id)))
            .group("slot-row")
            .w_full()
            .min_w(px(0.))
            .overflow_hidden()
            .flex()
            .items_center()
            .gap_4()
            .rounded_lg()
            .border_1()
            .border_color(if active_drop {
                theme::with_alpha(theme::accent(), 0.5)
            } else {
                theme::border()
            })
            .bg(if active_drop {
                theme::with_alpha(theme::accent(), 0.05)
            } else {
                theme::panel()
            })
            .p_4()
            .hover(|row| row.border_color(theme::border_strong()))
            .child(drag_handle)
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .font_family("JetBrains Mono")
                                    .text_size(rems(13. / 16.))
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(format!("@{}", slot.slot.slot_handle)),
                            )
                            .children(slot.slot.lead.then(|| {
                                div()
                                    .rounded_sm()
                                    .bg(theme::with_alpha(theme::accent(), 0.1))
                                    .px(rems(6. / 16.))
                                    .py(rems(2. / 16.))
                                    .text_size(rems(10. / 16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::accent())
                                    .child("LEAD")
                            }))
                            .child(Tooltip::new(
                                SharedString::from(format!(
                                    "slot-runtime-tooltip-{}",
                                    slot.slot.id
                                )),
                                if runtime_overridden {
                                    format!(
                                        "Runtime override — runner default is {}",
                                        slot.runner.runtime
                                    )
                                } else {
                                    "Runtime (runner default)".to_owned()
                                },
                                RuntimeBadge::new(effective_runtime).overridden(runtime_overridden),
                            ))
                            .child(
                                div()
                                    .font_family("JetBrains Mono")
                                    .text_size(rems(11. / 16.))
                                    .text_color(theme::faint())
                                    .child(format!("from @{}", slot.runner.handle)),
                            ),
                    )
                    .children(slot.runner.system_prompt.clone().map(|prompt| {
                        let prompt = prompt.split_whitespace().collect::<Vec<_>>().join(" ");
                        div()
                            .w_full()
                            .min_w(px(0.))
                            .mt_1()
                            .truncate()
                            .text_size(rems(12. / 16.))
                            .line_height(rems(1.))
                            .text_color(theme::muted())
                            .child(prompt)
                    }))
                    .children((!summary.is_empty()).then(|| {
                        div()
                            .mt_1()
                            .truncate()
                            .font_family("JetBrains Mono")
                            .text_size(rems(11. / 16.))
                            .text_color(theme::faint())
                            .child(format!("$ {summary}"))
                    })),
            )
            .child(
                div().flex_none().child(
                    IconButton::new(
                        SharedString::from(format!("slot-actions-{}", slot.slot.id)),
                        "more-horizontal.svg",
                    )
                    .size(IconButtonSize::Md)
                    .tooltip("Slot actions")
                    .on_press(move |window, cx| {
                        let position = window.mouse_position();
                        let slot = menu_slot.clone();
                        menu_root.update(cx, |this, cx| {
                            this.open_slot_menu(slot, position, window, cx)
                        });
                    }),
                ),
            );
        if draggable {
            let drag = SlotDrag {
                slot_id: slot.slot.id.clone(),
                label: format!("@{}", slot.slot.slot_handle),
            };
            let drag_root = cx.entity();
            row = row
                .cursor_move()
                .on_drag(drag, move |drag: &SlotDrag, _, _, cx| {
                    drag_root.update(cx, |this, cx| {
                        this.crew_surfaces.editor.dragged_slot_id = Some(drag.slot_id.clone());
                        cx.notify();
                    });
                    cx.new(|_| drag.clone())
                })
                .on_drag_move::<SlotDrag>(cx.listener(
                    move |this, event: &DragMoveEvent<SlotDrag>, _, cx| {
                        if event.bounds.contains(&event.event.position) {
                            this.crew_surfaces.editor.drop_target = Some(index);
                            cx.notify();
                        }
                    },
                ))
                .on_drop(cx.listener(move |this, drag: &SlotDrag, _, cx| {
                    this.commit_slot_reorder(&drag.slot_id, index, cx);
                }));
        }
        row.into_any_element()
    }

    fn open_slot_menu(
        &mut self,
        slot: SlotWithRunner,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let actions = [
            SlotMenuAction::SetLead(slot.slot.id.clone()),
            SlotMenuAction::Edit(slot.clone()),
            SlotMenuAction::Remove(slot.clone()),
        ];
        let items = vec![
            UiMenuItem::new(if slot.slot.lead {
                "Current lead"
            } else {
                "Set as lead"
            })
            .icon("star.svg")
            .disabled(slot.slot.lead),
            UiMenuItem::new("Edit runner").icon("square-pen.svg"),
            UiMenuItem::new("Remove from crew")
                .icon("trash.svg")
                .separator_before(true)
                .destructive(true),
        ];
        let root = cx.entity();
        let dismiss_root = root.clone();
        let menu = cx.new(move |menu_cx| {
            let action_root = root;
            ContextMenu::new(
                "slot-context-menu",
                menu_cx.focus_handle(),
                position,
                items,
                Rc::new(move |index, window, cx| {
                    if let Some(action) = actions.get(index).cloned() {
                        action_root.update(cx, |this, cx| {
                            this.handle_slot_menu_action(action, window, cx)
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
            .width(px(208.))
        });
        let focus = menu.read(cx).focus_handle();
        self.crew_surfaces.context_menu = Some(menu);
        focus.focus(window);
        cx.notify();
    }

    fn handle_slot_menu_action(
        &mut self,
        action: SlotMenuAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            SlotMenuAction::SetLead(slot_id) => self.set_crew_lead(slot_id, cx),
            SlotMenuAction::Edit(slot) => {
                self.open_runner_edit(slot.runner.clone(), Some(slot), window, cx)
            }
            SlotMenuAction::Remove(slot) => {
                self.crew_surfaces.slot_remove_confirm = Some(SlotRemoveConfirm { slot });
                cx.notify();
            }
        }
    }

    fn set_crew_lead(&mut self, slot_id: String, cx: &mut Context<Self>) {
        let crew_id = self.crew_surfaces.editor.crew_id.clone();
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            let result = runner_backend::ops::slot::slot_set_lead(&core, &slot_id)
                .map_err(|error| error.to_string());
            (crew_id, result)
        });
        cx.spawn(async move |weak, cx| {
            let (crew_id, result) = task.await;
            let _ = weak.update(cx, |this, cx| {
                if !matches!(
                    &this.route,
                    AppRoute::CrewEditor(active) if active == &crew_id
                ) {
                    return;
                }
                match result {
                    Ok(_) => this.load_crew_editor(crew_id, cx),
                    Err(error) => this.crew_surfaces.editor.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn clear_crew_slot_drag(&mut self, cx: &mut Context<Self>) {
        let editor = &mut self.crew_surfaces.editor;
        if editor.dragged_slot_id.is_none() && editor.drop_target.is_none() {
            return;
        }
        editor.dragged_slot_id = None;
        editor.drop_target = None;
        cx.notify();
    }

    fn commit_slot_reorder(&mut self, slot_id: &str, to: usize, cx: &mut Context<Self>) {
        if self.crew_surfaces.editor.reordering {
            return;
        }
        let Some(from) = self
            .crew_surfaces
            .editor
            .slots
            .iter()
            .position(|slot| slot.slot.id == slot_id)
        else {
            return;
        };
        self.crew_surfaces.editor.dragged_slot_id = None;
        self.crew_surfaces.editor.drop_target = None;
        if from == to {
            cx.notify();
            return;
        }
        let reordered = move_item(&self.crew_surfaces.editor.slots, from, to);
        let ordered_ids = reordered
            .iter()
            .map(|slot| slot.slot.id.clone())
            .collect::<Vec<_>>();
        let crew_id = self.crew_surfaces.editor.crew_id.clone();
        self.crew_surfaces.editor.slots = reordered;
        self.crew_surfaces.editor.reordering = true;
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            let requested_crew_id = crew_id.clone();
            runner_backend::ops::slot::slot_reorder(&core, &crew_id, ordered_ids)
                .map(|slots| (crew_id, slots))
                .map_err(|error| (requested_crew_id, error.to_string()))
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok((crew_id, slots)) => {
                        if this.crew_surfaces.editor.crew_id != crew_id {
                            return;
                        }
                        this.crew_surfaces.editor.reordering = false;
                        this.crew_surfaces.editor.slots = slots;
                        this.load_crew_page(cx);
                    }
                    Err((crew_id, error)) => {
                        if this.crew_surfaces.editor.crew_id != crew_id {
                            return;
                        }
                        this.crew_surfaces.editor.reordering = false;
                        if matches!(
                            &this.route,
                            AppRoute::CrewEditor(active) if active == &crew_id
                        ) {
                            this.load_crew_editor(crew_id, cx);
                        }
                        this.crew_surfaces.editor.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn render_slot_remove_confirm(&self, cx: &mut Context<Self>) -> AnyElement {
        let confirm = self
            .crew_surfaces
            .slot_remove_confirm
            .as_ref()
            .expect("slot remove confirm");
        let root = cx.entity();
        let confirm_root = root.clone();
        let cancel_root = root;
        let body = if confirm.slot.slot.lead {
            "As the LEAD, leadership will pass to the next slot by position."
        } else {
            ""
        };
        ConfirmDialog::new(
            format!(
                "Remove slot @{} from this crew?",
                confirm.slot.slot.slot_handle,
            ),
            body,
            "Remove from crew",
            "Removing…",
            self.crew_surfaces.slot_remove_busy,
            Rc::new(move |_, cx| {
                confirm_root.update(cx, |this, cx| this.confirm_slot_remove(cx));
            }),
            Rc::new(move |_, cx| {
                cancel_root.update(cx, |this, cx| {
                    if !this.crew_surfaces.slot_remove_busy {
                        this.crew_surfaces.slot_remove_confirm = None;
                        cx.notify();
                    }
                });
            }),
        )
        .into_any_element()
    }

    fn confirm_slot_remove(&mut self, cx: &mut Context<Self>) {
        let Some(confirm) = self.crew_surfaces.slot_remove_confirm.as_ref() else {
            return;
        };
        if self.crew_surfaces.slot_remove_busy {
            return;
        }
        self.crew_surfaces.slot_remove_busy = true;
        let slot_id = confirm.slot.slot.id.clone();
        let crew_id = confirm.slot.slot.crew_id.clone();
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            let result = runner_backend::ops::slot::slot_delete(&core, &slot_id)
                .map_err(|error| error.to_string());
            (crew_id, result)
        });
        cx.spawn(async move |weak, cx| {
            let (crew_id, result) = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.crew_surfaces.slot_remove_busy = false;
                this.crew_surfaces.slot_remove_confirm = None;
                match result {
                    Ok(()) => {
                        if matches!(
                            &this.route,
                            AppRoute::CrewEditor(active) if active == &crew_id
                        ) {
                            this.load_crew_editor(crew_id, cx);
                        }
                        this.load_crew_page(cx);
                        this.load_runner_page(cx);
                    }
                    Err(error)
                        if matches!(
                            &this.route,
                            AppRoute::CrewEditor(active) if active == &crew_id
                        ) =>
                    {
                        this.crew_surfaces.editor.error = Some(error);
                    }
                    Err(_) => {}
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn advance_crew_delete(&mut self, cx: &mut Context<Self>) {
        let Some(confirm) = self.crew_surfaces.delete_confirm.as_ref() else {
            return;
        };
        if self.crew_surfaces.delete_busy {
            return;
        }
        self.crew_surfaces.delete_busy = true;
        let id = confirm.id.clone();
        let name = confirm.name.clone();
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::crew::crew_delete(&core, &id).map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.crew_surfaces.delete_busy = false;
                this.crew_surfaces.delete_confirm = None;
                match result {
                    Ok(()) => {
                        this.load_crew_page(cx);
                        this.show_toast(
                            format!("Deleted crew \"{name}\"."),
                            crate::toast::ToastTone::Success,
                            cx,
                        );
                    }
                    Err(error) => {
                        this.show_toast(error, crate::toast::ToastTone::Error, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn render_crew_overlays(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut overlays = Vec::new();
        if self.crew_surfaces.create.is_some() {
            overlays.push(self.render_create_crew_modal(cx));
        }
        if self.crew_surfaces.add_slot.is_some() {
            overlays.push(self.render_add_slot_modal(cx));
        }
        if self.crew_surfaces.delete_confirm.is_some() {
            overlays.push(self.render_crew_delete_confirm(cx));
        }
        if self.crew_surfaces.slot_remove_confirm.is_some() {
            overlays.push(self.render_slot_remove_confirm(cx));
        }
        overlays
    }

    fn finish_crew_update(
        &mut self,
        task: gpui::Task<(String, Result<(), String>)>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |weak, cx| {
            let (crew_id, result) = task.await;
            let _ = weak.update(cx, |this, cx| {
                if this.crew_surfaces.editor.crew_id != crew_id {
                    return;
                }
                let reload_editor = matches!(
                    &this.route,
                    AppRoute::CrewEditor(active) if active == &crew_id
                );
                match result {
                    Ok(()) => {
                        let editor = &mut this.crew_surfaces.editor;
                        if editor.saving_goal {
                            editor.goal_edit = None;
                        }
                        if editor.saving_conventions {
                            editor.conventions_edit = None;
                        }
                        editor.saving_name = false;
                        editor.saving_goal = false;
                        editor.saving_conventions = false;
                        if reload_editor {
                            this.load_crew_editor(crew_id, cx);
                        }
                        this.load_crew_page(cx);
                    }
                    Err(error) => {
                        let editor = &mut this.crew_surfaces.editor;
                        editor.saving_name = false;
                        editor.saving_goal = false;
                        editor.saving_conventions = false;
                        editor.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

fn section_label(label: &'static str) -> AnyElement {
    div()
        .text_size(rems(10. / 16.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(theme::faint())
        .child(label.to_uppercase())
        .into_any_element()
}

fn slot_section_description() -> StyledText {
    let description = "Positions in the crew. Each slot binds a handle to a runner. The LEAD is the crew's face — receives human messages by default and dispatches back to other slots.";
    let lead_start = description
        .find("LEAD")
        .expect("slot description contains LEAD");
    StyledText::new(description).with_highlights([(
        lead_start..lead_start + 4,
        HighlightStyle {
            color: Some(theme::accent()),
            font_weight: Some(FontWeight::SEMIBOLD),
            ..Default::default()
        },
    )])
}

fn crew_name_state(value: &str, original: &str) -> (bool, bool, bool) {
    let trimmed = value.trim();
    (
        value != original,
        !trimmed.is_empty() && trimmed != original,
        trimmed.is_empty(),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CrewNameRefresh {
    MarkClean,
    Reset,
    Preserve,
}

fn crew_name_refresh(current: &str, persisted: &str, edited: bool) -> CrewNameRefresh {
    if current == persisted {
        CrewNameRefresh::MarkClean
    } else if edited {
        CrewNameRefresh::Preserve
    } else {
        CrewNameRefresh::Reset
    }
}

fn text_action(
    id: &'static str,
    label: &'static str,
    on_click: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let on_click = Rc::new(on_click);
    let key_click = Rc::clone(&on_click);
    div()
        .id(id)
        .tab_index(0)
        .cursor_pointer()
        .text_size(rems(12. / 16.))
        .text_color(theme::muted())
        .hover(|text| text.text_color(theme::text()))
        .focus_visible(|text| text.text_color(theme::text()).underline())
        .on_click(move |_, window, cx| on_click(window, cx))
        .on_key_down(move |event: &KeyDownEvent, window, cx| {
            if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                cx.stop_propagation();
                key_click(window, cx);
            }
        })
        .child(label)
        .into_any_element()
}

fn error_panel(error: String) -> AnyElement {
    div()
        .rounded_sm()
        .border_1()
        .border_color(theme::with_alpha(theme::danger(), 0.4))
        .bg(theme::with_alpha(theme::danger(), 0.1))
        .px_3()
        .py_2()
        .text_size(rems(14. / 16.))
        .text_color(theme::danger())
        .child(error)
        .into_any_element()
}

fn error_banner(error: String) -> AnyElement {
    div()
        .rounded_sm()
        .border_1()
        .border_color(theme::with_alpha(theme::danger(), 0.4))
        .bg(theme::with_alpha(theme::danger(), 0.1))
        .px_3()
        .py_2()
        .text_size(rems(12. / 16.))
        .text_color(theme::danger())
        .child(error)
        .into_any_element()
}

fn selected_add_slot_runner(form: &AddSlotForm) -> Option<&RunnerWithActivity> {
    let selected = form.selected_runner_id.as_deref()?;
    form.runners
        .iter()
        .find(|runner| runner.runner.id == selected)
}

fn runner_matches(runner: &RunnerWithActivity, query: &str) -> bool {
    query.is_empty()
        || runner.runner.handle.to_lowercase().contains(query)
        || runner.runner.display_name.to_lowercase().contains(query)
        || runner.runner.runtime.to_lowercase().contains(query)
}

fn suggest_slot_handle(base: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(base) {
        return base.to_owned();
    }
    (2..100)
        .map(|index| format!("{base}-{index}"))
        .find(|candidate| !taken.contains(candidate))
        .unwrap_or_else(|| base.to_owned())
}

fn add_slot_runtime_options(
    runtimes: &[RuntimeCatalogEntry],
    selected: Option<&RunnerWithActivity>,
) -> Vec<SelectOption> {
    let default = selected
        .map(|runner| format!("Runner default ({})", runner.runner.runtime))
        .unwrap_or_else(|| "Runner default".into());
    let mut options = vec![
        SelectOption::new("", default).description("Use the runtime configured on the runner.")
    ];
    options.extend(runtimes.iter().map(|runtime| {
        SelectOption::new(runtime.name.clone(), runtime.display_name.clone())
            .description(runtime.description.clone())
    }));
    options
}

fn runtime_models<'a>(
    runtimes: &'a [RuntimeCatalogEntry],
    name: &str,
) -> &'a [RuntimeCatalogOption] {
    runtimes
        .iter()
        .find(|runtime| runtime.name == name)
        .map(|runtime| runtime.models.as_slice())
        .unwrap_or_default()
}

fn validate_slot_handle(handle: &str) -> Option<&'static str> {
    if handle.is_empty() {
        return None;
    }
    let bytes = handle.as_bytes();
    let valid = bytes.len() <= 32
        && bytes
            .first()
            .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        });
    (!valid).then_some(
        "Lowercase letters, digits, '-' or '_'; must start with a letter or digit; up to 32 chars.",
    )
}

fn slot_handle_error(handle: &str, existing_handles: &HashSet<String>) -> Option<String> {
    validate_slot_handle(handle)
        .map(ToOwned::to_owned)
        .or_else(|| {
            existing_handles
                .contains(handle)
                .then(|| format!("'{handle}' is already used in this crew."))
        })
}

fn add_slot_can_submit(form: &AddSlotForm) -> bool {
    !form.loading
        && !form.submitting
        && selected_add_slot_runner(form).is_some()
        && !form.slot_handle_empty
        && form.slot_handle_error.is_none()
}

fn add_slot_focus_order(form: &AddSlotForm, cx: &Context<NativeRoot>) -> Vec<FocusHandle> {
    if form.submitting {
        return Vec::new();
    }
    let mut order = vec![
        form.close_focus.clone(),
        form.query.read(cx).focus_handle(),
        form.slot_handle_hint_focus.clone(),
        form.slot_handle.read(cx).focus_handle(),
        form.runtime_hint_focus.clone(),
        form.runtime_select.read(cx).focus_handle(),
    ];
    if !form.runtime_override.is_empty() {
        order.extend([
            form.model_hint_focus.clone(),
            form.model_override.read(cx).focus_handle(),
        ]);
    }
    order.extend([form.cancel_focus.clone(), form.submit_focus.clone()]);
    order
}

fn crew_usage_label(runner: &RunnerWithActivity) -> String {
    if runner.activity.crew_count == 1 {
        "in 1 crew".into()
    } else {
        format!("in {} crews", runner.activity.crew_count)
    }
}

fn runner_activity_label(runner: &RunnerWithActivity) -> String {
    if runner.activity.active_sessions > 0 {
        if runner.activity.active_sessions == 1 {
            "1 session".into()
        } else {
            format!("{} sessions", runner.activity.active_sessions)
        }
    } else if runner.activity.active_missions > 0 {
        if runner.activity.active_missions == 1 {
            "1 mission".into()
        } else {
            format!("{} missions", runner.activity.active_missions)
        }
    } else {
        "idle".into()
    }
}

fn slot_command_summary(slot: &SlotWithRunner) -> String {
    if let Some(runtime) = slot
        .slot
        .runtime_override
        .as_deref()
        .filter(|runtime| *runtime != slot.runner.runtime)
    {
        let command = runner_backend::ops::runtime::runtime_list()
            .into_iter()
            .find(|entry| entry.name == runtime)
            .map(|entry| entry.command)
            .unwrap_or_else(|| runtime.to_owned());
        let mut overrides = Vec::new();
        if let Some(model) = slot.slot.model_override.as_deref() {
            overrides.push(format!("model {model}"));
        }
        if let Some(effort) = slot.slot.effort_override.as_deref() {
            overrides.push(format!("effort {effort}"));
        }
        return if overrides.is_empty() {
            format!("{command} (runtime defaults)")
        } else {
            format!("{command} (runtime defaults · {})", overrides.join(" · "))
        };
    }
    let mut command = vec![slot.runner.command.clone()];
    command.extend(slot.runner.args.clone());
    let command = command
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let mut overrides = Vec::new();
    if let Some(model) = slot.slot.model_override.as_deref() {
        overrides.push(format!("model {model}"));
    }
    if let Some(effort) = slot.slot.effort_override.as_deref() {
        overrides.push(format!("effort {effort}"));
    }
    if overrides.is_empty() {
        command
    } else {
        format!("{command} ({})", overrides.join(" · "))
    }
}

fn move_item<T: Clone>(items: &[T], from: usize, to: usize) -> Vec<T> {
    let mut reordered = items.to_vec();
    if from >= reordered.len() || to >= reordered.len() || from == to {
        return reordered;
    }
    let item = reordered.remove(from);
    reordered.insert(to, item);
    reordered
}

fn trimmed_option(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn create_crew_form_is_composing(form: &CreateCrewForm, cx: &Context<NativeRoot>) -> bool {
    form.name.read(cx).is_composing()
        || form.purpose.read(cx).is_composing()
        || form.goal.read(cx).is_composing()
}

fn add_slot_form_is_composing(form: &AddSlotForm, cx: &Context<NativeRoot>) -> bool {
    form.query.read(cx).is_composing()
        || form.slot_handle.read(cx).is_composing()
        || form.model_override.read(cx).is_composing()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use runner_backend::model::{Runner, Slot};

    fn slot_with_runner(
        runtime_override: Option<&str>,
        model_override: Option<&str>,
        effort_override: Option<&str>,
    ) -> SlotWithRunner {
        let now = Utc::now();
        SlotWithRunner {
            slot: Slot {
                id: "slot".into(),
                crew_id: "crew".into(),
                runner_id: "runner".into(),
                slot_handle: "coder".into(),
                position: 0,
                lead: true,
                runtime_override: runtime_override.map(str::to_owned),
                model_override: model_override.map(str::to_owned),
                effort_override: effort_override.map(str::to_owned),
                added_at: now,
            },
            runner: Runner {
                id: "runner".into(),
                handle: "coder".into(),
                display_name: "Coder".into(),
                runtime: "codex".into(),
                command: "codex".into(),
                args: vec!["--quiet".into()],
                working_dir: None,
                system_prompt: None,
                env: Default::default(),
                model: None,
                effort: None,
                created_at: now,
                updated_at: now,
            },
        }
    }

    #[test]
    fn slot_handle_validation_matches_the_shipped_contract() {
        for valid in ["", "a", "0", "coder-2", "coder_2", &"a".repeat(32)] {
            assert_eq!(validate_slot_handle(valid), None, "{valid}");
        }
        for invalid in ["Coder", "-coder", "_coder", "coder!", &"a".repeat(33)] {
            assert!(validate_slot_handle(invalid).is_some(), "{invalid}");
        }
    }

    #[test]
    fn slot_handle_suggestions_take_the_first_available_suffix() {
        let taken = HashSet::from([
            "coder".to_owned(),
            "coder-2".to_owned(),
            "coder-3".to_owned(),
        ]);

        assert_eq!(suggest_slot_handle("reviewer", &taken), "reviewer");
        assert_eq!(suggest_slot_handle("coder", &taken), "coder-4");
    }

    #[test]
    fn crew_name_state_tracks_saved_reverted_and_empty_edits() {
        assert_eq!(crew_name_state("Crew", "Crew"), (false, false, false));
        assert_eq!(crew_name_state("New crew", "Crew"), (true, true, false));
        assert_eq!(crew_name_state(" Crew ", "Crew"), (true, false, false));
        assert_eq!(crew_name_state("  ", "Crew"), (true, false, true));
    }

    #[test]
    fn crew_name_refresh_preserves_live_edits_without_orphaning_the_field() {
        assert_eq!(
            crew_name_refresh("Draft", "Crew", true),
            CrewNameRefresh::Preserve
        );
        assert_eq!(
            crew_name_refresh("Old", "New", false),
            CrewNameRefresh::Reset
        );
        assert_eq!(
            crew_name_refresh("Saved", "Saved", true),
            CrewNameRefresh::MarkClean
        );
    }

    #[test]
    fn slot_command_summary_applies_runtime_and_model_effort_layers() {
        assert_eq!(
            slot_command_summary(&slot_with_runner(None, None, None)),
            "codex --quiet"
        );
        assert_eq!(
            slot_command_summary(&slot_with_runner(
                Some("codex"),
                Some("gpt-5"),
                Some("high")
            )),
            "codex --quiet (model gpt-5 · effort high)"
        );
        assert_eq!(
            slot_command_summary(&slot_with_runner(Some("claude-code"), None, None)),
            "claude (runtime defaults)"
        );
        assert_eq!(
            slot_command_summary(&slot_with_runner(
                Some("claude-code"),
                Some("opus"),
                Some("max")
            )),
            "claude (runtime defaults · model opus · effort max)"
        );
    }

    #[test]
    fn move_item_reorders_slots_in_both_directions() {
        assert_eq!(move_item(&["a", "b", "c", "d"], 1, 3), ["a", "c", "d", "b"]);
        assert_eq!(move_item(&["a", "b", "c", "d"], 3, 1), ["a", "d", "b", "c"]);
    }

    #[test]
    fn move_item_ignores_same_or_invalid_positions() {
        assert_eq!(move_item(&[1, 2, 3], 1, 1), [1, 2, 3]);
        assert_eq!(move_item(&[1, 2, 3], 8, 1), [1, 2, 3]);
        assert_eq!(move_item(&[1, 2, 3], 1, 8), [1, 2, 3]);
    }
}
