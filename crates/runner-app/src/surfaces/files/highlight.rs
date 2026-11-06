//! Syntax highlighting with tree-sitter: the bundled grammars, a file's
//! language from its name, and a file's spans, computed off the UI thread
//! and kept per line so the code view styles only the lines it draws.
//!
//! Spans carry an index into [`syntax::CAPTURES`], not a colour, so a theme
//! change recolours them without highlighting again.

use std::ops::Range;
use std::sync::atomic::AtomicUsize;
use std::sync::OnceLock;

use tree_sitter::Language;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

use super::document::Document;
use super::syntax;

/// A bundled grammar: how files are recognised, its parser and its queries.
pub(crate) struct Grammar {
    /// The name code fences and injections use.
    pub(crate) name: &'static str,
    /// Other names a code fence may use.
    aliases: &'static [&'static str],
    /// Lower-case extensions, without the dot.
    extensions: &'static [&'static str],
    /// Exact file names.
    file_names: &'static [&'static str],
    language: fn() -> Language,
    /// Joined in order; later patterns win.
    highlights: &'static [&'static str],
    injections: &'static str,
    /// The grammar's licence. Never read: it is here so the notice ships
    /// inside the app, which [`SHIPPED_NOTICES`] guarantees.
    #[allow(dead_code)]
    licence: &'static str,
}

macro_rules! licence {
    ($name:literal) => {
        include_str!(concat!("../../../../../assets/grammars/LICENSE-", $name))
    };
}

/// The runtime's and highlighter's own licence.
const TREE_SITTER_LICENCE: &str = licence!("tree-sitter");

/// Kept in the binary however much of the grammar table the optimiser can
/// see through, so every grammar's licence ships with the app.
#[used]
static SHIPPED_NOTICES: (&str, &[Grammar]) = (TREE_SITTER_LICENCE, GRAMMARS);

/// `tree-sitter-md`'s block injections, with the inline grammar given the
/// whole of each `inline` node: the block grammar marks code spans and
/// other inline tokens as its children, which an injection leaves out
/// unless it includes them.
const MARKDOWN_INJECTIONS: &str = r#"
(fenced_code_block
  (info_string
    (language) @injection.language)
  (code_fence_content) @injection.content)

