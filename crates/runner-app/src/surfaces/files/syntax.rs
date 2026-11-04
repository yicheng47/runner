//! What highlighted code is drawn in: Zed's capture names, and a colour for
//! each from the terminal palette in use. Each bundled palette has a
//! hand-tuned map; any other palette gets one derived from its 16 colours,
//! so code matches the terminal beside it.

use gpui::{App, Hsla};
use runner_terminal::palette::{self, TerminalPalette};

/// The capture names highlighting tells apart, as `tree-sitter-highlight`
/// matches them: a query's capture takes the entry sharing the most of its
/// dotted parts, so `type.builtin` keeps its own entry and `type.qualifier`
/// falls back to `type`. A tie goes to the earlier entry, so `keyword`
/// comes first, making `keyword.type` a keyword, and `function.method`
/// precedes `function.builtin` for `function.method.builtin`, as
/// `string.special` precedes `string.regex` for `string.special.regex`
/// and `number` precedes `float` for `number.float`.
pub(crate) const CAPTURES: &[&str] = &[
    "keyword",
    "keyword.function",
    "attribute",
    "boolean",
    "character",
    "comment",
    "comment.doc",
    "comment.documentation",
    "conditional",
    "constant",
    "constant.builtin",
    "constant.macro",
    "constructor",
    "delimiter",
    "embedded",
    "escape",
    "exception",
    "field",
    "function",
    "function.method",
    "function.builtin",
    "function.call",
    "function.macro",
    "include",
    "import",
    "label",
    "method",
    "module",
    "namespace",
    "number",
    "float",
    "operator",
    "parameter",
    "preproc",
    "property",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.special",
    "repeat",
    "storageclass",
    "string",
    "string.escape",
    "string.special",
    "string.special.symbol",
    "string.regex",
    "string.regexp",
    "tag",
    "text.emphasis",
    "text.literal",
    "text.reference",
    "text.strong",
    "text.title",
    "text.uri",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.member",
    "variable.parameter",
    "variable.special",
];

/// Older or other editors' names for a capture a map colours.
const ALIASES: &[(&str, &str)] = &[
    ("boolean", "constant.builtin"),
    ("character", "string"),
    ("comment.documentation", "comment.doc"),
    ("conditional", "keyword"),
    ("delimiter", "punctuation.delimiter"),
    ("escape", "string.escape"),
    ("exception", "keyword"),
    ("field", "property"),
    ("float", "number"),
    ("import", "keyword"),
    ("include", "keyword"),
    ("method", "function.method"),
    ("namespace", "module"),
    ("parameter", "variable.parameter"),
    ("preproc", "keyword"),
    ("repeat", "keyword"),
    ("storageclass", "keyword"),
    ("string.regexp", "string.regex"),
    ("variable.member", "property"),
];

type SyntaxMap = &'static [(&'static str, u32)];

/// Runner's palette, after the design: Material's keywords, types,
/// functions and modules over Runner's ground.
const RUNNER: SyntaxMap = &[
    ("attribute", 0xc792ea),
    ("comment", 0x6b6e7a),
    ("constant", 0xf78c6c),
    ("constructor", 0xffcb6b),
    ("function", 0x82aaff),
    ("function.macro", 0x89ddff),
    ("keyword", 0xc792ea),
    ("label", 0xc792ea),
    ("module", 0x89ddff),
    ("number", 0xf78c6c),
    ("operator", 0x89ddff),
    ("punctuation", 0x9a9ca8),
    ("punctuation.special", 0x89ddff),
    ("string", 0xc3e88d),
    ("string.escape", 0x89ddff),
    ("string.regex", 0x89ddff),
    ("string.special", 0x89ddff),
    ("string.special.symbol", 0xf78c6c),
    ("tag", 0xff7b8e),
    ("text.emphasis", 0xc792ea),
    ("text.literal", 0xc3e88d),
    ("text.reference", 0x82aaff),
    ("text.strong", 0xffcb6b),
    ("text.title", 0x82aaff),
    ("text.uri", 0x89ddff),
    ("type", 0xffcb6b),
    ("variable.builtin", 0xff7b8e),
];

