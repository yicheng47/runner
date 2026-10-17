// Read the per-mission roster sidecar that `mission_start` writes to
// `$APPDATA/runner/crews/{crew_id}/missions/{mission_id}/roster.json`.
// The CLI uses it to validate `runner msg post --to <handle>` (I2.4).
//
// Per-mission, not per-crew, so historical missions validate `--to`
// against the roster frozen at mission_start — even if crew membership
// later changes.

use std::path::{Path, PathBuf};

use runner_core::mission_roster::{MissionRoster, RosterSlot};

/// Locate the sidecar from the events-log path. The roster.json sits in
/// the same mission directory as events.ndjson.
pub fn sidecar_path(event_log: &Path) -> Option<PathBuf> {
    let mission_dir = event_log.parent()?;
    Some(mission_dir.join("roster.json"))
}

pub fn load(event_log: &Path) -> Option<Vec<RosterSlot>> {
    let path = sidecar_path(event_log)?;
    let bytes = std::fs::read(&path).ok()?;
    serde_json::from_slice::<MissionRoster>(&bytes)
        .ok()
        .map(|roster| roster.slots)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sidecar_path_is_in_same_dir_as_events() {
        let log = Path::new("/data/runner/crews/C/missions/M/events.ndjson");
        assert_eq!(
            sidecar_path(log).unwrap(),
            PathBuf::from("/data/runner/crews/C/missions/M/roster.json"),
        );
    }

    #[test]
    fn load_parses_lead_flag() {
        let dir = tempfile::tempdir().unwrap();
        let mission = dir.path().join("crews/C/missions/M");
        std::fs::create_dir_all(&mission).unwrap();
        std::fs::write(
            mission.join("roster.json"),
            br#"{"version":2,"crew_name":"Pair","crew_addendum":null,"slots":[{"id":"s1","role_id":"r1","position":0,"handle":"lead","lead":true},{"id":"s2","role_id":"r2","position":1,"handle":"impl","lead":false}]}"#,
        )
        .unwrap();
        let log = mission.join("events.ndjson");
        let r = load(&log).unwrap();
        assert_eq!(r.len(), 2);
        assert!(r.iter().any(|e| e.handle == "lead" && e.lead));
        assert!(r.iter().any(|e| e.handle == "impl" && !e.lead));
    }
}

#[cfg(test)]
mod version_tests {
    use super::*;

    #[test]
    fn only_version_two_is_loaded() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("events.ndjson");
        let path = dir.path().join("roster.json");
        for bytes in [
            br#"[{"handle":"lead","lead":true}]"#.as_slice(),
            br#"{"version":3,"slots":[]}"#.as_slice(),
            b"invalid",
        ] {
            std::fs::write(&path, bytes).unwrap();
            assert!(load(&log).is_none());
        }
        std::fs::write(&path, br#"{"version":2,"crew_name":"Pair","crew_addendum":null,"slots":[{"id":"s","handle":"original","lead":true,"position":0,"role_id":"r"}]}"#).unwrap();
        assert_eq!(load(&log).unwrap()[0].handle, "original");
    }
}
