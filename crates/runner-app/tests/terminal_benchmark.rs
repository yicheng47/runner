#![cfg(unix)]

use std::collections::HashMap;
use std::io::Write as _;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use runner_daemon::daemon::boot::{boot_core, NativePaths};
use runner_daemon::model::Role;
use runner_terminal::replay::visible_lines;
use runner_terminal::terminal::{TerminalBridge, TerminalMirror};

#[derive(Default)]
struct Wake {
    tick: Mutex<u64>,
    changed: Condvar,
}

impl Wake {
    fn notify(&self) {
        *self.tick.lock().unwrap() += 1;
        self.changed.notify_all();
    }

    fn until(&self, timeout: Duration, ready: impl Fn() -> bool) {
        let deadline = Instant::now() + timeout;
        loop {
            let tick = self.tick.lock().unwrap();
            let observed = *tick;
            drop(tick);
            if ready() {
                return;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            assert!(!remaining.is_zero(), "terminal benchmark timed out");
            let tick = self.tick.lock().unwrap();
            let (_tick, _) = self
                .changed
                .wait_timeout_while(tick, remaining, |tick| *tick == observed)
                .unwrap();
        }
    }
}

fn contains(terminal: &TerminalMirror, text: &str) -> bool {
    visible_lines(&*terminal.term.lock())
        .join("\n")
        .contains(text)
}

fn spawn_cat(
    core: &runner_daemon::AppCore,
    bridge: &TerminalBridge,
    wake: &Wake,
    script: &str,
) -> Arc<TerminalMirror> {
    let role = Role {
        id: "runtime:shell".into(),
        handle: format!("bench-{}", core.sessions.live_session_ids().len()),
        display_name: "Terminal benchmark".into(),
        runtime: "shell".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        working_dir: None,
        system_prompt: None,
        env: HashMap::new(),
        model: None,
        effort: None,
        codex_speed: None,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    let spawned = core
        .sessions
        .spawn_direct(
            &role,
            runner_daemon::session::DirectLaunch {
                cols: Some(80),
                rows: Some(24),
                runtime_direct: true,
                ..Default::default()
            },
            &core.app_data_dir,
            Arc::clone(&core.db),
            Arc::new(core.session_events()),
        )
        .unwrap();
    let terminal = bridge.attach(&spawned.id).unwrap();
    let view = terminal.view();
    wake.until(Duration::from_secs(10), || {
        contains(&terminal, "BENCH_READY")
    });
    drop(view);
    terminal
}

#[test]
#[ignore = "manual macOS/Windows terminal latency and throughput gate"]
fn terminal_benchmark() {
    let temp = tempfile::tempdir().unwrap();
    let core = boot_core(
        &NativePaths::new(temp.path().join("data"), temp.path().join("logs")),
        Vec::new(),
    )
    .unwrap();
    let wake = Arc::new(Wake::default());
    let notify = Arc::clone(&wake);
    let bridge = TerminalBridge::new(
        runner_daemon::daemon::InProcessTransport::client(core.clone()),
        Arc::new(move || notify.notify()),
    )
    .unwrap();
    let echo = spawn_cat(
        &core,
        &bridge,
        &wake,
        "stty raw -echo; printf 'BENCH_READY\r\n'; exec cat",
    );
    let echo_view = echo.view();
    let mut samples = Vec::with_capacity(10_000);
    for i in 0..10_000 {
        let character = char::from(b'A' + (i % 26) as u8);
        let before = echo.output_activity().last_seq;
        let start = Instant::now();
        echo.send_text(&character.to_string()).unwrap();
        wake.until(Duration::from_secs(5), || {
            echo.output_activity().last_seq > before
        });
        samples.push(start.elapsed().as_secs_f64() * 1_000.0);
        let term = echo.term.lock();
        let cursor = &term.grid().cursor;
        let mut point = cursor.point;
        if !cursor.input_needs_wrap {
            point.column.0 -= 1;
        }
        assert_eq!(term.grid()[point].c, character, "cat echo {i}");
    }
    samples.sort_by(f64::total_cmp);

    let mut burst_file = tempfile::NamedTempFile::new_in(temp.path()).unwrap();
    const BYTES: usize = 50 * 1024 * 1024;
    let mut line = vec![b'x'; 79];
    line.push(b'\n');
    let block = line.repeat(8192);
    for _ in 0..BYTES / block.len() {
        burst_file.write_all(&block).unwrap();
    }
    burst_file.write_all(&block[..BYTES % block.len()]).unwrap();
    burst_file.flush().unwrap();
    let path = burst_file.path().to_str().unwrap().replace('\'', "'\\''");
    let burst = spawn_cat(
        &core,
        &bridge,
        &wake,
        &format!(
            "stty raw -echo; printf 'BENCH_READY\r\n'; read start; cat '{path}'; printf '\r\nBENCH_DONE\r\n'; exec cat"
        ),
    );
    let burst_view = burst.view();
    let start = Instant::now();
    burst.write_user_bytes(b"\n").unwrap();
    wake.until(Duration::from_secs(180), || contains(&burst, "BENCH_DONE"));
    let elapsed = start.elapsed().as_secs_f64();
    println!(
        "terminal_benchmark platform={} samples={} echo_p50_ms={:.6} echo_p99_ms={:.6} burst_bytes={} burst_seconds={:.6} burst_mib_per_second={:.6}",
        std::env::consts::OS,
        samples.len(),
        samples[4999],
        samples[9899],
        BYTES,
        elapsed,
        50.0 / elapsed,
    );
    drop((echo_view, burst_view));
    core.sessions.kill(echo.session_id()).unwrap();
    core.sessions.kill(burst.session_id()).unwrap();
}

#[test]
#[ignore = "manual 30 fps Kitty graphics stream profile (#1489)"]
fn kitty_graphics_30fps_stream_profile() {
    use base64::Engine as _;
    use image::ImageEncoder as _;
    use runner_terminal::kitty::{StoredImage, PER_SCREEN_LIMIT};
    use runner_terminal::terminal::{LiveImageKey, MirrorGraphicsObserver};
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

    #[derive(Default)]
    struct BenchDecodeObserver {
        decoded_frames: AtomicU64,
        decode_ns_total: AtomicU64,
    }

    impl MirrorGraphicsObserver for BenchDecodeObserver {
        fn on_graphics_updated(
            &self,
            _mirror_id: u64,
            _sync_seq: u64,
            viewers: &Arc<AtomicUsize>,
            _live_keys: &[LiveImageKey],
            decode_candidates: &[StoredImage],
        ) {
            if viewers.load(Ordering::Acquire) == 0 {
                return;
            }
            for img in decode_candidates {
                let start = Instant::now();
                let bgra = img.decode_bgra(PER_SCREEN_LIMIT).unwrap();
                std::hint::black_box(&bgra);
                self.decode_ns_total
                    .fetch_add(start.elapsed().as_nanos() as u64, Ordering::Relaxed);
                self.decoded_frames.fetch_add(1, Ordering::Relaxed);
            }
        }
        fn on_last_viewer_dropped(&self, _mirror_id: u64, _viewers: &Arc<AtomicUsize>) {}
        fn on_mirror_dropped(&self, _mirror_id: u64) {}
    }

    let temp = tempfile::tempdir().unwrap();
    let core = boot_core(
        &NativePaths::new(temp.path().join("data"), temp.path().join("logs")),
        Vec::new(),
    )
    .unwrap();
    let client = runner_daemon::daemon::InProcessTransport::client(core.clone());

    let (w, h) = (1280u32, 720u32);
    let mut raw_rgba_a = vec![0u8; (w * h * 4) as usize];
    let mut raw_rgba_b = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            raw_rgba_a[i] = (x & 0xff) as u8;
            raw_rgba_a[i + 1] = (y & 0xff) as u8;
            raw_rgba_a[i + 2] = ((x ^ y) & 0xff) as u8;
            raw_rgba_a[i + 3] = 255;
            raw_rgba_b[i] = ((x + 17) & 0xff) as u8;
            raw_rgba_b[i + 1] = ((y + 31) & 0xff) as u8;
            raw_rgba_b[i + 2] = (x.wrapping_add(y) & 0xff) as u8;
            raw_rgba_b[i + 3] = 255;
        }
    }
    let encode_png = |rgba: &[u8]| {
        let mut out = Vec::new();
        let encoder = image::codecs::png::PngEncoder::new_with_quality(
            &mut out,
            image::codecs::png::CompressionType::Fast,
            image::codecs::png::FilterType::Adaptive,
        );
        encoder
            .write_image(rgba, w, h, image::ExtendedColorType::Rgba8)
            .unwrap();
        out
    };
    // Realistic UI/browser-like frame (large flat regions + text/bands) so PNG compresses realistically.
    let mut ui_rgba_a = vec![24u8; (w * h * 4) as usize];
    let mut ui_rgba_b = vec![24u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            let stripe = ((y / 32) + (x / 160)) % 4;
            ui_rgba_a[i] = (30 + stripe * 40) as u8;
            ui_rgba_a[i + 1] = (40 + (y / 8) % 120) as u8;
            ui_rgba_a[i + 2] = 90;
            ui_rgba_a[i + 3] = 255;
            ui_rgba_b[i] = (50 + stripe * 40) as u8;
            ui_rgba_b[i + 1] = (60 + (y / 8) % 120) as u8;
            ui_rgba_b[i + 2] = 110;
            ui_rgba_b[i + 3] = 255;
        }
    }
    let png_a = encode_png(&ui_rgba_a);
    let png_b = encode_png(&ui_rgba_b);

    let b64 = base64::engine::general_purpose::STANDARD;
    let raw_file_a = temp.path().join("frame_a.rgba");
    let raw_file_b = temp.path().join("frame_b.rgba");
    std::fs::write(&raw_file_a, &raw_rgba_a).unwrap();
    std::fs::write(&raw_file_b, &raw_rgba_b).unwrap();
    let raw_tf_cmd_a = format!(
        "\x1b_Ga=T,t=f,f=32,s={w},v={h},i=1,C=1,q=2;{}\x1b\\",
        b64.encode(raw_file_a.to_str().unwrap().as_bytes())
    )
    .into_bytes();
    let raw_tf_cmd_b = format!(
        "\x1b_Ga=T,t=f,f=32,s={w},v={h},i=1,C=1,q=2;{}\x1b\\",
        b64.encode(raw_file_b.to_str().unwrap().as_bytes())
    )
    .into_bytes();
    let raw_td_sample = format!(
        "\x1b_Ga=T,f=32,s={w},v={h},i=1,C=1,q=2;{}\x1b\\",
        b64.encode(&raw_rgba_a)
    )
    .into_bytes();
    let png_cmd_a =
        format!("\x1b_Ga=T,f=100,i=1,C=1,q=2;{}\x1b\\", b64.encode(&png_a)).into_bytes();
    let png_cmd_b =
        format!("\x1b_Ga=T,f=100,i=1,C=1,q=2;{}\x1b\\", b64.encode(&png_b)).into_bytes();

    // QA-scale 1269x1550 zlib-compressed direct RGB stream (matches emit.py stream).
    let (zw, zh) = (1269u32, 1550u32);
    let make_checker_rgb_zlib = |shift: usize| {
        use flate2::write::ZlibEncoder;
        use flate2::Compression as FlateLevel;
        let colors: [[u8; 3]; 6] = [
            [245, 61, 53],
            [23, 220, 116],
            [40, 99, 255],
            [249, 222, 65],
            [0, 0, 0],
            [255, 255, 255],
        ];
        let unit = (zw as usize) / (2 * colors.len());
        let rows: Vec<Vec<u8>> = (0..colors.len())
            .map(|phase| {
                let mut row = Vec::with_capacity((zw as usize) * 3);
                for x in 0..(zw as usize) {
                    row.extend_from_slice(&colors[(x / unit + phase + shift) % colors.len()]);
                }
                row
            })
            .collect();
        let mut raw = Vec::with_capacity((zw as usize) * (zh as usize) * 3);
        for y in 0..(zh as usize) {
            raw.extend_from_slice(&rows[(y / unit) % colors.len()]);
        }
        let mut enc = ZlibEncoder::new(Vec::new(), FlateLevel::fast());
        enc.write_all(&raw).unwrap();
        enc.finish().unwrap()
    };
    let zrgb_a = make_checker_rgb_zlib(0);
    let zrgb_b = make_checker_rgb_zlib(1);
    let zrgb_cmd_a = format!(
        "\x1b[H\x1b_Ga=T,C=1,q=2,i=77,f=24,s={zw},v={zh},o=z,z=-9;{}\x1b\\",
        b64.encode(&zrgb_a)
    )
    .into_bytes();
    let zrgb_cmd_b = format!(
        "\x1b[H\x1b_Ga=T,C=1,q=2,i=77,f=24,s={zw},v={zh},o=z,z=-9;{}\x1b\\",
        b64.encode(&zrgb_b)
    )
    .into_bytes();

    // Measure pure clone overhead of TerminalFrame::Output across 2 local subscribers.
    let shared_frame = runner_core::protocol::terminal::TerminalFrame::Output {
        seq: 1,
        bytes: raw_td_sample.clone().into(),
    };
    let clone_start = Instant::now();
    for _ in 0..90 {
        let c1 = shared_frame.clone();
        let c2 = shared_frame.clone();
        std::hint::black_box((c1, c2));
    }
    let clone_raw_ms = clone_start.elapsed().as_secs_f64() * 1000.0 / 90.0;

    let run_pass = |label: &str, cmd_a: &[u8], cmd_b: &[u8], frames: usize| {
        let fa = temp.path().join(format!("{label}_a.bin"));
        let fb = temp.path().join(format!("{label}_b.bin"));
        std::fs::write(&fa, cmd_a).unwrap();
        std::fs::write(&fb, cmd_b).unwrap();
        let py = format!(
            "import sys, os; \
             a = open({:?}, 'rb').read(); \
             b = open({:?}, 'rb').read(); \
             os.write(1, b'BENCH_READY\\r\\n'); \
             [os.write(1, a if i % 2 == 0 else b) for i in range({frames}) if sys.stdin.buffer.read(1)]; \
             os.write(1, b'BENCH_DONE\\r\\n'); \
             sys.stdin.buffer.read()",
            fa.to_str().unwrap(),
            fb.to_str().unwrap(),
        );
        let role = Role {
            id: "runtime:shell".into(),
            handle: format!("kitty-profile-{label}"),
            display_name: "Kitty 30fps profile".into(),
            runtime: "shell".into(),
            command: "/usr/bin/python3".into(),
            args: vec!["-u".into(), "-c".into(), py],
            working_dir: None,
            system_prompt: None,
            env: HashMap::new(),
            model: None,
            effort: None,
            codex_speed: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let spawned = core
            .sessions
            .spawn_direct(
                &role,
                runner_daemon::session::DirectLaunch {
                    cols: Some(160),
                    rows: Some(48),
                    runtime_direct: true,
                    ..Default::default()
                },
                &core.app_data_dir,
                Arc::clone(&core.db),
                Arc::new(core.session_events()),
            )
            .unwrap();
        let wake1 = Arc::new(Wake::default());
        let wake2 = Arc::new(Wake::default());
        let obs1 = Arc::new(BenchDecodeObserver::default());
        let obs2 = Arc::new(BenchDecodeObserver::default());
        let n1 = Arc::clone(&wake1);
        let m1 = TerminalMirror::attach(
            client.clone(),
            spawned.id.clone(),
            Arc::new(move || n1.notify()),
        )
        .unwrap();
        let n2 = Arc::clone(&wake2);
        let m2 = TerminalMirror::attach(
            client.clone(),
            spawned.id.clone(),
            Arc::new(move || n2.notify()),
        )
        .unwrap();
        m1.set_graphics_observer(obs1.clone());
        m2.set_graphics_observer(obs2.clone());
        let v1 = m1.view();
        let v2 = m2.view();
        wake1.until(Duration::from_secs(10), || contains(&m1, "BENCH_READY"));
        wake2.until(Duration::from_secs(10), || contains(&m2, "BENCH_READY"));

        let mut latencies_ms = Vec::with_capacity(frames);
        let start_all = Instant::now();
        for i in 0..frames {
            let expected_decoded = (i + 1) as u64;
            let frame_start = Instant::now();
            m1.write_user_bytes(b"\n").unwrap();
            wake1.until(Duration::from_secs(10), || {
                obs1.decoded_frames.load(Ordering::Relaxed) >= expected_decoded
            });
            wake2.until(Duration::from_secs(10), || {
                obs2.decoded_frames.load(Ordering::Relaxed) >= expected_decoded
            });
            latencies_ms.push(frame_start.elapsed().as_secs_f64() * 1000.0);
        }
        let elapsed = start_all.elapsed().as_secs_f64();
        latencies_ms.sort_by(f64::total_cmp);
        let p50 = latencies_ms[frames / 2];
        let p99 = latencies_ms[(frames * 99) / 100];
        let avg_decode_ms =
            (obs1.decode_ns_total.load(Ordering::Relaxed) as f64) / 1_000_000.0 / (frames as f64);
        println!(
            "kitty_profile mode={label} frames={frames} wire_kib={:.1} elapsed_s={:.3} fps={:.1} p50_ms={:.2} p99_ms={:.2} mirror_decode_avg_ms={:.2} clone_2x_raw_ms={:.2}",
            cmd_a.len() as f64 / 1024.0,
            elapsed,
            frames as f64 / elapsed,
            p50,
            p99,
            avg_decode_ms,
            clone_raw_ms,
        );
        drop((v1, v2));
        core.sessions.kill(&spawned.id).unwrap();
    };

    run_pass(
        "tf_raw_rgba_1280x720_2subs",
        &raw_tf_cmd_a,
        &raw_tf_cmd_b,
        90,
    );
    run_pass("td_png_1280x720_2subs", &png_cmd_a, &png_cmd_b, 90);
    run_pass("td_zlib_rgb_1269x1550_2subs", &zrgb_cmd_a, &zrgb_cmd_b, 90);
    core.sessions.drain_terminal_workers();
}