/// Catppuccin Mocha's own syntax colours.
const CATPPUCCIN_MOCHA: SyntaxMap = &[
    ("attribute", 0xf9e2af),
    ("comment", 0x9399b2),
    ("constant", 0xfab387),
    ("constructor", 0x74c7ec),
    ("function", 0x89b4fa),
    ("function.macro", 0x94e2d5),
    ("keyword", 0xcba6f7),
    ("label", 0x74c7ec),
    ("module", 0xf5e0dc),
    ("number", 0xfab387),
    ("operator", 0x89dceb),
    ("property", 0xb4befe),
    ("punctuation", 0x9399b2),
    ("punctuation.special", 0x89dceb),
    ("string", 0xa6e3a1),
    ("string.escape", 0xf5c2e7),
    ("string.regex", 0xf5c2e7),
    ("string.special", 0xf5c2e7),
    ("string.special.symbol", 0xf2cdcd),
    ("tag", 0x89b4fa),
    ("text.emphasis", 0xeba0ac),
    ("text.literal", 0xa6e3a1),
    ("text.reference", 0xb4befe),
    ("text.strong", 0xeba0ac),
    ("text.title", 0xf38ba8),
    ("text.uri", 0xf5e0dc),
    ("type", 0xf9e2af),
    ("variable.builtin", 0xf38ba8),
    ("variable.parameter", 0xeba0ac),
];

/// Rosé Pine Dawn's own syntax colours.
const ROSE_PINE_DAWN: SyntaxMap = &[
    ("attribute", 0x907aa9),
    ("comment", 0x9893a5),
    ("constant", 0xea9d34),
    ("constructor", 0x56949f),
    ("function", 0xd7827e),
    ("keyword", 0x286983),
    ("label", 0x907aa9),
    ("module", 0x907aa9),
    ("number", 0xea9d34),
    ("operator", 0x797593),
    ("property", 0x56949f),
    ("punctuation", 0x797593),
    ("string", 0xea9d34),
    ("string.escape", 0x286983),
    ("string.regex", 0x907aa9),
    ("string.special", 0x907aa9),
    ("tag", 0x56949f),
    ("text.emphasis", 0xb4637a),
    ("text.literal", 0xea9d34),
    ("text.reference", 0x56949f),
    ("text.strong", 0xb4637a),
    ("text.title", 0x907aa9),
    ("text.uri", 0x907aa9),
    ("type", 0x56949f),
    ("variable.builtin", 0xb4637a),
    ("variable.parameter", 0x907aa9),
];

/// A map for any other palette, from its 16 colours as terminal programs
/// use them: keywords magenta, strings green, types yellow, functions blue.
fn derived(palette: &TerminalPalette) -> Vec<(&'static str, u32)> {
    let ansi = |index: usize| {
        let rgb = palette.ansi[index];
        (u32::from(rgb.r) << 16) | (u32::from(rgb.g) << 8) | u32::from(rgb.b)
    };
    let (red, green, yellow, blue, magenta, cyan, grey) = (
        ansi(1),
        ansi(2),
        ansi(3),
        ansi(4),
        ansi(5),
        ansi(6),
        ansi(8),
    );
    vec![
        ("attribute", yellow),
        ("comment", grey),
        ("constant", red),
        ("constructor", yellow),
        ("function", blue),
        ("keyword", magenta),
        ("label", magenta),
        ("module", cyan),
        ("number", red),
        ("operator", cyan),
        ("punctuation.special", cyan),
        ("string", green),
        ("string.escape", cyan),
        ("string.special", cyan),
        ("tag", red),
        ("text.literal", green),
        ("text.reference", blue),
        ("text.title", blue),
        ("text.uri", cyan),
        ("type", yellow),
    ]
}

/// The colour `map` gives `name`: its own entry, else its alias's, else
/// the nearest head's, dropping one dotted part at a time.
pub(crate) fn lookup(map: &[(&str, u32)], name: &str) -> Option<u32> {
    let name = ALIASES
        .iter()
        .find(|(alias, _)| *alias == name)
        .map_or(name, |(_, target)| target);
    let mut name = name;
    loop {
        if let Some((_, color)) = map.iter().find(|(key, _)| *key == name) {
            return Some(*color);
        }
        let (head, _) = name.rsplit_once('.')?;
        name = head;
    }
}

/// Each capture's colour for one palette; `None` draws in the text colour.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SyntaxTheme {
    colors: Vec<Option<Hsla>>,
}

impl SyntaxTheme {
    pub(crate) fn for_palette(palette: &TerminalPalette) -> Self {
        let derived;
        let map: &[(&str, u32)] = if *palette == palette::RUNNER {
            RUNNER
        } else if *palette == palette::CATPPUCCIN_MOCHA {
            CATPPUCCIN_MOCHA
        } else if *palette == palette::ROSE_PINE_DAWN {
            ROSE_PINE_DAWN
        } else {
            derived = self::derived(palette);
            &derived
        };
        Self {
            colors: CAPTURES
                .iter()
                .map(|name| lookup(map, name).map(|color| gpui::rgb(color).into()))
                .collect(),
        }
    }

    /// The palette the terminal draws in for the app's light or dark mode.
    pub(crate) fn current(cx: &App) -> Self {
        let palette = match cx.try_global::<crate::app_store::GlobalAppStore>() {
            Some(store) => crate::app_settings::terminal_palette(
                &store.0.read(cx).settings,
                crate::theme::active_variant(),
            ),
            None => palette::RUNNER,
        };
        Self::for_palette(&palette)
    }

