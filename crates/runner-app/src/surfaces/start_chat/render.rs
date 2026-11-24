use super::*;
impl NativeRoot {
    pub(crate) fn render_start_chat_modal(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.render_add_role_modal(
            self.start_chat_modal
                .as_ref()
                .expect("add-role form is open"),
            window,
            cx,
        )
    }
    /// Puts one role control back to the role's value and hands focus to it,
    /// since its Reset goes with the override.
    pub(super) fn reset_start_chat_control(
        &mut self,
        kind: ResetKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(modal) = self.start_chat_modal.as_mut() else {
            return;
        };
        if modal.submitting || modal.loading_catalog() {
            return;
        }
        let focus = match kind {
            ResetKind::Runtime => {
                modal.role_runtime_override = None;
                sync_role_runtime_select(modal, cx);
                restore_baseline(modal, cx);
                modal.role_runtime_select.read(cx).focus_handle()
            }
            ResetKind::Model => {
                modal
                    .model
                    .update(cx, |input, input_cx| input.reset("", input_cx));
                modal.model.read(cx).focus_handle()
            }
            ResetKind::Effort => {
                modal.effort = modal.baseline_effort();
                sync_effort_control(modal, cx);
                modal.effort_select.read(cx).focus_handle()
            }
            ResetKind::Speed => {
                modal.speed = modal.baseline_speed();
                sync_speed_control(modal, cx);
                modal.speed_select.read(cx).focus_handle()
            }
        };
        focus.focus(window, cx);
        if kind == ResetKind::Runtime {
            if let Some(modal) = self.start_chat_modal.as_ref() {
                if !modal.is_remote() && self.remote_client.is_local() {
                    if let Some(runtime) = modal.active_runtime() {
                        self.request_model_catalog(runtime.name.key(), cx);
                    }
                }
            }
        }
        cx.notify();
    }
}
/// Column widths inside a card, from the width the modal leaves its form.
#[derive(Clone, Copy)]
pub(super) struct CardLayout {
    /// The card's own width, border included.
    pub(super) outer: f32,
    /// The width inside the border and the card's padding.
    pub(super) content: f32,
}

impl CardLayout {
    pub(super) fn new(form_width: f32) -> Self {
        Self {
            outer: form_width.floor(),
            content: form_width.floor() - 2. - 2. * CARD_PADDING,
        }
    }

    /// Runtime, Model and Effort of a role chat.
    pub(super) fn third(self) -> f32 {
        ((self.content - 2. * COLUMN_GAP) / 3.).floor()
    }
}

/// A role or an agent: who the chat is with on top, how it runs underneath.
pub(super) fn card(layout: CardLayout) -> Div {
    div()
        .when(cfg!(test), |card| {
            card.debug_selector(|| "START_CHAT_CARD".into())
        })
        .w(rems(layout.outer / 16.))
        .flex()
        .flex_col()
        .rounded(rems(8. / 16.))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg())
}

pub(super) fn card_section() -> Div {
    div()
        .flex()
        .flex_col()
        .gap(rems(COLUMN_GAP / 16.))
        .px(rems(CARD_PADDING / 16.))
        .py(rems(CARD_PADDING / 16.))
        .border_t_1()
        .border_color(theme::border())
}

/// A labelled column of a card, with a note under the control while it
/// overrides the role's value.
pub(super) fn setup_field(
    width: f32,
    label: &'static str,
    control: impl IntoElement,
    note: Option<AnyElement>,
) -> Div {
    div()
        .when(cfg!(test), |field| {
            field.debug_selector(move || format!("START_CHAT_FIELD {label}"))
        })
        .w(rems(width / 16.))
        .flex_none()
        .flex()
        .flex_col()
        .gap(rems(6. / 16.))
        .child(section_label(label))
        .child(control)
        .children(note)
}

/// What Reset takes of a column's width, and the gap before it.
pub(super) const RESET_WIDTH: f32 = 50.;

pub(super) const RESET_GAP: f32 = 8.;

