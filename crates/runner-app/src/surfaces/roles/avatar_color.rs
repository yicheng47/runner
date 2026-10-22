use gpui::prelude::*;
use gpui::{div, rems, rgb, AnyElement, Context, CursorStyle, Entity, MouseButton};
use runner_app::ui::{Button, ButtonSize, ButtonVariant, TextField};
use runner_core::protocol::model::{normalize_avatar_color, parse_avatar_color};

use super::logic::RoleFormKind;
use super::RoleFormFields;
use crate::{theme, NativeRoot};

pub(crate) const PRESETS: [(&str, &str); 8] = [
    ("#fb923c", "ui.avatar_color_orange"),
    ("#3b82f6", "ui.avatar_color_blue"),
    ("#10b981", "ui.avatar_color_emerald"),
    ("#8b5cf6", "ui.avatar_color_purple"),
    ("#f43f5e", "ui.avatar_color_rose"),
    ("#06b6d4", "ui.avatar_color_cyan"),
    ("#f59e0b", "ui.avatar_color_amber"),
    ("#64748b", "ui.avatar_color_slate"),
];

pub(super) fn is_preset(color: &str) -> bool {
    PRESETS
        .iter()
        .any(|(preset, _)| preset.eq_ignore_ascii_case(color))
}

pub(super) fn color_field(color: Option<&str>, cx: &mut Context<NativeRoot>) -> Entity<TextField> {
    cx.new(|cx| TextField::new(cx.focus_handle(), color.unwrap_or(""), "#RRGGBB", true))
}

impl NativeRoot {
    pub(super) fn on_role_avatar_color_input(
        &mut self,
        kind: RoleFormKind,
        input: Entity<TextField>,
        cx: &mut Context<Self>,
    ) {
        let value = input.read(cx).text().trim().to_owned();
        let Some(form) = self.role_avatar_form_mut(kind).filter(|form| {
            form.avatar_color == input && !form.submitting && !form.avatar.generating
        }) else {
            return;
        };
        if value.is_empty() {
            form.avatar.color = None;
            form.avatar.custom_color = false;
            form.avatar.color_error = false;
        } else if let Some(color) = normalize_avatar_color(&value) {
            if form.avatar.color.as_ref() != Some(&color) || form.avatar.color_error {
                form.avatar.custom_color = true;
            }
            form.avatar.color = Some(color);
            form.avatar.color_error = false;
        } else {
            form.avatar.custom_color = true;
            form.avatar.color_error = true;
        }
        cx.notify();
    }

    fn select_role_avatar_color(
        &mut self,
        kind: RoleFormKind,
        choice: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(form) = self
            .role_avatar_form_mut(kind)
            .filter(|form| !form.submitting && !form.avatar.generating)
        else {
            return;
        };
        let color = match choice {
            0 => None,
            1..=8 => Some(PRESETS[choice - 1].0.to_owned()),
            9 => Some(
                normalize_avatar_color(form.avatar_color.read(cx).text())
                    .unwrap_or_else(|| "#10b981".into()),
            ),
            _ => return,
        };
        form.avatar.color = color.clone();
        form.avatar.custom_color = choice == 9;
        form.avatar.color_error = false;
        form.avatar_color
            .update(cx, |field, cx| field.reset(color.unwrap_or_default(), cx));
        cx.notify();
    }

    pub(super) fn render_role_avatar_colors(
        &self,
        form: &RoleFormFields,
        kind: RoleFormKind,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = cx.entity();
        let busy = form.submitting || form.avatar.generating;
        form.avatar_color
            .update(cx, |field, cx| field.set_disabled(busy, cx));
        div()
            .when(cfg!(test), |palette| {
                palette.debug_selector(|| "ROLE_AVATAR_COLOR_PALETTE".into())
            })
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_size(theme::text_meta())
                    .text_color(theme::muted())
                    .child(crate::i18n::text("ui.avatar_theme_color")),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .children((0..10).map(|index| {
                        let (color, label, selector) = match index {
                            0 => (
                                None,
                                "ui.avatar_color_auto",
                                "ROLE_AVATAR_COLOR_AUTO".to_owned(),
                            ),
                            9 => (
                                form.avatar.color.as_deref(),
                                "ui.avatar_color_custom",
                                "ROLE_AVATAR_COLOR_CUSTOM".to_owned(),
                            ),
                            _ => {
                                let (color, label) = PRESETS[index - 1];
                                (
                                    Some(color),
                                    label,
                                    format!("ROLE_AVATAR_COLOR_{}", &color[1..]),
                                )
                            }
                        };
                        let selected = match index {
                            0 => form.avatar.color.is_none() && !form.avatar.color_error,
                            9 => form.avatar.custom_color,
                            _ => !form.avatar.custom_color && form.avatar.color.as_deref() == color,
                        };
                        let swatch = color
                            .and_then(parse_avatar_color)
                            .map(|color| rgb(color).into())
                            .unwrap_or_else(theme::accent);
                        let click_root = root.clone();
                        let swatch_root = root.clone();
                        div()
                            .when(cfg!(test), |chip| {
                                chip.debug_selector(move || selector.clone())
                            })
                            .flex()
                            .items_center()
                            .gap_1()
                            .pl(rems(6. / 16.))
                            .rounded(rems(5. / 16.))
                            .border_1()
                            .border_color(if selected { swatch } else { theme::border() })
                            .bg(if selected {
                                theme::raised()
                            } else {
                                theme::bg()
                            })
                            .child(
                                div()
                                    .id(format!("role-avatar-color-swatch-{index}"))
                                    .flex_none()
                                    .size(rems(9. / 16.))
                                    .rounded_full()
                                    .bg(swatch)
                                    .cursor(if busy {
                                        CursorStyle::OperationNotAllowed
                                    } else {
                                        CursorStyle::PointingHand
                                    })
                                    .when(!busy, |dot| {
                                        dot.on_mouse_down(MouseButton::Left, |_, _, cx| {
                                            cx.stop_propagation()
                                        })
                                        .on_click(
                                            move |_, _, cx| {
                                                swatch_root.update(cx, |root, cx| {
                                                    root.select_role_avatar_color(kind, index, cx)
                                                })
                                            },
                                        )
                                    }),
                            )
                            .child(
                                Button::new(
                                    format!("role-avatar-color-{index}"),
                                    crate::i18n::text(label),
                                )
                                .size(ButtonSize::Sm)
                                .variant(ButtonVariant::Ghost)
                                .focus_handle(form.avatar_color_focus[index].clone())
                                .disabled(busy)
                                .on_press(move |_, cx| {
                                    click_root.update(cx, |root, cx| {
                                        root.select_role_avatar_color(kind, index, cx)
                                    })
                                }),
                            )
                    })),
            )
            .child(
                div()
                    .when(cfg!(test), |field| {
                        field.debug_selector(|| "ROLE_AVATAR_COLOR_HEX".into())
                    })
                    .w(rems(140. / 16.))
                    .child(form.avatar_color.clone()),
            )
            .children(form.avatar.color_error.then(|| {
                div()
                    .when(cfg!(test), |error| {
                        error.debug_selector(|| "ROLE_AVATAR_COLOR_ERROR".into())
                    })
                    .text_size(theme::text_meta())
                    .text_color(theme::danger())
                    .child(crate::i18n::text("ui.avatar_color_invalid"))
            }))
            .into_any_element()
    }
}
