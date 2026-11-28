//! Trial parity harness for #1508: replays Runner's recorded terminal fixtures and
//! focused VT cases into `libghostty-vt` (`vt-1.0.2`) and compares the resulting
//! screens, cursor positions, and modes against the blessed `alacritty_terminal`
//! snapshots without modifying any existing test or snapshot.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::ptr;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use ghostty_vt_sys::*;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct FixtureHeader {
    v: u32,
    cols: u16,
    rows: u16,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum FixtureEvent {
    Data {
        #[allow(dead_code)]
        ms: u64,
        data: String,
    },
    Resize {
        #[allow(dead_code)]
        ms: u64,
        cols: u16,
        rows: u16,
    },
    #[allow(dead_code)]
    Other(serde_json::Value),
}

struct Fixture {
    header: FixtureHeader,
    events: Vec<FixtureEvent>,
}

impl Fixture {
    fn load(path: &Path) -> Self {
        let raw = fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read fixture {}: {e}", path.display()));
        let mut lines = raw.lines().filter(|l| !l.trim().is_empty());
        let header_line = lines.next().expect("fixture header");
        let header: FixtureHeader = serde_json::from_str(header_line).expect("parse header");
        assert_eq!(header.v, 1, "unsupported fixture version");
        let events = lines
            .map(|l| serde_json::from_str::<FixtureEvent>(l).expect("parse event"))
            .collect();
        Self { header, events }
    }

    fn output_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for ev in &self.events {
            if let FixtureEvent::Data { data, .. } = ev {
                out.extend(B64.decode(data).expect("decode fixture base64"));
            }
        }
        out
    }
}

struct VtTerminal {
    raw: GhosttyTerminal,
}

impl VtTerminal {
    fn new(cols: u16, rows: u16) -> Self {
        let mut raw: GhosttyTerminal = ptr::null_mut();
        let res = unsafe { ghostty_terminal_new(ptr::null(), &mut raw, cols, rows) };
        assert_eq!(res, GHOSTTY_SUCCESS, "ghostty_terminal_new failed");
        assert!(!raw.is_null());

        // Phase 1 disables Kitty graphics storage (`limit = 0`) and matches Runner's 10k-line history budget.
        let gfx_limit: u64 = 0;
        let max_bytes: usize = usize::MAX;
        let max_lines: usize = 10_000;
        unsafe {
            assert_eq!(
                ghostty_terminal_set(
                    raw,
                    GHOSTTY_TERMINAL_OPT_KITTY_IMAGE_STORAGE_LIMIT,
                    (&gfx_limit as *const u64).cast()
                ),
                GHOSTTY_SUCCESS
            );
            assert_eq!(
                ghostty_terminal_set(
                    raw,
                    GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_BYTES,
                    (&max_bytes as *const usize).cast()
                ),
                GHOSTTY_SUCCESS
            );
            assert_eq!(
                ghostty_terminal_set(
                    raw,
                    GHOSTTY_TERMINAL_OPT_SCROLLBACK_MAX_LINES,
                    (&max_lines as *const usize).cast()
                ),
                GHOSTTY_SUCCESS
            );
        }

        Self { raw }
    }

