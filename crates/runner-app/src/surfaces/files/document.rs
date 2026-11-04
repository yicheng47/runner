//! A file's text as the viewer reads it: decoded once off the UI thread into
//! lines, laid out on a grid of monospace cells, and searched by find.
//!
//! Every character takes the cells `unicode-width` gives it (a CJK character
//! two, a combining mark none, a tab up to the next stop), so drawing, hit
//! testing, find and selection all agree on where a character is. A long
//! line keeps a checkpoint every few hundred characters, so reaching the
//! columns in view never walks or copies the whole line.

use std::collections::HashMap;
use std::ops::Range;

use unicode_width::UnicodeWidthChar as _;

use super::highlight::Span;

/// A tab advances to the next multiple of this many columns.
pub(crate) const TAB_WIDTH: usize = 4;
/// Lines longer than this many bytes get checkpoints.
const LONG_LINE: usize = 512;
const CHECKPOINT_EVERY: usize = 256;

/// Find counts past this many read as "1000+", as in the terminal.
pub(crate) use runner_terminal::terminal::find::MATCH_LIMIT;

/// A place in the text: a line and a byte offset into it.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) struct Position {
    pub(crate) line: usize,
    pub(crate) offset: usize,
}

/// A run of the cells in view that share a width, as one string to shape:
/// tabs become spaces, and a combining mark stays with its base.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Segment {
    pub(crate) column: usize,
    /// Cells per character: 1, or 2 for a wide one.
    pub(crate) cells: usize,
    pub(crate) text: String,
    /// `text` in runs of one highlight capture: each run's length in bytes
    /// and its capture, if any.
    pub(crate) runs: Vec<(usize, Option<u16>)>,
}

/// The cells `character` at `column` and byte `offset` takes. A combining
/// mark with nothing before it takes a cell of its own.
fn advance(character: char, column: usize, offset: usize) -> usize {
    match character {
        '\t' => TAB_WIDTH - column % TAB_WIDTH,
        _ => match character.width() {
            Some(0) if offset == 0 => 1,
            Some(width) => width,
            None => 1,
        },
    }
}

pub(crate) struct Document {
    text: String,
    /// Each line's bytes in `text`, without its `\n` or a `\r` before it.
    lines: Vec<Range<usize>>,
    /// The widest line in cells.
    columns: usize,
    /// For a long line, its (byte offset, column) every `CHECKPOINT_EVERY`
    /// characters, from its start.
    checkpoints: HashMap<usize, Vec<(usize, usize)>>,
}

impl Document {
    /// Decodes `bytes`, replacing what is not UTF-8, and indexes its lines.
    /// Slow on a large file: run it off the UI thread.
    pub(crate) fn decode(bytes: &[u8]) -> Self {
        let text = String::from_utf8_lossy(bytes).into_owned();
        let mut lines = Vec::new();
        let mut columns = 0;
        let mut checkpoints = HashMap::new();
        let mut start = 0;
        for line in text.split('\n') {
            let end = start + line.len();
            let shown = line.strip_suffix('\r').unwrap_or(line);
            let long = shown.len() > LONG_LINE;
            let mut marks = Vec::new();
            let mut column = 0;
            for (index, (offset, character)) in shown.char_indices().enumerate() {
                if long && index % CHECKPOINT_EVERY == 0 {
                    marks.push((offset, column));
                }
                column += advance(character, column, offset);
            }
            columns = columns.max(column);
            if long {
                checkpoints.insert(lines.len(), marks);
            }
            lines.push(start..start + shown.len());
            start = end + 1;
        }
        if lines.len() > 1 && lines.last().is_some_and(Range::is_empty) {
            lines.pop();
        }
        Self {
            text,
            lines,
            columns,
            checkpoints,
        }
    }

    pub(crate) fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub(crate) fn line(&self, index: usize) -> &str {
        self.lines
            .get(index)
            .map_or("", |range| &self.text[range.clone()])
    }

    /// The whole decoded text, with its line endings.
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// Where `index`'s line sits in [`Self::text`], without its ending.
    pub(crate) fn line_range(&self, index: usize) -> Range<usize> {
        self.lines.get(index).cloned().unwrap_or_default()
    }

    pub(crate) fn columns(&self) -> usize {
        self.columns
    }

