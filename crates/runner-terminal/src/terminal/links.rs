use std::ops::Range;
#[cfg(not(windows))]
use std::path::Component;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

use crate::screen::ScreenView;
use crate::types::{CellFlags as Flags, Column, GridPoint as Point, Line};

pub(super) const MAX_WRAP_JOIN_ROWS: usize = 8;
pub(super) const MAX_LINK_PATH_PROBES: usize = 32;
const MAX_SPACED_PATH_WORDS: usize = 10;
const MAX_URL_SCHEME_PREFIX_LEN: usize = "vscode-insiders://".len();

const EDITOR_SCHEMES: &[&str] = &["vscode", "vscode-insiders", "cursor", "windsurf", "zed"];
const KNOWN_OPAQUE_URI_SCHEMES: &[&str] = &["about", "data", "javascript", "mailto", "tel", "urn"];

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FileLinkContext {
    pub extra_root: Option<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LinkTarget {
    Url(String),
    File {
        path: PathBuf,
        line: Option<u32>,
        column: Option<u32>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalLink {
    pub target: LinkTarget,
    pub start: Point,
    pub end: Point,
    spans: Vec<(Point, Point)>,
}

impl TerminalLink {
    pub fn contains(&self, point: Point) -> bool {
        self.spans.iter().any(|(start, end)| {
            point.line == start.line && point.column >= start.column && point.column <= end.column
        })
    }

    fn from_points(target: LinkTarget, points: impl Iterator<Item = Point>) -> Self {
        let mut spans: Vec<(Point, Point)> = Vec::new();
        for point in points {
            match spans.last_mut() {
                Some((_, end)) if end.line == point.line && end.column + 1 == point.column => {
                    *end = point;
                }
                _ => spans.push((point, point)),
            }
        }
        Self {
            target,
            start: spans[0].0,
            end: spans[spans.len() - 1].1,
            spans,
        }
    }
}

struct ProbeBudget<'a> {
    remaining: usize,
    is_file: &'a mut dyn FnMut(&Path) -> bool,
}

impl<'a> ProbeBudget<'a> {
    fn new(limit: usize, is_file: &'a mut dyn FnMut(&Path) -> bool) -> Self {
        Self {
            remaining: limit,
            is_file,
        }
    }

    fn exhausted(&self) -> bool {
        self.remaining == 0
    }

    fn probe(&mut self, path: &Path) -> bool {
        if self.remaining == 0 {
            return false;
        }
        self.remaining -= 1;
        (self.is_file)(path)
    }
}

pub(super) fn strip_file_scheme(uri: &str) -> Option<&str> {
    let prefix = uri.get(..7)?;
    prefix.eq_ignore_ascii_case("file://").then(|| &uri[7..])
}

/// Returns the part after `<editor-scheme>://file` when `uri` uses a supported
/// editor scheme with a literal `file` authority.
fn strip_editor_file_scheme(uri: &str) -> Option<&str> {
    let (scheme, after_scheme) = uri.split_once("://")?;
    if !EDITOR_SCHEMES
        .iter()
        .any(|known| scheme.eq_ignore_ascii_case(known))
    {
        return None;
    }
    let authority_end = after_scheme
        .find(['/', '?', '#'])
        .unwrap_or(after_scheme.len());
    after_scheme[..authority_end]
        .eq_ignore_ascii_case("file")
        .then(|| &after_scheme[authority_end..])
}

fn is_valid_rfc3986_scheme(scheme: &str) -> bool {
    let mut bytes = scheme.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_alphabetic())
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
}

fn starts_with_url_scheme(text: &str) -> bool {
    let Some(colon) = text.find("://") else {
        return false;
    };
    let scheme = &text[..colon];
    scheme.eq_ignore_ascii_case("http")
        || scheme.eq_ignore_ascii_case("https")
        || scheme.eq_ignore_ascii_case("file")
        || EDITOR_SCHEMES
            .iter()
            .any(|known| scheme.eq_ignore_ascii_case(known))
}

fn starts_with_any_authority_scheme(text: &str) -> bool {
    let Some((scheme, _)) = text.split_once("://") else {
        return false;
    };
    is_valid_rfc3986_scheme(scheme)
}

enum Osc8SchemeKind {
    /// Unambiguous URI with `scheme://...` or opaque RFC 3986 scheme.
    Url,
    /// Ambiguous `<scheme>:<location-like>` (e.g. `Makefile:12` vs `custom:12`):
    /// resolves as a file if it exists, otherwise stays a URL.
    AmbiguousUrlOrPath,
    /// No URI scheme (relative/absolute/Windows drive path, or `file.ext:line`).
    BarePath,
}

fn classify_osc8_uri(uri: &str, windows: bool) -> Osc8SchemeKind {
    if starts_with_any_authority_scheme(uri) {
        return Osc8SchemeKind::Url;
    }
    let Some((scheme, rest)) = uri.split_once(':') else {
        return Osc8SchemeKind::BarePath;
    };
    if rest.is_empty() {
        return Osc8SchemeKind::BarePath;
    }
    // Single-letter prefixes are either Windows drives (`C:\...`) or short
    // single-letter filenames (`a:42`), never opaque URI schemes.
    if scheme.len() < 2 || !is_valid_rfc3986_scheme(scheme) {
        return Osc8SchemeKind::BarePath;
    }
    if KNOWN_OPAQUE_URI_SCHEMES
        .iter()
        .any(|known| scheme.eq_ignore_ascii_case(known))
    {
        return Osc8SchemeKind::Url;
    }
    let (_, line, _) = split_plain_location_suffix(uri, windows);
    if line.is_some() {
        // Dotted names like `report.html:12` are filenames, not reverse-DNS URIs.
        if scheme.contains('.') {
            Osc8SchemeKind::BarePath
        } else {
            Osc8SchemeKind::AmbiguousUrlOrPath
        }
    } else {
        Osc8SchemeKind::Url
    }
}

/// `/C:/Users/me` as a Windows path: a file URI's path keeps a slash before
/// the drive letter.
pub(super) fn strip_drive_slash(path: &str) -> &str {
    match path.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => &path[1..],
        _ => path,
    }
}

#[derive(Clone, Copy)]
struct LinkCell {
    point: Point,
    start: usize,
    end: usize,
}

fn link_row_content(view: &ScreenView<'_>, line: Line) -> Option<(usize, usize)> {
    let cols = view.cols();
    let first = (0..cols).find(|&column| {
        view.row_cell(line, Column(column))
            .is_some_and(|cell| !cell.c().is_whitespace())
    })?;
    let last = (first..cols).rfind(|&column| {
        view.row_cell(line, Column(column))
            .is_some_and(|cell| !cell.c().is_whitespace())
    })?;
    Some((first, last))
}

fn wrapped_hyperlink_ids_match(left: &str, right: &str) -> bool {
    left == right || (left.ends_with("_alacritty") && right.ends_with("_alacritty"))
}

enum Osc8Lookup {
    None,
    Rejected,
    Link(TerminalLink),
}

fn terminal_hyperlink_at(
    view: &ScreenView<'_>,
    point: Point,
    cwds: &[&Path],
    budget: &mut ProbeBudget<'_>,
) -> Osc8Lookup {
    let Some(hyperlink) = view.cell(point).and_then(|cell| cell.hyperlink()) else {
        return Osc8Lookup::None;
    };
    let is_separator = |column| {
        view.row_cell(point.line, Column(column))
            .is_some_and(|cell| matches!(cell.c(), '│' | '|') && cell.hyperlink().is_none())
    };
    let first = (0..point.column.0)
        .rfind(|&column| is_separator(column))
        .map_or(0, |column| column + 1);
    let last = (point.column.0 + 1..view.cols())
        .find(|&column| is_separator(column))
        .unwrap_or(view.cols());
    let columns = first..last;
    let matches_link = |point: Point| {
        view.cell(point)
            .and_then(|cell| cell.hyperlink())
            .is_some_and(|other| {
                other.uri() == hyperlink.uri()
                    && wrapped_hyperlink_ids_match(other.id(), hyperlink.id())
            })
    };
    let row_has_link = |line| {
        columns
            .clone()
            .any(|column| matches_link(Point::new(line, Column(column))))
    };
    // Redraws can assign new anonymous IDs inside a row; table padding is not link text.
    let mut start = point.line;
    while start > view.topmost_line() && row_has_link(start - 1) {
        start -= 1;
    }
    let mut end = point.line;
    while end < view.bottommost_line() && row_has_link(end + 1) {
        end += 1;
    }
    let points = (start.0..=end.0)
        .flat_map(|line| {
            columns
                .clone()
                .map(move |column| Point::new(Line(line), Column(column)))
        })
        .filter(|&point| matches_link(point));
    let uri = hyperlink.uri();
    let target = if strip_file_scheme(uri).is_some() {
        let Some(target) = file_target_from_uri(uri) else {
            return Osc8Lookup::Rejected;
        };
        target
    } else if strip_editor_file_scheme(uri).is_some() {
        let Some(target) = file_target_from_editor_uri(uri) else {
            return Osc8Lookup::Rejected;
        };
        target
    } else {
        match classify_osc8_uri(uri, cfg!(windows)) {
            Osc8SchemeKind::Url => LinkTarget::Url(uri.to_owned()),
            Osc8SchemeKind::AmbiguousUrlOrPath => resolve_osc8_bare_path(uri, cwds, budget)
                .unwrap_or_else(|| LinkTarget::Url(uri.to_owned())),
            Osc8SchemeKind::BarePath => {
                let Some(target) = resolve_osc8_bare_path(uri, cwds, budget) else {
                    return Osc8Lookup::Rejected;
                };
                target
            }
        }
    };
    Osc8Lookup::Link(TerminalLink::from_points(target, points))
}

