use gpui::{ClipboardEntry, ClipboardItem, Image};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalPaste {
    Image(Image),
    Text(String),
}

pub fn resolve_terminal_paste(
    item: Option<&ClipboardItem>,
    file_paths: impl FnOnce() -> Vec<String>,
) -> Option<TerminalPaste> {
    if let Some(image) = item.and_then(|item| {
        item.entries().iter().find_map(|entry| match entry {
            ClipboardEntry::Image(image) => Some(image.clone()),
            ClipboardEntry::String(_) | ClipboardEntry::ExternalPaths(_) => None,
        })
    }) {
        return Some(TerminalPaste::Image(image));
    }

    let paths = file_paths();
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

    use super::{format_pasted_paths, resolve_terminal_paste, shell_quote_path, TerminalPaste};

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
    fn paste_resolution_prefers_image_then_file_urls_then_plain_text() {
        let image = Image::empty();
        assert_eq!(
            resolve_terminal_paste(Some(&ClipboardItem::new_image(&image)), || {
                panic!("file URLs must not be read for image paste")
            }),
            Some(TerminalPaste::Image(image))
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
