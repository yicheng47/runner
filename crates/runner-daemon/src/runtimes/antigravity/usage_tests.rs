use super::*;
use chrono::DateTime;
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::Instant;

#[test]
fn parses_dual_pools_ordered_five_hours_before_week() {
    let payload = serde_json::json!({
        "command": {
            "name": "usage",
            "data": {
                "groups": [
                    {
                        "name": "Gemini Models",
                        "buckets": [
                            {"window": "weekly", "remaining_fraction": 0.52, "reset_time": "2026-11-18T00:00:00Z"},
                            {"window": "5h", "remaining_fraction": 0.82, "reset_time": "2026-11-13T18:00:00Z"}
                        ]
                    },
                    {
                        "name": "Claude and GPT models",
                        "buckets": [
                            {"window": "weekly", "remaining_fraction": 0.18, "reset_time": 1770000000},
                            {"window": "5h", "remaining_fraction": 0.65, "reset_time": "2026-11-13T16:00:00Z"}
                        ]
                    }
                ]
            }
        },
        "status": "SUCCESS"
    });
    let windows = parse_antigravity(&payload).unwrap();
    assert_eq!(windows.len(), 4);
    assert_eq!(windows[0].name, "Gemini Models · 5 hours used");
    assert!((windows[0].used_percent - 18.0).abs() < 1e-6);
    assert_eq!(
        windows[0].resets_at,
        DateTime::parse_from_rfc3339("2026-11-13T18:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc)
            .into()
    );
    assert_eq!(windows[1].name, "Gemini Models · Week used");
    assert!((windows[1].used_percent - 48.0).abs() < 1e-6);
    assert_eq!(windows[2].name, "Claude and GPT models · 5 hours used");
    assert!((windows[2].used_percent - 35.0).abs() < 1e-6);
    assert_eq!(windows[3].name, "Claude and GPT models · Week used");
    assert!((windows[3].used_percent - 82.0).abs() < 1e-6);
    assert_eq!(
        windows[3].resets_at,
        DateTime::from_timestamp(1770000000, 0)
    );
}

#[test]
fn rejects_invalid_or_signed_out_usage_json() {
    assert_eq!(
        parse_antigravity(&serde_json::json!({"command": {"name": "other"}, "status": "SUCCESS"})),
        Err(UnavailableReason::InvalidResponse)
    );
    assert_eq!(
        parse_antigravity(
            &serde_json::json!({"command": {"name": "usage"}, "status": "FAILED", "error": "Not authenticated; run agy login"})
        ),
        Err(UnavailableReason::SignIn)
    );
    assert_eq!(
        parse_antigravity(
            &serde_json::json!({"command": {"name": "usage", "data": {"groups": []}}, "status": "SUCCESS"})
        ),
        Err(UnavailableReason::InvalidResponse)
    );
    assert_eq!(
        parse_antigravity(&serde_json::json!({
            "command": {"name": "usage", "data": {"groups": [{"name": "Gemini Models", "buckets": [{"window": "weekly", "remaining_fraction": 1.5}]}]}},
            "status": "SUCCESS"
        })),
        Err(UnavailableReason::InvalidResponse)
    );
    assert_eq!(
        parse_antigravity_output(b"not json at all"),
        Err(UnavailableReason::InvalidResponse)
    );
    // Unix CLI output strictly uses snake_case fields (`remaining_fraction` / `reset_time`),
    // never Cloud Code ProtoJSON camelCase aliases (`remainingFraction` / `resetTime`).
    assert_eq!(
        parse_antigravity(&serde_json::json!({
            "command": {"name": "usage", "data": {"groups": [{"name": "Gemini Models", "buckets": [{"window": "5h", "remainingFraction": 0.5, "resetTime": "2026-11-18T00:00:00Z"}]}]}},
            "status": "SUCCESS"
        })),
        Err(UnavailableReason::InvalidResponse)
    );
}

#[test]
fn extracts_json_after_leading_log_line() {
    let raw = b"Update available\n{\"command\":{\"name\":\"usage\",\"data\":{\"groups\":[{\"name\":\"Gemini Models\",\"buckets\":[{\"window\":\"weekly\",\"remaining_fraction\":0.25}]}]}},\"status\":\"SUCCESS\"}\n";
    let windows = parse_antigravity_output(raw).unwrap();
    assert_eq!(windows.len(), 1);
    assert_eq!(windows[0].name, "Gemini Models · Week used");
    assert!((windows[0].used_percent - 75.0).abs() < 1e-6);
}

#[cfg(unix)]
fn write_script(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

#[cfg(unix)]
#[test]
fn signed_out_gate_never_invokes_usage_probe() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("invocations.log");
    let script = write_script(
        dir.path(),
        "agy",
        &format!(
            r#"echo "auth=$AGY_CLI_CDE_AUTH_ACTION auto=$AGY_CLI_DISABLE_AUTO_UPDATE args=$*" >> "{log}"
if [ "$1" = "--version" ]; then
  echo "1.5.8"
  exit 0
fi
if [ "$AGY_CLI_CDE_AUTH_ACTION" = "check" ] && [ "$#" -eq 0 ]; then
  exit 1
fi
exit 99
"#,
            log = log.display()
        ),
    );
    let env = LoginShellEnv::default();
    let result = fetch_antigravity_with_timeouts(
        script.to_str().unwrap(),
        &env,
        Duration::from_secs(10),
        Duration::from_secs(10),
    );
    assert_eq!(result, Err(UnavailableReason::SignIn));
    let recorded = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        recorded.lines().collect::<Vec<_>>(),
        [
            "auth= auto=true args=--version",
            "auth=check auto=true args=",
        ]
    );
}

#[cfg(unix)]
#[test]
fn preflight_timeout_kills_process_and_never_invokes_usage_probe() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("invocations.log");
    let script = write_script(
        dir.path(),
        "agy",
        &format!(
            r#"if [ "$1" = "warmup" ]; then exit 0; fi
if [ "$1" = "--version" ]; then
  echo "1.5.8"
  exit 0
fi
echo "call args=$#" >> "{log}"
sleep 5
"#,
            log = log.display()
        ),
    );
    // Let macOS complete initial executable inspection before starting the tight timer.
    assert!(std::process::Command::new(&script)
        .arg("warmup")
        .status()
        .unwrap()
        .success());
    let env = LoginShellEnv::default();
    let result = fetch_antigravity_with_timeouts(
        script.to_str().unwrap(),
        &env,
        Duration::from_millis(300),
        Duration::from_secs(10),
    );
    assert_eq!(result, Err(UnavailableReason::AntigravityNoAnswer));
    let recorded = std::fs::read_to_string(&log).unwrap();
    assert_eq!(recorded.lines().collect::<Vec<_>>(), ["call args=0"]);
}

