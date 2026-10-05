#![cfg(unix)]

use std::collections::HashMap;
use std::io::Write as _;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use runner_backend::daemon::boot::{boot_core, NativePaths};
use runner_backend::model::Role;
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
    core: &runner_backend::AppCore,
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
        .spawn_runtime_direct(
            &role,
            None,
            None,
            Some(80),
            Some(24),
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
        runner_backend::daemon::InProcessTransport::client(core.clone()),
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
