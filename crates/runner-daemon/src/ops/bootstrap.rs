use crate::AppCore;
use runner_core::protocol::AppBootstrapSnapshot;

/// Everything the app store fetches while starting, in one request
/// instead of a serial burst of individual calls over the socket.
pub fn snapshot(state: &AppCore) -> crate::error::Result<AppBootstrapSnapshot> {
    let crews = {
        let conn = state.db.get()?;
        crate::ops::crew::list(&conn)?
    };
    Ok(AppBootstrapSnapshot {
        sessions: crate::ops::session::session_list_recent_direct(state)?,
        session_details: crate::ops::session::session_details(state)?,
        roles: crate::ops::role::role_list(state)?,
        crews,
        nodes: crate::ops::node::node_list(state)?,
        projects: crate::ops::project::project_list(state)?,
        missions: crate::daemon::block_on(crate::ops::mission::mission_list_summary_impl(
            state, None,
        ))?,
        live_session_counts: crate::ops::session::live_session_counts(state)?,
        activity: crate::ops::session::session_activity_snapshot(state),
        statuses: crate::ops::session::session_status_snapshot(state)?,
    })
}
