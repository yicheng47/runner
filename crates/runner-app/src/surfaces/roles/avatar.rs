use gpui::prelude::*;
use gpui::{div, rems, AnyElement, Context, EntityId};
use runner_app::ui::{Button, ButtonSize, ButtonVariant};
use runner_core::protocol::model::AvatarStyle;
use runner_core::protocol::role::GenerateRoleAvatarInput;
use runner_core::protocol::DaemonClient;

use super::logic::RoleFormKind;
use super::RoleFormFields;
use crate::{theme, NativeRoot};

#[derive(Default)]
pub(super) struct RoleAvatarDraft {
    pub path: Option<String>,
    pub style: AvatarStyle,
    pub color: Option<String>,
    pub custom_color: bool,
    pub color_error: bool,
    pub generation_prompt: Option<String>,
    pub prompt_is_custom: bool,
    pub generating: bool,
    pub error: Option<String>,
    preview: Option<AvatarPreview>,
}

impl RoleAvatarDraft {
    pub fn saved(path: Option<String>, style: AvatarStyle, color: Option<String>) -> Self {
        Self {
            path,
            style,
            custom_color: color
                .as_ref()
                .is_some_and(|color| !super::avatar_color::is_preset(color)),
            color,
            ..Self::default()
        }
    }

    fn complete(&mut self, result: Result<AvatarPreview, String>) {
        self.generating = false;
        match result {
            Ok(preview) => {
                self.path = Some(preview.path.clone());
                self.preview = Some(preview);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    pub fn select_style(&mut self, style: AvatarStyle) {
        self.style = style;
        self.path = None;
        self.preview = None;
        self.error = None;
    }

    pub fn saved_successfully(&mut self) {
        if let Some(preview) = &mut self.preview {
            preview.retained = true;
        }
    }
}

// An unadopted result owns its file even if its form or window disappears.
struct AvatarPreview {
    path: String,
    client: DaemonClient,
    retained: bool,
}

impl Drop for AvatarPreview {
    fn drop(&mut self) {
        if self.retained {
            return;
        }
        let client = self.client.clone();
        let path = self.path.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("role-avatar-cleanup".into())
            .spawn(move || {
                if let Err(error) = client.role_avatar_discard(path) {
                    tracing::warn!(%error, "could not discard role avatar preview");
                }
            })
        {
            tracing::warn!(%error, "could not start role avatar preview cleanup");
        }
    }
}

fn generation_source_matches(
    requested: &GenerateRoleAvatarInput,
    current: &GenerateRoleAvatarInput,
) -> bool {
    requested.handle == current.handle
        && requested.display_name == current.display_name
        && requested.system_prompt == current.system_prompt
        && requested.avatar_color == current.avatar_color
        && requested.prompt == current.prompt
}

impl NativeRoot {
    pub(super) fn role_avatar_form_mut(
        &mut self,
        kind: RoleFormKind,
    ) -> Option<&mut RoleFormFields> {
        match kind {
            RoleFormKind::Create => self
                .role_surfaces
                .create
                .as_mut()
                .map(|form| &mut form.fields),
            RoleFormKind::Edit => self
                .role_surfaces
                .edit
                .as_mut()
                .map(|form| &mut form.fields),
        }
    }

    pub(super) fn role_avatar_input(
        &self,
        kind: RoleFormKind,
        cx: &gpui::App,
    ) -> Option<GenerateRoleAvatarInput> {
        let (handle, form) = match kind {
            RoleFormKind::Create => {
                let form = self.role_surfaces.create.as_ref()?;
                (form.avatar_seed(cx), &form.fields)
            }
            RoleFormKind::Edit => {
                let form = self.role_surfaces.edit.as_ref()?;
                (form.role.handle.clone(), &form.fields)
            }
        };
        Some(GenerateRoleAvatarInput {
            handle,
            display_name: form.display_name.read(cx).text().trim().to_owned(),
            system_prompt: super::logic::trimmed_option(form.system_prompt.read(cx).text()),
            avatar_color: form.avatar.color.clone(),
            prompt: form.avatar.generation_prompt.clone(),
        })
    }

    pub(super) fn generate_role_avatar(&mut self, kind: RoleFormKind, cx: &mut Context<Self>) {
        let Some(input) = self
            .role_avatar_input(kind, cx)
            .filter(|input| !input.handle.trim().is_empty())
        else {
            return;
        };
        let Some(form) = self.role_avatar_form_mut(kind) else {
            return;
        };
        if form.submitting
            || form.avatar.generating
            || form.avatar.color_error
            || !form.display_name_valid
        {
            return;
        }
        let form_id = form.display_name.entity_id();
        let requested = input.clone();
        form.avatar.generating = true;
        form.avatar.error = None;
        let client = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            client
                .role_avatar_generate(input)
                .map(|image| AvatarPreview {
                    path: image.path,
                    client,
                    retained: false,
                })
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, move |this, cx| {
                this.complete_role_avatar_generation(kind, form_id, requested, result, cx);
            });
        })
        .detach();
        cx.notify();
    }