fn resolve_osc8_bare_path(
    uri: &str,
    cwds: &[&Path],
    budget: &mut ProbeBudget<'_>,
) -> Option<LinkTarget> {
    if uri.is_empty() || uri.contains('\0') {
        return None;
    }
    let (raw_path, line, column) = split_plain_location_suffix(uri, cfg!(windows));
    if raw_path.is_empty() {
        return None;
    }
    let resolved = resolve_file_candidate_verbatim(raw_path, cwds, budget).or_else(|| {
        stripped_prefix_candidates(raw_path, cfg!(windows))
            .into_iter()
            .find_map(|(_, stripped)| resolve_file_candidate_verbatim(stripped, cwds, budget))
    })?;
    Some(LinkTarget::File {
        path: resolved,
        line,
        column,
    })
}

fn link_rows_joined(view: &ScreenView<'_>, line: Line) -> bool {
    let cols = view.cols();
    if cols == 0 {
        return false;
    }
    if view
        .row_cell(line, Column(cols - 1))
        .is_some_and(|cell| cell.flags().contains(Flags::WRAPLINE))
    {
        return true;
    }
    let next_line = Line(line.0 + 1);
    let (Some((_, last)), Some((first, _))) = (
        link_row_content(view, line),
        link_row_content(view, next_line),
    ) else {
        return false;
    };
    let prefix: String = (first..(first + MAX_URL_SCHEME_PREFIX_LEN).min(cols))
        .filter_map(|column| {
            view.row_cell(next_line, Column(column))
                .map(|cell| cell.c())
        })
        .collect();
    if starts_with_url_scheme(&prefix) {
        return false;
    }
    let Some(left) = view.row_cell(line, Column(last)) else {
        return false;
    };
    let Some(right) = view.row_cell(next_line, Column(first)) else {
        return false;
    };
    // TUIs also wrap links with explicit newlines, leaving WRAPLINE unset.
    match (left.hyperlink(), right.hyperlink()) {
        (Some(left), Some(right)) => {
            left.uri() == right.uri() && wrapped_hyperlink_ids_match(left.id(), right.id())
        }
        (None, None) => {
            let left_flags = left.flags();
            let right_flags = right.flags();
            if !left_flags.intersects(Flags::ALL_UNDERLINES)
                || left_flags & Flags::ALL_UNDERLINES != right_flags & Flags::ALL_UNDERLINES
                || left.fg() != right.fg()
                || left.bg() != right.bg()
            {
                return false;
            }
            let fragment_end = (first..cols)
                .find(|&column| {
                    view.row_cell(next_line, Column(column))
                        .is_some_and(|cell| {
                            !cell.flags().contains(Flags::WIDE_CHAR_SPACER)
                                && (cell.c().is_whitespace()
                                    || (column > first && matches!(cell.c(), '/' | '\\')))
                        })
                })
                .map_or(cols, |column| {
                    column
                        + usize::from(
                            view.row_cell(next_line, Column(column))
                                .is_some_and(|cell| matches!(cell.c(), '/' | '\\')),
                        )
                });
            // Allow a two-column TUI margin, but do not join text that would have fit.
            fragment_end - first > (cols - last - 1).saturating_sub(2)
        }
        _ => false,
    }
}

pub(super) fn terminal_link_at(
    view: &ScreenView<'_>,
    point: Point,
    cwds: &[&Path],
) -> Option<TerminalLink> {
    let mut is_file =
        |path: &Path| std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file());
    terminal_link_at_with_prober(view, point, cwds, &mut is_file)
}

pub(super) fn terminal_link_at_with_prober(
    view: &ScreenView<'_>,
    point: Point,
    cwds: &[&Path],
    is_file: &mut dyn FnMut(&Path) -> bool,
) -> Option<TerminalLink> {
    let mut budget = ProbeBudget::new(MAX_LINK_PATH_PROBES, is_file);
    terminal_link_at_with_hard_breaks(view, point, cwds, true, &mut budget)
}

fn terminal_link_at_with_hard_breaks(
    view: &ScreenView<'_>,
    point: Point,
    cwds: &[&Path],
    hard_breaks: bool,
    budget: &mut ProbeBudget<'_>,
) -> Option<TerminalLink> {
    let mut point = view.clamp_grid_point(point);
    if view.cell(point).is_some_and(|cell| {
        cell.flags()
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
    }) {
        point.column = Column(point.column.0.saturating_sub(1));
    }
    match terminal_hyperlink_at(view, point, cwds, budget) {
        Osc8Lookup::Link(link) => return Some(link),
        Osc8Lookup::Rejected => return None,
        Osc8Lookup::None => {}
    }

    let (soft_start, soft_end) = view.soft_wrapped_line_bounds(point.line);
    let mut start = soft_start;
    let mut end = soft_end;
    if hard_breaks {
        while start > view.topmost_line() && link_rows_joined(view, start - 1) {
            start -= 1;
        }
        while end < view.bottommost_line() && link_rows_joined(view, end) {
            end += 1;
        }
    }
    let (text, cells) = collect_line_range(view, start, end);

    let hit = cells.iter().position(|cell| cell.point == point)?;
    let hit_cell = cells[hit];
    if let Some(uri_range) = find_uri_range_overlapping(&text, hit_cell) {
        let uri = &text[uri_range.clone()];
        let target = if strip_file_scheme(uri).is_some() {
            file_target_from_uri(uri)?
        } else if strip_editor_file_scheme(uri).is_some() {
            file_target_from_editor_uri(uri)?
        } else {
            LinkTarget::Url(uri.to_owned())
        };
        return link_from_byte_range(target, &cells, &uri_range);
    }
    if hard_breaks && (start != soft_start || end != soft_end) {
        return terminal_link_at_with_hard_breaks(view, point, cwds, false, budget);
    }

    resolve_plain_file_link(view, point, &text, &cells, hit_cell, cwds, budget)
}

fn local_uri_prefix_len(text: &str) -> Option<usize> {
    if strip_file_scheme(text).is_some() {
        return Some(7);
    }
    let after_file = strip_editor_file_scheme(text)?;
    Some(text.len() - after_file.len())
}

/// Scans URIs left-to-right so an outer URL (such as an HTTP(S) or non-file editor
/// URL carrying a nested `file://` query parameter) owns its full range, extending
/// `file://` and `<editor>://file` tokens for balanced route segments and trailing
/// `(LINE)` / `(LINE, COL)` suffixes only when they begin at the outer URI's start.
fn find_uri_range_overlapping(text: &str, hit_cell: LinkCell) -> Option<Range<usize>> {
    let mut search_from = 0usize;
    while search_from <= hit_cell.start && search_from < text.len() {
        let matched = url_regex().find(&text[search_from..])?;
        let start = search_from + matched.start();
        if start > hit_cell.start {
            return None;
        }
        let range = if let Some(prefix_len) = local_uri_prefix_len(&text[start..]) {
            scan_local_uri(text, start, start + prefix_len)
        } else {
            start..(search_from + matched.end())
        };
        if range_overlaps_cell(&range, hit_cell) {
            return Some(range);
        }
        search_from = range.end.max(start + 1);
    }
    None
}

