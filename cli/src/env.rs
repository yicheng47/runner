// Resolve the four `RUNNER_*` env vars that locate the caller in the
// coordination bus. Mission sessions set all four (see
// `SessionManager::spawn`); agent direct chats set only RUNNER_HANDLE,
// while terminals and ordinary shells set none. Anything else is a bug.
//
// We don't read any other env. PATH manipulation, signal handling, etc.
// are out of scope — the parent process owns those.

use std::path::PathBuf;

const VAR_CREW: &str = "RUNNER_CREW_ID";
const VAR_MISSION: &str = "RUNNER_MISSION_ID";
const VAR_HANDLE: &str = "RUNNER_HANDLE";
const VAR_LOG: &str = "RUNNER_EVENT_LOG";

/// Per-process env after presence resolution.
pub enum BusContext {
    /// All four vars present — proceed normally.
    Mission(MissionEnv),
    /// None set, or only a direct-chat handle set — off the mission bus.
    OffBus,
    /// Some-but-not-all set — the parent process is buggy. Bail with a
    /// pointer at the missing names.
    Partial { missing: Vec<&'static str> },
}

#[derive(Clone, Debug)]
pub struct MissionEnv {
    pub crew_id: String,
    pub mission_id: String,
    pub handle: String,
    pub event_log: PathBuf,
}

pub fn resolve() -> BusContext {
    resolve_from(|name| std::env::var(name).ok())
}

/// Test-friendly variant. Production goes through `resolve()`; tests
/// inject their own lookup so they don't have to mutate the live env.
pub fn resolve_from<F: Fn(&str) -> Option<String>>(get: F) -> BusContext {
    match (
        get(VAR_CREW),
        get(VAR_MISSION),
        get(VAR_HANDLE),
        get(VAR_LOG),
    ) {
        (None, None, None, None) | (None, None, Some(_), None) => BusContext::OffBus,
        (Some(crew_id), Some(mission_id), Some(handle), Some(event_log)) => {
            BusContext::Mission(MissionEnv {
                crew_id,
                mission_id,
                handle,
                event_log: PathBuf::from(event_log),
            })
        }
        (crew, mission, handle, log) => {
            let missing = [
                (VAR_CREW, crew.is_none()),
                (VAR_MISSION, mission.is_none()),
                (VAR_HANDLE, handle.is_none()),
                (VAR_LOG, log.is_none()),
            ]
            .into_iter()
            .filter_map(|(name, missing)| missing.then_some(name))
            .collect();
            BusContext::Partial { missing }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(map: HashMap<&'static str, &'static str>) -> impl Fn(&str) -> Option<String> {
        move |name| map.get(name).map(|s| s.to_string())
    }

    #[test]
    fn all_four_present_resolves_mission() {
        let map = HashMap::from([
            (VAR_CREW, "c"),
            (VAR_MISSION, "m"),
            (VAR_HANDLE, "impl"),
            (VAR_LOG, "/tmp/events.ndjson"),
        ]);
        let BusContext::Mission(env) = resolve_from(lookup(map)) else {
            panic!("expected Mission");
        };
        assert_eq!(env.crew_id, "c");
        assert_eq!(env.mission_id, "m");
        assert_eq!(env.handle, "impl");
        assert_eq!(env.event_log, PathBuf::from("/tmp/events.ndjson"));
    }

    #[test]
    fn none_set_is_off_bus() {
        let map = HashMap::new();
        assert!(matches!(resolve_from(lookup(map)), BusContext::OffBus));
    }

    #[test]
    fn direct_chat_handle_only_is_off_bus() {
        let map = HashMap::from([(VAR_HANDLE, "coder")]);
        assert!(matches!(resolve_from(lookup(map)), BusContext::OffBus));
    }

    #[test]
    fn partial_set_lists_missing_vars() {
        let map = HashMap::from([(VAR_CREW, "c"), (VAR_MISSION, "m")]);
        let BusContext::Partial { missing } = resolve_from(lookup(map)) else {
            panic!("expected Partial");
        };
        assert!(missing.contains(&VAR_HANDLE));
        assert!(missing.contains(&VAR_LOG));
        assert_eq!(missing.len(), 2);
    }
}
