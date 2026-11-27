//! Open in, line links and reveal in the platform file manager.

use super::*;

pub(in super::super) fn reveal_label() -> &'static str {
    crate::i18n::text(if cfg!(target_os = "macos") {
        "ui.reveal_in_finder"
    } else if cfg!(windows) {
        "ui.show_in_explorer"
    } else {
        "ui.open_containing_folder"
    })
}

/// Reveals `path` in the file manager; on Linux, which has no common call
/// that selects a file, opens the folder holding it.
pub(in super::super) fn reveal(path: PathBuf) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut select = std::ffi::OsString::from("/select,");
        select.push(&path);
        std::process::Command::new("explorer.exe")
            .arg(select)
            .spawn()
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
    #[cfg(not(windows))]
    {
        #[cfg(target_os = "macos")]
        let status = std::process::Command::new("open")
            .arg("-R")
            .arg(&path)
            .status();
        #[cfg(target_os = "linux")]
        let status = std::process::Command::new("xdg-open")
            .arg(path.parent().unwrap_or(&path))
            .status();
        match status {
            Ok(status) if status.success() => Ok(()),
            Ok(status) => Err(format!("exited with {status}")),
            Err(error) => Err(error.to_string()),
        }
    }
}

type RowAction = Box<dyn Fn(&mut UnifiedViewer, &mut Context<UnifiedViewer>)>;

const OPEN_IN_WIDTH: f32 = 260.;

/// One editor row shared by Settings → General → Editor and the viewer's
/// Open in menu.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct EditorMenuEntry {
    pub(crate) editor: FileLinkEditor,
    pub(crate) available: bool,
    pub(crate) selected: bool,
    pub(crate) missing_command: Option<&'static str>,
}

/// The editors whose command-line launcher is on PATH, then whether the
/// chosen one is among them.
pub(crate) fn found_editors(cx: &App) -> (Vec<FileLinkEditor>, FileLinkEditor) {
    let store = crate::app_store::global_app_store(cx).read(cx);
    let found = FileLinkEditor::MENU_ORDER
        .into_iter()
        .filter(|editor| {
            editor.cli().is_some_and(|command| {
                store
                    .file_link_environment
                    .1
                    .get(command)
                    .copied()
                    .unwrap_or(false)
            })
        })
        .collect::<Vec<_>>();
    let chosen = store.settings.file_link_editor;
    let effective = if found.contains(&chosen) {
        chosen
    } else {
        FileLinkEditor::DefaultApp
    };
    (found, effective)
}

/// Shared editor menu entries in display order: Zed, VS Code, Cursor (with
/// missing CLI commands disabled and annotated), then Default app.
pub(crate) fn editor_menu_entries(cx: &App) -> Vec<EditorMenuEntry> {
    let (found, effective) = found_editors(cx);
    FileLinkEditor::MENU_ORDER
        .into_iter()
        .map(|editor| {
            let available = editor == FileLinkEditor::DefaultApp || found.contains(&editor);
            EditorMenuEntry {
                editor,
                available,
                selected: editor == effective,
                missing_command: (!available).then(|| editor.cli()).flatten(),
            }
        })
        .collect()
}

/// `StyledSelect` options for Settings → General → Editor, backed by the same
/// editor list and icons as the viewer's Open in menu.
pub(crate) fn file_link_editor_options(cx: &App) -> Vec<runner_app::ui::SelectOption> {
    use runner_app::ui::{SelectLeading, SelectOption};
    editor_menu_entries(cx)
        .into_iter()
        .map(|entry| {
            let editor = entry.editor;
            let label = match editor {
                FileLinkEditor::DefaultApp => crate::i18n::message("ui.default_app"),
                _ => editor.label().into(),
            };
            let mut option = SelectOption::new(editor.key(), label)
                .leading(SelectLeading::new(
                    format!("file-link-editor-{}", editor.key()),
                    move |size| editor_icon(editor, size),
                ))
                .disabled(!entry.available)
                .separator_before(editor == FileLinkEditor::DefaultApp);
            if let Some(command) = entry.missing_command {
                option = option.trailing_hint(crate::i18n::messagef(
                    "format.not_on_path",
                    &[("command", command.to_owned())],
                ));
            }
            option
        })
        .collect()
}

pub(crate) fn editor_icon(editor: FileLinkEditor, size: f32) -> AnyElement {
    let png = match editor {
        FileLinkEditor::Zed => Some("open-in/zed.png"),
        FileLinkEditor::VsCode => Some("open-in/vscode.png"),
        FileLinkEditor::Cursor => Some("open-in/cursor.png"),
        FileLinkEditor::DefaultApp => None,
    };
    match png {
        Some(path) => img(ImageSource::Resource(Resource::Embedded(path.into())))
            .flex_none()
            .size(px(size))
            .rounded_sm()
            .into_any_element(),
        None => svg()
            .flex_none()
            .size(px(size))
            .path("app-window.svg")
            .text_color(theme::muted())
            .into_any_element(),
    }
}

