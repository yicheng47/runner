//! Phase 1a integration tests for Kitty graphics, stream handler scrolling/
//! clear/reset/resize/configure rules, synchronized updates, mode 1016,
//! `CSI 14 t` / `CSI 16 t`, SGR-Pixels / 1003 mouse encoders, local media,
//! daemon-wide `GlobalImageBudget`, and snapshot round-trips.

use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions as _;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use runner_terminal::kitty::media::{prepare_command, ForwardedItem, PreparedCommand, TempDirs};
use runner_terminal::kitty::scanner::{ScanItem, ScanOutput, Scanner};
use runner_terminal::kitty::storage::{
    CellSize, GlobalImageBudget, PinLocation, PlacementId, PlacementKey, PlacementLocation,
    TerminalGraphics, DEFAULT_GLOBAL_BUDGET,
};
use runner_terminal::kitty::{split_normalized_frames, ExecMode, MAX_NORMALIZED_FRAME_BYTES};
use runner_terminal::mappings::{
    encode_mouse_motion, encode_mouse_motion_at, encode_mouse_press, encode_mouse_press_at,
    encode_mouse_release, encode_mouse_release_at, encode_scroll, encode_scroll_at,
    session_pixel_position, MouseButton, MouseModifiers, MousePosition,
};
use runner_terminal::snapshot::{serialize_with_options, TrackedProcessor};
use runner_terminal::terminal::stream::{
    configure_term_and_graphics, resize_term_and_graphics, StreamHandler,
};
use runner_terminal::terminal::{ClipboardTarget, ModelOptions, TerminalHost, TerminalModel};

#[derive(Default)]
struct RecordingHost {
    replies: Mutex<Vec<Vec<u8>>>,
}

impl RecordingHost {
    fn take_replies(&self) -> Vec<String> {
        std::mem::take(&mut *self.replies.lock().unwrap())
            .into_iter()
            .map(|b| String::from_utf8(b).unwrap())
            .collect()
    }
}

impl TerminalHost for RecordingHost {
    fn write_input(&self, _: &str, _: &[u8]) -> anyhow::Result<()> {
        Ok(())
    }
    fn write_reply(&self, _: &str, bytes: &[u8]) -> anyhow::Result<()> {
        self.replies.lock().unwrap().push(bytes.to_vec());
        Ok(())
    }
    fn report_input_state(&self, _: &str, _: runner_terminal::input_state::InputObservation) {}
    fn set_live_title(&self, _: &str, _: &str) -> anyhow::Result<()> {
        Ok(())
    }
    fn resize(&self, _: &str, _: u16, _: u16) -> anyhow::Result<()> {
        Ok(())
    }
    fn is_local_host(&self, _: &str) -> bool {
        true
    }
    fn clipboard_store(&self, _: &str, _: ClipboardTarget, _: &str) {}
}

fn new_model(
    cols: u16,
    rows: u16,
    cell_size: CellSize,
    budget: Option<Arc<GlobalImageBudget>>,
) -> (Arc<TerminalModel>, Arc<RecordingHost>) {
    let host = Arc::new(RecordingHost::default());
    let model = TerminalModel::with_graphics(
        Arc::clone(&host) as Arc<dyn TerminalHost>,
        "test-session".into(),
        cols,
        rows,
        ModelOptions::default(),
        cell_size,
        budget,
    )
    .unwrap();
    (model, host)
}

fn flush_model_events(model: &TerminalModel) {
    let (tx, rx) = std::sync::mpsc::channel();
    assert!(model.apply_session_title(String::new(), move |_, _| {
        Some(Box::new(move || {
            let _ = tx.send(());
        }))
    }));
    rx.recv_timeout(Duration::from_secs(5)).unwrap();
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + data.len());
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc_input = Vec::with_capacity(4 + data.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(data);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    out
}

fn shutdown_model(model: &TerminalModel) {
    for worker in model.stop_workers() {
        let _ = worker.join();
    }
}

fn encode_png_rgba(width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().unwrap();
        writer.write_image_data(pixels).unwrap();
    }
    out
}

fn zlib_compress(data: &[u8]) -> Vec<u8> {
    let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
    enc.write_all(data).unwrap();
    enc.finish().unwrap()
}

fn apc(cmd: &str) -> Vec<u8> {
    format!("\x1b_G{cmd}\x1b\\").into_bytes()
}

fn scan_to_forwarded(
    scanner: &mut Scanner,
    bytes: &[u8],
    temp_dirs: &TempDirs,
) -> Vec<ForwardedItem> {
    match scanner.scan(bytes) {
        ScanOutput::Plain(text) => vec![ForwardedItem::Text(text.to_vec())],
        ScanOutput::Items(items) => items
            .into_iter()
            .map(|item| match item {
                ScanItem::Text(t) => ForwardedItem::Text(t),
                ScanItem::Command(raw) => match prepare_command(&raw, temp_dirs) {
                    Some(prepared) => ForwardedItem::Command(prepared),
                    None => ForwardedItem::Discarded,
                },
                ScanItem::Discarded => ForwardedItem::Discarded,
            })
            .collect(),
    }
}

/// Replay a normalized byte stream into a fresh mirror `Term` + `TerminalGraphics`.
fn replay_mirror(
    cols: usize,
    rows: usize,
    scrollback: usize,
    cell: CellSize,
    chunks: &[&[u8]],
) -> (Term<VoidListener>, TerminalGraphics, AtomicBool) {
    let mut term = Term::new(
        Config {
            scrolling_history: scrollback,
            kitty_keyboard: true,
            ..Config::default()
        },
        &TermSize::new(cols, rows),
        VoidListener,
    );
    let mut graphics = TerminalGraphics::mirror();
    let sgr_pixels = AtomicBool::new(false);
    let mut scanner = Scanner::new();
    let mut processor = TrackedProcessor::new();

    for chunk in chunks {
        let mut handler = StreamHandler::new(&mut term, &mut graphics, &sgr_pixels, cell, None);
        match scanner.scan(chunk) {
            ScanOutput::Plain(text) => processor.advance(&mut handler, text),
            ScanOutput::Items(items) => {
                for item in items {
                    match item {
                        ScanItem::Text(t) => processor.advance(&mut handler, &t),
                        ScanItem::Command(raw) => {
                            if let Some(prepared) = PreparedCommand::direct_only(&raw) {
                                handler.execute_kitty(&prepared, ExecMode::Mirror);
                            }
                        }
                        ScanItem::Discarded => {}
                    }
                }
            }
        }
    }
    (term, graphics, sgr_pixels)
}

#[allow(clippy::too_many_arguments)]
fn feed_term_with_stream(
    term: &mut Term<VoidListener>,
    graphics: &mut TerminalGraphics,
    sgr_pixels: &AtomicBool,
    cell: CellSize,
    processor: &mut TrackedProcessor,
    scanner: &mut Scanner,
    bytes: &[u8],
    mode: ExecMode,
) -> Vec<String> {
    let mut replies = Vec::new();
    let mut emit = |r: String| replies.push(r);
    let mut handler = StreamHandler::new(term, graphics, sgr_pixels, cell, Some(&mut emit));
    match scanner.scan(bytes) {
        ScanOutput::Plain(text) => processor.advance(&mut handler, text),
        ScanOutput::Items(items) => {
            for item in items {
                match item {
                    ScanItem::Text(t) => processor.advance(&mut handler, &t),
                    ScanItem::Command(raw) => {
                        if let Some(prepared) = PreparedCommand::direct_only(&raw) {
                            handler.execute_kitty(&prepared, mode);
                        }
                    }
                    ScanItem::Discarded => {}
                }
            }
        }
    }
    replies
}

fn make_term(cols: usize, rows: usize, scrollback: usize) -> Term<VoidListener> {
    Term::new(
        Config {
            scrolling_history: scrollback,
            kitty_keyboard: true,
            ..Config::default()
        },
        &TermSize::new(cols, rows),
        VoidListener,
    )
}

fn row_text<T>(term: &Term<T>, line: i32) -> String {
    runner_terminal::replay::row_cells(term, line)
        .trim_end()
        .to_string()
}

// ============================================================================
// 1. Scanner, ESC ESC _ G, non-Kitty APC, and stream-ordered replies
// ============================================================================

#[test]
fn scanner_split_at_every_byte_and_ordered_replies_across_graphics() {
    let cell = CellSize {
        width: 10,
        height: 20,
    };
    let (model, host) = new_model(40, 10, cell, None);

    // Interleave text queries, Kitty query, another APC, ESC ESC _ G, and text.
    let mut stream = Vec::new();
    stream.extend_from_slice(b"\x1b[14t"); // CSI 14 t -> 4;200;400t
    stream.extend_from_slice(b"\x1b[16t"); // CSI 16 t -> 6;20;10t
    stream.extend_from_slice(b"\x1b[?1016h\x1b[?1016$p"); // 1016 set -> 1
    stream.extend_from_slice(&apc("a=q,i=7,s=1,v=1,f=24;AAAA")); // kitty OK
    stream.extend_from_slice(b"\x1b_OTHER_APC\x1b\\"); // non-Kitty APC -> ignored by vte
    stream.extend_from_slice(b"\x1b\x1b_Ga=q,i=8,s=1,v=1,f=24;AAAA\x1b\\hello"); // ESC ESC _ G
    stream.extend_from_slice(b"\x1b[?1016l\x1b[?1016$p"); // 1016 reset -> 2
    stream.extend_from_slice(b"\x1b[18t"); // CSI 18 t -> 8;10;40t

    let mut scanner = Scanner::new();
    let temp = TempDirs::default();
    let mut all_frames = Vec::new();
    let mut next_seq = 1u64;
    // Feed byte-by-byte to stress every scanner and scheme-scanner split boundary.
    for &b in &stream {
        let items = scan_to_forwarded(&mut scanner, &[b], &temp);
        if items.is_empty() {
            continue;
        }
        let (_, frames, last_seq) = model.feed_forwarded(next_seq, &items);
        next_seq = last_seq + 1;
        all_frames.extend(frames);
    }
    flush_model_events(&model);

    assert_eq!(
        host.take_replies(),
        vec![
            "\x1b[4;200;400t",
            "\x1b[6;20;10t",
            "\x1b[?1016;1$y",
            "\x1b_Gi=7;OK\x1b\\",
            "\x1b_Gi=8;OK\x1b\\",
            "\x1b[?1016;2$y",
            "\x1b[8;10;40t",
        ]
    );
    assert_eq!(row_text(&model.term.lock(), 0), "hello");

    // Downstream mirror replaying the normalized stream must print "hello" intact
    // (proving `ESC ESC _ G` followed by a dropped `a=q` emitted CAN so 'h' was not eaten).
    let slices: Vec<&[u8]> = all_frames.iter().map(Vec::as_slice).collect();
    let (mirror_term, mirror_gfx, _) = replay_mirror(40, 10, 100, cell, &slices);
    assert_eq!(row_text(&mirror_term, 0), "hello");
    assert!(mirror_gfx.primary.is_empty());

    shutdown_model(&model);
}

#[test]
fn scanner_discard_over_limit_and_can_sub_esc_terminators() {
    let mut scanner = Scanner::with_max_bytes(16);
    let mut payload = b"before\x1b_Ga=t,i=1;".to_vec();
    payload.extend_from_slice(&[b'A'; 40]);
    payload.extend_from_slice(b"\x1b\\after");

    let ScanOutput::Items(items) = scanner.scan(&payload) else {
        panic!("expected items");
    };
    assert_eq!(
        items,
        vec![
            ScanItem::Text(b"before".to_vec()),
            ScanItem::Discarded,
            ScanItem::Text(b"after".to_vec()),
        ]
    );

    // CAN (0x18), SUB (0x1a), and stray ESC each end a discard cleanly and
    // pass the aborting control/escape sequence through to `vte` as text.
    for (term_bytes, expected_text) in [
        (b"\x18ok".as_slice(), b"\x18ok".as_slice()),
        (b"\x1aok".as_slice(), b"\x1aok".as_slice()),
        (b"\x1b[31mok".as_slice(), b"\x1b[31mok".as_slice()),
    ] {
        let mut sc = Scanner::with_max_bytes(8);
        let mut data = b"\x1b_Ga=t,i=1;AAAAAAAAAAAAAAAA".to_vec();
        data.extend_from_slice(term_bytes);
        let ScanOutput::Items(items) = sc.scan(&data) else {
            panic!("expected items");
        };
        assert_eq!(items, vec![ScanItem::Text(expected_text.to_vec())]);
    }
}

// ============================================================================
// 2. Commands, formats, chunking, relative placements, deletes & eviction
// ============================================================================