    pub(crate) fn end(&self) -> Position {
        let line = self.lines.len().saturating_sub(1);
        Position {
            line,
            offset: self.line(line).len(),
        }
    }

    /// `position` moved onto the text: a line that exists and a character
    /// boundary inside it.
    pub(crate) fn clamp(&self, position: Position) -> Position {
        let line = position.line.min(self.lines.len().saturating_sub(1));
        let text = self.line(line);
        let mut offset = position.offset.min(text.len());
        while !text.is_char_boundary(offset) {
            offset -= 1;
        }
        Position { line, offset }
    }

    /// The text from `start` to `end`, lines joined by `\n`.
    pub(crate) fn text_between(&self, start: Position, end: Position) -> String {
        let (start, end) = (self.clamp(start.min(end)), self.clamp(start.max(end)));
        if start.line == end.line {
            return self.line(start.line)[start.offset..end.offset].to_owned();
        }
        let mut text = self.line(start.line)[start.offset..].to_owned();
        for line in start.line + 1..end.line {
            text.push('\n');
            text.push_str(self.line(line));
        }
        text.push('\n');
        text.push_str(&self.line(end.line)[..end.offset]);
        text
    }

    /// The last checkpoint at or before `key` of (byte offset, column).
    fn checkpoint(
        &self,
        line: usize,
        key: impl Fn(&(usize, usize)) -> usize,
        at: usize,
    ) -> (usize, usize) {
        let Some(marks) = self.checkpoints.get(&line) else {
            return (0, 0);
        };
        let index = marks.partition_point(|mark| key(mark) <= at);
        marks[index.saturating_sub(1)]
    }

