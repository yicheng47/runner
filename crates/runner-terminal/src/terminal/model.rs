use super::*;

pub trait TerminalHost: Send + Sync + 'static {
    fn write_input(&self, session_id: &str, bytes: &[u8]) -> Result<()>;
    fn write_reply(&self, session_id: &str, bytes: &[u8]) -> Result<()>;
    fn report_input_state(&self, session_id: &str, observation: InputObservation);
    fn set_live_title(&self, session_id: &str, title: &str) -> Result<()>;
    fn resize(&self, session_id: &str, cols: u16, rows: u16) -> Result<()>;
    fn is_local_host(&self, host: &str) -> bool;
}

#[derive(Default)]
pub struct ModelOptions {
    pub agent: bool,
    pub shell: bool,
    pub initial_title: String,
    pub cwd: Option<String>,
}

pub struct TerminalModel {
    workers: Mutex<Vec<thread::JoinHandle<()>>>,
    pub term: Arc<FairMutex<Term<EventProxy>>>,
    host: Arc<dyn TerminalHost>,
    session_id: String,
    parser: Mutex<ParserState>,
    sync_flush: Sender<()>,
    sequence: Mutex<SequenceState>,
    size: Arc<Mutex<(u16, u16)>>,
    title: Arc<Mutex<String>>,
    palette: Arc<Mutex<PaletteState>>,
    scheme: Arc<SchemeState>,
    events: Sender<Event>,
    input_tracker: Mutex<InputTracker>,
    initial_observation: InputObservation,
    fixture_recorder: Option<FixtureRecorder>,
    cwd_reports: Option<Mutex<CwdReports>>,
    live_cwd: Mutex<Option<PathBuf>>,
}

impl TerminalModel {
    pub fn input_rows(&self) -> Vec<String> {
        let term = self.term.lock();
        (0..term.screen_lines())
            .map(|row| crate::replay::row_cells(&term, row as i32))
            .collect()
    }

    /// Serialize the revision and row checks through the write with native input observations.
    pub fn with_unchanged_input<T>(
        &self,
        revision: u64,
        probe: &crate::delivery_probe::InputProbe,
        write: impl FnOnce() -> T,
    ) -> Option<T> {
        let mut tracker = self.input_tracker.lock().unwrap();
        if tracker.reset_guard() != revision
            || probe.state(&self.input_rows()) != crate::delivery_probe::ProbeState::Present
        {
            return None;
        }
        {
            let term = self.term.lock_unfair();
            tracker.track_delivery_submit(Instant::now(), &term);
        }
        let result = write();
        drop(tracker);
        Some(result)
    }