#[test]
fn ghostty_vectors_formats_chunking_quiet_ids_and_retransmit_abort() {
    let cell = CellSize {
        width: 10,
        height: 20,
    };
    let (model, host) = new_model(20, 10, cell, None);
    let mut scanner = Scanner::new();
    let temp = TempDirs::default();
    let mut seq = 1u64;
    let mut mirror_frames: Vec<Vec<u8>> = Vec::new();

    let mut send = |bytes: &[u8]| -> Vec<String> {
        let items = scan_to_forwarded(&mut scanner, bytes, &temp);
        let (_, frames, last) = model.feed_forwarded(seq, &items);
        seq = last + 1;
        mirror_frames.extend(frames);
        flush_model_events(&model);
        host.take_replies()
    };

    // f=24 RGB, f=32 RGBA, f=0 (RGBA), f=100 PNG, with and without o=z.
    assert_eq!(
        send(&apc("a=t,f=24,s=1,v=1,i=1;/wAA")),
        vec!["\x1b_Gi=1;OK\x1b\\"]
    );
    assert_eq!(
        send(&apc("a=t,f=0,s=1,v=2,i=2;///////////")), // unpadded + trailing bits
        vec!["\x1b_Gi=2;OK\x1b\\"]
    );
    let z_rgb = BASE64.encode(zlib_compress(&[10, 20, 30, 40, 50, 60]));
    assert_eq!(
        send(&apc(&format!("a=t,f=24,o=z,s=2,v=1,i=3;{z_rgb}"))),
        vec!["\x1b_Gi=3;OK\x1b\\"]
    );
    let png_bytes = encode_png_rgba(2, 2, &[255u8; 16]);
    let png_b64 = BASE64.encode(&png_bytes);
    assert_eq!(
        send(&apc(&format!("a=t,f=100,i=4;{png_b64}"))),
        vec!["\x1b_Gi=4;OK\x1b\\"]
    );
    let z_png_b64 = BASE64.encode(zlib_compress(&png_bytes));
    assert_eq!(
        send(&apc(&format!("a=t,f=100,o=z,i=5;{z_png_b64}"))),
        vec!["\x1b_Gi=5;OK\x1b\\"]
    );

    // Short and long data rejected.
    assert_eq!(
        send(&apc("a=t,f=24,s=2,v=2,i=6;AAAA")),
        vec!["\x1b_Gi=6;EINVAL: invalid data\x1b\\"]
    );
    assert_eq!(
        send(&apc("a=t,f=24,s=1,v=1,i=6;AAAAAAAA")),
        vec!["\x1b_Gi=6;EINVAL: invalid data\x1b\\"]
    );

    // Unknown format with q=0 vs q=3.
    assert_eq!(
        send(&apc("a=t,f=42,s=1,v=1,i=31;AAAA")),
        vec!["\x1b_Gi=31;EINVAL: unsupported format\x1b\\"]
    );
    assert!(send(&apc("a=t,f=42,s=1,v=1,i=31,q=3;AAAA")).is_empty());

    // Chunked with padded chunks + initial identifier error reporting.
    assert!(send(&apc("a=t,f=24,s=1,v=1,i=41,p=7,m=1;AA==")).is_empty());
    assert_eq!(
        send(&apc("m=0;AA==")),
        vec!["\x1b_Gi=41,p=7;EINVAL: invalid data\x1b\\"]
    );

    // Chunked q=1 inheritance and escalation.
    assert!(send(&apc("a=T,f=24,s=1,v=2,i=42,m=1,q=1;////")).is_empty());
    assert!(send(&apc("m=0;////")).is_empty());
    assert!(model.graphics().lock().unwrap().primary.contains_image(42));

    assert!(send(&apc("a=t,f=24,s=1,v=2,i=43,m=1,q=0;////")).is_empty());
    assert!(send(&apc("m=0,q=1;////")).is_empty());
    assert!(model.graphics().lock().unwrap().primary.contains_image(43));

    // i and I mutually exclusive for every action, rejected before mutating storage.
    for action in [
        "a=t,f=24,s=1,v=1,i=1,I=9;/wAA",
        "a=T,f=24,s=1,v=1,i=1,I=9;/wAA",
        "a=p,i=1,I=9,p=3",
        "a=d,d=I,i=1,I=9",
        "a=q,f=24,s=1,v=1,i=1,I=9;/wAA",
    ] {
        let replies = send(&apc(action));
        assert_eq!(replies.len(), 1, "{action}");
        assert!(
            replies[0].contains("EINVAL: image ID and number are mutually exclusive"),
            "{action} -> {:?}",
            replies
        );
    }
    assert!(model.graphics().lock().unwrap().primary.contains_image(1));

    // Query validates and stores nothing, does not overwrite image 1.
    assert_eq!(
        send(&apc("a=q,f=24,s=1,v=1,i=1;AAD/")),
        vec!["\x1b_Gi=1;OK\x1b\\"]
    );
    assert_eq!(
        model
            .graphics()
            .lock()
            .unwrap()
            .primary
            .image_by_id(1)
            .unwrap()
            .encoded_data
            .as_ref(),
        &[255, 0, 0]
    );

    // Implicit ID (no reply, skips occupied 2147483647) & number-based ID (smallest free).
    assert_eq!(
        send(&apc("a=t,f=24,s=1,v=1,i=2147483647;/wAA")),
        vec!["\x1b_Gi=2147483647;OK\x1b\\"]
    );
    assert!(send(&apc("a=t,f=24,s=1,v=1;AAD/")).is_empty());
    assert!(model
        .graphics()
        .lock()
        .unwrap()
        .primary
        .contains_image(2_147_483_648));
    assert_eq!(
        send(&apc("a=t,f=24,s=1,v=1,I=99;/wAA")),
        vec!["\x1b_Gi=6,I=99;OK\x1b\\"]
    );

    // Retransmitting an existing `i=` deletes the old image immediately on chunk 1,
    // even if chunk 2 fails — in BOTH runnerd AND the mirror!
    assert_eq!(
        send(&apc("a=p,i=1,p=5,C=1")),
        vec!["\x1b_Gi=1,p=5;OK\x1b\\"]
    );
    assert!(send(&apc("a=t,f=24,s=1,v=1,i=1,m=1;AA==")).is_empty());
    assert!(!model.graphics().lock().unwrap().primary.contains_image(1));
    assert_eq!(
        send(&apc("m=0;AA==")),
        vec!["\x1b_Gi=1;EINVAL: invalid data\x1b\\"]
    );
    assert!(!model.graphics().lock().unwrap().primary.contains_image(1));

    // Mirror replaying all normalized frames must agree on every surviving image
    // and must NOT contain image 1 (deleted on retransmit chunk 1).
    let slices: Vec<&[u8]> = mirror_frames.iter().map(Vec::as_slice).collect();
    let (_, mirror_gfx, _) = replay_mirror(20, 10, 100, cell, &slices);
    assert!(!mirror_gfx.primary.contains_image(1));
    assert!(mirror_gfx.primary.contains_image(2));
    assert!(mirror_gfx.primary.contains_image(3));
    assert!(mirror_gfx.primary.contains_image(4));
    assert!(mirror_gfx.primary.contains_image(5));
    assert!(mirror_gfx.primary.contains_image(6));
    assert!(mirror_gfx.primary.contains_image(42));
    assert!(mirror_gfx.primary.contains_image(43));
    assert!(mirror_gfx.primary.contains_image(2_147_483_647));
    assert!(mirror_gfx.primary.contains_image(2_147_483_648));

    shutdown_model(&model);
}

#[test]
fn zlib_bomb_and_oversized_png_refused_without_full_allocation() {
    let cell = CellSize {
        width: 10,
        height: 20,
    };
    let (model, host) = new_model(20, 10, cell, None);

    // A 1 MiB all-zero payload compresses to ~1 KiB; claim s=1,v=1 (expected 3 bytes).
    let bomb = zlib_compress(&vec![0u8; 1024 * 1024]);
    assert!(bomb.len() < 4096);
    let b64 = BASE64.encode(&bomb);
    model.feed_output(1, &apc(&format!("a=t,f=24,o=z,s=1,v=1,i=1;{b64}")));
    flush_model_events(&model);
    assert_eq!(
        host.take_replies(),
        vec!["\x1b_Gi=1;EINVAL: invalid data\x1b\\"]
    );

    // Oversized PNG header (9000x9000 RGBA = 324 MB > 320 MB screen limit) with a
    // dummy 1-byte IDAT chunk: `read_info()` reads IHDR and stops before IDAT, so
    // header validation rejects it with ENOMEM before decoding or allocating 324 MB.
    let mut fake_png = vec![137, 80, 78, 71, 13, 10, 26, 10];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&9000u32.to_be_bytes());
    ihdr.extend_from_slice(&9000u32.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
    fake_png.extend_from_slice(&png_chunk(b"IHDR", &ihdr));
    fake_png.extend_from_slice(&png_chunk(b"IDAT", &[0]));
    fake_png.extend_from_slice(&png_chunk(b"IEND", &[]));

    let png_b64 = BASE64.encode(&fake_png);
    model.feed_output(2, &apc(&format!("a=t,f=100,i=2;{png_b64}")));
    flush_model_events(&model);
    assert_eq!(
        host.take_replies(),
        vec!["\x1b_Gi=2;ENOMEM: out of memory\x1b\\"]
    );

    // Dimensions > 10,000 refused from PNG header with DimensionsTooLarge.
    let mut huge_png = vec![137, 80, 78, 71, 13, 10, 26, 10];
    let mut ihdr2 = Vec::new();
    ihdr2.extend_from_slice(&10001u32.to_be_bytes());
    ihdr2.extend_from_slice(&1u32.to_be_bytes());
    ihdr2.extend_from_slice(&[8, 6, 0, 0, 0]);
    huge_png.extend_from_slice(&png_chunk(b"IHDR", &ihdr2));
    huge_png.extend_from_slice(&png_chunk(b"IDAT", &[0]));
    huge_png.extend_from_slice(&png_chunk(b"IEND", &[]));
    let huge_b64 = BASE64.encode(&huge_png);
    model.feed_output(3, &apc(&format!("a=t,f=100,i=3;{huge_b64}")));
    flush_model_events(&model);
    assert_eq!(
        host.take_replies(),
        vec!["\x1b_Gi=3;EINVAL: dimensions too large\x1b\\"]
    );

    shutdown_model(&model);
}

#[test]
fn relative_placements_errors_cascades_and_all_delete_selectors() {
    let cell = CellSize {
        width: 10,
        height: 20,
    };
    let (model, host) = new_model(20, 10, cell, None);
    let seq = AtomicUsize::new(1);
    let send_raw = |bytes: &[u8]| -> Vec<String> {
        let s = seq.fetch_add(1, Ordering::Relaxed) as u64;
        model.feed_output(s, bytes);
        flush_model_events(&model);
        host.take_replies()
    };
    let send = |s: &str| -> Vec<String> { send_raw(&apc(s)) };

    // Transmit images 1..=12 (10x20 px -> 1x1 cell).
    let raw = BASE64.encode(vec![0xffu8; 10 * 20 * 3]);
    for id in 1..=12u32 {
        assert_eq!(
            send(&format!("a=t,f=24,s=10,v=20,i={id};{raw}")),
            vec![format!("\x1b_Gi={id};OK\x1b\\")]
        );
    }

    // Virtual placement with parent rejected before lookup.
    assert_eq!(
        send("a=p,i=999,U=1,P=1"),
        vec!["\x1b_Gi=999;EINVAL: virtual placement cannot refer to a parent\x1b\\"]
    );
    // Missing parent image / missing parent placement.
    assert_eq!(
        send("a=p,i=2,P=999"),
        vec!["\x1b_Gi=2;ENOPARENT: parent image not found\x1b\\"]
    );
    assert_eq!(
        send("a=p,i=2,P=1,Q=5"),
        vec!["\x1b_Gi=2;ENOPARENT: parent placement not found\x1b\\"]
    );

    // Place parent (i=1, p=2 and p=1) — parent fallback with Q=0 picks lowest external id (p=1).
    assert_eq!(send("a=p,i=1,p=2,C=1"), vec!["\x1b_Gi=1,p=2;OK\x1b\\"]);
    assert_eq!(send("a=p,i=1,p=1,C=1"), vec!["\x1b_Gi=1,p=1;OK\x1b\\"]);

    // Self-parent -> EINVAL (matching Ghostty graphics_exec.zig).
    assert_eq!(
        send("a=p,i=1,p=1,P=1,Q=1"),
        vec!["\x1b_Gi=1,p=1;EINVAL: placement cannot be its own parent\x1b\\"]
    );

    // Chain of 8 links succeeds; 9th link -> ETOODEEP.
    // Root is (1,1), then 2->1, 3->2, ..., 9->8 (depth 8), 10->9 (depth 9 fails).
    let mut prev = 1u32;
    for id in 2..=9u32 {
        let q = if prev == 1 { 0 } else { 1 }; // Q=0 on first link tests fallback to p=1
        assert_eq!(
            send(&format!("a=p,i={id},p=1,P={prev},Q={q},H=1,V=0")),
            vec![format!("\x1b_Gi={id},p=1;OK\x1b\\")]
        );
        prev = id;
    }
    assert_eq!(
        send("a=p,i=10,p=1,P=9,Q=1"),
        vec!["\x1b_Gi=10,p=1;ETOODEEP: parent chain too deep\x1b\\"]
    );

    // Cycle detection: re-parent (2,1) onto (3,1) -> ECYCLE.
    assert_eq!(
        send("a=p,i=2,p=1,P=3,Q=1"),
        vec!["\x1b_Gi=2,p=1;ECYCLE: parent chain creates a cycle\x1b\\"]
    );

    // Uppercase delete on parent (1,1) deletes cascaded placements 2..=9 AND frees their images.
    send("a=d,d=I,i=1,p=1");
    {
        let g = model.graphics().lock().unwrap();
        // Image 1 still has placement p=2, so image 1 stays; images 2..=9 were freed!
        assert!(g.primary.contains_image(1));
        for id in 2..=9u32 {
            assert!(!g.primary.contains_image(id), "image {id} should be freed");
        }
        assert!(g.primary.contains_image(10));
    }

    // Test every delete selector (a/A, i/I, n/N, c/C, p/P, q/Q, x/X, y/Y, z/Z, r/R).
    // Place images at known coordinates & z-indices.
    send_raw(b"\x1b[1;1H");
    send("a=p,i=10,p=1,z=-5,C=1"); // (row 0, col 0), z=-5
    send_raw(b"\x1b[2;3H");
    send("a=p,i=11,p=1,z=7,C=1"); // (row 1, col 2), z=7
    send_raw(b"\x1b[4;5H");
    send("a=p,i=12,p=1,z=3,C=1"); // (row 3, col 4), z=3

    // d=q (cell + z mismatch keeps; match deletes placement only).
    send("a=d,d=q,x=3,y=2,z=99");
    assert_eq!(
        model.graphics().lock().unwrap().primary.placements().len(),
        4
    );
    send("a=d,d=q,x=3,y=2,z=7");
    assert!(model.graphics().lock().unwrap().primary.contains_image(11)); // lowercase kept image
    assert_eq!(
        model.graphics().lock().unwrap().primary.placements().len(),
        3
    );

    // Re-place 11, test d=P (cell uppercase frees image).
    send_raw(b"\x1b[2;3H");
    send("a=p,i=11,p=1,z=7,C=1");
    send("a=d,d=P,x=3,y=2");
    assert!(!model.graphics().lock().unwrap().primary.contains_image(11));

    // d=C (at cursor) deletes image 12 placed at (row 3, col 4 -> 1-based 4;5).
    send_raw(b"\x1b[4;5H");
    send("a=d,d=C");
    assert!(!model.graphics().lock().unwrap().primary.contains_image(12));

    // d=Z frees z=-5 (image 10).
    send("a=d,d=Z,z=-5");
    assert!(!model.graphics().lock().unwrap().primary.contains_image(10));

    // Re-transmit 20..=23 for column, row, range, number, and all deletes.
    for id in 20..=23u32 {
        send(&format!("a=t,f=24,s=10,v=20,i={id},q=2;{raw}"));
    }
    send(&format!("a=t,f=24,s=10,v=20,I=77,q=2;{raw}")); // gets smallest free ID (2)
    send_raw(b"\x1b[1;2H");
    send("a=p,i=20,C=1,q=2"); // col 1 (1-based x=2), row 0
    send_raw(b"\x1b[3;1H");
    send("a=p,i=21,C=1,q=2"); // col 0, row 2 (1-based y=3)
    send("a=p,i=22,C=1,q=2");
    send("a=p,i=23,C=1,q=2");
    send("a=p,I=77,C=1,q=2");

    send("a=d,d=X,x=2");
    assert!(!model.graphics().lock().unwrap().primary.contains_image(20));
    send("a=d,d=Y,y=3");
    assert!(!model.graphics().lock().unwrap().primary.contains_image(21));
    assert!(!model.graphics().lock().unwrap().primary.contains_image(22));
    assert!(!model.graphics().lock().unwrap().primary.contains_image(23));
    send("a=d,d=N,I=77");
    assert!(model
        .graphics()
        .lock()
        .unwrap()
        .primary
        .image_by_number(77)
        .is_none());

    // d=r / d=R range delete.
    for id in 30..=32u32 {
        send(&format!("a=t,f=24,s=10,v=20,i={id},q=2;{raw}"));
        send(&format!("a=p,i={id},C=1,q=2"));
    }
    send("a=d,d=R,x=30,y=31");
    {
        let g = model.graphics().lock().unwrap();
        assert!(!g.primary.contains_image(30));
        assert!(!g.primary.contains_image(31));
        assert!(g.primary.contains_image(32));
    }
    // d=A deletes all visible placements and frees images.
    send("a=d,d=A");
    assert!(model.graphics().lock().unwrap().primary.is_empty());

    shutdown_model(&model);
}

