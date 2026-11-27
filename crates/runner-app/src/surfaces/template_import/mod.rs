pub(crate) mod logic;
mod render;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap};
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{Context, Entity, FocusHandle, PathPromptOptions, ScrollHandle, Subscription, Window};
use runner_app::ui::{Scrollbar, StyledSelect, TextField};
use runner_core::protocol::model::Runtime;
use runner_core::protocol::template::TemplateImportInput;
use runner_core::template::{
    ExportedTemplate, Template, TemplatePreview, TemplatePreviewPlan, MAX_TEMPLATE_BYTES,
};

use self::logic::{
    build_reader_items, bulk_missing_runtimes, bulk_runtime_options, extract_error_line,
    missing_runtime_options, validate_template_bytes, ReaderItem,
};
use crate::surfaces::roles::logic::validate_role_handle;
use crate::toast::ToastTone;
use crate::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReaderMode {
    Source,
    Rendered,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ImportErrorState {
    pub(crate) filename: String,
    pub(crate) message: String,
    pub(crate) line: Option<usize>,
}

pub(crate) struct PreviewState {
    pub(crate) filename: String,
    pub(crate) toml: String,
    pub(crate) plan: TemplatePreviewPlan,
    pub(crate) crew_name: Option<Entity<TextField>>,
    pub(crate) role_display_name: Option<Entity<TextField>>,
    pub(crate) role_handle: Option<Entity<TextField>>,
    pub(crate) runtime_overrides: BTreeMap<String, Runtime>,
    pub(crate) slot_selects: HashMap<String, Entity<StyledSelect>>,
    pub(crate) bulk_selects: HashMap<String, Entity<StyledSelect>>,
    pub(crate) role_select: Option<Entity<StyledSelect>>,
    pub(crate) error: Option<String>,
    pub(crate) _subscriptions: Vec<Subscription>,
}

pub(crate) enum ImportStep {
    Upload,
    Preview(Box<PreviewState>),
    Reader {
        preview: Box<PreviewState>,
        items: Vec<ReaderItem>,
        index: usize,
        mode: ReaderMode,
    },
    Error(ImportErrorState),
}

pub(crate) struct TemplateImportModal {
    pub(crate) focus: FocusHandle,
    pub(crate) return_focus: Option<FocusHandle>,
    pub(crate) step: ImportStep,
    pub(crate) loading: bool,
    pub(crate) submitting: bool,
    pub(crate) drag_hover_filename: Option<String>,
    pub(crate) scroll: ScrollHandle,
    pub(crate) scrollbar: Entity<Scrollbar>,
    pub(crate) reader_scroll: ScrollHandle,
    pub(crate) reader_scrollbar: Entity<Scrollbar>,
}

impl TemplateImportModal {
    pub(crate) fn is_busy(&self) -> bool {
        self.loading || self.submitting
    }

    pub(crate) fn has_open_select(&self, cx: &App) -> bool {
        let ImportStep::Preview(preview) = &self.step else {
            return false;
        };
        preview.slot_selects.values().any(|s| s.read(cx).is_open())
            || preview.bulk_selects.values().any(|s| s.read(cx).is_open())
            || preview
                .role_select
                .as_ref()
                .is_some_and(|s| s.read(cx).is_open())
    }
}

pub(crate) fn write_export_file(path: &Path, content: &str) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(|err| err.to_string())?;
    tmp.write_all(content.as_bytes())
        .map_err(|err| err.to_string())?;
    tmp.flush().map_err(|err| err.to_string())?;
    tmp.persist(path).map_err(|err| err.error.to_string())?;
    Ok(())
}

impl NativeRoot {
    pub(crate) fn clear_template_import_modal(&mut self) {
        self.template_import_modal = None;
        self.template_import_generation = self.template_import_generation.wrapping_add(1);
    }