    pub fn take_workers(&self) -> Vec<thread::JoinHandle<()>> {
        std::mem::take(&mut *self.workers.lock().unwrap())
    }
    pub fn new(
        host: Arc<dyn TerminalHost>,
        session_id: String,
        cols: u16,
        rows: u16,
        options: ModelOptions,
    ) -> Result<Arc<Self>> {
        let (events, rx) = mpsc::channel();
        let (sync_flush, sync_flush_requests) = mpsc::channel();
        let term = Arc::new(FairMutex::new(Term::new(
            term_config(),
            &TermSize::new(cols as usize, rows as usize),
            EventProxy {
                tx: events.clone(),
                waker: Arc::new(|| {}),
            },
        )));
        let size = Arc::new(Mutex::new((cols, rows)));
        let title = Arc::new(Mutex::new(options.initial_title));
        let scheme = Arc::new(SchemeState::default());
        let palette = Arc::new(Mutex::new(PaletteState::new(palette::RUNNER)));
        let now = Instant::now();
        let mut input_tracker = InputTracker::new(now);
        let initial_observation = input_tracker.initial_observation(now);
        let fixture_recorder = match FixtureRecorder::from_env(&session_id, cols, rows) {
            Ok(recorder) => recorder,
            Err(error) => {
                log::error!("start input fixture recorder for {session_id} failed: {error:#}");
                None
            }
        };
        let model = Arc::new(Self {
            workers: Mutex::default(),
            term: Arc::clone(&term),
            host: Arc::clone(&host),
            session_id: session_id.clone(),
            parser: Mutex::default(),
            sync_flush,
            sequence: Mutex::default(),
            size: Arc::clone(&size),
            title: Arc::clone(&title),
            palette: Arc::clone(&palette),
            scheme: Arc::clone(&scheme),
            events,
            input_tracker: Mutex::new(input_tracker),
            initial_observation,
            fixture_recorder,
            cwd_reports: options.shell.then(Mutex::default),
            live_cwd: Mutex::new(None),
        });
        let term_for_events = Arc::downgrade(&term);
        let worker = thread::Builder::new()
            .name(format!("native-term-events-{session_id}"))
            .spawn(move || {
                let write = |bytes: &[u8]| {
                    let _ = host.write_reply(&session_id, bytes);
                };
                while let Ok(event) = rx.recv() {
                    match event {
                        Event::PtyWrite(text) if text == SCHEME_PROBE_UNSUPPORTED => {
                            let state = if scheme.subscribed.load(Ordering::Relaxed) {
                                1
                            } else {
                                2
                            };
                            write(format!("\x1b[?2031;{state}$y").as_bytes());
                        }
                        Event::PtyWrite(text) => match kitty_flags_reply(&text) {
                            Some(flags) => write(
                                format!(
                                    "\x1b[?{}u",
                                    (flags & KeyboardModes::DISAMBIGUATE_ESC_CODES).bits()
                                )
                                .as_bytes(),
                            ),
                            None => write(text.as_bytes()),
                        },
                        Event::ColorRequest(index, format) => {
                            let palette = *palette.lock().unwrap();
                            let rgb = term_for_events
                                .upgrade()
                                .map(|term| {
                                    query_color_for(
                                        &*term.lock_unfair(),
                                        index,
                                        &palette.base,
                                        palette.theme,
                                    )
                                })
                                .unwrap_or_else(|| {
                                    crate::palette::resolve_index_for(
                                        index,
                                        &palette.base,
                                        palette.theme,
                                    )
                                });
                            write(format(rgb).as_bytes());
                        }
                        Event::TextAreaSizeRequest(format) => {
                            let (cols, rows) = *size.lock().unwrap();
                            write(
                                format(WindowSize {
                                    num_lines: rows,
                                    num_cols: cols,
                                    cell_width: 0,
                                    cell_height: 0,
                                })
                                .as_bytes(),
                            );
                        }
                        Event::Title(_) | Event::ResetTitle => {
                            let raw = match event {
                                Event::Title(title) => title,
                                _ => String::new(),
                            };
                            let cleaned = if options.agent {
                                let Some(title) =
                                    runner_core::protocol::session_title::provider_title(
                                        &raw,
                                        options.cwd.as_deref(),
                                    )
                                else {
                                    continue;
                                };
                                title
                            } else {
                                sanitize_title(&raw)
                            };
                            let changed = {
                                let mut held = title.lock().unwrap();
                                let changed = *held != cleaned;
                                if changed {
                                    *held = cleaned.clone();
                                }
                                changed
                            };
                            if changed {
                                if let Err(error) = host.set_live_title(&session_id, &cleaned) {
                                    log::warn!("persist terminal title for {session_id}: {error}");
                                }
                            }
                        }
                        _ => {}
                    }
                }
            })
            .context("spawn terminal event thread")?;
        model.workers.lock().unwrap().push(worker);
        let weak = Arc::downgrade(&model);
        let worker = thread::Builder::new()
            .name(format!("native-term-sync-{}", model.session_id))
            .spawn(move || {
                while sync_flush_requests.recv().is_ok() {
                    loop {
                        let Some(next) = weak.upgrade().map(|model| model.flush_sync_update())
                        else {
                            return;
                        };
                        let Some(deadline) = next else {
                            break;
                        };
                        if let Err(RecvTimeoutError::Disconnected) = sync_flush_requests
                            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                        {
                            return;
                        }
                    }
                }
            })
            .context("spawn terminal sync flush thread")?;
        model.workers.lock().unwrap().push(worker);
        Ok(model)
    }