#[test]
fn eviction_order_and_byte_accounting_for_rgb_and_rgba() {
    let cell = CellSize {
        width: 10,
        height: 20,
    };
    let mut term = make_term(20, 10, 100);
    let mut graphics = TerminalGraphics::authoritative(None);
    // Limit to 200 bytes total so eviction triggers deterministically.
    graphics.primary.set_limit(200);
    let sgr = AtomicBool::new(false);
    let mut proc = TrackedProcessor::new();
    let mut sc = Scanner::new();

    // f=24 5x5 = 75 bytes; f=32 5x5 = 100 bytes.
    let rgb75 = BASE64.encode(vec![1u8; 75]);
    let rgba100 = BASE64.encode(vec![2u8; 100]);

    // Image 1: placed (75 B). Image 2: unused (75 B). Total = 150 B.
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        &apc(&format!("a=T,f=24,s=5,v=5,i=1,C=1;{rgb75}")),
        ExecMode::Authoritative,
    );
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        &apc(&format!("a=t,f=24,s=5,v=5,i=2;{rgb75}")),
        ExecMode::Authoritative,
    );
    assert_eq!(graphics.primary.total_bytes(), 150);

    // Image 3 (100 B RGBA) needs 50 B evicted -> unused image 2 goes BEFORE placed image 1!
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        &apc(&format!("a=t,f=32,s=5,v=5,i=3;{rgba100}")),
        ExecMode::Authoritative,
    );
    assert!(graphics.primary.contains_image(1));
    assert!(!graphics.primary.contains_image(2));
    assert!(graphics.primary.contains_image(3));
    assert_eq!(graphics.primary.total_bytes(), 175);

    // Image over limit on its own -> ENOMEM, existing images untouched.
    let too_big = BASE64.encode(vec![3u8; 204]); // 51x1 RGBA = 204 > 200
    let replies = feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        &apc(&format!("a=t,f=32,s=51,v=1,i=4;{too_big}")),
        ExecMode::Authoritative,
    );
    assert_eq!(replies, vec!["\x1b_Gi=4;ENOMEM: out of memory\x1b\\"]);
    assert!(graphics.primary.contains_image(1));
    assert!(graphics.primary.contains_image(3));
}

// ============================================================================
// 3. Cursor, scrolling, partial regions, IL/DL, ED 2/3, RIS, 1049, configure, resize
// ============================================================================

#[test]
fn cursor_movement_wrapping_scrolling_and_bounded_rows() {
    let cell = CellSize {
        width: 10,
        height: 20,
    };
    let mut term = make_term(10, 4, 50);
    let mut graphics = TerminalGraphics::authoritative(None);
    let sgr = AtomicBool::new(false);
    let mut proc = TrackedProcessor::new();
    let mut sc = Scanner::new();
    let px = BASE64.encode(vec![0u8; 3]);

    // Transmit 1x1 RGB image, place with c=3, r=2, C=1 -> cursor stays at (0,0).
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        &apc(&format!("a=T,f=24,s=1,v=1,i=1,c=3,r=2,C=1;{px}")),
        ExecMode::Authoritative,
    );
    assert_eq!(
        (
            term.grid().cursor.point.column.0,
            term.grid().cursor.point.line.0
        ),
        (0, 0)
    );

    // C=0 at (0,0) with c=3, r=2 -> moves down 1 row (row 1), col 3.
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        &apc("a=p,i=1,p=2,c=3,r=2,C=0"),
        ExecMode::Authoritative,
    );
    assert_eq!(
        (
            term.grid().cursor.point.column.0,
            term.grid().cursor.point.line.0
        ),
        (3, 1)
    );

    // Reaching right edge (col 3 + c=7 = 10 >= 10) wraps to col 0 and adds 1 linefeed!
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        &apc("a=p,i=1,p=3,c=7,r=2,C=0"),
        ExecMode::Authoritative,
    );
    assert_eq!(
        (
            term.grid().cursor.point.column.0,
            term.grid().cursor.point.line.0
        ),
        (0, 3)
    );

    // Placing a 3-row image at bottom row (row 3) with C=0 scrolls 2 rows up,
    // and the newly placed image itself scrolls up from row 3 to row 1!
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        &apc("a=p,i=1,p=4,c=2,r=3,C=0"),
        ExecMode::Authoritative,
    );
    assert_eq!(
        (
            term.grid().cursor.point.column.0,
            term.grid().cursor.point.line.0
        ),
        (2, 3)
    );
    let p4 = graphics
        .primary
        .placements()
        .get(&PlacementKey {
            image_id: 1,
            placement_id: PlacementId::External(4),
        })
        .unwrap();
    assert_eq!(
        p4.location,
        PlacementLocation::Pin(PinLocation { line: 1, column: 0 })
    );

    // Untrusted huge r=1_000_000 is bounded (at most rows_before_scroll + screen_rows).
    let hist_before = term.history_size();
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        &apc("a=p,i=1,p=5,c=2,r=1000000,C=0"),
        ExecMode::Authoritative,
    );
    assert!(term.history_size() - hist_before <= 4);
}