fn scan_local_uri(text: &str, start: usize, body_start: usize) -> Range<usize> {
    let mut cur = body_start;
    let mut delim_stack: Vec<(char, usize)> = Vec::new();
    let mut ended_with_paren_suffix = false;

    while cur < text.len() {
        let ch = text[cur..].chars().next().unwrap();
        match ch {
            '(' => {
                if delim_stack.is_empty() {
                    if let Some(suffix_end) = trailing_paren_location_end(text, cur, true) {
                        cur = suffix_end;
                        ended_with_paren_suffix = true;
                        break;
                    }
                }
                delim_stack.push((')', cur));
                cur += 1;
            }
            '[' => {
                delim_stack.push((']', cur));
                cur += 1;
            }
            ')' | ']' => {
                if delim_stack
                    .last()
                    .is_some_and(|&(expected, _)| expected == ch)
                {
                    delim_stack.pop();
                    cur += 1;
                } else {
                    break;
                }
            }
            c if is_local_uri_terminator(c) => break,
            c => cur += c.len_utf8(),
        }
    }

    if let Some(&(_, unclosed)) = delim_stack.first() {
        cur = unclosed;
        ended_with_paren_suffix = false;
    }

    if !ended_with_paren_suffix {
        while cur > body_start {
            let last = text[..cur].chars().next_back().unwrap();
            if matches!(last, ':' | ',' | '.' | '!' | '?' | '~' | ';') {
                cur -= last.len_utf8();
            } else {
                break;
            }
        }
    }
    start..cur
}

fn is_local_uri_terminator(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '"' | '\''
                | '`'
                | '{'
                | '}'
                | '<'
                | '>'
                | '|'
                | '^'
                | '!'
                | '*'
                | '，'
                | '。'
                | '、'
                | '：'
                | '；'
                | '（'
                | '）'
                | '「'
                | '」'
                | '【'
                | '】'
                | '—'
                | '…'
                | '“'
                | '”'
                | '‘'
                | '’'
        )
        || ('\u{2500}'..='\u{257F}').contains(&c)
}

fn collect_line_range(view: &ScreenView<'_>, start: Line, end: Line) -> (String, Vec<LinkCell>) {
    let cols = view.cols();
    let mut text = String::new();
    let mut cells = Vec::new();
    if cols == 0 {
        return (text, cells);
    }
    for line in start.0..=end.0 {
        let line = Line(line);
        let (first, last) = link_row_content(view, line).unwrap_or((0, cols - 1));
        let hard_break_before = line > start
            && !view
                .row_cell(Line(line.0 - 1), Column(cols - 1))
                .is_some_and(|cell| cell.flags().contains(Flags::WRAPLINE));
        let hard_break_after = line < end
            && !view
                .row_cell(line, Column(cols - 1))
                .is_some_and(|cell| cell.flags().contains(Flags::WRAPLINE));
        let first = if hard_break_before { first } else { 0 };
        let last = if hard_break_after { last } else { cols - 1 };
        append_row_cells(view, line, first, last, &mut text, &mut cells);
    }
    (text, cells)
}

fn append_row_cells(
    view: &ScreenView<'_>,
    line: Line,
    first: usize,
    last: usize,
    text: &mut String,
    cells: &mut Vec<LinkCell>,
) {
    for column in first..=last {
        let cell_point = Point::new(line, Column(column));
        let Some(cell) = view.cell(cell_point) else {
            continue;
        };
        if cell
            .flags()
            .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
        {
            continue;
        }
        let rendered_start = text.len();
        text.push(cell.c());
        if let Some(zerowidth) = cell.zerowidth() {
            text.extend(zerowidth.iter());
        }
        cells.push(LinkCell {
            point: cell_point,
            start: rendered_start,
            end: text.len(),
        });
    }
}

fn range_overlaps_cell(range: &Range<usize>, cell: LinkCell) -> bool {
    range.start < cell.end && range.end > cell.start
}

fn link_from_byte_range(
    target: LinkTarget,
    cells: &[LinkCell],
    range: &Range<usize>,
) -> Option<TerminalLink> {
    let first = cells.iter().position(|cell| cell.end > range.start)?;
    let last = cells.iter().rposition(|cell| cell.start < range.end)?;
    Some(TerminalLink::from_points(
        target,
        cells[first..=last].iter().map(|cell| cell.point),
    ))
}

fn link_ends_at_row_last_non_blank(view: &ScreenView<'_>, link: &TerminalLink) -> bool {
    link_row_content(view, link.end.line).is_some_and(|(_, last)| link.end.column.0 == last)
}

#[derive(Clone, Debug)]
pub(super) struct ParsedPathCandidate {
    pub(super) full_range: Range<usize>,
    path_end: usize,
    pub(super) path: String,
    pub(super) line: Option<u32>,
    pub(super) column: Option<u32>,
}

struct HardWrappedCandidate {
    text: String,
    cells: Vec<LinkCell>,
    row_count: usize,
}

