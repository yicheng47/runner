use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Barrier};
use std::time::{Duration, Instant};

use super::*;
use crate::model::{MissionStatus, SessionStatus};
use crate::repo::mission::MissionRow;
use crate::test_support::{insert_session_row, test_core, test_session_row};

fn stamped_config(endpoint: String) -> TelemetryConfig {
    TelemetryConfig {
        dev_build: false,
        endpoint,
        version: "0.20.4".into(),
        build_stamp: "20261126.0220".into(),
        channel: "production".into(),
        os: "macos".into(),
        arch: "aarch64".into(),
        timeout: Duration::from_secs(2),
        interval: HEARTBEAT_INTERVAL,
        env: TelemetryEnvSource::Fixed(BTreeMap::new()),
        network_enabled: true,
    }
}

struct MockServer {
    addr: SocketAddr,
    endpoint: String,
    rx: mpsc::Receiver<String>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl MockServer {
    fn start(status_line: &'static str, delay: Duration) -> Self {
        Self::start_with_response(
            format!("HTTP/1.1 {status_line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
            delay,
            1,
        )
    }

    fn start_many(status_line: &'static str, delay: Duration, max_requests: usize) -> Self {
        Self::start_with_response(
            format!("HTTP/1.1 {status_line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
            delay,
            max_requests,
        )
    }

    fn start_redirect(location: &str) -> Self {
        Self::start_with_response(
            format!(
                "HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            ),
            Duration::ZERO,
            1,
        )
    }

    fn start_with_response(response: String, delay: Duration, max_requests: usize) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            for _ in 0..max_requests {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                if stop_flag.load(Ordering::Acquire) {
                    return;
                }
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                let mut content_length = None;
                let mut header_end = None;
                loop {
                    match stream.read(&mut chunk) {
                        Ok(0) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&chunk[..n]);
                            if header_end.is_none() {
                                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                    header_end = Some(pos + 4);
                                    let headers = String::from_utf8_lossy(&buf[..pos]);
                                    for line in headers.lines() {
                                        if let Some((k, v)) = line.split_once(':') {
                                            if k.trim().eq_ignore_ascii_case("content-length") {
                                                content_length = v.trim().parse::<usize>().ok();
                                            }
                                        }
                                    }
                                }
                            }
                            if let Some(end) = header_end {
                                let expected = content_length.unwrap_or(0);
                                if buf.len() >= end + expected {
                                    break;
                                }
                            }
                        }
                        Err(_) => break,
                    }
                }
                let body = header_end
                    .map(|end| String::from_utf8_lossy(&buf[end..]).into_owned())
                    .unwrap_or_default();
                let _ = tx.send(body);
                if !delay.is_zero() {
                    let deadline = Instant::now() + delay;
                    while Instant::now() < deadline && !stop_flag.load(Ordering::Acquire) {
                        thread::sleep(Duration::from_millis(10));
                    }
                }
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Self {
            addr,
            endpoint: format!("http://{addr}/api/heartbeat"),
            rx,
            stop,
            handle: Some(handle),
        }
    }
}

impl Drop for MockServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            if !handle.is_finished() {
                let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(100));
            }
            let _ = handle.join();
        }
    }
}

fn insert_mission(
    conn: &Connection,
    id: &str,
    started_at: DateTime<Utc>,
    stopped_at: Option<DateTime<Utc>>,
) {
    crate::repo::mission::insert(
        conn,
        &MissionRow {
            id: id.into(),
            crew_id: None,
            project_id: None,
            title: id.into(),
            status: if stopped_at.is_some() {
                MissionStatus::Completed
            } else {
                MissionStatus::Running
            },
            goal_override: None,
            cwd: None,
            started_at,
            stopped_at,
            pinned_at: None,
            archived_at: None,
            linked_session_id: None,
            report_to_chat: false,
        },
    )
    .unwrap();
}

#[test]
fn unstamped_dev_build_never_sends_to_production_and_reports_dev_build() {
    let mut core = test_core();
    crate::db::mark_onboarding_completed(&core.db).unwrap();
    core.telemetry = Arc::new(TelemetryService::with_config(TelemetryConfig {
        dev_build: true,
        endpoint: DEFAULT_HEARTBEAT_ENDPOINT.into(),
        env: TelemetryEnvSource::Fixed(BTreeMap::new()),
        network_enabled: true,
        ..TelemetryConfig::default()
    }));

    let status = core.telemetry.status(&core).unwrap();
    assert!(status.dev_build);
    assert!(status.enabled);
    assert_eq!(status.endpoint, DEFAULT_HEARTBEAT_ENDPOINT);
    assert_eq!(
        core.telemetry.try_send_once(&core, Utc::now()),
        SendOutcome::SkippedDevBuild
    );
    assert_eq!(
        app_state_get(&core.db.get().unwrap(), TELEMETRY_LAST_SENT_AT_KEY).unwrap(),
        None
    );
}

