// `runner signal <type> [--payload <json>]` appends a single
// `EventDraft::signal` line to the mission's NDJSON event log via
// `runner_core::event_log::EventLog`.
//
// Per arch §5.2, signals always carry `to: null`; per-target routing
// lives in `payload.target` (only `human_said` uses this in v0). The CLI
// preserves this — `--to` is intentionally not exposed for `signal`.

use runner_core::event_log::EventLog;
use runner_core::model::{EventDraft, EventKind, KnownSignalType, SignalType};

use crate::env;

fn known_types_csv() -> String {
    KnownSignalType::ALL
        .iter()
        .map(|k| k.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn run(env: &env::MissionEnv, ty: &str, payload: Option<&str>) -> i32 {
    let Some(kind) = KnownSignalType::from_name(ty) else {
        eprintln!(
            "runner signal: unknown type {ty:?}. Known types: {}.",
            known_types_csv(),
        );
        return 2;
    };

    let payload_value = match parse_payload(payload) {
        Ok(value) => value,
        Err(message) => {
            eprintln!("{message}");
            return 2;
        }
    };

    append(env, kind.as_str(), payload_value)
}

pub fn parse_payload(payload: Option<&str>) -> Result<serde_json::Value, String> {
    let value = match payload {
        Some(value) => serde_json::from_str(value)
            .map_err(|error| format!("runner signal: --payload is not valid JSON: {error}"))?,
        None => serde_json::json!({}),
    };
    if !value.is_object() {
        return Err("runner signal: --payload must be a JSON object".to_owned());
    }
    Ok(value)
}

pub fn append(env: &env::MissionEnv, ty: &str, payload: serde_json::Value) -> i32 {
    // `EventLog::open` recreates the dir if needed; here it just resolves
    // the existing mission_dir. The flock + ULID floor logic lives there.
    let Some(mission_dir) = env.event_log.parent() else {
        eprintln!(
            "runner: RUNNER_EVENT_LOG has no parent directory: {}",
            env.event_log.display()
        );
        return 2;
    };
    let log = match EventLog::open(mission_dir) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("runner: failed to open event log: {e}");
            return 1;
        }
    };

    let draft = EventDraft {
        crew_id: env.crew_id.clone(),
        mission_id: env.mission_id.clone(),
        kind: EventKind::Signal,
        from: env.handle.clone(),
        to: None,
        signal_type: Some(SignalType::new(ty)),
        payload,
    };
    match log.append(draft) {
        Ok(_ev) => 0,
        Err(e) => {
            eprintln!("runner: append failed: {e}");
            1
        }
    }
}
