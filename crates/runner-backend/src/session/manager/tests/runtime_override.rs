use super::*;

fn assert_effective_command(command: &str, catalog_name: &str) {
    let path = std::path::Path::new(command);
    #[cfg(windows)]
    let name = path.file_stem();
    #[cfg(not(windows))]
    let name = path.file_name();
    assert_eq!(
        name.and_then(|name| name.to_str()),
        Some(catalog_name),
        "expected {catalog_name} or an absolute path ending in {catalog_name}, got {command}",
    );
}

#[test]
fn runtime_override_helper_distinguishes_absent_matching_and_differing() {
    let mut r = role("codex-custom", &["--custom"]);
    r.runtime = "codex".into();

    // Absent / blank: no rebuild, no pin.
    for value in [None, Some("  ")] {
        let res = resolve_runtime_override(&r, value, None, None).unwrap();
        assert!(res.effective.is_none());
        assert!(!res.pinned, "absent/blank override must not pin");
    }

    // Matching: no rebuild (spawn stays byte-identical), but pinned —
    // the session row must record the engine so a later role-
    // template edit can't re-engine its resume.
    let matching = resolve_runtime_override(&r, Some("codex"), None, None).unwrap();
    assert!(matching.effective.is_none());
    assert!(matching.pinned, "explicit matching override must pin");

    // Differing: rebuild + pin for every other catalog runtime.
    for runtime in ["claude-code", "trae", "copilot", "antigravity"] {
        let differing = resolve_runtime_override(&r, Some(runtime), None, None).unwrap();
        assert_eq!(
            differing.effective.as_ref().map(|r| r.runtime.as_str()),
            Some(runtime),
        );
        assert!(differing.pinned);
    }
}

#[test]
fn runtime_override_helper_resets_engine_fields_and_keeps_persona() {
    let mut r = role("codex-custom", &["--custom-flag"]);
    r.runtime = "codex".into();
    r.model = Some("gpt-5-codex".into());
    r.effort = Some("high".into());
    r.system_prompt = Some("persona".into());
    r.working_dir = Some("/work".into());
    r.env.insert("FOO".into(), "bar".into());

    let effective = resolve_runtime_override(&r, Some("claude-code"), None, None)
        .unwrap()
        .effective
        .expect("differing runtime must produce an effective role");
    // Engine fields reset to registry defaults.
    assert_eq!(effective.runtime, "claude-code");
    assert_eq!(effective.command, "claude");
    assert_eq!(
        effective.args,
        router::runtime::apply_permission_mode(
            Some(Runtime::ClaudeCode),
            &[],
            crate::ops::role::default_permission_mode(),
        ),
        "override args must be the registry default permission-mode pair",
    );
    assert!(!effective.args.contains(&"--custom-flag".to_string()));
    assert_eq!(effective.model, None);
    assert_eq!(effective.effort, None);
    // Persona fields carry over.
    assert_eq!(effective.system_prompt.as_deref(), Some("persona"));
    assert_eq!(effective.working_dir.as_deref(), Some("/work"));
    assert_eq!(effective.env.get("FOO").map(String::as_str), Some("bar"));
    assert_eq!(effective.id, r.id);
    assert_eq!(effective.handle, r.handle);
}

#[test]
fn runtime_override_helper_applies_slot_model_to_selected_runtime() {
    let mut r = role("codex-custom", &["--custom"]);
    r.runtime = "codex".into();
    r.model = Some("role-model".into());

    let differing = resolve_runtime_override(&r, Some("trae"), Some("trae-slot-model"), None)
        .unwrap()
        .effective
        .expect("differing runtime must produce an effective role");
    assert_eq!(differing.runtime, "trae");
    assert_eq!(differing.model.as_deref(), Some("trae-slot-model"));

    let matching = resolve_runtime_override(&r, Some("codex"), Some("codex-slot-model"), None)
        .unwrap()
        .effective
        .expect("a model override must rebuild even for a matching runtime");
    assert_eq!(matching.runtime, "codex");
    assert_eq!(matching.model.as_deref(), Some("codex-slot-model"));
    assert_eq!(matching.args, r.args);

    let unpinned = resolve_runtime_override(&r, None, Some("codex-slot-model"), None).unwrap();
    let effective = unpinned
        .effective
        .expect("a model-only override must rebuild the role config");
    assert_eq!(effective.runtime, "codex");
    assert_eq!(effective.model.as_deref(), Some("codex-slot-model"));
    assert_eq!(effective.effort, r.effort);
    assert!(!unpinned.pinned, "model-only overrides must not pin");
}

