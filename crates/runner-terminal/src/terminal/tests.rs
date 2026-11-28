use super::*;
use crate::types::{CellFlags, Column, GridPoint as Point, Line};

fn link_term(columns: usize, bytes: &[u8]) -> crate::replay::Terminal {
    link_term_sized(columns, 8, bytes)
}

fn terminal_link_at(
    term: &crate::replay::Terminal,
    point: Point,
    cwds: &[&Path],
) -> Option<TerminalLink> {
    super::terminal_link_at(&term.screen(), point, cwds)
}

fn terminal_link_at_with_prober(
    term: &crate::replay::Terminal,
    point: Point,
    cwds: &[&Path],
    is_file: &mut dyn FnMut(&Path) -> bool,
) -> Option<TerminalLink> {
    super::terminal_link_at_with_prober(&term.screen(), point, cwds, is_file)
}

#[test]
fn terminal_links_follow_fragmented_osc_8_in_table_cells() {
    let uri = "https://www.shopneighbour.com/en-us/collections/all-exclusions-cant-discount/products/our-legacy-mens-third-cut-blue-selvage";
    let rows = [
        vec!["Blue Selvedge at Neighbour", " (", "https://"],
        vec!["www.shopneighbour.com/en", "-us/"],
        vec!["collections/all-exclusions-cant-"],
        vec!["discount/products/our-legacy-mens-"],
        vec!["third-cut-blue-selvage", ")"],
    ];
    let mut bytes = String::new();
    let mut linked_points = Vec::new();
    let mut unlinked_points = Vec::new();
    for (line, fragments) in rows.iter().enumerate() {
        if line > 0 {
            bytes.push_str("\r\n");
        }
        bytes.push_str("  ");
        let mut column = 2;
        for fragment in fragments {
            let linked = !matches!(*fragment, " (" | ")");
            if linked {
                bytes.push_str(&format!("\x1b]8;;{uri}\x1b\\\x1b[4;34m"));
            }
            bytes.push_str(fragment);
            for offset in 0..fragment.len() {
                let point = Point::new(Line(line as i32), Column(column + offset));
                if linked {
                    linked_points.push(point);
                } else {
                    unlinked_points.push(point);
                }
            }
            if linked {
                bytes.push_str("\x1b[0m\x1b]8;;\x1b\\");
            }
            column += fragment.len();
        }
        bytes.push_str(&" ".repeat(38 - column));
        bytes.push_str("│ ");
        bytes.push_str(&format!("\x1b]8;;{uri}\x1b\\other cell\x1b]8;;\x1b\\"));
        unlinked_points.push(Point::new(Line(line as i32), Column(38)));
        unlinked_points.push(Point::new(Line(line as i32), Column(40)));
        unlinked_points.push(Point::new(Line(line as i32), Column(0)));
    }
    let term = link_term(72, bytes.as_bytes());
    let expected = terminal_link_at(&term, linked_points[0], &[]).unwrap();
    assert_eq!(expected.target, LinkTarget::Url(uri.into()));
    assert_eq!(expected.start, linked_points[0]);
    assert_eq!(expected.end, *linked_points.last().unwrap());
    for point in linked_points {
        assert!(expected.contains(point), "missing linked cell {point:?}");
        assert_eq!(terminal_link_at(&term, point, &[]), Some(expected.clone()));
    }
    for point in unlinked_points {
        assert!(
            !expected.contains(point),
            "included unlinked cell {point:?}"
        );
    }
    let restored = link_term(72, &crate::snapshot::serialize(&term, &[]));
    assert_eq!(
        terminal_link_at(&restored, Point::new(Line(1), Column(4)), &[]),
        Some(expected)
    );
}

#[test]
fn terminal_links_keep_unrelated_osc_8_rows_separate() {
    for separator in [
        "",
        "────────────────────",
        "\x1b]8;;https://other.example\x1b\\other\x1b]8;;\x1b\\",
    ] {
        let bytes = format!(
                "\x1b]8;;https://example.com\x1b\\first\x1b]8;;\x1b\\\r\n  \x1b]8;;https://example.com\x1b\\continued\x1b]8;;\x1b\\\r\n{separator}\r\n\x1b]8;;https://example.com\x1b\\separate\x1b]8;;\x1b\\"
            );
        let term = link_term(40, bytes.as_bytes());
        let first = terminal_link_at(&term, Point::new(Line(0), Column(1)), &[]).unwrap();
        let separate = terminal_link_at(&term, Point::new(Line(3), Column(1)), &[]).unwrap();
        assert_eq!(first.end, Point::new(Line(1), Column(10)));
        assert_eq!(separate.start, Point::new(Line(3), Column(0)));
        assert!(!first.contains(separate.start));
        assert!(!separate.contains(first.start));
    }
}

#[test]
fn terminal_links_follow_indented_underlined_url_rows() {
    let term = link_term(
        30,
        b"  (\x1b[4;34mhttps://example.com/repo/\x1b[0m\r\n  \x1b[4;34mcommit/abc123\x1b[0m).",
    );
    let first = terminal_link_at(&term, Point::new(Line(0), Column(5)), &[]).unwrap();
    let continuation =
        terminal_link_at(&term, Point::new(Line(1), Column(3)), &[]).expect("URL continuation");
    assert_eq!(first, continuation);
    assert_eq!(
        first.target,
        LinkTarget::Url("https://example.com/repo/commit/abc123".into())
    );
    assert_eq!(first.start, Point::new(Line(0), Column(3)));
    assert_eq!(first.end, Point::new(Line(1), Column(14)));
    assert!(first.contains(Point::new(Line(0), Column(27))));
    assert!(first.contains(Point::new(Line(1), Column(2))));
    assert!(!first.contains(Point::new(Line(0), Column(28))));
    assert!(!first.contains(Point::new(Line(1), Column(1))));
    assert!(!first.contains(Point::new(Line(1), Column(15))));

    let restored = link_term(30, &crate::snapshot::serialize(&term, &[]));
    assert_eq!(
        terminal_link_at(&restored, Point::new(Line(1), Column(3)), &[]),
        Some(first)
    );
}

#[test]
fn terminal_links_follow_osc_8_across_indented_rows() {
    let term = link_term(
            40,
            b"\x1b]8;;https://example.com/target\x1b\\first row\x1b]8;;\x1b\\\r\n  \x1b]8;;https://example.com/target\x1b\\second row\x1b]8;;\x1b\\",
        );
    let first = terminal_link_at(&term, Point::new(Line(0), Column(1)), &[]).unwrap();
    let second = terminal_link_at(&term, Point::new(Line(1), Column(3)), &[]).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.end, Point::new(Line(1), Column(11)));
    assert!(!first.contains(Point::new(Line(1), Column(0))));
    assert!(!first.contains(Point::new(Line(0), Column(9))));
}

#[test]
fn terminal_links_keep_unrelated_hard_lines_separate() {
    for bytes in [
        b"https://example.com/path\r\n  continuation".as_slice(),
        b"\x1b[4;34mhttps://example.com/path\x1b[0m\r\n  continuation",
        b"\x1b[4;34mhttps://example.com/path\r\n  https://other.example/path",
        b"\x1b[4;34mhttps://example.com/path\r\n  \x1b[31mcontinuation",
        b"\x1b[4;34mhttps://example.com/path\r\n\r\n  continuation",
        b"\x1b[4;34mhttps://example.com/path\r\n  www.b.example/two",
        b"\x1b[4;34mhttps://example.com/path\r\n  docs/guide.md",
        b"\x1b[4;34mSee https://example.com/path\r\n  and more text",
    ] {
        let term = link_term(40, bytes);
        let first = terminal_link_at(&term, Point::new(Line(0), Column(4)), &[]).unwrap();
        assert_eq!(
            first.target,
            LinkTarget::Url("https://example.com/path".into())
        );
        assert_eq!(first.end.line, Line(0));
    }
}

#[test]
fn terminal_links_follow_three_rows_and_preserve_soft_wraps() {
    let term = link_term(
            30,
            b"  (\x1b[4;34mhttps://example.com/repo/\x1b[0m\r\n  \x1b[4;34mcommit/abcdef0123456789/\x1b[0m\r\n  \x1b[4;34mabc123\x1b[0m.",
        );
    let first = terminal_link_at(&term, Point::new(Line(0), Column(4)), &[]).unwrap();
    for line in 1..=2 {
        assert_eq!(
            terminal_link_at(&term, Point::new(Line(line), Column(3)), &[]),
            Some(first.clone())
        );
        assert!(!first.contains(Point::new(Line(line), Column(0))));
    }
    assert_eq!(
        first.target,
        LinkTarget::Url("https://example.com/repo/commit/abcdef0123456789/abc123".into())
    );

    let term = link_term(14, b"xxhttps://example.com/path");
    let first = terminal_link_at(&term, Point::new(Line(0), Column(3)), &[]).unwrap();
    assert_eq!(
        terminal_link_at(&term, Point::new(Line(1), Column(3)), &[]),
        Some(first.clone())
    );
    assert_eq!(
        first.target,
        LinkTarget::Url("https://example.com/path".into())
    );
    assert_eq!(first.start, Point::new(Line(0), Column(2)));
    assert_eq!(first.end, Point::new(Line(1), Column(11)));
}

#[test]
fn terminal_links_keep_underlined_file_rows_separate() {
    let cwd = tempfile::tempdir().unwrap();
    std::fs::create_dir(cwd.path().join("src")).unwrap();
    for names in [
        ["a", "b", "c"],
        [
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "ccccccccccccccccccccccccccccc",
        ],
    ] {
        let mut bytes = String::from("\x1b[4;34m");
        for (line, name) in names.iter().enumerate() {
            std::fs::write(cwd.path().join(format!("src/{name}.rs")), "").unwrap();
            if line > 0 {
                bytes.push_str("\r\n");
            }
            bytes.push_str(&format!("src/{name}.rs"));
        }
        let term = link_term(40, bytes.as_bytes());
        for (line, name) in names.into_iter().enumerate() {
            let link = terminal_link_at(
                &term,
                Point::new(Line(line as i32), Column(2)),
                &[cwd.path()],
            )
            .expect("independent file link");
            assert_eq!(
                link.target,
                LinkTarget::File {
                    path: cwd.path().join(format!("src/{name}.rs")),
                    line: None,
                    column: None,
                }
            );
            assert_eq!(link.start.line, link.end.line);
        }
    }
}

#[test]
fn terminal_links_do_not_join_an_underlined_scrollback_block() {
    let mut bytes = String::from("\x1b[4;34m");
    for line in 0..3000 {
        if line > 0 {
            bytes.push_str("\r\n");
        }
        bytes.push_str(&format!("row https://example.com/{line}"));
    }
    let term = link_term(40, bytes.as_bytes());
    let point = Point::new(term.screen().topmost_line(), Column(5));
    let link = terminal_link_at(&term, point, &[]).unwrap();
    assert_eq!(link.target, LinkTarget::Url("https://example.com/0".into()));
    assert_eq!(link.start.line, point.line);
    assert_eq!(link.end.line, point.line);
}

#[test]
fn terminal_links_keep_distinct_explicit_hyperlink_ids_separate() {
    let term = link_term(
            40,
            b"\x1b]8;id=A;https://example.com\x1b\\first\x1b]8;id=B;https://example.com\x1b\\second\x1b]8;;\x1b\\\r\n\x1b]8;id=C;https://example.com\x1b\\third\x1b]8;;\x1b\\",
        );
    for (point, start, end) in [
        (Point::new(Line(0), Column(1)), 0, 4),
        (Point::new(Line(0), Column(6)), 5, 10),
        (Point::new(Line(1), Column(1)), 0, 4),
    ] {
        let link = terminal_link_at(&term, point, &[]).unwrap();
        assert_eq!(link.start, Point::new(point.line, Column(start)));
        assert_eq!(link.end, Point::new(point.line, Column(end)));
    }
}

#[test]
fn reconnect_cancels_an_idle_subscription_replaced_by_resync() {
    use runner_core::protocol::terminal::{TerminalAttachment, TerminalSubscription};
    use runner_core::protocol::{ClientError, EventSubscription, Request, Response, Transport};
    use std::time::Duration;
    type FrameResult = std::result::Result<TerminalFrame, ClientError>;
    struct Frames {
        rx: mpsc::Receiver<FrameResult>,
        tx: mpsc::Sender<FrameResult>,
        id: u64,
        ready: mpsc::Sender<u64>,
    }
    impl TerminalSubscription for Frames {
        fn recv(&mut self) -> std::result::Result<TerminalFrame, ClientError> {
            self.rx.recv().unwrap()
        }
        fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
            self.ready.send(self.id).unwrap();
            let tx = self.tx.clone();
            Arc::new(move || {
                let _ = tx.send(Err(ClientError::msg("cancelled")));
            })
        }
    }
    struct Mock {
        senders: Mutex<Vec<mpsc::Sender<FrameResult>>>,
        ready: mpsc::Sender<u64>,
    }
    impl Transport for Mock {
        fn call(&self, _: Request) -> std::result::Result<Response, ClientError> {
            Ok(Response::terminal_metadata(Ok(TerminalMetadata::default())))
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            unreachable!()
        }
        fn attach(&self, _: &str) -> std::result::Result<TerminalAttachment, ClientError> {
            let (tx, rx) = mpsc::channel();
            let mut senders = self.senders.lock().unwrap();
            let id = senders.len() as u64;
            senders.push(tx.clone());
            Ok(TerminalAttachment {
                subscriber_id: id,
                snapshot: runner_core::protocol::terminal::TerminalSnapshot {
                    seq: 0,
                    format: Default::default(),
                    unavailable: false,
                    cols: 80,
                    rows: 24,
                    bytes: Vec::new(),
                    unfinished_len: 0,
                    preceding_char: None,
                    activity: None,
                    controller: Default::default(),
                },
                frames: Box::new(Frames {
                    rx,
                    tx,
                    id,
                    ready: self.ready.clone(),
                }),
            })
        }
    }
    let (ready, installed) = mpsc::channel();
    let host = Arc::new(Mock {
        senders: Mutex::default(),
        ready,
    });
    let mirror = TerminalMirror::attach(
        DaemonClient::new(host.clone()),
        "idle".into(),
        Arc::new(|| {}),
    )
    .unwrap();
    assert_eq!(installed.recv_timeout(Duration::from_secs(2)).unwrap(), 0);
    host.senders.lock().unwrap()[0]
        .send(Ok(TerminalFrame::Resync))
        .unwrap();
    assert_eq!(installed.recv_timeout(Duration::from_secs(2)).unwrap(), 1);
    let next = mirror.clone();
    let (done, result) = mpsc::channel();
    let worker = thread::spawn(move || {
        done.send(next.reattach()).unwrap();
    });
    result
        .recv_timeout(Duration::from_secs(2))
        .expect("reattach must wake an idle Resync subscription")
        .unwrap();
    worker.join().unwrap();
    let frames = mirror.frame_worker.lock().unwrap().take().unwrap();
    {
        let state = frames.state.lock().unwrap();
        state.stopped.store(true, Ordering::Release);
        (state.cancel)();
    }
    frames.thread.join().unwrap();
}

#[test]
fn burst_hold_passes_echo_and_holds_clears_and_volume_until_quiet_or_cap() {
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    let mut hold = BurstHold::default();
    assert_eq!(hold.feed(b"a", at(0), false), Burst::Pass);
    assert_eq!(hold.deadline(), None);

    assert_eq!(hold.feed(b"\x1b[2J\x1b[3J", at(10), false), Burst::Held);
    assert_eq!(hold.feed(b"history", at(50), false), Burst::Held);
    assert_eq!(hold.deadline(), Some(at(130)));
    assert_eq!(hold.take(), b"\x1b[2J\x1b[3Jhistory");
    assert_eq!(hold.deadline(), None);

    let streaming = vec![b'x'; 16 * 1024];
    assert_eq!(hold.feed(&streaming, at(200), false), Burst::Pass);
    assert_eq!(
        hold.feed(&streaming[..15 * 1024], at(250), false),
        Burst::Pass
    );
    assert_eq!(
        hold.feed(&streaming[..2 * 1024], at(280), false),
        Burst::Held
    );
    assert_eq!(hold.deadline(), Some(at(360)));
    assert_eq!(hold.take().len(), 2 * 1024);
    assert_eq!(hold.feed(&streaming, at(1000), false), Burst::Pass);
    assert_eq!(
        hold.feed(&streaming, at(1150), false),
        Burst::Pass,
        "a new window starts after a quiet gap"
    );

    assert_eq!(hold.feed(b"\x1b[3J", at(2000), false), Burst::Held);
    for ms in (2060..2500).step_by(60) {
        assert_eq!(hold.feed(b"more", at(ms), false), Burst::Held);
    }
    assert_eq!(hold.deadline(), Some(at(2500)));
    let Burst::Release(released) = hold.feed(b"last", at(2500), false) else {
        panic!("a continuous burst is released at the cap");
    };
    assert!(released.starts_with(b"\x1b[3J") && released.ends_with(b"last"));
    assert_eq!(hold.deadline(), None);

    assert!(matches!(
        hold.feed(&vec![b'x'; BURST_MAX_HELD], at(3000), false),
        Burst::Release(bytes) if bytes.len() == BURST_MAX_HELD
    ));
}

fn compositions(marker: &[u8]) -> Vec<Vec<&[u8]>> {
    (0..1u32 << (marker.len() - 1))
        .map(|cuts| {
            let mut parts = Vec::new();
            let mut start = 0;
            for end in 1..=marker.len() {
                if end == marker.len() || cuts & (1 << (end - 1)) != 0 {
                    parts.push(&marker[start..end]);
                    start = end;
                }
            }
            parts
        })
        .collect()
}

#[test]
fn burst_hold_finds_a_clear_split_anywhere_across_chunks() {
    let now = Instant::now();
    for parts in compositions(CLEAR_SCROLLBACK) {
        let mut hold = BurstHold::default();
        assert_eq!(hold.feed(b"before\x1b[H", now, false), Burst::Pass);
        let (last, leading) = parts.split_last().unwrap();
        for part in leading {
            assert_eq!(hold.feed(part, now, false), Burst::Pass, "{parts:?}");
        }
        let mut completing = last.to_vec();
        completing.extend_from_slice(b"short repaint");
        assert_eq!(hold.feed(&completing, now, false), Burst::Held, "{parts:?}");
        assert_eq!(hold.feed(b"more repaint", now, false), Burst::Held);
        hold.take();
        assert_eq!(
            hold.feed(b"echo", now, false),
            Burst::Pass,
            "a marker already seen is not found again"
        );
    }
}

#[test]
fn burst_hold_yields_to_an_explicit_synchronized_update() {
    let now = Instant::now();
    let volume = vec![b'x'; 2 * BURST_BYTES];
    let mut hold = BurstHold::default();
    assert_eq!(hold.feed(&volume, now, true), Burst::Pass);
    assert_eq!(hold.feed(CLEAR_SCROLLBACK, now, true), Burst::Pass);
    assert_eq!(hold.feed(b"echo", now, false), Burst::Pass);

    for parts in compositions(BEGIN_SYNC) {
        let mut hold = BurstHold::default();
        assert_eq!(hold.feed(CLEAR_SCROLLBACK, now, false), Burst::Held);
        let (last, leading) = parts.split_last().unwrap();
        for part in leading {
            assert_eq!(hold.feed(part, now, false), Burst::Held, "{parts:?}");
        }
        let mut expected = CLEAR_SCROLLBACK.to_vec();
        expected.extend_from_slice(BEGIN_SYNC);
        assert_eq!(
            hold.feed(last, now, false),
            Burst::Release(expected),
            "{parts:?}"
        );
        assert_eq!(hold.deadline(), None);
    }

    let mut hold = BurstHold::default();
    let mut opening = BEGIN_SYNC.to_vec();
    opening.extend_from_slice(CLEAR_SCROLLBACK);
    opening.extend_from_slice(&volume);
    assert_eq!(hold.feed(&opening, now, false), Burst::Pass);
}

type ScriptedFrame = std::result::Result<TerminalFrame, ClientError>;

struct ScriptedFrames(mpsc::Receiver<ScriptedFrame>, mpsc::Sender<ScriptedFrame>);

impl runner_core::protocol::terminal::TerminalSubscription for ScriptedFrames {
    fn recv(&mut self) -> ScriptedFrame {
        self.0
            .recv()
            .unwrap_or_else(|_| Err(ClientError::msg("closed")))
    }
    fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
        let tx = self.1.clone();
        Arc::new(move || {
            let _ = tx.send(Err(ClientError::msg("cancelled")));
        })
    }
}

struct ScriptedHost(Mutex<Option<ScriptedFrames>>);

impl runner_core::protocol::Transport for ScriptedHost {
    fn call(
        &self,
        _: runner_core::protocol::Request,
    ) -> std::result::Result<runner_core::protocol::Response, ClientError> {
        Ok(runner_core::protocol::Response::terminal_metadata(Ok(
            TerminalMetadata::default(),
        )))
    }
    fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
        unreachable!()
    }
    fn attach(&self, _: &str) -> std::result::Result<TerminalAttachment, ClientError> {
        Ok(TerminalAttachment {
            subscriber_id: 1,
            snapshot: TerminalSnapshot {
                seq: 0,
                format: Default::default(),
                unavailable: false,
                cols: 80,
                rows: 24,
                bytes: Vec::new(),
                unfinished_len: 0,
                preceding_char: None,
                activity: None,
                controller: Default::default(),
            },
            frames: Box::new(self.0.lock().unwrap().take().expect("one attachment")),
        })
    }
}

fn scripted_mirror(
    remote: bool,
) -> (
    Arc<TerminalMirror>,
    mpsc::Sender<ScriptedFrame>,
    Arc<AtomicUsize>,
) {
    let (tx, rx) = mpsc::channel();
    let host = Arc::new(ScriptedHost(Mutex::new(Some(ScriptedFrames(
        rx,
        tx.clone(),
    )))));
    let wakes = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&wakes);
    let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        counter.fetch_add(1, Ordering::SeqCst);
    });
    let client = DaemonClient::new(host);
    let mirror = if remote {
        TerminalMirror::attach_remote(client, "burst".into(), waker)
    } else {
        TerminalMirror::attach(client, "burst".into(), waker)
    }
    .unwrap();
    (mirror, tx, wakes)
}

fn screen_text(mirror: &TerminalMirror) -> String {
    mirror
        .viewport_snapshot()
        .view()
        .visible_cells()
        .map(|cell| cell.c())
        .collect()
}

fn wait_for(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        thread::sleep(Duration::from_millis(5));
    }
    done()
}

fn send_output(frames: &mpsc::Sender<ScriptedFrame>, seq: u64, bytes: &[u8]) {
    frames
        .send(Ok(TerminalFrame::Output {
            seq,
            bytes: bytes.to_vec(),
        }))
        .unwrap();
}

#[test]
fn remote_mirror_paints_echo_at_once_and_a_full_repaint_in_one_wake() {
    let (mirror, frames, wakes) = scripted_mirror(true);
    let _view = mirror.view();
    send_output(&frames, 1, b"typed");
    assert!(wait_for(Duration::from_secs(2), || mirror
        .output_activity()
        .last_seq
        == 1));
    assert!(screen_text(&mirror).contains("typed"));
    assert!(wait_for(Duration::from_secs(2), || wakes
        .load(Ordering::SeqCst)
        == 1));

    let mut repaint = b"\x1b[H\x1b[2J\x1b[3J".to_vec();
    for line in 0..3000 {
        repaint.extend_from_slice(format!("history line {line}\r\n").as_bytes());
    }
    repaint.extend_from_slice(b"repaint done");
    let chunks = repaint.chunks(8 * 1024).collect::<Vec<_>>();
    assert!(chunks.len() > 4);
    for (index, chunk) in chunks.iter().enumerate() {
        send_output(&frames, index as u64 + 2, chunk);
    }
    assert!(wait_for(Duration::from_secs(2), || screen_text(&mirror)
        .contains("repaint done")));
    assert!(!screen_text(&mirror).contains("typed"));
    thread::sleep(BURST_QUIET * 2);
    assert_eq!(
        wakes.load(Ordering::SeqCst),
        2,
        "every held chunk is painted by a single release"
    );
    assert_eq!(mirror.sync_state(), (None, 0, false));
}

fn filler(bytes: usize) -> Vec<u8> {
    let mut filler = Vec::new();
    while filler.len() < bytes {
        filler.extend_from_slice(b"\x1b[1;1H");
        filler.extend_from_slice(&[b'f'; 60]);
    }
    filler
}

#[test]
fn remote_mirror_keeps_explicit_synchronized_updates_with_vte() {
    let (mirror, frames, _) = scripted_mirror(true);
    let parsed = |seq: u64| {
        assert!(wait_for(Duration::from_secs(2), || mirror
            .output_activity()
            .last_seq
            == seq));
        screen_text(&mirror)
    };

    send_output(&frames, 1, b"\x1b[?2026h\x1b[3;1Hprefix-a");
    send_output(&frames, 2, &filler(2 * BURST_BYTES));
    send_output(&frames, 3, b"\x1b[4;1Hafter-a\x1b[?2026l");
    let screen = parsed(3);
    assert!(
        screen.contains("prefix-a") && screen.contains("after-a"),
        "an update that ends after the volume threshold paints at its end marker"
    );

    let mut whole = b"\x1b[?2026h\x1b[3J".to_vec();
    whole.extend_from_slice(&filler(2 * BURST_BYTES));
    whole.extend_from_slice(b"\x1b[5;1Hwhole-b\x1b[?2026l");
    send_output(&frames, 4, &whole);
    assert!(parsed(4).contains("whole-b"));
    assert_eq!(mirror.sync_state().0, None);

    send_output(&frames, 5, b"\x1b[?2026h\x1b[6;1Hpartial-c");
    let mut unfinished = filler(2 * BURST_BYTES);
    unfinished.extend_from_slice(b"\x1b[7;1Htail-c");
    send_output(&frames, 6, &unfinished);
    assert!(wait_for(Duration::from_secs(2), || screen_text(&mirror)
        .contains("partial-c")));
    assert!(
        screen_text(&mirror).contains("tail-c"),
        "vte's own timeout flushes everything that arrived inside the update"
    );
    assert!(wait_for(Duration::from_secs(2), || mirror.sync_state()
        == (None, 0, false)));
}

#[test]
fn remote_mirror_releases_a_continuous_burst_at_the_cap() {
    let (mirror, frames, _) = scripted_mirror(true);
    send_output(&frames, 1, b"\x1b[3Jtick 1");
    let started = Instant::now();
    let mut seq = 1;
    while !screen_text(&mirror).contains("tick") {
        assert!(
            started.elapsed() < BURST_CAP * 3,
            "a burst that never goes quiet is still painted"
        );
        thread::sleep(BURST_QUIET / 2);
        seq += 1;
        send_output(&frames, seq, format!("\r\ntick {seq}").as_bytes());
    }
}

#[test]
fn a_released_burst_advances_the_scroll_mark_and_a_hold_keeps_the_viewport_still() {
    let (mirror, frames, _) = scripted_mirror(true);
    let _view = mirror.view();
    let lines = |range: std::ops::Range<usize>| {
        range
            .map(|line| format!("line {line:05} of the output\r\n"))
            .collect::<String>()
            .into_bytes()
    };
    send_output(&frames, 1, &lines(0..30));
    assert!(wait_for(Duration::from_secs(2), || mirror
        .output_activity()
        .last_seq
        == 1));
    let mark = mirror.scroll_mark();
    let hold = mirror.hold_viewport();
    let burst = lines(30..6030);
    let chunks = burst.chunks(8 * 1024).collect::<Vec<_>>();
    assert!(burst.len() > 2 * BURST_BYTES);
    for (index, chunk) in chunks.iter().enumerate() {
        send_output(&frames, index as u64 + 2, chunk);
    }
    assert!(wait_for(Duration::from_secs(2), || mark
        .moved_since(mirror.scroll_mark())
        == Some(6000)));
    assert_eq!(mirror.scroll_state().display_offset, 6000);
    drop(hold);
}

#[test]
fn local_mirror_parses_a_clear_scrollback_at_once() {
    let (mirror, frames, _) = scripted_mirror(false);
    send_output(&frames, 1, b"\x1b[3Jlocal output");
    assert!(wait_for(Duration::from_secs(2), || mirror
        .output_activity()
        .last_seq
        == 1));
    assert!(screen_text(&mirror).contains("local output"));
    let (mirror, frames, _) = scripted_mirror(false);
    let volume = vec![b'x'; 4 * BURST_BYTES];
    send_output(&frames, 1, &volume);
    assert!(wait_for(Duration::from_secs(2), || mirror
        .output_activity()
        .last_seq
        == 1));
    assert!(screen_text(&mirror).contains('x'));
}

#[test]
fn output_advances_the_scroll_mark_and_a_hold_keeps_the_viewport_still() {
    use runner_core::protocol::terminal::{TerminalAttachment, TerminalSubscription};
    use runner_core::protocol::{EventSubscription, Request, Response, Transport};
    struct Frames {
        tx: mpsc::Sender<TerminalFrame>,
        rx: mpsc::Receiver<TerminalFrame>,
        cancelled: Arc<AtomicBool>,
    }
    impl TerminalSubscription for Frames {
        fn recv(&mut self) -> std::result::Result<TerminalFrame, ClientError> {
            let frame = self.rx.recv().map_err(|_| ClientError::msg("closed"))?;
            if self.cancelled.load(Ordering::Acquire) {
                return Err(ClientError::msg("cancelled"));
            }
            Ok(frame)
        }
        fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
            let tx = self.tx.clone();
            let cancelled = Arc::clone(&self.cancelled);
            Arc::new(move || {
                cancelled.store(true, Ordering::Release);
                let _ = tx.send(TerminalFrame::Resync);
            })
        }
    }
    struct Host(Mutex<Option<Frames>>);
    impl Transport for Host {
        fn call(&self, _: Request) -> std::result::Result<Response, ClientError> {
            Ok(Response::terminal_metadata(Ok(TerminalMetadata::default())))
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            unreachable!()
        }
        fn attach(&self, _: &str) -> std::result::Result<TerminalAttachment, ClientError> {
            let frames = self.0.lock().unwrap().take().unwrap();
            Ok(TerminalAttachment {
                subscriber_id: 0,
                snapshot: TerminalSnapshot {
                    seq: 0,
                    format: Default::default(),
                    unavailable: false,
                    cols: 20,
                    rows: 5,
                    bytes: Vec::new(),
                    unfinished_len: 0,
                    preceding_char: None,
                    activity: None,
                    controller: Default::default(),
                },
                frames: Box::new(frames),
            })
        }
    }
    let (frames, rx) = mpsc::channel();
    let wakes = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&wakes);
    let mirror = TerminalMirror::attach(
        DaemonClient::new(Arc::new(Host(Mutex::new(Some(Frames {
            tx: frames.clone(),
            rx,
            cancelled: Arc::new(AtomicBool::new(false)),
        }))))),
        "find".into(),
        Arc::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
        }),
    )
    .unwrap();
    let _view = mirror.view();
    let lines = |count: usize| "\r\nline".repeat(count).into_bytes();
    mirror.test_feed(1, &lines(4));
    let start = mirror.scroll_mark();
    mirror.test_feed(2, &lines(6));
    assert_eq!(start.moved_since(mirror.scroll_mark()), Some(6));
    assert_eq!(mirror.scroll_state().display_offset, 0);

    let hold = mirror.hold_viewport();
    mirror.test_feed(3, &lines(3));
    assert_eq!(mirror.scroll_state().display_offset, 3);
    drop(hold);
    mirror.test_feed(4, &lines(2));
    assert_eq!(mirror.scroll_state().display_offset, 5);
    mirror.scroll_to_bottom();
    mirror.test_feed(5, &lines(2));
    assert_eq!(mirror.scroll_state().display_offset, 0);
    assert_eq!(start.moved_since(mirror.scroll_mark()), Some(13));

    mirror.resize_local(20, 5);
    assert_eq!(start.moved_since(mirror.scroll_mark()), Some(13));

    // Right-margin HT with wrap pending stays on the full bottom row; the next
    // printable character wraps and scrolls a held viewport once.
    let hold = mirror.hold_viewport();
    let mark = mirror.scroll_mark();
    mirror.test_feed(6, b"\x1b[5;1Habcdefghijklmnopqrst");
    assert_eq!(mark.moved_since(mirror.scroll_mark()), Some(0));
    mirror.test_feed(7, b"\t");
    assert_eq!(mark.moved_since(mirror.scroll_mark()), Some(0));
    assert_eq!(mirror.scroll_state().display_offset, 0);
    mirror.test_feed(8, b"z");
    assert_eq!(mark.moved_since(mirror.scroll_mark()), Some(1));
    assert_eq!(mirror.scroll_state().display_offset, 1);
    drop(hold);
    mirror.scroll_to_bottom();

    // Bursts past the whole-page scrollback evict older pages and leave a held
    // viewport clamped at the top of actual retained history. Chunked overflow
    // tracks exact scrolled rows across page recycling; a single write larger
    // than the whole retained grid advances the scroll mark by at least the
    // full grid height so prior matches leave scrollback and held views clamp.
    mirror.configure(20, CursorShape::Block, false);
    let mut seq = 9u64;
    let mut line_num = 0usize;
    {
        let mark = mirror.scroll_mark();
        let hold = mirror.hold_viewport();
        let total_rows = 2_500usize;
        for _ in 0..250 {
            let payload: Vec<u8> = (line_num..line_num + 10)
                .map(|n| format!("\r\nrow-{n:05}"))
                .collect::<String>()
                .into_bytes();
            line_num += 10;
            mirror.test_feed(seq, &payload);
            seq += 1;
        }
        let state = mirror.scroll_state();
        assert!(
            state.history_lines > 0 && state.history_lines < total_rows,
            "expected whole-page eviction for chunked overflow, got history={}",
            state.history_lines
        );
        assert_eq!(state.display_offset, state.history_lines);
        assert_eq!(
            mark.moved_since(mirror.scroll_mark()),
            Some(total_rows as i32)
        );
        drop(hold);
        mirror.scroll_to_bottom();
    }
    for burst in [2_500usize, 3_500] {
        let mark = mirror.scroll_mark();
        let hold = mirror.hold_viewport();
        let payload: Vec<u8> = (line_num..line_num + burst)
            .map(|n| format!("\r\nrow-{n:05}"))
            .collect::<String>()
            .into_bytes();
        line_num += burst;
        mirror.test_feed(seq, &payload);
        seq += 1;
        let state = mirror.scroll_state();
        assert!(
            state.history_lines > 0 && state.history_lines < burst,
            "expected whole-page eviction for burst={burst}, got history={}",
            state.history_lines
        );
        assert_eq!(state.display_offset, state.history_lines, "{burst}");
        assert_eq!(
            mark.moved_since(mirror.scroll_mark()),
            Some(burst as i32),
            "single-write overflow should track exact scrolled rows: burst={burst} state={state:?}"
        );
        drop(hold);
        mirror.scroll_to_bottom();
    }

    // Scrollback 0 retains zero rows and keeps a held viewport at offset 0.
    mirror.configure(0, CursorShape::Block, false);
    assert_eq!(mirror.scroll_state().history_lines, 0);
    let hold = mirror.hold_viewport();
    mirror.test_feed(seq, &lines(10));
    seq += 1;
    assert_eq!(mirror.scroll_state().history_lines, 0);
    assert_eq!(mirror.scroll_state().display_offset, 0);
    drop(hold);

    // A resize from the daemon, with the controller unchanged, still
    // wakes the views so an open find bar recomputes.
    let before = wakes.load(Ordering::SeqCst);
    let before_activity = mirror.output_activity();
    let mark = mirror.scroll_mark();
    frames
        .send(TerminalFrame::Resized {
            seq,
            cols: 24,
            rows: 5,
            controller: Default::default(),
        })
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while wakes.load(Ordering::SeqCst) == before {
        assert!(
            Instant::now() < deadline,
            "a resize frame did not wake the views"
        );
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(mirror.size(), (24, 5));
    let after_resize = mirror.output_activity();
    assert_eq!(after_resize.last_seq, seq);
    assert_eq!(
        after_resize.last_output_seq,
        before_activity.last_output_seq
    );
    assert_eq!(after_resize.last_paint_seq, before_activity.last_paint_seq);
    assert_eq!(mark.moved_since(mirror.scroll_mark()), None);
    let resized = mirror.scroll_mark();
    mirror.test_feed(seq + 1, b"\x1b[?1049h");
    assert_eq!(resized.moved_since(mirror.scroll_mark()), None);
    drop(frames);
    mirror.test_stop_frames();
}