    fn complete_role_avatar_generation(
        &mut self,
        kind: RoleFormKind,
        form_id: EntityId,
        requested: GenerateRoleAvatarInput,
        result: Result<AvatarPreview, String>,
        cx: &mut Context<Self>,
    ) {
        let source_matches = self
            .role_avatar_input(kind, cx)
            .is_some_and(|current| generation_source_matches(&requested, &current));
        let Some(form) = self
            .role_avatar_form_mut(kind)
            .filter(|form| form.display_name.entity_id() == form_id && form.avatar.generating)
        else {
            return;
        };
        if source_matches {
            form.avatar.complete(result);
        } else {
            form.avatar.complete(Err(
                crate::i18n::text("ui.role_avatar_source_changed").to_string()
            ));
        }
        cx.notify();
    }

    pub(super) fn select_role_avatar_style(
        &mut self,
        kind: RoleFormKind,
        style: AvatarStyle,
        cx: &mut Context<Self>,
    ) {
        if let Some(form) = self
            .role_avatar_form_mut(kind)
            .filter(|form| !form.submitting && !form.avatar.generating)
        {
            form.avatar.select_style(style);
            cx.notify();
        }
    }

    pub(super) fn render_role_avatar_controls(
        &self,
        form: &RoleFormFields,
        kind: RoleFormKind,
        handle_valid: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = cx.entity();
        let generate_root = root.clone();
        let has_image = form.avatar.path.is_some();
        let busy = form.submitting || form.avatar.generating;
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div().flex().flex_wrap().gap_2().children(
                    [AvatarStyle::Initials, AvatarStyle::Pixel]
                        .into_iter()
                        .map(|style| {
                            let select_root = root.clone();
                            let (id, label, selector, focus) = match style {
                                AvatarStyle::Initials => (
                                    "role-avatar-initials",
                                    "ui.role_avatar_initials",
                                    "ROLE_AVATAR_STYLE_INITIALS",
                                    form.avatar_restore_focus.clone(),
                                ),
                                AvatarStyle::Pixel => (
                                    "role-avatar-pixel",
                                    "ui.role_avatar_pixel",
                                    "ROLE_AVATAR_STYLE_PIXEL",
                                    form.avatar_pixel_focus.clone(),
                                ),
                            };
                            div()
                                .when(cfg!(test), |control| {
                                    control.debug_selector(move || selector.into())
                                })
                                .child(
                                    div()
                                        .when(
                                            cfg!(test)
                                                && has_image
                                                && style == AvatarStyle::Initials,
                                            |control| {
                                                control
                                                    .debug_selector(|| "ROLE_AVATAR_RESTORE".into())
                                            },
                                        )
                                        .child(
                                            Button::new(id, crate::i18n::text(label))
                                                .size(ButtonSize::Sm)
                                                .variant(
                                                    if !has_image && form.avatar.style == style {
                                                        ButtonVariant::Tinted
                                                    } else {
                                                        ButtonVariant::Secondary
                                                    },
                                                )
                                                .focus_handle(focus)
                                                .disabled(busy)
                                                .on_press(move |_, cx| {
                                                    select_root.update(cx, |root, cx| {
                                                        root.select_role_avatar_style(
                                                            kind, style, cx,
                                                        );
                                                    })
                                                }),
                                        ),
                                )
                        }),
                ),
            )
            .child(self.render_role_avatar_colors(form, kind, cx))
            .child(
                div()
                    .when(cfg!(test), |control| {
                        control.debug_selector(|| "ROLE_AVATAR_GENERATE".into())
                    })
                    .child(
                        Button::new(
                            "role-avatar-generate",
                            crate::i18n::text(if form.avatar.generating {
                                "ui.role_avatar_generating"
                            } else if has_image {
                                "ui.role_avatar_regenerate"
                            } else {
                                "ui.role_avatar_generate"
                            }),
                        )
                        .size(ButtonSize::Sm)
                        .variant(if has_image {
                            ButtonVariant::Tinted
                        } else {
                            ButtonVariant::Secondary
                        })
                        .focus_handle(form.avatar_generate_focus.clone())
                        .loading(form.avatar.generating)
                        .disabled(
                            busy || form.avatar.color_error
                                || !form.display_name_valid
                                || !handle_valid,
                        )
                        .on_press(move |window, cx| {
                            generate_root.update(cx, |root, cx| {
                                root.open_role_avatar_prompt(kind, window, cx)
                            })
                        }),
                    ),
            )
            .child(
                div()
                    .text_size(theme::text_meta())
                    .text_color(theme::faint())
                    .max_w(rems(super::ROLE_COLUMN_WIDTH / 16.))
                    .child(crate::i18n::text("ui.role_avatar_codex_hint")),
            )
            .children(form.avatar.error.clone().map(|error| {
                div()
                    .when(cfg!(test), |error| {
                        error.debug_selector(|| "ROLE_AVATAR_ERROR".into())
                    })
                    .max_w(rems(super::ROLE_COLUMN_WIDTH / 16.))
                    .text_size(theme::text_meta())
                    .text_color(theme::danger())
                    .child(error)
            }))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runner_core::protocol::{ClientError, EventSubscription, Request, Response, Transport};
    use std::sync::{mpsc, Arc, Mutex};
    use std::time::Duration;