#[test]
fn env_overrides_lock_telemetry_off_without_creating_install_id() {
    for (key, value, expected) in [
        ("DO_NOT_TRACK", "1", "DO_NOT_TRACK=1"),
        ("DO_NOT_TRACK", "true", "DO_NOT_TRACK=true"),
        ("RUNNER_TELEMETRY", "0", "RUNNER_TELEMETRY=0"),
        ("RUNNER_TELEMETRY", "false", "RUNNER_TELEMETRY=false"),
    ] {
        let mut core = test_core();
        crate::db::mark_onboarding_completed(&core.db).unwrap();
        let env = BTreeMap::from([(key.to_owned(), value.to_owned())]);
        core.telemetry = Arc::new(TelemetryService::with_config(TelemetryConfig {
            env: TelemetryEnvSource::Fixed(env),
            ..stamped_config("http://127.0.0.1:1/api/heartbeat".into())
        }));

        let status = core.telemetry.status(&core).unwrap();
        assert!(!status.enabled, "{key}={value}");
        assert!(status.stored_enabled, "{key}={value}");
        assert_eq!(status.env_override.as_deref(), Some(expected));
        assert_eq!(status.install_id, None);
        assert_eq!(
            app_state_get(&core.db.get().unwrap(), TELEMETRY_INSTALL_ID_KEY).unwrap(),
            None
        );
        assert_eq!(
            core.telemetry.try_send_once(&core, Utc::now()),
            SendOutcome::SkippedEnvOverride
        );

        let after_reset = core.telemetry.reset_id(&core).unwrap();
        assert_eq!(after_reset.install_id, None);
    }

    // Non-matching values leave telemetry enabled.
    let mut core = test_core();
    let env = BTreeMap::from([
        ("DO_NOT_TRACK".into(), "0".into()),
        ("RUNNER_TELEMETRY".into(), "1".into()),
    ]);
    core.telemetry = Arc::new(TelemetryService::with_config(TelemetryConfig {
        env: TelemetryEnvSource::Fixed(env),
        ..stamped_config("http://127.0.0.1:1/api/heartbeat".into())
    }));
    let status = core.telemetry.status(&core).unwrap();
    assert!(status.enabled);
    assert_eq!(status.env_override, None);
    assert!(status
        .install_id
        .as_deref()
        .is_some_and(is_valid_install_id));
}

#[test]
fn opt_out_clears_install_id_and_reset_rotates_uuid_v4() {
    let mut core = test_core();
    core.telemetry = Arc::new(TelemetryService::with_config(stamped_config(
        DEFAULT_HEARTBEAT_ENDPOINT.into(),
    )));

    let initial = core.telemetry.status(&core).unwrap();
    assert!(initial.enabled);
    let first_id = initial.install_id.clone().expect("install id generated");
    assert!(is_valid_install_id(&first_id));
    assert!(initial.payload_preview.contains(&first_id));

    let rotated = core.telemetry.reset_id(&core).unwrap();
    let second_id = rotated.install_id.clone().expect("rotated install id");
    assert!(is_valid_install_id(&second_id));
    assert_ne!(first_id, second_id);
    assert!(rotated.payload_preview.contains(&second_id));

    let disabled = core.telemetry.set_enabled(&core, false).unwrap();
    assert!(!disabled.enabled);
    assert!(!disabled.stored_enabled);
    assert_eq!(disabled.install_id, None);
    assert_eq!(
        app_state_get(&core.db.get().unwrap(), TELEMETRY_INSTALL_ID_KEY).unwrap(),
        None
    );

    // Reset ID while disabled is a no-op and does not store an ID.
    let still_disabled = core.telemetry.reset_id(&core).unwrap();
    assert_eq!(still_disabled.install_id, None);
    assert_eq!(
        app_state_get(&core.db.get().unwrap(), TELEMETRY_INSTALL_ID_KEY).unwrap(),
        None
    );

    // Re-enabling generates a fresh UUID distinct from the cleared one.
    let reenabled = core.telemetry.set_enabled(&core, true).unwrap();
    assert!(reenabled.enabled);
    let third_id = reenabled.install_id.expect("fresh install id");
    assert!(is_valid_install_id(&third_id));
    assert_ne!(second_id, third_id);
}

