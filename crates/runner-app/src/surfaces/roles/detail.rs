use super::logic::distinct_crew_count;
use super::logic::error_banner;
use super::logic::live_activity_label;
use super::logic::local_short_timestamp;
use super::logic::permission_mode_description;
use super::logic::permission_mode_label;
use super::logic::permission_modes;
use super::logic::prompt_meta;
use super::logic::prompt_preview;
use super::logic::role_edit_form_is_composing;
use super::logic::role_edit_is_dirty;
use super::logic::role_permission_mode;
use super::logic::role_setting_label;
use super::logic::runtime_display_name;
use super::logic::runtime_efforts;
use super::logic::short_id;

use gpui::prelude::*;
use gpui::{
    div, linear_color_stop, linear_gradient, px, rems, svg, AnyElement, Context, Div, Entity,
    FontWeight, KeyDownEvent, SharedString, Window,
};
use runner_app::ui::{focus_ring, Button, ButtonVariant, IconButton, IconButtonSize, RoleAvatar};
use runner_backend::model::Role;
use runner_backend::ops::role::RoleActivity;
use runner_backend::ops::slot::CrewMembership;

use super::ROLE_COLUMN_WIDTH;
use crate::chat_icon::ChatIcon;
use crate::surfaces::mission_markdown::render_markdown;
use crate::*;

/// Below this the prompt card wraps under the profile column.
const PROMPT_COLUMN_BASIS: f32 = 360.;
const PROMPT_CAPTION: &str = "Used in every chat and crew slot. A crew adds its own conventions; a slot can override runtime, model and effort.";

