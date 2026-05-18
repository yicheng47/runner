// PTY host protocol — shared wire types for the Tauri app and the
// `runner-pty-host` sidecar binary.
//
// Architecture: docs/impls/0011-pty-host-terminal-runtime.md (Step 1).
//
// The app sends `HostRequest` framed messages over a local Unix socket;
// the sidecar replies with `HostResponse` and emits `HostEvent` pushes on
// subscribed sessions. Per-message framing is the responsibility of the
// transport layer (Step 2's IPC loop) — these types only define the
// payload shapes.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// --- SpawnSpecWire -------------------------------------------------------
//
// Mirrors `SpawnSpec` from `src-tauri/src/session/runtime.rs` for the
// host-side launcher. Kept narrow on purpose: anything the host can't act
// on directly (DB ids, mission router state) does not belong on the wire.

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpawnSpecWire {
    /// Pre-resolved binary or shell path. The host invokes it verbatim; PATH
    /// resolution is the app's responsibility (`session::launch`).
    pub command: String,
    pub args: Vec<String>,
    /// Working directory for the child. None = inherit the host's cwd, which
    /// after `--detach` is `/`.
    pub cwd: Option<String>,
    /// Filtered environment. The host does NOT inherit its own env into the
    /// child beyond what's in this map — keep `session::launch`'s filter as
    /// the single source of truth.
    pub env: BTreeMap<String, String>,
    /// Initial PTY geometry. SIGWINCH after spawn goes through `Resize`.
    pub cols: u16,
    pub rows: u16,
}

// --- TerminalReplayEvent -------------------------------------------------
//
// The on-the-wire form of a terminal event, carried both on the live
// `HostEvent::Output` path (raw PTY bytes) and inside `HostSnapshot.events`
// on attach (synthetic `screen_to_ansi` bytes). See plan §"Resize-stack fix
// mechanism" for why the carrier matters.

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TerminalReplayEvent {
    Output {
        seq: u64,
        /// base64 of a terminal byte stream the frontend should write
        /// verbatim into xterm. Origin varies by carrier:
        ///   - on `HostEvent::Output` (live path): raw PTY bytes
        ///     forwarded from the child;
        ///   - on `HostSnapshot.events[0]` (attach path): synthetic
        ///     bytes produced by the host's `screen_to_ansi` serializer
        ///     over the headless `Terminal`'s current screen. See
        ///     plan Step 5's snapshot semantics.
        data: String,
    },
    Resize {
        seq: u64,
        cols: u16,
        rows: u16,
    },
}

// --- HostRequest --------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum HostRequest {
    Spawn {
        spec: SpawnSpecWire,
    },
    Attach {
        session_id: String,
    },
    Input {
        session_id: String,
        /// base64-encoded raw input bytes.
        data_base64: String,
    },
    Paste {
        session_id: String,
        /// base64-encoded payload to deliver under bracketed-paste semantics.
        data_base64: String,
    },
    Key {
        session_id: String,
        /// Symbolic key name (e.g. "enter", "escape"). The host owns the
        /// xterm.js → bytes translation table.
        key: String,
    },
    Resize {
        session_id: String,
        cols: u16,
        rows: u16,
    },
    Stop {
        session_id: String,
    },
    Status {
        session_id: String,
    },
    List,
}

