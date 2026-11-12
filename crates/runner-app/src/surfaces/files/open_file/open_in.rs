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

type RowAction = Box<dyn Fn(&mut FilesView, &mut Context<FilesView>)>;

const OPEN_IN_WIDTH: f32 = 224.;

/// The editors whose command-line launcher is on PATH, then whether the
/// chosen one is among them.
pub(super) fn found_editors(cx: &App) -> (Vec<FileLinkEditor>, FileLinkEditor) {
    let store = crate::app_store::global_app_store(cx).read(cx);
    let found = FileLinkEditor::ALL
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

pub(super) fn editor_icon(editor: FileLinkEditor, size: f32) -> AnyElement {
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

impl FilesView {
    pub(super) fn active_path(&self) -> Option<PathBuf> {
        let root = self.root_path()?;
        let tab = self.viewer.tabs.active()?;
        Some(absolute(root, &tab.path))
    }

    pub(super) fn open_line_handler(
        &self,
        relative: String,
        cx: &mut Context<Self>,
    ) -> super::super::code::OpenLine {
        let root = cx.weak_entity();
        Rc::new(move |line, _, cx| {
            let _ = root.update(cx, |this, cx| this.open_at_line(&relative, line, cx));
        })
    }

    /// ⌘-click on a line: the file at that line in the chosen editor,
    /// unless it was deleted.
    fn open_at_line(&mut self, relative: &str, line: u32, cx: &mut Context<Self>) {
        let deleted = matches!(
            self.viewer.files.get(relative).map(|file| &file.content),
            None | Some(Content::Deleted(_))
        );
        let Some(root) = self.root_path().filter(|_| !deleted) else {
            return;
        };
        let path = absolute(root, relative);
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
            self.viewer.launched.push((editor, path, line));
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
                None => file_links::open_file_link(target, cx),
            }
        }
    }

    /// Opens the active file in `editor`, at the line in view; picking an
    /// editor from the menu makes it the editor for Open in and for
    /// ⌘-clicked links the preview does not take.
    pub(super) fn open_in(&mut self, editor: Option<FileLinkEditor>, cx: &mut Context<Self>) {
        self.viewer.open_in_open = false;
        cx.notify();
        if self.active_deleted() {
            return;
        }
        let Some(path) = self.active_path() else {
            return;
        };
        let line = self.active_view().map(|view| view.read(cx).line_in_view());
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
        self.viewer.open_in_open = false;
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

    pub(super) fn render_open_in_menu(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        if !self.viewer.open_in_open || self.active_deleted() {
            return None;
        }
        let name = self.viewer.tabs.active()?.name().to_owned();
        let (found, effective) = found_editors(cx);
        let row = |id: (&'static str, usize),
                   icon: AnyElement,
                   label: String,
                   checked: bool,
                   cx: &mut Context<Self>,
                   on_click: RowAction| {
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
                .cursor_pointer()
                .hover(|element| element.bg(theme::sidebar_selected()))
                .on_click(cx.listener(move |this, _, _, cx| on_click(this, cx)))
                .child(icon)
                .child(
                    div()
                        .flex_1()
                        .text_size(theme::text_ui())
                        .text_color(theme::text())
                        .child(label),
                )
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
        let mut rows = found
            .iter()
            .enumerate()
            .map(|(index, editor)| {
                let editor = *editor;
                row(
                    ("file-open-in-editor", index),
                    editor_icon(editor, 16.),
                    editor.label().to_owned(),
                    editor == effective,
                    cx,
                    Box::new(move |this, cx| this.open_in(Some(editor), cx)),
                )
            })
            .collect::<Vec<_>>();
        if !rows.is_empty() {
            rows.push(
                div()
                    .my(px(4.))
                    .h(px(1.))
                    .bg(theme::border())
                    .into_any_element(),
            );
        }
        rows.push(row(
            ("file-open-in-default", 0),
            editor_icon(FileLinkEditor::DefaultApp, 16.),
            crate::i18n::text("ui.default_app").to_owned(),
            effective == FileLinkEditor::DefaultApp,
            cx,
            Box::new(|this, cx| this.open_in(Some(FileLinkEditor::DefaultApp), cx)),
        ));
        rows.push(row(
            ("file-open-in-reveal", 0),
            reveal_icon(16.),
            reveal_label().to_owned(),
            false,
            cx,
            Box::new(|this, cx| this.reveal_active(cx)),
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
            self.viewer.open_in_anchor.get(),
            window,
            px(OPEN_IN_WIDTH),
            content,
            Rc::new(move |_, cx| {
                root.update(cx, |this, cx| {
                    this.viewer.open_in_open = false;
                    cx.notify();
                })
            }),
        ))
    }
}
