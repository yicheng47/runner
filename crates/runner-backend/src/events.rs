// App-wide event fanout (impl 0031 Phase 2).
//
// A single `tokio::sync::broadcast` channel is the source of truth for
// every event a frontend can observe. Producers (command bodies, the
// session manager's forwarder threads, the per-mission event bus, MCP
// tools) send `AppEvent`s; each frontend holds one subscriber and forwards
// to its own surface — the Tauri layer re-emits every event to the webview
// under the same name with the same JSON payload, so webview-observable
// behavior is unchanged from the old direct `AppHandle::emit` calls.
//
// Payloads are converted to `serde_json::Value` at send time. Tauri's
// `emit` serialized payloads to JSON anyway, so this is the same wire
// shape with one intermediate representation.

use serde::Serialize;
use tokio::sync::broadcast;

/// One frontend-observable event. `name` is the channel the webview
/// subscribes to (`"session/output"`, `"chat/layout-changed"`, …); every
/// producer uses a static string so a typo is greppable.
#[derive(Debug, Clone)]
pub struct AppEvent {
    pub name: &'static str,
    pub payload: serde_json::Value,
}

/// Buffered events per subscriber. `session/output` is the high-rate
/// producer (coalesced PTY chunks); the buffer must absorb a burst while
/// the forwarder serializes into the webview. A lagged subscriber drops
/// oldest-first and logs — same failure mode as any bounded queue.
const CHANNEL_CAPACITY: usize = 8192;

/// Cheap-to-clone handle on the broadcast channel. Sending never blocks;
/// with no live subscriber the event is dropped, mirroring the old
/// `let _ = app.emit(..)` behavior against a not-yet-loaded webview.
#[derive(Clone)]
pub struct EventChannel {
    tx: broadcast::Sender<AppEvent>,
}

impl EventChannel {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(CHANNEL_CAPACITY);
        Self { tx }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.tx.subscribe()
    }

    pub fn emit<T: Serialize + ?Sized>(&self, name: &'static str, payload: &T) {
        let payload = match serde_json::to_value(payload) {
            Ok(v) => v,
            Err(e) => {
                log::error!("event {name}: payload serialization failed: {e}");
                return;
            }
        };
        let _ = self.tx.send(AppEvent { name, payload });
    }
}

impl Default for EventChannel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_without_subscribers_is_a_no_op() {
        let ch = EventChannel::new();
        ch.emit("test/event", &serde_json::json!({ "k": 1 }));
    }

    #[test]
    fn subscriber_receives_name_and_json_payload() {
        let ch = EventChannel::new();
        let mut rx = ch.subscribe();
        ch.emit("test/event", &serde_json::json!({ "k": 1 }));
        // Unit payloads serialize as null — the shape `app.emit(name, ())`
        // produced for parameterless invalidation pings.
        ch.emit("test/unit", &());

        let ev = rx.try_recv().unwrap();
        assert_eq!(ev.name, "test/event");
        assert_eq!(ev.payload, serde_json::json!({ "k": 1 }));
        let ev = rx.try_recv().unwrap();
        assert_eq!(ev.name, "test/unit");
        assert_eq!(ev.payload, serde_json::Value::Null);
    }
}
