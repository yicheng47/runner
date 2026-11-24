#[cfg(not(windows))]
use std::path::Component;
use std::path::{Path, PathBuf};

use alacritty_terminal::grid::Dimensions as _;
use alacritty_terminal::index::{Boundary, Column, Line, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::Term;
use regex::Regex;

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

pub(super) fn strip_file_scheme(uri: &str) -> Option<&str> {
    let prefix = uri.get(..7)?;
    prefix.eq_ignore_ascii_case("file://").then(|| &uri[7..])
}

/// `/C:/Users/me` as a Windows path: a file URI's path keeps a slash before
/// the drive letter.
pub(super) fn strip_drive_slash(path: &str) -> &str {
    match path.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => &path[1..],
        _ => path,
    }
}

struct LinkCell {
    point: Point,
    start: usize,
    end: usize,
}

fn link_row_content<T>(term: &Term<T>, line: Line) -> Option<(usize, usize)> {
    let row = &term.grid()[line];
    let first = (0..term.columns()).find(|&column| !row[Column(column)].c.is_whitespace())?;
    let last = (first..term.columns()).rfind(|&column| !row[Column(column)].c.is_whitespace())?;
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

fn terminal_hyperlink_at<T>(term: &Term<T>, point: Point) -> Osc8Lookup {
    let Some(hyperlink) = term.grid()[point].hyperlink() else {
        return Osc8Lookup::None;
    };
    let is_separator = |column| {
        let cell = &term.grid()[point.line][Column(column)];
        matches!(cell.c, '│' | '|') && cell.hyperlink().is_none()
    };
    let first = (0..point.column.0)
        .rfind(|&column| is_separator(column))
        .map_or(0, |column| column + 1);
    let last = (point.column.0 + 1..term.columns())
        .find(|&column| is_separator(column))
        .unwrap_or(term.columns());
    let columns = first..last;
    let matches_link = |point: Point| {
        term.grid()[point].hyperlink().is_some_and(|other| {
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
    while start > term.topmost_line() && row_has_link(start - 1) {
        start -= 1;
    }
    let mut end = point.line;
    while end < term.bottommost_line() && row_has_link(end + 1) {
        end += 1;
    }
    let points = (start.0..=end.0)
        .flat_map(|line| {
            columns
                .clone()
                .map(move |column| Point::new(Line(line), Column(column)))
        })
        .filter(|&point| matches_link(point));
    let target = if strip_file_scheme(hyperlink.uri()).is_some() {
        let Some(target) = file_target_from_uri(hyperlink.uri()) else {
            return Osc8Lookup::Rejected;
        };
        target
    } else {
        LinkTarget::Url(hyperlink.uri().to_owned())
    };
    Osc8Lookup::Link(TerminalLink::from_points(target, points))
}

fn link_rows_joined<T>(term: &Term<T>, line: Line) -> bool {
    if term.grid()[line][Column(term.columns() - 1)]
        .flags
        .contains(Flags::WRAPLINE)
    {
        return true;
    }
    let (Some((_, last)), Some((first, _))) = (
        link_row_content(term, line),
        link_row_content(term, Line(line.0 + 1)),
    ) else {
        return false;
    };
    let next = &term.grid()[Line(line.0 + 1)];
    let prefix: String = (first..(first + 8).min(term.columns()))
        .map(|column| next[Column(column)].c)
        .collect();
    let prefix = prefix.to_ascii_lowercase();
    if prefix.starts_with("http://")
        || prefix.starts_with("https://")
        || prefix.starts_with("file://")
    {
        return false;
    }
    let left = &term.grid()[line][Column(last)];
    let right = &next[Column(first)];
    // TUIs also wrap links with explicit newlines, leaving WRAPLINE unset.
    match (left.hyperlink(), right.hyperlink()) {
        (Some(left), Some(right)) => {
            left.uri() == right.uri() && wrapped_hyperlink_ids_match(left.id(), right.id())
        }
        (None, None) => {
            if !left.flags.intersects(Flags::ALL_UNDERLINES)
                || left.flags & Flags::ALL_UNDERLINES != right.flags & Flags::ALL_UNDERLINES
                || left.fg != right.fg
                || left.bg != right.bg
            {
                return false;
            }
            let fragment_end = (first..term.columns())
                .find(|&column| {
                    let cell = &next[Column(column)];
                    !cell.flags.contains(Flags::WIDE_CHAR_SPACER)
                        && (cell.c.is_whitespace()
                            || (column > first && matches!(cell.c, '/' | '\\')))
                })
                .map_or(term.columns(), |column| {
                    column + usize::from(matches!(next[Column(column)].c, '/' | '\\'))
                });
            // Allow a two-column TUI margin, but do not join text that would have fit.
            fragment_end - first > (term.columns() - last - 1).saturating_sub(2)
        }
        _ => false,
    }
}

pub(super) fn terminal_link_at<T>(
    term: &Term<T>,
    point: Point,
    cwds: &[&Path],
) -> Option<TerminalLink> {
    terminal_link_at_with_hard_breaks(term, point, cwds, true)
}

fn terminal_link_at_with_hard_breaks<T>(
    term: &Term<T>,
    point: Point,
    cwds: &[&Path],
    hard_breaks: bool,
) -> Option<TerminalLink> {
    let mut point = point.grid_clamp(term, Boundary::Grid);
    if term.grid()[point]
        .flags
        .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
    {
        point.column = Column(point.column.0.saturating_sub(1));
    }
    match terminal_hyperlink_at(term, point) {
        Osc8Lookup::Link(link) => return Some(link),
        Osc8Lookup::Rejected => return None,
        Osc8Lookup::None => {}
    }

    let soft_start = term.line_search_left(point).line;
    let soft_end = term.line_search_right(point).line;
    let mut start = soft_start;
    let mut end = soft_end;
    if hard_breaks {
        while start > term.topmost_line() && link_rows_joined(term, start - 1) {
            start -= 1;
        }
        while end < term.bottommost_line() && link_rows_joined(term, end) {
            end += 1;
        }
    }
    let mut text = String::new();
    let mut cells = Vec::new();
    for line in start.0..=end.0 {
        let line = Line(line);
        let (first, last) = link_row_content(term, line).unwrap_or((0, term.columns() - 1));
        let hard_break_before = line > start
            && !term.grid()[Line(line.0 - 1)][Column(term.columns() - 1)]
                .flags
                .contains(Flags::WRAPLINE);
        let hard_break_after = line < end
            && !term.grid()[line][Column(term.columns() - 1)]
                .flags
                .contains(Flags::WRAPLINE);
        let first = if hard_break_before { first } else { 0 };
        let last = if hard_break_after {
            last
        } else {
            term.columns() - 1
        };
        for column in first..=last {
            let cell_point = Point::new(line, Column(column));
            let cell = &term.grid()[cell_point];
            if cell
                .flags
                .intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
            {
                continue;
            }
            let rendered_start = text.len();
            text.push(cell.c);
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

    let hit = cells.iter().position(|cell| cell.point == point)?;
    let hit_cell = &cells[hit];
    let overlaps_hit =
        |range: &std::ops::Range<usize>| range.start < hit_cell.end && range.end > hit_cell.start;
    let (range, target) = match url_regex()
        .find_iter(&text)
        .find(|matched| overlaps_hit(&matched.range()))
    {
        Some(matched) => {
            let uri = matched.as_str();
            let target = if strip_file_scheme(uri).is_some() {
                file_target_from_uri(uri)?
            } else {
                LinkTarget::Url(uri.to_owned())
            };
            (matched.range(), target)
        }
        None if hard_breaks && (start != soft_start || end != soft_end) => {
            return terminal_link_at_with_hard_breaks(term, point, cwds, false);
        }
        None => {
            let captures = file_link_regex()
                .captures_iter(&text)
                .find(|captures| overlaps_hit(&captures.get(0).map_or(0..0, |m| m.range())))?;
            (
                captures.get(0)?.range(),
                file_target_from_captures(&captures, cwds)?,
            )
        }
    };
    let first = cells.iter().position(|cell| cell.end > range.start)?;
    let last = cells.iter().rposition(|cell| cell.start < range.end)?;
    Some(TerminalLink::from_points(
        target,
        cells[first..=last].iter().map(|cell| cell.point),
    ))
}

/// A path-shaped token with an optional `:LINE[:COL]`, `(LINE,COL)`, or
/// `#LLINE` suffix. Deliberately loose: the filesystem check in
/// `resolve_file_candidate` is what separates `src/lib.rs` from `and/or`.
pub(super) fn file_link_regex() -> &'static Regex {
    static FILE_REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    FILE_REGEX.get_or_init(|| {
        Regex::new(
            r##"(?x)
            (?P<path>
                (?: ~[/\\] | (?:\.\.?[/\\])+ | [A-Za-z]:[/\\] | \\\\ | / )?
                (?: [^\s/:()\[\]{}<>'"`|\\,;=*?!\x23]+ [/\\] )*
                [^\s/:()\[\]{}<>'"`|\\,;=*?!\x23]* [^\s/:()\[\]{}<>'"`|\\,;=*?!\x23.]
            )
            (?:
                : (?P<line>\d+) (?: : (?P<column>\d+) )?
              | \( (?P<pline>\d+) , \s? (?P<pcolumn>\d+) \)
              | \x23 L (?P<hline>\d+)
            )?
            "##,
        )
        .expect("valid terminal file link regex")
    })
}

fn file_target_from_captures(captures: &regex::Captures<'_>, cwds: &[&Path]) -> Option<LinkTarget> {
    let path = resolve_file_candidate(captures.name("path")?.as_str(), cwds)?;
    let number = |name: &str| {
        captures
            .name(name)
            .and_then(|matched| matched.as_str().parse::<u32>().ok())
    };
    Some(LinkTarget::File {
        path,
        line: number("line")
            .or_else(|| number("pline"))
            .or_else(|| number("hline")),
        column: number("column").or_else(|| number("pcolumn")),
    })
}

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
    raw.into_iter()
        .map(|path| normalize_path_for(&path, windows))
        .collect()
}

fn resolve_file_candidate(candidate: &str, cwds: &[&Path]) -> Option<PathBuf> {
    let home = strip_tilde_prefix(candidate, cfg!(windows))
        .and_then(|_| runner_core::app_paths::home_dir());
    candidate_paths(candidate, cwds, home.as_deref(), cfg!(windows))
        .into_iter()
        .find(|path| std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file()))
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
    if decoded.contains('\0')
        || decoded.starts_with("//")
        || decoded.starts_with(r"/\\")
        || decoded.starts_with("\\\\")
        || (windows && decoded.starts_with(r"/\"))
    {
        return None;
    }
    let (path, line, column) = split_line_suffix(&decoded);
    let line = line.or_else(|| {
        fragment?
            .strip_prefix('L')?
            .split(|c: char| !c.is_ascii_digit())
            .next()?
            .parse()
            .ok()
    });
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

fn split_line_suffix(path: &str) -> (&str, Option<u32>, Option<u32>) {
    let mut path = path;
    let mut numbers = Vec::new();
    while numbers.len() < 2 {
        let Some((head, tail)) = path.rsplit_once(':') else {
            break;
        };
        let Ok(number) = tail.parse::<u32>() else {
            break;
        };
        numbers.push(number);
        path = head;
    }
    match numbers.as_slice() {
        [line] => (path, Some(*line), None),
        [column, line] => (path, Some(*line), Some(*column)),
        _ => (path, None, None),
    }
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
    static URL_REGEX: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    URL_REGEX.get_or_init(|| {
        Regex::new(r#"(?i:https?|file)://[^\s\"'!*(){}|\\\^<>`]*[^\s\"':,.!?{}|\\\^~\[\]`()<>]"#)
            .expect("valid terminal URL regex")
    })
}