#[test]
fn large_synchronized_updates_stay_held_until_end_marker() {
    let mut term = crate::replay::Terminal::new(20, 5);
    let mut first = b"\x1b[?2026h".to_vec();
    first.resize(1024 * 1024, b'x');
    let mut second = vec![b'y'; 1024 * 1024 - 12];
    second.extend_from_slice(b"\x1b[?2026hheld");
    for chunk in [first.as_slice(), second.as_slice(), b"z\x1b[?2026hnew"] {
        term.advance(chunk);
    }
    assert!(term.is_sync_held());
    term.advance(b"\x1b[?2026l");
    assert!(!term.is_sync_held());
    assert!(term.screen().visible_lines().join("").contains("new"));
}

#[test]
fn shell_title_sanitization_preserves_content_and_bounds_the_label() {
    assert_eq!(sanitize_title("  ⠋ shell ⠹  "), "  ⠋ shell ⠹  ");
    assert_eq!(sanitize_title("A\nB\tC\u{2028}D"), "A B C D");
    assert_eq!(sanitize_title(&"界".repeat(600)).chars().count(), 512);
}

fn osc7_payloads(chunks: &[&[u8]]) -> Vec<String> {
    let mut term = crate::vt::Terminal::new(&crate::vt::TerminalConfig::default()).unwrap();
    let mut out = Vec::new();
    for chunk in chunks {
        term.vt_write(chunk);
        for effect in term.take_effects() {
            if let crate::vt::TerminalEffect::PwdChanged(pwd) = effect {
                out.push(String::from_utf8(pwd).unwrap());
            }
        }
    }
    out
}

#[test]
fn osc7_reports_end_at_bel_or_st_across_any_chunk_split() {
    let stream: &[u8] = "\x1b[31mls\x1b[0m\r\n\x1b]7;file://h/a\x07\x1b]0;title\x07\x1b]7;file://h/b%20c\x1b\\\x1b]8;;x\x1b\\$ "
            .as_bytes();
    let expected = vec!["file://h/a".to_string(), "file://h/b%20c".to_string()];
    assert_eq!(osc7_payloads(&[stream]), expected);
    for first in 0..=stream.len() {
        for second in first..=stream.len() {
            assert_eq!(
                osc7_payloads(&[&stream[..first], &stream[first..second], &stream[second..]]),
                expected,
                "split at {first} and {second}"
            );
        }
    }
}

#[test]
fn osc7_reports_abort_on_can_and_sub_and_skip_overlong_payloads() {
    // Under Ghostty's VT500 parser, ESC ends an OSC string (`osc_end`), while
    // CAN (0x18) and SUB (0x1a) abort it without dispatching.
    assert_eq!(
        osc7_payloads(&[b"\x1b]7;file:///a\x18 \x1b]7;file:///b\x1a \x1b]7;file:///c\x07"]),
        vec!["file:///c"]
    );
    let overlong = format!("\x1b]7;file:///{}", "a".repeat(16 * 1024));
    assert!(osc7_payloads(&[format!("{overlong}\x07").as_bytes()]).is_empty());
    assert_eq!(
        osc7_payloads(&[
            overlong.as_bytes(),
            b"aaaa\x07 \x1b]7;file:///ok\x1b",
            b"\\"
        ]),
        vec!["file:///ok"]
    );
    // 0x9C ends a report only as C1 ST, never inside UTF-8 text: 朝 is E6 9C 9D.
    assert_eq!(
        osc7_payloads(&["\x1b]7;file:///tmp/朝\x07".as_bytes()]),
        vec!["file:///tmp/朝"]
    );
}

#[cfg(unix)]
#[test]
fn osc7_reports_name_an_absolute_directory_on_this_machine() {
    let parse = |uri: &str| {
        super::parse_cwd_report(uri.as_bytes(), &|host| {
            matches!(host, "" | "localhost" | "test-host")
        })
    };
    let host = "test-host";
    let label = host.split('.').next().unwrap().to_owned();
    let tmp = Some(std::path::PathBuf::from("/tmp/a b/中文"));
    assert_eq!(parse("file:///tmp/a%20b/%E4%B8%AD%E6%96%87"), tmp);
    assert_eq!(parse("file://localhost/tmp/a b/中文"), tmp);
    assert_eq!(parse("FILE://local%68ost/tmp/a%20b/中文"), tmp);
    assert_eq!(parse(&format!("file://{host}/tmp/a%20b/中文")), tmp);
    assert_eq!(parse(&format!("file://{label}/tmp/a%20b/中文")), tmp);
    assert_eq!(parse("file:///tmp/x?query#fragment"), Some("/tmp/x".into()));
    for ignored in [
        "file://runner-575-elsewhere.invalid/tmp",
        "kitty-shell-cwd://localhost/tmp",
        "http://localhost/tmp",
        "file://localhost",
        "file:",
        "file:///tmp/%FF",
        "",
        "garbage",
    ] {
        assert_eq!(parse(ignored), None, "{ignored:?}");
    }
    assert_eq!(
        super::parse_cwd_report(b"file:///tmp/\xff", &|_| true),
        None
    );
}

#[test]
fn osc7_file_uri_paths_drop_the_slash_before_a_windows_drive() {
    assert_eq!(super::strip_drive_slash("/C:/Users/me"), "C:/Users/me");
    assert_eq!(super::strip_drive_slash("/d:"), "d:");
    assert_eq!(super::strip_drive_slash("/Users/me"), "/Users/me");
    assert_eq!(super::strip_drive_slash("/1:/x"), "/1:/x");
}

#[test]
fn only_kitty_flag_queries_are_masked() {
    let masked = |text: &str| {
        String::from_utf8(super::model::mask_kitty_keyboard_query_reply(
            text.as_bytes().to_vec(),
        ))
        .unwrap()
    };
    assert_eq!(masked("\x1b[?0u"), "\x1b[?0u");
    assert_eq!(masked("\x1b[?1u"), "\x1b[?1u");
    assert_eq!(masked("\x1b[?31u"), "\x1b[?1u");
    assert_eq!(masked("\x1b[?6u"), "\x1b[?0u");
    for other in [
        "\x1b[?u",
        "\x1b[?6c",
        "\x1b[?2031;0$y",
        "\x1b[0n",
        "\x1b[?1;2u",
        "\x1b[?999u",
    ] {
        assert_eq!(masked(other), other, "{other:?}");
    }
}

#[test]
fn scheme_sequences_are_seen_once_across_chunk_splits() {
    let mut term = crate::vt::Terminal::new(&crate::vt::TerminalConfig::default()).unwrap();
    term.set_color_scheme(crate::vt::ColorScheme::Dark);
    term.vt_write(b"\x1b[?2031h\x1b[?996n");
    assert!(term.mode(crate::vt::dec_mode(2031)));
    let replies: Vec<Vec<u8>> = term
        .take_effects()
        .into_iter()
        .filter_map(|e| match e {
            crate::vt::TerminalEffect::WritePty(b) => Some(b),
            _ => None,
        })
        .collect();
    assert_eq!(replies, vec![b"\x1b[?997;1n".to_vec()]);
    term.vt_write(b"plain");
    assert!(term.take_effects().is_empty());
    term.vt_write(b"\x1b[?20");
    assert!(term.mode(crate::vt::dec_mode(2031)));
    term.vt_write(b"31l");
    assert!(!term.mode(crate::vt::dec_mode(2031)));
    term.vt_write(b"");
    term.vt_write(b"\x1b[?25h");
    assert!(!term.mode(crate::vt::dec_mode(2031)));
}

#[test]
fn file_uri_targets_resolve_cross_platform_and_windows_paths() {
    let file_target = |path: &str, line: Option<u32>, column: Option<u32>| LinkTarget::File {
        path: PathBuf::from(path),
        line,
        column,
    };
    let local_name = Some("Jasons-Mac-Studio.local");

    assert_eq!(
        super::file_target_from_uri_with_platform("file:///tmp/report.html", false, local_name),
        Some(file_target("/tmp/report.html", None, None))
    );
    assert_eq!(
        super::file_target_from_uri_with_platform(
            "FILE:///tmp/report.html:12:5",
            false,
            local_name
        ),
        Some(file_target("/tmp/report.html", Some(12), Some(5)))
    );
    assert_eq!(
        super::file_target_from_uri_with_platform(
            "file://localhost/tmp/my%20report.html?view=1#L42",
            false,
            local_name
        ),
        Some(file_target("/tmp/my report.html", Some(42), None))
    );
    assert_eq!(
        super::file_target_from_uri_with_platform(
            "file://Jasons-Mac-Studio.local/tmp/report.html",
            false,
            local_name
        ),
        Some(file_target("/tmp/report.html", None, None))
    );
    assert_eq!(
        super::file_target_from_uri_with_platform(
            "file://jasons-mac-studio/tmp/report.html#L8",
            false,
            local_name
        ),
        Some(file_target("/tmp/report.html", Some(8), None))
    );

    assert_eq!(
        super::file_target_from_uri_with_platform(
            "file:///C:/Users/Jason/report.html",
            true,
            local_name
        ),
        Some(file_target(r"C:\Users\Jason\report.html", None, None))
    );
    assert_eq!(
        super::file_target_from_uri_with_platform(
            "file:///C:/Users/Jason/report.html",
            false,
            local_name
        ),
        Some(file_target("/C:/Users/Jason/report.html", None, None))
    );
    assert_eq!(
        super::file_target_from_uri_with_platform(
            "file://LocalHost/C:/Users/Jason%20Wang/report.html:12:5",
            true,
            local_name
        ),
        Some(file_target(
            r"C:\Users\Jason Wang\report.html",
            Some(12),
            Some(5)
        ))
    );
    assert_eq!(
        super::file_target_from_uri_with_platform(
            "file://JASONS-MAC-STUDIO/C:/Users/Jason/report.html",
            true,
            local_name
        ),
        Some(file_target(r"C:\Users\Jason\report.html", None, None))
    );
    assert_eq!(
        super::file_target_from_uri_with_platform(
            "FILE:///c%3A/Users/me/../you/report.html?foo=bar:1#L9",
            true,
            local_name
        ),
        Some(file_target(r"c:\Users\you\report.html", Some(9), None))
    );
    assert_eq!(
        super::file_target_from_uri_with_platform("file:///tmp/x.rs:7", true, local_name),
        Some(file_target("/tmp/x.rs", Some(7), None))
    );

    for invalid in [
        "file://other-host.invalid/tmp/report.html",
        "file://other-host.invalid/C:/report.html",
        "file:////other-host.invalid/share/report.html",
        r"file:///\\other-host.invalid\share\report.html",
        "file:///",
        "file://localhost",
        "file:///tmp/%FF",
        "file:///tmp/bad%00path",
        "https://example.com/report.html",
    ] {
        assert_eq!(
            super::file_target_from_uri_with_platform(invalid, false, local_name),
            None,
            "unix {invalid:?}"
        );
        assert_eq!(
            super::file_target_from_uri_with_platform(invalid, true, local_name),
            None,
            "windows {invalid:?}"
        );
    }
    for windows_invalid in ["file:///C:/", "file:///C:", "file:///C:relative.html"] {
        assert_eq!(
            super::file_target_from_uri_with_platform(windows_invalid, true, local_name),
            None,
            "{windows_invalid:?}"
        );
    }
}

#[test]
fn plain_text_and_osc8_file_uris_resolve_to_file_links() {
    let cwd = tempfile::tempdir().unwrap();
    std::fs::write(cwd.path().join("report.html"), "").unwrap();
    let host = runner_core::local_host::local_hostname().expect("local hostname");
    let short_host = host.split('.').next().unwrap_or(&host).to_ascii_lowercase();

    let output = format!(
        "see file:///tmp/report.html:12:5 and FILE://localhost/tmp/my%20file.html#L7.\r\n\
         remote file://other-host.invalid/tmp/secret.txt https://example.com/ok\r\n\
         \x1b]8;;file:///tmp/osc.html?v=1#L9\x1b\\open\x1b]8;;\x1b\\ \
         \x1b]8;;file://other-host.invalid/report.html\x1b\\report.html\x1b]8;;\x1b\\ \
         \x1b]8;;file://other-host.invalid/report.html\x1b\\https://example.com/label\x1b]8;;\x1b\\\r\n\
         \x1b]8;;file://{host}/tmp/local-full.html\x1b\\full\x1b]8;;\x1b\\ \
         \x1b]8;;file://{short_host}/tmp/local-short.html\x1b\\short\x1b]8;;\x1b\\"
    );
    let term = link_term(120, output.as_bytes());
    let cwds = [cwd.path()];

    let first = terminal_link_at(&term, Point::new(Line(0), Column(10)), &cwds).unwrap();
    assert_eq!(
        first.target,
        LinkTarget::File {
            path: PathBuf::from("/tmp/report.html"),
            line: Some(12),
            column: Some(5),
        }
    );
    assert_eq!(first.start, Point::new(Line(0), Column(4)));
    assert_eq!(first.end, Point::new(Line(0), Column(31)));

    let second = terminal_link_at(&term, Point::new(Line(0), Column(45)), &cwds).unwrap();
    assert_eq!(
        second.target,
        LinkTarget::File {
            path: PathBuf::from("/tmp/my file.html"),
            line: Some(7),
            column: None,
        }
    );
    assert_eq!(second.end, Point::new(Line(0), Column(74)));
    assert!(
        terminal_link_at(&term, Point::new(Line(0), Column(75)), &cwds).is_none(),
        "trailing period is excluded"
    );

    assert!(
        terminal_link_at(&term, Point::new(Line(1), Column(12)), &cwds).is_none(),
        "remote file:// URI in plain text must not link or fall back to Url"
    );
    assert_eq!(
        terminal_link_at(&term, Point::new(Line(1), Column(52)), &cwds)
            .unwrap()
            .target,
        LinkTarget::Url("https://example.com/ok".into())
    );

    let osc = terminal_link_at(&term, Point::new(Line(2), Column(1)), &cwds).unwrap();
    assert_eq!(
        osc.target,
        LinkTarget::File {
            path: PathBuf::from("/tmp/osc.html"),
            line: Some(9),
            column: None,
        }
    );
    assert!(
        terminal_link_at(&term, Point::new(Line(2), Column(8)), &cwds).is_none(),
        "rejected OSC 8 file:// URI must not fall back to file_link_regex on label"
    );
    assert!(
        terminal_link_at(&term, Point::new(Line(2), Column(25)), &cwds).is_none(),
        "rejected OSC 8 file:// URI must not fall back to url_regex on label"
    );

    let local_full = terminal_link_at(&term, Point::new(Line(3), Column(1)), &cwds).unwrap();
    assert_eq!(
        local_full.target,
        LinkTarget::File {
            path: PathBuf::from("/tmp/local-full.html"),
            line: None,
            column: None,
        }
    );
    let local_short = terminal_link_at(&term, Point::new(Line(3), Column(6)), &cwds).unwrap();
    assert_eq!(
        local_short.target,
        LinkTarget::File {
            path: PathBuf::from("/tmp/local-short.html"),
            line: None,
            column: None,
        }
    );

    let underlined = link_term(
        40,
        b"\x1b[4;34mhttps://example.com/path\x1b[0m\r\n  file:///tmp/next.html",
    );
    let top = terminal_link_at(&underlined, Point::new(Line(0), Column(5)), &[]).unwrap();
    assert_eq!(
        top.target,
        LinkTarget::Url("https://example.com/path".into())
    );
    assert_eq!(top.end.line, Line(0));
}

#[test]
fn file_link_regex_matches_windows_drive_and_unc_paths_with_suffixes() {
    let parse_match = |text: &str, needle: &str| {
        let offset = text.find(needle)?;
        let candidate = super::scan_plain_file_candidates(text)
            .into_iter()
            .find(|c| c.full_range.start <= offset && c.full_range.end > offset)?;
        Some((candidate.path, candidate.line, candidate.column))
    };

    for (input, needle, expected) in [
        (
            r"C:\Users\me\report.html",
            "report.html",
            (r"C:\Users\me\report.html", None, None),
        ),
        (
            "C:/Users/me/report.html",
            "report.html",
            ("C:/Users/me/report.html", None, None),
        ),
        (
            r"C:\Users\me\report.html:12",
            "report.html",
            (r"C:\Users\me\report.html", Some(12), None),
        ),
        (
            r"C:\Users\me\report.html:12:5",
            "report.html",
            (r"C:\Users\me\report.html", Some(12), Some(5)),
        ),
        (
            "C:/Users/me/report.html(3, 4)",
            "report.html",
            ("C:/Users/me/report.html", Some(3), Some(4)),
        ),
        (
            r"C:\Users\me\report.html#L9",
            "report.html",
            (r"C:\Users\me\report.html", Some(9), None),
        ),
        (
            r"See (C:\Users\me\report.html:2).",
            "C:",
            (r"C:\Users\me\report.html", Some(2), None),
        ),
        (
            r"Error in C:\Users\me\report.html: error",
            "C:",
            (r"C:\Users\me\report.html", None, None),
        ),
        (
            r"\\server\shared\report.html:7",
            "report.html",
            (r"\\server\shared\report.html", Some(7), None),
        ),
        (
            r"src\lib.rs:12:5",
            "lib.rs",
            (r"src\lib.rs", Some(12), Some(5)),
        ),
        (r".\README.md", "README", (r".\README.md", None, None)),
        (
            r"..\..\docs\guide.md",
            "guide.md",
            (r"..\..\docs\guide.md", None, None),
        ),
        (r"~\notes.txt", "notes.txt", (r"~\notes.txt", None, None)),
    ] {
        assert_eq!(
            parse_match(input, needle),
            Some((expected.0.to_owned(), expected.1, expected.2)),
            "{input:?}"
        );
    }
    assert!(parse_match(r"C:\", r"\").is_none());
}

#[test]
fn windows_candidate_paths_and_normalization_handle_drives_unc_and_relative_cwds() {
    let win_cwd = Path::new(r"C:\repo\sub");
    assert_eq!(
        super::candidate_paths(r"C:\Users\me\..\you\report.html", &[], None, true),
        vec![PathBuf::from(r"C:\Users\you\report.html")]
    );
    assert_eq!(
        super::candidate_paths("C:/Users/me/./report.html", &[win_cwd], None, true),
        vec![PathBuf::from(r"C:\Users\me\report.html")]
    );
    assert_eq!(
        super::candidate_paths(r"\\server\share\dir\..\report.html", &[], None, true),
        vec![PathBuf::from(r"\\server\share\report.html")]
    );
    assert_eq!(
        super::candidate_paths(r"\\server\share\..\..\report.html", &[], None, true),
        vec![PathBuf::from(r"\\server\share\report.html")]
    );
    assert_eq!(
        super::candidate_paths(r"..\other\lib.rs", &[win_cwd], None, true),
        vec![PathBuf::from(r"C:\repo\other\lib.rs")]
    );
    assert_eq!(
        super::candidate_paths(
            r"~\notes.txt",
            &[],
            Some(Path::new(r"C:\Users\jason")),
            true
        ),
        vec![PathBuf::from(r"C:\Users\jason\notes.txt")]
    );

    assert_eq!(
        super::candidate_paths("src/lib.rs", &[Path::new("/repo")], None, false),
        vec![PathBuf::from("/repo/src/lib.rs")]
    );
    assert_eq!(
        super::candidate_paths("/C:/Users/me/report.html", &[], None, false),
        vec![PathBuf::from("/C:/Users/me/report.html")]
    );
    assert_eq!(
        super::candidate_paths("/C:/Users/me/../you/./report.html", &[], None, false),
        vec![PathBuf::from("/C:/Users/you/report.html")]
    );
}

#[cfg(unix)]
#[test]
fn posix_candidate_paths_preserve_non_utf8_bytes() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let home = PathBuf::from(OsString::from_vec(b"/tmp/home-\xFF".to_vec()));
    let expected = PathBuf::from(OsString::from_vec(b"/tmp/home-\xFF/report.html".to_vec()));
    assert_eq!(
        super::candidate_paths("~/sub/../report.html", &[], Some(&home), false),
        vec![expected]
    );
}

#[cfg(windows)]
#[test]
fn windows_drive_paths_and_file_uris_resolve_in_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("project");
    std::fs::create_dir_all(cwd.join("src")).unwrap();
    let report = cwd.join("report.html");
    let lib = cwd.join("src").join("lib.rs");
    std::fs::write(&report, "<html></html>").unwrap();
    std::fs::write(&lib, "fn main() {}\n").unwrap();

    let backslash_path = report.to_string_lossy().into_owned();
    let slash_path = backslash_path.replace('\\', "/");
    let uri_path = format!("file:///{slash_path}");
    let localhost_uri = format!("file://localhost/{slash_path}");

    let output = format!(
        "{backslash_path}:12:5 {slash_path}(3,4)\r\n\
         {uri_path}:7 {localhost_uri}#L9\r\n\
         src\\lib.rs:4:2\r\n\
         \x1b]8;;{uri_path}#L15\x1b\\open\x1b]8;;\x1b\\"
    );
    let term = link_term(240, output.as_bytes());

    let line0_bs = terminal_link_at(&term, Point::new(Line(0), Column(3)), &[]).unwrap();
    assert_eq!(
        line0_bs.target,
        LinkTarget::File {
            path: report.clone(),
            line: Some(12),
            column: Some(5),
        }
    );

    let line0_sl = terminal_link_at(
        &term,
        Point::new(Line(0), Column(backslash_path.len() + 8)),
        &[],
    )
    .unwrap();
    assert_eq!(
        line0_sl.target,
        LinkTarget::File {
            path: report.clone(),
            line: Some(3),
            column: Some(4),
        }
    );

    let line1_uri = terminal_link_at(&term, Point::new(Line(1), Column(2)), &[]).unwrap();
    assert_eq!(
        line1_uri.target,
        LinkTarget::File {
            path: report.clone(),
            line: Some(7),
            column: None,
        }
    );

    let line1_local =
        terminal_link_at(&term, Point::new(Line(1), Column(uri_path.len() + 6)), &[]).unwrap();
    assert_eq!(
        line1_local.target,
        LinkTarget::File {
            path: report.clone(),
            line: Some(9),
            column: None,
        }
    );

    let line2_rel =
        terminal_link_at(&term, Point::new(Line(2), Column(2)), &[cwd.as_path()]).unwrap();
    assert_eq!(
        line2_rel.target,
        LinkTarget::File {
            path: lib,
            line: Some(4),
            column: Some(2),
        }
    );

    let line3_osc = terminal_link_at(&term, Point::new(Line(3), Column(1)), &[]).unwrap();
    assert_eq!(
        line3_osc.target,
        LinkTarget::File {
            path: report,
            line: Some(15),
            column: None,
        }
    );
}

fn snapshot_mirror(seq: u64, chunks: &[&[u8]]) -> Arc<TerminalMirror> {
    use runner_core::protocol::terminal::{
        TerminalAttachment, TerminalSnapshot, TerminalSubscription,
    };
    use runner_core::protocol::{ClientError, EventSubscription, Request, Response, Transport};

    struct ClosedFrames;
    impl TerminalSubscription for ClosedFrames {
        fn recv(&mut self) -> std::result::Result<TerminalFrame, ClientError> {
            Err(ClientError::msg("closed"))
        }
        fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
            Arc::new(|| {})
        }
    }

    struct SnapshotHost {
        snapshot: Mutex<Option<TerminalSnapshot>>,
    }
    impl Transport for SnapshotHost {
        fn call(&self, _: Request) -> std::result::Result<Response, ClientError> {
            Ok(Response::terminal_metadata(Ok(TerminalMetadata::default())))
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            unreachable!()
        }
        fn attach(&self, _: &str) -> std::result::Result<TerminalAttachment, ClientError> {
            Ok(TerminalAttachment {
                subscriber_id: 1,
                snapshot: self.snapshot.lock().unwrap().take().unwrap(),
                frames: Box::new(ClosedFrames),
            })
        }
    }

    let mut term = crate::replay::Terminal::with_kitty_keyboard(80, 24);
    for chunk in chunks {
        term.advance(chunk);
    }
    let unfinished = term.unfinished().to_vec();
    let snapshot = TerminalSnapshot {
        seq,
        format: SnapshotFormat::Vt,
        unavailable: false,
        cols: 80,
        rows: 24,
        bytes: crate::snapshot::serialize(&term, &unfinished),
        unfinished_len: unfinished.len(),
        preceding_char: term.preceding_char(),
        activity: None,
        controller: Default::default(),
    };
    let host = Arc::new(SnapshotHost {
        snapshot: Mutex::new(Some(snapshot)),
    });
    TerminalMirror::attach(DaemonClient::new(host), "snapshot".into(), Arc::new(|| {})).unwrap()
}

#[test]
fn restore_ignores_serializer_alt_screen_scaffolding_on_empty_and_plain_snapshots() {
    let empty = snapshot_mirror(0, &[]);
    let activity = empty.output_activity();
    assert_eq!(activity.last_seq, 0);
    assert_eq!(activity.tui_ready_seq, 0);
    assert_eq!(activity.first_paint_seq, 0);
    assert!(activity.last_output_at.is_none());

    let plain_blank = snapshot_mirror(1, &[b"\r\n\x1b]7;file://localhost/tmp\x1b\\   "]);
    let activity = plain_blank.output_activity();
    assert_eq!(activity.last_seq, 1);
    assert_eq!(activity.tui_ready_seq, 0);
    assert_eq!(activity.first_paint_seq, 0);
    assert!(activity.last_output_at.is_some());

    let plain_prompt = snapshot_mirror(2, &[b"host% "]);
    let activity = plain_prompt.output_activity();
    assert_eq!(activity.last_seq, 2);
    assert_eq!(activity.tui_ready_seq, 0);
    assert_eq!(activity.first_paint_seq, 2);
    assert!(activity.last_output_at.is_some());
}

#[test]
fn restore_preserves_tui_ready_modes_and_open_sync_updates_from_snapshot() {
    for chunks in [
        &[b"\x1b[?1049h\x1b[2J\x1b[H".as_slice()][..],
        &[b"\x1b[?2004h\x1b[?1004h".as_slice()][..],
        &[b"\x1b[?25l".as_slice()][..],
        &[b"\x1b[>5u".as_slice()][..],
        &[b"\x1b[?2026h\x1b[2J\x1b[H".as_slice()][..],
    ] {
        let mirror = snapshot_mirror(3, chunks);
        let activity = mirror.output_activity();
        assert_eq!(activity.last_seq, 3);
        assert_eq!(activity.tui_ready_seq, 3);
        assert_eq!(activity.first_paint_seq, 0);
        assert!(activity.last_output_at.is_some());
    }
}

#[test]
fn claude_code_startup_marks_tui_ready_before_first_paint_across_snapshot_splits() {
    let pre_paint: &[&[u8]] = &[
        b"\x1b7\x1b[r\x1b8\x1b[?25h",
        b"\x1b[?25l",
        b"\x1b[?2004h\x1b[?2031h\x1b[?1004h",
        b"\x1b]11;?\x07\x1b[c",
        b"\x1b[>0q",
        b"\x1b[?u\x1b[c",
        b"\x1b[<u\x1b[>5u\x1b[>4;2m",
        b"\x1b[>4m\x1b[<u\x1b[?1004l\x1b[?2031l\x1b[?2004l",
        b"\x1b]0;\xe2\x9c\xb3 Claude Code\x07\x1b[?2004h\x1b[?1049h\x1b[2J\x1b[H",
    ];
    let first_frame = b"\x1b[1;1HClaude Code v2.1";

    for split in 0..=pre_paint.len() {
        let mirror = snapshot_mirror(split as u64, &pre_paint[..split]);
        if split >= 2 {
            let restored = mirror.output_activity();
            assert!(
                restored.tui_ready_seq > 0,
                "expected snapshot split {split} to restore tui_ready_seq"
            );
            assert_eq!(restored.first_paint_seq, 0, "split {split}");
            assert!(restored.last_output_at.is_some(), "split {split}");
        }
        for (index, chunk) in pre_paint[split..].iter().enumerate() {
            let seq = (split + index + 1) as u64;
            mirror.test_feed(seq, chunk);
            let activity = mirror.output_activity();
            if seq >= 2 {
                assert!(
                    activity.tui_ready_seq > 0,
                    "expected live seq {seq} after split {split} to have tui_ready_seq"
                );
            }
            assert_eq!(activity.first_paint_seq, 0, "split {split} seq {seq}");
        }
        let paint_seq = (pre_paint.len() + 1) as u64;
        mirror.test_feed(paint_seq, first_frame);
        let painted = mirror.output_activity();
        assert_eq!(painted.first_paint_seq, paint_seq, "split {split}");
        assert!(painted.tui_ready_seq > 0, "split {split}");
    }
}

#[test]
fn split_mode_sequence_across_live_chunks_marks_tui_ready() {
    let mirror = snapshot_mirror(0, &[]);
    mirror.test_feed(1, b"\x1b[?10");
    assert_eq!(mirror.output_activity().tui_ready_seq, 0);
    assert_eq!(mirror.output_activity().first_paint_seq, 0);

    mirror.test_feed(2, b"49h");
    assert_eq!(mirror.output_activity().tui_ready_seq, 2);
    assert_eq!(mirror.output_activity().first_paint_seq, 0);
}

#[test]
fn restore_empty_nonzero_snapshot_and_resume_repaint_track_output_and_last_paint_seq() {
    // Resize-only pre-resume snapshot (non-zero seq, serializer scaffold only).
    let resized_empty = snapshot_mirror(2, &[]);
    let empty_activity = resized_empty.output_activity();
    assert_eq!(empty_activity.last_seq, 2);
    assert_eq!(empty_activity.last_output_seq, 0);
    assert_eq!(empty_activity.tui_ready_seq, 0);
    assert_eq!(empty_activity.first_paint_seq, 0);
    assert_eq!(empty_activity.last_paint_seq, 0);
    assert!(empty_activity.last_output_at.is_none());

    // Resumed mirror inheriting pre-stop painted content at baseline seq 50.
    let mirror = snapshot_mirror(50, &[b"\x1b[?2004hClaude Code v2.0"]);
    let initial = mirror.output_activity();
    assert_eq!(initial.last_seq, 50);
    assert_eq!(initial.last_output_seq, 50);
    assert_eq!(initial.tui_ready_seq, 50);
    assert_eq!(initial.first_paint_seq, 50);
    assert_eq!(initial.last_paint_seq, 50);

    // Cursor hide / alt-screen clear after resume must not advance last_paint_seq onto old text.
    mirror.test_feed(51, b"\x1b[?25l");
    let hid_cursor = mirror.output_activity();
    assert_eq!(hid_cursor.last_output_seq, 51);
    assert_eq!(hid_cursor.last_paint_seq, 50);

    mirror.test_feed(52, b"\x1b[?1049h\x1b[2J\x1b[H");
    let cleared = mirror.output_activity();
    assert_eq!(cleared.last_output_seq, 52);
    assert_eq!(cleared.tui_ready_seq, 52);
    assert_eq!(cleared.last_paint_seq, 50);

    // Draw + clear in the same chunk leaves the screen blank and must not report painted.
    mirror.test_feed(53, b"flash\x1b[2J\x1b[H");
    assert_eq!(mirror.output_activity().last_paint_seq, 50);

    // First post-resume visible frame advances last_paint_seq past baseline.
    mirror.test_feed(54, b"\x1b[1;1HWelcome back");
    let repainted = mirror.output_activity();
    assert_eq!(repainted.last_output_seq, 54);
    assert_eq!(repainted.first_paint_seq, 50);
    assert_eq!(repainted.last_paint_seq, 54);
}

#[test]
fn sync_timeout_flush_updates_last_paint_seq_without_trailing_chunk() {
    let (mirror, frames, _) = scripted_mirror(false);
    send_output(&frames, 1, b"\x1b[?2026h\x1b[1;1HWelcome back");
    assert!(wait_for(Duration::from_secs(2), || {
        mirror.output_activity().last_output_seq == 1
    }));
    let before_timeout = mirror.output_activity();
    assert!(before_timeout.tui_ready_seq == 1);
    assert_eq!(before_timeout.first_paint_seq, 0);
    assert_eq!(before_timeout.last_paint_seq, 0);
    assert!(mirror.sync_state().0.is_some());

    assert!(wait_for(Duration::from_secs(2), || {
        mirror.sync_state().0.is_none() && mirror.output_activity().last_paint_seq == 1
    }));
    let after_timeout = mirror.output_activity();
    assert_eq!(after_timeout.first_paint_seq, 1);
    assert_eq!(after_timeout.last_paint_seq, 1);
    assert!(screen_text(&mirror).contains("Welcome back"));

    // Simulate an already-painted resumed baseline, then a sync-wrapped mode-only chunk
    // that times out without painting anything new.
    send_output(&frames, 50, b"\x1b[?2004hClaude Code v2.0");
    assert!(wait_for(Duration::from_secs(2), || {
        mirror.output_activity().last_paint_seq == 50
    }));
    send_output(&frames, 51, b"\x1b[?2026h\x1b[?25l");
    assert!(wait_for(Duration::from_secs(2), || {
        mirror.output_activity().last_output_seq == 51 && mirror.sync_state().0.is_none()
    }));
    let mode_only = mirror.output_activity();
    assert_eq!(mode_only.tui_ready_seq, 51);
    assert_eq!(mode_only.last_paint_seq, 50);

    // Sync-wrapped visible paint without an end marker advances last_paint_seq on timeout
    // even when no later chunk arrives.
    send_output(&frames, 52, b"\x1b[?2026h\x1b[2;1HUpdated frame");
    assert!(wait_for(Duration::from_secs(2), || {
        mirror.sync_state().0.is_none() && mirror.output_activity().last_paint_seq == 52
    }));
    assert_eq!(mirror.output_activity().first_paint_seq, 1);
    assert_eq!(mirror.output_activity().last_paint_seq, 52);
    assert!(screen_text(&mirror).contains("Updated frame"));
}

