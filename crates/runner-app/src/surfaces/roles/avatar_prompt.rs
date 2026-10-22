use std::rc::Rc;

use gpui::prelude::*;
use gpui::{div, AnyElement, Context, Entity, FocusHandle, Subscription, Window};
use runner_app::ui::{Button, ButtonVariant, Modal, OverlayWidth, TextField};

use super::logic::RoleFormKind;
use crate::{theme, NativeRoot};

pub(super) struct AvatarPromptDialog {
    pub input: Entity<TextField>,
    default_prompt: String,
    reset_focus: FocusHandle,
    cancel_focus: FocusHandle,
    generate_focus: FocusHandle,
    _subscription: Subscription,
}

fn prompt_valid(prompt: &str) -> bool {
    !prompt.trim().is_empty() && prompt.len() <= 16 * 1024
}

impl NativeRoot {
    pub(super) fn open_role_avatar_prompt(
        &mut self,
        kind: RoleFormKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(context) = self.role_avatar_input(kind, cx) else {
            return;
        };
        let default_prompt = crate::i18n::tf(
            "format.role_avatar_default_prompt",
            &[
                ("name", context.display_name.clone()),
                ("handle", context.handle.clone()),
                (
                    "responsibilities",
                    context
                        .system_prompt
                        .clone()
                        .unwrap_or_else(|| context.display_name.clone()),
                ),
                (
                    "color",
                    context
                        .avatar_color
                        .clone()
                        .unwrap_or_else(|| crate::i18n::text("ui.avatar_color_auto").to_string()),
                ),
            ],
        );
        let Some(form) = self.role_avatar_form_mut(kind).filter(|form| {
            !form.submitting
                && !form.avatar.generating
                && !form.avatar.color_error
                && form.display_name_valid
        }) else {
            return;
        };
        let value = if form.avatar.prompt_is_custom {
            form.avatar
                .generation_prompt
                .clone()
                .unwrap_or_else(|| default_prompt.clone())
        } else {
            default_prompt.clone()
        };
        let input = cx.new(|cx| {
            TextField::textarea(cx.focus_handle(), value, "", 9, false)
                .text_size(theme::text_body())
                .with_scrollbar(cx)
        });
        let subscription = cx.observe(&input, |_, _, cx| cx.notify());
        input.read(cx).focus_handle().focus(window, cx);
        form.avatar_prompt = Some(AvatarPromptDialog {
            input,
            default_prompt,
            reset_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            generate_focus: cx.focus_handle(),
            _subscription: subscription,
        });
        cx.notify();
    }

    fn close_role_avatar_prompt(
        &mut self,
        kind: RoleFormKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.role_avatar_form_mut(kind) else {
            return;
        };
        form.avatar_prompt = None;
        form.avatar_generate_focus.focus(window, cx);
        cx.notify();
    }

    fn confirm_role_avatar_prompt(
        &mut self,
        kind: RoleFormKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self.role_avatar_form_mut(kind) else {
            return;
        };
        let Some(dialog) = &form.avatar_prompt else {
            return;
        };
        let input = dialog.input.read(cx);
        if input.is_composing() || !prompt_valid(input.text()) {
            return;
        }
        form.avatar.prompt_is_custom = input.text().trim() != dialog.default_prompt.trim();
        form.avatar.generation_prompt = Some(input.text().trim().to_owned());
        form.avatar_prompt = None;
        form.avatar_generate_focus.focus(window, cx);
        self.generate_role_avatar(kind, cx);
    }

