use super::*;

#[test]
fn login_shell_proxy_env_reaches_spawn_with_role_env_taking_precedence() {
    // Issue #152: GUI-launched Role.app inherits launchd's
    // stripped env, so HTTPS_PROXY / NO_PROXY from the user's
    // shell rc files never reaches PTY children and claude /
    // codex login fails behind a corporate VPN / ClashX.
    //
    // The captured login-shell env on `SessionManager` should:
    //   - land in every spawn's env so children see the same
    //     proxy vars Terminal.app's children see;
    //   - lose to an explicit role.env override on the same
    //     key, because the role row is the more specific
    //     configuration surface.
    let pool = pool_with_schema();
    let role_id = ulid::Ulid::new().to_string();
    let mut role = role("/bin/sh", &["-c", "true"]);
    role.id = role_id;
    role.handle = "proxied".into();
    // The role row overrides HTTPS_PROXY but leaves
    // NO_PROXY / lowercase variants untouched, so we expect
    // those to come straight from the login-shell snapshot.
    role.env
        .insert("HTTPS_PROXY".into(), "http://role-override:9999".into());
    insert_role_row(&pool.get().unwrap(), &role);

    let fake = fake_runtime();
    let mut vars = std::collections::BTreeMap::new();
    vars.insert("HTTPS_PROXY".into(), "http://login-shell:7890".into());
    vars.insert("https_proxy".into(), "http://login-shell:7890".into());
    vars.insert("NO_PROXY".into(), "localhost,127.0.0.1,*.byted.org".into());
    let mgr = manager_with_runtime(
        crate::shell_path::LoginShellEnv { path: None, vars },
        Arc::clone(&fake) as Arc<dyn SessionRuntime>,
    );
    mgr.spawn_direct(
        &role,
        DirectLaunch {
            cwd: Some(fixture_tmp_dir().to_str().unwrap().into()),
            ..Default::default()
        },
        fixture_tmp_dir(),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();

    let spec = fake.last_spawn_spec().expect("spawn was called");
    assert_eq!(
        spec.env.get("HTTPS_PROXY").map(String::as_str),
        Some("http://role-override:9999"),
        "role.env must override the login-shell capture",
    );
    assert_eq!(
        spec.env.get("https_proxy").map(String::as_str),
        Some("http://login-shell:7890"),
        "lowercase variant must flow through unchanged",
    );
    assert_eq!(
        spec.env.get("NO_PROXY").map(String::as_str),
        Some("localhost,127.0.0.1,*.byted.org"),
        "NO_PROXY (with wildcard) must flow through unchanged",
    );
}

#[cfg(target_os = "macos")]
#[test]
fn utf8_locale_fallback_applies_only_when_no_locale_present() {
    use super::spawn::ensure_utf8_locale;

    let mut env = std::collections::BTreeMap::new();
    ensure_utf8_locale(&mut env, false);
    assert_eq!(
        env.get("LC_CTYPE").map(String::as_str),
        Some("UTF-8"),
        "no locale anywhere on macOS must fall back to LC_CTYPE=UTF-8",
    );

    let mut env = std::collections::BTreeMap::new();
    ensure_utf8_locale(&mut env, true);
    assert!(env.is_empty(), "an inherited process locale must win");

    for var in ["LANG", "LC_ALL", "LC_CTYPE"] {
        let mut env = std::collections::BTreeMap::new();
        env.insert(var.to_string(), "zh_CN.GB18030".to_string());
        ensure_utf8_locale(&mut env, false);
        assert_eq!(env.len(), 1, "a configured locale must not be augmented");
        assert_eq!(env.get(var).map(String::as_str), Some("zh_CN.GB18030"));
    }
}

#[test]
fn spawn_env_respects_configured_locale_and_falls_back_to_utf8() {
    let pool = pool_with_schema();
    let localized_id = ulid::Ulid::new().to_string();
    let bare_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        for (id, handle) in [(&localized_id, "localized"), (&bare_id, "bare")] {
            crate::test_support::insert_test_role(&conn, id, handle, "shell", "/bin/sh");
        }
    }

    let fake = fake_runtime();
    let mgr = manager_with_runtime(
        crate::shell_path::LoginShellEnv::default(),
        Arc::clone(&fake) as Arc<dyn SessionRuntime>,
    );

    let mut localized = role("/bin/sh", &["-c", "true"]);
    localized.id = localized_id;
    localized.handle = "localized".into();
    localized.env.insert("LC_ALL".into(), "zh_CN.UTF-8".into());
    mgr.spawn_direct(
        &localized,
        DirectLaunch {
            cwd: Some(fixture_tmp_dir().to_str().unwrap().into()),
            ..Default::default()
        },
        fixture_tmp_dir(),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();
    let spec = fake.last_spawn_spec().expect("spawn was called");
    assert_eq!(
        spec.env.get("LC_ALL").map(String::as_str),
        Some("zh_CN.UTF-8"),
        "role.env locale must flow through",
    );
    assert!(
        !spec.env.contains_key("LC_CTYPE"),
        "a role-configured locale must suppress the fallback",
    );

    let mut bare = role("/bin/sh", &["-c", "true"]);
    bare.id = bare_id;
    bare.handle = "bare".into();
    mgr.spawn_direct(
        &bare,
        DirectLaunch {
            cwd: Some(fixture_tmp_dir().to_str().unwrap().into()),
            ..Default::default()
        },
        fixture_tmp_dir(),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();
    let spec = fake.last_spawn_spec().expect("spawn was called");
    #[cfg(target_os = "macos")]
    {
        let process_has_locale = ["LANG", "LC_ALL", "LC_CTYPE"]
            .iter()
            .any(|var| std::env::var_os(var).is_some());
        if process_has_locale {
            assert!(
                !spec.env.contains_key("LC_CTYPE"),
                "children inherit the process locale — no fallback expected",
            );
        } else {
            assert_eq!(
                spec.env.get("LC_CTYPE").map(String::as_str),
                Some("UTF-8"),
                "locale-free macOS spawn must get the UTF-8 fallback",
            );
        }
    }
    #[cfg(not(target_os = "macos"))]
    assert!(
        !spec.env.contains_key("LC_CTYPE"),
        "non-macOS spawns must not inject bare LC_CTYPE=UTF-8",
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn non_macos_agent_env_skips_utf8_fallback_and_preserves_configured_locale() {
    let bare = spawn::agent_env(BTreeMap::new(), &HashMap::new(), BTreeMap::new(), "shell");
    assert!(
        !bare.contains_key("LC_CTYPE"),
        "non-macOS agent_env must not inject bare LC_CTYPE=UTF-8",
    );

    let localized = spawn::agent_env(
        BTreeMap::from([("LANG".into(), "en_US.UTF-8".into())]),
        &HashMap::from([("LC_ALL".into(), "zh_CN.UTF-8".into())]),
        BTreeMap::new(),
        "shell",
    );
    assert_eq!(
        localized.get("LANG").map(String::as_str),
        Some("en_US.UTF-8"),
        "shell locale vars must pass through on non-macOS",
    );
    assert_eq!(
        localized.get("LC_ALL").map(String::as_str),
        Some("zh_CN.UTF-8"),
        "explicit role.env locale must pass through on non-macOS",
    );
    assert!(
        !localized.contains_key("LC_CTYPE"),
        "configured locale must not trigger LC_CTYPE injection on non-macOS",
    );
}

#[test]
fn every_session_names_its_daemons_runner_and_a_slot_names_its_shim() {
    use runner_core::cli_install::AGENT_DEST_BIN_NAME;
    use runner_core::protocol::SESSION_CLI_ENV;

    let pool = pool_with_schema();
    let data = tempfile::tempdir().unwrap();
    let cwd = data.path().to_str().unwrap();
    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let bundled = data.path().join("bin").join(AGENT_DEST_BIN_NAME);
    let session_cli = || {
        fake.last_spawn_spec()
            .unwrap()
            .env
            .get(SESSION_CLI_ENV)
            .map(PathBuf::from)
    };

    let chat = role("/bin/sh", &["-c", "true"]);
    insert_role_row(&pool.get().unwrap(), &chat);
    let spawned = mgr
        .spawn_direct(
            &chat,
            DirectLaunch {
                cwd: Some(cwd.into()),
                ..Default::default()
            },
            data.path(),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();
    assert_eq!(session_cli(), Some(bundled.clone()), "chat");
    mgr.kill(&spawned.id).unwrap();

    let shell = runtime_direct_role("shell", Some("/bin/sh"), None, None).unwrap();
    let terminal = mgr
        .spawn_direct(
            &shell,
            DirectLaunch {
                cwd: Some(cwd.into()),
                runtime_direct: true,
                ..Default::default()
            },
            data.path(),
            Arc::clone(&pool),
            capture(),
        )
        .unwrap();
    assert_eq!(session_cli(), Some(bundled), "terminal pane");
    mgr.kill(&terminal.id).unwrap();

    let mission = mission();
    let member = role("/bin/sh", &["-c", "true"]);
    insert_crew_role(&pool, &mission.id, &member.id);
    let slot = slot_for(&member);
    let slot_session = mgr
        .spawn(
            &mission,
            &member,
            &slot,
            data.path(),
            data.path().join("events.ndjson"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    let shim = data
        .path()
        .join("missions")
        .join(&mission.id)
        .join("shims")
        .join(&slot.slot_handle)
        .join("bin")
        .join(AGENT_DEST_BIN_NAME);
    assert_eq!(session_cli(), Some(shim.clone()), "mission slot");
    #[cfg(unix)]
    assert!(shim.is_file());
    mgr.kill(&slot_session.id).unwrap();
}