impl NativeRoot {
    pub(super) fn render_role_detail(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let root = cx.entity();
        let back_root = root.clone();
        let back_key_root = root.clone();
        let detail = &self.role_surfaces.detail;
        let handle = detail.handle.clone();
        let editing = self
            .role_surfaces
            .edit
            .as_ref()
            .is_some_and(|form| form.slot.is_none() && form.role.handle == handle);
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
                self.render_role_edit_page(role, activity, crews, cx)
            } else {
                self.render_role_view_page(role, activity, crews, cx)
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
                div()
                    .when(cfg!(test), |container| {
                        container.debug_selector(|| "ROLE_DETAIL_CONTAINER".into())
                    })
                    .mx_auto()
                    .w_full()
                    .max_w(rems(1104. / 16.))
                    .flex()
                    .flex_col()
                    .gap(rems(28. / 16.))
                    .px_8()
                    .pt(rems(40. / 16.))
                    .pb_8()
                    .child(
                        div()
                            .when(cfg!(test), |header| {
                                header.debug_selector(|| "ROLE_DETAIL_HEADER".into())
                            })
                            .flex()
                            .items_center()
                            .gap_2()
                            .text_size(theme::text_body())
                            .text_color(theme::muted())
                            .child(
                                div()
                                    .id("role-detail-back")
                                    .flex_none()
                                    .tab_index(0)
                                    .cursor_pointer()
                                    .hover(|text| text.text_color(theme::text()))
                                    .focus_visible(|text| {
                                        text.text_color(theme::text()).underline()
                                    })
                                    .on_click(move |_, window, cx| {
                                        back_root
                                            .update(cx, |this, cx| this.open_roles(window, cx));
                                    })
                                    .on_key_down(move |event: &KeyDownEvent, window, cx| {
                                        if matches!(event.keystroke.key.as_str(), "enter" | "space")
                                        {
                                            cx.stop_propagation();
                                            back_key_root
                                                .update(cx, |this, cx| this.open_roles(window, cx));
                                        }
                                    })
                                    .child("Roles"),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .text_color(theme::border_strong())
                                    .child("›"),
                            )
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .truncate()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::text())
                                    .child(format!("@{handle}")),
                            )
                            .children(editing.then(|| {
                                div()
                                    .when(cfg!(test), |tag| {
                                        tag.debug_selector(|| "ROLE_EDITING_TAG".into())
                                    })
                                    .flex_none()
                                    .rounded(rems(3. / 16.))
                                    .bg(theme::raised())
                                    .px(rems(6. / 16.))
                                    .py(rems(1. / 16.))
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_micro())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::muted())
                                    .child("EDITING")
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
        let cancel = self.role_surfaces.edit.as_ref().is_some_and(|form| {
            form.slot.is_none() && !form.submitting && !role_edit_form_is_composing(form, cx)
        });
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
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                        div()
                            .min_w(px(0.))
                            .truncate()
                            .text_size(theme::text_display())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child(role.display_name.clone()),
                    )
                    .child(
                        div()
                            .min_w(px(0.))
                            .truncate()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_body())
                            .text_color(theme::muted())
                            .child(format!("@{}", role.handle)),
                    ),
            )
            .child(
                div()
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
                                edit_root.update(cx, |this, cx| {
                                    this.open_role_edit(role, None, window, cx)
                                });
                            }),
                    ),
            );
        let setup = section()
            .gap(rems(14. / 16.))
            .child(setup_row(
                "Runtime",
                div()
                    .min_w(px(0.))
                    .flex()
                    .items_center()
                    .gap(rems(6. / 16.))
                    .child(
                        svg()
                            .flex_none()
                            .path(icon.path)
                            .size(rems(12. / 16.))
                            .text_color(icon.color(theme::muted(), true)),
                    )
                    .child(setup_value(
                        runtime_display_name(&role.runtime),
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
                        setup_row("Model", setup_value(model, !model_default, model_default))
                            .flex_1(),
                    )
                    .child(
                        setup_row(
                            "Effort",
                            setup_value(effort, !effort_default, effort_default),
                        )
                        .flex_1(),
                    ),
            )
            .children(role_permission_mode(&role).map(|mode| {
                setup_row(
                    "Permissions",
                    setup_value(permission_mode_label(mode), false, false),
                )
            }))
            .child(setup_row(
                "Command",
                setup_value(format!("$ {command}"), true, false),
            ))
            .child(setup_row(
                "Working directory",
                match role.working_dir.clone() {
                    Some(dir) => setup_value(dir, true, false),
                    None => setup_value("default", false, true),
                },
            ));
        let left = div()
            .when(cfg!(test), |column| {
                column.debug_selector(|| "ROLE_PAGE_PROFILE".into())
            })
            .w(rems(ROLE_COLUMN_WIDTH / 16.))
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap(rems(20. / 16.))
            .child(profile)
            .child(setup)
            .child(self.render_role_crews(crews, true, cx))
            .child(role_activity_lines(&role, activity.as_ref(), true));
        let right = prompt_column()
            .child(self.render_prompt_card(&role, cx))
            .child(prompt_caption());
        role_page_columns(left, right, false)
    }

    fn render_role_edit_page(
        &self,
        role: Role,
        activity: Option<RoleActivity>,
        crews: Vec<CrewMembership>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
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
                    .child(div().w_full().child(form.display_name.clone()))
                    .child(
                        div()
                            .mt_1()
                            .min_w(px(0.))
                            .truncate()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_body())
                            .text_color(theme::muted())
                            .child(format!("@{}", role.handle)),
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
                    .children(dirty.then(|| {
                        div()
                            .when(cfg!(test), |line| {
                                line.debug_selector(|| "ROLE_EDIT_DIRTY".into())
                            })
                            .flex()
                            .items_center()
                            .gap(rems(6. / 16.))
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .child(
                                div()
                                    .flex_none()
                                    .size(rems(6. / 16.))
                                    .rounded_full()
                                    .bg(theme::warning()),
                            )
                            .child("Unsaved changes")
                    })),
            );
        let setup = section()
            .gap(rems(14. / 16.))
            .child(edit_row("Runtime", form.runtime_select.clone()).children(
                form.agents_error.clone().map(|error| {
                    div()
                        .text_size(theme::text_meta())
                        .text_color(theme::danger())
                        .child(error)
                }),
            ))
            .child(
                div()
                    .flex()
                    .gap_3()
                    .child(edit_row("Model", form.model_field.clone()).flex_1())
                    .children(
                        has_efforts
                            .then(|| edit_row("Effort", form.effort_select.clone()).flex_none()),
                    ),
            )
            .children(has_permissions.then(|| {
                edit_row("Permissions", form.permission_select.clone()).child(
                    div()
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
            .child(edit_row(
                "Working directory",
                div().relative().child(form.working_dir.clone()).child(
                    div()
                        .absolute()
                        .top(rems(5. / 16.))
                        .right(rems(5. / 16.))
                        .child(
                            IconButton::new("role-page-browse", "folder.svg")
                                .size(IconButtonSize::Sm)
                                .focus_handle(form.browse_focus.clone())
                                .tooltip("Pick a working directory")
                                .disabled(submitting)
                                .on_press(move |_, cx| {
                                    browse_root
                                        .update(cx, |this, cx| this.browse_role_edit_cwd(cx));
                                }),
                        ),
                ),
            ));
        let left = div()
            .when(cfg!(test), |column| {
                column.debug_selector(|| "ROLE_PAGE_PROFILE".into())
            })
            .w(rems(ROLE_COLUMN_WIDTH / 16.))
            .min_w(px(0.))
            .flex()
            .flex_col()
            .gap(rems(20. / 16.))
            .child(profile)
            .child(setup)
            .child(self.render_role_crews(crews, false, cx))
            .child(role_activity_lines(&role, activity.as_ref(), false));
        let right = prompt_column()
            .child(self.render_prompt_editor_card(cx))
            .child(prompt_caption());
        role_page_columns(left, right, true)
    }

    fn render_role_crews(
        &self,
        crews: Vec<CrewMembership>,
        interactive: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = cx.entity();
        let label = format!("Crews using this role · {}", distinct_crew_count(&crews));
        let rows = if crews.is_empty() {
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
                .children(
                    crews
                        .into_iter()
                        .map(|membership| crew_row(membership, interactive, root.clone())),
                )
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
        let key_root = root.clone();
        let prompt = role
            .system_prompt
            .as_deref()
            .filter(|prompt| !prompt.trim().is_empty());
        let meta = prompt.map(prompt_meta);
        let body = match prompt {
            None => div()
                .text_size(theme::text_body())
                .italic()
                .text_color(theme::faint())
                .child("No system prompt yet. Edit the role to add one.")
                .into_any_element(),
            Some(prompt) => {
                let expanded = self.role_surfaces.prompt_expanded;
                let preview = prompt_preview(prompt);
                let clamped = !expanded && preview.is_some();
                let shown = if clamped {
                    preview.unwrap_or(prompt)
                } else {
                    prompt
                };
                let lines = prompt.lines().count();
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .child(
                        div()
                            .when(cfg!(test), |text| {
                                text.debug_selector(|| "ROLE_PROMPT_TEXT".into())
                            })
                            .relative()
                            .min_w(px(0.))
                            .text_size(theme::text_body())
                            .text_color(theme::text())
                            .child(render_markdown(
                                &format!("role-prompt-{}", role.id),
                                shown,
                                cx.entity_id(),
                                None,
                                theme::accent(),
                                None,
                                cx,
                            ))
                            .when(clamped, |text| {
                                text.child(
                                    div()
                                        .absolute()
                                        .left_0()
                                        .right_0()
                                        .bottom_0()
                                        .h(rems(64. / 16.))
                                        .bg(linear_gradient(
                                            180.,
                                            linear_color_stop(
                                                theme::with_alpha(theme::panel(), 0.),
                                                0.,
                                            ),
                                            linear_color_stop(theme::panel(), 1.),
                                        )),
                                )
                            }),
                    )
                    .children(preview.is_some().then(|| {
                        div().flex().child(
                            div()
                                .id("role-prompt-toggle")
                                .when(cfg!(test), |toggle| {
                                    toggle.debug_selector(|| "ROLE_PROMPT_TOGGLE".into())
                                })
                                .tab_index(0)
                                .flex()
                                .items_center()
                                .gap_1()
                                .rounded(rems(3. / 16.))
                                .text_size(theme::text_ui())
                                .text_color(theme::muted())
                                .cursor_pointer()
                                .hover(|toggle| toggle.text_color(theme::text()))
                                .focus_visible(|toggle| {
                                    toggle
                                        .text_color(theme::text())
                                        .shadow(focus_ring(theme::border_strong()))
                                })
                                .on_click(move |_, _, cx| {
                                    root.update(cx, |this, cx| this.toggle_role_prompt(cx));
                                })
                                .on_key_down(move |event: &KeyDownEvent, _, cx| {
                                    if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                                        cx.stop_propagation();
                                        key_root.update(cx, |this, cx| this.toggle_role_prompt(cx));
                                    }
                                })
                                .child(if expanded {
                                    "Show less".to_owned()
                                } else {
                                    format!("Show all {lines} lines")
                                })
                                .child(
                                    svg()
                                        .flex_none()
                                        .path(if expanded {
                                            "chevron-up.svg"
                                        } else {
                                            "chevron-down.svg"
                                        })
                                        .size(rems(12. / 16.))
                                        .text_color(theme::faint()),
                                ),
                        )
                    }))
                    .into_any_element()
            }
        };
        prompt_card(
            div()
                .flex_none()
                .font_family(theme::UI_MONOSPACE_FONT)
                .text_size(theme::text_caption())
                .text_color(theme::faint())
                .children(meta)
                .into_any_element(),
        )
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
        let draft = form.system_prompt.read(cx).text().to_owned();
        let meta = (!draft.trim().is_empty()).then(|| prompt_meta(&draft));
        let body = if preview {
            div()
                .id("role-prompt-preview")
                .when(cfg!(test), |body| {
                    body.debug_selector(|| "ROLE_PROMPT_PREVIEW".into())
                })
                .flex_1()
                .min_h(px(0.))
                .overflow_y_scroll()
                .px_5()
                .py_4()
                .text_size(theme::text_body())
                .text_color(theme::text())
                .child(if draft.trim().is_empty() {
                    div()
                        .italic()
                        .text_color(theme::faint())
                        .child("Nothing to preview.")
                        .into_any_element()
                } else {
                    render_markdown(
                        "role-prompt-draft",
                        &draft,
                        cx.entity_id(),
                        None,
                        theme::accent(),
                        None,
                        cx,
                    )
                })
                .into_any_element()
        } else {
            div()
                .when(cfg!(test), |body| {
                    body.debug_selector(|| "ROLE_PROMPT_EDITOR".into())
                })
                .flex_1()
                .min_h(px(0.))
                .flex()
                .flex_col()
                .px_5()
                .py_4()
                .child(form.system_prompt.clone())
                .into_any_element()
        };
        prompt_card(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_3()
                .child(prompt_mode_switch(preview, root))
                .children(meta.map(|meta| {
                    div()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_caption())
                        .text_color(theme::faint())
                        .child(meta)
                }))
                .into_any_element(),
        )
        .flex_1()
        .min_h(rems(420. / 16.))
        .child(body)
        .into_any_element()
    }

    fn toggle_role_prompt(&mut self, cx: &mut Context<Self>) {
        self.role_surfaces.prompt_expanded = !self.role_surfaces.prompt_expanded;
        cx.notify();
    }
}

/// The profile column and the prompt column. The prompt wraps under the
/// profile when the page is too narrow for both; while editing, the prompt
/// card stretches to the profile column's height.
fn role_page_columns(left: Div, right: Div, stretch: bool) -> AnyElement {
    div()
        .when(cfg!(test), |body| {
            body.debug_selector(|| "ROLE_DETAIL_BODY".into())
        })
        .flex()
        .flex_wrap()
        .when(!stretch, |body| body.items_start())
        .gap_x(rems(48. / 16.))
        .gap_y(rems(32. / 16.))
        .child(left)
        .child(right)
        .into_any_element()
}

fn prompt_column() -> Div {
    div()
        .when(cfg!(test), |column| {
            column.debug_selector(|| "ROLE_PAGE_PROMPT".into())
        })
        .flex_1()
        .flex_basis(rems(PROMPT_COLUMN_BASIS / 16.))
        .min_w(px(0.))
        .flex()
        .flex_col()
        .gap_3()
}

fn prompt_caption() -> AnyElement {
    div()
        .flex_none()
        .text_size(theme::text_ui())
        .line_height(rems(18. / 16.))
        .text_color(theme::faint())
        .child(PROMPT_CAPTION)
        .into_any_element()
}

fn prompt_card(header_right: AnyElement) -> Div {
    div()
        .when(cfg!(test), |card| {
            card.debug_selector(|| "ROLE_PROMPT_CARD".into())
        })
        .min_w(px(0.))
        .flex()
        .flex_col()
        .overflow_hidden()
        .rounded_lg()
        .border_1()
        .border_color(theme::border())
        .bg(theme::panel())
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap_3()
                .px_4()
                .py(rems(10. / 16.))
                .border_b_1()
                .border_color(theme::border())
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            svg()
                                .flex_none()
                                .path("file-text.svg")
                                .size(rems(14. / 16.))
                                .text_color(theme::muted()),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_size(theme::text_body())
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme::text())
                                .child("System prompt"),
                        ),
                )
                .child(header_right),
        )
}