fn preceded_by_uri_scheme(text: &str, start: usize) -> bool {
    let before = &text[..start];
    let after = &text[start..];
    let head = if let Some(head) = before.strip_suffix("://") {
        head
    } else if let Some(head) = before.strip_suffix(":/").filter(|_| after.starts_with('/')) {
        head
    } else if let Some(head) = before.strip_suffix(':').filter(|_| after.starts_with("//")) {
        head
    } else {
        return false;
    };
    let scheme_start = head
        .char_indices()
        .rfind(|(_, c)| !(c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
        .map_or(0, |(index, ch)| index + ch.len_utf8());
    let scheme = &head[scheme_start..];
    scheme
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphabetic)
}

/// Resolves plain-text file links under `point` in the spec's probe-budget order:
/// 1. single-line match (verbatim);
/// 2. hard-wrap joins, longest first (verbatim);
/// 3. spaced readings (verbatim);
/// 4. prefix-stripped forms (`@`, then `a/` or `b/`).
fn resolve_plain_file_link(
    view: &ScreenView<'_>,
    point: Point,
    text: &str,
    cells: &[LinkCell],
    hit_cell: LinkCell,
    cwds: &[&Path],
    budget: &mut ProbeBudget<'_>,
) -> Option<TerminalLink> {
    let single_candidates = scan_plain_file_candidates(text);
    let single_candidate = single_candidates
        .iter()
        .find(|candidate| range_overlaps_cell(&candidate.full_range, hit_cell))
        .filter(|candidate| !preceded_by_uri_scheme(text, candidate.full_range.start))
        .cloned();

    // 1. Single-line match (verbatim).
    let mut single_fallback = None;
    if let Some(candidate) = single_candidate.as_ref() {
        if let Some(path) = resolve_file_candidate_verbatim(&candidate.path, cwds, budget) {
            let link = link_from_byte_range(
                LinkTarget::File {
                    path,
                    line: candidate.line,
                    column: candidate.column,
                },
                cells,
                &candidate.full_range,
            )?;
            if !link_ends_at_row_last_non_blank(view, &link) {
                return Some(link);
            }
            single_fallback = Some(link);
        }
    }

    // 2. Hard-wrap joins, longest first (verbatim).
    let hard_wrapped = build_hard_wrapped_candidates(view, point);
    let mut hard_wrapped_matches: Vec<(usize, ParsedPathCandidate)> = Vec::new();
    for (index, candidate) in hard_wrapped.iter().enumerate() {
        if budget.exhausted() {
            break;
        }
        let Some(hit) = candidate.cells.iter().position(|cell| cell.point == point) else {
            continue;
        };
        let wrapped_hit = candidate.cells[hit];
        let Some(parsed) = scan_plain_file_candidates(&candidate.text)
            .into_iter()
            .find(|parsed| range_overlaps_cell(&parsed.full_range, wrapped_hit))
            .filter(|parsed| !preceded_by_uri_scheme(&candidate.text, parsed.full_range.start))
        else {
            continue;
        };
        if !match_spans_multiple_rows(&candidate.cells, &parsed.full_range) {
            continue;
        }
        if let Some(path) = resolve_file_candidate_verbatim(&parsed.path, cwds, budget) {
            if let Some(link) = link_from_byte_range(
                LinkTarget::File {
                    path,
                    line: parsed.line,
                    column: parsed.column,
                },
                &candidate.cells,
                &parsed.full_range,
            ) {
                return Some(link);
            }
        }
        hard_wrapped_matches.push((index, parsed));
    }

    if let Some(link) = single_fallback {
        return Some(link);
    }

    if budget.exhausted() {
        return None;
    }

    // 3. Spaced readings (verbatim).
    let spaced_candidates = collect_spaced_candidates(text, &single_candidates, hit_cell);
    for candidate in &spaced_candidates {
        if budget.exhausted() {
            break;
        }
        if let Some(path) = resolve_file_candidate_verbatim(&candidate.path, cwds, budget) {
            if let Some(link) = link_from_byte_range(
                LinkTarget::File {
                    path,
                    line: candidate.line,
                    column: candidate.column,
                },
                cells,
                &candidate.full_range,
            ) {
                return Some(link);
            }
        }
    }

    if budget.exhausted() {
        return None;
    }

    // 4. Prefix-stripped forms (`@`, then `a/` or `b/`).
    let mut stripped_single_fallback = None;
    if let Some(candidate) = single_candidate.as_ref() {
        if let Some(link) =
            resolve_prefix_stripped_candidate(candidate, cells, hit_cell, cwds, budget)
        {
            if !link_ends_at_row_last_non_blank(view, &link) {
                return Some(link);
            }
            stripped_single_fallback = Some(link);
        }
    }

    for (index, parsed) in &hard_wrapped_matches {
        if budget.exhausted() {
            break;
        }
        let candidate = &hard_wrapped[*index];
        let Some(hit) = candidate.cells.iter().position(|cell| cell.point == point) else {
            continue;
        };
        let wrapped_hit = candidate.cells[hit];
        if let Some(link) =
            resolve_prefix_stripped_candidate(parsed, &candidate.cells, wrapped_hit, cwds, budget)
        {
            if link.start.line != link.end.line {
                return Some(link);
            }
        }
    }

    if let Some(link) = stripped_single_fallback {
        return Some(link);
    }

    for candidate in &spaced_candidates {
        if budget.exhausted() {
            break;
        }
        if let Some(link) =
            resolve_prefix_stripped_candidate(candidate, cells, hit_cell, cwds, budget)
        {
            return Some(link);
        }
    }

    None
}

fn match_spans_multiple_rows(cells: &[LinkCell], range: &Range<usize>) -> bool {
    let Some(first) = cells.iter().position(|cell| cell.end > range.start) else {
        return false;
    };
    let Some(last) = cells.iter().rposition(|cell| cell.start < range.end) else {
        return false;
    };
    cells[first].point.line != cells[last].point.line
}

fn resolve_prefix_stripped_candidate(
    candidate: &ParsedPathCandidate,
    cells: &[LinkCell],
    hit_cell: LinkCell,
    cwds: &[&Path],
    budget: &mut ProbeBudget<'_>,
) -> Option<TerminalLink> {
    for (prefix_len, stripped) in stripped_prefix_candidates(&candidate.path, cfg!(windows)) {
        if budget.exhausted() {
            return None;
        }
        let stripped_range = (candidate.full_range.start + prefix_len)..candidate.full_range.end;
        if !range_overlaps_cell(&stripped_range, hit_cell) {
            continue;
        }
        if let Some(path) = resolve_file_candidate_verbatim(stripped, cwds, budget) {
            return link_from_byte_range(
                LinkTarget::File {
                    path,
                    line: candidate.line,
                    column: candidate.column,
                },
                cells,
                &stripped_range,
            );
        }
    }
    None
}

fn strip_git_diff_prefix(s: &str, windows: bool) -> Option<(usize, &str)> {
    if let Some(rest) = s.strip_prefix("a/").or_else(|| s.strip_prefix("b/")) {
        return (!rest.is_empty()).then_some((2, rest));
    }
    if windows {
        if let Some(rest) = s.strip_prefix("a\\").or_else(|| s.strip_prefix("b\\")) {
            return (!rest.is_empty()).then_some((2, rest));
        }
    }
    None
}

pub(super) fn stripped_prefix_candidates(candidate: &str, windows: bool) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    if let Some(after_at) = candidate.strip_prefix('@') {
        if !after_at.is_empty() {
            out.push((1, after_at));
            if let Some((diff_len, after_diff)) = strip_git_diff_prefix(after_at, windows) {
                out.push((1 + diff_len, after_diff));
            }
        }
    } else if let Some((diff_len, after_diff)) = strip_git_diff_prefix(candidate, windows) {
        out.push((diff_len, after_diff));
    }
    out
}

// ── Hard-wrapped file path candidates (Orca's continuation rule) ──────────

/// One soft-wrapped logical line (`WRAPLINE` boundaries intact), trimmed only at
/// its leading and trailing hard-break edges.
struct TrimmedLogicalLine {
    start_line: Line,
    end_line: Line,
    text: String,
    cells: Vec<LinkCell>,
}

impl TrimmedLogicalLine {
    fn slice(&self, byte_range: Range<usize>) -> (&str, Vec<LinkCell>) {
        let slice = &self.text[byte_range.clone()];
        let offset = byte_range.start;
        let cells = self
            .cells
            .iter()
            .filter_map(|cell| {
                let start = cell.start.max(byte_range.start);
                let end = cell.end.min(byte_range.end);
                (start < end).then(|| LinkCell {
                    point: cell.point,
                    start: start - offset,
                    end: end - offset,
                })
            })
            .collect();
        (slice, cells)
    }
}

fn trim_logical_line(
    view: &ScreenView<'_>,
    start_line: Line,
    end_line: Line,
) -> Option<TrimmedLogicalLine> {
    let cols = view.cols();
    if cols == 0 {
        return None;
    }
    let mut raw_text = String::new();
    let mut raw_cells = Vec::new();
    for line in start_line.0..=end_line.0 {
        append_row_cells(view, Line(line), 0, cols - 1, &mut raw_text, &mut raw_cells);
    }
    let first_non_ws = raw_text.find(|c: char| !c.is_whitespace())?;
    let last_non_ws_start = raw_text.rfind(|c: char| !c.is_whitespace())?;
    let last_non_ws_end =
        last_non_ws_start + raw_text[last_non_ws_start..].chars().next()?.len_utf8();
    let text = raw_text[first_non_ws..last_non_ws_end].to_owned();
    let cells = raw_cells
        .into_iter()
        .filter_map(|cell| {
            let start = cell.start.max(first_non_ws);
            let end = cell.end.min(last_non_ws_end);
            (start < end).then(|| LinkCell {
                point: cell.point,
                start: start - first_non_ws,
                end: end - first_non_ws,
            })
        })
        .collect();
    (!text.is_empty()).then_some(TrimmedLogicalLine {
        start_line,
        end_line,
        text,
        cells,
    })
}

fn soft_wrapped_bounds(view: &ScreenView<'_>, line: Line) -> (Line, Line) {
    view.soft_wrapped_line_bounds(line)
}

fn hard_wrap_fragment_full_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^[\p{L}\p{N}\p{M}._~@%+=:,/\\-]+$").expect("valid hard-wrap fragment regex")
    })
}

fn hard_wrap_fragment_suffix_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"[\p{L}\p{N}\p{M}._~@%+=:,/\\-]+$").expect("valid hard-wrap suffix regex")
    })
}

fn hard_wrap_fragment_prefix_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^[\p{L}\p{N}\p{M}._~@%+=:,/\\-]+").expect("valid hard-wrap prefix regex")
    })
}

fn hard_wrap_trailing_location_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^(?:\(\d+(?:,\s?\d+)?\)|#L\d+(?:C\d+)?(?:[-–]L\d+(?:C\d+)?)?)")
            .expect("valid hard-wrap trailing location regex")
    })
}

fn has_alnum(text: &str) -> bool {
    text.chars().any(char::is_alphanumeric)
}

fn is_incomplete_hard_wrapped_path_start(text: &str) -> bool {
    matches!(
        text,
        "/" | "\\" | "./" | "../" | ".\\" | "..\\" | "~/" | "~\\"
    ) || (text.len() == 2 && has_windows_drive_prefix(text))
}

fn is_hard_wrapped_path_continuation(text: &str) -> bool {
    !starts_with_url_scheme(text)
        && !starts_with_any_authority_scheme(text)
        && hard_wrap_fragment_full_regex().is_match(text)
        && (has_alnum(text) || is_incomplete_hard_wrapped_path_start(text))
}

fn can_start_hard_wrapped_path(text: &str) -> bool {
    if starts_with_url_scheme(text) || starts_with_any_authority_scheme(text) {
        return false;
    }
    text.contains('/') || text.contains('\\') || is_incomplete_hard_wrapped_path_start(text)
}

fn hard_wrap_row_prefix_end(text: &str) -> usize {
    let base_end = hard_wrap_fragment_prefix_regex()
        .find(text)
        .map_or(0, |matched| matched.end());
    let rest = &text[base_end..];
    let suffix_len = hard_wrap_trailing_location_regex()
        .find(rest)
        .map_or(0, |matched| matched.end());
    base_end + suffix_len
}