((html_block) @injection.content
  (#set! injection.language "html"))

((minus_metadata) @injection.content
  (#set! injection.language "yaml"))

((plus_metadata) @injection.content
  (#set! injection.language "toml"))

((inline) @injection.content
  (#set! injection.language "markdown_inline")
  (#set! injection.include-children))
"#;

/// Every bundled grammar. Keep it in step with the `tree-sitter-*`
/// dependencies and `assets/grammars/README.md`.
pub(crate) static GRAMMARS: &[Grammar] = &[
    Grammar {
        name: "bash",
        aliases: &["sh", "shell", "zsh", "console"],
        extensions: &["sh", "bash", "zsh"],
        file_names: &[
            ".bashrc",
            ".bash_profile",
            ".zshrc",
            ".zprofile",
            ".profile",
        ],
        language: || tree_sitter_bash::LANGUAGE.into(),
        highlights: &[tree_sitter_bash::HIGHLIGHT_QUERY],
        injections: "",
        licence: licence!("bash"),
    },
    Grammar {
        name: "c",
        aliases: &[],
        extensions: &["c", "h"],
        file_names: &[],
        language: || tree_sitter_c::LANGUAGE.into(),
        highlights: &[tree_sitter_c::HIGHLIGHT_QUERY],
        injections: "",
        licence: licence!("c"),
    },
    Grammar {
        name: "c-sharp",
        aliases: &["csharp", "cs", "c#"],
        extensions: &["cs"],
        file_names: &[],
        language: || tree_sitter_c_sharp::LANGUAGE.into(),
        highlights: &[tree_sitter_c_sharp::HIGHLIGHTS_QUERY],
        injections: "",
        licence: licence!("c-sharp"),
    },
    Grammar {
        name: "cpp",
        aliases: &["c++"],
        extensions: &["cc", "cpp", "cxx", "c++", "hh", "hpp", "hxx", "h++"],
        file_names: &[],
        language: || tree_sitter_cpp::LANGUAGE.into(),
        highlights: &[
            tree_sitter_c::HIGHLIGHT_QUERY,
            tree_sitter_cpp::HIGHLIGHT_QUERY,
        ],
        injections: "",
        licence: licence!("cpp"),
    },
    Grammar {
        name: "css",
        aliases: &[],
        extensions: &["css"],
        file_names: &[],
        language: || tree_sitter_css::LANGUAGE.into(),
        highlights: &[tree_sitter_css::HIGHLIGHTS_QUERY],
        injections: "",
        licence: licence!("css"),
    },
    Grammar {
        name: "go",
        aliases: &["golang"],
        extensions: &["go"],
        file_names: &[],
        language: || tree_sitter_go::LANGUAGE.into(),
        highlights: &[tree_sitter_go::HIGHLIGHTS_QUERY],
        injections: "",
        licence: licence!("go"),
    },
    Grammar {
        name: "html",
        aliases: &["htm", "xhtml"],
        extensions: &["html", "htm", "xhtml"],
        file_names: &[],
        language: || tree_sitter_html::LANGUAGE.into(),
        highlights: &[tree_sitter_html::HIGHLIGHTS_QUERY],
        injections: tree_sitter_html::INJECTIONS_QUERY,
        licence: licence!("html"),
    },
    Grammar {
        name: "java",
        aliases: &[],
        extensions: &["java"],
        file_names: &[],
        language: || tree_sitter_java::LANGUAGE.into(),
        highlights: &[tree_sitter_java::HIGHLIGHTS_QUERY],
        injections: "",
        licence: licence!("java"),
    },
    Grammar {
        name: "javascript",
        aliases: &["js", "jsx", "node"],
        extensions: &["js", "mjs", "cjs", "jsx"],
        file_names: &[],
        language: || tree_sitter_javascript::LANGUAGE.into(),
        highlights: &[
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
        ],
        injections: "",
        licence: licence!("javascript"),
    },
    Grammar {
        name: "json",
        aliases: &["jsonc"],
        extensions: &["json", "jsonc"],
        file_names: &[".prettierrc", ".eslintrc", ".babelrc"],
        language: || tree_sitter_json::LANGUAGE.into(),
        highlights: &[tree_sitter_json::HIGHLIGHTS_QUERY],
        injections: "",
        licence: licence!("json"),
    },
    Grammar {
        name: "kotlin",
        aliases: &["kt"],
        extensions: &["kt", "kts"],
        file_names: &[],
        language: || tree_sitter_kotlin_ng::LANGUAGE.into(),
        highlights: &[include_str!(
            "../../../../../assets/grammars/kotlin/highlights.scm"
        )],
        injections: "",
        licence: licence!("kotlin"),
    },
    Grammar {
        name: "lua",
        aliases: &[],
        extensions: &["lua"],
        file_names: &[],
        language: || tree_sitter_lua::LANGUAGE.into(),
        highlights: &[tree_sitter_lua::HIGHLIGHTS_QUERY],
        injections: "",
        licence: licence!("lua"),
    },
    Grammar {
        name: "make",
        aliases: &["makefile", "mk"],
        extensions: &["mk", "mak"],
        file_names: &["Makefile", "makefile", "GNUmakefile"],
        language: || tree_sitter_make::LANGUAGE.into(),
        highlights: &[tree_sitter_make::HIGHLIGHTS_QUERY],
        injections: "",
        licence: licence!("make"),
    },
    Grammar {
        name: "markdown",
        aliases: &["md"],
        extensions: &["md", "markdown", "mdown", "mkd"],
        file_names: &[],
        language: || tree_sitter_md::LANGUAGE.into(),
        highlights: &[tree_sitter_md::HIGHLIGHT_QUERY_BLOCK],
        injections: MARKDOWN_INJECTIONS,
        licence: licence!("markdown"),
    },
    Grammar {
        name: "markdown_inline",
        aliases: &[],
        extensions: &[],
        file_names: &[],
        language: || tree_sitter_md::INLINE_LANGUAGE.into(),
        highlights: &[tree_sitter_md::HIGHLIGHT_QUERY_INLINE],
        injections: "",
        licence: licence!("markdown"),
    },
    Grammar {
        name: "php",
        aliases: &[],
        extensions: &["php"],
        file_names: &[],
        language: || tree_sitter_php::LANGUAGE_PHP.into(),
        highlights: &[tree_sitter_php::HIGHLIGHTS_QUERY],
        injections: tree_sitter_php::INJECTIONS_QUERY,
        licence: licence!("php"),
    },
    Grammar {
        name: "python",
        aliases: &["py", "python3"],
        extensions: &["py", "pyi", "pyw"],
        file_names: &[],
        language: || tree_sitter_python::LANGUAGE.into(),
        highlights: &[tree_sitter_python::HIGHLIGHTS_QUERY],
        injections: "",
        licence: licence!("python"),
    },
    Grammar {
        name: "ruby",
        aliases: &["rb"],
        extensions: &["rb", "rake", "gemspec"],
        file_names: &["Gemfile", "Rakefile"],
        language: || tree_sitter_ruby::LANGUAGE.into(),
        highlights: &[tree_sitter_ruby::HIGHLIGHTS_QUERY],
        injections: "",
        licence: licence!("ruby"),
    },
    Grammar {
        name: "rust",
        aliases: &["rs"],
        extensions: &["rs"],
        file_names: &[],
        language: || tree_sitter_rust::LANGUAGE.into(),
        highlights: &[tree_sitter_rust::HIGHLIGHTS_QUERY],
        injections: tree_sitter_rust::INJECTIONS_QUERY,
        licence: licence!("rust"),
    },
    Grammar {
        name: "sql",
        aliases: &[],
        extensions: &["sql"],
        file_names: &[],
        language: || tree_sitter_sequel::LANGUAGE.into(),
        highlights: &[tree_sitter_sequel::HIGHLIGHTS_QUERY],
        injections: "",
        licence: licence!("sql"),
    },
    Grammar {
        name: "swift",
        aliases: &[],
        extensions: &["swift"],
        file_names: &[],
        language: || tree_sitter_swift::LANGUAGE.into(),
        highlights: &[tree_sitter_swift::HIGHLIGHTS_QUERY],
        injections: "",
        licence: licence!("swift"),
    },
    Grammar {
        name: "toml",
        aliases: &[],
        extensions: &["toml"],
        file_names: &["Cargo.lock", "uv.lock", "poetry.lock", "Pipfile"],
        language: || tree_sitter_toml_ng::LANGUAGE.into(),
        highlights: &[tree_sitter_toml_ng::HIGHLIGHTS_QUERY],
        injections: "",
        licence: licence!("toml"),
    },
    Grammar {
        name: "tsx",
        aliases: &[],
        extensions: &["tsx"],
        file_names: &[],
        language: || tree_sitter_typescript::LANGUAGE_TSX.into(),
        highlights: &[
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ],
        injections: "",
        licence: licence!("typescript"),
    },
    Grammar {
        name: "typescript",
        aliases: &["ts"],
        extensions: &["ts", "mts", "cts"],
        file_names: &[],
        language: || tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        highlights: &[
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
        ],
        injections: "",
        licence: licence!("typescript"),
    },
    Grammar {
        name: "yaml",
        aliases: &["yml"],
        extensions: &["yaml", "yml"],
        file_names: &[],
        language: || tree_sitter_yaml::LANGUAGE.into(),
        highlights: &[tree_sitter_yaml::HIGHLIGHTS_QUERY],
        injections: "",
        licence: licence!("yaml"),
    },
];

/// The grammar for a file, from its name and then its extension.
pub(crate) fn grammar_for_path(path: &str) -> Option<&'static Grammar> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    if let Some(grammar) = GRAMMARS
        .iter()
        .find(|grammar| grammar.file_names.contains(&name))
    {
        return Some(grammar);
    }
    let (stem, extension) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    let extension = extension.to_ascii_lowercase();
    GRAMMARS
        .iter()
        .find(|grammar| grammar.extensions.contains(&extension.as_str()))
}

/// The grammar a code fence or an injection names: its name, an alias or
/// an extension, in any case.
pub(crate) fn grammar_named(name: &str) -> Option<&'static Grammar> {
    let name = name.trim().to_ascii_lowercase();
    GRAMMARS.iter().find(|grammar| {
        grammar.name == name
            || grammar.aliases.contains(&name.as_str())
            || grammar.extensions.contains(&name.as_str())
    })
}

/// Each grammar's compiled queries, built the first time one is needed,
/// on the thread highlighting with it.
fn config(grammar: &'static Grammar) -> Option<&'static HighlightConfiguration> {
    static CONFIGS: OnceLock<Vec<OnceLock<Option<HighlightConfiguration>>>> = OnceLock::new();
    let configs = CONFIGS.get_or_init(|| GRAMMARS.iter().map(|_| OnceLock::new()).collect());
    let index = GRAMMARS
        .iter()
        .position(|candidate| std::ptr::eq(candidate, grammar))?;
    configs[index]
        .get_or_init(|| {
            let highlights = grammar.highlights.join("\n");
            match HighlightConfiguration::new(
                (grammar.language)(),
                grammar.name,
                &highlights,
                grammar.injections,
                "",
            ) {
                Ok(mut config) => {
                    config.configure(syntax::CAPTURES);
                    Some(config)
                }
                Err(error) => {
                    tracing::warn!(grammar = grammar.name, %error, "highlight query failed");
                    None
                }
            }
        })
        .as_ref()
}

#[cfg(test)]
pub(crate) fn test_config(grammar: &'static Grammar) -> &'static HighlightConfiguration {
    config(grammar).unwrap_or_else(|| panic!("{}", grammar.name))
}

/// A run of a line's bytes drawn as one capture.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct Span {
    pub(crate) start: u32,
    pub(crate) end: u32,
    /// An index into [`syntax::CAPTURES`].
    pub(crate) capture: u16,
}

/// A file's spans, kept per line: each line's spans are sorted, do not
/// overlap and hold byte offsets into that line.
#[derive(Debug, Default)]
pub(crate) struct Highlights {
    spans: Vec<Span>,
    /// Where each line's spans end in `spans`.
    line_ends: Vec<u32>,
}

impl Highlights {
    pub(crate) fn line(&self, line: usize) -> &[Span] {
        let Some(&end) = self.line_ends.get(line) else {
            return &[];
        };
        let start = match line {
            0 => 0,
            _ => self.line_ends[line - 1],
        };
        &self.spans[start as usize..end as usize]
    }

    #[cfg(test)]
    pub(crate) fn line_count(&self) -> usize {
        self.line_ends.len()
    }
}

/// Highlights `document` as `grammar`. Slow on a large file: run it off the
/// UI thread. `None` when the grammar fails or `cancel` is set (non-zero)
/// before it finishes.
pub(crate) fn highlight(
    document: &Document,
    grammar: &'static Grammar,
    cancel: Option<&AtomicUsize>,
) -> Option<Highlights> {
    let root = config(grammar)?;
    let text = document.text();
    let mut highlighter = Highlighter::new();
    let events = highlighter
        .highlight(root, text.as_bytes(), None, cancel, |name| {
            grammar_named(name).and_then(config)
        })
        .ok()?;
    let mut builder = Builder {
        document,
        line: 0,
        highlights: Highlights::default(),
    };
    let mut stack: Vec<u16> = Vec::new();
    for event in events {
        match event.ok()? {
            HighlightEvent::HighlightStart(highlight) => stack.push(highlight.0 as u16),
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                if let Some(&capture) = stack.last() {
                    builder.push(start..end, capture);
                }
            }
        }
    }
    builder.finish();
    Some(builder.highlights)
}

struct Builder<'a> {
    document: &'a Document,
    /// The line the last span ended on; spans arrive in order.
    line: usize,
    highlights: Highlights,
}

impl Builder<'_> {
    /// Splits `range` of the whole text across the lines it covers.
    fn push(&mut self, range: Range<usize>, capture: u16) {
        let last = self.document.line_count() - 1;
        let mut start = range.start;
        while self.line < last && start > self.document.line_range(self.line).end {
            self.close_line();
        }
        loop {
            let bounds = self.document.line_range(self.line);
            let start_in = start.max(bounds.start);
            let end = range.end.min(bounds.end);
            if start_in < end {
                self.add(Span {
                    start: (start_in - bounds.start) as u32,
                    end: (end - bounds.start) as u32,
                    capture,
                });
            }
            if range.end <= bounds.end || self.line == last {
                return;
            }
            self.close_line();
            start = self.document.line_range(self.line).start;
        }
    }

    /// Adds a span to the current line, joining it to the one before when
    /// they touch and match.
    fn add(&mut self, span: Span) {
        let first = self.highlights.line_ends.last().copied().unwrap_or(0) as usize;
        let on_line = self.highlights.spans.len() > first;
        match self.highlights.spans.last_mut() {
            Some(last) if on_line && last.capture == span.capture && last.end == span.start => {
                last.end = span.end;
            }
            _ => self.highlights.spans.push(span),
        }
    }

    fn close_line(&mut self) {
        self.highlights
            .line_ends
            .push(self.highlights.spans.len() as u32);
        self.line += 1;
    }

    fn finish(&mut self) {
        while self.highlights.line_ends.len() < self.document.line_count() {
            self.highlights
                .line_ends
                .push(self.highlights.spans.len() as u32);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn captures(
        highlights: &Highlights,
        document: &Document,
        line: usize,
    ) -> Vec<(String, &'static str)> {
        highlights
            .line(line)
            .iter()
            .map(|span| {
                (
                    document.line(line)[span.start as usize..span.end as usize].to_owned(),
                    syntax::CAPTURES[span.capture as usize],
                )
            })
            .collect()
    }

    #[test]
    fn languages_come_from_the_file_name_then_the_extension() {
        let name = |path: &str| grammar_for_path(path).map(|grammar| grammar.name);
        assert_eq!(name("crates/runner-app/src/main.rs"), Some("rust"));
        assert_eq!(name("web/App.TSX"), Some("tsx"));
        assert_eq!(name("index.ts"), Some("typescript"));
        assert_eq!(name("Cargo.toml"), Some("toml"));
        assert_eq!(name("Cargo.lock"), Some("toml"));
        assert_eq!(name("Makefile"), Some("make"));
        assert_eq!(name("docs\\README.md"), Some("markdown"));
        assert_eq!(name("script/build.sh"), Some("bash"));
        assert_eq!(name(".zshrc"), Some("bash"));
        assert_eq!(name("include/a.hpp"), Some("cpp"));
        assert_eq!(name("Main.kt"), Some("kotlin"));
        assert_eq!(name("Dockerfile"), None, "not bundled");
        assert_eq!(name(".gitignore"), None, "a dotfile is not an extension");
        assert_eq!(name("notes.txt"), None);
        assert_eq!(name("LICENSE"), None);
        let fence = |name: &str| grammar_named(name).map(|grammar| grammar.name);
        assert_eq!(fence("rs"), Some("rust"));
        assert_eq!(fence("Shell"), Some("bash"));
        assert_eq!(fence("c++"), Some("cpp"));
        assert_eq!(fence("markdown_inline"), Some("markdown_inline"));
        assert_eq!(fence("brainfuck"), None);
    }

    #[test]
    fn every_bundled_query_compiles_and_names_known_captures() {
        for grammar in GRAMMARS {
            let config = config(grammar).unwrap_or_else(|| panic!("{}", grammar.name));
            let names = config.names();
            assert!(!names.is_empty(), "{}", grammar.name);
            assert!(!grammar.licence.is_empty(), "{}", grammar.name);
        }
        assert!(TREE_SITTER_LICENCE.contains("MIT"));
    }

    #[test]
    fn rust_is_highlighted_per_line_with_zed_capture_names() {
        let document = Document::decode(
            b"/// Docs.\npub struct Client {\n    reader: BufReader<u32>,\n}\nfn main() { let s = \"a\nb\"; }\n",
        );
        let highlights = highlight(&document, grammar_named("rust").unwrap(), None).unwrap();
        assert_eq!(highlights.line_count(), document.line_count());
        assert_eq!(
            captures(&highlights, &document, 0),
            [("/// Docs.".to_owned(), "comment.documentation")]
        );
        let line = captures(&highlights, &document, 1);
        assert!(line.contains(&("pub".into(), "keyword")), "{line:?}");
        assert!(line.contains(&("Client".into(), "type")), "{line:?}");
        let line = captures(&highlights, &document, 2);
        assert!(line.contains(&("reader".into(), "property")), "{line:?}");
        assert!(line.contains(&("u32".into(), "type.builtin")), "{line:?}");
        let line = captures(&highlights, &document, 4);
        assert!(line.contains(&("main".into(), "function")), "{line:?}");
        // A string across two lines is split at the line end.
        assert!(line.contains(&("\"a".into(), "string")), "{line:?}");
        assert_eq!(
            captures(&highlights, &document, 5)[0],
            ("b\"".into(), "string")
        );
    }

    #[test]
    fn typescript_toml_kotlin_and_markdown_highlight() {
        let check = |name: &str, text: &str, expected: &[(&str, &str)]| {
            let document = Document::decode(text.as_bytes());
            let highlights = highlight(&document, grammar_named(name).unwrap(), None).unwrap();
            let found: Vec<_> = (0..document.line_count())
                .flat_map(|line| captures(&highlights, &document, line))
                .collect();
            for (text, capture) in expected {
                assert!(
                    found.contains(&((*text).to_owned(), *capture)),
                    "{name}: {text} as {capture} in {found:?}"
                );
            }
        };
        check(
            "typescript",
            "interface A { b: string }\nconst x: number = f(1);\n",
            &[
                ("interface", "keyword"),
                ("const", "keyword"),
                ("f", "function"),
                ("1", "number"),
            ],
        );
        check(
            "toml",
            "[package]\nname = \"runner\" # c\n",
            &[
                ("package", "type"),
                ("name ", "property"),
                ("\"runner\"", "string"),
                ("# c", "comment"),
            ],
        );
        check(
            "kotlin",
            "class User(val name: String) {\n  fun greet() = listOf(1)\n}\n",
            &[
                ("class", "keyword"),
                ("User", "type"),
                ("greet", "function"),
                ("listOf", "function"),
                ("1", "number"),
            ],
        );
        // The inline grammar and a fenced block's language are injected.
        check(
            "markdown",
            "# Title\n\nSome `code`.\n\n```rust\nfn main() {}\n```\n",
            &[
                ("Title", "text.title"),
                ("code", "text.literal"),
                ("fn", "keyword"),
            ],
        );
    }

    #[test]
    fn a_cancelled_highlight_returns_nothing() {
        let document = Document::decode("fn a() {}\n".repeat(10_000).as_bytes());
        let cancel = AtomicUsize::new(1);
        assert!(highlight(&document, grammar_named("rust").unwrap(), Some(&cancel)).is_none());
    }

    #[test]
    fn a_markdown_fence_of_a_hundred_thousand_lines_highlights() {
        // tree-sitter 0.25 faulted on this through the fence's injection.
        let mut text = String::from("```unknown\n");
        for line in 0..100_000 {
            text.push_str(&format!("let v{line} = {line};\n"));
        }
        text.push_str("```\n");
        let document = Document::decode(text.as_bytes());
        let highlights = highlight(&document, grammar_named("markdown").unwrap(), None).unwrap();
        assert_eq!(highlights.line_count(), document.line_count());
        assert_eq!(
            captures(&highlights, &document, 50_000),
            [("let v49999 = 49999;".to_owned(), "text.literal")]
        );
    }
}
