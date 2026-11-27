use super::*;

/// Parses `git log -z --format=%H%x1f%h%x1f%s%x1f%an%x1f%aI%x1f%P` output.
pub(crate) fn parse_commits_z(bytes: &[u8]) -> Result<Vec<CommitSummary>> {
    let mut commits = Vec::new();
    for record in bytes.split(|&b| b == 0).filter(|rec| !rec.is_empty()) {
        let text = String::from_utf8_lossy(record);
        let parts: Vec<&str> = text.split('\x1f').collect();
        let [id, short_id, subject, author, date_str, parents] = parts.as_slice() else {
            return Err(Error::msg("malformed git log record"));
        };
        let date: DateTime<Utc> = DateTime::parse_from_rfc3339(date_str)
            .map_err(|error| Error::msg(format!("parse commit date {date_str:?}: {error}")))?
            .with_timezone(&Utc);
        commits.push(CommitSummary {
            id: (*id).to_owned(),
            short_id: (*short_id).to_owned(),
            subject: (*subject).to_owned(),
            author: (*author).to_owned(),
            date,
            parent_count: parents.split_whitespace().count(),
        });
    }
    Ok(commits)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NumstatEntry {
    pub path: String,
    pub old_path: Option<String>,
    pub counts: DiffCounts,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawDiffEntry {
    old_mode: String,
    new_mode: String,
    old_blob: String,
    new_blob: String,
    status: String,
    path: String,
    old_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RawNumstatEntry {
    pub old_mode: String,
    pub new_mode: String,
    pub old_blob: String,
    pub new_blob: String,
    pub status: String,
    pub path: String,
    pub old_path: Option<String>,
    pub counts: DiffCounts,
}

/// Parses `git diff --numstat -z` records (including `-z` rename/copy pairs, binary `- -`, and
/// paths containing spaces or newlines).
pub(crate) fn parse_numstat_z(bytes: &[u8]) -> Result<Vec<NumstatEntry>> {
    let tokens: Vec<&[u8]> = bytes.split(|&b| b == 0).collect();
    let mut cursor = 0usize;
    parse_numstat_tokens(&tokens, &mut cursor)
}

fn parse_numstat_tokens(tokens: &[&[u8]], cursor: &mut usize) -> Result<Vec<NumstatEntry>> {
    let mut out = Vec::new();
    while *cursor < tokens.len() {
        let token = tokens[*cursor];
        *cursor += 1;
        if token.is_empty() {
            continue;
        }
        let line = String::from_utf8_lossy(token);
        let mut cols = line.splitn(3, '\t');
        let (Some(added_s), Some(removed_s), Some(rest)) = (cols.next(), cols.next(), cols.next())
        else {
            return Err(Error::msg(format!("malformed numstat entry: {line:?}")));
        };
        let counts = if added_s == "-" && removed_s == "-" {
            DiffCounts::Binary
        } else {
            let added = added_s
                .parse::<u64>()
                .map_err(|error| Error::msg(format!("parse added count {added_s:?}: {error}")))?;
            let removed = removed_s.parse::<u64>().map_err(|error| {
                Error::msg(format!("parse removed count {removed_s:?}: {error}"))
            })?;
            DiffCounts::Lines { added, removed }
        };

        let (old_path, path) = if rest.is_empty() {
            let old = next_non_empty_utf8(tokens, cursor, "numstat rename old path")?;
            let new = next_non_empty_utf8(tokens, cursor, "numstat rename new path")?;
            (Some(old), new)
        } else {
            (None, rest.to_owned())
        };
        out.push(NumstatEntry {
            path,
            old_path,
            counts,
        });
    }
    Ok(out)
}

fn next_non_empty_utf8(tokens: &[&[u8]], cursor: &mut usize, what: &str) -> Result<String> {
    let token = tokens
        .get(*cursor)
        .ok_or_else(|| Error::msg(format!("unexpected end of git output for {what}")))?;
    *cursor += 1;
    Ok(String::from_utf8_lossy(token).into_owned())
}

/// Parses combined `git diff --raw --numstat -z` output into unified records.
pub(crate) fn parse_raw_and_numstat_z(bytes: &[u8]) -> Result<Vec<RawNumstatEntry>> {
    let tokens: Vec<&[u8]> = bytes.split(|&b| b == 0).collect();
    let mut cursor = 0usize;
    let mut raw_entries = Vec::new();

    while cursor < tokens.len() {
        let token = tokens[cursor];
        if token.is_empty() {
            cursor += 1;
            continue;
        }
        if !token.starts_with(b":") {
            break;
        }
        cursor += 1;
        let header = String::from_utf8_lossy(&token[1..]);
        let parts: Vec<&str> = header.split(' ').collect();
        let [old_mode, new_mode, old_blob, new_blob, status] = parts.as_slice() else {
            return Err(Error::msg(format!("malformed raw diff header: {header:?}")));
        };
        let path1 = next_non_empty_utf8(&tokens, &mut cursor, "raw diff path")?;
        let (old_path, path) = if status.starts_with('R') || status.starts_with('C') {
            let path2 = next_non_empty_utf8(&tokens, &mut cursor, "raw diff target path")?;
            (Some(path1), path2)
        } else {
            (None, path1)
        };
        raw_entries.push(RawDiffEntry {
            old_mode: (*old_mode).to_owned(),
            new_mode: (*new_mode).to_owned(),
            old_blob: (*old_blob).to_owned(),
            new_blob: (*new_blob).to_owned(),
            status: (*status).to_owned(),
            path,
            old_path,
        });
    }

    let numstat_entries = parse_numstat_tokens(&tokens, &mut cursor)?;
    let mut by_key: HashMap<(Option<String>, String), DiffCounts> =
        HashMap::with_capacity(numstat_entries.len());
    for entry in &numstat_entries {
        by_key.insert((entry.old_path.clone(), entry.path.clone()), entry.counts);
    }

    let mut out = Vec::with_capacity(raw_entries.len());
    for (idx, raw) in raw_entries.into_iter().enumerate() {
        let counts = if let Some(ns) = numstat_entries.get(idx).filter(|ns| ns.path == raw.path) {
            ns.counts
        } else if let Some(&counts) = by_key.get(&(raw.old_path.clone(), raw.path.clone())) {
            counts
        } else {
            DiffCounts::Lines {
                added: 0,
                removed: 0,
            }
        };
        out.push(RawNumstatEntry {
            old_mode: raw.old_mode,
            new_mode: raw.new_mode,
            old_blob: raw.old_blob,
            new_blob: raw.new_blob,
            status: raw.status,
            path: raw.path,
            old_path: raw.old_path,
            counts,
        });
    }
    Ok(out)
}

/// Parses `git diff -U3` output into structured hunks, handling omitted `,1` counts
/// and `\ No newline at end of file` markers cleanly.
pub(crate) fn parse_hunks(bytes: &[u8]) -> Result<Vec<DiffHunk>> {
    let trimmed = bytes.strip_suffix(b"\n").unwrap_or(bytes);
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }

    let mut hunks = Vec::new();
    let mut current: Option<(DiffHunk, u32, u32)> = None;

    for raw_line in trimmed.split(|&b| b == b'\n') {
        if raw_line.starts_with(b"@@ ") {
            if let Some((hunk, _, _)) = current.take() {
                hunks.push(hunk);
            }
            let header = String::from_utf8_lossy(raw_line)
                .trim_end_matches('\r')
                .to_owned();
            let (old_start, old_lines, new_start, new_lines) = parse_hunk_header(&header)?;
            current = Some((
                DiffHunk {
                    header,
                    old_start,
                    old_lines,
                    new_start,
                    new_lines,
                    ops: Vec::new(),
                },
                old_lines,
                new_lines,
            ));
            continue;
        }

        let Some((hunk, rem_old, rem_new)) = current.as_mut() else {
            continue;
        };

        if raw_line.starts_with(b"\\") {
            // `\ No newline at end of file` annotates the preceding line without adding ops.
            continue;
        }

        if *rem_old == 0 && *rem_new == 0 {
            let finished = current.take().expect("current is Some").0;
            hunks.push(finished);
            continue;
        }

        match raw_line.first().copied() {
            Some(b' ') => {
                *rem_old = rem_old.saturating_sub(1);
                *rem_new = rem_new.saturating_sub(1);
                push_diff_op(&mut hunk.ops, DiffOp::Context { count: 1 });
            }
            Some(b'-') => {
                *rem_old = rem_old.saturating_sub(1);
                push_diff_op(&mut hunk.ops, DiffOp::Removed { count: 1 });
            }
            Some(b'+') => {
                *rem_new = rem_new.saturating_sub(1);
                push_diff_op(&mut hunk.ops, DiffOp::Added { count: 1 });
            }
            _ => {
                let finished = current.take().expect("current is Some").0;
                hunks.push(finished);
            }
        }
    }

    if let Some((hunk, _, _)) = current.take() {
        hunks.push(hunk);
    }
    Ok(hunks)
}

fn push_diff_op(ops: &mut Vec<DiffOp>, next: DiffOp) {
    match (ops.last_mut(), next) {
        (Some(DiffOp::Context { count }), DiffOp::Context { count: add })
        | (Some(DiffOp::Removed { count }), DiffOp::Removed { count: add })
        | (Some(DiffOp::Added { count }), DiffOp::Added { count: add }) => {
            *count = count.saturating_add(add);
        }
        _ => ops.push(next),
    }
}

fn parse_hunk_header(header: &str) -> Result<(u32, u32, u32, u32)> {
    let rest = header
        .strip_prefix("@@ ")
        .and_then(|s| s.split(" @@").next())
        .ok_or_else(|| Error::msg(format!("malformed hunk header: {header:?}")))?;
    let mut parts = rest.split_whitespace();
    let (Some(old_spec), Some(new_spec)) = (parts.next(), parts.next()) else {
        return Err(Error::msg(format!("malformed hunk range: {header:?}")));
    };
    let (old_start, old_lines) = parse_hunk_range(old_spec, '-')?;
    let (new_start, new_lines) = parse_hunk_range(new_spec, '+')?;
    Ok((old_start, old_lines, new_start, new_lines))
}

fn parse_hunk_range(spec: &str, sign: char) -> Result<(u32, u32)> {
    let body = spec
        .strip_prefix(sign)
        .ok_or_else(|| Error::msg(format!("malformed hunk range token: {spec:?}")))?;
    if let Some((start, count)) = body.split_once(',') {
        let start = start
            .parse::<u32>()
            .map_err(|error| Error::msg(format!("parse hunk start {start:?}: {error}")))?;
        let count = count
            .parse::<u32>()
            .map_err(|error| Error::msg(format!("parse hunk count {count:?}: {error}")))?;
        Ok((start, count))
    } else {
        let start = body
            .parse::<u32>()
            .map_err(|error| Error::msg(format!("parse hunk start {body:?}: {error}")))?;
        Ok((start, 1))
    }
}

/// Minimal FIPS 180-4 SHA-1 hasher for git blob object hashes (`sha1("blob <len>\0" + bytes)`).
pub(super) struct Sha1 {
    state: [u32; 5],
    buf: [u8; 64],
    buf_len: usize,
    total_bytes: u64,
}

impl Sha1 {
    pub(super) fn new() -> Self {
        Self {
            state: [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0],
            buf: [0u8; 64],
            buf_len: 0,
            total_bytes: 0,
        }
    }

    pub(super) fn update(&mut self, mut data: &[u8]) {
        self.total_bytes = self.total_bytes.saturating_add(data.len() as u64);
        if self.buf_len > 0 {
            let take = data.len().min(64 - self.buf_len);
            self.buf[self.buf_len..self.buf_len + take].copy_from_slice(&data[..take]);
            self.buf_len += take;
            data = &data[take..];
            if self.buf_len == 64 {
                let block = self.buf;
                self.compress(&block);
                self.buf_len = 0;
            }
        }
        while data.len() >= 64 {
            let block: [u8; 64] = data[..64].try_into().expect("64-byte slice");
            self.compress(&block);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buf[..data.len()].copy_from_slice(data);
            self.buf_len = data.len();
        }
    }

    fn finalize(mut self) -> [u8; 20] {
        let total_bits = self.total_bytes.wrapping_mul(8);
        self.update(&[0x80]);
        while self.buf_len != 56 {
            if self.buf_len > 56 {
                self.buf[self.buf_len..64].fill(0);
                let block = self.buf;
                self.compress(&block);
                self.buf_len = 0;
            } else {
                self.buf[self.buf_len..56].fill(0);
                self.buf_len = 56;
            }
        }
        self.buf[56..64].copy_from_slice(&total_bits.to_be_bytes());
        let block = self.buf;
        self.compress(&block);

        let mut out = [0u8; 20];
        for (i, word) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    pub(super) fn hex(self) -> String {
        let digest = self.finalize();
        let mut s = String::with_capacity(40);
        for b in digest {
            use std::fmt::Write;
            let _ = write!(&mut s, "{b:02x}");
        }
        s
    }

    fn compress(&mut self, block: &[u8; 64]) {
        let mut w = [0u32; 80];
        for (i, &chunk) in block.as_chunks::<4>().0.iter().enumerate() {
            w[i] = u32::from_be_bytes(chunk);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }

        let [mut a, mut b, mut c, mut d, mut e] = self.state;
        for (i, &word) in w.iter().enumerate() {
            let (f, k) = match i {
                0..20 => ((b & c) | ((!b) & d), 0x5A827999u32),
                20..40 => (b ^ c ^ d, 0x6ED9EBA1u32),
                40..60 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDCu32),
                _ => (b ^ c ^ d, 0xCA62C1D6u32),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
    }
}