fn build_hard_wrapped_candidates(view: &ScreenView<'_>, point: Point) -> Vec<HardWrappedCandidate> {
    let max_offset = (MAX_WRAP_JOIN_ROWS - 1) as i32;
    let topmost = view.topmost_line();
    let bottommost = view.bottommost_line();
    let min_physical = Line((point.line.0 - max_offset).max(topmost.0));
    let max_physical = Line((point.line.0 + max_offset).min(bottommost.0));

    let (cur_soft_start, cur_soft_end) = soft_wrapped_bounds(view, point.line);
    let cur_clamped_start = Line(cur_soft_start.0.max(min_physical.0));
    let cur_clamped_end = Line(cur_soft_end.0.min(max_physical.0));
    let Some(current_logical) = trim_logical_line(view, cur_clamped_start, cur_clamped_end) else {
        return Vec::new();
    };

    // Collect logical lines above `current_logical` (up to `min_physical`).
    let mut above: Vec<Option<TrimmedLogicalLine>> = Vec::new();
    let mut cursor = cur_soft_start;
    while cursor > min_physical {
        let prev_end = Line(cursor.0 - 1);
        let (prev_start, _) = soft_wrapped_bounds(view, prev_end);
        let clamped_start = Line(prev_start.0.max(min_physical.0));
        let item = trim_logical_line(view, clamped_start, prev_end);
        let stop = match &item {
            None => true,
            Some(row) => !is_hard_wrapped_path_continuation(&row.text),
        };
        above.push(item);
        if stop || prev_start <= min_physical {
            break;
        }
        cursor = prev_start;
    }
    above.reverse();
    let current_index = above.len();

    // Collect logical lines below `current_logical` (up to `max_physical`).
    let mut logical_lines: Vec<Option<TrimmedLogicalLine>> = above;
    logical_lines.push(Some(current_logical));
    let mut cursor = cur_soft_end;
    while cursor < max_physical {
        let next_start = Line(cursor.0 + 1);
        let (_, next_end) = soft_wrapped_bounds(view, next_start);
        let clamped_end = Line(next_end.0.min(max_physical.0));
        let item = trim_logical_line(view, next_start, clamped_end);
        let is_blank = item.is_none();
        logical_lines.push(item);
        if is_blank || next_end >= max_physical {
            break;
        }
        cursor = next_end;
    }

    let mut candidates = Vec::new();
    for start_idx in (0..=current_index).rev() {
        let Some(start_row) = logical_lines[start_idx].as_ref() else {
            break;
        };
        let Some(suffix_match) = hard_wrap_fragment_suffix_regex().find(&start_row.text) else {
            continue;
        };
        if !can_start_hard_wrapped_path(suffix_match.as_str()) {
            continue;
        }
        let (start_slice, start_cells) = start_row.slice(suffix_match.range());
        let first_physical = start_cells
            .first()
            .map_or(start_row.start_line, |cell| cell.point.line);
        let mut joined_text = start_slice.to_owned();
        let mut joined_cells = start_cells;

        for (next_idx, next_row) in logical_lines.iter().enumerate().skip(start_idx + 1) {
            let Some(next_row) = next_row.as_ref() else {
                break;
            };
            if (next_row.start_line.0 - first_physical.0 + 1) as usize > MAX_WRAP_JOIN_ROWS {
                break;
            }
            if starts_with_url_scheme(&next_row.text)
                || starts_with_any_authority_scheme(&next_row.text)
            {
                break;
            }
            if is_hard_wrapped_path_continuation(&next_row.text) {
                let span_rows = (next_row.end_line.0 - first_physical.0 + 1) as usize;
                if span_rows > MAX_WRAP_JOIN_ROWS {
                    break;
                }
                let offset = joined_text.len();
                joined_text.push_str(&next_row.text);
                joined_cells.extend(next_row.cells.iter().map(|cell| LinkCell {
                    point: cell.point,
                    start: cell.start + offset,
                    end: cell.end + offset,
                }));
                if next_idx >= current_index {
                    candidates.push(HardWrappedCandidate {
                        text: joined_text.clone(),
                        cells: joined_cells.clone(),
                        row_count: span_rows,
                    });
                }
                continue;
            }

            // First mixed logical line adds only its leading path run (plus any
            // immediate trailing location suffix) and ends the join.
            let prefix_end = hard_wrap_row_prefix_end(&next_row.text);
            if prefix_end > 0 {
                let (prefix_slice, prefix_cells) = next_row.slice(0..prefix_end);
                if has_alnum(prefix_slice)
                    && !starts_with_url_scheme(prefix_slice)
                    && !starts_with_any_authority_scheme(prefix_slice)
                {
                    let offset = joined_text.len();
                    joined_text.push_str(prefix_slice);
                    joined_cells.extend(prefix_cells.into_iter().map(|cell| LinkCell {
                        point: cell.point,
                        start: cell.start + offset,
                        end: cell.end + offset,
                    }));
                    let last_physical = joined_cells
                        .last()
                        .map_or(next_row.start_line, |cell| cell.point.line);
                    let actual_span = (last_physical.0 - first_physical.0 + 1) as usize;
                    if next_idx >= current_index && actual_span <= MAX_WRAP_JOIN_ROWS {
                        candidates.push(HardWrappedCandidate {
                            text: joined_text.clone(),
                            cells: joined_cells.clone(),
                            row_count: actual_span,
                        });
                    }
                }
            }
            break;
        }
    }

    candidates.sort_by(|left, right| {
        right
            .row_count
            .cmp(&left.row_count)
            .then_with(|| right.text.len().cmp(&left.text.len()))
    });
    candidates
}

// ── Spaced path growth (Orca's growth rule) ───────────────────────────────

fn is_spaced_word_delimiter(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '(' | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '<'
                | '>'
                | '\''
                | '"'
                | '`'
                | ','
                | ';'
                | '|'
                | '，'
                | '。'
                | '、'
                | '：'
                | '；'
                | '（'
                | '）'
                | '「'
                | '」'
                | '【'
                | '】'
                | '—'
                | '…'
                | '“'
                | '”'
                | '‘'
                | '’'
        )
        || ('\u{2500}'..='\u{257F}').contains(&c)
}

fn is_rooted_path_start(text: &str) -> bool {
    let text = text.strip_prefix('@').unwrap_or(text);
    text.starts_with('/')
        || text.starts_with('\\')
        || text.starts_with("~/")
        || text.starts_with("~\\")
        || text.starts_with("./")
        || text.starts_with(".\\")
        || text.starts_with("../")
        || text.starts_with("..\\")
        || is_windows_drive_path(text)
}

fn filename_word_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"(?x)
            \.[\p{L}\p{N}][\p{L}\p{N}\p{M}_+-]*
            (?:
                :\d+(?::\d+(?:[-–]\d+(?::\d+)?)?|[-–]\d+)?
              | \(\d+(?:,\s?\d+)?\)
              | \#L\d+(?:C\d+)?(?:[-–]L\d+(?:C\d+)?)?
            )?
            [.,;:!?]*$
            ",
        )
        .expect("valid filename word regex")
    })
}

fn spaced_paren_location_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\(\d+(?:,\s?\d+)?\)").expect("valid spaced paren location regex")
    })
}

