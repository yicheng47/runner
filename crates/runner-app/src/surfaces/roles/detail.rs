use super::logic::distinct_crew_count;
use super::logic::error_banner;
use super::logic::live_activity_label;
use super::logic::local_short_timestamp;
use super::logic::permission_mode_description;
use super::logic::permission_mode_label;
use super::logic::permission_modes;
use super::logic::prompt_meta;
use super::logic::role_edit_form_is_composing;
use super::logic::role_edit_is_dirty;
use super::logic::role_permission_mode;
use super::logic::role_setting_label;
use super::logic::runtime_display_name;
use super::logic::runtime_efforts;
use super::logic::short_id;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, rems, svg, AnyElement, Context, Div, Entity, FontWeight, KeyDownEvent, SharedString,
    Window,
};
use runner_app::ui::{focus_ring, Button, ButtonVariant, RoleAvatar, WorkingDirField};
use runner_backend::model::CodexSpeed;
use runner_backend::model::Role;
use runner_backend::ops::role::RoleActivity;
use runner_backend::ops::slot::CrewMembership;

use super::ROLE_COLUMN_WIDTH;
use crate::chat_icon::ChatIcon;
pub(super) use crate::surfaces::profile_page::column_text;
use crate::surfaces::profile_page::{
    breadcrumb, caption, card, card_column, card_meta, clamped_markdown, dot_note, editing_tag,
    markdown_editor_body, markdown_mode_switch, page_columns, page_container, profile_column,
    section, section_label,
};
use crate::*;

/// Model and Effort share a row of the profile column, 16 px apart.
fn half_column(column: f32) -> f32 {
    (column - 16.) / 2.
}

/// A crew row's text, beside its 20 px avatar and 14 px chevron with 10 px gaps.
fn crew_text_width(column: f32) -> f32 {
    column - 20. - 14. - 2. * 10.
}
const PROMPT_CAPTION: &str = "Used in every chat and crew slot. A crew adds its own conventions; a slot can override runtime, model and effort.";

