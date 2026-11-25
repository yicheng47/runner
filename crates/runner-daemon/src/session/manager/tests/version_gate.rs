use super::*;

#[test]
fn known_old_runtime_is_refused_before_every_launch_path_and_unknown_is_allowed() {
    let pool = pool_with_schema();
    let data = tempfile::tempdir().unwrap();
    let fake = fake_runtime();
    let manager = mgr_with_fake(None, fake.clone());
    let mut role = role("fixture-codex", &[]);
    role.runtime = "codex".into();
    let source = manager
        .spawn_direct(
            &role,
            DirectLaunch {
                cwd: Some(fixture_tmp_dir().to_str().unwrap().into()),
                runtime_direct: true,
                ..Default::default()
            },
            data.path(),
            pool.clone(),
            capture(),
        )
        .unwrap();
    manager.kill(&source.id).unwrap();
    pool.get()
        .unwrap()
        .execute(
            "UPDATE sessions SET agent_session_key='source-key' WHERE id=?1",
            [&source.id],
        )
        .unwrap();
    manager
        .discovery_state
        .write()
        .unwrap()
        .versions
        .set_installed(Runtime::Codex, &role.command, Some("0.163.0"));
    let check = |error: Error| {
        let Error::RuntimeVersionGate(gate) = error else {
            panic!("expected typed version gate, got {error}")
        };
        assert_eq!(gate.runtime, Runtime::Codex);
        assert_eq!(gate.installed_version, "0.163.0");
        assert_eq!(gate.min_version, "0.164.0");
    };
    check(
        manager
            .spawn_direct(
                &role,
                DirectLaunch {
                    runtime_direct: true,
                    ..Default::default()
                },
                data.path(),
                pool.clone(),
                capture(),
            )
            .unwrap_err(),
    );
    check(
        manager
            .spawn_direct(
                &role,
                DirectLaunch::default(),
                data.path(),
                pool.clone(),
                capture(),
            )
            .unwrap_err(),
    );
    check(
        manager
            .resume(&source.id, None, None, data.path(), pool.clone(), capture())
            .unwrap_err(),
    );
    check(
        manager
            .resume_on_launch(&source.id, None, None, data.path(), pool.clone(), capture())
            .unwrap_err(),
    );
    check(
        manager
            .spawn_fork(
                &source.id,
                None,
                None,
                None,
                data.path(),
                pool.clone(),
                capture(),
            )
            .unwrap_err(),
    );
    match manager.register_mission_session(
        &mission(),
        &role,
        &slot_for(&role),
        data.path(),
        data.path().join("events.ndjson"),
        pool.clone(),
        None,
        None,
        None,
        "test",
    ) {
        Err(error) => check(error),
        Ok(_) => panic!("old runtime registered a mission launch"),
    }
    manager
        .discovery_state
        .write()
        .unwrap()
        .versions
        .set_installed(Runtime::Codex, &role.command, Some("0.164.0"));
    assert!(manager.resolve_role_executable(&role, &pool).is_ok());
    manager
        .discovery_state
        .write()
        .unwrap()
        .versions
        .set_installed(Runtime::Codex, &role.command, None);
    assert!(manager.resolve_role_executable(&role, &pool).is_ok());
}