    fn write(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
            unsafe { ghostty_terminal_vt_write(self.raw, bytes.as_ptr(), bytes.len()) };
        }
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        let res = unsafe { ghostty_terminal_resize(self.raw, cols, rows, 0, 0) };
        assert_eq!(res, GHOSTTY_SUCCESS, "ghostty_terminal_resize failed");
    }

    fn cols(&self) -> u16 {
        let mut out: u16 = 0;
        let res = unsafe {
            ghostty_terminal_get(
                self.raw,
                GHOSTTY_TERMINAL_DATA_COLS,
                (&mut out as *mut u16).cast(),
            )
        };
        assert_eq!(res, GHOSTTY_SUCCESS);
        out
    }

    fn rows(&self) -> u16 {
        let mut out: u16 = 0;
        let res = unsafe {
            ghostty_terminal_get(
                self.raw,
                GHOSTTY_TERMINAL_DATA_ROWS,
                (&mut out as *mut u16).cast(),
            )
        };
        assert_eq!(res, GHOSTTY_SUCCESS);
        out
    }

    fn cursor(&self) -> (u16, u16) {
        let mut x: u16 = 0;
        let mut y: u16 = 0;
        unsafe {
            assert_eq!(
                ghostty_terminal_get(
                    self.raw,
                    GHOSTTY_TERMINAL_DATA_CURSOR_X,
                    (&mut x as *mut u16).cast()
                ),
                GHOSTTY_SUCCESS
            );
            assert_eq!(
                ghostty_terminal_get(
                    self.raw,
                    GHOSTTY_TERMINAL_DATA_CURSOR_Y,
                    (&mut y as *mut u16).cast()
                ),
                GHOSTTY_SUCCESS
            );
        }
        (y, x)
    }

    fn is_alt_screen(&self) -> bool {
        let mut screen: GhosttyTerminalScreen = GHOSTTY_TERMINAL_SCREEN_PRIMARY;
        let res = unsafe {
            ghostty_terminal_get(
                self.raw,
                GHOSTTY_TERMINAL_DATA_ACTIVE_SCREEN,
                (&mut screen as *mut GhosttyTerminalScreen).cast(),
            )
        };
        assert_eq!(res, GHOSTTY_SUCCESS);
        screen == GHOSTTY_TERMINAL_SCREEN_ALTERNATE
    }

    fn mouse_tracking(&self) -> bool {
        let mut active = false;
        let res = unsafe {
            ghostty_terminal_get(
                self.raw,
                GHOSTTY_TERMINAL_DATA_MOUSE_TRACKING,
                (&mut active as *mut bool).cast(),
            )
        };
        assert_eq!(res, GHOSTTY_SUCCESS);
        active
    }

    fn mode(&self, value: u16, ansi: bool) -> bool {
        let packed = (value & 0x7FFF) | ((ansi as u16) << 15);
        let mut cfg = GhosttyTerminalModeConfig {
            mode: packed,
            value: false,
        };
        let res = unsafe {
            ghostty_terminal_get(
                self.raw,
                GHOSTTY_TERMINAL_DATA_MODE,
                (&mut cfg as *mut GhosttyTerminalModeConfig).cast(),
            )
        };
        assert_eq!(
            res, GHOSTTY_SUCCESS,
            "mode query ({value}, ansi={ansi}) failed"
        );
        cfg.value
    }

    fn scrollback_rows(&self) -> usize {
        let mut out: usize = 0;
        let res = unsafe {
            ghostty_terminal_get(
                self.raw,
                GHOSTTY_TERMINAL_DATA_SCROLLBACK_ROWS,
                (&mut out as *mut usize).cast(),
            )
        };
        assert_eq!(res, GHOSTTY_SUCCESS);
        out
    }

    fn scrollbar(&self) -> GhosttyTerminalScrollbar {
        let mut sb = GhosttyTerminalScrollbar::default();
        let res = unsafe {
            ghostty_terminal_get(
                self.raw,
                GHOSTTY_TERMINAL_DATA_SCROLLBAR,
                (&mut sb as *mut GhosttyTerminalScrollbar).cast(),
            )
        };
        assert_eq!(res, GHOSTTY_SUCCESS);
        sb
    }

    fn display_offset_from_bottom(&self) -> usize {
        let sb = self.scrollbar();
        let max_offset = sb.total.saturating_sub(sb.len);
        max_offset.saturating_sub(sb.offset) as usize
    }

    fn scroll_top(&mut self) {
        let behavior = GhosttyTerminalScrollViewport {
            tag: GHOSTTY_TERMINAL_SCROLL_VIEWPORT_TOP,
            value: GhosttyTerminalScrollViewportValue::default(),
        };
        assert_eq!(
            unsafe { ghostty_terminal_scroll_viewport(self.raw, behavior) },
            GHOSTTY_SUCCESS
        );
    }

    fn scroll_bottom(&mut self) {
        let behavior = GhosttyTerminalScrollViewport {
            tag: GHOSTTY_TERMINAL_SCROLL_VIEWPORT_BOTTOM,
            value: GhosttyTerminalScrollViewportValue::default(),
        };
        assert_eq!(
            unsafe { ghostty_terminal_scroll_viewport(self.raw, behavior) },
            GHOSTTY_SUCCESS
        );
    }

    fn grid_ref(&self, tag: GhosttyPointTag, col: u16, row: u32) -> GhosttyGridRef {
        let point = GhosttyPoint {
            tag,
            value: GhosttyPointValue {
                coordinate: GhosttyPointCoordinate { x: col, y: row },
            },
        };
        let mut gref = GhosttyGridRef::default();
        let res = unsafe { ghostty_terminal_grid_ref(self.raw, point, &mut gref) };
        assert_eq!(
            res, GHOSTTY_SUCCESS,
            "grid_ref(tag={tag}, col={col}, row={row}) failed"
        );
        gref
    }

    fn cell_info(&self, tag: GhosttyPointTag, col: u16, row: u32) -> (GhosttyCellWide, Vec<char>) {
        let gref = self.grid_ref(tag, col, row);
        let mut cell: GhosttyCell = 0;
        assert_eq!(
            unsafe { ghostty_grid_ref_cell(&gref, &mut cell) },
            GHOSTTY_SUCCESS
        );

        let mut wide: GhosttyCellWide = GHOSTTY_CELL_WIDE_NARROW;
        assert_eq!(
            unsafe {
                ghostty_cell_get(
                    cell,
                    GHOSTTY_CELL_DATA_WIDE,
                    (&mut wide as *mut GhosttyCellWide).cast(),
                )
            },
            GHOSTTY_SUCCESS
        );

        let mut buf = [0u32; 16];
        let mut written: usize = 0;
        let res =
            unsafe { ghostty_grid_ref_graphemes(&gref, buf.as_mut_ptr(), buf.len(), &mut written) };
        let codepoints: Vec<u32> = if res == GHOSTTY_OUT_OF_SPACE {
            let mut heap = vec![0u32; written];
            let mut written2: usize = 0;
            assert_eq!(
                unsafe {
                    ghostty_grid_ref_graphemes(&gref, heap.as_mut_ptr(), heap.len(), &mut written2)
                },
                GHOSTTY_SUCCESS
            );
            heap.truncate(written2);
            heap
        } else {
            assert_eq!(res, GHOSTTY_SUCCESS);
            buf[..written].to_vec()
        };

        let chars = codepoints.into_iter().filter_map(char::from_u32).collect();
        (wide, chars)
    }

    fn cell_bg_rgb(&self, tag: GhosttyPointTag, col: u16, row: u32) -> Option<(u8, u8, u8)> {
        let gref = self.grid_ref(tag, col, row);
        let mut cell: GhosttyCell = 0;
        assert_eq!(
            unsafe { ghostty_grid_ref_cell(&gref, &mut cell) },
            GHOSTTY_SUCCESS
        );
        let mut content_tag: GhosttyCellContentTag = GHOSTTY_CELL_CONTENT_CODEPOINT;
        assert_eq!(
            unsafe {
                ghostty_cell_get(
                    cell,
                    GHOSTTY_CELL_DATA_CONTENT_TAG,
                    (&mut content_tag as *mut GhosttyCellContentTag).cast(),
                )
            },
            GHOSTTY_SUCCESS
        );
        if content_tag == GHOSTTY_CELL_CONTENT_BG_COLOR_RGB {
            let mut rgb = GhosttyColorRgb::default();
            assert_eq!(
                unsafe {
                    ghostty_cell_get(
                        cell,
                        GHOSTTY_CELL_DATA_COLOR_RGB,
                        (&mut rgb as *mut GhosttyColorRgb).cast(),
                    )
                },
                GHOSTTY_SUCCESS
            );
            return Some((rgb.r, rgb.g, rgb.b));
        }
        let mut style = GhosttyStyle::default();
        assert_eq!(
            unsafe { ghostty_grid_ref_style(&gref, &mut style) },
            GHOSTTY_SUCCESS
        );
        if style.bg_color.tag == GHOSTTY_STYLE_COLOR_RGB as u8 {
            let rgb = unsafe { style.bg_color.value.rgb };
            return Some((rgb.r, rgb.g, rgb.b));
        }
        None
    }

    fn row_cells_tagged(&self, tag: GhosttyPointTag, row: u32) -> String {
        let cols = self.cols();
        let mut s = String::new();
        for col in 0..cols {
            let (wide, chars) = self.cell_info(tag, col, row);
            if wide == GHOSTTY_CELL_WIDE_SPACER_TAIL || wide == GHOSTTY_CELL_WIDE_SPACER_HEAD {
                continue;
            }
            if chars.is_empty() {
                s.push(' ');
            } else {
                for ch in chars {
                    s.push(ch);
                }
            }
        }
        s
    }

    fn row_to_string(&self, line: i32) -> String {
        let (tag, row) = if line >= 0 {
            (GHOSTTY_POINT_TAG_ACTIVE, line as u32)
        } else {
            let history = self.scrollback_rows();
            let idx = history
                .checked_sub((-line) as usize)
                .expect("history line out of bounds");
            (GHOSTTY_POINT_TAG_HISTORY, idx as u32)
        };
        let mut s = self.row_cells_tagged(tag, row);
        s.truncate(s.trim_end().len());
        s
    }

    fn visible_lines(&self) -> Vec<String> {
        (0..self.rows() as i32)
            .map(|l| self.row_to_string(l))
            .collect()
    }

    fn screen_snapshot(&self) -> String {
        let (cy, cx) = self.cursor();
        let mut out = format!(
            "cols={} rows={} cursor={},{}\n",
            self.cols(),
            self.rows(),
            cy,
            cx
        );
        for line in self.visible_lines() {
            out.push('|');
            out.push_str(&line);
            out.push('\n');
        }
        out
    }
}