#[cfg(unix)]
fn wait_until_process_exits(pid: i32, max_wait: Duration) -> bool {
    let deadline = Instant::now() + max_wait;
    loop {
        if !crate::session::process::process_exists(pid) {
            return true;
        }
        if Instant::now() >= deadline {
            unsafe {
                libc::kill(pid, libc::SIGKILL);
            }
            return false;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(unix)]
#[test]
fn signed_in_gate_detaches_tty_and_parses_pools() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("invocations.log");
    let script = write_script(
        dir.path(),
        "agy",
        &format!(
            r#"if ( : < /dev/tty ) 2>/dev/null; then tty=yes; else tty=no; fi
if [ ! -e /dev/fd/0 ]; then stdin_char=missing; elif [ -c /dev/fd/0 ]; then stdin_char=yes; else stdin_char=no; fi
echo "auth=$AGY_CLI_CDE_AUTH_ACTION tty=$tty stdin_char=$stdin_char args=$*" >> "{log}"
if [ "$1" = "--version" ]; then
  echo "1.5.8"
  exit 0
fi
if [ "$AGY_CLI_CDE_AUTH_ACTION" = "check" ] && [ "$#" -eq 0 ]; then
  echo "Already authenticated as user@example.com."
  exit 0
fi
if [ "$1" = "-p" ] && [ "$2" = "/usage" ]; then
  printf '%s\n' '{{"command":{{"name":"usage","data":{{"groups":[{{"name":"Gemini Models","buckets":[{{"window":"weekly","remaining_fraction":0.52}},{{"window":"5h","remaining_fraction":0.82}}]}},{{"name":"Claude and GPT models","buckets":[{{"window":"weekly","remaining_fraction":0.18}},{{"window":"5h","remaining_fraction":0.65}}]}}]}}}},"status":"SUCCESS"}}'
  exit 0
fi
exit 2
"#,
            log = log.display()
        ),
    );
    let env = LoginShellEnv::default();
    let reading = fetch_antigravity_with_timeouts(
        script.to_str().unwrap(),
        &env,
        Duration::from_secs(10),
        Duration::from_secs(10),
    )
    .unwrap();
    assert_eq!(reading.windows.len(), 4);
    assert_eq!(reading.windows[0].name, "Gemini Models · 5 hours used");
    assert!((reading.windows[3].used_percent - 82.0).abs() < 1e-6);
    let recorded = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        recorded.lines().collect::<Vec<_>>(),
        [
            "auth= tty=no stdin_char=no args=--version",
            "auth=check tty=no stdin_char=no args=",
            "auth= tty=no stdin_char=no args=-p /usage --output-format json",
        ]
    );
}

#[cfg(unix)]
#[test]
fn usage_timeout_maps_to_no_answer() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("invocations.log");
    let script = write_script(
        dir.path(),
        "agy",
        &format!(
            r#"if [ "$1" = "warmup" ]; then exit 0; fi
if [ "$1" = "--version" ]; then
  echo "1.5.8"
  exit 0
fi
if [ "$AGY_CLI_CDE_AUTH_ACTION" = "check" ]; then
  echo "check" >> "{log}"
  exit 0
fi
echo "usage" >> "{log}"
sleep 5
"#,
            log = log.display()
        ),
    );
    // Let macOS complete initial executable inspection before the tight usage timer.
    assert!(std::process::Command::new(&script)
        .arg("warmup")
        .status()
        .unwrap()
        .success());
    let env = LoginShellEnv::default();
    let result = fetch_antigravity_with_timeouts(
        script.to_str().unwrap(),
        &env,
        Duration::from_secs(10),
        Duration::from_millis(200),
    );
    assert_eq!(result, Err(UnavailableReason::AntigravityNoAnswer));
    let recorded = std::fs::read_to_string(&log).unwrap();
    assert_eq!(recorded.lines().collect::<Vec<_>>(), ["check", "usage"]);
}

#[cfg(unix)]
#[test]
fn usage_auth_error_after_check_maps_to_sign_in() {
    let dir = tempfile::tempdir().unwrap();
    let script = write_script(
        dir.path(),
        "agy",
        r#"if [ "$1" = "--version" ]; then
  echo "1.5.8"
  exit 0
fi
if [ "$AGY_CLI_CDE_AUTH_ACTION" = "check" ]; then
  exit 0
fi
echo "Session expired, please run agy login" >&2
exit 1
"#,
    );
    let env = LoginShellEnv::default();
    let result = fetch_antigravity_with_timeouts(
        script.to_str().unwrap(),
        &env,
        Duration::from_secs(10),
        Duration::from_secs(10),
    );
    assert_eq!(result, Err(UnavailableReason::SignIn));
}

#[cfg(unix)]
#[test]
fn usage_timeout_bounds_pipe_holding_descendant_and_kills_it() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("descendant.pid");
    let script = write_script(
        dir.path(),
        "agy",
        &format!(
            r#"if [ "$1" = "warmup" ]; then exit 0; fi
if [ "$1" = "--version" ]; then
  echo "1.5.8"
  exit 0
fi
if [ "$AGY_CLI_CDE_AUTH_ACTION" = "check" ]; then
  exit 0
fi
sleep 600 &
echo "$!" > "{pid_file}"
exit 0
"#,
            pid_file = pid_file.display()
        ),
    );
    assert!(std::process::Command::new(&script)
        .arg("warmup")
        .status()
        .unwrap()
        .success());
    let env = LoginShellEnv::default();
    let started = Instant::now();
    let result = fetch_antigravity_with_timeouts(
        script.to_str().unwrap(),
        &env,
        Duration::from_secs(10),
        Duration::from_millis(200),
    );
    assert_eq!(result, Err(UnavailableReason::AntigravityNoAnswer));
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "pipe timeout exceeded bounded deadline: {:?}",
        started.elapsed()
    );
    let pid: i32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        wait_until_process_exits(pid, Duration::from_secs(2)),
        "pipe-holding descendant {pid} was not killed"
    );
}

#[cfg(unix)]
#[test]
fn normal_exit_cleans_up_detached_descendant() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("descendant.pid");
    let script = write_script(
        dir.path(),
        "agy",
        &format!(
            r#"if [ "$1" = "warmup" ]; then exit 0; fi
if [ "$1" = "--version" ]; then
  echo "1.5.8"
  exit 0
fi
if [ "$AGY_CLI_CDE_AUTH_ACTION" = "check" ]; then
  exit 0
fi
sleep 600 </dev/null >/dev/null 2>&1 &
echo "$!" > "{pid_file}"
printf '%s\n' '{{"command":{{"name":"usage","data":{{"groups":[{{"name":"Gemini Models","buckets":[{{"window":"weekly","remaining_fraction":0.52}}]}},{{"name":"Claude and GPT models","buckets":[{{"window":"weekly","remaining_fraction":0.18}}]}}]}}}},"status":"SUCCESS"}}'
exit 0
"#,
            pid_file = pid_file.display()
        ),
    );
    assert!(std::process::Command::new(&script)
        .arg("warmup")
        .status()
        .unwrap()
        .success());
    let env = LoginShellEnv::default();
    let reading = fetch_antigravity_with_timeouts(
        script.to_str().unwrap(),
        &env,
        Duration::from_secs(10),
        Duration::from_secs(10),
    )
    .unwrap();
    assert_eq!(reading.windows.len(), 2);
    let pid: i32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        wait_until_process_exits(pid, Duration::from_secs(2)),
        "background descendant {pid} survived normal command exit"
    );
}

#[cfg(unix)]
#[test]
fn unsupported_version_never_invokes_auth_check_or_usage_probe() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("invocations.log");
    let script = write_script(
        dir.path(),
        "agy",
        &format!(
            r#"if [ "$1" = "warmup" ]; then exit 0; fi
echo "auth=$AGY_CLI_CDE_AUTH_ACTION args=$*" >> "{log}"
if [ "$1" = "--version" ]; then
  echo "$TEST_VERSION"
  exit 0
fi
exit 99
"#,
            log = log.display()
        ),
    );
    assert!(std::process::Command::new(&script)
        .arg("warmup")
        .status()
        .unwrap()
        .success());
    for version in ["1.5.7", "1.2.9", "1.5.8-rc.1"] {
        let _ = std::fs::remove_file(&log);
        let mut env = LoginShellEnv::default();
        env.vars.insert("TEST_VERSION".into(), version.into());
        let result = fetch_antigravity_with_timeouts(
            script.to_str().unwrap(),
            &env,
            Duration::from_secs(10),
            Duration::from_secs(10),
        );
        assert_eq!(result, Err(UnavailableReason::AntigravityNoAnswer));
        let recorded = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            recorded.lines().collect::<Vec<_>>(),
            ["auth= args=--version"],
            "version {version} must not reach auth check or usage"
        );
    }
}

