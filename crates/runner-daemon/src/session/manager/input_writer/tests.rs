use super::*;
use std::sync::mpsc;

/// Signals that the blocked write started, then waits for its release.
type Gate = (mpsc::Sender<()>, mpsc::Receiver<()>);

/// Records each write. An armed gate makes the next write report that it started and block
/// until released, like a PTY whose agent has stopped reading.
#[derive(Clone, Default)]
struct Pty {
    written: Arc<Mutex<Vec<Vec<u8>>>>,
    gate: Arc<Mutex<Option<Gate>>>,
}

impl Pty {
    fn writer(&self, deadline: Duration) -> Arc<InputWriter> {
        let pty = self.clone();
        InputWriter::spawn(
            "s",
            deadline,
            Sink::Bound(Box::new(move |bytes| {
                let gate = pty.gate.lock().unwrap().take();
                if let Some((started, release)) = gate {
                    started.send(()).unwrap();
                    let _ = release.recv();
                }
                pty.written.lock().unwrap().push(bytes.to_vec());
                Ok(())
            })),
        )
    }

    /// Blocks the next write; returns its start signal and its release.
    fn block_next(&self) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
        let (started, entered) = mpsc::channel();
        let (release, released) = mpsc::channel();
        *self.gate.lock().unwrap() = Some((started, released));
        (entered, release)
    }

    fn written(&self) -> Vec<Vec<u8>> {
        self.written.lock().unwrap().clone()
    }
}

const NO_DEADLINE: Duration = Duration::from_secs(600);

fn started(entered: &mpsc::Receiver<()>) {
    entered
        .recv_timeout(Duration::from_secs(30))
        .expect("the blocked write starts");
}

#[test]
fn writes_land_in_the_order_they_were_queued() {
    let pty = Pty::default();
    let writer = pty.writer(NO_DEADLINE);
    let (entered, release) = pty.block_next();
    let first = writer.push(b"a").unwrap();
    started(&entered);
    let rest = [b"b", b"c", b"\r"].map(|bytes| writer.push(bytes).unwrap());
    release.send(()).unwrap();
    assert_eq!(first.wait().unwrap(), Written::Done);
    for write in rest {
        assert_eq!(write.wait().unwrap(), Written::Done);
    }
    assert_eq!(pty.written(), [&b"a"[..], b"b", b"c", b"\r"]);
}

#[test]
fn an_overdue_write_stands_while_what_queued_behind_it_is_dropped() {
    let pty = Pty::default();
    let deadline = Duration::from_millis(200);
    let writer = pty.writer(deadline);
    let (entered, release) = pty.block_next();
    let pushed = Instant::now();
    let enter = writer.push(b"\r").unwrap();
    started(&entered);
    let body = writer.push(b"body").unwrap();

    let dropped = body.wait().unwrap_err().to_string();
    assert!(pushed.elapsed() >= deadline);
    assert!(dropped.contains("was dropped"), "{dropped}");
    match enter.wait().unwrap() {
        Written::Overdue(message) => assert!(
            message.contains("arrives if the session reads again"),
            "{message}"
        ),
        Written::Done => panic!("the Enter is still blocked"),
    }
    let refused = writer
        .push(b"c")
        .err()
        .expect("input is refused while the write is overdue");
    assert!(refused.to_string().contains("was dropped"), "{refused}");

    // The agent reads again: the overdue Enter lands once and nothing dropped follows it.
    release.send(()).unwrap();
    writer.wait_until(|state| state.in_flight.is_none() && !state.stalled);
    assert_eq!(writer.push(b"d").unwrap().wait().unwrap(), Written::Done);
    assert_eq!(pty.written(), [&b"\r"[..], b"d"]);
}

#[test]
fn unwritten_input_never_exceeds_the_limit_while_a_write_is_under_way() {
    let pty = Pty::default();
    let writer = pty.writer(NO_DEADLINE);
    let (entered, release) = pty.block_next();
    let first = writer.push(b"a").unwrap();
    started(&entered);
    let to_limit = writer.push(&vec![b'x'; INPUT_QUEUE_LIMIT - 2]).unwrap();
    let at_limit = writer.push(b"y").unwrap();
    for bytes in [&b"z"[..], &vec![b'x'; INPUT_QUEUE_LIMIT + 1]] {
        let full = writer.push(bytes).err().expect("the queue is full");
        assert!(full.to_string().contains("is full"), "{full}");
    }
    release.send(()).unwrap();
    for write in [first, to_limit, at_limit] {
        write.wait().unwrap();
    }

    // With nothing else unwritten, one large paste still goes.
    writer.wait_until_idle();
    writer
        .push(&vec![b'x'; INPUT_QUEUE_LIMIT + 1])
        .unwrap()
        .wait()
        .unwrap();
    assert_eq!(pty.written().len(), 4);
}

#[test]
fn a_conditional_write_is_declined_while_anything_is_unwritten() {
    let pty = Pty::default();
    let writer = pty.writer(NO_DEADLINE);
    let (entered, release) = pty.block_next();
    let first = writer.push(b"a").unwrap();
    started(&entered);
    assert!(writer.push_when_idle(b"\r").unwrap().is_none());
    release.send(()).unwrap();
    first.wait().unwrap();
    writer.wait_until_idle();
    writer
        .push_when_idle(b"\r")
        .unwrap()
        .expect("an idle writer takes it")
        .wait()
        .unwrap();
    assert_eq!(pty.written(), [&b"a"[..], b"\r"]);
}

#[test]
fn closing_fails_pending_input_at_once_and_never_writes_it() {
    let pty = Pty::default();
    let writer = pty.writer(NO_DEADLINE);
    let (entered, release) = pty.block_next();
    let first = writer.push(b"a").unwrap();
    started(&entered);
    let queued = writer.push(b"b").unwrap();
    writer.close();

    let error = first.wait().unwrap_err().to_string();
    assert!(
        error.contains("stopped before its input was written"),
        "{error}"
    );
    assert!(queued.wait().is_err());
    assert!(writer.push(b"c").is_err());
    release.send(()).unwrap();
    writer.wait_until(|state| state.in_flight.is_none());
    assert_eq!(pty.written(), [b"a"]);
}