fn prompt_mode_switch(preview: bool, root: Entity<NativeRoot>) -> AnyElement {
    let segment = |id: &'static str, label: &'static str, active: bool, show_preview: bool| {
        let click_root = root.clone();
        let key_root = root.clone();
        div()
            .id(id)
            .tab_index(0)
            .px_2()
            .py(rems(2. / 16.))
            .rounded(rems(3. / 16.))
            .text_size(theme::text_ui())
            .text_color(if active {
                theme::text()
            } else {
                theme::muted()
            })
            .when(active, |segment| segment.bg(theme::raised()))
            .cursor_pointer()
            .hover(|segment| segment.text_color(theme::text()))
            .focus_visible(|segment| segment.shadow(focus_ring(theme::border_strong())))
            .on_click(move |_, _, cx| {
                click_root.update(cx, |this, cx| {
                    this.set_role_prompt_preview(show_preview, cx)
                });
            })
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    key_root.update(cx, |this, cx| {
                        this.set_role_prompt_preview(show_preview, cx)
                    });
                }
            })
            .child(label)
    };
    div()
        .flex()
        .items_center()
        .gap(rems(2. / 16.))
        .p(rems(2. / 16.))
        .rounded(rems(4. / 16.))
        .border_1()
        .border_color(theme::border())
        .bg(theme::bg())
        .child(segment(
            "role-prompt-mode-markdown",
            "Markdown",
            !preview,
            false,
        ))
        .child(segment(
            "role-prompt-mode-preview",
            "Preview",
            preview,
            true,
        ))
        .into_any_element()
}