#[test]
fn snapshot_provenance_preserves_paint_and_output_seq_across_attach_and_resync_races() {
    use runner_core::protocol::terminal::{
        TerminalAttachment, TerminalSnapshot, TerminalSubscription,
    };
    use runner_core::protocol::{ClientError, EventSubscription, Request, Response, Transport};

    struct ModelHost;
    impl TerminalHost for ModelHost {
        fn write_input(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn write_reply(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn report_input_state(&self, _: &str, _: InputObservation) {}
        fn set_live_title(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn resize(&self, _: &str, _: u16, _: u16) -> Result<()> {
            Ok(())
        }
        fn is_local_host(&self, _: &str) -> bool {
            true
        }
        fn clipboard_store(&self, _: &str, _: ClipboardTarget, _: &str) {}
    }

    struct DynamicFrames {
        rx: mpsc::Receiver<std::result::Result<TerminalFrame, ClientError>>,
        tx: mpsc::Sender<std::result::Result<TerminalFrame, ClientError>>,
    }
    impl TerminalSubscription for DynamicFrames {
        fn recv(&mut self) -> std::result::Result<TerminalFrame, ClientError> {
            self.rx.recv().map_err(|_| ClientError::msg("closed"))?
        }
        fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
            let tx = self.tx.clone();
            Arc::new(move || {
                let _ = tx.send(Err(ClientError::msg("cancelled")));
            })
        }
    }

    struct ProvenanceHost {
        snapshots: Mutex<std::collections::VecDeque<TerminalSnapshot>>,
        senders: Mutex<Vec<mpsc::Sender<std::result::Result<TerminalFrame, ClientError>>>>,
    }
    impl Transport for ProvenanceHost {
        fn call(&self, _: Request) -> std::result::Result<Response, ClientError> {
            Ok(Response::terminal_metadata(Ok(TerminalMetadata::default())))
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            unreachable!()
        }
        fn attach(&self, _: &str) -> std::result::Result<TerminalAttachment, ClientError> {
            let (tx, rx) = mpsc::channel();
            let mut senders = self.senders.lock().unwrap();
            let subscriber_id = senders.len() as u64 + 1;
            senders.push(tx.clone());
            Ok(TerminalAttachment {
                subscriber_id,
                snapshot: self.snapshots.lock().unwrap().pop_front().unwrap(),
                frames: Box::new(DynamicFrames { rx, tx }),
            })
        }
    }

    let model = TerminalModel::new(
        Arc::new(ModelHost),
        "resume-provenance".into(),
        80,
        24,
        ModelOptions::default(),
    )
    .unwrap();
    model.feed_output(1, b"\x1b[?2004hClaude Code v2.0");
    let baseline = model.snapshot_with_history(1, usize::MAX);
    assert_eq!(
        baseline.activity,
        Some(TerminalSnapshotActivity {
            output_seq: 1,
            tui_ready_seq: 1,
            first_paint_seq: 1,
            paint_seq: 1,
        })
    );

    // Resize after resume before attach must not bump last_output_seq or last_paint_seq.
    model.resize(100, 30);
    let resized_snapshot = model.snapshot_with_history(2, usize::MAX);
    assert_eq!(resized_snapshot.seq, 2);
    assert_eq!(
        resized_snapshot.activity,
        Some(TerminalSnapshotActivity {
            output_seq: 1,
            tui_ready_seq: 1,
            first_paint_seq: 1,
            paint_seq: 1,
        })
    );

    // Mode-only output after resume before attach / Resync must bump tui_ready_seq and
    // output_seq while leaving paint_seq at pre-stop baseline 1.
    model.feed_output(3, b"\x1b[?25l");
    let mode_snapshot = model.snapshot_with_history(3, usize::MAX);
    assert_eq!(
        mode_snapshot.activity,
        Some(TerminalSnapshotActivity {
            output_seq: 3,
            tui_ready_seq: 3,
            first_paint_seq: 1,
            paint_seq: 1,
        })
    );

    let attach_host = Arc::new(ProvenanceHost {
        snapshots: Mutex::new([resized_snapshot, mode_snapshot.clone()].into()),
        senders: Mutex::default(),
    });
    let attached_after_resize = TerminalMirror::attach(
        DaemonClient::new(attach_host.clone()),
        "resume-provenance".into(),
        Arc::new(|| {}),
    )
    .unwrap();
    let resize_activity = attached_after_resize.output_activity();
    assert_eq!(resize_activity.last_seq, 2);
    assert_eq!(resize_activity.last_output_seq, 1);
    assert_eq!(resize_activity.last_paint_seq, 1);

    let attached_after_mode = TerminalMirror::attach(
        DaemonClient::new(attach_host),
        "resume-provenance".into(),
        Arc::new(|| {}),
    )
    .unwrap();
    let mode_activity = attached_after_mode.output_activity();
    assert_eq!(mode_activity.last_seq, 3);
    assert_eq!(mode_activity.last_output_seq, 3);
    assert_eq!(mode_activity.tui_ready_seq, 3);
    assert_eq!(mode_activity.first_paint_seq, 1);
    assert_eq!(mode_activity.last_paint_seq, 1);

    // Resync onto a reused TerminalMirror at baseline=1 must likewise keep last_paint_seq at 1
    // until real visible output paints (even across sync-timeout flush on the model).
    let resync_host = Arc::new(ProvenanceHost {
        snapshots: Mutex::new([baseline, mode_snapshot].into()),
        senders: Mutex::default(),
    });
    let resync_mirror = TerminalMirror::attach(
        DaemonClient::new(resync_host.clone()),
        "resume-provenance".into(),
        Arc::new(|| {}),
    )
    .unwrap();
    resync_host.senders.lock().unwrap()[0]
        .send(Ok(TerminalFrame::Resync))
        .unwrap();
    assert!(wait_for(Duration::from_secs(2), || {
        resync_mirror.output_activity().last_seq == 3
    }));
    let after_resync = resync_mirror.output_activity();
    assert_eq!(after_resync.last_output_seq, 3);
    assert_eq!(after_resync.tui_ready_seq, 3);
    assert_eq!(after_resync.first_paint_seq, 1);
    assert_eq!(after_resync.last_paint_seq, 1);

    // Sync timeout on TerminalModel with real visible output advances snapshot.activity.paint_seq.
    model.feed_output(4, b"\x1b[?2026h\x1b[2;1HWelcome back");
    assert!(wait_for(Duration::from_secs(2), || {
        model
            .snapshot_with_history(4, usize::MAX)
            .activity
            .is_some_and(|activity| activity.paint_seq == 4)
    }));
    resync_host
        .snapshots
        .lock()
        .unwrap()
        .push_back(model.snapshot_with_history(4, usize::MAX));
    resync_host.senders.lock().unwrap()[1]
        .send(Ok(TerminalFrame::Resync))
        .unwrap();
    assert!(wait_for(Duration::from_secs(2), || {
        resync_mirror.output_activity().last_paint_seq == 4
    }));

    for worker in model.stop_workers() {
        worker.join().unwrap();
    }
}

fn link_term_sized(columns: usize, rows: usize, bytes: &[u8]) -> crate::replay::Terminal {
    let mut term = crate::replay::Terminal::new(columns as u16, rows as u16);
    term.advance(bytes);
    term
}

fn test_root(temp: &tempfile::TempDir) -> PathBuf {
    if cfg!(windows) {
        temp.path().to_path_buf()
    } else {
        temp.path().canonicalize().unwrap()
    }
}

fn native_path(root: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .fold(root.to_path_buf(), |path, part| path.join(part))
}

#[test]
fn hard_wrapped_paths_join_across_rows_including_1451_shape_suffixes_and_cjk() {
    let temp = tempfile::tempdir().unwrap();
    let root = test_root(&temp);

    // 1. The #1451 Codex wrapped shape: `(<path-part-1>` + `<path-part-2>)` with indentation.
    let nested = native_path(&root, "runner-1451-qa.UAUZ0TI8/codex-chat/qa-link-1451.md");
    std::fs::create_dir_all(nested.parent().unwrap()).unwrap();
    std::fs::write(&nested, "# qa\n").unwrap();
    let full = nested.to_string_lossy().into_owned();
    let split_at = full.find("1451-qa").unwrap();
    let (part1, part2) = full.split_at(split_at);
    let bytes = format!("  ({part1}\r\n  {part2})");
    let term = link_term_sized(80, 12, bytes.as_bytes());
    // Ensure WRAPLINE is unset, matching agent TUI hard-wrapping.
    assert!(!term
        .screen()
        .cell(Point::new(Line(0), Column(79)))
        .unwrap()
        .flags()
        .contains(CellFlags::WRAPLINE));
    let on_row0 = terminal_link_at(&term, Point::new(Line(0), Column(5)), &[]).unwrap();
    let on_row1 = terminal_link_at(&term, Point::new(Line(1), Column(6)), &[]).unwrap();
    assert_eq!(on_row0, on_row1);
    assert_eq!(
        on_row0.target,
        LinkTarget::File {
            path: nested.clone(),
            line: None,
            column: None,
        }
    );
    assert_eq!(on_row0.start, Point::new(Line(0), Column(3)));
    assert_eq!(
        on_row0.end,
        Point::new(Line(1), Column(2 + part2.len() - 1))
    );
    // Indentation, `(` and `)` are never highlighted.
    assert!(!on_row0.contains(Point::new(Line(0), Column(0))));
    assert!(!on_row0.contains(Point::new(Line(0), Column(2))));
    assert!(!on_row0.contains(Point::new(Line(1), Column(1))));
    assert!(!on_row0.contains(Point::new(Line(1), Column(2 + part2.len()))));

    // 2. Path spanning 8 rows (MAX_WRAP_JOIN_ROWS) via cursor positioning, plus 9-row cap check.
    let deep_dir = native_path(&root, "aa/bb/cc/dd/ee/ff/gg");
    std::fs::create_dir_all(&deep_dir).unwrap();
    let deep_file = deep_dir.join("target.rs");
    std::fs::write(&deep_file, "fn main() {}\n").unwrap();
    let rel_parts = ["aa/", "bb/", "cc/", "dd/", "ee/", "ff/", "gg/", "target.rs"];
    assert_eq!(rel_parts.len(), MAX_WRAP_JOIN_ROWS);
    let mut positioned = String::new();
    for (idx, chunk) in rel_parts.iter().enumerate() {
        positioned.push_str(&format!("\x1b[{};3H{chunk}", idx + 1));
    }
    let term8 = link_term_sized(40, 12, positioned.as_bytes());
    let first_row = terminal_link_at(&term8, Point::new(Line(0), Column(2)), &[&root]).unwrap();
    let last_row = terminal_link_at(
        &term8,
        Point::new(Line((MAX_WRAP_JOIN_ROWS - 1) as i32), Column(4)),
        &[&root],
    )
    .unwrap();
    assert_eq!(first_row, last_row);
    assert_eq!(
        first_row.target,
        LinkTarget::File {
            path: deep_file.clone(),
            line: None,
            column: None,
        }
    );
    assert!(!first_row.contains(Point::new(Line(0), Column(1))));
    assert!(!first_row.contains(Point::new(Line(7), Column(1))));

    // 9 rows exceeds MAX_WRAP_JOIN_ROWS = 8 so it must not join across all 9.
    let nine_parts = [
        "aa/", "bb/", "cc/", "dd/", "ee/", "ff/", "gg/", "tar", "get.rs",
    ];
    let term9 = link_term_sized(40, 12, nine_parts.join("\r\n").as_bytes());
    assert!(terminal_link_at(&term9, Point::new(Line(0), Column(0)), &[&root]).is_none());

    // 3. `:42` suffix wrapped onto the next row after an existing file path.
    std::fs::create_dir_all(root.join("src")).unwrap();
    let lib = native_path(&root, "src/lib.rs");
    std::fs::write(&lib, "pub fn hi() {}\n").unwrap();
    let suffix_term = link_term_sized(40, 8, b"  src/lib.rs\r\n  :42");
    let top = terminal_link_at(&suffix_term, Point::new(Line(0), Column(4)), &[&root]).unwrap();
    let bottom = terminal_link_at(&suffix_term, Point::new(Line(1), Column(3)), &[&root]).unwrap();
    assert_eq!(top, bottom);
    assert_eq!(
        top.target,
        LinkTarget::File {
            path: lib.clone(),
            line: Some(42),
            column: None,
        }
    );
    assert_eq!(top.start, Point::new(Line(0), Column(2)));
    assert_eq!(top.end, Point::new(Line(1), Column(4)));

    // 4. Wrapped CJK path.
    let cjk_dir = native_path(&root, "docs/我的项目");
    std::fs::create_dir_all(&cjk_dir).unwrap();
    let cjk_file = cjk_dir.join("计划.md");
    std::fs::write(&cjk_file, "# 计划\n").unwrap();
    let cjk_term = link_term_sized(40, 8, "  docs/我的项目/\r\n  计划.md:5".as_bytes());
    let cjk_top = terminal_link_at(&cjk_term, Point::new(Line(0), Column(3)), &[&root]).unwrap();
    let cjk_bot = terminal_link_at(&cjk_term, Point::new(Line(1), Column(2)), &[&root]).unwrap();
    assert_eq!(cjk_top, cjk_bot);
    assert_eq!(
        cjk_top.target,
        LinkTarget::File {
            path: cjk_file,
            line: Some(5),
            column: None,
        }
    );
    assert!(!cjk_top.contains(Point::new(Line(0), Column(1))));
    assert!(!cjk_top.contains(Point::new(Line(1), Column(1))));

    // 5. Path starting with a bare `/` root prefix on the row above the pointer.
    let full_no_slash = lib.to_string_lossy();
    if let Some(after_slash) = full_no_slash.strip_prefix('/') {
        let root_wrap = format!("see /\r\n{after_slash}");
        let term = link_term_sized(120, 8, root_wrap.as_bytes());
        let link = terminal_link_at(&term, Point::new(Line(1), Column(2)), &[]).unwrap();
        assert_eq!(
            link.target,
            LinkTarget::File {
                path: lib.clone(),
                line: None,
                column: None,
            }
        );
        assert_eq!(link.start, Point::new(Line(0), Column(4)));
    }

    // 6. Soft-wrapped spaced path preserves the boundary space rather than joining into a
    // non-spaced sibling file.
    let spaced = native_path(&root, "src/dir file.rs");
    let unspaced = native_path(&root, "src/dirfile.rs");
    std::fs::write(&spaced, "spaced\n").unwrap();
    std::fs::write(&unspaced, "unspaced\n").unwrap();
    let soft_term = link_term_sized(8, 4, b"src/dir file.rs");
    assert!(soft_term
        .screen()
        .cell(Point::new(Line(0), Column(7)))
        .unwrap()
        .flags()
        .contains(CellFlags::WRAPLINE));
    let soft_top = terminal_link_at(&soft_term, Point::new(Line(0), Column(2)), &[&root]).unwrap();
    let soft_bot = terminal_link_at(&soft_term, Point::new(Line(1), Column(2)), &[&root]).unwrap();
    assert_eq!(soft_top, soft_bot);
    assert_eq!(
        soft_top.target,
        LinkTarget::File {
            path: spaced,
            line: None,
            column: None,
        }
    );
    assert_eq!(soft_top.start, Point::new(Line(0), Column(0)));
    assert_eq!(soft_top.end, Point::new(Line(1), Column(6)));

    // 7. Hard-wrapped mixed final row containing a trailing URL still joins its leading fragment.
    let dir_file = native_path(&root, "src/dir/file.rs");
    std::fs::create_dir_all(dir_file.parent().unwrap()).unwrap();
    std::fs::write(&dir_file, "ok\n").unwrap();
    let mixed_term = link_term_sized(60, 4, b"src/dir/\r\nfile.rs see https://example.com");
    let mixed_top =
        terminal_link_at(&mixed_term, Point::new(Line(0), Column(2)), &[&root]).unwrap();
    let mixed_bot =
        terminal_link_at(&mixed_term, Point::new(Line(1), Column(2)), &[&root]).unwrap();
    assert_eq!(mixed_top, mixed_bot);
    assert_eq!(
        mixed_top.target,
        LinkTarget::File {
            path: dir_file,
            line: None,
            column: None,
        }
    );
    assert_eq!(mixed_top.start, Point::new(Line(0), Column(0)));
    assert_eq!(mixed_top.end, Point::new(Line(1), Column(6)));
    let trailing_url =
        terminal_link_at(&mixed_term, Point::new(Line(1), Column(15)), &[&root]).unwrap();
    assert_eq!(
        trailing_url.target,
        LinkTarget::Url("https://example.com".into())
    );

    // 8. A mixed final logical line with many soft-wrapped prose rows still joins when the
    // included file fragment stays within MAX_WRAP_JOIN_ROWS, hovering either row.
    let long_prose = format!("src/dir/\r\nfile.rs see {}", "word ".repeat(80));
    let long_term = link_term_sized(32, 20, long_prose.as_bytes());
    let long_top = terminal_link_at(&long_term, Point::new(Line(0), Column(2)), &[&root]).unwrap();
    let long_bot = terminal_link_at(&long_term, Point::new(Line(1), Column(2)), &[&root]).unwrap();
    assert_eq!(long_top, long_bot);
    assert_eq!(
        long_top.target,
        LinkTarget::File {
            path: native_path(&root, "src/dir/file.rs"),
            line: None,
            column: None,
        }
    );
    assert_eq!(long_top.start, Point::new(Line(0), Column(0)));
    assert_eq!(long_top.end, Point::new(Line(1), Column(6)));

    // 9. Unicode boundary safety: CJK punctuation before a custom scheme and a combining mark
    // attached to a punctuation cell before a wrapped path never panic.
    let cjk_scheme = link_term(80, "查看，custom://src/lib.rs".as_bytes());
    let custom_col = unicode_width::UnicodeWidthStr::width("查看，custom://");
    assert!(terminal_link_at(
        &cjk_scheme,
        Point::new(Line(0), Column(custom_col)),
        &[&root]
    )
    .is_none());
    let combining_wrap = link_term_sized(40, 6, "  (\u{0301}src/\r\n  lib.rs".as_bytes());
    let _ = terminal_link_at(&combining_wrap, Point::new(Line(0), Column(4)), &[&root]);
    let _ = terminal_link_at(&combining_wrap, Point::new(Line(1), Column(3)), &[&root]);
}

#[test]
fn hard_wrapped_rows_that_must_not_join_stay_separate_and_bounded() {
    let temp = tempfile::tempdir().unwrap();
    let root = test_root(&temp);
    std::fs::create_dir_all(root.join("src")).unwrap();
    let a = native_path(&root, "src/a.rs");
    let b = native_path(&root, "src/b.rs");
    std::fs::write(&a, "a\n").unwrap();
    std::fs::write(&b, "b\n").unwrap();

    // 1. Consecutive file paths stay separate.
    let term = link_term(40, b"src/a.rs\r\nsrc/b.rs");
    let first = terminal_link_at(&term, Point::new(Line(0), Column(2)), &[&root]).unwrap();
    let second = terminal_link_at(&term, Point::new(Line(1), Column(2)), &[&root]).unwrap();
    assert_eq!(
        first.target,
        LinkTarget::File {
            path: a.clone(),
            line: None,
            column: None,
        }
    );
    assert_eq!(first.start.line, Line(0));
    assert_eq!(first.end.line, Line(0));
    assert_eq!(
        second.target,
        LinkTarget::File {
            path: b,
            line: None,
            column: None,
        }
    );
    assert_eq!(second.start.line, Line(1));
    assert_eq!(second.end.line, Line(1));

    // 2. A path row followed by a URL or editor-scheme row does not join.
    for next_url in [
        "https://example.com/foo",
        "file:///tmp/other.rs",
        "vscode://file/tmp/other.rs",
        "vscode-insiders://file/tmp/other.rs",
    ] {
        let bytes = format!("src/a.rs\r\n{next_url}");
        let term = link_term(60, bytes.as_bytes());
        let top = terminal_link_at(&term, Point::new(Line(0), Column(2)), &[&root]).unwrap();
        assert_eq!(top.start.line, Line(0), "{next_url}");
        assert_eq!(top.end.line, Line(0), "{next_url}");
        assert_eq!(
            top.target,
            LinkTarget::File {
                path: a.clone(),
                line: None,
                column: None,
            }
        );
    }

    // Also verify underlined URL hard-wrap join (`link_rows_joined`) stops at editor schemes.
    let underlined_editor = link_term(
        60,
        b"\x1b[4;34mhttps://example.com/path\x1b[0m\r\n  \x1b[4;34mvscode-insiders://file/tmp/x.rs\x1b[0m",
    );
    let top_url =
        terminal_link_at(&underlined_editor, Point::new(Line(0), Column(5)), &[]).unwrap();
    assert_eq!(
        top_url.target,
        LinkTarget::Url("https://example.com/path".into())
    );
    assert_eq!(top_url.end.line, Line(0));

    // 3. A 3,000-row scrollback block of path-like rows stays bounded by MAX_LINK_PATH_PROBES.
    let mut scrollback = String::new();
    for row in 0..3000 {
        if row > 0 {
            scrollback.push_str("\r\n");
        }
        scrollback.push_str(&format!("dir/sub/row_{row}"));
    }
    let big_term = link_term(40, scrollback.as_bytes());
    let mut probes = 0usize;
    let found = terminal_link_at_with_prober(
        &big_term,
        Point::new(Line(0), Column(4)),
        &[&root],
        &mut |_| {
            probes += 1;
            false
        },
    );
    assert!(found.is_none());
    assert!(
        probes > 0 && probes <= MAX_LINK_PATH_PROBES,
        "probes={probes}"
    );
}

#[test]
fn path_allow_list_brackets_closers_prefixes_and_location_suffixes() {
    let temp = tempfile::tempdir().unwrap();
    let root = test_root(&temp);
    for rel in [
        "src/lib.rs",
        "src/call().rs",
        "src/[[[name]]]/page.tsx",
        "docs/计划.md",
        "docs/guide.md",
        "docs/cafe\u{0301}.md",
        "docs/report(1).pdf",
        "qa-link-1451.md",
        "app/users/[id]/page.tsx",
        "app/(marketing)/layout.tsx",
        "src/routes/[...slug]/+page.svelte",
        "app/[[optional]]/page.tsx",
        "a/foo.rs",
        "@types/foo/index.d.ts",
    ] {
        let full = native_path(&root, rel);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, "ok\n").unwrap();
    }
    let cwds = [root.as_path()];

    // CJK punctuation, em dash, CJK filename, decomposed accent, box border.
    let check_single =
        |line: &str, needle: &str, expected_rel: &str, exp_line, exp_col, exp_text: &str| {
            let term = link_term(120, line.as_bytes());
            let byte_col = line.find(needle).unwrap();
            let cell_col = unicode_width::UnicodeWidthStr::width(&line[..byte_col]);
            let link = terminal_link_at(&term, Point::new(Line(0), Column(cell_col)), &cwds)
                .unwrap_or_else(|| panic!("expected link in {line:?} at {needle:?}"));
            assert_eq!(
                link.target,
                LinkTarget::File {
                    path: native_path(&root, expected_rel),
                    line: exp_line,
                    column: exp_col,
                },
                "{line:?}"
            );
            let exp_start =
                unicode_width::UnicodeWidthStr::width(&line[..line.find(exp_text).unwrap()]);
            let exp_width = unicode_width::UnicodeWidthStr::width(exp_text);
            assert_eq!(
                link.start,
                Point::new(Line(0), Column(exp_start)),
                "{line:?}"
            );
            // Wide last char occupies two cells; its anchor cell is the leading cell.
            let last_char_width = exp_text
                .chars()
                .rev()
                .find(|c| unicode_width::UnicodeWidthChar::width(*c).unwrap_or(0) > 0)
                .and_then(unicode_width::UnicodeWidthChar::width)
                .unwrap_or(1);
            assert_eq!(
                link.end,
                Point::new(Line(0), Column(exp_start + exp_width - last_char_width)),
                "{line:?}"
            );
        };

    check_single(
        "查看 src/lib.rs，测试",
        "src/lib.rs",
        "src/lib.rs",
        None,
        None,
        "src/lib.rs",
    );
    check_single(
        "src/lib.rs—see",
        "src/lib.rs",
        "src/lib.rs",
        None,
        None,
        "src/lib.rs",
    );
    check_single(
        "查看 docs/计划.md。",
        "docs/",
        "docs/计划.md",
        None,
        None,
        "docs/计划.md",
    );
    check_single(
        "│ src/lib.rs │",
        "src/lib.rs",
        "src/lib.rs",
        None,
        None,
        "src/lib.rs",
    );
    check_single(
        "docs/cafe\u{0301}.md",
        "docs/",
        "docs/cafe\u{0301}.md",
        None,
        None,
        "docs/cafe\u{0301}.md",
    );

    // Parentheses, brackets, wrappers, and `(LINE)` / `(LINE, COL)` suffixes.
    check_single(
        "app/users/[id]/page.tsx",
        "users",
        "app/users/[id]/page.tsx",
        None,
        None,
        "app/users/[id]/page.tsx",
    );
    check_single(
        "app/(marketing)/layout.tsx",
        "marketing",
        "app/(marketing)/layout.tsx",
        None,
        None,
        "app/(marketing)/layout.tsx",
    );
    check_single(
        "src/routes/[...slug]/+page.svelte",
        "slug",
        "src/routes/[...slug]/+page.svelte",
        None,
        None,
        "src/routes/[...slug]/+page.svelte",
    );
    check_single(
        "app/[[optional]]/page.tsx",
        "optional",
        "app/[[optional]]/page.tsx",
        None,
        None,
        "app/[[optional]]/page.tsx",
    );
    check_single(
        "src/call().rs",
        "call",
        "src/call().rs",
        None,
        None,
        "src/call().rs",
    );
    check_single(
        "src/[[[name]]]/page.tsx",
        "name",
        "src/[[[name]]]/page.tsx",
        None,
        None,
        "src/[[[name]]]/page.tsx",
    );
    check_single(
        "Read(src/call().rs:5)",
        "call",
        "src/call().rs",
        Some(5),
        None,
        "src/call().rs:5",
    );
    check_single(
        "foo/(src/lib.rs",
        "lib.rs",
        "src/lib.rs",
        None,
        None,
        "src/lib.rs",
    );
    // Repeated unclosed openers stay linear and still recover the inner path.
    let repeated_unclosed = format!("{}src/lib.rs", "a/(".repeat(4000));
    let scanned = super::scan_plain_file_candidates(&repeated_unclosed);
    assert_eq!(scanned.last().map(|c| c.path.as_str()), Some("src/lib.rs"));
    check_single(
        "docs/report(1).pdf(42)",
        "report",
        "docs/report(1).pdf",
        Some(42),
        None,
        "docs/report(1).pdf(42)",
    );
    check_single(
        "docs/report(1).pdf.",
        "report",
        "docs/report(1).pdf",
        None,
        None,
        "docs/report(1).pdf",
    );
    check_single(
        "(qa-link-1451.md)",
        "qa-link",
        "qa-link-1451.md",
        None,
        None,
        "qa-link-1451.md",
    );
    check_single(
        "Read(src/lib.rs)",
        "src/lib.rs",
        "src/lib.rs",
        None,
        None,
        "src/lib.rs",
    );
    check_single(
        "Read(app/(marketing)/layout.tsx)",
        "marketing",
        "app/(marketing)/layout.tsx",
        None,
        None,
        "app/(marketing)/layout.tsx",
    );
    check_single(
        "[label](docs/guide.md)",
        "docs/guide.md",
        "docs/guide.md",
        None,
        None,
        "docs/guide.md",
    );
    check_single(
        "src/lib.rs(12, 4)",
        "src/lib.rs",
        "src/lib.rs",
        Some(12),
        Some(4),
        "src/lib.rs(12, 4)",
    );
    check_single(
        "src/lib.rs(42)",
        "src/lib.rs",
        "src/lib.rs",
        Some(42),
        None,
        "src/lib.rs(42)",
    );

    // Location ranges and `#L` anchors: open at start line/col, highlight whole suffix.
    check_single(
        "src/lib.rs:10-25",
        "src/lib.rs",
        "src/lib.rs",
        Some(10),
        None,
        "src/lib.rs:10-25",
    );
    check_single(
        "src/lib.rs:10–25",
        "src/lib.rs",
        "src/lib.rs",
        Some(10),
        None,
        "src/lib.rs:10–25",
    );
    check_single(
        "src/lib.rs:10:5-20",
        "src/lib.rs",
        "src/lib.rs",
        Some(10),
        Some(5),
        "src/lib.rs:10:5-20",
    );
    check_single(
        "src/lib.rs:10:5-14:9",
        "src/lib.rs",
        "src/lib.rs",
        Some(10),
        Some(5),
        "src/lib.rs:10:5-14:9",
    );
    check_single(
        "src/lib.rs#L10C3",
        "src/lib.rs",
        "src/lib.rs",
        Some(10),
        Some(3),
        "src/lib.rs#L10C3",
    );
    check_single(
        "src/lib.rs#L10-L20",
        "src/lib.rs",
        "src/lib.rs",
        Some(10),
        None,
        "src/lib.rs#L10-L20",
    );
    check_single(
        "src/lib.rs#L10C3-L14C9",
        "src/lib.rs",
        "src/lib.rs",
        Some(10),
        Some(3),
        "src/lib.rs#L10C3-L14C9",
    );

    // Prefix fallbacks (`@`, `a/`, `b/`): highlight leaves out stripped prefix; real paths resolve verbatim.
    check_single(
        "@src/lib.rs:3-5",
        "src/lib.rs",
        "src/lib.rs",
        Some(3),
        None,
        "src/lib.rs:3-5",
    );
    check_single(
        "a/src/lib.rs",
        "src/lib.rs",
        "src/lib.rs",
        None,
        None,
        "src/lib.rs",
    );
    check_single(
        "b/src/lib.rs",
        "src/lib.rs",
        "src/lib.rs",
        None,
        None,
        "src/lib.rs",
    );
    // Hovering directly on the stripped prefix does not link.
    let at_term = link_term(40, b"@src/lib.rs");
    assert!(terminal_link_at(&at_term, Point::new(Line(0), Column(0)), &cwds).is_none());
    let diff_term = link_term(40, b"b/src/lib.rs");
    assert!(terminal_link_at(&diff_term, Point::new(Line(0), Column(0)), &cwds).is_none());
    // Real paths starting with `a/` or `@` resolve verbatim (including prefix in highlight).
    check_single("a/foo.rs", "a/foo.rs", "a/foo.rs", None, None, "a/foo.rs");
    check_single(
        "@types/foo/index.d.ts",
        "@types",
        "@types/foo/index.d.ts",
        None,
        None,
        "@types/foo/index.d.ts",
    );

    // Cross-platform Windows git-diff prefix stripping helper.
    assert_eq!(
        stripped_prefix_candidates(r"a\src\lib.rs", true),
        vec![(2, r"src\lib.rs")]
    );
    assert_eq!(
        stripped_prefix_candidates(r"b\src\lib.rs", true),
        vec![(2, r"src\lib.rs")]
    );
    assert_eq!(stripped_prefix_candidates(r"a\src\lib.rs", false), vec![]);
}

#[test]
fn spaced_paths_grow_rooted_and_relative_without_joining_adjacent_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = test_root(&temp);
    let project_file = native_path(&root, "My Project/readme.md");
    std::fs::create_dir_all(project_file.parent().unwrap()).unwrap();
    std::fs::write(&project_file, "# readme\n").unwrap();
    let plan_file = native_path(&root, "docs/my plan.md");
    std::fs::create_dir_all(plan_file.parent().unwrap()).unwrap();
    std::fs::write(&plan_file, "# plan\n").unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let a = native_path(&root, "src/a.rs");
    let b = native_path(&root, "src/b.rs");
    std::fs::write(&a, "a\n").unwrap();
    std::fs::write(&b, "b\n").unwrap();
    let cwds = [root.as_path()];

    // Rooted spaced path with suffix, hovered on start, middle word, and filename.
    let abs_text = format!("open {}:2 now", project_file.display());
    let abs_term = link_term(200, abs_text.as_bytes());
    let start_col = "open ".len();
    let mid_col = abs_text.find("Project").unwrap();
    let end_col = abs_text.find("readme.md").unwrap();
    let expected_end = abs_text.rfind(":2").unwrap() + 1;
    for col in [start_col, mid_col, end_col] {
        let link = terminal_link_at(&abs_term, Point::new(Line(0), Column(col)), &cwds).unwrap();
        assert_eq!(
            link.target,
            LinkTarget::File {
                path: project_file.clone(),
                line: Some(2),
                column: None,
            }
        );
        assert_eq!(link.start, Point::new(Line(0), Column(start_col)));
        assert_eq!(link.end, Point::new(Line(0), Column(expected_end)));
    }

    // Quoted relative spaced path excludes quotes from highlight.
    let rel_term = link_term(80, b"read \"docs/my plan.md\" here");
    for col in [6, 15] {
        let link = terminal_link_at(&rel_term, Point::new(Line(0), Column(col)), &cwds).unwrap();
        assert_eq!(
            link.target,
            LinkTarget::File {
                path: plan_file.clone(),
                line: None,
                column: None,
            }
        );
        assert_eq!(link.start, Point::new(Line(0), Column(6)));
        assert_eq!(link.end, Point::new(Line(0), Column(20)));
    }

    // Adjacent relative paths stay separate links.
    let pair_term = link_term(80, b"src/a.rs src/b.rs");
    let first = terminal_link_at(&pair_term, Point::new(Line(0), Column(2)), &cwds).unwrap();
    let second = terminal_link_at(&pair_term, Point::new(Line(0), Column(12)), &cwds).unwrap();
    assert_eq!(
        first.target,
        LinkTarget::File {
            path: a,
            line: None,
            column: None,
        }
    );
    assert_eq!(first.end, Point::new(Line(0), Column(7)));
    assert_eq!(
        second.target,
        LinkTarget::File {
            path: b,
            line: None,
            column: None,
        }
    );
    assert_eq!(second.start, Point::new(Line(0), Column(9)));
}