#[cfg(unix)]
#[test]
fn unknown_or_failing_version_never_invokes_auth_check_or_usage_probe() {
    let dir = tempfile::tempdir().unwrap();
    let log = dir.path().join("invocations.log");
    let script = write_script(
        dir.path(),
        "agy",
        &format!(
            r#"if [ "$1" = "warmup" ]; then exit 0; fi
echo "auth=$AGY_CLI_CDE_AUTH_ACTION args=$*" >> "{log}"
if [ -n "$TEST_SLEEP" ]; then
  sleep 5
fi
if [ "$1" = "--version" ]; then
  if [ -n "$TEST_STDOUT" ]; then echo "$TEST_STDOUT"; fi
  exit "${{TEST_EXIT:-0}}"
fi
exit 99
"#,
            log = log.display()
        ),
    );
    assert!(std::process::Command::new(&script)
        .arg("warmup")
        .status()
        .unwrap()
        .success());

    for (stdout, code) in [("unknown", "0"), ("", "0"), ("1.5.8", "1")] {
        let _ = std::fs::remove_file(&log);
        let mut env = LoginShellEnv::default();
        env.vars.insert("TEST_STDOUT".into(), stdout.into());
        env.vars.insert("TEST_EXIT".into(), code.into());
        let result = fetch_antigravity_with_timeouts(
            script.to_str().unwrap(),
            &env,
            Duration::from_secs(10),
            Duration::from_secs(10),
        );
        assert_eq!(result, Err(UnavailableReason::AntigravityNoAnswer));
        let recorded = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            recorded.lines().collect::<Vec<_>>(),
            ["auth= args=--version"]
        );
    }

    let _ = std::fs::remove_file(&log);
    let mut env = LoginShellEnv::default();
    env.vars.insert("TEST_SLEEP".into(), "1".into());
    let result = fetch_antigravity_with_timeouts(
        script.to_str().unwrap(),
        &env,
        Duration::from_millis(200),
        Duration::from_secs(10),
    );
    assert_eq!(result, Err(UnavailableReason::AntigravityNoAnswer));
    let recorded = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        recorded.lines().collect::<Vec<_>>(),
        ["auth= args=--version"]
    );
}

#[cfg(windows)]
#[test]
fn windows_probe_spawns_no_child_process() {
    let dir = tempfile::tempdir().unwrap();
    let bin_dir = dir.path().join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let sentinel = dir.path().join("spawned.txt");
    let fake_agy = dir.path().join("fake-agy.cmd");
    let path_agy = bin_dir.join("agy.cmd");
    let batch = format!("@echo spawned>\"{}\"\r\n", sentinel.display());
    std::fs::write(&fake_agy, &batch).unwrap();
    std::fs::write(&path_agy, &batch).unwrap();

    let marker = dir
        .path()
        .join(".gemini")
        .join("antigravity-cli")
        .join("cache")
        .join("antigravity-keyring-unavailable");
    std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
    std::fs::write(&marker, "test-isolated").unwrap();

    let mut env = LoginShellEnv::default();
    env.vars.insert(
        "USERPROFILE".into(),
        dir.path().to_string_lossy().into_owned(),
    );
    env.vars
        .insert("HOME".into(), dir.path().to_string_lossy().into_owned());
    env.vars
        .insert("PATH".into(), bin_dir.to_string_lossy().into_owned());

    let result = fetch_antigravity_with_timeouts(
        fake_agy.to_str().unwrap(),
        &env,
        Duration::from_secs(2),
        Duration::from_secs(2),
    );
    assert_eq!(result, Err(UnavailableReason::SignIn));
    assert!(
        !sentinel.exists(),
        "Windows signed-out probe must never spawn agy or any child process"
    );

    // Unexpired credential path on disk must also spawn zero processes.
    let cli_dir = dir.path().join(".gemini").join("antigravity-cli");
    let token_path = cli_dir.join("antigravity-oauth-token");
    std::fs::write(
        &token_path,
        serde_json::json!({
            "token": {
                "access_token": "unexpired-win-tok",
                "refresh_token": "win-refresh",
                "expiry": "2099-01-01T00:00:00Z"
            },
            "auth_method": "consumer"
        })
        .to_string(),
    )
    .unwrap();
    let (base_url, _rx, server) = spawn_fake_http_server(2, |_, req| {
        match req.path.as_str() {
            "/v1internal:loadCodeAssist" => (
                200,
                "OK",
                r#"{"cloudaicompanionProject":"projects/win-p"}"#.into(),
            ),
            "/v1internal:retrieveUserQuotaSummary" => (
                200,
                "OK",
                r#"{"groups":[{"displayName":"Gemini Models","buckets":[{"window":"5h","remainingFraction":0.75}]}]}"#.into(),
            ),
            other => panic!("unexpected path: {other}"),
        }
    });
    let reading = direct_quota::fetch_with_options_with_proxy(
        &env,
        Duration::from_secs(2),
        Duration::from_secs(2),
        Some(direct_quota::EndpointConfig {
            oauth_token_url: &format!("{base_url}/oauth2/token"),
            cloud_code_url: &base_url,
        }),
        &|_| None,
        false,
        || Ok(None),
    )
    .unwrap();
    server.join().unwrap();
    assert_eq!(reading.windows.len(), 1);
    assert!(
        !sentinel.exists(),
        "Windows quota fetch must never spawn agy"
    );

    // Expired token refresh path must also spawn zero processes.
    std::fs::write(
        &token_path,
        serde_json::json!({
            "token": {
                "access_token": "stale-win-tok",
                "refresh_token": "win-refresh",
                "expiry": "2020-01-01T00:00:00Z"
            },
            "auth_method": "consumer"
        })
        .to_string(),
    )
    .unwrap();
    let (base_url, _rx, server) = spawn_fake_http_server(3, |_, req| {
        match req.path.as_str() {
            "/oauth2/token" => (
                200,
                "OK",
                r#"{"access_token":"refreshed-win-tok","expires_in":3600}"#.into(),
            ),
            "/v1internal:loadCodeAssist" => (
                200,
                "OK",
                r#"{"cloudaicompanionProject":"projects/win-p"}"#.into(),
            ),
            "/v1internal:retrieveUserQuotaSummary" => (
                200,
                "OK",
                r#"{"groups":[{"displayName":"Gemini Models","buckets":[{"window":"5h","remainingFraction":0.5}]}]}"#.into(),
            ),
            other => panic!("unexpected path: {other}"),
        }
    });
    let reading = direct_quota::fetch_with_options_with_proxy(
        &env,
        Duration::from_secs(2),
        Duration::from_secs(2),
        Some(direct_quota::EndpointConfig {
            oauth_token_url: &format!("{base_url}/oauth2/token"),
            cloud_code_url: &base_url,
        }),
        &|_| None,
        false,
        || Ok(None),
    )
    .unwrap();
    server.join().unwrap();
    assert!((reading.windows[0].used_percent - 50.0).abs() < 1e-6);
    assert!(
        !sentinel.exists(),
        "Windows refresh path must never spawn agy"
    );
}