#[test]
fn scroll_regions_il_dl_ed2_ed3_ris_1049_configure_and_resize() {
    let cell = CellSize {
        width: 10,
        height: 20,
    };
    let mut term = make_term(20, 6, 10);
    let mut graphics = TerminalGraphics::authoritative(None);
    let sgr = AtomicBool::new(false);
    let mut proc = TrackedProcessor::new();
    let mut sc = Scanner::new();
    let px = BASE64.encode(vec![0u8; 3]);

    let pin_line = |g: &TerminalGraphics, alt: bool, id: u32, pid: u32| -> Option<i32> {
        let storage = g.active(alt);
        storage
            .placements()
            .get(&PlacementKey {
                image_id: id,
                placement_id: PlacementId::External(pid),
            })
            .and_then(|p| match p.location {
                PlacementLocation::Pin(pin) => Some(pin.line),
                _ => None,
            })
    };

    // Transmit image 1 and place inside history (by scrolling) + on rows 1..5.
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        &apc(&format!("a=T,f=24,s=1,v=1,i=1,p=1,c=1,r=1,C=1;{px}")),
        ExecMode::Authoritative,
    );
    // Scroll full screen up by 2 rows -> p=1 moves to line -2 in history.
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        b"\x1b[2S",
        ExecMode::Authoritative,
    );
    assert_eq!(pin_line(&graphics, false, 1, 1), Some(-2));

    // Set partial region rows 2..5 (0-based 1..=4).
    // Place p=2 at row 2 (inside), p=3 straddling bottom margin (row 4, r=2), p=4 outside (row 0).
    let mut seq_bytes = b"\x1b[2;5r".to_vec();
    seq_bytes.extend_from_slice(b"\x1b[3;1H");
    seq_bytes.extend_from_slice(&apc("a=p,i=1,p=2,c=1,r=2,C=1")); // rows 2..=3 (wholly inside 1..=4)
    seq_bytes.extend_from_slice(b"\x1b[5;1H");
    seq_bytes.extend_from_slice(&apc("a=p,i=1,p=3,c=1,r=2,C=1")); // rows 4..=5 (straddles!)
    seq_bytes.extend_from_slice(b"\x1b[1;1H");
    seq_bytes.extend_from_slice(&apc("a=p,i=1,p=4,c=1,r=1,C=1")); // row 0 (outside)
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        &seq_bytes,
        ExecMode::Authoritative,
    );

    // SU 1 inside partial region 1..=4: p=2 moves 2->1; p=3 (straddling), p=4 (outside), p=1 (history) DO NOT MOVE!
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        b"\x1b[1S",
        ExecMode::Authoritative,
    );
    assert_eq!(pin_line(&graphics, false, 1, 1), Some(-2));
    assert_eq!(pin_line(&graphics, false, 1, 2), Some(1));
    assert_eq!(pin_line(&graphics, false, 1, 3), Some(4));
    assert_eq!(pin_line(&graphics, false, 1, 4), Some(0));

    // SD 1 (and RI at top margin row 1) inside region 1..=4: moves p=2 down 1->2->3.
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        b"\x1b[1T\x1b[2;1H\x1bM",
        ExecMode::Authoritative,
    );
    assert_eq!(pin_line(&graphics, false, 1, 2), Some(3));
    assert_eq!(pin_line(&graphics, false, 1, 1), Some(-2));

    // IL and DL inside region do NOT move screen placements.
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        b"\x1b[2;1H\x1b[1L\x1b[1M",
        ExecMode::Authoritative,
    );
    assert_eq!(pin_line(&graphics, false, 1, 2), Some(3));

    // Partial region starting at row 0 (rows 1..4 -> 0-based 0..=3):
    // Place p=5 at row 0 (r=1), then SU 1 -> p=5 clipped & deleted (not moved to history!),
    // while history placement p=1 moves up by 1 (-2 -> -3)!
    let mut top_region = b"\x1b[1;4r\x1b[1;1H".to_vec();
    top_region.extend_from_slice(&apc("a=p,i=1,p=5,c=1,r=1,C=1"));
    top_region.extend_from_slice(b"\x1b[1S");
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        &top_region,
        ExecMode::Authoritative,
    );
    assert_eq!(pin_line(&graphics, false, 1, 5), None);
    assert_eq!(pin_line(&graphics, false, 1, 1), Some(-3));

    // DL at row 0 pushes history placements up by 1 (-3 -> -4) without moving screen placements!
    let mut dl_seq = b"\x1b[1;1H".to_vec();
    dl_seq.extend_from_slice(&apc("a=p,i=1,p=6,c=1,r=1,C=1")); // screen placement on row 0
    dl_seq.extend_from_slice(b"\x1b[1;1H\x1b[1M");
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        &dl_seq,
        ExecMode::Authoritative,
    );
    assert_eq!(pin_line(&graphics, false, 1, 1), Some(-4));
    assert_eq!(pin_line(&graphics, false, 1, 6), Some(0)); // screen placement stayed!

    // Full-screen SD / RI moves screen placements down, clips at bottom, leaves history alone.
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        b"\x1b[r\x1b[1;1H\x1bM",
        ExecMode::Authoritative,
    );
    assert_eq!(pin_line(&graphics, false, 1, 6), Some(1));
    assert_eq!(pin_line(&graphics, false, 1, 1), Some(-4));

    // Lowering scrollback via configure prunes history placements above new topmost_line.
    configure_term_and_graphics(
        &mut term,
        &mut graphics,
        Config {
            scrolling_history: 2,
            ..Config::default()
        },
        cell,
    );
    assert_eq!(pin_line(&graphics, false, 1, 1), None); // -4 < -2 pruned!

    // ED 2: text on row 0 pushes row 0 into history (keeping a placement on row 0),
    // deletes visible placements still on screen, and frees unplaced image 9.
    let mut ed2_setup = b"\x1b[1;1Hkeep-me".to_vec();
    ed2_setup.extend_from_slice(&apc(&format!("a=t,f=24,s=1,v=1,i=9;{px}"))); // unplaced!
    ed2_setup.extend_from_slice(&apc("a=p,i=1,p=10,c=1,r=1,C=1")); // on row 0 (pushed into history by ED 2!)
    ed2_setup.extend_from_slice(b"\x1b[5;1H");
    ed2_setup.extend_from_slice(&apc("a=p,i=1,p=11,c=1,r=1,C=1")); // on empty row 4 (stays on screen -> deleted by ED 2!)
    ed2_setup.extend_from_slice(b"\x1b[2J");
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        &ed2_setup,
        ExecMode::Authoritative,
    );
    assert_eq!(pin_line(&graphics, false, 1, 10), Some(-1));
    assert_eq!(pin_line(&graphics, false, 1, 11), None);
    assert!(!graphics.primary.contains_image(9)); // unplaced image freed by ED 2!

    // ED 3 erases history placements only.
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        b"\x1b[3J",
        ExecMode::Authoritative,
    );
    assert_eq!(pin_line(&graphics, false, 1, 10), None);
    assert!(graphics.primary.contains_image(1));

    // Mode 1049: entering alt clears alt storage; primary untouched.
    let mut alt_seq = b"\x1b[1;1H".to_vec();
    alt_seq.extend_from_slice(&apc("a=p,i=1,p=20,c=1,r=1,C=1"));
    alt_seq.extend_from_slice(b"\x1b[?1049h");
    alt_seq.extend_from_slice(&apc(&format!("a=T,f=24,s=1,v=1,i=50,p=1,c=1,r=1,C=1;{px}")));
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        &alt_seq,
        ExecMode::Authoritative,
    );
    assert!(graphics.primary.contains_image(1));
    assert!(graphics.alt.contains_image(50));

    // Leaving and re-entering 1049 erases alt (like ED 2) while primary stays intact.
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        b"\x1b[?1049l\x1b[?1049h",
        ExecMode::Authoritative,
    );
    assert!(!graphics.alt.contains_image(50));
    assert!(graphics.primary.contains_image(1));

    // Resize on BOTH screens: shrink rows on alt screen with cursor at bottom scrolls alt up
    // and prunes top placement; shrink columns deletes placements past new_cols.
    let mut resize_setup = b"\x1b[1;1H".to_vec();
    resize_setup.extend_from_slice(&apc(&format!("a=T,f=24,s=1,v=1,i=60,p=1,c=1,r=1,C=1;{px}"))); // alt row 0, col 0
    resize_setup.extend_from_slice(b"\x1b[6;15H");
    resize_setup.extend_from_slice(&apc("a=p,i=60,p=2,c=1,r=1,C=1")); // alt row 5, col 14
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        &resize_setup,
        ExecMode::Authoritative,
    );
    // Shrink rows 6 -> 4 while cursor is at row 5 -> shifts alt up by 2 (p=1 at -2 pruned; p=2 at row 3).
    resize_term_and_graphics(&mut term, &mut graphics, 20, 4, cell);
    assert_eq!(pin_line(&graphics, true, 60, 1), None);
    assert_eq!(pin_line(&graphics, true, 60, 2), Some(3));
    // Shrink cols 20 -> 10 -> p=2 at col 14 >= 10 deleted!
    resize_term_and_graphics(&mut term, &mut graphics, 10, 4, cell);
    assert_eq!(pin_line(&graphics, true, 60, 2), None);

    // DECSTR (`CSI ! p`) does not reset graphics or mode 1016; RIS (`ESC c`) resets both.
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        b"\x1b[?1016h\x1b[!p",
        ExecMode::Authoritative,
    );
    assert!(sgr.load(Ordering::Acquire));
    assert!(graphics.primary.contains_image(1));

    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        b"\x1bc",
        ExecMode::Authoritative,
    );
    assert!(!sgr.load(Ordering::Acquire));
    assert!(graphics.is_empty());
}

#[test]
fn resize_primary_with_full_history_and_grow_from_history() {
    let cell = CellSize {
        width: 10,
        height: 20,
    };
    let mut term = make_term(20, 4, 4);
    let mut graphics = TerminalGraphics::authoritative(None);
    let sgr = AtomicBool::new(false);
    let mut proc = TrackedProcessor::new();
    let mut sc = Scanner::new();
    let px = BASE64.encode(vec![0u8; 3]);

    // Fill history completely (4 lines of history), with p=1 at line -3 and p=2 at line 2.
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        &apc(&format!("a=T,f=24,s=1,v=1,i=1,p=1,c=1,r=1,C=1;{px}")),
        ExecMode::Authoritative,
    );
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        b"\r\n\r\n\r\n\r\n\r\n\r\n\r\n", // 7 newlines on 4-row screen fills all 4 history lines; p=1 is at -4, cursor at row 3
        ExecMode::Authoritative,
    );
    assert_eq!(term.history_size(), 4);
    assert_eq!(
        graphics
            .primary
            .placements()
            .get(&PlacementKey {
                image_id: 1,
                placement_id: PlacementId::External(1),
            })
            .unwrap()
            .location,
        PlacementLocation::Pin(PinLocation {
            line: -4,
            column: 0
        })
    );
    let mut place2 = b"\x1b[3;1H".to_vec();
    place2.extend_from_slice(&apc("a=p,i=1,p=2,c=1,r=1,C=1"));
    place2.extend_from_slice(b"\x1b[4;1H"); // cursor at row 3
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        &place2,
        ExecMode::Authoritative,
    );

    // Shrink rows 4 -> 2 with cursor at row 3 (shift = -2):
    // p=1 moves -3 -> -5 (< topmost -4 -> pruned!), p=2 moves 2 -> 0 (survives!).
    resize_term_and_graphics(&mut term, &mut graphics, 20, 2, cell);
    assert!(!graphics.primary.placements().contains_key(&PlacementKey {
        image_id: 1,
        placement_id: PlacementId::External(1),
    }));
    assert_eq!(
        graphics
            .primary
            .placements()
            .get(&PlacementKey {
                image_id: 1,
                placement_id: PlacementId::External(2),
            })
            .unwrap()
            .location,
        PlacementLocation::Pin(PinLocation { line: 0, column: 0 })
    );

    // Grow rows 2 -> 5 with history_size == 4 -> pulls min(4, 3) = 3 lines from history -> p=2 shifts 0 -> 3!
    resize_term_and_graphics(&mut term, &mut graphics, 20, 5, cell);
    assert_eq!(
        graphics
            .primary
            .placements()
            .get(&PlacementKey {
                image_id: 1,
                placement_id: PlacementId::External(2),
            })
            .unwrap()
            .location,
        PlacementLocation::Pin(PinLocation { line: 3, column: 0 })
    );
}

#[test]
fn sync_session_title_clears_half_sent_command_and_chunked_load() {
    let cell = CellSize {
        width: 10,
        height: 20,
    };
    let (model, host) = new_model(20, 5, cell, None);

    // Old process starts a chunked load on primary AND leaves half an APC in the scanner.
    let mut half = apc("a=t,f=24,s=1,v=1,i=1,m=1;AA==");
    half.extend_from_slice(b"\x1b_Ga=t,i=2;");
    model.feed_output(1, &half);
    assert!(model.graphics().lock().unwrap().primary.loading.is_some());

    // Resume seam clears half-sent scanner and chunked loads on both screens.
    let done = Arc::new(AtomicBool::new(false));
    let done_flag = Arc::clone(&done);
    model.sync_session_title("fresh".into(), move || {
        done_flag.store(true, Ordering::Release);
    });
    flush_model_events(&model);
    assert!(done.load(Ordering::Acquire));
    assert!(model.graphics().lock().unwrap().primary.loading.is_none());

    // New process sends clean text and a brand-new image -> works with no leftover corruption!
    let mut fresh = b"clean".to_vec();
    fresh.extend_from_slice(&apc("a=t,f=24,s=1,v=1,i=9;/wAA"));
    model.feed_output(2, &fresh);
    flush_model_events(&model);
    assert_eq!(row_text(&model.term.lock(), 0), "clean");
    assert_eq!(host.take_replies(), vec!["\x1b_Gi=9;OK\x1b\\"]);

    shutdown_model(&model);
}

// ============================================================================
// 4. Synchronized updates (ordering, unfinished bytes, 150 ms cap)
// ============================================================================

#[test]
fn synchronized_update_places_image_between_runs_and_orders_csi_14_16_18() {
    let cell = CellSize {
        width: 10,
        height: 20,
    };
    let (model, host) = new_model(20, 5, cell, None);
    let px = BASE64.encode(vec![0u8; 3]);

    // Inside one closed sync update: "AB" (col 0..2), then a 3-col 1-row C=0 image
    // (lands at col 2, advances cursor to col 5), then "CD" (lands at col 5..7).
    let mut chunk = b"\x1b[?2026hAB".to_vec();
    chunk.extend_from_slice(&apc(&format!("a=T,f=24,s=1,v=1,i=1,p=1,c=3,r=1,C=0;{px}")));
    chunk.extend_from_slice(b"CD\x1b[?2026l");
    model.feed_output(1, &chunk);

    assert_eq!(model.snapshot(1).unfinished_len, 0);
    assert_eq!(row_text(&model.term.lock(), 0), "AB   CD");
    assert_eq!(
        model
            .graphics()
            .lock()
            .unwrap()
            .primary
            .placements()
            .get(&PlacementKey {
                image_id: 1,
                placement_id: PlacementId::External(1),
            })
            .unwrap()
            .location,
        PlacementLocation::Pin(PinLocation { line: 0, column: 2 })
    );
    flush_model_events(&model);
    let _ = host.take_replies();

    // Synchronized native + side-scanned queries (`CSI 14 t`, `CSI 16 t`,
    // `CSI ? 996 n`, `OSC 7501 ; ?`, `CSI 18 t`) must reply in exact stream
    // order both all-at-once and byte-by-byte.
    let mut seq = 2u64;
    for (input, expected) in [
        (
            b"\x1b[?2026h\x1b[14t\x1b[16t\x1b[?996n\x1b]7501;?\x07\x1b[18t\x1b[?2026l".as_slice(),
            vec![
                "\x1b[4;100;200t",
                "\x1b[6;20;10t",
                "\x1b[?997;1n",
                runner_terminal::program_status::PROBE_REPLY,
                "\x1b[8;5;20t",
            ],
        ),
        (
            b"\x1b[?2026h\x1b[16t\x1b[?996n\x1b]7501;?\x1b\\\x1b[14t\x1b[16t\x1b[?2026l".as_slice(),
            vec![
                "\x1b[6;20;10t",
                "\x1b[?997;1n",
                runner_terminal::program_status::PROBE_REPLY,
                "\x1b[4;100;200t",
                "\x1b[6;20;10t",
            ],
        ),
    ] {
        model.feed_output(seq, input);
        seq += 1;
        flush_model_events(&model);
        assert_eq!(host.take_replies(), expected, "single chunk {input:?}");

        for &b in input {
            model.feed_output(seq, &[b]);
            seq += 1;
        }
        flush_model_events(&model);
        assert_eq!(host.take_replies(), expected, "byte-by-byte {input:?}");
    }

    // Graphics command inside a synchronized update flushes all buffered native
    // and side-scanned queries in order before emitting the Kitty reply, then
    // trailing queries reply on close.
    let mut mixed = b"\x1b[?2026h\x1b[16t\x1b[?996n\x1b]7501;?\x07\x1b[14t".to_vec();
    mixed.extend_from_slice(&apc(&format!("a=q,f=24,s=1,v=1,i=7;{px}")));
    mixed.extend_from_slice(b"\x1b[18t\x1b[?996n\x1b[?2026l");
    model.feed_output(seq, &mixed);
    flush_model_events(&model);
    assert_eq!(
        host.take_replies(),
        vec![
            "\x1b[6;20;10t",
            "\x1b[?997;1n",
            runner_terminal::program_status::PROBE_REPLY,
            "\x1b[4;100;200t",
            "\x1b_Gi=7;OK\x1b\\",
            "\x1b[8;5;20t",
            "\x1b[?997;1n",
        ]
    );

    shutdown_model(&model);
}

