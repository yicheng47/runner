//! Find in terminal: the query's pattern, its matches over a `Term`'s active
//! grid, and the scroll-mark helper that carries matches across output.

use alacritty_terminal::grid::Dimensions as _;
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point};
use alacritty_terminal::term::search::{Match, RegexSearch};
use alacritty_terminal::term::Term;
use unicode_width::UnicodeWidthChar as _;

/// Counts past this many read as "1000+".
pub const MATCH_LIMIT: usize = 1000;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FindQuery {
    pub text: String,
    pub match_case: bool,
    pub regex: bool,
}

pub enum FindPattern {
    Empty,
    Invalid,
    Ready(Box<RegexSearch>),
}

impl FindQuery {
    /// `RegexSearch` turns on smart case by itself, so the case flag is
    /// always written into the pattern to override it. The grid keeps
    /// zero-width characters (combining marks, variation selectors, ZWJ) on
    /// the previous cell, outside the text `RegexSearch` reads, so plain text
    /// leaves them out; a regex pattern stays as typed.
    pub fn pattern(&self) -> FindPattern {
        let body = if self.regex {
            self.text.clone()
        } else {
            let searchable = self
                .text
                .chars()
                .filter(|character| character.width() != Some(0))
                .collect::<String>();
            regex::escape(&searchable)
        };
        if body.is_empty() {
            return FindPattern::Empty;
        }
        let flag = if self.match_case { "(?-i)" } else { "(?i)" };
        let pattern = format!("{flag}{body}");
        let can_match_empty = regex_syntax::Parser::new()
            .parse(&pattern)
            .map_or(true, |hir| hir.properties().minimum_len() == Some(0));
        if can_match_empty {
            return FindPattern::Invalid;
        }
        RegexSearch::new(&pattern).map_or(FindPattern::Invalid, |regex| {
            FindPattern::Ready(Box::new(regex))
        })
    }
}

/// How many matches there are, up to `MATCH_LIMIT`, and where the current
/// one falls counting from the newest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FindCount {
    pub total: usize,
    pub more: bool,
    pub index: Option<usize>,
}

impl FindCount {
    pub fn new(newest: &[Match], more: bool, current: Option<&Match>) -> Self {
        Self {
            total: newest.len(),
            more,
            index: current.and_then(|current| newest.iter().position(|found| found == current)),
        }
    }
}

/// The newest `MATCH_LIMIT` matches over the active grid, scrollback
/// included, newest first, and whether older ones exist. On the alternate
/// screen that grid is the visible screen. The cap bounds only the count:
/// selection and stepping search the whole grid on demand.
pub fn newest_matches<T>(term: &Term<T>, regex: &mut RegexSearch) -> (Vec<Match>, bool) {
    let mut matches = Vec::new();
    let more = matches_left(
        term,
        regex,
        bottom(term),
        top(term),
        MATCH_LIMIT,
        &mut matches,
    );
    (matches, more)
}

/// Every match touching the lines `first..=last`, oldest first.
pub fn matches_in_lines<T>(
    term: &Term<T>,
    regex: &mut RegexSearch,
    first: Line,
    last: Line,
) -> Vec<Match> {
    let start = term.line_search_right(Point::new(last, term.last_column()));
    let end = term.line_search_left(Point::new(first, Column(0)));
    let mut matches = Vec::new();
    matches_left(term, regex, start, end, usize::MAX, &mut matches);
    matches.reverse();
    matches
}

/// The newest match starting at or above `line`, or the nearest one below it
/// when there is none.
pub fn newest_at_or_above<T>(term: &Term<T>, regex: &mut RegexSearch, line: Line) -> Option<Match> {
    let line = clamp_line(term, line);
    let end = top(term);
    let mut origin = term.line_search_right(Point::new(line, term.last_column()));
    while let Some(found) = term.regex_search_left(regex, origin, end) {
        if found.start().line <= line {
            return Some(found);
        }
        if *found.start() <= end {
            break;
        }
        origin = before(term, *found.start());
    }
    matches_in_lines(term, regex, line, line)
        .into_iter()
        .find(|found| found.start().line > line)
        .or_else(|| oldest_from(term, regex, next_line(term, line)?))
}