    pub(super) fn render_role_avatar_prompt(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (kind, form) = if let Some(form) = self
            .role_surfaces
            .create
            .as_ref()
            .filter(|form| form.avatar_prompt.is_some())
        {
            (RoleFormKind::Create, &form.fields)
        } else {
            let form = self
                .role_surfaces
                .edit
                .as_ref()
                .filter(|form| form.avatar_prompt.is_some())?;
            (RoleFormKind::Edit, &form.fields)
        };
        let dialog = form.avatar_prompt.as_ref()?;
        let root = cx.entity();
        let cancel_root = root.clone();
        let generate_root = root.clone();
        let reset_root = root.clone();
        let valid = prompt_valid(dialog.input.read(cx).text());
        let body = div()
            .when(cfg!(test), |body| {
                body.debug_selector(|| "ROLE_AVATAR_PROMPT_DIALOG".into())
            })
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_size(theme::text_body())
                    .text_color(theme::muted())
                    .child(crate::i18n::text("ui.avatar_prompt_hint")),
            )
            .child(
                div()
                    .when(cfg!(test), |field| {
                        field.debug_selector(|| "ROLE_AVATAR_PROMPT_INPUT".into())
                    })
                    .child(dialog.input.clone()),
            )
            .children((!valid).then(|| {
                div()
                    .text_size(theme::text_meta())
                    .text_color(theme::danger())
                    .child(crate::i18n::text("ui.avatar_prompt_invalid"))
            }));
        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .when(cfg!(test), |control| {
                        control.debug_selector(|| "ROLE_AVATAR_PROMPT_RESET".into())
                    })
                    .flex_1()
                    .child(
                        Button::new(
                            "avatar-prompt-reset",
                            crate::i18n::text("ui.avatar_prompt_reset"),
                        )
                        .variant(ButtonVariant::Ghost)
                        .focus_handle(dialog.reset_focus.clone())
                        .on_press(move |_, cx| {
                            reset_root.update(cx, |root, cx| {
                                let Some(form) = root.role_avatar_form_mut(kind) else {
                                    return;
                                };
                                let Some(dialog) = &form.avatar_prompt else {
                                    return;
                                };
                                dialog.input.update(cx, |field, cx| {
                                    field.reset(dialog.default_prompt.clone(), cx)
                                });
                            })
                        }),
                    ),
            )
            .child(
                div()
                    .when(cfg!(test), |control| {
                        control.debug_selector(|| "ROLE_AVATAR_PROMPT_CANCEL".into())
                    })
                    .child(
                        Button::new("avatar-prompt-cancel", crate::i18n::text("ui.cancel"))
                            .focus_handle(dialog.cancel_focus.clone())
                            .on_press(move |window, cx| {
                                cancel_root.update(cx, |root, cx| {
                                    root.close_role_avatar_prompt(kind, window, cx)
                                })
                            }),
                    ),
            )
            .child(
                div()
                    .when(cfg!(test), |control| {
                        control.debug_selector(|| "ROLE_AVATAR_PROMPT_CONFIRM".into())
                    })
                    .child(
                        Button::new(
                            "avatar-prompt-confirm",
                            crate::i18n::text("ui.avatar_prompt_generate"),
                        )
                        .variant(ButtonVariant::Primary)
                        .focus_handle(dialog.generate_focus.clone())
                        .disabled(!valid)
                        .on_press(move |window, cx| {
                            generate_root.update(cx, |root, cx| {
                                root.confirm_role_avatar_prompt(kind, window, cx)
                            })
                        }),
                    ),
            );
        Some(
            Modal::new(
                crate::i18n::text("ui.avatar_prompt_title"),
                body,
                Rc::new(move |window, cx| {
                    root.update(cx, |root, cx| {
                        root.close_role_avatar_prompt(kind, window, cx)
                    })
                }),
            )
            .width(OverlayWidth::Md)
            .footer(footer)
            .focus_order(vec![
                dialog.input.read(cx).focus_handle(),
                dialog.reset_focus.clone(),
                dialog.cancel_focus.clone(),
                dialog.generate_focus.clone(),
            ])
            .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_prompt_requires_nonempty_bounded_text() {
        assert!(!prompt_valid("  \n "));
        assert!(prompt_valid("A friendly robot avatar"));
        assert!(!prompt_valid(&"a".repeat(16 * 1024 + 1)));
    }
}