    pub(crate) fn color(&self, capture: u16) -> Option<Hsla> {
        self.colors.get(capture as usize).copied().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(name: &str) -> u16 {
        CAPTURES
            .iter()
            .position(|capture| *capture == name)
            .unwrap() as u16
    }

    /// The entry `tree-sitter-highlight` gives `capture`, by its rule.
    fn matched(capture: &str) -> Option<&'static str> {
        let parts: Vec<_> = capture.split('.').collect();
        let mut best = None;
        let mut best_len = 0;
        for name in CAPTURES {
            let len = name.split('.').count();
            if name.split('.').all(|part| parts.contains(&part)) && len > best_len {
                best = Some(*name);
                best_len = len;
            }
        }
        best
    }

    #[test]
    fn a_dotted_capture_falls_back_to_its_head() {
        assert_eq!(lookup(RUNNER, "type.builtin"), lookup(RUNNER, "type"));
        assert_eq!(lookup(RUNNER, "function.method.builtin"), Some(0x82aaff));
        assert_eq!(lookup(RUNNER, "punctuation.bracket"), Some(0x9a9ca8));
        assert_eq!(lookup(RUNNER, "string.special.symbol"), Some(0xf78c6c));
        assert_eq!(lookup(RUNNER, "variable"), None, "the text colour");
        assert_eq!(lookup(RUNNER, "comment.documentation"), Some(0x6b6e7a));
        assert_eq!(lookup(RUNNER, "boolean"), lookup(RUNNER, "constant"));
        assert_eq!(lookup(RUNNER, "parameter"), None);
        assert_eq!(lookup(CATPPUCCIN_MOCHA, "parameter"), Some(0xeba0ac));
    }

    #[test]
    fn every_query_capture_resolves_to_its_own_name_or_a_head() {
        for grammar in super::super::highlight::GRAMMARS {
            let config = super::super::highlight::test_config(grammar);
            for name in config.names() {
                if name.starts_with('_') {
                    continue;
                }
                if let Some(matched) = matched(name) {
                    assert!(
                        name == &matched || name.starts_with(&format!("{matched}.")),
                        "{}: {name} took {matched}",
                        grammar.name
                    );
                }
            }
        }
        assert_eq!(matched("keyword.type"), Some("keyword"));
        assert_eq!(matched("keyword.function"), Some("keyword.function"));
        assert_eq!(matched("type.qualifier"), Some("type"));
    }

    #[test]
    fn bundled_palettes_have_hand_tuned_maps_and_others_derive_one() {
        let runner = SyntaxTheme::for_palette(&palette::RUNNER);
        assert_eq!(
            runner.color(index("keyword")),
            Some(gpui::rgb(0xc792ea).into())
        );
        assert_eq!(
            runner.color(index("type.builtin")),
            Some(gpui::rgb(0xffcb6b).into())
        );
        assert_eq!(runner.color(index("variable")), None);
        let mocha = SyntaxTheme::for_palette(&palette::CATPPUCCIN_MOCHA);
        assert_eq!(
            mocha.color(index("keyword")),
            Some(gpui::rgb(0xcba6f7).into())
        );
        let dawn = SyntaxTheme::for_palette(&palette::ROSE_PINE_DAWN);
        assert_eq!(
            dawn.color(index("keyword")),
            Some(gpui::rgb(0x286983).into())
        );

        let mut imported = palette::RUNNER;
        imported.background = alacritty_terminal::vte::ansi::Rgb { r: 0, g: 0, b: 0 };
        imported.ansi[5] = alacritty_terminal::vte::ansi::Rgb {
            r: 0x12,
            g: 0x34,
            b: 0x56,
        };
        imported.ansi[2] = alacritty_terminal::vte::ansi::Rgb {
            r: 0x0a,
            g: 0xb0,
            b: 0x0c,
        };
        let derived = SyntaxTheme::for_palette(&imported);
        assert_eq!(
            derived.color(index("keyword")),
            Some(gpui::rgb(0x123456).into())
        );
        assert_eq!(
            derived.color(index("keyword.function")),
            Some(gpui::rgb(0x123456).into())
        );
        assert_eq!(
            derived.color(index("string")),
            Some(gpui::rgb(0x0ab00c).into())
        );
        assert_eq!(
            derived.color(index("type.builtin")),
            derived.color(index("type")),
            "dotted names fall back here too"
        );
        assert_eq!(
            derived.color(index("comment")),
            Some(gpui::rgb(0x5a5c66).into())
        );
        assert_eq!(derived.color(index("variable")), None);
    }
}