#[cfg(windows)]
#[test]
fn windows_credential_manager_roundtrip() {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Security::Credentials::{
        CredDeleteW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    };

    struct DeleteCredGuard(Vec<u16>);
    impl Drop for DeleteCredGuard {
        fn drop(&mut self) {
            unsafe {
                CredDeleteW(self.0.as_ptr(), CRED_TYPE_GENERIC, 0);
            }
        }
    }

    // Always use a test-only target name; never touch `"gemini:antigravity"`.
    let target = format!("runner-test:antigravity-usage-{}", std::process::id());
    let wide_target: Vec<u16> = std::ffi::OsStr::new(&target)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let cleanup_guard = DeleteCredGuard(wide_target.clone());

    let secret_json = serde_json::json!({
        "token": {
            "access_token": "win-test-access",
            "refresh_token": "win-test-refresh",
            "expiry": "2099-01-01T00:00:00Z"
        },
        "auth_method": "consumer",
        "project_id": "win-proj"
    })
    .to_string();

    let write_blob = |bytes: &[u8]| {
        let mut target_buf = wide_target.clone();
        let mut blob = bytes.to_vec();
        let mut cred: CREDENTIALW = unsafe { std::mem::zeroed() };
        cred.Type = CRED_TYPE_GENERIC;
        cred.TargetName = target_buf.as_mut_ptr();
        cred.CredentialBlobSize = blob.len() as u32;
        cred.CredentialBlob = blob.as_mut_ptr();
        cred.Persist = CRED_PERSIST_LOCAL_MACHINE;
        let wrote = unsafe { CredWriteW(&cred, 0) };
        assert_ne!(wrote, 0, "CredWriteW should store test credential");
    };

    // Raw UTF-8 JSON (go-keyring default on Windows)
    write_blob(secret_json.as_bytes());
    let read_back = direct_quota::read_windows_credential_target(&target)
        .expect("CredReadW should succeed")
        .expect("test credential should be present");
    let parsed = direct_quota::parse_stored_credential(&read_back)
        .expect("stored credential JSON should parse");
    assert_eq!(parsed.access_token, "win-test-access");
    assert_eq!(parsed.refresh_token, "win-test-refresh");
    assert_eq!(parsed.auth_method.as_deref(), Some("consumer"));
    assert_eq!(parsed.project_id.as_deref(), Some("win-proj"));

    // `go-keyring-base64:` encoded as UTF-16LE
    use base64::Engine as _;
    let b64_secret = format!(
        "go-keyring-base64:{}",
        base64::engine::general_purpose::STANDARD.encode(secret_json.as_bytes())
    );
    let utf16_blob: Vec<u8> = b64_secret
        .encode_utf16()
        .flat_map(u16::to_le_bytes)
        .collect();
    write_blob(&utf16_blob);
    let read_back_b64 = direct_quota::read_windows_credential_target(&target)
        .expect("CredReadW should succeed for UTF-16LE base64")
        .expect("UTF-16LE base64 credential should be present");
    let parsed_b64 = direct_quota::parse_stored_credential(&read_back_b64)
        .expect("UTF-16LE base64 JSON should parse");
    assert_eq!(parsed_b64.access_token, "win-test-access");

    drop(cleanup_guard);
    assert_eq!(
        direct_quota::read_windows_credential_target(&target),
        Ok(None)
    );
}

#[derive(Clone, Debug)]
struct RecordedHttpRequest {
    path: String,
    headers: std::collections::BTreeMap<String, String>,
    body: String,
}

fn spawn_fake_http_server<F>(
    responses: usize,
    mut handler: F,
) -> (
    String,
    std::sync::mpsc::Receiver<RecordedHttpRequest>,
    std::thread::JoinHandle<()>,
)
where
    F: FnMut(usize, &RecordedHttpRequest) -> (u16, &'static str, String) + Send + 'static,
{
    use std::io::{Read, Write};
    use std::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let handle = std::thread::spawn(move || {
        for idx in 0..responses {
            let (mut stream, _) = listener.accept().unwrap();
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            let mut header_end = None;
            while header_end.is_none() {
                let n = stream.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    header_end = Some(pos + 4);
                }
            }
            let Some(head_end) = header_end else {
                continue;
            };
            let head_str = String::from_utf8_lossy(&buf[..head_end - 4]);
            let mut lines = head_str.lines();
            let request_line = lines.next().unwrap_or_default();
            let path = request_line
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_owned();
            let mut headers = std::collections::BTreeMap::new();
            let mut content_length = 0usize;
            for line in lines {
                if let Some((k, v)) = line.split_once(':') {
                    let key = k.trim().to_ascii_lowercase();
                    let val = v.trim().to_owned();
                    if key == "content-length" {
                        content_length = val.parse().unwrap_or(0);
                    }
                    headers.insert(key, val);
                }
            }
            while buf.len() < head_end + content_length {
                let n = stream.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            let body = String::from_utf8_lossy(
                &buf[head_end..head_end + content_length.min(buf.len() - head_end)],
            )
            .into_owned();
            let req = RecordedHttpRequest {
                path,
                headers,
                body,
            };
            let _ = tx.send(req.clone());
            let (status, reason, resp_body) = handler(idx, &req);
            let reply = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    resp_body.len(),
                    resp_body
                );
            let _ = stream.write_all(reply.as_bytes());
            let _ = stream.flush();
        }
    });
    (format!("http://{addr}"), rx, handle)
}

#[test]
fn direct_quota_decodes_keyring_blobs() {
    use base64::Engine as _;

    let json = r#"{"token":{"access_token":"abc","refresh_token":"def"}}"#;
    assert_eq!(
        direct_quota::decode_keyring_blob(json.as_bytes()).as_deref(),
        Some(json)
    );

    let mut with_bom = b"\xEF\xBB\xBF".to_vec();
    with_bom.extend_from_slice(json.as_bytes());
    assert_eq!(
        direct_quota::decode_keyring_blob(&with_bom).as_deref(),
        Some(json)
    );

    let utf16_le: Vec<u8> = json.encode_utf16().flat_map(u16::to_le_bytes).collect();
    assert_eq!(
        direct_quota::decode_keyring_blob(&utf16_le).as_deref(),
        Some(json)
    );

    let mut utf16_bom = vec![0xFF, 0xFE];
    utf16_bom.extend_from_slice(&utf16_le);
    assert_eq!(
        direct_quota::decode_keyring_blob(&utf16_bom).as_deref(),
        Some(json)
    );

    let encoded = format!(
        "go-keyring-base64:{}",
        base64::engine::general_purpose::STANDARD.encode(json.as_bytes())
    );
    assert_eq!(
        direct_quota::decode_keyring_blob(encoded.as_bytes()).as_deref(),
        Some(json)
    );

    assert_eq!(direct_quota::decode_keyring_blob(b""), None);
    assert_eq!(direct_quota::decode_keyring_blob(b"   \0\0"), None);
    assert_eq!(
        direct_quota::decode_keyring_blob(b"go-keyring-base64:not-valid!"),
        None
    );
}

#[test]
fn direct_quota_parses_nested_and_flat_token_formats() {
    let nested = r#"{
            "token": {
                "access_token": "tok-1",
                "token_type": "Bearer",
                "refresh_token": "ref-1",
                "expiry": "2099-01-01T00:00:00Z"
            },
            "auth_method": "gcp",
            "project_id": "gcp-proj"
        }"#;
    let parsed = direct_quota::parse_stored_credential(nested).unwrap();
    assert_eq!(parsed.access_token, "tok-1");
    assert_eq!(parsed.refresh_token, "ref-1");
    assert_eq!(parsed.auth_method.as_deref(), Some("gcp"));
    assert_eq!(parsed.project_id.as_deref(), Some("gcp-proj"));
    assert!(parsed.expiry.is_some());
    assert!(!direct_quota::is_expired_or_missing_access_token(
        &parsed,
        chrono::Utc::now()
    ));

    let flat_zero_expiry = r#"{
            "access_token": "tok-2",
            "refresh_token": "ref-2",
            "expiry": "0001-01-01T00:00:00Z"
        }"#;
    let parsed_flat = direct_quota::parse_stored_credential(flat_zero_expiry).unwrap();
    assert_eq!(parsed_flat.access_token, "tok-2");
    assert_eq!(parsed_flat.refresh_token, "ref-2");
    assert_eq!(parsed_flat.expiry, None);
    assert!(!direct_quota::is_expired_or_missing_access_token(
        &parsed_flat,
        chrono::Utc::now()
    ));

    let expired = direct_quota::parse_stored_credential(
        r#"{"access_token":"tok-3","refresh_token":"ref-3","expiry":"2020-01-01T00:00:00Z"}"#,
    )
    .unwrap();
    assert!(direct_quota::is_expired_or_missing_access_token(
        &expired,
        chrono::Utc::now()
    ));

    assert_eq!(
        direct_quota::parse_stored_credential(
            r#"{"token":{"access_token":"","refresh_token":""}}"#
        ),
        None
    );
}

