//! Live terminal session: PTY child (portable-pty) feeding an
//! `alacritty_terminal::Term`. One parser, one grid — the Term is
//! both model and render buffer (#307). GUI-framework-free; the GPUI
//! layer passes a waker closure and paints from `with_content`.

use std::io::{Read, Write};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;
use anyhow::Context as _;
use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use crate::palette;

pub struct EventProxy {
    tx: Sender<Event>,
    waker: Arc<dyn Fn() + Send + Sync>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        let _ = self.tx.send(event);
        (self.waker)();
    }
}

type SharedWriter = Arc<Mutex<Box<dyn Write + Send>>>;

/// Answer an OSC color query (`Event::ColorRequest`): prefer the
/// runtime override a program set via OSC 4/10/11/12 (stored in
/// `Term::colors()`), fall back to our static defaults.
pub fn query_color<T>(
    term: &Term<T>,
    index: usize,
    base: &[alacritty_terminal::vte::ansi::Rgb; 256],
) -> alacritty_terminal::vte::ansi::Rgb {
    let stored = if index < alacritty_terminal::term::color::COUNT {
        term.colors()[index]
    } else {
        None
    };
    stored.unwrap_or_else(|| palette::resolve_index(index, base))
}

pub struct TerminalSession {
    pub term: Arc<FairMutex<Term<EventProxy>>>,
    writer: SharedWriter,
    master: Mutex<Box<dyn MasterPty + Send>>,
    child: Mutex<Box<dyn Child + Send + Sync>>,
    size: Arc<Mutex<(u16, u16)>>,
    pub title: Arc<Mutex<String>>,
    pid: Option<u32>,
}