    pub fn feed_output(&self, seq: u64, bytes: &[u8]) -> Option<InputObservation> {
        let now = Instant::now();
        let mut sequence = self.sequence.lock().unwrap();
        if seq <= sequence.last {
            return None;
        }
        if let Some(recorder) = &self.fixture_recorder {
            recorder.record_output(bytes);
        }
        let scheme_sequences = scan_scheme_sequences(&mut self.scheme.tail.lock().unwrap(), bytes);
        if let Some(reports) = &self.cwd_reports {
            if let Some(cwd) = reports
                .lock()
                .unwrap()
                .scan_with_host(bytes, |host| self.host.is_local_host(host))
            {
                *self.live_cwd.lock().unwrap() = Some(cwd);
            }
        }
        sequence.last = seq;
        let mut parser = self.parser.lock().unwrap();
        let mut tracker = self.input_tracker.lock().unwrap();
        let mut term = self.term.lock();
        let previous_mode = *term.mode();
        let mut fed = 0;
        for (end, sequence) in scheme_sequences {
            parse(&mut parser, &mut term, &bytes[fed..end]);
            fed = end;
            match sequence {
                SchemeSequence::Subscribe => self.scheme.subscribed.store(true, Ordering::Relaxed),
                SchemeSequence::Unsubscribe => {
                    self.scheme.subscribed.store(false, Ordering::Relaxed)
                }
                SchemeSequence::Query => {
                    let _ = self.events.send(Event::PtyWrite(scheme_report(
                        self.palette.lock().unwrap().theme,
                    )));
                }
            }
        }
        parse(&mut parser, &mut term, &bytes[fed..]);
        let observation = observe_parsed(
            &mut sequence,
            &mut tracker,
            &mut term,
            previous_mode,
            seq,
            now,
        );
        let schedule = parser.processor.sync_timeout().sync_timeout().is_some()
            && !std::mem::replace(&mut parser.flush_scheduled, true);
        drop(term);
        drop(tracker);
        drop(parser);
        drop(sequence);
        if schedule {
            let _ = self.sync_flush.send(());
        }
        observation
    }

    pub fn publish_initial(&self) {
        self.publish_input(Some(self.initial_observation));
    }

    pub fn publish_input(&self, observation: Option<InputObservation>) {
        if let Some(observation) = observation {
            self.host.report_input_state(&self.session_id, observation);
        }
    }

    fn flush_sync_update(&self) -> Option<Instant> {
        let mut sequence = self.sequence.lock().unwrap();
        let mut parser = self.parser.lock().unwrap();
        let now = Instant::now();
        match parser.processor.sync_timeout().sync_timeout() {
            Some(deadline) if deadline > now => return Some(deadline),
            Some(_) => {}
            None => {
                parser.flush_scheduled = false;
                return None;
            }
        }
        let mut tracker = self.input_tracker.lock().unwrap();
        let mut term = self.term.lock();
        let previous_mode = *term.mode();
        let held = parser.processor.sync_bytes_count();
        parser.processor.stop_sync(&mut *term);
        parser.boundary.end_sync();
        parser.flush_scheduled = false;
        let seq = sequence.last;
        let observation = observe_parsed(
            &mut sequence,
            &mut tracker,
            &mut term,
            previous_mode,
            seq,
            now,
        );
        drop(term);
        drop(tracker);
        drop(parser);
        drop(sequence);
        log::info!("terminal {}: synchronized update timed out without its end marker; flushed {held} held bytes", self.session_id);
        self.publish_input(observation);
        None
    }

    pub fn snapshot(&self, seq: u64) -> TerminalSnapshot {
        self.snapshot_with_history(seq, usize::MAX)
    }

    pub fn snapshot_with_history(&self, seq: u64, history: usize) -> TerminalSnapshot {
        let parser = self.parser.lock().unwrap();
        let term = self.term.lock();
        let unfinished = parser.boundary.unfinished();
        TerminalSnapshot {
            seq,
            cols: term.columns() as u16,
            rows: term.screen_lines() as u16,
            bytes: crate::snapshot::serialize_with_history(&*term, unfinished, history),
            unfinished_len: unfinished.len(),
            preceding_char: parser.processor.preceding_char(),
            controller: Default::default(),
        }
    }