#[test]
fn direct_quota_storage_precedence_and_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = LoginShellEnv::default();
    env.vars.insert(
        "USERPROFILE".into(),
        dir.path().to_string_lossy().into_owned(),
    );
    env.vars
        .insert("HOME".into(), dir.path().to_string_lossy().into_owned());

    let cli_dir = dir.path().join(".gemini").join("antigravity-cli");
    let gemini_dir = dir.path().join(".gemini");
    std::fs::create_dir_all(cli_dir.join("cache")).unwrap();

    let disk_cli = cli_dir.join("antigravity-oauth-token");
    std::fs::write(
        &disk_cli,
        r#"{"token":{"access_token":"disk-cli","refresh_token":"ref-disk"}}"#,
    )
    .unwrap();
    std::fs::write(
        gemini_dir.join("jetski-standalone-oauth-token"),
        r#"{"access_token":"disk-legacy","refresh_token":"ref-legacy"}"#,
    )
    .unwrap();

    // Keyring wins when available and no marker is present.
    let cred = direct_quota::load_stored_credential(&env, || {
        Ok(Some(
            r#"{"token":{"access_token":"keyring-tok","refresh_token":"ref-key"}}"#.into(),
        ))
    })
    .unwrap();
    assert_eq!(cred.access_token, "keyring-tok");

    // Fallback to cli disk file when keyring returns None.
    let cred = direct_quota::load_stored_credential(&env, || Ok(None)).unwrap();
    assert_eq!(cred.access_token, "disk-cli");

    // Recent marker (< 1h) skips keyring even if keyring would return a value.
    let marker_path = cli_dir
        .join("cache")
        .join("antigravity-keyring-unavailable");
    std::fs::write(&marker_path, "1").unwrap();
    let cred = direct_quota::load_stored_credential(&env, || {
        Ok(Some(
            r#"{"token":{"access_token":"keyring-tok","refresh_token":"ref-key"}}"#.into(),
        ))
    })
    .unwrap();
    assert_eq!(cred.access_token, "disk-cli");

    // Stale marker (>= 1h) does not suppress keyring read, and Runner never deletes it.
    std::fs::File::options()
        .write(true)
        .open(&marker_path)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - Duration::from_secs(7200))
        .unwrap();
    let cred = direct_quota::load_stored_credential(&env, || {
        Ok(Some(
            r#"{"token":{"access_token":"keyring-tok","refresh_token":"ref-key"}}"#.into(),
        ))
    })
    .unwrap();
    assert_eq!(cred.access_token, "keyring-tok");
    assert!(marker_path.exists());

    // Removing primary disk file falls back to legacy standalone token file.
    std::fs::remove_file(&disk_cli).unwrap();
    let cred = direct_quota::load_stored_credential(&env, || Ok(None)).unwrap();
    assert_eq!(cred.access_token, "disk-legacy");

    // Removing all token files yields SignIn.
    std::fs::remove_file(gemini_dir.join("jetski-standalone-oauth-token")).unwrap();
    assert_eq!(
        direct_quota::load_stored_credential(&env, || Ok(None)),
        Err(UnavailableReason::SignIn)
    );
}

#[test]
fn direct_quota_reuses_unexpired_token_and_maps_windows() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = LoginShellEnv::default();
    env.vars.insert(
        "USERPROFILE".into(),
        dir.path().to_string_lossy().into_owned(),
    );
    env.vars
        .insert("HOME".into(), dir.path().to_string_lossy().into_owned());

    let (base_url, rx, server) = spawn_fake_http_server(2, |_idx, req| {
        match req.path.as_str() {
            "/v1internal:loadCodeAssist" => (
                200,
                "OK",
                serde_json::json!({ "cloudaicompanionProject": "projects/live-proj" }).to_string(),
            ),
            "/v1internal:retrieveUserQuotaSummary" => (
                200,
                "OK",
                serde_json::json!({
                    "groups": [
                        {
                            "displayName": "Gemini Models",
                            "buckets": [
                                {"window": "weekly", "remainingFraction": 0.52, "resetTime": "2026-11-18T00:00:00Z"},
                                {"window": "5h", "remainingFraction": 0.82, "resetTime": "2026-11-13T18:00:00Z"},
                                {"window": "daily", "remainingAmount": "10", "disabled": false},
                                {"window": "5h", "remainingFraction": 0.0, "disabled": true}
                            ]
                        },
                        {
                            "displayName": "Claude and GPT models",
                            "buckets": [
                                {"window": "weekly", "remainingFraction": 0.18, "resetTime": "2026-11-19T00:00:00Z"},
                                {"window": "5h", "remainingFraction": 0.65, "resetTime": "2026-11-13T16:00:00Z"}
                            ]
                        }
                    ]
                })
                .to_string(),
            ),
            other => panic!("unexpected path: {other}"),
        }
    });

    let reading = direct_quota::fetch_with_options(
        &env,
        Duration::from_secs(2),
        Duration::from_secs(2),
        Some(direct_quota::EndpointConfig {
            oauth_token_url: &format!("{base_url}/oauth2/token"),
            cloud_code_url: &base_url,
        }),
        || {
            Ok(Some(
                serde_json::json!({
                    "token": {
                        "access_token": "unexpired-secret",
                        "refresh_token": "refresh-secret",
                        "expiry": "2099-01-01T00:00:00Z"
                    },
                    "auth_method": "consumer"
                })
                .to_string(),
            ))
        },
    )
    .unwrap();

    server.join().unwrap();
    let reqs: Vec<_> = rx.try_iter().collect();
    assert_eq!(reqs.len(), 2);
    assert_eq!(reqs[0].path, "/v1internal:loadCodeAssist");
    assert_eq!(
        reqs[0].headers.get("authorization").map(String::as_str),
        Some("Bearer unexpired-secret")
    );
    assert_eq!(
        reqs[0].headers.get("user-agent").map(String::as_str),
        Some(concat!(
            "Runner/",
            env!("CARGO_PKG_VERSION"),
            " (antigravity/cli/1.5.9)"
        ))
    );
    assert_eq!(
        reqs[1].headers.get("user-agent").map(String::as_str),
        Some(concat!(
            "Runner/",
            env!("CARGO_PKG_VERSION"),
            " (antigravity/cli/1.5.9)"
        ))
    );
    assert_eq!(
        serde_json::from_str::<Value>(&reqs[0].body).unwrap(),
        serde_json::json!({"metadata": {"ideType": "ANTIGRAVITY"}})
    );
    assert_eq!(reqs[1].path, "/v1internal:retrieveUserQuotaSummary");
    assert_eq!(
        serde_json::from_str::<Value>(&reqs[1].body).unwrap(),
        serde_json::json!({"project": "projects/live-proj"})
    );

    assert_eq!(reading.windows.len(), 4);
    assert_eq!(reading.windows[0].name, "Gemini Models · 5 hours used");
    assert!((reading.windows[0].used_percent - 18.0).abs() < 1e-6);
    assert_eq!(reading.windows[1].name, "Gemini Models · Week used");
    assert!((reading.windows[1].used_percent - 48.0).abs() < 1e-6);
    assert_eq!(
        reading.windows[2].name,
        "Claude and GPT models · 5 hours used"
    );
    assert!((reading.windows[2].used_percent - 35.0).abs() < 1e-6);
    assert_eq!(reading.windows[3].name, "Claude and GPT models · Week used");
    assert!((reading.windows[3].used_percent - 82.0).abs() < 1e-6);
}

