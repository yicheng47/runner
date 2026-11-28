use std::alloc::{alloc as rust_alloc, dealloc as rust_dealloc, Layout};
use std::ffi::c_void;
use std::io::Cursor;
use std::ptr;
use std::slice;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use ghostty_vt_sys as sys;

use super::*;
use crate::palette::Rgb;
use crate::terminal::MAX_CLIPBOARD_BYTES;

fn encode_test_png(color_type: png::ColorType, width: u32, height: u32, pixels: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(color_type);
    encoder.set_depth(png::BitDepth::Eight);
    if color_type == png::ColorType::Indexed {
        encoder.set_palette(vec![255, 0, 0, 0, 255, 0, 0, 0, 255]);
    }
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(pixels).unwrap();
    drop(writer);
    out
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

fn craft_oversized_png_with_idat(width: u32, height: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = Vec::with_capacity(17);
    ihdr.extend_from_slice(b"IHDR");
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
    out.extend_from_slice(&13u32.to_be_bytes());
    out.extend_from_slice(&ihdr);
    out.extend_from_slice(&crc32(&ihdr).to_be_bytes());
    // Include a minimal IDAT chunk so `png::Decoder::read_info()` succeeds and
    // exercises our dimension/byte-budget guard rather than failing MissingImageData.
    let idat = b"IDAT\x78\x9c";
    out.extend_from_slice(&2u32.to_be_bytes());
    out.extend_from_slice(idat);
    out.extend_from_slice(&crc32(idat).to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(b"IEND");
    out.extend_from_slice(&crc32(b"IEND").to_be_bytes());
    out
}

fn collect_pty_replies(effects: &[TerminalEffect]) -> Vec<u8> {
    effects
        .iter()
        .filter_map(|e| match e {
            TerminalEffect::WritePty(b) => Some(b.as_slice()),
            _ => None,
        })
        .flatten()
        .copied()
        .collect()
}

#[test]
fn sys_png_callback_decodes_pixels_and_enforces_limits_before_allocating() {
    let cases: [(&str, png::ColorType, Vec<u8>, Vec<u8>); 5] = [
        (
            "rgba",
            png::ColorType::Rgba,
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        ),
        (
            "rgb",
            png::ColorType::Rgb,
            vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 100, 110, 120],
            vec![
                10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 100, 110, 120, 255,
            ],
        ),
        (
            "gray",
            png::ColorType::Grayscale,
            vec![10, 64, 128, 250],
            vec![
                10, 10, 10, 255, 64, 64, 64, 255, 128, 128, 128, 255, 250, 250, 250, 255,
            ],
        ),
        (
            "gray_alpha",
            png::ColorType::GrayscaleAlpha,
            vec![10, 20, 30, 40, 50, 60, 70, 80],
            vec![
                10, 10, 10, 20, 30, 30, 30, 40, 50, 50, 50, 60, 70, 70, 70, 80,
            ],
        ),
        (
            "indexed",
            png::ColorType::Indexed,
            vec![0, 1, 2, 0],
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 0, 0, 255,
            ],
        ),
    ];

    for (idx, (label, color_type, pixels, expected_rgba)) in cases.into_iter().enumerate() {
        let png_bytes = encode_test_png(color_type, 2, 2, &pixels);
        let decoded = unsafe { decode_png_into_ghostty(ptr::null(), &png_bytes) }
            .unwrap_or_else(|| panic!("direct decode failed for {label}"));
        assert_eq!((decoded.width, decoded.height), (2, 2), "{label}");
        let got = unsafe { slice::from_raw_parts(decoded.data, decoded.data_len) }.to_vec();
        unsafe { sys::ghostty_free(ptr::null(), decoded.data, decoded.data_len) };
        assert_eq!(got, expected_rgba, "pixel mismatch for {label}");

        let mut term = Terminal::new(&TerminalConfig {
            kitty_image_storage_limit: 1024 * 1024,
            ..TerminalConfig::default()
        })
        .unwrap();
        let b64 = STANDARD.encode(&png_bytes);
        let id = idx + 1;
        let cmd = format!("\x1b_Ga=t,f=100,i={id};{b64}\x1b\\");
        term.vt_write(cmd.as_bytes());
        let replies = collect_pty_replies(&term.take_effects());
        let expected = format!("\x1b_Gi={id};OK\x1b\\");
        assert_eq!(
            String::from_utf8_lossy(&replies),
            expected,
            "PNG decode via sys.h failed for {label}"
        );
    }

    // Counting custom allocator obeying `allocator.h` (`alloc` and `free` both provided).
    struct CountingCtx {
        alloc_calls: AtomicUsize,
        free_calls: AtomicUsize,
        fail_alloc: AtomicBool,
    }
    unsafe extern "C" fn counting_alloc(
        ctx: *mut c_void,
        len: usize,
        alignment: u8,
        _ret_addr: usize,
    ) -> *mut c_void {
        let state = unsafe { &*ctx.cast::<CountingCtx>() };
        state.alloc_calls.fetch_add(1, Ordering::SeqCst);
        if state.fail_alloc.load(Ordering::SeqCst) || len == 0 {
            return ptr::null_mut();
        }
        let align = 1usize << usize::from(alignment);
        let Ok(layout) = Layout::from_size_align(len, align) else {
            return ptr::null_mut();
        };
        unsafe { rust_alloc(layout) }.cast()
    }
    unsafe extern "C" fn counting_free(
        ctx: *mut c_void,
        memory: *mut c_void,
        memory_len: usize,
        alignment: u8,
        _ret_addr: usize,
    ) {
        let state = unsafe { &*ctx.cast::<CountingCtx>() };
        state.free_calls.fetch_add(1, Ordering::SeqCst);
        if memory.is_null() || memory_len == 0 {
            return;
        }
        let align = 1usize << usize::from(alignment);
        let layout = Layout::from_size_align(memory_len, align).unwrap();
        unsafe { rust_dealloc(memory.cast(), layout) };
    }
    let mut ctx = CountingCtx {
        alloc_calls: AtomicUsize::new(0),
        free_calls: AtomicUsize::new(0),
        fail_alloc: AtomicBool::new(false),
    };
    let counting_allocator = sys::GhosttyAllocator {
        ctx: (&raw mut ctx).cast(),
        vtable: &sys::GhosttyAllocatorVtable {
            alloc: Some(counting_alloc),
            free: Some(counting_free),
            ..Default::default()
        },
    };

    // Oversized fixtures must pass `png::Decoder::read_info()` (reaching our guard)
    // and still be rejected before any call to Ghostty's allocator.
    let encoded_wide = encode_test_png(
        png::ColorType::Grayscale,
        MAX_PNG_DIMENSION + 1,
        1,
        &vec![0u8; (MAX_PNG_DIMENSION + 1) as usize],
    );
    let crafted_huge = craft_oversized_png_with_idat(50_000, 50_000);
    for (w, h, bytes) in [
        (MAX_PNG_DIMENSION + 1, 1, encoded_wide),
        (50_000, 50_000, crafted_huge),
    ] {
        let header_reader = png::Decoder::new(Cursor::new(&bytes))
            .read_info()
            .unwrap_or_else(|e| panic!("oversized fixture {w}x{h} must pass read_info: {e}"));
        assert_eq!(
            (header_reader.info().width, header_reader.info().height),
            (w, h)
        );
        ctx.alloc_calls.store(0, Ordering::SeqCst);
        ctx.free_calls.store(0, Ordering::SeqCst);
        assert!(
            unsafe { decode_png_into_ghostty(&counting_allocator, &bytes) }.is_none(),
            "oversized header {w}x{h} should fail"
        );
        assert_eq!(
            ctx.alloc_calls.load(Ordering::SeqCst),
            0,
            "oversized header {w}x{h} must fail before allocator call"
        );
        assert_eq!(ctx.free_calls.load(Ordering::SeqCst), 0);
    }

    // In-range control through the same counting allocator allocates once and frees cleanly.
    let valid_small = encode_test_png(png::ColorType::Rgba, 1, 1, &[1, 2, 3, 255]);
    ctx.alloc_calls.store(0, Ordering::SeqCst);
    ctx.free_calls.store(0, Ordering::SeqCst);
    let img = unsafe { decode_png_into_ghostty(&counting_allocator, &valid_small) }
        .expect("in-range control should decode");
    assert_eq!(ctx.alloc_calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        unsafe { slice::from_raw_parts(img.data, img.data_len) },
        &[1, 2, 3, 255]
    );
    unsafe { sys::ghostty_free(&counting_allocator, img.data, img.data_len) };
    assert_eq!(ctx.free_calls.load(Ordering::SeqCst), 1);

    // Same in-range control fails cleanly when Ghostty's allocator denies allocation.
    ctx.fail_alloc.store(true, Ordering::SeqCst);
    ctx.alloc_calls.store(0, Ordering::SeqCst);
    ctx.free_calls.store(0, Ordering::SeqCst);
    assert!(unsafe { decode_png_into_ghostty(&counting_allocator, &valid_small) }.is_none());
    assert_eq!(ctx.alloc_calls.load(Ordering::SeqCst), 1);
    assert_eq!(ctx.free_calls.load(Ordering::SeqCst), 0);

    // Corrupt PNG through Kitty graphics gets an error reply.
    let mut term = Terminal::new(&TerminalConfig {
        kitty_image_storage_limit: 1024 * 1024,
        ..TerminalConfig::default()
    })
    .unwrap();
    let bad_b64 = STANDARD.encode(b"not a png");
    let cmd = format!("\x1b_Ga=t,f=100,i=99;{bad_b64}\x1b\\");
    term.vt_write(cmd.as_bytes());
    let replies = collect_pty_replies(&term.take_effects());
    let text = String::from_utf8_lossy(&replies);
    assert!(
        text.starts_with("\x1b_Gi=99;") && !text.contains(";OK"),
        "expected error reply for invalid PNG, got {text:?}"
    );
}

