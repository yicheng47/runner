//! Find in terminal: the query's pattern, its matches over a screen's active
//! grid, and the scroll-mark tracker backed by `libghostty-vt`'s native
//! per-screen scroll-up row counter.

use std::sync::Arc;

use regex::Regex;
use regex_automata::hybrid::dfa::{
    Builder as HybridBuilder, Cache as HybridCache, Config as HybridConfig, DFA as HybridDfa,
};
use regex_automata::nfa::thompson::Config as ThompsonConfig;
use regex_automata::{Anchored, Input, MatchKind};
use unicode_width::UnicodeWidthChar as _;

use crate::screen::{ScreenGuard, ScreenView};
use crate::types::{CellFlags, Column, FindMatch, GridPoint, Line};
use crate::vt::{ActiveScreen, Terminal as VtTerminal};

/// Counts past this many read as "1000+".
pub const MATCH_LIMIT: usize = 1000;

#[derive(Debug)]
struct CompiledFindRegex {
    prefilter: Regex,
    rev_dfa: HybridDfa,
    fwd_dfa: HybridDfa,
}

/// Compiled terminal search pattern.
#[derive(Clone, Debug)]
pub struct FindRegex {
    compiled: Arc<CompiledFindRegex>,
}

#[derive(Default)]
struct FindSearchScratch {
    text: String,
    map: Vec<(usize, GridPoint, bool)>,
    caches: Option<(HybridCache, HybridCache)>,
}

impl FindSearchScratch {
    fn caches_for(
        &mut self,
        rev_dfa: &HybridDfa,
        fwd_dfa: &HybridDfa,
    ) -> (&mut HybridCache, &mut HybridCache) {
        let (rev, fwd) = self
            .caches
            .get_or_insert_with(|| (rev_dfa.create_cache(), fwd_dfa.create_cache()));
        (rev, fwd)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FindQuery {
    pub text: String,
    pub match_case: bool,
    pub regex: bool,
}

pub enum FindPattern {
    Empty,
    Invalid,
    Ready(Box<FindRegex>),
}

impl FindQuery {
    /// The grid keeps zero-width characters (combining marks, variation
    /// selectors, ZWJ) attached to the previous cell, outside the base cell
    /// text searched here, so plain text leaves them out; a regex pattern
    /// stays as typed.
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
        let Ok(prefilter) = Regex::new(&pattern) else {
            return FindPattern::Invalid;
        };
        let config = HybridConfig::new()
            .minimum_cache_clear_count(Some(3))
            .minimum_bytes_per_state(Some(10));
        let max_size = config.get_cache_capacity();
        let Ok(fwd_dfa) = HybridBuilder::new()
            .configure(config.clone().match_kind(MatchKind::All))
            .thompson(
                ThompsonConfig::new()
                    .reverse(false)
                    .nfa_size_limit(Some(max_size)),
            )
            .build(&pattern)
        else {
            return FindPattern::Invalid;
        };
        let Ok(rev_dfa) = HybridBuilder::new()
            .configure(config.match_kind(MatchKind::LeftmostFirst))
            .thompson(
                ThompsonConfig::new()
                    .reverse(true)
                    .nfa_size_limit(Some(max_size)),
            )
            .build(&pattern)
        else {
            return FindPattern::Invalid;
        };
        FindPattern::Ready(Box::new(FindRegex {
            compiled: Arc::new(CompiledFindRegex {
                prefilter,
                rev_dfa,
                fwd_dfa,
            }),
        }))
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
    pub fn new(newest: &[FindMatch], more: bool, current: Option<&FindMatch>) -> Self {
        Self {
            total: newest.len(),
            more,
            index: current.and_then(|current| newest.iter().position(|found| found == current)),
        }
    }
}

/// Which selection transition a [`TerminalMirror::find_matches`] request performs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FindAction {
    /// Start from the newest match at or above the current viewport bottom.
    NewQuery,
    /// Keep the anchored/carried match when still present, else snap to nearest.
    Refresh {
        anchor: Option<u64>,
        carried: Option<FindMatch>,
    },
    /// Step to the next older (`older == true`) or newer (`older == false`) match.
    Step {
        anchor: Option<u64>,
        carried: Option<FindMatch>,
        older: bool,
    },
}

/// Result of running a search inside `TerminalMirror`'s actor.
pub struct FindUpdate {
    pub count: FindCount,
    pub mark: ScrollMark,
    pub current: Option<FindMatch>,
    pub anchor: Option<super::MatchAnchor>,
    pub alt_screen: bool,
}

mod private {
    use super::*;

    pub trait Sealed {
        fn newest_matches_inner(&self, regex: &mut FindRegex) -> (Vec<FindMatch>, bool);
        fn matches_in_lines_inner(
            &self,
            regex: &mut FindRegex,
            first: Line,
            last: Line,
        ) -> Vec<FindMatch>;
        fn newest_at_or_above_inner(&self, regex: &mut FindRegex, line: Line) -> Option<FindMatch>;
        fn step_match_inner(
            &self,
            regex: &mut FindRegex,
            current: &FindMatch,
            older: bool,
        ) -> Option<FindMatch>;
        fn locate_inner(&self, regex: &mut FindRegex, carried: &FindMatch) -> Option<FindMatch>;
    }
}

/// Sealed trait implemented by locked screens and headless terminals that can be searched.
pub trait FindTarget: private::Sealed {}

impl<T: private::Sealed> FindTarget for T {}

impl private::Sealed for ScreenView<'_> {
    fn newest_matches_inner(&self, regex: &mut FindRegex) -> (Vec<FindMatch>, bool) {
        newest_matches_view(self, &regex.compiled)
    }

    fn matches_in_lines_inner(
        &self,
        regex: &mut FindRegex,
        first: Line,
        last: Line,
    ) -> Vec<FindMatch> {
        matches_in_lines_view(self, regex, first, last)
    }