#[test]
fn repeated_graphics_commands_inside_unclosed_sync_do_not_postpone_150ms_flush() {
    let cell = CellSize {
        width: 10,
        height: 20,
    };
    let (model, _host) = new_model(20, 5, cell, None);
    let px = BASE64.encode(vec![0u8; 3]);

    let opened = Instant::now();
    model.feed_output(1, b"\x1b[?2026hhello");

    // Repeatedly send graphics commands + trailing text without ever sending
    // `CSI ? 2026 l`. Reopening sync after each graphics command leaves
    // `unfinished_len > 0` until the sync-flush thread closes the update at the
    // original ~150 ms deadline.
    let mut seq = 2u64;
    let mut closed_by_timer = false;
    while opened.elapsed() < Duration::from_secs(2) {
        if model.snapshot(seq - 1).unfinished_len == 0 {
            closed_by_timer = true;
            break;
        }
        let mut msg = apc(&format!("a=q,f=24,s=1,v=1,i=1,q=2;{px}"));
        msg.extend_from_slice(b"x");
        model.feed_output(seq, &msg);
        seq += 1;
        let step_deadline = Instant::now() + Duration::from_millis(25);
        while Instant::now() < step_deadline {
            if model.snapshot(seq - 1).unfinished_len == 0 {
                closed_by_timer = true;
                break;
            }
            thread::yield_now();
        }
        if closed_by_timer {
            break;
        }
    }
    assert!(
        closed_by_timer,
        "sync update should close at the original 150 ms deadline despite repeated graphics commands"
    );
    assert!(row_text(&model.term.lock(), 0).starts_with("hello"));

    shutdown_model(&model);
}

// ============================================================================
// 5. Mode 1016, DECRQM, Geometry CSI 14 t & CSI 16 t
// ============================================================================

#[test]
fn mode_1016_transitions_decrqm_and_snapshot_round_trip() {
    let cell = CellSize {
        width: 10,
        height: 20,
    };
    let (model, host) = new_model(20, 5, cell, None);

    // Combined CSI ? 1003 ; 1016 h sets both MOUSE_MOTION and sgr_pixels.
    model.feed_output(1, b"\x1b[?1003;1016h");
    assert!(model.sgr_pixels());
    assert!(model.term.lock().mode().contains(TermMode::MOUSE_MOTION));

    // Set, query, reset, query in one chunk answers 1 then 2 in order.
    model.feed_output(2, b"\x1b[?1016h\x1b[?1016$p\x1b[?1016l\x1b[?1016$p");
    flush_model_events(&model);
    assert_eq!(
        host.take_replies(),
        vec!["\x1b[?1016;1$y", "\x1b[?1016;2$y"]
    );

    // Later 1006, 1005, or 1015 clears 1016; resetting 1016 preserves 1006.
    model.feed_output(3, b"\x1b[?1006h\x1b[?1016h");
    assert!(model.sgr_pixels());
    assert!(model.term.lock().mode().contains(TermMode::SGR_MOUSE));
    // Snapshot round-trip preserves BOTH 1006 and 1016 (1016 emitted after 1006).
    let snap = model.snapshot(3);
    let (restored_term, _, restored_sgr) = replay_mirror(20, 5, 100, cell, &[&snap.bytes]);
    assert!(restored_sgr.load(Ordering::Acquire));
    assert!(restored_term.mode().contains(TermMode::SGR_MOUSE));

    model.feed_output(4, b"\x1b[?1016l");
    assert!(!model.sgr_pixels());
    assert!(model.term.lock().mode().contains(TermMode::SGR_MOUSE));

    let mut seq = 10u64;
    for clearing in [
        b"\x1b[?1006h".as_slice(),
        b"\x1b[?1005h",
        b"\x1b[?1015h",
        b"\x1bc",
    ] {
        model.feed_output(seq, b"\x1b[?1016h");
        seq += 1;
        assert!(model.sgr_pixels());
        model.feed_output(seq, clearing);
        seq += 1;
        assert!(!model.sgr_pixels(), "clearing sequence {clearing:?} failed");
    }

    shutdown_model(&model);
}

#[test]
fn geometry_csi_14_and_16_zero_normal_large_u32_and_clamping() {
    let (model, host) = new_model(200, 100, CellSize::ZERO, None);

    // Zero before a cell size is known.
    model.feed_output(1, b"\x1b[14t\x1b[16t");
    flush_model_events(&model);
    assert_eq!(host.take_replies(), vec!["\x1b[4;0;0t", "\x1b[6;0;0t"]);

    // Large cell size exceeds u16::MAX (200 * 500 = 100_000, 100 * 800 = 80_000) without overflow.
    model.resize_with_cell_size(
        200,
        100,
        Some(CellSize {
            width: 500,
            height: 800,
        }),
    );
    model.feed_output(2, b"\x1b[14t\x1b[16t");
    flush_model_events(&model);
    assert_eq!(
        host.take_replies(),
        vec!["\x1b[4;80000;100000t", "\x1b[6;800;500t"]
    );

    // Zero cell size keeps the current one; >1024 clamps to 1024.
    model.resize_with_cell_size(200, 100, Some(CellSize::ZERO));
    assert_eq!(
        model.cell_size(),
        CellSize {
            width: 500,
            height: 800
        }
    );
    model.resize_with_cell_size(
        200,
        100,
        Some(CellSize {
            width: 5000,
            height: 2048,
        }),
    );
    assert_eq!(
        model.cell_size(),
        CellSize {
            width: 1024,
            height: 1024
        }
    );

    shutdown_model(&model);
}

// ============================================================================
// 6. SGR-Pixels and buttonless 1003 mouse encoders
// ============================================================================

#[test]
fn sgr_pixels_and_buttonless_1003_encoders() {
    let session = CellSize {
        width: 18,
        height: 36,
    };
    let viewer_dev = CellSize {
        width: 9,
        height: 18,
    };
    // Zero-based mapping and clamping.
    assert_eq!(
        session_pixel_position(0.0, 0.0, 9.0, 18.0, 10, 5, session, viewer_dev),
        (0, 0)
    );
    assert_eq!(
        session_pixel_position(-10.0, -5.0, 9.0, 18.0, 10, 5, session, viewer_dev),
        (0, 0)
    );
    assert_eq!(
        session_pixel_position(4.5, 9.0, 9.0, 18.0, 10, 5, session, viewer_dev),
        (9, 18)
    );
    assert_eq!(
        session_pixel_position(999.0, 999.0, 9.0, 18.0, 10, 5, session, viewer_dev),
        (179, 179)
    );
    // Session cell size zero falls back to viewer device cell size.
    assert_eq!(
        session_pixel_position(4.5, 9.0, 9.0, 18.0, 10, 5, CellSize::ZERO, viewer_dev),
        (4, 9)
    );

    let pos = MousePosition {
        column: 2,
        row: 3,
        pixel_x: 0,
        pixel_y: 0,
    };
    let pos2 = MousePosition {
        column: 2,
        row: 3,
        pixel_x: 42,
        pixel_y: 99,
    };
    let mods = MouseModifiers {
        shift: false,
        alt: true,
        control: false,
    };
    let any = TermMode::MOUSE_MOTION;

    // Press, release, drag, wheel, and buttonless motion under 1016.
    assert_eq!(
        encode_mouse_press_at(
            any,
            true,
            MouseButton::Left,
            pos,
            MouseModifiers::default(),
            false
        ),
        Some(b"\x1b[<0;0;0M".to_vec())
    );
    assert_eq!(
        encode_mouse_release_at(
            any,
            true,
            MouseButton::Left,
            pos2,
            MouseModifiers::default(),
            false
        ),
        Some(b"\x1b[<0;42;99m".to_vec())
    );
    assert_eq!(
        encode_mouse_motion_at(
            any,
            true,
            Some(MouseButton::Right),
            pos2,
            MouseModifiers::default(),
            false
        ),
        Some(b"\x1b[<34;42;99M".to_vec())
    );
    assert_eq!(
        encode_mouse_motion_at(any, true, None, pos2, mods, false),
        Some(b"\x1b[<43;42;99M".to_vec()) // 35 + alt(8) = 43
    );
    assert_eq!(
        encode_scroll_at(any, true, 1, false, pos2),
        Some(b"\x1b[<64;42;99M".to_vec())
    );
    assert_eq!(
        encode_scroll_at(any, true, -2, false, pos2),
        Some(b"\x1b[<65;42;99M\x1b[<65;42;99M".to_vec())
    );

    // Buttonless motion ONLY under mode 1003, never under 1000 or 1002 or bypass.
    assert_eq!(
        encode_mouse_motion_at(
            TermMode::MOUSE_REPORT_CLICK,
            true,
            None,
            pos2,
            MouseModifiers::default(),
            false
        ),
        None
    );
    assert_eq!(
        encode_mouse_motion_at(
            TermMode::MOUSE_DRAG,
            true,
            None,
            pos2,
            MouseModifiers::default(),
            false
        ),
        None
    );
    assert_eq!(
        encode_mouse_motion_at(any, true, None, pos2, MouseModifiers::default(), true),
        None
    );

    // Without 1016, SGR and X10 stay unchanged.
    let sgr = TermMode::MOUSE_DRAG | TermMode::SGR_MOUSE;
    assert_eq!(
        encode_mouse_press_at(
            sgr,
            false,
            MouseButton::Left,
            pos2,
            MouseModifiers::default(),
            false
        ),
        encode_mouse_press(
            sgr,
            MouseButton::Left,
            2,
            3,
            MouseModifiers::default(),
            false
        )
    );
    assert_eq!(
        encode_mouse_release_at(
            sgr,
            false,
            MouseButton::Left,
            pos2,
            MouseModifiers::default(),
            false
        ),
        encode_mouse_release(
            sgr,
            MouseButton::Left,
            2,
            3,
            MouseModifiers::default(),
            false
        )
    );
    assert_eq!(
        encode_mouse_motion_at(
            sgr,
            false,
            Some(MouseButton::Left),
            pos2,
            MouseModifiers::default(),
            false
        ),
        encode_mouse_motion(
            sgr,
            MouseButton::Left,
            2,
            3,
            MouseModifiers::default(),
            false
        )
    );
    assert_eq!(
        encode_scroll_at(sgr, false, 1, false, pos2),
        encode_scroll(sgr, 1, false, 2, 3)
    );
}

// ============================================================================
// 7. Local media safety (t=f, t=t, t=s)
// ============================================================================

#[cfg(unix)]
#[test]
fn local_media_t_f_t_t_and_t_s_safety_and_error_checks() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let temp_dirs = TempDirs::explicit(vec![tmp.path().to_path_buf()]);

    // Symlink to /dev/null refused by blocklist after opening.
    let link_path = tmp.path().join("link-to-dev-null");
    symlink("/dev/null", &link_path).unwrap();
    let raw = format!(
        "a=t,t=f,f=24,s=1,v=1,i=1;{}",
        BASE64.encode(link_path.to_str().unwrap())
    );
    let prep = prepare_command(raw.as_bytes(), &temp_dirs).unwrap();
    assert!(prep.media.unwrap().is_err());

    // FIFO refused without blocking.
    let fifo_path = tmp.path().join("test-fifo");
    let c_fifo = std::ffi::CString::new(fifo_path.to_str().unwrap()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c_fifo.as_ptr(), 0o600) }, 0);
    let raw_fifo = format!(
        "a=t,t=f,f=24,s=1,v=1,i=2;{}",
        BASE64.encode(fifo_path.to_str().unwrap())
    );
    let prep_fifo = prepare_command(raw_fifo.as_bytes(), &temp_dirs).unwrap();
    assert!(prep_fifo.media.unwrap().is_err());

    // t=t failing stateless header check (i + I) does NOT touch or delete the file!
    let untouched = tmp.path().join("tty-graphics-protocol-keep.bin");
    std::fs::write(&untouched, [1u8, 2, 3]).unwrap();
    let raw_bad_header = format!(
        "a=t,t=t,f=24,s=1,v=1,i=1,I=2;{}",
        BASE64.encode(untouched.to_str().unwrap())
    );
    let prep_bad = prepare_command(raw_bad_header.as_bytes(), &temp_dirs).unwrap();
    assert!(prep_bad.media.is_none());
    assert!(
        untouched.exists(),
        "file must not be deleted on failed stateless check"
    );

    // Valid t=t file is deleted even when image validation later fails (truncated 1 byte for 1x1 RGB).
    let truncated = tmp.path().join("tty-graphics-protocol-trunc.bin");
    std::fs::write(&truncated, [42u8]).unwrap();
    let raw_trunc = format!(
        "a=t,t=t,f=24,s=1,v=1,i=3;{}",
        BASE64.encode(truncated.to_str().unwrap())
    );
    let prep_trunc = prepare_command(raw_trunc.as_bytes(), &temp_dirs).unwrap();
    assert!(
        !truncated.exists(),
        "t=t file must be deleted after open+check"
    );
    let (model, host) = new_model(
        20,
        5,
        CellSize {
            width: 10,
            height: 20,
        },
        None,
    );
    let (_, frames, _) = model.feed_forwarded(1, &[ForwardedItem::Command(prep_trunc)]);
    assert!(frames.is_empty());
    flush_model_events(&model);
    assert_eq!(
        host.take_replies(),
        vec!["\x1b_Gi=3;EINVAL: invalid data\x1b\\"]
    );

    // t=f with S and O slicing + model execution.
    let slice_file = tmp.path().join("slice.bin");
    std::fs::write(&slice_file, [0u8, 0, 10, 20, 30, 99]).unwrap();
    let raw_slice = format!(
        "a=T,t=f,f=24,s=1,v=1,O=2,S=3,i=4,C=1;{}",
        BASE64.encode(slice_file.to_str().unwrap())
    );
    let prep_slice = prepare_command(raw_slice.as_bytes(), &temp_dirs).unwrap();
    let (_, frames, _) = model.feed_forwarded(2, &[ForwardedItem::Command(prep_slice)]);
    assert_eq!(frames.len(), 1);
    flush_model_events(&model);
    assert_eq!(host.take_replies(), vec!["\x1b_Gi=4;OK\x1b\\"]);
    assert_eq!(
        model
            .graphics()
            .lock()
            .unwrap()
            .primary
            .image_by_id(4)
            .unwrap()
            .encoded_data
            .as_ref(),
        &[10, 20, 30]
    );
    // Normalized frame contains direct data (`t=d`), never `t=f` or the file path!
    let frame_str = String::from_utf8(frames[0].clone()).unwrap();
    assert!(frame_str.contains("t=d"));
    assert!(!frame_str.contains("t=f"));
    assert!(!frame_str.contains("slice.bin"));

    shutdown_model(&model);
}