// --- HostResponse -------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostResponse {
    /// Generic ack for fire-and-forget requests (`Input`, `Paste`, `Key`,
    /// `Resize`, `Stop`).
    Ack,
    /// Request failed. `message` is human-readable; programmatic callers
    /// route on the request that triggered it. Step 2's sidecar emits this
    /// for any session-bearing request (sessions land in Step 3).
    Error {
        message: String,
    },
    /// Reply to `Spawn`. The session id is host-assigned (ULID).
    Spawned {
        session_id: String,
        pid: u32,
    },
    /// Reply to `Status`.
    SessionStatus(HostSessionStatus),
    /// Reply to `Attach`. The snapshot wraps the host's `screen_to_ansi`
    /// serialization plus pre-resize geometry — see plan §"Snapshot
    /// semantics (v1)".
    Snapshot(HostSnapshot),
    /// Reply to `List`. One entry per session the host currently owns.
    Sessions {
        sessions: Vec<HostSessionStatus>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostSessionStatus {
    pub session_id: String,
    pub alive: bool,
    pub exit_code: Option<i32>,
    pub pid: Option<u32>,
    /// `command [args...]` summary for diagnostics. Not parsed by the app.
    pub command: String,
    pub cols: u16,
    pub rows: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostSnapshot {
    /// v1: exactly one `TerminalReplayEvent::Output` carrying the host's
    /// `screen_to_ansi` serialization of the current headless terminal
    /// state. Future versions may include preceding `Resize` entries; the
    /// frontend must tolerate that shape but pre-size from the top-level
    /// `cols`/`rows` (plan Step 5, replay step 4).
    pub events: Vec<TerminalReplayEvent>,
    /// Sequence number of the last live event observed at snapshot time.
    /// The frontend resumes the live stream from `last_seq + 1`.
    pub last_seq: u64,
    /// Geometry of the host-side headless terminal at snapshot time. The
    /// frontend resizes xterm to these dims *before* writing
    /// `events[0]`'s bytes.
    pub cols: u16,
    pub rows: u16,
}

// --- HostMessage --------------------------------------------------------
//
// On-the-wire envelope. Every frame is either a request-correlated
// `Response` or a push `Event`. The discriminator lets the Tauri side
// route without having to disambiguate two `kind`-tagged inner enums
// whose tag values happen not to collide today (they did when the
// protocol was sketched and may again).

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum HostMessage {
    Response(HostResponse),
    Event(HostEvent),
}

// --- HostEvent ----------------------------------------------------------
//
// Push events the sidecar emits to subscribed app clients. `seq` is
// session-scoped and monotonic; the same numbers appear in
// `HostSnapshot.last_seq` so the merge in Step 5's replay algorithm
// doesn't duplicate or drop.

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostEvent {
    Output {
        session_id: String,
        seq: u64,
        /// base64 of the raw PTY bytes. The frontend writes them verbatim
        /// to xterm without parsing.
        data: String,
    },
    Resize {
        session_id: String,
        seq: u64,
        cols: u16,
        rows: u16,
    },
    Exit {
        session_id: String,
        seq: u64,
        exit_code: Option<i32>,
    },
    /// Busy/idle and similar advisory bits the app currently reads off the
    /// tmux runtime. Kept abstract here — the producer side lands with
    /// Step 3.
    RunnerStatus {
        session_id: String,
        seq: u64,
        busy: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> SpawnSpecWire {
        let mut env = BTreeMap::new();
        env.insert("PATH".into(), "/usr/bin".into());
        SpawnSpecWire {
            command: "/bin/cat".into(),
            args: vec!["-u".into()],
            cwd: Some("/tmp".into()),
            env,
            cols: 80,
            rows: 24,
        }
    }

    fn roundtrip<T: Serialize + for<'a> Deserialize<'a> + PartialEq + std::fmt::Debug>(
        value: &T,
    ) {
        let json = serde_json::to_string(value).expect("serialize");
        let back: T = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(value, &back, "round-trip mismatch via {json}");
    }

    #[test]
    fn spawn_request_roundtrips() {
        roundtrip(&HostRequest::Spawn { spec: spec() });
    }

    #[test]
    fn input_request_roundtrips() {
        roundtrip(&HostRequest::Input {
            session_id: "01HSESS".into(),
            data_base64: "aGVsbG8=".into(),
        });
    }

    #[test]
    fn resize_request_roundtrips() {
        roundtrip(&HostRequest::Resize {
            session_id: "01HSESS".into(),
            cols: 120,
            rows: 40,
        });
    }

    #[test]
    fn key_request_roundtrips() {
        roundtrip(&HostRequest::Key {
            session_id: "01HSESS".into(),
            key: "enter".into(),
        });
    }

    #[test]
    fn list_request_roundtrips() {
        roundtrip(&HostRequest::List);
    }

    #[test]
    fn ack_response_roundtrips() {
        roundtrip(&HostResponse::Ack);
    }

    #[test]
    fn error_response_roundtrips() {
        roundtrip(&HostResponse::Error {
            message: "unimplemented".into(),
        });
    }

    #[test]
    fn spawned_response_roundtrips() {
        roundtrip(&HostResponse::Spawned {
            session_id: "01HSESS".into(),
            pid: 4242,
        });
    }

    #[test]
    fn snapshot_response_roundtrips() {
        roundtrip(&HostResponse::Snapshot(HostSnapshot {
            events: vec![TerminalReplayEvent::Output {
                seq: 7,
                data: "ZGF0YQ==".into(),
            }],
            last_seq: 7,
            cols: 100,
            rows: 30,
        }));
    }

    #[test]
    fn session_status_response_roundtrips() {
        roundtrip(&HostResponse::SessionStatus(HostSessionStatus {
            session_id: "01HSESS".into(),
            alive: true,
            exit_code: None,
            pid: Some(4242),
            command: "/bin/cat -u".into(),
            cols: 80,
            rows: 24,
        }));
    }

    #[test]
    fn sessions_list_response_roundtrips() {
        roundtrip(&HostResponse::Sessions {
            sessions: vec![HostSessionStatus {
                session_id: "01HSESS".into(),
                alive: false,
                exit_code: Some(0),
                pid: None,
                command: "/bin/cat".into(),
                cols: 80,
                rows: 24,
            }],
        });
    }

    #[test]
    fn output_event_roundtrips() {
        roundtrip(&HostEvent::Output {
            session_id: "01HSESS".into(),
            seq: 1,
            data: "ZGF0YQ==".into(),
        });
    }

    #[test]
    fn resize_event_roundtrips() {
        roundtrip(&HostEvent::Resize {
            session_id: "01HSESS".into(),
            seq: 2,
            cols: 120,
            rows: 40,
        });
    }

    #[test]
    fn exit_event_roundtrips() {
        roundtrip(&HostEvent::Exit {
            session_id: "01HSESS".into(),
            seq: 3,
            exit_code: Some(0),
        });
    }

    #[test]
    fn runner_status_event_roundtrips() {
        roundtrip(&HostEvent::RunnerStatus {
            session_id: "01HSESS".into(),
            seq: 4,
            busy: true,
        });
    }

    #[test]
    fn replay_event_output_tag_is_snake_case() {
        let v = serde_json::to_value(&TerminalReplayEvent::Output {
            seq: 0,
            data: String::new(),
        })
        .unwrap();
        assert_eq!(v["kind"], "output");
    }

    #[test]
    fn replay_event_resize_tag_is_snake_case() {
        let v = serde_json::to_value(&TerminalReplayEvent::Resize {
            seq: 0,
            cols: 80,
            rows: 24,
        })
        .unwrap();
        assert_eq!(v["kind"], "resize");
    }

    #[test]
    fn host_request_uses_op_discriminator() {
        let v = serde_json::to_value(&HostRequest::List).unwrap();
        assert_eq!(v["op"], "list");
    }

    #[test]
    fn host_response_uses_kind_discriminator() {
        let v = serde_json::to_value(&HostResponse::Ack).unwrap();
        assert_eq!(v["kind"], "ack");
    }

    #[test]
    fn host_message_wraps_response() {
        let msg = HostMessage::Response(HostResponse::Ack);
        roundtrip(&msg);
        let v = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["type"], "response");
    }

    #[test]
    fn host_message_wraps_event() {
        let msg = HostMessage::Event(HostEvent::Exit {
            session_id: "01HSESS".into(),
            seq: 5,
            exit_code: Some(0),
        });
        roundtrip(&msg);
        let v = serde_json::to_value(&msg).unwrap();
        assert_eq!(v["type"], "event");
    }
}
