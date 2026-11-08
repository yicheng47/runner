use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;

use gpui::{App, AppContext as _, Context, Entity, SharedString, WeakEntity};
use runner_core::protocol::runtime_paths::find_executable;
use runner_terminal::terminal::LinkTarget;

use crate::app_settings::FileLinkEditor;
use crate::app_store::global_app_store;
use crate::surfaces::files::{FilesView, ShowPreview};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileLinkTarget {
    pub(crate) path: PathBuf,
    pub(crate) line: Option<u32>,
    pub(crate) column: Option<u32>,
}

impl FileLinkTarget {
    fn locator(&self) -> String {
        let mut locator = self.path.to_string_lossy().into_owned();
        if let Some(line) = self.line {
            locator.push_str(&format!(":{line}"));
            if let Some(column) = self.column {
                locator.push_str(&format!(":{column}"));
            }
        }
        locator
    }
}

type LinkView = Rc<dyn Fn(&mut App) -> Option<(Entity<FilesView>, ShowPreview)>>;

/// Where a terminal's ⌘-clicked file links open first: the side panel of
/// the chat, terminal tab or mission the terminal is in, whose preview takes
/// a file inside its Files root (Jason, 2026-11-05). Any other link opens in
/// the chosen editor.
#[derive(Clone)]
pub(crate) struct FileLinkRoute {
    view: LinkView,
}

impl FileLinkRoute {
    /// `view` gives the panel's Files view while its preview can take links,
    /// and `show` opens the panel on it.
    pub(crate) fn new<T: 'static>(
        owner: WeakEntity<T>,
        view: impl Fn(&mut T, &mut Context<T>) -> Option<Entity<FilesView>> + 'static,
        show: impl Fn(&mut T, &mut Context<T>) + 'static,
    ) -> Self {
        let show = Rc::new(show);
        Self {
            view: Rc::new(move |cx| {
                let view = owner.update(cx, |this, cx| view(this, cx)).ok().flatten()?;
                let owner = owner.clone();
                let show = show.clone();
                let show: ShowPreview = Rc::new(move |cx| {
                    let _ = owner.update(cx, |owner, cx| show(owner, cx));
                });
                Some((view, show))
            }),
        }
    }

    pub(crate) fn open(&self, target: FileLinkTarget, cx: &mut App) {
        match (self.view)(cx) {
            Some((view, show)) => {
                view.update(cx, |view, cx| view.open_terminal_link(target, show, cx));
            }
            None => open_file_link(target, cx),
        }
    }

    /// Whether a click on `path`, hovered since `since`, opens in the
    /// preview; `None` while the panel works that out, calling `redraw`
    /// once it may know.
    pub(crate) fn previews(
        &self,
        path: &Path,
        since: std::time::Instant,
        redraw: ShowPreview,
        cx: &mut App,
    ) -> Option<bool> {
        match (self.view)(cx) {
            Some((view, _)) => {
                view.update(cx, |view, cx| view.link_previews(path, since, redraw, cx))
            }
            None => Some(false),
        }
    }
}

/// The hover tooltip for a terminal link: the preview when it takes the
/// file, else the editor setting.
pub(crate) fn link_tooltip_content(
    target: &LinkTarget,
    modifier_held: bool,
    previews: bool,
    cx: &App,
) -> SharedString {
    let store = global_app_store(cx);
    let store = store.read(cx);
    let editor = store.settings.file_link_editor;
    let cli_found = matches!(target, LinkTarget::File { .. })
        .then(|| {
            editor.cli().map(|command| {
                store
                    .file_link_environment
                    .1
                    .get(command)
                    .copied()
                    .unwrap_or(false)
            })
        })
        .flatten();
    link_tooltip(target, modifier_held, previews, editor, cli_found).into()
}

