mod add_slot;
mod create;
mod editor;
mod editor_sections;
mod list;
mod logic;
mod overlays;
mod popup;
mod slots;
#[cfg(test)]
mod tests;

use std::cell::Cell;
use std::collections::HashSet;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, rems, Bounds, Context, Entity, FocusHandle, Pixels, ScrollHandle, Subscription, Window,
};
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
    /// Edit in place: the name and conventions, saved together.
    edit: Option<CrewEditForm>,
    /// The conventions card shows the whole text instead of its first lines.
    conventions_expanded: bool,
    /// The conventions editor shows the rendered draft.
    conventions_preview: bool,
    /// The missions card lists every mission instead of the latest four.
    missions_expanded: bool,
    popup: Option<SlotPopup>,
    reordering: bool,
    dragged_slot_id: Option<String>,
    drop_target: Option<usize>,
}

struct CrewEditForm {
    name: Entity<TextField>,
    conventions: Entity<TextField>,
    saving: bool,
    _subscriptions: Vec<Subscription>,
}

/// The popover beside a clicked slot row.
struct SlotPopup {
    slot_id: String,
    /// The row's bounds, which its canvas refreshes every frame.
    anchor: Rc<Cell<Bounds<Pixels>>>,
    focus: FocusHandle,
    open_role_focus: FocusHandle,
    remove_focus: FocusHandle,
    edit: Option<SlotOverrideForm>,
}

/// Edit overrides saved to the slot.
struct SlotOverrideForm {
    runtimes: Vec<RuntimeCatalogEntry>,
    /// The runtime override; `None` runs the role's runtime.
    runtime: Option<String>,
    runtime_select: Entity<StyledSelect>,
    model: Entity<TextField>,
    model_field: Entity<ModelField>,
    /// The effort override; empty inherits.
    effort: String,
    effort_select: Entity<StyledSelect>,
    speed: Option<runner_backend::model::CodexSpeed>,
    speed_select: Entity<StyledSelect>,
    /// Reset for runtime, model, effort and Speed.
    reset_focus: [FocusHandle; 4],
    saving: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

struct CreateCrewForm {
    name: Entity<TextField>,
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

struct CrewDeleteConfirm {
    id: String,
    name: String,
}

struct SlotRemoveConfirm {
    slot: SlotWithRole,
}

pub(crate) struct CrewSurfaces {
    list: ListControls<CrewListItem>,
    /// The list row under the pointer, which trades its play icon for a button.
    hovered_row: Option<String>,
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
                "Search crews, slots and roles",
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
            hovered_row: None,
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
