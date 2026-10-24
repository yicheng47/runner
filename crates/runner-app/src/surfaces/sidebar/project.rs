use super::elements::project_name_from_path;

use super::*;
use crate::surfaces::*;
use crate::*;
use gpui::{FontWeight, PathPromptOptions};
use runner_app::ui::{
    working_dir_text_field, ButtonVariant, ConfirmDialog, Field, Modal, OverlayWidth, TextField,
    WorkingDirField,
};

impl NativeRoot {
    pub(super) fn open_project_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.remote_window.is_some() {
            self.open_remote_project_picker(window, cx);
            return;
        }

        let cwd = self.settings(cx).default_working_dir.trim().to_owned();
        let name = project_name_from_path(&cwd);
        let cwd_input = cx.new(|input_cx| {
            working_dir_text_field(input_cx.focus_handle(), cwd, "/Users/you/projects/runner")
                .text_size(theme::text_ui())
        });
        let name_input = cx.new(|input_cx| {
            TextField::new(input_cx.focus_handle(), name, "runner", false)
                .text_size(theme::text_body())
        });
        let watched_name = name_input.clone();
        self._project_cwd_subscription =
            Some(cx.observe(&cwd_input, move |this, cwd_input, cx| {
                let Some(modal) = this.project_modal.as_ref() else {
                    return;
                };
                if modal.name != watched_name || modal.name.read(cx).edited() {
                    return;
                }
                let derived = project_name_from_path(cwd_input.read(cx).text());
                watched_name.update(cx, |name, name_cx| name.reset(derived, name_cx));
            }));
        let cwd_focus = cwd_input.read(cx).focus_handle();
        self.project_modal = Some(ProjectModal {
            cwd: cwd_input,
            name: name_input,
            browse_focus: cx.focus_handle(),
            close_focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            submit_focus: cx.focus_handle(),
            error: None,
            submitting: false,
        });
        self.update_app_settings(cx, true, |settings| {
            settings.sidebar_projects_open = true;
            true
        });
        cwd_focus.focus(window, cx);
        cx.notify();
    }

    fn browse_project_cwd(&mut self, cx: &mut Context<Self>) {
        let Some(cwd_input) = self.project_modal.as_ref().map(|modal| modal.cwd.clone()) else {
            return;
        };
        let selected = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(crate::i18n::text("ui.pick_a_project_directory").into()),
        });
        cx.spawn(async move |weak, cx| {
            let result = selected
                .await
                .map_err(|error| error.to_string())
                .and_then(|result| result.map_err(|error| error.to_string()));
            let _ = weak.update(cx, |this, cx| {
                let Some(modal) = this.project_modal.as_mut() else {
                    return;
                };
                if modal.cwd != cwd_input {
                    return;
                }
                match result {
                    Ok(Some(paths)) => {
                        if let Some(path) = paths.into_iter().next() {
                            modal.cwd.update(cx, |field, field_cx| {
                                field.reset(path.to_string_lossy().into_owned(), field_cx)
                            });
                        }
                    }
                    Ok(None) => {}
                    Err(error) => modal.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn close_project_modal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self
            .project_modal
            .as_ref()
            .is_some_and(|modal| modal.submitting)
        {
            return;
        }
        self.project_modal = None;
        self._project_cwd_subscription = None;
        self.focus_active_terminal(window, cx);
        cx.notify();
    }

    fn submit_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(modal) = self.project_modal.as_mut() else {
            return;
        };
        let cwd = modal.cwd.read(cx).text().trim().to_owned();
        let name = modal.name.read(cx).text().trim().to_owned();
        if cwd.is_empty() || name.is_empty() || modal.submitting {
            return;
        }
        modal.submitting = true;
        modal.error = None;
        match self.core(cx).project_create(name, cwd) {
            Ok(project) => {
                self.project_modal = None;
                self._project_cwd_subscription = None;
                self.refresh_store(StoreRefreshKind::All, cx);
                self.sidebar.update(cx, |sidebar, sidebar_cx| {
                    sidebar.set_active_project(Some(project.id), sidebar_cx)
                });
                self.focus_active_terminal(window, cx);
            }
            Err(error) => {
                if let Some(modal) = self.project_modal.as_mut() {
                    modal.submitting = false;
                    modal.error = Some(error.to_string());
                }
            }
        }
        cx.notify();
    }

    fn on_project_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key == "enter"
            && self.project_modal.as_ref().is_some_and(|modal| {
                !modal.cwd.read(cx).is_composing() && !modal.name.read(cx).is_composing()
            })
        {
            cx.stop_propagation();
            self.submit_project(window, cx);
        }
    }

    fn confirm_delete_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.remote_window.is_some() {
            self.delete_remote_project(cx);
            return;
        }

        let Some(project_id) = self.project_delete_confirm.clone() else {
            return;
        };
        let deleting_active_chat = self
            .tabs
            .active_tab_id()
            .and_then(|tab_id| {
                self.app_store
                    .read(cx)
                    .nodes
                    .iter()
                    .find(|node| node.id == tab_id)
            })
            .and_then(|node| node_project_id(&self.app_store.read(cx).nodes, node))
            .as_deref()
            == Some(project_id.as_str());
        if deleting_active_chat {
            self.set_route(AppRoute::Roles, cx);
            self.load_role_page(cx);
            window.focus(&self.root_focus, cx);
        }
        self.project_delete_busy = true;
        let core = self.core(cx).clone();
        let deleting_project_id = project_id.clone();
        cx.spawn(async move |weak, cx| {
            let result = core.project_delete(project_id);
            let _ = weak.update(cx, |this, cx| {
                this.project_delete_busy = false;
                if let Err(error) = result {
                    this.error = Some(error.to_string().into());
                } else {
                    this.project_delete_confirm = None;
                    this.sidebar.update(cx, |sidebar, sidebar_cx| {
                        if sidebar.active_project_id.as_deref()
                            == Some(deleting_project_id.as_str())
                        {
                            sidebar.set_active_project(None, sidebar_cx);
                        }
                    });
                }
                this.refresh_store(StoreRefreshKind::All, cx);
            });
        })
        .detach();
    }
}

