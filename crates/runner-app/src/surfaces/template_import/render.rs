use std::collections::HashMap;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    div, px, relative, rems, svg, AnyElement, Context, CursorStyle, DragMoveEvent, ExternalPaths,
    FontWeight, KeyDownEvent, MouseButton, SharedString, Window,
};
use runner_app::ui::markdown::render_compact;
use runner_app::ui::{Button, ButtonSize, ButtonVariant, IconButton, IconButtonSize, RoleAvatar};
use runner_core::template::{
    prompt_stats, CrewTemplate, RoleTemplate, StandaloneRoleTemplate, Template, TemplateMeta,
};

use super::logic::{
    bulk_missing_runtimes, effective_slot_summary, format_prompt_size, prompt_excerpt,
    runtime_meta_line, source_line_segments, ReaderItem, ReaderItemKind, SourceSegment,
    SAMPLE_CREWS_URL,
};
use super::{ImportErrorState, ImportStep, PreviewState, ReaderMode};
use crate::chat_icon::ChatIcon;
use crate::surfaces::crews::{crew_picture, crew_summary};
use crate::surfaces::roles::logic::{runtime_display_name, validate_role_handle};
use crate::theme;
use crate::NativeRoot;

const UPLOAD_MODAL_WIDTH: f32 = 560.;
const CREW_PREVIEW_MODAL_WIDTH: f32 = 640.;
const ROLE_PREVIEW_MODAL_WIDTH: f32 = 580.;
const READER_MODAL_WIDTH: f32 = 640.;
const ERROR_MODAL_WIDTH: f32 = 540.;

fn warn_tint() -> gpui::Hsla {
    theme::with_alpha(
        theme::warning(),
        if theme::active_variant().is_light() {
            0.10
        } else {
            0.12
        },
    )
}

fn warn_border() -> gpui::Hsla {
    theme::with_alpha(
        theme::warning(),
        if theme::active_variant().is_light() {
            0.35
        } else {
            0.38
        },
    )
}

fn danger_tint() -> gpui::Hsla {
    theme::with_alpha(
        theme::danger(),
        if theme::active_variant().is_light() {
            0.09
        } else {
            0.12
        },
    )
}

fn danger_border() -> gpui::Hsla {
    theme::with_alpha(
        theme::danger(),
        if theme::active_variant().is_light() {
            0.32
        } else {
            0.38
        },
    )
}

fn accent_tint() -> gpui::Hsla {
    theme::with_alpha(
        theme::accent(),
        if theme::active_variant().is_light() {
            0.10
        } else {
            0.12
        },
    )
}

fn accent_border() -> gpui::Hsla {
    theme::with_alpha(
        theme::accent(),
        if theme::active_variant().is_light() {
            0.30
        } else {
            0.28
        },
    )
}

impl NativeRoot {
    pub(crate) fn render_template_import_modal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let modal = self.template_import_modal.as_ref()?;
        let busy = modal.is_busy();
        let has_open_select = modal.has_open_select(cx);
        let focus = modal.focus.clone();
        let width_px = match &modal.step {
            ImportStep::Upload => UPLOAD_MODAL_WIDTH,
            ImportStep::Preview(state) => match &state.plan.template {
                Template::Crew(_) => CREW_PREVIEW_MODAL_WIDTH,
                Template::Role(_) => ROLE_PREVIEW_MODAL_WIDTH,
            },
            ImportStep::Reader { .. } => READER_MODAL_WIDTH,
            ImportStep::Error(_) => ERROR_MODAL_WIDTH,
        };
        let rem = window.rem_size();
        let preferred = rem * (width_px / 16.);
        let available = window.viewport_size().width - rem * 2.;
        let panel_width = if preferred < available {
            preferred
        } else {
            available
        };

        let (header, body, footer, fixed_body) = match &modal.step {
            ImportStep::Upload => (
                self.render_upload_header(busy, cx),
                self.render_upload_body(busy, cx),
                self.render_upload_footer(busy, cx),
                false,
            ),
            ImportStep::Preview(state) => match &state.plan.template {
                Template::Crew(crew) => (
                    self.render_crew_preview_header(state, crew, busy, cx),
                    self.render_crew_preview_body(state, crew, cx),
                    self.render_crew_preview_footer(state, crew, busy, cx),
                    false,
                ),
                Template::Role(standalone) => (
                    self.render_role_preview_header(state, standalone, busy, cx),
                    self.render_role_preview_body(state, standalone, cx),
                    self.render_role_preview_footer(state, busy, cx),
                    false,
                ),
            },
            ImportStep::Reader {
                items, index, mode, ..
            } => {
                let item = &items[*index];
                (
                    self.render_reader_header(item, cx),
                    self.render_reader_body(item, *mode, cx),
                    self.render_reader_footer(items, *index, cx),
                    true,
                )
            }
            ImportStep::Error(error) => (
                self.render_error_header(busy, cx),
                self.render_error_body(error, busy, cx),
                self.render_error_footer(busy, cx),
                false,
            ),
        };

        let scroll = modal.scroll.clone();
        let scrollbar = modal.scrollbar.clone();
        let root = cx.entity();
        let backdrop_root = root.clone();
        let key_root = root;