/// The next older match, or the next newer one, wrapping at the ends. A
/// current match that has left the scrollback has nothing older than it.
pub fn step_match<T>(
    term: &Term<T>,
    regex: &mut RegexSearch,
    current: &Match,
    older: bool,
) -> Option<Match> {
    let line = current.start().line;
    if line < term.topmost_line() {
        return if older {
            term.regex_search_left(regex, bottom(term), top(term))
        } else {
            oldest_from(term, regex, term.topmost_line())
        };
    }
    let same_line = matches_in_lines(term, regex, line, line);
    if older {
        if let Some(found) = same_line
            .into_iter()
            .rev()
            .find(|found| found.start() < current.start())
        {
            return Some(found);
        }
        let line_start = term.line_search_left(*current.start());
        if line_start > top(term) {
            if let Some(found) = term.regex_search_left(regex, before(term, line_start), top(term))
            {
                return Some(found);
            }
        }
        term.regex_search_left(regex, bottom(term), top(term))
    } else {
        same_line
            .into_iter()
            .find(|found| found.start() > current.start())
            .or_else(|| oldest_from(term, regex, next_line(term, line)?))
            .or_else(|| oldest_from(term, regex, term.topmost_line()))
    }
}

/// `carried`, a match moved to the current positions: the same match when
/// it is still there, otherwise the nearest one.
pub fn locate<T>(term: &Term<T>, regex: &mut RegexSearch, carried: &Match) -> Option<Match> {
    let line = carried.start().line;
    if line >= term.topmost_line() && line < Line(term.screen_lines() as i32) {
        if let Some(found) = matches_in_lines(term, regex, line, line)
            .into_iter()
            .find(|found| found == carried)
        {
            return Some(found);
        }
    }
    newest_at_or_above(term, regex, line)
}

/// The oldest match on the logical lines from the one holding `line` down.
fn oldest_from<T>(term: &Term<T>, regex: &mut RegexSearch, mut line: Line) -> Option<Match> {
    loop {
        if let Some(found) = matches_in_lines(term, regex, line, line).into_iter().next() {
            return Some(found);
        }
        line = next_line(term, line)?;
    }
}

/// The first line of the logical line after the one holding `line`.
fn next_line<T>(term: &Term<T>, line: Line) -> Option<Line> {
    let next = term
        .line_search_right(Point::new(line, term.last_column()))
        .line
        + 1;
    (next < Line(term.screen_lines() as i32)).then_some(next)
}

fn clamp_line<T>(term: &Term<T>, line: Line) -> Line {
    line.max(term.topmost_line())
        .min(Line(term.screen_lines() as i32 - 1))
}

fn top<T>(term: &Term<T>) -> Point {
    Point::new(term.topmost_line(), Column(0))
}

fn bottom<T>(term: &Term<T>) -> Point {
    Point::new(Line(term.screen_lines() as i32 - 1), term.last_column())
}

fn before<T>(term: &Term<T>, point: Point) -> Point {
    term.expand_wide(point, Direction::Left)
        .sub(term, Boundary::None, 1)
}

/// Searches leftward from `origin` to `end`, so the newest match comes
/// first. Returns whether a match beyond `limit` exists.
fn matches_left<T>(
    term: &Term<T>,
    regex: &mut RegexSearch,
    mut origin: Point,
    end: Point,
    limit: usize,
    matches: &mut Vec<Match>,
) -> bool {
    while let Some(found) = term.regex_search_left(regex, origin, end) {
        if matches.len() == limit {
            return true;
        }
        let start = *found.start();
        matches.push(found);
        if start <= end {
            break;
        }
        origin = before(term, start);
    }
    false
}

/// How far the content has moved up since the terminal was last rebuilt.
/// `epoch` changes when positions from before cannot be carried forward:
/// a resize, a reattach or a switch between the primary and alternate screens.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScrollMark {
    pub epoch: u64,
    pub lines: u64,
}