impl Drop for VtTerminal {
    fn drop(&mut self) {
        unsafe { ghostty_terminal_free(self.raw) };
    }
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../runner-terminal/fixtures")
}

fn replay_fixture(fixture: &Fixture) -> VtTerminal {
    let mut term = VtTerminal::new(fixture.header.cols, fixture.header.rows);
    for event in &fixture.events {
        match event {
            FixtureEvent::Data { data, .. } => {
                let bytes = B64.decode(data).expect("decode data");
                term.write(&bytes);
            }
            FixtureEvent::Resize { cols, rows, .. } => term.resize(*cols, *rows),
            FixtureEvent::Other(_) => {}
        }
    }
    term
}

fn replay_bytes(cols: u16, rows: u16, bytes: &[u8]) -> VtTerminal {
    let mut term = VtTerminal::new(cols, rows);
    term.write(bytes);
    term
}

fn format_diff(expected: &str, actual: &str) -> String {
    let exp: Vec<&str> = expected.lines().collect();
    let act: Vec<&str> = actual.lines().collect();
    let max_len = exp.len().max(act.len());
    let mut out = String::new();
    for i in 0..max_len {
        let e = exp.get(i).copied();
        let a = act.get(i).copied();
        if e != a {
            out.push_str(&format!("  line {}:\n", i + 1));
            out.push_str(&format!("    expected: {:?}\n", e));
            out.push_str(&format!("    actual:   {:?}\n", a));
        }
    }
    out
}