impl NativeRoot {
    fn set_role_prompt_preview(&mut self, preview: bool, cx: &mut Context<Self>) {
        if self.role_surfaces.prompt_preview != preview {
            self.role_surfaces.prompt_preview = preview;
            cx.notify();
        }
    }
}

fn crew_row(membership: CrewMembership, interactive: bool, root: Entity<NativeRoot>) -> AnyElement {
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
                .flex_1()
                .min_w(px(0.))
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
                    div()
                        .min_w(px(0.))
                        .truncate()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_meta())
                        .text_color(theme::faint())
                        .child(format!("as @{}", membership.slot_handle)),
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

/// A left-column block under a hairline rule.
fn section() -> Div {
    div()
        .flex()
        .flex_col()
        .border_t_1()
        .border_color(theme::border())
        .pt(rems(20. / 16.))
}

fn section_label(label: impl Into<SharedString>) -> AnyElement {
    div()
        .text_size(theme::text_meta())
        .text_color(theme::faint())
        .child(label.into())
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

fn setup_value(value: impl Into<SharedString>, monospace: bool, dim: bool) -> AnyElement {
    div()
        .min_w(px(0.))
        .truncate()
        .text_size(theme::text_body())
        .text_color(if dim { theme::faint() } else { theme::text() })
        .when(monospace, |value| {
            value.font_family(theme::UI_MONOSPACE_FONT)
        })
        .child(value.into())
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
