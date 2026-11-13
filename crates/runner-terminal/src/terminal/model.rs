use super::*;

pub trait TerminalHost: Send + Sync + 'static {
    fn write_input(&self, session_id: &str, bytes: &[u8]) -> Result<()>;
    fn write_reply(&self, session_id: &str, bytes: &[u8]) -> Result<()>;
    fn report_input_state(&self, session_id: &str, observation: InputObservation);
    fn set_live_title(&self, session_id: &str, title: &str) -> Result<()>;
    fn resize(&self, session_id: &str, cols: u16, rows: u16) -> Result<()>;
    fn is_local_host(&self, host: &str) -> bool;
    /// An OSC 52 write from the program in the pane; `text` is never empty and
    /// at most [`MAX_CLIPBOARD_BYTES`].
    fn clipboard_store(&self, session_id: &str, target: ClipboardTarget, text: &str);
}

pub const MAX_CLIPBOARD_BYTES: usize = 1 << 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardTarget {
    Clipboard,
    /// The primary selection on Linux; the clipboard elsewhere.
    Selection,
}
impl ClipboardTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clipboard => "clipboard",
            Self::Selection => "selection",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "clipboard" => Some(Self::Clipboard),
            "selection" => Some(Self::Selection),
            _ => None,
        }
    }
}

#[derive(Default)]
pub struct ModelOptions {
    pub agent: bool,
    pub shell: bool,
    pub initial_title: String,
    pub cwd: Option<String>,
    pub codex_native_attention: bool,
}

