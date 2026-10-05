use gpui::prelude::*;
use gpui::{AnyElement, Context, Window};

use crate::surfaces::*;
use crate::*;

impl NativeRoot {
    pub(super) fn confirm_slot_remove(&mut self, cx: &mut Context<Self>) {
        let Some(confirm) = self.crew_surfaces.slot_remove_confirm.as_ref() else {
            return;
        };
        if self.crew_surfaces.slot_remove_busy {
            return;
        }
        self.crew_surfaces.slot_remove_busy = true;
        let slot_id = confirm.slot.slot.id.clone();
        let crew_id = confirm.slot.slot.crew_id.clone();
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            let result = core
                .slot_delete(&slot_id)
                .map_err(|error| error.to_string());
            (crew_id, result)
        });
        cx.spawn(async move |weak, cx| {
            let (crew_id, result) = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.crew_surfaces.slot_remove_busy = false;
                this.crew_surfaces.slot_remove_confirm = None;
                match result {
                    Ok(()) => {
                        if matches!(
                            &this.route,
                            AppRoute::CrewEditor(active) if active == &crew_id
                        ) {
                            this.load_crew_editor(crew_id, cx);
                        }
                        this.load_crew_page(cx);
                        this.load_role_page(cx);
                    }
                    Err(error)
                        if matches!(
                            &this.route,
                            AppRoute::CrewEditor(active) if active == &crew_id
                        ) =>
                    {
                        this.crew_surfaces.editor.error = Some(error);
                    }
                    Err(_) => {}
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn advance_crew_delete(&mut self, cx: &mut Context<Self>) {
        let Some(confirm) = self.crew_surfaces.delete_confirm.as_ref() else {
            return;
        };
        if self.crew_surfaces.delete_busy {
            return;
        }
        self.crew_surfaces.delete_busy = true;
        let id = confirm.id.clone();
        let name = confirm.name.clone();
        let core = self.core(cx).clone();
        let task =
            cx.background_spawn(
                async move { core.crew_delete(&id).map_err(|error| error.to_string()) },
            );
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.crew_surfaces.delete_busy = false;
                this.crew_surfaces.delete_confirm = None;
                match result {
                    Ok(()) => {
                        this.load_crew_page(cx);
                        this.show_toast(
                            format!("Deleted crew \"{name}\"."),
                            crate::toast::ToastTone::Success,
                            cx,
                        );
                    }
                    Err(error) => {
                        this.show_toast(error, crate::toast::ToastTone::Error, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(crate) fn render_crew_overlays(
        &mut self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut overlays = Vec::new();
        overlays.extend(self.render_slot_popup(window, cx));
        if self.crew_surfaces.add_slot.is_some() {
            overlays.push(self.render_add_slot_modal(cx));
        }
        if self.crew_surfaces.delete_confirm.is_some() {
            overlays.push(self.render_crew_delete_confirm(cx));
        }
        if self.crew_surfaces.slot_remove_confirm.is_some() {
            overlays.push(self.render_slot_remove_confirm(cx));
        }
        overlays
    }

    pub(super) fn finish_crew_update(
        &mut self,
        task: gpui::Task<(String, Result<(), String>)>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |weak, cx| {
            let (crew_id, result) = task.await;
            let _ = weak.update(cx, |this, cx| {
                if this.crew_surfaces.editor.crew_id != crew_id {
                    return;
                }
                let reload_editor = matches!(
                    &this.route,
                    AppRoute::CrewEditor(active) if active == &crew_id
                );
                match result {
                    Ok(()) => {
                        this.crew_surfaces.editor.edit = None;
                        if reload_editor {
                            this.load_crew_editor(crew_id, cx);
                        }
                        this.load_crew_page(cx);
                    }
                    Err(error) => {
                        let editor = &mut this.crew_surfaces.editor;
                        if let Some(form) = editor.edit.as_mut() {
                            form.saving = false;
                        }
                        editor.error = Some(error);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
