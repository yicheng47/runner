use std::path::{Path, PathBuf};

use gpui::prelude::*;
use gpui::{
    div, px, rems, rgb, AnyElement, Context, CursorStyle, Entity, FontWeight, KeyDownEvent,
    PathPromptOptions, SharedString, Window,
};
use runner_app::ui::{
    Button, ButtonSize, ButtonVariant, RoleAvatar, SettingsCard, SettingsRow, TextField,
};
use runner_core::protocol::model::{normalize_avatar_color, parse_avatar_color, AvatarStyle};

use crate::app_settings::{import_personal_avatar, PersonalAvatarSettings};
use crate::surfaces::roles::avatar_color::PRESETS;
use crate::{theme, NativeRoot};

pub(crate) struct PersonalAvatarEditor {
    color: Entity<TextField>,
    importing: bool,
    error: Option<String>,
}

impl PersonalAvatarEditor {
    pub(crate) fn new(settings: &PersonalAvatarSettings, cx: &mut Context<NativeRoot>) -> Self {
        let color = cx.new(|cx| {
            TextField::new(
                cx.focus_handle(),
                settings.color.as_deref().unwrap_or_default(),
                "#RRGGBB",
                true,
            )
        });
        Self {
            color,
            importing: false,
            error: None,
        }
    }

    pub(crate) fn render(
        &self,
        settings: &PersonalAvatarSettings,
        root: Entity<NativeRoot>,
        cx: &mut Context<NativeRoot>,
    ) -> AnyElement {
        self.color
            .update(cx, |field, cx| field.set_disabled(self.importing, cx));
        let busy = self.importing;
        let preview = div()
            .when(cfg!(test), |preview| {
                preview.debug_selector(|| "SETTINGS_PERSONAL_AVATAR_PREVIEW".into())
            })
            .flex()
            .items_center()
            .gap_4()
            .child(
                RoleAvatar::new("human", 80.)
                    .style(settings.style)
                    .color(settings.color.clone())
                    .image_path(settings.image_path.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .text_size(theme::text_body())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::text())
                            .child(crate::i18n::text("ui.personal_avatar_you")),
                    )
                    .child(
                        div()
                            .text_size(theme::text_meta())
                            .text_color(theme::muted())
                            .child(crate::i18n::text("ui.personal_avatar_preview_hint")),
                    ),
            );

        let styles = [
            (AvatarStyle::Initials, "ui.role_avatar_initials", "initials"),
            (AvatarStyle::Pixel, "ui.role_avatar_pixel", "pixel"),
        ];
        let style_buttons = styles.into_iter().map(|(style, label, key)| {
            let selected = settings.image_path.is_none() && settings.style == style;
            let click_root = root.clone();
            let selector_key = key;
            div()
                .when(cfg!(test), |button| {
                    button.debug_selector(move || {
                        format!(
                            "SETTINGS_PERSONAL_AVATAR_STYLE_{}_{}",
                            selector_key.to_ascii_uppercase(),
                            if selected { "SELECTED" } else { "UNSELECTED" }
                        )
                    })
                })
                .child(
                    Button::new(
                        format!("settings-personal-avatar-style-{key}"),
                        crate::i18n::text(label),
                    )
                    .size(ButtonSize::Sm)
                    .variant(if selected {
                        ButtonVariant::Tinted
                    } else {
                        ButtonVariant::Secondary
                    })
                    .disabled(busy)
                    .on_press(move |_, app| {
                        click_root
                            .update(app, |this, cx| this.set_personal_avatar_style(style, cx));
                    }),
                )
        });
        let style_control = div().flex().items_center().gap_2().children(style_buttons);