impl NativeRoot {
    pub(super) fn render_role_detail(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let column = self.profile_page_column_width(window, cx);
        let back_root = cx.entity();
        let detail = &self.role_surfaces.detail;
        let handle = detail.handle.clone();
        let editing = self
            .role_surfaces
            .edit
            .as_ref()
            .is_some_and(|form| form.role.handle == handle);
        let edit_error = self
            .role_surfaces
            .edit
            .as_ref()
            .filter(|_| editing)
            .and_then(|form| form.error.clone());
        let detail_error = detail.error.clone();
        let body = if detail.loading {
            div()
                .text_size(theme::text_title())
                .text_color(theme::muted())
                .child("Loading…")
                .into_any_element()
        } else if let Some(role) = detail.role.clone() {
            let activity = detail.activity.clone();
            let crews = detail.crews.clone();
            if editing {
                self.render_role_edit_page(role, activity, crews, column, cx)
            } else {
                self.render_role_view_page(role, activity, crews, column, cx)
            }
        } else {
            div()
                .rounded_sm()
                .border_1()
                .border_color(theme::with_alpha(theme::danger(), 0.4))
                .bg(theme::with_alpha(theme::danger(), 0.1))
                .px_3()
                .py_2()
                .text_size(theme::text_title())
                .text_color(theme::danger())
                .child(format!("Role @{handle} not found."))
                .into_any_element()
        };
        let on_back: crate::surfaces::profile_page::ClickHandler = Rc::new(move |window, cx| {
            back_root.update(cx, |this, cx| this.open_roles(window, cx));
        });
        div()
            .id("role-detail-scroll")
            .when(cfg!(test), |scroll| {
                scroll.debug_selector(|| "ROLE_DETAIL_SCROLL".into())
            })
            .flex_1()
            .min_h(px(0.))
            .overflow_y_scroll()
            .when(editing, |page| {
                page.on_key_down(cx.listener(Self::on_role_page_key_down))
            })
            .child(
                page_container()
                    .when(cfg!(test), |container| {
                        container.debug_selector(|| "ROLE_DETAIL_CONTAINER".into())
                    })
                    .child(
                        breadcrumb(
                            "role-detail-back",
                            "Roles",
                            on_back,
                            div()
                                .min_w(px(0.))
                                .truncate()
                                .font_family(theme::UI_MONOSPACE_FONT)
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme::text())
                                .child(format!("@{handle}")),
                        )
                        .when(cfg!(test), |header| {
                            header.debug_selector(|| "ROLE_DETAIL_HEADER".into())
                        })
                        .children(editing.then(|| {
                            editing_tag().when(cfg!(test), |tag| {
                                tag.debug_selector(|| "ROLE_EDITING_TAG".into())
                            })
                        })),
                    )
                    .children(detail_error.map(error_banner))
                    .children(edit_error.map(error_banner))
                    .child(body),
            )
            .into_any_element()
    }

    fn on_role_page_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key != "escape" {
            self.on_role_edit_key_down(event, window, cx);
            return;
        }
        let cancel = self
            .role_surfaces
            .edit
            .as_ref()
            .is_some_and(|form| !form.submitting && !role_edit_form_is_composing(form, cx));
        if cancel {
            cx.stop_propagation();
            self.close_role_edit(window, cx);
        }
    }

    fn render_role_view_page(
        &self,
        role: Role,
        activity: Option<RoleActivity>,
        crews: Vec<CrewMembership>,
        column: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // The actions keep the fixed column's width when the column spans a
        // stacked page.
        let actions = rems(column.min(ROLE_COLUMN_WIDTH) / 16.);
        let root = cx.entity();
        let chat_root = root.clone();
        let edit_root = root.clone();
        let pending = self.role_surfaces.chat_pending.as_deref() == Some(role.id.as_str());
        let chat_role = role.clone();
        let edit_role = role.clone();
        let (model, model_default) = role_setting_label(role.model.as_deref());
        let (effort, effort_default) = role_setting_label(role.effort.as_deref());
        let command = if role.args.is_empty() {
            role.command.clone()
        } else {
            format!("{} {}", role.command, role.args.join(" "))
        };
        let icon = ChatIcon::for_runtime(&role.runtime);
        let profile = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(RoleAvatar::new(role.handle.clone(), 96.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        column_text(role.display_name.clone(), column)
                            .text_size(theme::text_display())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text()),
                    )
                    .child(
                        column_text(format!("@{}", role.handle), column)
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_body())
                            .text_color(theme::muted()),
                    ),
            )
            .child(
                div()
                    .when(cfg!(test), |actions| {
                        actions.debug_selector(|| "ROLE_PAGE_ACTIONS".into())
                    })
                    .w(actions)
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div().flex_1().min_w(px(0.)).child(
                            Button::new(
                                "role-detail-chat",
                                if pending { "Starting…" } else { "Chat now" },
                            )
                            .icon("message-square.svg")
                            .variant(ButtonVariant::Primary)
                            .full_width(true)
                            .disabled(pending)
                            .on_press(move |window, cx| {
                                let role = chat_role.clone();
                                chat_root
                                    .update(cx, |this, cx| this.start_role_chat(role, window, cx));
                            }),
                        ),
                    )
                    .child(
                        Button::new("edit-role", "Edit")
                            .icon("pencil.svg")
                            .tooltip("Edit role")
                            .on_press(move |window, cx| {
                                let role = edit_role.clone();
                                edit_root
                                    .update(cx, |this, cx| this.open_role_edit(role, window, cx));
                            }),
                    ),
            );
        let setup = section()
            .when(cfg!(test), |setup| {
                setup.debug_selector(|| "ROLE_PAGE_SETUP".into())
            })
            .gap(rems(14. / 16.))
            .child(setup_row(
                "Runtime",
                div()
                    .min_w(px(0.))
                    .flex()
                    .items_center()
                    .gap(rems(6. / 16.))
                    .child(icon.render(rems(12. / 16.), icon.color(theme::muted(), true), true))
                    .child(setup_value(
                        runtime_display_name(&role.runtime),
                        column - 18.,
                        false,
                        false,
                    ))
                    .into_any_element(),
            ))
            .child(
                div()
                    .flex()
                    .gap_4()
                    .child(
                        setup_row(
                            "Model",
                            setup_value(model, half_column(column), !model_default, model_default),
                        )
                        .flex_1(),
                    )
                    .child(
                        setup_row(
                            "Effort",
                            setup_value(
                                effort,
                                half_column(column),
                                !effort_default,
                                effort_default,
                            ),
                        )
                        .flex_1(),
                    ),
            )
            .children((role.runtime == "codex").then(|| {
                setup_row(
                    "Speed",
                    setup_value(
                        match role.codex_speed {
                            None => "Inherit",
                            Some(CodexSpeed::Standard) => "Standard",
                            Some(CodexSpeed::Fast) => "Fast",
                        },
                        column,
                        false,
                        role.codex_speed.is_none(),
                    ),
                )
                .when(cfg!(test), |row| {
                    row.debug_selector(|| "ROLE_SPEED_DETAIL".into())
                })
                .children((role.codex_speed == Some(CodexSpeed::Fast)).then(|| {
                    div()
                        .text_size(theme::text_meta())
                        .text_color(theme::faint())
                        .child("Fast uses more credits.")
                }))
            }))
            .children(role_permission_mode(&role).map(|mode| {
                setup_row(
                    "Permissions",
                    setup_value(permission_mode_label(mode), column, false, false),
                )
            }))
            .child(setup_row(
                "Command",
                setup_value(format!("$ {command}"), column, true, false),
            ))
            .child(
                setup_row(
                    "Working directory",
                    match role.working_dir.clone() {
                        Some(dir) => setup_value(dir, column, true, false),
                        None => setup_value("default", column, false, true),
                    },
                )
                .when(cfg!(test), |row| {
                    row.debug_selector(|| "ROLE_SETUP_LAST".into())
                }),
            );
        let left = profile_column(column)
            .when(cfg!(test), |column| {
                column.debug_selector(|| "ROLE_PAGE_PROFILE".into())
            })
            .child(profile)
            .child(setup)
            .child(self.render_role_crews(crews, true, column, cx))
            .child(role_activity_lines(&role, activity.as_ref(), true));
        let right = prompt_column()
            .child(self.render_prompt_card(&role, cx))
            .child(caption(PROMPT_CAPTION));
        role_page_columns(left, right, false)
    }

    fn render_role_edit_page(
        &self,
        role: Role,
        activity: Option<RoleActivity>,
        crews: Vec<CrewMembership>,
        column: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // The form keeps the fixed column's width: its selects are sized when
        // the edit opens. The column's rules and lists still span the page.
        let form_width = rems(column.min(ROLE_COLUMN_WIDTH) / 16.);
        let form = self
            .role_surfaces
            .edit
            .as_ref()
            .expect("in-place role edit form");
        let root = cx.entity();
        let submit_root = root.clone();
        let cancel_root = root.clone();
        let browse_root = root.clone();
        let submitting = form.submitting;
        let can_submit = !submitting && form.display_name_valid;
        let dirty = role_edit_is_dirty(form, cx);
        let has_efforts = !runtime_efforts(&form.runtimes, &form.runtime).is_empty();
        let has_permissions = !permission_modes(&form.runtime).is_empty();
        let profile = div()
            .when(cfg!(test), |profile| {
                profile.debug_selector(|| "ROLE_EDIT_IN_PLACE".into())
            })
            .flex()
            .flex_col()
            .gap_4()
            .child(RoleAvatar::new(role.handle.clone(), 96.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(div().w(form_width).child(form.display_name.clone()))
                    .child(
                        column_text(format!("@{}", role.handle), column)
                            .mt_1()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_body())
                            .text_color(theme::muted()),
                    )
                    .child(
                        div()
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .child("The handle can't change."),
                    ),
            )
            .child(
                div()
                    .w(form_width)
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .when(cfg!(test), |save| {
                                        save.debug_selector(|| "ROLE_EDIT_SAVE".into())
                                    })
                                    .flex_1()
                                    .min_w(px(0.))
                                    .child(
                                        Button::new(
                                            "role-page-save",
                                            if submitting { "Saving…" } else { "Save" },
                                        )
                                        .icon("check.svg")
                                        .variant(ButtonVariant::Primary)
                                        .full_width(true)
                                        .focus_handle(form.submit_focus.clone())
                                        .disabled(!can_submit)
                                        .on_press(
                                            move |window, cx| {
                                                submit_root.update(cx, |this, cx| {
                                                    this.submit_role_edit(window, cx)
                                                });
                                            },
                                        ),
                                    ),
                            )
                            .child(
                                div()
                                    .when(cfg!(test), |cancel| {
                                        cancel.debug_selector(|| "ROLE_EDIT_CANCEL".into())
                                    })
                                    .child(
                                        Button::new("role-page-cancel", "Cancel")
                                            .focus_handle(form.cancel_focus.clone())
                                            .disabled(submitting)
                                            .on_press(move |window, cx| {
                                                cancel_root.update(cx, |this, cx| {
                                                    this.close_role_edit(window, cx)
                                                });
                                            }),
                                    ),
                            ),
                    )
                    // Always laid out, so the setup below never jumps.
                    .child(
                        dot_note("Unsaved changes")
                            .when(!dirty, |line| line.opacity(0.))
                            .when(cfg!(test) && dirty, |line| {
                                line.debug_selector(|| "ROLE_EDIT_DIRTY".into())
                            }),
                    ),
            );
        let setup = section()
            .when(cfg!(test), |setup| {
                setup.debug_selector(|| "ROLE_PAGE_SETUP".into())
            })
            .child(
                div()
                    .w(form_width)
                    .flex()
                    .flex_col()
                    .gap(rems(14. / 16.))
                    .child(edit_row("Runtime", form.runtime_select.clone()).children(
                        form.agents_error.clone().map(|error| {
                            div()
                                .text_size(theme::text_meta())
                                .text_color(theme::danger())
                                .child(error)
                        }),
                    ))
                    .child(edit_row("Model", form.model_field.clone()))
                    .children(has_efforts.then(|| edit_row("Effort", form.effort_select.clone())))
                    .children((form.runtime == "codex").then(|| {
                        edit_row("Speed", form.speed_select.clone())
                            .when(cfg!(test), |row| {
                                row.debug_selector(|| "ROLE_SPEED_EDIT".into())
                            })
                            .children((form.speed == "fast").then(|| {
                                div()
                                    .text_size(theme::text_meta())
                                    .text_color(theme::faint())
                                    .child("Fast uses more credits.")
                            }))
                    }))
                    .children(has_permissions.then(|| {
                        edit_row("Permissions", form.permission_select.clone()).child(
                            // Wraps at the column's width from the first sizing
                            // pass, which offers this min_w(0) row none.
                            div()
                                .w(rems(ROLE_COLUMN_WIDTH / 16.))
                                .text_size(theme::text_meta())
                                .line_height(rems(1.))
                                .text_color(theme::faint())
                                .child(permission_mode_description(
                                    &form.runtime,
                                    form.permission_mode,
                                )),
                        )
                    }))
                    .child(edit_row("Command", form.command.clone()))
                    .child(edit_row("Args", form.args.clone()))
                    .child(
                        edit_row(
                            "Working directory",
                            WorkingDirField::new(
                                form.working_dir.clone(),
                                submitting,
                                Rc::new(move |_, cx| {
                                    browse_root
                                        .update(cx, |this, cx| this.browse_role_edit_cwd(cx));
                                }),
                            )
                            .browse_id("role-page-browse")
                            .browse_focus(form.browse_focus.clone()),
                        )
                        .when(cfg!(test), |row| {
                            row.debug_selector(|| "ROLE_SETUP_LAST".into())
                        }),
                    ),
            );
        let left = profile_column(column)
            .when(cfg!(test), |column| {
                column.debug_selector(|| "ROLE_PAGE_PROFILE".into())
            })
            .child(profile)
            .child(setup)
            .child(self.render_role_crews(crews, false, column, cx))
            .child(role_activity_lines(&role, activity.as_ref(), false));
        let right = prompt_column()
            .child(self.render_prompt_editor_card(cx))
            .child(caption(PROMPT_CAPTION));
        role_page_columns(left, right, true)
    }

    fn render_role_crews(
        &self,
        crews: Vec<CrewMembership>,
        interactive: bool,
        column: f32,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let text_width = crew_text_width(column);
        let root = cx.entity();
        let label = format!("Crews using this role · {}", distinct_crew_count(&crews));
        let rows =
            if crews.is_empty() {
                div()
                    .text_size(theme::text_ui())
                    .text_color(theme::faint())
                    .child("Not in any crew yet. Add it to one from a crew's page.")
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .children(crews.into_iter().map(|membership| {
                        crew_row(membership, interactive, text_width, root.clone())
                    }))
                    .into_any_element()
            };
        section()
            .gap_2()
            .when(!interactive, |section| section.opacity(0.4))
            .child(section_label(label))
            .child(rows)
            .into_any_element()
    }

    fn render_prompt_card(&self, role: &Role, cx: &mut Context<Self>) -> AnyElement {
        let root = cx.entity();
        let prompt = role
            .system_prompt
            .as_deref()
            .filter(|prompt| !prompt.trim().is_empty());
        let body = match prompt {
            None => div()
                .text_size(theme::text_body())
                .italic()
                .text_color(theme::faint())
                .child("No system prompt yet. Edit the role to add one.")
                .into_any_element(),
            Some(prompt) => clamped_markdown(
                &format!("role-prompt-{}", role.id),
                prompt,
                self.role_surfaces.prompt_expanded,
                "ROLE_PROMPT",
                Rc::new(move |_, cx| {
                    root.update(cx, |this, cx| this.toggle_role_prompt(cx));
                }),
                cx.entity_id(),
                cx,
            ),
        };
        prompt_card(card_meta(prompt.map(prompt_meta)))
            .child(div().px_5().py_4().child(body))
            .into_any_element()
    }

    fn render_prompt_editor_card(&self, cx: &mut Context<Self>) -> AnyElement {
        let form = self
            .role_surfaces
            .edit
            .as_ref()
            .expect("in-place role edit form");
        let root = cx.entity();
        let preview = self.role_surfaces.prompt_preview;
        let draft = form.system_prompt.read(cx).text();
        let meta = (!draft.trim().is_empty()).then(|| prompt_meta(draft));
        prompt_card(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_3()
                .child(markdown_mode_switch(
                    "role-prompt-mode",
                    preview,
                    Rc::new(move |preview, cx| {
                        root.update(cx, |this, cx| this.set_role_prompt_preview(preview, cx));
                    }),
                ))
                .child(card_meta(meta))
                .into_any_element(),
        )
        .flex_1()
        .min_h(rems(420. / 16.))
        .child(markdown_editor_body(
            "role-prompt-draft",
            form.system_prompt.clone(),
            preview,
            "ROLE_PROMPT",
            cx.entity_id(),
            cx,
        ))
        .into_any_element()
    }

    fn toggle_role_prompt(&mut self, cx: &mut Context<Self>) {
        self.role_surfaces.prompt_expanded = !self.role_surfaces.prompt_expanded;
        cx.notify();
    }

    fn set_role_prompt_preview(&mut self, preview: bool, cx: &mut Context<Self>) {
        if self.role_surfaces.prompt_preview != preview {
            self.role_surfaces.prompt_preview = preview;
            cx.notify();
        }
    }
}

