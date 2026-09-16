mod add_slot;
mod create;
mod editor;
mod editor_sections;
mod list;
mod logic;
mod overlays;
mod slots;
#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{div, rems, Context, Entity, FocusHandle, ScrollHandle, Subscription, Window};
use runner_app::ui::{ContextMenu, ModelField, Scrollbar, SearchInput, StyledSelect, TextField};
use runner_backend::model::{Crew, SlotWithRole};
use runner_backend::ops::crew::CrewListItem;
use runner_backend::ops::role::RoleWithActivity;
use runner_backend::ops::runtime::RuntimeCatalogEntry;

use crate::list_controls::ListControls;
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
            .font_family(theme::UI_MONOSPACE_FONT)
            .text_size(theme::text_body())
            .text_color(theme::text())
            .child(self.label.clone())
    }
}

#[derive(Default)]
struct CrewEditorState {
    crew_id: String,
    crew: Option<Crew>,
    slots: Vec<SlotWithRole>,
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
    roles: Vec<RoleWithActivity>,
    runtimes: Vec<RuntimeCatalogEntry>,
    query: Entity<TextField>,
    last_synced_query: String,
    selected_role_id: Option<String>,
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
    Edit(SlotWithRole),
    Remove(SlotWithRole),
}

struct CrewDeleteConfirm {
    id: String,
    name: String,
}

struct SlotRemoveConfirm {
    slot: SlotWithRole,
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