#[test]
fn payload_includes_allowlisted_7d_runtimes_and_requires_recent_multi_agent_mission() {
    let mut core = test_core();
    core.telemetry = Arc::new(TelemetryService::with_config(stamped_config(
        DEFAULT_HEARTBEAT_ENDPOINT.into(),
    )));

    let now: DateTime<Utc> = "2026-11-26T12:00:00Z".parse().unwrap();
    {
        let conn = core.db.get().unwrap();
        let mut recent_codex = test_session_row("s-codex", SessionStatus::Stopped);
        recent_codex.agent_runtime = Some("codex".into());
        recent_codex.started_at = Some(now - TimeDelta::days(2));
        insert_session_row(&conn, &recent_codex).unwrap();

        let mut recent_claude = test_session_row("s-claude", SessionStatus::Stopped);
        recent_claude.agent_runtime = Some("claude-code".into());
        recent_claude.started_at = Some(now - TimeDelta::days(6));
        insert_session_row(&conn, &recent_claude).unwrap();

        // Excluded: shell, unknown runtime, and >7d stopped session.
        let mut shell = test_session_row("s-shell", SessionStatus::Running);
        shell.agent_runtime = Some("shell".into());
        shell.started_at = Some(now - TimeDelta::hours(1));
        insert_session_row(&conn, &shell).unwrap();

        let mut unknown = test_session_row("s-unknown", SessionStatus::Stopped);
        unknown.agent_runtime = Some("custom-internal-agent".into());
        unknown.started_at = Some(now - TimeDelta::hours(1));
        insert_session_row(&conn, &unknown).unwrap();

        let mut old_pi = test_session_row("s-old-pi", SessionStatus::Stopped);
        old_pi.agent_runtime = Some("pi".into());
        old_pi.started_at = Some(now - TimeDelta::days(10));
        old_pi.stopped_at = Some(now - TimeDelta::days(9));
        insert_session_row(&conn, &old_pi).unwrap();

        // 1. Empty recent mission -> NOT a multi-agent mission.
        insert_mission(
            &conn,
            "m-empty",
            now - TimeDelta::days(1),
            Some(now - TimeDelta::days(1)),
        );

        // 2. Single-agent mission (even with a restarted session on the same slot + a shell drawer) -> false.
        insert_mission(
            &conn,
            "m-single",
            now - TimeDelta::days(1),
            Some(now - TimeDelta::days(1)),
        );
        for (sid, slot, rt) in [
            ("s-m-single-1", Some("slot-coder"), "claude-code"),
            ("s-m-single-2", Some("slot-coder"), "claude-code"),
            ("s-m-single-shell", Some("slot-shell"), "shell"),
        ] {
            let mut row = test_session_row(sid, SessionStatus::Stopped);
            row.mission_id = Some("m-single".into());
            row.slot_id = slot.map(str::to_owned);
            row.agent_runtime = Some(rt.into());
            row.started_at = Some(now - TimeDelta::days(1));
            row.stopped_at = Some(now - TimeDelta::hours(20));
            insert_session_row(&conn, &row).unwrap();
        }

        // 3. Old multi-agent mission (>7d ago) -> false.
        insert_mission(
            &conn,
            "m-old",
            now - TimeDelta::days(12),
            Some(now - TimeDelta::days(10)),
        );
        for (sid, slot, rt) in [
            ("s-m-old-1", "slot-old-a", "claude-code"),
            ("s-m-old-2", "slot-old-b", "codex"),
        ] {
            let mut row = test_session_row(sid, SessionStatus::Stopped);
            row.mission_id = Some("m-old".into());
            row.slot_id = Some(slot.into());
            row.agent_runtime = Some(rt.into());
            row.started_at = Some(now - TimeDelta::days(12));
            row.stopped_at = Some(now - TimeDelta::days(10));
            insert_session_row(&conn, &row).unwrap();
        }
    }

    let before_multi = core.telemetry.status_at(&core, now).unwrap();
    let payload_before: HeartbeatPayload =
        serde_json::from_str(&before_multi.payload_preview).unwrap();
    assert_eq!(
        payload_before.meta.active_runtimes,
        vec!["claude-code", "codex"]
    );
    assert!(
        !payload_before.meta.used_mission_7d,
        "empty, single-agent, and >7d missions must not set used_mission_7d"
    );

    // 4. Recent multi-agent mission with 2 distinct non-shell slots -> true.
    {
        let conn = core.db.get().unwrap();
        insert_mission(
            &conn,
            "m-multi",
            now - TimeDelta::days(2),
            Some(now - TimeDelta::days(2)),
        );
        for (sid, slot, rt) in [
            ("s-m-multi-1", "slot-multi-a", "claude-code"),
            ("s-m-multi-2", "slot-multi-b", "codex"),
        ] {
            let mut row = test_session_row(sid, SessionStatus::Stopped);
            row.mission_id = Some("m-multi".into());
            row.slot_id = Some(slot.into());
            row.agent_runtime = Some(rt.into());
            row.started_at = Some(now - TimeDelta::days(2));
            row.stopped_at = Some(now - TimeDelta::days(2));
            insert_session_row(&conn, &row).unwrap();
        }
    }

    let status = core.telemetry.status_at(&core, now).unwrap();
    let payload: HeartbeatPayload = serde_json::from_str(&status.payload_preview).unwrap();
    assert_eq!(payload.version, "0.20.4");
    assert_eq!(payload.build_stamp, "20261126.0220");
    assert_eq!(payload.channel, "production");
    assert_eq!(payload.os, "macos");
    assert_eq!(payload.arch, "aarch64");
    assert!(!payload.remote_enabled);
    assert_eq!(payload.meta.active_runtimes, vec!["claude-code", "codex"]);
    assert!(payload.meta.used_mission_7d);
}