#[test]
fn gate_remains_typed_after_daemon_dispatch_and_wire_roundtrip() {
    let gate = runner_core::protocol::RuntimeVersionGate {
        runtime: Runtime::Codex,
        installed_version: "0.163.0".into(),
        min_version: "0.164.0".into(),
    };
    let error = Error::RuntimeVersionGate(gate.clone()).into_client_error();
    let wire = serde_json::to_vec(&error).unwrap();
    let client: runner_core::protocol::ClientError = serde_json::from_slice(&wire).unwrap();
    assert_eq!(client.version_gate, Some(gate));
    assert!(client.to_string().contains("Settings → Agents"));
    let old: runner_core::protocol::ClientError =
        serde_json::from_str(r#"{"message":"old error"}"#).unwrap();
    assert!(old.version_gate.is_none());
}

#[test]
fn public_start_api_preserves_the_gate_through_operation_context() {
    let core = crate::test_support::test_core();
    crate::test_support::isolate_runtime_path(&core, None);
    let dir = tempfile::tempdir().unwrap();
    let executable = dir.path().join("codex-fixture.exe");
    std::fs::write(&executable, "fixture").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    crate::db::set_runtime_override(&core.db, "codex", executable.to_str()).unwrap();
    core.runtime_discovery
        .write()
        .unwrap()
        .versions
        .set_installed(
            Runtime::Codex,
            executable.to_str().unwrap(),
            Some("0.147.0"),
        );
    let client = crate::daemon::InProcessTransport::client(core);
    let error = client
        .session_start_runtime_with_speed(
            "codex",
            runner_core::protocol::ProjectScope::Root,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap_err();
    assert_eq!(error.version_gate.unwrap().installed_version, "0.147.0");
}

#[test]
#[cfg(unix)]
fn unprobed_opencode_v1_is_probed_synchronously_and_refused_on_start_and_resume() {
    use std::os::unix::fs::PermissionsExt;
    let pool = pool_with_schema();
    let data = tempfile::tempdir().unwrap();
    let bin_dir = tempfile::tempdir().unwrap();
    let bin = bin_dir.path().join("opencode");
    std::fs::write(&bin, "#!/bin/sh\necho '2.1.0'\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();

    let fake = fake_runtime();
    let manager = mgr_with_fake(None, fake.clone());
    *manager.shell_env.write().unwrap() =
        crate::test_support::isolated_shell_env(Some(bin_dir.path()));
    manager.discovery_state.write().unwrap().checking = false;

    let mut role = role("opencode", &[]);
    role.runtime = "opencode".into();
    let started = manager
        .spawn_direct(
            &role,
            DirectLaunch {
                cwd: Some(fixture_tmp_dir().to_str().unwrap().into()),
                runtime_direct: true,
                ..Default::default()
            },
            data.path(),
            pool.clone(),
            capture(),
        )
        .unwrap();
    manager.kill(&started.id).unwrap();
    pool.get()
        .unwrap()
        .execute(
            "UPDATE sessions SET agent_session_key='ses_abc123' WHERE id=?1",
            [&started.id],
        )
        .unwrap();

    // Downgrade binary on PATH to v1 and clear cached version probes to simulate cold launch/resume.
    std::fs::write(
        &bin,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo '1.19.5'; exit 0; fi\necho 'Usage: opencode [project]'\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    manager.discovery_state.write().unwrap().versions = Default::default();
    let spawns_before = fake.spawns.lock().unwrap().len();

    let Err(Error::RuntimeVersionGate(gate)) = manager.spawn_direct(
        &role,
        DirectLaunch {
            cwd: Some(fixture_tmp_dir().to_str().unwrap().into()),
            runtime_direct: true,
            ..Default::default()
        },
        data.path(),
        pool.clone(),
        capture(),
    ) else {
        panic!("expected RuntimeVersionGate on unprobed OpenCode v1 launch");
    };
    assert_eq!(gate.runtime, Runtime::OpenCode);
    assert_eq!(gate.installed_version, "1.19.5");
    assert_eq!(gate.min_version, "2.1.0");

    // Clear cache again and verify stopped-session resume also probes synchronously and gates cleanly.
    manager.discovery_state.write().unwrap().versions = Default::default();
    let Err(Error::RuntimeVersionGate(resume_gate)) = manager.resume(
        &started.id,
        None,
        None,
        data.path(),
        pool.clone(),
        capture(),
    ) else {
        panic!("expected RuntimeVersionGate on unprobed OpenCode v1 resume");
    };
    assert_eq!(resume_gate.runtime, Runtime::OpenCode);
    assert_eq!(resume_gate.installed_version, "1.19.5");
    assert_eq!(resume_gate.min_version, "2.1.0");
    assert_eq!(
        fake.spawns.lock().unwrap().len(),
        spawns_before,
        "must refuse v1 before spawning opencode --standalone"
    );
}

#[test]
fn claude_code_antigravity_pi_grok_and_opencode_enforce_osc7501_minimum_versions() {
    for (runtime, below, at_min, min_version) in [
        (Runtime::ClaudeCode, "2.1.294", "2.1.295", "2.1.295"),
        (Runtime::Antigravity, "1.4.7", "1.5.8", "1.5.8"),
        (Runtime::Pi, "1.0.9", "1.1.0", "1.1.0"),
        (Runtime::Grok, "1.0.88", "1.0.89", "1.0.89"),
        (Runtime::OpenCode, "2.0.29", "2.1.0", "2.1.0"),
    ] {
        let pool = pool_with_schema();
        let data = tempfile::tempdir().unwrap();
        let fake = fake_runtime();
        let manager = mgr_with_fake(None, fake);
        let mut agent_role = role(&format!("fixture-{}", runtime.key()), &[]);
        agent_role.runtime = runtime.key().into();

        manager
            .discovery_state
            .write()
            .unwrap()
            .versions
            .set_installed(runtime, &agent_role.command, Some(below));

        let err = manager
            .spawn_direct(
                &agent_role,
                DirectLaunch::default(),
                data.path(),
                pool.clone(),
                capture(),
            )
            .unwrap_err();
        let Error::RuntimeVersionGate(gate) = err else {
            panic!("{runtime:?} expected RuntimeVersionGate, got {err}");
        };
        assert_eq!(gate.runtime, runtime);
        assert_eq!(gate.installed_version, below);
        assert_eq!(gate.min_version, min_version);

        match manager.register_mission_session(
            &mission(),
            &agent_role,
            &slot_for(&agent_role),
            data.path(),
            data.path().join("events.ndjson"),
            pool.clone(),
            None,
            None,
            None,
            "test",
        ) {
            Err(Error::RuntimeVersionGate(mission_gate)) => {
                assert_eq!(mission_gate, gate);
            }
            Err(other) => panic!("{runtime:?} mission launch returned wrong error: {other}"),
            Ok(_) => panic!("{runtime:?} mission launch must gate old version"),
        }

        manager
            .discovery_state
            .write()
            .unwrap()
            .versions
            .set_installed(runtime, &agent_role.command, Some(at_min));
        assert!(
            manager.resolve_role_executable(&agent_role, &pool).is_ok(),
            "{runtime:?} at {at_min} must pass version gate"
        );

        manager
            .discovery_state
            .write()
            .unwrap()
            .versions
            .set_installed(runtime, &agent_role.command, None);
        assert!(
            manager.resolve_role_executable(&agent_role, &pool).is_ok(),
            "{runtime:?} unknown version remains allowed"
        );
    }
}

#[test]
#[cfg(unix)]
fn grok_and_opencode_v2_below_minimum_build_update_specs_while_opencode_v1_stays_withheld() {
    use std::os::unix::fs::PermissionsExt;

    for (runtime, bin_name, version, min_version, expected_args) in [
        (
            Runtime::Antigravity,
            "agy",
            "1.4.7",
            "1.5.8",
            vec!["update"],
        ),
        (Runtime::Grok, "grok", "1.0.88", "1.0.89", vec!["update"]),
        (
            Runtime::OpenCode,
            "opencode",
            "2.0.29",
            "2.1.0",
            vec!["upgrade"],
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join(bin_name);
        std::fs::write(&bin, format!("#!/bin/sh\necho '{version}'\n")).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();

        let core = crate::test_support::test_core();
        crate::test_support::isolate_runtime_path(&core, Some(dir.path()));
        crate::runtime_status::versions::probe_now(
            runtime,
            &core.db,
            &core.runtime_shell_env,
            &core.runtime_discovery,
            &core.events,
        );

        let status = crate::ops::runtime::runtime_status_list(&core).unwrap();
        let row = status
            .runtimes
            .iter()
            .find(|entry| entry.name == runtime)
            .unwrap();
        assert_eq!(
            row.state,
            runner_core::protocol::RuntimeRowState::Detected,
            "{runtime:?} {version} must stay Detected so Runner offers Update needed"
        );
        assert!(row.below_minimum);
        assert_eq!(row.installed_version.as_deref(), Some(version));
        assert_eq!(row.min_version.as_deref(), Some(min_version));

        let spec = crate::ops::runtime::runtime_update_spawn_spec(&core, runtime, (80, 24))
            .expect("below-minimum self-updatable runtime must build update spec");
        assert_eq!(spec.command, bin.display().to_string());
        assert_eq!(spec.args, expected_args);
        assert!(!spec
            .env
            .keys()
            .any(|key| key.eq_ignore_ascii_case("AGY_CLI_DISABLE_AUTO_UPDATE")));

        let err = crate::ops::session::session_start_direct(
            &core,
            crate::ops::session::DirectLaunch {
                runtime: Some(runtime.key().into()),
                scope: runner_core::protocol::ProjectScope::Root,
                cwd: Some(dir.path().to_str().unwrap().to_owned()),
                cols: Some(80),
                rows: Some(24),
                ..Default::default()
            },
        )
        .unwrap_err();
        let Error::RuntimeVersionGate(gate) = err else {
            panic!("{runtime:?} {version} launch expected RuntimeVersionGate, got {err}");
        };
        assert_eq!(gate.installed_version, version);
        assert_eq!(gate.min_version, min_version);
    }

    // OpenCode v1 (< 2.0.0) and prereleases of 2.0.0 (2.0.0-rc.1) stay withheld:
    // NotFound in status_list and refused by update spec.
    for withheld in ["1.19.5", "2.0.0-rc.1"] {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("opencode");
        std::fs::write(&bin, format!("#!/bin/sh\necho '{withheld}'\n")).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let core = crate::test_support::test_core();
        crate::test_support::isolate_runtime_path(&core, Some(dir.path()));
        crate::runtime_status::versions::probe_now(
            Runtime::OpenCode,
            &core.db,
            &core.runtime_shell_env,
            &core.runtime_discovery,
            &core.events,
        );
        let status = crate::ops::runtime::runtime_status_list(&core).unwrap();
        let row = status
            .runtimes
            .iter()
            .find(|entry| entry.name == Runtime::OpenCode)
            .unwrap();
        assert_eq!(
            row.state,
            runner_core::protocol::RuntimeRowState::NotFound,
            "OpenCode {withheld} must stay withheld as NotFound"
        );
        assert!(row.below_minimum);
        assert!(matches!(
            crate::ops::runtime::runtime_update_spawn_spec(&core, Runtime::OpenCode, (80, 24)),
            Err(Error::RuntimeVersionGate(gate)) if gate.installed_version == withheld
        ));
    }
}

#[test]
#[cfg(unix)]
fn unprobed_antigravity_below_minimum_is_probed_synchronously_and_refused_on_start_and_resume() {
    use std::os::unix::fs::PermissionsExt;
    let pool = pool_with_schema();
    let data = tempfile::tempdir().unwrap();
    let bin_dir = tempfile::tempdir().unwrap();
    let bin = bin_dir.path().join("agy");
    std::fs::write(&bin, "#!/bin/sh\necho '1.5.8'\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();

    let fake = fake_runtime();
    let manager = mgr_with_fake(None, fake.clone());
    *manager.shell_env.write().unwrap() =
        crate::test_support::isolated_shell_env(Some(bin_dir.path()));
    manager.discovery_state.write().unwrap().checking = false;

    let mut role = role("agy", &[]);
    role.runtime = "antigravity".into();
    let started = manager
        .spawn_direct(
            &role,
            DirectLaunch {
                cwd: Some(fixture_tmp_dir().to_str().unwrap().into()),
                runtime_direct: true,
                ..Default::default()
            },
            data.path(),
            pool.clone(),
            capture(),
        )
        .unwrap();
    manager.kill(&started.id).unwrap();
    pool.get()
        .unwrap()
        .execute(
            "UPDATE sessions SET agent_session_key='019fa1b9-a133-7841-b4dd-730d376ab1d1' WHERE id=?1",
            [&started.id],
        )
        .unwrap();

    // Downgrade agy on PATH to 1.4.7 and clear cached version probes.
    std::fs::write(&bin, "#!/bin/sh\necho '1.4.7'\n").unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    manager.discovery_state.write().unwrap().versions = Default::default();
    let spawns_before = fake.spawns.lock().unwrap().len();

    let Err(Error::RuntimeVersionGate(gate)) = manager.spawn_direct(
        &role,
        DirectLaunch {
            cwd: Some(fixture_tmp_dir().to_str().unwrap().into()),
            runtime_direct: true,
            ..Default::default()
        },
        data.path(),
        pool.clone(),
        capture(),
    ) else {
        panic!("expected RuntimeVersionGate on unprobed Antigravity 1.4.7 launch");
    };
    assert_eq!(gate.runtime, Runtime::Antigravity);
    assert_eq!(gate.installed_version, "1.4.7");
    assert_eq!(gate.min_version, "1.5.8");

    manager.discovery_state.write().unwrap().versions = Default::default();
    let Err(Error::RuntimeVersionGate(resume_gate)) = manager.resume(
        &started.id,
        None,
        None,
        data.path(),
        pool.clone(),
        capture(),
    ) else {
        panic!("expected RuntimeVersionGate on unprobed Antigravity 1.4.7 resume");
    };
    assert_eq!(resume_gate.runtime, Runtime::Antigravity);
    assert_eq!(resume_gate.installed_version, "1.4.7");
    assert_eq!(resume_gate.min_version, "1.5.8");

    manager.discovery_state.write().unwrap().versions = Default::default();
    let Err(Error::RuntimeVersionGate(relaunch_gate)) = manager.resume_on_launch(
        &started.id,
        None,
        None,
        data.path(),
        pool.clone(),
        capture(),
    ) else {
        panic!("expected RuntimeVersionGate on unprobed Antigravity 1.4.7 resume_on_launch");
    };
    assert_eq!(relaunch_gate, resume_gate);
    assert_eq!(
        fake.spawns.lock().unwrap().len(),
        spawns_before,
        "must refuse agy < 1.5.8 before spawning"
    );
}
