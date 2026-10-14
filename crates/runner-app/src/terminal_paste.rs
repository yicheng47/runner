use gpui::{ClipboardEntry, ClipboardItem, ExternalPaths};
use runner_core::Runtime;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalPaste {
    Image,
    Text(String),
}

/// The agent reads the image from the system clipboard itself, as it would in
/// any terminal, so Runner only sends the agent's image-paste shortcut.
pub fn image_paste_shortcut(runtime: Option<Runtime>) -> &'static [u8] {
    // Claude Code binds image paste to Alt+V on Windows.
    if cfg!(windows) && runtime == Some(Runtime::ClaudeCode) {
        b"\x1bv"
    } else {
        b"\x16"
    }
}

pub fn resolve_terminal_paste(
    item: Option<&ClipboardItem>,
    file_paths: impl FnOnce() -> Vec<String>,
) -> Option<TerminalPaste> {
    if item.is_some_and(|item| {
        item.entries()
            .iter()
            .any(|entry| matches!(entry, ClipboardEntry::Image(_)))
    }) {
        return Some(TerminalPaste::Image);
    }

    let mut paths = file_paths();
    if paths.is_empty() {
        paths = item
            .into_iter()
            .flat_map(ClipboardItem::entries)
            .filter_map(|entry| match entry {
                ClipboardEntry::ExternalPaths(paths) => Some(paths.paths()),
                ClipboardEntry::String(_) | ClipboardEntry::Image(_) => None,
            })
            .flatten()
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
    }
    if !paths.is_empty() {
        return Some(TerminalPaste::Text(format_pasted_paths(&paths)));
    }

    item.filter(|item| {
        item.entries()
            .iter()
            .any(|entry| matches!(entry, ClipboardEntry::String(_)))
    })
    .and_then(ClipboardItem::text)
    .map(TerminalPaste::Text)
}

pub fn dropped_paths_text(paths: &ExternalPaths) -> Option<String> {
    let paths: Vec<String> = paths
        .paths()
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    (!paths.is_empty()).then(|| format_pasted_paths(&paths))
}

pub fn shell_quote_path(path: &str) -> String {
    if !path.chars().any(needs_quoting) {
        return path.to_owned();
    }
    format!("'{}'", path.replace('\'', "'\\''"))
}

pub fn format_pasted_paths(paths: &[String]) -> String {
    paths
        .iter()
        .map(|path| shell_quote_path(path))
        .collect::<Vec<_>>()
        .join(" ")
}

fn needs_quoting(character: char) -> bool {
    character.is_whitespace()
        || matches!(
            character,
            '!' | '"'
                | '#'
                | '$'
                | '&'
                | '\''
                | '('
                | ')'
                | '*'
                | ';'
                | '<'
                | '>'
                | '?'
                | '['
                | '\\'
                | ']'
                | '^'
                | '`'
                | '{'
                | '|'
                | '}'
                | '~'
        )
}

#[cfg(test)]
mod tests {
    use gpui::{ClipboardItem, Image};

    use super::*;

    #[test]
    fn image_paste_selects_the_runtime_and_platform_shortcut() {
        assert_eq!(image_paste_shortcut(None), b"\x16");
        for runtime in Runtime::ALL {
            let input = image_paste_shortcut(Some(runtime));
            let expected: &[u8] = if cfg!(windows) && runtime == Runtime::ClaudeCode {
                b"\x1bv"
            } else {
                b"\x16"
            };
            assert_eq!(input, expected, "{}", runtime.key());
        }
    }

    #[test]
    fn shell_quote_path_matches_the_react_contract() {
        assert_eq!(shell_quote_path("/tmp/plain.png"), "/tmp/plain.png");
        assert_eq!(
            shell_quote_path("/tmp/two words.png"),
            "'/tmp/two words.png'"
        );
        assert_eq!(shell_quote_path("/tmp/it's.png"), "'/tmp/it'\\''s.png'");

        for character in [
            ' ', '\t', '\n', '!', '"', '#', '$', '&', '\'', '(', ')', '*', ';', '<', '>', '?', '[',
            '\\', ']', '^', '`', '{', '|', '}', '~',
        ] {
            let path = format!("/tmp/a{character}b");
            let quoted = shell_quote_path(&path);
            assert!(quoted.starts_with('\'') && quoted.ends_with('\''));
        }
    }

    #[test]
    fn format_pasted_paths_space_joins_multiple_files() {
        assert_eq!(
            format_pasted_paths(&["/tmp/a.png".into(), "/tmp/two words.jpg".into()]),
            "/tmp/a.png '/tmp/two words.jpg'"
        );
    }

    #[test]
    fn dropped_paths_insert_like_pasted_files() {
        let paths = ExternalPaths(
            ["/tmp/a.png", "/tmp/two words.jpg"]
                .into_iter()
                .map(Into::into)
                .collect(),
        );
        assert_eq!(
            dropped_paths_text(&paths),
            Some("/tmp/a.png '/tmp/two words.jpg'".into())
        );
        assert_eq!(dropped_paths_text(&ExternalPaths::default()), None);
    }

    #[test]
    fn paste_resolution_prefers_image_then_file_urls_then_plain_text() {
        assert_eq!(
            resolve_terminal_paste(Some(&ClipboardItem::new_image(&Image::empty())), || {
                panic!("file URLs must not be read for image paste")
            }),
            Some(TerminalPaste::Image)
        );

        let text = ClipboardItem::new_string("picture.png".into());
        assert_eq!(
            resolve_terminal_paste(Some(&text), || vec!["/tmp/two words/picture.png".into()]),
            Some(TerminalPaste::Text("'/tmp/two words/picture.png'".into()))
        );
        assert_eq!(
            resolve_terminal_paste(Some(&text), Vec::new),
            Some(TerminalPaste::Text("picture.png".into()))
        );
        assert_eq!(
            resolve_terminal_paste(None, || vec!["/tmp/file-only.png".into()]),
            Some(TerminalPaste::Text("/tmp/file-only.png".into()))
        );
        assert_eq!(resolve_terminal_paste(None, Vec::new), None);
    }
}
