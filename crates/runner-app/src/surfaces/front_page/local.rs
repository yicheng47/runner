use runner_core::protocol::{ClientResult, DaemonClient};

pub(super) fn resume_mission_with_options(
    daemon: &DaemonClient,
    mission: &str,
    size: (u16, u16),
    restore_in_main: bool,
) -> ClientResult<super::client::MissionResume> {
    use runner_core::protocol::model::SessionStatus;
    let mut sessions = daemon.session_list(mission)?;
    let mut first_error = None;
    let mut resumed = Vec::new();
    let mut pending_restore_in_main = restore_in_main;
    for candidate in sessions.clone() {
        let already_running = sessions
            .iter()
            .find(|session| session.session.id == candidate.session.id)
            .is_some_and(|session| session.session.status == SessionStatus::Running);
        if already_running {
            continue;
        }
        match daemon.session_resume(
            &candidate.session.id,
            Some(size.0),
            Some(size.1),
            pending_restore_in_main,
        ) {
            Ok(session) => {
                pending_restore_in_main = false;
                resumed.push(session.id);
            }
            Err(error) => {
                let message = error.to_string();
                if super::client::is_concurrent_resume_error(&message) {
                    match daemon.session_list(mission) {
                        Ok(refreshed) => sessions = refreshed,
                        Err(error) => {
                            first_error.get_or_insert_with(|| error.to_string());
                        }
                    }
                } else {
                    first_error.get_or_insert(message);
                }
            }
        }
    }
    match daemon.session_list(mission) {
        Ok(refreshed) => sessions = refreshed,
        Err(error) => {
            first_error.get_or_insert_with(|| error.to_string());
        }
    }
    Ok(super::client::MissionResume {
        sessions: resumed,
        local_sessions: sessions,
        error: first_error,
    })
}