    pub(crate) fn open_template_import_modal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.remote_client.is_active() {
            return;
        }
        self.role_surfaces.context_menu = None;
        self.crew_surfaces.context_menu = None;
        self.template_import_generation = self.template_import_generation.wrapping_add(1);
        let focus = cx.focus_handle();
        let return_focus = window.focused(cx);
        let scroll = ScrollHandle::new();
        let owner = cx.entity_id();
        let scrollbar = cx.new(|_| Scrollbar::app(scroll.clone(), owner));
        let reader_scroll = ScrollHandle::new();
        let reader_scrollbar = cx.new(|_| Scrollbar::app(reader_scroll.clone(), owner));
        self.template_import_modal = Some(TemplateImportModal {
            focus: focus.clone(),
            return_focus,
            step: ImportStep::Upload,
            loading: false,
            submitting: false,
            drag_hover_filename: None,
            scroll,
            scrollbar,
            reader_scroll,
            reader_scrollbar,
        });
        focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn close_template_import_modal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .template_import_modal
            .as_ref()
            .is_some_and(TemplateImportModal::is_busy)
        {
            return;
        }
        self.template_import_generation = self.template_import_generation.wrapping_add(1);
        let return_focus = self
            .template_import_modal
            .take()
            .and_then(|modal| modal.return_focus);
        if let Some(focus) = return_focus {
            focus.focus(window, cx);
        } else {
            window.focus(&self.root_focus, cx);
        }
        cx.notify();
    }

    pub(crate) fn set_template_drag_hover(
        &mut self,
        filename: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(modal) = self.template_import_modal.as_mut() else {
            return;
        };
        if modal.drag_hover_filename != filename {
            modal.drag_hover_filename = filename;
            cx.notify();
        }
    }

    pub(crate) fn handle_template_import_escape(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(modal) = self.template_import_modal.as_mut() else {
            return false;
        };
        if modal.is_busy() {
            return true;
        }
        if matches!(modal.step, ImportStep::Reader { .. }) {
            let step = std::mem::replace(&mut modal.step, ImportStep::Upload);
            if let ImportStep::Reader { preview, .. } = step {
                modal.step = ImportStep::Preview(preview);
            }
            let focus = modal.focus.clone();
            focus.focus(window, cx);
            cx.notify();
            return true;
        }
        self.close_template_import_modal(window, cx);
        true
    }

    pub(crate) fn choose_template_import_file(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.remote_client.is_active()
            || self
                .template_import_modal
                .as_ref()
                .is_none_or(TemplateImportModal::is_busy)
        {
            return;
        }
        let generation = self.template_import_generation;
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(crate::i18n::t("template.choose_file_prompt")),
        });
        cx.spawn_in(window, async move |weak, cx| {
            let Ok(Ok(Some(mut paths))) = selected.await else {
                return;
            };
            let Some(path) = paths.drain(..).next() else {
                return;
            };
            let _ = weak.update_in(cx, |this, window, cx| {
                if this.remote_client.is_active()
                    || this.template_import_generation != generation
                    || this.template_import_modal.is_none()
                {
                    return;
                }
                this.load_template_import_file(path, window, cx);
            });
        })
        .detach();
    }

    pub(crate) fn load_template_import_file(
        &mut self,
        path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.remote_client.is_active() {
            return;
        }
        let Some(modal) = self.template_import_modal.as_mut() else {
            return;
        };
        if modal.is_busy() {
            return;
        }
        let filename = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        modal.drag_hover_filename = None;
        modal.loading = true;
        self.template_import_generation = self.template_import_generation.wrapping_add(1);
        let generation = self.template_import_generation;
        #[cfg(test)]
        {
            self.template_import_in_flight += 1;
        }
        cx.notify();

        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            // Check file type before opening so special files (FIFOs, devices)
            // fail immediately instead of blocking `File::open` indefinitely.
            let metadata = std::fs::metadata(&path).map_err(|err| {
                (
                    crate::i18n::tf("template.read_file_failed", &[("error", err.to_string())]),
                    None,
                )
            })?;
            if !metadata.is_file() {
                return Err((
                    crate::i18n::text("template.file_not_regular").to_owned(),
                    None,
                ));
            }
            if metadata.len() as usize > MAX_TEMPLATE_BYTES {
                return Err((
                    crate::i18n::text("template.file_too_large").to_owned(),
                    None,
                ));
            }
            let file = std::fs::File::open(&path).map_err(|err| {
                (
                    crate::i18n::tf("template.read_file_failed", &[("error", err.to_string())]),
                    None,
                )
            })?;
            let fd_meta = file.metadata().map_err(|err| {
                (
                    crate::i18n::tf("template.read_file_failed", &[("error", err.to_string())]),
                    None,
                )
            })?;
            if !fd_meta.is_file() {
                return Err((
                    crate::i18n::text("template.file_not_regular").to_owned(),
                    None,
                ));
            }
            let mut bytes = Vec::new();
            file.take((MAX_TEMPLATE_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|err| {
                    (
                        crate::i18n::tf("template.read_file_failed", &[("error", err.to_string())]),
                        None,
                    )
                })?;
            let toml = validate_template_bytes(&bytes)
                .map_err(|text| (text.resolve().to_string(), None))?;
            let preview = core.template_preview(&toml).map_err(|err| {
                let message = err.to_string();
                let line = extract_error_line(&message);
                (message, line)
            })?;
            match preview {
                TemplatePreview::Valid(plan) => Ok::<_, (String, Option<usize>)>((toml, *plan)),
                TemplatePreview::Invalid { error } => Err((error.message, error.line)),
            }
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                #[cfg(test)]
                {
                    this.template_import_in_flight =
                        this.template_import_in_flight.saturating_sub(1);
                }
                if this.remote_client.is_active()
                    || this.template_import_generation != generation
                    || this.template_import_modal.is_none()
                {
                    return;
                }
                match result {
                    Ok((toml, plan)) => {
                        let state = this.build_preview_state(filename, toml, plan, cx);
                        if let Some(modal) = this.template_import_modal.as_mut() {
                            modal.loading = false;
                            modal.drag_hover_filename = None;
                            modal
                                .scroll
                                .set_offset(gpui::point(gpui::px(0.), gpui::px(0.)));
                            modal.step = ImportStep::Preview(Box::new(state));
                            modal.focus.focus(window, cx);
                        }
                    }
                    Err((message, line)) => {
                        if let Some(modal) = this.template_import_modal.as_mut() {
                            modal.loading = false;
                            modal.drag_hover_filename = None;
                            modal.step = ImportStep::Error(ImportErrorState {
                                filename,
                                message,
                                line,
                            });
                            modal.focus.focus(window, cx);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn build_preview_state(
        &self,
        filename: String,
        toml: String,
        plan: TemplatePreviewPlan,
        cx: &mut Context<Self>,
    ) -> PreviewState {
        let root = cx.entity();
        let mut subscriptions = Vec::new();
        let mut crew_name = None;
        let mut role_display_name = None;
        let mut role_handle = None;
        let mut slot_selects = HashMap::new();
        let mut bulk_selects = HashMap::new();
        let mut role_select = None;

        match &plan.template {
            Template::Crew(crew) => {
                let initial_name = plan
                    .suggested_crew_name
                    .clone()
                    .unwrap_or_else(|| crew.crew.name.clone());
                let field = cx.new(|field_cx| {
                    TextField::new(
                        field_cx.focus_handle(),
                        initial_name,
                        crate::i18n::message("template.crew_name_placeholder"),
                        false,
                    )
                });
                subscriptions.push(cx.observe(&field, |_, _, cx| cx.notify()));
                crew_name = Some(field);

                let roles_by_handle: HashMap<&str, &runner_core::template::RoleTemplate> = crew
                    .roles
                    .iter()
                    .map(|role| (role.handle.as_str(), role))
                    .collect();

                for slot in &crew.slots {
                    let Some(role) = roles_by_handle.get(slot.role.as_str()) else {
                        continue;
                    };
                    let template_rt = slot.runtime.unwrap_or(role.runtime);
                    if !plan.installed_runtimes.contains(&template_rt) {
                        let options =
                            missing_runtime_options(&plan.installed_runtimes, template_rt);
                        let slot_handle = slot.handle.clone();
                        let select_root = root.clone();
                        let select_id: gpui::SharedString =
                            format!("template-slot-rt-{}", slot.handle).into();
                        let select = cx.new(|select_cx| {
                            StyledSelect::new(
                                select_id,
                                select_cx.focus_handle(),
                                template_rt.key(),
                                options,
                                Rc::new(move |value, _, cx| {
                                    let slot_handle = slot_handle.clone();
                                    select_root.update(cx, |this, cx| {
                                        this.set_template_slot_runtime(&slot_handle, &value, cx);
                                    });
                                }),
                                select_cx,
                            )
                            .compact(true)
                            .width(gpui::px(220.))
                            .min_menu_width(gpui::px(240.))
                        });
                        slot_selects.insert(slot.handle.clone(), select);
                    }
                }

                if !plan.installed_runtimes.is_empty() {
                    let bulk_options = bulk_runtime_options(&plan.installed_runtimes);
                    for bulk in
                        bulk_missing_runtimes(crew, &BTreeMap::new(), &plan.installed_runtimes)
                    {
                        let from_rt = bulk.runtime;
                        let select_root = root.clone();
                        let select_id: gpui::SharedString =
                            format!("template-bulk-rt-{}", from_rt.key()).into();
                        let opts = bulk_options.clone();
                        let select = cx.new(|select_cx| {
                            StyledSelect::new(
                                select_id,
                                select_cx.focus_handle(),
                                "",
                                opts,
                                Rc::new(move |value, _, cx| {
                                    select_root.update(cx, |this, cx| {
                                        this.bulk_switch_template_runtime(from_rt, &value, cx);
                                    });
                                }),
                                select_cx,
                            )
                            .placeholder(crate::i18n::message("template.switch_all_placeholder"))
                            .compact(true)
                            .width(gpui::px(156.))
                            .min_menu_width(gpui::px(210.))
                        });
                        bulk_selects.insert(from_rt.key().to_owned(), select);
                    }
                }
            }
            Template::Role(standalone) => {
                let name_field = cx.new(|field_cx| {
                    TextField::new(
                        field_cx.focus_handle(),
                        standalone.role.display_name.clone(),
                        crate::i18n::message("template.display_name_placeholder"),
                        false,
                    )
                });
                subscriptions.push(cx.observe(&name_field, |_, _, cx| cx.notify()));
                role_display_name = Some(name_field);

                let initial_handle = plan
                    .planned_roles
                    .first()
                    .map(|role| role.planned_handle.clone())
                    .unwrap_or_else(|| standalone.role.handle.clone());
                let handle_field = cx.new(|field_cx| {
                    TextField::new(
                        field_cx.focus_handle(),
                        initial_handle,
                        crate::i18n::message("template.handle_placeholder"),
                        false,
                    )
                });
                subscriptions.push(cx.observe(&handle_field, |_, _, cx| cx.notify()));
                role_handle = Some(handle_field);

                let template_rt = standalone.role.runtime;
                if !plan.installed_runtimes.contains(&template_rt) {
                    let options = missing_runtime_options(&plan.installed_runtimes, template_rt);
                    let select_root = root;
                    let select = cx.new(|select_cx| {
                        StyledSelect::new(
                            "template-standalone-role-rt",
                            select_cx.focus_handle(),
                            template_rt.key(),
                            options,
                            Rc::new(move |value, _, cx| {
                                select_root.update(cx, |this, cx| {
                                    this.set_template_role_runtime(&value, cx);
                                });
                            }),
                            select_cx,
                        )
                        .compact(true)
                        .width(gpui::px(240.))
                        .min_menu_width(gpui::px(260.))
                    });
                    role_select = Some(select);
                }
            }
        }

        PreviewState {
            filename,
            toml,
            plan,
            crew_name,
            role_display_name,
            role_handle,
            runtime_overrides: BTreeMap::new(),
            slot_selects,
            bulk_selects,
            role_select,
            error: None,
            _subscriptions: subscriptions,
        }
    }

    pub(crate) fn set_template_slot_runtime(
        &mut self,
        slot_handle: &str,
        runtime_key: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(runtime) = Runtime::parse(runtime_key) else {
            return;
        };
        let Some(modal) = self.template_import_modal.as_mut() else {
            return;
        };
        let ImportStep::Preview(state) = &mut modal.step else {
            return;
        };
        let Template::Crew(crew) = &state.plan.template else {
            return;
        };
        let template_rt = crew
            .slots
            .iter()
            .find(|s| s.handle == slot_handle)
            .and_then(|slot| {
                let role = crew.roles.iter().find(|r| r.handle == slot.role)?;
                Some(slot.runtime.unwrap_or(role.runtime))
            });
        if template_rt == Some(runtime) {
            state.runtime_overrides.remove(slot_handle);
        } else {
            state
                .runtime_overrides
                .insert(slot_handle.to_owned(), runtime);
        }
        // StyledSelect::choose already sets the initiating select's value
        // before invoking on_change, so never re-enter that entity here.
        cx.notify();
    }

    pub(crate) fn bulk_switch_template_runtime(
        &mut self,
        from_runtime: Runtime,
        to_runtime_key: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = Runtime::parse(to_runtime_key) else {
            return;
        };
        let Some(modal) = self.template_import_modal.as_mut() else {
            return;
        };
        let ImportStep::Preview(state) = &mut modal.step else {
            return;
        };
        let Template::Crew(crew) = &state.plan.template else {
            return;
        };
        let roles_by_handle: HashMap<&str, &runner_core::template::RoleTemplate> = crew
            .roles
            .iter()
            .map(|role| (role.handle.as_str(), role))
            .collect();
        let mut updated_slots = Vec::new();
        for slot in &crew.slots {
            let Some(role) = roles_by_handle.get(slot.role.as_str()) else {
                continue;
            };
            let template_rt = slot.runtime.unwrap_or(role.runtime);
            let current_rt = state
                .runtime_overrides
                .get(&slot.handle)
                .copied()
                .unwrap_or(template_rt);
            if current_rt == from_runtime {
                if target == template_rt {
                    state.runtime_overrides.remove(&slot.handle);
                } else {
                    state.runtime_overrides.insert(slot.handle.clone(), target);
                }
                updated_slots.push(slot.handle.clone());
            }
        }
        for handle in updated_slots {
            if let Some(select) = state.slot_selects.get(&handle) {
                let value = target.key().to_owned();
                select.update(cx, |s, cx| s.set_value(value, cx));
            }
        }
        // Defer resetting the initiating bulk select back to its placeholder
        // so we never re-enter its currently borrowed GPUI entity inside on_change.
        if let Some(bulk_select) = state.bulk_selects.get(from_runtime.key()).cloned() {
            cx.defer(move |cx| {
                bulk_select.update(cx, |s, cx| s.set_value(String::new(), cx));
            });
        }
        cx.notify();
    }

    pub(crate) fn set_template_role_runtime(&mut self, runtime_key: &str, cx: &mut Context<Self>) {
        let Some(runtime) = Runtime::parse(runtime_key) else {
            return;
        };
        let Some(modal) = self.template_import_modal.as_mut() else {
            return;
        };
        let ImportStep::Preview(state) = &mut modal.step else {
            return;
        };
        let Template::Role(standalone) = &state.plan.template else {
            return;
        };
        if runtime == standalone.role.runtime {
            state.runtime_overrides.remove(&standalone.role.handle);
        } else {
            state
                .runtime_overrides
                .insert(standalone.role.handle.clone(), runtime);
        }
        // StyledSelect::choose already updated role_select's value before on_change.
        cx.notify();
    }

    pub(crate) fn open_template_reader(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(modal) = self.template_import_modal.as_mut() else {
            return;
        };
        if !matches!(modal.step, ImportStep::Preview(_)) {
            return;
        }
        let step = std::mem::replace(&mut modal.step, ImportStep::Upload);
        let ImportStep::Preview(preview) = step else {
            return;
        };
        let items = build_reader_items(&preview.plan, &preview.runtime_overrides);
        if items.is_empty() {
            modal.step = ImportStep::Preview(preview);
            return;
        }
        let clamped = index.min(items.len() - 1);
        modal
            .reader_scroll
            .set_offset(gpui::point(gpui::px(0.), gpui::px(0.)));
        modal.step = ImportStep::Reader {
            preview,
            items,
            index: clamped,
            mode: ReaderMode::Source,
        };
        modal.focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn close_template_reader(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.template_import_modal.as_mut() else {
            return;
        };
        if !matches!(modal.step, ImportStep::Reader { .. }) {
            return;
        }
        let step = std::mem::replace(&mut modal.step, ImportStep::Upload);
        if let ImportStep::Reader { preview, .. } = step {
            modal.step = ImportStep::Preview(preview);
            modal.focus.focus(window, cx);
        }
        cx.notify();
    }

    pub(crate) fn set_template_reader_mode(&mut self, mode: ReaderMode, cx: &mut Context<Self>) {
        let Some(modal) = self.template_import_modal.as_mut() else {
            return;
        };
        if let ImportStep::Reader { mode: current, .. } = &mut modal.step {
            if *current != mode {
                *current = mode;
                modal
                    .reader_scroll
                    .set_offset(gpui::point(gpui::px(0.), gpui::px(0.)));
                cx.notify();
            }
        }
    }

    pub(crate) fn step_template_reader(&mut self, delta: isize, cx: &mut Context<Self>) {
        let Some(modal) = self.template_import_modal.as_mut() else {
            return;
        };
        if let ImportStep::Reader { items, index, .. } = &mut modal.step {
            let next =
                (*index as isize + delta).clamp(0, items.len().saturating_sub(1) as isize) as usize;
            if next != *index {
                *index = next;
                modal
                    .reader_scroll
                    .set_offset(gpui::point(gpui::px(0.), gpui::px(0.)));
                cx.notify();
            }
        }
    }

    pub(crate) fn return_to_template_upload(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(modal) = self.template_import_modal.as_mut() else {
            return;
        };
        if modal.is_busy() {
            return;
        }
        self.template_import_generation = self.template_import_generation.wrapping_add(1);
        modal.drag_hover_filename = None;
        modal.step = ImportStep::Upload;
        modal.focus.focus(window, cx);
        cx.notify();
    }

    pub(crate) fn submit_template_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.remote_client.is_active() {
            return;
        }
        let Some(modal) = self.template_import_modal.as_ref() else {
            return;
        };
        if modal.is_busy() {
            return;
        }
        let ImportStep::Preview(state) = &modal.step else {
            return;
        };

        let mut input = TemplateImportInput {
            toml: state.toml.clone(),
            runtime_overrides: state.runtime_overrides.clone(),
            ..Default::default()
        };
        match &state.plan.template {
            Template::Crew(_) => {
                let name = state
                    .crew_name
                    .as_ref()
                    .map(|field| field.read(cx).text().trim().to_owned())
                    .unwrap_or_default();
                if name.is_empty() {
                    if let Some(modal) = self.template_import_modal.as_mut() {
                        if let ImportStep::Preview(state) = &mut modal.step {
                            state.error =
                                Some(crate::i18n::text("template.crew_name_required").to_owned());
                        }
                    }
                    cx.notify();
                    return;
                }
                input.crew_name = Some(name);
            }
            Template::Role(_) => {
                let display_name = state
                    .role_display_name
                    .as_ref()
                    .map(|field| field.read(cx).text().trim().to_owned())
                    .unwrap_or_default();
                let raw_handle = state
                    .role_handle
                    .as_ref()
                    .map(|field| field.read(cx).text().trim().to_owned())
                    .unwrap_or_default();
                let handle = raw_handle
                    .strip_prefix('@')
                    .unwrap_or(&raw_handle)
                    .to_owned();
                if display_name.is_empty() {
                    if let Some(modal) = self.template_import_modal.as_mut() {
                        if let ImportStep::Preview(state) = &mut modal.step {
                            state.error = Some(
                                crate::i18n::text("template.display_name_required").to_owned(),
                            );
                        }
                    }
                    cx.notify();
                    return;
                }
                if handle.is_empty() {
                    if let Some(modal) = self.template_import_modal.as_mut() {
                        if let ImportStep::Preview(state) = &mut modal.step {
                            state.error =
                                Some(crate::i18n::text("template.handle_required").to_owned());
                        }
                    }
                    cx.notify();
                    return;
                }
                if let Some(err_key) = validate_role_handle(&handle) {
                    if let Some(modal) = self.template_import_modal.as_mut() {
                        if let ImportStep::Preview(state) = &mut modal.step {
                            state.error = Some(crate::i18n::text(err_key).to_owned());
                        }
                    }
                    cx.notify();
                    return;
                }
                input.role_display_name = Some(display_name);
                input.role_handle = Some(handle);
            }
        }

        self.template_import_generation = self.template_import_generation.wrapping_add(1);
        let generation = self.template_import_generation;
        #[cfg(test)]
        {
            self.template_import_in_flight += 1;
        }
        if let Some(modal) = self.template_import_modal.as_mut() {
            modal.submitting = true;
            if let ImportStep::Preview(state) = &mut modal.step {
                state.error = None;
            }
        }
        cx.notify();

        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            core.template_import(input).map_err(|err| err.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                #[cfg(test)]
                {
                    this.template_import_in_flight =
                        this.template_import_in_flight.saturating_sub(1);
                }
                if this.remote_client.is_active() || this.template_import_generation != generation {
                    if result.is_ok() {
                        this.refresh_store(StoreRefreshKind::All, cx);
                    }
                    return;
                }
                match result {
                    Ok(imported) => {
                        this.clear_template_import_modal();
                        this.refresh_store(StoreRefreshKind::All, cx);
                        this.load_crew_page(cx);
                        this.load_role_page(cx);
                        if let Some(crew) = imported.crew {
                            let name = crew.name.clone();
                            let crew_id = crew.id.clone();
                            this.show_toast(
                                crate::i18n::messagef(
                                    "template.imported_crew_toast",
                                    &[("name", name)],
                                ),
                                ToastTone::Success,
                                cx,
                            );
                            this.open_crew_editor(crew_id, window, cx);
                        } else if let Some(role) = imported.roles.into_iter().next() {
                            let handle = role.handle.clone();
                            this.show_toast(
                                crate::i18n::messagef(
                                    "template.imported_role_toast",
                                    &[("handle", handle.clone())],
                                ),
                                ToastTone::Success,
                                cx,
                            );
                            this.open_role_detail(handle, window, cx);
                        }
                    }
                    Err(err) => {
                        if let Some(modal) = this.template_import_modal.as_mut() {
                            modal.submitting = false;
                            if let ImportStep::Preview(state) = &mut modal.step {
                                state.error = Some(err);
                            }
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(crate) fn export_crew_template(
        &mut self,
        crew_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.remote_client.is_active() {
            return;
        }
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            core.template_export_crew(&crew_id)
                .map_err(|err| err.to_string())
        });
        self.finish_template_export(task, window, cx);
    }

    pub(crate) fn export_role_template(
        &mut self,
        role_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.remote_client.is_active() {
            return;
        }
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            core.template_export_role(&role_id)
                .map_err(|err| err.to_string())
        });
        self.finish_template_export(task, window, cx);
    }

    fn finish_template_export(
        &mut self,
        task: gpui::Task<Result<ExportedTemplate, String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        cx.spawn_in(window, async move |weak, cx| {
            let exported = match task.await {
                Ok(exported) => exported,
                Err(err) => {
                    let _ = weak.update(cx, |this, cx| {
                        this.show_toast(err, ToastTone::Error, cx);
                    });
                    return;
                }
            };
            let prompt = weak.update(cx, |this, cx| {
                if this.remote_client.is_active() {
                    return None;
                }
                let dir = runner_core::app_paths::home_dir().unwrap_or_else(|| PathBuf::from("."));
                Some(cx.prompt_for_new_path(&dir, Some(&exported.filename)))
            });
            let Ok(Some(prompt)) = prompt else {
                return;
            };
            let path = match prompt.await {
                Ok(Ok(Some(path))) => path,
                Ok(Ok(None)) => return,
                Ok(Err(err)) => {
                    let _ = weak.update(cx, |this, cx| {
                        this.show_toast(
                            crate::i18n::messagef(
                                "template.save_file_failed",
                                &[("error", err.to_string())],
                            ),
                            ToastTone::Error,
                            cx,
                        );
                    });
                    return;
                }
                Err(_) => return,
            };
            let display_name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| exported.filename.clone());
            let content = exported.toml;
            let write_task = cx.background_spawn(async move { write_export_file(&path, &content) });
            let write_result = write_task.await;
            let _ = weak.update(cx, |this, cx| {
                match write_result {
                    Ok(()) => this.show_toast(
                        crate::i18n::messagef(
                            "template.exported_toast",
                            &[("filename", display_name)],
                        ),
                        ToastTone::Success,
                        cx,
                    ),
                    Err(err) => this.show_toast(
                        crate::i18n::messagef("template.save_file_failed", &[("error", err)]),
                        ToastTone::Error,
                        cx,
                    ),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
