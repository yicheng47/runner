use super::*;

impl NativeRoot {
    pub(super) fn render_role_fields(
        &self,
        modal: &StartChatModal,
        layout: CardLayout,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut card = card(layout).child(modal.role_select.clone());
        let role = modal.selected_role();
        if let Some(role) = role {
            card = card.child(self.render_role_setup(modal, role, layout, cx));
        }
        let mut field = Field::new(
            "start-chat-role-field",
            crate::i18n::text("ui.role"),
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(card)
                .when(modal.roles.is_empty(), |field| {
                    field.child(
                        div()
                            .text_size(theme::text_meta())
                            .text_color(theme::warning())
                            .child(crate::i18n::text(
                                "ui.no_roles_yet_create_one_from_the_roles_page_first",
                            )),
                    )
                }),
        )
        .focus_target(modal.role_select.read(cx).focus_handle())
        .emphasized(true);
        if role.is_some() {
            field = field.subtitle(if modal.mission_role.is_some() {
                crate::i18n::text(
                    "ui.starts_with_the_role_s_settings_change_any_of_them_for_this_agent_only",
                )
            } else {
                crate::i18n::text(ROLE_HINT)
            });
        }
        field.into_any_element()
    }

    /// The role's Runtime, Model and Effort as controls filled with the role's
    /// values; an override adds the amber dot and, under the control, the
    /// role's value with Reset.
    pub(super) fn render_role_setup(
        &self,
        modal: &StartChatModal,
        role: &RoleChoice,
        layout: CardLayout,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let third = layout.third();
        let root = cx.entity();
        let overrides = modal.role_overrides(cx);
        let own = modal.on_own_agent();
        let agent = modal.role_agent().unwrap_or(&role.runtime).to_owned();
        let runtime = modal.active_runtime();
        let effort_available = runtime.is_some_and(|runtime| !runtime.efforts.is_empty());
        let note = |label: &'static str, kind: ResetKind, changed: bool, value: String| {
            changed.then(|| {
                override_note(
                    label,
                    third,
                    crate::i18n::tf("format.role", &[("setting", (value).to_string())]),
                    reset_action(&root, &modal.reset_focus[kind as usize], kind),
                )
            })
        };
        let row = || div().flex().items_start().gap(rems(COLUMN_GAP / 16.));
        let controls = row()
            .child(setup_field(
                third,
                crate::i18n::text("ui.runtime"),
                modal.role_runtime_select.clone(),
                note(
                    crate::i18n::text("ui.runtime"),
                    ResetKind::Runtime,
                    overrides.runtime,
                    runtime_display_name(&modal.runtimes, &role.runtime),
                ),
            ))
            .child(setup_field(
                third,
                crate::i18n::text("ui.model"),
                div().w_full().child(modal.model_field.clone()),
                note(
                    crate::i18n::text("ui.model"),
                    ResetKind::Model,
                    overrides.model,
                    model_placeholder(Some(role)).resolve().to_string(),
                ),
            ))
            .when(effort_available, |controls| {
                controls.child(setup_field(
                    third,
                    crate::i18n::text("ui.effort"),
                    modal.effort_select.clone(),
                    note(
                        crate::i18n::text("ui.effort"),
                        ResetKind::Effort,
                        overrides.effort,
                        crate::surfaces::roles::logic::role_effort_label(Some(
                            &modal.baseline_effort(),
                        ))
                        .0,
                    ),
                ))
            });
        card_section()
            .when(cfg!(test), |section| {
                section.debug_selector(|| "START_CHAT_SETUP".into())
            })
            .child(controls)
            .when(modal.codex_speed_visible(), |section| {
                section.child(row().child(render_speed_column(
                    modal,
                    third,
                    note(
                        crate::i18n::text("ui.speed"),
                        ResetKind::Speed,
                        overrides.speed,
                        speed_label(role.codex_speed).into(),
                    ),
                )))
            })
            .children(speed_note(modal))
            .children((!own).then(|| {
                let note = agent_note(
                    &runtime_display_name(&modal.runtimes, &agent),
                    &runtime_display_name(&modal.runtimes, &role.runtime),
                    trimmed(role.model.as_deref()),
                    trimmed(role.effort.as_deref()),
                );
                div()
                    .w(rems(layout.content / 16.))
                    .text_size(theme::text_ui())
                    .text_color(theme::faint())
                    .child(note)
            }))
            .into_any_element()
    }
}