fn reveal_icon(size: f32) -> AnyElement {
    if cfg!(target_os = "macos") {
        img(ImageSource::Resource(Resource::Embedded(
            "open-in/finder.png".into(),
        )))
        .flex_none()
        .size(px(size))
        .into_any_element()
    } else {
        svg()
            .flex_none()
            .size(px(size))
            .path("folder-open.svg")
            .text_color(theme::muted())
            .into_any_element()
    }
}

impl UnifiedViewer {
    pub(super) fn active_path(&self) -> Option<PathBuf> {
        let tab = self.tabs.active()?;
        if let Some(outside) = tab.key.outside_path() {
            return Some(outside.to_path_buf());
        }
        let root = self.root_path()?;
        Some(absolute(root, tab.path()))
    }

    pub(super) fn open_line_handler(
        &self,
        key: TabKey,
        cx: &mut Context<Self>,
    ) -> super::super::code::OpenLine {
        let root = cx.weak_entity();
        Rc::new(move |line, _, cx| {
            let _ = root.update(cx, |this, cx| this.open_at_line(&key, line, cx));
        })
    }

    /// ⌘-click on a line: the file at that line in the chosen editor,
    /// unless it was deleted.
    fn open_at_line(&mut self, key: &TabKey, line: u32, cx: &mut Context<Self>) {
        let deleted = self
            .docs
            .get(key)
            .is_none_or(|doc| doc.is_working_tree_deleted(key));
        if deleted {
            return;
        }
        let Some(path) = (match key.outside_path() {
            Some(outside) => Some(outside.to_path_buf()),
            None => self.root_path().map(|root| absolute(root, &key.path)),
        }) else {
            return;
        };
        self.launch(None, path, Some(line), cx);
    }

    /// Opens `path` outside the app: in `editor`, or the chosen one.
    pub(super) fn launch(
        &mut self,
        editor: Option<FileLinkEditor>,
        path: PathBuf,
        line: Option<u32>,
        cx: &mut Context<Self>,
    ) {
        #[cfg(test)]
        {
            let _ = cx;
            self.launched.push((editor, path, line));
        }
        #[cfg(not(test))]
        {
            let target = FileLinkTarget {
                path,
                line,
                column: None,
            };
            match editor {
                Some(editor) => file_links::open_file_with(editor, target, cx),
                None => file_links::open_file_in_editor(target, cx),
            }
        }
    }

    /// Opens the active file in `editor`, at the line in view; picking an
    /// editor from the menu makes it the editor for Open in and for
    /// ⌘-clicked links the preview does not take.
    pub(super) fn open_in(&mut self, editor: Option<FileLinkEditor>, cx: &mut Context<Self>) {
        self.open_in_open = false;
        cx.notify();
        if self.active_deleted() {
            return;
        }
        let Some(path) = self.active_path() else {
            return;
        };
        let line = self
            .active_view()
            .map(|view| view.read(cx).line_in_view())
            .or_else(|| {
                self.active_diff_view()
                    .and_then(|diff| diff.read(cx).current_new_line())
            });
        if let Some(editor) = editor {
            crate::app_store::global_app_store(cx).update(cx, |store, cx| {
                store.update_settings(
                    |settings| {
                        let changed = settings.file_link_editor != editor;
                        settings.file_link_editor = editor;
                        changed
                    },
                    true,
                    cx,
                );
            });
        }
        let editor = editor.unwrap_or_else(|| found_editors(cx).1);
        self.launch(Some(editor), path, line, cx);
    }

    fn reveal_active(&mut self, cx: &mut Context<Self>) {
        self.open_in_open = false;
        if let Some(path) = self.active_path().filter(|_| !self.active_deleted()) {
            cx.background_spawn(async move {
                if let Err(error) = reveal(path) {
                    tracing::warn!(target: "runner_app::surfaces::files::viewer", "reveal failed: {error}");
                }
            })
            .detach();
        }
        cx.notify();
    }