        let color_choices = std::iter::once((None, "ui.avatar_color_auto", "auto".to_owned()))
            .chain(
                PRESETS
                    .iter()
                    .map(|(color, label)| (Some(*color), *label, color[1..].to_owned())),
            )
            .map(|(color, label, key)| {
                let selected = match color {
                    None => settings.color.is_none(),
                    Some(color) => settings
                        .color
                        .as_deref()
                        .is_some_and(|current| current.eq_ignore_ascii_case(color)),
                };
                let swatch = color
                    .and_then(parse_avatar_color)
                    .map(|value| rgb(value).into())
                    .unwrap_or_else(theme::accent);
                let choice = color.map(str::to_owned);
                let click_choice = choice.clone();
                let key_choice = choice;
                let click_root = root.clone();
                let key_root = root.clone();
                let selector_key = key.clone();
                div()
                    .when(cfg!(test), |button| {
                        button.debug_selector(move || {
                            format!(
                                "SETTINGS_PERSONAL_AVATAR_COLOR_{}",
                                selector_key.to_ascii_uppercase()
                            )
                        })
                    })
                    .id(format!("settings-personal-avatar-color-{key}"))
                    .tab_index(0)
                    .flex()
                    .items_center()
                    .gap_1()
                    .px(rems(6. / 16.))
                    .rounded(rems(5. / 16.))
                    .border_1()
                    .border_color(if selected { swatch } else { theme::border() })
                    .bg(if selected {
                        theme::raised()
                    } else {
                        theme::bg()
                    })
                    .opacity(if busy { 0.5 } else { 1. })
                    .cursor(if busy {
                        CursorStyle::OperationNotAllowed
                    } else {
                        CursorStyle::PointingHand
                    })
                    .child(div().size(rems(9. / 16.)).rounded_full().bg(swatch))
                    .child(
                        div()
                            .text_size(theme::text_meta())
                            .text_color(if selected {
                                theme::text()
                            } else {
                                theme::muted()
                            })
                            .child(crate::i18n::text(label)),
                    )
                    .on_click(move |_, _, cx| {
                        if !busy {
                            let choice = click_choice.clone();
                            click_root.update(cx, move |this, cx| {
                                this.select_personal_avatar_color(choice, cx)
                            });
                        }
                    })
                    .on_key_down(move |event: &KeyDownEvent, _, cx| {
                        if !busy && matches!(event.keystroke.key.as_str(), "enter" | "space") {
                            cx.stop_propagation();
                            let choice = key_choice.clone();
                            key_root.update(cx, move |this, cx| {
                                this.select_personal_avatar_color(choice, cx)
                            });
                        }
                    })
            });
        let color_palette = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .children(color_choices);
        let color_field = div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().w(px(190.)).child(self.color.clone()))
            .child(
                Button::new(
                    "settings-personal-avatar-color-apply",
                    crate::i18n::text("ui.personal_avatar_apply"),
                )
                .size(ButtonSize::Sm)
                .variant(ButtonVariant::Secondary)
                .disabled(busy)
                .on_press({
                    let apply_root = root.clone();
                    move |_, app| {
                        apply_root.update(app, |this, cx| this.apply_personal_avatar_color(cx));
                    }
                }),
            );

        let image_name = settings
            .image_path
            .as_deref()
            .and_then(|path| Path::new(path).file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| crate::i18n::text("ui.personal_avatar_no_image").to_owned());
        let choose_root = root.clone();
        let choose = Button::new(
            "settings-personal-avatar-choose-image",
            if busy {
                crate::i18n::text("ui.personal_avatar_importing")
            } else {
                crate::i18n::text("ui.personal_avatar_choose_png")
            },
        )
        .size(ButtonSize::Sm)
        .variant(ButtonVariant::Secondary)
        .icon(if busy {
            "loader.svg"
        } else {
            "folder-open.svg"
        })
        .disabled(busy)
        .on_press(move |window, app| {
            choose_root.update(app, |this, cx| {
                this.choose_personal_avatar_image(window, cx)
            });
        });
        let clear_root = root.clone();
        let clear = Button::new(
            "settings-personal-avatar-clear-image",
            crate::i18n::text("ui.personal_avatar_clear_image"),
        )
        .size(ButtonSize::Sm)
        .variant(ButtonVariant::Ghost)
        .disabled(busy || settings.image_path.is_none())
        .on_press(move |_, app| {
            clear_root.update(app, |this, cx| this.clear_personal_avatar_image(cx));
        });
        let image_control = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap_2()
            .child(
                div()
                    .min_w(px(120.))
                    .text_size(theme::text_meta())
                    .text_color(theme::muted())
                    .child(SharedString::from(image_name)),
            )
            .child(choose)
            .child(clear);

        let rows = vec![
            SettingsRow::new(crate::i18n::text("ui.personal_avatar_preview"), preview)
                .into_any_element(),
            SettingsRow::new(crate::i18n::text("ui.personal_avatar_style"), style_control)
                .into_any_element(),
            SettingsRow::new(crate::i18n::text("ui.avatar_theme_color"), color_palette)
                .wrap_control()
                .into_any_element(),
            SettingsRow::new(crate::i18n::text("ui.avatar_color_custom"), color_field)
                .wrap_control()
                .subtitle(crate::i18n::text("ui.personal_avatar_color_hint"))
                .into_any_element(),
            SettingsRow::new(crate::i18n::text("ui.personal_avatar_image"), image_control)
                .wrap_control()
                .subtitle(crate::i18n::text("ui.personal_avatar_png_limits"))
                .into_any_element(),
        ];
        div()
            .when(cfg!(test), |content| {
                content.debug_selector(|| "SETTINGS_PERSONAL_AVATAR_CONTENT".into())
            })
            .flex()
            .flex_col()
            .gap_3()
            .child(SettingsCard::new(rows))
            .children(self.error.as_ref().map(|error| {
                div()
                    .when(cfg!(test), |message| {
                        message.debug_selector(|| "SETTINGS_PERSONAL_AVATAR_ERROR".into())
                    })
                    .rounded(rems(6. / 16.))
                    .border_1()
                    .border_color(theme::with_alpha(theme::danger(), 0.35))
                    .bg(theme::with_alpha(theme::danger(), 0.08))
                    .px_3()
                    .py_2()
                    .text_size(theme::text_meta())
                    .text_color(theme::danger())
                    .child(error.clone())
            }))
            .into_any_element()
    }
}