#[test]
fn sends_exact_payload_throttles_24h_after_2xx_and_retains_same_launch_daily_schedule() {
    let server = MockServer::start_many("204 No Content", Duration::ZERO, 2);
    let mut core = test_core();
    core.telemetry = Arc::new(TelemetryService::with_config(stamped_config(
        server.endpoint.clone(),
    )));

    let t0: DateTime<Utc> = "2026-11-26T12:00:00Z".parse().unwrap();
    // Fresh install before onboarding completes must not send.
    assert_eq!(
        core.telemetry.try_send_once(&core, t0),
        SendOutcome::SkippedOnboardingIncomplete
    );

    crate::db::mark_onboarding_completed(&core.db).unwrap();
    assert_eq!(core.telemetry.try_send_once(&core, t0), SendOutcome::Sent);

    let body = server.rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let parsed: HeartbeatPayload = serde_json::from_str(&body).unwrap();
    let status = core.telemetry.status_at(&core, t0).unwrap();
    assert_eq!(
        Some(parsed.install_id.as_str()),
        status.install_id.as_deref()
    );
    assert_eq!(parsed.version, "0.20.4");
    assert_eq!(parsed.build_stamp, "20261126.0220");
    assert_eq!(parsed.channel, "production");
    assert_eq!(
        status.last_sent_at.as_deref(),
        Some("2026-11-26T12:00:00+00:00")
    );

    // Within 24h, both same-launch and fresh-instance (relaunch) attempts are throttled.
    assert_eq!(
        core.telemetry
            .try_send_once(&core, t0 + TimeDelta::hours(23)),
        SendOutcome::SkippedThrottled
    );
    let relaunched = TelemetryService::with_config(stamped_config("http://127.0.0.1:1/hb".into()));
    assert_eq!(
        relaunched.try_send_once(&core, t0 + TimeDelta::hours(23)),
        SendOutcome::SkippedThrottled
    );

    // After 24h in the SAME launch, daily scheduling continues normally.
    assert_eq!(
        core.telemetry
            .try_send_once(&core, t0 + TimeDelta::hours(24)),
        SendOutcome::Sent
    );
    let body2 = server.rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let parsed2: HeartbeatPayload = serde_json::from_str(&body2).unwrap();
    assert_eq!(parsed2.install_id, parsed.install_id);
}

#[test]
fn non_2xx_redirect_timeout_and_unreachable_latch_failure_for_launch() {
    assert_eq!(HEARTBEAT_TIMEOUT, Duration::from_secs(5));
    let t0: DateTime<Utc> = "2026-11-26T12:00:00Z".parse().unwrap();

    // 1. HTTP 503 error drops silently and latches failure for the ENTIRE launch (even +48h).
    let server_503 = MockServer::start("503 Service Unavailable", Duration::ZERO);
    let mut core = test_core();
    crate::db::mark_onboarding_completed(&core.db).unwrap();
    core.telemetry = Arc::new(TelemetryService::with_config(stamped_config(
        server_503.endpoint.clone(),
    )));

    assert_eq!(
        core.telemetry.try_send_once(&core, t0),
        SendOutcome::FailedSilently
    );
    let _ = server_503.rx.recv_timeout(Duration::from_secs(2)).unwrap();
    drop(server_503);
    assert_eq!(
        app_state_get(&core.db.get().unwrap(), TELEMETRY_LAST_SENT_AT_KEY).unwrap(),
        None
    );
    assert_eq!(
        core.telemetry
            .try_send_once(&core, t0 + TimeDelta::minutes(5)),
        SendOutcome::SkippedFailedThisLaunch
    );
    assert_eq!(
        core.telemetry
            .try_send_once(&core, t0 + TimeDelta::hours(48)),
        SendOutcome::SkippedFailedThisLaunch
    );

    // 2. Local 302 redirect is NOT followed and counts as a failed attempt.
    let target_204 = MockServer::start("204 No Content", Duration::ZERO);
    let redirect = MockServer::start_redirect(&target_204.endpoint);
    let redirect_service = TelemetryService::with_config(stamped_config(redirect.endpoint.clone()));
    assert_eq!(
        redirect_service.try_send_once(&core, t0),
        SendOutcome::FailedSilently
    );
    let _ = redirect.rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(
        target_204.rx.try_recv().is_err(),
        "heartbeat must not follow HTTP redirects"
    );
    assert_eq!(
        app_state_get(&core.db.get().unwrap(), TELEMETRY_LAST_SENT_AT_KEY).unwrap(),
        None
    );
    drop(redirect);
    drop(target_204);

    // 3. Hanging server hits timeout cleanly without panicking.
    let server_slow = MockServer::start("204 No Content", Duration::from_millis(600));
    let mut slow_config = stamped_config(server_slow.endpoint.clone());
    slow_config.timeout = Duration::from_millis(150);
    let slow_service = TelemetryService::with_config(slow_config);
    let started = Instant::now();
    assert_eq!(
        slow_service.try_send_once(&core, t0),
        SendOutcome::FailedSilently
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "slow endpoint exceeded bounded timeout: {:?}",
        started.elapsed()
    );
    drop(server_slow);

    // 4. Connection refused / closed port drops silently within its configured timeout.
    // Use a sub-second timeout because Windows delays loopback WSAECONNREFUSED ~2s.
    let closed_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let closed_port = closed_listener.local_addr().unwrap().port();
    drop(closed_listener);
    let mut refused_config =
        stamped_config(format!("http://127.0.0.1:{closed_port}/api/heartbeat"));
    refused_config.timeout = Duration::from_millis(250);
    let refused_service = TelemetryService::with_config(refused_config);
    let started = Instant::now();
    assert_eq!(
        refused_service.try_send_once(&core, t0),
        SendOutcome::FailedSilently
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "unreachable endpoint exceeded bounded timeout: {:?}",
        started.elapsed()
    );
    assert_eq!(
        refused_service.try_send_once(&core, t0 + TimeDelta::minutes(5)),
        SendOutcome::SkippedFailedThisLaunch
    );

    // 5. Next launch (new TelemetryService) retries and succeeds once endpoint recovers.
    let recovery = MockServer::start("204 No Content", Duration::ZERO);
    let next_launch = TelemetryService::with_config(stamped_config(recovery.endpoint.clone()));
    assert_eq!(next_launch.try_send_once(&core, t0), SendOutcome::Sent);
    let _ = recovery.rx.recv_timeout(Duration::from_secs(2)).unwrap();
}