#[test]
fn direct_quota_refreshes_expired_token_in_memory_without_writing_disk() {
    let dir = tempfile::tempdir().unwrap();
    let mut env = LoginShellEnv::default();
    env.vars.insert(
        "USERPROFILE".into(),
        dir.path().to_string_lossy().into_owned(),
    );
    env.vars
        .insert("HOME".into(), dir.path().to_string_lossy().into_owned());

    let token_path = dir
        .path()
        .join(".gemini")
        .join("antigravity-cli")
        .join("antigravity-oauth-token");
    std::fs::create_dir_all(token_path.parent().unwrap()).unwrap();
    let original_contents = serde_json::json!({
        "token": {
            "access_token": "stale-token",
            "refresh_token": "refresh-secret",
            "expiry": "2020-01-01T00:00:00Z"
        },
        "auth_method": "gcp",
        "project_id": "fallback-gcp-proj"
    })
    .to_string();
    std::fs::write(&token_path, &original_contents).unwrap();

    let (base_url, rx, server) = spawn_fake_http_server(3, |_idx, req| match req.path.as_str() {
        "/oauth2/token" => (
            200,
            "OK",
            serde_json::json!({
                "access_token": "fresh-in-memory-token",
                "refresh_token": "rotated-not-saved",
                "expires_in": 3600
            })
            .to_string(),
        ),
        "/v1internal:loadCodeAssist" => (200, "OK", "{}".into()),
        "/v1internal:retrieveUserQuotaSummary" => (
            200,
            "OK",
            serde_json::json!({
                "groups": [{
                    "displayName": "Gemini Models",
                    "buckets": [{"window": "5h", "remainingFraction": 0.9}]
                }]
            })
            .to_string(),
        ),
        other => panic!("unexpected path: {other}"),
    });

    let gcp_creds = direct_quota::oauth_client_for_method(Some("gcp"));
    let consumer_creds = direct_quota::oauth_client_for_method(None);
    assert_ne!(gcp_creds.client_id, consumer_creds.client_id);
    assert_ne!(gcp_creds.client_secret, consumer_creds.client_secret);
    assert_eq!(
        gcp_creds.default_cloud_code_url,
        "https://cloudcode-pa.googleapis.com"
    );
    assert_eq!(
        consumer_creds.default_cloud_code_url,
        "https://daily-cloudcode-pa.googleapis.com"
    );

    let reading = direct_quota::fetch_with_options(
        &env,
        Duration::from_secs(2),
        Duration::from_secs(2),
        Some(direct_quota::EndpointConfig {
            oauth_token_url: &format!("{base_url}/oauth2/token"),
            cloud_code_url: &base_url,
        }),
        || Ok(None),
    )
    .unwrap();

    server.join().unwrap();
    let reqs: Vec<_> = rx.try_iter().collect();
    assert_eq!(reqs.len(), 3);
    assert_eq!(reqs[0].path, "/oauth2/token");
    assert_eq!(
        reqs[0].headers.get("user-agent").map(String::as_str),
        Some(concat!("Runner/", env!("CARGO_PKG_VERSION")))
    );
    assert!(reqs[0].body.contains("grant_type=refresh_token"));
    assert!(reqs[0].body.contains("refresh_token=refresh-secret"));
    assert!(reqs[0].body.contains("client_id=884354919052"));
    assert_eq!(
        reqs[1].headers.get("user-agent").map(String::as_str),
        Some(concat!(
            "Runner/",
            env!("CARGO_PKG_VERSION"),
            " (antigravity/cli/1.5.9)"
        ))
    );
    assert_eq!(
        reqs[1].headers.get("authorization").map(String::as_str),
        Some("Bearer fresh-in-memory-token")
    );
    assert_eq!(
        serde_json::from_str::<Value>(&reqs[2].body).unwrap(),
        serde_json::json!({"project": "fallback-gcp-proj"})
    );
    assert_eq!(reading.windows.len(), 1);
    assert!((reading.windows[0].used_percent - 10.0).abs() < 1e-6);

    // Credential on disk must remain strictly unmodified.
    assert_eq!(
        std::fs::read_to_string(&token_path).unwrap(),
        original_contents
    );
}

#[test]
fn direct_quota_retries_once_after_401_with_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let env = isolated_direct_quota_env(dir.path());

    let (base_url, rx, server) = spawn_fake_http_server(4, |_idx, req| match req.path.as_str() {
        "/v1internal:loadCodeAssist"
            if req.headers.get("authorization").map(String::as_str) == Some("Bearer old-tok") =>
        {
            (401, "Unauthorized", r#"{"error":{"code":401}}"#.into())
        }
        "/oauth2/token" => (
            200,
            "OK",
            serde_json::json!({"access_token": "retried-tok"}).to_string(),
        ),
        "/v1internal:loadCodeAssist" => (
            200,
            "OK",
            serde_json::json!({"cloudaicompanionProject": "proj-after-retry"}).to_string(),
        ),
        "/v1internal:retrieveUserQuotaSummary" => (
            200,
            "OK",
            serde_json::json!({
                "groups": [{
                    "displayName": "Gemini Models",
                    "buckets": [{"window": "weekly", "remainingFraction": 0.25}]
                }]
            })
            .to_string(),
        ),
        other => panic!("unexpected path: {other}"),
    });

    let reading = direct_quota::fetch_with_options(
        &env,
        Duration::from_secs(2),
        Duration::from_secs(2),
        Some(direct_quota::EndpointConfig {
            oauth_token_url: &format!("{base_url}/oauth2/token"),
            cloud_code_url: &base_url,
        }),
        || {
            Ok(Some(
                r#"{"access_token":"old-tok","refresh_token":"ref-tok"}"#.into(),
            ))
        },
    )
    .unwrap();

    server.join().unwrap();
    assert_eq!(rx.try_iter().count(), 4);
    assert_eq!(reading.windows[0].name, "Gemini Models · Week used");
    assert!((reading.windows[0].used_percent - 75.0).abs() < 1e-6);
}