impl NativeRoot {
    fn try_update_personal_avatar(
        &mut self,
        update: impl FnOnce(&mut PersonalAvatarSettings) -> bool,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<bool> {
        self.app_store.update(cx, |store, cx| {
            store.begin_personal_avatar_edit();
            store.try_update_settings(|settings| update(&mut settings.personal_avatar), cx)
        })
    }

    fn set_personal_avatar_error(&mut self, error: Option<String>, cx: &mut Context<Self>) {
        self.settings_page.personal_avatar.error = error;
        cx.notify();
    }

    fn save_personal_avatar_error(
        &mut self,
        error: impl std::fmt::Display,
        cx: &mut Context<Self>,
    ) {
        let error = crate::i18n::tf(
            "ui.personal_avatar_save_error",
            &[("error", error.to_string())],
        );
        self.set_personal_avatar_error(Some(error), cx);
    }

    fn set_personal_avatar_style(&mut self, style: AvatarStyle, cx: &mut Context<Self>) {
        match self.try_update_personal_avatar(
            move |settings| {
                let changed = settings.style != style || settings.image_path.is_some();
                if !changed {
                    return false;
                }
                settings.style = style;
                settings.image_path = None;
                true
            },
            cx,
        ) {
            Ok(_) => self.set_personal_avatar_error(None, cx),
            Err(error) => self.save_personal_avatar_error(error, cx),
        }
    }

    fn select_personal_avatar_color(&mut self, color: Option<String>, cx: &mut Context<Self>) {
        let current = self
            .app_store
            .read(cx)
            .settings
            .personal_avatar
            .color
            .clone();
        let result = self.try_update_personal_avatar(
            move |settings| {
                if settings.color == color {
                    return false;
                }
                settings.color = color;
                true
            },
            cx,
        );
        match result {
            Ok(_) => {
                let saved = self
                    .app_store
                    .read(cx)
                    .settings
                    .personal_avatar
                    .color
                    .clone();
                self.settings_page
                    .personal_avatar
                    .color
                    .update(cx, |field, cx| field.reset(saved.unwrap_or_default(), cx));
                self.set_personal_avatar_error(None, cx);
            }
            Err(error) => {
                self.settings_page
                    .personal_avatar
                    .color
                    .update(cx, |field, cx| field.reset(current.unwrap_or_default(), cx));
                self.save_personal_avatar_error(error, cx);
            }
        }
    }

    fn apply_personal_avatar_color(&mut self, cx: &mut Context<Self>) {
        let value = self
            .settings_page
            .personal_avatar
            .color
            .read(cx)
            .text()
            .trim()
            .to_owned();
        let color = if value.is_empty() {
            None
        } else if let Some(color) = normalize_avatar_color(&value) {
            Some(color)
        } else {
            self.set_personal_avatar_error(
                Some(crate::i18n::text("ui.avatar_color_invalid").into()),
                cx,
            );
            return;
        };
        let result = self.try_update_personal_avatar(
            move |settings| {
                if settings.color == color {
                    return false;
                }
                settings.color = color;
                true
            },
            cx,
        );
        match result {
            Ok(_) => {
                let saved = self
                    .app_store
                    .read(cx)
                    .settings
                    .personal_avatar
                    .color
                    .clone();
                self.settings_page
                    .personal_avatar
                    .color
                    .update(cx, |field, cx| field.reset(saved.unwrap_or_default(), cx));
                self.set_personal_avatar_error(None, cx);
            }
            Err(error) => self.save_personal_avatar_error(error, cx),
        }
    }

    fn clear_personal_avatar_image(&mut self, cx: &mut Context<Self>) {
        match self.try_update_personal_avatar(|settings| settings.image_path.take().is_some(), cx) {
            Ok(_) => self.set_personal_avatar_error(None, cx),
            Err(error) => self.save_personal_avatar_error(error, cx),
        }
    }

    fn choose_personal_avatar_image(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_page.personal_avatar.importing {
            return;
        }
        let epoch = self
            .app_store
            .update(cx, |store, _| store.begin_personal_avatar_edit());
        self.settings_page.personal_avatar.importing = true;
        self.settings_page.personal_avatar.error = None;
        cx.notify();
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(crate::i18n::text("ui.personal_avatar_choose_png").into()),
        });
        cx.spawn(async move |weak, cx| {
            let selected = selected
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result.map_err(|error| error.to_string()));
            match selected {
                Ok(Some(paths)) => match paths.into_iter().next() {
                    Some(path) => {
                        let _ = weak.update(cx, |this, cx| {
                            this.start_personal_avatar_import(path, epoch, cx)
                        });
                    }
                    None => {
                        let _ = weak.update(cx, |this, cx| {
                            this.settings_page.personal_avatar.importing = false;
                            cx.notify();
                        });
                    }
                },
                Ok(None) => {
                    let _ = weak.update(cx, |this, cx| {
                        this.settings_page.personal_avatar.importing = false;
                        cx.notify();
                    });
                }
                Err(error) => {
                    let _ = weak.update(cx, |this, cx| {
                        this.settings_page.personal_avatar.importing = false;
                        this.set_personal_avatar_error(
                            Some(crate::i18n::tf(
                                "ui.personal_avatar_import_error",
                                &[("error", error)],
                            )),
                            cx,
                        );
                    });
                }
            }
        })
        .detach();
    }

    fn start_personal_avatar_import(
        &mut self,
        source: PathBuf,
        epoch: u64,
        cx: &mut Context<Self>,
    ) {
        let app_data_dir = self.app_store.read(cx).app_data_dir.clone();
        let task = cx.background_spawn(async move {
            import_personal_avatar(&source, &app_data_dir).map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let cleanup = result.as_ref().ok().cloned();
            if weak
                .update(cx, |this, cx| {
                    this.complete_personal_avatar_import(result, epoch, cx)
                })
                .is_err()
            {
                if let Some(path) = cleanup {
                    let _ = std::fs::remove_file(path);
                }
            }
        })
        .detach();
    }

    fn complete_personal_avatar_import(
        &mut self,
        result: Result<PathBuf, String>,
        epoch: u64,
        cx: &mut Context<Self>,
    ) {
        self.settings_page.personal_avatar.importing = false;
        let path = match result {
            Ok(path) => path,
            Err(error) => {
                self.set_personal_avatar_error(
                    Some(crate::i18n::tf(
                        "ui.personal_avatar_import_error",
                        &[("error", error)],
                    )),
                    cx,
                );
                return;
            }
        };
        if self.app_store.read(cx).personal_avatar_edit_epoch != epoch {
            let _ = std::fs::remove_file(&path);
            self.set_personal_avatar_error(
                Some(crate::i18n::text("ui.personal_avatar_source_changed").into()),
                cx,
            );
            return;
        }
        let saved_path = path.to_string_lossy().into_owned();
        match self.try_update_personal_avatar(
            move |settings| {
                settings.image_path = Some(saved_path);
                true
            },
            cx,
        ) {
            Ok(true) => self.set_personal_avatar_error(None, cx),
            Ok(false) => {
                let _ = std::fs::remove_file(&path);
                self.set_personal_avatar_error(
                    Some(crate::i18n::text("ui.personal_avatar_not_saved").into()),
                    cx,
                );
            }
            Err(error) => {
                let _ = std::fs::remove_file(&path);
                self.save_personal_avatar_error(error, cx);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[test]
    fn personal_avatar_late_import_does_not_override_another_windows_choice() {
        let root = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, root.path());
        let first = cx.add_window(|window, cx| {
            NativeRoot::new(
                "avatar-first".into(),
                root.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            )
        });
        let second = cx.add_window(|window, cx| {
            NativeRoot::new(
                "avatar-second".into(),
                root.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            )
        });
        cx.run_until_parked();
        let epoch = store.update(&mut cx, |store, _| store.begin_personal_avatar_edit());
        second
            .update(&mut cx, |root, _, cx| {
                root.set_personal_avatar_style(AvatarStyle::Initials, cx);
            })
            .unwrap();
        let copy = root.path().join("personal-avatars/avatar-candidate.png");
        std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
        std::fs::write(&copy, b"already validated by import helper").unwrap();
        first
            .update(&mut cx, |root, _, cx| {
                root.complete_personal_avatar_import(Ok(copy.clone()), epoch, cx);
                assert!(root.settings_page.personal_avatar.error.is_some());
            })
            .unwrap();
        assert!(!copy.exists());
        store.read_with(&cx, |store, _| {
            assert_eq!(store.settings.personal_avatar.style, AvatarStyle::Initials);
            assert!(store.settings.personal_avatar.image_path.is_none());
        });

        let kept = root.path().join("personal-avatars/avatar-kept.png");
        std::fs::write(&kept, b"validated image").unwrap();
        let epoch = store.update(&mut cx, |store, _| store.begin_personal_avatar_edit());
        first
            .update(&mut cx, |root, _, cx| {
                root.complete_personal_avatar_import(Ok(kept.clone()), epoch, cx);
                assert!(root.settings_page.personal_avatar.error.is_none());
            })
            .unwrap();
        let previous = store.read_with(&cx, |store, _| store.settings.personal_avatar.clone());
        assert_eq!(previous.image_path.as_deref(), kept.to_str());
        assert!(kept.is_file());
        let failed = root.path().join("personal-avatars/avatar-failed.png");
        std::fs::write(&failed, b"validated image").unwrap();
        let settings_path = root.path().join("settings.json");
        std::fs::remove_file(&settings_path).unwrap();
        std::fs::create_dir(&settings_path).unwrap();
        let epoch = store.update(&mut cx, |store, _| store.begin_personal_avatar_edit());
        first
            .update(&mut cx, |root, _, cx| {
                root.complete_personal_avatar_import(Ok(failed.clone()), epoch, cx);
                assert!(root.settings_page.personal_avatar.error.is_some());
            })
            .unwrap();
        assert!(!failed.exists());
        assert!(kept.is_file());
        assert_eq!(
            store.read_with(&cx, |store, _| store.settings.personal_avatar.clone()),
            previous
        );
    }
}