#[test]
fn runtime_override_helper_applies_effort_to_selected_runtime() {
    let mut r = role("codex-custom", &["--custom"]);
    r.runtime = "codex".into();
    r.model = Some("role-model".into());
    r.effort = Some("role-effort".into());

    let differing = resolve_runtime_override(&r, Some("claude-code"), Some("fable"), Some("max"))
        .unwrap()
        .effective
        .expect("differing runtime must produce an effective role");
    assert_eq!(differing.runtime, "claude-code");
    assert_eq!(differing.model.as_deref(), Some("fable"));
    assert_eq!(differing.effort.as_deref(), Some("max"));

    let cleared = resolve_runtime_override(&r, Some("claude-code"), None, None)
        .unwrap()
        .effective
        .expect("differing runtime must produce an effective role");
    assert_eq!(cleared.model, None);
    assert_eq!(cleared.effort, None);

    let matching = resolve_runtime_override(&r, Some("codex"), None, Some("xhigh"))
        .unwrap()
        .effective
        .expect("an effort override must rebuild even for a matching runtime");
    assert_eq!(matching.model.as_deref(), Some("role-model"));
    assert_eq!(matching.effort.as_deref(), Some("xhigh"));

    let unpinned = resolve_runtime_override(&r, None, None, Some("high")).unwrap();
    let effective = unpinned
        .effective
        .expect("an effort-only override must rebuild the role config");
    assert_eq!(effective.runtime, "codex");
    assert_eq!(effective.model.as_deref(), Some("role-model"));
    assert_eq!(effective.effort.as_deref(), Some("high"));
    assert!(!unpinned.pinned, "effort-only overrides must not pin");

    let blank = resolve_runtime_override(&r, Some("codex"), Some("  "), Some("")).unwrap();
    assert!(blank.effective.is_none());
    assert!(blank.pinned);
}

#[test]
fn runtime_override_helper_rejects_unknown_runtime() {
    let r = role("/bin/sh", &[]);
    let err = resolve_runtime_override(&r, Some("aider-future"), None, None).unwrap_err();
    assert!(err.to_string().contains("unknown runtime"), "got: {err}",);
}

