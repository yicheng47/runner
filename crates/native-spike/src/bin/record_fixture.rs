//! Record a real PTY session into the fixture corpus.
//!
//! Spawns `<command>` under a PTY, tees every output chunk (with
//! timing) into an NDJSON fixture, optionally sending scripted input
//! along the way:
//!
//! ```sh
//! cargo run -p native-spike --bin record-fixture -- \
//!   --out crates/native-spike/fixtures/claude-hello.ndjson \
//!   --cols 100 --rows 30 --duration-ms 30000 \
//!   --input '2000:hi, reply with one short line\r' \
//!   --input '20000:/exit\r' \
//!   -- claude
//! ```
//!
//! `--input` is `<delay_ms>:<text>` where text supports \r \n \t \e
//! and \\ escapes. Delays are measured from spawn, not from the
//! previous input.

use std::io::{Read, Write as _};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Context as _;
use clap::Parser;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};

use native_spike::fixtures::{encode_chunk, FixtureEvent, FixtureHeader};

#[derive(Parser)]
struct Args {
    /// Output fixture path (NDJSON).
    #[arg(long)]
    out: std::path::PathBuf,
    #[arg(long, default_value_t = 100)]
    cols: u16,
    #[arg(long, default_value_t = 30)]
    rows: u16,
    /// Stop recording after this long, even if the child is still alive.
    #[arg(long, default_value_t = 15_000)]
    duration_ms: u64,
    /// Scripted input: "<delay_ms>:<text>", repeatable.
    #[arg(long = "input")]
    inputs: Vec<String>,
    /// Free-form note stored in the fixture header.
    #[arg(long)]
    note: Option<String>,
    /// Command and args to run under the PTY.
    #[arg(trailing_var_arg = true, required = true)]
    command: Vec<String>,
}

struct ScriptedInput {
    at: Duration,
    bytes: Vec<u8>,
}

fn parse_input(spec: &str) -> anyhow::Result<ScriptedInput> {
    let (delay, text) = spec
        .split_once(':')
        .with_context(|| format!("--input {spec:?}: expected <delay_ms>:<text>"))?;
    let at = Duration::from_millis(delay.parse::<u64>().context("--input delay_ms")?);
    let mut bytes = Vec::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            let mut buf = [0u8; 4];
            bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            continue;
        }
        match chars.next() {
            Some('r') => bytes.push(b'\r'),
            Some('n') => bytes.push(b'\n'),
            Some('t') => bytes.push(b'\t'),
            Some('e') => bytes.push(0x1b),
            Some('\\') => bytes.push(b'\\'),
            other => anyhow::bail!("--input {spec:?}: unknown escape \\{other:?}"),
        }
    }
    Ok(ScriptedInput { at, bytes })
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let mut inputs = args
        .inputs
        .iter()
        .map(|s| parse_input(s))
        .collect::<anyhow::Result<Vec<_>>>()?;
    inputs.sort_by_key(|i| i.at);

    let (program, program_args) = args.command.split_first().context("empty command")?;

    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: args.rows,
            cols: args.cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("openpty")?;

    let mut cmd = CommandBuilder::new(program);
    cmd.args(program_args);
    cmd.env("TERM", "xterm-256color");
    cmd.env("COLUMNS", args.cols.to_string());
    cmd.env("LINES", args.rows.to_string());
    if let Ok(cwd) = std::env::current_dir() {
        cmd.cwd(cwd);
    }

    let mut child = pair.slave.spawn_command(cmd).context("spawn_command")?;
    drop(pair.slave);

    let mut reader = pair.master.try_clone_reader().context("try_clone_reader")?;
    let mut writer = pair.master.take_writer().context("take_writer")?;

    let start = Instant::now();
    let (tx, rx) = mpsc::channel::<(u64, Vec<u8>)>();
    let reader_thread = thread::spawn(move || {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let ms = start.elapsed().as_millis() as u64;
                    if tx.send((ms, buf[..n].to_vec())).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let mut events: Vec<FixtureEvent> = Vec::new();
    let deadline = start + Duration::from_millis(args.duration_ms);
    let mut pending = inputs.into_iter().peekable();
    let mut exit: Option<i32> = None;

    loop {
        let now = Instant::now();
        if now >= deadline {
            break;
        }
        while pending.peek().is_some_and(|i| start + i.at <= now) {
            let input = pending.next().unwrap();
            writer.write_all(&input.bytes).context("write input")?;
            writer.flush().ok();
            events.push(FixtureEvent::Input {
                ms: start.elapsed().as_millis() as u64,
                input: encode_chunk(&input.bytes),
            });
        }
        if exit.is_none() {
            if let Some(status) = child.try_wait().context("try_wait")? {
                exit = Some(status.exit_code() as i32);
            }
        }
        let wait = pending
            .peek()
            .map(|i| (start + i.at).min(deadline))
            .unwrap_or(deadline)
            .saturating_duration_since(now)
            .min(Duration::from_millis(50));
        match rx.recv_timeout(wait) {
            Ok((ms, chunk)) => {
                events.push(FixtureEvent::Data {
                    ms,
                    data: encode_chunk(&chunk),
                });
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // Child gone and nothing left to send: stop instead of
                // idling out the full duration window.
                if exit.is_some() && pending.peek().is_none() {
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    if exit.is_none() {
        child.kill().ok();
        if let Ok(Some(status)) = child.try_wait() {
            exit = Some(status.exit_code() as i32);
        }
    }
    drop(pair.master);
    // Drain whatever the reader already captured.
    while let Ok((ms, chunk)) = rx.recv_timeout(Duration::from_millis(200)) {
        events.push(FixtureEvent::Data {
            ms,
            data: encode_chunk(&chunk),
        });
    }
    reader_thread.join().ok();
    if let Some(code) = exit {
        events.push(FixtureEvent::Exit {
            ms: start.elapsed().as_millis() as u64,
            exit: code,
        });
    }

    let header = FixtureHeader {
        v: 1,
        cols: args.cols,
        rows: args.rows,
        command: program.clone(),
        args: program_args.to_vec(),
        note: args.note.clone(),
    };
    let mut out = serde_json::to_string(&header)?;
    out.push('\n');
    for ev in &events {
        out.push_str(&serde_json::to_string(ev)?);
        out.push('\n');
    }
    if let Some(parent) = args.out.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&args.out, out).with_context(|| format!("write {}", args.out.display()))?;

    let data_bytes: usize = events
        .iter()
        .filter_map(|e| match e {
            FixtureEvent::Data { data, .. } => Some(data.len() * 3 / 4),
            _ => None,
        })
        .sum();
    eprintln!(
        "recorded {} events (~{} KiB output) -> {}",
        events.len(),
        data_bytes / 1024,
        args.out.display()
    );
    Ok(())
}
