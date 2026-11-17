use std::rc::Rc;

use gpui::prelude::*;
use gpui::{Context, Window};
use runner_app::ui::{ContextMenu, MenuItem as UiMenuItem};
use runner_core::protocol::model::Role;
use runner_core::protocol::role::RoleWithActivity;

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
            RoleMenuAction::Edit(Box::new(item.role.clone())),
            RoleMenuAction::Delete {
                id: item.role.id,
                handle: item.role.handle,
            },
        ];
        let items = vec![
            UiMenuItem::new(crate::i18n::message("ui.edit_details")).icon("pencil.svg"),
            UiMenuItem::new(crate::i18n::message("ui.delete_role"))
                .icon("trash.svg")
                .destructive(true),
        ];
        let root = cx.entity();
        let dismiss_root = root.clone();
        let return_focus = window.focused(cx);
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
            .return_focus(return_focus)
        });
        let focus = menu.read(cx).focus_handle();
        self.role_surfaces.context_menu = Some(menu);
        focus.focus(window, cx);
        cx.notify();
    }

    pub(super) fn handle_role_menu_action(
        &mut self,
        action: RoleMenuAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            RoleMenuAction::Edit(role) => {
                self.open_role_detail(role.handle.clone(), window, cx);
                self.open_role_edit(*role, window, cx);
            }
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
        let core = self.front_page_client(cx);
        let role_id = role.id.clone();
        let recent_role = role.id.clone();
        let target = core.target.clone();
        let directory = self.app_store.read(cx).app_data_dir.clone();
        let task = cx.background_spawn(async move {
            core.start_role_chat(role_id, cwd, (INITIAL_COLS, INITIAL_ROWS))
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                this.role_surfaces.chat_pending = None;
                match result {
                    Ok(spawned) => {
                        crate::surfaces::start_chat::launcher::record_launcher_use(
                            &directory,
                            &target,
                            crate::surfaces::start_chat::launcher_model::EntryKey::Role(
                                recent_role,
                            ),
                        );
                        let attach = (|| -> Result<()> {
                            this.refresh_sessions(cx);
                            this.reload_tabs(cx)?;
                            this.tabs.activate_session(&spawned);
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
                                    &spawned,
                                    chat_lifecycle::TransitionKind::Starting,
                                    Some(0),
                                    window,
                                    cx,
                                );
                            }
                            Err(error) => this.error = Some(error.to_string().into()),
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

impl NativeRoot {
    pub(crate) fn start_remote_role_direct(
        &mut self,
        role: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use crate::surfaces::front_page::client::{ChatLaunch, ChatStart};
        use crate::surfaces::start_chat::{
            launcher::record_launcher_use, launcher_model::EntryKey,
        };
        let client = self.front_page_client(cx);
        if self.role_page_surfaces().chat_pending.is_some()
            || !client.supports(&["catalog", "startChat"])
        {
            return;
        }
        self.role_page_surfaces_mut().chat_pending = Some(role.clone());
        let action = self.remote_client.prepare_chat_launch(client.clone(), None);
        let directory = self.app_store.read(cx).app_data_dir.clone();
        let target = client.target.clone();
        let task = cx.background_spawn(async move {
            let mut launch = ChatLaunch {
                input: ChatStart {
                    role: Some(role.clone()),
                    runtime: None,
                    model: None,
                    effort: None,
                    speed: None,
                    project: None,
                    cwd: None,
                    worktree: None,
                    size: (INITIAL_COLS, INITIAL_ROWS),
                    initial_prompt: None,
                },
                placement: None,
                started: None,
            };
            let result = client.launch_chat(&mut launch);
            if launch.started.is_some() {
                record_launcher_use(&directory, &target, EntryKey::Role(role));
            }
            result
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |root, _, cx| {
                let Some(action) = action else {
                    return;
                };
                if !root.remote_client.settle_chat_launch(&action) {
                    return;
                }
                root.role_page_surfaces_mut().chat_pending = None;
                match result {
                    Ok(output) => {
                        root.remote_client.complete_chat_launch(
                            action,
                            output.session,
                            output.layout,
                            cx,
                        );
                        root.sync_remote_state(cx);
                    }
                    Err(error) => {
                        root.show_toast(error.to_string(), crate::toast::ToastTone::Error, cx)
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