impl NativeRoot {
    pub(crate) fn render_sidebar_overlays(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut overlays = Vec::new();
        if let Some(menu) = &self.front_page_sidebar().read(cx).context_menu {
            overlays.push(menu.clone().into_any_element());
        }
        if self.project_modal.is_some() {
            overlays.push(self.render_project_modal(cx));
        }
        if let Some(picker) = self
            .remote_window
            .as_ref()
            .and_then(|remote| remote.project_picker.clone())
        {
            overlays.push(picker.into_any_element());
        }
        let deletion = if let Some(remote) = &self.remote_window {
            remote.delete_project.as_ref().and_then(|id| {
                self.app_store
                    .read(cx)
                    .remote_clients
                    .read(cx)
                    .overviews
                    .get(&remote.computer.id)?
                    .projects
                    .iter()
                    .find(|project| &project.id == id)
                    .map(|project| (project.name.clone(), remote.delete_project_busy))
            })
        } else {
            self.project_delete_confirm.as_ref().and_then(|id| {
                self.app_store
                    .read(cx)
                    .projects
                    .iter()
                    .find(|project| &project.id == id)
                    .map(|project| (project.name.clone(), self.project_delete_busy))
            })
        };
        if let Some((project_name, busy)) = deletion {
            let confirm_root = cx.entity();
            let cancel_root = confirm_root.clone();
            overlays.push(
                    ConfirmDialog::new(
                        crate::i18n::tf("format.delete_project", &[("project", project_name.to_string())]),
                        self.remote_window.as_ref().and_then(|remote| remote.delete_project_error.as_ref()).map(|error| format!("{}\n{}", crate::i18n::text("ui.deleting_this_project_archives_every_chat_and_mission_inside_it_running_ones_are_stopped_f"), error)).unwrap_or_else(|| crate::i18n::text("ui.deleting_this_project_archives_every_chat_and_mission_inside_it_running_ones_are_stopped_f").to_owned()),
                        crate::i18n::text("ui.delete_project"),
                        crate::i18n::text("ui.archiving"),
                        busy,
                        Rc::new(move |window, cx| {
                            confirm_root.update(cx, |this, cx| {
                                this.confirm_delete_project(window, cx)
                            });
                        }),
                        Rc::new(move |_, cx| {
                            cancel_root.update(cx, |this, cx| {
                                if let Some(remote) = this.remote_window.as_mut() { if !remote.delete_project_busy { remote.delete_project = None; cx.notify(); } }
                                else if !this.project_delete_busy { this.project_delete_confirm = None; cx.notify(); }
                            });
                        }),
                    )
                    .into_any_element(),
                );
        }
        overlays
    }

