use super::*;

fn link_term(columns: usize, bytes: &[u8]) -> Term<alacritty_terminal::event::VoidListener> {
    let mut term = Term::new(
        Config::default(),
        &TermSize::new(columns, 8),
        alacritty_terminal::event::VoidListener,
    );
    crate::snapshot::TrackedProcessor::new().advance(&mut term, bytes);
    term
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
    let point = Point::new(term.topmost_line(), Column(5));
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
                    cols: 80,
                    rows: 24,
                    bytes: Vec::new(),
                    unfinished_len: 0,
                    preceding_char: None,
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
                cols: 80,
                rows: 24,
                bytes: Vec::new(),
                unfinished_len: 0,
                preceding_char: None,
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
        .term
        .lock()
        .grid()
        .display_iter()
        .map(|cell| cell.c)
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
    struct Frames(mpsc::Receiver<TerminalFrame>);
    impl TerminalSubscription for Frames {
        fn recv(&mut self) -> std::result::Result<TerminalFrame, ClientError> {
            self.0.recv().map_err(|_| ClientError::msg("closed"))
        }
        fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
            Arc::new(|| {})
        }
    }
    struct Host(Mutex<Option<mpsc::Receiver<TerminalFrame>>>);
    impl Transport for Host {
        fn call(&self, _: Request) -> std::result::Result<Response, ClientError> {
            Ok(Response::terminal_metadata(Ok(TerminalMetadata::default())))
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            unreachable!()
        }
        fn attach(&self, _: &str) -> std::result::Result<TerminalAttachment, ClientError> {
            let rx = self.0.lock().unwrap().take().unwrap();
            Ok(TerminalAttachment {
                subscriber_id: 0,
                snapshot: TerminalSnapshot {
                    seq: 0,
                    cols: 20,
                    rows: 5,
                    bytes: Vec::new(),
                    unfinished_len: 0,
                    preceding_char: None,
                    controller: Default::default(),
                },
                frames: Box::new(Frames(rx)),
            })
        }
    }
    let (frames, rx) = mpsc::channel();
    let wakes = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&wakes);
    let mirror = TerminalMirror::attach(
        DaemonClient::new(Arc::new(Host(Mutex::new(Some(rx))))),
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

    // A tab that wraps a full bottom row scrolls a held viewport too.
    let hold = mirror.hold_viewport();
    let mark = mirror.scroll_mark();
    mirror.test_feed(6, b"\x1b[5;1Habcdefghijklmnopqrst");
    assert_eq!(mark.moved_since(mirror.scroll_mark()), Some(0));
    mirror.test_feed(7, b"\t");
    assert_eq!(mark.moved_since(mirror.scroll_mark()), Some(1));
    assert_eq!(mirror.scroll_state().display_offset, 1);
    drop(hold);
    mirror.scroll_to_bottom();

    // A burst past the scrollback leaves a held viewport at its top, also
    // when it scrolls the ring buffer's whole length (1005 rows here) so
    // the same row allocations come back to the top.
    mirror.configure(20, CursorShape::Block, false);
    for (seq, burst) in [(8, 40), (9, 1005), (10, 1007)] {
        let mark = mirror.scroll_mark();
        let hold = mirror.hold_viewport();
        mirror.test_feed(seq, &lines(burst));
        let state = mirror.scroll_state();
        assert_eq!(
            (state.history_lines, state.display_offset),
            (20, 20),
            "{burst}"
        );
        assert_eq!(mark.moved_since(mirror.scroll_mark()), Some(burst as i32));
        drop(hold);
        mirror.scroll_to_bottom();
    }

    // A resize from the daemon, with the controller unchanged, still
    // wakes the views so an open find bar recomputes.
    let before = wakes.load(Ordering::SeqCst);
    let mark = mirror.scroll_mark();
    frames
        .send(TerminalFrame::Resized {
            seq: 11,
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
    assert_eq!(mark.moved_since(mirror.scroll_mark()), None);
    let resized = mirror.scroll_mark();
    mirror.test_feed(12, b"\x1b[?1049h");
    assert_eq!(resized.moved_since(mirror.scroll_mark()), None);
    drop(frames);
    mirror.test_stop_frames();
}

#[test]
fn shared_parse_keeps_only_the_bytes_held_after_a_capacity_restart() {
    let (tx, _) = mpsc::channel();
    let mut term = Term::new(
        Config::default(),
        &TermSize::new(20, 5),
        EventProxy {
            tx,
            waker: Arc::new(|| {}),
        },
    );
    let mut parser = ParserState::default();
    let mut first = b"\x1b[?2026h".to_vec();
    first.resize(1024 * 1024, b'x');
    let mut second = vec![b'y'; 1024 * 1024 - 12];
    second.extend_from_slice(b"\x1b[?2026hheld");
    for chunk in [first.as_slice(), second.as_slice(), b"z\x1b[?2026hnew"] {
        parse(&mut parser, &mut term, chunk);
    }
    assert_eq!(parser.processor.sync_bytes_count(), 3);
    assert_eq!(parser.boundary.unfinished().len(), 11);
    assert_eq!(parser.boundary.unfinished(), b"\x1b[?2026hnew");
}

#[test]
fn shell_title_sanitization_preserves_content_and_bounds_the_label() {
    assert_eq!(sanitize_title("  ⠋ shell ⠹  "), "  ⠋ shell ⠹  ");
    assert_eq!(sanitize_title("A\nB\tC\u{2028}D"), "A B C D");
    assert_eq!(sanitize_title(&"界".repeat(600)).chars().count(), 512);
}

fn osc7_payloads(chunks: &[&[u8]]) -> Vec<String> {
    let mut reports = super::CwdReports::default();
    chunks
        .iter()
        .flat_map(|chunk| reports.payloads(chunk))
        .map(|payload| String::from_utf8(payload).unwrap())
        .collect()
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
fn osc7_reports_abort_on_esc_can_and_sub_and_skip_overlong_payloads() {
    assert_eq!(
        osc7_payloads(&[b"\x1b]7;file:///a\x1b[0m\x1b]7;file:///b\x07"]),
        vec!["file:///b"]
    );
    assert_eq!(
        osc7_payloads(&[b"\x1b]7;file:///a\x1b", b"\x1b]7;file:///b\x07"]),
        vec!["file:///b"]
    );
    assert_eq!(
        osc7_payloads(&[b"\x1b]7;file:///a\x18 \x1b]7;file:///b\x1a \x1b]7;file:///c\x07"]),
        vec!["file:///c"]
    );
    let overlong = format!("\x1b]7;file:///{}", "a".repeat(super::OSC7_MAX_PAYLOAD));
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
    use super::kitty_flags_reply;
    let bits = |text| kitty_flags_reply(text).map(|flags| flags.bits());
    assert_eq!(bits("\x1b[?0u"), Some(0));
    assert_eq!(bits("\x1b[?31u"), Some(31));
    for other in [
        "\x1b[?u",
        "\x1b[?6c",
        "\x1b[?2031;0$y",
        "\x1b[0n",
        "\x1b[?1;2u",
        "\x1b[?999u",
    ] {
        assert_eq!(bits(other), None, "{other:?}");
    }
}

#[test]
fn scheme_sequences_are_seen_once_across_chunk_splits() {
    use super::{scan_scheme_sequences, SchemeSequence};
    let mut tail = Vec::new();
    assert_eq!(
        scan_scheme_sequences(&mut tail, b"\x1b[?2031h\x1b[?996n"),
        vec![(8, SchemeSequence::Subscribe), (15, SchemeSequence::Query)]
    );
    assert_eq!(scan_scheme_sequences(&mut tail, b"plain"), vec![]);
    assert_eq!(scan_scheme_sequences(&mut tail, b"\x1b[?20"), vec![]);
    assert_eq!(
        scan_scheme_sequences(&mut tail, b"31l"),
        vec![(3, SchemeSequence::Unsubscribe)]
    );
    assert_eq!(scan_scheme_sequences(&mut tail, b""), vec![]);
    assert_eq!(scan_scheme_sequences(&mut tail, b"\x1b[?25h"), vec![]);
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
        let captures = super::file_link_regex()
            .captures_iter(text)
            .find(|captures| {
                captures
                    .get(0)
                    .is_some_and(|m| m.start() <= offset && m.end() > offset)
            })?;
        let number = |name: &str| {
            captures
                .name(name)
                .and_then(|matched| matched.as_str().parse::<u32>().ok())
        };
        Some((
            captures.name("path")?.as_str().to_owned(),
            number("line")
                .or_else(|| number("pline"))
                .or_else(|| number("hline")),
            number("column").or_else(|| number("pcolumn")),
        ))
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
