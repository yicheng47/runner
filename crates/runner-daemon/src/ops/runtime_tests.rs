use super::*;

#[test]
fn effort_filter_keeps_default_and_falls_back_for_unknown_capabilities() {
    let mut runtime = runtime_catalog_options().remove(0);
    let model = runtime.models[1].value.clone();
    let values = |runtime: &RuntimeCatalogEntry, model: &str| {
        runtime
            .efforts_for_model(model)
            .into_iter()
            .map(|option| option.value)
            .collect::<Vec<_>>()
    };
    let fallback = values(&runtime, &model);
    assert_eq!(values(&runtime, ""), [""]);
    runtime.models[1].supported_efforts = Some(vec!["low".into(), "high".into()]);
    assert_eq!(values(&runtime, &model), ["", "low", "high"]);
    assert_eq!(values(&runtime, "custom-model"), fallback);
    runtime.default_model = Some(model.clone());
    assert_eq!(values(&runtime, ""), [""]);
    assert_eq!(values(&runtime, "  "), [""]);
    runtime.models[1].supported_efforts = Some(Vec::new());
    assert_eq!(values(&runtime, &model), [""]);
    runtime.models[1].supported_efforts = None;
    assert_eq!(values(&runtime, &model), fallback);
}

#[test]
fn catalog_matches_supported_runtime_order_and_defaults() {
    let definitions = runtime_list();
    let pi = definitions
        .iter()
        .find(|runtime| runtime.name == Runtime::Pi)
        .unwrap();
    assert_eq!(pi.command, "pi");
    assert!(pi.native_fork);
    let agy = definitions
        .iter()
        .find(|runtime| runtime.name == Runtime::Antigravity)
        .unwrap();
    assert_eq!(agy.display_name, "Antigravity CLI");
    assert_eq!(agy.command, "agy");
    assert!(!agy.native_fork);

    let catalog = runtime_catalog_options();
    let expected = [
        Runtime::Codex,
        Runtime::ClaudeCode,
        Runtime::Antigravity,
        Runtime::Pi,
        Runtime::Copilot,
        Runtime::Trae,
        Runtime::Cursor,
        Runtime::Grok,
        Runtime::OpenCode,
    ];
    assert_eq!(
        definitions
            .iter()
            .map(|runtime| runtime.name)
            .collect::<Vec<_>>(),
        expected
    );
    assert_eq!(&Runtime::ALL[..9], &expected);
    assert_eq!(
        catalog
            .iter()
            .map(|runtime| runtime.name)
            .collect::<Vec<_>>(),
        expected
    );
    assert!(catalog[0].default_enabled);
    assert!(catalog[1].default_enabled);
    assert!(catalog[2].default_enabled);
    assert_eq!(catalog[4].models[1].value, "auto");
    assert_eq!(catalog[4].models.len(), 28);
    assert!(catalog[4].default_enabled);
    assert!(catalog[3].default_enabled);
    assert!(catalog[3].native_fork);
    assert_eq!(catalog[3].models.len(), 1);
    assert_eq!(
        catalog[3]
            .efforts
            .iter()
            .map(|effort| effort.value.as_str())
            .collect::<Vec<_>>(),
        ["", "off", "minimal", "low", "medium", "high", "xhigh", "max"]
    );
    assert_eq!(
        catalog[4]
            .efforts
            .iter()
            .map(|effort| effort.value.as_str())
            .collect::<Vec<_>>(),
        ["", "none", "minimal", "low", "medium", "high", "xhigh", "max"]
    );
    assert_eq!(
        catalog[0]
            .models
            .iter()
            .map(|model| model.value.as_str())
            .collect::<Vec<_>>(),
        [
            "",
            "gpt-6-astra",
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
            "gpt-5.5",
            "gpt-5.4",
            "gpt-5.4-mini",
            "gpt-5.3-codex-spark",
        ]
    );
    assert_eq!(
        catalog[0]
            .efforts
            .iter()
            .map(|effort| effort.value.as_str())
            .collect::<Vec<_>>(),
        ["", "low", "medium", "high", "xhigh", "max", "ultra"]
    );
    assert_eq!(
        catalog[5]
            .efforts
            .iter()
            .map(|effort| effort.value.as_str())
            .collect::<Vec<_>>(),
        ["", "low", "medium", "high", "xhigh"]
    );

    let agy = &catalog[2];
    assert_eq!(agy.command, "agy");
    assert!(!agy.native_fork);
    assert!(agy.default_enabled);
    assert_eq!(
        agy.models
            .iter()
            .map(|model| model.value.as_str())
            .collect::<Vec<_>>(),
        [
            "",
            "gemini-4-carbon",
            "gemini-4-argon",
            "gemini-4-hydrogen",
            "gemini-pro-orchestrator",
            "gemini-pro-orchestrator-argon",
            "gemini-3.8-flash",
            "claude-opus-5-5",
            "claude-sonnet-5-5",
            "gpt-oss-120b",
        ]
    );
    let efforts = |model: &str| {
        agy.efforts_for_model(model)
            .into_iter()
            .map(|effort| effort.value)
            .collect::<Vec<_>>()
    };
    assert_eq!(efforts(""), [""]);
    assert_eq!(
        efforts("gemini-4-hydrogen"),
        ["", "low", "medium", "high", "xhigh"]
    );
    assert_eq!(efforts("claude-opus-5-5"), ["", "low", "medium", "high"]);
    assert_eq!(efforts("gpt-oss-120b"), ["", "medium"]);
    for model in [
        "gemini-pro-orchestrator",
        "gemini-pro-orchestrator-argon",
        "gemini-4-hydrogen-xhigh",
        "gpt-oss-120b-medium",
    ] {
        assert_eq!(efforts(model), [""], "{model}");
    }
}

