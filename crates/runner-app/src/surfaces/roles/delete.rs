use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyElement, Context};
use runner_app::ui::ConfirmDialog;

use crate::surfaces::*;
use crate::*;

impl NativeRoot {
    pub(crate) fn render_entity_overlays(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut overlays = Vec::new();
        if let Some(menu) = self.role_surfaces.context_menu.clone() {
            overlays.push(menu.into_any_element());
        }
        if let Some(menu) = self.crew_surfaces.context_menu.clone() {
            overlays.push(menu.into_any_element());
        }
        if self.role_surfaces.create.is_some() {
            overlays.push(self.render_create_role_modal(cx));
        }
        if self.role_surfaces.edit.is_some() {
            overlays.push(self.render_role_edit_drawer(cx));
        }
        if self.role_surfaces.delete_confirm.is_some() {
            overlays.push(self.render_role_delete_confirm(cx));
        }
        if self.start_mission_modal.is_some() {
            overlays.push(self.render_start_mission_modal(cx));
        }
        if matches!(self.route, AppRoute::Mission(_)) {
            let workspace = self.mission_workspace.clone();
            overlays.extend(workspace.update(cx, |workspace, workspace_cx| {
                workspace.render_mission_overlays(workspace_cx)
            }));
        }
        overlays.extend(self.render_crew_overlays(cx));
        overlays
    }

    fn render_role_delete_confirm(&self, cx: &mut Context<Self>) -> AnyElement {
        let confirm = self
            .role_surfaces
            .delete_confirm
            .as_ref()
            .expect("role delete confirm");
        let root = cx.entity();
        let confirm_root = root.clone();
        let cancel_root = root;
        ConfirmDialog::new(
            format!("Delete role @{}?", confirm.handle),
            format!(
                "This deletes @{} and its archived session history. Remove it from every crew and archive its chats first. Crews and missions are kept.",
                confirm.handle
            ),
            "Delete role",
            "Deleting…",
            self.role_surfaces.delete_busy,
            Rc::new(move |_, cx| {
                confirm_root.update(cx, |this, cx| this.confirm_role_delete(cx));
            }),
            Rc::new(move |_, cx| {
                cancel_root.update(cx, |this, cx| {
                    if !this.role_surfaces.delete_busy {
                        this.role_surfaces.delete_confirm = None;
                        cx.notify();
                    }
                });
            }),
        )
        .into_any_element()
    }

    fn confirm_role_delete(&mut self, cx: &mut Context<Self>) {
        let Some(confirm) = self.role_surfaces.delete_confirm.as_ref() else {
            return;
        };
        if self.role_surfaces.delete_busy {
            return;
        }
        self.role_surfaces.delete_busy = true;
        let id = confirm.id.clone();
        let handle = confirm.handle.clone();
        let core = self.core(cx).clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::role::role_delete(&core, &id).map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                this.role_surfaces.delete_busy = false;
                match result {
                    Ok(()) => {
                        this.role_surfaces.delete_confirm = None;
                        if let Ok(roles) = runner_backend::ops::role::role_list(this.core(cx)) {
                            this.app_store
                                .update(cx, |store, store_cx| store.replace_roles(roles, store_cx));
                        }
                        this.load_role_page(cx);
                        this.show_toast(
                            format!("Deleted role @{handle}."),
                            crate::toast::ToastTone::Success,
                            cx,
                        );
                    }
                    Err(error) => {
                        this.role_surfaces.delete_confirm = None;
                        this.show_toast(error, crate::toast::ToastTone::Error, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
