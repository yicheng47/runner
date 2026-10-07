use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use runner_core::protocol::hook::{
    HookReport, MAX_ENVELOPE_BYTES, MAX_QUEUE_BYTES, MAX_QUEUE_REPORTS,
};
use runner_core::protocol::Runtime;

use crate::error::{Error, Result};

#[derive(Default)]
pub struct HookRoutes {
    routes: Mutex<HashMap<String, Arc<Queue>>>,
}

struct Queue {
    runtime: Runtime,
    session: String,
    generation: String,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    closed: bool,
    failed: bool,
    bytes: usize,
    reports: VecDeque<Queued>,
}

struct Queued {
    report: HookReport,
    bytes: usize,
    ready: Arc<AtomicBool>,
}

pub struct Admission {
    ready: Arc<AtomicBool>,
}

impl Drop for Admission {
    fn drop(&mut self) {
        // A lost acknowledgment is ambiguous delivery, not permission to replay.
        self.ready.store(true, Ordering::Release);
    }
}

pub struct HookReceiver {
    routes: Weak<HookRoutes>,
    queue: Arc<Queue>,
}

#[cfg(test)]
pub(crate) struct TestHookRoute {
    routes: Arc<HookRoutes>,
    runtime: Runtime,
    generation: String,
}

#[cfg(test)]
impl TestHookRoute {
    pub(crate) fn new(runtime: Runtime, generation: String) -> (Self, HookReceiver) {
        let routes = Arc::new(HookRoutes::default());
        let receiver = routes.register(runtime, "fixture".into(), generation.clone());
        (
            Self {
                routes,
                runtime,
                generation,
            },
            receiver,
        )
    }

    pub(crate) fn admit(&self, payload: serde_json::Value) -> Result<Admission> {
        self.routes.admit(HookReport {
            version: runner_core::protocol::hook::VERSION,
            runtime: self.runtime,
            session_id: "fixture".into(),
            generation: payload["generation"]
                .as_str()
                .unwrap_or(&self.generation)
                .into(),
            event: payload["hook_event_name"]
                .as_str()
                .unwrap_or_default()
                .into(),
            caller_thread_id: (self.runtime == Runtime::Codex)
                .then(|| payload["session_id"].as_str().map(str::to_owned))
                .flatten(),
            payload,
            bridge_unavailable: false,
        })
    }

    pub(crate) fn admit_json(&self, payload: &str) -> Result<Admission> {
        self.admit(serde_json::from_str(payload)?)
    }

    pub(crate) fn admit_event(
        &self,
        event: &str,
        mut payload: serde_json::Value,
    ) -> Result<Admission> {
        payload["hook_event_name"] = event.into();
        self.admit(payload)
    }

    pub(crate) fn retire(&self) {
        self.routes.retire("fixture");
    }
}

impl HookRoutes {
    pub fn register(
        self: &Arc<Self>,
        runtime: Runtime,
        session: String,
        generation: String,
    ) -> HookReceiver {
        let queue = Arc::new(Queue {
            runtime,
            session: session.clone(),
            generation,
            state: Mutex::default(),
        });
        if let Some(previous) = self.routes.lock().unwrap().insert(session, queue.clone()) {
            previous.close();
        }
        HookReceiver {
            routes: Arc::downgrade(self),
            queue,
        }
    }

    pub fn retire(&self, session: &str) {
        if let Some(queue) = self.routes.lock().unwrap().remove(session) {
            queue.close();
        }
    }

    pub fn admit(&self, report: HookReport) -> Result<Admission> {
        if !report.valid() {
            return Err(Error::msg("invalid hook report"));
        }
        let bytes = serde_json::to_vec(&report)?.len();
        if bytes > MAX_ENVELOPE_BYTES {
            return Err(Error::msg("hook envelope too large"));
        }
        let queue = self
            .routes
            .lock()
            .unwrap()
            .get(&report.session_id)
            .cloned()
            .ok_or_else(|| Error::msg("hook launch ended"))?;
        if queue.runtime != report.runtime || queue.generation != report.generation {
            return Err(Error::msg("stale hook launch"));
        }
        let mut state = queue.state.lock().unwrap();
        if state.closed || state.failed {
            return Err(Error::msg("hook bridge unavailable"));
        }
        if report.bridge_unavailable {
            state.failed = true;
            state.reports.clear();
            state.bytes = 0;
            return Ok(Admission {
                ready: Arc::new(AtomicBool::new(false)),
            });
        }
        if state.reports.len() == MAX_QUEUE_REPORTS || state.bytes + bytes > MAX_QUEUE_BYTES {
            state.failed = true;
            state.reports.clear();
            state.bytes = 0;
            return Err(Error::msg("hook queue exhausted"));
        }
        let ready = Arc::new(AtomicBool::new(false));
        state.bytes += bytes;
        state.reports.push_back(Queued {
            report,
            bytes,
            ready: ready.clone(),
        });
        Ok(Admission { ready })
    }
}

