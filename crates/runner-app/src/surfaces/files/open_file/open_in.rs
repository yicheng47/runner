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
#[cfg(not(windows))]
pub(in super::super) fn reveal(path: PathBuf) -> Result<(), String> {
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

#[cfg(windows)]
#[derive(Debug, PartialEq, Eq)]
enum WindowsReveal {
    SelectItem(PathBuf),
    OpenRoot(PathBuf),
}

/// Reveals `path` in Windows File Explorer via GPUI's `SHOpenFolderAndSelectItems`
/// wrapper instead of spawning `explorer.exe /select,<path>`, which breaks on
/// spaces when Rust quotes the combined argument. Filesystem roots have no parent
/// for `SHOpenFolderAndSelectItems`, so open them directly instead.
#[cfg(windows)]
pub(crate) fn reveal_in_explorer(path: &Path, cx: &App) {
    match windows_reveal(path) {
        WindowsReveal::SelectItem(target) => cx.reveal_path(&target),
        WindowsReveal::OpenRoot(target) => cx.open_with_system(&target),
    }
}

#[cfg(windows)]
fn windows_reveal(path: &Path) -> WindowsReveal {
    let target = windows_reveal_path(path);
    if target.parent().is_none() {
        WindowsReveal::OpenRoot(target)
    } else {
        WindowsReveal::SelectItem(target)
    }
}

#[cfg(windows)]
fn unc_prefix(server: &std::ffi::OsStr, share: &std::ffi::OsStr) -> PathBuf {
    let mut unc = std::ffi::OsString::from(r"\\");
    unc.push(server);
    unc.push(r"\");
    unc.push(share);
    PathBuf::from(unc)
}

/// Converts a verbatim Windows path to its ordinary shell spelling when doing
/// so names the exact same target, and lets `Path::components` normalize slash
/// direction and trailing separators on ordinary paths.
#[cfg(windows)]
fn windows_reveal_path(path: &Path) -> PathBuf {
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Component, Prefix};

    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return path.components().collect();
    };
    let mut ordinary = match prefix.kind() {
        Prefix::VerbatimDisk(drive) => PathBuf::from(format!("{}:", char::from(drive))),
        Prefix::VerbatimUNC(server, share) if !server.is_empty() && !share.is_empty() => {
            unc_prefix(server, share)
        }
        Prefix::UNC(server, share) if !server.is_empty() && !share.is_empty() => {
            let mut normalized = unc_prefix(server, share);
            for component in components {
                normalized.push(component);
            }
            return normalized;
        }
        Prefix::VerbatimUNC(_, _) | Prefix::Verbatim(_) | Prefix::DeviceNS(_) => {
            return path.to_path_buf();
        }
        _ => return path.components().collect(),
    };
    for component in components {
        match component {
            Component::RootDir => ordinary.push(Component::RootDir),
            Component::Normal(name) => {
                if matches!(name.as_encoded_bytes().last(), Some(b'.' | b' ')) {
                    return path.to_path_buf();
                }
                ordinary.push(name);
            }
            Component::Prefix(_) | Component::CurDir | Component::ParentDir => {
                return path.to_path_buf();
            }
        }
    }
    // Keep verbatim spelling when Win32 path limits or normalization (e.g.
    // reserved DOS names) would resolve the ordinary spelling elsewhere.
    if ordinary.as_os_str().encode_wide().count() >= 260 {
        return path.to_path_buf();
    }
    if std::path::absolute(&ordinary).is_ok_and(|abs| abs == ordinary) {
        ordinary
    } else {
        path.to_path_buf()
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
            #[cfg(windows)]
            reveal_in_explorer(&path, cx);
            #[cfg(not(windows))]
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

#[cfg(all(test, windows))]
mod tests {
    use super::{windows_reveal, WindowsReveal};
    use std::path::{Path, PathBuf};

    #[test]
    fn windows_reveal_routes_roots_and_preserves_spaces_unicode() {
        for (input, expected) in [
            (
                r"C:\Work\Demo Project\output\ppt\Sample Report.pptx",
                WindowsReveal::SelectItem(PathBuf::from(
                    r"C:\Work\Demo Project\output\ppt\Sample Report.pptx",
                )),
            ),
            (
                r"C:\Work\Demo Project\输出 目录\示例 报告.pptx",
                WindowsReveal::SelectItem(PathBuf::from(
                    r"C:\Work\Demo Project\输出 目录\示例 报告.pptx",
                )),
            ),
            (
                "C:/Work/Demo Project/输出 目录/示例 报告.pptx",
                WindowsReveal::SelectItem(PathBuf::from(
                    r"C:\Work\Demo Project\输出 目录\示例 报告.pptx",
                )),
            ),
            (
                r"C:\Work\Demo Project\",
                WindowsReveal::SelectItem(PathBuf::from(r"C:\Work\Demo Project")),
            ),
            (
                r"C:\Work\Demo Project\\",
                WindowsReveal::SelectItem(PathBuf::from(r"C:\Work\Demo Project")),
            ),
            (r"C:\", WindowsReveal::OpenRoot(PathBuf::from(r"C:\"))),
            ("C:/", WindowsReveal::OpenRoot(PathBuf::from(r"C:\"))),
            (r"C:\\", WindowsReveal::OpenRoot(PathBuf::from(r"C:\"))),
            (r"\\?\C:\", WindowsReveal::OpenRoot(PathBuf::from(r"C:\"))),
            (
                r"\\?\C:\Work\Demo Project\示例 报告.pptx",
                WindowsReveal::SelectItem(PathBuf::from(r"C:\Work\Demo Project\示例 报告.pptx")),
            ),
            (
                r"\\?\UNC\server\share\Demo Project\示例 报告.pptx",
                WindowsReveal::SelectItem(PathBuf::from(
                    r"\\server\share\Demo Project\示例 报告.pptx",
                )),
            ),
            (
                r"\\?\UNC\server\share\Demo Project\",
                WindowsReveal::SelectItem(PathBuf::from(r"\\server\share\Demo Project")),
            ),
            (
                "//server/share/输出 目录/file.txt",
                WindowsReveal::SelectItem(PathBuf::from(r"\\server\share\输出 目录\file.txt")),
            ),
            (
                "//server/share/Demo Project/",
                WindowsReveal::SelectItem(PathBuf::from(r"\\server\share\Demo Project")),
            ),
            (
                "//server/share/",
                WindowsReveal::OpenRoot(PathBuf::from(r"\\server\share\")),
            ),
            (
                "//server/share",
                WindowsReveal::OpenRoot(PathBuf::from(r"\\server\share\")),
            ),
            (
                r"\\server\share\",
                WindowsReveal::OpenRoot(PathBuf::from(r"\\server\share\")),
            ),
            (
                r"\\server\share",
                WindowsReveal::OpenRoot(PathBuf::from(r"\\server\share\")),
            ),
            (
                r"\\?\UNC\server\share\",
                WindowsReveal::OpenRoot(PathBuf::from(r"\\server\share\")),
            ),
            (
                r"\\?\Volume{1234}\file.txt",
                WindowsReveal::SelectItem(PathBuf::from(r"\\?\Volume{1234}\file.txt")),
            ),
            (
                r"\\?\C:\work\NUL",
                WindowsReveal::SelectItem(PathBuf::from(r"\\?\C:\work\NUL")),
            ),
        ] {
            let actual = windows_reveal(Path::new(input));
            assert_eq!(actual, expected, "{input}");
            let (actual_path, expected_path) = match (&actual, &expected) {
                (WindowsReveal::SelectItem(a), WindowsReveal::SelectItem(e))
                | (WindowsReveal::OpenRoot(a), WindowsReveal::OpenRoot(e)) => (a, e),
                _ => unreachable!(),
            };
            assert_eq!(
                actual_path.as_os_str(),
                expected_path.as_os_str(),
                "{input}"
            );
        }
    }

    #[test]
    fn windows_reveal_preserves_verbatim_paths_that_ordinary_spelling_aliases() {
        let temp = tempfile::tempdir().unwrap();
        let verbatim = temp.path().canonicalize().unwrap();
        let ordinary_root = match windows_reveal(&verbatim) {
            WindowsReveal::SelectItem(path) => path,
            other => panic!("expected temp directory to select inside parent, got {other:?}"),
        };
        assert!(!ordinary_root.to_string_lossy().starts_with(r"\\?\"));

        let cjk_dir = verbatim.join("输出 目录");
        std::fs::create_dir_all(&cjk_dir).unwrap();
        let cjk_file = cjk_dir.join("示例 报告.pptx");
        std::fs::write(&cjk_file, b"demo").unwrap();
        assert_eq!(
            windows_reveal(&cjk_file),
            WindowsReveal::SelectItem(ordinary_root.join("输出 目录").join("示例 报告.pptx"))
        );

        let plain_dir = verbatim.join("work");
        let dotted_dir = verbatim.join("work.");
        let dotted_nested = dotted_dir.join("nested.txt");
        let dotted_file = verbatim.join("report.");
        let spaced_file = verbatim.join("report ");
        std::fs::create_dir_all(&plain_dir).unwrap();
        std::fs::create_dir_all(&dotted_dir).unwrap();
        std::fs::write(&dotted_nested, b"nested").unwrap();
        std::fs::write(&dotted_file, b"dot").unwrap();
        std::fs::write(&spaced_file, b"space").unwrap();

        assert_eq!(
            windows_reveal(&plain_dir),
            WindowsReveal::SelectItem(ordinary_root.join("work"))
        );
        for aliased in [&dotted_dir, &dotted_nested, &dotted_file, &spaced_file] {
            assert_eq!(
                windows_reveal(aliased),
                WindowsReveal::SelectItem(aliased.clone()),
                "{}",
                aliased.display()
            );
        }

        let drive_root = temp.path().ancestors().last().unwrap();
        assert!(matches!(
            windows_reveal(drive_root),
            WindowsReveal::OpenRoot(_)
        ));

        let _ = std::fs::remove_file(&dotted_nested);
        let _ = std::fs::remove_dir(&dotted_dir);
        let _ = std::fs::remove_file(&dotted_file);
        let _ = std::fs::remove_file(&spaced_file);
    }
}