/// Without the modifier the tooltip teaches the gesture; with it, it names
/// the action and the target so the user knows where focus is about to go.
fn link_tooltip(
    target: &LinkTarget,
    modifier_held: bool,
    previews: bool,
    editor: FileLinkEditor,
    cli_found: Option<bool>,
) -> String {
    let (path, line) = match target {
        LinkTarget::Url(_) => {
            return if modifier_held {
                crate::i18n::text("ui.open_in_browser").into()
            } else {
                crate::i18n::tf(
                    "format.click_to_open_in_browser",
                    &[(
                        "shortcut",
                        (crate::platform_ui::PRIMARY_MODIFIER).to_string(),
                    )],
                )
            };
        }
        LinkTarget::File { path, line, .. } => (path, *line),
    };
    let name = || {
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    if previews {
        return match (modifier_held, line) {
            (false, _) => crate::i18n::tf(
                "format.click_to_open_in",
                &[
                    ("shortcut", crate::platform_ui::PRIMARY_MODIFIER.to_string()),
                    (
                        "destination",
                        crate::i18n::text("ui.side_panel_destination").to_string(),
                    ),
                ],
            ),
            (true, Some(line)) => crate::i18n::tf(
                "format.open_in_side_panel_at_line",
                &[("name", name()), ("line", line.to_string())],
            ),
            (true, None) => crate::i18n::tf("format.open_in_side_panel", &[("name", name())]),
        };
    }
    let falls_back = editor == FileLinkEditor::DefaultApp || cli_found == Some(false);
    if !modifier_held {
        let destination = if falls_back {
            crate::i18n::text("ui.default_app_destination")
        } else {
            editor.label()
        };
        return crate::i18n::tf(
            "format.click_to_open_in",
            &[
                (
                    "shortcut",
                    (crate::platform_ui::PRIMARY_MODIFIER).to_string(),
                ),
                ("destination", (destination).to_string()),
            ],
        );
    }
    let name = name();
    match (editor, cli_found, line) {
        (FileLinkEditor::DefaultApp, _, Some(line)) => crate::i18n::tf(
            "format.open_in_default_app_line_is_lost",
            &[("line", (line).to_string())],
        ),
        (FileLinkEditor::DefaultApp, _, None) => crate::i18n::tf(
            "format.open_in_default_app",
            &[("name", (name).to_string())],
        ),
        (editor, Some(false), _) => crate::i18n::tf(
            "format.open_in_default_app_not_on_path",
            &[("command", (editor.cli().unwrap_or_default()).to_string())],
        ),
        (editor, _, Some(line)) => crate::i18n::tf(
            "format.open_in",
            &[
                ("editor", (editor.label()).to_string()),
                ("name", (name).to_string()),
                ("line", (line).to_string()),
            ],
        ),
        (editor, _, None) => crate::i18n::tf(
            "format.open_file_in_editor",
            &[
                ("editor", (editor.label()).to_string()),
                ("name", (name).to_string()),
            ],
        ),
    }
}

pub(crate) fn open_file_link(target: FileLinkTarget, cx: &mut App) {
    let editor = global_app_store(cx).read(cx).settings.file_link_editor;
    open_file_with(editor, target, cx);
}

#[cfg(test)]
thread_local! {
    /// What `open_file_with` would have launched on this thread.
    pub(crate) static LAUNCHED: std::cell::RefCell<Vec<(FileLinkEditor, FileLinkTarget)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Opens `target` in `editor`, or the default app when its launcher fails.
pub(crate) fn open_file_with(editor: FileLinkEditor, target: FileLinkTarget, cx: &mut App) {
    #[cfg(test)]
    if cfg!(test) {
        LAUNCHED.with(|launched| launched.borrow_mut().push((editor, target)));
        return;
    }
    let client = global_app_store(cx).read(cx).client.clone();
    cx.background_spawn(async move {
        let path_env = client.direct_chat_path().unwrap_or_default();
        launch(editor, &target, &path_env)
    })
    .detach();
}

fn launch(editor: FileLinkEditor, target: &FileLinkTarget, path_env: &str) {
    let Err(error) = run(&editor_argv(editor, target), path_env) else {
        return;
    };
    tracing::warn!("file link editor launch failed: {error}; falling back to open");
    if let Err(error) = run(&open_argv(target), path_env) {
        tracing::warn!("file link open fallback failed: {error}");
    }
}

fn editor_argv(editor: FileLinkEditor, target: &FileLinkTarget) -> Vec<String> {
    match editor {
        FileLinkEditor::Zed => vec!["zed".into(), target.locator()],
        FileLinkEditor::VsCode => vec!["code".into(), "--goto".into(), target.locator()],
        FileLinkEditor::Cursor => vec!["cursor".into(), "--goto".into(), target.locator()],
        FileLinkEditor::DefaultApp => open_argv(target),
    }
}

#[cfg(target_os = "macos")]
fn open_argv(target: &FileLinkTarget) -> Vec<String> {
    vec![
        "/usr/bin/open".into(),
        target.path.to_string_lossy().into_owned(),
    ]
}

#[cfg(target_os = "linux")]
fn open_argv(target: &FileLinkTarget) -> Vec<String> {
    vec![
        "xdg-open".into(),
        target.path.to_string_lossy().into_owned(),
    ]
}

#[cfg(windows)]
fn open_argv(target: &FileLinkTarget) -> Vec<String> {
    vec![
        "rundll32.exe".into(),
        "url.dll,FileProtocolHandler".into(),
        target.path.to_string_lossy().into_owned(),
    ]
}

fn run(argv: &[String], path_env: &str) -> Result<(), String> {
    let (program, args) = argv.split_first().ok_or("empty command")?;
    let program = if program.contains('/') {
        PathBuf::from(program)
    } else {
        find_executable(program, path_env).ok_or_else(|| format!("{program} not found on PATH"))?
    };
    let status = Command::new(program)
        .args(args)
        .env("PATH", path_env)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("{}: {error}", argv.join(" ")))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{} exited with {status}", argv.join(" ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(line: Option<u32>, column: Option<u32>) -> FileLinkTarget {
        FileLinkTarget {
            path: "/tmp/src/lib.rs".into(),
            line,
            column,
        }
    }

    fn file_target() -> LinkTarget {
        LinkTarget::File {
            path: "/repo/crates/runner-app/src/surfaces/panes.rs".into(),
            line: Some(1311),
            column: None,
        }
    }

    #[test]
    fn a_cmd_click_on_a_path_opens_it_with_the_side_panel_and_files_off() {
        use gpui::TestAppContext;
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, cx| {
            store.update_settings(
                |settings| {
                    settings.show_side_panel = false;
                    settings.files_view = false;
                    settings.file_link_editor = FileLinkEditor::Zed;
                    true
                },
                false,
                cx,
            );
        });
        LAUNCHED.with(|launched| launched.borrow_mut().clear());
        cx.update(|cx| open_file_link(target(Some(4), None), cx));
        assert_eq!(
            LAUNCHED.with(|launched| launched.borrow().clone()),
            [(FileLinkEditor::Zed, target(Some(4), None))]
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_cmd_click_on_a_path_in_a_terminal_goes_through_its_surfaces_route() {
        use crate::terminal::control::tests::{attached, mirror_named};
        use crate::terminal::element::{TerminalElement, TerminalStyle};
        use gpui::{
            div, prelude::*, px, Modifiers, MouseButton, TestAppContext, VisualTestContext, Window,
        };
        use runner_core::protocol::terminal::TerminalController;

        struct Host {
            chat: crate::AttachedChat,
            routed: bool,
            asked: usize,
        }
        impl Host {
            fn view(&mut self, _: &mut Context<Self>) -> Option<Entity<FilesView>> {
                self.asked += 1;
                None
            }
            fn show(&mut self, _: &mut Context<Self>) {}
        }
        impl Render for Host {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                let style = TerminalStyle {
                    palette: runner_terminal::palette::RUNNER,
                    font: crate::app_settings::AppSettings::default()
                        .terminal_font_family
                        .font(cx),
                    font_size: 14.,
                    app_zoom: 1.,
                    line_height_factor: crate::app_settings::TERMINAL_LINE_HEIGHT_DEFAULT,
                    copy_on_select: false,
                };
                let route = self
                    .routed
                    .then(|| FileLinkRoute::new(cx.weak_entity(), Self::view, Self::show));
                div().size_full().child(
                    div()
                        .debug_selector(|| "LINK_TERMINAL".into())
                        .w(px(800.))
                        .h(px(300.))
                        .child(
                            TerminalElement::new(
                                self.chat.terminal.clone(),
                                self.chat.terminal_interaction.clone(),
                                self.chat.terminal_input.clone(),
                                self.chat.terminal_focus.clone(),
                                true,
                                true,
                                style,
                            )
                            .file_links(route),
                        ),
                )
            }
        }

        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().canonicalize().unwrap().join("cited.rs");
        std::fs::write(&file, "one\ntwo\n").unwrap();
        let mut cx = TestAppContext::single();
        crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let (_host, terminal, _tx, _ack) = mirror_named("links", TerminalController::Desktop);
        terminal.test_feed(1, format!("{}:2", file.display()).as_bytes());
        let window = cx.add_window(|_, cx| Host {
            chat: attached(terminal, cx, 100, 20),
            routed: true,
            asked: 0,
        });
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        visual.run_until_parked();
        let bounds = visual.debug_bounds("LINK_TERMINAL").unwrap();
        let on_path = bounds.origin + gpui::point(px(30.), px(8.));
        let control = Modifiers {
            control: true,
            ..Default::default()
        };
        let expected = FileLinkTarget {
            path: file.clone(),
            line: Some(2),
            column: None,
        };
        // Hovering past the tooltip's delay asks the route from prepaint,
        // outside any update of its owner.
        visual.simulate_mouse_move(on_path, None, control);
        std::thread::sleep(std::time::Duration::from_millis(600));
        window.update(&mut visual, |_, _, cx| cx.notify()).unwrap();
        visual.run_until_parked();
        assert!(window.read_with(&visual, |host, _| host.asked).unwrap() >= 1);
        visual.simulate_mouse_move(gpui::point(px(0.), px(500.)), None, Modifiers::default());
        visual.run_until_parked();
        for routed in [true, false] {
            window
                .update(&mut visual, |host, _, _| host.asked = 0)
                .unwrap();
            window
                .update(&mut visual, |host, _, cx| {
                    host.routed = routed;
                    cx.notify();
                })
                .unwrap();
            visual.run_until_parked();
            LAUNCHED.with(|launched| launched.borrow_mut().clear());
            visual.simulate_mouse_move(on_path, None, control);
            visual.simulate_mouse_down(on_path, MouseButton::Left, control);
            visual.simulate_mouse_up(on_path, MouseButton::Left, control);
            visual.run_until_parked();
            assert_eq!(
                window.read_with(&visual, |host, _| host.asked).unwrap(),
                usize::from(routed),
                "routed {routed}"
            );
            // The route had no preview to give, so the editor opens it.
            assert_eq!(
                LAUNCHED.with(|launched| launched.borrow().clone()),
                [(FileLinkEditor::DefaultApp, expected.clone())],
                "routed {routed}"
            );
        }
    }

    #[test]
    fn link_tooltip_teaches_the_gesture_then_names_the_action() {
        let modifier = if cfg!(any(windows, target_os = "linux")) {
            "Ctrl"
        } else {
            "⌘"
        };
        let url = LinkTarget::Url("https://example.com".into());
        assert_eq!(
            link_tooltip(&url, false, false, FileLinkEditor::Zed, None),
            format!("{modifier}-click to open in browser")
        );
        assert_eq!(
            link_tooltip(&url, true, false, FileLinkEditor::Zed, None),
            "Open in browser"
        );

        let file = file_target();
        assert_eq!(
            link_tooltip(&file, false, false, FileLinkEditor::Zed, Some(true)),
            format!("{modifier}-click to open in Zed")
        );
        assert_eq!(
            link_tooltip(&file, true, false, FileLinkEditor::Zed, Some(true)),
            "Open in Zed · panes.rs:1311"
        );
        assert_eq!(
            link_tooltip(&file, false, false, FileLinkEditor::VsCode, Some(false)),
            format!("{modifier}-click to open in default app")
        );
        assert_eq!(
            link_tooltip(&file, true, false, FileLinkEditor::VsCode, Some(false)),
            "Open in default app · code not on PATH"
        );
        assert_eq!(
            link_tooltip(&file, true, false, FileLinkEditor::DefaultApp, None),
            "Open in default app · line 1311 is lost"
        );
        let no_line = LinkTarget::File {
            path: "/repo/README.md".into(),
            line: None,
            column: None,
        };
        assert_eq!(
            link_tooltip(&no_line, true, false, FileLinkEditor::DefaultApp, None),
            "Open in default app · README.md"
        );
        assert_eq!(
            link_tooltip(&no_line, true, false, FileLinkEditor::Cursor, Some(true)),
            "Open in Cursor · README.md"
        );
    }

    #[test]
    fn link_tooltip_names_the_side_panel_when_the_preview_takes_the_file() {
        let modifier = if cfg!(any(windows, target_os = "linux")) {
            "Ctrl"
        } else {
            "⌘"
        };
        let file = file_target();
        for editor in [FileLinkEditor::Zed, FileLinkEditor::DefaultApp] {
            assert_eq!(
                link_tooltip(&file, false, true, editor, Some(false)),
                format!("{modifier}-click to open in side panel")
            );
            assert_eq!(
                link_tooltip(&file, true, true, editor, Some(false)),
                "Open in side panel · panes.rs:1311"
            );
        }
        let no_line = LinkTarget::File {
            path: "/repo/README.md".into(),
            line: None,
            column: None,
        };
        assert_eq!(
            link_tooltip(&no_line, true, true, FileLinkEditor::Zed, Some(true)),
            "Open in side panel · README.md"
        );
        let url = LinkTarget::Url("https://example.com".into());
        assert_eq!(
            link_tooltip(&url, true, true, FileLinkEditor::Zed, None),
            "Open in browser"
        );
    }

    #[test]
    fn editor_argv_passes_line_and_column_through_each_cli() {
        assert_eq!(
            editor_argv(FileLinkEditor::Zed, &target(Some(12), Some(5))),
            ["zed", "/tmp/src/lib.rs:12:5"]
        );
        assert_eq!(
            editor_argv(FileLinkEditor::VsCode, &target(Some(12), None)),
            ["code", "--goto", "/tmp/src/lib.rs:12"]
        );
        assert_eq!(
            editor_argv(FileLinkEditor::Cursor, &target(None, Some(3))),
            ["cursor", "--goto", "/tmp/src/lib.rs"]
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            editor_argv(FileLinkEditor::DefaultApp, &target(Some(1), None)),
            ["/usr/bin/open", "/tmp/src/lib.rs"]
        );
        #[cfg(windows)]
        assert_eq!(
            editor_argv(FileLinkEditor::DefaultApp, &target(Some(1), None)),
            [
                "rundll32.exe",
                "url.dll,FileProtocolHandler",
                "/tmp/src/lib.rs"
            ]
        );
    }

    #[cfg(windows)]
    #[test]
    fn open_argv_passes_windows_paths_verbatim() {
        for path in [
            r"C:\Users\Jason Wang\项目\notes & %PATH% !.txt",
            r"\\server\shared files\notes.txt",
        ] {
            let target = FileLinkTarget {
                path: path.into(),
                line: Some(12),
                column: Some(5),
            };
            assert_eq!(
                open_argv(&target),
                ["rundll32.exe", "url.dll,FileProtocolHandler", path]
            );
        }
    }
}
