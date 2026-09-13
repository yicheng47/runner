use std::rc::Rc;

use gpui::prelude::*;
use gpui::{px, Context, Window};
use runner_app::ui::{ContextMenu, MenuItem as UiMenuItem};
use runner_backend::model::Runner;
use runner_backend::ops::runner::RunnerWithActivity;

use super::*;
use crate::surfaces::*;
use crate::*;

impl NativeRoot {
    pub(super) fn open_runner_menu(
        &mut self,
        item: RunnerWithActivity,
        position: gpui::Point<gpui::Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let actions = [
            RunnerMenuAction::Open(item.runner.handle.clone()),
            RunnerMenuAction::Delete {
                id: item.runner.id,
                handle: item.runner.handle,
            },
        ];
        let items = vec![
            UiMenuItem::new("Edit details").icon("pencil.svg"),
            UiMenuItem::new("Delete runner")
                .icon("trash.svg")
                .destructive(true),
        ];
        let root = cx.entity();
        let dismiss_root = root.clone();
        let menu = cx.new(move |menu_cx| {
            let action_root = root;
            ContextMenu::new(
                "runner-context-menu",
                menu_cx.focus_handle(),
                position,
                items,
                Rc::new(move |index, window, cx| {
                    if let Some(action) = actions.get(index).cloned() {
                        action_root.update(cx, |this, cx| {
                            this.handle_runner_menu_action(action, window, cx)
                        });
                    }
                }),
                Rc::new(move |_, cx| {
                    dismiss_root.update(cx, |this, cx| {
                        this.runner_surfaces.context_menu = None;
                        cx.notify();
                    });
                }),
            )
            .width(px(176.))
        });
        let focus = menu.read(cx).focus_handle();
        self.runner_surfaces.context_menu = Some(menu);
        focus.focus(window);
        cx.notify();
    }

    fn handle_runner_menu_action(
        &mut self,
        action: RunnerMenuAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            RunnerMenuAction::Open(handle) => self.open_runner_detail(handle, window, cx),
            RunnerMenuAction::Delete { id, handle } => {
                self.runner_surfaces.delete_confirm = Some(RunnerDeleteConfirm { id, handle });
                cx.notify();
            }
        }
    }

    pub(super) fn start_runner_chat(
        &mut self,
        runner: Runner,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.runner_surfaces.chat_pending.is_some() {
            return;
        }
        let detail_origin = matches!(self.route, AppRoute::RunnerDetail(_));
        self.runner_surfaces.chat_pending = Some(runner.id.clone());
        let cwd = if runner.working_dir.is_none() {
            let default = self.settings(cx).default_working_dir.trim();
            (!default.is_empty()).then(|| default.to_owned())
        } else {
            None
        };
        let core = self.core(cx).clone();
        let runner_id = runner.id.clone();
        let task = cx.background_spawn(async move {
            runner_backend::ops::session::session_start_direct(
                &core,
                runner_id,
                None,
                None,
                None,
                None,
                cwd,
                Some(INITIAL_COLS),
                Some(INITIAL_ROWS),
            )
            .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, window, cx| {
                this.runner_surfaces.chat_pending = None;
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
                                this.remember_active_runner(cx);
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
                        if detail_origin && matches!(this.route, AppRoute::RunnerDetail(_)) =>
                    {
                        this.runner_surfaces.detail.error = Some(error);
                    }
                    Err(error) => this.show_toast(error, crate::toast::ToastTone::Error, cx),
                }
                cx.notify();
            });
        })
        .detach();
    }
}