#[test]
fn sys_log_callback_routes_libghostty_warnings_to_tracing() {
    struct CaptureSubscriber {
        events: Arc<Mutex<Vec<(tracing::Level, String)>>>,
    }
    impl tracing::Subscriber for CaptureSubscriber {
        fn register_callsite(
            &self,
            metadata: &'static tracing::Metadata<'static>,
        ) -> tracing::subscriber::Interest {
            if metadata.target() == "libghostty_vt" {
                tracing::subscriber::Interest::sometimes()
            } else {
                tracing::subscriber::Interest::never()
            }
        }
        fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
            metadata.target() == "libghostty_vt"
        }
        fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
            tracing::span::Id::from_u64(1)
        }
        fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
        fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
        fn event(&self, event: &tracing::Event<'_>) {
            struct Msg(String);
            impl tracing::field::Visit for Msg {
                fn record_debug(
                    &mut self,
                    field: &tracing::field::Field,
                    value: &dyn std::fmt::Debug,
                ) {
                    if !self.0.is_empty() {
                        self.0.push(' ');
                    }
                    self.0.push_str(&format!("{}={value:?}", field.name()));
                }
            }
            let mut msg = Msg(String::new());
            event.record(&mut msg);
            self.events
                .lock()
                .unwrap()
                .push((*event.metadata().level(), msg.0));
        }
        fn enter(&self, _: &tracing::span::Id) {}
        fn exit(&self, _: &tracing::span::Id) {}
    }

    let events = Arc::new(Mutex::new(Vec::new()));
    let subscriber = CaptureSubscriber {
        events: Arc::clone(&events),
    };
    let mut term = Terminal::new(&TerminalConfig::default()).unwrap();
    // Register the `tracing::warn!` callsite inside `sys_log` before
    // `rebuild_interest_cache()` so a concurrent test thread cannot register
    // it with `NoSubscriber` (`Interest::never`) in between.
    term.vt_write(b"\x1b]7501;state=nonsense\x1b\\");
    tracing::subscriber::with_default(subscriber, || {
        tracing::callsite::rebuild_interest_cache();
        // Malformed OSC 7501 triggers `log.warn("invalid OSC 7501 report ...")` inside Ghostty.
        term.vt_write(b"\x1b]7501;state=nonsense\x1b\\");
    });
    let captured = events.lock().unwrap();
    assert!(
        captured
            .iter()
            .any(|(level, msg)| *level == tracing::Level::WARN && msg.contains("7501")),
        "expected tracing WARN from libghostty_vt, got {captured:?}"
    );
}