// ============================================================================
// 8. Daemon-wide GlobalImageBudget concurrency & isolation
// ============================================================================

#[test]
fn global_image_budget_enforced_across_concurrent_sessions_without_crossing_locks() {
    assert_eq!(DEFAULT_GLOBAL_BUDGET, 1024 * 1024 * 1024);

    // Budget = 300 bytes shared by two sessions; each 5x5 RGBA image is 100 bytes.
    let budget = Arc::new(GlobalImageBudget::new(300));
    let cell = CellSize {
        width: 10,
        height: 20,
    };
    let (m1, h1) = new_model(20, 5, cell, Some(Arc::clone(&budget)));
    let (m2, h2) = new_model(20, 5, cell, Some(Arc::clone(&budget)));
    let img100 = BASE64.encode(vec![7u8; 100]);

    // Session 1 takes 200 bytes (images 1 and 2).
    m1.feed_output(1, &apc(&format!("a=t,f=32,s=5,v=5,i=1;{img100}")));
    m1.feed_output(2, &apc(&format!("a=t,f=32,s=5,v=5,i=2;{img100}")));
    // Session 2 takes 100 bytes (image 10). Budget is now 300/300.
    m2.feed_output(1, &apc(&format!("a=t,f=32,s=5,v=5,i=10;{img100}")));
    assert_eq!(budget.used(), 300);

    // Session 2 tries to add a 200-byte image (needs 200 freed, but Session 2 only has 100).
    // Must fail with ENOMEM and keep image 10 AND never touch Session 1!
    let img200 = BASE64.encode(vec![8u8; 200]);
    m2.feed_output(2, &apc(&format!("a=t,f=32,s=10,v=5,i=11;{img200}")));
    flush_model_events(&m2);
    assert!(h2
        .take_replies()
        .iter()
        .any(|r| r == "\x1b_Gi=11;ENOMEM: out of memory\x1b\\"));
    assert!(m2.graphics().lock().unwrap().primary.contains_image(10));
    assert!(m1.graphics().lock().unwrap().primary.contains_image(1));
    assert!(m1.graphics().lock().unwrap().primary.contains_image(2));
    assert_eq!(budget.used(), 300);

    // Session 1 adds another 100-byte image -> evicts its own oldest image 1, never touches Session 2.
    m1.feed_output(3, &apc(&format!("a=t,f=32,s=5,v=5,i=3;{img100}")));
    assert!(!m1.graphics().lock().unwrap().primary.contains_image(1));
    assert!(m1.graphics().lock().unwrap().primary.contains_image(2));
    assert!(m1.graphics().lock().unwrap().primary.contains_image(3));
    assert!(m2.graphics().lock().unwrap().primary.contains_image(10));
    assert_eq!(budget.used(), 300);

    // Concurrent streaming on both sessions must never exceed budget.limit().
    let peak_exceeded = Arc::new(AtomicBool::new(false));
    let b1 = Arc::clone(&budget);
    let b2 = Arc::clone(&budget);
    let pe1 = Arc::clone(&peak_exceeded);
    let pe2 = Arc::clone(&peak_exceeded);
    let m1_t = Arc::clone(&m1);
    let m2_t = Arc::clone(&m2);
    let d1 = img100.clone();
    let d2 = img100.clone();

    let t1 = thread::spawn(move || {
        for i in 100..140u32 {
            m1_t.feed_output(i as u64, &apc(&format!("a=t,f=32,s=5,v=5,i={i},q=2;{d1}")));
            if b1.used() > b1.limit() {
                pe1.store(true, Ordering::Release);
            }
        }
    });
    let t2 = thread::spawn(move || {
        for i in 200..240u32 {
            m2_t.feed_output(i as u64, &apc(&format!("a=t,f=32,s=5,v=5,i={i},q=2;{d2}")));
            if b2.used() > b2.limit() {
                pe2.store(true, Ordering::Release);
            }
        }
    });
    t1.join().unwrap();
    t2.join().unwrap();
    assert!(!peak_exceeded.load(Ordering::Acquire));
    assert!(budget.used() <= budget.limit());

    // Dropping both sessions releases all bytes back to 0.
    shutdown_model(&m1);
    shutdown_model(&m2);
    drop(h1);
    drop(h2);
    drop(m1);
    drop(m2);
    assert_eq!(budget.used(), 0);
}

// ============================================================================
// 9. Snapshots (screen + scrollback + alt + relative + budget + companion-free)
// ============================================================================

#[test]
fn snapshot_round_trips_screen_scrollback_both_screens_and_relative_chains() {
    let cell = CellSize {
        width: 10,
        height: 20,
    };
    let (model, _host) = new_model(20, 4, cell, None);
    let px = BASE64.encode(vec![11u8, 22, 33]);

    // Primary screen: place image 1 at row 0, scroll it into history (-2),
    // place image 2 on row 1 with relative child image 3 (p=5 -> parent (2,1)).
    let mut seq = 1u64;
    let mut feed = |bytes: &[u8]| {
        model.feed_output(seq, bytes);
        seq += 1;
    };
    feed(b"hist-row");
    feed(&apc(&format!("a=T,f=24,s=1,v=1,i=1,p=1,c=2,r=1,C=1;{px}")));
    feed(b"\r\nline1\r\nline2\r\nline3\r\nline4\r\nline5");
    feed(b"\x1b[2;3H");
    feed(&apc(&format!("a=T,f=24,s=1,v=1,i=2,p=1,c=1,r=1,C=1;{px}")));
    feed(&apc(&format!(
        "a=T,f=24,s=1,v=1,i=3,p=5,P=2,Q=1,H=2,V=1;{px}"
    )));
    // Enter alt screen and place image 10 on alt screen row 2, col 4.
    feed(b"\x1b[?1049h\x1b[3;5Halt-text");
    feed(&apc(&format!("a=T,f=24,s=1,v=1,i=10,p=7,c=2,r=2,C=1;{px}")));

    // Full snapshot WITH graphics.
    let snap = model.snapshot_with_options(seq, usize::MAX, true);
    let (restored_term, restored_gfx, _) = replay_mirror(20, 4, 1000, cell, &[&snap.bytes]);
    assert!(restored_term.mode().contains(TermMode::ALT_SCREEN));
    // Alt screen image 10 and placement p=7 survived.
    assert!(restored_gfx.alt.contains_image(10));
    assert_eq!(
        restored_gfx
            .alt
            .placements()
            .get(&PlacementKey {
                image_id: 10,
                placement_id: PlacementId::External(7),
            })
            .unwrap()
            .location,
        PlacementLocation::Pin(PinLocation {
            line: 2,
            column: 12
        })
    );
    // Primary screen scrollback placement (1,1), screen placement (2,1), and relative child (3,5) survived!
    assert_eq!(
        restored_gfx
            .primary
            .placements()
            .get(&PlacementKey {
                image_id: 1,
                placement_id: PlacementId::External(1),
            })
            .unwrap()
            .location,
        PlacementLocation::Pin(PinLocation {
            line: -2,
            column: 8
        })
    );
    assert_eq!(
        restored_gfx
            .primary
            .placements()
            .get(&PlacementKey {
                image_id: 2,
                placement_id: PlacementId::External(1),
            })
            .unwrap()
            .location,
        PlacementLocation::Pin(PinLocation { line: 1, column: 2 })
    );
    let rel = restored_gfx
        .primary
        .placements()
        .get(&PlacementKey {
            image_id: 3,
            placement_id: PlacementId::External(5),
        })
        .unwrap();
    assert_eq!(
        restored_gfx.primary.resolved_anchor(rel),
        Some(PinLocation { line: 2, column: 4 })
    );

    // Companion snapshot (`include_graphics: false`) contains ZERO graphics commands.
    let companion_snap = model.snapshot_with_options(seq, usize::MAX, false);
    assert!(!companion_snap.bytes.windows(3).any(|w| w == b"\x1b_G"));
    let (_, companion_gfx, _) = replay_mirror(20, 4, 1000, cell, &[&companion_snap.bytes]);
    assert!(companion_gfx.is_empty());

    shutdown_model(&model);
}

#[test]
fn snapshot_budget_drops_lru_non_visible_images_first_and_stays_under_max_frame() {
    let cell = CellSize {
        width: 10,
        height: 20,
    };
    let mut term = make_term(20, 4, 100);
    let mut graphics = TerminalGraphics::authoritative(None);
    let sgr = AtomicBool::new(false);
    let mut proc = TrackedProcessor::new();
    let mut sc = Scanner::new();

    // Store three 15 MiB raw RGB images (each base64-encodes to ~20 MiB):
    // image 1 (oldest, non-visible/unused), image 2 (middle, non-visible/unused),
    // image 3 (newest, visible on screen). Total base64 would be ~60 MiB > 48 MiB budget.
    // Budget allows at most 2 images (~40 MiB <= 48 MiB): visible image 3 and newer image 2
    // must be kept, oldest unused image 1 dropped, and total snapshot < wire::MAX_FRAME!
    let data = vec![0x42u8; 15 * 1024 * 1024]; // 5120 x 1024 x 3 = 15 MiB
    let b64 = BASE64.encode(&data);
    for (id, action) in [(1, "a=t"), (2, "a=t"), (3, "a=T,p=1,c=1,r=1,C=1")] {
        feed_term_with_stream(
            &mut term,
            &mut graphics,
            &sgr,
            cell,
            &mut proc,
            &mut sc,
            &apc(&format!("{action},f=24,s=5120,v=1024,i={id},q=2;{b64}")),
            ExecMode::Authoritative,
        );
    }

    let snap = serialize_with_options(&term, &[], usize::MAX, false, Some((&graphics, cell)));
    assert!(snap.len() < runner_core::protocol::wire::MAX_FRAME);

    let (_, restored, _) = replay_mirror(20, 4, 100, cell, &[&snap]);
    assert!(
        !restored.primary.contains_image(1),
        "oldest LRU should be dropped"
    );
    assert!(
        restored.primary.contains_image(2),
        "newer image fits budget"
    );
    assert!(
        restored.primary.contains_image(3),
        "visible image kept last"
    );
}

#[test]
fn normalized_frame_splitting_and_empty_normalized_burst_sequence_accounting() {
    // Many sub-1 MiB commands pack into <= 1 MiB frames; a > 1 MiB command travels alone.
    let small = vec![b'a'; 400 * 1024];
    let big = vec![b'b'; 2 * 1024 * 1024];
    let frames = split_normalized_frames(vec![
        small.clone(),
        small.clone(),
        small.clone(),
        big.clone(),
        small.clone(),
    ]);
    assert_eq!(frames.len(), 4);
    assert_eq!(frames[0].len(), 800 * 1024);
    assert!(frames[0].len() <= MAX_NORMALIZED_FRAME_BYTES);
    assert_eq!(frames[1].len(), 400 * 1024);
    assert_eq!(frames[2].len(), 2 * 1024 * 1024);
    assert_eq!(frames[3].len(), 400 * 1024);

    // A burst whose normalized form is empty (`a=q` only) emits 0 frames,
    // advances `last_seq` to `first_seq`, and matches the snapshot's `output_seq`.
    let cell = CellSize {
        width: 10,
        height: 20,
    };
    let (model, host) = new_model(20, 4, cell, None);
    let mut sc = Scanner::new();
    let temp = TempDirs::default();
    let items = scan_to_forwarded(&mut sc, &apc("a=q,f=24,s=1,v=1,i=1;/wAA"), &temp);
    let (_, out_frames, last_seq) = model.feed_forwarded(5, &items);
    assert!(out_frames.is_empty());
    assert_eq!(last_seq, 5);
    flush_model_events(&model);
    assert_eq!(host.take_replies(), vec!["\x1b_Gi=1;OK\x1b\\"]);
    let snap = model.snapshot(last_seq);
    assert_eq!(snap.activity.unwrap().output_seq, 5);

    shutdown_model(&model);
}

// ============================================================================
// 10. Regression coverage for review findings (encoded budget, lazy mirror
//     decode, companion CAN & side-scanner aborts, i64 rows, reparent depth,
//     exact clipped snapshot budgeting)
// ============================================================================