    /// The characters of `line` from the checkpoint before `column`, each
    /// with its byte offset, column and cells.
    fn cells_from(
        &self,
        line: usize,
        column: usize,
    ) -> impl Iterator<Item = (usize, usize, usize, char)> + '_ {
        let text = self.line(line);
        let (offset, start) = self.checkpoint(line, |mark| mark.1, column);
        let mut at = start;
        text[offset..]
            .char_indices()
            .map(move |(index, character)| {
                let offset = offset + index;
                let column = at;
                let cells = advance(character, column, offset);
                at += cells;
                (offset, column, cells, character)
            })
    }

    /// The column `offset` starts at in `line`.
    pub(crate) fn column_at(&self, line: usize, offset: usize) -> usize {
        let offset = self.clamp(Position { line, offset }).offset;
        let text = self.line(line);
        let (from, column) = self.checkpoint(line, |mark| mark.0, offset);
        let mut column = column;
        for (index, character) in text[from..offset].char_indices() {
            column += advance(character, column, from + index);
        }
        column
    }

    /// The character boundary in `line` nearest the fractional `column`,
    /// never between a character and the marks combining with it.
    pub(crate) fn offset_near(&self, line: usize, column: f32) -> usize {
        let text = self.line(line);
        let floor = column.max(0.) as usize;
        let mut cells = self.cells_from(line, floor.saturating_sub(2)).peekable();
        while let Some((offset, start, width, _)) = cells.next() {
            if width == 0 {
                continue;
            }
            let mut end = text.len();
            while let Some(&(next, _, next_width, _)) = cells.peek() {
                if next_width != 0 {
                    end = next;
                    break;
                }
                cells.next();
            }
            if column < start as f32 + width as f32 / 2. {
                return offset;
            }
            if column < (start + width) as f32 {
                return end;
            }
        }
        text.len()
    }

    /// The cells of `line` from `from` to `to`, as runs to shape, split
    /// where `spans` (the line's highlights) change capture: the column the
    /// first starts at may be before `from`, when a wide character or a tab
    /// straddles it.
    pub(crate) fn segments(
        &self,
        line: usize,
        from: usize,
        to: usize,
        spans: &[Span],
    ) -> Vec<Segment> {
        let mut segments: Vec<Segment> = Vec::new();
        let mut span = 0;
        for (offset, column, cells, character) in self.cells_from(line, from) {
            if column >= to {
                break;
            }
            if cells == 0 {
                if let Some(last) = segments.last_mut() {
                    last.text.push(character);
                    if let Some(run) = last.runs.last_mut() {
                        run.0 += character.len_utf8();
                    }
                }
                continue;
            }
            if column + cells <= from {
                continue;
            }
            while spans
                .get(span)
                .is_some_and(|found| found.end as usize <= offset)
            {
                span += 1;
            }
            let capture = spans
                .get(span)
                .filter(|found| found.start as usize <= offset)
                .map(|found| found.capture);
            let wide = if character == '\t' { 1 } else { cells };
            let same = segments.last().is_some_and(|last| last.cells == wide);
            if !same {
                segments.push(Segment {
                    column,
                    cells: wide,
                    text: String::new(),
                    runs: Vec::new(),
                });
            }
            let last = segments.last_mut().expect("a segment");
            let before = last.text.len();
            if character == '\t' {
                last.text.extend(std::iter::repeat_n(' ', cells));
            } else {
                last.text.push(character);
            }
            let added = last.text.len() - before;
            match last.runs.last_mut() {
                Some(run) if run.1 == capture => run.0 += added,
                _ => last.runs.push((added, capture)),
            }
        }
        segments
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) struct FileQuery {
    pub(crate) text: String,
    pub(crate) match_case: bool,
    pub(crate) regex: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FileMatch {
    pub(crate) line: usize,
    /// Bytes in the line.
    pub(crate) range: Range<usize>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum Found {
    #[default]
    Empty,
    Invalid,
    /// The first matches, up to the limit, and whether there are more.
    Matches(Vec<FileMatch>, bool),
}

/// Every match of `query` in `document`, a line at a time, as the
/// terminal's find reads its rows. Slow on a large file: run it off the UI
/// thread.
pub(crate) fn find(document: &Document, query: &FileQuery) -> Found {
    if query.text.is_empty() {
        return Found::Empty;
    }
    let body = if query.regex {
        query.text.clone()
    } else {
        regex::escape(&query.text)
    };
    let flag = if query.match_case { "(?-i)" } else { "(?i)" };
    let Ok(pattern) = regex::Regex::new(&format!("{flag}{body}")) else {
        return Found::Invalid;
    };
    if pattern.is_match("") {
        return Found::Invalid;
    }
    let mut matches = Vec::new();
    for line in 0..document.line_count() {
        for found in pattern.find_iter(document.line(line)) {
            if found.is_empty() {
                continue;
            }
            if matches.len() == MATCH_LIMIT {
                return Found::Matches(matches, true);
            }
            matches.push(FileMatch {
                line,
                range: found.range(),
            });
        }
    }
    Found::Matches(matches, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(document: &Document) -> Vec<&str> {
        (0..document.line_count())
            .map(|line| document.line(line))
            .collect()
    }

    #[test]
    fn lines_drop_their_endings_and_crlf_leaves_no_stray_character() {
        let document = Document::decode(b"one\r\ntwo\n\nthree\n");
        assert_eq!(lines(&document), ["one", "two", "", "three"]);
        let unterminated = Document::decode(b"a\nb");
        assert_eq!(lines(&unterminated), ["a", "b"]);
        let empty = Document::decode(b"");
        assert_eq!(empty.line_count(), 1);
        assert_eq!(empty.line(0), "");
    }

    #[test]
    fn text_that_is_not_utf8_is_decoded_lossily() {
        let document = Document::decode(b"ok \xff\xfe end\nnext");
        assert_eq!(document.line(0), "ok \u{fffd}\u{fffd} end");
        assert_eq!(document.line(1), "next");
    }

    #[test]
    fn characters_take_their_cells_and_offsets_map_both_ways() {
        let document = Document::decode("a\tbc\td\n汉字x\ne\u{301}f".as_bytes());
        assert_eq!(document.columns(), 9);
        // a, then a tab to column 4, then bc, a tab to 8 and d.
        assert_eq!(document.column_at(0, 2), 4);
        assert_eq!(document.column_at(0, 5), 8);
        assert_eq!(document.offset_near(0, 4.2), 2);
        assert_eq!(document.offset_near(0, 1.2), 1);
        assert_eq!(document.offset_near(0, 3.), 2, "past the tab's middle");
        assert_eq!(document.offset_near(0, 99.), 6);
        // Wide characters take two cells.
        assert_eq!(document.column_at(1, 3), 2);
        assert_eq!(document.column_at(1, 6), 4);
        assert_eq!(document.offset_near(1, 2.6), 3);
        assert_eq!(document.offset_near(1, 3.1), 6);
        // A combining mark takes none and never splits from its base.
        assert_eq!(document.column_at(2, 3), 1);
        assert_eq!(document.offset_near(2, 0.8), 3);
        assert_eq!(document.offset_near(2, 1.2), 3);
        assert_eq!(
            document.segments(0, 0, 99, &[]),
            [Segment {
                column: 0,
                cells: 1,
                text: "a   bc  d".into(),
                runs: vec![(9, None)],
            }]
        );
        assert_eq!(
            document.segments(1, 1, 99, &[]),
            [
                Segment {
                    column: 0,
                    cells: 2,
                    text: "汉字".into(),
                    runs: vec![(6, None)],
                },
                Segment {
                    column: 4,
                    cells: 1,
                    text: "x".into(),
                    runs: vec![(1, None)],
                }
            ]
        );
        assert_eq!(document.segments(2, 0, 99, &[])[0].text, "e\u{301}f");
    }

    #[test]
    fn a_long_line_reaches_its_columns_from_checkpoints() {
        let line = "汉\te\u{301}x".repeat(5000);
        let document = Document::decode(line.as_bytes());
        let walked = |offset: usize| {
            let mut column = 0;
            for (index, character) in line[..offset].char_indices() {
                column += advance(character, column, index);
            }
            column
        };
        for offset in [0, 7, 3001, 20_000, line.len() - 1, line.len()] {
            let offset = document.clamp(Position { line: 0, offset }).offset;
            assert_eq!(document.column_at(0, offset), walked(offset), "{offset}");
        }
        let column = walked(30_000);
        let near = document.offset_near(0, column as f32);
        assert_eq!(document.column_at(0, near), column);
        let segments = document.segments(0, column, column + 40, &[]);
        let shown: usize = segments
            .iter()
            .map(|segment| segment.text.chars().count())
            .sum();
        assert!(shown < 80, "only the columns in view: {shown}");
        assert!(segments[0].column <= column && segments[0].column + 4 > column);
    }

    #[test]
    fn a_selection_copies_across_lines_and_clamps_to_characters() {
        let document = Document::decode("first\nsécond\nthird\n".as_bytes());
        let at = |line, offset| Position { line, offset };
        assert_eq!(
            document.text_between(at(0, 2), at(2, 3)),
            "rst\nsécond\nthi"
        );
        assert_eq!(
            document.text_between(at(2, 3), at(0, 2)),
            "rst\nsécond\nthi"
        );
        assert_eq!(document.text_between(at(1, 0), at(1, 3)), "sé");
        assert_eq!(
            document.text_between(at(0, 0), at(9, 99)),
            "first\nsécond\nthird"
        );
        assert_eq!(document.end(), at(2, 5));
    }

    #[test]
    fn find_matches_per_line_with_case_regex_and_the_limit() {
        let document = Document::decode(b"Client client\nno\nCLIENT\n");
        let query = |text: &str, match_case, regex| FileQuery {
            text: text.into(),
            match_case,
            regex,
        };
        let Found::Matches(found, false) = find(&document, &query("client", false, false)) else {
            panic!("matches");
        };
        assert_eq!(
            found
                .iter()
                .map(|m| (m.line, m.range.clone()))
                .collect::<Vec<_>>(),
            [(0, 0..6), (0, 7..13), (2, 0..6)]
        );
        let Found::Matches(found, _) = find(&document, &query("client", true, false)) else {
            panic!("matches");
        };
        assert_eq!(found.len(), 1);
        let Found::Matches(found, _) = find(&document, &query("C.i", true, true)) else {
            panic!("matches");
        };
        assert_eq!(found.len(), 1);
        assert_eq!(
            find(&document, &query("a.b", false, false)),
            Found::Matches(vec![], false)
        );
        assert_eq!(find(&document, &query("(", false, true)), Found::Invalid);
        assert_eq!(find(&document, &query("x*", false, true)), Found::Invalid);
        assert_eq!(find(&document, &query("", false, false)), Found::Empty);
        let many = Document::decode("x\n".repeat(MATCH_LIMIT + 5).as_bytes());
        let Found::Matches(found, more) = find(&many, &query("x", false, false)) else {
            panic!("matches");
        };
        assert_eq!((found.len(), more), (MATCH_LIMIT, true));
    }
}