#[test]
fn terminal_callbacks_and_graphics_disabled_behave_as_configured() {
    let mut term = Terminal::new(&TerminalConfig {
        cols: 40,
        rows: 10,
        kitty_image_storage_limit: 0,
        color_scheme: ColorScheme::Dark,
        cell_size_px: (9, 18),
        ..TerminalConfig::default()
    })
    .unwrap();
    let palette = [Rgb { r: 0, g: 0, b: 0 }; 256];
    term.set_default_colors(
        Some(Rgb {
            r: 0xAA,
            g: 0xBB,
            b: 0xCC,
        }),
        Some(Rgb {
            r: 0x11,
            g: 0x22,
            b: 0x33,
        }),
        Some(Rgb {
            r: 0xEE,
            g: 0xDD,
            b: 0xCC,
        }),
        Some(&palette),
    )
    .unwrap();

    // Graphics disabled (limit = 0): Kitty query gets no OK reply.
    let png_b64 = STANDARD.encode(encode_test_png(png::ColorType::Rgb, 1, 1, &[1, 2, 3]));
    term.vt_write(format!("\x1b_Ga=q,f=100,i=1;{png_b64}\x1b\\").as_bytes());
    let effects = term.take_effects();
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, TerminalEffect::WritePty(b) if b.windows(2).any(|w| w == b"OK"))),
        "graphics should be disabled when storage limit is 0: {effects:?}"
    );

    // Core callbacks: title, pwd, color scheme, size, clipboard, sync hold, OSC 133, OSC 7501, RIS.
    term.vt_write(
        b"\x1b]2;hello\x07\
          \x1b]7;file:///tmp/dir\x07\
          \x1b[?996n\
          \x1b[16t\
          \x1b]52;c;aGk=\x07\
          \x1b[?2026h\x1b[?2026l\
          \x1b]133;C\x07\x1b]133;D;0\x07\x1b]133;A\x07\
          \x1b]7501;state=blocked:kind=permission:progress=42:id=a/b:app=codex:title=VGl0bGU=:msg=SGk=\x1b\\\
          \x1bc",
    );
    let effects = term.take_effects();
    assert!(effects
        .iter()
        .any(|e| matches!(e, TerminalEffect::TitleChanged(t) if t == "hello")));
    assert!(effects
        .iter()
        .any(|e| matches!(e, TerminalEffect::PwdChanged(p) if p == b"file:///tmp/dir")));
    assert!(effects
        .iter()
        .any(|e| matches!(e, TerminalEffect::WritePty(b) if b == b"\x1b[?997;1n")));
    assert!(effects
        .iter()
        .any(|e| matches!(e, TerminalEffect::WritePty(b) if b == b"\x1b[6;18;9t")));
    assert!(effects.iter().any(|e| matches!(
        e,
        TerminalEffect::ClipboardWrite {
            location: ClipboardLocation::Standard,
            text,
        } if text == "hi"
    )));
    assert!(effects
        .iter()
        .any(|e| matches!(e, TerminalEffect::RenderHold(true))));
    assert!(effects
        .iter()
        .any(|e| matches!(e, TerminalEffect::RenderHold(false))));
    assert!(effects.iter().any(|e| matches!(
        e,
        TerminalEffect::SemanticPrompt(SemanticPromptEvent {
            kind: SemanticPromptKind::CommandEnd,
            exit_code: Some(0),
            ..
        })
    )));
    assert!(effects.iter().any(|e| matches!(
        e,
        TerminalEffect::ProgramStatus(RawProgramStatus {
            state: ProgramStatusState::Blocked,
            kind: Some(ProgramStatusKind::Permission),
            progress: Some(42),
            id,
            app: Some(app),
            title: Some(title),
            message: Some(msg),
        }) if id == "a/b" && app == "codex" && title == "Title" && msg == "Hi"
    )));
    // RIS emits Clear followed by Reset.
    let clear_pos = effects.iter().position(|e| {
        matches!(
            e,
            TerminalEffect::ProgramStatus(RawProgramStatus {
                state: ProgramStatusState::Clear,
                id,
                ..
            }) if id.is_empty()
        )
    });
    let reset_pos = effects
        .iter()
        .position(|e| matches!(e, TerminalEffect::Reset));
    assert!(clear_pos.is_some() && reset_pos.is_some() && clear_pos < reset_pos);
}