    pub(super) fn render_open_in_control(
        &self,
        deleted: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let open_in_anchor = self.open_in_anchor.clone();
        let (_, effective) = found_editors(cx);
        let open_in_label = crate::i18n::tf(
            "format.open_in_editor",
            &[("editor", effective.label().to_owned())],
        );
        div()
            .relative()
            .flex_none()
            .flex()
            .items_center()
            .rounded_md()
            .border_1()
            .border_color(theme::border())
            .bg(theme::bg())
            .when(deleted, |element| element.opacity(0.4))
            .child(
                canvas(
                    move |bounds, _, _| open_in_anchor.set(bounds),
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .child(
                div()
                    .id("file-open-in")
                    .when(cfg!(test), |element| {
                        element.debug_selector(|| "FILE_OPEN_IN".into())
                    })
                    .h(px(22.))
                    .px(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(4.))
                    .rounded_l_md()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .when(!deleted, |element| {
                        element
                            .cursor_pointer()
                            .hover(|element| {
                                element.bg(theme::with_alpha(theme::sidebar_selected(), 0.6))
                            })
                            .tooltip(move |_, cx| {
                                runner_app::ui::tooltip::tooltip_view(open_in_label.clone(), cx)
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.open_in(None, cx)))
                    })
                    .child(editor_icon(effective, 13.)),
            )
            .child(div().w(px(1.)).h(px(14.)).bg(theme::border()))
            .child(
                div()
                    .id("file-open-in-menu")
                    .when(cfg!(test), |element| {
                        element.debug_selector(|| "FILE_OPEN_IN_MENU".into())
                    })
                    .h(px(22.))
                    .w(px(18.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_r_md()
                    .when(self.open_in_open, |element| {
                        element.bg(theme::sidebar_selected())
                    })
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .when(!deleted, |element| {
                        element
                            .cursor_pointer()
                            .hover(|element| {
                                element.bg(theme::with_alpha(theme::sidebar_selected(), 0.6))
                            })
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.open_in_open = !this.open_in_open;
                                this.list_open = false;
                                cx.notify();
                            }))
                    })
                    .child(
                        svg()
                            .size(px(11.))
                            .path("chevron-down.svg")
                            .text_color(theme::muted()),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn render_open_in_menu(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.open_in_open || self.active_deleted() {
            return None;
        }
        let name = self.tabs.active()?.name().to_owned();
        let entries = editor_menu_entries(cx);
        let row = |id: (&'static str, usize),
                   icon: AnyElement,
                   label: String,
                   hint: Option<String>,
                   checked: bool,
                   disabled: bool,
                   cx: &mut Context<Self>,
                   on_click: Option<RowAction>| {
            div()
                .id(id)
                .when(cfg!(test), |element| {
                    element.debug_selector(move || format!("FILE_OPEN_IN_ITEM {} {}", id.0, id.1))
                })
                .px(px(8.))
                .py(px(6.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded_md()
                .opacity(if disabled { 0.5 } else { 1. })
                .when(!disabled, |element| {
                    element
                        .cursor_pointer()
                        .hover(|element| element.bg(theme::sidebar_selected()))
                        .when_some(on_click, |element, on_click| {
                            element.on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)))
                        })
                })
                .child(icon)
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .text_size(theme::text_ui())
                        .text_color(if disabled {
                            theme::faint()
                        } else {
                            theme::text()
                        })
                        .child(label),
                )
                .children(hint.map(|hint| {
                    div()
                        .flex_none()
                        .text_size(theme::text_meta())
                        .text_color(theme::faint())
                        .child(hint)
                }))
                .when(checked, |element| {
                    element.child(
                        svg()
                            .flex_none()
                            .size(px(13.))
                            .path("check.svg")
                            .text_color(theme::text()),
                    )
                })
                .into_any_element()
        };
        let mut rows = Vec::new();
        let mut editor_index = 0usize;
        for entry in entries {
            let editor = entry.editor;
            if editor == FileLinkEditor::DefaultApp {
                rows.push(
                    div()
                        .my(px(4.))
                        .h(px(1.))
                        .bg(theme::border())
                        .into_any_element(),
                );
                rows.push(row(
                    ("file-open-in-default", 0),
                    editor_icon(FileLinkEditor::DefaultApp, 16.),
                    crate::i18n::text("ui.default_app").to_owned(),
                    None,
                    entry.selected,
                    false,
                    cx,
                    Some(Box::new(|this, cx| {
                        this.open_in(Some(FileLinkEditor::DefaultApp), cx)
                    })),
                ));
                continue;
            }
            let hint = entry.missing_command.map(|command| {
                crate::i18n::tf("format.not_on_path", &[("command", command.to_owned())])
            });
            let idx = editor_index;
            editor_index += 1;
            rows.push(row(
                ("file-open-in-editor", idx),
                editor_icon(editor, 16.),
                editor.label().to_owned(),
                hint,
                entry.selected,
                !entry.available,
                cx,
                entry.available.then(|| {
                    Box::new(
                        move |this: &mut UnifiedViewer, cx: &mut Context<UnifiedViewer>| {
                            this.open_in(Some(editor), cx)
                        },
                    ) as RowAction
                }),
            ));
        }
        rows.push(row(
            ("file-open-in-reveal", 0),
            reveal_icon(16.),
            reveal_label().to_owned(),
            None,
            false,
            false,
            cx,
            Some(Box::new(|this, cx| this.reveal_active(cx))),
        ));
        let content = div()
            .when(cfg!(test), |element| {
                element.debug_selector(|| "FILE_OPEN_IN_POPUP".into())
            })
            .p(px(6.))
            .flex()
            .flex_col()
            .rounded(rems(8. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::raised())
            .shadow_2xl()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(
                div()
                    .px(px(8.))
                    .pt(px(2.))
                    .pb(px(4.))
                    .text_size(theme::text_meta())
                    .text_color(theme::faint())
                    .truncate()
                    .child(crate::i18n::tf("format.open_file_in", &[("name", name)])),
            )
            .children(rows)
            .into_any_element();
        let root = cx.entity();
        Some(popup_layer_right_aligned(
            self.open_in_anchor.get(),
            window,
            px(OPEN_IN_WIDTH),
            content,
            Rc::new(move |_, cx| {
                root.update(cx, |this, cx| {
                    this.open_in_open = false;
                    cx.notify();
                })
            }),
        ))
    }
}
