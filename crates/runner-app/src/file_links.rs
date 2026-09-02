use std::path::PathBuf;
use std::process::{Command, Stdio};

use gpui::{App, AppContext as _, SharedString};
use runner_backend::runtime_status::{direct_chat_path, find_executable};
use runner_backend::shell_path::LoginShellEnv;
use runner_terminal::terminal::LinkTarget;

use crate::app_settings::FileLinkEditor;
use crate::app_store::global_app_store;

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

/// Whether the editor's CLI is on the login-shell `PATH`; `None` for the
/// default app, which doesn't launch through a CLI.
pub(crate) fn cli_found(editor: FileLinkEditor, shell_env: &LoginShellEnv) -> Option<bool> {
    let cli = editor.cli()?;
    Some(find_executable(cli, &direct_chat_path(shell_env)).is_some())
}

/// The hover tooltip for a terminal link under the current editor setting.
pub(crate) fn link_tooltip_content(
    target: &LinkTarget,
    modifier_held: bool,
    cx: &App,
) -> SharedString {
    let store = global_app_store(cx);
    let store = store.read(cx);
    let editor = store.settings.file_link_editor;
    let cli_found = matches!(target, LinkTarget::File { .. })
        .then(|| {
            let shell_env = store
                .core
                .runtime_shell_env
                .read()
                .map(|env| env.clone())
                .unwrap_or_default();
            cli_found(editor, &shell_env)
        })
        .flatten();
    link_tooltip(target, modifier_held, editor, cli_found).into()
}

/// Without the modifier the tooltip teaches the gesture; with it, it names
/// the action and the target so the user knows where focus is about to go.
fn link_tooltip(
    target: &LinkTarget,
    modifier_held: bool,
    editor: FileLinkEditor,
    cli_found: Option<bool>,
) -> String {
    let (path, line) = match target {
        LinkTarget::Url(_) => {
            return if modifier_held {
                "Open in browser".into()
            } else {
                "⌘-click to open in browser".into()
            };
        }
        LinkTarget::File { path, line, .. } => (path, *line),
    };
    let falls_back = editor == FileLinkEditor::DefaultApp || cli_found == Some(false);
    if !modifier_held {
        let destination = if falls_back {
            "default app"
        } else {
            editor.label()
        };
        return format!("⌘-click to open in {destination}");
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    match (editor, cli_found, line) {
        (FileLinkEditor::DefaultApp, _, Some(line)) => {
            format!("Open in default app · line {line} is lost")
        }
        (FileLinkEditor::DefaultApp, _, None) => format!("Open in default app · {name}"),
        (editor, Some(false), _) => format!(
            "Open in default app · {} not on PATH",
            editor.cli().unwrap_or_default()
        ),
        (editor, _, Some(line)) => format!("Open in {} · {name}:{line}", editor.label()),
        (editor, _, None) => format!("Open in {} · {name}", editor.label()),
    }
}

pub(crate) fn open_file_link(target: FileLinkTarget, cx: &mut App) {
    let store = global_app_store(cx);
    let (editor, path_env) = {
        let store = store.read(cx);
        let shell_env = store
            .core
            .runtime_shell_env
            .read()
            .map(|env| env.clone())
            .unwrap_or_default();
        (
            store.settings.file_link_editor,
            direct_chat_path(&shell_env),
        )
    };
    cx.background_spawn(async move { launch(editor, &target, &path_env) })
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

fn open_argv(target: &FileLinkTarget) -> Vec<String> {
    vec![
        "/usr/bin/open".into(),
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
    fn link_tooltip_teaches_the_gesture_then_names_the_action() {
        let url = LinkTarget::Url("https://example.com".into());
        assert_eq!(
            link_tooltip(&url, false, FileLinkEditor::Zed, None),
            "⌘-click to open in browser"
        );
        assert_eq!(
            link_tooltip(&url, true, FileLinkEditor::Zed, None),
            "Open in browser"
        );

        let file = file_target();
        assert_eq!(
            link_tooltip(&file, false, FileLinkEditor::Zed, Some(true)),
            "⌘-click to open in Zed"
        );
        assert_eq!(
            link_tooltip(&file, true, FileLinkEditor::Zed, Some(true)),
            "Open in Zed · panes.rs:1311"
        );
        assert_eq!(
            link_tooltip(&file, false, FileLinkEditor::VsCode, Some(false)),
            "⌘-click to open in default app"
        );
        assert_eq!(
            link_tooltip(&file, true, FileLinkEditor::VsCode, Some(false)),
            "Open in default app · code not on PATH"
        );
        assert_eq!(
            link_tooltip(&file, true, FileLinkEditor::DefaultApp, None),
            "Open in default app · line 1311 is lost"
        );
        let no_line = LinkTarget::File {
            path: "/repo/README.md".into(),
            line: None,
            column: None,
        };
        assert_eq!(
            link_tooltip(&no_line, true, FileLinkEditor::DefaultApp, None),
            "Open in default app · README.md"
        );
        assert_eq!(
            link_tooltip(&no_line, true, FileLinkEditor::Cursor, Some(true)),
            "Open in Cursor · README.md"
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
        assert_eq!(
            editor_argv(FileLinkEditor::DefaultApp, &target(Some(1), None)),
            ["/usr/bin/open", "/tmp/src/lib.rs"]
        );
    }
}