pub struct TerminalModel {
    workers: Mutex<Vec<thread::JoinHandle<()>>>,
    stopped: Arc<AtomicBool>,
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
    events: Sender<ProxyEvent>,
    input_tracker: Mutex<InputTracker>,
    publish_order: Mutex<()>,
    initial_observation: InputObservation,
    fixture_recorder: Option<FixtureRecorder>,
    cwd_reports: Option<Mutex<CwdReports>>,
    live_cwd: Mutex<Option<PathBuf>>,
    program_status: Mutex<crate::program_status::Parser>,
    pending_program_status: Mutex<Vec<(u64, Option<crate::program_status::Report>)>>,
    codex_native_attention: bool,
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
            || (probe.state(&self.input_rows()) != crate::delivery_probe::ProbeState::Present
                && !tracker.unsent_notice(&self.term.lock_unfair()))
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
    pub fn stop_workers(&self) -> Vec<thread::JoinHandle<()>> {
        self.stopped.store(true, Ordering::Release);
        let _ = self.events.send(ProxyEvent::Terminal(Event::Wakeup));
        let _ = self.sync_flush.send(());
        self.take_workers()
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
        let stopped = Arc::new(AtomicBool::new(false));
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
        let mut input_tracker = InputTracker::for_session(session_id.clone(), now);
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
            stopped: Arc::clone(&stopped),
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
            publish_order: Mutex::default(),
            initial_observation,
            fixture_recorder,
            cwd_reports: options.shell.then(Mutex::default),
            live_cwd: Mutex::new(None),
            program_status: Mutex::default(),
            pending_program_status: Mutex::default(),
            codex_native_attention: options.codex_native_attention,
        });
        let term_for_events = Arc::downgrade(&term);
        let events_stopped = Arc::clone(&stopped);
        let worker = thread::Builder::new()
            .name(format!("native-term-events-{session_id}"))
            .spawn(move || {
                let write = |bytes: &[u8]| {
                    let _ = host.write_reply(&session_id, bytes);
                };
                while let Ok(event) = rx.recv() {
                    if events_stopped.load(Ordering::Acquire) {
                        break;
                    }
                    let event = match event {
                        ProxyEvent::SyncSessionTitle {
                            initial_title,
                            apply,
                        } => {
                            *title.lock().unwrap() = initial_title;
                            apply();
                            continue;
                        }
                        ProxyEvent::ApplySessionTitle {
                            title: next_title,
                            persist,
                        } => {
                            let cache_changed = *title.lock().unwrap() != next_title;
                            if let Some(after_cache_update) = persist(&next_title, cache_changed) {
                                if cache_changed {
                                    *title.lock().unwrap() = next_title;
                                }
                                after_cache_update();
                            }
                            continue;
                        }
                        ProxyEvent::Terminal(event) => event,
                    };
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
                        Event::ClipboardStore(target, text)
                            if !text.is_empty() && text.len() <= MAX_CLIPBOARD_BYTES =>
                        {
                            let target = match target {
                                ClipboardType::Clipboard => ClipboardTarget::Clipboard,
                                ClipboardType::Selection => ClipboardTarget::Selection,
                            };
                            host.clipboard_store(&session_id, target, &text);
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
                        if stopped.load(Ordering::Acquire) {
                            return;
                        }
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
        if self.stopped.load(Ordering::Acquire) {
            return None;
        }
        let now = Instant::now();
        let mut sequence = self.sequence.lock().unwrap();
        if seq <= sequence.last {
            return None;
        }
        if let Some(recorder) = &self.fixture_recorder {
            recorder.record_output(bytes);
        }
        let mut queries = scan_scheme_sequences(&mut self.scheme.tail.lock().unwrap(), bytes)
            .into_iter()
            .map(|(end, sequence)| (end, Some(sequence)))
            .collect::<Vec<_>>();
        for (end, event) in self.program_status.lock().unwrap().scan(bytes) {
            match event {
                crate::program_status::Event::Query => queries.push((end, None)),
                crate::program_status::Event::Changed(report) => {
                    if !self.codex_native_attention {
                        self.pending_program_status
                            .lock()
                            .unwrap()
                            .push((seq, report));
                    }
                }
                crate::program_status::Event::Shell(_) => {}
            }
        }
        queries.sort_by_key(|(end, _)| *end);
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
        for (end, sequence) in queries {
            parse(&mut parser, &mut term, &bytes[fed..end]);
            fed = end;
            match sequence {
                Some(SchemeSequence::Subscribe) => {
                    self.scheme.subscribed.store(true, Ordering::Relaxed)
                }
                Some(SchemeSequence::Unsubscribe) => {
                    self.scheme.subscribed.store(false, Ordering::Relaxed)
                }
                Some(SchemeSequence::Query) => {
                    let _ = self
                        .events
                        .send(ProxyEvent::Terminal(Event::PtyWrite(scheme_report(
                            self.palette.lock().unwrap().theme,
                        ))));
                }
                None => {
                    let _ = self.events.send(ProxyEvent::Terminal(Event::PtyWrite(
                        crate::program_status::PROBE_REPLY.into(),
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

    pub fn take_program_statuses(&self) -> Vec<(u64, Option<crate::program_status::Report>)> {
        std::mem::take(&mut *self.pending_program_status.lock().unwrap())
    }

    pub fn take_program_status(&self) -> Option<(u64, Option<crate::program_status::Report>)> {
        self.take_program_statuses().pop()
    }

    pub fn clear_program_status(&self) {
        *self.program_status.lock().unwrap() = Default::default();
        self.pending_program_status.lock().unwrap().clear();
    }

    pub fn invalidate_program_status(&self) {
        self.program_status.lock().unwrap().invalidate_records();
        self.pending_program_status.lock().unwrap().clear();
    }

    /// Callers publish after releasing the tracker, so each publication reports the tracker's
    /// latest observation in order: a delayed caller cannot replace a newer state with its own.
    pub fn publish_input(&self, observation: Option<InputObservation>) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        if observation.is_none() {
            return;
        }
        let _order = self.publish_order.lock().unwrap();
        let latest = self.input_tracker.lock().unwrap().latest_observation();
        self.host.report_input_state(&self.session_id, latest);
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
        if self.stopped.load(Ordering::Acquire) {
            anyhow::bail!("terminal stopped");
        }
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
    pub fn sync_session_title(&self, initial_title: String, apply: impl FnOnce() + Send + 'static) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        let observation = {
            let mut sequence = self.sequence.lock().unwrap();
            let mut parser = self.parser.lock().unwrap();
            let mut tracker = self.input_tracker.lock().unwrap();
            let mut term = self.term.lock();
            let now = Instant::now();
            let previous_mode = *term.mode();
            let held_sync = parser.processor.sync_timeout().sync_timeout().is_some();
            if held_sync {
                parser.processor.stop_sync(&mut *term);
                parser.boundary.end_sync();
            }
            parser.flush_scheduled = false;
            let preceding = parser.processor.preceding_char();
            parser.processor.restore_preceding_char(preceding);
            parser.boundary = Default::default();
            parser.burst = Default::default();
            while !term.snapshot_state().title_stack.is_empty() {
                Handler::pop_title(&mut *term);
            }
            let _ = self.events.send(ProxyEvent::SyncSessionTitle {
                initial_title,
                apply: Box::new(apply),
            });
            held_sync
                .then(|| {
                    let seq = sequence.last;
                    observe_parsed(
                        &mut sequence,
                        &mut tracker,
                        &mut term,
                        previous_mode,
                        seq,
                        now,
                    )
                })
                .flatten()
        };
        if let Some(reports) = &self.cwd_reports {
            reports.lock().unwrap().carry.clear();
        }
        self.scheme.tail.lock().unwrap().clear();
        self.publish_input(observation);
    }
    pub fn apply_session_title(
        &self,
        title: String,
        persist: impl FnOnce(&str, bool) -> Option<Box<dyn FnOnce() + Send>> + Send + 'static,
    ) -> bool {
        if self.stopped.load(Ordering::Acquire) {
            return false;
        }
        self.events
            .send(ProxyEvent::ApplySessionTitle {
                title,
                persist: Box::new(persist),
            })
            .is_ok()
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
            let _ = self
                .events
                .send(ProxyEvent::Terminal(Event::PtyWrite(scheme_report(
                    palette,
                ))));
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
    pub fn confirm_delivery(&self, revision: u64) -> Option<InputObservation> {
        self.input_tracker
            .lock()
            .unwrap()
            .confirm_delivery(revision, Instant::now())
    }
    pub fn recheck_delivery(&self) {
        let observation = {
            let mut tracker = self.input_tracker.lock().unwrap();
            let term = self.term.lock_unfair();
            tracker.recheck_delivery(Instant::now(), &term)
        };
        self.publish_input(observation);
    }
    pub fn input_reset_guard(&self) -> u64 {
        self.input_tracker.lock().unwrap().reset_guard()
    }
    pub fn delivery_held(&self) -> bool {
        self.input_tracker.lock().unwrap().delivery_held()
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
            .send(ProxyEvent::Terminal(Event::TextAreaSizeRequest(Arc::new(
                move |_| {
                    let _ = tx.send(());
                    String::new()
                },
            ))))
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
        fn clipboard_store(&self, _: &str, _: ClipboardTarget, _: &str) {}
    }

    #[test]
    fn stopped_workers_join_while_the_model_is_still_retained() {
        let model = TerminalModel::new(
            Arc::new(Host::default()),
            "stopped".into(),
            80,
            4,
            ModelOptions::default(),
        )
        .unwrap();
        model.feed_output(1, b"\x1b[?2026hbuffered update");
        let workers = model.stop_workers();
        assert_eq!(workers.len(), 2);
        for worker in workers {
            worker.join().unwrap();
        }
        assert!(model.stop_workers().is_empty());
        assert!(model.feed_output(2, b"\x1b]10;?\x1b\\").is_none());
        assert!(model.write_input(b"late input").is_err());
    }

    #[test]
    fn sync_session_title_orders_queued_titles_across_generations_and_resets_cache() {
        struct GenerationHost {
            generation: Arc<Mutex<String>>,
            persisted: Mutex<Vec<(String, String)>>,
            first_entered: Sender<()>,
            release_first: Mutex<Option<mpsc::Receiver<()>>>,
        }
        impl TerminalHost for GenerationHost {
            fn write_input(&self, _: &str, _: &[u8]) -> Result<()> {
                Ok(())
            }
            fn write_reply(&self, _: &str, _: &[u8]) -> Result<()> {
                Ok(())
            }
            fn report_input_state(&self, _: &str, _: InputObservation) {}
            fn set_live_title(&self, _: &str, title: &str) -> Result<()> {
                let first = self.release_first.lock().unwrap().take();
                if let Some(release) = first {
                    let _ = self.first_entered.send(());
                    release.recv_timeout(Duration::from_secs(5)).unwrap();
                }
                let generation = self.generation.lock().unwrap().clone();
                self.persisted
                    .lock()
                    .unwrap()
                    .push((generation, title.to_owned()));
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

        let generation = Arc::new(Mutex::new("gen-1".to_owned()));
        let (first_entered, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let host = Arc::new(GenerationHost {
            generation: Arc::clone(&generation),
            persisted: Mutex::default(),
            first_entered,
            release_first: Mutex::new(Some(release_rx)),
        });
        let model = TerminalModel::new(
            host.clone(),
            "rebind".into(),
            80,
            4,
            ModelOptions {
                agent: true,
                ..ModelOptions::default()
            },
        )
        .unwrap();

        // Hold the worker inside the first title callback, then queue a late pre-resume
        // title, the rebind fence (clearing cached title for fresh fallback), and an
        // identical post-resume title before releasing the worker.
        model.feed_output(1, b"\x1b]2;First Run\x07");
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        model.feed_output(2, b"\x1b]2;Same Topic\x07");
        let rebind_gen = Arc::clone(&generation);
        model.sync_session_title(String::new(), move || {
            *rebind_gen.lock().unwrap() = "gen-2".to_owned();
        });
        model.feed_output(3, b"\x1b]2;Same Topic\x07");
        release_tx.send(()).unwrap();
        model.flush_events();

        assert_eq!(
            *host.persisted.lock().unwrap(),
            vec![
                ("gen-1".to_owned(), "First Run".to_owned()),
                ("gen-1".to_owned(), "Same Topic".to_owned()),
                ("gen-2".to_owned(), "Same Topic".to_owned()),
            ]
        );
        let workers = model.stop_workers();
        for worker in workers {
            worker.join().unwrap();
        }
    }

    #[test]
    fn sync_session_title_drains_unfinished_synchronized_updates_and_partial_osc_before_rebind() {
        struct RecordingHost {
            generation: Arc<Mutex<&'static str>>,
            emitted: Mutex<Vec<(&'static str, String)>>,
        }
        impl TerminalHost for RecordingHost {
            fn write_input(&self, _: &str, _: &[u8]) -> Result<()> {
                Ok(())
            }
            fn write_reply(&self, _: &str, _: &[u8]) -> Result<()> {
                Ok(())
            }
            fn report_input_state(&self, _: &str, _: InputObservation) {}
            fn set_live_title(&self, _: &str, title: &str) -> Result<()> {
                self.emitted
                    .lock()
                    .unwrap()
                    .push((*self.generation.lock().unwrap(), title.to_owned()));
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

        let generation = Arc::new(Mutex::new("old"));
        let host = Arc::new(RecordingHost {
            generation: Arc::clone(&generation),
            emitted: Mutex::default(),
        });
        let model = TerminalModel::new(
            host.clone(),
            "sync-rebind".into(),
            80,
            4,
            ModelOptions {
                agent: true,
                ..ModelOptions::default()
            },
        )
        .unwrap();

        // 1. Unfinished ESC[?2026h block followed by rebind and later ESC[?2026l.
        model.feed_output(1, b"\x1b[?2026h\x1b]2;Old buffered title\x07");
        let (bound_tx, bound_rx) = mpsc::channel();
        let gen_for_rebind = Arc::clone(&generation);
        model.sync_session_title(String::new(), move || {
            *gen_for_rebind.lock().unwrap() = "new";
            bound_tx.send(()).unwrap();
        });
        bound_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        model.feed_output(2, b"\x1b[?2026l");
        model.flush_events();
        assert_eq!(model.title(), "");
        assert_eq!(
            *host.emitted.lock().unwrap(),
            vec![("old", "Old buffered title".to_owned())]
        );

        // 2. Unfinished ESC[?2026h block racing with the sync-flush timeout worker, plus
        // stacked title (`CSI 22;0t`) and unterminated OSC 2 bytes before rebind.
        model.feed_output(
            3,
            b"\x1b]2;Stacked Old\x07\x1b[22;0t\x1b[?2026h\x1b]2;Buffered Before Timeout\x07",
        );
        model.feed_output(4, b"\x1b[?2026l\x1b]2;Unterminated Old ");
        model.feed_output(5, b"\x07\x1b[?2026h\x1b]2;Held Sync Two\x07");
        let (bound2_tx, bound2_rx) = mpsc::channel();
        let gen_for_rebind2 = Arc::clone(&generation);
        model.sync_session_title(String::new(), move || {
            *gen_for_rebind2.lock().unwrap() = "gen-3";
            bound2_tx.send(()).unwrap();
        });
        bound2_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        // Also test an unterminated OSC sequence across rebind into gen-4.
        model.feed_output(6, b"\x1b]2;Dangling Prefix ");
        let (bound3_tx, bound3_rx) = mpsc::channel();
        let gen_for_rebind3 = Arc::clone(&generation);
        model.sync_session_title(String::new(), move || {
            *gen_for_rebind3.lock().unwrap() = "gen-4";
            bound3_tx.send(()).unwrap();
        });
        bound3_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        model.feed_output(7, b"Plain Text\x07\x1b[23;0t\x1b]2;Clean Gen Four\x07");
        model.flush_events();

        let emitted = host.emitted.lock().unwrap().clone();
        assert!(
            !emitted.iter().any(|(generation, _)| *generation == "gen-3"),
            "unexpected gen-3 emissions: {emitted:?}"
        );
        let gen4_titles: Vec<_> = emitted
            .iter()
            .filter(|(generation, _)| *generation == "gen-4")
            .map(|(_, title)| title.as_str())
            .collect();
        assert_eq!(gen4_titles, vec!["Clean Gen Four"]);
        assert_eq!(model.title(), "Clean Gen Four");

        let workers = model.stop_workers();
        for worker in workers {
            worker.join().unwrap();
        }
    }

    #[test]
    fn program_probe_reply_precedes_da_in_the_same_or_next_chunk() {
        #[derive(Default)]
        struct Replies(Mutex<Vec<Vec<u8>>>);
        impl TerminalHost for Replies {
            fn write_input(&self, _: &str, _: &[u8]) -> Result<()> {
                Ok(())
            }
            fn write_reply(&self, _: &str, bytes: &[u8]) -> Result<()> {
                if !bytes.is_empty() {
                    self.0.lock().unwrap().push(bytes.to_vec());
                }
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
        for terminator in ["\x07", "\x1b\\"] {
            let bytes = format!("\x1b]7501;?{terminator}\x1b[c");
            for split in 0..=bytes.len() {
                let host = Arc::new(Replies::default());
                let model = TerminalModel::new(
                    host.clone(),
                    "probe".into(),
                    80,
                    4,
                    ModelOptions::default(),
                )
                .unwrap();
                model.feed_output(1, &bytes.as_bytes()[..split]);
                model.feed_output(2, &bytes.as_bytes()[split..]);
                model.flush_events();
                let replies = host.0.lock().unwrap();
                assert_eq!(replies.len(), 2, "split {split}");
                assert_eq!(replies[0], crate::program_status::PROBE_REPLY.as_bytes());
                assert!(replies[1].starts_with(b"\x1b[?"));
                assert!(replies[1].ends_with(b"c"));
            }
        }
    }

    #[test]
    fn resubmit_accepts_the_trackers_unsent_notice_when_the_probe_cannot_see_it() {
        let model = TerminalModel::new(
            Arc::new(Host::default()),
            "delivery".into(),
            80,
            4,
            ModelOptions::default(),
        )
        .unwrap();
        model.feed_output(1, b"> ");
        let text = "[inbox] 1 new messages — run `runner msg read` to view.";
        let blind = InputProbe::new(Vec::new(), text);
        model.track_delivery_input(text);
        model.feed_output(2, text.as_bytes());
        let revision = model.input_reset_guard();
        assert_eq!(model.with_unchanged_input(revision, &blind, || ()), None);
        model.track_delivery_submit();
        model.feed_output(3, format!("\r> {text}").as_bytes());
        assert_eq!(
            model.with_unchanged_input(revision, &blind, || ()),
            Some(())
        );
        model.observe_input(&InputEvent::Key {
            kind: crate::mappings::InputKind::Navigate,
        });
        assert!(model
            .input_tracker
            .lock()
            .unwrap()
            .unsent_notice(&model.term.lock()));
        assert_eq!(model.with_unchanged_input(revision, &blind, || ()), None);
        let workers = model.take_workers();
        drop(model);
        for worker in workers {
            worker.join().unwrap();
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

#[cfg(test)]
mod clipboard_tests {
    use super::*;
    use base64::Engine as _;

    #[derive(Default)]
    struct Host {
        stores: Mutex<Vec<(ClipboardTarget, String)>>,
        replies: Mutex<Vec<Vec<u8>>>,
    }
    impl TerminalHost for Host {
        fn write_input(&self, _: &str, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn write_reply(&self, _: &str, bytes: &[u8]) -> Result<()> {
            if !bytes.is_empty() {
                self.replies.lock().unwrap().push(bytes.to_vec());
            }
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
        fn clipboard_store(&self, session_id: &str, target: ClipboardTarget, text: &str) {
            assert_eq!(session_id, "osc52");
            self.stores.lock().unwrap().push((target, text.to_owned()));
        }
    }

    fn feed(output: &[u8]) -> Arc<Host> {
        let host = Arc::new(Host::default());
        let model =
            TerminalModel::new(host.clone(), "osc52".into(), 80, 4, ModelOptions::default())
                .unwrap();
        model.feed_output(1, output);
        model.flush_events();
        let workers = model.take_workers();
        drop(model);
        for worker in workers {
            worker.join().unwrap();
        }
        host
    }

    fn osc52(selector: &str, payload: &str, terminator: &str) -> Vec<u8> {
        format!("\x1b]52;{selector};{payload}{terminator}").into_bytes()
    }

    fn encode(text: &str) -> String {
        base64::engine::general_purpose::STANDARD.encode(text)
    }

    #[test]
    fn osc52_writes_reach_the_host_decoded() {
        for terminator in ["\x07", "\x1b\\"] {
            let host = feed(&osc52("c", &encode("runner-osc52 ✓"), terminator));
            assert_eq!(
                *host.stores.lock().unwrap(),
                [(ClipboardTarget::Clipboard, "runner-osc52 ✓".to_owned())]
            );
        }
        for selector in ["p", "s"] {
            let host = feed(&osc52(selector, &encode("primary"), "\x07"));
            assert_eq!(
                *host.stores.lock().unwrap(),
                [(ClipboardTarget::Selection, "primary".to_owned())]
            );
        }
    }

    #[test]
    fn empty_oversize_and_invalid_osc52_writes_are_ignored() {
        let largest = "a".repeat(MAX_CLIPBOARD_BYTES);
        let host = feed(&osc52("c", &encode(&largest), "\x07"));
        assert_eq!(host.stores.lock().unwrap().len(), 1);
        for payload in [
            String::new(),
            encode(&format!("{largest}a")),
            "not base64!".into(),
        ] {
            let host = feed(&osc52("c", &payload, "\x07"));
            assert!(host.stores.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn osc52_reads_are_refused_without_a_reply() {
        for terminator in ["\x07", "\x1b\\"] {
            let host = feed(&osc52("c", "?", terminator));
            assert!(host.stores.lock().unwrap().is_empty());
            assert!(host.replies.lock().unwrap().is_empty());
        }
    }

    #[test]
    fn program_status_preserves_ordered_transitions_within_a_single_chunk() {
        use crate::program_status::State;
        let model = TerminalModel::new(
            Arc::new(Host::default()),
            "coalesced-status".into(),
            80,
            4,
            ModelOptions::default(),
        )
        .unwrap();
        model.feed_output(
            1,
            b"\x1b]7501;state=working:app=agy\x07\x1b]7501;state=done:app=agy\x07\x1b]7501;state=idle:app=agy\x07",
        );
        let states = model
            .take_program_statuses()
            .into_iter()
            .map(|(seq, report)| (seq, report.map(|report| report.state)))
            .collect::<Vec<_>>();
        assert_eq!(
            states,
            vec![
                (1, Some(State::Working)),
                (1, Some(State::Done)),
                (1, Some(State::Idle)),
            ]
        );
        assert!(model.take_program_statuses().is_empty());
    }
}