    struct CleanupTransport(Mutex<mpsc::Sender<String>>);

    impl Transport for CleanupTransport {
        fn call(&self, request: Request) -> Result<Response, ClientError> {
            match request {
                Request::role_avatar_discard { path } => {
                    self.0.lock().unwrap().send(path).unwrap();
                    Ok(Response::role_avatar_discard(Ok(())))
                }
                _ => panic!("unexpected avatar cleanup request"),
            }
        }

        fn subscribe(&self) -> Box<dyn EventSubscription> {
            unreachable!()
        }
    }

    fn preview(path: &str) -> (AvatarPreview, mpsc::Receiver<String>) {
        let (sender, receiver) = mpsc::channel();
        let client = DaemonClient::new(Arc::new(CleanupTransport(Mutex::new(sender))));
        (
            AvatarPreview {
                path: path.into(),
                client,
                retained: false,
            },
            receiver,
        )
    }

    #[test]
    fn failed_generation_preserves_the_previous_avatar() {
        let mut draft =
            RoleAvatarDraft::saved(Some("saved.png".into()), AvatarStyle::Initials, None);
        draft.generating = true;
        draft.complete(Err("generation failed".into()));
        assert!(!draft.generating);
        assert_eq!(draft.path.as_deref(), Some("saved.png"));
        assert_eq!(draft.error.as_deref(), Some("generation failed"));
    }

    #[test]
    fn cancelled_or_restored_previews_are_discarded() {
        let (image, discarded) = preview("preview.png");
        let mut draft = RoleAvatarDraft::default();
        draft.complete(Ok(image));
        assert_eq!(draft.path.as_deref(), Some("preview.png"));
        draft.select_style(AvatarStyle::Initials);
        assert_eq!(draft.path, None);
        assert_eq!(
            discarded.recv_timeout(Duration::from_secs(2)).unwrap(),
            "preview.png"
        );
        let (image, discarded) = preview("cancelled.png");
        draft.complete(Ok(image));
        drop(draft);
        assert_eq!(
            discarded.recv_timeout(Duration::from_secs(2)).unwrap(),
            "cancelled.png"
        );
    }

    #[test]
    fn replacing_a_preview_discards_only_the_superseded_image() {
        let (first, discarded) = preview("first.png");
        let (second, retained) = preview("second.png");
        let mut draft =
            RoleAvatarDraft::saved(Some("saved.png".into()), AvatarStyle::Initials, None);
        draft.complete(Ok(first));
        draft.complete(Ok(second));
        assert_eq!(
            discarded.recv_timeout(Duration::from_secs(2)).unwrap(),
            "first.png"
        );
        draft.saved_successfully();
        drop(draft);
        assert!(retained.recv_timeout(Duration::from_millis(50)).is_err());
    }

    #[test]
    fn a_late_result_without_a_form_cleans_up_its_image() {
        let (image, discarded) = preview("late.png");
        drop(Ok::<_, String>(image));
        assert_eq!(
            discarded.recv_timeout(Duration::from_secs(2)).unwrap(),
            "late.png"
        );
    }
    #[test]
    fn generation_results_require_the_original_role_information() {
        let requested = GenerateRoleAvatarInput {
            handle: "coder".into(),
            display_name: "Coder".into(),
            system_prompt: Some("Build".into()),
            avatar_color: None,
            prompt: None,
        };
        assert!(generation_source_matches(&requested, &requested));
        for current in [
            GenerateRoleAvatarInput {
                handle: "reviewer".into(),
                ..requested.clone()
            },
            GenerateRoleAvatarInput {
                display_name: "Reviewer".into(),
                ..requested.clone()
            },
            GenerateRoleAvatarInput {
                system_prompt: Some("Review".into()),
                ..requested.clone()
            },
        ] {
            assert!(!generation_source_matches(&requested, &current));
        }
    }

    #[test]
    fn selecting_pixel_clears_the_preview_but_keeps_saved_images_unadopted() {
        let (image, discarded) = preview("preview.png");
        let mut draft =
            RoleAvatarDraft::saved(Some("saved.png".into()), AvatarStyle::Initials, None);
        draft.complete(Ok(image));
        draft.select_style(AvatarStyle::Pixel);
        assert_eq!(draft.style, AvatarStyle::Pixel);
        assert_eq!(draft.path, None);
        assert_eq!(
            discarded.recv_timeout(Duration::from_secs(2)).unwrap(),
            "preview.png"
        );
        draft.select_style(AvatarStyle::Initials);
        assert_eq!(draft.style, AvatarStyle::Initials);
    }

    #[test]
    fn an_ai_image_keeps_the_selected_base_style_for_fallback() {
        let (image, _) = preview("preview.png");
        let mut draft = RoleAvatarDraft::saved(None, AvatarStyle::Pixel, None);
        draft.complete(Ok(image));
        assert_eq!(draft.style, AvatarStyle::Pixel);
        assert_eq!(draft.path.as_deref(), Some("preview.png"));
        draft.saved_successfully();
    }
}