fn collect_spaced_candidates(
    text: &str,
    base_candidates: &[ParsedPathCandidate],
    hit_cell: LinkCell,
) -> Vec<ParsedPathCandidate> {
    if !text.contains('/') && !text.contains('\\') {
        return Vec::new();
    }
    let mut results = Vec::new();
    let mut grown_word_end = 0usize;

    for base in base_candidates {
        let start = base.full_range.start;
        if start > hit_cell.start {
            break;
        }
        if start < grown_word_end || preceded_by_uri_scheme(text, start) {
            continue;
        }
        if !base.path.contains('/') && !base.path.contains('\\') {
            continue;
        }
        // A path that already carried a location suffix is complete as written.
        if base.full_range.end > base.path_end {
            continue;
        }

        // Continue to the end of the base token's own word (e.g. `C#` in `C# Projects`).
        let rest_from_end = &text[base.full_range.end..];
        let cont_len = rest_from_end
            .find(is_spaced_word_delimiter)
            .unwrap_or(rest_from_end.len());
        let continuation = &rest_from_end[..cont_len];
        let word_end = base.full_range.end + cont_len;
        grown_word_end = word_end;
        if continuation.contains("://") || continuation.contains('/') || continuation.contains('\\')
        {
            continue;
        }

        let relative = !is_rooted_path_start(&base.path);
        if relative && filename_word_regex().is_match(&text[start..word_end]) {
            continue;
        }

        let mut cursor = word_end;
        for _ in 0..MAX_SPACED_PATH_WORDS {
            let Some(after_space) = text[cursor..].strip_prefix(' ') else {
                break;
            };
            let word_len = after_space
                .find(is_spaced_word_delimiter)
                .unwrap_or(after_space.len());
            if word_len == 0 {
                break;
            }
            let word = &after_space[..word_len];
            if is_rooted_path_start(word) || word.contains("://") {
                break;
            }
            cursor += 1 + word_len;

            let paren_len = spaced_paren_location_regex()
                .find(&text[cursor..])
                .map_or(0, |matched| matched.end());
            let raw_end = cursor + paren_len;
            let trimmed_end = if paren_len > 0 {
                raw_end
            } else {
                trim_trailing_sentence_punctuation(&text[start..raw_end])
                    .map_or(start, |len| start + len)
            };
            if trimmed_end <= start {
                break;
            }

            let word_with_paren = &text[cursor - word_len..raw_end];
            let is_filename = filename_word_regex().is_match(word_with_paren);
            if relative && !is_filename {
                if paren_len > 0 {
                    break;
                }
                continue;
            }

            let candidate_range = start..trimmed_end;
            if range_overlaps_cell(&candidate_range, hit_cell) {
                let candidate_text = &text[candidate_range.clone()];
                let (candidate_path, line, column) =
                    split_plain_location_suffix(candidate_text, cfg!(windows));
                if !candidate_path.is_empty() {
                    let path_end = candidate_range.start + candidate_path.len();
                    results.push(ParsedPathCandidate {
                        full_range: candidate_range,
                        path_end,
                        path: candidate_path.to_owned(),
                        line,
                        column,
                    });
                }
            }

            if relative || paren_len > 0 {
                break;
            }
        }
    }
    results
}

fn trim_trailing_sentence_punctuation(text: &str) -> Option<usize> {
    let trimmed = text.trim_end_matches(['.', ',', ';', ':', '!', '?']);
    (!trimmed.is_empty()).then_some(trimmed.len())
}

// ── Allow-list plain-text file scanner and location suffix parsers ────────

fn unicode_mark_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^\p{M}").expect("valid unicode mark regex"))
}

fn is_unicode_mark(c: char) -> bool {
    if c.is_ascii() {
        return false;
    }
    let mut buf = [0u8; 4];
    unicode_mark_regex().is_match(c.encode_utf8(&mut buf))
}

fn is_base_path_char(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(c, '.' | '_' | '-' | '~' | '%' | '+' | '@')
        || is_unicode_mark(c)
}

fn is_path_sep(c: char) -> bool {
    matches!(c, '/' | '\\')
}

fn leading_root_prefix_len(text: &str) -> Option<usize> {
    if text.starts_with("~/") || text.starts_with("~\\") {
        return Some(2);
    }
    if text.starts_with("\\\\") {
        return Some(2);
    }
    if is_windows_drive_path(text) {
        return Some(3);
    }
    if text.starts_with('/') {
        return Some(1);
    }
    let mut consumed = 0usize;
    let mut rest = text;
    while let Some(next) = rest
        .strip_prefix("./")
        .or_else(|| rest.strip_prefix(".\\"))
        .map(|tail| (2, tail))
        .or_else(|| {
            rest.strip_prefix("../")
                .or_else(|| rest.strip_prefix("..\\"))
                .map(|tail| (3, tail))
        })
    {
        consumed += next.0;
        rest = next.1;
    }
    (consumed > 0).then_some(consumed)
}

fn leading_colon_location_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^:(?P<line>\d+)(?::(?P<column>\d+)(?:[-–]\d+(?::\d+)?)?|[-–]\d+)?")
            .expect("valid leading colon location regex")
    })
}

fn leading_paren_location_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^\((?P<line>\d+)(?:,\s?(?P<column>\d+))?\)")
            .expect("valid leading paren location regex")
    })
}

fn leading_hash_location_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^#L(?P<line>\d+)(?:C(?P<column>\d+))?(?:[-–]L\d+(?:C\d+)?)?")
            .expect("valid leading hash location regex")
    })
}

/// Returns the end byte offset of a trailing `(LINE)` or `(LINE, COL)` suffix
/// starting at `paren_start`, or `None` when the parenthesized group continues
/// inside a path (such as `app/(1)/page.tsx` or `report(1).pdf`).
fn trailing_paren_location_end(
    text: &str,
    paren_start: usize,
    allow_interior_parens: bool,
) -> Option<usize> {
    let captures = leading_paren_location_regex().captures(&text[paren_start..])?;
    let line = captures.name("line")?.as_str().parse::<u32>().ok();
    line?;
    if let Some(col) = captures.name("column") {
        col.as_str().parse::<u32>().ok()?;
    }
    let end = paren_start + captures.get(0)?.end();
    if allow_interior_parens {
        let rest = &text[end..];
        if let Some(next) = rest.chars().next() {
            if is_path_sep(next) || matches!(next, '(' | '[') {
                return None;
            }
            if next == '.' {
                if rest[1..].chars().next().is_some_and(is_base_path_char) {
                    return None;
                }
            } else if is_base_path_char(next) {
                return None;
            }
        }
    }
    Some(end)
}

fn parse_leading_paren_location(
    text: &str,
    paren_start: usize,
    allow_interior_parens: bool,
) -> Option<(usize, u32, Option<u32>)> {
    let end = trailing_paren_location_end(text, paren_start, allow_interior_parens)?;
    let captures = leading_paren_location_regex().captures(&text[paren_start..end])?;
    let line = captures.name("line")?.as_str().parse::<u32>().ok()?;
    let column = match captures.name("column") {
        Some(matched) => Some(matched.as_str().parse::<u32>().ok()?),
        None => None,
    };
    Some((end, line, column))
}

fn parse_leading_colon_location(text: &str, start: usize) -> Option<(usize, u32, Option<u32>)> {
    let captures = leading_colon_location_regex().captures(&text[start..])?;
    let line = captures.name("line")?.as_str().parse::<u32>().ok()?;
    let column = match captures.name("column") {
        Some(matched) => Some(matched.as_str().parse::<u32>().ok()?),
        None => None,
    };
    Some((start + captures.get(0)?.end(), line, column))
}

fn parse_leading_hash_location(text: &str, start: usize) -> Option<(usize, u32, Option<u32>)> {
    let captures = leading_hash_location_regex().captures(&text[start..])?;
    let line = captures.name("line")?.as_str().parse::<u32>().ok()?;
    let column = match captures.name("column") {
        Some(matched) => Some(matched.as_str().parse::<u32>().ok()?),
        None => None,
    };
    Some((start + captures.get(0)?.end(), line, column))
}

/// Precomputes in O(N) which `(` and `[` byte offsets close cleanly within a
/// contiguous run of path characters and nested delimiters, so scanning never
/// rewinds on unclosed openers.
fn matched_path_openers(text: &str) -> Vec<bool> {
    if !text.as_bytes().iter().any(|&b| b == b'(' || b == b'[') {
        return Vec::new();
    }
    let mut matched = vec![false; text.len()];
    let mut stack: Vec<(char, usize)> = Vec::new();
    for (idx, ch) in text.char_indices() {
        match ch {
            '(' => stack.push((')', idx)),
            '[' => stack.push((']', idx)),
            ')' | ']' => {
                if stack.last().is_some_and(|&(expected, _)| expected == ch) {
                    let (_, open_idx) = stack.pop().unwrap();
                    matched[open_idx] = true;
                } else {
                    stack.clear();
                }
            }
            c if is_base_path_char(c) || is_path_sep(c) => {}
            _ => stack.clear(),
        }
    }
    matched
}