#[test]
fn mission_spawn_with_slot_override_uses_registry_engine_and_records_runtime() {
    let pool = pool_with_schema();
    let mission_row = mission();
    let role_id = ulid::Ulid::new().to_string();
    let slot_id = insert_crew_role(&pool, &mission_row.id, &role_id);

    // Role row is a codex engine with custom flags + pinned
    // model/effort; the slot overrides to claude-code and selects
    // its own model and effort.
    let mut role = role("codex-custom", &["--custom-flag"]);
    role.id = role_id.clone();
    role.runtime = "codex".into();
    role.model = Some("gpt-5-codex".into());
    role.effort = Some("high".into());
    role.env.insert("FOO".into(), "bar".into());
    let mut slot = slot_for(&role);
    slot.id = slot_id;
    slot.runtime_override = Some("claude-code".into());
    slot.model_override = Some("opus".into());
    slot.effort_override = Some("max".into());

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    // Pin the app-wide mode off so the override's registry-default
    // pair reaches the spawn unchanged; convergence under Bypass /
    // Auto is covered by the `mission_spawn_converges_*` tests.
    mgr.set_mission_permission_mode(MissionPermissionMode::RoleDefault);
    let spawned = mgr
        .spawn(
            &mission_row,
            &role,
            &slot,
            fixture_tmp_dir(),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();

    let spec = fake.last_spawn_spec().expect("spawn was called");
    assert_effective_command(&spec.command, "claude");
    assert!(
        !spec.args.contains(&"--custom-flag".to_string()),
        "role args are engine flags and must not carry across runtimes: {:?}",
        spec.args,
    );
    assert!(spec
        .args
        .windows(2)
        .any(|w| w[0] == "--model" && w[1] == "opus"));
    assert!(spec
        .args
        .windows(2)
        .any(|w| w[0] == "--effort" && w[1] == "max"));
    assert!(
        spec.args
            .windows(2)
            .any(|w| w[0] == "--permission-mode" && w[1] == "auto"),
        "override args must be the registry default permission-mode pair: {:?}",
        spec.args,
    );
    assert!(
        spec.args.contains(&"--session-id".to_string()),
        "resume plan must be computed for the effective runtime: {:?}",
        spec.args,
    );
    assert_eq!(
        spec.env.get("FOO").map(String::as_str),
        Some("bar"),
        "persona env must carry over",
    );

    // Session row records the effective runtime for respawn/resume.
    let (agent_runtime, agent_command, agent_model, agent_effort): (
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT agent_runtime, agent_command, agent_model, agent_effort
               FROM sessions WHERE id = ?1",
            params![spawned.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .unwrap();
    assert_eq!(agent_runtime.as_deref(), Some("claude-code"));
    assert_effective_command(agent_command.as_deref().unwrap(), "claude");
    assert_eq!(agent_model.as_deref(), Some("opus"));
    assert_eq!(agent_effort.as_deref(), Some("max"));

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn mission_spawn_with_model_only_slot_override_uses_role_runtime_without_pinning() {
    let pool = pool_with_schema();
    let mission_row = mission();
    let role_id = ulid::Ulid::new().to_string();
    let slot_id = insert_crew_role(&pool, &mission_row.id, &role_id);

    let mut role = role("codex-custom", &["--custom-flag"]);
    role.id = role_id;
    role.runtime = "codex".into();
    role.model = Some("role-model".into());
    role.effort = Some("high".into());
    update_role_row(&pool.get().unwrap(), &role);
    let mut slot = slot_for(&role);
    slot.id = slot_id;
    slot.model_override = Some("slot-model".into());

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn(
            &mission_row,
            &role,
            &slot,
            fixture_tmp_dir(),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();

    let spec = fake.last_spawn_spec().expect("spawn was called");
    assert_eq!(spec.command, "codex-custom");
    assert!(spec.args.contains(&"--custom-flag".to_string()));
    assert!(spec
        .args
        .windows(2)
        .any(|w| w[0] == "--model" && w[1] == "slot-model"));
    assert!(spec
        .args
        .windows(2)
        .any(|w| w[0] == "-c" && w[1] == "model_reasoning_effort=high"));

    let (agent_runtime, agent_model, agent_effort): (
        Option<String>,
        Option<String>,
        Option<String>,
    ) = pool
        .get()
        .unwrap()
        .query_row(
            "SELECT agent_runtime, agent_model, agent_effort
               FROM sessions WHERE id = ?1",
            params![spawned.id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(agent_runtime, None, "model-only overrides must not pin");
    assert_eq!(agent_model.as_deref(), Some("slot-model"));
    assert_eq!(agent_effort.as_deref(), Some("high"));

    mgr.kill(&spawned.id).unwrap();
    mgr.resume(
        &spawned.id,
        None,
        None,
        fixture_tmp_dir(),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();
    let resumed = fake.last_spawn_spec().expect("resume should spawn");
    assert_eq!(resumed.command, "codex-custom");
    assert!(resumed
        .args
        .windows(2)
        .any(|w| w[0] == "--model" && w[1] == "slot-model"));
    assert!(resumed
        .args
        .windows(2)
        .any(|w| w[0] == "-c" && w[1] == "model_reasoning_effort=high"));
    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn mission_spawn_with_matching_override_keeps_args_and_pins_runtime() {
    // An override naming the role's own runtime must spawn
    // byte-identically to no override (same command, same args) but
    // still record the effective runtime on the row: the slot is
    // explicitly pinned, so a later edit to the role template's
    // runtime must not re-engine this session's resume.
    let pool = pool_with_schema();
    let mission_row = mission();
    let role_id = ulid::Ulid::new().to_string();
    let slot_id = insert_crew_role(&pool, &mission_row.id, &role_id);

    // "codex" is a registry runtime — the only kind the slot write
    // validator can actually store as an override.
    let mut role = role("codex-custom", &["--custom-flag"]);
    role.id = role_id.clone();
    role.runtime = "codex".into();
    let mut slot = slot_for(&role);
    slot.id = slot_id;

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));

    // Baseline: no override.
    slot.runtime_override = None;
    let baseline = mgr
        .spawn(
            &mission_row,
            &role,
            &slot,
            fixture_tmp_dir(),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    let baseline_spec = fake.last_spawn_spec().expect("baseline spawn");

    // Matching override.
    slot.runtime_override = Some("codex".into());
    let pinned = mgr
        .spawn(
            &mission_row,
            &role,
            &slot,
            fixture_tmp_dir(),
            PathBuf::from("/dev/null"),
            Arc::clone(&pool),
            capture(),
            None,
        )
        .unwrap();
    let pinned_spec = fake.last_spawn_spec().expect("pinned spawn");

    assert_eq!(pinned_spec.command, baseline_spec.command);
    assert_eq!(
        pinned_spec
            .args
            .iter()
            .map(|arg| arg.replace(&pinned_spec.session_id, "SESSION"))
            .collect::<Vec<_>>(),
        baseline_spec
            .args
            .iter()
            .map(|arg| arg.replace(&baseline_spec.session_id, "SESSION"))
            .collect::<Vec<_>>(),
        "matching override must preserve args apart from the owning status path",
    );
    assert_eq!(pinned_spec.command, "codex-custom");

    let runtime_for = |id: &str| -> (Option<String>, Option<String>) {
        pool.get()
            .unwrap()
            .query_row(
                "SELECT agent_runtime, agent_command FROM sessions WHERE id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap()
    };
    assert_eq!(
        runtime_for(&baseline.id),
        (None, None),
        "no override must not record agent_runtime",
    );
    assert_eq!(
        runtime_for(&pinned.id),
        (Some("codex".into()), Some("codex-custom".into())),
        "matching override must pin the effective runtime on the row",
    );

    mgr.kill(&baseline.id).unwrap();
    mgr.kill(&pinned.id).unwrap();
}

#[test]
fn resume_keeps_pinned_runtime_after_role_template_edit() {
    // The scenario the pin exists for: a session spawned with an
    // explicit override matching the role's then-runtime ("codex"),
    // recorded on the row. The user later edits the role template
    // to claude-code. Resume must respawn this session on codex —
    // registry defaults — not on the template's new runtime, which
    // would hand the codex-native session key to the wrong CLI.
    let pool = pool_with_schema();
    let role_id = ulid::Ulid::new().to_string();
    {
        let conn = pool.get().unwrap();
        let mut persisted_role = role("codex-custom", &["--custom-flag"]);
        persisted_role.id = role_id.clone();
        persisted_role.runtime = "codex".into();
        crate::repo::role::insert(&conn, &crate::repo::role::RoleRow::from(&persisted_role))
            .unwrap();
        let mut row =
            crate::test_support::test_session_row("pin-sid", crate::model::SessionStatus::Stopped);
        row.role_id = Some(role_id.clone());
        row.cwd = Some(fixture_tmp_dir().to_string_lossy().into_owned());
        row.agent_runtime = Some("codex".into());
        row.agent_command = Some("codex-custom".into());
        crate::repo::session::insert(&conn, &row).unwrap();
        // The role template moves on to a different engine.
        persisted_role.runtime = "claude-code".into();
        persisted_role.command = "claude-custom".into();
        crate::repo::role::update(&conn, &crate::repo::role::RoleRow::from(&persisted_role))
            .unwrap();
    }

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    mgr.resume(
        "pin-sid",
        None,
        None,
        fixture_tmp_dir(),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();

    let spec = fake.last_spawn_spec().expect("resume should have spawned");
    assert_effective_command(&spec.command, "codex");
    assert!(
        !spec.args.contains(&"--custom-flag".to_string()) && spec.command != "claude-custom",
        "neither the template's new engine nor its old flags may leak in: {:?}",
        spec.args,
    );

    mgr.kill("pin-sid").unwrap();
}

#[test]
fn direct_spawn_with_override_uses_registry_engine_and_records_runtime() {
    let pool = pool_with_schema();
    let mut role = role("codex-custom", &["--custom-flag"]);
    role.runtime = "codex".into();
    insert_role_row(&pool.get().unwrap(), &role);

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    let spawned = mgr
        .spawn_direct(
            &role,
            Some("claude-code"),
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
    assert_effective_command(&spec.command, "claude");
    assert!(!spec.args.contains(&"--custom-flag".to_string()));

    let stored = crate::repo::session::get_row(&pool.get().unwrap(), &spawned.id)
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.role_id.as_deref(),
        Some(role.id.as_str()),
        "overridden chats stay role-backed",
    );
    assert_eq!(stored.agent_runtime.as_deref(), Some("claude-code"));
    assert_effective_command(stored.agent_command.as_deref().unwrap(), "claude");

    mgr.kill(&spawned.id).unwrap();
}

#[test]
fn resume_respawns_recorded_override_runtime() {
    // A stopped role-backed session that recorded an effective
    // runtime must resume on that engine — not the role row's —
    // with registry defaults instead of the role's engine flags.
    let pool = pool_with_schema();
    let role_id = ulid::Ulid::new().to_string();
    let key = uuid::Uuid::new_v4().to_string();
    {
        let conn = pool.get().unwrap();
        let mut persisted_role = role("codex-custom", &["--custom-flag"]);
        persisted_role.id = role_id.clone();
        persisted_role.runtime = "codex".into();
        crate::repo::role::insert(&conn, &crate::repo::role::RoleRow::from(&persisted_role))
            .unwrap();
        let mut row =
            crate::test_support::test_session_row("ovr-sid", crate::model::SessionStatus::Stopped);
        row.role_id = Some(role_id.clone());
        row.cwd = Some(fixture_tmp_dir().to_string_lossy().into_owned());
        row.agent_session_key = Some(key.clone());
        row.agent_runtime = Some("claude-code".into());
        row.agent_command = Some("claude".into());
        crate::repo::session::insert(&conn, &row).unwrap();
    }

    let fake = fake_runtime();
    let mgr = mgr_with_fake(None, Arc::clone(&fake));
    mgr.resume(
        "ovr-sid",
        None,
        None,
        fixture_tmp_dir(),
        Arc::clone(&pool),
        capture(),
    )
    .unwrap();

    let spec = fake.last_spawn_spec().expect("resume should have spawned");
    assert_effective_command(&spec.command, "claude");
    assert!(
        !spec.args.contains(&"--custom-flag".to_string()),
        "role engine flags must not leak into an overridden resume: {:?}",
        spec.args,
    );
    assert!(
        spec.args
            .windows(2)
            .any(|w| w[0] == "--resume" && w[1] == key),
        "resume must hand the prior agent_session_key to the effective runtime: {:?}",
        spec.args,
    );

    mgr.kill("ovr-sid").unwrap();
}