        let panel = div()
            .id("template-import-modal-panel")
            .when(cfg!(test), |el| {
                el.debug_selector(|| "TEMPLATE_IMPORT_MODAL".into())
            })
            .w(panel_width)
            .max_h(relative(0.86))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded(rems(10. / 16.))
            .border_1()
            .border_color(theme::border_strong())
            .bg(theme::panel())
            .shadow_2xl()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .flex_none()
                    .px(rems(20. / 16.))
                    .py(rems(14. / 16.))
                    .border_b_1()
                    .border_color(theme::border())
                    .child(header),
            )
            .child(if fixed_body {
                div()
                    .relative()
                    .min_h(px(0.))
                    .flex_1()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .child(body)
            } else {
                div()
                    .relative()
                    .min_h(px(0.))
                    .flex_auto()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .id("template-import-body-scroll")
                            .min_h(px(0.))
                            .flex_auto()
                            .overflow_y_scroll()
                            .scrollbar_width(px(0.))
                            .track_scroll(&scroll)
                            .px(rems(20. / 16.))
                            .py(rems(18. / 16.))
                            .child(body),
                    )
                    .child(scrollbar)
            })
            .child(
                div()
                    .flex_none()
                    .border_t_1()
                    .border_color(theme::border())
                    .bg(theme::with_alpha(theme::bg(), 0.45))
                    .px(rems(20. / 16.))
                    .py(rems(14. / 16.))
                    .child(footer),
            );

        Some(
            div()
                .key_context("TemplateImportModal")
                .track_focus(&focus)
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .p_4()
                .bg(gpui::rgba(0x00000099))
                .occlude()
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    if !busy && !has_open_select {
                        backdrop_root
                            .update(cx, |this, cx| this.close_template_import_modal(window, cx));
                    }
                })
                .on_key_down(move |event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" && !has_open_select {
                        cx.stop_propagation();
                        key_root.update(cx, |this, cx| {
                            this.handle_template_import_escape(window, cx);
                        });
                    }
                })
                .child(panel)
                .map(crate::platform_ui::window_scrim)
                .into_any_element(),
        )
    }

    fn render_close_button(&self, busy: bool, cx: &mut Context<Self>) -> AnyElement {
        let root = cx.entity();
        IconButton::new("template-import-close", "close.svg")
            .size(IconButtonSize::Sm)
            .disabled(busy)
            .tooltip(crate::i18n::text("ui.close"))
            .on_press(move |window, cx| {
                root.update(cx, |this, cx| this.close_template_import_modal(window, cx));
            })
            .into_any_element()
    }

    fn render_upload_header(&self, busy: bool, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(2. / 16.))
                    .child(
                        div()
                            .text_size(theme::text_lead())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child(crate::i18n::text("template.import_title")),
                    )
                    .child(
                        div()
                            .text_size(theme::text_meta())
                            .text_color(theme::muted())
                            .child(crate::i18n::text("template.import_subtitle")),
                    ),
            )
            .child(self.render_close_button(busy, cx))
            .into_any_element()
    }

    fn render_upload_body(&self, busy: bool, cx: &mut Context<Self>) -> AnyElement {
        let root = cx.entity();
        let drag_move_root = root.clone();
        let drag_exit_root = root.clone();
        let drop_root = root.clone();
        let choose_root = root;
        let active_drag_file = cx
            .has_active_drag()
            .then(|| {
                self.template_import_modal
                    .as_ref()
                    .and_then(|modal| modal.drag_hover_filename.clone())
            })
            .flatten();
        let is_drag_active = active_drag_file.is_some();
        let title_text = match &active_drag_file {
            Some(filename) => crate::i18n::tf(
                "template.drop_release_file",
                &[("filename", filename.clone())],
            ),
            None => crate::i18n::text("template.drop_title").to_owned(),
        };
        div()
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .id("template-import-drop-zone")
                    .when(cfg!(test), |el| {
                        el.debug_selector(|| "TEMPLATE_IMPORT_DROP_ZONE".into())
                    })
                    .w_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .px_6()
                    .py(rems(32. / 16.))
                    .rounded(rems(10. / 16.))
                    .bg(if is_drag_active {
                        accent_tint()
                    } else {
                        theme::bg()
                    })
                    .border_1()
                    .border_color(if is_drag_active {
                        theme::accent()
                    } else {
                        theme::border_strong()
                    })
                    .drag_over::<ExternalPaths>(|style, _, _, _| {
                        style
                            .bg(accent_tint())
                            .border_1()
                            .border_color(theme::accent())
                    })
                    .on_drag_move::<ExternalPaths>(
                        move |event: &DragMoveEvent<ExternalPaths>, _, cx| {
                            let hovered = event.bounds.contains(&event.event.position);
                            let filename = hovered.then(|| {
                                event
                                    .drag(cx)
                                    .paths()
                                    .first()
                                    .map(|path| {
                                        path.file_name()
                                            .map(|n| n.to_string_lossy().into_owned())
                                            .unwrap_or_else(|| path.display().to_string())
                                    })
                                    .unwrap_or_default()
                            });
                            let filename = filename.filter(|s| !s.is_empty());
                            drag_move_root.update(cx, |this, cx| {
                                this.set_template_drag_hover(filename, cx);
                            });
                        },
                    )
                    .on_file_drop_exit(move |_, _, cx| {
                        drag_exit_root.update(cx, |this, cx| {
                            this.set_template_drag_hover(None, cx);
                        });
                    })
                    .on_drop(move |paths: &ExternalPaths, window, cx| {
                        drop_root.update(cx, |this, cx| {
                            this.set_template_drag_hover(None, cx);
                            if let Some(path) = paths.paths().first().cloned() {
                                this.load_template_import_file(path, window, cx);
                            }
                        });
                    })
                    .child(
                        div()
                            .size(rems(44. / 16.))
                            .rounded(rems(10. / 16.))
                            .bg(accent_tint())
                            .border_1()
                            .border_color(accent_border())
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                svg()
                                    .path("download.svg")
                                    .size(rems(20. / 16.))
                                    .text_color(theme::accent()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(theme::text_title())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::text())
                            .child(title_text),
                    )
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::muted())
                            .child(crate::i18n::text("template.drop_subtitle")),
                    )
                    .child(if is_drag_active {
                        div()
                            .pt_1()
                            .child(
                                div()
                                    .when(cfg!(test), |el| {
                                        el.debug_selector(|| "TEMPLATE_IMPORT_DROP_ACTIVE".into())
                                    })
                                    .px_3()
                                    .py(rems(5. / 16.))
                                    .rounded(rems(6. / 16.))
                                    .bg(accent_tint())
                                    .border_1()
                                    .border_color(accent_border())
                                    .text_size(theme::text_ui())
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::accent())
                                    .child(crate::i18n::text("template.ready_to_drop")),
                            )
                            .into_any_element()
                    } else {
                        div()
                            .pt_1()
                            .child(
                                Button::new(
                                    "template-import-choose-file",
                                    crate::i18n::text("template.choose_file"),
                                )
                                .debug_selector("TEMPLATE_IMPORT_CHOOSE_FILE")
                                .variant(ButtonVariant::Primary)
                                .loading(busy)
                                .on_press(move |window, cx| {
                                    choose_root.update(cx, |this, cx| {
                                        this.choose_template_import_file(window, cx);
                                    });
                                }),
                            )
                            .into_any_element()
                    }),
            )
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(rems(6. / 16.))
                    .px_3()
                    .py(rems(10. / 16.))
                    .rounded(rems(6. / 16.))
                    .bg(theme::raised())
                    .border_1()
                    .border_color(theme::border())
                    .text_size(theme::text_ui())
                    .child(
                        div()
                            .text_color(theme::muted())
                            .child(crate::i18n::text("template.samples_prefix")),
                    )
                    .child(
                        div()
                            .id("template-import-samples-link")
                            .when(cfg!(test), |el| {
                                el.debug_selector(|| "TEMPLATE_IMPORT_SAMPLES_LINK".into())
                            })
                            .flex()
                            .items_center()
                            .gap(rems(3. / 16.))
                            .cursor(CursorStyle::PointingHand)
                            .text_color(theme::accent())
                            .font_weight(FontWeight::MEDIUM)
                            .on_click(|_, _, cx| cx.open_url(SAMPLE_CREWS_URL))
                            .child("runnersh.dev/crews")
                            .child(
                                svg()
                                    .path("arrow-up-right.svg")
                                    .size(rems(12. / 16.))
                                    .text_color(theme::accent()),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_upload_footer(&self, busy: bool, cx: &mut Context<Self>) -> AnyElement {
        let root = cx.entity();
        div()
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .text_size(theme::text_meta())
                    .text_color(theme::faint())
                    .child(crate::i18n::text("template.step1_footer_note")),
            )
            .child(
                Button::new("template-import-cancel", crate::i18n::text("ui.cancel"))
                    .disabled(busy)
                    .on_press(move |window, cx| {
                        root.update(cx, |this, cx| this.close_template_import_modal(window, cx));
                    }),
            )
            .into_any_element()
    }

    fn render_preview_meta_subtitle(filename: &str, meta: &TemplateMeta) -> String {
        let mut parts = vec![
            filename.to_owned(),
            crate::i18n::text("template.template_v1").to_owned(),
        ];
        if let Some(author) = meta.author.as_deref().filter(|s| !s.trim().is_empty()) {
            parts.push(crate::i18n::tf(
                "template.by_author",
                &[("author", author.to_owned())],
            ));
        }
        parts.join(" · ")
    }

    fn render_crew_preview_header(
        &self,
        state: &PreviewState,
        crew: &CrewTemplate,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let handles: Vec<SharedString> = crew
            .slots
            .iter()
            .map(|slot| SharedString::from(slot.handle.clone()))
            .collect();
        let change_root = cx.entity();
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(crew_picture(&handles, 34.))
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(rems(2. / 16.))
                            .child(
                                div()
                                    .text_size(theme::text_lead())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::text())
                                    .child(crate::i18n::text("template.import_crew_title")),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_meta())
                                    .text_color(theme::muted())
                                    .child(Self::render_preview_meta_subtitle(
                                        &state.filename,
                                        &crew.meta,
                                    )),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new(
                            "template-import-change-file",
                            crate::i18n::text("template.change_file"),
                        )
                        .debug_selector("TEMPLATE_IMPORT_CHANGE_FILE")
                        .size(ButtonSize::Sm)
                        .disabled(busy)
                        .on_press(move |window, cx| {
                            change_root
                                .update(cx, |this, cx| this.return_to_template_upload(window, cx));
                        }),
                    )
                    .child(self.render_close_button(busy, cx)),
            )
            .into_any_element()
    }

    fn render_meta_notes(meta: &TemplateMeta) -> Option<AnyElement> {
        let desc = meta.description.as_deref().filter(|s| !s.trim().is_empty());
        let homepage = meta.homepage.as_deref().filter(|s| !s.trim().is_empty());
        let has_tags = !meta.tags.is_empty();
        if desc.is_none() && homepage.is_none() && !has_tags {
            return None;
        }
        Some(
            div()
                .flex()
                .flex_col()
                .gap(rems(6. / 16.))
                .px_3()
                .py(rems(10. / 16.))
                .rounded(rems(6. / 16.))
                .bg(theme::bg())
                .border_1()
                .border_color(theme::border())
                .children(desc.map(|text| {
                    div()
                        .text_size(theme::text_ui())
                        .text_color(theme::muted())
                        .child(text.to_owned())
                }))
                .when(has_tags, |el| {
                    el.child(
                        div()
                            .when(cfg!(test), |el| {
                                el.debug_selector(|| "TEMPLATE_IMPORT_META_TAGS".into())
                            })
                            .flex()
                            .flex_wrap()
                            .items_center()
                            .gap(rems(6. / 16.))
                            .children(meta.tags.iter().map(|tag| {
                                div()
                                    .px(rems(6. / 16.))
                                    .py(rems(2. / 16.))
                                    .rounded(rems(4. / 16.))
                                    .bg(theme::raised())
                                    .border_1()
                                    .border_color(theme::border())
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_caption())
                                    .text_color(theme::muted())
                                    .child(tag.clone())
                            })),
                    )
                })
                .children(homepage.map(|url| {
                    div()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_meta())
                        .text_color(theme::faint())
                        .child(format!(
                            "{}: {url}",
                            crate::i18n::text("template.homepage_label")
                        ))
                }))
                .into_any_element(),
        )
    }

    fn render_crew_preview_body(
        &self,
        state: &PreviewState,
        crew: &CrewTemplate,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let root = cx.entity();
        let roles_by_handle: HashMap<&str, &RoleTemplate> = crew
            .roles
            .iter()
            .map(|role| (role.handle.as_str(), role))
            .collect();
        let planned_by_template: HashMap<&str, &str> = state
            .plan
            .planned_roles
            .iter()
            .map(|r| (r.template_handle.as_str(), r.planned_handle.as_str()))
            .collect();
        let bulk_missing = bulk_missing_runtimes(
            crew,
            &state.runtime_overrides,
            &state.plan.installed_runtimes,
        );
        let no_installed_runtimes = state.plan.installed_runtimes.is_empty();
        let any_uninstalled = crew.slots.iter().any(|slot| {
            let Some(role) = roles_by_handle.get(slot.role.as_str()) else {
                return false;
            };
            let rt = state
                .runtime_overrides
                .get(&slot.handle)
                .copied()
                .unwrap_or_else(|| slot.runtime.unwrap_or(role.runtime));
            !state.plan.installed_runtimes.contains(&rt)
        });

        let name_field = state.crew_name.clone();
        let slot_count_label = crew_summary(crew.slots.len());

        let mut slot_rows = Vec::with_capacity(crew.slots.len());
        for (index, slot) in crew.slots.iter().enumerate() {
            let Some(role) = roles_by_handle.get(slot.role.as_str()) else {
                continue;
            };
            let summary = effective_slot_summary(
                slot,
                role,
                state.runtime_overrides.get(&slot.handle).copied(),
                &state.plan.installed_runtimes,
            );
            let planned_handle = planned_by_template
                .get(role.handle.as_str())
                .copied()
                .unwrap_or(role.handle.as_str());
            let (prompt_bytes, _) = prompt_stats(role.system_prompt.as_deref());
            let icon = ChatIcon::for_runtime(summary.effective_runtime.key());
            let open_root = root.clone();
            let select = state.slot_selects.get(&slot.handle).cloned();
            let row_selector = format!("TEMPLATE_IMPORT_SLOT:{index}");
            let rt_selector = format!("TEMPLATE_IMPORT_SLOT_RUNTIME:{index}");

            let subtitle = if summary.switched {
                crate::i18n::tf(
                    "template.switched_from",
                    &[
                        ("from", runtime_display_name(summary.template_runtime.key())),
                        ("to", runtime_display_name(summary.effective_runtime.key())),
                    ],
                )
            } else {
                runtime_meta_line(
                    summary.effective_runtime,
                    summary.model.as_deref(),
                    summary.effort.as_deref(),
                    summary.codex_speed,
                )
            };

            let new_role_badge = {
                let badge_selector = format!("TEMPLATE_IMPORT_NEW_ROLE_BADGE:{index}");
                div()
                    .when(cfg!(test), |el| {
                        el.debug_selector(move || badge_selector.clone())
                    })
                    .px(rems(6. / 16.))
                    .py(rems(2. / 16.))
                    .rounded(rems(4. / 16.))
                    .bg(accent_tint())
                    .border_1()
                    .border_color(accent_border())
                    .font_family(theme::UI_MONOSPACE_FONT)
                    .text_size(theme::text_caption())
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::accent())
                    .child(crate::i18n::tf(
                        "template.new_role_badge",
                        &[("handle", planned_handle.to_owned())],
                    ))
            };
            let missing_badge = (!summary.effective_installed).then(|| {
                div()
                    .px(rems(6. / 16.))
                    .py(rems(2. / 16.))
                    .rounded(rems(4. / 16.))
                    .bg(warn_tint())
                    .border_1()
                    .border_color(warn_border())
                    .text_size(theme::text_caption())
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme::warning())
                    .child(crate::i18n::tf(
                        "template.runtime_not_installed_badge",
                        &[(
                            "runtime",
                            runtime_display_name(summary.effective_runtime.key()),
                        )],
                    ))
            });

            let needs_picker_row = !summary.template_installed && select.is_some();
            let row = div()
                .flex()
                .flex_col()
                .when(index > 0, |el| {
                    el.border_t_1().border_color(theme::border())
                })
                .child(
                    div()
                        .id(SharedString::from(format!("template-slot-row-{index}")))
                        .when(cfg!(test), |el| {
                            el.debug_selector(move || row_selector.clone())
                        })
                        .w_full()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .px_3()
                        .py(rems(10. / 16.))
                        .cursor(CursorStyle::PointingHand)
                        .hover(|s| s.bg(theme::raised()))
                        .on_click(move |_, window, cx| {
                            open_root.update(cx, |this, cx| {
                                this.open_template_reader(index, window, cx);
                            });
                        })
                        .child(
                            div()
                                .min_w(px(0.))
                                .flex_1()
                                .flex()
                                .items_center()
                                .gap(rems(10. / 16.))
                                .child(RoleAvatar::new(slot.handle.clone(), 28.))
                                .child(
                                    div()
                                        .min_w(px(0.))
                                        .flex_1()
                                        .flex()
                                        .flex_col()
                                        .gap(rems(2. / 16.))
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap_2()
                                                .child(
                                                    div()
                                                        .font_family(theme::UI_MONOSPACE_FONT)
                                                        .text_size(theme::text_body())
                                                        .font_weight(FontWeight::MEDIUM)
                                                        .text_color(theme::text())
                                                        .child(format!("@{}", slot.handle)),
                                                )
                                                .child(
                                                    div()
                                                        .truncate()
                                                        .text_size(theme::text_ui())
                                                        .text_color(theme::muted())
                                                        .child(role.display_name.clone()),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .flex()
                                                .items_center()
                                                .gap(rems(6. / 16.))
                                                .child(icon.render(
                                                    rems(12. / 16.),
                                                    icon.color(theme::muted(), true),
                                                    true,
                                                ))
                                                .when(
                                                    summary.has_slot_override && !summary.switched,
                                                    |el| {
                                                        el.child(
                                                            div()
                                                                .size(rems(6. / 16.))
                                                                .rounded_full()
                                                                .bg(theme::warning()),
                                                        )
                                                    },
                                                )
                                                .child(
                                                    div()
                                                        .min_w(px(0.))
                                                        .flex_1()
                                                        .truncate()
                                                        .text_size(theme::text_meta())
                                                        .text_color(if summary.switched {
                                                            theme::accent()
                                                        } else {
                                                            theme::muted()
                                                        })
                                                        .child(subtitle),
                                                ),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .flex_none()
                                .flex()
                                .items_center()
                                .gap_2()
                                .children(missing_badge)
                                .child(new_role_badge)
                                .child(
                                    div()
                                        .font_family(theme::UI_MONOSPACE_FONT)
                                        .text_size(theme::text_meta())
                                        .text_color(theme::faint())
                                        .child(format_prompt_size(prompt_bytes)),
                                )
                                .child(
                                    svg()
                                        .path("chevron-right.svg")
                                        .size(rems(14. / 16.))
                                        .text_color(theme::faint()),
                                ),
                        ),
                )
                .when(needs_picker_row, |el| {
                    let select = select.unwrap();
                    el.child(
                        div()
                            .when(cfg!(test), |el| {
                                el.debug_selector(move || rt_selector.clone())
                            })
                            .w_full()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .px_3()
                            .py(rems(8. / 16.))
                            .bg(if summary.effective_installed {
                                theme::with_alpha(theme::accent(), 0.05)
                            } else {
                                warn_tint()
                            })
                            .border_t_1()
                            .border_color(if summary.effective_installed {
                                accent_border()
                            } else {
                                warn_border()
                            })
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .flex_1()
                                    .flex()
                                    .items_center()
                                    .gap(rems(6. / 16.))
                                    .text_size(theme::text_meta())
                                    .text_color(if summary.effective_installed {
                                        theme::muted()
                                    } else {
                                        theme::warning()
                                    })
                                    .child(
                                        svg()
                                            .flex_none()
                                            .path(if summary.effective_installed {
                                                "check.svg"
                                            } else {
                                                "triangle-alert.svg"
                                            })
                                            .size(rems(12. / 16.))
                                            .text_color(if summary.effective_installed {
                                                theme::accent()
                                            } else {
                                                theme::warning()
                                            }),
                                    )
                                    .child(
                                        div()
                                            .min_w(px(0.))
                                            .flex_1()
                                            .when(cfg!(test) && summary.switched, |el| {
                                                let reset_selector =
                                                    format!("TEMPLATE_IMPORT_SLOT_RESET:{index}");
                                                el.debug_selector(move || reset_selector.clone())
                                            })
                                            .child(crate::i18n::text(if summary.switched {
                                                "template.switched_reset_note"
                                            } else {
                                                "template.switch_runtime_prompt"
                                            })),
                                    ),
                            )
                            .child(div().flex_none().child(select)),
                    )
                });
            slot_rows.push(row);
        }

        let conventions_row = crew
            .crew
            .conventions
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|text| {
                let (bytes, _) = prompt_stats(Some(text));
                let conv_index = crew.slots.len();
                let open_root = root.clone();
                div()
                    .id("template-import-conventions-row")
                    .when(cfg!(test), |el| {
                        el.debug_selector(|| "TEMPLATE_IMPORT_CONVENTIONS".into())
                    })
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .px_3()
                    .py(rems(10. / 16.))
                    .rounded(rems(8. / 16.))
                    .bg(theme::bg())
                    .border_1()
                    .border_color(theme::border())
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| s.bg(theme::raised()))
                    .on_click(move |_, window, cx| {
                        open_root.update(cx, |this, cx| {
                            this.open_template_reader(conv_index, window, cx);
                        });
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(10. / 16.))
                            .child(
                                svg()
                                    .path("file-text.svg")
                                    .size(rems(15. / 16.))
                                    .text_color(theme::muted()),
                            )
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(rems(2. / 16.))
                                    .child(
                                        div()
                                            .text_size(theme::text_body())
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme::text())
                                            .child(crate::i18n::text("template.team_conventions")),
                                    )
                                    .child(
                                        div()
                                            .text_size(theme::text_meta())
                                            .text_color(theme::muted())
                                            .child(crate::i18n::text(
                                                "template.spliced_into_every_slot",
                                            )),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_meta())
                                    .text_color(theme::faint())
                                    .child(format_prompt_size(bytes)),
                            )
                            .child(
                                svg()
                                    .path("chevron-right.svg")
                                    .size(rems(14. / 16.))
                                    .text_color(theme::faint()),
                            ),
                    )
            });

        div()
            .flex()
            .flex_col()
            .gap_4()
            .children(state.error.as_ref().map(|err| {
                runner_app::ui::notice_banner(err.clone(), runner_app::ui::Tone::Danger)
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(6. / 16.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(theme::text_ui())
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::text())
                                    .child(crate::i18n::text("template.crew_name_label")),
                            )
                            .child(
                                div()
                                    .text_size(theme::text_meta())
                                    .text_color(theme::muted())
                                    .child(slot_count_label),
                            ),
                    )
                    .children(name_field.map(|field| {
                        div()
                            .when(cfg!(test), |el| {
                                el.debug_selector(|| "TEMPLATE_IMPORT_CREW_NAME".into())
                            })
                            .child(field)
                    }))
                    .child(
                        div()
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .child(crate::i18n::text("template.review_before_import_hint")),
                    ),
            )
            .children(Self::render_meta_notes(&crew.meta))
            .children(
                (no_installed_runtimes && any_uninstalled)
                    .then(Self::render_no_installed_runtimes_banner),
            )
            .children(bulk_missing.into_iter().filter_map(|bulk| {
                let select = state.bulk_selects.get(bulk.runtime.key())?.clone();
                let runtime_name = runtime_display_name(bulk.runtime.key());
                let count = bulk.slot_indices.len();
                Some(
                    div()
                        .when(cfg!(test), |el| {
                            el.debug_selector(|| "TEMPLATE_IMPORT_BULK_BANNER".into())
                        })
                        .w_full()
                        .flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .px_3()
                        .py(rems(10. / 16.))
                        .rounded(rems(8. / 16.))
                        .bg(warn_tint())
                        .border_1()
                        .border_color(warn_border())
                        .child(
                            div()
                                .min_w(px(0.))
                                .flex_1()
                                .flex()
                                .items_start()
                                .gap(rems(8. / 16.))
                                .child(
                                    svg()
                                        .path("triangle-alert.svg")
                                        .size(rems(15. / 16.))
                                        .mt(rems(2. / 16.))
                                        .flex_none()
                                        .text_color(theme::warning()),
                                )
                                .child(
                                    div()
                                        .min_w(px(0.))
                                        .flex()
                                        .flex_col()
                                        .gap(rems(2. / 16.))
                                        .child(
                                            div()
                                                .text_size(theme::text_ui())
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(theme::text())
                                                .child(crate::i18n::tf(
                                                    "template.runtime_not_installed_title",
                                                    &[("runtime", runtime_name)],
                                                )),
                                        )
                                        .child(
                                            div()
                                                .text_size(theme::text_meta())
                                                .text_color(theme::muted())
                                                .child(crate::i18n::tf(
                                                    "template.bulk_switch_desc",
                                                    &[("count", count.to_string())],
                                                )),
                                        ),
                                ),
                        )
                        .child(div().flex_none().child(select)),
                )
            }))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(8. / 16.))
                    .child(
                        div()
                            .text_size(theme::text_caption())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::faint())
                            .child(crate::i18n::tf(
                                "template.slots_and_roles_heading",
                                &[("count", crew.slots.len().to_string())],
                            )),
                    )
                    .when(!slot_rows.is_empty(), |el| {
                        el.child(
                            div()
                                .w_full()
                                .flex()
                                .flex_col()
                                .rounded(rems(8. / 16.))
                                .bg(theme::bg())
                                .border_1()
                                .border_color(theme::border())
                                .overflow_hidden()
                                .children(slot_rows),
                        )
                    }),
            )
            .children(conventions_row)
            .child(Self::render_broad_access_note())
            .into_any_element()
    }

    fn render_crew_preview_footer(
        &self,
        state: &PreviewState,
        crew: &CrewTemplate,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cancel_root = cx.entity();
        let confirm_root = cx.entity();
        let name_empty = state
            .crew_name
            .as_ref()
            .is_some_and(|field| field.read(cx).text().trim().is_empty());
        let role_count = crew.roles.len();
        div()
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap(rems(2. / 16.))
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::text())
                            .child(crate::i18n::tn("template.crew_footer_creates", role_count)),
                    )
                    .child(
                        div()
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .child(crate::i18n::text("template.crew_footer_excluded")),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("template-import-cancel", crate::i18n::text("ui.cancel"))
                            .disabled(busy)
                            .on_press(move |window, cx| {
                                cancel_root.update(cx, |this, cx| {
                                    this.close_template_import_modal(window, cx);
                                });
                            }),
                    )
                    .child(
                        Button::new(
                            "template-import-confirm",
                            if busy {
                                crate::i18n::text("template.importing")
                            } else {
                                crate::i18n::text("template.import_crew_title")
                            },
                        )
                        .debug_selector("TEMPLATE_IMPORT_CONFIRM")
                        .variant(ButtonVariant::Primary)
                        .loading(busy)
                        .disabled(name_empty)
                        .on_press(move |window, cx| {
                            confirm_root
                                .update(cx, |this, cx| this.submit_template_import(window, cx));
                        }),
                    ),
            )
            .into_any_element()
    }

    fn render_role_preview_header(
        &self,
        state: &PreviewState,
        standalone: &StandaloneRoleTemplate,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let change_root = cx.entity();
        let current_handle = state
            .role_handle
            .as_ref()
            .map(|f| f.read(cx).text().to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| standalone.role.handle.clone());
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(RoleAvatar::new(current_handle, 34.))
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(rems(2. / 16.))
                            .child(
                                div()
                                    .text_size(theme::text_lead())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::text())
                                    .child(crate::i18n::text("template.import_role_title")),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_meta())
                                    .text_color(theme::muted())
                                    .child(Self::render_preview_meta_subtitle(
                                        &state.filename,
                                        &standalone.meta,
                                    )),
                            ),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new(
                            "template-import-change-file",
                            crate::i18n::text("template.change_file"),
                        )
                        .debug_selector("TEMPLATE_IMPORT_CHANGE_FILE")
                        .size(ButtonSize::Sm)
                        .disabled(busy)
                        .on_press(move |window, cx| {
                            change_root
                                .update(cx, |this, cx| this.return_to_template_upload(window, cx));
                        }),
                    )
                    .child(self.render_close_button(busy, cx)),
            )
            .into_any_element()
    }

    fn render_role_preview_body(
        &self,
        state: &PreviewState,
        standalone: &StandaloneRoleTemplate,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let role = &standalone.role;
        let open_root = cx.entity();
        let template_rt = role.runtime;
        let effective_rt = state
            .runtime_overrides
            .get(&role.handle)
            .copied()
            .unwrap_or(template_rt);
        let template_installed = state.plan.installed_runtimes.contains(&template_rt);
        let effective_installed = state.plan.installed_runtimes.contains(&effective_rt);
        let switched = effective_rt != template_rt;
        let icon = ChatIcon::for_runtime(effective_rt.key());
        let planned_suffixed = state
            .plan
            .planned_roles
            .first()
            .is_some_and(|planned| planned.auto_suffixed);
        let raw_handle = state
            .role_handle
            .as_ref()
            .map(|f| f.read(cx).text().trim().to_owned())
            .unwrap_or_default();
        let current_handle = raw_handle.strip_prefix('@').unwrap_or(&raw_handle);
        let handle_error = if current_handle.is_empty() {
            Some(crate::i18n::text("template.handle_required"))
        } else {
            validate_role_handle(current_handle).map(crate::i18n::text)
        };
        let prompt_text = role.system_prompt.as_deref().unwrap_or("");
        let (prompt_bytes, prompt_lines) = prompt_stats(role.system_prompt.as_deref());

        let model_label = if switched {
            crate::i18n::text("template.default_reset_on_switch").to_owned()
        } else {
            role.model
                .clone()
                .unwrap_or_else(|| crate::i18n::text("template.runtime_default").to_owned())
        };
        let effort_label = if switched {
            crate::i18n::text("template.default_reset_on_switch").to_owned()
        } else {
            role.effort
                .clone()
                .unwrap_or_else(|| crate::i18n::text("template.runtime_default").to_owned())
        };

        div()
            .flex()
            .flex_col()
            .gap_4()
            .children(state.error.as_ref().map(|err| {
                runner_app::ui::notice_banner(err.clone(), runner_app::ui::Tone::Danger)
            }))
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .gap(rems(6. / 16.))
                            .child(
                                div()
                                    .text_size(theme::text_ui())
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::text())
                                    .child(crate::i18n::text("template.display_name_label")),
                            )
                            .children(state.role_display_name.clone().map(|field| {
                                div()
                                    .when(cfg!(test), |el| {
                                        el.debug_selector(|| "TEMPLATE_IMPORT_ROLE_NAME".into())
                                    })
                                    .child(field)
                            })),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .flex_col()
                            .gap(rems(6. / 16.))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .justify_between()
                                    .child(
                                        div()
                                            .text_size(theme::text_ui())
                                            .font_weight(FontWeight::MEDIUM)
                                            .text_color(theme::text())
                                            .child(crate::i18n::text("template.handle_label")),
                                    )
                                    .when(planned_suffixed, |el| {
                                        el.child(
                                            div()
                                                .px(rems(6. / 16.))
                                                .py(rems(1. / 16.))
                                                .rounded(rems(4. / 16.))
                                                .bg(accent_tint())
                                                .text_size(theme::text_caption())
                                                .text_color(theme::accent())
                                                .child(crate::i18n::text("template.auto_suffixed")),
                                        )
                                    }),
                            )
                            .children(state.role_handle.clone().map(|field| {
                                div()
                                    .when(cfg!(test), |el| {
                                        el.debug_selector(|| "TEMPLATE_IMPORT_ROLE_HANDLE".into())
                                    })
                                    .child(field)
                            }))
                            .children(handle_error.map(|err| {
                                div()
                                    .text_size(theme::text_meta())
                                    .text_color(theme::danger())
                                    .child(err)
                            })),
                    ),
            )
            .children(Self::render_meta_notes(&standalone.meta))
            .children(
                (state.plan.installed_runtimes.is_empty() && !effective_installed)
                    .then(Self::render_no_installed_runtimes_banner),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(10. / 16.))
                    .p_3()
                    .rounded(rems(8. / 16.))
                    .bg(theme::bg())
                    .border_1()
                    .border_color(if !effective_installed {
                        warn_border()
                    } else {
                        theme::border()
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(rems(4. / 16.))
                                    .child(
                                        div()
                                            .text_size(theme::text_caption())
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme::faint())
                                            .child(crate::i18n::text("template.runtime_heading")),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap(rems(6. / 16.))
                                            .child(icon.render(
                                                rems(14. / 16.),
                                                icon.color(theme::muted(), true),
                                                true,
                                            ))
                                            .child(
                                                div()
                                                    .text_size(theme::text_body())
                                                    .font_weight(FontWeight::MEDIUM)
                                                    .text_color(theme::text())
                                                    .child(runtime_display_name(
                                                        effective_rt.key(),
                                                    )),
                                            ),
                                    ),
                            )
                            .child(if effective_installed {
                                div()
                                    .px(rems(6. / 16.))
                                    .py(rems(2. / 16.))
                                    .rounded(rems(4. / 16.))
                                    .bg(accent_tint())
                                    .text_size(theme::text_caption())
                                    .text_color(theme::accent())
                                    .child(crate::i18n::text("template.runtime_installed_badge"))
                            } else {
                                div()
                                    .px(rems(6. / 16.))
                                    .py(rems(2. / 16.))
                                    .rounded(rems(4. / 16.))
                                    .bg(warn_tint())
                                    .border_1()
                                    .border_color(warn_border())
                                    .text_size(theme::text_caption())
                                    .text_color(theme::warning())
                                    .child(crate::i18n::tf(
                                        "template.runtime_not_installed_badge",
                                        &[("runtime", runtime_display_name(effective_rt.key()))],
                                    ))
                            }),
                    )
                    .children(
                        (!template_installed)
                            .then(|| {
                                state.role_select.clone().map(|select| {
                                    div()
                                        .when(cfg!(test), |el| {
                                            el.debug_selector(|| {
                                                "TEMPLATE_IMPORT_ROLE_RUNTIME".into()
                                            })
                                        })
                                        .flex()
                                        .items_center()
                                        .justify_between()
                                        .gap_2()
                                        .pt_2()
                                        .border_t_1()
                                        .border_color(theme::border())
                                        .child(
                                            div()
                                                .text_size(theme::text_meta())
                                                .text_color(theme::muted())
                                                .child(crate::i18n::text(
                                                    "template.switch_runtime_prompt",
                                                )),
                                        )
                                        .child(select)
                                })
                            })
                            .flatten(),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_4()
                            .pt_2()
                            .border_t_1()
                            .border_color(theme::border())
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .flex()
                                    .flex_col()
                                    .gap(rems(2. / 16.))
                                    .child(
                                        div()
                                            .text_size(theme::text_caption())
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme::faint())
                                            .child(crate::i18n::text("template.model_heading")),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .font_family(theme::UI_MONOSPACE_FONT)
                                            .text_size(theme::text_ui())
                                            .text_color(if switched || role.model.is_none() {
                                                theme::muted()
                                            } else {
                                                theme::text()
                                            })
                                            .child(model_label),
                                    ),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.))
                                    .flex()
                                    .flex_col()
                                    .gap(rems(2. / 16.))
                                    .child(
                                        div()
                                            .text_size(theme::text_caption())
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme::faint())
                                            .child(crate::i18n::text("template.effort_heading")),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .font_family(theme::UI_MONOSPACE_FONT)
                                            .text_size(theme::text_ui())
                                            .text_color(if switched || role.effort.is_none() {
                                                theme::muted()
                                            } else {
                                                theme::text()
                                            })
                                            .child(effort_label),
                                    ),
                            ),
                    ),
            )
            .child(
                div()
                    .id("template-import-role-prompt-card")
                    .when(cfg!(test), |el| {
                        el.debug_selector(|| "TEMPLATE_IMPORT_ROLE_PROMPT".into())
                    })
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p_3()
                    .rounded(rems(8. / 16.))
                    .bg(theme::bg())
                    .border_1()
                    .border_color(theme::border())
                    .cursor(CursorStyle::PointingHand)
                    .hover(|s| s.border_color(theme::border_strong()))
                    .on_click(move |_, window, cx| {
                        open_root.update(cx, |this, cx| this.open_template_reader(0, window, cx));
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(theme::text_caption())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::faint())
                                    .child(crate::i18n::tnf(
                                        "template.system_prompt_heading",
                                        prompt_lines,
                                        &[("size", format_prompt_size(prompt_bytes))],
                                    )),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(rems(2. / 16.))
                                    .text_size(theme::text_ui())
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(theme::accent())
                                    .child(crate::i18n::text("template.read_full_prompt"))
                                    .child(
                                        svg()
                                            .path("chevron-right.svg")
                                            .size(rems(12. / 16.))
                                            .text_color(theme::accent()),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_ui())
                            .text_color(if prompt_text.is_empty() {
                                theme::faint()
                            } else {
                                theme::muted()
                            })
                            .child(if prompt_text.is_empty() {
                                crate::i18n::text("template.empty_prompt").to_owned()
                            } else {
                                prompt_excerpt(prompt_text, 4)
                            }),
                    ),
            )
            .child(Self::render_broad_access_note())
            .into_any_element()
    }

    fn render_role_preview_footer(
        &self,
        state: &PreviewState,
        busy: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cancel_root = cx.entity();
        let confirm_root = cx.entity();
        let name_empty = state
            .role_display_name
            .as_ref()
            .is_some_and(|field| field.read(cx).text().trim().is_empty());
        let raw_handle = state
            .role_handle
            .as_ref()
            .map(|field| field.read(cx).text().trim().to_owned())
            .unwrap_or_default();
        let handle = raw_handle.strip_prefix('@').unwrap_or(&raw_handle);
        let handle_invalid = handle.is_empty() || validate_role_handle(handle).is_some();
        div()
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .text_size(theme::text_meta())
                    .text_color(theme::faint())
                    .child(crate::i18n::text("template.role_footer_creates")),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Button::new("template-import-cancel", crate::i18n::text("ui.cancel"))
                            .disabled(busy)
                            .on_press(move |window, cx| {
                                cancel_root.update(cx, |this, cx| {
                                    this.close_template_import_modal(window, cx);
                                });
                            }),
                    )
                    .child(
                        Button::new(
                            "template-import-confirm",
                            if busy {
                                crate::i18n::text("template.importing")
                            } else {
                                crate::i18n::text("template.import_role_title")
                            },
                        )
                        .debug_selector("TEMPLATE_IMPORT_CONFIRM")
                        .variant(ButtonVariant::Primary)
                        .loading(busy)
                        .disabled(name_empty || handle_invalid)
                        .on_press(move |window, cx| {
                            confirm_root
                                .update(cx, |this, cx| this.submit_template_import(window, cx));
                        }),
                    ),
            )
            .into_any_element()
    }

    fn render_no_installed_runtimes_banner() -> AnyElement {
        div()
            .when(cfg!(test), |el| {
                el.debug_selector(|| "TEMPLATE_IMPORT_NO_RUNTIMES_BANNER".into())
            })
            .w_full()
            .flex()
            .items_start()
            .gap(rems(8. / 16.))
            .px_3()
            .py(rems(10. / 16.))
            .rounded(rems(8. / 16.))
            .bg(warn_tint())
            .border_1()
            .border_color(warn_border())
            .child(
                svg()
                    .path("triangle-alert.svg")
                    .size(rems(15. / 16.))
                    .mt(rems(1. / 16.))
                    .flex_none()
                    .text_color(theme::warning()),
            )
            .child(
                div()
                    .text_size(theme::text_ui())
                    .text_color(theme::text())
                    .child(crate::i18n::text("template.no_installed_runtimes")),
            )
            .into_any_element()
    }

    fn render_broad_access_note() -> AnyElement {
        div()
            .w_full()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py(rems(8. / 16.))
            .rounded(rems(6. / 16.))
            .bg(theme::raised())
            .border_1()
            .border_color(theme::border())
            .child(
                svg()
                    .path("shield.svg")
                    .size(rems(14. / 16.))
                    .flex_none()
                    .text_color(theme::muted()),
            )
            .child(
                div()
                    .text_size(theme::text_meta())
                    .text_color(theme::muted())
                    .child(crate::i18n::text("template.broad_access_note")),
            )
            .into_any_element()
    }

    fn render_reader_header(&self, item: &ReaderItem, cx: &mut Context<Self>) -> AnyElement {
        let back_root = cx.entity();
        let (leading, title, subtitle) = match &item.kind {
            ReaderItemKind::Role {
                handle,
                display_name,
                runtime,
                model,
                effort,
                codex_speed,
            } => (
                RoleAvatar::new(handle.clone(), 30.).into_any_element(),
                format!("@{handle}"),
                format!(
                    "{display_name} · {}",
                    runtime_meta_line(*runtime, model.as_deref(), effort.as_deref(), *codex_speed)
                ),
            ),
            ReaderItemKind::Conventions => (
                div()
                    .size(rems(30. / 16.))
                    .rounded(rems(6. / 16.))
                    .bg(theme::raised())
                    .border_1()
                    .border_color(theme::border())
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        svg()
                            .path("file-text.svg")
                            .size(rems(15. / 16.))
                            .text_color(theme::muted()),
                    )
                    .into_any_element(),
                crate::i18n::text("template.team_conventions").to_owned(),
                crate::i18n::text("template.spliced_into_every_slot").to_owned(),
            ),
        };
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .min_w(px(0.))
                    .flex_1()
                    .flex()
                    .items_center()
                    .gap(rems(10. / 16.))
                    .child(
                        div()
                            .when(cfg!(test), |el| {
                                el.debug_selector(|| "TEMPLATE_IMPORT_READER_BACK".into())
                            })
                            .child(
                                IconButton::new("template-reader-back", "arrow-left.svg")
                                    .size(IconButtonSize::Sm)
                                    .tooltip(crate::i18n::text("template.back_to_import"))
                                    .on_press(move |window, cx| {
                                        back_root.update(cx, |this, cx| {
                                            this.close_template_reader(window, cx);
                                        });
                                    }),
                            ),
                    )
                    .child(leading)
                    .child(
                        div()
                            .min_w(px(0.))
                            .flex_1()
                            .flex()
                            .flex_col()
                            .gap(rems(2. / 16.))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(theme::text_lead())
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(theme::text())
                                    .child(title),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .font_family(theme::UI_MONOSPACE_FONT)
                                    .text_size(theme::text_meta())
                                    .text_color(theme::muted())
                                    .child(subtitle),
                            ),
                    ),
            )
            .child(self.render_close_button(false, cx))
            .into_any_element()
    }

    fn render_reader_body(
        &self,
        item: &ReaderItem,
        mode: ReaderMode,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let modal = self.template_import_modal.as_ref().expect("import modal");
        let scroll = modal.reader_scroll.clone();
        let scrollbar = modal.reader_scrollbar.clone();
        let source_root = cx.entity();
        let rendered_root = cx.entity();
        let meta = match item.kind {
            ReaderItemKind::Role { .. } => crate::i18n::tnf(
                "template.system_prompt_meta",
                item.lines,
                &[("size", format_prompt_size(item.bytes))],
            ),
            ReaderItemKind::Conventions => crate::i18n::tnf(
                "template.conventions_meta",
                item.lines,
                &[("size", format_prompt_size(item.bytes))],
            ),
        };

        type ReaderTabHandler = Rc<dyn Fn(&mut Window, &mut gpui::App)>;
        let seg_button = |id: &'static str,
                          selector: &'static str,
                          label: &'static str,
                          active: bool,
                          on_press: ReaderTabHandler| {
            div()
                .id(id)
                .when(cfg!(test), |el| el.debug_selector(move || selector.into()))
                .px(rems(10. / 16.))
                .py(rems(4. / 16.))
                .rounded(rems(5. / 16.))
                .border_1()
                .border_color(if active {
                    theme::seg_selected_border()
                } else {
                    gpui::transparent_black()
                })
                .bg(if active {
                    theme::seg_selected()
                } else {
                    gpui::transparent_black()
                })
                .text_size(theme::text_ui())
                .font_weight(if active {
                    FontWeight::MEDIUM
                } else {
                    FontWeight::NORMAL
                })
                .text_color(if active {
                    theme::text()
                } else {
                    theme::muted()
                })
                .cursor(CursorStyle::PointingHand)
                .on_click(move |_, window, cx| on_press(window, cx))
                .child(label)
        };

        let content: AnyElement = if item.text.is_empty() {
            div()
                .text_size(theme::text_ui())
                .text_color(theme::faint())
                .child(crate::i18n::text("template.empty_prompt"))
                .into_any_element()
        } else {
            match mode {
                ReaderMode::Source => {
                    let lines: Vec<&str> = item.text.lines().collect();
                    let gutter_digits = lines.len().max(1).to_string().len().max(2);
                    let gutter_width = rems((gutter_digits as f32 * 8. + 16.) / 16.);
                    div()
                        .when(cfg!(test), |el| {
                            el.debug_selector(|| "TEMPLATE_IMPORT_READER_SOURCE_BODY".into())
                        })
                        .w_full()
                        .flex()
                        .flex_col()
                        .gap(rems(2. / 16.))
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(theme::text_ui())
                        .line_height(rems(19. / 16.))
                        .children(lines.into_iter().enumerate().map(|(i, line)| {
                            let segments = source_line_segments(line);
                            div()
                                .w_full()
                                .flex()
                                .items_start()
                                .gap_3()
                                .child(
                                    div()
                                        .w(gutter_width)
                                        .flex_none()
                                        .text_right()
                                        .text_color(theme::faint())
                                        .child(format!("{}", i + 1)),
                                )
                                .child(match segments.as_slice() {
                                    [] => {
                                        div().min_w(px(0.)).flex_1().child(" ").into_any_element()
                                    }
                                    [SourceSegment::Text(text)] => div()
                                        .min_w(px(0.))
                                        .flex_1()
                                        .text_color(theme::text())
                                        .child(text.clone())
                                        .into_any_element(),
                                    _ => div()
                                        .min_w(px(0.))
                                        .flex_1()
                                        .flex()
                                        .flex_wrap()
                                        .items_center()
                                        .gap(rems(2. / 16.))
                                        .children(segments.into_iter().map(|segment| {
                                            match segment {
                                                SourceSegment::Text(text) => div()
                                                    .max_w_full()
                                                    .text_color(theme::text())
                                                    .child(text)
                                                    .into_any_element(),
                                                SourceSegment::Marker(marker) => div()
                                                    .when(cfg!(test), |el| {
                                                        el.debug_selector(|| {
                                                            "TEMPLATE_READER_HIDDEN_CHAR".into()
                                                        })
                                                    })
                                                    .px(rems(4. / 16.))
                                                    .rounded(rems(3. / 16.))
                                                    .bg(warn_tint())
                                                    .border_1()
                                                    .border_color(warn_border())
                                                    .text_size(theme::text_caption())
                                                    .font_weight(FontWeight::SEMIBOLD)
                                                    .text_color(theme::warning())
                                                    .child(marker)
                                                    .into_any_element(),
                                            }
                                        }))
                                        .into_any_element(),
                                })
                        }))
                        .into_any_element()
                }
                ReaderMode::Rendered => div()
                    .when(cfg!(test), |el| {
                        el.debug_selector(|| "TEMPLATE_IMPORT_READER_RENDERED_BODY".into())
                    })
                    .w_full()
                    .child(render_compact(
                        &format!("template-reader-{}", item.label),
                        &item.text,
                        cx.entity_id(),
                        cx,
                    ))
                    .into_any_element(),
            }
        };

        div()
            .flex_1()
            .min_h(px(0.))
            .flex()
            .flex_col()
            .gap_3()
            .px(rems(20. / 16.))
            .py(rems(14. / 16.))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(2. / 16.))
                            .p(rems(2. / 16.))
                            .rounded(rems(6. / 16.))
                            .bg(theme::bg())
                            .border_1()
                            .border_color(theme::border())
                            .child(seg_button(
                                "template-reader-source",
                                "TEMPLATE_IMPORT_READER_SOURCE",
                                crate::i18n::text("template.source_tab"),
                                mode == ReaderMode::Source,
                                Rc::new(move |_, cx| {
                                    source_root.update(cx, |this, cx| {
                                        this.set_template_reader_mode(ReaderMode::Source, cx);
                                    });
                                }),
                            ))
                            .child(seg_button(
                                "template-reader-rendered",
                                "TEMPLATE_IMPORT_READER_RENDERED",
                                crate::i18n::text("template.rendered_tab"),
                                mode == ReaderMode::Rendered,
                                Rc::new(move |_, cx| {
                                    rendered_root.update(cx, |this, cx| {
                                        this.set_template_reader_mode(ReaderMode::Rendered, cx);
                                    });
                                }),
                            )),
                    )
                    .child(
                        div()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_meta())
                            .text_color(theme::faint())
                            .child(meta),
                    ),
            )
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(rems(400. / 16.))
                    .max_h(rems(440. / 16.))
                    .min_h(rems(180. / 16.))
                    .flex_1()
                    .rounded(rems(8. / 16.))
                    .bg(theme::bg())
                    .border_1()
                    .border_color(theme::border())
                    .overflow_hidden()
                    .child(
                        div()
                            .id("template-reader-scroll")
                            .size_full()
                            .overflow_y_scroll()
                            .scrollbar_width(px(0.))
                            .track_scroll(&scroll)
                            .p(rems(14. / 16.))
                            .child(content),
                    )
                    .child(scrollbar),
            )
            .into_any_element()
    }

    fn render_reader_footer(
        &self,
        items: &[ReaderItem],
        index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let prev_root = cx.entity();
        let next_root = cx.entity();
        let done_root = cx.entity();
        let has_prev = index > 0;
        let has_next = index + 1 < items.len();
        let next_hint = items.get(index + 1).map(|next| {
            crate::i18n::tf(
                "template.reader_next_hint",
                &[("label", next.label.clone())],
            )
        });
        div()
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_size(theme::text_meta())
                    .child(
                        div()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_color(theme::muted())
                            .child(crate::i18n::tf(
                                "template.reader_progress",
                                &[
                                    ("current", (index + 1).to_string()),
                                    ("total", items.len().to_string()),
                                ],
                            )),
                    )
                    .children(
                        next_hint.map(|hint| {
                            div().text_color(theme::faint()).child(format!("· {hint}"))
                        }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(items.len() > 1, |el| {
                        el.child(
                            Button::new(
                                "template-reader-prev",
                                crate::i18n::text("template.previous"),
                            )
                            .debug_selector("TEMPLATE_IMPORT_READER_PREV")
                            .size(ButtonSize::Sm)
                            .disabled(!has_prev)
                            .on_press(move |_, cx| {
                                prev_root.update(cx, |this, cx| {
                                    this.step_template_reader(-1, cx);
                                });
                            }),
                        )
                        .child(
                            Button::new("template-reader-next", crate::i18n::text("template.next"))
                                .debug_selector("TEMPLATE_IMPORT_READER_NEXT")
                                .size(ButtonSize::Sm)
                                .disabled(!has_next)
                                .on_press(move |_, cx| {
                                    next_root.update(cx, |this, cx| {
                                        this.step_template_reader(1, cx);
                                    });
                                }),
                        )
                    })
                    .child(
                        Button::new(
                            "template-reader-done",
                            crate::i18n::text("template.back_to_import"),
                        )
                        .debug_selector("TEMPLATE_IMPORT_READER_DONE")
                        .size(ButtonSize::Sm)
                        .variant(ButtonVariant::Primary)
                        .on_press(move |window, cx| {
                            done_root.update(cx, |this, cx| this.close_template_reader(window, cx));
                        }),
                    ),
            )
            .into_any_element()
    }

    fn render_error_header(&self, busy: bool, cx: &mut Context<Self>) -> AnyElement {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        svg()
                            .path("circle-alert.svg")
                            .size(rems(1.))
                            .text_color(theme::danger()),
                    )
                    .child(
                        div()
                            .text_size(theme::text_lead())
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::text())
                            .child(crate::i18n::text("template.error_title")),
                    ),
            )
            .child(self.render_close_button(busy, cx))
            .into_any_element()
    }

    fn render_error_body(
        &self,
        error: &ImportErrorState,
        _busy: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let drop_root = cx.entity();
        let file_meta = match error.line {
            Some(line) => crate::i18n::tf(
                "template.error_file_line",
                &[("name", error.filename.clone()), ("line", line.to_string())],
            ),
            None => crate::i18n::tf("template.error_file", &[("name", error.filename.clone())]),
        };
        div()
            .when(cfg!(test), |el| {
                el.debug_selector(|| "TEMPLATE_IMPORT_ERROR_STATE".into())
            })
            .flex()
            .flex_col()
            .gap_4()
            .child(
                div()
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .p(rems(14. / 16.))
                    .rounded(rems(8. / 16.))
                    .bg(danger_tint())
                    .border_1()
                    .border_color(danger_border())
                    .child(
                        div()
                            .text_size(theme::text_body())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::text())
                            .child(error.message.clone()),
                    )
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .text_color(theme::muted())
                            .child(crate::i18n::text("template.error_nothing_created")),
                    )
                    .child(
                        div()
                            .font_family(theme::UI_MONOSPACE_FONT)
                            .text_size(theme::text_meta())
                            .text_color(theme::muted())
                            .child(file_meta),
                    ),
            )
            .child(
                div()
                    .id("template-import-error-drop-zone")
                    .when(cfg!(test), |el| {
                        el.debug_selector(|| "TEMPLATE_IMPORT_ERROR_DROP_ZONE".into())
                    })
                    .w_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .px_4()
                    .py_4()
                    .rounded(rems(8. / 16.))
                    .bg(theme::bg())
                    .border_1()
                    .border_color(theme::border_strong())
                    .text_size(theme::text_ui())
                    .text_color(theme::muted())
                    .drag_over::<ExternalPaths>(|style, _, _, _| {
                        style
                            .bg(accent_tint())
                            .border_1()
                            .border_color(theme::accent())
                    })
                    .on_drop(move |paths: &ExternalPaths, window, cx| {
                        if let Some(path) = paths.paths().first().cloned() {
                            drop_root.update(cx, |this, cx| {
                                this.load_template_import_file(path, window, cx);
                            });
                        }
                    })
                    .child(crate::i18n::text("template.error_drop_retry")),
            )
            .into_any_element()
    }

    fn render_error_footer(&self, busy: bool, cx: &mut Context<Self>) -> AnyElement {
        let close_root = cx.entity();
        let choose_root = cx.entity();
        div()
            .w_full()
            .flex()
            .items_center()
            .justify_end()
            .gap_2()
            .child(
                Button::new("template-import-error-close", crate::i18n::text("ui.close"))
                    .disabled(busy)
                    .on_press(move |window, cx| {
                        close_root
                            .update(cx, |this, cx| this.close_template_import_modal(window, cx));
                    }),
            )
            .child(
                Button::new(
                    "template-import-error-choose",
                    crate::i18n::text("template.choose_another_file"),
                )
                .debug_selector("TEMPLATE_IMPORT_ERROR_CHOOSE")
                .variant(ButtonVariant::Primary)
                .loading(busy)
                .on_press(move |window, cx| {
                    choose_root.update(cx, |this, cx| {
                        this.choose_template_import_file(window, cx);
                    });
                }),
            )
            .into_any_element()
    }
}