impl ScrollMark {
    pub fn advance(&mut self, moved: Option<usize>) {
        match moved {
            Some(lines) => self.lines += lines as u64,
            None => self.rebuild(),
        }
    }

    pub fn rebuild(&mut self) {
        self.epoch += 1;
    }

    /// Lines the content moved between `self` and `later`, if positions
    /// carry over.
    pub fn moved_since(self, later: ScrollMark) -> Option<i32> {
        (self.epoch == later.epoch)
            .then(|| i32::try_from(later.lines.saturating_sub(self.lines)).unwrap_or(i32::MAX))
    }
}

/// `found` moved up by `lines`.
pub fn shift_match(found: &Match, lines: i32) -> Match {
    let shift = |point: Point| Point::new(Line(point.line.0.saturating_sub(lines)), point.column);
    shift(*found.start())..=shift(*found.end())
}

/// The display offset that centres `found`, when it is not already on screen.
pub fn centring_offset(
    found: &Match,
    screen_lines: usize,
    history_lines: usize,
    display_offset: usize,
) -> Option<usize> {
    let top = -(display_offset as i32);
    let bottom = top + screen_lines as i32 - 1;
    if found.start().line.0 >= top && found.end().line.0 <= bottom {
        return None;
    }
    let middle = (found.start().line.0 + found.end().line.0).div_euclid(2);
    let offset = (screen_lines as i32 / 2 - middle).clamp(0, history_lines as i32);
    Some(offset as usize)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::term::Config;

    fn term(columns: usize, lines: usize, scrollback: usize) -> Term<VoidListener> {
        Term::new(
            Config {
                scrolling_history: scrollback,
                ..Config::default()
            },
            &TermSize::new(columns, lines),
            VoidListener,
        )
    }

    fn feed(term: &mut Term<VoidListener>, bytes: &[u8]) {
        crate::snapshot::TrackedProcessor::new().advance(term, bytes);
    }

    fn query(text: &str, match_case: bool, regex: bool) -> FindQuery {
        FindQuery {
            text: text.into(),
            match_case,
            regex,
        }
    }

    fn regex(query: &FindQuery) -> Box<RegexSearch> {
        match query.pattern() {
            FindPattern::Ready(regex) => regex,
            _ => panic!("{query:?} should build"),
        }
    }

    fn plain(text: &str) -> Box<RegexSearch> {
        regex(&query(text, false, false))
    }

    /// Every match in the grid, oldest first.
    fn all(term: &Term<VoidListener>, regex: &mut RegexSearch) -> Vec<Match> {
        let bottom = Line(term.screen_lines() as i32 - 1);
        matches_in_lines(term, regex, term.topmost_line(), bottom)
    }

    fn count(
        term: &Term<VoidListener>,
        regex: &mut RegexSearch,
        current: Option<&Match>,
    ) -> FindCount {
        let (newest, more) = newest_matches(term, regex);
        FindCount::new(&newest, more, current)
    }

    fn text(term: &Term<VoidListener>, found: &Match) -> String {
        term.bounds_to_string(*found.start(), *found.end())
    }

    fn numbered(range: std::ops::Range<usize>, word: &str) -> Vec<u8> {
        range
            .map(|line| format!("\r\n{word} {line}"))
            .collect::<String>()
            .into_bytes()
    }

    #[test]
    fn a_match_crosses_a_soft_wrap() {
        let mut term = term(10, 4, 100);
        feed(&mut term, b"build: panicked at x");
        let found = all(&term, &mut plain("panicked"));
        assert_eq!(found.len(), 1);
        assert_eq!(*found[0].start(), Point::new(Line(0), Column(7)));
        assert_eq!(*found[0].end(), Point::new(Line(1), Column(4)));
        assert_eq!(text(&term, &found[0]), "panicked");
    }

    #[test]
    fn the_count_is_capped_from_the_newest_match() {
        let mut term = term(20, 10, 3000);
        feed(&mut term, &numbered(0..1500, "hit"));
        let mut regex = plain("hit");
        let newest = newest_at_or_above(&term, &mut regex, Line(9)).unwrap();
        assert_eq!(newest.start().line, Line(9));
        let counted = count(&term, &mut regex, Some(&newest));
        assert_eq!(
            (counted.total, counted.more, counted.index),
            (MATCH_LIMIT, true, Some(0))
        );
        let older = step_match(&term, &mut regex, &newest, true).unwrap();
        assert_eq!(older.start().line, Line(8));
        assert_eq!(count(&term, &mut regex, Some(&older)).index, Some(1));

        let mut term = self::term(20, 10, 3000);
        feed(&mut term, b"hit\r\nmiss\r\nhit hit");
        let counted = count(&term, &mut regex, None);
        assert_eq!(
            (counted.total, counted.more, counted.index),
            (3, false, None)
        );
    }

    #[test]
    fn selection_past_the_cap_starts_at_an_older_viewport() {
        let mut term = term(20, 10, 3000);
        feed(&mut term, &numbered(0..1500, "hit"));
        let mut regex = plain("hit");
        let current = newest_at_or_above(&term, &mut regex, Line(-1200)).unwrap();
        assert_eq!(current.start().line, Line(-1200));
        assert_eq!(count(&term, &mut regex, Some(&current)).index, None);
        let older = step_match(&term, &mut regex, &current, true).unwrap();
        assert_eq!(older.start().line, Line(-1201));
        let newer = step_match(&term, &mut regex, &current, false).unwrap();
        assert_eq!(newer.start().line, Line(-1199));
    }

    #[test]
    fn match_case_and_regex_toggles_override_smart_case() {
        let mut term = term(40, 4, 100);
        feed(&mut term, b"FAIL fail Fail\r\nfa1l f.il");
        let count = |text: &str, match_case: bool, is_regex: bool| {
            all(&term, &mut regex(&query(text, match_case, is_regex))).len()
        };
        assert_eq!(count("fail", false, false), 3);
        assert_eq!(count("FAIL", false, false), 3);
        assert_eq!(count("fail", true, false), 1);
        assert_eq!(count("Fail", true, false), 1);
        assert_eq!(count("f.il", false, false), 1);
        assert_eq!(count("f.il", false, true), 4);
        assert_eq!(count("fa\\dl|FAIL", true, true), 2);
    }

    #[test]
    fn plain_text_queries_ignore_zero_width_characters() {
        let mut term = term(60, 4, 100);
        feed(
            &mut term,
            "cafe\u{301} cafe caf\u{e9} \u{2764}\u{fe0f} \u{1f468}\u{200d}\u{1f469}".as_bytes(),
        );
        // The decomposed é also matches a plain e, as accepted; not U+00E9.
        assert_eq!(all(&term, &mut plain("cafe\u{301}")).len(), 2);
        assert_eq!(all(&term, &mut plain("caf\u{e9}")).len(), 1);
        assert_eq!(all(&term, &mut plain("\u{2764}\u{fe0f}")).len(), 1);
        let family = all(&term, &mut plain("\u{1f468}\u{200d}\u{1f469}"));
        assert_eq!(family.len(), 1);
        assert_eq!(text(&term, &family[0]), "\u{1f468}\u{200d}\u{1f469}");
        for marks in ["\u{301}", "\u{fe0f}\u{200d}"] {
            assert!(matches!(
                query(marks, false, false).pattern(),
                FindPattern::Empty
            ));
        }
        assert_eq!(
            all(&term, &mut regex(&query("cafe\u{301}", false, true))).len(),
            0
        );
        assert_eq!(all(&term, &mut regex(&query("cafe", false, true))).len(), 2);
    }

    #[test]
    fn plain_queries_escape_regex_syntax() {
        let mut term = term(40, 4, 100);
        feed(&mut term, b"a+b (x) [y] a.b axb");
        for literal in ["a+b", "(x)", "[y]", "a.b"] {
            let found = all(&term, &mut plain(literal));
            assert_eq!(found.len(), 1, "{literal}");
            assert_eq!(text(&term, &found[0]), literal);
        }
        assert!(matches!(
            query("(", false, false).pattern(),
            FindPattern::Ready(_)
        ));
    }

    #[test]
    fn invalid_and_empty_matching_patterns_are_rejected() {
        assert!(matches!(
            query("", false, true).pattern(),
            FindPattern::Empty
        ));
        for pattern in ["(", "[a-", "a{2,1}", "a*", "x?", "\\b", "^", "(|a)"] {
            assert!(
                matches!(query(pattern, false, true).pattern(), FindPattern::Invalid),
                "{pattern}"
            );
        }
        assert!(matches!(
            query("a+", false, true).pattern(),
            FindPattern::Ready(_)
        ));
    }

    #[test]
    fn the_alternate_screen_searches_only_the_visible_screen() {
        let mut term = term(20, 4, 100);
        feed(&mut term, b"needle 1\r\nneedle 2\r\n\r\n\r\n\r\n\r\nend");
        let mut regex = plain("needle");
        assert_eq!(count(&term, &mut regex, None).total, 2);
        feed(&mut term, b"\x1b[?1049h\x1b[Hneedle on screen");
        assert_eq!(count(&term, &mut regex, None).total, 1);
        let only = newest_at_or_above(&term, &mut regex, Line(3)).unwrap();
        assert_eq!(only.start().line, Line(0));
    }

    #[test]
    fn matches_in_lines_include_one_that_starts_above_them() {
        let mut term = term(10, 6, 100);
        feed(&mut term, b"\r\nxxxxxxxpanicked\r\npanicked");
        let found = matches_in_lines(&term, &mut plain("panicked"), Line(2), Line(5));
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].start().line, Line(1));
        assert_eq!(found[1].start().line, Line(3));
    }

    #[test]
    fn stepping_visits_every_match_in_order_and_wraps() {
        let mut term = term(12, 8, 100);
        feed(
            &mut term,
            b"hit a hit\r\nmiss\r\nxxxxxxxxhit hit\r\n\r\nhit",
        );
        let mut regex = plain("hit");
        let expected = all(&term, &mut regex);
        assert_eq!(expected.len(), 5);
        let mut current = newest_at_or_above(&term, &mut regex, Line(7)).unwrap();
        assert_eq!(current, expected[4]);
        let mut visited = vec![current.clone()];
        for _ in 0..5 {
            current = step_match(&term, &mut regex, &current, true).unwrap();
            visited.push(current.clone());
        }
        let mut older_first = expected.clone();
        older_first.reverse();
        older_first.push(expected[4].clone());
        assert_eq!(visited, older_first);
        current = expected[0].clone();
        for expected_newer in expected.iter().skip(1).chain(expected.first()) {
            current = step_match(&term, &mut regex, &current, false).unwrap();
            assert_eq!(&current, expected_newer);
        }
    }

    #[test]
    fn a_new_query_falls_back_to_the_nearest_match_below_the_viewport() {
        let mut term = term(10, 6, 100);
        feed(&mut term, b"\r\nxxxxxxxxxxxxhit\r\n\r\nhit");
        let mut regex = plain("hit");
        let found = newest_at_or_above(&term, &mut regex, Line(1)).unwrap();
        assert_eq!(found.start().line, Line(2));
        assert_eq!(newest_at_or_above(&term, &mut regex, Line(2)), Some(found));
    }

    #[test]
    fn locate_keeps_a_surviving_match_and_otherwise_takes_the_nearest() {
        let mut term = term(20, 6, 100);
        feed(&mut term, b"hit 1\r\nhit 2\r\nhit 3");
        let mut regex = plain("hit");
        let second = newest_at_or_above(&term, &mut regex, Line(1)).unwrap();
        assert_eq!(locate(&term, &mut regex, &second), Some(second.clone()));
        let gone = shift_match(&second, -3);
        assert_eq!(
            locate(&term, &mut regex, &gone).unwrap().start().line,
            Line(2)
        );
        let above = shift_match(&second, 50);
        assert_eq!(
            locate(&term, &mut regex, &above).unwrap().start().line,
            Line(0)
        );
    }

    #[test]
    fn stepping_from_a_match_that_left_the_scrollback_wraps() {
        let mut term = term(20, 8, 120);
        feed(&mut term, &numbered(0..200, "hit"));
        let mut regex = plain("hit");
        let oldest = newest_at_or_above(&term, &mut regex, term.topmost_line()).unwrap();
        assert_eq!(oldest.start().line, Line(-120));
        feed(&mut term, &numbered(0..1, "hit"));
        let evicted = shift_match(&oldest, 1);
        assert_eq!(evicted.start().line, Line(-121));
        let newer = step_match(&term, &mut regex, &evicted, false).unwrap();
        assert_eq!(newer.start().line, Line(-120));
        let older = step_match(&term, &mut regex, &evicted, true).unwrap();
        assert_eq!(older.start().line, Line(7));
        assert_eq!(
            locate(&term, &mut regex, &evicted).unwrap().start().line,
            Line(-120)
        );
    }

    #[test]
    fn centring_scrolls_only_for_an_off_screen_match() {
        let at = |line: i32| Point::new(Line(line), Column(0))..=Point::new(Line(line), Column(3));
        assert_eq!(centring_offset(&at(3), 10, 100, 0), None);
        assert_eq!(centring_offset(&at(-5), 10, 100, 8), None);
        assert_eq!(centring_offset(&at(-20), 10, 100, 0), Some(25));
        assert_eq!(centring_offset(&at(-98), 10, 100, 0), Some(100));
        assert_eq!(centring_offset(&at(8), 10, 100, 50), Some(0));
    }

    /// Lines `bytes` moved the content, measured the way the mirror does.
    fn moved(term: &mut Term<VoidListener>, bytes: &[u8]) -> Option<usize> {
        let mut graphics = crate::kitty::TerminalGraphics::mirror();
        let sgr_pixels = std::sync::atomic::AtomicBool::new(false);
        let mut tracker = crate::terminal::stream::StreamHandler::new(
            term,
            &mut graphics,
            &sgr_pixels,
            crate::kitty::CellSize::ZERO,
            None,
        );
        crate::snapshot::TrackedProcessor::new().advance(&mut tracker, bytes);
        tracker.finish()
    }

    /// Feeds the chunks the way the mirror does and carries `found` along.
    fn follow(term: &mut Term<VoidListener>, found: &Match, chunks: &[Vec<u8>]) -> Match {
        let mut mark = ScrollMark::default();
        let start = mark;
        for chunk in chunks {
            mark.advance(moved(term, chunk));
        }
        shift_match(found, start.moved_since(mark).unwrap())
    }

    #[test]
    fn the_current_match_keeps_its_text_while_output_scrolls() {
        // Not full; full throughout; and filling up across the ring buffer's
        // last reallocation, which happens inside the single chunk below.
        for (before, after, scrollback) in [(4, 40, 2000), (200, 100, 120), (1900, 700, 2500)] {
            for chunked in [true, false] {
                let mut term = term(30, 8, scrollback);
                feed(&mut term, &numbered(0..before, "out"));
                feed(&mut term, b"\r\nthe needle line");
                let mut regex = plain("needle");
                let found = newest_at_or_above(&term, &mut regex, Line(7)).unwrap();
                let chunks = if chunked {
                    (0..after)
                        .map(|line| numbered(line..line + 1, "out"))
                        .collect::<Vec<_>>()
                } else {
                    vec![numbered(0..after, "out")]
                };
                let carried = follow(&mut term, &found, &chunks);
                assert_eq!(locate(&term, &mut regex, &carried), Some(carried.clone()));
                assert_eq!(
                    text(&term, &carried),
                    "needle",
                    "{before} {after} {chunked}"
                );
                assert_eq!(
                    term.history_size() == scrollback,
                    before + after > scrollback
                );
            }
        }
    }

    #[test]
    fn a_burst_past_the_scrollback_keeps_the_rows_that_survive() {
        let mut term = term(30, 8, 120);
        feed(&mut term, &numbered(0..200, "line"));
        feed(&mut term, b"\r\nold needle");
        let mut regex = plain("needle");
        let found = newest_at_or_above(&term, &mut regex, Line(7)).unwrap();
        assert_eq!(found.start().line, Line(7));
        let mut burst = numbered(0..60, "more");
        burst.extend_from_slice(b"\r\nnew needle");
        burst.extend_from_slice(&numbered(60..120, "more"));
        let carried = follow(&mut term, &found, &[burst]);
        assert_eq!(carried.start().line, Line(-114));
        assert_eq!(locate(&term, &mut regex, &carried), Some(carried.clone()));
        assert_eq!(
            term.bounds_to_string(Point::new(Line(-114), Column(0)), *carried.end()),
            "old needle"
        );
        assert_eq!(count(&term, &mut regex, Some(&carried)).index, Some(1));
    }

    #[test]
    fn the_tracker_counts_every_line_past_the_ring_buffers_length() {
        // With 120 lines of scrollback on 8 rows the ring holds 1008 rows, so
        // 1008 and 1010 lines bring the same row allocations back to the top.
        for burst in [121, 200, 1004, 1008, 1010] {
            let mut term = term(30, 8, 120);
            feed(&mut term, &numbered(0..200, "line"));
            assert_eq!(moved(&mut term, &numbered(0..burst, "more")), Some(burst));
        }
        // The default scrollback on 24 rows: the ring holds 10,024 rows.
        for burst in [10_020, 10_024, 10_026] {
            let mut term = term(80, 24, 10_000);
            feed(&mut term, &numbered(0..10_100, "line"));
            assert_eq!(moved(&mut term, &numbered(0..burst, "more")), Some(burst));
        }
    }

    #[test]
    fn the_tracker_counts_a_synchronized_update_and_skips_the_other_screen() {
        let full = || {
            let mut term = term(30, 8, 120);
            feed(&mut term, &numbered(0..200, "line"));
            term
        };
        let mut term = full();
        let mut update = b"\x1b[?2026h".to_vec();
        update.extend_from_slice(&numbered(0..1010, "more"));
        update.extend_from_slice(b"\x1b[?2026l");
        assert_eq!(moved(&mut term, &update), Some(1010));

        // An update left open is applied when the flusher stops it.
        let mut term = full();
        let mut processor = crate::snapshot::TrackedProcessor::new();
        let mut graphics = crate::kitty::TerminalGraphics::mirror();
        let sgr_pixels = std::sync::atomic::AtomicBool::new(false);
        let mut open = b"\x1b[?2026h".to_vec();
        open.extend_from_slice(&numbered(0..1010, "more"));
        let mut tracker = crate::terminal::stream::StreamHandler::new(
            &mut term,
            &mut graphics,
            &sgr_pixels,
            crate::kitty::CellSize::ZERO,
            None,
        );
        processor.advance(&mut tracker, &open);
        assert_eq!(tracker.finish(), Some(0));
        let mut tracker = crate::terminal::stream::StreamHandler::new(
            &mut term,
            &mut graphics,
            &sgr_pixels,
            crate::kitty::CellSize::ZERO,
            None,
        );
        processor.stop_sync(&mut tracker);
        assert_eq!(tracker.finish(), Some(1010));

        let mut term = full();
        let mut excursion = numbered(0..3, "more");
        excursion.extend_from_slice(b"\x1b[?1049h");
        excursion.extend_from_slice(&numbered(0..50, "alt"));
        excursion.extend_from_slice(b"\x1b[?1049l");
        excursion.extend_from_slice(&numbered(0..2, "more"));
        assert_eq!(moved(&mut term, &excursion), Some(5));

        // A tab after a full bottom row wraps onto a new line.
        let mut term = self::term(10, 8, 100);
        assert_eq!(moved(&mut term, b"\x1b[8;1Habcdefghij"), Some(0));
        assert_eq!(moved(&mut term, b"\t"), Some(1), "tab");
        assert_eq!(term.history_size(), 1);

        let mut term = full();
        assert_eq!(
            moved(&mut term, b"\x1b[H\x1bM\x1bM"),
            Some(0),
            "reverse index"
        );
        let mut term = full();
        feed(&mut term, b"\x1b[?1049h\x1b[8H");
        assert_eq!(moved(&mut term, b"\n\n"), Some(2), "alternate screen");
    }

    #[test]
    fn the_tracker_leaves_the_terminal_as_a_plain_parse_does() {
        use crate::fixtures::{decode_chunk, Fixture, FixtureEvent};
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
        let mut paths = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "ndjson"))
            .collect::<Vec<_>>();
        paths.sort();
        assert!(paths.len() > 20);
        for path in paths {
            let fixture = Fixture::load(&path).unwrap();
            let (cols, rows) = (fixture.header.cols as usize, fixture.header.rows as usize);
            let mut plain = term(cols, rows, 10_000);
            let mut tracked = term(cols, rows, 10_000);
            let mut graphics = crate::kitty::TerminalGraphics::mirror();
            let sgr_pixels = std::sync::atomic::AtomicBool::new(false);
            let mut plain_parser = crate::snapshot::TrackedProcessor::new();
            let mut tracked_parser = crate::snapshot::TrackedProcessor::new();
            for event in &fixture.events {
                match event {
                    FixtureEvent::Data { data, .. } => {
                        let bytes = decode_chunk(data).unwrap();
                        plain_parser.advance(&mut plain, &bytes);
                        let mut handler = crate::terminal::stream::StreamHandler::new(
                            &mut tracked,
                            &mut graphics,
                            &sgr_pixels,
                            crate::kitty::CellSize::ZERO,
                            None,
                        );
                        tracked_parser.advance(&mut handler, &bytes);
                    }
                    FixtureEvent::Resize { cols, rows, .. } => {
                        let size = || TermSize::new(*cols as usize, *rows as usize);
                        plain.resize(size());
                        crate::terminal::stream::resize_term_and_graphics(
                            &mut tracked,
                            &mut graphics,
                            *cols as usize,
                            *rows as usize,
                            crate::kitty::CellSize::ZERO,
                        );
                    }
                    _ => {}
                }
            }
            assert_eq!(
                crate::snapshot::serialize(&tracked, &[]),
                crate::snapshot::serialize(&plain, &[]),
                "{}",
                path.display()
            );
        }
    }

    #[test]
    fn the_current_match_survives_a_thousand_newer_matches() {
        let mut term = term(20, 10, 3000);
        feed(&mut term, &numbered(0..20, "hit"));
        let mut regex = plain("hit");
        let found = newest_at_or_above(&term, &mut regex, Line(5)).unwrap();
        let chunks = (0..1100)
            .map(|line| numbered(line..line + 1, "hit"))
            .collect::<Vec<_>>();
        let carried = follow(&mut term, &found, &chunks);
        assert_eq!(carried.start().line, Line(5 - 1100));
        assert_eq!(locate(&term, &mut regex, &carried), Some(carried.clone()));
        let count = count(&term, &mut regex, Some(&carried));
        assert_eq!((count.more, count.index), (true, None));
    }

    #[test]
    fn the_tracker_reports_a_switch_of_screens_and_a_scroll_region() {
        let mut term = term(20, 6, 100);
        feed(&mut term, b"a\r\nb\r\nc");
        assert_eq!(moved(&mut term, b"\x1b[?1049h"), None);

        let mut term = self::term(20, 6, 100);
        feed(&mut term, b"header\x1b[2;6r\x1b[6H");
        assert_eq!(moved(&mut term, b"\r\nx\r\ny"), Some(0));
    }
}