#[test]
fn editor_scheme_links_and_osc8_bare_paths_resolve_and_reject_malformed_uris() {
    let file_target = |path: &str, line: Option<u32>, column: Option<u32>| LinkTarget::File {
        path: PathBuf::from(path),
        line,
        column,
    };

    // All five editor schemes, case-insensitively, on POSIX and Windows helpers.
    for scheme in ["vscode", "VSCODE-INSIDERS", "Cursor", "windsurf", "ZED"] {
        assert_eq!(
            file_target_from_editor_uri_with_platform(
                &format!("{scheme}://file/tmp/my%20file.rs:10:5-14:9"),
                false,
            ),
            Some(file_target("/tmp/my file.rs", Some(10), Some(5))),
            "posix {scheme}"
        );
        assert_eq!(
            file_target_from_editor_uri_with_platform(
                &format!("{scheme}://FILE/C:/Users/me/app.rs#L10C3-L14C9"),
                true,
            ),
            Some(file_target(r"C:\Users\me\app.rs", Some(10), Some(3))),
            "windows {scheme}"
        );
    }
    // New location suffixes in `file://` URIs too.
    assert_eq!(
        file_target_from_uri_with_platform("file:///tmp/a.rs:10-25", false, None),
        Some(file_target("/tmp/a.rs", Some(10), None))
    );
    assert_eq!(
        file_target_from_uri_with_platform("file:///tmp/a.rs#L10C3-L14C9", false, None),
        Some(file_target("/tmp/a.rs", Some(10), Some(3)))
    );

    // Malformed editor file URIs are rejected on both POSIX and Windows.
    for malformed in [
        "vscode://file/",
        "vscode://file",
        "vscode://file//host/share/a.rs",
        r"vscode://file/\\host\share\a.rs",
        "cursor://file/tmp/bad%00path",
        "zed://file/tmp/%FF",
    ] {
        assert_eq!(
            file_target_from_editor_uri_with_platform(malformed, false),
            None,
            "posix {malformed}"
        );
        assert_eq!(
            file_target_from_editor_uri_with_platform(malformed, true),
            None,
            "windows {malformed}"
        );
    }
    // POSIX single-letter filenames with line suffixes keep working; Windows bare-drive forms stay rejected.
    assert_eq!(
        file_target_from_uri_with_platform("file:///a:42", false, None),
        Some(file_target("/a", Some(42), None))
    );
    assert_eq!(
        file_target_from_editor_uri_with_platform("vscode://file/a:42", false),
        Some(file_target("/a", Some(42), None))
    );
    for win_malformed in [
        "vscode://file/C:/",
        "vscode://file/C:",
        "vscode://file/C:relative.rs",
        "vscode://file/C:12",
    ] {
        assert_eq!(
            file_target_from_editor_uri_with_platform(win_malformed, true),
            None,
            "windows {win_malformed}"
        );
    }
    assert_eq!(
        file_target_from_uri_with_platform("file:///C:12", true, None),
        None
    );

    // Table-driven end-to-end transport checks across all five editor schemes in both OSC 8 and
    // plain text.
    for scheme in ["vscode", "VSCODE-INSIDERS", "Cursor", "windsurf", "ZED"] {
        let plain = format!("{scheme}://file/tmp/b.rs:7:2");
        let output = format!("\x1b]8;;{scheme}://file/tmp/a.rs:3\x1b\\open\x1b]8;;\x1b\\ {plain}");
        let term = link_term(120, output.as_bytes());
        let osc = terminal_link_at(&term, Point::new(Line(0), Column(1)), &[]).unwrap();
        assert_eq!(
            osc.target,
            file_target("/tmp/a.rs", Some(3), None),
            "osc8 {scheme}"
        );
        assert_eq!(osc.start, Point::new(Line(0), Column(0)));
        assert_eq!(osc.end, Point::new(Line(0), Column(3)));

        let plain_start = "open ".len();
        let plain_end = plain_start + plain.len() - 1;
        for col in [plain_start + 2, plain_end] {
            let link = terminal_link_at(&term, Point::new(Line(0), Column(col)), &[]).unwrap();
            assert_eq!(
                link.target,
                file_target("/tmp/b.rs", Some(7), Some(2)),
                "plain {scheme} col {col}"
            );
            assert_eq!(link.start, Point::new(Line(0), Column(plain_start)));
            assert_eq!(link.end, Point::new(Line(0), Column(plain_end)));
        }
    }

    // Plain-text `file://` and editor URIs preserve `(LINE)` / `(LINE, COL)` suffixes and
    // balanced `(...)` / `[...]` route segments, while HTTP(S) keeps stopping before `(`.
    let uri_line = "see file:///tmp/x.rs(42) and cursor://file/tmp/x.rs(12, 4)\r\n\
         (vscode://file/tmp/app/(marketing)/layout.tsx:3). (file:///tmp/app/[id]/page.tsx(7)).\r\n\
         https://example.com/path(42)";
    let uri_term = link_term(120, uri_line.as_bytes());
    let first_start = "see ".len();
    let first_end = first_start + "file:///tmp/x.rs(42)".len() - 1;
    for col in [first_start + 4, first_end - 1] {
        let link = terminal_link_at(&uri_term, Point::new(Line(0), Column(col)), &[]).unwrap();
        assert_eq!(link.target, file_target("/tmp/x.rs", Some(42), None));
        assert_eq!(link.start, Point::new(Line(0), Column(first_start)));
        assert_eq!(link.end, Point::new(Line(0), Column(first_end)));
    }
    let second_start = "see file:///tmp/x.rs(42) and ".len();
    let second_end = second_start + "cursor://file/tmp/x.rs(12, 4)".len() - 1;
    for col in [second_start + 8, second_end - 1] {
        let link = terminal_link_at(&uri_term, Point::new(Line(0), Column(col)), &[]).unwrap();
        assert_eq!(link.target, file_target("/tmp/x.rs", Some(12), Some(4)));
        assert_eq!(link.start, Point::new(Line(0), Column(second_start)));
        assert_eq!(link.end, Point::new(Line(0), Column(second_end)));
    }
    let route_start = 1;
    let route_end = route_start + "vscode://file/tmp/app/(marketing)/layout.tsx:3".len() - 1;
    for col in [route_start + 15, route_start + 26, route_end] {
        let link = terminal_link_at(&uri_term, Point::new(Line(1), Column(col)), &[]).unwrap();
        assert_eq!(
            link.target,
            file_target("/tmp/app/(marketing)/layout.tsx", Some(3), None)
        );
        assert_eq!(link.start, Point::new(Line(1), Column(route_start)));
        assert_eq!(link.end, Point::new(Line(1), Column(route_end)));
    }
    let bracket_start = "(vscode://file/tmp/app/(marketing)/layout.tsx:3). (".len();
    let bracket_end = bracket_start + "file:///tmp/app/[id]/page.tsx(7)".len() - 1;
    for col in [bracket_start + 18, bracket_end - 1] {
        let link = terminal_link_at(&uri_term, Point::new(Line(1), Column(col)), &[]).unwrap();
        assert_eq!(
            link.target,
            file_target("/tmp/app/[id]/page.tsx", Some(7), None)
        );
        assert_eq!(link.start, Point::new(Line(1), Column(bracket_start)));
        assert_eq!(link.end, Point::new(Line(1), Column(bracket_end)));
    }
    let http_link = terminal_link_at(&uri_term, Point::new(Line(2), Column(10)), &[]).unwrap();
    assert_eq!(
        http_link.target,
        LinkTarget::Url("https://example.com/path".into())
    );

    // An outer HTTP(S) or non-file editor URL owns its entire range even when it carries a nested
    // local `file://` or `<editor>://file/` URI in its query string.
    for outer_url in [
        "https://example.org/open?target=file:///tmp/report.md",
        "cursor://settings/open?target=vscode://file/tmp/report.md",
    ] {
        let nested_term = link_term(120, outer_url.as_bytes());
        let first_half =
            terminal_link_at(&nested_term, Point::new(Line(0), Column(8)), &[]).unwrap();
        let inner_col = outer_url.rfind("://").unwrap() + 4;
        let second_half =
            terminal_link_at(&nested_term, Point::new(Line(0), Column(inner_col)), &[]).unwrap();
        assert_eq!(first_half, second_half, "{outer_url}");
        assert_eq!(
            first_half.target,
            LinkTarget::Url(outer_url.to_owned()),
            "{outer_url}"
        );
        assert_eq!(first_half.start, Point::new(Line(0), Column(0)));
        assert_eq!(
            first_half.end,
            Point::new(Line(0), Column(outer_url.len() - 1))
        );
    }

    // End-to-end in terminal: non-file editor URLs, malformed OSC 8 rejection without label
    // fallback, opaque external OSC 8 schemes, ambiguous `Makefile:12`, and OSC 8 bare paths.
    let temp = tempfile::tempdir().unwrap();
    let root = test_root(&temp);
    std::fs::create_dir_all(root.join("docs")).unwrap();
    let guide = native_path(&root, "docs/guide.md");
    let makefile = root.join("Makefile");
    let report = root.join("report.html");
    std::fs::write(&guide, "# guide\n").unwrap();
    std::fs::write(&makefile, "all:\n").unwrap();
    std::fs::write(&report, "<html/>\n").unwrap();
    #[cfg(not(windows))]
    let single_a = {
        let path = root.join("a");
        std::fs::write(&path, "a\n").unwrap();
        path
    };
    let cwds = [root.as_path()];

    let output = "\
        vscode://vscode-remote/ssh-remote+box/tmp/c.rs \
        \x1b]8;;cursor://settings/editor\x1b\\prefs\x1b]8;;\x1b\\\r\n\
        \x1b]8;;vscode://file/\x1b\\docs/guide.md\x1b]8;;\x1b\\ \
        \x1b]8;;docs/guide.md:4\x1b\\guide\x1b]8;;\x1b\\ \
        \x1b]8;;docs/missing.md\x1b\\docs/guide.md\x1b]8;;\x1b\\\r\n\
        \x1b]8;;urn:123\x1b\\urn\x1b]8;;\x1b\\ \
        \x1b]8;;tel:12345\x1b\\tel\x1b]8;;\x1b\\ \
        \x1b]8;;mailto:user@example.com\x1b\\mail\x1b]8;;\x1b\\ \
        \x1b]8;;scheme:/path\x1b\\opaque\x1b]8;;\x1b\\ \
        \x1b]8;;Makefile:12\x1b\\make\x1b]8;;\x1b\\ \
        \x1b]8;;report.html:12\x1b\\rep\x1b]8;;\x1b\\ \
        \x1b]8;;a:42\x1b\\short\x1b]8;;\x1b\\";
    let term = link_term(120, output.as_bytes());

    let remote_url = terminal_link_at(&term, Point::new(Line(0), Column(5)), &cwds).unwrap();
    assert_eq!(
        remote_url.target,
        LinkTarget::Url("vscode://vscode-remote/ssh-remote+box/tmp/c.rs".into())
    );
    let prefs = terminal_link_at(&term, Point::new(Line(0), Column(48)), &cwds).unwrap();
    assert_eq!(
        prefs.target,
        LinkTarget::Url("cursor://settings/editor".into())
    );

    assert!(
        terminal_link_at(&term, Point::new(Line(1), Column(2)), &cwds).is_none(),
        "malformed OSC 8 vscode://file/ must reject without falling back to label"
    );

    let bare_osc = terminal_link_at(&term, Point::new(Line(1), Column(15)), &cwds).unwrap();
    assert_eq!(
        bare_osc.target,
        LinkTarget::File {
            path: guide,
            line: Some(4),
            column: None,
        }
    );

    assert!(
        terminal_link_at(&term, Point::new(Line(1), Column(22)), &cwds).is_none(),
        "missing OSC 8 bare path must not link or fall back to label"
    );

    // Opaque external schemes stay LinkTarget::Url.
    assert_eq!(
        terminal_link_at(&term, Point::new(Line(2), Column(1)), &cwds)
            .unwrap()
            .target,
        LinkTarget::Url("urn:123".into())
    );
    assert_eq!(
        terminal_link_at(&term, Point::new(Line(2), Column(5)), &cwds)
            .unwrap()
            .target,
        LinkTarget::Url("tel:12345".into())
    );
    assert_eq!(
        terminal_link_at(&term, Point::new(Line(2), Column(9)), &cwds)
            .unwrap()
            .target,
        LinkTarget::Url("mailto:user@example.com".into())
    );
    assert_eq!(
        terminal_link_at(&term, Point::new(Line(2), Column(15)), &cwds)
            .unwrap()
            .target,
        LinkTarget::Url("scheme:/path".into())
    );
    // Ambiguous `Makefile:12` resolves to a file when present, and falls back to Url when absent.
    assert_eq!(
        terminal_link_at(&term, Point::new(Line(2), Column(22)), &cwds)
            .unwrap()
            .target,
        LinkTarget::File {
            path: makefile,
            line: Some(12),
            column: None,
        }
    );
    assert_eq!(
        terminal_link_at(&term, Point::new(Line(2), Column(22)), &[])
            .unwrap()
            .target,
        LinkTarget::Url("Makefile:12".into())
    );
    // Dotted `report.html:12` is a bare path: resolves when present, rejects when missing.
    assert_eq!(
        terminal_link_at(&term, Point::new(Line(2), Column(27)), &cwds)
            .unwrap()
            .target,
        LinkTarget::File {
            path: report,
            line: Some(12),
            column: None,
        }
    );
    assert!(terminal_link_at(&term, Point::new(Line(2), Column(27)), &[]).is_none());
    #[cfg(not(windows))]
    assert_eq!(
        terminal_link_at(&term, Point::new(Line(2), Column(31)), &cwds)
            .unwrap()
            .target,
        LinkTarget::File {
            path: single_a,
            line: Some(42),
            column: None,
        }
    );
    #[cfg(windows)]
    assert!(terminal_link_at(&term, Point::new(Line(2), Column(31)), &cwds).is_none());
}

#[test]
fn link_probe_budget_caps_lookups_and_extra_root_resolves_via_link_at_with() {
    let temp = tempfile::tempdir().unwrap();
    let spawn_dir = temp.path().join("spawn");
    let extra_dir = temp.path().join("extra");
    std::fs::create_dir_all(&spawn_dir).unwrap();
    std::fs::create_dir_all(extra_dir.join("src")).unwrap();
    let only_in_extra = native_path(&extra_dir, "src/only_extra.rs");
    std::fs::write(&only_in_extra, "fn extra() {}\n").unwrap();

    // 1. Lookup stops at MAX_LINK_PATH_PROBES even when many hard-wrap & spaced candidates fail
    // across 3 distinct roots, and probes in order: single-line, hard-wrap (longest first),
    // spaced readings, then prefix-stripped forms.
    let many_words = format!(
        "{}\r\n{}\r\n{}\r\n{}\r\n{}\r\n{}\r\n{}\r\n{}",
        "@miss/a",
        "miss/b",
        "miss/c",
        "miss/d",
        "miss/e",
        "miss/f",
        "miss/g",
        "./miss/h w1 w2 w3 w4 w5 w6 w7 w8 w9 w10"
    );
    let term = link_term_sized(80, 12, many_words.as_bytes());
    let third_dir = temp.path().join("third");
    let cwds = [
        spawn_dir.as_path(),
        extra_dir.as_path(),
        third_dir.as_path(),
    ];
    let mut probed = Vec::new();
    let res =
        terminal_link_at_with_prober(&term, Point::new(Line(7), Column(2)), &cwds, &mut |path| {
            probed.push(path.to_path_buf());
            false
        });
    assert!(res.is_none());
    assert_eq!(probed.len(), MAX_LINK_PATH_PROBES);
    // Single-line match is probed first (across all 3 roots), followed by longest hard-wrap join,
    // followed by spaced readings before hitting the cap.
    assert_eq!(probed[0], native_path(&spawn_dir, "miss/h"));
    assert_eq!(probed[1], native_path(&extra_dir, "miss/h"));
    assert_eq!(probed[2], native_path(&third_dir, "miss/h"));
    assert_eq!(
        probed[3],
        candidate_paths(
            "@miss/amiss/bmiss/cmiss/dmiss/emiss/fmiss/g./miss/h",
            &[spawn_dir.as_path()],
            None,
            cfg!(windows),
        )[0],
        "expected longest hard-wrap join next"
    );

    // Also verify all 4 stages in order (1: single-line, 2: hard-wrap, 3: spaced, 4: prefix-stripped)
    // within one lookup on a single root.
    let order_term = link_term_sized(80, 8, b"@dir/part1\r\n@dir/part2 word.md");
    let mut order = Vec::new();
    let _ = terminal_link_at_with_prober(
        &order_term,
        Point::new(Line(1), Column(3)),
        &[spawn_dir.as_path()],
        &mut |path| {
            order.push(path.to_path_buf());
            false
        },
    );
    assert_eq!(
        order,
        [
            "@dir/part2",
            "@dir/part1@dir/part2",
            "@dir/part2 word.md",
            "dir/part2",
            "dir/part1@dir/part2",
            "dir/part2 word.md",
        ]
        .map(|rel| native_path(&spawn_dir, rel))
        .to_vec()
    );

    // 2. `TerminalMirror::link_at_with` resolves a path that exists only under `extra_root`.
    let mirror = snapshot_mirror(1, &[b"src/only_extra.rs:3"]);
    assert!(
        mirror.link_at(Point::new(Line(0), Column(4))).is_none(),
        "default link_at has no extra_root"
    );
    let linked = mirror
        .link_at_with(
            Point::new(Line(0), Column(4)),
            &FileLinkContext {
                extra_root: Some(extra_dir),
            },
        )
        .expect("resolved under extra_root");
    assert_eq!(
        linked.target,
        LinkTarget::File {
            path: only_in_extra,
            line: Some(3),
            column: None,
        }
    );
}

#[test]
#[ignore = "manual #1508 Open Question 3 25-session scale benchmark"]
fn benchmark_25_open_sessions_memory_threads_and_latency() {
    use runner_core::protocol::terminal::{
        TerminalAttachment, TerminalSnapshot, TerminalSubscription,
    };
    use runner_core::protocol::{ClientError, EventSubscription, Request, Response, Transport};

    struct BenchHost;
    impl TerminalHost for BenchHost {
        fn write_input(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn write_reply(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn report_input_state(&self, _: &str, _: InputObservation) {}
        fn set_live_title(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn resize(&self, _: &str, _: u16, _: u16) -> Result<()> {
            Ok(())
        }
        fn is_local_host(&self, _: &str) -> bool {
            true
        }
        fn clipboard_store(&self, _: &str, _: ClipboardTarget, _: &str) {}
    }

    type SharedFrameSink = Arc<dyn Fn(Option<TerminalFrame>) -> bool + Send + Sync>;

    struct BenchQueue {
        sink: Mutex<Option<SharedFrameSink>>,
    }
    struct BenchFrames {
        queue: Arc<BenchQueue>,
    }
    impl TerminalSubscription for BenchFrames {
        fn recv(&mut self) -> std::result::Result<TerminalFrame, ClientError> {
            Err(ClientError::msg("push-only bench frames"))
        }
        fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
            let queue = Arc::clone(&self.queue);
            Arc::new(move || {
                if let Some(sink) = queue.sink.lock().unwrap().take() {
                    let _ = sink(None);
                }
            })
        }
        fn install_sink(&mut self, sink: runner_core::protocol::terminal::FrameSink) -> bool {
            *self.queue.sink.lock().unwrap() = Some(Arc::from(sink));
            true
        }
    }

    struct BenchTransport {
        snapshot: Mutex<Option<TerminalSnapshot>>,
        frames: Mutex<Option<BenchFrames>>,
    }
    impl Transport for BenchTransport {
        fn call(&self, _: Request) -> std::result::Result<Response, ClientError> {
            Ok(Response::terminal_metadata(Ok(TerminalMetadata::default())))
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            unreachable!()
        }
        fn attach(&self, _: &str) -> std::result::Result<TerminalAttachment, ClientError> {
            Ok(TerminalAttachment {
                subscriber_id: 1,
                snapshot: self.snapshot.lock().unwrap().take().unwrap(),
                frames: Box::new(self.frames.lock().unwrap().take().unwrap()),
            })
        }
    }

    fn process_rss_kib_and_threads() -> (u64, u64) {
        let pid = std::process::id().to_string();
        let rss = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &pid])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0);
        let threads = std::process::Command::new("ps")
            .args(["-M", "-p", &pid])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.lines().count().saturating_sub(1) as u64)
            .unwrap_or(0);
        (rss, threads)
    }

    const SESSIONS: usize = 25;
    const COLS: u16 = 120;
    const ROWS: u16 = 40;
    const HISTORY_LINES: usize = 2_000;

    let (base_rss, base_threads) = process_rss_kib_and_threads();
    let host: Arc<dyn TerminalHost> = Arc::new(BenchHost);
    let mut models = Vec::with_capacity(SESSIONS);
    let mut mirrors = Vec::with_capacity(SESSIONS);
    let mut queues = Vec::with_capacity(SESSIONS);

    for i in 0..SESSIONS {
        let id = format!("bench-{i:02}");
        let model = TerminalModel::new(
            Arc::clone(&host),
            id.clone(),
            COLS,
            ROWS,
            ModelOptions::default(),
        )
        .unwrap();
        let initial_snap = model.snapshot(0);
        let queue = Arc::new(BenchQueue {
            sink: Mutex::new(None),
        });
        let transport = Arc::new(BenchTransport {
            snapshot: Mutex::new(Some(initial_snap)),
            frames: Mutex::new(Some(BenchFrames {
                queue: Arc::clone(&queue),
            })),
        });
        let mirror =
            TerminalMirror::attach(DaemonClient::new(transport), id, Arc::new(|| {})).unwrap();
        models.push(model);
        mirrors.push(mirror);
        queues.push(queue);
    }
    let (idle_rss, idle_threads) = process_rss_kib_and_threads();

    // Feed 2,000 realistic colored lines into each model & mirror (hidden first).
    let mut payload = Vec::with_capacity(HISTORY_LINES * 96);
    for line in 0..HISTORY_LINES {
        payload.extend_from_slice(
            format!(
                "\x1b[32m[{line:04}]\x1b[0m \x1b[1mrunner\x1b[0m src/terminal/model.rs:{line} \x1b[36mstatus=working\x1b[0m\r\n"
            )
            .as_bytes(),
        );
    }
    for (model, mirror) in models.iter().zip(mirrors.iter()) {
        model.feed_output(1, &payload);
        mirror.test_feed(1, &payload);
    }
    let (populated_rss, populated_threads) = process_rss_kib_and_threads();

    // 1. Hidden mirrors (`viewers == 0`): lazy viewport publishing skips RenderState::update.
    let mut model_feed_us = Vec::with_capacity(SESSIONS * 20);
    let mut hidden_mirror_feed_us = Vec::with_capacity(SESSIONS * 20);
    for round in 0..20u64 {
        let seq = 2 + round;
        let line = format!("\x1b[33mhidden {round:02}\x1b[0m hello from hidden benchmark\r\n");
        for (model, mirror) in models.iter().zip(mirrors.iter()) {
            let t0 = Instant::now();
            model.feed_output(seq, line.as_bytes());
            model_feed_us.push(t0.elapsed().as_secs_f64() * 1e6);

            let t1 = Instant::now();
            mirror.test_feed(seq, line.as_bytes());
            hidden_mirror_feed_us.push(t1.elapsed().as_secs_f64() * 1e6);
        }
    }

    // 2. Visible mirrors (`viewers == 1`): publish ViewportSnapshot per output chunk + published-slot Arc::clone reads.
    let views: Vec<_> = mirrors.iter().map(|m| m.view()).collect();
    let mut visible_mirror_feed_us = Vec::with_capacity(SESSIONS * 20);
    let mut paint_read_ns = Vec::with_capacity(SESSIONS * 40);
    for round in 0..20u64 {
        let seq = 30 + round;
        let line = format!("\x1b[33mvisible {round:02}\x1b[0m hello from visible benchmark\r\n");
        for mirror in &mirrors {
            let t1 = Instant::now();
            mirror.test_feed(seq, line.as_bytes());
            visible_mirror_feed_us.push(t1.elapsed().as_secs_f64() * 1e6);

            for _ in 0..2 {
                let t2 = Instant::now();
                let snap = mirror.viewport_snapshot();
                std::hint::black_box(snap.cols());
                paint_read_ns.push(t2.elapsed().as_secs_f64() * 1e9);
            }
        }
    }
    drop(views);

    let mut encode_us = Vec::with_capacity(SESSIONS);
    let mut decode_us = Vec::with_capacity(SESSIONS);
    let mut snap_sizes = Vec::with_capacity(SESSIONS);
    for model in &models {
        let t0 = Instant::now();
        let snap = model.snapshot(100);
        encode_us.push(t0.elapsed().as_secs_f64() * 1e6);
        snap_sizes.push(snap.bytes.len());

        let t1 = Instant::now();
        let decoded = crate::replay::Terminal::from_snapshot(&snap).unwrap();
        decode_us.push(t1.elapsed().as_secs_f64() * 1e6);
        std::hint::black_box(decoded.history_size());
    }

    let pct = |xs: &mut Vec<f64>, p: f64| {
        xs.sort_by(f64::total_cmp);
        let idx = ((xs.len() as f64 - 1.0) * p).round() as usize;
        xs[idx]
    };

    drop(mirrors);
    drop(queues);
    for model in &models {
        for worker in model.stop_workers() {
            worker.join().unwrap();
        }
    }
    drop(models);
    let (_after_rss, after_threads) = process_rss_kib_and_threads();

    println!(
        "bench_25_sessions: base_rss_kib={base_rss} base_threads={base_threads} \
         idle_rss_kib={idle_rss} idle_delta_mib={:.2} idle_threads={idle_threads} \
         populated_rss_kib={populated_rss} populated_delta_mib={:.2} populated_threads={populated_threads} \
         after_shutdown_threads={after_threads} \
         model_feed_p50_us={:.1} model_feed_p99_us={:.1} \
         hidden_mirror_feed_p50_us={:.1} hidden_mirror_feed_p99_us={:.1} \
         visible_mirror_feed_p50_us={:.1} visible_mirror_feed_p99_us={:.1} \
         paint_read_p50_ns={:.0} paint_read_p99_ns={:.0} \
         snap_bytes_avg={} snap_encode_p50_us={:.1} snap_decode_p50_us={:.1}",
        (idle_rss.saturating_sub(base_rss)) as f64 / 1024.0,
        (populated_rss.saturating_sub(base_rss)) as f64 / 1024.0,
        pct(&mut model_feed_us, 0.50),
        pct(&mut model_feed_us, 0.99),
        pct(&mut hidden_mirror_feed_us, 0.50),
        pct(&mut hidden_mirror_feed_us, 0.99),
        pct(&mut visible_mirror_feed_us, 0.50),
        pct(&mut visible_mirror_feed_us, 0.99),
        pct(&mut paint_read_ns, 0.50),
        pct(&mut paint_read_ns, 0.99),
        snap_sizes.iter().sum::<usize>() / snap_sizes.len().max(1),
        pct(&mut encode_us, 0.50),
        pct(&mut decode_us, 0.50),
    );
}

#[test]
fn selection_copies_tabs_as_spaces_including_across_soft_wraps() {
    use runner_core::protocol::terminal::{TerminalAttachment, TerminalSubscription};
    use runner_core::protocol::{EventSubscription, Request, Response, Transport};
    struct Frames {
        tx: mpsc::Sender<TerminalFrame>,
        rx: mpsc::Receiver<TerminalFrame>,
        cancelled: Arc<AtomicBool>,
    }
    impl TerminalSubscription for Frames {
        fn recv(&mut self) -> std::result::Result<TerminalFrame, ClientError> {
            let frame = self.rx.recv().map_err(|_| ClientError::msg("closed"))?;
            if self.cancelled.load(Ordering::Acquire) {
                return Err(ClientError::msg("cancelled"));
            }
            Ok(frame)
        }
        fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
            let tx = self.tx.clone();
            let cancelled = Arc::clone(&self.cancelled);
            Arc::new(move || {
                cancelled.store(true, Ordering::Release);
                let _ = tx.send(TerminalFrame::Resync);
            })
        }
    }
    struct StaticHost {
        cols: u16,
        rows: u16,
        frames: Mutex<Option<Frames>>,
    }
    impl Transport for StaticHost {
        fn call(&self, _: Request) -> std::result::Result<Response, ClientError> {
            Ok(Response::terminal_metadata(Ok(TerminalMetadata::default())))
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            unreachable!()
        }
        fn attach(&self, _: &str) -> std::result::Result<TerminalAttachment, ClientError> {
            let frames = self.frames.lock().unwrap().take().unwrap();
            Ok(TerminalAttachment {
                subscriber_id: 0,
                snapshot: TerminalSnapshot {
                    seq: 0,
                    format: Default::default(),
                    unavailable: false,
                    cols: self.cols,
                    rows: self.rows,
                    bytes: Vec::new(),
                    unfinished_len: 0,
                    preceding_char: None,
                    activity: None,
                    controller: Default::default(),
                },
                frames: Box::new(frames),
            })
        }
    }

    // 1. Plain row with HT copies spaces up to the 8-column tab stop.
    let (tx1, rx1) = mpsc::channel();
    let mirror = TerminalMirror::attach(
        DaemonClient::new(Arc::new(StaticHost {
            cols: 20,
            rows: 4,
            frames: Mutex::new(Some(Frames {
                tx: tx1.clone(),
                rx: rx1,
                cancelled: Arc::new(AtomicBool::new(false)),
            })),
        })),
        "tab-plain".into(),
        Arc::new(|| {}),
    )
    .unwrap();
    mirror.test_feed(1, b"A\tB");
    mirror.start_selection(
        SelectionKind::Simple,
        GridPoint::new(Line(0), Column(0)),
        Side::Left,
    );
    mirror.update_selection(GridPoint::new(Line(0), Column(8)), Side::Right);
    assert_eq!(mirror.selection_text().as_deref(), Some("A       B"));
    drop(tx1);
    mirror.test_stop_frames();

    // 2. Selection spanning a soft wrap after HT preserves spaces to the tab
    //    stop and emits no spurious newline across the wrap boundary.
    let (tx2, rx2) = mpsc::channel();
    let wrapped = TerminalMirror::attach(
        DaemonClient::new(Arc::new(StaticHost {
            cols: 10,
            rows: 4,
            frames: Mutex::new(Some(Frames {
                tx: tx2.clone(),
                rx: rx2,
                cancelled: Arc::new(AtomicBool::new(false)),
            })),
        })),
        "tab-wrap".into(),
        Arc::new(|| {}),
    )
    .unwrap();
    wrapped.test_feed(1, b"A\tBCDE");
    wrapped.start_selection(
        SelectionKind::Simple,
        GridPoint::new(Line(0), Column(0)),
        Side::Left,
    );
    wrapped.update_selection(GridPoint::new(Line(1), Column(1)), Side::Right);
    assert_eq!(wrapped.selection_text().as_deref(), Some("A       BCDE"));
    drop(tx2);
    wrapped.test_stop_frames();
}

fn wait_until(timeout: Duration, mut pred: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if pred() {
            return true;
        }
        thread::sleep(Duration::from_millis(5));
    }
    pred()
}

