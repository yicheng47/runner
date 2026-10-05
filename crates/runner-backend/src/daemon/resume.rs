use crate::{ops, repo, AppCore};
use runner_core::protocol::AutoResumeReport;
use std::time::Duration;
const AUTO_RESUME_STAGGER_MS: u64 = 300;
pub fn consume_resume_on_launch(
    core: &AppCore,
    enabled: bool,
    dims_for: impl Fn(&str) -> Option<(u16, u16)>,
) -> crate::error::Result<AutoResumeReport> {
    let drawer_session_ids = {
        let conn = core.db.get().map_err(|error| {
            crate::error::Error::msg(format!("get launch-resume connection: {error}"))
        })?;
        repo::node::list(&conn)
            .map_err(|error| {
                crate::error::Error::msg(format!("list launch-resume node layouts: {error}"))
            })?
            .into_iter()
            .filter(|row| {
                matches!(
                    row.node_type,
                    repo::node::NodeType::Tab | repo::node::NodeType::Mission
                )
            })
            .flat_map(|row| repo::node::drawer_session_ids(&row))
            .collect()
    };
    consume_launch_claims(
        enabled,
        || {
            let conn = core.db.get().map_err(|error| {
                crate::error::Error::msg(format!("get launch-resume connection: {error}"))
            })?;
            repo::session::clear_chat_resume_on_launch(&conn).map_err(|error| {
                crate::error::Error::msg(format!("clear chat launch-resume claims: {error}"))
            })?;
            Ok(())
        },
        || {
            let mut conn = core.db.get().map_err(|error| {
                crate::error::Error::msg(format!("get launch-resume connection: {error}"))
            })?;
            repo::session::take_resume_on_launch_excluding(&mut conn, &drawer_session_ids).map_err(
                |error| crate::error::Error::msg(format!("take launch-resume claim: {error}")),
            )
        },
        |session_id| {
            let dims = dims_for(session_id);
            ops::session::session_resume_on_launch(
                core,
                session_id,
                dims.map(|size| size.0),
                dims.map(|size| size.1),
            )
            .map(drop)
            .map_err(|error| error.to_string())
        },
        || std::thread::sleep(Duration::from_millis(AUTO_RESUME_STAGGER_MS)),
    )
}

pub fn consume_launch_claims(
    enabled: bool,
    mut clear: impl FnMut() -> crate::error::Result<()>,
    mut take: impl FnMut() -> crate::error::Result<Option<repo::session::ResumeOnLaunchClaim>>,
    mut resume: impl FnMut(&str) -> std::result::Result<(), String>,
    mut wait: impl FnMut(),
) -> crate::error::Result<AutoResumeReport> {
    if !enabled {
        clear()?;
    }

    let mut report = AutoResumeReport::default();
    let mut attempted_chat = false;
    while let Some(claim) = take()? {
        if attempted_chat && !claim.shell {
            wait();
        }
        attempted_chat |= !claim.shell;
        match resume(&claim.session_id) {
            Ok(()) => report.resumed.push(claim.session_id),
            Err(error) => report.errors.push(format!("{}: {error}", claim.session_id)),
        }
    }
    Ok(report)
}
