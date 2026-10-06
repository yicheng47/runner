use crate::{ops, repo, AppCore};
use runner_core::protocol::AutoResumeReport;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
const AUTO_RESUME_STAGGER_MS: u64 = 300;

#[derive(Default)]
pub struct ResumeConsumer {
    stopped: AtomicBool,
    active: Mutex<()>,
}
impl ResumeConsumer {
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        drop(self.active.lock().unwrap());
    }
}

pub fn consume_resume_on_launch(
    core: &AppCore,
    enabled: bool,
    dims_for: impl Fn(&str) -> Option<(u16, u16)>,
) -> crate::error::Result<AutoResumeReport> {
    consume_resume_on_launch_until(core, enabled, dims_for, &ResumeConsumer::default())
}

pub fn consume_resume_on_launch_until(
    core: &AppCore,
    enabled: bool,
    dims_for: impl Fn(&str) -> Option<(u16, u16)>,
    consumer: &ResumeConsumer,
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
    consume_launch_claims_until(
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
        consumer,
    )
}

pub fn consume_launch_claims(
    enabled: bool,
    clear: impl FnMut() -> crate::error::Result<()>,
    take: impl FnMut() -> crate::error::Result<Option<repo::session::ResumeOnLaunchClaim>>,
    resume: impl FnMut(&str) -> std::result::Result<(), String>,
    wait: impl FnMut(),
) -> crate::error::Result<AutoResumeReport> {
    consume_launch_claims_until(
        enabled,
        clear,
        take,
        resume,
        wait,
        &ResumeConsumer::default(),
    )
}

fn consume_launch_claims_until(
    enabled: bool,
    mut clear: impl FnMut() -> crate::error::Result<()>,
    mut take: impl FnMut() -> crate::error::Result<Option<repo::session::ResumeOnLaunchClaim>>,
    mut resume: impl FnMut(&str) -> std::result::Result<(), String>,
    mut wait: impl FnMut(),
    consumer: &ResumeConsumer,
) -> crate::error::Result<AutoResumeReport> {
    if !enabled {
        let _active = consumer.active.lock().unwrap();
        if consumer.stopped.load(Ordering::Acquire) {
            return Ok(AutoResumeReport::default());
        }
        clear()?;
    }

    let mut report = AutoResumeReport::default();
    let mut attempted_chat = false;
    loop {
        let _active = consumer.active.lock().unwrap();
        if consumer.stopped.load(Ordering::Acquire) {
            break;
        }
        let Some(claim) = take()? else { break };
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{mpsc, Arc};

    #[test]
    fn stop_quiesces_an_active_claim_and_prevents_another_take() {
        let consumer = Arc::new(ResumeConsumer::default());
        let (entered, active) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let worker_consumer = consumer.clone();
        let worker = std::thread::spawn(move || {
            let mut takes = 0;
            let report = consume_launch_claims_until(
                true,
                || Ok(()),
                || {
                    takes += 1;
                    Ok(Some(repo::session::ResumeOnLaunchClaim {
                        session_id: "active".into(),
                        shell: true,
                    }))
                },
                |_| {
                    entered.send(()).unwrap();
                    released.recv().unwrap();
                    Ok(())
                },
                || {},
                &worker_consumer,
            )
            .unwrap();
            (takes, report)
        });
        active.recv_timeout(Duration::from_secs(2)).unwrap();
        let stopped = consumer.clone();
        let (done, finished) = mpsc::channel();
        let stopper = std::thread::spawn(move || {
            stopped.stop();
            done.send(()).unwrap();
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !consumer.stopped.load(Ordering::Acquire) {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(matches!(
            finished.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
        release.send(()).unwrap();
        finished.recv_timeout(Duration::from_secs(2)).unwrap();
        stopper.join().unwrap();
        let (takes, report) = worker.join().unwrap();
        assert_eq!(takes, 1);
        assert_eq!(report.resumed, ["active"]);
    }
}