#[test]
fn direct_quota_classifies_auth_rejections_as_sign_in() {
    let dir = tempfile::tempdir().unwrap();
    let env = isolated_direct_quota_env(dir.path());

    // Missing credential -> SignIn
    assert_eq!(
        direct_quota::fetch_with_options(
            &env,
            Duration::from_secs(1),
            Duration::from_secs(1),
            None,
            || Ok(None),
        ),
        Err(UnavailableReason::SignIn)
    );

    // OAuth invalid_grant (400) -> SignIn
    let (base_url, _rx, server) = spawn_fake_http_server(1, |_, _| {
        (400, "Bad Request", r#"{"error":"invalid_grant"}"#.into())
    });
    assert_eq!(
        direct_quota::fetch_with_options(
            &env,
            Duration::from_secs(2),
            Duration::from_secs(2),
            Some(direct_quota::EndpointConfig {
                oauth_token_url: &format!("{base_url}/token"),
                cloud_code_url: &base_url,
            }),
            || {
                Ok(Some(
                    r#"{"access_token":"","refresh_token":"revoked"}"#.into(),
                ))
            },
        ),
        Err(UnavailableReason::SignIn)
    );
    server.join().unwrap();

    // Cloud Code 401 when refresh retry also gets 401 -> SignIn
    let (base_url, _rx, server) = spawn_fake_http_server(2, |_, req| {
        if req.path == "/token" {
            (200, "OK", r#"{"access_token":"still-rejected"}"#.into())
        } else {
            (401, "Unauthorized", "{}".into())
        }
    });
    assert_eq!(
        direct_quota::fetch_with_options(
            &env,
            Duration::from_secs(2),
            Duration::from_secs(2),
            Some(direct_quota::EndpointConfig {
                oauth_token_url: &format!("{base_url}/token"),
                cloud_code_url: &base_url,
            }),
            || {
                Ok(Some(
                    r#"{"access_token":"","refresh_token":"valid-ref"}"#.into(),
                ))
            },
        ),
        Err(UnavailableReason::SignIn)
    );
    server.join().unwrap();
}

#[test]
fn direct_quota_classifies_transient_and_malformed_failures_as_no_answer() {
    let dir = tempfile::tempdir().unwrap();
    let env = isolated_direct_quota_env(dir.path());

    for (status, reason, body) in [
        (429, "Too Many Requests", "{}"),
        (500, "Internal Server Error", "{}"),
    ] {
        let body = body.to_owned();
        let (base_url, _rx, server) =
            spawn_fake_http_server(1, move |_, _| (status, reason, body.clone()));
        assert_eq!(
            direct_quota::fetch_with_options(
                &env,
                Duration::from_secs(2),
                Duration::from_secs(2),
                Some(direct_quota::EndpointConfig {
                    oauth_token_url: &format!("{base_url}/token"),
                    cloud_code_url: &base_url,
                }),
                || { Ok(Some(r#"{"access_token":"","refresh_token":"ref"}"#.into(),)) },
            ),
            Err(UnavailableReason::AntigravityNoAnswer)
        );
        server.join().unwrap();
    }

    // Unparseable / empty quota response -> AntigravityNoAnswer
    for quota_body in [
        "not-json",
        "{}",
        r#"{"groups":[{"displayName":"Gemini Models","buckets":[{"window":"5h","remainingFraction":1.5}]}]}"#,
    ] {
        let quota_body = quota_body.to_owned();
        let (base_url, _rx, server) =
            spawn_fake_http_server(2, move |_, req| match req.path.as_str() {
                "/v1internal:loadCodeAssist" => (200, "OK", "{}".into()),
                _ => (200, "OK", quota_body.clone()),
            });
        assert_eq!(
            direct_quota::fetch_with_options(
                &env,
                Duration::from_secs(2),
                Duration::from_secs(2),
                Some(direct_quota::EndpointConfig {
                    oauth_token_url: &format!("{base_url}/token"),
                    cloud_code_url: &base_url,
                }),
                || {
                    Ok(Some(
                        r#"{"access_token":"tok","expiry":"2099-01-01T00:00:00Z"}"#.into(),
                    ))
                },
            ),
            Err(UnavailableReason::AntigravityNoAnswer)
        );
        server.join().unwrap();
    }

    // Slow server exceeds usage_timeout -> AntigravityNoAnswer
    let (base_url, _rx, server) = spawn_fake_http_server(1, |_, _| {
        std::thread::sleep(Duration::from_millis(350));
        (200, "OK", "{}".into())
    });
    assert_eq!(
        direct_quota::fetch_with_options(
            &env,
            Duration::from_millis(120),
            Duration::from_millis(120),
            Some(direct_quota::EndpointConfig {
                oauth_token_url: &format!("{base_url}/token"),
                cloud_code_url: &base_url,
            }),
            || {
                Ok(Some(
                    r#"{"access_token":"tok","expiry":"2099-01-01T00:00:00Z"}"#.into(),
                ))
            },
        ),
        Err(UnavailableReason::AntigravityNoAnswer)
    );
    server.join().unwrap();

    // Cloud Code 429 / 503 -> AntigravityNoAnswer
    for (status, reason) in [(429, "Too Many Requests"), (503, "Service Unavailable")] {
        let (base_url, _rx, server) =
            spawn_fake_http_server(1, move |_, _| (status, reason, "{}".into()));
        assert_eq!(
            direct_quota::fetch_with_options(
                &env,
                Duration::from_secs(2),
                Duration::from_secs(2),
                Some(direct_quota::EndpointConfig {
                    oauth_token_url: &format!("{base_url}/token"),
                    cloud_code_url: &base_url,
                }),
                || {
                    Ok(Some(
                        r#"{"access_token":"tok","expiry":"2099-01-01T00:00:00Z"}"#.into(),
                    ))
                },
            ),
            Err(UnavailableReason::AntigravityNoAnswer)
        );
        server.join().unwrap();
    }
}

#[test]
fn direct_quota_respects_antigravity_gemini_dir_override() {
    let dir = tempfile::tempdir().unwrap();
    let home_dir = dir.path().join("home");
    let real_cli_dir = home_dir.join(".gemini").join("antigravity-cli");
    let custom_dir = dir.path().join("custom-gemini");
    std::fs::create_dir_all(&real_cli_dir).unwrap();
    std::fs::create_dir_all(&custom_dir).unwrap();
    // Real HOME credential must NOT leak when ANTIGRAVITY_GEMINI_DIR is overridden.
    std::fs::write(
        real_cli_dir.join("antigravity-oauth-token"),
        r#"{"access_token":"real-home-leak","refresh_token":"real-home-ref"}"#,
    )
    .unwrap();

    let mut env = LoginShellEnv::default();
    env.vars
        .insert("HOME".into(), home_dir.to_string_lossy().into_owned());
    env.vars.insert(
        "ANTIGRAVITY_GEMINI_DIR".into(),
        custom_dir.to_string_lossy().into_owned(),
    );

    assert_eq!(
        direct_quota::load_stored_credential(&env, || Ok(None)),
        Err(UnavailableReason::SignIn)
    );

    // Supports `<ANTIGRAVITY_GEMINI_DIR>/antigravity-oauth-token`.
    let flat_tok = custom_dir.join("antigravity-oauth-token");
    std::fs::write(
        &flat_tok,
        r#"{"access_token":"custom-flat","refresh_token":"custom-ref"}"#,
    )
    .unwrap();
    let cred = direct_quota::load_stored_credential(&env, || Ok(None)).unwrap();
    assert_eq!(cred.access_token, "custom-flat");

    // Prefers `<ANTIGRAVITY_GEMINI_DIR>/antigravity-cli/antigravity-oauth-token` (agy AppDataDir).
    let nested_dir = custom_dir.join("antigravity-cli");
    std::fs::create_dir_all(&nested_dir).unwrap();
    std::fs::write(
        nested_dir.join("antigravity-oauth-token"),
        r#"{"access_token":"custom-cli","refresh_token":"custom-ref"}"#,
    )
    .unwrap();
    let cred = direct_quota::load_stored_credential(&env, || Ok(None)).unwrap();
    assert_eq!(cred.access_token, "custom-cli");
}

fn isolated_direct_quota_env(dir: &std::path::Path) -> LoginShellEnv {
    let dir_str = dir.to_string_lossy().into_owned();
    let mut env = LoginShellEnv::default();
    env.vars.insert("HOME".into(), dir_str.clone());
    env.vars.insert("USERPROFILE".into(), dir_str.clone());
    env.vars.insert("ANTIGRAVITY_GEMINI_DIR".into(), dir_str);
    env
}

#[test]
fn direct_quota_does_not_follow_redirects_with_secrets() {
    use std::io::{ErrorKind, Read, Write};
    use std::net::TcpListener;

    let dir = tempfile::tempdir().unwrap();
    let env = isolated_direct_quota_env(dir.path());

    let sink_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    sink_listener.set_nonblocking(true).unwrap();
    let sink_addr = sink_listener.local_addr().unwrap();

    let spawn_redirect_once = |target_location: String| {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 2048];
            let _ = stream.read(&mut buf);
            let reply = format!(
                    "HTTP/1.1 308 Permanent Redirect\r\nLocation: {target_location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
            let _ = stream.write_all(reply.as_bytes());
            let _ = stream.flush();
        });
        (format!("http://{addr}"), handle)
    };

    // Case 1: /oauth2/token redirect must not forward refresh_token/client_secret.
    let (redir_oauth, server1) = spawn_redirect_once(format!("http://{sink_addr}/steal-oauth"));
    assert_eq!(
        direct_quota::fetch_with_options(
            &env,
            Duration::from_secs(2),
            Duration::from_secs(2),
            Some(direct_quota::EndpointConfig {
                oauth_token_url: &format!("{redir_oauth}/oauth2/token"),
                cloud_code_url: &redir_oauth,
            }),
            || Ok(Some(
                r#"{"access_token":"","refresh_token":"secret-refresh"}"#.into(),
            )),
        ),
        Err(UnavailableReason::AntigravityNoAnswer)
    );
    server1.join().unwrap();
    assert_eq!(
        sink_listener.accept().unwrap_err().kind(),
        ErrorKind::WouldBlock
    );

    // Case 2: Cloud Code redirect must not forward Bearer access_token.
    let (redir_cloud, server2) = spawn_redirect_once(format!("http://{sink_addr}/steal-bearer"));
    assert_eq!(
        direct_quota::fetch_with_options(
            &env,
            Duration::from_secs(2),
            Duration::from_secs(2),
            Some(direct_quota::EndpointConfig {
                oauth_token_url: &format!("{redir_cloud}/oauth2/token"),
                cloud_code_url: &redir_cloud,
            }),
            || Ok(Some(
                r#"{"access_token":"secret-bearer","expiry":"2099-01-01T00:00:00Z"}"#.into(),
            )),
        ),
        Err(UnavailableReason::AntigravityNoAnswer)
    );
    server2.join().unwrap();
    assert_eq!(
        sink_listener.accept().unwrap_err().kind(),
        ErrorKind::WouldBlock
    );
}

#[test]
fn direct_quota_preserves_inherited_proxy_and_no_proxy() {
    use std::io::{ErrorKind, Read, Write};
    use std::net::TcpListener;

    let dir = tempfile::tempdir().unwrap();
    let creds = || {
        Ok(Some(
            r#"{"access_token":"proxy-tok","expiry":"2099-01-01T00:00:00Z"}"#.to_owned(),
        ))
    };
    let quota_reply = |path: &str| {
        if path.ends_with("/v1internal:loadCodeAssist") {
            (
                200,
                "OK",
                r#"{"cloudaicompanionProject":"projects/p"}"#.into(),
            )
        } else {
            (
                    200,
                    "OK",
                    r#"{"groups":[{"displayName":"Gemini Models","buckets":[{"window":"5h","remainingFraction":0.6}]}]}"#.into(),
                )
        }
    };

    // Case 1: Empty proxy vars in LoginShellEnv still honor inherited HTTP_PROXY.
    // Pin HOME/USERPROFILE/ANTIGRAVITY_GEMINI_DIR to an empty tempdir so local host markers or tokens never leak into the test.
    let base_env = isolated_direct_quota_env(dir.path());
    let unused_origin = TcpListener::bind("127.0.0.1:0").unwrap();
    unused_origin.set_nonblocking(true).unwrap();
    let origin_addr = unused_origin.local_addr().unwrap();
    let (proxy_url, proxy_rx, proxy_server) =
        spawn_fake_http_server(2, move |_, req| quota_reply(&req.path));
    let reading = direct_quota::fetch_with_options_with_proxy(
        &base_env,
        Duration::from_secs(2),
        Duration::from_secs(2),
        Some(direct_quota::EndpointConfig {
            oauth_token_url: &format!("http://{origin_addr}/oauth2/token"),
            cloud_code_url: &format!("http://{origin_addr}"),
        }),
        &|key| (key == "HTTP_PROXY").then(|| proxy_url.clone()),
        false,
        creds,
    )
    .unwrap();
    proxy_server.join().unwrap();
    assert!((reading.windows[0].used_percent - 40.0).abs() < 1e-6);
    let proxy_reqs: Vec<_> = proxy_rx.try_iter().collect();
    assert_eq!(proxy_reqs.len(), 2);
    assert_eq!(
        proxy_reqs[0].path,
        format!("http://{origin_addr}/v1internal:loadCodeAssist")
    );
    assert_eq!(
        unused_origin.accept().unwrap_err().kind(),
        ErrorKind::WouldBlock
    );

    // Case 2: Inherited NO_PROXY bypasses inherited HTTP_PROXY.
    let unused_proxy = TcpListener::bind("127.0.0.1:0").unwrap();
    unused_proxy.set_nonblocking(true).unwrap();
    let unused_proxy_url = format!("http://{}", unused_proxy.local_addr().unwrap());
    let (origin_url, origin_rx, origin_server) =
        spawn_fake_http_server(2, move |_, req| quota_reply(&req.path));
    let reading = direct_quota::fetch_with_options_with_proxy(
        &base_env,
        Duration::from_secs(2),
        Duration::from_secs(2),
        Some(direct_quota::EndpointConfig {
            oauth_token_url: &format!("{origin_url}/oauth2/token"),
            cloud_code_url: &origin_url,
        }),
        &|key| match key {
            "HTTP_PROXY" => Some(unused_proxy_url.clone()),
            "NO_PROXY" => Some("127.0.0.1".into()),
            _ => None,
        },
        false,
        creds,
    )
    .unwrap();
    origin_server.join().unwrap();
    assert!((reading.windows[0].used_percent - 40.0).abs() < 1e-6);
    assert_eq!(origin_rx.try_iter().count(), 2);
    assert_eq!(
        unused_proxy.accept().unwrap_err().kind(),
        ErrorKind::WouldBlock
    );

    // Case 3: env.vars (including lowercase `http_proxy`) overrides inherited `HTTP_PROXY`.
    let (override_proxy_url, override_rx, override_server) =
        spawn_fake_http_server(2, move |_, req| quota_reply(&req.path));
    let mut env_override = base_env.clone();
    env_override
        .vars
        .insert("http_proxy".into(), override_proxy_url);
    let reading = direct_quota::fetch_with_options_with_proxy(
        &env_override,
        Duration::from_secs(2),
        Duration::from_secs(2),
        Some(direct_quota::EndpointConfig {
            oauth_token_url: &format!("http://{origin_addr}/oauth2/token"),
            cloud_code_url: &format!("http://{origin_addr}"),
        }),
        &|key| (key == "HTTP_PROXY").then(|| "http://127.0.0.1:1".into()),
        false,
        creds,
    )
    .unwrap();
    override_server.join().unwrap();
    assert!((reading.windows[0].used_percent - 40.0).abs() < 1e-6);
    assert_eq!(override_rx.try_iter().count(), 2);

    // Case 4: Inherited HTTPS_PROXY and ALL_PROXY tunnel HTTPS targets via CONNECT.
    for var_name in ["HTTPS_PROXY", "ALL_PROXY"] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req_line = String::from_utf8_lossy(&buf[..n])
                .lines()
                .next()
                .unwrap_or_default()
                .to_owned();
            let _ = tx.send(req_line);
            let _ = stream.write_all(
                b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
            let _ = stream.flush();
        });

        let result = direct_quota::fetch_with_options_with_proxy(
            &base_env,
            Duration::from_secs(2),
            Duration::from_secs(2),
            Some(direct_quota::EndpointConfig {
                oauth_token_url: "https://oauth.example.invalid/token",
                cloud_code_url: "https://cloudcode.example.invalid",
            }),
            &|key| (key == var_name).then(|| format!("http://{proxy_addr}")),
            false,
            creds,
        );
        assert_eq!(result, Err(UnavailableReason::AntigravityNoAnswer));
        server.join().unwrap();
        let connect_line = rx.recv().unwrap();
        assert!(
            connect_line.starts_with("CONNECT cloudcode.example.invalid:443 "),
            "{var_name} expected CONNECT request, got: {connect_line}"
        );
    }
}