    fn render_project_modal(&self, cx: &mut Context<Self>) -> AnyElement {
        let modal = self.project_modal.as_ref().expect("project modal");
        let submitting = modal.submitting;
        let can_create = !modal.cwd.read(cx).text().trim().is_empty()
            && !modal.name.read(cx).text().trim().is_empty()
            && !submitting;
        let root = cx.entity();
        let close_root = root.clone();
        let cancel_root = root.clone();
        let browse_root = root.clone();
        let submit_root = root.clone();
        let modal_close_root = root;
        let title = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_4()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(rems(2. / 16.))
                    .child(
                        div()
                            .text_size(theme::text_heading())
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(crate::i18n::text("ui.start_project")),
                    )
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::muted())
                            .child(crate::i18n::text(
                                "ui.add_a_named_working_directory_to_the_sidebar",
                            )),
                    ),
            )
            .child(
                IconButton::new("close-project-modal", "close.svg")
                    .focus_handle(modal.close_focus.clone())
                    .tooltip(crate::i18n::text("ui.close_start_project"))
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        close_root.update(cx, |this, cx| this.close_project_modal(window, cx));
                    }),
            );
        let mut body = div()
            .flex()
            .flex_col()
            .gap_5()
            .on_key_down(cx.listener(Self::on_project_key_down))
            .children(
                modal
                    .error
                    .as_ref()
                    .filter(|_| !self.app_store.read(cx).daemon_disconnected)
                    .map(|error| {
                        runner_app::ui::notice_banner(error.clone(), runner_app::ui::Tone::Danger)
                    }),
            )
            .child(
                Field::new(
                    "project-directory",
                    crate::i18n::text("ui.directory"),
                    WorkingDirField::new(
                        modal.cwd.clone(),
                        submitting,
                        Rc::new(move |_, cx| {
                            browse_root.update(cx, |this, cx| this.browse_project_cwd(cx));
                        }),
                    )
                    .browse_focus(modal.browse_focus.clone()),
                )
                .focus_target(modal.cwd.read(cx).focus_handle())
                .emphasized(true),
            )
            .child(
                Field::new(
                    "project-name",
                    crate::i18n::text("ui.name"),
                    modal.name.clone(),
                )
                .focus_target(modal.name.read(cx).focus_handle())
                .emphasized(true),
            );
        if submitting {
            body = body.opacity(0.7);
        }
        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("cancel-project", crate::i18n::text("ui.cancel"))
                    .focus_handle(modal.cancel_focus.clone())
                    .disabled(submitting)
                    .on_press(move |window, cx| {
                        cancel_root.update(cx, |this, cx| this.close_project_modal(window, cx));
                    }),
            )
            .child(
                Button::new(
                    "create-project",
                    if submitting {
                        crate::i18n::text("ui.creating")
                    } else {
                        crate::i18n::text("ui.create_project")
                    },
                )
                .focus_handle(modal.submit_focus.clone())
                .variant(ButtonVariant::Primary)
                .disabled(!can_create)
                .on_press(move |window, cx| {
                    submit_root.update(cx, |this, cx| this.submit_project(window, cx));
                }),
            );
        Modal::new(
            title,
            body,
            Rc::new(move |window, cx| {
                modal_close_root.update(cx, |this, cx| this.close_project_modal(window, cx));
            }),
        )
        .width(OverlayWidth::Custom(560.))
        .busy(submitting)
        .focus_order(if submitting {
            Vec::new()
        } else {
            vec![
                modal.cwd.read(cx).focus_handle(),
                modal.browse_focus.clone(),
                modal.name.read(cx).focus_handle(),
                modal.cancel_focus.clone(),
                modal.submit_focus.clone(),
                modal.close_focus.clone(),
            ]
        })
        .footer(footer)
        .into_any_element()
    }
}

