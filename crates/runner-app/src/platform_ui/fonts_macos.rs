pub const SYSTEM_MONOSPACE_FONT: &str = "Menlo";
pub const UI_MONOSPACE_FONT: &str = "JetBrainsMono Nerd Font Mono";
// No "PingFang SC" in either list: CoreText resolves that name to the reserved
// PingFangUI.ttc, which has Han but no kana, while CoreText's own fallback uses
// the regular PingFang.ttc. Both files carry the same PostScript names and gpui
// identifies fallback fonts by that name alone, so glyphs shaped against one
// file were painted from the other (#730). CoreText's default cascade serves
// every CJK character from one file.
pub const APP_FONT_FALLBACKS: &[&str] = &[
    "Inter Variable",
    "Segoe UI",
    "Microsoft YaHei",
    "sans-serif",
];
pub const TERMINAL_FONT_FALLBACKS: &[&str] = &["Microsoft YaHei", "sans-serif"];