#[test]
fn clipboard_write_preserves_osc52_rules_and_rejects_invalid_requests_truthfully() {
    let mut term = Terminal::new(&TerminalConfig::default()).unwrap();

    // OSC 52 targets, UTF-8, exact MAX_CLIPBOARD_BYTES cap, rejections, and silent reads.
    for (selector, expected_loc) in [
        ("c", ClipboardLocation::Standard),
        ("p", ClipboardLocation::Primary),
        ("s", ClipboardLocation::Selection),
    ] {
        term.vt_write(format!("\x1b]52;{selector};{}\x07", STANDARD.encode("runner ✓")).as_bytes());
        let writes: Vec<(ClipboardLocation, String)> = term
            .take_effects()
            .into_iter()
            .filter_map(|e| match e {
                TerminalEffect::ClipboardWrite { location, text } => Some((location, text)),
                _ => None,
            })
            .collect();
        assert_eq!(writes, vec![(expected_loc, "runner ✓".to_owned())]);
    }

    let max_payload = "a".repeat(MAX_CLIPBOARD_BYTES);
    term.vt_write(format!("\x1b]52;c;{}\x07", STANDARD.encode(&max_payload)).as_bytes());
    let writes: Vec<String> = term
        .take_effects()
        .into_iter()
        .filter_map(|e| match e {
            TerminalEffect::ClipboardWrite { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(writes.len(), 1);
    assert_eq!(writes[0].len(), MAX_CLIPBOARD_BYTES);

    let oversize_payload = format!("{max_payload}a");
    for seq in [
        format!("\x1b]52;c;{}\x07", STANDARD.encode(&oversize_payload)),
        "\x1b]52;c;\x07".to_owned(),
        format!("\x1b]52;c;{}\x07", STANDARD.encode(b"\xff\xfe")),
        "\x1b]52;c;not base64!\x07".to_owned(),
        "\x1b]52;c;?\x07".to_owned(),
    ] {
        term.vt_write(seq.as_bytes());
        let effects = term.take_effects();
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, TerminalEffect::ClipboardWrite { .. })),
            "invalid/oversize/empty/read OSC 52 must not queue ClipboardWrite"
        );
        assert!(
            collect_pty_replies(&effects).is_empty(),
            "OSC 52 must emit no PTY reply"
        );
    }

    // Exercise callback rejection acknowledgements (without pinning OSC 5522 acceptance).
    let send_5522 = |term: &mut Terminal, id: &str, entries: &[(&str, &[u8])]| {
        term.vt_write(format!("\x1b]5522;type=write:id={id}\x1b\\").as_bytes());
        for &(mime, data) in entries {
            let mime_b64 = STANDARD.encode(mime);
            let data_b64 = STANDARD.encode(data);
            term.vt_write(
                format!("\x1b]5522;type=wdata:mime={mime_b64};{data_b64}\x1b\\").as_bytes(),
            );
        }
        term.vt_write(b"\x1b]5522;type=wdata\x1b\\");
        let effects = term.take_effects();
        let replies = String::from_utf8_lossy(&collect_pty_replies(&effects)).into_owned();
        let writes: Vec<(ClipboardLocation, String)> = effects
            .into_iter()
            .filter_map(|e| match e {
                TerminalEffect::ClipboardWrite { location, text } => Some((location, text)),
                _ => None,
            })
            .collect();
        (replies, writes)
    };

    // Non-text MIME -> ENOSYS + nothing queued.
    let (replies, writes) = send_5522(&mut term, "png", &[("image/png", b"\x89PNG")]);
    assert_eq!(replies, "\x1b]5522;type=write:status=ENOSYS:id=png\x1b\\");
    assert!(writes.is_empty());

    // Multi-representation write cannot be committed atomically -> ENOSYS + nothing queued.
    let (replies, writes) = send_5522(
        &mut term,
        "multi",
        &[("text/plain", b"hello"), ("text/html", b"<b>hello</b>")],
    );
    assert_eq!(replies, "\x1b]5522;type=write:status=ENOSYS:id=multi\x1b\\");
    assert!(writes.is_empty());

    // Clear write (0 representations) -> ENOSYS + nothing queued.
    let (replies, writes) = send_5522(&mut term, "clr", &[]);
    assert_eq!(replies, "\x1b]5522;type=write:status=ENOSYS:id=clr\x1b\\");
    assert!(writes.is_empty());

    // Invalid UTF-8 in text/plain -> EINVAL + nothing queued.
    let (replies, writes) = send_5522(&mut term, "badutf8", &[("text/plain", b"\xff\xfe")]);
    assert_eq!(
        replies,
        "\x1b]5522;type=write:status=EINVAL:id=badutf8\x1b\\"
    );
    assert!(writes.is_empty());

    // Non-UTF-8 charset parameter -> ENOSYS + nothing queued.
    let (replies, writes) = send_5522(
        &mut term,
        "latin1",
        &[("text/plain;charset=iso-8859-1", b"hi")],
    );
    assert_eq!(
        replies,
        "\x1b]5522;type=write:status=ENOSYS:id=latin1\x1b\\"
    );
    assert!(writes.is_empty());

    // Oversize write hits Ghostty's configured max bytes -> EFBIG + nothing queued.
    let mut small_limit = Terminal::new(&TerminalConfig {
        clipboard_write_max_bytes: 8,
        ..TerminalConfig::default()
    })
    .unwrap();
    let (replies, writes) = send_5522(&mut small_limit, "big", &[("text/plain", b"012345678")]);
    assert_eq!(replies, "\x1b]5522;type=write:status=EFBIG:id=big\x1b\\");
    assert!(writes.is_empty());
}