impl Queue {
    fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        state.reports.clear();
        state.bytes = 0;
    }
}

impl HookReceiver {
    pub fn drain(&mut self, mut observe: impl FnMut(HookReport)) -> Result<()> {
        loop {
            let next = {
                let mut state = self.queue.state.lock().unwrap();
                if state.closed || state.failed {
                    return Err(Error::msg("hook bridge unavailable"));
                }
                if state
                    .reports
                    .front()
                    .is_none_or(|report| !report.ready.load(Ordering::Acquire))
                {
                    return Ok(());
                }
                let next = state.reports.pop_front().unwrap();
                state.bytes -= next.bytes;
                next.report
            };
            if next.generation == self.queue.generation && next.runtime == self.queue.runtime {
                observe(next);
            }
        }
    }
}

impl Drop for HookReceiver {
    fn drop(&mut self) {
        self.queue.close();
        if let Some(routes) = self.routes.upgrade() {
            let mut routes = routes.routes.lock().unwrap();
            if routes
                .get(&self.queue.session)
                .is_some_and(|queue| Arc::ptr_eq(queue, &self.queue))
            {
                routes.remove(&self.queue.session);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runner_core::protocol::hook::VERSION;
    use serde_json::json;

    #[test]
    fn concurrent_launches_keep_independent_fifo_and_exact_envelope_boundary() {
        let routes = Arc::new(HookRoutes::default());
        let mut receivers = Vec::new();
        let mut producers = Vec::new();
        for session in ["one", "two", "three"] {
            receivers.push(routes.register(Runtime::Codex, session.into(), "launch".into()));
            let routes = routes.clone();
            producers.push(std::thread::spawn(move || {
                for number in 0..MAX_QUEUE_REPORTS {
                    let mut value = report("launch", number);
                    value.session_id = session.into();
                    drop(routes.admit(value).unwrap());
                }
            }));
        }
        for producer in producers {
            producer.join().unwrap();
        }
        for receiver in &mut receivers {
            let mut sequence = Vec::new();
            receiver
                .drain(|report| {
                    sequence.push(
                        report.payload["turn_id"]
                            .as_str()
                            .unwrap()
                            .parse::<usize>()
                            .unwrap(),
                    )
                })
                .unwrap();
            assert_eq!(sequence, (0..MAX_QUEUE_REPORTS).collect::<Vec<_>>());
        }
        let mut receiver = routes.register(Runtime::Codex, "runner".into(), "launch".into());
        let mut value = report("launch", 1);
        value.payload["prompt"] = json!("");
        let remaining = MAX_ENVELOPE_BYTES - serde_json::to_vec(&value).unwrap().len();
        value.payload["prompt"] = json!(format!(
            "{}{}",
            "你".repeat(remaining / 3),
            "x".repeat(remaining % 3)
        ));
        assert_eq!(
            serde_json::to_vec(&value).unwrap().len(),
            MAX_ENVELOPE_BYTES
        );
        drop(routes.admit(value.clone()).unwrap());
        let mut text = value.payload["prompt"].as_str().unwrap().to_owned();
        text.push('x');
        value.payload["prompt"] = json!(text);
        assert!(routes.admit(value).is_err());
        let mut received = 0;
        receiver.drain(|_| received += 1).unwrap();
        assert_eq!(received, 1);
        routes.retire("runner");
        assert!(receiver.drain(|_| panic!("retired launch")).is_err());
    }

    fn report(generation: &str, number: usize) -> HookReport {
        HookReport {
            bridge_unavailable: false,
            version: VERSION,
            runtime: Runtime::Codex,
            session_id: "runner".into(),
            generation: generation.into(),
            event: "UserPromptSubmit".into(),
            payload: json!({"session_id":"root", "turn_id":number.to_string()}),
            caller_thread_id: Some("root".into()),
        }
    }

    #[test]
    fn admission_precedes_consumption_and_preserves_fifo() {
        let routes = Arc::new(HookRoutes::default());
        let mut receiver = routes.register(Runtime::Codex, "runner".into(), "one".into());
        let first = routes.admit(report("one", 1)).unwrap();
        drop(routes.admit(report("one", 2)).unwrap());
        receiver.drain(|_| panic!("not acknowledged")).unwrap();
        drop(first);
        let mut received = Vec::new();
        receiver
            .drain(|report| received.push(report.payload["turn_id"].clone()))
            .unwrap();
        assert_eq!(received, vec![json!("1"), json!("2")]);
    }

    #[test]
    fn replacement_rejects_stale_reports_and_queued_work() {
        let routes = Arc::new(HookRoutes::default());
        let mut old = routes.register(Runtime::Codex, "runner".into(), "one".into());
        let pending = routes.admit(report("one", 1)).unwrap();
        let mut current = routes.register(Runtime::Codex, "runner".into(), "two".into());
        drop(pending);
        assert!(old.drain(|_| panic!("stale queued event")).is_err());
        assert!(routes.admit(report("one", 2)).is_err());
        drop(old);
        drop(routes.admit(report("two", 3)).unwrap());
        let mut received = 0;
        current.drain(|_| received += 1).unwrap();
        assert_eq!(received, 1);
        drop(current);
        assert!(routes.admit(report("two", 4)).is_err());
        assert!(routes.routes.lock().unwrap().is_empty());
    }

    #[test]
    fn unavailable_control_is_generation_checked_and_bypasses_a_full_queue() {
        let routes = Arc::new(HookRoutes::default());
        let mut receiver = routes.register(Runtime::Codex, "runner".into(), "current".into());
        let mut control = report("old", 1);
        control.bridge_unavailable = true;
        assert!(routes.admit(control.clone()).is_err());
        control.generation = "current".into();
        control.caller_thread_id = Some("child".into());
        assert!(routes.admit(control.clone()).is_err());
        control.caller_thread_id = Some("root".into());
        for number in 0..MAX_QUEUE_REPORTS {
            drop(routes.admit(report("current", number)).unwrap());
        }
        drop(routes.admit(control.clone()).unwrap());
        assert!(receiver
            .drain(|_| panic!("failed bridge consumed pending work"))
            .is_err());
        assert!(routes.admit(report("current", 1)).is_err());
        let mut replacement = routes.register(Runtime::Codex, "runner".into(), "new".into());
        assert!(routes.admit(control).is_err());
        drop(routes.admit(report("new", 1)).unwrap());
        let mut accepted = 0;
        replacement.drain(|_| accepted += 1).unwrap();
        assert_eq!(accepted, 1);
    }

    #[test]
    fn queue_count_and_byte_exhaustion_fail_the_bridge() {
        for large in [false, true] {
            let routes = Arc::new(HookRoutes::default());
            let mut receiver = routes.register(Runtime::Codex, "runner".into(), "one".into());
            let mut value = report("one", 1);
            if large {
                value.payload["prompt"] = json!("你".repeat(1024 * 1024));
            }
            let mut admitted = 0;
            while let Ok(receipt) = routes.admit(value.clone()) {
                drop(receipt);
                admitted += 1;
            }
            assert_eq!(admitted, if large { 5 } else { MAX_QUEUE_REPORTS });
            assert!(receiver.drain(|_| panic!("lost boundary")).is_err());
            assert!(routes.admit(report("one", 2)).is_err());
        }
    }

    #[test]
    fn child_first_missing_identity_wrong_runtime_and_oversize_do_not_mutate_queue() {
        let routes = Arc::new(HookRoutes::default());
        let mut receiver = routes.register(Runtime::Codex, "runner".into(), "one".into());
        for identity in [None, Some(""), Some("child")] {
            let mut value = report("one", 1);
            value.caller_thread_id = identity.map(str::to_owned);
            assert!(routes.admit(value).is_err());
        }
        let mut value = report("one", 1);
        value.runtime = Runtime::Pi;
        assert!(routes.admit(value).is_err());
        let mut value = report("one", 1);
        value.payload["prompt"] = json!("你".repeat(MAX_ENVELOPE_BYTES / 3 + 1));
        assert!(routes.admit(value).is_err());
        receiver
            .drain(|_| panic!("invalid report admitted"))
            .unwrap();
        drop(routes.admit(report("one", 2)).unwrap());
        receiver.drain(|_| {}).unwrap();
    }
}