impl TerminalSession {
    /// Spawn `command` under a fresh PTY and start the reader/event
    /// threads. `waker` is called whenever the grid may have changed.
    pub fn spawn(
        command: &str,
        args: &[String],
        cols: u16,
        rows: u16,
        waker: Arc<dyn Fn() + Send + Sync>,
    ) -> anyhow::Result<Arc<Self>> {
        let pty = native_pty_system();
        let pair = pty
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("openpty")?;

        let mut cmd = CommandBuilder::new(command);
        cmd.args(args);
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLUMNS", cols.to_string());
        cmd.env("LINES", rows.to_string());
        if let Ok(cwd) = std::env::current_dir() {
            cmd.cwd(cwd);
        }

        let child = pair.slave.spawn_command(cmd).context("spawn_command")?;
        drop(pair.slave);

        let mut reader = pair.master.try_clone_reader().context("try_clone_reader")?;
        let writer: SharedWriter = Arc::new(Mutex::new(
            pair.master.take_writer().context("take_writer")?,
        ));

        let (tx, rx) = mpsc::channel::<Event>();
        let proxy = EventProxy {
            tx,
            waker: Arc::clone(&waker),
        };
        let term = Arc::new(FairMutex::new(Term::new(
            Config::default(),
            &TermSize::new(cols as usize, rows as usize),
            proxy,
        )));

        let pid = child.process_id();
        let size = Arc::new(Mutex::new((cols, rows)));
        let title = Arc::new(Mutex::new(String::new()));
        let session = Arc::new(Self {
            term: Arc::clone(&term),
            writer: Arc::clone(&writer),
            master: Mutex::new(pair.master),
            child: Mutex::new(child),
            size: Arc::clone(&size),
            title: Arc::clone(&title),
            pid,
        });

        // Reader: PTY bytes -> parser -> Term (locked per chunk).
        let term_for_reader = Arc::clone(&term);
        let waker_for_reader = Arc::clone(&waker);
        thread::Builder::new()
            .name("spike-pty-reader".into())
            .spawn(move || {
                let mut parser: Processor = Processor::new();
                let mut buf = [0u8; 65536];
                loop {
                    match reader.read(&mut buf) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            {
                                let mut term = term_for_reader.lock();
                                parser.advance(&mut *term, &buf[..n]);
                            }
                            waker_for_reader();
                        }
                    }
                }
                waker_for_reader();
            })
            .context("spawn reader thread")?;

        // Event pump: answer terminal queries, track title. Holds only
        // the shared pieces it needs — NOT the session — so the last
        // external Arc<TerminalSession> can actually drop. Shutdown
        // chain: session drop kills the child and closes the master →
        // the reader thread sees EOF and drops its Term handle → the
        // Term's EventProxy sender drops → rx disconnects → this
        // thread exits.
        let writer_for_events = Arc::clone(&writer);
        let size_for_events = Arc::clone(&size);
        let title_for_events = Arc::clone(&title);
        // Weak so this thread never keeps the Term (and its EventProxy
        // sender) alive — that would re-create the drop cycle.
        let term_for_events = Arc::downgrade(&term);
        thread::Builder::new()
            .name("spike-term-events".into())
            .spawn(move || {
                let base = palette::base_palette();
                let write = |bytes: &[u8]| {
                    if let Ok(mut writer) = writer_for_events.lock() {
                        let _ = writer.write_all(bytes);
                        let _ = writer.flush();
                    }
                };
                while let Ok(event) = rx.recv() {
                    match event {
                        Event::PtyWrite(s) => write(s.as_bytes()),
                        Event::ColorRequest(index, format) => {
                            let rgb = term_for_events
                                .upgrade()
                                .map(|term| query_color(&*term.lock_unfair(), index, &base))
                                .unwrap_or_else(|| palette::resolve_index(index, &base));
                            write(format(rgb).as_bytes());
                        }
                        Event::TextAreaSizeRequest(format) => {
                            let (cols, rows) = *size_for_events.lock().unwrap();
                            let reply = format(WindowSize {
                                num_lines: rows,
                                num_cols: cols,
                                cell_width: 0,
                                cell_height: 0,
                            });
                            write(reply.as_bytes());
                        }
                        Event::Title(title) => {
                            *title_for_events.lock().unwrap() = title;
                        }
                        Event::ResetTitle => {
                            title_for_events.lock().unwrap().clear();
                        }
                        // Wakeup/Bell/etc: the proxy already woke the UI.
                        _ => {}
                    }
                }
            })
            .context("spawn event thread")?;

        Ok(session)
    }

    pub fn write_bytes(&self, bytes: &[u8]) {
        if let Ok(mut writer) = self.writer.lock() {
            let _ = writer.write_all(bytes);
            let _ = writer.flush();
        }
    }

    /// Encode a key chord into PTY bytes, honoring APP_CURSOR mode.
    /// Returns false if the chord isn't one we encode.
    pub fn send_key(&self, key: &str, ctrl: bool, alt: bool, key_char: Option<&str>) -> bool {
        let app_cursor = self
            .term
            .lock_unfair()
            .mode()
            .contains(TermMode::APP_CURSOR);
        let mut bytes: Vec<u8> = Vec::new();
        if ctrl {
            let encoded = match key {
                k if k.len() == 1 && k.as_bytes()[0].is_ascii_alphabetic() => {
                    Some(k.as_bytes()[0].to_ascii_uppercase() - b'@')
                }
                "space" | "@" => Some(0),
                "[" => Some(0x1b),
                "\\" => Some(0x1c),
                "]" => Some(0x1d),
                _ => None,
            };
            match encoded {
                Some(b) => bytes.push(b),
                None => return false,
            }
        } else {
            let seq: &[u8] = match key {
                "enter" => b"\r",
                "backspace" => b"\x7f",
                "tab" => b"\t",
                "escape" => b"\x1b",
                "up" if app_cursor => b"\x1bOA",
                "down" if app_cursor => b"\x1bOB",
                "right" if app_cursor => b"\x1bOC",
                "left" if app_cursor => b"\x1bOD",
                "up" => b"\x1b[A",
                "down" => b"\x1b[B",
                "right" => b"\x1b[C",
                "left" => b"\x1b[D",
                "home" => b"\x1b[H",
                "end" => b"\x1b[F",
                "pageup" => b"\x1b[5~",
                "pagedown" => b"\x1b[6~",
                "delete" => b"\x1b[3~",
                "space" => b" ",
                _ => match key_char {
                    Some(text) if !text.is_empty() => text.as_bytes(),
                    _ => return false,
                },
            };
            bytes.extend_from_slice(seq);
        }
        if alt {
            bytes.insert(0, 0x1b);
        }
        self.write_bytes(&bytes);
        true
    }

    /// Paste text, bracketed when the app asked for it.
    pub fn paste(&self, text: &str) {
        let bracketed = self
            .term
            .lock_unfair()
            .mode()
            .contains(TermMode::BRACKETED_PASTE);
        if bracketed {
            let mut bytes = b"\x1b[200~".to_vec();
            bytes.extend_from_slice(text.replace('\x1b', "").as_bytes());
            bytes.extend_from_slice(b"\x1b[201~");
            self.write_bytes(&bytes);
        } else {
            self.write_bytes(text.replace('\x1b', "").as_bytes());
        }
    }

    pub fn resize(&self, cols: u16, rows: u16) {
        let cols = cols.max(2);
        let rows = rows.max(2);
        {
            let mut size = self.size.lock().unwrap();
            if *size == (cols, rows) {
                return;
            }
            *size = (cols, rows);
        }
        if let Ok(master) = self.master.lock() {
            let _ = master.resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
        self.term
            .lock()
            .resize(TermSize::new(cols as usize, rows as usize));
    }

    pub fn scroll(&self, delta_lines: i32) {
        self.term.lock().scroll_display(Scroll::Delta(delta_lines));
    }

    pub fn scroll_to_bottom(&self) {
        self.term.lock().scroll_display(Scroll::Bottom);
    }

    pub fn size(&self) -> (u16, u16) {
        *self.size.lock().unwrap()
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            // Reap so the pid doesn't linger as a zombie.
            let _ = child.wait();
        }
    }
}