/// Scans `text` in linear time for path-shaped tokens using the allow-list
/// character class, permitting arbitrarily nested balanced `(...)` and `[...]`
/// after the first separator, stopping at any unmatched closer, and parsing
/// location suffixes.
pub(super) fn scan_plain_file_candidates(text: &str) -> Vec<ParsedPathCandidate> {
    let matched_openers = matched_path_openers(text);
    let mut results = Vec::new();
    let mut pos = 0usize;

    while pos < text.len() {
        let rest = &text[pos..];
        let (start, mut cur, mut has_sep) = if let Some(prefix_len) = leading_root_prefix_len(rest)
        {
            (pos, pos + prefix_len, true)
        } else {
            let ch = rest.chars().next().unwrap();
            if !is_base_path_char(ch) {
                pos += ch.len_utf8();
                continue;
            }
            (pos, pos + ch.len_utf8(), false)
        };

        let mut delim_stack: Vec<char> = Vec::new();
        let mut suffix: Option<(usize, usize, u32, Option<u32>)> = None;

        while cur < text.len() {
            let ch = text[cur..].chars().next().unwrap();
            match ch {
                c if is_path_sep(c) => {
                    has_sep = true;
                    cur += c.len_utf8();
                }
                c if is_base_path_char(c) => {
                    cur += c.len_utf8();
                }
                '(' => {
                    if delim_stack.is_empty() && cur > start {
                        if let Some((end, line, col)) =
                            parse_leading_paren_location(text, cur, has_sep)
                        {
                            suffix = Some((cur, end, line, col));
                            break;
                        }
                    }
                    if !has_sep || !matched_openers.get(cur).copied().unwrap_or(false) {
                        break;
                    }
                    delim_stack.push(')');
                    cur += 1;
                }
                '[' => {
                    if !has_sep || !matched_openers.get(cur).copied().unwrap_or(false) {
                        break;
                    }
                    delim_stack.push(']');
                    cur += 1;
                }
                ')' | ']' => {
                    if delim_stack.last() == Some(&ch) {
                        delim_stack.pop();
                        cur += 1;
                    } else {
                        break;
                    }
                }
                ':' => {
                    if delim_stack.is_empty() && cur > start {
                        if let Some((end, line, col)) = parse_leading_colon_location(text, cur) {
                            suffix = Some((cur, end, line, col));
                        }
                    }
                    break;
                }
                '#' => {
                    if delim_stack.is_empty() && cur > start {
                        if let Some((end, line, col)) = parse_leading_hash_location(text, cur) {
                            suffix = Some((cur, end, line, col));
                        }
                    }
                    break;
                }
                _ => break,
            }
        }

        let next_pos = suffix.map_or(cur, |(_, end, _, _)| end);
        let (mut path_end, full_end, line, column) = match suffix {
            Some((p_end, f_end, line, col)) => (p_end, f_end, Some(line), col),
            None => (cur, cur, None, None),
        };
        if line.is_none() {
            while path_end > start {
                let last = text[..path_end].chars().next_back().unwrap();
                if last == '.' || is_path_sep(last) {
                    path_end -= last.len_utf8();
                } else {
                    break;
                }
            }
        }
        let full_end = if line.is_none() { path_end } else { full_end };

        let valid_path = path_end > start && {
            let candidate = &text[start..path_end];
            !candidate.ends_with('.')
                && !candidate.ends_with('/')
                && !candidate.ends_with('\\')
                && !candidate.ends_with(':')
        };
        if valid_path {
            results.push(ParsedPathCandidate {
                full_range: start..full_end,
                path_end,
                path: text[start..path_end].to_owned(),
                line,
                column,
            });
        }

        if next_pos <= start {
            pos = start + text[start..].chars().next().unwrap().len_utf8();
        } else {
            pos = next_pos;
        }
    }

    results
}

fn colon_suffix_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r":(?P<line>\d+)(?::(?P<column>\d+)(?:[-–]\d+(?::\d+)?)?|[-–]\d+)?$")
            .expect("valid colon suffix regex")
    })
}

fn paren_suffix_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"\((?P<line>\d+)(?:,\s?(?P<column>\d+))?\)$").expect("valid paren suffix regex")
    })
}

fn hash_location_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"^L(?P<line>\d+)(?:C(?P<column>\d+))?(?:[-–]L\d+(?:C\d+)?)?$")
            .expect("valid hash location regex")
    })
}

fn parse_hash_location(fragment: &str) -> Option<(Option<u32>, Option<u32>)> {
    let captures = hash_location_regex().captures(fragment)?;
    let line = captures.name("line")?.as_str().parse::<u32>().ok()?;
    let column = match captures.name("column") {
        Some(matched) => Some(matched.as_str().parse::<u32>().ok()?),
        None => None,
    };
    Some((Some(line), column))
}

fn is_bare_drive_head(head: &str) -> bool {
    matches!(
        head.as_bytes(),
        [drive] | [b'/' | b'\\', drive] if drive.is_ascii_alphabetic()
    )
}

fn split_line_suffix(path: &str, windows: bool) -> (&str, Option<u32>, Option<u32>) {
    if let Some(captures) = colon_suffix_regex().captures(path) {
        if let Some(full) = captures.get(0) {
            let head = &path[..full.start()];
            let blocked_by_drive = windows && is_bare_drive_head(head);
            if !head.is_empty() && head != "/" && !blocked_by_drive {
                if let Some(line) = captures
                    .name("line")
                    .and_then(|matched| matched.as_str().parse::<u32>().ok())
                {
                    let column = match captures.name("column") {
                        Some(matched) => match matched.as_str().parse::<u32>().ok() {
                            Some(col) => Some(col),
                            None => return (path, None, None),
                        },
                        None => None,
                    };
                    return (head, Some(line), column);
                }
            }
        }
    }
    if let Some(captures) = paren_suffix_regex().captures(path) {
        if let Some(full) = captures.get(0) {
            let head = &path[..full.start()];
            if !head.is_empty() && head != "/" {
                if let Some(line) = captures
                    .name("line")
                    .and_then(|matched| matched.as_str().parse::<u32>().ok())
                {
                    let column = match captures.name("column") {
                        Some(matched) => match matched.as_str().parse::<u32>().ok() {
                            Some(col) => Some(col),
                            None => return (path, None, None),
                        },
                        None => None,
                    };
                    return (head, Some(line), column);
                }
            }
        }
    }
    (path, None, None)
}

fn split_plain_location_suffix(text: &str, windows: bool) -> (&str, Option<u32>, Option<u32>) {
    if let Some((head, fragment)) = text.rsplit_once('#') {
        if !head.is_empty() {
            if let Some((line, column)) = parse_hash_location(fragment) {
                return (head, line, column);
            }
        }
    }
    split_line_suffix(text, windows)
}

// ── Path normalization and probing ────────────────────────────────────────

fn is_windows_drive_path(path: &str) -> bool {
    matches!(
        path.as_bytes(),
        [drive, b':', b'/' | b'\\', ..] if drive.is_ascii_alphabetic()
    )
}

fn has_windows_drive_prefix(path: &str) -> bool {
    matches!(
        path.as_bytes(),
        [drive, b':', ..] if drive.is_ascii_alphabetic()
    )
}

fn is_windows_drive_root(path: &str) -> bool {
    matches!(
        path.as_bytes(),
        [drive, b':'] | [drive, b':', b'/' | b'\\'] if drive.is_ascii_alphabetic()
    )
}

fn is_windows_unc_path(path: &str) -> bool {
    path.starts_with("\\\\")
}

fn strip_tilde_prefix(candidate: &str, windows: bool) -> Option<&str> {
    candidate
        .strip_prefix("~/")
        .or_else(|| windows.then(|| candidate.strip_prefix("~\\")).flatten())
}

fn join_platform_path(base: &Path, rest: &str, windows: bool) -> PathBuf {
    if !windows {
        return base.join(rest);
    }
    let base = base.to_string_lossy();
    let base = base.as_ref();
    if base.is_empty() {
        return PathBuf::from(rest);
    }
    let rest = rest.trim_start_matches(['/', '\\']);
    if base.ends_with(['/', '\\']) {
        PathBuf::from(format!("{base}{rest}"))
    } else {
        PathBuf::from(format!("{base}\\{rest}"))
    }
}

pub(super) fn candidate_paths(
    candidate: &str,
    cwds: &[&Path],
    home: Option<&Path>,
    windows: bool,
) -> Vec<PathBuf> {
    let raw = if let Some(rest) = strip_tilde_prefix(candidate, windows) {
        let Some(home) = home else {
            return Vec::new();
        };
        vec![join_platform_path(home, rest, windows)]
    } else if (windows && (is_windows_drive_path(candidate) || is_windows_unc_path(candidate)))
        || candidate.starts_with('/')
    {
        vec![PathBuf::from(candidate)]
    } else {
        cwds.iter()
            .map(|cwd| join_platform_path(cwd, candidate, windows))
            .collect()
    };
    let mut unique = Vec::with_capacity(raw.len());
    for path in raw {
        let normalized = normalize_path_for(&path, windows);
        if !unique.contains(&normalized) {
            unique.push(normalized);
        }
    }
    unique
}