impl NativeRoot {
    fn open_remote_project_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let client = self.front_page_client(cx);
        if !client.supports(&["folders", "createProject"]) {
            return;
        }
        let Some(anchor) = self.front_page_sidebar().read(cx).project_anchor else {
            return;
        };
        let remote = self.remote_window.as_ref().unwrap();
        let name = remote.computer.name.clone();
        let platform = remote.computer.info.platform.clone();
        let input = cx.new(|cx| TextField::new(cx.focus_handle(), "", "", false));
        let picker = cx.new(|cx| {
            crate::surfaces::front_page::folder_picker::FolderPicker::new(
                client.clone(),
                name,
                platform,
                input.clone(),
                Vec::new(),
                cx,
            )
        });
        let epoch = self.client_ui.epoch;
        let subscription = cx.observe(&input, move |root, input, cx| {
            if root.client_ui.epoch != epoch
                || !client.valid()
                || root.front_page_client(cx).target != client.target
            {
                return;
            }
            let path = input.read(cx).text().to_owned();
            if path.is_empty() {
                return;
            }
            let name = path
                .rsplit(['/', '\\'])
                .find(|part| !part.is_empty())
                .unwrap_or(&path)
                .to_owned();
            if let Some(remote) = root.remote_window.as_mut() {
                remote.project_picker = None;
                remote.project_picker_subscription = None;
            }
            root.remote_action(
                "createProject".into(),
                &["createProject"],
                move |client| {
                    client.create_project(name, path)?;
                    Ok(Default::default())
                },
                cx,
            );
        });
        let remote = self.remote_window.as_mut().unwrap();
        remote.project_picker = Some(picker.clone());
        remote.project_picker_subscription = Some(subscription);
        let popup_width = px(320. * self.settings(cx).app_zoom);
        picker.update(cx, |picker, cx| {
            picker.set_anchor(anchor, cx);
            picker.set_popup_width(popup_width);
            picker.return_focus(window.focused(cx));
            picker.browse(window, cx);
        });
        cx.notify();
    }

    fn delete_remote_project(&mut self, cx: &mut Context<Self>) {
        let client = self.front_page_client(cx);
        if !client.supports(&["deleteProject"]) {
            return;
        }
        let Some(remote) = self.remote_window.as_mut() else {
            return;
        };
        let Some(project) = remote.delete_project.clone() else {
            return;
        };
        if remote.delete_project_busy {
            return;
        }
        let force = remote.delete_project_force;
        remote.delete_project_busy = true;
        remote.delete_project_error = None;
        let epoch = self.client_ui.epoch;
        let guard = client.clone();
        let id = project.clone();
        let work = cx.background_spawn(async move { client.delete_project(project, force) });
        cx.spawn(async move |weak, cx| {
            let result = work.await;
            let _ = weak.update(cx, |root, cx| {
                if root.client_ui.epoch != epoch || !guard.valid() {
                    return;
                }
                let Some(remote) = root.remote_window.as_mut().filter(|remote| {
                    remote.front_page.target == guard.target
                        && remote.delete_project.as_ref() == Some(&id)
                }) else {
                    return;
                };
                remote.delete_project_busy = false;
                match result {
                    Ok(()) => remote.delete_project = None,
                    Err(error) => {
                        remote.delete_project_force =
                            error.message.to_ascii_lowercase().contains("running");
                        remote.delete_project_error = Some(error.to_string());
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}