fn role_page_columns(left: Div, right: Div, stretch: bool) -> AnyElement {
    page_columns(left, right, stretch)
        .when(cfg!(test), |body| {
            body.debug_selector(|| "ROLE_DETAIL_BODY".into())
        })
        .into_any_element()
}

fn prompt_column() -> Div {
    card_column().when(cfg!(test), |column| {
        column.debug_selector(|| "ROLE_PAGE_PROMPT".into())
    })
}

fn prompt_card(header_right: AnyElement) -> Div {
    card("file-text.svg", "System prompt", None, header_right).when(cfg!(test), |card| {
        card.debug_selector(|| "ROLE_PROMPT_CARD".into())
    })
}

fn crew_row(
    membership: CrewMembership,
    interactive: bool,
    text_width: f32,
    root: Entity<NativeRoot>,
) -> AnyElement {
    let crew_id = membership.crew_id.clone();
    let key_crew_id = crew_id.clone();
    let key_root = root.clone();
    let group = SharedString::from(format!(
        "role-crew-{}-{}",
        membership.crew_id, membership.slot_id
    ));
    div()
        .id(group.clone())
        .group(group.clone())
        .flex()
        .items_center()
        .gap(rems(10. / 16.))
        .rounded(rems(4. / 16.))
        .py(rems(6. / 16.))
        .child(RoleAvatar::new(membership.slot_handle.clone(), 20.))
        .child(
            div()
                .w(rems(text_width / 16.))
                .flex_none()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(rems(6. / 16.))
                        .child(
                            div()
                                .min_w(px(0.))
                                .truncate()
                                .text_size(theme::text_body())
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme::text())
                                .child(membership.crew_name),
                        )
                        .children(membership.lead.then(|| {
                            div()
                                .flex_none()
                                .rounded(rems(3. / 16.))
                                .border_1()
                                .border_color(theme::border_strong())
                                .px(rems(4. / 16.))
                                .font_family(theme::UI_MONOSPACE_FONT)
                                .text_size(theme::text_micro())
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::faint())
                                .child("LEAD")
                        })),
                )
                .child(
                    column_text(format!("as @{}", membership.slot_handle), text_width)
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_meta())
                        .text_color(theme::faint()),
                ),
        )
        .child(
            svg()
                .flex_none()
                .path("chevron-right.svg")
                .size(rems(14. / 16.))
                .text_color(theme::faint())
                .when(interactive, |chevron| {
                    chevron.group_hover(group.clone(), |chevron| chevron.text_color(theme::text()))
                }),
        )
        .when(interactive, |row| {
            row.tab_index(0)
                .cursor_pointer()
                .focus_visible(|row| row.shadow(focus_ring(theme::border_strong())))
                .on_click(move |_, window, cx| {
                    root.update(cx, |this, cx| {
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
        })
        .into_any_element()
}

fn role_activity_lines(
    role: &Role,
    activity: Option<&RoleActivity>,
    interactive: bool,
) -> AnyElement {
    let live = activity
        .and_then(live_activity_label)
        .map(|live| format!("{live} live"))
        .unwrap_or_else(|| "Nothing live".to_owned());
    let last_seen = activity
        .and_then(|activity| activity.last_started_at)
        .map(|started| format!("Last seen {}", local_short_timestamp(started)))
        .unwrap_or_else(|| "Never started".to_owned());
    section()
        .gap(rems(6. / 16.))
        .when(!interactive, |section| section.opacity(0.4))
        .text_size(theme::text_ui())
        .child(div().text_color(theme::muted()).child(live))
        .child(div().text_color(theme::faint()).child(last_seen))
        .child(div().text_color(theme::faint()).child(format!(
            "Created {}",
            local_short_timestamp(role.created_at)
        )))
        .child(
            div()
                .font_family(theme::UI_MONOSPACE_FONT)
                .text_size(theme::text_caption())
                .text_color(theme::faint())
                .child(short_id(&role.id)),
        )
        .into_any_element()
}

fn setup_row(label: &'static str, value: AnyElement) -> Div {
    div()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap(rems(3. / 16.))
        .child(section_label(label))
        .child(value)
}

fn setup_value(
    value: impl Into<SharedString>,
    width: f32,
    monospace: bool,
    dim: bool,
) -> AnyElement {
    column_text(value, width)
        .text_size(theme::text_body())
        .text_color(if dim { theme::faint() } else { theme::text() })
        .when(monospace, |value| {
            value.font_family(theme::UI_MONOSPACE_FONT)
        })
        .into_any_element()
}

fn edit_row(label: &'static str, control: impl IntoElement) -> Div {
    div()
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap(rems(6. / 16.))
        .child(section_label(label))
        .child(control)
}