#[test]
fn snapshot_round_trips_split_escape_sequences_and_render_state_graphemes() {
    let config = TerminalConfig {
        cols: 20,
        rows: 5,
        ..TerminalConfig::default()
    };
    let mut term = Terminal::new(&config).unwrap();
    // Write an ASCII cell, a combining grapheme cluster, and leave an unfinished
    // CSI color sequence across the snapshot boundary.
    term.vt_write("Ae\u{0301}\r\n\x1b[38;2;255;0;".as_bytes());
    assert!(!term.vt_ground());
    let snap = term.encode_snapshot().unwrap();
    assert!(!snap.is_empty());

    let mut restored =
        Terminal::decode_snapshot(&snap, SnapshotDecodeOptions::default(), &config).unwrap();
    assert!(!restored.vt_ground());
    restored.vt_write(b"0mred");
    assert!(restored.vt_ground());

    let mut render = RenderState::new().unwrap();
    render.update(&restored).unwrap();
    let rows = render.collect_rows().unwrap();
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0].cells[0].info.codepoint, 'A' as u32);
    assert_eq!(rows[0].cells[0].graphemes, vec!['A']);
    assert_eq!(rows[0].cells[1].info.codepoint, 'e' as u32);
    assert_eq!(rows[0].cells[1].graphemes, vec!['e', '\u{0301}']);
    assert!(rows[0].cells[2].graphemes.is_empty());
    assert_eq!(rows[1].cells[0].info.codepoint, 'r' as u32);
    assert_eq!(rows[1].cells[0].graphemes, vec!['r']);
    assert_eq!(
        rows[1].cells[0].style.fg,
        StyleColor::Rgb(Rgb { r: 255, g: 0, b: 0 })
    );
}

