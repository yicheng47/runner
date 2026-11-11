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
        .spawn_runtime_direct(
            &role,
            None,
            Some(fixture_tmp_dir().to_str().unwrap()),
            None,
            None,
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
        .set_installed(Runtime::Codex, &role.command, Some("0.147.0"));
    let check = |error: Error| {
        let Error::RuntimeVersionGate(gate) = error else {
            panic!("expected typed version gate, got {error}")
        };
        assert_eq!(gate.runtime, Runtime::Codex);
        assert_eq!(gate.installed_version, "0.147.0");
        assert_eq!(gate.min_version, "0.149.0");
    };
    check(
        manager
            .spawn_runtime_direct(
                &role,
                None,
                None,
                None,
                None,
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
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                data.path(),
                pool.clone(),
                capture(),
                None,
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
        .set_installed(Runtime::Codex, &role.command, None);
    assert!(manager.resolve_role_executable(&role, &pool).is_ok());
}

#[test]
fn gate_remains_typed_after_daemon_dispatch_and_wire_roundtrip() {
    let gate = runner_core::protocol::RuntimeVersionGate {
        runtime: Runtime::Codex,
        installed_version: "0.147.0".into(),
        min_version: "0.149.0".into(),
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