#[test]
fn global_image_budget_accounts_for_encoded_bytes_across_png_and_zlib() {
    let cell = CellSize {
        width: 10,
        height: 20,
    };

    // Case A: small 1x1 PNG has decoded_bytes=4 but encoded_data.len() (~70 B) > decoded.
    let png_1x1 = encode_png_rgba(1, 1, &[255, 0, 0, 255]);
    assert!(png_1x1.len() > 40, "expected PNG overhead > 40 bytes");
    let budget_small = Arc::new(GlobalImageBudget::new(png_1x1.len() + 20));
    let (m1, _h1) = new_model(20, 5, cell, Some(Arc::clone(&budget_small)));
    let (m2, h2) = new_model(20, 5, cell, Some(Arc::clone(&budget_small)));

    let png_b64 = BASE64.encode(&png_1x1);
    m1.feed_output(1, &apc(&format!("a=t,f=100,i=1;{png_b64}")));
    assert_eq!(budget_small.used(), png_1x1.len());
    assert_eq!(m1.graphics().lock().unwrap().primary.total_bytes(), 4);
    assert_eq!(
        m1.graphics().lock().unwrap().primary.encoded_bytes(),
        png_1x1.len()
    );

    // Second 1x1 PNG in session 2 exceeds global encoded budget -> ENOMEM (not silently allowed!).
    m2.feed_output(1, &apc(&format!("a=t,f=100,i=2;{png_b64}")));
    flush_model_events(&m2);
    assert_eq!(
        h2.take_replies(),
        vec!["\x1b_Gi=2;ENOMEM: out of memory\x1b\\"]
    );
    assert!(!m2.graphics().lock().unwrap().primary.contains_image(2));
    assert!(m1.graphics().lock().unwrap().primary.contains_image(1));
    assert_eq!(budget_small.used(), png_1x1.len());

    shutdown_model(&m1);
    shutdown_model(&m2);
    drop(m1);
    drop(m2);
    assert_eq!(budget_small.used(), 0);

    // Case B: compressed o=z image has decoded_bytes=30_000 but encoded_data.len() < 200.
    let compressed = zlib_compress(&vec![0u8; 100 * 100 * 3]);
    assert!(compressed.len() < 200);
    let budget_z = Arc::new(GlobalImageBudget::new(compressed.len() * 2 + 16));
    let (mz, _hz) = new_model(20, 5, cell, Some(Arc::clone(&budget_z)));
    // Set per-screen decoded limit to 40_000 (holds 1 image = 30_000, not 2 = 60_000)
    // after verifying two fit when per-screen limit allows.
    let z_b64 = BASE64.encode(&compressed);
    mz.feed_output(1, &apc(&format!("a=t,f=24,o=z,s=100,v=100,i=1;{z_b64}")));
    mz.feed_output(2, &apc(&format!("a=t,f=24,o=z,s=100,v=100,i=2;{z_b64}")));
    assert_eq!(budget_z.used(), compressed.len() * 2);
    assert_eq!(mz.graphics().lock().unwrap().primary.total_bytes(), 60_000);

    // Tightening per-screen decoded limit forces eviction of both old images on
    // inserting image 3 (30_000 B decoded), releasing their encoded bytes accurately.
    mz.graphics().lock().unwrap().primary.set_limit(40_000);
    mz.feed_output(3, &apc(&format!("a=t,f=24,o=z,s=100,v=100,i=3;{z_b64}")));
    assert_eq!(budget_z.used(), compressed.len());
    assert_eq!(mz.graphics().lock().unwrap().primary.total_bytes(), 30_000);
    assert!(!mz.graphics().lock().unwrap().primary.contains_image(1));
    assert!(!mz.graphics().lock().unwrap().primary.contains_image(2));
    assert!(mz.graphics().lock().unwrap().primary.contains_image(3));

    shutdown_model(&mz);
    drop(mz);
    assert_eq!(budget_z.used(), 0);
}

#[test]
fn hidden_mirror_stores_normalized_png_and_zlib_without_decoding_under_lock() {
    let cell = CellSize {
        width: 10,
        height: 20,
    };
    let (model, _host) = new_model(20, 5, cell, None);
    let mut scanner = Scanner::new();
    let temp = TempDirs::default();

    let png_bytes = encode_png_rgba(
        2,
        2,
        &[
            10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 1, 2, 3, 255,
        ],
    );
    let png_b64 = BASE64.encode(&png_bytes);
    let z_rgb = zlib_compress(&[10, 20, 30, 40, 50, 60]);
    let z_b64 = BASE64.encode(&z_rgb);

    let mut input = apc(&format!("a=T,f=100,i=1,C=1;{png_b64}"));
    input.extend_from_slice(&apc(&format!("a=T,f=24,o=z,s=2,v=1,i=2,C=1;{z_b64}")));
    let items = scan_to_forwarded(&mut scanner, &input, &temp);
    let (_, frames, _) = model.feed_forwarded(1, &items);

    // Normalized PNG transmission carries explicit `s=2,v=2` dimensions.
    let joined: Vec<u8> = frames.iter().flatten().copied().collect();
    let joined_str = String::from_utf8(joined).unwrap();
    assert!(
        joined_str.contains("f=100,s=2,v=2"),
        "normalized PNG must carry s= and v=: {joined_str}"
    );

    let slices: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
    let (_, mirror_gfx, _) = replay_mirror(20, 5, 100, cell, &slices);
    let img1 = mirror_gfx.primary.image_by_id(1).unwrap();
    assert_eq!((img1.width, img1.height), (2, 2));
    assert_eq!(img1.decode_bgra(1024).unwrap()[..4], [30, 20, 10, 255]);
    let img2 = mirror_gfx.primary.image_by_id(2).unwrap();
    assert_eq!((img2.width, img2.height), (2, 1));
    assert_eq!(
        img2.decode_bgra(1024).unwrap(),
        vec![30, 20, 10, 255, 60, 50, 40, 255]
    );

    // Proof that mirrors skip PNG/zlib decoding under the terminal lock:
    // synthetic normalized commands with explicit s/v and non-decodable sentinel
    // bytes are accepted by ExecMode::Mirror (and rejected by Authoritative).
    let sentinel = BASE64.encode(b"NOT_A_PNG_OR_ZLIB");
    let synth_png = apc(&format!("a=t,t=d,i=9,f=100,s=2,v=2,q=2;{sentinel}"));
    let synth_z = apc(&format!("a=t,t=d,i=10,f=24,s=2,v=1,o=z,q=2;{sentinel}"));
    let (_, synth_mirror, _) = replay_mirror(20, 5, 100, cell, &[&synth_png, &synth_z]);
    assert_eq!(
        synth_mirror
            .primary
            .image_by_id(9)
            .map(|i| (i.width, i.height)),
        Some((2, 2))
    );
    assert_eq!(
        synth_mirror
            .primary
            .image_by_id(10)
            .map(|i| (i.width, i.height)),
        Some((2, 1))
    );

    // Authoritative model still validates and rejects the same sentinel payloads.
    model.feed_output(10, &synth_png);
    model.feed_output(11, &synth_z);
    assert!(!model.graphics().lock().unwrap().primary.contains_image(9));
    assert!(!model.graphics().lock().unwrap().primary.contains_image(10));

    // Un-normalized PNG missing `s=` / `v=` (e.g. raw PTY output from an older
    // remote host) is rejected on a mirror without invoking the PNG decoder.
    let missing_dims_png = apc(&format!("a=t,t=d,i=11,f=100,q=2;{png_b64}"));
    // Mismatched metadata (`s=4,v=4` wrapping a 2x2 PNG) is stored lazily on
    // the mirror, and deferred decode rejects the header/metadata mismatch.
    let mismatched_dims_png = apc(&format!("a=t,t=d,i=12,f=100,s=4,v=4,q=2;{png_b64}"));
    let (_, edge_mirror, _) =
        replay_mirror(20, 5, 100, cell, &[&missing_dims_png, &mismatched_dims_png]);
    assert!(
        !edge_mirror.primary.contains_image(11),
        "mirror must reject PNG missing s/v metadata without decoding under lock"
    );
    let mismatched = edge_mirror.primary.image_by_id(12).unwrap();
    assert_eq!((mismatched.width, mismatched.height), (4, 4));
    assert_eq!(
        mismatched.decode_rgba(4096),
        Err(runner_terminal::kitty::image::ImageError::InvalidData)
    );
    assert_eq!(
        mismatched.decode_bgra(4096),
        Err(runner_terminal::kitty::image::ImageError::InvalidData)
    );

    shutdown_model(&model);
}

#[test]
fn interrupted_sequences_emit_can_for_companions_and_abort_side_scanners() {
    let cell = CellSize {
        width: 10,
        height: 20,
    };
    let host = Arc::new(RecordingHost::default());
    let model = TerminalModel::with_graphics(
        Arc::clone(&host) as Arc<dyn TerminalHost>,
        "shell-abort".into(),
        40,
        5,
        ModelOptions {
            runtime: Some(runner_core::Runtime::Shell),
            ..ModelOptions::default()
        },
        cell,
        None,
    )
    .unwrap();
    let mut scanner = Scanner::new();
    let temp = TempDirs::default();

    // 1. Interrupted CSI (`\x1b[31`) + SUCCESSFUL transmit + `hello` must leave
    //    `hello` intact on model, local mirror, AND companion after stripping graphics!
    let mut seq1 = b"\x1b[31".to_vec();
    seq1.extend_from_slice(&apc("a=t,f=24,s=1,v=1,i=1;/wAA"));
    seq1.extend_from_slice(b"hello");
    let items = scan_to_forwarded(&mut scanner, &seq1, &temp);
    let (_, frames, last_seq) = model.feed_forwarded(1, &items);
    assert_eq!(row_text(&model.term.lock(), 0), "hello");

    let slices: Vec<&[u8]> = frames.iter().map(Vec::as_slice).collect();
    let (mirror_term, mirror_gfx, _) = replay_mirror(40, 5, 100, cell, &slices);
    assert_eq!(row_text(&mirror_term, 0), "hello");
    assert!(mirror_gfx.primary.contains_image(1));

    let companion_frames: Vec<Vec<u8>> = frames
        .iter()
        .map(|f| runner_terminal::kitty::strip_graphics_commands(f).unwrap_or_else(|| f.clone()))
        .collect();
    let comp_slices: Vec<&[u8]> = companion_frames.iter().map(Vec::as_slice).collect();
    let (comp_term, comp_gfx, _) = replay_mirror(40, 5, 100, cell, &comp_slices);
    assert_eq!(row_text(&comp_term, 0), "hello");
    assert!(comp_gfx.is_empty());

    // 2. Interrupted partial OSC 7501 (`\x1b]7501;?`) + graphics + `\x07` must NOT
    //    emit a spurious OSC 7501 probe reply, while preserving prior complete records.
    flush_model_events(&model);
    let _ = host.take_replies();

    model.feed_output(last_seq + 1, b"\x1b]7501;state=working:app=pi\x07");
    assert_eq!(model.take_program_statuses().len(), 1);

    let mut bad_osc7501 = b"\x1b]7501;?".to_vec();
    bad_osc7501.extend_from_slice(&apc("a=q,f=24,s=1,v=1,i=7;/wAA"));
    bad_osc7501.push(0x07);
    model.feed_output(last_seq + 2, &bad_osc7501);
    flush_model_events(&model);
    assert_eq!(
        host.take_replies(),
        vec!["\x1b_Gi=7;OK\x1b\\"],
        "interrupted partial OSC 7501 must not fire probe reply"
    );

    // 3. Interrupted partial OSC 7 (`\x1b]7;file:///tmp`) + graphics + `/x\x07`
    //    must not update live_cwd.
    let mut bad_osc7 = b"\x1b]7;file:///tmp".to_vec();
    bad_osc7.extend_from_slice(&apc("a=d,d=a"));
    bad_osc7.extend_from_slice(b"/x\x07");
    model.feed_output(last_seq + 3, &bad_osc7);
    assert_eq!(model.live_cwd(), None);

    // Subsequent valid OSC 7501 and OSC 7 still work cleanly.
    model.feed_output(
        last_seq + 4,
        b"\x1b]7501;state=done:app=pi\x07\x1b]7;file:///tmp\x07",
    );
    assert_eq!(model.take_program_statuses().len(), 1);
    assert_eq!(model.live_cwd(), Some(std::path::PathBuf::from("/tmp")));

    shutdown_model(&model);
}