#[test]
fn search_selection_handle_guards_and_encoders_work_end_to_end() {
    let mut term = Terminal::new(&TerminalConfig {
        cols: 20,
        rows: 5,
        ..TerminalConfig::default()
    })
    .unwrap();
    let other_term = Terminal::new(&TerminalConfig {
        cols: 20,
        rows: 5,
        ..TerminalConfig::default()
    })
    .unwrap();
    term.vt_write(b"alpha beta\r\nALPHA gamma");

    let mut search = Search::new(&term).unwrap();
    search.set_select_scroll(SearchScroll::None).unwrap();
    // Wrong terminal is rejected.
    assert_eq!(
        search.set_needle(&other_term, Some("alpha")),
        Err(VtError::InvalidValue)
    );
    search.set_needle(&term, Some("alpha")).unwrap();
    search.run(&term).unwrap();
    assert_eq!(search.total_matches(), 2);
    assert!(search.select_next(&mut term).unwrap());
    assert_eq!(search.selected_index(), Some(0));
    {
        let matched = search.selected_match(&term).unwrap().unwrap();
        assert_eq!(
            other_term.format_selection(FormatOptions::default(), &matched),
            Err(VtError::InvalidValue)
        );
        let formatted = term
            .format_selection(
                FormatOptions {
                    kind: FormatKind::Plain,
                    unwrap: true,
                    trim: true,
                    extras: 0,
                },
                &matched,
            )
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&formatted), "ALPHA");
    }
    let span = search
        .selected_match_span(&term, GridPointTag::Active)
        .unwrap()
        .unwrap();
    assert_eq!(span.start, (0, 1));
    assert_eq!(span.end, (4, 1));

    // Mutating the terminal invalidates unsynced search match reads until re-fed.
    term.vt_write(b"!");
    assert_eq!(
        search.selected_match(&term).err(),
        Some(VtError::InvalidValue)
    );
    assert_eq!(search.matches(&term).err(), Some(VtError::InvalidValue));
    search.run(&term).unwrap();
    assert_eq!(search.matches(&term).unwrap().len(), 2);

    // Regression: active-row edit after caching viewport matches must not leave
    // stale viewport matches after `select_next`, and scrollback navigation must
    // preserve selection while refreshing viewport matches.
    let mut nav_term = Terminal::new(&TerminalConfig {
        cols: 20,
        rows: 3,
        ..TerminalConfig::default()
    })
    .unwrap();
    nav_term.vt_write(b"cat");
    let mut nav_search = Search::new(&nav_term).unwrap();
    nav_search.set_needle(&nav_term, Some("cat")).unwrap();
    nav_search.run(&nav_term).unwrap();
    assert_eq!(nav_search.viewport_matches(&nav_term).unwrap().len(), 1);
    nav_term.vt_write(b"\r\x1b[2Kdog");
    assert!(!nav_search.select_next(&mut nav_term).unwrap());
    assert_eq!(nav_search.total_matches(), 0);
    assert!(nav_search.viewport_matches(&nav_term).unwrap().is_empty());

    nav_term.vt_write(b"\r\nneedle top\r\nline2\r\nline3\r\nline4\r\nneedle bottom");
    nav_search
        .set_select_scroll(SearchScroll::IfNeeded)
        .unwrap();
    nav_search.set_needle(&nav_term, Some("needle")).unwrap();
    nav_search.run(&nav_term).unwrap();
    assert_eq!(nav_search.total_matches(), 2);
    assert!(nav_search.select_next(&mut nav_term).unwrap());
    assert_eq!(nav_search.selected_index(), Some(0));
    assert!(nav_search.select_next(&mut nav_term).unwrap());
    assert_eq!(nav_search.selected_index(), Some(1));
    let scrolled_vp = nav_search
        .viewport_matches_spans(&nav_term, GridPointTag::Viewport)
        .unwrap();
    assert!(
        !scrolled_vp.is_empty(),
        "scrolled viewport should report the newly visible match"
    );

    // Selection mutation and tracked refs enforce owner and active-screen checks.
    assert!(term
        .select_and_set_word(GridPointTag::Active, 6, 0, None)
        .unwrap());
    let copied = term
        .format_active_selection(FormatOptions {
            kind: FormatKind::Plain,
            unwrap: true,
            trim: true,
            extras: 0,
        })
        .unwrap()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&copied), "beta");
    let active_span = term.active_selection_span(GridPointTag::Active).unwrap();
    assert_eq!(active_span.start, (6, 0));
    assert_eq!(active_span.end, (9, 0));

    let tracked = term.track_grid_ref(GridPointTag::Active, 6, 0).unwrap();
    assert_eq!(
        tracked.snapshot(&other_term).err(),
        Some(VtError::InvalidValue)
    );
    {
        let snap_ref = tracked.snapshot(&term).unwrap().unwrap();
        assert_eq!(snap_ref.cell().unwrap().codepoint, 'b' as u32);
    }
    // Switching to alternate screen hides primary-screen tracked snapshots so
    // inactive-screen refs are never handed to active-screen selection APIs.
    term.vt_write(b"\x1b[?1049h");
    assert!(tracked.snapshot(&term).unwrap().is_none());
    term.vt_write(b"\x1b[?1049l");
    {
        let snap_ref = tracked.snapshot(&term).unwrap().unwrap();
        assert_eq!(snap_ref.cell().unwrap().codepoint, 'b' as u32);
    }

    assert_eq!(
        encode_color_scheme_report(ColorScheme::Dark).unwrap(),
        b"\x1b[?997;1n"
    );
    assert_eq!(
        encode_color_scheme_report(ColorScheme::Light).unwrap(),
        b"\x1b[?997;2n"
    );
    assert_eq!(
        encode_size_report(SizeReportStyle::Csi18T, 24, 80, 10, 20).unwrap(),
        b"\x1b[8;24;80t"
    );
    assert_eq!(encode_focus(true).unwrap(), b"\x1b[I");
    assert_eq!(encode_focus(false).unwrap(), b"\x1b[O");
    assert_eq!(encode_paste("hi", true).unwrap(), b"\x1b[200~hi\x1b[201~");
    assert!(paste_is_safe("hello"));
    assert!(!paste_is_safe("hello\nworld"));

    let mut key_enc = KeyEncoder::new().unwrap();
    key_enc.sync_from_terminal(&term).unwrap();
    // Owned UTF-8 text is cleared from the event before encode() returns.
    {
        let dynamic = String::from("x");
        let encoded_char = key_enc
            .encode(
                sys::GHOSTTY_KEY_ACTION_PRESS,
                sys::GHOSTTY_KEY_X,
                0,
                0,
                &dynamic,
                'x' as u32,
            )
            .unwrap();
        assert_eq!(encoded_char, b"x");
    }
    let encoded = key_enc
        .encode(
            sys::GHOSTTY_KEY_ACTION_PRESS,
            sys::GHOSTTY_KEY_ARROW_UP,
            0,
            0,
            "",
            0,
        )
        .unwrap();
    assert_eq!(encoded, b"\x1b[A");

    let mut mouse_enc = MouseEncoder::new().unwrap();
    mouse_enc
        .set_tracking(sys::GHOSTTY_MOUSE_TRACKING_NORMAL)
        .unwrap();
    mouse_enc.set_format(sys::GHOSTTY_MOUSE_FORMAT_SGR).unwrap();
    mouse_enc.set_size(200, 100, 10, 20).unwrap();
    let m = mouse_enc
        .encode(
            sys::GHOSTTY_MOUSE_ACTION_PRESS,
            Some(sys::GHOSTTY_MOUSE_BUTTON_LEFT),
            0,
            15.0,
            25.0,
        )
        .unwrap();
    assert_eq!(m, b"\x1b[<0;2;2M");
}