    fn newest_at_or_above_inner(&self, regex: &mut FindRegex, line: Line) -> Option<FindMatch> {
        newest_at_or_above_view(self, &regex.compiled, line)
    }

    fn step_match_inner(
        &self,
        regex: &mut FindRegex,
        current: &FindMatch,
        older: bool,
    ) -> Option<FindMatch> {
        step_match_view(self, &regex.compiled, current, older)
    }

    fn locate_inner(&self, regex: &mut FindRegex, carried: &FindMatch) -> Option<FindMatch> {
        locate_view(self, &regex.compiled, carried)
    }
}

impl private::Sealed for ScreenGuard<'_> {
    fn newest_matches_inner(&self, regex: &mut FindRegex) -> (Vec<FindMatch>, bool) {
        newest_matches_view(&self.view(), &regex.compiled)
    }

    fn matches_in_lines_inner(
        &self,
        regex: &mut FindRegex,
        first: Line,
        last: Line,
    ) -> Vec<FindMatch> {
        matches_in_lines_view(&self.view(), regex, first, last)
    }

    fn newest_at_or_above_inner(&self, regex: &mut FindRegex, line: Line) -> Option<FindMatch> {
        newest_at_or_above_view(&self.view(), &regex.compiled, line)
    }

    fn step_match_inner(
        &self,
        regex: &mut FindRegex,
        current: &FindMatch,
        older: bool,
    ) -> Option<FindMatch> {
        step_match_view(&self.view(), &regex.compiled, current, older)
    }

    fn locate_inner(&self, regex: &mut FindRegex, carried: &FindMatch) -> Option<FindMatch> {
        locate_view(&self.view(), &regex.compiled, carried)
    }
}

impl private::Sealed for crate::replay::Terminal {
    fn newest_matches_inner(&self, regex: &mut FindRegex) -> (Vec<FindMatch>, bool) {
        newest_matches_view(&self.screen(), &regex.compiled)
    }

    fn matches_in_lines_inner(
        &self,
        regex: &mut FindRegex,
        first: Line,
        last: Line,
    ) -> Vec<FindMatch> {
        matches_in_lines_view(&self.screen(), regex, first, last)
    }

    fn newest_at_or_above_inner(&self, regex: &mut FindRegex, line: Line) -> Option<FindMatch> {
        newest_at_or_above_view(&self.screen(), &regex.compiled, line)
    }

    fn step_match_inner(
        &self,
        regex: &mut FindRegex,
        current: &FindMatch,
        older: bool,
    ) -> Option<FindMatch> {
        step_match_view(&self.screen(), &regex.compiled, current, older)
    }

    fn locate_inner(&self, regex: &mut FindRegex, carried: &FindMatch) -> Option<FindMatch> {
        locate_view(&self.screen(), &regex.compiled, carried)
    }
}

/// The newest `MATCH_LIMIT` matches over the active grid, scrollback
/// included, newest first, and whether older ones exist. On the alternate
/// screen that grid is the visible screen. The cap bounds only the count:
/// selection and stepping search the whole grid on demand.
pub fn newest_matches(target: &impl FindTarget, regex: &mut FindRegex) -> (Vec<FindMatch>, bool) {
    target.newest_matches_inner(regex)
}

/// Every match touching the lines `first..=last`, oldest first.
pub fn matches_in_lines(
    target: &impl FindTarget,
    regex: &mut FindRegex,
    first: Line,
    last: Line,
) -> Vec<FindMatch> {
    target.matches_in_lines_inner(regex, first, last)
}

/// The newest match starting at or above `line`, or the nearest one below it
/// when there is none.
pub fn newest_at_or_above(
    target: &impl FindTarget,
    regex: &mut FindRegex,
    line: Line,
) -> Option<FindMatch> {
    target.newest_at_or_above_inner(regex, line)
}

/// The next older match, or the next newer one, wrapping at the ends. A
/// current match that has left the scrollback has nothing older than it.
pub fn step_match(
    target: &impl FindTarget,
    regex: &mut FindRegex,
    current: &FindMatch,
    older: bool,
) -> Option<FindMatch> {
    target.step_match_inner(regex, current, older)
}

/// `carried`, a match moved to the current positions: the same match when
/// it is still there, otherwise the nearest one.
pub fn locate(
    target: &impl FindTarget,
    regex: &mut FindRegex,
    carried: &FindMatch,
) -> Option<FindMatch> {
    target.locate_inner(regex, carried)
}

fn rightmost_non_overlapping_spans_dfa(
    text: &str,
    rev_dfa: &HybridDfa,
    fwd_dfa: &HybridDfa,
    scratch: &mut FindSearchScratch,
) -> Option<Vec<(usize, usize)>> {
    let (rev_cache, fwd_cache) = scratch.caches_for(rev_dfa, fwd_dfa);
    let mut out = Vec::new();
    let mut limit = text.len();
    while limit > 0 {
        let rev_input = Input::new(text).range(0..limit);
        let Some(rev_hit) = rev_dfa.try_search_rev(rev_cache, &rev_input).ok()? else {
            break;
        };
        let start = rev_hit.offset();
        if start >= limit {
            break;
        }
        let fwd_input = Input::new(text).range(start..limit).anchored(Anchored::Yes);
        let Some(fwd_hit) = fwd_dfa.try_search_fwd(fwd_cache, &fwd_input).ok()? else {
            break;
        };
        let end = fwd_hit.offset();
        if end <= start || end > limit {
            break;
        }
        out.push((start, end));
        limit = start;
    }
    out.reverse();
    Some(out)
}