#[test]
fn parity_report_all_fixtures() {
    let mut entries: Vec<_> = fs::read_dir(fixtures_dir())
        .expect("fixtures dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "ndjson"))
        .collect();
    entries.sort();
    assert!(!entries.is_empty(), "fixture corpus is empty");

    let mut identical = Vec::new();
    let mut differing: BTreeMap<String, String> = BTreeMap::new();

    for path in &entries {
        let name = path.file_name().unwrap().to_string_lossy().to_string();
        let fixture = Fixture::load(path);
        let term = replay_fixture(&fixture);
        let actual = term.screen_snapshot();
        let expected_path = path.with_extension("snapshot.txt");
        let expected = fs::read_to_string(&expected_path)
            .unwrap_or_else(|e| panic!("read {}: {e}", expected_path.display()));
        if actual == expected {
            identical.push(name);
        } else {
            differing.insert(name, format_diff(&expected, &actual));
        }
    }

    eprintln!(
        "\n=== FIXTURE PARITY SUMMARY: {}/{} identical, {} differing ===",
        identical.len(),
        entries.len(),
        differing.len()
    );
    for name in &identical {
        eprintln!("  [IDENTICAL] {name}");
    }
    for (name, diff) in &differing {
        eprintln!("  [DIFFERS]   {name}\n{diff}");
    }

    assert!(
        differing.is_empty(),
        "{} fixture(s) differed from blessed snapshots (see stderr diff above)",
        differing.len()
    );
}

#[test]
fn agy_first_turn_returns_to_primary_screen_for_idle_wheel() {
    let fixture = Fixture::load(&fixtures_dir().join("agy-first-turn.ndjson"));
    let bytes = fixture.output_bytes();
    assert!(bytes.windows(8).any(|w| w == b"\x1b[?1049h"));
    assert!(bytes.windows(8).any(|w| w == b"\x1b[?1049l"));
    let term = replay_fixture(&fixture);
    assert!(!term.is_alt_screen());
    assert!(!term.mouse_tracking());
    let screen = term.screen_snapshot();
    assert!(screen.contains("OK"));
    assert!(screen.contains("Gemini 3.8 Flash"));
}

#[test]
fn grok_first_turn_uses_alternate_screen_and_reports_mouse_at_idle() {
    let fixture = Fixture::load(&fixtures_dir().join("grok-first-turn.ndjson"));
    let term = replay_fixture(&fixture);
    assert!(term.is_alt_screen());
    assert!(term.mouse_tracking());
    assert!(term.mode(2004, false), "bracketed paste mode");
    let screen = term.screen_snapshot();
    assert!(screen.contains("GROK_FIXTURE_OK"));
    assert!(screen.contains("Grok 4.5"));
}

#[test]
fn cjk_cells_are_wide_with_spacers() {
    let term = replay_bytes(20, 4, "a中b".as_bytes());
    assert_eq!(
        term.cell_info(GHOSTTY_POINT_TAG_ACTIVE, 0, 0),
        (GHOSTTY_CELL_WIDE_NARROW, vec!['a'])
    );
    assert_eq!(
        term.cell_info(GHOSTTY_POINT_TAG_ACTIVE, 1, 0),
        (GHOSTTY_CELL_WIDE_WIDE, vec!['中'])
    );
    assert_eq!(
        term.cell_info(GHOSTTY_POINT_TAG_ACTIVE, 2, 0).0,
        GHOSTTY_CELL_WIDE_SPACER_TAIL
    );
    assert_eq!(
        term.cell_info(GHOSTTY_POINT_TAG_ACTIVE, 3, 0),
        (GHOSTTY_CELL_WIDE_NARROW, vec!['b'])
    );
    assert_eq!(term.visible_lines()[0], "a中b");
}

#[test]
fn emoji_is_wide() {
    let term = replay_bytes(20, 4, "x😀y".as_bytes());
    assert_eq!(
        term.cell_info(GHOSTTY_POINT_TAG_ACTIVE, 1, 0),
        (GHOSTTY_CELL_WIDE_WIDE, vec!['😀'])
    );
    assert_eq!(
        term.cell_info(GHOSTTY_POINT_TAG_ACTIVE, 2, 0).0,
        GHOSTTY_CELL_WIDE_SPACER_TAIL
    );
    assert_eq!(
        term.cell_info(GHOSTTY_POINT_TAG_ACTIVE, 3, 0),
        (GHOSTTY_CELL_WIDE_NARROW, vec!['y'])
    );
}

#[test]
fn combining_marks_attach_to_base_cell() {
    let term = replay_bytes(20, 4, "e\u{0301}x".as_bytes());
    assert_eq!(
        term.cell_info(GHOSTTY_POINT_TAG_ACTIVE, 0, 0),
        (GHOSTTY_CELL_WIDE_NARROW, vec!['e', '\u{0301}'])
    );
    assert_eq!(
        term.cell_info(GHOSTTY_POINT_TAG_ACTIVE, 1, 0),
        (GHOSTTY_CELL_WIDE_NARROW, vec!['x'])
    );
    assert_eq!(term.visible_lines()[0], "e\u{0301}x");
}

#[test]
fn box_drawing_is_narrow() {
    let term = replay_bytes(20, 4, "┌─┐".as_bytes());
    assert_eq!(
        term.cell_info(GHOSTTY_POINT_TAG_ACTIVE, 0, 0),
        (GHOSTTY_CELL_WIDE_NARROW, vec!['┌'])
    );
    assert_eq!(
        term.cell_info(GHOSTTY_POINT_TAG_ACTIVE, 2, 0),
        (GHOSTTY_CELL_WIDE_NARROW, vec!['┐'])
    );
}

#[test]
fn wide_char_at_line_end_wraps_with_leading_spacer() {
    let term = replay_bytes(10, 4, "123456789中".as_bytes());
    assert_eq!(
        term.cell_info(GHOSTTY_POINT_TAG_ACTIVE, 9, 0).0,
        GHOSTTY_CELL_WIDE_SPACER_HEAD
    );
    assert_eq!(
        term.cell_info(GHOSTTY_POINT_TAG_ACTIVE, 0, 1),
        (GHOSTTY_CELL_WIDE_WIDE, vec!['中'])
    );
    assert_eq!(term.visible_lines()[0], "123456789");
    assert_eq!(term.visible_lines()[1], "中");
}

#[test]
fn resize_reflows_wrapped_lines() {
    // 30 chars in a 10-col grid wrap to 3 rows; widening to 40 pulls them back
    // into one row with the cursor at (0, 30).
    //
    // Classification (real engine difference / alacritty quirk encoded by test):
    // When narrowing 40x6 -> 10x6, alacritty pins the cursor's viewport row (row 0)
    // and pushes the two re-wrapped rows above it into scrollback even though rows
    // 1..5 of the viewport are blank, leaving cursor at (0, 9). Ghostty keeps all
    // three re-wrapped rows in the active 6-row viewport (`scrollback == 0`) and
    // places the cursor at (3, 0), the column-30 continuation position.
    let text = "abcdefghijklmnopqrstuvwxyz0123";
    let mut term = replay_bytes(10, 6, text.as_bytes());
    assert_eq!(term.visible_lines()[0], "abcdefghij");
    assert_eq!(term.visible_lines()[2], "uvwxyz0123");
    assert_eq!(term.scrollback_rows(), 0);
    assert_eq!(term.cursor(), (2, 9));

    term.resize(40, 6);
    assert_eq!(term.visible_lines()[0], text);
    assert_eq!(term.cursor(), (0, 30));
    assert_eq!(term.scrollback_rows(), 0);

    term.resize(10, 6);
    assert_eq!(term.scrollback_rows(), 0);
    assert_eq!(term.visible_lines()[0], "abcdefghij");
    assert_eq!(term.visible_lines()[1], "klmnopqrst");
    assert_eq!(term.visible_lines()[2], "uvwxyz0123");
    assert_eq!(term.cursor(), (3, 0));

    // When the viewport cannot hold all re-wrapped rows (e.g. 2 rows tall),
    // Ghostty overflows the top wrapped lines into scrollback cleanly.
    term.resize(10, 2);
    assert!(term.scrollback_rows() >= 1);
    assert_eq!(
        term.row_to_string(-(term.scrollback_rows() as i32)),
        "abcdefghij"
    );
}

#[test]
fn grapheme_cluster_mode_default_matches_corpus() {
    let term = VtTerminal::new(80, 24);
    assert!(
        !term.mode(2027, false),
        "DEC mode 2027 defaults off until enabled by program or OPT_MODE_DEFAULT"
    );
}

#[test]
fn resize_reflow_preserves_wide_chars() {
    let mut term = replay_bytes(10, 6, "汉字宽度测试abcd".as_bytes());
    term.resize(6, 6);
    term.resize(20, 6);
    let joined = term.visible_lines().join("");
    assert!(
        joined.contains("汉字宽度测试abcd"),
        "wide chars corrupted by reflow round-trip: {joined:?}"
    );
}

#[test]
fn alt_screen_enter_and_leave_restores_primary() {
    let mut term = VtTerminal::new(20, 5);
    term.write(b"primary line\r\n");
    term.write(b"\x1b[?1049halt content\x1b[?1049l");
    assert_eq!(term.visible_lines()[0], "primary line");
}

#[test]
fn scrollback_survives_and_display_offset_clamps() {
    let mut term = VtTerminal::new(10, 4);
    for i in 0..20 {
        term.write(format!("line{i}\r\n").as_bytes());
    }
    let history = term.scrollback_rows();
    assert!(history > 0);
    term.scroll_top();
    assert_eq!(term.display_offset_from_bottom(), history);
    term.scroll_bottom();
    assert_eq!(term.display_offset_from_bottom(), 0);
}

#[test]
fn alt_screen_does_not_reflow_on_resize() {
    const GRAY: (u8, u8, u8) = (50, 50, 56);
    let painted_row = |width: usize| {
        let mut bytes = b"\x1b[48;2;50;50;56m".to_vec();
        bytes.extend(std::iter::repeat_n(b' ', width));
        bytes.extend_from_slice(b"\x1b[0m\r\n");
        bytes
    };
    let col0_painted = |term: &VtTerminal| -> Vec<bool> {
        (0..6)
            .map(|row| term.cell_bg_rgb(GHOSTTY_POINT_TAG_ACTIVE, 0, row) == Some(GRAY))
            .collect()
    };

    let mut narrow = VtTerminal::new(111, 10);
    narrow.write(b"\x1b[?1049h");
    for _ in 0..3 {
        narrow.write(&painted_row(112));
    }
    assert_eq!(col0_painted(&narrow), [true; 6]);
    narrow.resize(112, 10);
    assert_eq!(col0_painted(&narrow), [true; 6]);

    let mut clean = VtTerminal::new(112, 10);
    clean.write(b"\x1b[?1049h");
    for _ in 0..3 {
        clean.write(&painted_row(112));
    }
    assert_eq!(
        col0_painted(&clean),
        [true, true, true, false, false, false]
    );
}