#[test]
fn large_row_span_placements_do_not_overflow_on_scroll_prune_resize_or_snapshot() {
    let cell = CellSize {
        width: 10,
        height: 20,
    };
    let mut term = make_term(20, 6, 10);
    let mut graphics = TerminalGraphics::authoritative(None);
    let sgr = AtomicBool::new(false);
    let mut proc = TrackedProcessor::new();
    let mut sc = Scanner::new();
    let px = BASE64.encode(vec![0u8; 3]);

    // Place a huge-row-span placement (`r = i32::MAX as u32 + 10`) on row 1 inside
    // partial scroll region rows 2..5 (0-based 1..=4).
    let huge_r = (i32::MAX as u32) + 10;
    let mut setup = b"\x1b[2;5r\x1b[2;1H".to_vec();
    setup.extend_from_slice(&apc(&format!(
        "a=T,f=24,s=1,v=1,i=1,p=1,c=1,r={huge_r},C=1;{px}"
    )));
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        &setup,
        ExecMode::Authoritative,
    );

    let key = PlacementKey {
        image_id: 1,
        placement_id: PlacementId::External(1),
    };
    // Partial margin scroll must recognize it straddles the bottom margin and NOT move or delete it.
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        b"\x1b[1S\x1b[1T",
        ExecMode::Authoritative,
    );
    assert_eq!(
        graphics.primary.placements().get(&key).unwrap().location,
        PlacementLocation::Pin(PinLocation { line: 1, column: 0 })
    );

    // Full-screen scroll up pushes it to line -3; prune_above(-2) must KEEP it
    // because its last row (`-3 + huge_r - 1`) is far below `-2`!
    graphics.primary.scroll_full_up(4, -2, cell);
    assert_eq!(
        graphics.primary.placements().get(&key).unwrap().location,
        PlacementLocation::Pin(PinLocation {
            line: -3,
            column: 0
        })
    );

    // Resize must also keep it (last_row >= topmost) without wrapping negative.
    resize_term_and_graphics(&mut term, &mut graphics, 20, 5, cell);
    assert!(graphics.primary.placements().contains_key(&key));

    // Snapshot must succeed without overflowing or panicking.
    let snap = serialize_with_options(&term, &[], usize::MAX, false, Some((&graphics, cell)));
    assert!(!snap.is_empty());

    // Full-screen scroll down on a visible huge-r placement clips safely without panicking.
    graphics.primary.reset();
    let mut vis = b"\x1b[r\x1b[1;1H".to_vec();
    vis.extend_from_slice(&apc(&format!(
        "a=T,f=24,s=1,v=1,i=2,p=1,c=1,r={},C=1;{px}",
        u32::MAX
    )));
    feed_term_with_stream(
        &mut term,
        &mut graphics,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        &vis,
        ExecMode::Authoritative,
    );
    graphics.primary.scroll_full_down(1, 5, cell);
    // Moving from row 0 -> row 1 on a 5-row screen clips `u32::MAX - 4` rows off
    // the bottom, leaving `rows == 4` spanning rows 1..=4 without wrapping.
    let clipped = graphics
        .primary
        .placements()
        .get(&PlacementKey {
            image_id: 2,
            placement_id: PlacementId::External(1),
        })
        .expect("placement clipped to remaining 4 rows should survive");
    assert_eq!(
        clipped.location,
        PlacementLocation::Pin(PinLocation { line: 1, column: 0 })
    );
    assert_eq!(clipped.rows, 4);
}

#[test]
fn reparenting_subtree_beyond_max_depth_returns_etoodeep_and_snapshot_clips_to_exact_budget() {
    let cell = CellSize {
        width: 10,
        height: 20,
    };
    let (model, host) = new_model(20, 6, cell, None);
    let raw = BASE64.encode(vec![0xffu8; 10 * 20 * 3]);
    let seq = AtomicUsize::new(1);
    let send = |cmd: &str| -> Vec<String> {
        let s = seq.fetch_add(1, Ordering::Relaxed) as u64;
        model.feed_output(s, &apc(cmd));
        flush_model_events(&model);
        host.take_replies()
    };

    for id in 1..=12u32 {
        assert_eq!(
            send(&format!("a=t,f=24,s=10,v=20,i={id};{raw}")),
            vec![format!("\x1b_Gi={id};OK\x1b\\")]
        );
    }

    // Chain A (5 links): root (1,1) <- (2,1) <- (3,1) <- (4,1) <- (5,1) <- (6,1).
    assert_eq!(send("a=p,i=1,p=1,C=1"), vec!["\x1b_Gi=1,p=1;OK\x1b\\"]);
    for id in 2..=6u32 {
        assert_eq!(
            send(&format!("a=p,i={id},p=1,P={},Q=1", id - 1)),
            vec![format!("\x1b_Gi={id},p=1;OK\x1b\\")]
        );
    }

    // Chain B (3 links beneath q1): root2 (7,1) <- (8,1) <- (9,1) <- (10,1) <- (11,1).
    assert_eq!(send("a=p,i=7,p=1,C=1"), vec!["\x1b_Gi=7,p=1;OK\x1b\\"]);
    for id in 8..=11u32 {
        assert_eq!(
            send(&format!("a=p,i={id},p=1,P={},Q=1", id - 1)),
            vec![format!("\x1b_Gi={id},p=1;OK\x1b\\")]
        );
    }

    // Reparenting (8,1) (which has descendant depth 3: 9->10->11) onto (6,1) (depth 5)
    // would put (11,1) at depth 5 + 1 + 3 = 9 > 8 -> MUST fail with ETOODEEP!
    assert_eq!(
        send("a=p,i=8,p=1,P=6,Q=1"),
        vec!["\x1b_Gi=8,p=1;ETOODEEP: parent chain too deep\x1b\\"]
    );
    // Old parent (7,1) must remain unchanged.
    assert_eq!(
        model
            .graphics()
            .lock()
            .unwrap()
            .primary
            .placements()
            .get(&PlacementKey {
                image_id: 8,
                placement_id: PlacementId::External(1),
            })
            .unwrap()
            .location,
        PlacementLocation::Relative(runner_terminal::kitty::storage::RelativeLocation {
            parent: PlacementKey {
                image_id: 7,
                placement_id: PlacementId::External(1),
            },
            horizontal_offset: 0,
            vertical_offset: 0,
        })
    );

    // Reparenting (9,1) (descendant depth 2: 10->11) onto (6,1) (depth 5) gives
    // (11,1) exact depth 8 -> MUST succeed and round-trip cleanly in a snapshot!
    assert_eq!(send("a=p,i=9,p=1,P=6,Q=1"), vec!["\x1b_Gi=9,p=1;OK\x1b\\"]);

    // Wide-fanout tree check: give (12,1) 64 direct children, then verify
    // reparenting (12,1) onto depth-7 (10,1) fails with ETOODEEP (children
    // would reach depth 9) while reparenting onto depth-6 (9,1) succeeds
    // (children reach exact depth 8).
    assert_eq!(send("a=p,i=12,p=1,C=1"), vec!["\x1b_Gi=12,p=1;OK\x1b\\"]);
    for pid in 2..=65u32 {
        assert_eq!(
            send(&format!("a=p,i=12,p={pid},P=12,Q=1")),
            vec![format!("\x1b_Gi=12,p={pid};OK\x1b\\")]
        );
    }
    assert_eq!(
        send("a=p,i=12,p=1,P=10,Q=1"),
        vec!["\x1b_Gi=12,p=1;ETOODEEP: parent chain too deep\x1b\\"]
    );
    assert_eq!(
        send("a=p,i=12,p=1,P=9,Q=1"),
        vec!["\x1b_Gi=12,p=1;OK\x1b\\"]
    );

    let snap = model.snapshot(seq.load(Ordering::Relaxed) as u64);
    let (_, restored, _) = replay_mirror(20, 6, 100, cell, &[&snap.bytes]);
    let p11 = restored
        .primary
        .placements()
        .get(&PlacementKey {
            image_id: 11,
            placement_id: PlacementId::External(1),
        })
        .expect("depth-8 descendant must survive snapshot round-trip");
    assert!(restored.primary.resolved_anchor(p11).is_some());

    shutdown_model(&model);

    // Exact snapshot budget test for a clipped history placement whose crop adds
    // longer `x/y/w/h/r` keys than the unclipped placement.
    let mut term = make_term(20, 4, 10);
    let mut gfx = TerminalGraphics::authoritative(None);
    let sgr = AtomicBool::new(false);
    let mut proc = TrackedProcessor::new();
    let mut sc = Scanner::new();
    let px = BASE64.encode(vec![0xabu8; 10 * 100 * 3]); // 10x100 RGB = 5 rows at 10x20 cells
    let mut stream = b"\x1b[1;5H".to_vec();
    stream.extend_from_slice(&apc(&format!(
        "a=T,f=24,s=10,v=100,i=1,p=1,c=1,r=5,C=1;{px}"
    )));
    stream.extend_from_slice(b"\r\n\r\n\r\n\r\n\r\n\r\n"); // push placement to line -3
    feed_term_with_stream(
        &mut term,
        &mut gfx,
        &sgr,
        cell,
        &mut proc,
        &mut sc,
        &stream,
        ExecMode::Authoritative,
    );
    // Build a plan clipped to `first_line = -1` (crops top 2 rows -> adds y=40,w=10,h=60,r=3).
    let unlimited = runner_terminal::kitty::snapshot::SnapshotGraphicsPlan::build(
        &gfx,
        false,
        -1,
        4,
        4,
        20,
        cell,
        usize::MAX,
    );
    let mut emitted = String::new();
    unlimited.emit_screen_images(false, &gfx.primary, &mut emitted);
    unlimited.emit_row_placements(false, -1, &mut emitted);
    let exact_len = emitted.len();
    assert!(exact_len > 0);

    // Budget == exact_len MUST fit and emit <= budget bytes.
    let exact_plan = runner_terminal::kitty::snapshot::SnapshotGraphicsPlan::build(
        &gfx, false, -1, 4, 4, 20, cell, exact_len,
    );
    assert!(!exact_plan.is_empty());
    let mut exact_out = String::new();
    exact_plan.emit_screen_images(false, &gfx.primary, &mut exact_out);
    exact_plan.emit_row_placements(false, -1, &mut exact_out);
    assert_eq!(exact_out.len(), exact_len);

    // Budget == exact_len - 1 MUST drop the image rather than exceed budget.
    let tight_plan = runner_terminal::kitty::snapshot::SnapshotGraphicsPlan::build(
        &gfx,
        false,
        -1,
        4,
        4,
        20,
        cell,
        exact_len - 1,
    );
    assert!(tight_plan.is_empty());

    // Shared row wrapper (`\x1b7` / `\x1b8`) is charged ONCE per occupied row:
    // (a) one image with two placements on row 0;
    // (b) two distinct images sharing row 0.
    let mut term2 = make_term(20, 4, 10);
    let mut gfx2 = TerminalGraphics::authoritative(None);
    let mut proc2 = TrackedProcessor::new();
    let mut sc2 = Scanner::new();
    let tiny = BASE64.encode([1u8, 2, 3]);
    let mut same_row_one_img = b"\x1b[1;1H".to_vec();
    same_row_one_img.extend_from_slice(&apc(&format!("a=T,f=24,s=1,v=1,i=1,p=1,C=1;{tiny}")));
    same_row_one_img.extend_from_slice(b"\x1b[1;5H");
    same_row_one_img.extend_from_slice(&apc("a=p,i=1,p=2,C=1"));
    feed_term_with_stream(
        &mut term2,
        &mut gfx2,
        &sgr,
        cell,
        &mut proc2,
        &mut sc2,
        &same_row_one_img,
        ExecMode::Authoritative,
    );
    let emit_all_row0 = |plan: &runner_terminal::kitty::snapshot::SnapshotGraphicsPlan,
                         storage: &runner_terminal::kitty::storage::ImageStorage|
     -> String {
        let mut s = String::new();
        plan.emit_screen_images(false, storage, &mut s);
        plan.emit_row_placements(false, 0, &mut s);
        s
    };
    let one_img_unlimited = runner_terminal::kitty::snapshot::SnapshotGraphicsPlan::build(
        &gfx2,
        false,
        0,
        4,
        4,
        20,
        cell,
        usize::MAX,
    );
    let one_img_len = emit_all_row0(&one_img_unlimited, &gfx2.primary).len();
    let one_img_exact = runner_terminal::kitty::snapshot::SnapshotGraphicsPlan::build(
        &gfx2,
        false,
        0,
        4,
        4,
        20,
        cell,
        one_img_len,
    );
    assert_eq!(
        emit_all_row0(&one_img_exact, &gfx2.primary).len(),
        one_img_len,
        "one image with two placements on the same row must fit at its exact emitted byte length"
    );
    let one_img_tight = runner_terminal::kitty::snapshot::SnapshotGraphicsPlan::build(
        &gfx2,
        false,
        0,
        4,
        4,
        20,
        cell,
        one_img_len - 1,
    );
    assert!(one_img_tight.is_empty());

    // Add a second distinct image sharing row 0: both must fit at their exact
    // combined emitted length (row 0 wrapper paid only once, not twice).
    let mut second_img = b"\x1b[1;9H".to_vec();
    second_img.extend_from_slice(&apc(&format!("a=T,f=24,s=1,v=1,i=2,p=1,C=1;{tiny}")));
    feed_term_with_stream(
        &mut term2,
        &mut gfx2,
        &sgr,
        cell,
        &mut proc2,
        &mut sc2,
        &second_img,
        ExecMode::Authoritative,
    );
    let two_img_unlimited = runner_terminal::kitty::snapshot::SnapshotGraphicsPlan::build(
        &gfx2,
        false,
        0,
        4,
        4,
        20,
        cell,
        usize::MAX,
    );
    let two_img_out = emit_all_row0(&two_img_unlimited, &gfx2.primary);
    let two_img_len = two_img_out.len();
    assert!(two_img_out.contains("i=1") && two_img_out.contains("i=2"));

    let two_img_exact = runner_terminal::kitty::snapshot::SnapshotGraphicsPlan::build(
        &gfx2,
        false,
        0,
        4,
        4,
        20,
        cell,
        two_img_len,
    );
    let two_img_exact_out = emit_all_row0(&two_img_exact, &gfx2.primary);
    assert_eq!(two_img_exact_out.len(), two_img_len);
    assert!(two_img_exact_out.contains("i=1") && two_img_exact_out.contains("i=2"));

    let two_img_tight = runner_terminal::kitty::snapshot::SnapshotGraphicsPlan::build(
        &gfx2,
        false,
        0,
        4,
        4,
        20,
        cell,
        two_img_len - 1,
    );
    let two_img_tight_out = emit_all_row0(&two_img_tight, &gfx2.primary);
    assert!(two_img_tight_out.len() < two_img_len);
    assert!(two_img_tight_out.contains("i=2") && !two_img_tight_out.contains("i=1"));
}