fn rightmost_non_overlapping_spans_fallback(regex: &Regex, text: &str) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut limit = text.len();
    while limit > 0 {
        let mut best: Option<(usize, usize)> = None;
        let mut pos = 0;
        while pos < limit {
            let Some(found) = regex.find_at(text, pos) else {
                break;
            };
            let start = found.start();
            if start >= limit {
                break;
            }
            let end = found.end();
            if end <= limit
                && best.is_none_or(|(best_start, best_end)| {
                    end > best_end || (end == best_end && start < best_start)
                })
            {
                best = Some((start, end));
            }
            let step = text[start..].chars().next().map_or(1, char::len_utf8);
            pos = start + step;
        }
        let Some((start, end)) = best else {
            break;
        };
        if end <= start {
            break;
        }
        out.push((start, end));
        limit = start;
    }
    out.reverse();
    out
}

fn collect_regex_matches(
    text: &str,
    map: &[(usize, GridPoint, bool)],
    cols: usize,
    compiled: &CompiledFindRegex,
    scratch: &mut FindSearchScratch,
) -> Vec<FindMatch> {
    if text.is_empty() || !compiled.prefilter.is_match(text) {
        return Vec::new();
    }

    let spans =
        rightmost_non_overlapping_spans_dfa(text, &compiled.rev_dfa, &compiled.fwd_dfa, scratch)
            .unwrap_or_else(|| rightmost_non_overlapping_spans_fallback(&compiled.prefilter, text));

    let mut out = Vec::with_capacity(spans.len());
    for (found_start, found_end) in spans {
        let start_idx = match map.binary_search_by_key(&found_start, |(off, _, _)| *off) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let end_idx = match map.binary_search_by_key(&found_end, |(off, _, _)| *off) {
            Ok(i) => i.saturating_sub(1),
            Err(i) => i.saturating_sub(1),
        };
        if start_idx >= map.len() || end_idx >= map.len() || start_idx > end_idx {
            continue;
        }
        let start = map[start_idx].1;
        let (mut end, end_is_wide) = (map[end_idx].1, map[end_idx].2);
        if end_is_wide && end.column.0 + 1 < cols {
            end.column.0 += 1;
        }
        out.push(FindMatch::new(start, end));
    }
    out
}

