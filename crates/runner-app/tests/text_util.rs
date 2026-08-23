//! Regressions for the composer review findings: IME marked-text
//! selection offsets are relative to the marked string, and cursor
//! navigation must respect grapheme clusters, not Unicode scalars.

use runner_app::text_util::{
    marked_selection, next_grapheme_boundary, offset_from_utf16, offset_to_utf16,
    prev_grapheme_boundary, range_from_utf16,
};

#[test]
fn utf16_offsets_round_trip_multibyte() {
    // "a中b😀c": utf16 units a=1, 中=1, b=1, 😀=2, c=1.
    let s = "a中b😀c";
    assert_eq!(offset_from_utf16(s, 0), 0);
    assert_eq!(offset_from_utf16(s, 1), 1); // after 'a'
    assert_eq!(offset_from_utf16(s, 2), 4); // after '中' (3 bytes)
    assert_eq!(offset_from_utf16(s, 3), 5); // after 'b'
    assert_eq!(offset_from_utf16(s, 5), 9); // after '😀' (4 bytes, 2 units)
    assert_eq!(offset_to_utf16(s, 9), 5);
    assert_eq!(offset_to_utf16(s, 4), 2);
}

#[test]
fn marked_selection_is_relative_to_new_text() {
    // Composition at a nonzero insertion point with a multibyte
    // prefix: content "héllo " (é = 2 bytes), marked text "拼音"
    // inserted at byte 7. The IME reports the selection 0..2 in
    // UTF-16 units RELATIVE to "拼音" — the absolute range must be
    // 7..13, not a conversion against the whole content.
    let insert_at = "héllo ".len(); // 7 bytes
    let sel = marked_selection(insert_at, "拼音", Some(&(0..2)));
    assert_eq!(sel, 7..13);

    // Caret collapsed at the end of the composition when the IME
    // sends no explicit range.
    let sel = marked_selection(insert_at, "拼音", None);
    assert_eq!(sel, 13..13);

    // Caret after the first ideograph (relative 1..1) lands mid-way.
    let sel = marked_selection(insert_at, "拼音", Some(&(1..1)));
    assert_eq!(sel, 10..10);
}

#[test]
fn range_from_utf16_against_marked_text_only() {
    assert_eq!(range_from_utf16("拼音", &(0..2)), 0..6);
    assert_eq!(range_from_utf16("nihao", &(0..5)), 0..5);
}

#[test]
fn grapheme_boundaries_keep_combining_marks_together() {
    // "e" + U+0301 is one grapheme of 3 bytes.
    let s = "e\u{301}x";
    assert_eq!(prev_grapheme_boundary(s, 3), 0);
    assert_eq!(next_grapheme_boundary(s, 0), 3);
}

#[test]
fn grapheme_boundaries_keep_zwj_emoji_together() {
    // Family emoji: 4 scalars joined by ZWJ, one grapheme (25 bytes).
    let family = "👨\u{200d}👩\u{200d}👧\u{200d}👦";
    let s = format!("a{family}b");
    let end_of_family = 1 + family.len();
    assert_eq!(prev_grapheme_boundary(&s, end_of_family), 1);
    assert_eq!(next_grapheme_boundary(&s, 1), end_of_family);
}

#[test]
fn grapheme_boundaries_keep_skin_tone_and_flags_together() {
    let thumbs = "👍🏽"; // base + skin tone modifier
    assert_eq!(next_grapheme_boundary(thumbs, 0), thumbs.len());
    assert_eq!(prev_grapheme_boundary(thumbs, thumbs.len()), 0);

    let flag = "🇨🇳"; // two regional indicators
    assert_eq!(next_grapheme_boundary(flag, 0), flag.len());
    assert_eq!(prev_grapheme_boundary(flag, flag.len()), 0);
}