#[test]
fn selectable_catalog_requires_availability_and_honors_agent_settings() {
    let mut catalog = runtime_catalog_options();
    assert!(filter_selectable_runtime_catalog(catalog.clone(), None).is_empty());
    assert!(filter_selectable_runtime_catalog(catalog.clone(), Some(&["trae".into()])).is_empty());
    for runtime in &mut catalog {
        runtime.available = true;
    }
    let expected = vec![
        Runtime::Codex,
        Runtime::ClaudeCode,
        Runtime::Antigravity,
        Runtime::Pi,
        Runtime::Copilot,
        Runtime::Cursor,
        Runtime::Grok,
        Runtime::OpenCode,
    ];
    assert_eq!(
        filter_selectable_runtime_catalog(catalog.clone(), None)
            .iter()
            .map(|runtime| runtime.name)
            .collect::<Vec<_>>(),
        expected
    );

    let enabled = vec!["trae".into()];
    assert_eq!(
        filter_selectable_runtime_catalog(catalog, Some(&enabled))
            .iter()
            .map(|runtime| runtime.name)
            .collect::<Vec<_>>(),
        [Runtime::Trae]
    );
}

#[cfg(unix)]
#[test]
fn update_spec_runs_the_effective_executable_with_its_update_argument() {
    use std::os::unix::fs::PermissionsExt;
    let bin = tempfile::tempdir().unwrap();
    for command in ["codex", "traecli"] {
        let path = bin.path().join(command);
        std::fs::write(&path, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let state = crate::test_support::test_core();
    state.runtime_shell_env.write().unwrap().path = Some(bin.path().display().to_string());

    let spec = runtime_update_spawn_spec(&state, Runtime::Codex, (90, 28)).unwrap();
    assert_eq!(spec.command, bin.path().join("codex").display().to_string());
    assert_eq!(spec.args, ["update"]);
    assert_eq!(spec.cwd, runner_core::app_paths::home_dir());
    assert_eq!(spec.initial_size, Some((90, 28)));

    assert!(runtime_update_spawn_spec(&state, Runtime::Trae, (90, 28)).is_err());
}
#[test]
fn missing_agent_preview_uses_catalog_command_without_requiring_installation() {
    let state = crate::test_support::test_core();
    let root = tempfile::tempdir().unwrap();
    state.runtime_shell_env.write().unwrap().path = Some(root.path().display().to_string());
    state.runtime_discovery.write().unwrap().checking = false;
    let preview = runtime_command_preview(&state, Runtime::Trae, false).unwrap();
    assert_eq!(preview.lines[0].command, "traecli");
    assert!(!preview.lines.is_empty());
}

#[test]
fn claude_preview_only_sets_config_dir_for_an_active_non_native_account() {
    use runner_core::protocol::runtime::RuntimeCommandNote;
    let state = crate::test_support::test_core();
    let preview = runtime_command_preview(&state, Runtime::ClaudeCode, false).unwrap();
    assert!(!preview
        .lines
        .iter()
        .any(|line| line.command.starts_with("CLAUDE_CONFIG_DIR=")));
    let mut accounts = crate::claude_accounts::load(&state.db).unwrap();
    accounts
        .accounts
        .push(runner_core::protocol::ClaudeAccount {
            id: "work".into(),
            name: "Work".into(),
            folder: "/tmp/runner-preview-work".into(),
            native: false,
        });
    accounts.active_id = "work".into();
    crate::db::app_state_set(
        &state.db.get().unwrap(),
        "claude_accounts",
        &serde_json::to_string(&accounts).unwrap(),
    )
    .unwrap();
    let preview = runtime_command_preview(&state, Runtime::ClaudeCode, false).unwrap();
    assert_eq!(
        preview.lines[0].command,
        "CLAUDE_CONFIG_DIR=/tmp/runner-preview-work"
    );
    assert_eq!(
        preview.lines[0].note,
        RuntimeCommandNote::ActiveAccount {
            name: "Work".into()
        }
    );
    accounts.active_id = accounts
        .accounts
        .iter()
        .find(|account| account.native)
        .unwrap()
        .id
        .clone();
    crate::db::app_state_set(
        &state.db.get().unwrap(),
        "claude_accounts",
        &serde_json::to_string(&accounts).unwrap(),
    )
    .unwrap();
    let preview = runtime_command_preview(&state, Runtime::ClaudeCode, false).unwrap();
    assert!(!preview
        .lines
        .iter()
        .any(|line| line.command.starts_with("CLAUDE_CONFIG_DIR=")));
}

#[test]
fn custom_flags_validation_save_refusal_warning_and_clear() {
    let state = crate::test_support::test_core();
    {
        let mut discovery = state.runtime_discovery.write().unwrap();
        discovery.versions.set_help_flags(
            Runtime::Codex,
            Some(
                ["-c", "--config", "--search", "--add-dir"]
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
            ),
        );
    }

    // Save valid flags -> persisted verbatim, no warning.
    let initial = r#"--search --add-dir "/tmp/a b""#;
    let saved = runtime_set_custom_flags(&state, Runtime::Codex, initial).unwrap();
    assert_eq!(saved.warning, None);
    let codex = saved
        .status
        .runtimes
        .iter()
        .find(|r| r.name == Runtime::Codex)
        .unwrap();
    assert_eq!(codex.launch_options.custom_flags, initial);

    // Refusal (unclosed quote and reserved flag) keeps previously saved flags untouched.
    for bad in [
        "--search \"unclosed",
        "--model gpt-5",
        "-c features.remote_plugin=true",
    ] {
        assert!(
            runtime_set_custom_flags(&state, Runtime::Codex, bad).is_err(),
            "{bad}"
        );
        let current = crate::db::runtime_launch_options(&state.db).unwrap();
        assert_eq!(current["codex"].custom_flags, initial);
    }

    // Warning still saves and returns warning; preview recomputes it too.
    let warned = runtime_set_custom_flags(&state, Runtime::Codex, "--serch").unwrap();
    assert_eq!(
        warned.warning.as_deref(),
        Some("codex --help doesn't list --serch.")
    );
    let current = crate::db::runtime_launch_options(&state.db).unwrap();
    assert_eq!(current["codex"].custom_flags, "--serch");
    let preview = runtime_command_preview(&state, Runtime::Codex, false).unwrap();
    assert_eq!(
        preview.custom_flags_warning.as_deref(),
        Some("codex --help doesn't list --serch.")
    );

    // Clearing (whitespace/empty) removes the stored flags and warning.
    let cleared = runtime_set_custom_flags(&state, Runtime::Codex, "   ").unwrap();
    assert_eq!(cleared.warning, None);
    let codex = cleared
        .status
        .runtimes
        .iter()
        .find(|r| r.name == Runtime::Codex)
        .unwrap();
    assert_eq!(codex.launch_options.custom_flags, "");
    let preview = runtime_command_preview(&state, Runtime::Codex, false).unwrap();
    assert_eq!(preview.custom_flags_warning, None);
}

#[cfg(unix)]
#[test]
fn custom_flags_reads_stderr_from_help_probe_like_agy() {
    use std::os::unix::fs::PermissionsExt;

    let bin = tempfile::tempdir().unwrap();
    let agy = bin.path().join("agy");
    std::fs::write(
            &agy,
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo \"1.0.0\"; exit 0; fi\necho \"  --known-flag  help\" >&2\n",
        )
        .unwrap();
    std::fs::set_permissions(&agy, std::fs::Permissions::from_mode(0o755)).unwrap();

    let state = crate::test_support::test_core();
    state.runtime_shell_env.write().unwrap().path = Some(bin.path().display().to_string());
    runtime_set_override(&state, Runtime::Antigravity, agy.to_str().unwrap()).unwrap();

    let ok = runtime_set_custom_flags(&state, Runtime::Antigravity, "--known-flag").unwrap();
    assert_eq!(ok.warning, None);

    let warned = runtime_set_custom_flags(&state, Runtime::Antigravity, "--unknown").unwrap();
    assert_eq!(
        warned.warning.as_deref(),
        Some("agy --help doesn't list --unknown.")
    );
}

#[cfg(unix)]
#[test]
fn rejected_opencode_override_preserves_active_path_version_cache() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let path_dir = dir.path().join("path");
    let other_dir = dir.path().join("other");
    std::fs::create_dir_all(&path_dir).unwrap();
    std::fs::create_dir_all(&other_dir).unwrap();
    let path_v1 = path_dir.join("opencode");
    let other_v1 = other_dir.join("opencode");
    for script in [&path_v1, &other_v1] {
        std::fs::write(script, "#!/bin/sh\necho '1.19.5'\n").unwrap();
        std::fs::set_permissions(script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let state = crate::test_support::test_core();
    state.runtime_shell_env.write().unwrap().path = Some(path_dir.display().to_string());
    {
        let mut discovery = state.runtime_discovery.write().unwrap();
        discovery.checking = false;
        discovery.result = Some(crate::shell_path::DiscoveryResult {
            shell: Some("/bin/zsh".into()),
            outcome: crate::shell_path::DiscoveryOutcome::Ok,
            duration_ms: 10,
            env: crate::shell_path::LoginShellEnv::default(),
        });
    }

    // Cold model request probes the active PATH executable and caches its v1 result.
    let mut rx = state.events.subscribe();
    runtime_request_models(&state, &[Runtime::OpenCode]);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut saw_change = false;
    while std::time::Instant::now() < deadline {
        if let Ok(ev) = rx.try_recv() {
            if ev.name == "runtime/changed" {
                saw_change = true;
                break;
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    assert!(
        saw_change,
        "expected cold runtime_request_models sync probe to emit runtime/changed"
    );

    let before = runtime_catalog(&state)
        .unwrap()
        .into_iter()
        .find(|entry| entry.name == Runtime::OpenCode)
        .unwrap();
    assert!(!before.available);
    assert!(before.below_minimum);
    assert_eq!(before.installed_version.as_deref(), Some("1.19.5"));

    // Rejected v1 override leaves DB unset AND preserves the active PATH cache.
    let err =
        runtime_set_override(&state, Runtime::OpenCode, other_v1.to_str().unwrap()).unwrap_err();
    assert!(err.to_string().contains("2.1.0"), "{err}");
    assert!(!crate::db::runtime_overrides(&state.db)
        .unwrap()
        .contains_key("opencode"));

    let after = runtime_catalog(&state)
        .unwrap()
        .into_iter()
        .find(|entry| entry.name == Runtime::OpenCode)
        .unwrap();
    assert!(!after.available);
    assert!(after.below_minimum);
    assert_eq!(after.installed_version.as_deref(), Some("1.19.5"));
}

#[cfg(unix)]
#[test]
fn runtime_status_list_reports_npm_availability_and_read_only_sign_in_statuses() {
    use std::os::unix::fs::PermissionsExt;

    let bin = tempfile::tempdir().unwrap();
    for cmd in ["npm", "codex", "pi"] {
        let path = bin.path().join(cmd);
        std::fs::write(&path, "#!/bin/sh\necho '9.9.9'\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let codex_home = tempfile::tempdir().unwrap();
    let empty_home = tempfile::tempdir().unwrap();

    let state = crate::test_support::test_core();
    {
        let mut env = state.runtime_shell_env.write().unwrap();
        env.path = Some(bin.path().display().to_string());
        env.vars.insert(
            crate::runtime_status::TEST_ISOLATE_PATH_ENV.into(),
            "1".into(),
        );
        // Point HOME and CODEX_HOME away from developer machine creds so the
        // initial check stays deterministic even when run locally.
        env.vars.insert(
            "HOME".into(),
            empty_home.path().to_string_lossy().into_owned(),
        );
        env.vars.insert(
            "CODEX_HOME".into(),
            codex_home.path().to_string_lossy().into_owned(),
        );
    }
    state.runtime_discovery.write().unwrap().checking = false;

    let initial = runtime_status_list(&state).unwrap();
    assert!(initial.npm_available);
    assert_eq!(
        initial
            .runtimes
            .iter()
            .find(|r| r.name == Runtime::Codex)
            .unwrap()
            .sign_in,
        RuntimeSignInStatus::Unknown
    );
    assert_eq!(
        initial
            .runtimes
            .iter()
            .find(|r| r.name == Runtime::Pi)
            .unwrap()
            .sign_in,
        RuntimeSignInStatus::Unknown
    );

    // Codex `auth.json` tokens mark Codex SignedIn while Pi stays Unknown.
    std::fs::write(
        codex_home.path().join("auth.json"),
        r#"{"tokens":{"access_token":"tok-123"}}"#,
    )
    .unwrap();
    let with_auth_file = runtime_status_list(&state).unwrap();
    assert_eq!(
        with_auth_file
            .runtimes
            .iter()
            .find(|r| r.name == Runtime::Codex)
            .unwrap()
            .sign_in,
        RuntimeSignInStatus::SignedIn
    );
    assert_eq!(
        with_auth_file
            .runtimes
            .iter()
            .find(|r| r.name == Runtime::Pi)
            .unwrap()
            .sign_in,
        RuntimeSignInStatus::Unknown
    );

    // Without npm on PATH, npm_available flips false.
    let without_npm = tempfile::tempdir().unwrap();
    state.runtime_shell_env.write().unwrap().path = Some(without_npm.path().display().to_string());
    assert!(!runtime_status_list(&state).unwrap().npm_available);
}

#[cfg(unix)]
#[test]
fn sync_probe_preserves_async_notification_and_generation_ordering() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    // Part 1: an unchanneled (events = None) sync probe finishing while a background
    // probe for the same executable is in flight still lets the background probe emit.
    let dir = tempfile::tempdir().unwrap();
    let started = dir.path().join("async-started");
    let release_async = dir.path().join("release-async");
    let opencode = dir.path().join("opencode");
    std::fs::write(
            &opencode,
            format!(
                "#!/bin/sh\nif [ ! -f \"{}\" ]; then\n  : > \"{}\"\n  i=0\n  while [ ! -f \"{}\" ] && [ $i -lt 300 ]; do\n    sleep 0.01\n    i=$((i + 1))\n  done\n  echo 'unrecognized'\nelse\n  echo '1.19.5'\nfi\n",
                started.display(),
                started.display(),
                release_async.display(),
            ),
        )
        .unwrap();
    std::fs::set_permissions(&opencode, std::fs::Permissions::from_mode(0o755)).unwrap();

    let state = crate::test_support::test_core();
    state.runtime_shell_env.write().unwrap().path = Some(dir.path().display().to_string());
    let mut rx = state.events.subscribe();

    crate::runtime_status::versions::request_probes(
        &state.db,
        &state.runtime_shell_env,
        &state.runtime_discovery,
        &state.events,
        &[Runtime::OpenCode],
        false,
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while !started.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(started.exists(), "background probe did not start");

    // Sync probe without EventChannel completes first and populates cache.
    assert_eq!(
        crate::runtime_status::versions::ensure_probed(
            Runtime::OpenCode,
            opencode.to_str().unwrap(),
            &state.runtime_shell_env,
            &state.runtime_discovery,
            None,
        )
        .as_deref(),
        Some("1.19.5")
    );
    assert_eq!(
        state
            .runtime_discovery
            .read()
            .unwrap()
            .versions
            .installed(Runtime::OpenCode, opencode.to_str().unwrap()),
        Some("1.19.5")
    );

    // Releasing the background probe still delivers runtime/changed.
    std::fs::write(&release_async, "go").unwrap();
    let mut saw_change = false;
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if let Ok(ev) = rx.try_recv() {
            if ev.name == "runtime/changed" {
                saw_change = true;
                break;
            }
        } else {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    assert!(saw_change, "background probe notification was swallowed");
    assert_eq!(
        state
            .runtime_discovery
            .read()
            .unwrap()
            .versions
            .installed(Runtime::OpenCode, opencode.to_str().unwrap()),
        Some("1.19.5")
    );

    // Part 2: an older in-flight sync probe cannot overwrite a newer accepted override.
    let slow_v1 = dir.path().join("slow-v1");
    let slow_started = dir.path().join("slow-started");
    let release_slow = dir.path().join("release-slow");
    std::fs::write(
            &slow_v1,
            format!(
                "#!/bin/sh\n: > \"{}\"\ni=0\nwhile [ ! -f \"{}\" ] && [ $i -lt 300 ]; do\n  sleep 0.01\n  i=$((i + 1))\ndone\necho '1.19.5'\n",
                slow_started.display(),
                release_slow.display(),
            ),
        )
        .unwrap();
    std::fs::set_permissions(&slow_v1, std::fs::Permissions::from_mode(0o755)).unwrap();

    let fast_v2 = dir.path().join("fast-v2");
    std::fs::write(&fast_v2, "#!/bin/sh\necho '2.1.0'\n").unwrap();
    std::fs::set_permissions(&fast_v2, std::fs::Permissions::from_mode(0o755)).unwrap();

    let state2 = crate::test_support::test_core();
    crate::db::set_runtime_override(&state2.db, "opencode", Some(slow_v1.to_str().unwrap()))
        .unwrap();

    let shell_env = std::sync::Arc::clone(&state2.runtime_shell_env);
    let discovery = std::sync::Arc::clone(&state2.runtime_discovery);
    let events = state2.events.clone();
    let slow_path = slow_v1.to_str().unwrap().to_owned();
    let sync_thread = std::thread::spawn(move || {
        crate::runtime_status::versions::ensure_probed(
            Runtime::OpenCode,
            &slow_path,
            &shell_env,
            &discovery,
            Some(&events),
        )
    });

    let deadline = Instant::now() + Duration::from_secs(10);
    while !slow_started.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(slow_started.exists(), "slow sync probe did not start");

    // Supersede the in-flight sync probe with a new v2 override, then release the old one.
    runtime_set_override(&state2, Runtime::OpenCode, fast_v2.to_str().unwrap()).unwrap();
    std::fs::write(&release_slow, "go").unwrap();
    assert_eq!(sync_thread.join().unwrap().as_deref(), Some("1.19.5"));

    assert_eq!(
        state2
            .runtime_discovery
            .read()
            .unwrap()
            .versions
            .installed(Runtime::OpenCode, fast_v2.to_str().unwrap()),
        Some("2.1.0")
    );
}

#[cfg(unix)]
fn wait_for(deadline_secs: u64, mut ready: impl FnMut() -> bool, failure_msg: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(deadline_secs);
    while !ready() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(ready(), "{failure_msg}");
}

#[cfg(unix)]
fn write_two_stage_probe_script(
    path: &std::path::Path,
    first_started: &std::path::Path,
    release_first: &std::path::Path,
    first_output: &str,
    second_started: &std::path::Path,
    release_second: &std::path::Path,
    second_output: &str,
) {
    use std::os::unix::fs::PermissionsExt;

    std::fs::write(
            path,
            format!(
                "#!/bin/sh\nif [ \"$1\" != \"--version\" ]; then\n  exit 9\nfi\nif [ ! -f \"{}\" ]; then\n  : > \"{}\"\n  i=0\n  while [ ! -f \"{}\" ] && [ $i -lt 300 ]; do\n    sleep 0.01\n    i=$((i + 1))\n  done\n  echo '{}'\n  exit 0\nfi\n: > \"{}\"\ni=0\nwhile [ ! -f \"{}\" ] && [ $i -lt 300 ]; do\n  sleep 0.01\n  i=$((i + 1))\ndone\necho '{}'\n",
                first_started.display(),
                first_started.display(),
                release_first.display(),
                first_output,
                second_started.display(),
                release_second.display(),
                second_output,
            ),
        )
        .unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
#[test]
fn older_async_probe_completing_first_cannot_bypass_sync_launch_gate() {
    let dir = tempfile::tempdir().unwrap();
    let async_started = dir.path().join("async-started");
    let release_async = dir.path().join("release-async");
    let sync_started = dir.path().join("sync-started");
    let release_sync = dir.path().join("release-sync");
    let opencode = dir.path().join("opencode");
    write_two_stage_probe_script(
        &opencode,
        &async_started,
        &release_async,
        "unrecognized",
        &sync_started,
        &release_sync,
        "1.19.5",
    );

    let state = crate::test_support::test_core();
    state.runtime_shell_env.write().unwrap().path = Some(dir.path().display().to_string());

    request_version_probe(&state, Runtime::OpenCode);
    wait_for(
        10,
        || async_started.exists(),
        "background probe did not start",
    );

    let role = crate::session::manager::runtime_direct_role(
        "opencode",
        Some(opencode.to_str().unwrap()),
        None,
        None,
    )
    .unwrap();
    let sessions = Arc::clone(&state.sessions);
    let db = Arc::clone(&state.db);
    let events = Arc::new(state.session_events());
    let app_dir = dir.path().to_path_buf();
    let spawn_handle = std::thread::spawn(move || {
        sessions.spawn_runtime_direct(
            &role,
            None,
            app_dir.to_str(),
            None,
            None,
            &app_dir,
            db,
            events,
        )
    });

    wait_for(
        10,
        || sync_started.exists(),
        "sync launch probe did not start",
    );

    // Complete older async worker first with unrecognized output.
    std::fs::write(&release_async, "go").unwrap();
    wait_for(
        10,
        || {
            !state
                .runtime_discovery
                .read()
                .unwrap()
                .versions
                .has_in_flight_async(Runtime::OpenCode)
        },
        "background probe did not finish",
    );
    assert!(
        state
            .runtime_discovery
            .read()
            .unwrap()
            .versions
            .probed(Runtime::OpenCode, opencode.to_str().unwrap())
            .is_none(),
        "superseded background probe must not write None into cache"
    );

    // Now complete newer sync probe with v1.19.5; launch must be refused by RuntimeVersionGate.
    std::fs::write(&release_sync, "go").unwrap();
    let err = spawn_handle
        .join()
        .unwrap()
        .expect_err("opencode 1.19.5 must be refused before launching");
    let crate::error::Error::RuntimeVersionGate(gate) = err else {
        panic!("expected RuntimeVersionGate, got {err:#}");
    };
    assert_eq!(gate.runtime, Runtime::OpenCode);
    assert_eq!(gate.installed_version, "1.19.5");
    assert_eq!(gate.min_version, "2.1.0");
    assert_eq!(
        state
            .runtime_discovery
            .read()
            .unwrap()
            .versions
            .installed(Runtime::OpenCode, opencode.to_str().unwrap()),
        Some("1.19.5")
    );
}

#[cfg(unix)]
#[test]
fn older_async_probe_completing_first_still_emits_runtime_changed_on_status_read() {
    let dir = tempfile::tempdir().unwrap();
    let async_started = dir.path().join("async-started");
    let release_async = dir.path().join("release-async");
    let sync_started = dir.path().join("sync-started");
    let release_sync = dir.path().join("release-sync");
    let opencode = dir.path().join("opencode");
    write_two_stage_probe_script(
        &opencode,
        &async_started,
        &release_async,
        "unrecognized",
        &sync_started,
        &release_sync,
        "1.19.5",
    );

    let state = crate::test_support::test_core();
    crate::db::set_runtime_override(&state.db, "opencode", Some(opencode.to_str().unwrap()))
        .unwrap();
    let mut rx = state.events.subscribe();

    request_version_probe(&state, Runtime::OpenCode);
    wait_for(
        10,
        || async_started.exists(),
        "background probe did not start",
    );

    let state_bg = state.clone();
    let status_handle = std::thread::spawn(move || runtime_status_list(&state_bg));
    wait_for(
        10,
        || sync_started.exists(),
        "status sync probe did not start",
    );

    // Older async probe completes first; status read's sync probe still finishes and emits.
    std::fs::write(&release_async, "go").unwrap();
    wait_for(
        10,
        || {
            !state
                .runtime_discovery
                .read()
                .unwrap()
                .versions
                .has_in_flight_async(Runtime::OpenCode)
        },
        "background probe did not finish",
    );

    std::fs::write(&release_sync, "go").unwrap();
    let response = status_handle.join().unwrap().unwrap();
    let row = response
        .runtimes
        .into_iter()
        .find(|entry| entry.name == Runtime::OpenCode)
        .unwrap();
    assert_eq!(
        row.state,
        crate::runtime_status::RuntimeRowState::InvalidOverride
    );
    assert!(row.below_minimum);
    assert_eq!(row.installed_version.as_deref(), Some("1.19.5"));

    let mut saw_change = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while std::time::Instant::now() < deadline {
        if let Ok(ev) = rx.try_recv() {
            if ev.name == "runtime/changed" {
                saw_change = true;
                break;
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    assert!(
        saw_change,
        "status read must emit runtime/changed when older async worker finished first"
    );
}

#[cfg(unix)]
#[test]
fn same_path_replacement_before_refresh_refuses_launch_without_spawning_standalone() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let opencode = dir.path().join("opencode");
    let standalone_called = dir.path().join("standalone.called");
    let write_script = |tmp_name: &str, version: &str| {
        let tmp = dir.path().join(tmp_name);
        std::fs::write(
                &tmp,
                format!(
                    "#!/bin/sh\nif [ \"$1\" = --version ]; then\n  echo '{version}'\n  exit 0\nfi\ntouch '{}'\nexit 0\n",
                    standalone_called.display()
                ),
            )
            .unwrap();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::rename(&tmp, &opencode).unwrap();
    };

    write_script("opencode.v2", "2.1.0");

    let state = crate::test_support::test_core();
    state.runtime_shell_env.write().unwrap().path = Some(dir.path().display().to_string());
    assert_eq!(
        crate::runtime_status::versions::ensure_probed(
            Runtime::OpenCode,
            opencode.to_str().unwrap(),
            &state.runtime_shell_env,
            &state.runtime_discovery,
            None,
        )
        .as_deref(),
        Some("2.1.0")
    );

    // Replace binary on disk with v1.19.5 without clicking Refresh first.
    write_script("opencode.v1", "1.19.5");
    assert!(!standalone_called.exists());

    let role = crate::session::manager::runtime_direct_role("opencode", None, None, None).unwrap();
    let err = state
        .sessions
        .spawn_runtime_direct(
            &role,
            None,
            dir.path().to_str(),
            None,
            None,
            dir.path(),
            Arc::clone(&state.db),
            Arc::new(state.session_events()),
        )
        .expect_err("replaced v1 binary must be blocked before launching --standalone");
    let crate::error::Error::RuntimeVersionGate(gate) = err else {
        panic!("expected RuntimeVersionGate, got {err:#}");
    };
    assert_eq!(gate.runtime, Runtime::OpenCode);
    assert_eq!(gate.installed_version, "1.19.5");
    assert_eq!(gate.min_version, "2.1.0");
    assert!(
        !standalone_called.exists(),
        "opencode v1 must not be spawned when binary changed on disk before Refresh"
    );
}

#[cfg(unix)]
#[test]
fn classified_opencode_startup_error_clears_cached_probe_so_status_list_preserves_error() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let opencode = dir.path().join("opencode");
    let mode_file = dir.path().join("mode");
    std::fs::write(&mode_file, "v2").unwrap();
    std::fs::write(
            &opencode,
            format!(
                "#!/bin/sh\nmode=$(cat '{}')\nif [ \"$1\" = --version ]; then\n  if [ \"$mode\" = v2 ]; then\n    echo '2.1.0'\n  else\n    echo '1.19.5'\n  fi\n  exit 0\nfi\nif [ \"$mode\" = v1 ]; then\n  echo \"error: unknown option '--standalone'\"\n  exit 1\nfi\nexit 0\n",
                mode_file.display()
            ),
        )
        .unwrap();
    std::fs::set_permissions(&opencode, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut state = crate::test_support::test_core_in(dir.path().to_path_buf());
    state.sessions = crate::session::manager::SessionManager::new(
        Arc::clone(&state.runtime_shell_env),
        Arc::clone(&state.runtime_discovery),
        Arc::new(crate::session::pty_runtime::PtyRuntime::new()),
    );
    state.sessions.set_runtime_events(state.events.clone());
    state.runtime_shell_env.write().unwrap().path = Some(dir.path().display().to_string());
    state.runtime_discovery.write().unwrap().checking = false;

    assert_eq!(
        crate::runtime_status::versions::ensure_probed(
            Runtime::OpenCode,
            opencode.to_str().unwrap(),
            &state.runtime_shell_env,
            &state.runtime_discovery,
            Some(&state.events),
        )
        .as_deref(),
        Some("2.1.0")
    );

    // Flip mode behind an untouched wrapper script so cached 2.1.0 allows spawn once,
    // then v1 rejects --standalone.
    std::fs::write(&mode_file, "v1").unwrap();
    let mut rx = state.events.subscribe();
    let role = crate::session::manager::runtime_direct_role("opencode", None, None, None).unwrap();
    let spawned = state
        .sessions
        .spawn_runtime_direct(
            &role,
            None,
            dir.path().to_str(),
            None,
            None,
            dir.path(),
            Arc::clone(&state.db),
            Arc::new(state.session_events()),
        )
        .unwrap();

    wait_for(
        10,
        || {
            state
                .sessions
                .agent_status(&spawned.id)
                .startup_error
                .is_some()
        },
        "startup error was not classified",
    );

    let mut saw_change = false;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if let Ok(ev) = rx.try_recv() {
            if ev.name == "runtime/changed" {
                saw_change = true;
                break;
            }
        } else {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    assert!(saw_change, "clearing stale probe must emit runtime/changed");
    assert_eq!(
        state
            .runtime_discovery
            .read()
            .unwrap()
            .versions
            .installed(Runtime::OpenCode, opencode.to_str().unwrap()),
        None
    );

    // Passive status_list must not immediately retire the StartupError using stale v2 cache.
    let status = runtime_status_list(&state).unwrap();
    let row = status
        .runtimes
        .into_iter()
        .find(|entry| entry.name == Runtime::OpenCode)
        .unwrap();
    assert_eq!(row.installed_version, None);
    assert!(
        state
            .sessions
            .agent_status(&spawned.id)
            .startup_error
            .is_some(),
        "startup error must remain visible until a compatible v2 probe succeeds"
    );

    // Once switched back to v2 and re-probed, runtime_status_list retires the StartupError.
    std::fs::write(&mode_file, "v2").unwrap();
    assert_eq!(
        crate::runtime_status::versions::probe_now(
            Runtime::OpenCode,
            &state.db,
            &state.runtime_shell_env,
            &state.runtime_discovery,
            &state.events,
        )
        .as_deref(),
        Some("2.1.0")
    );
    let status = runtime_status_list(&state).unwrap();
    let row = status
        .runtimes
        .into_iter()
        .find(|entry| entry.name == Runtime::OpenCode)
        .unwrap();
    assert_eq!(row.installed_version.as_deref(), Some("2.1.0"));
    assert!(!row.below_minimum);
    assert!(
        state
            .sessions
            .agent_status(&spawned.id)
            .startup_error
            .is_none(),
        "compatible v2 refresh must retire obsolete startup error"
    );
}

#[cfg(unix)]
#[test]
fn failed_claude_auth_status_probe_caches_error_without_looping_until_explicit_recheck() {
    use std::os::unix::fs::PermissionsExt;
    use std::time::{Duration, Instant};

    let temp = tempfile::tempdir().unwrap();
    let app_data_dir = temp.path().join("app-data");
    let empty_home = temp.path().join("home");
    let claude_dir = temp.path().join("claude-cfg");
    let bin = temp.path().join("bin");
    let probe_log = temp.path().join("probes.log");
    std::fs::create_dir_all(&app_data_dir).unwrap();
    std::fs::create_dir_all(&empty_home).unwrap();
    std::fs::create_dir_all(&claude_dir).unwrap();
    std::fs::create_dir_all(&bin).unwrap();

    let claude_bin = bin.join("claude");
    std::fs::write(
            &claude_bin,
            format!(
                "#!/bin/sh\nif [ \"$1\" = \"auth\" ] && [ \"$2\" = \"status\" ]; then\n  printf 'probe\\n' >> '{}'\n  echo 'not-json'\n  exit 1\nfi\necho '2.1.0'\n",
                probe_log.display()
            ),
        )
        .unwrap();
    std::fs::set_permissions(&claude_bin, std::fs::Permissions::from_mode(0o755)).unwrap();

    let mut state = crate::test_support::test_core_in(app_data_dir);
    state.claude_accounts = Arc::new(crate::claude_accounts::AccountService::default());
    state.usage = Arc::new(crate::usage::UsageService::without_fetch());
    {
        let mut env = state.runtime_shell_env.write().unwrap();
        env.path = Some(bin.display().to_string());
        env.vars.insert(
            crate::runtime_status::TEST_ISOLATE_PATH_ENV.into(),
            "1".into(),
        );
        env.vars
            .insert("HOME".into(), empty_home.to_string_lossy().into_owned());
    }
    state.runtime_discovery.write().unwrap().checking = false;
    crate::db::set_runtime_override(&state.db, "claude-code", claude_bin.to_str()).unwrap();
    crate::db::app_state_set(
        &state.db.get().unwrap(),
        "claude_accounts",
        &serde_json::json!({
            "accounts": [{
                "id": "native",
                "name": "Personal",
                "folder": claude_dir.to_string_lossy().into_owned(),
                "native": true
            }],
            "main_id": "native",
            "active_id": "native"
        })
        .to_string(),
    )
    .unwrap();

    let mut events = state.events.subscribe();
    let initial = runtime_status_list(&state).unwrap();
    assert_eq!(
        initial
            .runtimes
            .iter()
            .find(|r| r.name == Runtime::ClaudeCode)
            .unwrap()
            .sign_in,
        RuntimeSignInStatus::Unknown
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    while !crate::claude_accounts::snapshot(&state)
        .unwrap()
        .statuses
        .get("native")
        .is_some_and(|s| s.error)
        || state.claude_accounts.is_refreshing()
    {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for initial auth probe error"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    let probe_count = || -> usize {
        std::fs::read_to_string(&probe_log)
            .map(|raw| raw.lines().count())
            .unwrap_or(0)
    };
    assert_eq!(probe_count(), 1);

    let mut saw_runtime_changed = false;
    while let Ok(evt) = events.try_recv() {
        if evt.name == "runtime/changed" {
            saw_runtime_changed = true;
        }
    }
    assert!(saw_runtime_changed);

    for _ in 0..3 {
        let reloaded = runtime_status_list(&state).unwrap();
        assert_eq!(
            reloaded
                .runtimes
                .iter()
                .find(|r| r.name == Runtime::ClaudeCode)
                .unwrap()
                .sign_in,
            RuntimeSignInStatus::Unknown
        );
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(probe_count(), 1);
        assert!(events.try_recv().is_err());
    }

    runtime_refresh(&state, &[]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while probe_count() < 2 || state.claude_accounts.is_refreshing() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for explicit recheck probe"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(probe_count(), 2);
    let mut emitted = Vec::new();
    while let Ok(evt) = events.try_recv() {
        emitted.push(evt.name);
    }
    assert!(!emitted.contains(&"claude_accounts/changed"));
}