fn logical_line_matches(
    view: &ScreenView<'_>,
    compiled: &CompiledFindRegex,
    seg_start: Line,
    seg_end: Line,
    scratch: &mut FindSearchScratch,
) -> Vec<FindMatch> {
    let cols = view.cols();
    if cols == 0 {
        return Vec::new();
    }
    let row_count = (seg_end.0 - seg_start.0 + 1).max(0) as usize;
    let mut text = String::with_capacity(row_count * cols);
    let mut map: Vec<(usize, GridPoint, bool)> = Vec::with_capacity(row_count * cols);

    for row_i in seg_start.0..=seg_end.0 {
        let line = Line(row_i);
        for col in 0..cols {
            let Some(cell) = view.row_cell(line, Column(col)) else {
                break;
            };
            let flags = cell.flags();
            if flags.intersects(CellFlags::WIDE_CHAR_SPACER | CellFlags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            map.push((
                text.len(),
                GridPoint::new(line, Column(col)),
                flags.contains(CellFlags::WIDE_CHAR),
            ));
            text.push(cell.c());
        }
    }

    collect_regex_matches(&text, &map, cols, compiled, scratch)
}

#[inline]
fn line_to_screen_y(history: usize, line: Line) -> u32 {
    (line.0 + history as i32).max(0) as u32
}

/// Abstract row source for lazy regex search over `vt::Terminal`.
pub(crate) trait FindRowSource {
    fn cols(&self) -> usize;
    fn rows(&self) -> usize;
    fn history_size(&self) -> usize;
    fn row_is_wrapped(&self, line: Line) -> bool;
    fn append_row_search_cells(
        &self,
        line: Line,
        text: &mut String,
        map: &mut Vec<(usize, GridPoint, bool)>,
    );
}

impl FindRowSource for VtTerminal {
    #[inline]
    fn cols(&self) -> usize {
        usize::from(VtTerminal::cols(self))
    }

    #[inline]
    fn rows(&self) -> usize {
        usize::from(VtTerminal::rows(self))
    }

    #[inline]
    fn history_size(&self) -> usize {
        self.scrollback_rows()
    }

    #[inline]
    fn row_is_wrapped(&self, line: Line) -> bool {
        VtTerminal::row_is_wrapped(self, line_to_screen_y(self.scrollback_rows(), line))
    }

    #[inline]
    fn append_row_search_cells(
        &self,
        line: Line,
        text: &mut String,
        map: &mut Vec<(usize, GridPoint, bool)>,
    ) {
        VtTerminal::append_row_search_cells(
            self,
            line_to_screen_y(self.scrollback_rows(), line),
            line,
            text,
            map,
        );
    }
}

pub(crate) fn soft_wrapped_bounds_source<S: FindRowSource + ?Sized>(
    source: &S,
    line: Line,
) -> (Line, Line) {
    let history = source.history_size();
    let topmost = Line(-(history as i32));
    let bottommost = Line((source.rows() as i32).max(1) - 1);
    let line = line.clamp(topmost, bottommost);
    let mut start = line;
    while start > topmost && source.row_is_wrapped(start - 1) {
        start -= 1;
    }
    let mut end = line;
    while end < bottommost && source.row_is_wrapped(end) {
        end += 1;
    }
    (start, end)
}

pub(crate) fn soft_wrapped_bounds_vt(term: &VtTerminal, line: Line) -> (Line, Line) {
    soft_wrapped_bounds_source(term, line)
}

fn prev_logical_line_source<S: FindRowSource + ?Sized>(
    source: &S,
    seg_start: Line,
    topmost: Line,
) -> Option<(Line, Line)> {
    if seg_start <= topmost {
        return None;
    }
    Some(soft_wrapped_bounds_source(source, seg_start - 1))
}

fn next_logical_line_source<S: FindRowSource + ?Sized>(
    source: &S,
    seg_end: Line,
    bottommost: Line,
) -> Option<(Line, Line)> {
    if seg_end >= bottommost {
        return None;
    }
    Some(soft_wrapped_bounds_source(source, seg_end + 1))
}

fn logical_line_matches_source<S: FindRowSource + ?Sized>(
    source: &S,
    compiled: &CompiledFindRegex,
    seg_start: Line,
    seg_end: Line,
    scratch: &mut FindSearchScratch,
) -> Vec<FindMatch> {
    let cols = source.cols();
    if cols == 0 {
        return Vec::new();
    }
    let mut text = std::mem::take(&mut scratch.text);
    let mut map = std::mem::take(&mut scratch.map);
    text.clear();
    map.clear();
    for row_i in seg_start.0..=seg_end.0 {
        source.append_row_search_cells(Line(row_i), &mut text, &mut map);
    }
    let out = collect_regex_matches(&text, &map, cols, compiled, scratch);
    scratch.text = text;
    scratch.map = map;
    out
}

/// Lazily walks backward from the newest row directly on `source`,
/// stopping once `MATCH_LIMIT` matches have been collected.
pub(crate) fn newest_matches_source<S: FindRowSource + ?Sized>(
    source: &S,
    regex: &FindRegex,
) -> (Vec<FindMatch>, bool) {
    let history = source.history_size();
    let topmost = Line(-(history as i32));
    let bottommost = Line((source.rows() as i32).max(1) - 1);
    let mut matches = Vec::new();
    let mut scratch = FindSearchScratch::default();
    let mut seg = Some(soft_wrapped_bounds_source(source, bottommost));
    while let Some((seg_start, seg_end)) = seg {
        let line_matches =
            logical_line_matches_source(source, &regex.compiled, seg_start, seg_end, &mut scratch);
        for found in line_matches.into_iter().rev() {
            if matches.len() == MATCH_LIMIT {
                return (matches, true);
            }
            matches.push(found);
        }
        seg = prev_logical_line_source(source, seg_start, topmost);
    }
    (matches, false)
}

/// Every match touching `first..=last` (computed with full soft-wrapped
/// logical-line context), oldest first, read directly from `source`.
pub(crate) fn matches_in_lines_source<S: FindRowSource + ?Sized>(
    source: &S,
    regex: &FindRegex,
    first: Line,
    last: Line,
) -> Vec<FindMatch> {
    if first > last || source.rows() == 0 {
        return Vec::new();
    }
    let history = source.history_size();
    let topmost = Line(-(history as i32));
    let bottommost = Line((source.rows() as i32).max(1) - 1);
    let first = first.clamp(topmost, bottommost);
    let last = last.clamp(topmost, bottommost);
    let (mut seg_start, mut seg_end) = soft_wrapped_bounds_source(source, first);
    let (_, limit_end) = soft_wrapped_bounds_source(source, last);
    let mut matches = Vec::new();
    let mut scratch = FindSearchScratch::default();
    loop {
        matches.extend(
            logical_line_matches_source(source, &regex.compiled, seg_start, seg_end, &mut scratch)
                .into_iter()
                .filter(|found| found.end.line >= first && found.start.line <= last),
        );
        if seg_end >= limit_end {
            break;
        }
        let Some(next) = next_logical_line_source(source, seg_end, bottommost) else {
            break;
        };
        seg_start = next.0;
        seg_end = next.1;
    }
    matches
}

fn oldest_from_source<S: FindRowSource + ?Sized>(
    source: &S,
    bottommost: Line,
    compiled: &CompiledFindRegex,
    first_seg: (Line, Line),
    scratch: &mut FindSearchScratch,
) -> Option<FindMatch> {
    let mut seg = Some(first_seg);
    while let Some((seg_start, seg_end)) = seg {
        let line_matches =
            logical_line_matches_source(source, compiled, seg_start, seg_end, scratch);
        if let Some(found) = line_matches.into_iter().next() {
            return Some(found);
        }
        seg = next_logical_line_source(source, seg_end, bottommost);
    }
    None
}

fn newest_overall_source<S: FindRowSource + ?Sized>(
    source: &S,
    topmost: Line,
    bottommost: Line,
    compiled: &CompiledFindRegex,
    scratch: &mut FindSearchScratch,
) -> Option<FindMatch> {
    let mut seg = Some(soft_wrapped_bounds_source(source, bottommost));
    while let Some((seg_start, seg_end)) = seg {
        let line_matches =
            logical_line_matches_source(source, compiled, seg_start, seg_end, scratch);
        if let Some(found) = line_matches.into_iter().next_back() {
            return Some(found);
        }
        seg = prev_logical_line_source(source, seg_start, topmost);
    }
    None
}

pub(crate) fn newest_at_or_above_source<S: FindRowSource + ?Sized>(
    source: &S,
    regex: &FindRegex,
    line: Line,
) -> Option<FindMatch> {
    let history = source.history_size();
    let topmost = Line(-(history as i32));
    let bottommost = Line((source.rows() as i32).max(1) - 1);
    let line = line.clamp(topmost, bottommost);
    let (start_seg, end_seg) = soft_wrapped_bounds_source(source, line);
    let mut scratch = FindSearchScratch::default();
    let current_line_matches =
        logical_line_matches_source(source, &regex.compiled, start_seg, end_seg, &mut scratch);
    if let Some(found) = current_line_matches
        .iter()
        .rev()
        .find(|found| found.start.line <= line)
    {
        return Some(*found);
    }
    let mut seg = prev_logical_line_source(source, start_seg, topmost);
    while let Some((seg_start, seg_end)) = seg {
        let line_matches =
            logical_line_matches_source(source, &regex.compiled, seg_start, seg_end, &mut scratch);
        if let Some(found) = line_matches.into_iter().next_back() {
            return Some(found);
        }
        seg = prev_logical_line_source(source, seg_start, topmost);
    }
    if let Some(found) = current_line_matches
        .into_iter()
        .find(|found| found.start.line > line)
    {
        return Some(found);
    }
    oldest_from_source(
        source,
        bottommost,
        &regex.compiled,
        next_logical_line_source(source, end_seg, bottommost)?,
        &mut scratch,
    )
}

pub(crate) fn step_match_source<S: FindRowSource + ?Sized>(
    source: &S,
    regex: &FindRegex,
    current: &FindMatch,
    older: bool,
) -> Option<FindMatch> {
    let history = source.history_size();
    let topmost = Line(-(history as i32));
    let bottommost = Line((source.rows() as i32).max(1) - 1);
    let mut scratch = FindSearchScratch::default();
    let line = current.start.line;
    if line < topmost {
        return if older {
            newest_overall_source(source, topmost, bottommost, &regex.compiled, &mut scratch)
        } else {
            oldest_from_source(
                source,
                bottommost,
                &regex.compiled,
                soft_wrapped_bounds_source(source, topmost),
                &mut scratch,
            )
        };
    }
    let clamped = line.clamp(topmost, bottommost);
    let (seg_start, seg_end) = soft_wrapped_bounds_source(source, clamped);
    let same_line =
        logical_line_matches_source(source, &regex.compiled, seg_start, seg_end, &mut scratch);
    if older {
        if let Some(found) = same_line
            .into_iter()
            .rev()
            .find(|found| found.start < current.start)
        {
            return Some(found);
        }
        let mut seg = prev_logical_line_source(source, seg_start, topmost);
        while let Some((s, e)) = seg {
            let line_matches =
                logical_line_matches_source(source, &regex.compiled, s, e, &mut scratch);
            if let Some(found) = line_matches.into_iter().next_back() {
                return Some(found);
            }
            seg = prev_logical_line_source(source, s, topmost);
        }
        newest_overall_source(source, topmost, bottommost, &regex.compiled, &mut scratch)
    } else {
        if let Some(found) = same_line
            .into_iter()
            .find(|found| found.start > current.start)
        {
            return Some(found);
        }
        if let Some(next_seg) = next_logical_line_source(source, seg_end, bottommost) {
            if let Some(found) =
                oldest_from_source(source, bottommost, &regex.compiled, next_seg, &mut scratch)
            {
                return Some(found);
            }
        }
        oldest_from_source(
            source,
            bottommost,
            &regex.compiled,
            soft_wrapped_bounds_source(source, topmost),
            &mut scratch,
        )
    }
}

pub(crate) fn locate_source<S: FindRowSource + ?Sized>(
    source: &S,
    regex: &FindRegex,
    carried: &FindMatch,
) -> Option<FindMatch> {
    let history = source.history_size();
    let topmost = Line(-(history as i32));
    let bottommost = Line((source.rows() as i32).max(1) - 1);
    let line = carried.start.line;
    if line >= topmost && line <= bottommost {
        let (seg_start, seg_end) = soft_wrapped_bounds_source(source, line);
        let mut scratch = FindSearchScratch::default();
        if let Some(found) =
            logical_line_matches_source(source, &regex.compiled, seg_start, seg_end, &mut scratch)
                .into_iter()
                .find(|found| found == carried)
        {
            return Some(found);
        }
    }
    newest_at_or_above_source(source, regex, carried.start.line)
}

fn clamp_line(view: &ScreenView<'_>, line: Line) -> Line {
    line.clamp(view.topmost_line(), view.bottommost_line())
}

fn prev_logical_line(view: &ScreenView<'_>, seg_start: Line) -> Option<(Line, Line)> {
    if seg_start <= view.topmost_line() {
        return None;
    }
    Some(view.soft_wrapped_line_bounds(seg_start - 1))
}

fn next_logical_line(view: &ScreenView<'_>, seg_end: Line) -> Option<(Line, Line)> {
    if seg_end >= view.bottommost_line() {
        return None;
    }
    Some(view.soft_wrapped_line_bounds(seg_end + 1))
}

fn newest_matches_view(
    view: &ScreenView<'_>,
    compiled: &CompiledFindRegex,
) -> (Vec<FindMatch>, bool) {
    let mut matches = Vec::new();
    let mut scratch = FindSearchScratch::default();
    let mut seg = Some(view.soft_wrapped_line_bounds(view.bottommost_line()));
    while let Some((seg_start, seg_end)) = seg {
        let line_matches = logical_line_matches(view, compiled, seg_start, seg_end, &mut scratch);
        for found in line_matches.into_iter().rev() {
            if matches.len() == MATCH_LIMIT {
                return (matches, true);
            }
            matches.push(found);
        }
        seg = prev_logical_line(view, seg_start);
    }
    (matches, false)
}

fn matches_in_lines_view(
    view: &ScreenView<'_>,
    regex: &FindRegex,
    first: Line,
    last: Line,
) -> Vec<FindMatch> {
    if first > last || view.rows() == 0 {
        return Vec::new();
    }
    let first = clamp_line(view, first);
    let last = clamp_line(view, last);
    let (mut seg_start, mut seg_end) = view.soft_wrapped_line_bounds(first);
    let (_, limit_end) = view.soft_wrapped_line_bounds(last);
    let mut matches = Vec::new();
    let mut scratch = FindSearchScratch::default();
    loop {
        matches.extend(
            logical_line_matches(view, &regex.compiled, seg_start, seg_end, &mut scratch)
                .into_iter()
                .filter(|found| found.end.line >= first && found.start.line <= last),
        );
        if seg_end >= limit_end {
            break;
        }
        let Some(next) = next_logical_line(view, seg_end) else {
            break;
        };
        seg_start = next.0;
        seg_end = next.1;
    }
    matches
}

fn newest_at_or_above_view(
    view: &ScreenView<'_>,
    compiled: &CompiledFindRegex,
    line: Line,
) -> Option<FindMatch> {
    let line = clamp_line(view, line);
    let (start_seg, end_seg) = view.soft_wrapped_line_bounds(line);
    let mut scratch = FindSearchScratch::default();
    let current_line_matches =
        logical_line_matches(view, compiled, start_seg, end_seg, &mut scratch);
    if let Some(found) = current_line_matches
        .iter()
        .rev()
        .find(|found| found.start.line <= line)
    {
        return Some(*found);
    }
    let mut seg = prev_logical_line(view, start_seg);
    while let Some((seg_start, seg_end)) = seg {
        let line_matches = logical_line_matches(view, compiled, seg_start, seg_end, &mut scratch);
        if let Some(found) = line_matches.into_iter().next_back() {
            return Some(found);
        }
        seg = prev_logical_line(view, seg_start);
    }
    if let Some(found) = current_line_matches
        .into_iter()
        .find(|found| found.start.line > line)
    {
        return Some(found);
    }
    oldest_from_view(
        view,
        compiled,
        next_logical_line(view, end_seg)?,
        &mut scratch,
    )
}

fn oldest_from_view(
    view: &ScreenView<'_>,
    compiled: &CompiledFindRegex,
    first_seg: (Line, Line),
    scratch: &mut FindSearchScratch,
) -> Option<FindMatch> {
    let mut seg = Some(first_seg);
    while let Some((seg_start, seg_end)) = seg {
        let line_matches = logical_line_matches(view, compiled, seg_start, seg_end, scratch);
        if let Some(found) = line_matches.into_iter().next() {
            return Some(found);
        }
        seg = next_logical_line(view, seg_end);
    }
    None
}

fn newest_overall_view(
    view: &ScreenView<'_>,
    compiled: &CompiledFindRegex,
    scratch: &mut FindSearchScratch,
) -> Option<FindMatch> {
    let mut seg = Some(view.soft_wrapped_line_bounds(view.bottommost_line()));
    while let Some((seg_start, seg_end)) = seg {
        let line_matches = logical_line_matches(view, compiled, seg_start, seg_end, scratch);
        if let Some(found) = line_matches.into_iter().next_back() {
            return Some(found);
        }
        seg = prev_logical_line(view, seg_start);
    }
    None
}

fn step_match_view(
    view: &ScreenView<'_>,
    compiled: &CompiledFindRegex,
    current: &FindMatch,
    older: bool,
) -> Option<FindMatch> {
    let mut scratch = FindSearchScratch::default();
    let line = current.start.line;
    if line < view.topmost_line() {
        return if older {
            newest_overall_view(view, compiled, &mut scratch)
        } else {
            oldest_from_view(
                view,
                compiled,
                view.soft_wrapped_line_bounds(view.topmost_line()),
                &mut scratch,
            )
        };
    }
    let clamped = clamp_line(view, line);
    let (seg_start, seg_end) = view.soft_wrapped_line_bounds(clamped);
    let same_line = logical_line_matches(view, compiled, seg_start, seg_end, &mut scratch);
    if older {
        if let Some(found) = same_line
            .into_iter()
            .rev()
            .find(|found| found.start < current.start)
        {
            return Some(found);
        }
        let mut seg = prev_logical_line(view, seg_start);
        while let Some((s, e)) = seg {
            let line_matches = logical_line_matches(view, compiled, s, e, &mut scratch);
            if let Some(found) = line_matches.into_iter().next_back() {
                return Some(found);
            }
            seg = prev_logical_line(view, s);
        }
        newest_overall_view(view, compiled, &mut scratch)
    } else {
        if let Some(found) = same_line
            .into_iter()
            .find(|found| found.start > current.start)
        {
            return Some(found);
        }
        if let Some(next_seg) = next_logical_line(view, seg_end) {
            if let Some(found) = oldest_from_view(view, compiled, next_seg, &mut scratch) {
                return Some(found);
            }
        }
        oldest_from_view(
            view,
            compiled,
            view.soft_wrapped_line_bounds(view.topmost_line()),
            &mut scratch,
        )
    }
}

fn locate_view(
    view: &ScreenView<'_>,
    compiled: &CompiledFindRegex,
    carried: &FindMatch,
) -> Option<FindMatch> {
    let line = carried.start.line;
    if line >= view.topmost_line() && line <= view.bottommost_line() {
        let (seg_start, seg_end) = view.soft_wrapped_line_bounds(line);
        let mut scratch = FindSearchScratch::default();
        if let Some(found) = logical_line_matches(view, compiled, seg_start, seg_end, &mut scratch)
            .into_iter()
            .find(|found| found == carried)
        {
            return Some(found);
        }
    }
    newest_at_or_above_view(view, compiled, carried.start.line)
}

#[derive(Clone, Copy, Debug)]
struct ScreenEpochState {
    screen_gen: u64,
    epoch: u64,
    base_rows: u64,
    last_rows: u64,
}

/// Tracks per-screen upward scroll counts using `libghostty-vt`'s native
/// `GHOSTTY_TERMINAL_DATA_SCROLL_UP_ROWS` and `GHOSTTY_TERMINAL_DATA_SCREEN_GENERATION`,
/// without any Rust VT byte scanner.
///
/// Primary and alternate screens keep independent epochs: a primary mark stays
/// incomparable while the alternate screen is active, then compares identically
/// when the same primary screen returns regardless of PTY chunk boundaries.
/// True invalidations (resize, reattach/resync, or physical screen reset /
/// replacement changing `screen_generation`) allocate a fresh epoch rather than
/// subtracting unrelated counters.
#[derive(Default)]
pub(crate) struct ScreenScrollTracker {
    next_epoch: u64,
    screens: [Option<ScreenEpochState>; 2],
    current: ScrollMark,
}

impl ScreenScrollTracker {
    fn slot(screen: ActiveScreen) -> usize {
        match screen {
            ActiveScreen::Primary => 0,
            ActiveScreen::Alternate => 1,
        }
    }

    pub(crate) fn new(term: &VtTerminal) -> Self {
        let mut tracker = Self::default();
        tracker.sync(term);
        tracker
    }

    pub(crate) fn mark(&self) -> ScrollMark {
        self.current
    }

    pub(crate) fn reset(&mut self, term: &VtTerminal) {
        self.screens = [None, None];
        self.sync(term);
    }

    pub(crate) fn measure_write(
        term: &mut VtTerminal,
        bytes: &[u8],
        mut feed: impl FnMut(&mut VtTerminal, &[u8]),
    ) -> Option<usize> {
        let before_screen = term.active_screen();
        let before_gen = term.screen_generation();
        let before_rows = term.scroll_up_rows();
        if !bytes.is_empty() {
            feed(term, bytes);
        }
        let after_screen = term.active_screen();
        let after_gen = term.screen_generation();
        let after_rows = term.scroll_up_rows();
        if after_screen == before_screen && after_gen == before_gen {
            Some(usize::try_from(after_rows.saturating_sub(before_rows)).unwrap_or(usize::MAX))
        } else {
            None
        }
    }

    pub(crate) fn write_and_track(
        &mut self,
        term: &mut VtTerminal,
        bytes: &[u8],
        mut feed: impl FnMut(&mut VtTerminal, &[u8]),
    ) -> Option<usize> {
        if !bytes.is_empty() {
            feed(term, bytes);
        }
        self.sync(term)
    }

    fn sync(&mut self, term: &VtTerminal) -> Option<usize> {
        let idx = Self::slot(term.active_screen());
        let screen_gen = term.screen_generation();
        let rows = term.scroll_up_rows();
        let (state, delta) = match self.screens[idx] {
            Some(mut existing)
                if existing.screen_gen == screen_gen && rows >= existing.last_rows =>
            {
                let delta =
                    usize::try_from(rows.saturating_sub(existing.last_rows)).unwrap_or(usize::MAX);
                existing.last_rows = rows;
                (existing, Some(delta))
            }
            _ => {
                let epoch = self.next_epoch;
                self.next_epoch = self.next_epoch.wrapping_add(1);
                (
                    ScreenEpochState {
                        screen_gen,
                        epoch,
                        base_rows: rows,
                        last_rows: rows,
                    },
                    None,
                )
            }
        };
        self.screens[idx] = Some(state);
        self.current = ScrollMark {
            epoch: state.epoch,
            lines: rows.saturating_sub(state.base_rows),
        };
        delta
    }
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
pub fn shift_match(found: &FindMatch, lines: i32) -> FindMatch {
    let shift =
        |point: GridPoint| GridPoint::new(Line(point.line.0.saturating_sub(lines)), point.column);
    FindMatch::new(shift(found.start), shift(found.end))
}

/// The display offset that centres `found`, when it is not already on screen.
pub fn centring_offset(
    found: &FindMatch,
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
    use crate::replay::Terminal;
    use crate::types::Column;

    fn term(columns: usize, lines: usize, scrollback: usize) -> Terminal {
        Terminal::with_scrollback(columns as u16, lines as u16, scrollback)
    }

    fn feed(term: &mut Terminal, bytes: &[u8]) {
        term.advance(bytes);
    }

    fn query(text: &str, match_case: bool, regex: bool) -> FindQuery {
        FindQuery {
            text: text.into(),
            match_case,
            regex,
        }
    }

    fn regex(query: &FindQuery) -> Box<FindRegex> {
        match query.pattern() {
            FindPattern::Ready(regex) => regex,
            _ => panic!("{query:?} should build"),
        }
    }

    fn plain(text: &str) -> Box<FindRegex> {
        regex(&query(text, false, false))
    }

    /// Every match in the grid, oldest first.
    fn all(term: &Terminal, regex: &mut FindRegex) -> Vec<FindMatch> {
        let bottom = Line(term.rows() as i32 - 1);
        matches_in_lines(term, regex, term.screen().topmost_line(), bottom)
    }

    fn count(term: &Terminal, regex: &mut FindRegex, current: Option<&FindMatch>) -> FindCount {
        let (newest, more) = newest_matches(term, regex);
        FindCount::new(&newest, more, current)
    }

    fn text(term: &Terminal, found: &FindMatch) -> String {
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
        assert_eq!(*found[0].start(), GridPoint::new(Line(0), Column(7)));
        assert_eq!(*found[0].end(), GridPoint::new(Line(1), Column(4)));
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
        let mut visited = vec![current];
        for _ in 0..5 {
            current = step_match(&term, &mut regex, &current, true).unwrap();
            visited.push(current);
        }
        let mut older_first = expected.clone();
        older_first.reverse();
        older_first.push(expected[4]);
        assert_eq!(visited, older_first);
        current = expected[0];
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
        assert_eq!(locate(&term, &mut regex, &second), Some(second));
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
        let mut term = term(80, 8, 120);
        feed(&mut term, &numbered(0..2000, "hit"));
        let history_before = term.history_size();
        assert!(
            history_before > 0 && history_before < 1990,
            "expected whole-page eviction below 2000 rows, got {history_before}"
        );
        let mut regex = plain("hit");
        let oldest = newest_at_or_above(&term, &mut regex, term.screen().topmost_line()).unwrap();
        assert_eq!(oldest.start().line, Line(-(history_before as i32)));
        let oldest_row = term.bounds_to_string(
            GridPoint::new(oldest.start().line, Column(0)),
            GridPoint::new(oldest.start().line, Column(19)),
        );
        assert_ne!(oldest_row, "hit 0", "row 0 should already be evicted");

        // Feed numbered rows until the page holding `oldest` is genuinely evicted.
        let mut total_moved = 0i32;
        let mut next_id = 2000usize;
        while shift_match(&oldest, total_moved).start().line >= term.screen().topmost_line() {
            let chunk = numbered(next_id..next_id + 100, "hit");
            next_id += 100;
            total_moved += moved(&mut term, &chunk).unwrap() as i32;
            assert!(next_id < 6000, "expected page eviction within 4000 rows");
        }
        let evicted = shift_match(&oldest, total_moved);
        let topmost = term.screen().topmost_line();
        assert!(evicted.start().line < topmost);

        let newer = step_match(&term, &mut regex, &evicted, false).unwrap();
        assert_eq!(newer.start().line, topmost);
        let new_oldest_row = term.bounds_to_string(
            GridPoint::new(newer.start().line, Column(0)),
            GridPoint::new(newer.start().line, Column(19)),
        );
        assert_ne!(new_oldest_row, oldest_row);
        let older = step_match(&term, &mut regex, &evicted, true).unwrap();
        assert_eq!(older.start().line, Line(7));
        assert_eq!(
            locate(&term, &mut regex, &evicted).unwrap().start().line,
            topmost
        );
    }

    #[test]
    fn centring_scrolls_only_for_an_off_screen_match() {
        let at = |line: i32| {
            FindMatch::new(
                GridPoint::new(Line(line), Column(0)),
                GridPoint::new(Line(line), Column(3)),
            )
        };
        assert_eq!(centring_offset(&at(3), 10, 100, 0), None);
        assert_eq!(centring_offset(&at(-5), 10, 100, 8), None);
        assert_eq!(centring_offset(&at(-20), 10, 100, 0), Some(25));
        assert_eq!(centring_offset(&at(-98), 10, 100, 0), Some(100));
        assert_eq!(centring_offset(&at(8), 10, 100, 50), Some(0));
    }

    /// Lines `bytes` moved the content, measured the way the mirror does.
    fn moved(term: &mut Terminal, bytes: &[u8]) -> Option<usize> {
        term.advance_tracking_scroll(bytes)
    }

    /// Feeds the chunks the way the mirror does and carries `found` along.
    fn follow(term: &mut Terminal, found: &FindMatch, chunks: &[Vec<u8>]) -> FindMatch {
        let mut mark = ScrollMark::default();
        let start = mark;
        for chunk in chunks {
            mark.advance(moved(term, chunk));
        }
        shift_match(found, start.moved_since(mark).unwrap())
    }

    #[test]
    fn the_current_match_keeps_its_text_while_output_scrolls() {
        for (before, after, scrollback, expects_eviction) in [
            (4, 40, 2000, false),
            (1500, 100, 120, true),
            (3000, 700, 2500, true),
        ] {
            for chunked in [true, false] {
                let mut term = term(80, 8, scrollback);
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
                assert_eq!(locate(&term, &mut regex, &carried), Some(carried));
                assert_eq!(
                    text(&term, &carried),
                    "needle",
                    "{before} {after} {chunked}"
                );
                let total_scrolled = (before + 1 + after).saturating_sub(7);
                if expects_eviction {
                    assert!(
                        term.history_size() < total_scrolled && term.history_size() >= after,
                        "expected real eviction while retaining needle: before={before} after={after} history={} total={total_scrolled}",
                        term.history_size()
                    );
                } else {
                    assert_eq!(term.history_size(), total_scrolled);
                }
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
        assert_eq!(locate(&term, &mut regex, &carried), Some(carried));
        assert_eq!(
            term.bounds_to_string(GridPoint::new(Line(-114), Column(0)), *carried.end()),
            "old needle"
        );
        assert_eq!(count(&term, &mut regex, Some(&carried)).index, Some(1));
    }

    #[test]
    fn the_tracker_counts_every_line_past_the_ring_buffers_length() {
        for burst in [121, 200, 1004, 1008, 1010] {
            let mut term = term(30, 8, 120);
            feed(&mut term, &numbered(0..200, "line"));
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
        assert!(!term.is_sync_held());

        // An open DEC 2026 update is parsed into the live grid immediately while
        // `render_hold` keeps the published viewport frozen; stopping sync ends
        // the hold without duplicating scroll lines.
        let mut term = full();
        let mut open = b"\x1b[?2026h".to_vec();
        open.extend_from_slice(&numbered(0..1010, "more"));
        assert_eq!(moved(&mut term, &open), Some(1010));
        assert!(term.is_sync_held());
        term.stop_sync();
        assert!(!term.is_sync_held());

        let mut term = full();
        let mut excursion = numbered(0..3, "more");
        excursion.extend_from_slice(b"\x1b[?1049h");
        excursion.extend_from_slice(&numbered(0..50, "alt"));
        excursion.extend_from_slice(b"\x1b[?1049l");
        excursion.extend_from_slice(&numbered(0..2, "more"));
        assert_eq!(moved(&mut term, &excursion), Some(5));

        // Right-margin HT with wrap pending stays on the full bottom row; the
        // next printable character wraps and scrolls once.
        let mut term = self::term(10, 8, 100);
        assert_eq!(moved(&mut term, b"\x1b[8;1Habcdefghij"), Some(0));
        assert_eq!(moved(&mut term, b"\t"), Some(0), "tab at right margin");
        assert_eq!(term.history_size(), 0);
        assert_eq!(moved(&mut term, b"z"), Some(1), "printable after tab");
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
            for event in &fixture.events {
                match event {
                    FixtureEvent::Data { data, .. } => {
                        let bytes = decode_chunk(data).unwrap();
                        plain.advance(&bytes);
                        let _ = tracked.advance_tracking_scroll(&bytes);
                    }
                    FixtureEvent::Resize { cols, rows, .. } => {
                        plain.resize(*cols, *rows);
                        tracked.resize(*cols, *rows);
                    }
                    _ => {}
                }
            }
            assert_eq!(
                tracked.encode_binary_snapshot(),
                plain.encode_binary_snapshot(),
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
        assert_eq!(locate(&term, &mut regex, &carried), Some(carried));
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
