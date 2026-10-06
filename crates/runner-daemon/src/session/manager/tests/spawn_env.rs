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
        None,
        None,
        None,
        None,
        Some(fixture_tmp_dir().to_str().unwrap()),
        None,
        None,
        fixture_tmp_dir(),
        Arc::clone(&pool),
        capture(),
        None,
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

#[test]
fn utf8_locale_fallback_applies_only_when_no_locale_present() {
    use super::spawn::ensure_utf8_locale;

    let mut env = std::collections::BTreeMap::new();
    ensure_utf8_locale(&mut env, false);
    assert_eq!(
        env.get("LC_CTYPE").map(String::as_str),
        Some("UTF-8"),
        "no locale anywhere must fall back to LC_CTYPE=UTF-8",
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
        None,
        None,
        None,
        None,
        Some(fixture_tmp_dir().to_str().unwrap()),
        None,
        None,
        fixture_tmp_dir(),
        Arc::clone(&pool),
        capture(),
        None,
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
        None,
        None,
        None,
        None,
        Some(fixture_tmp_dir().to_str().unwrap()),
        None,
        None,
        fixture_tmp_dir(),
        Arc::clone(&pool),
        capture(),
        None,
    )
    .unwrap();
    let spec = fake.last_spawn_spec().expect("spawn was called");
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
            "locale-free spawn must get the UTF-8 fallback",
        );
    }
}