    pub fn write_input(&self, bytes: &[u8]) -> Result<()> {
        self.host.write_input(&self.session_id, bytes)
    }
    pub fn size(&self) -> (u16, u16) {
        *self.size.lock().unwrap()
    }
    pub fn configure_named(&self, scrollback: usize, shape: &str) {
        let shape = match shape {
            "underline" => CursorShape::Underline,
            "beam" => CursorShape::Beam,
            "hollow" => CursorShape::HollowBlock,
            "hidden" => CursorShape::Hidden,
            _ => CursorShape::Block,
        };
        self.configure(scrollback, shape);
    }
    pub fn title(&self) -> String {
        self.title.lock().unwrap().clone()
    }
    pub fn live_cwd(&self) -> Option<PathBuf> {
        self.live_cwd.lock().unwrap().clone()
    }
    pub fn set_palette(&self, palette: palette::TerminalPalette) {
        let mut current = self.palette.lock().unwrap();
        if current.theme == palette {
            return;
        }
        let flipped = current.theme.is_light() != palette.is_light();
        *current = PaletteState::new(palette);
        drop(current);
        if flipped && self.scheme.subscribed.load(Ordering::Relaxed) {
            let _ = self.events.send(Event::PtyWrite(scheme_report(palette)));
        }
    }
    pub fn configure(&self, scrollback: usize, cursor_shape: CursorShape) {
        self.term.lock().set_options(Config {
            scrolling_history: scrollback,
            semantic_escape_chars: XTERM_WORD_SEPARATORS.to_owned(),
            default_cursor_style: CursorStyle {
                shape: cursor_shape,
                blinking: true,
            },
            ..term_config()
        });
    }
    pub fn observe_input(&self, input: &InputEvent) {
        if let Some(recorder) = &self.fixture_recorder {
            recorder.record_input(input);
        }
        let observation = {
            let mut tracker = self.input_tracker.lock().unwrap();
            let term = self.term.lock_unfair();
            tracker.observe_input(input, Instant::now(), &term)
        };
        self.publish_input(observation);
    }
    /// Observe the injected composer's echo without advancing the person's input revision.
    pub fn track_delivery_input(&self, text: &str) {
        let mut tracker = self.input_tracker.lock().unwrap();
        let term = self.term.lock_unfair();
        tracker.track_delivery_input(text, Instant::now(), &term);
    }
    pub fn track_delivery_submit(&self) {
        let mut tracker = self.input_tracker.lock().unwrap();
        let term = self.term.lock_unfair();
        tracker.track_delivery_submit(Instant::now(), &term);
    }
    pub fn input_reset_guard(&self) -> u64 {
        self.input_tracker.lock().unwrap().reset_guard()
    }
    pub fn reset_input_state(&self, guard: u64) {
        let observation = self
            .input_tracker
            .lock()
            .unwrap()
            .reset_if_unchanged(guard, Instant::now());
        self.publish_input(observation);
    }
    pub fn resize(&self, cols: u16, rows: u16) {
        let _parser = self.parser.lock().unwrap();
        let mut term = self.term.lock();
        let mut size = self.size.lock().unwrap();
        if *size != (cols, rows) {
            if let Some(recorder) = &self.fixture_recorder {
                recorder.record_resize(cols, rows);
            }
        }
        *size = (cols, rows);
        term.resize(TermSize::new(cols as usize, rows as usize));
    }
}

#[cfg(any(test, feature = "test-support"))]
impl TerminalModel {
    pub fn flush_events(&self) {
        let (tx, rx) = mpsc::channel();
        self.events
            .send(Event::TextAreaSizeRequest(Arc::new(move |_| {
                let _ = tx.send(());
                String::new()
            })))
            .unwrap();
        rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
    }
    pub fn is_shell(&self) -> bool {
        self.cwd_reports.is_some()
    }
}

#[cfg(test)]
mod delivery_tests {
    use super::*;
    use crate::delivery_probe::InputProbe;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    #[derive(Default)]
    struct Host {
        entered: AtomicBool,
    }
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
    }

    #[test]
    fn resubmit_serializes_composition_at_the_revision_check_and_write_boundary() {
        let host = Arc::new(Host::default());
        let model = TerminalModel::new(
            host.clone(),
            "delivery".into(),
            80,
            4,
            ModelOptions::default(),
        )
        .unwrap();
        model.feed_output(1, b"> ");
        let text = "[inbox] 1 new messages — run `runner msg read` to view.";
        let mut probe = InputProbe::new(model.input_rows(), text);
        model.feed_output(2, text.as_bytes());
        probe.locate(&model.input_rows());
        let revision = model.input_reset_guard();
        let (start, go) = mpsc::channel();
        let (ready, started) = mpsc::channel();
        let (done, finished) = mpsc::channel();
        let observing = model.clone();
        let observing_host = host.clone();
        let observer = thread::spawn(move || {
            go.recv_timeout(Duration::from_secs(2)).unwrap();
            ready.send(()).unwrap();
            observing.observe_input(&InputEvent::Composing { composing: true });
            assert!(observing_host.entered.load(Ordering::SeqCst));
            done.send(()).unwrap();
        });
        assert_eq!(
            model.with_unchanged_input(revision, &probe, || {
                assert!(model.input_tracker.try_lock().is_err());
                start.send(()).unwrap();
                started.recv_timeout(Duration::from_secs(2)).unwrap();
                assert!(finished.try_recv().is_err());
                host.entered.store(true, Ordering::SeqCst);
            }),
            Some(())
        );
        observer.join().unwrap();
        finished.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            model.with_unchanged_input(revision, &probe, || panic!(
                "composition must suppress Enter"
            )),
            None::<()>
        );
        let workers = model.take_workers();
        drop(model);
        for worker in workers {
            worker.join().unwrap();
        }
    }
}