#[test]
fn unavailable_and_corrupt_snapshots_never_blank_a_synced_mirror() {
    use runner_core::protocol::terminal::{
        FrameSink, TerminalAttachment, TerminalSnapshot, TerminalSubscription,
    };
    use runner_core::protocol::{ClientError, EventSubscription, Request, Response, Transport};

    type SharedFrameSink = Arc<dyn Fn(Option<TerminalFrame>) -> bool + Send + Sync>;

    struct PushQueue {
        state: Mutex<(
            Option<SharedFrameSink>,
            std::collections::VecDeque<TerminalFrame>,
        )>,
    }
    impl PushQueue {
        fn new() -> Self {
            Self {
                state: Mutex::new((None, std::collections::VecDeque::new())),
            }
        }
        fn push(&self, frame: TerminalFrame) {
            let ready = {
                let mut guard = self.state.lock().unwrap();
                if let Some(sink) = guard.0.as_ref() {
                    Some((Arc::clone(sink), frame))
                } else {
                    guard.1.push_back(frame);
                    None
                }
            };
            if let Some((sink, frame)) = ready {
                let _ = sink(Some(frame));
            }
        }
        fn has_sink(&self) -> bool {
            self.state.lock().unwrap().0.is_some()
        }
    }
    struct PushFrames(Arc<PushQueue>);
    impl TerminalSubscription for PushFrames {
        fn recv(&mut self) -> std::result::Result<TerminalFrame, ClientError> {
            Err(ClientError::msg("push-only"))
        }
        fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
            let q = Arc::clone(&self.0);
            Arc::new(move || {
                let sink = q.state.lock().unwrap().0.take();
                if let Some(sink) = sink {
                    let _ = sink(None);
                }
            })
        }
        fn install_sink(&mut self, sink: FrameSink) -> bool {
            let sink: SharedFrameSink = Arc::from(sink);
            let pending: Vec<TerminalFrame> = {
                let mut guard = self.0.state.lock().unwrap();
                guard.0 = Some(Arc::clone(&sink));
                guard.1.drain(..).collect()
            };
            for frame in pending {
                if !sink(Some(frame)) {
                    break;
                }
            }
            true
        }
    }

    struct Host;
    impl TerminalHost for Host {
        fn write_input(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn write_reply(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn report_input_state(&self, _: &str, _: InputObservation) {}
        fn set_live_title(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn resize(&self, _: &str, _: u16, _: u16) -> Result<()> {
            Ok(())
        }
        fn is_local_host(&self, _: &str) -> bool {
            true
        }
        fn clipboard_store(&self, _: &str, _: ClipboardTarget, _: &str) {}
    }

    let model = TerminalModel::new(
        Arc::new(Host),
        "unavail".into(),
        20,
        4,
        ModelOptions::default(),
    )
    .unwrap();
    model.feed_output(1, b"KEEPME");
    let good_snap = model.snapshot(1);
    assert!(!good_snap.unavailable);
    assert_eq!(good_snap.format, SnapshotFormat::GhosttyBinary);
    assert_eq!(good_snap.preceding_char, None);

    // Unfinished sequence past the 64 KiB continuation cap marks both snapshot
    // formats unavailable while keeping the model's screen intact.
    model.feed_output(2, b"\x1b]2;");
    model.feed_output(3, &vec![b'x'; 65_537]);
    assert_eq!(model.visible_lines()[0], "KEEPME");
    let unavail_bin = model.snapshot(3);
    assert!(unavail_bin.unavailable);
    assert!(unavail_bin.bytes.is_empty());
    assert!(crate::replay::Terminal::from_snapshot(&unavail_bin).is_err());
    let unavail_vt = model.snapshot_with_history(3, 0);
    assert!(unavail_vt.unavailable);
    assert!(unavail_vt.bytes.is_empty());
    assert!(crate::replay::Terminal::from_snapshot(&unavail_vt).is_err());
    assert!(!model.take_resync_needed());

    // Aborting the oversize OSC with CAN and immediately opening a short
    // partial CSI makes snapshots representable again even though the parser is
    // not at ground at the end of the write.
    model.feed_output(4, b"\x18MORE\x1b[");
    assert!(model.take_resync_needed());
    assert!(!model.take_resync_needed());
    let recovered_snap = model.snapshot(4);
    assert!(!recovered_snap.unavailable);

    // Corrupt binary snapshot on initial attach fails loudly.
    struct SingleSnapTransport(Mutex<Option<TerminalSnapshot>>);
    impl Transport for SingleSnapTransport {
        fn call(&self, _: Request) -> std::result::Result<Response, ClientError> {
            Ok(Response::terminal_metadata(Ok(TerminalMetadata::default())))
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            unreachable!()
        }
        fn attach(&self, _: &str) -> std::result::Result<TerminalAttachment, ClientError> {
            Ok(TerminalAttachment {
                subscriber_id: 1,
                snapshot: self.0.lock().unwrap().take().unwrap(),
                frames: Box::new(PushFrames(Arc::new(PushQueue::new()))),
            })
        }
    }
    let mut corrupt = good_snap.clone();
    corrupt.bytes = b"not-a-valid-snapshot".to_vec();
    assert!(TerminalMirror::attach(
        DaemonClient::new(Arc::new(SingleSnapTransport(Mutex::new(Some(
            corrupt.clone()
        ))))),
        "corrupt-init".into(),
        Arc::new(|| {}),
    )
    .is_err());

    // On resync, an unavailable or corrupt snapshot retains the prior good view
    // and does not advance `last_seq`, then recovers once a valid snapshot arrives.
    struct SequenceTransport {
        snaps: Mutex<std::collections::VecDeque<TerminalSnapshot>>,
        queues: Mutex<Vec<Arc<PushQueue>>>,
    }
    impl Transport for SequenceTransport {
        fn call(&self, _: Request) -> std::result::Result<Response, ClientError> {
            Ok(Response::terminal_metadata(Ok(TerminalMetadata::default())))
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            unreachable!()
        }
        fn attach(&self, _: &str) -> std::result::Result<TerminalAttachment, ClientError> {
            let snap = self.snaps.lock().unwrap().pop_front().unwrap();
            let q = Arc::new(PushQueue::new());
            self.queues.lock().unwrap().push(Arc::clone(&q));
            Ok(TerminalAttachment {
                subscriber_id: 1,
                snapshot: snap,
                frames: Box::new(PushFrames(q)),
            })
        }
    }

    let transport = Arc::new(SequenceTransport {
        snaps: Mutex::new(
            vec![good_snap, unavail_bin, corrupt, recovered_snap]
                .into_iter()
                .collect(),
        ),
        queues: Mutex::new(Vec::new()),
    });
    let mirror = TerminalMirror::attach(
        DaemonClient::new(transport.clone()),
        "resync-keep".into(),
        Arc::new(|| {}),
    )
    .unwrap();
    let _view = mirror.view();
    assert_eq!(mirror.visible_lines()[0], "KEEPME");
    assert_eq!(mirror.output_activity().last_seq, 1);

    // 1. Resync receives unavailable snapshot -> keeps KEEPME and seq=1,
    // and ignores a Resized frame's sequence/dimensions while still applying its
    // controller update.
    assert!(wait_until(Duration::from_secs(2), || {
        transport
            .queues
            .lock()
            .unwrap()
            .first()
            .is_some_and(|q| q.has_sink())
    }));
    transport.queues.lock().unwrap()[0].push(TerminalFrame::Resync);
    assert!(wait_until(Duration::from_secs(2), || {
        transport
            .queues
            .lock()
            .unwrap()
            .get(1)
            .is_some_and(|q| q.has_sink())
    }));
    let browser_controller = TerminalController::Browser {
        channel: 1,
        name: "Phone".into(),
    };
    transport.queues.lock().unwrap()[1].push(TerminalFrame::Resized {
        seq: 5,
        cols: 42,
        rows: 11,
        controller: browser_controller.clone(),
    });
    assert!(wait_until(Duration::from_secs(2), || {
        mirror.control().controller == browser_controller
    }));
    assert_eq!(mirror.size(), (20, 4));
    assert_eq!(mirror.visible_lines()[0], "KEEPME");
    assert_eq!(mirror.output_activity().last_seq, 1);

    // 2. Resync receives corrupt binary snapshot -> retains KEEPME at seq=1,
    // ignores normal output and resize frames while awaiting recovery, and
    // automatically recovers via bounded decode retry without an external Resync.
    transport.queues.lock().unwrap()[1].push(TerminalFrame::Resync);
    assert!(wait_until(Duration::from_secs(2), || {
        transport
            .queues
            .lock()
            .unwrap()
            .get(2)
            .is_some_and(|q| q.has_sink())
    }));
    // Send normal output and a resize on the corrupt-step subscription (no manual Resync).
    transport.queues.lock().unwrap()[2].push(TerminalFrame::Resized {
        seq: 6,
        cols: 50,
        rows: 12,
        controller: TerminalController::Desktop,
    });
    transport.queues.lock().unwrap()[2].push(TerminalFrame::Output {
        seq: 7,
        bytes: b"IGNORED".to_vec(),
    });
    assert!(wait_until(Duration::from_secs(2), || {
        mirror.output_activity().last_seq == 4 && mirror.visible_lines()[0] == "KEEPMEMORE"
    }));
    assert_eq!(mirror.size(), (20, 4));
    // Normal output on the automatically recovered subscription applies cleanly.
    assert!(wait_until(Duration::from_secs(2), || {
        transport
            .queues
            .lock()
            .unwrap()
            .get(3)
            .is_some_and(|q| q.has_sink())
    }));
    transport.queues.lock().unwrap()[3].push(TerminalFrame::Output {
        seq: 5,
        bytes: b"m!".to_vec(),
    });
    assert!(wait_until(Duration::from_secs(2), || {
        mirror.output_activity().last_seq == 5 && mirror.visible_lines()[0] == "KEEPMEMORE!"
    }));

    drop(mirror);

    // 3. Fallback recv-based ResizedFrame while awaiting resync likewise keeps
    // last_seq and dimensions intact while applying controller updates.
    {
        struct RecvQueue {
            tx: mpsc::Sender<TerminalFrame>,
            rx: mpsc::Receiver<TerminalFrame>,
            cancelled: Arc<AtomicBool>,
        }
        impl TerminalSubscription for RecvQueue {
            fn recv(&mut self) -> std::result::Result<TerminalFrame, ClientError> {
                let frame = self.rx.recv().map_err(|_| ClientError::msg("closed"))?;
                if self.cancelled.load(Ordering::Acquire) {
                    return Err(ClientError::msg("cancelled"));
                }
                Ok(frame)
            }
            fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
                let tx = self.tx.clone();
                let cancelled = Arc::clone(&self.cancelled);
                Arc::new(move || {
                    cancelled.store(true, Ordering::Release);
                    let _ = tx.send(TerminalFrame::Resync);
                })
            }
        }
        struct RecvSeqTransport {
            snaps: Mutex<std::collections::VecDeque<TerminalSnapshot>>,
            senders: Mutex<Vec<mpsc::Sender<TerminalFrame>>>,
        }
        impl Transport for RecvSeqTransport {
            fn call(&self, _: Request) -> std::result::Result<Response, ClientError> {
                Ok(Response::terminal_metadata(Ok(TerminalMetadata::default())))
            }
            fn subscribe(&self) -> Box<dyn EventSubscription> {
                unreachable!()
            }
            fn attach(&self, _: &str) -> std::result::Result<TerminalAttachment, ClientError> {
                let snap = self.snaps.lock().unwrap().pop_front().unwrap();
                let (tx, rx) = mpsc::channel();
                self.senders.lock().unwrap().push(tx.clone());
                Ok(TerminalAttachment {
                    subscriber_id: 1,
                    snapshot: snap,
                    frames: Box::new(RecvQueue {
                        tx,
                        rx,
                        cancelled: Arc::new(AtomicBool::new(false)),
                    }),
                })
            }
        }
        let good2 = model.snapshot(1);
        let mut unavail2 = good2.clone();
        unavail2.unavailable = true;
        unavail2.bytes.clear();
        let recv_transport = Arc::new(RecvSeqTransport {
            snaps: Mutex::new(vec![good2, unavail2].into_iter().collect()),
            senders: Mutex::new(Vec::new()),
        });
        let recv_mirror = TerminalMirror::attach(
            DaemonClient::new(recv_transport.clone()),
            "resync-recv-resize".into(),
            Arc::new(|| {}),
        )
        .unwrap();
        let _recv_view = recv_mirror.view();
        recv_transport.senders.lock().unwrap()[0]
            .send(TerminalFrame::Resync)
            .unwrap();
        assert!(wait_until(Duration::from_secs(2), || {
            recv_transport.senders.lock().unwrap().len() >= 2
        }));
        let remote = TerminalController::Browser {
            channel: 2,
            name: "iPad".into(),
        };
        recv_transport.senders.lock().unwrap()[1]
            .send(TerminalFrame::Resized {
                seq: 9,
                cols: 55,
                rows: 15,
                controller: remote.clone(),
            })
            .unwrap();
        assert!(wait_until(Duration::from_secs(2), || {
            recv_mirror.control().controller == remote
        }));
        assert_eq!(recv_mirror.size(), (20, 4));
        assert_eq!(recv_mirror.output_activity().last_seq, 1);
        drop(recv_mirror);
    }

    for worker in model.stop_workers() {
        worker.join().unwrap();
    }
}

#[test]
fn companion_vt_snapshot_preserves_primary_screen_soft_wraps_and_rep_through_mirror() {
    use runner_core::protocol::terminal::{
        FrameSink, TerminalAttachment, TerminalSnapshot, TerminalSubscription,
    };
    use runner_core::protocol::{ClientError, EventSubscription, Request, Response, Transport};

    struct NoopFrames;
    impl TerminalSubscription for NoopFrames {
        fn recv(&mut self) -> std::result::Result<TerminalFrame, ClientError> {
            Err(ClientError::msg("closed"))
        }
        fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
            Arc::new(|| {})
        }
        fn install_sink(&mut self, _sink: FrameSink) -> bool {
            true
        }
    }
    struct SnapTransport(Mutex<Option<TerminalSnapshot>>);
    impl Transport for SnapTransport {
        fn call(&self, _: Request) -> std::result::Result<Response, ClientError> {
            Ok(Response::terminal_metadata(Ok(TerminalMetadata::default())))
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            unreachable!()
        }
        fn attach(&self, _: &str) -> std::result::Result<TerminalAttachment, ClientError> {
            Ok(TerminalAttachment {
                subscriber_id: 1,
                snapshot: self.0.lock().unwrap().take().unwrap(),
                frames: Box::new(NoopFrames),
            })
        }
    }
    struct Host;
    impl TerminalHost for Host {
        fn write_input(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn write_reply(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn report_input_state(&self, _: &str, _: InputObservation) {}
        fn set_live_title(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn resize(&self, _: &str, _: u16, _: u16) -> Result<()> {
            Ok(())
        }
        fn is_local_host(&self, _: &str) -> bool {
            true
        }
        fn clipboard_store(&self, _: &str, _: ClipboardTarget, _: &str) {}
    }

    // 1. Alternate screen active: companion VT snapshot restores BOTH alt screen
    //    and primary screen (including scrollback history) on `TerminalMirror`.
    let model = TerminalModel::new(
        Arc::new(Host),
        "alt-v1".into(),
        20,
        4,
        ModelOptions::default(),
    )
    .unwrap();
    model.feed_output(
        1,
        b"hist0\r\nhist1\r\nhist2\r\nhist3\r\nhist4\r\nKEEPME\x1b[?1049hALT",
    );
    let snap = model.snapshot_with_history(1, 10);
    assert_eq!(snap.format, SnapshotFormat::Vt);
    assert!(!snap.unavailable);
    let mirror = TerminalMirror::attach(
        DaemonClient::new(Arc::new(SnapTransport(Mutex::new(Some(snap))))),
        "alt-v1-mirror".into(),
        Arc::new(|| {}),
    )
    .unwrap();
    assert_eq!(mirror.visible_lines(), model.visible_lines());
    mirror.test_feed(2, b"\x1b[?1049l");
    model.feed_output(2, b"\x1b[?1049l");
    assert_eq!(mirror.visible_lines(), model.visible_lines());
    assert!(mirror.visible_lines().iter().any(|l| l == "KEEPME"));
    assert_eq!(mirror.scroll_state().history_lines, 2);
    mirror.with_screen(|screen| {
        assert_eq!(screen.view().row_to_string(Line(-2)), "hist0");
        assert_eq!(screen.view().row_to_string(Line(-1)), "hist1");
    });
    drop(mirror);
    for worker in model.stop_workers() {
        worker.join().unwrap();
    }

    // 2. Soft-wrapped primary line reflows identically to host when widened on `TerminalMirror`.
    let model = TerminalModel::new(
        Arc::new(Host),
        "wrap-v1".into(),
        8,
        4,
        ModelOptions::default(),
    )
    .unwrap();
    model.feed_output(1, b"abcdefghijklmnopq");
    let snap = model.snapshot_with_history(1, 0);
    let mirror = TerminalMirror::attach(
        DaemonClient::new(Arc::new(SnapTransport(Mutex::new(Some(snap))))),
        "wrap-v1-mirror".into(),
        Arc::new(|| {}),
    )
    .unwrap();
    model.resize(20, 4);
    mirror.resize_local(20, 4);
    assert_eq!(mirror.visible_lines(), model.visible_lines());
    assert_eq!(mirror.visible_lines()[0], "abcdefghijklmnopq");
    drop(mirror);
    for worker in model.stop_workers() {
        worker.join().unwrap();
    }

    // 3. Split CSI + REP (`abcccc`) still works through real `TerminalMirror` restore.
    let model = TerminalModel::new(
        Arc::new(Host),
        "rep-v1".into(),
        20,
        4,
        ModelOptions::default(),
    )
    .unwrap();
    model.feed_output(1, b"abc\x1b[3");
    let snap = model.snapshot_with_history(1, 0);
    assert_eq!(snap.preceding_char, Some('c'));
    assert_eq!(snap.unfinished_len, 3);
    let mirror = TerminalMirror::attach(
        DaemonClient::new(Arc::new(SnapTransport(Mutex::new(Some(snap))))),
        "rep-v1-mirror".into(),
        Arc::new(|| {}),
    )
    .unwrap();
    mirror.test_feed(2, b"b");
    assert_eq!(mirror.visible_lines()[0], "abcccc");
    drop(mirror);
    for worker in model.stop_workers() {
        worker.join().unwrap();
    }
}

#[test]
fn dec_2026_preserves_pre_hold_output_in_same_chunk_and_freezes_resync_while_held() {
    use runner_core::protocol::terminal::{
        FrameSink, TerminalAttachment, TerminalSnapshot, TerminalSubscription,
    };
    use runner_core::protocol::{ClientError, EventSubscription, Request, Response, Transport};

    type SharedFrameSink = Arc<dyn Fn(Option<TerminalFrame>) -> bool + Send + Sync>;

    struct PushQueue {
        state: Mutex<(
            Option<SharedFrameSink>,
            std::collections::VecDeque<TerminalFrame>,
        )>,
    }
    impl PushQueue {
        fn new() -> Self {
            Self {
                state: Mutex::new((None, std::collections::VecDeque::new())),
            }
        }
        fn push(&self, frame: TerminalFrame) {
            let ready = {
                let mut guard = self.state.lock().unwrap();
                if let Some(sink) = guard.0.as_ref() {
                    Some((Arc::clone(sink), frame))
                } else {
                    guard.1.push_back(frame);
                    None
                }
            };
            if let Some((sink, frame)) = ready {
                let _ = sink(Some(frame));
            }
        }
    }
    struct PushFrames(Arc<PushQueue>);
    impl TerminalSubscription for PushFrames {
        fn recv(&mut self) -> std::result::Result<TerminalFrame, ClientError> {
            Err(ClientError::msg("push-only"))
        }
        fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
            Arc::new(|| {})
        }
        fn install_sink(&mut self, sink: FrameSink) -> bool {
            let sink: SharedFrameSink = Arc::from(sink);
            let pending: Vec<TerminalFrame> = {
                let mut guard = self.0.state.lock().unwrap();
                guard.0 = Some(Arc::clone(&sink));
                guard.1.drain(..).collect()
            };
            for frame in pending {
                if !sink(Some(frame)) {
                    break;
                }
            }
            true
        }
    }
    struct SequenceTransport {
        snaps: Mutex<std::collections::VecDeque<TerminalSnapshot>>,
        queues: Mutex<Vec<Arc<PushQueue>>>,
    }
    impl Transport for SequenceTransport {
        fn call(&self, _: Request) -> std::result::Result<Response, ClientError> {
            Ok(Response::terminal_metadata(Ok(TerminalMetadata::default())))
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            unreachable!()
        }
        fn attach(&self, _: &str) -> std::result::Result<TerminalAttachment, ClientError> {
            let snap = self.snaps.lock().unwrap().pop_front().unwrap();
            let q = Arc::new(PushQueue::new());
            self.queues.lock().unwrap().push(Arc::clone(&q));
            Ok(TerminalAttachment {
                subscriber_id: 1,
                snapshot: snap,
                frames: Box::new(PushFrames(q)),
            })
        }
    }
    struct Host;
    impl TerminalHost for Host {
        fn write_input(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn write_reply(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn report_input_state(&self, _: &str, _: InputObservation) {}
        fn set_live_title(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn resize(&self, _: &str, _: u16, _: u16) -> Result<()> {
            Ok(())
        }
        fn is_local_host(&self, _: &str) -> bool {
            true
        }
        fn clipboard_store(&self, _: &str, _: ClipboardTarget, _: &str) {}
    }

    let model = TerminalModel::new(
        Arc::new(Host),
        "sync-m".into(),
        20,
        4,
        ModelOptions::default(),
    )
    .unwrap();
    let init_snap = model.snapshot(0);
    model.feed_output(1, b"BASE");
    model.feed_output(2, b"VISIBLE\x1b[?2026hHELD");
    let mid_hold_snap = model.snapshot(2);

    let transport = Arc::new(SequenceTransport {
        snaps: Mutex::new(vec![init_snap, mid_hold_snap].into_iter().collect()),
        queues: Mutex::new(Vec::new()),
    });
    let mirror = TerminalMirror::attach(
        DaemonClient::new(transport.clone()),
        "sync-mirror".into(),
        Arc::new(|| {}),
    )
    .unwrap();
    let _view = mirror.view();

    // 1. Under Decision 18, an unsplit chunk containing a prefix and an open
    //    `CSI ? 2026 h` keeps the pre-chunk published frame ("BASE") frozen
    //    while held (never leaking `HELD`) and publishes the complete frame
    //    ("BASEVISIBLEHELD") once `CSI ? 2026 l` arrives.
    mirror.test_feed(1, b"BASE");
    mirror.test_feed(2, b"VISIBLE\x1b[?2026hHELD");
    assert_eq!(mirror.viewport_snapshot().view().visible_lines()[0], "BASE");
    mirror.test_feed(3, b"\x1b[?2026l");
    assert_eq!(
        mirror.viewport_snapshot().view().visible_lines()[0],
        "BASEVISIBLEHELD"
    );

    // 2. Large (>64 KiB) synchronized update stays frozen on the pre-hold frame
    //    until its end marker arrives.
    let mut big = b"\x1b[?2026h\x1b[2J\x1b[H".to_vec();
    big.extend(std::iter::repeat_n(b'X', 70_000));
    mirror.test_feed(4, &big);
    assert_eq!(
        mirror.viewport_snapshot().view().visible_lines()[0],
        "BASEVISIBLEHELD"
    );
    mirror.test_feed(5, b"\x1b[HAFTERBIG\x1b[?2026l");
    assert!(mirror.viewport_snapshot().view().visible_lines()[0].starts_with("AFTERBIG"));

    // 3. Resync to a snapshot taken mid-DEC-2026 keeps the prior committed
    //    frame frozen instead of exposing `HELD`, then flushes at the deadline.
    transport.queues.lock().unwrap()[0].push(TerminalFrame::Resync);
    assert!(wait_until(Duration::from_secs(2), || {
        transport.queues.lock().unwrap().len() >= 2
    }));
    let before_flush = mirror.viewport_snapshot().view().visible_lines()[0].clone();
    assert!(
        before_flush.starts_with("AFTERBIG"),
        "resync mid-hold exposed torn frame: {before_flush:?}"
    );
    assert!(wait_until(Duration::from_secs(2), || {
        mirror.viewport_snapshot().view().visible_lines()[0] == "BASEVISIBLEHELD"
    }));

    drop(mirror);
    for worker in model.stop_workers() {
        worker.join().unwrap();
    }
}

#[test]
fn post_invalidation_osc_7501_accepts_subsequent_report_in_same_or_split_chunk() {
    struct Host;
    impl TerminalHost for Host {
        fn write_input(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn write_reply(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn report_input_state(&self, _: &str, _: InputObservation) {}
        fn set_live_title(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn resize(&self, _: &str, _: u16, _: u16) -> Result<()> {
            Ok(())
        }
        fn is_local_host(&self, _: &str) -> bool {
            true
        }
        fn clipboard_store(&self, _: &str, _: ClipboardTarget, _: &str) {}
    }

    let cases: &[(&str, &[u8], &[u8])] = &[
        (
            "can-abort",
            b"\x1b]7501;state=working:id=old",
            b"\x18\x1b]7501;state=blocked:id=new\x07",
        ),
        (
            "sub-abort",
            b"\x1b]7501;state=working:id=old",
            b"\x1a\x1b]7501;state=blocked:id=new\x07",
        ),
        (
            "bel-valid-old",
            b"\x1b]7501;state=working:id=old",
            b"\x07\x1b]7501;state=blocked:id=new\x07",
        ),
        (
            "bel-invalid-old",
            b"\x1b]7501;state=bogus:id=old",
            b"\x07\x1b]7501;state=blocked:id=new\x07",
        ),
        (
            "st-valid-old",
            b"\x1b]7501;state=working:id=old",
            b"\x1b\\\x1b]7501;state=blocked:id=new\x07",
        ),
        (
            "esc-starts-next-osc",
            b"\x1b]7501;state=working:id=old",
            b"\x1b]7501;state=blocked:id=new\x07",
        ),
    ];

    for (name, prefix, suffix) in cases {
        // Single-chunk post-invalidation delivery.
        let model = TerminalModel::new(
            Arc::new(Host),
            format!("7501-{name}"),
            40,
            4,
            ModelOptions::default(),
        )
        .unwrap();
        model.feed_output(1, prefix);
        model.invalidate_program_status();
        model.feed_output(2, suffix);
        let statuses = model.take_program_statuses();
        assert_eq!(statuses.len(), 1, "{name} single-chunk");
        assert_eq!(
            statuses[0].1.current.as_ref().map(|r| r.id.as_str()),
            Some("new"),
            "{name} single-chunk"
        );
        for w in model.stop_workers() {
            w.join().unwrap();
        }

        // Every 1-byte split of the post-invalidation suffix yields the exact same result.
        for split in 1..suffix.len() {
            let model = TerminalModel::new(
                Arc::new(Host),
                format!("7501-{name}-{split}"),
                40,
                4,
                ModelOptions::default(),
            )
            .unwrap();
            model.feed_output(1, prefix);
            model.invalidate_program_status();
            model.feed_output(2, &suffix[..split]);
            model.feed_output(3, &suffix[split..]);
            let statuses = model.take_program_statuses();
            assert_eq!(statuses.len(), 1, "{name} split={split}");
            assert_eq!(
                statuses[0].1.current.as_ref().map(|r| r.id.as_str()),
                Some("new"),
                "{name} split={split}"
            );
            for w in model.stop_workers() {
                w.join().unwrap();
            }
        }
    }
}

#[test]
fn title_stack_da1_da2_da3_and_kitty_keyboard_bit0_cap_match_contract() {
    struct RecordingHost {
        replies: Mutex<Vec<Vec<u8>>>,
        titles: Mutex<Vec<String>>,
    }
    impl TerminalHost for RecordingHost {
        fn write_input(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn write_reply(&self, _: &str, bytes: &[u8]) -> Result<()> {
            self.replies.lock().unwrap().push(bytes.to_vec());
            Ok(())
        }
        fn report_input_state(&self, _: &str, _: InputObservation) {}
        fn set_live_title(&self, _: &str, title: &str) -> Result<()> {
            self.titles.lock().unwrap().push(title.to_owned());
            Ok(())
        }
        fn resize(&self, _: &str, _: u16, _: u16) -> Result<()> {
            Ok(())
        }
        fn is_local_host(&self, _: &str) -> bool {
            true
        }
        fn clipboard_store(&self, _: &str, _: ClipboardTarget, _: &str) {}
    }

    let host = Arc::new(RecordingHost {
        replies: Mutex::default(),
        titles: Mutex::default(),
    });
    let model = TerminalModel::new(
        host.clone(),
        "da-title-kitty".into(),
        40,
        4,
        ModelOptions::default(),
    )
    .unwrap();

    // Title push/pop honors xterm 0/2 params and ignores 1 (icon-only).
    model.feed_output(
        1,
        b"\x1b]2;oldtitle\x07\x1b[22;0t\x1b]2;newtitle\x07\x1b[22;1t\x1b[23;1t\x1b[23;0t",
    );
    model.flush_events();
    assert_eq!(model.title(), "oldtitle");
    assert_eq!(
        *host.titles.lock().unwrap(),
        vec![
            "oldtitle".to_owned(),
            "newtitle".to_owned(),
            "oldtitle".to_owned()
        ]
    );

    // DA1, DA2, DA3 each produce exactly one reply from the model.
    model.feed_output(2, b"\x1b[c\x1b[>c\x1b[=c");
    model.flush_events();
    assert_eq!(
        *host.replies.lock().unwrap(),
        vec![
            b"\x1b[?6c".to_vec(),
            b"\x1b[>0;2600;1c".to_vec(),
            b"\x1bP!|00000000\x1b\\".to_vec(),
        ]
    );

    // Kitty keyboard bit-0 cap across queries, exported VT snapshots, and stack inspection.
    host.replies.lock().unwrap().clear();
    model.feed_output(3, b"\x1b[>31u\x1b[?u\x1b[>30u\x1b[?u\x1b[<u\x1b[?u");
    model.flush_events();
    assert_eq!(
        *host.replies.lock().unwrap(),
        vec![
            b"\x1b[?1u".to_vec(),
            b"\x1b[?0u".to_vec(),
            b"\x1b[?1u".to_vec()
        ]
    );
    let vt_snap = model.snapshot_with_history(3, 0);
    let vt_text = String::from_utf8_lossy(&vt_snap.bytes);
    assert!(
        !vt_text.contains("\x1b[=31;1u") && !vt_text.contains("\x1b[=30;1u"),
        "companion VT snapshot leaked uncapped Kitty flags: {vt_text:?}"
    );
    assert!(vt_text.contains("\x1b[>1u"));
    model.with_screen(|screen| {
        let state = screen.snapshot_state();
        assert!(
            state.keyboard_mode_stack.iter().all(|&f| f <= 1),
            "uncapped stack: {:?}",
            state.keyboard_mode_stack
        );
    });

    // Mirror emits zero replies for DA1/DA2/DA3.
    let mut replay = crate::replay::Terminal::new(40, 4);
    replay.advance(b"\x1b]2;A\x07\x1b[22;2t\x1b]2;B\x07");
    assert_eq!(replay.snapshot_state().title.as_deref(), Some("B"));
    assert_eq!(replay.snapshot_state().title_stack, vec![Some("A".into())]);
    replay.advance(b"\x1b[23;2t");
    assert_eq!(replay.snapshot_state().title.as_deref(), Some("A"));
    assert!(replay.snapshot_state().title_stack.is_empty());

    for worker in model.stop_workers() {
        worker.join().unwrap();
    }
}

#[test]
fn mirror_push_sink_uses_one_thread_publishes_lazily_and_exits_cleanly_on_drop() {
    use runner_core::protocol::terminal::{
        FrameSink, TerminalAttachment, TerminalSnapshot, TerminalSubscription,
    };
    use runner_core::protocol::{ClientError, EventSubscription, Request, Response, Transport};

    type SharedFrameSink = Arc<dyn Fn(Option<TerminalFrame>) -> bool + Send + Sync>;

    struct PushQueue {
        sink: Mutex<Option<SharedFrameSink>>,
        dropped: Arc<AtomicBool>,
    }
    struct PushFrames(Arc<PushQueue>);
    impl Drop for PushFrames {
        fn drop(&mut self) {
            self.0.dropped.store(true, Ordering::SeqCst);
        }
    }
    impl TerminalSubscription for PushFrames {
        fn recv(&mut self) -> std::result::Result<TerminalFrame, ClientError> {
            Err(ClientError::msg("push-only"))
        }
        fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
            Arc::new(|| {})
        }
        fn install_sink(&mut self, sink: FrameSink) -> bool {
            *self.0.sink.lock().unwrap() = Some(Arc::from(sink));
            true
        }
    }
    struct PushTransport {
        queue: Arc<PushQueue>,
    }
    impl Transport for PushTransport {
        fn call(&self, _: Request) -> std::result::Result<Response, ClientError> {
            Ok(Response::terminal_metadata(Ok(TerminalMetadata::default())))
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            unreachable!()
        }
        fn attach(&self, _: &str) -> std::result::Result<TerminalAttachment, ClientError> {
            Ok(TerminalAttachment {
                subscriber_id: 1,
                snapshot: TerminalSnapshot {
                    seq: 0,
                    format: SnapshotFormat::Vt,
                    unavailable: false,
                    cols: 20,
                    rows: 4,
                    bytes: Vec::new(),
                    unfinished_len: 0,
                    preceding_char: None,
                    activity: None,
                    controller: Default::default(),
                },
                frames: Box::new(PushFrames(Arc::clone(&self.queue))),
            })
        }
    }

    let dropped = Arc::new(AtomicBool::new(false));
    let queue = Arc::new(PushQueue {
        sink: Mutex::new(None),
        dropped: Arc::clone(&dropped),
    });
    let mirror = TerminalMirror::attach(
        DaemonClient::new(Arc::new(PushTransport {
            queue: Arc::clone(&queue),
        })),
        "push-one-thread".into(),
        Arc::new(|| {}),
    )
    .unwrap();
    // Push subscription runs on MirrorActor with no fallback FrameWorker thread.
    assert!(mirror.frame_worker.lock().unwrap().is_none());

    // Hidden mirror (`viewers == 0`) marks viewport dirty without calling
    // `RenderState::update` on printable OR cursor/SGR/erase control chunks.
    let updates_after_attach = mirror.test_render_state_updates();
    queue.sink.lock().unwrap().as_ref().unwrap()(Some(TerminalFrame::Output {
        seq: 1,
        bytes: b"HIDDEN_LINE".to_vec(),
    }));
    queue.sink.lock().unwrap().as_ref().unwrap()(Some(TerminalFrame::Output {
        seq: 2,
        bytes: b"\x1b[2;5H\x1b[31;1m".to_vec(),
    }));
    queue.sink.lock().unwrap().as_ref().unwrap()(Some(TerminalFrame::Output {
        seq: 3,
        bytes: b"\x1b[2J\x1b[1;1H".to_vec(),
    }));
    queue.sink.lock().unwrap().as_ref().unwrap()(Some(TerminalFrame::Output {
        seq: 4,
        bytes: b"HIDDEN_AFTER".to_vec(),
    }));
    assert!(wait_until(Duration::from_secs(2), || {
        mirror.output_activity().last_seq == 4
    }));
    assert_eq!(
        mirror.test_render_state_updates(),
        updates_after_attach,
        "hidden mirror must not call RenderState::update on printable or control traffic"
    );
    assert_eq!(mirror.output_activity().first_paint_seq, 1);
    assert_eq!(mirror.output_activity().last_paint_seq, 4);
    assert!(mirror.shared.viewport_dirty.load(Ordering::Acquire));

    // Becoming visible (`0 -> 1`) immediately syncs and clears `viewport_dirty`.
    let view = mirror.view();
    assert!(!mirror.shared.viewport_dirty.load(Ordering::Acquire));
    assert_eq!(mirror.test_render_state_updates(), updates_after_attach + 1);
    assert_eq!(
        mirror.viewport_snapshot().view().visible_lines()[0],
        "HIDDEN_AFTER"
    );
    drop(view);

    // Dropping the mirror shuts down the actor thread and drops the subscription cleanly.
    drop(mirror);
    assert!(dropped.load(Ordering::SeqCst));
}

#[test]
fn companion_vt_preserves_saved_cursors_and_kitty_stack_depth_on_both_screens() {
    struct Host;
    impl TerminalHost for Host {
        fn write_input(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn write_reply(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn report_input_state(&self, _: &str, _: InputObservation) {}
        fn set_live_title(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn resize(&self, _: &str, _: u16, _: u16) -> Result<()> {
            Ok(())
        }
        fn is_local_host(&self, _: &str) -> bool {
            true
        }
        fn clipboard_store(&self, _: &str, _: ClipboardTarget, _: &str) {}
    }

    let cases: &[(&str, &[u8], &[u8])] = &[
        (
            "alt-cursor",
            b"PRIMARY\x1b[2;4H\x1b[?1049hALT",
            b"\x1b[?1049lZ",
        ),
        (
            "saved-primary-cursor",
            b"A\x1b[2;4H\x1b7\x1b[4;7HX",
            b"\x1b8Z",
        ),
        (
            "saved-alt-cursor",
            b"PRIMARY\x1b[2;4H\x1b[?1049h\x1b[2;3H\x1b7\x1b[3;4HX",
            b"\x1b8Z",
        ),
        (
            "alt-csi",
            b"PRIMARY\x1b[2;4H\x1b[?1049hALT\x1b[3",
            b"b\x1b[?1049lZ",
        ),
        (
            "alt-osc",
            b"PRIMARY\x1b[2;4H\x1b[?1049hALT\x1b]2;partial",
            b"title\x07\x1b[?1049lZ",
        ),
        (
            "alt-dcs",
            b"PRIMARY\x1b[2;4H\x1b[?1049hALT\x1bPqpartial",
            b"rest\x1b\\\x1b[?1049lZ",
        ),
    ];
    for (name, init, follow) in cases {
        let model = TerminalModel::new(
            Arc::new(Host),
            format!("companion-{name}"),
            8,
            4,
            ModelOptions::default(),
        )
        .unwrap();
        model.feed_output(1, init);
        let vt_snap = model.snapshot_with_history(1, 0);
        assert!(!vt_snap.unavailable, "{name} snapshot unavailable");
        let mut remote = crate::replay::Terminal::from_snapshot(&vt_snap).unwrap();
        assert_eq!(
            remote.screen().visible_lines(),
            model.visible_lines(),
            "{name} pre"
        );
        model.feed_output(2, follow);
        remote.advance(follow);
        assert_eq!(
            remote.screen().visible_lines(),
            model.visible_lines(),
            "{name} post"
        );
        for w in model.stop_workers() {
            w.join().unwrap();
        }
    }

    // Two Kitty bit-0 pushes survive companion attach on both screens: one pop
    // keeps DISAMBIGUATE_ESC_CODES on both host and remote; second pop returns legacy.
    let model = TerminalModel::new(
        Arc::new(Host),
        "companion-kitty-steps".into(),
        20,
        4,
        ModelOptions::default(),
    )
    .unwrap();
    model.feed_output(1, b"\x1b[>31u\x1b[>1u\x1b[?1049h\x1b[>1u\x1b[>1u");
    let vt_snap = model.snapshot_with_history(1, 0);
    let mut remote = crate::replay::Terminal::from_snapshot(&vt_snap).unwrap();
    let expected = [
        (2u64, b"\x1b[<u".as_slice(), true),
        (3, b"\x1b[<u", false),
        (4, b"\x1b[?1049l\x1b[<u", true),
        (5, b"\x1b[<u", false),
    ];
    for (seq, bytes, expect_kitty) in expected {
        model.feed_output(seq, bytes);
        remote.advance(bytes);
        assert_eq!(
            model
                .mode()
                .contains(TerminalModeFlags::DISAMBIGUATE_ESC_CODES),
            expect_kitty,
            "host mode at seq {seq}"
        );
        assert_eq!(
            remote
                .mode()
                .contains(TerminalModeFlags::DISAMBIGUATE_ESC_CODES),
            expect_kitty,
            "remote mode at seq {seq}"
        );
        assert_eq!(
            crate::mappings::encode_key("a", true, false, true, Some("a"), remote.mode()),
            crate::mappings::encode_key("a", true, false, true, Some("a"), model.mode()),
            "key encoding parity at seq {seq}"
        );
    }
    for w in model.stop_workers() {
        w.join().unwrap();
    }
}

#[test]
fn scroll_mark_counts_non_lf_alt_screen_indices_identically_in_one_or_many_writes() {
    let (mirror, _tx, _) = scripted_mirror(false);
    mirror.resize_local(20, 4);
    mirror.test_feed(1, b"\x1b[?1049h\x1b[4;1H");
    let mark = mirror.scroll_mark();
    mirror.test_feed(2, &b"\x1bD".repeat(12));
    assert_eq!(mark.moved_since(mirror.scroll_mark()), Some(12));

    let mark = mirror.scroll_mark();
    for seq in 3..15 {
        mirror.test_feed(seq, b"\x1bD");
    }
    assert_eq!(mark.moved_since(mirror.scroll_mark()), Some(12));

    let mark = mirror.scroll_mark();
    mirror.test_feed(15, &b"\x1bE".repeat(10));
    assert_eq!(mark.moved_since(mirror.scroll_mark()), Some(10));
}

#[test]
fn push_mailbox_bounds_backlog_to_single_resync_and_services_deadlines_under_traffic() {
    use runner_core::protocol::terminal::{
        FrameSink, TerminalAttachment, TerminalSnapshot, TerminalSubscription,
    };
    use runner_core::protocol::{ClientError, EventSubscription, Request, Response, Transport};

    type SharedFrameSink = Arc<dyn Fn(Option<TerminalFrame>) -> bool + Send + Sync>;

    struct PushQueue {
        sink: Mutex<Option<SharedFrameSink>>,
    }
    impl PushQueue {
        fn new() -> Self {
            Self {
                sink: Mutex::new(None),
            }
        }
    }
    struct PushFrames(Arc<PushQueue>);
    impl TerminalSubscription for PushFrames {
        fn recv(&mut self) -> std::result::Result<TerminalFrame, ClientError> {
            Err(ClientError::msg("push-only"))
        }
        fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
            Arc::new(|| {})
        }
        fn install_sink(&mut self, sink: FrameSink) -> bool {
            *self.0.sink.lock().unwrap() = Some(Arc::from(sink));
            true
        }
    }
    struct SlowMetadataTransport {
        gate: Mutex<Option<(Sender<()>, mpsc::Receiver<()>)>>,
        attaches: AtomicUsize,
        queues: Mutex<Vec<Arc<PushQueue>>>,
    }
    impl Transport for SlowMetadataTransport {
        fn call(&self, _: Request) -> std::result::Result<Response, ClientError> {
            let gate = self.gate.lock().unwrap().take();
            if let Some((entered, release)) = gate {
                let _ = entered.send(());
                let _ = release.recv_timeout(Duration::from_secs(5));
            }
            Ok(Response::terminal_metadata(Ok(TerminalMetadata::default())))
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            unreachable!()
        }
        fn attach(&self, _: &str) -> std::result::Result<TerminalAttachment, ClientError> {
            let n = self.attaches.fetch_add(1, Ordering::SeqCst);
            let q = Arc::new(PushQueue::new());
            self.queues.lock().unwrap().push(Arc::clone(&q));
            let (seq, bytes) = if n == 0 {
                (0, Vec::new())
            } else {
                (200, b"RECOVERED_AFTER_FLOOD".to_vec())
            };
            Ok(TerminalAttachment {
                subscriber_id: (n + 1) as u64,
                snapshot: TerminalSnapshot {
                    seq,
                    format: SnapshotFormat::Vt,
                    unavailable: false,
                    cols: 30,
                    rows: 4,
                    bytes,
                    unfinished_len: 0,
                    preceding_char: None,
                    activity: None,
                    controller: Default::default(),
                },
                frames: Box::new(PushFrames(q)),
            })
        }
    }

    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let transport = Arc::new(SlowMetadataTransport {
        gate: Mutex::new(None),
        attaches: AtomicUsize::new(0),
        queues: Mutex::new(Vec::new()),
    });
    let mirror = TerminalMirror::attach(
        DaemonClient::new(transport.clone()),
        "push-bounded".into(),
        Arc::new(|| {}),
    )
    .unwrap();
    let _view = mirror.view();

    // Arm the slow metadata gate so the consumer blocks on the first frame,
    // then flood > TERMINAL_QUEUE_CAPACITY frames non-blockingly into the sink.
    *transport.gate.lock().unwrap() = Some((entered_tx, release_rx));
    let sink = transport.queues.lock().unwrap()[0]
        .sink
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    assert!(sink(Some(TerminalFrame::Output {
        seq: 1,
        bytes: b"FIRST".to_vec(),
    })));
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();

    let mut overflowed_at = None;
    for seq in 2..=(TERMINAL_QUEUE_CAPACITY as u64 + 16) {
        if !sink(Some(TerminalFrame::Output {
            seq,
            bytes: b"flood".to_vec(),
        })) {
            overflowed_at = Some(seq);
            break;
        }
    }
    assert!(
        overflowed_at.is_some(),
        "bounded push mailbox must refuse frames past TERMINAL_QUEUE_CAPACITY"
    );
    release_tx.send(()).unwrap();
    assert!(wait_until(Duration::from_secs(2), || {
        mirror.output_activity().last_seq == 200
            && mirror.visible_lines()[0] == "RECOVERED_AFTER_FLOOD"
    }));
    assert_eq!(transport.attaches.load(Ordering::SeqCst), 2);

    // Under sustained output traffic on a held DEC 2026 frame, the hold deadline
    // must flush before draining the remaining queued backlog. Arm Gate 1 BEFORE
    // sending the frame that starts the hold so the actor freezes inside
    // `refresh_metadata()` on seq 201 and the hold cannot expire before we read
    // its deadline or enqueue the backlog.
    assert!(wait_until(Duration::from_secs(2), || {
        transport
            .queues
            .lock()
            .unwrap()
            .get(1)
            .is_some_and(|q| q.sink.lock().unwrap().is_some())
    }));
    let sink2 = transport.queues.lock().unwrap()[1]
        .sink
        .lock()
        .unwrap()
        .clone()
        .unwrap();
    let (gate1_entered_tx, gate1_entered_rx) = mpsc::channel();
    let (gate1_release_tx, gate1_release_rx) = mpsc::channel();
    *transport.gate.lock().unwrap() = Some((gate1_entered_tx, gate1_release_rx));
    assert!(sink2(Some(TerminalFrame::Output {
        seq: 201,
        bytes: b"\r\n\x1b[?2026hHELD".to_vec(),
    })));
    gate1_entered_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    assert_eq!(mirror.output_activity().last_seq, 201);
    let deadline = mirror
        .sync_state()
        .0
        .expect("hold deadline must be live while actor is frozen at Gate 1");

    const BACKLOG_END: u64 = 240;
    for seq in 202..=BACKLOG_END {
        assert!(sink2(Some(TerminalFrame::Output {
            seq,
            bytes: b".".to_vec(),
        })));
    }

    // Wait for the real hold deadline to become overdue while the actor is held
    // at Gate 1 with `seq 202..=240` queued ahead of it.
    let remaining = deadline.saturating_duration_since(Instant::now());
    if !remaining.is_zero() {
        let (_never_tx, wait_rx) = mpsc::channel::<()>();
        let _ = wait_rx.recv_timeout(remaining + Duration::from_millis(5));
    }

    // Arm Gate 2 on the next `refresh_metadata()` call (after at most one
    // PUSH_DRAIN_SLICE of backlog) and release Gate 1. The actor must flush the
    // expired hold BEFORE draining the backlog, so when Gate 2 catches it
    // mid-backlog, `HELD` is already published and `last_seq < BACKLOG_END`.
    let (gate2_entered_tx, gate2_entered_rx) = mpsc::channel();
    let (gate2_release_tx, gate2_release_rx) = mpsc::channel();
    *transport.gate.lock().unwrap() = Some((gate2_entered_tx, gate2_release_rx));
    gate1_release_tx.send(()).unwrap();
    gate2_entered_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap();

    assert!(
        mirror.sync_state().0.is_none(),
        "hold deadline must flush while backlog is still queued"
    );
    assert!(mirror.visible_lines().iter().any(|l| l.contains("HELD")));
    let mid_seq = mirror.output_activity().last_seq;
    assert!(
        mid_seq < BACKLOG_END,
        "expected hold release before backlog drained (last_seq={mid_seq}, end={BACKLOG_END})"
    );

    gate2_release_tx.send(()).unwrap();
    assert!(wait_until(Duration::from_secs(2), || {
        mirror.output_activity().last_seq == BACKLOG_END
    }));
    drop(mirror);
}

#[test]
fn scroll_mark_is_split_invariant_across_su_rep_dl_alt_and_zero_scrollback() {
    fn run_split(
        setup: &[u8],
        payload: &[u8],
        split: usize,
        cols: u16,
        rows: u16,
        scrollback: usize,
    ) -> (Option<i32>, Option<i32>) {
        let (mirror, _tx, _) = scripted_mirror(false);
        mirror.configure(scrollback, CursorShape::Block, false);
        mirror.resize_local(cols, rows);
        if !setup.is_empty() {
            mirror.test_feed(1, setup);
        }
        let start = mirror.scroll_mark();
        mirror.test_feed(2, &payload[..split]);
        let after_first = start.moved_since(mirror.scroll_mark());
        mirror.test_feed(3, &payload[split..]);
        let total = start.moved_since(mirror.scroll_mark());
        (after_first, total)
    }

    // 1. Six `CSI 2 S` on 20×4 alternate screen AND on primary with scrollback=0:
    //    always exactly 12 rows at every byte split offset.
    let six_su2 = b"\x1b[2S".repeat(6);
    for (setup, sb) in [
        (b"\x1b[?1049h\x1b[4;1H".as_slice(), 100usize),
        (b"\x1b[4;1H".as_slice(), 0usize),
    ] {
        for split in 0..=six_su2.len() {
            let (_, total) = run_split(setup, &six_su2, split, 20, 4, sb);
            assert_eq!(total, Some(12), "SU 2 x6 split={split} sb={sb}");
        }
    }

    // 2. `CSI 100 b` (REP) past a 4-row screen (1x = 5 rows, 6x = 30 rows) on
    //    alternate screen and zero-scrollback primary at every byte split offset.
    for (count, expected) in [(1usize, 5i32), (6usize, 30i32)] {
        let rep = b"\x1b[100b".repeat(count);
        for (setup, sb) in [
            (b"\x1b[?1049h\x1b[4;1Hx".as_slice(), 100usize),
            (b"\x1b[4;1Hx".as_slice(), 0usize),
        ] {
            for split in 0..=rep.len() {
                let (_, total) = run_split(setup, &rep, split, 20, 4, sb);
                assert_eq!(
                    total,
                    Some(expected),
                    "REP 100 x{count} split={split} sb={sb}"
                );
            }
        }
    }

    // 3. `DL` (`CSI n M`) at row 0 (top of region) counts exact deleted rows,
    //    while `DL` below row 0 does not move row 0 off the top.
    let dl_mix = b"\x1b[1;1H\x1b[2M\x1b[2;1H\x1b[2M\x1b[1;1H\x1b[1M";
    for (setup, sb) in [
        (b"".as_slice(), 0usize),
        (b"\x1b[?1049h".as_slice(), 100usize),
    ] {
        for split in 0..=dl_mix.len() {
            let (_, total) = run_split(setup, dl_mix, split, 20, 4, sb);
            assert_eq!(total, Some(3), "DL mix split={split} sb={sb}");
        }
    }

    // 4. Temporary alternate-screen excursion is split-invariant at every byte
    //    offset: primary marks are incomparable (`None`) while alternate is
    //    active, and report the exact same surviving primary mark and 5 rows
    //    once the same primary screen returns.
    let prefix = b"\x1b[4;1H\n\n";
    let alt_enter = b"\x1b[?1049h";
    let alt_body = b"\x1b[4;1H\x1b[2S\x1b[2S\x1b[2S";
    let alt_exit = b"\x1b[?1049l";
    let suffix = b"\x1b[4;1H\n\n\n";
    let mut excursion_stream = Vec::new();
    excursion_stream.extend_from_slice(prefix);
    excursion_stream.extend_from_slice(alt_enter);
    excursion_stream.extend_from_slice(alt_body);
    excursion_stream.extend_from_slice(alt_exit);
    excursion_stream.extend_from_slice(suffix);
    let alt_active_start = prefix.len() + alt_enter.len();
    let alt_active_end = alt_active_start + alt_body.len() + alt_exit.len();
    for sb in [0usize, 100] {
        // Whole stream in one write: 2 + 3 = 5 primary rows; 6 alt rows excluded.
        let (_, total) = run_split(b"", &excursion_stream, 0, 20, 4, sb);
        assert_eq!(total, Some(5), "alt excursion whole sb={sb}");
        for split in 0..=excursion_stream.len() {
            let (after_first, total) = run_split(b"", &excursion_stream, split, 20, 4, sb);
            if split >= alt_active_start && split < alt_active_end {
                assert_eq!(
                    after_first, None,
                    "primary mark must be incomparable while alternate is active (split={split}, sb={sb})"
                );
            }
            assert_eq!(
                total,
                Some(5),
                "surviving primary mark must be split-invariant at split={split} sb={sb}"
            );
        }
        // Ending on alternate screen keeps primary marks incomparable.
        let (_, end_alt) = run_split(b"", b"\x1b[4;1H\n\x1b[?1049h\x1b[2S", 0, 20, 4, sb);
        assert_eq!(end_alt, None, "end on alt sb={sb}");
    }

    // 4b. A held primary viewport stays pinned by the exact same 5 rows across
    //     a temporary alternate-screen excursion at every byte split offset.
    for split in 0..=excursion_stream.len() {
        let (mirror, _tx, _) = scripted_mirror(false);
        mirror.configure(100, CursorShape::Block, false);
        mirror.resize_local(20, 4);
        let _hold = mirror.hold_viewport();
        let origin = mirror.scroll_mark();
        mirror.test_feed(1, &excursion_stream[..split]);
        mirror.test_feed(2, &excursion_stream[split..]);
        assert_eq!(
            origin.moved_since(mirror.scroll_mark()),
            Some(5),
            "held excursion scroll mark mismatch at split={split}"
        );
        assert_eq!(
            mirror.display_offset(),
            5,
            "held primary viewport offset mismatch at split={split}"
        );
    }

    // 5. Physical screen reset / replacement (`RIS`, ESC c) starts a fresh
    //    generation for both primary and a freshly reallocated alternate screen
    //    rather than subtracting unrelated counters.
    {
        let (mirror, _tx, _) = scripted_mirror(false);
        mirror.resize_local(20, 4);
        mirror.test_feed(1, b"\x1b[4;1H\n\n");
        let pri_before_ris = mirror.scroll_mark();
        mirror.test_feed(2, b"\x1b[?1049h\x1b[2S");
        let alt_before_ris = mirror.scroll_mark();
        assert_eq!(pri_before_ris.moved_since(alt_before_ris), None);

        // Single write: RIS destroys alternate and resets primary, then enters a
        // brand-new alternate screen whose native counter starts at 0.
        mirror.test_feed(3, b"\x1bc\x1b[?1049h");
        let alt_after_ris = mirror.scroll_mark();
        assert_eq!(
            alt_before_ris.moved_since(alt_after_ris),
            None,
            "replaced alternate screen must get a new epoch instead of subtracting 0 - 2"
        );
        assert_eq!(alt_after_ris.lines, 0);
        mirror.test_feed(4, b"\x1b[2S");
        assert_eq!(alt_after_ris.moved_since(mirror.scroll_mark()), Some(2));

        // Returning to the reset primary screen also has a new epoch relative
        // to the pre-RIS primary mark.
        mirror.test_feed(5, b"\x1b[?1049l");
        let pri_after_ris = mirror.scroll_mark();
        assert_eq!(pri_before_ris.moved_since(pri_after_ris), None);
        mirror.test_feed(6, b"\x1b[4;1H\n\n\n");
        assert_eq!(pri_after_ris.moved_since(mirror.scroll_mark()), Some(3));
    }
}

#[test]
fn companion_vt_preserves_saved_pending_wrap_charsets_overline_and_primary_origin() {
    struct Host;
    impl TerminalHost for Host {
        fn write_input(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn write_reply(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn report_input_state(&self, _: &str, _: InputObservation) {}
        fn set_live_title(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn resize(&self, _: &str, _: u16, _: u16) -> Result<()> {
            Ok(())
        }
        fn is_local_host(&self, _: &str) -> bool {
            true
        }
        fn clipboard_store(&self, _: &str, _: ClipboardTarget, _: &str) {}
    }

    let cases: &[(&str, &[u8], &[u8])] = &[
        ("saved-wrap", b"12345678\x1b7\x1b[3;2H", b"\x1b8Z"),
        (
            "saved-wide-wrap",
            b"123456\xe7\x95\x8c\x1b7\x1b[3;2H",
            b"\x1b8Z",
        ),
        (
            "saved-wrap-alt",
            b"\x1b[?1049h12345678\x1b7\x1b[3;2H",
            b"\x1b8Z",
        ),
        (
            "cursor-wrap-park",
            b"12345678\x1b[?1049hALT",
            b"\x1b[?1049lZ",
        ),
        (
            "cursor-wrap-park-irm",
            b"12345678\x1b[?1049hALT\x1b[4h",
            b"\x1b[2;2HX",
        ),
        (
            "primary-origin-after-alt",
            b"1234\x1b[2;3r\x1b[?6h\x1b[2;3H\x1b[?1049hALT",
            b"\x1b[?1049lZ",
        ),
        (
            "primary-origin-changed-while-alt",
            b"AB\x1b[1;2r\x1b[?6h\x1b[1;1H\x1b[?1049hALT\x1b[3;4r",
            b"\x1b[?1049lZ",
        ),
        (
            "saved-origin-margins-changed-primary",
            b"AB\x1b[1;2r\x1b[?6h\x1b[1;1H\x1b7\x1b[3;4r",
            b"\x1b8Z",
        ),
        (
            "saved-link",
            b"12345678\x1b7\x1b[2;1H\x1b]8;id=after;https://example.org/a\x1b\\",
            b"L\x1b8Z",
        ),
        (
            "saved-bg",
            b"\x1b[42m12345678\x1b7\x1b[2;1H\x1b[0m",
            b"\x1b8Z",
        ),
        (
            "saved-empty-bg",
            b"\x1b[42m12345678\x1b7\x1b[1;8H\x1b[X\x1b[2;1H",
            b"\x1b8Z",
        ),
        (
            "saved-protected",
            b"\x1b[1\"q12345678\x1b7\x1b[2;1H\x1b[0\"q",
            b"\x1b8Z",
        ),
        ("saved-DEC-line", b"\x1b(0\x1b7\x1b(B", b"\x1b8qx"),
        ("saved-ascii", b"\x1b(B\x1b7\x1b(0", b"\x1b8qx"),
        ("active-DEC", b"\x1b7\x1b(0", b"qx"),
        ("saved-british", b"\x1b(A\x1b7\x1b(B", b"\x1b8#"),
    ];

    for &(name, init, follow) in cases {
        let model =
            TerminalModel::new(Arc::new(Host), name.into(), 8, 4, ModelOptions::default()).unwrap();
        model.feed_output(1, init);
        let bin_snap = model.snapshot(1);
        let vt_snap = model.snapshot_with_history(1, 0);
        let mut reference = crate::replay::Terminal::from_snapshot(&bin_snap).unwrap();
        let mut remote = crate::replay::Terminal::from_snapshot(&vt_snap).unwrap();
        assert_eq!(
            remote.snapshot_state().active_grid.rows,
            reference.snapshot_state().active_grid.rows,
            "{name}: pre-follow cell attributes mismatch"
        );
        if name == "cursor-wrap-park" || name == "cursor-wrap-park-irm" {
            let first = GridPoint::new(Line(0), Column(0));
            let last = GridPoint::new(Line(1), Column(2));
            assert_eq!(
                remote.bounds_to_string(first, last),
                reference.bounds_to_string(first, last),
                "{name}: pre-follow copy parity across blank soft wrap"
            );
            assert_eq!(remote.bounds_to_string(first, last), "        ALT");
        }
        assert_eq!(
            remote.screen().visible_lines(),
            model.visible_lines(),
            "{name}: pre-follow lines mismatch"
        );
        model.feed_output(2, follow);
        reference.advance(follow);
        remote.advance(follow);
        assert_eq!(
            remote.screen().visible_lines(),
            model.visible_lines(),
            "{name}: post-follow lines mismatch"
        );
        if name == "cursor-wrap-park-irm" {
            assert_eq!(
                remote.screen().visible_lines(),
                vec!["", "AXLT", "", ""],
                "IRM must remain active so mid-row insertion shifts existing cells"
            );
        }
        assert_eq!(
            remote.snapshot_state().active_grid.rows,
            reference.snapshot_state().active_grid.rows,
            "{name}: post-follow cell attributes mismatch"
        );
        for w in model.stop_workers() {
            w.join().unwrap();
        }
    }

    // Verify protected cell bit survives pending-wrap replay independently of active DECSCA.
    {
        let model = TerminalModel::new(
            Arc::new(Host),
            "saved-protected-bit".into(),
            8,
            4,
            ModelOptions::default(),
        )
        .unwrap();
        model.feed_output(1, b"\x1b[1\"q12345678\x1b7\x1b[2;1H\x1b[0\"q");
        let vt_snap = model.snapshot_with_history(1, 0);
        let mut remote_vt = crate::vt::Terminal::new(&crate::vt::TerminalConfig {
            cols: 8,
            rows: 4,
            ..crate::vt::TerminalConfig::default()
        })
        .unwrap();
        remote_vt.vt_write(&vt_snap.bytes);
        let cell = remote_vt
            .grid_ref(crate::vt::GridPointTag::Active, 7, 0)
            .unwrap()
            .cell()
            .unwrap();
        assert!(
            cell.protected,
            "margin cell must retain its DECSCA protected bit after pending-wrap replay"
        );
        // Active DECSCA remains unprotected (`0"q`) for subsequent writes before DECRC.
        remote_vt.vt_write(b"U");
        let active_cell = remote_vt
            .grid_ref(crate::vt::GridPointTag::Active, 0, 1)
            .unwrap()
            .cell()
            .unwrap();
        assert!(!active_cell.protected);
        for w in model.stop_workers() {
            w.join().unwrap();
        }
    }

    // Verify SGR 53 (OVERLINE) round-trips on both saved and active pens.
    let model = TerminalModel::new(
        Arc::new(Host),
        "overline-pen".into(),
        8,
        4,
        ModelOptions::default(),
    )
    .unwrap();
    model.feed_output(1, b"\x1b[53m\x1b7\x1b[0m\x1b[53mA");
    let vt_snap = model.snapshot_with_history(1, 0);
    let mut remote_vt = crate::vt::Terminal::new(&crate::vt::TerminalConfig {
        cols: 8,
        rows: 4,
        ..crate::vt::TerminalConfig::default()
    })
    .unwrap();
    remote_vt.vt_write(&vt_snap.bytes);
    assert_ne!(
        remote_vt.cursor_sgr_style().flags & crate::vt::STYLE_FLAG_OVERLINE,
        0,
        "active overline pen should round-trip through companion VT snapshot"
    );
    remote_vt.vt_write(b"\x1b[0m\x1b8");
    assert_ne!(
        remote_vt.cursor_sgr_style().flags & crate::vt::STYLE_FLAG_OVERLINE,
        0,
        "saved overline pen should round-trip through companion VT snapshot"
    );
    for w in model.stop_workers() {
        w.join().unwrap();
    }
}

#[test]
fn recv_based_mirror_drop_cancels_idle_reader_and_avoids_self_join_deadlock() {
    use runner_core::protocol::terminal::{
        TerminalAttachment, TerminalSnapshot, TerminalSubscription,
    };
    use runner_core::protocol::{ClientError, EventSubscription, Request, Response, Transport};

    struct BlockingRecvFrames {
        rx: mpsc::Receiver<std::result::Result<TerminalFrame, ClientError>>,
        tx: mpsc::Sender<std::result::Result<TerminalFrame, ClientError>>,
        cancels: Arc<AtomicUsize>,
        entered: mpsc::Sender<()>,
        exited: Arc<AtomicBool>,
    }
    impl Drop for BlockingRecvFrames {
        fn drop(&mut self) {
            self.exited.store(true, Ordering::SeqCst);
        }
    }
    impl TerminalSubscription for BlockingRecvFrames {
        fn recv(&mut self) -> std::result::Result<TerminalFrame, ClientError> {
            let _ = self.entered.send(());
            self.rx
                .recv()
                .unwrap_or_else(|_| Err(ClientError::msg("closed")))
        }
        fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
            let tx = self.tx.clone();
            let cancels = Arc::clone(&self.cancels);
            Arc::new(move || {
                cancels.fetch_add(1, Ordering::SeqCst);
                let _ = tx.send(Err(ClientError::msg("cancelled")));
            })
        }
    }
    struct RecvTransport {
        frames: Mutex<Option<BlockingRecvFrames>>,
        on_metadata: Mutex<Option<Box<dyn FnMut() + Send>>>,
    }
    impl Transport for RecvTransport {
        fn call(&self, _: Request) -> std::result::Result<Response, ClientError> {
            if let Some(cb) = self.on_metadata.lock().unwrap().as_mut() {
                cb();
            }
            Ok(Response::terminal_metadata(Ok(TerminalMetadata::default())))
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            unreachable!()
        }
        fn attach(&self, _: &str) -> std::result::Result<TerminalAttachment, ClientError> {
            Ok(TerminalAttachment {
                subscriber_id: 1,
                snapshot: TerminalSnapshot {
                    seq: 0,
                    format: SnapshotFormat::Vt,
                    unavailable: false,
                    cols: 20,
                    rows: 4,
                    bytes: Vec::new(),
                    unfinished_len: 0,
                    preceding_char: None,
                    activity: None,
                    controller: Default::default(),
                },
                frames: Box::new(self.frames.lock().unwrap().take().unwrap()),
            })
        }
    }

    // 1. Idle drop cancels a blocked recv-based reader and joins it cleanly.
    let (tx, rx) = mpsc::channel();
    let (entered_tx, entered_rx) = mpsc::channel();
    let cancels = Arc::new(AtomicUsize::new(0));
    let exited = Arc::new(AtomicBool::new(false));
    let mirror = TerminalMirror::attach(
        DaemonClient::new(Arc::new(RecvTransport {
            frames: Mutex::new(Some(BlockingRecvFrames {
                rx,
                tx,
                cancels: Arc::clone(&cancels),
                entered: entered_tx,
                exited: Arc::clone(&exited),
            })),
            on_metadata: Mutex::new(None),
        })),
        "recv-idle-drop".into(),
        Arc::new(|| {}),
    )
    .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    drop(mirror);
    assert_eq!(cancels.load(Ordering::SeqCst), 1);
    assert!(exited.load(Ordering::SeqCst));

    // 2. Dropping the final Arc on the reader thread itself does not self-join deadlock.
    let (tx, rx) = mpsc::channel();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (meta_entered_tx, meta_entered_rx) = mpsc::channel();
    let (meta_release_tx, meta_release_rx) = mpsc::channel();
    let cancels = Arc::new(AtomicUsize::new(0));
    let exited = Arc::new(AtomicBool::new(false));
    let transport = Arc::new(RecvTransport {
        frames: Mutex::new(Some(BlockingRecvFrames {
            rx,
            tx: tx.clone(),
            cancels: Arc::clone(&cancels),
            entered: entered_tx,
            exited: Arc::clone(&exited),
        })),
        on_metadata: Mutex::new(None),
    });
    let mirror = TerminalMirror::attach(
        DaemonClient::new(transport.clone()),
        "recv-self-drop".into(),
        Arc::new(|| {}),
    )
    .unwrap();
    entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    *transport.on_metadata.lock().unwrap() = Some(Box::new(move || {
        let _ = meta_entered_tx.send(());
        let _ = meta_release_rx.recv_timeout(Duration::from_secs(5));
    }));
    tx.send(Ok(TerminalFrame::Output {
        seq: 1,
        bytes: b"hi".to_vec(),
    }))
    .unwrap();
    meta_entered_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    // Release external Arc while reader thread holds the only remaining Arc.
    drop(mirror);
    meta_release_tx.send(()).unwrap();
    assert!(wait_until(Duration::from_secs(2), || {
        exited.load(Ordering::SeqCst)
    }));
    assert_eq!(cancels.load(Ordering::SeqCst), 1);
}

#[test]
fn tracked_grid_ref_anchors_drag_selection_viewport_and_native_find_across_scroll_reflow_and_ris() {
    use crate::terminal::find::{FindAction, FindQuery};
    use crate::types::SelectionKind;

    let (mirror, _tx, _) = scripted_mirror(false);
    mirror.configure(500, CursorShape::Block, false);
    mirror.resize_local(20, 6);
    let _view = mirror.view();

    // 1. Selection drag anchor tracks scrollback via `TrackedGridRef` while output streams.
    let initial: String = (0..12).map(|i| format!("line-{i:02} alpha\r\n")).collect();
    mirror.test_feed(1, initial.as_bytes());
    let before_lines = mirror.visible_lines();
    assert!(before_lines[0].starts_with("line-07"), "{before_lines:?}");
    mirror.start_selection(
        SelectionKind::Simple,
        Point::new(Line(0), Column(0)),
        Side::Left,
    );
    // Scroll 4 more lines during an active drag; `anchor_ref` moves to Line(-4).
    mirror.test_feed(
        2,
        b"line-12 beta\r\nline-13 beta\r\nline-14 beta\r\nline-15 beta\r\n",
    );
    assert!(mirror.update_selection(Point::new(Line(0), Column(6)), Side::Right));
    let dragged = mirror.selection_text().expect("dragged selection text");
    assert!(
        dragged.starts_with("line-07"),
        "TrackedGridRef drag anchor should keep original start cell across scrollback: {dragged:?}"
    );
    assert!(dragged.contains("line-11"), "{dragged:?}");
    assert!(mirror.clear_selection());

    // 2. Viewport `GridAnchor` restores the exact scrolled line across output, reflow, and eviction.
    mirror.scroll_to_display_offset(6);
    let pinned_top_before = mirror.viewport_snapshot().view().row_to_string(Line(-6));
    let viewport_anchor = mirror.track_viewport_top().expect("viewport anchor");
    mirror.scroll_to_bottom();
    mirror.test_feed(3, b"extra-1\r\nextra-2\r\nextra-3\r\nextra-4\r\n");
    mirror.restore_viewport_anchor(&viewport_anchor);
    let offset_after_output = mirror.scroll_state().display_offset;
    assert_eq!(offset_after_output, 10);
    assert_eq!(
        mirror
            .viewport_snapshot()
            .view()
            .row_to_string(Line(-(offset_after_output as i32))),
        pinned_top_before
    );

    // 3. Native `GhosttySearch` + `MatchAnchor` track a wrapped match across scrollback and reflow.
    mirror.scroll_to_bottom();
    mirror.test_feed(
        4,
        b"first-target-hit\r\n0123456789abcdef target-wrap-end\r\nplain-row\r\nsecond-target-hit\r\n",
    );
    let query = FindQuery {
        text: "target".into(),
        match_case: false,
        regex: false,
    };
    let found = mirror.find_matches(&query, FindAction::NewQuery);
    assert_eq!(found.count.total, 3);
    assert_eq!(found.count.index, Some(0));
    let first_anchor = found.anchor.expect("match anchor");

    // Step older to the wrapped middle match (`target-wrap-end`).
    let stepped = mirror.find_matches(
        &query,
        FindAction::Step {
            anchor: Some(first_anchor.id()),
            carried: found.current,
            older: true,
        },
    );
    assert_eq!(stepped.count.index, Some(1));
    let middle_anchor = stepped.anchor.expect("middle match anchor");
    let middle_before = stepped.current.expect("middle match");

    // Narrow columns so the wrapped line reflows to a new line/column; `MatchAnchor`
    // follows the reflowed cells directly without `carried`.
    mirror.resize_local(12, 6);
    let reflowed = mirror.find_matches(
        &query,
        FindAction::Refresh {
            anchor: Some(middle_anchor.id()),
            carried: None,
        },
    );
    assert_eq!(reflowed.count.total, 3);
    assert_eq!(reflowed.count.index, Some(1));
    let middle_after = reflowed.current.expect("reflowed middle match");
    let text_after =
        mirror.with_screen(|s| s.bounds_to_string(*middle_after.start(), *middle_after.end()));
    assert_eq!(text_after, "target");
    assert_ne!(
        middle_after.start().column,
        middle_before.start().column,
        "narrowing should have reflowed the wrapped target column"
    );

    // 4. RIS invalidates tracked anchors cleanly without leaking or crashing on restore/drop.
    mirror.test_feed(5, b"\x1bcFresh after RIS\r\n");
    mirror.restore_viewport_anchor(&viewport_anchor);
    assert_eq!(mirror.scroll_state().display_offset, 0);
    drop(viewport_anchor);
    drop(first_anchor);
    drop(middle_anchor);
    mirror.clear_find();

    // 5. `link_at_with` resolves links in the viewport interior (zero-copy `viewport_snapshot`)
    //    and soft-wrapped links crossing the viewport's top edge (bounded `LinkWindow`).
    mirror.resize_local(40, 20);
    let mut url_stream = (0..25)
        .map(|i| format!("pad-{i:02}\r\n"))
        .collect::<String>();
    url_stream.push_str("open https://example.com/wrapped-across-viewport-boundary/path\r\n");
    for i in 0..12 {
        url_stream.push_str(&format!("mid-{i:02}\r\n"));
    }
    url_stream.push_str("visit https://example.com/in-view\r\n");
    for i in 0..8 {
        url_stream.push_str(&format!("tail-{i:02}\r\n"));
    }
    mirror.test_feed(6, url_stream.as_bytes());
    let snap = mirror.viewport_snapshot();
    let in_view_line = (0..snap.view().rows() as i32)
        .map(Line)
        .find(|&line| {
            snap.view()
                .row_to_string(line)
                .contains("https://example.com/in-view")
        })
        .expect("in-view url row");
    assert!(
        in_view_line.0 >= 8,
        "expected in-view URL in viewport interior (line={in_view_line:?})"
    );
    let in_view = mirror
        .link_at(Point::new(in_view_line, Column(10)))
        .expect("interior link via viewport_snapshot");
    assert_eq!(
        in_view.target,
        LinkTarget::Url("https://example.com/in-view".into())
    );
    // Scroll so the second half of the wrapped URL sits on the top visible row while
    // its `https://` prefix sits above the viewport boundary.
    let wrapped_tail_line = mirror.with_screen(|screen| {
        let view = screen.view();
        (view.topmost_line().0..=view.bottommost_line().0)
            .map(Line)
            .find(|&line| view.row_to_string(line).contains("viewport-boundary/path"))
            .expect("wrapped URL continuation row")
    });
    assert!(
        wrapped_tail_line.0 < 0,
        "expected wrapped URL continuation in scrollback (line={wrapped_tail_line:?})"
    );
    mirror.scroll_to_display_offset((-wrapped_tail_line.0) as usize);
    let wrapped = mirror
        .link_at(Point::new(wrapped_tail_line, Column(5)))
        .expect("wrapped link crossing viewport top edge");
    assert_eq!(
        wrapped.target,
        LinkTarget::Url("https://example.com/wrapped-across-viewport-boundary/path".into())
    );
}

#[test]
fn find_parity_overlaps_unicode_deep_navigation_and_unclipped_long_wrapped_links() {
    use crate::terminal::find::{FindAction, FindQuery, MATCH_LIMIT};
    use crate::terminal::LinkLookup;

    let (mirror, _tx, _) = scripted_mirror(false);
    mirror.configure(5_000, CursorShape::Block, false);
    mirror.resize_local(40, 4);
    let _view = mirror.view();

    // 1. Overlaps, Unicode case folding (K, ſ), and combining marks (é) preserve baseline
    //    counts, exact rightmost non-overlapping spans, and newest-first navigation.
    mirror.test_feed(1, "key Key ſk sk e\u{301}x ex\r\naaaaa\r\n".as_bytes());
    for (needle, expected_newest, expected_older) in [
        (
            "key",
            FindMatch::new(
                Point::new(Line(0), Column(4)),
                Point::new(Line(0), Column(6)),
            ),
            FindMatch::new(
                Point::new(Line(0), Column(0)),
                Point::new(Line(0), Column(2)),
            ),
        ),
        (
            "sk",
            FindMatch::new(
                Point::new(Line(0), Column(11)),
                Point::new(Line(0), Column(12)),
            ),
            FindMatch::new(
                Point::new(Line(0), Column(8)),
                Point::new(Line(0), Column(9)),
            ),
        ),
        (
            "ex",
            FindMatch::new(
                Point::new(Line(0), Column(17)),
                Point::new(Line(0), Column(18)),
            ),
            FindMatch::new(
                Point::new(Line(0), Column(14)),
                Point::new(Line(0), Column(15)),
            ),
        ),
        (
            "aa",
            FindMatch::new(
                Point::new(Line(1), Column(3)),
                Point::new(Line(1), Column(4)),
            ),
            FindMatch::new(
                Point::new(Line(1), Column(1)),
                Point::new(Line(1), Column(2)),
            ),
        ),
    ] {
        let query = FindQuery {
            text: needle.into(),
            match_case: false,
            regex: false,
        };
        let first = mirror.find_matches_in_session(1, &query, FindAction::NewQuery);
        assert_eq!(first.count.total, 2, "query={needle}");
        assert!(!first.count.more, "query={needle}");
        assert_eq!(first.count.index, Some(0), "query={needle}");
        assert_eq!(first.current, Some(expected_newest), "query={needle}");
        let snap = mirror.viewport_snapshot();
        assert_eq!(
            snap.find_highlights(1),
            Some([expected_older, expected_newest].as_slice()),
            "published highlights for query={needle}"
        );
        let older = mirror.find_matches_in_session(
            1,
            &query,
            FindAction::Step {
                anchor: first.anchor.as_ref().map(|a| a.id()),
                carried: Some(expected_newest),
                older: true,
            },
        );
        assert_eq!(older.count.index, Some(1), "query={needle}");
        assert_eq!(older.current, Some(expected_older), "query={needle}");
        // Stepping older again wraps back to newest.
        let wrapped = mirror.find_matches_in_session(
            1,
            &query,
            FindAction::Step {
                anchor: older.anchor.as_ref().map(|a| a.id()),
                carried: Some(expected_older),
                older: true,
            },
        );
        assert_eq!(wrapped.current, Some(expected_newest), "query={needle}");
    }
    mirror.clear_find_session(1);

    // 2. Long soft-wrapped URL (> 8 rows) resolves its full unclipped extent, while
    //    blank cells in the top rows with history resolve synchronously without offscreen lookup.
    mirror.test_feed(2, b"\x1bc");
    mirror.resize_local(20, 5);
    let long_url = format!("https://example.org/{}", "a".repeat(210));
    mirror.test_feed(3, long_url.as_bytes());
    let snap = mirror.viewport_snapshot();
    assert_eq!(
        mirror.try_link_at_published(
            &snap,
            Point::new(Line(2), Column(3)),
            &FileLinkContext::default()
        ),
        LinkLookup::NeedsOffscreen
    );
    let resolved = mirror
        .link_at(Point::new(Line(2), Column(3)))
        .expect("full 230-byte wrapped URL");
    assert_eq!(resolved.target, LinkTarget::Url(long_url));
    // Blank row in top visible rows with scrollback resolves synchronously as Ready(None).
    mirror.test_feed(4, b"\r\n\r\n\r\n\r\n\r\n");
    let blank_snap = mirror.viewport_snapshot();
    assert_eq!(
        mirror.try_link_at_published(
            &blank_snap,
            Point::new(Line(0), Column(0)),
            &FileLinkContext::default()
        ),
        LinkLookup::Ready(None)
    );

    // 3. Long soft-wrapped line publishes ONLY viewport-intersecting highlights (100 cells
    //    on 20×5, not all 2,000 offscreen hits), while count still caps at 1,000+.
    mirror.test_feed(5, b"\x1bc");
    mirror.resize_local(20, 5);
    mirror.test_feed(6, "a".repeat(2_000).as_bytes());
    let a_query = FindQuery {
        text: "a".into(),
        match_case: false,
        regex: false,
    };
    let wrapped_a = mirror.find_matches_in_session(2, &a_query, FindAction::NewQuery);
    assert_eq!(wrapped_a.count.total, MATCH_LIMIT);
    assert!(wrapped_a.count.more);
    let wrapped_snap = mirror.viewport_snapshot();
    let published_a = wrapped_snap.find_highlights(2).unwrap_or(&[]);
    assert_eq!(
        published_a.len(),
        100,
        "expected only 100 visible viewport hits, got {}",
        published_a.len()
    );
    assert!(published_a
        .iter()
        .all(|m| m.end.line >= Line(0) && m.start.line <= Line(4)));
    mirror.clear_find_session(2);

    // 4. Deep history (> MATCH_LIMIT matches) caps count at 1,000+ while still stepping
    //    beyond the 1,000th match into older scrollback.
    mirror.test_feed(7, b"\x1bc");
    mirror.resize_local(80, 24);
    let deep = ("a".repeat(60) + "\r\n").repeat(200);
    mirror.test_feed(8, deep.as_bytes());
    let head = mirror.find_matches(&a_query, FindAction::NewQuery);
    assert_eq!(head.count.total, MATCH_LIMIT);
    assert!(head.count.more);
    assert_eq!(head.count.index, Some(0));
    // Jump directly to a match deeper than 1,000 hits and verify stepping older still works.
    let deep_match = FindMatch::new(
        Point::new(Line(-100), Column(10)),
        Point::new(Line(-100), Column(10)),
    );
    let stepped = mirror.find_matches(
        &a_query,
        FindAction::Step {
            anchor: None,
            carried: Some(deep_match),
            older: true,
        },
    );
    assert_eq!(stepped.count.total, MATCH_LIMIT);
    assert!(stepped.count.more);
    assert_eq!(stepped.count.index, None);
    assert_eq!(
        stepped.current,
        Some(FindMatch::new(
            Point::new(Line(-100), Column(9)),
            Point::new(Line(-100), Column(9)),
        ))
    );
    // 5. Regex validity preserves baseline acceptance rules (e.g. Unicode word boundaries
    //    reject as Invalid just like main's hybrid DFA search).
    for invalid in [r"\btest\b", r"\b\w+\b", r"\bÉrr\b", r"t\b"] {
        let query = FindQuery {
            text: invalid.into(),
            match_case: false,
            regex: true,
        };
        assert!(
            matches!(query.pattern(), crate::terminal::find::FindPattern::Invalid),
            "expected Invalid for baseline-unsupported regex {invalid:?}"
        );
    }
}

#[test]
fn held_frame_find_highlights_keep_pre_hold_wrap_context_and_ignore_hidden_post_hold_text() {
    use crate::terminal::find::{FindAction, FindQuery};

    let (mirror, _tx, _) = scripted_mirror(false);
    mirror.configure(500, CursorShape::Block, false);
    mirror.resize_local(8, 3);
    let _view = mirror.view();

    let xxyy = FindQuery {
        text: "XXYY".into(),
        match_case: false,
        regex: false,
    };
    let hidden = FindQuery {
        text: "hidden".into(),
        match_case: false,
        regex: false,
    };

    // Publish pre-hold scrollback + wrapped XXYY + visible XXYY before the hold opens.
    mirror.test_feed(1, b"XXYY\r\naaaaaaXXYYbbbbbb\r\nmid\r\nXXYY");
    let pre_hold = mirror.find_matches_in_session(1, &xxyy, FindAction::NewQuery);
    let expected_visible = [
        FindMatch::new(
            Point::new(Line(-1), Column(6)),
            Point::new(Line(0), Column(1)),
        ),
        FindMatch::new(
            Point::new(Line(2), Column(0)),
            Point::new(Line(2), Column(3)),
        ),
    ];
    assert_eq!(pre_hold.count.total, 3);
    assert_eq!(pre_hold.count.index, Some(0));
    assert_eq!(pre_hold.current, Some(expected_visible[1]));

    // Opening a DEC 2026 hold that clears the screen and writes hidden text
    // keeps the published pre-hold frame and highlights immutable until release.
    mirror.test_feed(2, b"\x1b[?2026h\x1b[2J\x1b[Hhidden");
    assert!(mirror.is_render_held());
    let held_snap = mirror.viewport_snapshot();
    assert_eq!(held_snap.view().row_to_string(Line(0)), "YYbbbbbb");
    assert_eq!(held_snap.view().row_to_string(Line(1)), "mid");
    assert_eq!(held_snap.view().row_to_string(Line(2)), "XXYY");
    assert_eq!(
        held_snap.find_highlights(1),
        Some(expected_visible.as_slice()),
        "held frame must keep pre-hold published highlights with wrap context"
    );

    // Querying or stepping during the hold returns the last published state
    // promptly without touching the live grid and queues the interaction for release.
    let during_hold = mirror.find_matches_in_session(1, &xxyy, FindAction::NewQuery);
    assert_eq!(during_hold.count.total, 3);
    assert_eq!(during_hold.count.index, Some(0));
    assert_eq!(during_hold.current, Some(expected_visible[1]));
    assert_eq!(during_hold.mark, mirror.scroll_mark());
    assert_eq!(
        mirror.viewport_snapshot().find_highlights(1),
        Some(expected_visible.as_slice()),
        "find during hold must keep published pre-hold highlights unchanged"
    );

    // Searching post-hold text in a new session during the hold returns promptly
    // with zero hits and never leaks hidden post-hold text before release.
    let hidden_during_hold = mirror.find_matches_in_session(2, &hidden, FindAction::NewQuery);
    assert_eq!(hidden_during_hold.count.total, 0);
    assert_eq!(hidden_during_hold.current, None);
    assert_eq!(
        mirror.viewport_snapshot().find_highlights(2),
        None,
        "held frame highlights must never leak hidden post-hold text"
    );

    // Releasing the hold publishes the live post-hold frame and executes queued
    // find queries once on the live grid.
    mirror.test_feed(3, b"\x1b[?2026l");
    assert!(!mirror.is_render_held());
    let expected_hidden = FindMatch::new(
        Point::new(Line(0), Column(0)),
        Point::new(Line(0), Column(5)),
    );
    let released = mirror.viewport_snapshot();
    assert_eq!(released.view().row_to_string(Line(0)), "hidden");
    assert_eq!(
        released.find_highlights(2),
        Some([expected_hidden].as_slice())
    );
    let released_find = mirror.find_matches_in_session(
        2,
        &hidden,
        FindAction::Refresh {
            anchor: None,
            carried: None,
        },
    );
    assert_eq!(released_find.count.total, 1);
    assert_eq!(released_find.current, Some(expected_hidden));

    // Local scrolling during a hold stays on the published snapshot while held
    // and applies cleanly upon hold release (Decision 18).
    let (scroll_mirror, _stx, _) = scripted_mirror(false);
    scroll_mirror.configure(500, CursorShape::Block, false);
    scroll_mirror.resize_local(12, 3);
    let _scroll_view = scroll_mirror.view();
    let lines = (0..20)
        .map(|i| format!("row{i:02} needle\r\n"))
        .collect::<String>();
    scroll_mirror.test_feed(1, lines.as_bytes());
    scroll_mirror.test_feed(2, b"\x1b[?2026h\rhidden");
    scroll_mirror.scroll_to_display_offset(8);
    assert_eq!(
        scroll_mirror.scroll_state().display_offset,
        0,
        "published viewport stays frozen while held"
    );
    scroll_mirror.test_feed(3, b"\x1b[?2026l");
    assert_eq!(scroll_mirror.scroll_state().display_offset, 8);
    assert_eq!(
        scroll_mirror
            .viewport_snapshot()
            .view()
            .row_to_string(Line(-8)),
        "row10 needle"
    );

    // A query + step queued during a hold executes once on release, creates a
    // live TrackedGridRef anchor, and survives resize reflow (26 -> 24 columns)
    // retaining the exact same occurrence (index 1).
    let (anchor_mirror, _atx, _) = scripted_mirror(false);
    anchor_mirror.configure(500, CursorShape::Block, false);
    anchor_mirror.resize_local(26, 6);
    let _anchor_view = anchor_mirror.view();
    anchor_mirror.test_feed(1, b"older needle\r\nmiddle needle\r\nlatest needle");
    anchor_mirror.test_feed(2, b"\x1b[?2026h");
    let needle_q = FindQuery {
        text: "needle".into(),
        match_case: false,
        regex: false,
    };
    let queued_first = anchor_mirror.find_matches_in_session(2, &needle_q, FindAction::NewQuery);
    assert_eq!(queued_first.count.total, 0);
    let queued_second = anchor_mirror.find_matches_in_session(
        2,
        &needle_q,
        FindAction::Step {
            anchor: None,
            carried: None,
            older: true,
        },
    );
    assert_eq!(queued_second.count.total, 0);
    assert!(anchor_mirror.is_render_held());

    anchor_mirror.test_feed(3, b"\x1b[?2026l");
    let released_step = anchor_mirror.find_matches_in_session(
        2,
        &needle_q,
        FindAction::Refresh {
            anchor: None,
            carried: None,
        },
    );
    assert_eq!(released_step.count.index, Some(1));
    assert!(
        released_step.anchor.is_some(),
        "released queued step must produce a live tracked anchor"
    );
    anchor_mirror.resize_local(24, 6);
    let carried = released_step
        .mark
        .moved_since(anchor_mirror.scroll_mark())
        .and_then(|moved| {
            released_step
                .current
                .map(|m| crate::terminal::find::shift_match(&m, moved))
        });
    let after = anchor_mirror.find_matches_in_session(
        2,
        &needle_q,
        FindAction::Refresh {
            anchor: released_step.anchor.as_ref().map(|a| a.id()),
            carried,
        },
    );
    assert_eq!(after.count.index, Some(1));
    assert_eq!(after.current, released_step.current);

    // Split `CSI ? 2026 h` and split `CSI ? 2026 l` across chunk boundaries stay
    // held without leaking post-hold bytes and release cleanly on the closing byte.
    let (edge_mirror, _etx, _) = scripted_mirror(false);
    edge_mirror.resize_local(20, 3);
    let _edge_view = edge_mirror.view();
    edge_mirror.test_feed(1, b"PRE\x1b[?202");
    assert_eq!(edge_mirror.sync_state().0, None);
    edge_mirror.test_feed(2, b"6hSPLIT_HELD\x1b[?202");
    assert!(edge_mirror.sync_state().0.is_some());
    assert_eq!(
        edge_mirror
            .viewport_snapshot()
            .view()
            .row_to_string(Line(0)),
        "PRE"
    );
    edge_mirror.test_feed(3, b"6l");
    assert_eq!(edge_mirror.sync_state().0, None);
    assert_eq!(
        edge_mirror
            .viewport_snapshot()
            .view()
            .row_to_string(Line(0)),
        "PRESPLIT_HELD"
    );
}

#[test]
fn dec_2026_grouped_decset_every_split_resize_parity_and_release_query_resolution() {
    use crate::terminal::find::{FindAction, FindQuery};

    struct Host;
    impl TerminalHost for Host {
        fn write_input(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn write_reply(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn report_input_state(&self, _: &str, _: InputObservation) {}
        fn set_live_title(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn resize(&self, _: &str, _: u16, _: u16) -> Result<()> {
            Ok(())
        }
        fn is_local_host(&self, _: &str) -> bool {
            true
        }
        fn clipboard_store(&self, _: &str, _: ClipboardTarget, _: &str) {}
    }

    let secret_q = FindQuery {
        text: "SECRET".into(),
        match_case: false,
        regex: false,
    };

    // 1. Grouped DECSET (`?2026h`, `?25;2026h`, `?02026h`, `?2026;1049h`,
    //    `?1049;2026h`) under Decision 18:
    //    - Split before DECSET publishes "pre" first and freezes "pre" while held.
    //    - Unsplit `pre + begin + SECRET` in one chunk stays on the pre-chunk
    //      published frame ("") while held (never leaking "SECRET").
    //    - After release (`?2026l`), split and unsplit converge to the exact
    //      same final screen and resolve queries queued during the hold.
    for (begin, expected_released) in [
        (b"\x1b[?2026h".as_slice(), "preSECRET"),
        (b"\x1b[?25;2026h", "preSECRET"),
        (b"\x1b[?02026h", "preSECRET"),
        (b"\x1b[?2026;1049h", "   SECRET"),
        (b"\x1b[?1049;2026h", "   SECRET"),
    ] {
        for split in [false, true] {
            let (mirror, _tx, _) = scripted_mirror(false);
            mirror.resize_local(40, 3);
            let _view = mirror.view();
            if split {
                let first = [b"pre".as_slice(), &begin[..begin.len() - 1]].concat();
                let second = [&begin[begin.len() - 1..], b"SECRET"].concat();
                mirror.test_feed(1, &first);
                mirror.test_feed(2, &second);
            } else {
                let chunk = [b"pre".as_slice(), begin, b"SECRET"].concat();
                mirror.test_feed(1, &chunk);
            }
            let expected_held = if split { "pre" } else { "" };
            assert!(
                mirror.sync_state().0.is_some(),
                "expected active render hold for begin={:?} split={split}",
                String::from_utf8_lossy(begin)
            );
            assert_eq!(
                mirror.with_screen(|s| s.view().visible_lines())[0],
                expected_held,
                "held with_screen mismatch for begin={:?} split={split}",
                String::from_utf8_lossy(begin)
            );
            assert_eq!(
                mirror.viewport_snapshot().view().visible_lines()[0],
                expected_held,
                "held viewport mismatch for begin={:?} split={split}",
                String::from_utf8_lossy(begin)
            );
            let secret_held = mirror.find_matches_in_session(2, &secret_q, FindAction::NewQuery);
            assert_eq!(
                secret_held.count.total,
                0,
                "SECRET searchable while held for begin={:?} split={split}",
                String::from_utf8_lossy(begin)
            );
            assert_eq!(mirror.viewport_snapshot().find_highlights(2), None);

            // Releasing the hold publishes the identical final screen for both
            // split and unsplit and resolves the queued SECRET query immediately.
            mirror.test_feed(3, b"\x1b[?2026l");
            assert_eq!(mirror.sync_state().0, None);
            assert_eq!(
                mirror.viewport_snapshot().view().visible_lines()[0],
                expected_released,
                "released viewport mismatch for begin={:?} split={split}",
                String::from_utf8_lossy(begin)
            );
            assert_eq!(
                mirror
                    .viewport_snapshot()
                    .find_highlights(2)
                    .map(|h| h.len()),
                Some(1),
                "query queued during hold must highlight immediately on release"
            );
            let secret_after = mirror.find_matches_in_session(
                2,
                &secret_q,
                FindAction::Refresh {
                    anchor: None,
                    carried: None,
                },
            );
            assert_eq!(secret_after.count.total, 1);
        }
    }

    // 2. Same-cursor overwrite prefix (`abc` + `X\x08` + `?25;2026h` + `SECRET`):
    //    split shows `"abcX"` while held, unsplit shows pre-chunk `"abc"` while
    //    held (Decision 18), and both converge to `"abcSECRET"` on release.
    for split in [false, true] {
        let (mirror, _tx, _) = scripted_mirror(false);
        mirror.resize_local(16, 3);
        let _view = mirror.view();
        mirror.test_feed(1, b"abc");
        if split {
            mirror.test_feed(2, b"X\x08");
            mirror.test_feed(3, b"\x1b[?25;2026hSECRET");
        } else {
            mirror.test_feed(2, b"X\x08\x1b[?25;2026hSECRET");
        }
        assert!(mirror.is_render_held());
        let expected_held = if split { "abcX" } else { "abc" };
        assert_eq!(
            mirror.with_screen(|s| s.view().row_to_string(Line(0))),
            expected_held
        );
        assert_eq!(
            mirror.viewport_snapshot().view().row_to_string(Line(0)),
            expected_held
        );
        mirror.test_feed(4, b"\x1b[?2026l");
        assert!(!mirror.is_render_held());
        assert_eq!(
            mirror.viewport_snapshot().view().row_to_string(Line(0)),
            "abcSECRET"
        );
    }

    // 3. Every byte split of `CSI ? 2026 h` and `CSI ? 2026 l` produces the exact
    //    same frozen pre-hold view and released post-hold view.
    let begin_seq = b"\x1b[?2026h";
    let end_seq = b"\x1b[?2026l";
    for b_split in 1..begin_seq.len() {
        for e_split in 1..end_seq.len() {
            let (mirror, _tx, _) = scripted_mirror(false);
            mirror.resize_local(30, 3);
            let _view = mirror.view();
            let c1 = [b"pre".as_slice(), &begin_seq[..b_split]].concat();
            let c2 = [&begin_seq[b_split..], b"BODY", &end_seq[..e_split]].concat();
            let c3 = &end_seq[e_split..];
            mirror.test_feed(1, &c1);
            assert_eq!(mirror.sync_state().0, None);
            mirror.test_feed(2, &c2);
            assert!(
                mirror.sync_state().0.is_some(),
                "hold missing for b_split={b_split} e_split={e_split}"
            );
            assert_eq!(
                mirror.viewport_snapshot().view().visible_lines()[0],
                "pre",
                "BODY leaked for b_split={b_split} e_split={e_split}"
            );
            mirror.test_feed(3, c3);
            assert_eq!(mirror.sync_state().0, None);
            assert_eq!(
                mirror.viewport_snapshot().view().visible_lines()[0],
                "preBODY",
                "release mismatch for b_split={b_split} e_split={e_split}"
            );
        }
    }

    // 4. Daemon (`TerminalModel`) and mirror (`TerminalMirror`) converge to the
    //    exact same grid when a resize arrives during a DEC 2026 hold.
    let host = TerminalModel::new(
        Arc::new(Host),
        "sync-resize-host".into(),
        12,
        3,
        ModelOptions::default(),
    )
    .unwrap();
    let (mirror, frames, _) = scripted_mirror(false);
    mirror.resize_local(12, 3);
    let _view = mirror.view();
    let bytes = b"S\x1b[?2026h\x1b[1;25HX";
    mirror.test_feed(1, bytes);
    host.feed_output(1, bytes);
    assert!(mirror.is_render_held());
    host.resize(26, 3);
    frames
        .send(Ok(TerminalFrame::Resized {
            seq: 2,
            cols: 26,
            rows: 3,
            controller: TerminalController::Desktop,
        }))
        .unwrap();
    assert!(wait_until(Duration::from_secs(2), || mirror.size() == (26, 3)));
    let end = b"\x1b[?2026l";
    mirror.test_feed(3, end);
    host.feed_output(3, end);
    assert_eq!(host.visible_lines()[0].find('X'), Some(11));
    assert_eq!(mirror.visible_lines()[0].find('X'), Some(11));
    assert_eq!(mirror.visible_lines(), host.visible_lines());
    drop(frames);
    mirror.test_stop_frames();
    for worker in host.stop_workers() {
        worker.join().unwrap();
    }
}

#[test]
fn dec_2026_preserves_scrolled_viewport_cached_find_and_anchors_across_ed3_alt_scroll_and_counts_native_updates(
) {
    use crate::terminal::find::{FindAction, FindQuery};

    let q = FindQuery {
        text: "needle".into(),
        match_case: false,
        regex: false,
    };

    // 1. Scrolled back 3 rows into history, then opening a DEC 2026 hold that
    //    clears scrollback (CSI 3 J), enters alt screen (CSI ? 1049 h), or
    //    scrolls active output must keep both the published/inspected scrolled
    //    viewport ("first needle") and last published find count (3 -> 3)
    //    intact until release.
    for (name, post) in [
        ("ED3", b"\x1b[3J".as_slice()),
        ("ALT", b"\x1b[?1049h"),
        ("scroll", b"\r\npost"),
    ] {
        let (mirror, _tx, _) = scripted_mirror(false);
        mirror.resize_local(20, 3);
        let _view = mirror.view();
        mirror.test_feed(
            1,
            b"first needle\r\nsecond needle\r\nthird needle\r\nactive A\r\nactive B\r\nactive C",
        );
        mirror.scroll_to_display_offset(3);
        let before_top = mirror.with_screen(|s| {
            let v = s.view();
            v.row_to_string(Line(-(v.display_offset() as i32)))
        });
        assert_eq!(before_top, "first needle");
        let before = mirror.find_matches_in_session(17, &q, FindAction::NewQuery);
        assert_eq!(before.count.total, 3);

        let held_chunk = [b"\x1b[?2026h".as_slice(), post].concat();
        mirror.test_feed(2, &held_chunk);
        assert!(mirror.sync_state().0.is_some(), "{name} should stay held");
        let during = mirror.find_matches_in_session(17, &q, FindAction::NewQuery);
        assert_eq!(
            during.count.total, 3,
            "{name} dropped find count during hold"
        );
        let during_top = mirror.with_screen(|s| {
            let v = s.view();
            v.row_to_string(Line(-(v.display_offset() as i32)))
        });
        assert_eq!(
            during_top, "first needle",
            "{name} moved with_screen top row during hold"
        );
        let painted_top = {
            let snap = mirror.viewport_snapshot();
            let v = snap.view();
            v.row_to_string(Line(-(v.display_offset() as i32)))
        };
        assert_eq!(
            painted_top, "first needle",
            "{name} moved published top row during hold"
        );
        mirror.test_feed(3, b"\x1b[?2026l");
    }

    // 2. Deterministic queued find during hold (Blocker #3):
    //    (a) Uncached NewQuery on a scrolled viewport under ED3 returns promptly
    //        while the hold is still active (never blocking on release), keeps
    //        the pre-release state unchanged, and resolves once on release.
    {
        let (mirror, _frames, _) = scripted_mirror(false);
        mirror.resize_local(20, 3);
        let _view = mirror.view();
        mirror.test_feed(
            1,
            b"hist alpha\r\nhist beta\r\nhist gamma\r\nactive A\r\nactive B\r\nactive C",
        );
        mirror.scroll_to_display_offset(3);
        mirror.test_feed(2, b"\x1b[?2026h\x1b[3J\x1b[Hreleased_token");
        assert!(mirror.is_render_held());

        let q_new = FindQuery {
            text: "released_token".into(),
            match_case: false,
            regex: false,
        };
        let pre_release = mirror.find_matches_in_session(29, &q_new, FindAction::NewQuery);
        // The synchronous round-trip confirmed the actor queued the request
        // while the hold was still open, returning the last published state
        // promptly instead of blocking until release or timeout.
        assert!(
            mirror.is_render_held(),
            "find_matches_in_session must return promptly while hold is still active"
        );
        assert_eq!(pre_release.count.total, 0);
        assert_eq!(pre_release.current, None);
        assert_eq!(mirror.viewport_snapshot().find_highlights(29), None);

        mirror.test_feed(3, b"\x1b[?2026l");
        assert!(!mirror.is_render_held());
        let expected = FindMatch::new(
            Point::new(Line(0), Column(0)),
            Point::new(Line(0), Column(13)),
        );
        assert_eq!(
            mirror.viewport_snapshot().find_highlights(29),
            Some([expected].as_slice()),
            "queued query must publish highlights once at release"
        );
        let released = mirror.find_matches_in_session(
            29,
            &q_new,
            FindAction::Refresh {
                anchor: None,
                carried: None,
            },
        );
        assert_eq!(released.count.total, 1);
        assert_eq!(released.current, Some(expected));
    }

    //    (b) Step during a hold with intact history returns the last published
    //        index (0 -> 0) promptly without recomputing before release, then
    //        advances once on release (0 -> 1).
    {
        let (mirror, _frames, _) = scripted_mirror(false);
        mirror.resize_local(16, 3);
        let _view = mirror.view();
        let key_q = FindQuery {
            text: "key".into(),
            match_case: false,
            regex: false,
        };
        mirror.test_feed(1, b"key1\r\nkey2\r\nkey3");
        let pre = mirror.find_matches_in_session(31, &key_q, FindAction::NewQuery);
        assert_eq!(pre.count.total, 3);
        assert_eq!(pre.count.index, Some(0));

        mirror.test_feed(2, b"\x1b[?2026h");
        assert!(mirror.is_render_held());
        let during_step = mirror.find_matches_in_session(
            31,
            &key_q,
            FindAction::Step {
                older: true,
                carried: pre.current,
                anchor: pre.anchor.as_ref().map(|a| a.id()),
            },
        );
        assert!(mirror.is_render_held());
        assert_eq!(
            during_step.count.index,
            Some(0),
            "Step during hold must return last published index before release"
        );
        assert_eq!(during_step.current, pre.current);

        mirror.test_feed(3, b"\x1b[?2026l");
        let after_release = mirror.find_matches_in_session(
            31,
            &key_q,
            FindAction::Refresh {
                anchor: during_step.anchor.as_ref().map(|a| a.id()),
                carried: during_step.current,
            },
        );
        assert_eq!(after_release.count.total, 3);
        assert_eq!(after_release.count.index, Some(1));
        assert_eq!(
            after_release.current,
            Some(FindMatch::new(
                Point::new(Line(1), Column(0)),
                Point::new(Line(1), Column(2)),
            ))
        );
    }

    // 3. Refreshing an existing selected match during a hold that scrolls live
    //    output must stay in published pre-hold coordinates (row 2, index 1),
    //    never resolving the live shifted TrackedGridRef while held, then follow
    //    "two  needle" to live row 1 after release.
    {
        let (mirror, _tx, _) = scripted_mirror(false);
        mirror.resize_local(28, 4);
        let _view = mirror.view();
        mirror.test_feed(
            1,
            b"zero needle\r\none  needle\r\ntwo  needle\r\nlast needle",
        );
        let newest = mirror.find_matches_in_session(33, &q, FindAction::NewQuery);
        let selected = mirror.find_matches_in_session(
            33,
            &q,
            FindAction::Step {
                older: true,
                anchor: newest.anchor.as_ref().map(|a| a.id()),
                carried: newest.current,
            },
        );
        assert_eq!(selected.count.index, Some(1));
        assert_eq!(
            selected.current,
            Some(FindMatch::new(
                Point::new(Line(2), Column(5)),
                Point::new(Line(2), Column(10)),
            ))
        );

        mirror.test_feed(2, b"\x1b[?2026h\r\nafter");
        for carried in [selected.current, None] {
            let during = mirror.find_matches_in_session(
                33,
                &q,
                FindAction::Refresh {
                    anchor: selected.anchor.as_ref().map(|a| a.id()),
                    carried,
                },
            );
            assert_eq!(during.count.index, Some(1));
            assert_eq!(during.current, selected.current);
        }

        mirror.test_feed(3, b"\x1b[?2026l");
        let after = mirror.find_matches_in_session(
            33,
            &q,
            FindAction::Refresh {
                anchor: selected.anchor.as_ref().map(|a| a.id()),
                carried: None,
            },
        );
        assert_eq!(after.count.index, Some(1));
        assert_eq!(
            after.current,
            Some(FindMatch::new(
                Point::new(Line(1), Column(5)),
                Point::new(Line(1), Column(10)),
            ))
        );
    }

    // 4. Rectangular selection copy outside a hold and synchronous live-grid
    //    Copy during a hold (both pre-hold selections and selections dragged
    //    inside the hold): formats once on demand with Ghostty's native
    //    formatter without releasing the hold or calling `RenderState::update`.
    for held in [false, true] {
        let (mirror, _tx, _) = scripted_mirror(false);
        mirror.resize_local(16, 3);
        let _view = mirror.view();
        mirror.test_feed(1, b"01234567\r\nabcdefgh\r\nABCDEFGH");
        mirror.start_selection(
            SelectionKind::Block,
            Point::new(Line(0), Column(2)),
            Side::Left,
        );
        mirror.update_selection(Point::new(Line(2), Column(4)), Side::Right);
        if held {
            mirror.test_feed(2, b"\x1b[?2026h");
            assert!(mirror.is_render_held());
            let before_formats = mirror.test_selection_formats();
            let before_updates = mirror.test_render_state_updates();
            assert_eq!(mirror.selection_text().as_deref(), Some("234\ncde\nCDE"));
            assert!(
                mirror.is_render_held(),
                "synchronous Copy must not release the active DEC 2026 hold"
            );
            assert_eq!(mirror.test_selection_formats(), before_formats + 1);
            assert_eq!(mirror.test_render_state_updates(), before_updates);
            mirror.test_feed(3, b"\x1b[?2026l");
        } else {
            assert_eq!(mirror.selection_text().as_deref(), Some("234\ncde\nCDE"));
        }
    }
    {
        let (mirror, _tx, _) = scripted_mirror(false);
        mirror.resize_local(16, 3);
        let _view = mirror.view();
        mirror.test_feed(1, b"01234567\r\nabcdefgh\r\nABCDEFGH");
        mirror.test_feed(2, b"\x1b[?2026h");
        mirror.start_selection(
            SelectionKind::Block,
            Point::new(Line(0), Column(2)),
            Side::Left,
        );
        mirror.update_selection(Point::new(Line(2), Column(4)), Side::Right);
        let before_formats = mirror.test_selection_formats();
        let before_updates = mirror.test_render_state_updates();
        assert_eq!(mirror.selection_text().as_deref(), Some("234\ncde\nCDE"));
        assert!(mirror.is_render_held());
        assert_eq!(mirror.test_selection_formats(), before_formats + 1);
        assert_eq!(mirror.test_render_state_updates(), before_updates);
        mirror.test_feed(3, b"\x1b[?2026l");
        assert_eq!(mirror.test_render_state_updates(), before_updates + 1);
    }

    // 5. Native RenderState::update count and zero unrequested selection
    //    formatting across plain, unprefixed sync, prefixed sync, open hold
    //    interactions, and hidden mirrors:
    //    - visible mirror (including with an active multi-row selection):
    //      exactly 1 native update and 0 selection formats per complete frame
    //      (plain, unprefixed sync, cursor-moving prefixed sync, and
    //      same-cursor overwrite prefixed sync), 0 updates / 0 formats during
    //      an open hold, and 1 update / 0 formats on release.
    //    - hidden mirror (`viewers == 0`): 0 native updates on all frames.
    {
        let (mirror, _tx, _) = scripted_mirror(false);
        mirror.resize_local(40, 4);
        let _view = mirror.view();
        mirror.test_feed(1, b"row0\r\nrow1\r\nrow2\r\nrow3\r\nrow4\r\nrow5");
        mirror.start_selection(
            SelectionKind::Simple,
            Point::new(Line(-2), Column(0)),
            Side::Left,
        );
        mirror.update_selection(Point::new(Line(-1), Column(3)), Side::Right);

        for (seq, label, chunk) in [
            (2u64, "plain", b"\rplain".as_slice()),
            (3, "unprefixed sync", b"\x1b[?2026h\rsynced\x1b[?2026l"),
            (
                4,
                "cursor-moving prefixed sync",
                b"pre\x1b[?25;2026hSECRET\x1b[?2026l",
            ),
            (
                5,
                "same-cursor overwrite prefixed sync",
                b"X\x08\x1b[?25;2026hSECRET2\x1b[?2026l",
            ),
        ] {
            let before = mirror.test_render_state_updates();
            let before_formats = mirror.test_selection_formats();
            mirror.test_feed(seq, chunk);
            let delta = mirror.test_render_state_updates() - before;
            assert_eq!(
                delta, 1,
                "{label} complete frame must invoke native RenderState::update exactly once"
            );
            assert_eq!(
                mirror.test_selection_formats(),
                before_formats,
                "{label} complete frame must do zero selection formatting"
            );
        }

        // Open hold + held writes + scroll + find + selection hit-testing
        // perform 0 native updates and 0 selection formats while held, then
        // exactly 1 native update and 0 selection formats on release.
        let before_hold = mirror.test_render_state_updates();
        let before_formats = mirror.test_selection_formats();
        mirror.test_feed(6, b"pre\x1b[?25;2026hHELD_BODY");
        mirror.scroll_local(1);
        let _ = mirror.find_matches_in_session(41, &q, FindAction::NewQuery);
        let _ = mirror.selection_contains(Point::new(Line(0), Column(0)));
        assert_eq!(
            mirror.test_render_state_updates() - before_hold,
            0,
            "open hold and held reads/queued interactions must do 0 native RenderState::update calls"
        );
        assert_eq!(mirror.test_selection_formats(), before_formats);
        mirror.test_feed(7, b"\x1b[?2026l");
        assert_eq!(
            mirror.test_render_state_updates() - before_hold,
            1,
            "hold release must perform exactly 1 native RenderState::update"
        );
        assert_eq!(
            mirror.test_selection_formats(),
            before_formats,
            "hold release without requested Copy must do 0 selection formatting"
        );
    }
    {
        let (hidden, _tx, _) = scripted_mirror(false);
        hidden.resize_local(40, 4);
        let before = hidden.test_render_state_updates();
        hidden.test_feed(1, b"\x1b[?2026h\rhidden_sync\x1b[?2026l");
        hidden.test_feed(2, b"pre\x1b[?25;2026h\rhidden_prefixed\x1b[?2026l");
        hidden.test_feed(3, b"X\x08\x1b[?25;2026h\rhidden_open");
        hidden.test_feed(4, b"\x1b[?2026l");
        assert_eq!(
            hidden.test_render_state_updates(),
            before,
            "hidden mirror must perform zero native RenderState::update calls on synchronized frames"
        );
    }
}

#[test]
fn dec_2026_offscreen_copy_scrolled_queued_selection_dropped_viewport_anchor_and_deadline_bound() {
    // 1. Copy formats synchronously on demand from the live grid with Ghostty's
    //    native formatter for both Block and Simple selections extending into
    //    scrollback, without releasing an open DEC 2026 hold.
    for (kind, expected) in [
        (SelectionKind::Block, "al\nbr\nch\nde"),
        (SelectionKind::Simple, "alpha\n11bravo\n22charlie\n33de"),
    ] {
        for held in [false, true] {
            let (mirror, _tx, _) = scripted_mirror(false);
            mirror.resize_local(12, 3);
            let _view = mirror.view();
            mirror.test_feed(1, b"00alpha\r\n11bravo\r\n22charlie\r\n33delta\r\n44echo");
            mirror.start_selection(kind, Point::new(Line(-2), Column(2)), Side::Left);
            mirror.update_selection(Point::new(Line(1), Column(3)), Side::Right);
            if held {
                mirror.test_feed(2, b"\x1b[?2026h");
                assert!(mirror.is_render_held());
            }
            assert_eq!(
                mirror.selection_text().as_deref(),
                Some(expected),
                "offscreen {kind:?} copy mismatch for held={held}"
            );
            if held {
                assert!(
                    mirror.is_render_held(),
                    "synchronous Copy must not end the open DEC 2026 hold"
                );
                mirror.test_feed(3, b"\x1b[?2026l");
            }
        }
    }

    // 2. Queued selection endpoints translate from published pre-hold
    //    coordinates onto the surviving live content when whole-screen output
    //    scrolled during the hold (both Simple drag and Semantic double-click),
    //    and an evicted selection on a zero-scrollback screen clears cleanly.
    {
        let (mirror, _tx, _) = scripted_mirror(false);
        mirror.resize_local(16, 3);
        let _view = mirror.view();
        mirror.test_feed(1, b"alpha\r\nbravo\r\ncello");
        mirror.test_feed(2, b"\x1b[?2026h\r\ndelta");
        assert_eq!(
            mirror.with_screen(|s| s.view().row_to_string(Line(1))),
            "bravo"
        );
        mirror.start_selection(
            SelectionKind::Simple,
            Point::new(Line(1), Column(0)),
            Side::Left,
        );
        mirror.update_selection(Point::new(Line(1), Column(4)), Side::Right);
        mirror.test_feed(3, b"\x1b[?2026l");
        assert_eq!(mirror.selection_text().as_deref(), Some("bravo"));
    }
    {
        let (mirror, _tx, _) = scripted_mirror(false);
        mirror.resize_local(16, 3);
        let _view = mirror.view();
        mirror.test_feed(1, b"alpha\r\nbravo\r\ncello");
        mirror.test_feed(2, b"\x1b[?2026h\r\ndelta");
        mirror.start_selection(
            SelectionKind::Semantic,
            Point::new(Line(1), Column(2)),
            Side::Left,
        );
        mirror.test_feed(3, b"\x1b[?2026l");
        assert_eq!(mirror.selection_text().as_deref(), Some("bravo"));
    }
    {
        let (mirror, _tx, _) = scripted_mirror(false);
        mirror.configure(0, CursorShape::Block, false);
        mirror.resize_local(16, 3);
        let _view = mirror.view();
        mirror.test_feed(1, b"alpha\r\nbravo\r\ncello");
        mirror.test_feed(2, b"\x1b[?2026h\r\ndelta");
        mirror.start_selection(
            SelectionKind::Simple,
            Point::new(Line(0), Column(0)),
            Side::Left,
        );
        mirror.update_selection(Point::new(Line(0), Column(4)), Side::Right);
        mirror.test_feed(3, b"\x1b[?2026l");
        assert_eq!(mirror.selection_text(), None);
    }

    // 3. Closing Find during a hold queues `RestoreViewportAnchor(id)` and
    //    immediately drops its `GridAnchor`; the actor keeps the native pin
    //    alive until release consumes the restore, matching unheld behavior.
    for held in [false, true] {
        let (mirror, _tx, _) = scripted_mirror(false);
        mirror.resize_local(12, 3);
        let _view = mirror.view();
        let text = (0..20)
            .map(|i| format!("row{i:02}\r\n"))
            .collect::<String>();
        mirror.test_feed(1, text.as_bytes());
        mirror.scroll_to_display_offset(5);
        let anchor = mirror.track_viewport_top().unwrap();
        let top_row = |m: &TerminalMirror| {
            let snap = m.viewport_snapshot();
            let v = snap.view();
            v.row_to_string(Line(-(v.display_offset() as i32)))
        };
        let expected = top_row(&mirror);
        mirror.scroll_to_display_offset(0);
        if held {
            mirror.test_feed(2, b"\x1b[?2026h");
        }
        mirror.restore_viewport_anchor(&anchor);
        drop(anchor);
        if held {
            mirror.test_feed(3, b"\x1b[?2026l");
        }
        assert_eq!(
            top_row(&mirror),
            expected,
            "viewport anchor restore mismatch for held={held}"
        );
        assert_eq!(
            mirror.scroll_state().display_offset,
            5,
            "display offset mismatch for held={held}"
        );
    }

    // 4. Continuous paired `CSI ? 2026 l` + `CSI ? 2026 h` reopenings retain
    //    the exact original hold deadline Instant (no sliding extension) and
    //    publish deterministically at expiry — verified both with a controlled
    //    clock (`test_feed_at`) and through the real actor publication event.
    {
        let (tx, rx) = mpsc::channel();
        let host = Arc::new(ScriptedHost(Mutex::new(Some(ScriptedFrames(
            rx,
            tx.clone(),
        )))));
        let (wake_tx, wake_rx) = mpsc::channel::<()>();
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = wake_tx.send(());
        });
        let mirror =
            TerminalMirror::attach(DaemonClient::new(host), "deadline".into(), waker).unwrap();
        mirror.resize_local(16, 3);
        let _view = mirror.view();
        mirror.test_feed(1, b"BASE");

        // Controlled clock (future epoch so OS wall-clock cannot preempt):
        // paired close/open writes keep the exact original deadline until
        // `t0 + MIRROR_SYNC_HOLD_TIMEOUT`, where expiry publishes `frame7`.
        let t0 = Instant::now() + Duration::from_secs(10);
        mirror.test_feed_at(2, b"\x1b[?2026h", t0);
        let original_deadline = mirror.sync_state().0.expect("hold deadline armed");
        assert_eq!(original_deadline, t0 + MIRROR_SYNC_HOLD_TIMEOUT);
        for (seq, elapsed_ms) in [(3u64, 40u64), (4, 80), (5, 120), (6, 149)] {
            let chunk = format!("\rframe{seq}\x1b[?2026l\x1b[?2026h");
            mirror.test_feed_at(
                seq,
                chunk.as_bytes(),
                t0 + Duration::from_millis(elapsed_ms),
            );
            assert_eq!(
                mirror.sync_state().0,
                Some(original_deadline),
                "paired DEC 2026 close/open must retain the original hold deadline (seq={seq})"
            );
            assert_eq!(
                mirror.viewport_snapshot().view().row_to_string(Line(0)),
                "BASE"
            );
        }
        mirror.test_feed_at(
            7,
            b"\rframe7\x1b[?2026l\x1b[?2026h\x1b[?2026h",
            t0 + MIRROR_SYNC_HOLD_TIMEOUT,
        );
        assert_eq!(mirror.sync_state().0, None);
        assert_eq!(
            mirror.viewport_snapshot().view().row_to_string(Line(0)),
            "frame7"
        );

        // Repeated begin inside an active hold (plain, split across writes,
        // grouped DECSET `?25;2026h`, leading zero `?02026h`) extends the
        // deadline, while literal `CSI ? 2026 h` bytes inside an OSC string
        // do not extend it and paired close+open within one write still keeps
        // the prior bound.
        let t1 = Instant::now() + Duration::from_secs(20);
        mirror.test_feed_at(8, b"\x1b[?2026h\rhold1", t1);
        let d1 = mirror.sync_state().0.expect("hold1 armed");
        assert_eq!(d1, t1 + MIRROR_SYNC_HOLD_TIMEOUT);

        // Unrelated DECSET, ANSI mode 2026 (without `?`), DECRQM `?2026$p`
        // queries, and saved-mode restore (`?2026s` followed by `?2026r` of
        // already-active mode 2026) must not extend the hold deadline.
        mirror.test_feed_at(
            9,
            b"\x1b[2026h\x1b[?2026$p\x1b[?25h\x1b]2;[?2026h\x07\x1b[?2026s",
            t1 + Duration::from_millis(20),
        );
        assert_eq!(mirror.sync_state().0, Some(d1));
        mirror.test_feed_at(10, b"\x1b[?2026r", t1 + Duration::from_millis(30));
        assert_eq!(
            mirror.sync_state().0,
            Some(d1),
            "saved-mode restore (?2026r) of already-active mode 2026 must not extend hold deadline"
        );

        // Plain repeated begin extends.
        let t2 = t1 + Duration::from_millis(40);
        mirror.test_feed_at(11, b"\x1b[?2026h", t2);
        let d2 = mirror.sync_state().0.expect("plain reopen extended");
        assert_eq!(d2, t2 + MIRROR_SYNC_HOLD_TIMEOUT);
        assert!(d2 > d1);

        // Split repeated begin across two writes extends on completion.
        let t3 = t2 + Duration::from_millis(30);
        mirror.test_feed_at(12, b"\x1b[?20", t3);
        assert_eq!(mirror.sync_state().0, Some(d2));
        let t4 = t3 + Duration::from_millis(20);
        mirror.test_feed_at(13, b"26h", t4);
        let d4 = mirror.sync_state().0.expect("split reopen extended");
        assert_eq!(d4, t4 + MIRROR_SYNC_HOLD_TIMEOUT);
        assert!(d4 > d2);

        // Grouped and zero-padded DECSET forms extend.
        let t5 = t4 + Duration::from_millis(25);
        mirror.test_feed_at(14, b"\x1b[?25;2026h", t5);
        let d5 = mirror.sync_state().0.expect("grouped reopen extended");
        assert_eq!(d5, t5 + MIRROR_SYNC_HOLD_TIMEOUT);
        assert!(d5 > d4);

        let t6 = t5 + Duration::from_millis(25);
        mirror.test_feed_at(15, b"\x1b[?02026h", t6);
        let d6 = mirror.sync_state().0.expect("zero-padded reopen extended");
        assert_eq!(d6, t6 + MIRROR_SYNC_HOLD_TIMEOUT);
        assert!(d6 > d5);

        // Paired close+open (even followed by a redundant begin in the same
        // write) keeps the prior bound `d6` rather than sliding further.
        let t7 = t6 + Duration::from_millis(50);
        mirror.test_feed_at(16, b"\rpaired\x1b[?2026l\x1b[?2026h\x1b[?2026h", t7);
        assert_eq!(mirror.sync_state().0, Some(d6));
        mirror.test_feed_at(17, b"\x1b[?2026l", t7 + Duration::from_millis(5));
        assert_eq!(mirror.sync_state().0, None);
        assert_eq!(
            mirror.viewport_snapshot().view().row_to_string(Line(0)),
            "paired"
        );

        // Real actor timer expiry driven by the real publication wake event
        // (no fixed sleep and no polling loop).
        let before_open = Instant::now();
        mirror.test_feed(18, b"\x1b[?2026h");
        let after_open = Instant::now();
        let real_deadline = mirror.sync_state().0.expect("real hold deadline armed");
        assert!(
            real_deadline >= before_open + MIRROR_SYNC_HOLD_TIMEOUT
                && real_deadline <= after_open + MIRROR_SYNC_HOLD_TIMEOUT
        );
        mirror.test_feed(19, b"\rframe9\x1b[?2026l\x1b[?2026h");
        assert_eq!(mirror.sync_state().0, Some(real_deadline));
        let guard = Instant::now() + Duration::from_secs(1);
        while mirror.sync_state().0.is_some() {
            let remaining = guard.saturating_duration_since(Instant::now());
            assert!(
                !remaining.is_zero(),
                "hold deadline expiry must emit a publication wake and clear hold"
            );
            let _ = wake_rx.recv_timeout(remaining);
        }
        assert!(Instant::now() >= real_deadline);
        assert_eq!(
            mirror.viewport_snapshot().view().row_to_string(Line(0)),
            "frame9"
        );

        // Synchronous Copy during four repeated DECSET ?2026 reopens 65 ms
        // apart returns live grid text immediately in microseconds without
        // releasing the hold or updating the frozen published snapshot.
        mirror.test_feed(20, b"\x1b[2J\x1b[Hhello world");
        mirror.start_selection(
            SelectionKind::Simple,
            Point::new(Line(0), Column(0)),
            Side::Left,
        );
        mirror.update_selection(Point::new(Line(0), Column(4)), Side::Right);
        assert_eq!(mirror.selection_text().as_deref(), Some("hello"));
        mirror.test_feed(21, b"\x1b[?2026h\x1b[1;1HLIVE!\x1b[2;1Hlive-mid-hold");
        assert!(mirror.is_render_held());
        let before_updates = mirror.test_render_state_updates();

        let (reopens_done_tx, reopens_done_rx) = mpsc::channel::<()>();
        let (close_tx, close_rx) = mpsc::channel::<()>();
        let feeder_mirror = Arc::clone(&mirror);
        let feeder = std::thread::spawn(move || {
            for seq in 22..=25 {
                std::thread::sleep(Duration::from_millis(65));
                feeder_mirror.test_feed(seq, b"\x1b[?2026h");
            }
            let _ = reopens_done_tx.send(());
            let _ = close_rx.recv_timeout(Duration::from_secs(2));
            feeder_mirror.test_feed(26, b"\x1b[?2026l");
        });
        reopens_done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("all four 65 ms DEC 2026 reopens applied");
        assert!(
            mirror.is_render_held(),
            "hold must remain active after four 65 ms reopens"
        );
        let started = Instant::now();
        let live = mirror.selection_text();
        let copy_elapsed = started.elapsed();
        assert_eq!(live.as_deref(), Some("LIVE!"));
        assert!(
            copy_elapsed < Duration::from_millis(5),
            "synchronous selection_text after 4x65 ms reopens took {copy_elapsed:?}"
        );
        assert!(mirror.is_render_held());
        assert_eq!(mirror.test_render_state_updates(), before_updates);
        assert_eq!(
            mirror.viewport_snapshot().view().row_to_string(Line(0)),
            "hello world",
            "published display must remain frozen on pre-hold frame"
        );
        let _ = close_tx.send(());
        feeder.join().unwrap();
        assert!(!mirror.is_render_held());
        assert_eq!(mirror.test_render_state_updates(), before_updates + 1);
        assert_eq!(
            mirror.viewport_snapshot().view().row_to_string(Line(1)),
            "live-mid-hold"
        );

        // Explicit written spaces vs unwritten empty cells vs block per-row
        // trailing-whitespace trim vs same-side zero-width drag during hold,
        // plus single RenderState::update per held frame release (both explicit
        // `?2026l` close and 150 ms deadline expiry).
        mirror.test_feed(27, b"\x1b[2J\x1b[Habc   D\r\nxyz  Q");
        mirror.start_selection(
            SelectionKind::Block,
            Point::new(Line(0), Column(0)),
            Side::Left,
        );
        mirror.update_selection(Point::new(Line(1), Column(5)), Side::Right);
        assert_eq!(
            mirror.selection_text().as_deref(),
            Some("abc\nxyz  Q"),
            "block selection must preserve per-row trailing-whitespace trimming"
        );
        mirror.start_selection(
            SelectionKind::Simple,
            Point::new(Line(0), Column(0)),
            Side::Left,
        );
        mirror.update_selection(Point::new(Line(1), Column(5)), Side::Right);
        assert_eq!(
            mirror.selection_text().as_deref(),
            Some("abc   D\nxyz  Q"),
            "simple selection across rows must preserve interior text and spaces"
        );
        mirror.start_selection(
            SelectionKind::Lines,
            Point::new(Line(0), Column(0)),
            Side::Left,
        );
        mirror.update_selection(Point::new(Line(1), Column(5)), Side::Right);
        assert_eq!(
            mirror.selection_text().as_deref(),
            Some("abc   D\nxyz  Q\n"),
            "line selection must keep full lines with trailing newline"
        );
        mirror.start_selection(
            SelectionKind::Simple,
            Point::new(Line(0), Column(3)),
            Side::Left,
        );
        mirror.update_selection(Point::new(Line(0), Column(5)), Side::Right);
        assert_eq!(
            mirror.selection_text().as_deref(),
            Some("   "),
            "explicitly written interior spaces must be preserved"
        );
        mirror.start_selection(
            SelectionKind::Simple,
            Point::new(Line(0), Column(8)),
            Side::Left,
        );
        mirror.update_selection(Point::new(Line(0), Column(12)), Side::Right);
        assert_eq!(
            mirror.selection_text(),
            None,
            "unwritten cells past end of line must resolve to None"
        );
        mirror.start_selection(
            SelectionKind::Simple,
            Point::new(Line(0), Column(1)),
            Side::Left,
        );
        mirror.test_feed(28, b"\x1b[?2026h");
        mirror.update_selection(Point::new(Line(0), Column(1)), Side::Left);
        assert_eq!(
            mirror.selection_text(),
            None,
            "same-side zero-width drag during hold must resolve to None"
        );
        assert!(mirror.is_render_held());
        mirror.test_feed(29, b"\x1b[?2026l");

        let mut seq = 30u64;
        for clear in [false, true] {
            for via_deadline in [false, true] {
                mirror.test_feed(seq, b"\x1b[2J\x1b[Hhello");
                seq += 1;
                mirror.start_selection(
                    SelectionKind::Simple,
                    Point::new(Line(0), Column(0)),
                    Side::Left,
                );
                mirror.update_selection(Point::new(Line(0), Column(4)), Side::Right);
                let before_updates = mirror.test_render_state_updates();
                let t_pub = Instant::now() + Duration::from_secs(60);
                mirror.test_feed_at(seq, b"\x1b[?2026h", t_pub);
                seq += 1;
                assert_eq!(mirror.selection_text().as_deref(), Some("hello"));
                if clear {
                    mirror.clear_selection();
                    assert_eq!(
                        mirror.selection_text(),
                        None,
                        "second read after clear_selection during hold must see None immediately"
                    );
                }
                assert!(mirror.is_render_held());
                assert_eq!(
                    mirror.test_render_state_updates(),
                    before_updates,
                    "Copy and clear_selection during hold must perform 0 RenderState::update calls"
                );
                if via_deadline {
                    mirror.test_feed_at(
                        seq,
                        b"\x1b[2;1Hdeadline-frame",
                        t_pub + MIRROR_SYNC_HOLD_TIMEOUT,
                    );
                } else {
                    mirror.test_feed_at(
                        seq,
                        b"\x1b[2;1Hclose-frame\x1b[?2026l",
                        t_pub + Duration::from_millis(10),
                    );
                }
                seq += 1;
                if clear {
                    assert_eq!(mirror.selection_text(), None);
                }
                assert_eq!(
                    mirror.test_render_state_updates() - before_updates,
                    1,
                    "held frame release must perform exactly 1 RenderState::update (clear={clear}, via_deadline={via_deadline})"
                );
            }
        }
        drop(tx);
    }

    // 5. On-demand native formatting always reflects current text (hidden
    //    viewer mutation and active edit + scroll into history both return ZE)
    //    and split synchronized frames over an untouched offscreen selection do
    //    zero selection formatting work.
    for hidden in [false, true] {
        let (mirror, _tx, _) = scripted_mirror(false);
        mirror.resize_local(12, 3);
        let mut view = Some(mirror.view());
        mirror.test_feed(1, b"00alpha\r\n11bravo\r\n22charlie\r\n33delta\r\n44echo");
        mirror.start_selection(
            SelectionKind::Block,
            Point::new(Line(-2), Column(2)),
            Side::Left,
        );
        mirror.update_selection(Point::new(Line(1), Column(3)), Side::Right);
        if hidden {
            drop(view.take());
        }
        mirror.test_feed(2, b"\x1b[2;3HZEBRA");
        if hidden {
            view = Some(mirror.view());
        }
        assert_eq!(
            mirror.with_screen(|s| s.view().row_to_string(Line(1))),
            "33ZEBRA"
        );
        assert_eq!(
            mirror.selection_text().as_deref(),
            Some("al\nbr\nch\nZE"),
            "plain copy stale after hidden={hidden} mutation"
        );
        mirror.test_feed(3, b"\x1b[?2026h");
        assert_eq!(
            mirror.selection_text().as_deref(),
            Some("al\nbr\nch\nZE"),
            "open-hold deferred copy stale after hidden={hidden} mutation"
        );
        mirror.test_feed(4, b"\x1b[?2026l");
        drop(view);
    }
    for scroll in [false, true] {
        let (mirror, _tx, _) = scripted_mirror(false);
        mirror.resize_local(12, 3);
        let _view = mirror.view();
        mirror.test_feed(
            1,
            b"prefix\r\n00alpha\r\n11bravo\r\n22charlie\r\n33delta\r\n44echo",
        );
        mirror.start_selection(
            SelectionKind::Block,
            Point::new(Line(-2), Column(2)),
            Side::Left,
        );
        mirror.update_selection(Point::new(Line(0), Column(3)), Side::Right);
        let edit = if scroll {
            b"\x1b[1;3HZEBRA\x1b[3;1H\n".as_slice()
        } else {
            b"\x1b[1;3HZEBRA"
        };
        mirror.test_feed(2, edit);
        assert_eq!(
            mirror.selection_text().as_deref(),
            Some("al\nbr\nZE"),
            "plain copy stale after active edit + scroll={scroll}"
        );
        mirror.test_feed(3, b"\x1b[?2026h");
        assert_eq!(
            mirror.selection_text().as_deref(),
            Some("al\nbr\nZE"),
            "held deferred copy stale after active edit + scroll={scroll}"
        );
        mirror.test_feed(4, b"\x1b[?2026l");
    }
    {
        let (mirror, _tx, _) = scripted_mirror(false);
        mirror.resize_local(40, 4);
        let _view = mirror.view();
        mirror.test_feed(1, b"h0\r\nh1\r\nh2\r\nh3\r\nh4\r\nh5");
        mirror.start_selection(
            SelectionKind::Simple,
            Point::new(Line(-2), Column(0)),
            Side::Left,
        );
        mirror.update_selection(Point::new(Line(-1), Column(1)), Side::Right);
        let before_formats = mirror.test_selection_formats();
        for seq in 2..=10u64 {
            mirror.test_feed(2 * seq, b"\x1b[?2026h");
            let end = format!("\rframe {seq:02}\x1b[?2026l");
            mirror.test_feed(2 * seq + 1, end.as_bytes());
        }
        assert_eq!(
            mirror.test_selection_formats(),
            before_formats,
            "split synchronized frames over untouched offscreen selection must not format selection"
        );
    }

    // 6. No-selection, single-click, and active-selection `selection_text()`
    //    reads all return synchronously during an open DEC 2026 hold without
    //    releasing the hold or calling `RenderState::update`.
    {
        let (mirror, _tx, _) = scripted_mirror(false);
        mirror.resize_local(20, 3);
        let _view = mirror.view();
        mirror.test_feed(1, b"hello world");
        mirror.test_feed(2, b"\x1b[?2026h");
        assert!(mirror.is_render_held());

        assert_eq!(mirror.selection_text(), None);
        assert!(mirror.is_render_held());

        mirror.start_selection(
            SelectionKind::Block,
            Point::new(Line(0), Column(2)),
            Side::Left,
        );
        assert_eq!(mirror.selection_text(), None);
        assert!(mirror.is_render_held());
        mirror.test_feed(3, b"\x1b[?2026l");

        mirror.start_selection(
            SelectionKind::Simple,
            Point::new(Line(0), Column(0)),
            Side::Left,
        );
        mirror.update_selection(Point::new(Line(0), Column(4)), Side::Right);
        mirror.test_feed(4, b"\x1b[?2026h");
        assert!(mirror.is_render_held());
        assert_eq!(mirror.selection_text().as_deref(), Some("hello"));
        assert!(mirror.is_render_held());
        mirror.test_feed(5, b"\x1b[?2026l");
        assert!(!mirror.is_render_held());
    }
}

#[test]
fn sync_copy_and_input_ordering_during_dec_2026_hold_preserve_later_selections() {
    struct InputRecordingHost {
        frames: Mutex<Option<ScriptedFrames>>,
        inputs: Mutex<Vec<Vec<u8>>>,
    }

    impl runner_core::protocol::Transport for InputRecordingHost {
        fn call(
            &self,
            request: runner_core::protocol::Request,
        ) -> std::result::Result<runner_core::protocol::Response, ClientError> {
            match request {
                runner_core::protocol::Request::terminal_metadata { .. } => {
                    Ok(runner_core::protocol::Response::terminal_metadata(Ok(
                        TerminalMetadata::default(),
                    )))
                }
                runner_core::protocol::Request::terminal_observe_input { .. } => Ok(
                    runner_core::protocol::Response::terminal_observe_input(Ok(())),
                ),
                _ => Err(ClientError::msg("unexpected request")),
            }
        }
        fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
            unreachable!()
        }
        fn attach(&self, _: &str) -> std::result::Result<TerminalAttachment, ClientError> {
            Ok(TerminalAttachment {
                subscriber_id: 1,
                snapshot: TerminalSnapshot {
                    seq: 0,
                    format: Default::default(),
                    unavailable: false,
                    cols: 40,
                    rows: 3,
                    bytes: Vec::new(),
                    unfinished_len: 0,
                    preceding_char: None,
                    activity: None,
                    controller: Default::default(),
                },
                frames: Box::new(self.frames.lock().unwrap().take().expect("one attachment")),
            })
        }
        fn input(&self, _: &str, bytes: &[u8]) -> std::result::Result<(), ClientError> {
            self.inputs.lock().unwrap().push(bytes.to_vec());
            Ok(())
        }
    }

    let (tx, rx) = mpsc::channel();
    let host = Arc::new(InputRecordingHost {
        frames: Mutex::new(Some(ScriptedFrames(rx, tx.clone()))),
        inputs: Mutex::new(Vec::new()),
    });
    let mirror = TerminalMirror::attach(
        DaemonClient::new(host.clone()),
        "sync-copy-later-selection".into(),
        Arc::new(|| {}),
    )
    .unwrap();
    let inputs = &host.inputs;
    let _view = mirror.view();

    // 1. Empty Ctrl+C during a hold sends `\x03` immediately; selecting `"world"`
    //    afterward keeps `"world"` selected across hold release.
    mirror.test_feed(1, b"\x1b[2J\x1b[Hhello world\x1b[?2004h");
    mirror.test_feed(2, b"\x1b[?2026h");
    assert_eq!(mirror.selection_text(), None);
    assert!(mirror.send_key("c", true, false, false, None).unwrap());
    mirror.start_selection(
        SelectionKind::Simple,
        Point::new(Line(0), Column(6)),
        Side::Left,
    );
    mirror.update_selection(Point::new(Line(0), Column(10)), Side::Right);
    mirror.test_feed(3, b"\x1b[?2026l");
    assert_eq!(*inputs.lock().unwrap(), vec![b"\x03".to_vec()]);
    assert_eq!(mirror.selection_text().as_deref(), Some("world"));

    // 2. Non-empty Ctrl+C (`"hello"`) + `Enter` during a hold copies `"hello"`,
    //    clears it, sends `\r` immediately, and keeps a newer `"world"` selection.
    inputs.lock().unwrap().clear();
    mirror.start_selection(
        SelectionKind::Simple,
        Point::new(Line(0), Column(0)),
        Side::Left,
    );
    mirror.update_selection(Point::new(Line(0), Column(4)), Side::Right);
    mirror.test_feed(4, b"\x1b[?2026h");
    assert_eq!(mirror.selection_text().as_deref(), Some("hello"));
    mirror.clear_selection();
    assert!(mirror.send_key("enter", false, false, false, None).unwrap());
    mirror.start_selection(
        SelectionKind::Simple,
        Point::new(Line(0), Column(6)),
        Side::Left,
    );
    mirror.update_selection(Point::new(Line(0), Column(10)), Side::Right);
    mirror.test_feed(5, b"\x1b[?2026l");
    assert_eq!(*inputs.lock().unwrap(), vec![b"\r".to_vec()]);
    assert_eq!(mirror.selection_text().as_deref(), Some("world"));

    // 3. Copy during a hold -> Paste -> Enter immediately writes `[Paste, \r]`
    //    in natural order while keeping any selection made afterward.
    inputs.lock().unwrap().clear();
    mirror.test_feed(6, b"\x1b[?2026h");
    let copied = mirror.selection_text().unwrap();
    assert_eq!(copied, "world");
    mirror.paste(&copied).unwrap();
    assert!(mirror.send_key("enter", false, false, false, None).unwrap());
    mirror.start_selection(
        SelectionKind::Simple,
        Point::new(Line(0), Column(0)),
        Side::Left,
    );
    mirror.update_selection(Point::new(Line(0), Column(4)), Side::Right);
    mirror.test_feed(7, b"\x1b[?2026l");
    assert_eq!(
        *inputs.lock().unwrap(),
        vec![b"\x1b[200~world\x1b[201~".to_vec(), b"\r".to_vec()]
    );
    assert_eq!(mirror.selection_text().as_deref(), Some("hello"));

    // 4. `selection_text_if_published` decides hold state atomically on the
    //    actor thread, so racing it against a large opening-hold overwrite
    //    (`"FAIL"` -> `"PASS"`) can only observe `Ready(Some("FAIL"))` before
    //    the hold or `Held` once the hold is open — never unpublished `"PASS"`.
    use crate::terminal::SelectionSeed;
    let mut seq = 8u64;
    for pad_len in [64_000usize, 128_000] {
        mirror.test_feed(seq, b"\x1b[2J\x1b[HFAIL");
        seq += 1;
        mirror.start_selection(
            SelectionKind::Simple,
            Point::new(Line(0), Column(0)),
            Side::Left,
        );
        mirror.update_selection(Point::new(Line(0), Column(3)), Side::Right);
        assert_eq!(
            mirror.selection_text_if_published(),
            SelectionSeed::Ready(Some("FAIL".into()))
        );
        let mut chunk = Vec::with_capacity(pad_len + 32);
        chunk.extend_from_slice(b"\x1b[?2026h");
        while chunk.len() < pad_len {
            chunk.extend_from_slice(b"\x1b[2;1Hx");
        }
        chunk.extend_from_slice(b"\x1b[1;1HPASS");
        let future_t = Instant::now() + Duration::from_secs(60);
        let writer_mirror = Arc::clone(&mirror);
        let write_seq = seq;
        seq += 1;
        let writer = thread::spawn(move || {
            writer_mirror.test_feed_at(write_seq, &chunk, future_t);
        });
        let observed = mirror.selection_text_if_published();
        assert!(
            matches!(
                observed,
                SelectionSeed::Ready(Some(ref text)) if text == "FAIL"
            ) || observed == SelectionSeed::Held,
            "selection_text_if_published observed unexpected state during concurrent opening-hold write (pad_len={pad_len}): {observed:?}"
        );
        writer.join().unwrap();
        assert_eq!(mirror.selection_text_if_published(), SelectionSeed::Held);
        assert_eq!(mirror.selection_text().as_deref(), Some("PASS"));
        mirror.test_feed_at(seq, b"\x1b[?2026l", future_t + Duration::from_millis(10));
        seq += 1;
        assert_eq!(
            mirror.selection_text_if_published(),
            SelectionSeed::Ready(Some("PASS".into()))
        );
    }
}