/// Phase 1a Open Question 2 parity spike: verify `libghostty-vt`'s native
/// `GHOSTTY_TERMINAL_OPT_PROGRAM_STATUS` parser and query reply against every
/// rule in `docs/features/archive/861-program-status.md`.
#[test]
fn osc_7501_parity_spike_against_861_rules() {
    fn feed_7501(bytes: &[u8]) -> (Vec<Vec<u8>>, Vec<RawProgramStatus>) {
        let mut term = Terminal::new(&TerminalConfig::default()).unwrap();
        term.vt_write(bytes);
        let mut replies = Vec::new();
        let mut reports = Vec::new();
        for effect in term.take_effects() {
            match effect {
                TerminalEffect::WritePty(b) => replies.push(b),
                TerminalEffect::ProgramStatus(r) => reports.push(r),
                _ => {}
            }
        }
        (replies, reports)
    }

    // 1. Probe query replies (`OSC 7501 ; ?`):
    // Ghostty replies with capability lists and echoes the query's terminator.
    let (st_replies, _) = feed_7501(b"\x1b]7501;?\x1b\\");
    assert_eq!(
        st_replies,
        vec![b"\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x1b\\".to_vec()]
    );
    let (bel_replies, _) = feed_7501(b"\x1b]7501;?\x07");
    assert_eq!(
        bel_replies,
        vec![b"\x1b]7501;?:states=idle,working,done,blocked,error:kinds=permission,question,auth\x07".to_vec()]
    );

    // 2. Grammar: bad pairs & unknown keys skipped, last value wins, whitespace trimmed.
    let (_, reports) = feed_7501(
        b"\x1b]7501;bad:=x:bad-key=x:nope=$:future=yes: state = working :state=blocked:kind=future:progress=101:app=bad/name:msg=SGk=\x1b\\",
    );
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].state, ProgramStatusState::Blocked);
    assert_eq!(reports[0].kind, None);
    assert_eq!(reports[0].progress, None);
    assert_eq!(reports[0].app, None);
    assert_eq!(reports[0].message.as_deref(), Some("Hi"));

    // 3. Invalid payloads are rejected as a whole.
    for payload in [
        "".to_owned(),
        "app=test".to_owned(),
        "state=future".to_owned(),
        "state=working:state=future".to_owned(),
        "state=working:id=".to_owned(),
        "state=working:id=a!".to_owned(),
        "state=working:id=/a".to_owned(),
        "state=working:id=a//b".to_owned(),
        format!("state=working:{}=x", "a".repeat(17)),
        format!("state=working:app={}", "a".repeat(33)),
        format!("state=working:id={}", "a".repeat(33)),
        format!("state=working:id={}", ["a"; 9].join("/")),
        format!(
            "state=working:id={}",
            ["abcdefghijklmnopqrstuvwxyz"; 5].join("/")
        ),
        format!("state=working:id={}", vec!["a".repeat(32); 4].join("/")) + "aa",
        "state=working:msg=YQ==:msg=a".to_owned(),
        format!("state=idle:msg={}", STANDARD.encode("a".repeat(2049))),
        format!("state=idle:title={}", STANDARD.encode("a".repeat(193))),
        format!("state=idle:msg={}", STANDARD.encode("\n")),
        format!("state=idle:msg={}", STANDARD.encode("\t")),
        format!("state=idle:msg={}", STANDARD.encode("\x1b")),
        format!("state=idle:msg={}", STANDARD.encode("\u{7f}")),
        format!("state=idle:msg={}", STANDARD.encode("\u{80}")),
        format!("state=idle:msg={}", STANDARD.encode("\u{9f}")),
        "state=idle:msg=_".to_owned(),
        "state=idle:msg=/w==".to_owned(),
    ] {
        let seq = format!("\x1b]7501;{payload}\x1b\\");
        let (_, reports) = feed_7501(seq.as_bytes());
        assert!(
            reports.is_empty(),
            "libghostty-vt unexpectedly accepted invalid OSC 7501 payload: {payload:?}"
        );
    }

    // 4. Exact boundary lengths (msg=2048, title=192, sequence=4096 with ST).
    for (key, limit) in [("msg", 2048usize), ("title", 192usize)] {
        let seq = format!(
            "\x1b]7501;state=working:{key}={}\x1b\\",
            STANDARD.encode("a".repeat(limit))
        );
        let (_, reports) = feed_7501(seq.as_bytes());
        assert_eq!(reports.len(), 1, "{key} at exact limit {limit}");
    }
    let prefix = "\x1b]7501;state=idle:unknown=";
    let st_4096 = format!("{prefix}{}\x1b\\", "a".repeat(4096 - prefix.len() - 2));
    assert_eq!(st_4096.len(), 4096);
    assert_eq!(feed_7501(st_4096.as_bytes()).1.len(), 1);
    let st_4097 = st_4096.replacen("unknown=", "unknown=a", 1);
    assert_eq!(st_4097.len(), 4097);
    assert!(feed_7501(st_4097.as_bytes()).1.is_empty());

    // 5. VT100 control/nesting behavior (Open Question 2 findings 3 & 4):
    assert!(feed_7501(b"\x1b]7501;state=blocked\x18\x07").1.is_empty());
    assert!(feed_7501(b"\x1b]7501;state=blocked\x1a\x1b\\").1.is_empty());
    assert_eq!(feed_7501(b"\x1b]0;\x1b]7501;state=blocked\x07").1.len(), 1);
    assert_eq!(feed_7501(b"\x1b]7501;state=blo\tcked\x07").1.len(), 1);
    assert_eq!(feed_7501(b"\x1b]7501;state=blocked\x1b\x1b\\").1.len(), 1);
    assert_eq!(feed_7501(b"\x1b]7501;state=blocked\x1bX").1.len(), 1);
    assert_eq!(
        feed_7501(b"\x1bP\x1b]7501;state=blocked\x07\x1b\\").1.len(),
        1
    );

    // 6. BEL-terminated 4096-byte sequence (Open Question 2 finding 2):
    let bel_4095 = format!("{prefix}{}\x07", "a".repeat(4095 - prefix.len() - 1));
    assert_eq!(bel_4095.len(), 4095);
    assert_eq!(feed_7501(bel_4095.as_bytes()).1.len(), 1);
    let bel_4096 = format!("{prefix}{}\x07", "a".repeat(4096 - prefix.len() - 1));
    assert_eq!(bel_4096.len(), 4096);
    assert!(feed_7501(bel_4096.as_bytes()).1.is_empty());

    // 7. Duplicate-key order & explicit empty text (Open Question 2 findings 5 & 6):
    // - Ghostty validates only the winning `msg` (last value wins), so an invalid
    //   earlier `msg=a` followed by valid `msg=SGk=` emits a report, whereas
    //   Runner's byte scanner rejects the whole report.
    // - Ghostty's `Report.readOption` returns `null` for `v.len == 0` on `title`
    //   and `msg`, collapsing explicit `title=` / `msg=` to absence (`None`),
    //   whereas Runner preserves `Some("")`.
    let (_, dup_reports) = feed_7501(b"\x1b]7501;state=working:msg=a:msg=SGk=\x1b\\");
    assert_eq!(dup_reports.len(), 1);
    assert_eq!(dup_reports[0].message.as_deref(), Some("Hi"));

    let (_, empty_text_reports) = feed_7501(b"\x1b]7501;state=working:title=:msg=\x1b\\");
    assert_eq!(empty_text_reports.len(), 1);
    assert_eq!(empty_text_reports[0].title, None);
    assert_eq!(empty_text_reports[0].message, None);
}

