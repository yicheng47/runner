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
        cwd_focus.focus(window);
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
            prompt: Some("Pick a project directory".into()),
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
        match runner_backend::ops::project::project_create(self.core(cx), name, cwd) {
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
            self.set_route(AppRoute::Runners, cx);
            self.load_runner_page(cx);
            window.focus(&self.root_focus);
        }
        self.project_delete_busy = true;
        let core = self.core(cx).clone();
        let deleting_project_id = project_id.clone();
        cx.spawn(async move |weak, cx| {
            let result = runner_backend::ops::project::project_delete(&core, project_id).await;
            let _ = weak.update(cx, |this, cx| {
                this.project_delete_busy = false;
                if let Err(error) = result {
                    this.error = Some(error.to_string());
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
        if let Some(menu) = &self.sidebar.read(cx).context_menu {
            overlays.push(menu.clone().into_any_element());
        }
        if self.project_modal.is_some() {
            overlays.push(self.render_project_modal(cx));
        }
        if let Some(project_id) = &self.project_delete_confirm {
            if let Some(project) = self
                .app_store
                .read(cx)
                .projects
                .iter()
                .find(|project| project.id == *project_id)
                .cloned()
            {
                let confirm_root = cx.entity();
                let cancel_root = confirm_root.clone();
                overlays.push(
                    ConfirmDialog::new(
                        format!("Delete project \"{}\"?", project.name),
                        "Deleting this project archives every chat and mission inside it (running ones are stopped first). Archived items appear in Settings → Archived. The on-disk directory and all of its files remain untouched.",
                        "Delete project",
                        "Archiving…",
                        self.project_delete_busy,
                        Rc::new(move |window, cx| {
                            confirm_root.update(cx, |this, cx| {
                                this.confirm_delete_project(window, cx)
                            });
                        }),
                        Rc::new(move |_, cx| {
                            cancel_root.update(cx, |this, cx| {
                                if !this.project_delete_busy {
                                    this.project_delete_confirm = None;
                                    cx.notify();
                                }
                            });
                        }),
                    )
                    .into_any_element(),
                );
            }
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
                            .child("Start project"),
                    )
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .font_weight(FontWeight::NORMAL)
                            .text_color(theme::muted())
                            .child("Add a named working directory to the sidebar."),
                    ),
            )
            .child(
                IconButton::new("close-project-modal", "close.svg")
                    .focus_handle(modal.close_focus.clone())
                    .tooltip("Close start project")
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
            .children(modal.error.as_ref().map(|error| {
                div()
                    .rounded_sm()
                    .border_1()
                    .border_color(theme::with_alpha(theme::danger(), 0.4))
                    .bg(theme::with_alpha(theme::danger(), 0.1))
                    .px_3()
                    .py_2()
                    .text_size(theme::text_ui())
                    .text_color(theme::danger())
                    .child(error.clone())
            }))
            .child(
                Field::new(
                    "project-directory",
                    "Directory",
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
                Field::new("project-name", "Name", modal.name.clone())
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
                Button::new("cancel-project", "Cancel")
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
                        "Creating…"
                    } else {
                        "Create project"
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