fn resolve_file_candidate_verbatim(
    candidate: &str,
    cwds: &[&Path],
    budget: &mut ProbeBudget<'_>,
) -> Option<PathBuf> {
    if budget.exhausted() {
        return None;
    }
    let home = strip_tilde_prefix(candidate, cfg!(windows))
        .and_then(|_| runner_core::app_paths::home_dir());
    candidate_paths(candidate, cwds, home.as_deref(), cfg!(windows))
        .into_iter()
        .find(|path| budget.probe(path))
}

fn normalize_path_for(path: &Path, windows: bool) -> PathBuf {
    if windows {
        PathBuf::from(normalize_windows_path(&path.to_string_lossy()))
    } else {
        normalize_posix_path(path)
    }
}

fn normalize_posix_path(path: &Path) -> PathBuf {
    #[cfg(not(windows))]
    {
        let mut normalized = PathBuf::new();
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    if normalized.as_os_str().is_empty()
                        || normalized
                            .components()
                            .next_back()
                            .is_some_and(|last| last == Component::ParentDir)
                    {
                        normalized.push(Component::ParentDir);
                    } else {
                        normalized.pop();
                    }
                }
                other => normalized.push(other),
            }
        }
        normalized
    }
    #[cfg(windows)]
    {
        let raw = path.to_string_lossy();
        let (has_root, rest) = match raw.strip_prefix('/') {
            Some(rest) => (true, rest),
            None => (false, raw.as_ref()),
        };
        PathBuf::from(normalize_lexical_path(
            String::new(),
            has_root,
            rest,
            &['/'],
            '/',
        ))
    }
}

fn normalize_lexical_path(
    mut normalized: String,
    has_root: bool,
    rest: &str,
    separators: &[char],
    separator: char,
) -> String {
    let mut segments: Vec<&str> = Vec::new();
    for part in rest.split(separators) {
        match part {
            "" | "." => {}
            ".." => {
                if segments.last().is_some_and(|last| *last != "..") {
                    segments.pop();
                } else if !has_root {
                    segments.push("..");
                }
            }
            other => segments.push(other),
        }
    }
    if has_root {
        normalized.push(separator);
    }
    let mut buf = [0u8; 4];
    let sep = separator.encode_utf8(&mut buf);
    normalized.push_str(&segments.join(sep));
    normalized
}

fn split_unc_share<'a>(after_unc: &'a str, leader: &str) -> Option<(String, &'a str)> {
    let mut parts = after_unc.splitn(3, ['/', '\\']);
    let server = parts.next().filter(|s| !s.is_empty())?;
    let share = parts.next().filter(|s| !s.is_empty())?;
    let rest = parts.next().unwrap_or("");
    Some((format!("{leader}{server}\\{share}"), rest))
}

fn split_windows_prefix(path: &str) -> (String, bool, &str) {
    if let Some(after_verbatim) = path.strip_prefix("\\\\?\\") {
        if let Some(after_unc) = after_verbatim
            .get(..4)
            .filter(|prefix| prefix.eq_ignore_ascii_case("UNC\\"))
            .map(|_| &after_verbatim[4..])
        {
            if let Some((prefix, rest)) = split_unc_share(after_unc, "\\\\?\\UNC\\") {
                return (prefix, true, rest);
            }
        }
        if is_windows_drive_path(after_verbatim) {
            let prefix = format!("\\\\?\\{}", &after_verbatim[..2]);
            return (prefix, true, &after_verbatim[3..]);
        }
    }
    if let Some(after_unc) = path.strip_prefix("\\\\") {
        if let Some((prefix, rest)) = split_unc_share(after_unc, "\\\\") {
            return (prefix, true, rest);
        }
    }
    if has_windows_drive_prefix(path) {
        let has_root = matches!(path.as_bytes().get(2), Some(b'/' | b'\\'));
        let rest = if has_root { &path[3..] } else { &path[2..] };
        return (path[..2].to_owned(), has_root, rest);
    }
    if let Some(rest) = path.strip_prefix(['/', '\\']) {
        return (String::new(), true, rest);
    }
    (String::new(), false, path)
}

fn normalize_windows_path(path: &str) -> String {
    let (normalized, has_root, rest) = split_windows_prefix(path);
    normalize_lexical_path(normalized, has_root, rest, &['/', '\\'], '\\')
}

// ── File and editor-scheme URI parsing ────────────────────────────────────

fn file_target_from_uri(uri: &str) -> Option<LinkTarget> {
    let local_name = runner_core::local_host::local_hostname();
    file_target_from_uri_with_platform(uri, cfg!(windows), local_name.as_deref())
}

pub(super) fn file_target_from_uri_with_platform(
    uri: &str,
    windows: bool,
    local_name: Option<&str>,
) -> Option<LinkTarget> {
    let rest = strip_file_scheme(uri)?;
    let (rest, fragment) = rest
        .split_once('#')
        .map_or((rest, None), |(rest, fragment)| (rest, Some(fragment)));
    let (before_query, _) = rest.split_once('?').unwrap_or((rest, ""));
    let slash = before_query.find('/')?;
    let host = String::from_utf8(percent_decode_bytes(&before_query[..slash])).ok()?;
    if !runner_core::local_host::is_local_host_for(&host, local_name) {
        return None;
    }
    let decoded = String::from_utf8(percent_decode_bytes(&before_query[slash..])).ok()?;
    parse_uri_local_file_path(&decoded, fragment, windows)
}

fn file_target_from_editor_uri(uri: &str) -> Option<LinkTarget> {
    file_target_from_editor_uri_with_platform(uri, cfg!(windows))
}

pub(super) fn file_target_from_editor_uri_with_platform(
    uri: &str,
    windows: bool,
) -> Option<LinkTarget> {
    let rest = strip_editor_file_scheme(uri)?;
    let (rest, fragment) = rest
        .split_once('#')
        .map_or((rest, None), |(rest, fragment)| (rest, Some(fragment)));
    let (before_query, _) = rest.split_once('?').unwrap_or((rest, ""));
    if !before_query.starts_with('/') {
        return None;
    }
    let decoded = String::from_utf8(percent_decode_bytes(before_query)).ok()?;
    parse_uri_local_file_path(&decoded, fragment, windows)
}

fn parse_uri_local_file_path(
    decoded: &str,
    fragment: Option<&str>,
    windows: bool,
) -> Option<LinkTarget> {
    if decoded.contains('\0')
        || decoded.starts_with("//")
        || decoded.starts_with(r"/\\")
        || decoded.starts_with("\\\\")
        || (windows && decoded.starts_with(r"/\"))
    {
        return None;
    }
    let (path, line, column) = split_line_suffix(decoded, windows);
    let (line, column) = if line.is_some() {
        (line, column)
    } else if let Some((hline, hcol)) = fragment.and_then(parse_hash_location) {
        (hline, hcol)
    } else {
        (None, None)
    };
    let path = if windows {
        let stripped = strip_drive_slash(path);
        if is_windows_drive_path(stripped) {
            let normalized = normalize_windows_path(stripped);
            if is_windows_drive_root(&normalized) {
                return None;
            }
            normalized
        } else if has_windows_drive_prefix(stripped) || path == "/" || path.is_empty() {
            return None;
        } else {
            path.to_owned()
        }
    } else {
        if path == "/" || path.is_empty() {
            return None;
        }
        path.to_owned()
    };
    Some(LinkTarget::File {
        path: PathBuf::from(path),
        line,
        column,
    })
}

pub(super) fn percent_decode(text: &str) -> String {
    String::from_utf8_lossy(&percent_decode_bytes(text)).into_owned()
}

pub(super) fn percent_decode_bytes(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = (bytes[index] == b'%' && index + 2 < bytes.len())
            .then(|| std::str::from_utf8(&bytes[index + 1..index + 3]).ok())
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                decoded.push(byte);
                index += 3;
            }
            None => {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
    }
    decoded
}

fn url_regex() -> &'static Regex {
    static URL_REGEX: OnceLock<Regex> = OnceLock::new();
    URL_REGEX.get_or_init(|| {
        Regex::new(
            r#"(?i:https?|file|vscode(?:-insiders)?|cursor|windsurf|zed)://[^\s\"'!*(){}|\\\^<>`]*[^\s\"':,.!?{}|\\\^~\[\]`()<>]"#,
        )
        .expect("valid terminal URL regex")
    })
}