/// The role's value under an overridden control, with its Reset.
pub(super) fn override_note(
    label: &'static str,
    width: f32,
    text: String,
    reset: gpui::Stateful<Div>,
) -> AnyElement {
    let selector = format!("START_CHAT_NOTE {label} {text}");
    div()
        .flex()
        .items_center()
        .gap(rems(RESET_GAP / 16.))
        .child(
            column_text(text, width - RESET_WIDTH - RESET_GAP)
                .when(cfg!(test), |note| {
                    note.debug_selector(move || selector.clone())
                })
                .text_size(theme::text_meta())
                .text_color(theme::faint()),
        )
        .child(reset)
        .into_any_element()
}

/// A control's Reset: restores that one value to the role's.
pub(super) fn reset_action(
    root: &Entity<NativeRoot>,
    focus: &FocusHandle,
    kind: ResetKind,
) -> gpui::Stateful<Div> {
    let root = root.clone();
    text_action(
        match kind {
            ResetKind::Runtime => "start-chat-reset-runtime",
            ResetKind::Model => "start-chat-reset-model",
            ResetKind::Effort => "start-chat-reset-effort",
            ResetKind::Speed => "start-chat-reset-speed",
        },
        focus,
        move |window, cx| {
            root.update(cx, |this, cx| {
                this.reset_start_chat_control(kind, window, cx)
            });
        },
    )
    .when(cfg!(test), move |reset| {
        reset.debug_selector(move || format!("START_CHAT_RESET {kind:?}"))
    })
    .flex_none()
    .gap(rems(6. / 16.))
    .text_size(theme::text_ui())
    .text_color(theme::muted())
    .hover(|reset| reset.text_color(theme::text()))
    .child(
        svg()
            .flex_none()
            .path("rotate-ccw.svg")
            .size(rems(11. / 16.))
            .text_color(theme::muted()),
    )
    .child(crate::i18n::text("ui.reset"))
}

pub(super) fn render_speed_column(
    modal: &StartChatModal,
    width: f32,
    note: Option<AnyElement>,
) -> Div {
    setup_field(
        width,
        crate::i18n::text("ui.speed"),
        div()
            .when(cfg!(test), |field| {
                field.debug_selector(|| "START_CHAT_SPEED_FIELD".into())
            })
            .child(modal.speed_select.clone()),
        note,
    )
}

pub(super) fn speed_note(modal: &StartChatModal) -> Option<Div> {
    (modal.effective_speed() == Some(CodexSpeed::Fast)).then(|| {
        div()
            .when(cfg!(test), |note| {
                note.debug_selector(|| "START_CHAT_SPEED_NOTE".into())
            })
            .text_size(theme::text_meta())
            .text_color(theme::faint())
            .child(crate::i18n::text("ui.fast_uses_more_credits"))
    })
}

/// Why a role's model and effort are not on show once another agent is
/// picked.
pub(super) fn agent_note(
    agent: &str,
    role_agent: &str,
    role_model: Option<&str>,
    role_effort: Option<&str>,
) -> String {
    let role_effort =
        role_effort.map(|effort| runner_app::ui::select::effort_label(effort, effort).resolve());
    let role_effort = role_effort.as_deref();
    let start = crate::i18n::tf(
        "format.starts_from_its_own_model_and_effort",
        &[("agent", (agent).to_string())],
    );
    match (role_model, role_effort) {
        (Some(model), Some(effort)) => crate::i18n::tf(
            "format.the_role_s_and_belong_to_and_don_t_carry_over",
            &[
                ("start", (start).to_string()),
                ("model", (model).to_string()),
                ("effort", (effort).to_string()),
                ("role_agent", (role_agent).to_string()),
            ],
        ),
        (Some(value), None) | (None, Some(value)) => crate::i18n::tf(
            "format.the_role_s_belongs_to_and_doesn_t_carry_over",
            &[
                ("start", (start).to_string()),
                ("inherited_setting", (value).to_string()),
                ("role_agent", (role_agent).to_string()),
            ],
        ),
        (None, None) => start,
    }
}