#[test]
fn scheduler_defers_first_ping_until_onboarding_completes_and_stops_promptly() {
    let server = MockServer::start("204 No Content", Duration::ZERO);
    let mut core = test_core();
    core.runtime_discovery.write().unwrap().checking = false;
    core.telemetry = Arc::new(TelemetryService::with_config(stamped_config(
        server.endpoint.clone(),
    )));

    core.telemetry.start_scheduler(core.clone());
    assert!(core.telemetry.wait_for_cycles(1, Duration::from_secs(5)));
    assert!(server.rx.try_recv().is_err());

    // Completing onboarding wakes the scheduler to send the initial heartbeat.
    crate::ops::onboarding::complete(&core).unwrap();
    let body = server
        .rx
        .recv_timeout(Duration::from_secs(5))
        .expect("heartbeat sent after onboarding completed");
    let parsed: HeartbeatPayload = serde_json::from_str(&body).unwrap();
    assert!(is_valid_install_id(&parsed.install_id));

    let stop_started = Instant::now();
    core.telemetry.stop();
    assert!(core.telemetry.wait_for_exit(Duration::from_secs(5)));
    assert!(stop_started.elapsed() < Duration::from_secs(1));
}

#[test]
fn scheduler_does_not_lose_wakeup_received_during_in_flight_cycle() {
    let server = MockServer::start("204 No Content", Duration::ZERO);
    let mut core = test_core();
    core.runtime_discovery.write().unwrap().checking = false;
    core.telemetry = Arc::new(TelemetryService::with_config(stamped_config(
        server.endpoint.clone(),
    )));

    core.telemetry.start_scheduler(core.clone());
    assert!(core.telemetry.wait_for_cycles(1, Duration::from_secs(5)));

    // Hold local_op so cycle 2 blocks inside try_send_once strictly AFTER capturing cycle_seq.
    let guard = core.telemetry.local_op.lock().unwrap();
    core.telemetry.wake_scheduler();
    assert!(core.telemetry.wait_until_in_cycle(Duration::from_secs(5)));
    // Bump wake_seq while cycle 2 is still in flight.
    core.telemetry.notify_onboarding_completed();
    drop(guard);

    // Cycle 2 returns SkippedOnboardingIncomplete, observes wake_seq != cycle_seq,
    // and immediately runs cycle 3 instead of sleeping 24 hours.
    assert!(core.telemetry.wait_for_cycles(3, Duration::from_secs(5)));

    core.telemetry.stop();
    assert!(core.telemetry.wait_for_exit(Duration::from_secs(5)));
}

#[test]
fn scheduler_never_sends_if_user_opts_out_before_onboarding_completes() {
    let server = MockServer::start("204 No Content", Duration::ZERO);
    let mut core = test_core();
    core.runtime_discovery.write().unwrap().checking = false;
    core.telemetry = Arc::new(TelemetryService::with_config(stamped_config(
        server.endpoint.clone(),
    )));

    core.telemetry.start_scheduler(core.clone());
    assert!(core.telemetry.wait_for_cycles(1, Duration::from_secs(5)));

    core.telemetry.set_enabled(&core, false).unwrap();
    crate::ops::onboarding::complete(&core).unwrap();
    assert!(core.telemetry.wait_for_cycles(2, Duration::from_secs(5)));

    assert!(server.rx.try_recv().is_err());
    assert_eq!(
        app_state_get(&core.db.get().unwrap(), TELEMETRY_INSTALL_ID_KEY).unwrap(),
        None
    );
    assert_eq!(
        app_state_get(&core.db.get().unwrap(), TELEMETRY_LAST_SENT_AT_KEY).unwrap(),
        None
    );
    core.telemetry.stop();
    assert!(core.telemetry.wait_for_exit(Duration::from_secs(5)));
}

#[test]
fn dev_build_test_url_requires_parsed_loopback_destination() {
    let server = MockServer::start("204 No Content", Duration::ZERO);
    let mut core = test_core();
    crate::db::mark_onboarding_completed(&core.db).unwrap();
    core.telemetry = Arc::new(TelemetryService::with_config(TelemetryConfig {
        dev_build: true,
        env: TelemetryEnvSource::Fixed(BTreeMap::from([(
            "RUNNER_HEARTBEAT_TEST_URL".into(),
            server.endpoint.clone(),
        )])),
        ..TelemetryConfig::default()
    }));

    let status = core.telemetry.status(&core).unwrap();
    assert!(status.dev_build);
    assert_eq!(status.endpoint, server.endpoint);
    assert_eq!(
        core.telemetry.try_send_once(&core, Utc::now()),
        SendOutcome::Sent
    );
    let _ = server.rx.recv_timeout(Duration::from_secs(2)).unwrap();

    for allowed in [
        "http://127.0.0.1:9999/api/heartbeat",
        "http://localhost:9999/api/heartbeat",
        "http://[::1]:9999/api/heartbeat",
        "https://127.0.0.1:9999/hb",
    ] {
        let env = TelemetryEnvSource::Fixed(BTreeMap::from([(
            "RUNNER_HEARTBEAT_TEST_URL".into(),
            allowed.into(),
        )]));
        assert!(
            env.test_url().is_some(),
            "expected loopback URL accepted: {allowed}"
        );
    }

    for rejected in [
        DEFAULT_HEARTBEAT_ENDPOINT,
        "https://example.com/api/heartbeat",
        "http://10.0.0.1:8080/api/heartbeat",
        "http://localhost.evil.test/api/heartbeat",
        "ftp://127.0.0.1/api/heartbeat",
        "not-a-url",
    ] {
        let blocked = TelemetryService::with_config(TelemetryConfig {
            dev_build: true,
            env: TelemetryEnvSource::Fixed(BTreeMap::from([(
                "RUNNER_HEARTBEAT_TEST_URL".into(),
                rejected.into(),
            )])),
            ..TelemetryConfig::default()
        });
        assert_eq!(
            blocked.try_send_once(&core, Utc::now()),
            SendOutcome::SkippedDevBuild,
            "expected non-loopback URL rejected: {rejected}"
        );
    }
}

