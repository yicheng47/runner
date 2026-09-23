use super::*;

#[derive(Debug, PartialEq, Eq)]
enum ForwardedEvent {
    Output(u64, Vec<u8>),
    Status(SessionActivityState, String),
}

#[derive(Default)]
struct ForwarderCapture(Mutex<Vec<ForwardedEvent>>);

impl SessionEvents for ForwarderCapture {
    fn output(&self, ev: &OutputEvent) {
        self.0
            .lock()
            .unwrap()
            .push(ForwardedEvent::Output(ev.seq, ev.bytes.clone()));
    }

    fn status(&self, ev: &SessionActivityEvent) {
        self.0
            .lock()
            .unwrap()
            .push(ForwardedEvent::Status(ev.state, ev.source.clone()));
    }

    fn exit(&self, _: &ExitEvent) {}
}

fn forward_queued_output(items: Vec<RuntimeOutput>) -> Vec<ForwardedEvent> {
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let (rt_session, output) = fake
        .spawn(SpawnSpec {
            session_id: "burst-test".into(),
            ..Default::default()
        })
        .unwrap();
    for item in items {
        match item {
            RuntimeOutput::Stream(bytes) => fake.push_output(0, &bytes),
            RuntimeOutput::StatusTransition { state, .. } => fake.push_status(0, state),
            RuntimeOutput::AgentObservation(_) | RuntimeOutput::StatusBridgeFailed => {
                panic!("not a byte-batching fixture")
            }
        }
    }
    fake.close_spawn(0);
    let capture = Arc::new(ForwarderCapture::default());
    let app_data = tempfile::tempdir().unwrap();
    mgr.start_forwarder_thread(
        rt_session.session_id.clone(),
        None,
        rt_session,
        output,
        pool_with_schema(),
        capture.clone(),
        role("fake", &[]),
        false,
        false,
        None,
        app_data.path().to_path_buf(),
    )
    .join()
    .unwrap();
    let events = std::mem::take(&mut *capture.0.lock().unwrap());
    events
}

#[test]
fn forwarder_coalesces_queued_stream_chunks_into_one_output_event() {
    let chunks = [
        vec![b'a'; 8 * 1024],
        vec![b'b'; 8 * 1024],
        vec![b'c'; 8 * 1024],
    ];
    let events = forward_queued_output(
        chunks
            .iter()
            .map(|bytes| RuntimeOutput::Stream(bytes.to_vec()))
            .collect(),
    );
    assert_eq!(events, vec![ForwardedEvent::Output(1, chunks.concat())]);
    eprintln!(
        "forwarder burst: {} queued 8 KiB chunks -> {} output event",
        chunks.len(),
        events.len()
    );
}

#[cfg(windows)]
#[test]
fn forwarder_delivers_a_cursor_burst_without_waiting_for_eof() {
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let (rt_session, output) = fake
        .spawn(SpawnSpec {
            session_id: "cursor-burst-test".into(),
            ..Default::default()
        })
        .unwrap();
    let redraw = b"\x1b[?2026h\x1b[?25l\x1b[38;1H\x1b[?25h\x1b[0 q\x1b[?2026l";
    let restore = b"\x1b[?25l \x1b[42;3H\x1b[?25h";
    fake.push_output(0, redraw);
    fake.push_output(0, restore);
    let capture = Arc::new(ForwarderCapture::default());
    let app_data = tempfile::tempdir().unwrap();
    let forwarder = mgr.start_forwarder_thread(
        rt_session.session_id.clone(),
        None,
        rt_session,
        output,
        pool_with_schema(),
        capture.clone(),
        role("fake", &[]),
        false,
        false,
        None,
        app_data.path().to_path_buf(),
    );
    let deadline = Instant::now() + Duration::from_secs(2);
    while capture.0.lock().unwrap().is_empty() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    let events = std::mem::take(&mut *capture.0.lock().unwrap());
    fake.close_spawn(0);
    forwarder.join().unwrap();
    assert_eq!(
        events,
        vec![ForwardedEvent::Output(
            1,
            [redraw.as_slice(), restore.as_slice()].concat()
        )],
    );
}

#[test]
fn forwarder_preserves_status_transition_between_stream_chunks() {
    let events = forward_queued_output(vec![
        RuntimeOutput::Stream(b"before".to_vec()),
        RuntimeOutput::StatusTransition {
            state: SessionActivityState::Idle,
            source: "forwarder",
        },
        RuntimeOutput::Stream(b"after".to_vec()),
    ]);
    assert_eq!(
        events,
        vec![
            ForwardedEvent::Output(1, b"before".to_vec()),
            ForwardedEvent::Status(SessionActivityState::Idle, "forwarder".into()),
            ForwardedEvent::Output(2, b"after".to_vec()),
        ]
    );
}

#[test]
fn forwarder_caps_bursts_without_losing_the_next_chunk() {
    for first_len in [1024 * 1024, 1024 * 1024 - 1] {
        let first = vec![b'a'; first_len];
        let events = forward_queued_output(vec![
            RuntimeOutput::Stream(first.clone()),
            RuntimeOutput::Stream(b"bc".to_vec()),
            RuntimeOutput::Stream(b"de".to_vec()),
        ]);
        assert_eq!(
            events,
            vec![
                ForwardedEvent::Output(1, first),
                ForwardedEvent::Output(2, b"bcde".to_vec()),
            ]
        );
    }
}
