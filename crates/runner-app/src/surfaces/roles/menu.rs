use std::rc::Rc;

use gpui::prelude::*;
use gpui::{px, Context, Window};
use runner_app::ui::{ContextMenu, MenuItem as UiMenuItem};
use runner_backend::model::Role;
use runner_backend::ops::role::RoleWithActivity;

use super::*;
use crate::surfaces::*;
use crate::*;

impl NativeRoot {
    pub(super) fn open_role_menu(
        &mut self,
        item: RoleWithActivity,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let actions = [
            RoleMenuAction::Open(item.role.handle.clone()),
            RoleMenuAction::Delete {
                id: item.role.id,
                handle: item.role.handle,
            },
        ];
        let items = vec![
            UiMenuItem::new("Edit details").icon("pencil.svg"),
            UiMenuItem::new("Delete role")
                .icon("trash.svg")
                .destructive(true),
        ];
        let root = cx.entity();
        let dismiss_root = root.clone();
        let menu = cx.new(move |menu_cx| {
            let action_root = root;
            ContextMenu::new(
                "role-context-menu",
                menu_cx.focus_handle(),
                position,
                items,
                Rc::new(move |index, window, cx| {
                    if let Some(action) = actions.get(index).cloned() {
                        action_root.update(cx, |this, cx| {
                            this.handle_role_menu_action(action, window, cx)
                        });
                    }
                }),
                Rc::new(move |_, cx| {
                    dismiss_root.update(cx, |this, cx| {
                        this.role_surfaces.context_menu = None;
                        cx.notify();
                    });
                }),
            )
            .width(px(176.))
        });
        let focus = menu.read(cx).focus_handle();
        self.role_surfaces.context_menu = Some(menu);
        focus.focus(window);
        cx.notify();
    }

    fn handle_role_menu_action(
        &mut self,
        action: RoleMenuAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            RoleMenuAction::Open(handle) => self.open_role_detail(handle, window, cx),
            RoleMenuAction::Delete { id, handle } => {
                self.role_surfaces.delete_confirm = Some(RoleDeleteConfirm { id, handle });
                cx.notify();
            }
        }
    }

    pub(super) fn start_role_chat(
        &mut self,
        role: Role,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.role_surfaces.chat_pending.is_some() {
            return;
        }
        let detail_origin = matches!(self.route, AppRoute::RoleDetail(_));
        self.role_surfaces.chat_pending = Some(role.id.clone());
        let cwd = if role.working_dir.is_none() {
            let default = self.settings(cx).default_working_dir.trim();
            (!default.is_empty()).then(|| default.to_owned())
        } else {
            None
        };
        let core = self.core(cx).clone();
        let role_id = role.id.clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::session::session_start_direct(
                &core,
                role_id,
                None,
                None,
                None,
                runner_backend::ops::project::ProjectScope::Root,
                cwd,
                Some(INITIAL_COLS),
                Some(INITIAL_ROWS),
            )
            .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                this.role_surfaces.chat_pending = None;
                match result {
                    Ok(spawned) => {
                        let attach = (|| -> Result<()> {
                            this.refresh_sessions(cx);
                            this.reload_tabs(cx)?;
                            this.tabs.activate_session(&spawned.id);
                            this.sync_active_project_from_active_tab(cx);
                            this.set_route(AppRoute::Chat, cx);
                            this.ensure_active_tab_attached(window, cx)?;
                            Ok(())
                        })();
                        match attach {
                            Ok(()) => {
                                this.remember_active_role(cx);
                                this.mark_active_tab_viewed(window, cx);
                                this.sync_active_chat_detail(cx);
                                this.begin_chat_transition(
                                    &spawned.id,
                                    chat_lifecycle::TransitionKind::Starting,
                                    Some(0),
                                    window,
                                    cx,
                                );
                            }
                            Err(error) => this.error = Some(error.to_string()),
                        }
                    }
                    Err(error)
                        if detail_origin && matches!(this.route, AppRoute::RoleDetail(_)) =>
                    {
                        this.role_surfaces.detail.error = Some(error);
                    }
                    Err(error) => this.show_toast(error, crate::toast::ToastTone::Error, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