#[test]
fn concurrent_status_reset_send_and_opt_out_stay_atomic_on_file_backed_sqlite() {
    let temp = tempfile::tempdir().unwrap();
    let db_path = temp.path().join("runner.db");
    let mut core = test_core();
    core.db = Arc::new(crate::db::open_pool(&db_path).unwrap());
    crate::db::mark_onboarding_completed(&core.db).unwrap();

    let server = MockServer::start_many("204 No Content", Duration::ZERO, 64);
    core.telemetry = Arc::new(TelemetryService::with_config(stamped_config(
        server.endpoint.clone(),
    )));

    // 1. Simultaneous first status reads across 8 threads must all return the exact same UUID.
    let start = Arc::new(Barrier::new(8));
    let ids: Vec<String> = thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let c = core.clone();
                let b = Arc::clone(&start);
                scope.spawn(move || {
                    b.wait();
                    c.telemetry
                        .status(&c)
                        .unwrap()
                        .install_id
                        .expect("install id")
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(ids.iter().all(|id| id == &ids[0]));

    // 2. Hammer concurrent status, reset_id, and try_send_once against opt-out; once
    //    opt-out commits, no racing status/reset/send may ever resurrect an install_id.
    let done = Arc::new(AtomicBool::new(false));
    let opted_out = Arc::new(AtomicBool::new(false));
    let ops = Arc::new(AtomicUsize::new(0));
    let race_start = Arc::new(Barrier::new(5));
    thread::scope(|scope| {
        for worker in 0..4 {
            let c = core.clone();
            let b = Arc::clone(&race_start);
            let done = Arc::clone(&done);
            let opted_out = Arc::clone(&opted_out);
            let ops = Arc::clone(&ops);
            scope.spawn(move || {
                b.wait();
                while !done.load(Ordering::Acquire) {
                    let was_opted_out = opted_out.load(Ordering::Acquire);
                    match worker {
                        0 | 1 => {
                            let st = c.telemetry.status(&c).unwrap();
                            if was_opted_out {
                                assert!(!st.enabled);
                                assert!(st.install_id.is_none());
                            }
                        }
                        2 => {
                            let st = c.telemetry.reset_id(&c).unwrap();
                            if was_opted_out {
                                assert!(!st.enabled);
                                assert!(st.install_id.is_none());
                            }
                        }
                        _ => {
                            let outcome = c.telemetry.try_send_once(&c, Utc::now());
                            if was_opted_out {
                                assert_eq!(outcome, SendOutcome::SkippedDisabled);
                            }
                        }
                    }
                    ops.fetch_add(1, Ordering::Relaxed);
                }
            });
        }
        race_start.wait();
        while ops.load(Ordering::Relaxed) < 16 {
            thread::yield_now();
        }
        let disabled = core.telemetry.set_enabled(&core, false).unwrap();
        assert!(!disabled.enabled);
        assert!(disabled.install_id.is_none());
        opted_out.store(true, Ordering::Release);
        let target = ops.load(Ordering::Relaxed) + 40;
        while ops.load(Ordering::Relaxed) < target {
            thread::yield_now();
        }
        done.store(true, Ordering::Release);
    });

    let conn = core.db.get().unwrap();
    assert_eq!(
        app_state_get(&conn, TELEMETRY_ENABLED_KEY)
            .unwrap()
            .as_deref(),
        Some("false")
    );
    assert_eq!(
        app_state_get(&conn, TELEMETRY_INSTALL_ID_KEY).unwrap(),
        None
    );
}

#[test]
fn slow_network_request_does_not_block_settings_status_or_opt_out() {
    let slow_server = MockServer::start("204 No Content", Duration::from_millis(800));
    let mut core = test_core();
    crate::db::mark_onboarding_completed(&core.db).unwrap();
    core.telemetry = Arc::new(TelemetryService::with_config(stamped_config(
        slow_server.endpoint.clone(),
    )));

    let c_bg = core.clone();
    let sender = thread::spawn(move || c_bg.telemetry.try_send_once(&c_bg, Utc::now()));

    // Wait until the HTTP request has actually arrived at the slow server.
    let _ = slow_server.rx.recv_timeout(Duration::from_secs(2)).unwrap();

    // Settings status and opt-out must complete immediately while the network call is in flight.
    let started = Instant::now();
    let _ = core.telemetry.status(&core).unwrap();
    let disabled = core.telemetry.set_enabled(&core, false).unwrap();
    assert!(!disabled.enabled);
    assert!(disabled.install_id.is_none());
    assert!(
        started.elapsed() < Duration::from_millis(300),
        "Settings operations blocked on in-flight heartbeat HTTP call ({:?})",
        started.elapsed()
    );

    let _ = sender.join().unwrap();
}

#[test]
fn same_daemon_new_desktop_launch_resets_failure_latch_while_preserving_24h_throttle() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let endpoint = format!("http://{addr}/api/heartbeat");
    let (tx, rx) = mpsc::channel::<usize>();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);
    // Request 1 returns 503; request 2 returns 204.
    let server = thread::spawn(move || {
        for attempt in 1..=2 {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            if stop_flag.load(Ordering::Acquire) {
                return;
            }
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let status_line = if attempt == 1 {
                "503 Service Unavailable"
            } else {
                "204 No Content"
            };
            let response =
                format!("HTTP/1.1 {status_line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            let _ = stream.write_all(response.as_bytes());
            let _ = tx.send(attempt);
        }
    });

    let mut core = test_core();
    core.runtime_discovery.write().unwrap().checking = false;
    crate::db::mark_onboarding_completed(&core.db).unwrap();
    core.telemetry = Arc::new(TelemetryService::with_config(stamped_config(endpoint)));

    // First desktop launch connects; scheduler attempts heartbeat and fails with 503.
    core.telemetry.app_client_connected();
    core.telemetry.start_scheduler(core.clone());
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 1);
    assert!(core.telemetry.wait_for_cycles(1, Duration::from_secs(5)));

    // A second desktop connection while the first is still alive does NOT reset the latch.
    core.telemetry.app_client_connected();
    core.telemetry.app_client_disconnected();
    assert_eq!(
        core.telemetry.try_send_once(&core, Utc::now()),
        SendOutcome::SkippedFailedThisLaunch
    );
    assert!(rx.try_recv().is_err());

    // Last desktop client disconnects (e.g., Keep running quit), then user relaunches desktop app.
    core.telemetry.app_client_disconnected();
    core.telemetry.app_client_connected();

    // Same running daemon's scheduler wakes, retries, and succeeds with 204.
    assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
    assert!(core.telemetry.wait_for_cycles(2, Duration::from_secs(5)));
    assert!(
        app_state_get(&core.db.get().unwrap(), TELEMETRY_LAST_SENT_AT_KEY)
            .unwrap()
            .is_some()
    );

    // Another desktop disconnect + relaunch within 24h still honors the 24h successful-send throttle.
    core.telemetry.app_client_disconnected();
    core.telemetry.app_client_connected();
    assert_eq!(
        core.telemetry.try_send_once(&core, Utc::now()),
        SendOutcome::SkippedThrottled
    );

    core.telemetry.stop();
    assert!(core.telemetry.wait_for_exit(Duration::from_secs(5)));
    stop.store(true, Ordering::Release);
    if !server.is_finished() {
        let _ = TcpStream::connect_timeout(&addr, Duration::from_millis(100));
    }
    let _ = server.join();
}

