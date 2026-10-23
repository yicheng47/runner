//! Text formatting and IME offsets: UTF-16 ⇄ UTF-8 conversion
//! (NSTextInputClient speaks UTF-16 code units) and grapheme-cluster
//! boundaries (so Backspace deletes 👨‍👩‍👧‍👦 as one unit, not one code
//! point at a time).

use unicode_segmentation::UnicodeSegmentation as _;

pub fn format_relative_time(
    value: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> String {
    let minutes = now.signed_duration_since(value).num_minutes().max(0) as usize;
    if minutes == 0 {
        crate::i18n::text("ui.just_now").into()
    } else if minutes < 60 {
        crate::i18n::tnf("format.minute_ago", minutes, &[])
    } else if minutes < 1440 {
        crate::i18n::tnf("format.hour_ago", minutes / 60, &[])
    } else {
        crate::i18n::tnf("format.day_ago", minutes / 1440, &[])
    }
}

/// UTF-8 byte offset for a UTF-16 code-unit offset into `s`.
pub fn offset_from_utf16(s: &str, utf16_offset: usize) -> usize {
    let mut utf8_offset = 0;
    let mut utf16_count = 0;
    for ch in s.chars() {
        if utf16_count >= utf16_offset {
            break;
        }
        utf16_count += ch.len_utf16();
        utf8_offset += ch.len_utf8();
    }
    utf8_offset
}

/// UTF-16 code-unit offset for a UTF-8 byte offset into `s`.
pub fn offset_to_utf16(s: &str, utf8_offset: usize) -> usize {
    let mut utf16_offset = 0;
    let mut utf8_count = 0;
    for ch in s.chars() {
        if utf8_count >= utf8_offset {
            break;
        }
        utf8_count += ch.len_utf8();
        utf16_offset += ch.len_utf16();
    }
    utf16_offset
}

pub fn range_from_utf16(s: &str, range: &std::ops::Range<usize>) -> std::ops::Range<usize> {
    offset_from_utf16(s, range.start)..offset_from_utf16(s, range.end)
}

pub fn range_to_utf16(s: &str, range: &std::ops::Range<usize>) -> std::ops::Range<usize> {
    offset_to_utf16(s, range.start)..offset_to_utf16(s, range.end)
}

/// Absolute UTF-8 selection after a marked-text (IME composition)
/// update. `relative_utf16` comes from NSTextInputClient's
/// setMarkedText and is relative to `new_text`, not the content;
/// `insert_at` is the UTF-8 byte offset where `new_text` was
/// inserted. `None` collapses the caret to the end of the insertion.
pub fn marked_selection(
    insert_at: usize,
    new_text: &str,
    relative_utf16: Option<&std::ops::Range<usize>>,
) -> std::ops::Range<usize> {
    match relative_utf16 {
        Some(rel) => {
            let rel = range_from_utf16(new_text, rel);
            insert_at + rel.start..insert_at + rel.end
        }
        None => insert_at + new_text.len()..insert_at + new_text.len(),
    }
}

/// Start of the grapheme cluster ending at `offset` (0 at the start).
pub fn prev_grapheme_boundary(s: &str, offset: usize) -> usize {
    s[..offset]
        .grapheme_indices(true)
        .next_back()
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// End of the grapheme cluster starting at `offset` (len at the end).
pub fn next_grapheme_boundary(s: &str, offset: usize) -> usize {
    s[offset..]
        .graphemes(true)
        .next()
        .map(|g| offset + g.len())
        .unwrap_or(s.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_time_keeps_future_and_unit_boundaries_localized() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-23T12:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        for (locale, expected) in [
            (
                crate::i18n::Locale::English,
                [
                    "just now",
                    "just now",
                    "1 minute ago",
                    "59 minutes ago",
                    "1 hour ago",
                    "23 hours ago",
                    "1 day ago",
                ],
            ),
            (
                crate::i18n::Locale::SimplifiedChinese,
                [
                    "刚刚",
                    "刚刚",
                    "1 分钟前",
                    "59 分钟前",
                    "1 小时前",
                    "23 小时前",
                    "1 天前",
                ],
            ),
        ] {
            crate::i18n::testing::with_locale(locale, || {
                for (minutes, expected) in [-1, 0, 1, 59, 60, 1439, 1440].into_iter().zip(expected)
                {
                    assert_eq!(
                        format_relative_time(now - chrono::Duration::minutes(minutes), now),
                        expected
                    );
                }
            });
        }
    }
}