#[test]
fn scrollback_zero_disables_history_and_positive_retains_multi_page_output() {
    // 1. Scrollback 0 retains zero rows initially, erases retained history on
    //    live downgrade, and resumes retention when raised again.
    let mut zero = Terminal::new(&TerminalConfig {
        cols: 80,
        rows: 5,
        scrollback_lines: 0,
        ..TerminalConfig::default()
    })
    .unwrap();
    for i in 0..50 {
        zero.vt_write(format!("zero-{i:04}\r\n").as_bytes());
    }
    assert_eq!(zero.scrollback_rows(), 0);

    zero.set_scrollback_lines(500).unwrap();
    for i in 0..100 {
        zero.vt_write(format!("restored-{i:04}\r\n").as_bytes());
    }
    assert!(
        zero.scrollback_rows() >= 90,
        "raising scrollback above 0 should retain new history, got {}",
        zero.scrollback_rows()
    );

    zero.set_scrollback_lines(0).unwrap();
    assert_eq!(
        zero.scrollback_rows(),
        0,
        "lowering scrollback to 0 must erase retained history immediately"
    );
    for i in 0..30 {
        zero.vt_write(format!("after-zero-{i:04}\r\n").as_bytes());
    }
    assert_eq!(zero.scrollback_rows(), 0);

    // 2. Positive limits lift the default 10,000-byte cap (`SIZE_MAX`) so long
    //    output retains about the configured line count across multiple pages.
    let mut multi = Terminal::new(&TerminalConfig {
        cols: 80,
        rows: 24,
        scrollback_lines: 1_500,
        ..TerminalConfig::default()
    })
    .unwrap();
    for i in 0..4_000 {
        multi.vt_write(format!("line-{i:04} payload\r\n").as_bytes());
    }
    let retained = multi.scrollback_rows();
    assert!(
        (1_000..=2_200).contains(&retained),
        "expected whole-page multi-page retention around 1,500 lines, got {retained}"
    );
    let plain = multi
        .format(FormatOptions {
            kind: FormatKind::Plain,
            unwrap: false,
            trim: true,
            extras: sys::GHOSTTY_FORMATTER_EXTRAS_NONE,
        })
        .unwrap();
    let text = String::from_utf8_lossy(&plain);
    assert!(
        !text.contains("line-0000"),
        "oldest lines must be evicted past scrollback limit"
    );
    assert!(
        text.contains("line-3999"),
        "newest lines must remain on the active screen"
    );

    // Lowering to a smaller positive limit prunes eligible whole pages immediately.
    multi.set_scrollback_lines(500).unwrap();
    let pruned = multi.scrollback_rows();
    assert!(
        pruned < retained && pruned <= 1_100,
        "lowering scrollback to 500 should prune older pages (before={retained}, after={pruned})"
    );
}

#[test]
fn tab_selection_expands_to_spaces_and_preserves_soft_wraps() {
    let opts = FormatOptions {
        kind: FormatKind::Plain,
        unwrap: true,
        trim: true,
        extras: sys::GHOSTTY_FORMATTER_EXTRAS_NONE,
    };

    // 1. Plain row with a horizontal tab expands to spaces up to the 8-col tab stop.
    let mut term = Terminal::new(&TerminalConfig {
        cols: 20,
        rows: 4,
        scrollback_lines: 100,
        ..TerminalConfig::default()
    })
    .unwrap();
    term.vt_write(b"A\tB");
    term.set_selection_points(GridPointTag::Active, (0, 0), (8, 0), false)
        .unwrap();
    let selected = String::from_utf8(term.format_active_selection(opts).unwrap().unwrap()).unwrap();
    assert_eq!(selected, "A       B");
    assert!(!selected.contains('\t'));

    // 2. Selection spanning a soft wrap after a tab keeps spaces to the tab
    //    stop and emits no spurious newline across the wrap boundary.
    let mut wrapped = Terminal::new(&TerminalConfig {
        cols: 10,
        rows: 4,
        scrollback_lines: 100,
        ..TerminalConfig::default()
    })
    .unwrap();
    wrapped.vt_write(b"A\tBCDE");
    wrapped
        .set_selection_points(GridPointTag::Active, (0, 0), (1, 1), false)
        .unwrap();
    let selected =
        String::from_utf8(wrapped.format_active_selection(opts).unwrap().unwrap()).unwrap();
    assert_eq!(selected, "A       BCDE");
    assert!(!selected.contains('\n'));
    assert!(!selected.contains('\t'));
}