#[test]
fn in_flight_failure_from_previous_launch_or_headless_boot_cannot_silence_new_desktop_launch() {
    // Part A: Request 1 is held in flight until AFTER a Keep-running quit + desktop relaunch,
    // then fails with 503. The stale failure must not latch the new desktop launch.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let endpoint = format!("http://{addr}/api/heartbeat");
    let (arrived_tx, arrived_rx) = mpsc::channel::<usize>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let (done_tx, done_rx) = mpsc::channel::<usize>();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = Arc::clone(&stop);
    let server = thread::spawn(move || {
        for attempt in 1..=2 {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            if stop_flag.load(Ordering::Acquire) {
                return;
            }
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = arrived_tx.send(attempt);
            if attempt == 1 {
                let _ = release_rx.recv();
            }
            let status_line = if attempt == 1 {
                "503 Service Unavailable"
            } else {
                "204 No Content"
            };
            let response =
                format!("HTTP/1.1 {status_line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            let _ = stream.write_all(response.as_bytes());
            let _ = done_tx.send(attempt);
        }
    });

    let mut core = test_core();
    core.runtime_discovery.write().unwrap().checking = false;
    crate::db::mark_onboarding_completed(&core.db).unwrap();
    core.telemetry = Arc::new(TelemetryService::with_config(stamped_config(endpoint)));

    // Desktop launch 1 connects and starts request 1.
    core.telemetry.app_client_connected();
    core.telemetry.start_scheduler(core.clone());
    assert_eq!(arrived_rx.recv_timeout(Duration::from_secs(5)).unwrap(), 1);

    // While request 1 is STILL in flight, user quits (1 -> 0) and relaunches (0 -> 1).
    core.telemetry.app_client_disconnected();
    core.telemetry.app_client_connected();

    // Now release request 1's 503 failure strictly AFTER desktop launch 2 connected.
    let _ = release_tx.send(());
    assert_eq!(done_rx.recv_timeout(Duration::from_secs(5)).unwrap(), 1);

    // Launch 2 must NOT be silenced by launch 1's late 503; scheduler immediately retries and succeeds.
    assert_eq!(arrived_rx.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
    assert_eq!(done_rx.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
    assert!(core.telemetry.wait_for_cycles(2, Duration::from_secs(5)));
    assert!(
        app_state_get(&core.db.get().unwrap(), TELEMETRY_LAST_SENT_AT_KEY)
            .unwrap()
            .is_some()
    );

    // Persisted 24h throttle still holds on a subsequent relaunch.
    core.telemetry.app_client_disconnected();
    core.telemetry.app_client_connected();
    assert_eq!(
        core.telemetry.try_send_once(&core, Utc::now()),
        SendOutcome::SkippedThrottled
    );

    core.telemetry.stop();
    assert!(core.telemetry.wait_for_exit(Duration::from_secs(5)));
    stop.store(true, Ordering::Release);
    if !server.is_finished() {
        let _ = TcpStream::connect_timeout(&addr, Duration::from_millis(100));
    }
    let _ = server.join();

    // Part B: Headless daemon starts and fails BEFORE any desktop client connects;
    // the first desktop connection (0 -> 1) starts a fresh launch generation and succeeds.
    let listener2 = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr2 = listener2.local_addr().unwrap();
    let endpoint2 = format!("http://{addr2}/api/heartbeat");
    let (done2_tx, done2_rx) = mpsc::channel::<usize>();
    let stop2 = Arc::new(AtomicBool::new(false));
    let stop2_flag = Arc::clone(&stop2);
    let server2 = thread::spawn(move || {
        for attempt in 1..=2 {
            let Ok((mut stream, _)) = listener2.accept() else {
                break;
            };
            if stop2_flag.load(Ordering::Acquire) {
                return;
            }
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let status_line = if attempt == 1 {
                "503 Service Unavailable"
            } else {
                "204 No Content"
            };
            let response =
                format!("HTTP/1.1 {status_line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            let _ = stream.write_all(response.as_bytes());
            let _ = done2_tx.send(attempt);
        }
    });

    let mut headless_core = test_core();
    headless_core.runtime_discovery.write().unwrap().checking = false;
    crate::db::mark_onboarding_completed(&headless_core.db).unwrap();
    headless_core.telemetry = Arc::new(TelemetryService::with_config(stamped_config(endpoint2)));

    // Start scheduler with zero desktop clients connected; headless attempt fails with 503.
    headless_core
        .telemetry
        .start_scheduler(headless_core.clone());
    assert_eq!(done2_rx.recv_timeout(Duration::from_secs(5)).unwrap(), 1);
    assert!(headless_core
        .telemetry
        .wait_for_cycles(1, Duration::from_secs(5)));

    // First desktop app connects (0 -> 1); retries and succeeds with 204.
    headless_core.telemetry.app_client_connected();
    assert_eq!(done2_rx.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
    assert!(headless_core
        .telemetry
        .wait_for_cycles(2, Duration::from_secs(5)));
    assert!(
        app_state_get(&headless_core.db.get().unwrap(), TELEMETRY_LAST_SENT_AT_KEY)
            .unwrap()
            .is_some()
    );

    headless_core.telemetry.stop();
    assert!(headless_core
        .telemetry
        .wait_for_exit(Duration::from_secs(5)));
    stop2.store(true, Ordering::Release);
    if !server2.is_finished() {
        let _ = TcpStream::connect_timeout(&addr2, Duration::from_millis(100));
    }
    let _ = server2.join();
}

#[test]
fn stored_opt_out_takes_precedence_over_throttle_and_failure_latch() {
    let t0: DateTime<Utc> = "2026-11-26T12:00:00Z".parse().unwrap();

    // 1. After a successful 2xx send (within 24h throttle), disabling returns SkippedDisabled.
    let server_ok = MockServer::start("204 No Content", Duration::ZERO);
    let mut core = test_core();
    crate::db::mark_onboarding_completed(&core.db).unwrap();
    core.telemetry = Arc::new(TelemetryService::with_config(stamped_config(
        server_ok.endpoint.clone(),
    )));
    assert_eq!(core.telemetry.try_send_once(&core, t0), SendOutcome::Sent);
    let _ = server_ok.rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(
        core.telemetry
            .try_send_once(&core, t0 + TimeDelta::minutes(1)),
        SendOutcome::SkippedThrottled
    );
    core.telemetry.set_enabled(&core, false).unwrap();
    assert_eq!(
        core.telemetry
            .try_send_once(&core, t0 + TimeDelta::minutes(2)),
        SendOutcome::SkippedDisabled
    );

    // 2. After a latched 503 failure, disabling also returns SkippedDisabled.
    let server_fail = MockServer::start("503 Service Unavailable", Duration::ZERO);
    let mut core_fail = test_core();
    crate::db::mark_onboarding_completed(&core_fail.db).unwrap();
    core_fail.telemetry = Arc::new(TelemetryService::with_config(stamped_config(
        server_fail.endpoint.clone(),
    )));
    assert_eq!(
        core_fail.telemetry.try_send_once(&core_fail, t0),
        SendOutcome::FailedSilently
    );
    let _ = server_fail.rx.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(
        core_fail
            .telemetry
            .try_send_once(&core_fail, t0 + TimeDelta::minutes(1)),
        SendOutcome::SkippedFailedThisLaunch
    );
    core_fail.telemetry.set_enabled(&core_fail, false).unwrap();
    assert_eq!(
        core_fail
            .telemetry
            .try_send_once(&core_fail, t0 + TimeDelta::minutes(2)),
        SendOutcome::SkippedDisabled
    );
}
