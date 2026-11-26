use super::*;
use crate::surfaces::start_chat::launcher_model::EntryKey;
use gpui::{px, size, Entity, Modifiers, TestAppContext, VisualTestContext};
use runner_core::protocol::onboarding::StarterCrewSlotProposal;
use runner_core::protocol::runtime::{
    RuntimeExecutableStatus, RuntimeRowState, RuntimeSignInStatus, ShellDiscoveryStatus,
};
use runner_core::protocol::{
    DiscoveryOutcome, RuntimeTaskKind, RuntimeUpdateState, RuntimeUpdateTask,
};

fn runtime_exec(
    name: Runtime,
    display_name: &str,
    state: RuntimeRowState,
    sign_in: RuntimeSignInStatus,
    installed_version: Option<&str>,
) -> RuntimeExecutableStatus {
    RuntimeExecutableStatus {
        launch_options: Default::default(),
        name,
        display_name: display_name.into(),
        command: name.key().into(),
        default_model: None,
        default_effort: None,
        detected_path: (state == RuntimeRowState::Detected)
            .then(|| format!("/usr/local/bin/{}", name.key())),
        override_path: None,
        effective_command: (state == RuntimeRowState::Detected).then(|| name.key().into()),
        effective_source: None,
        state,
        invalid_reason: None,
        installed_version: installed_version.map(str::to_owned),
        available_version: None,
        min_version: None,
        below_minimum: false,
        sign_in,
    }
}

fn runtime_status(
    npm_available: bool,
    runtimes: Vec<RuntimeExecutableStatus>,
) -> RuntimeStatusResponse {
    RuntimeStatusResponse {
        shell: ShellDiscoveryStatus {
            shell: Some("/bin/zsh".into()),
            outcome: Some(DiscoveryOutcome::Ok),
            duration_ms: Some(10),
            checking: false,
            using_last_known_good: false,
            last_known_good_captured_at: None,
        },
        npm_available,
        runtimes,
    }
}

fn test_catalog() -> Vec<RuntimeCatalogEntry> {
    runtime_metadata::runtime_list()
        .into_iter()
        .map(|meta| {
            let capabilities = RuntimeCatalogEntry::for_runtime(meta.name)
                .map(|entry| entry.capabilities)
                .unwrap_or_default();
            RuntimeCatalogEntry {
                name: meta.name,
                display_name: meta.display_name,
                command: meta.command,
                description: String::new(),
                install_url: if meta.name == Runtime::Trae {
                    String::new()
                } else {
                    format!("https://example.com/{}/install", meta.name)
                },
                default_enabled: true,
                available: false,
                native_fork: meta.native_fork,
                default_model: None,
                default_effort: None,
                installed_version: None,
                available_version: None,
                min_version: None,
                below_minimum: false,
                models: Vec::new(),
                efforts: Vec::new(),
                capabilities,
            }
        })
        .collect()
}

fn install_task(runtime: Runtime, state: RuntimeUpdateState) -> RuntimeUpdateTask {
    RuntimeUpdateTask {
        id: format!("install-{runtime}"),
        runtime,
        kind: RuntimeTaskKind::Install,
        state,
        from_version: None,
        to_version: None,
        installed_version: None,
        step: String::new(),
        message: None,
        exit_code: None,
        running_sessions: 0,
    }
}

fn two_slot_proposal() -> StarterCrewProposal {
    StarterCrewProposal {
        name: "Pair coding".into(),
        slots: vec![
            StarterCrewSlotProposal {
                handle: "coder".into(),
                display_name: "Coder".into(),
                runtime: Runtime::ClaudeCode,
            },
            StarterCrewSlotProposal {
                handle: "reviewer".into(),
                display_name: "Reviewer".into(),
                runtime: Runtime::Codex,
            },
        ],
    }
}

fn open_test_root_with_size(
    temp: &tempfile::TempDir,
    cx: &mut TestAppContext,
    window_size: gpui::Size<gpui::Pixels>,
    init: impl FnOnce(&mut NativeRoot, &mut Context<NativeRoot>),
) -> (
    Entity<crate::app_store::AppStore>,
    gpui::WindowHandle<NativeRoot>,
    VisualTestContext,
) {
    let _ = crate::theme_snapshot::ThemeGuard::new();
    let store = crate::app_store::test_lifecycle_store(cx, temp.path());
    let handle = cx.add_window(|window, cx| {
        window.resize(window_size);
        let mut root = NativeRoot::new(
            "main".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.setup.loaded_once = true;
        root.setup.catalog = test_catalog();
        init(&mut root, cx);
        root
    });
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(window_size);
    let _ = handle.update(&mut visual, |_, window, _| window.refresh());
    visual.run_until_parked();
    (store, handle, visual)
}

fn open_test_root(
    temp: &tempfile::TempDir,
    cx: &mut TestAppContext,
    init: impl FnOnce(&mut NativeRoot, &mut Context<NativeRoot>),
) -> (
    Entity<crate::app_store::AppStore>,
    gpui::WindowHandle<NativeRoot>,
    VisualTestContext,
) {
    open_test_root_with_size(temp, cx, size(px(1200.), px(960.)), init)
}

fn wait_until(
    visual: &mut VisualTestContext,
    mut predicate: impl FnMut(&mut VisualTestContext) -> bool,
) {
    for _ in 0..200 {
        visual.run_until_parked();
        if predicate(visual) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("condition not satisfied before timeout");
}

fn click_selector(visual: &mut VisualTestContext, selector: &'static str) {
    let bounds = visual.debug_bounds(selector).expect(selector);
    visual.simulate_click(bounds.center(), Modifiers::default());
    visual.run_until_parked();
}

#[test]
fn partition_setup_runtimes_preserves_catalog_order_across_status_and_tasks() {
    let mut runtimes = vec![
        runtime_exec(
            Runtime::OpenCode,
            "OpenCode",
            RuntimeRowState::NotFound,
            RuntimeSignInStatus::Unknown,
            None,
        ),
        runtime_exec(
            Runtime::Copilot,
            "GitHub Copilot",
            RuntimeRowState::Detected,
            RuntimeSignInStatus::Unknown,
            Some("1.2.0"),
        ),
        runtime_exec(
            Runtime::Pi,
            "pi",
            RuntimeRowState::Detected,
            RuntimeSignInStatus::Unknown,
            Some("1.5.2"),
        ),
        runtime_exec(
            Runtime::Codex,
            "Codex",
            RuntimeRowState::NotFound,
            RuntimeSignInStatus::Unknown,
            None,
        ),
        runtime_exec(
            Runtime::Antigravity,
            "Antigravity",
            RuntimeRowState::Detected,
            RuntimeSignInStatus::SignedIn,
            Some("1.5.9"),
        ),
        runtime_exec(
            Runtime::ClaudeCode,
            "Claude Code",
            RuntimeRowState::Detected,
            RuntimeSignInStatus::SignedIn,
            Some("1.0.0"),
        ),
    ];
    let (initial, overflow) = partition_setup_runtimes(&runtimes);
    assert_eq!(
        initial.iter().map(|r| r.name).collect::<Vec<_>>(),
        vec![
            Runtime::ClaudeCode,
            Runtime::Codex,
            Runtime::Antigravity,
            Runtime::Pi
        ]
    );
    assert_eq!(
        overflow.iter().map(|r| r.name).collect::<Vec<_>>(),
        vec![Runtime::Copilot, Runtime::OpenCode]
    );

    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (_store, _handle, mut visual) = open_test_root(&temp, &mut cx, |root, _| {
        root.setup.status = Some(runtime_status(true, runtimes.clone()));
        root.setup.proposal = Some(two_slot_proposal());
        root.agent_tasks.apply(vec![install_task(
            Runtime::OpenCode,
            RuntimeUpdateState::Running,
        )]);
    });

    // Collapsed rows keep the fixed top four in catalog order even with uninstalled Codex
    // and installed / active-task overflow runtimes.
    let collapsed_order = [
        "SETUP_AGENT_ROW_claude-code",
        "SETUP_AGENT_ROW_codex",
        "SETUP_AGENT_ROW_antigravity",
        "SETUP_AGENT_ROW_pi",
    ]
    .into_iter()
    .map(|selector| {
        visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing collapsed row {selector}"))
            .origin
            .y
    })
    .collect::<Vec<_>>();
    assert!(collapsed_order.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(visual.debug_bounds("SETUP_INSTALL_codex").is_some());
    assert!(visual.debug_bounds("SETUP_AGENT_ROW_copilot").is_none());
    assert!(visual.debug_bounds("SETUP_AGENT_ROW_opencode").is_none());

    click_selector(&mut visual, "SETUP_MORE_AGENTS");
    let expanded_order = [
        "SETUP_AGENT_ROW_claude-code",
        "SETUP_AGENT_ROW_codex",
        "SETUP_AGENT_ROW_antigravity",
        "SETUP_AGENT_ROW_pi",
        "SETUP_AGENT_ROW_copilot",
        "SETUP_AGENT_ROW_opencode",
    ]
    .into_iter()
    .map(|selector| {
        visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing expanded row {selector}"))
            .origin
            .y
    })
    .collect::<Vec<_>>();
    assert!(expanded_order.windows(2).all(|pair| pair[0] < pair[1]));
    assert!(visual.debug_bounds("SETUP_INSTALL_STOP_opencode").is_some());

    for runtime in &mut runtimes {
        runtime.state = RuntimeRowState::Detected;
    }
    let (all_installed_initial, all_installed_overflow) = partition_setup_runtimes(&runtimes);
    assert_eq!(
        all_installed_initial
            .iter()
            .map(|r| r.name)
            .collect::<Vec<_>>(),
        vec![
            Runtime::ClaudeCode,
            Runtime::Codex,
            Runtime::Antigravity,
            Runtime::Pi
        ]
    );
    assert_eq!(
        all_installed_overflow
            .iter()
            .map(|r| r.name)
            .collect::<Vec<_>>(),
        vec![Runtime::Copilot, Runtime::OpenCode]
    );
}

#[test]
fn setup_visible_respects_onboarding_split_remote_and_update_retry() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, handle, mut visual) = open_test_root(&temp, &mut cx, |root, _| {
        root.setup.status = Some(runtime_status(true, Vec::new()));
        root.setup.proposal = Some(two_slot_proposal());
    });

    assert!(handle
        .read_with(&visual, |root, cx| root.setup_visible(cx))
        .unwrap());
    assert!(visual.debug_bounds("SETUP_SURFACE").is_some());

    // Active update-wait gate takes precedence over incomplete onboarding.
    handle
        .update(&mut visual, |root, window, cx| {
            root.open_launcher_page(
                ProjectScope::Root,
                super::super::launcher::Step::Start,
                false,
                window,
                cx,
            );
            let launcher = root.render_launcher(None, true, window, cx);
            launcher.update(cx, |launcher, cx| {
                launcher.set_update_retry_for_test(Runtime::Codex, cx);
            });
            assert!(!root.setup_visible(cx));
        })
        .unwrap();

    // Completing onboarding hides Setup even on a fresh root window.
    store.update(&mut cx, |store, _| {
        runner_daemon::ops::onboarding::complete(&store.test_core).unwrap();
    });
    cx.run_until_parked();

    let temp2 = tempfile::tempdir().unwrap();
    let mut cx2 = TestAppContext::single();
    let (store2, handle2, mut visual2) = open_test_root(&temp2, &mut cx2, |root, _| {
        root.setup.status = Some(runtime_status(true, Vec::new()));
    });
    store2.update(&mut cx2, |store, _| {
        runner_daemon::ops::onboarding::complete(&store.test_core).unwrap();
    });
    cx2.run_until_parked();
    visual2.run_until_parked();
    assert!(!handle2
        .read_with(&visual2, |root, cx| root.setup_visible(cx))
        .unwrap());
    assert!(visual2.debug_bounds("START_SURFACE").is_some());

    // Reopening setup marks setup.open without clearing onboarding_completed;
    // opening a split/tab pane still hides Setup.
    handle2
        .update(&mut visual2, |root, window, cx| {
            root.open_setup(window, cx);
            assert!(root.setup_visible(cx));
            assert!(store2.read(cx).onboarding_completed());
            root.test_open_empty_tab(window, cx);
            assert!(!root.setup_visible(cx));
        })
        .unwrap();

    // Remote targets never replace their launcher with local Setup.
    let temp_remote = tempfile::tempdir().unwrap();
    let mut cx_remote = TestAppContext::single();
    let (remote_store, _fake, _) =
        crate::surfaces::remote_client::tests::fixture(&mut cx_remote, temp_remote.path());
    let remote_window = crate::surfaces::remote_client::tests::add_window(
        &mut cx_remote,
        &remote_store,
        temp_remote.path(),
        "remote-setup",
    );
    remote_window
        .update(
            &mut cx_remote,
            crate::surfaces::remote_client::tests::choose_host,
        )
        .unwrap();
    cx_remote.run_until_parked();
    assert!(!remote_window
        .read_with(&cx_remote, |root, cx| root.setup_visible(cx))
        .unwrap());
}

#[test]
fn setup_renders_agent_states_overflow_and_install_actions() {
    let make_runtimes = || {
        vec![
            runtime_exec(
                Runtime::ClaudeCode,
                "Claude Code",
                RuntimeRowState::Detected,
                RuntimeSignInStatus::SignedIn,
                Some("2.0.1"),
            ),
            runtime_exec(
                Runtime::Codex,
                "Codex",
                RuntimeRowState::Detected,
                RuntimeSignInStatus::SignInNeeded,
                Some("0.155.0"),
            ),
            runtime_exec(
                Runtime::Antigravity,
                "Antigravity",
                RuntimeRowState::Detected,
                RuntimeSignInStatus::Unknown,
                Some("0.5.0"),
            ),
            runtime_exec(
                Runtime::Pi,
                "pi",
                RuntimeRowState::NotFound,
                RuntimeSignInStatus::Unknown,
                None,
            ),
            runtime_exec(
                Runtime::Cursor,
                "Cursor CLI",
                RuntimeRowState::NotFound,
                RuntimeSignInStatus::Unknown,
                None,
            ),
        ]
    };

    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (_store, handle, mut visual) = open_test_root(&temp, &mut cx, |root, _| {
        root.setup.status = Some(runtime_status(true, make_runtimes()));
        root.setup.proposal = Some(two_slot_proposal());
    });

    assert!(visual.debug_bounds("SETUP_AGENTS_COUNT").is_some());
    assert!(visual.debug_bounds("SETUP_AGENT_ROW_claude-code").is_some());
    assert!(visual
        .debug_bounds("SETUP_OPEN_TERMINAL_claude-code")
        .is_none());
    assert!(visual.debug_bounds("SETUP_AGENT_ROW_codex").is_some());
    assert!(visual.debug_bounds("SETUP_OPEN_TERMINAL_codex").is_some());
    assert!(visual.debug_bounds("SETUP_AGENT_ROW_antigravity").is_some());
    assert!(visual
        .debug_bounds("SETUP_OPEN_TERMINAL_antigravity")
        .is_none());
    assert!(visual.debug_bounds("SETUP_INSTALL_pi").is_some());
    assert!(visual.debug_bounds("SETUP_COPY_pi").is_some());
    assert!(visual.debug_bounds("SETUP_AGENT_ROW_cursor").is_none());

    // Copy npm command writes expected install command to clipboard.
    click_selector(&mut visual, "SETUP_COPY_pi");
    assert_eq!(
        visual.read_from_clipboard().and_then(|item| item.text()),
        runner_core::protocol::npm_install_command(Runtime::Pi)
    );

    // Reveal overflow row for non-npm agent (Cursor) and verify Copy writes its script.
    click_selector(&mut visual, "SETUP_MORE_AGENTS");
    assert!(visual.debug_bounds("SETUP_AGENT_ROW_cursor").is_some());
    assert!(visual.debug_bounds("SETUP_COPY_cursor").is_some());
    assert!(visual.debug_bounds("SETUP_GUIDE_cursor").is_some());
    click_selector(&mut visual, "SETUP_COPY_cursor");
    assert_eq!(
        visual
            .read_from_clipboard()
            .and_then(|item| item.text())
            .as_deref(),
        Some("curl https://cursor.com/install -fsS | bash")
    );

    // Running, MayNeedInput, and Failed install task states render Stop / Answer / Retry.
    handle
        .update(&mut visual, |root, _, cx| {
            root.agent_tasks
                .apply(vec![install_task(Runtime::Pi, RuntimeUpdateState::Running)]);
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("SETUP_INSTALL_STOP_pi").is_some());

    assert_eq!(
        setup_install_task_badge_key(RuntimeUpdateState::MayNeedInput),
        "onboarding.may_need_input"
    );
    assert_eq!(
        crate::i18n::text("onboarding.may_need_input"),
        "May need input"
    );

    handle
        .update(&mut visual, |root, _, cx| {
            root.agent_tasks.apply(vec![install_task(
                Runtime::Pi,
                RuntimeUpdateState::MayNeedInput,
            )]);
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("SETUP_INSTALL_ANSWER_pi").is_some());

    handle
        .update(&mut visual, |root, _, cx| {
            root.agent_tasks
                .apply(vec![install_task(Runtime::Pi, RuntimeUpdateState::Failed)]);
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("SETUP_INSTALL_RETRY_pi").is_some());

    // Even once the executable appears on PATH (e.g. npm symlink mid-install or leftover after
    // failure), active/input-needed/failed Install tasks win until Succeeded.
    handle
        .update(&mut visual, |root, _, cx| {
            let mut detected = make_runtimes();
            let pi = detected.iter_mut().find(|r| r.name == Runtime::Pi).unwrap();
            pi.state = RuntimeRowState::Detected;
            pi.sign_in = RuntimeSignInStatus::SignInNeeded;
            pi.detected_path = Some("/usr/local/bin/pi".into());
            pi.installed_version = Some("0.9.0".into());
            root.setup.status = Some(runtime_status(true, detected));
            root.agent_tasks
                .apply(vec![install_task(Runtime::Pi, RuntimeUpdateState::Running)]);
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("SETUP_INSTALL_STOP_pi").is_some());
    assert!(visual.debug_bounds("SETUP_OPEN_TERMINAL_pi").is_none());

    handle
        .update(&mut visual, |root, _, cx| {
            root.agent_tasks.apply(vec![install_task(
                Runtime::Pi,
                RuntimeUpdateState::MayNeedInput,
            )]);
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("SETUP_INSTALL_ANSWER_pi").is_some());
    assert!(visual.debug_bounds("SETUP_OPEN_TERMINAL_pi").is_none());

    handle
        .update(&mut visual, |root, _, cx| {
            root.agent_tasks
                .apply(vec![install_task(Runtime::Pi, RuntimeUpdateState::Failed)]);
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("SETUP_INSTALL_RETRY_pi").is_some());
    assert!(visual.debug_bounds("SETUP_OPEN_TERMINAL_pi").is_none());

    handle
        .update(&mut visual, |root, _, cx| {
            root.agent_tasks.apply(vec![install_task(
                Runtime::Pi,
                RuntimeUpdateState::Succeeded,
            )]);
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("SETUP_OPEN_TERMINAL_pi").is_some());

    // Without npm, npm-packaged Not installed agents offer a Node.js download link.
    let temp_no_npm = tempfile::tempdir().unwrap();
    let mut cx_no_npm = TestAppContext::single();
    let (_store, _handle, mut visual_no_npm) =
        open_test_root(&temp_no_npm, &mut cx_no_npm, |root, _| {
            root.setup.status = Some(runtime_status(false, make_runtimes()));
            root.setup.proposal = Some(two_slot_proposal());
        });
    assert!(visual_no_npm.debug_bounds("SETUP_NODEJS_pi").is_some());
    assert!(visual_no_npm.debug_bounds("SETUP_INSTALL_pi").is_none());
}

#[test]
fn setup_starter_crew_skip_reopen_sample_prompt_and_zero_crew_launcher() {
    // Needs-an-agent proposal hides the starter toggle.
    let temp_ineligible = tempfile::tempdir().unwrap();
    let mut cx_ineligible = TestAppContext::single();
    let (_store, _handle, mut visual_ineligible) =
        open_test_root(&temp_ineligible, &mut cx_ineligible, |root, _| {
            root.setup.status = Some(runtime_status(true, Vec::new()));
            root.setup.proposal = Some(StarterCrewProposal {
                name: "Pair coding".into(),
                slots: Vec::new(),
            });
        });
    assert!(visual_ineligible
        .debug_bounds("SETUP_STARTER_CREW")
        .is_some());
    assert!(visual_ineligible
        .debug_bounds("SETUP_STARTER_TOGGLE")
        .is_none());

    // Toggle Will create <-> Won't create, then Skip setup and inspect zero-crew launcher.
    let temp_skip = tempfile::tempdir().unwrap();
    let mut cx_skip = TestAppContext::single();
    let (store_skip, handle_skip, mut visual_skip) =
        open_test_root(&temp_skip, &mut cx_skip, |root, _| {
            root.setup.status = Some(runtime_status(true, Vec::new()));
            root.setup.proposal = Some(two_slot_proposal());
        });
    assert!(visual_skip.debug_bounds("SETUP_STARTER_TOGGLE").is_some());
    click_selector(&mut visual_skip, "SETUP_STARTER_TOGGLE");
    assert_eq!(
        handle_skip
            .read_with(&visual_skip, |root, _| root.setup.create_starter_crew)
            .unwrap(),
        Some(false)
    );
    click_selector(&mut visual_skip, "SETUP_STARTER_TOGGLE");
    assert_eq!(
        handle_skip
            .read_with(&visual_skip, |root, _| root.setup.create_starter_crew)
            .unwrap(),
        Some(true)
    );

    click_selector(&mut visual_skip, "SETUP_SKIP");
    wait_until(&mut visual_skip, |visual| {
        handle_skip
            .read_with(visual, |root, cx| !root.setup_visible(cx))
            .unwrap()
    });
    assert!(visual_skip.debug_bounds("START_SURFACE").is_some());
    assert!(store_skip.read_with(&visual_skip, |store, _| {
        store.onboarding_completed() && !store.has_crews()
    }));

    // Switching to Crews step on zero-crew DB shows "No crews yet", Create a crew, and Open setup.
    handle_skip
        .update(&mut visual_skip, |root, window, cx| {
            root.open_launcher_page(
                ProjectScope::Root,
                super::super::launcher::Step::Crews,
                false,
                window,
                cx,
            );
        })
        .unwrap();
    wait_until(&mut visual_skip, |visual| {
        visual.debug_bounds("LAUNCHER_NO_CREWS").is_some()
    });
    assert!(visual_skip
        .debug_bounds("LAUNCHER_NO_CREWS_CREATE")
        .is_some());
    assert!(visual_skip
        .debug_bounds("LAUNCHER_NO_CREWS_SETUP")
        .is_some());

    // Clicking Open setup reopens Setup without clearing onboarding_completed.
    click_selector(&mut visual_skip, "LAUNCHER_NO_CREWS_SETUP");
    assert!(handle_skip
        .read_with(&visual_skip, |root, cx| root.setup_visible(cx)
            && root.setup.open)
        .unwrap());
    assert!(store_skip.read_with(&visual_skip, |store, _| {
        runner_daemon::ops::onboarding::status(&store.test_core)
            .unwrap()
            .completed
    }));

    // Start your first chat creates starter crew "Pair coding" and prefills sample prompt.
    let fake_bin = tempfile::tempdir().unwrap();
    for binary in [
        Runtime::ClaudeCode.command().unwrap(),
        Runtime::Codex.command().unwrap(),
    ] {
        #[cfg(windows)]
        let file = fake_bin.path().join(format!("{binary}.cmd"));
        #[cfg(not(windows))]
        let file = fake_bin.path().join(binary);
        std::fs::write(&file, "#!/bin/sh\necho 99.0.0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&file).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&file, perms).unwrap();
        }
    }

    let temp_start = tempfile::tempdir().unwrap();
    let mut cx_start = TestAppContext::single();
    let (store_start, handle_start, mut visual_start) =
        open_test_root(&temp_start, &mut cx_start, |root, cx| {
            root.setup.status = Some(runtime_status(
                true,
                vec![
                    runtime_exec(
                        Runtime::ClaudeCode,
                        "Claude Code",
                        RuntimeRowState::Detected,
                        RuntimeSignInStatus::Unknown,
                        Some("99.0.0"),
                    ),
                    runtime_exec(
                        Runtime::Codex,
                        "Codex",
                        RuntimeRowState::Detected,
                        RuntimeSignInStatus::SignedIn,
                        Some("99.0.0"),
                    ),
                ],
            ));
            root.setup.proposal = Some(two_slot_proposal());
            root.app_store
                .read(cx)
                .test_core
                .runtime_discovery
                .write()
                .unwrap()
                .checking = false;
            let mut env = root
                .app_store
                .read(cx)
                .test_core
                .runtime_shell_env
                .write()
                .unwrap();
            env.path = Some(fake_bin.path().display().to_string());
            env.vars
                .insert("__RUNNER_TEST_ISOLATE_PATH".into(), "1".into());
            env.vars
                .insert("HOME".into(), fake_bin.path().display().to_string());
            env.vars
                .insert("OPENAI_API_KEY".into(), "sk-test-token".into());
        });
    click_selector(&mut visual_start, "SETUP_START_CHAT");
    wait_until(&mut visual_start, |visual| {
        handle_start
            .read_with(visual, |root, cx| !root.setup_visible(cx))
            .unwrap()
            && visual.debug_bounds("LAUNCHER_SAMPLE_PROMPT").is_some()
    });
    let crews = store_start.read_with(&visual_start, |store, _| store.crews.clone());
    assert_eq!(crews.len(), 1);
    assert_eq!(crews[0].crew.name, "Pair coding");
    assert_eq!(crews[0].members.len(), 2);
    assert_ne!(crews[0].members[0].runtime, crews[0].members[1].runtime);

    let launcher = handle_start
        .read_with(&visual_start, |root, cx| {
            let target = root.front_page_client(cx).target;
            root.launchers.page(&target).cloned().unwrap()
        })
        .unwrap();
    // Folder change keeps updating unedited sample prompt; manual edit disables auto-sync.
    launcher.update(&mut visual_start, |launcher, cx| {
        launcher.set_cwd_for_test("/tmp/alpha-repo", cx);
    });
    let synced = launcher.read_with(&visual_start, |launcher, cx| {
        launcher.sample_prompt_text_for_test(cx).unwrap()
    });
    assert!(
        synced.contains("alpha-repo"),
        "expected alpha-repo in {synced:?}"
    );

    launcher.update(&mut visual_start, |launcher, cx| {
        launcher.set_sample_prompt_for_test("Custom user prompt", cx);
        launcher.set_cwd_for_test("/tmp/beta-repo", cx);
    });
    let preserved = launcher.read_with(&visual_start, |launcher, cx| {
        launcher.sample_prompt_text_for_test(cx).unwrap()
    });
    assert_eq!(preserved, "Custom user prompt");

    let chat_start = super::super::launcher::launcher_input(
        &EntryKey::Agent("codex".into()),
        None,
        Some("/tmp/beta-repo".into()),
        None,
        (80, 24),
        Some(preserved),
    );
    assert_eq!(
        chat_start.initial_prompt.as_deref(),
        Some("Custom user prompt")
    );

    click_selector(&mut visual_start, "LAUNCHER_CLEAR_SAMPLE_PROMPT");
    assert_eq!(
        launcher.read_with(&visual_start, |launcher, cx| launcher
            .sample_prompt_text_for_test(cx)),
        None
    );

    // Reopening Setup when crews already exist hides Starter crew and keeps onboarding_completed.
    let temp_reopen = tempfile::tempdir().unwrap();
    let mut cx_reopen = TestAppContext::single();
    let (store_reopen, handle_reopen, mut visual_reopen) =
        open_test_root(&temp_reopen, &mut cx_reopen, |root, cx| {
            let core = root.app_store.read(cx).test_core.clone();
            core.runtime_discovery.write().unwrap().checking = false;
            {
                let mut env = core.runtime_shell_env.write().unwrap();
                env.path = Some(fake_bin.path().display().to_string());
                env.vars
                    .insert("__RUNNER_TEST_ISOLATE_PATH".into(), "1".into());
            }
            runner_daemon::ops::onboarding::create_starter_crew(&core, Default::default())
                .unwrap()
                .unwrap();
            runner_daemon::ops::onboarding::complete(&core).unwrap();
            root.app_store.update(cx, |store, _| {
                store.refresh_onboarding_inner();
                store.refresh_crews_inner();
            });
            root.setup.open = true;
            root.setup.status = Some(runtime_status(true, Vec::new()));
            root.setup.proposal = Some(two_slot_proposal());
        });
    assert!(handle_reopen
        .read_with(&visual_reopen, |root, cx| root.setup_visible(cx))
        .unwrap());
    assert!(
        store_reopen.read_with(&visual_reopen, |s, _| s.onboarding_completed()
            && s.has_crews())
    );
    assert!(visual_reopen.debug_bounds("SETUP_SURFACE").is_some());
    assert!(visual_reopen.debug_bounds("SETUP_STARTER_CREW").is_none());
}

#[test]
fn setup_start_chat_requires_resolved_detection_and_eligible_enabled_agent() {
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, handle, mut visual) = open_test_root(&temp, &mut cx, |root, _| {
        root.setup.status = Some(runtime_status(true, Vec::new()));
        root.setup.proposal = Some(StarterCrewProposal {
            name: "Pair coding".into(),
            slots: Vec::new(),
        });
    });

    // 1. No eligible agents keeps Start your first chat disabled and ignores complete calls.
    assert!(!handle
        .read_with(&visual, |root, cx| root.setup.can_start(root.settings(cx)))
        .unwrap());
    click_selector(&mut visual, "SETUP_START_CHAT");
    handle
        .update(&mut visual, |root, window, cx| {
            root.finish_setup(true, window, cx);
        })
        .unwrap();
    visual.run_until_parked();
    assert!(handle
        .read_with(&visual, |root, cx| root.setup_visible(cx))
        .unwrap());
    assert!(!store.read_with(&visual, |store, _| store.onboarding_completed()));

    // 2. Installed but disabled agent (and empty proposal) still cannot start.
    handle
        .update(&mut visual, |root, _, cx| {
            root.setup.status = Some(runtime_status(
                true,
                vec![runtime_exec(
                    Runtime::Codex,
                    "Codex",
                    RuntimeRowState::Detected,
                    RuntimeSignInStatus::SignedIn,
                    Some("0.155.0"),
                )],
            ));
            root.setup.proposal = Some(StarterCrewProposal {
                name: "Pair coding".into(),
                slots: vec![StarterCrewSlotProposal {
                    handle: "coder".into(),
                    display_name: "Coder".into(),
                    runtime: Runtime::Codex,
                }],
            });
            root.app_store.update(cx, |store, cx| {
                store.settings.disabled_agents.insert("codex".into());
                cx.notify();
            });
        })
        .unwrap();
    assert!(!handle
        .read_with(&visual, |root, cx| root.setup.can_start(root.settings(cx)))
        .unwrap());

    // 3. Re-enabling Codex enables Start, while unresolved probe states block it again.
    handle
        .update(&mut visual, |root, _, cx| {
            root.app_store.update(cx, |store, cx| {
                store.settings.disabled_agents.clear();
                cx.notify();
            });
            assert!(root.setup.can_start(root.settings(cx)));

            root.setup.loading = true;
            assert!(!root.setup.can_start(root.settings(cx)));
            root.setup.loading = false;

            root.setup.rechecking = true;
            assert!(!root.setup.can_start(root.settings(cx)));
            root.setup.rechecking = false;

            root.setup.status.as_mut().unwrap().shell.checking = true;
            assert!(!root.setup.can_start(root.settings(cx)));
            root.setup.status.as_mut().unwrap().shell.checking = false;

            let saved_proposal = root.setup.proposal.take();
            assert!(!root.setup.can_start(root.settings(cx)));
            root.setup.proposal = saved_proposal;
            assert!(root.setup.can_start(root.settings(cx)));
        })
        .unwrap();

    // 4. Skip setup stays available even when no eligible agent exists.
    handle
        .update(&mut visual, |root, _, cx| {
            root.setup.status = Some(runtime_status(true, Vec::new()));
            root.setup.proposal = Some(StarterCrewProposal {
                name: "Pair coding".into(),
                slots: Vec::new(),
            });
            assert!(!root.setup.can_start(root.settings(cx)));
            cx.notify();
        })
        .unwrap();
    click_selector(&mut visual, "SETUP_SKIP");
    wait_until(&mut visual, |visual| {
        handle
            .read_with(visual, |root, cx| !root.setup_visible(cx))
            .unwrap()
    });
    assert!(store.read_with(&visual, |store, _| store.onboarding_completed()));
}

#[test]
fn setup_centers_vertically_without_clipping_compact_windows() {
    let populated_runtimes = vec![
        runtime_exec(
            Runtime::ClaudeCode,
            "Claude Code",
            RuntimeRowState::Detected,
            RuntimeSignInStatus::SignedIn,
            Some("2.1.325"),
        ),
        runtime_exec(
            Runtime::Codex,
            "Codex",
            RuntimeRowState::Detected,
            RuntimeSignInStatus::SignedIn,
            Some("0.169.0"),
        ),
        runtime_exec(
            Runtime::Antigravity,
            "Antigravity CLI",
            RuntimeRowState::Detected,
            RuntimeSignInStatus::SignedIn,
            Some("1.5.9"),
        ),
        runtime_exec(
            Runtime::Pi,
            "pi",
            RuntimeRowState::Detected,
            RuntimeSignInStatus::Unknown,
            Some("1.5.2"),
        ),
    ];

    let tall_temp = tempfile::tempdir().unwrap();
    let mut tall_cx = TestAppContext::single();
    let (_, _, mut tall_visual) = open_test_root_with_size(
        &tall_temp,
        &mut tall_cx,
        size(px(1200.), px(960.)),
        |root, _| {
            root.setup.status = Some(runtime_status(true, populated_runtimes.clone()));
            root.setup.proposal = Some(two_slot_proposal());
        },
    );
    let tall_surface = tall_visual.debug_bounds("SETUP_SURFACE").unwrap();
    let tall_brand = tall_visual.debug_bounds("SETUP_BRAND").unwrap();
    let tall_start_chat = tall_visual.debug_bounds("SETUP_START_CHAT").unwrap();
    let top_gap = f32::from(tall_brand.origin.y - tall_surface.origin.y);
    let bottom_gap = f32::from(tall_surface.bottom() - tall_start_chat.bottom());
    assert!(
            top_gap > 60.,
            "expected Setup content to sit vertically centered rather than top-pinned (top_gap={top_gap}, bottom_gap={bottom_gap})"
        );
    assert!(
            (top_gap - bottom_gap).abs() <= 4.,
            "expected top and bottom vertical margins to balance (top_gap={top_gap}, bottom_gap={bottom_gap})"
        );

    let short_temp = tempfile::tempdir().unwrap();
    let mut short_cx = TestAppContext::single();
    let (_, _, mut short_visual) = open_test_root_with_size(
        &short_temp,
        &mut short_cx,
        size(px(1200.), px(480.)),
        |root, _| {
            root.setup.status = Some(runtime_status(true, populated_runtimes));
            root.setup.proposal = Some(two_slot_proposal());
        },
    );
    let short_surface = short_visual.debug_bounds("SETUP_SURFACE").unwrap();
    let short_brand = short_visual.debug_bounds("SETUP_BRAND").unwrap();
    assert!(
        short_surface.size.height < px(500.),
        "expected compact viewport height < 500px (got {:?})",
        short_surface.size.height
    );
    let compact_top_gap = f32::from(short_brand.origin.y - short_surface.origin.y);
    assert!(
            compact_top_gap >= 20.,
            "expected compact viewport to keep top header unclipped (compact_top_gap={compact_top_gap})"
        );
}

#[test]
fn setup_compact_scroll_clears_footer_above_background_task_pill() {
    let runtimes = vec![
        runtime_exec(
            Runtime::ClaudeCode,
            "Claude Code",
            RuntimeRowState::Detected,
            RuntimeSignInStatus::SignedIn,
            Some("2.1.325"),
        ),
        runtime_exec(
            Runtime::Codex,
            "Codex",
            RuntimeRowState::NotFound,
            RuntimeSignInStatus::Unknown,
            None,
        ),
        runtime_exec(
            Runtime::Antigravity,
            "Antigravity CLI",
            RuntimeRowState::Detected,
            RuntimeSignInStatus::SignedIn,
            Some("1.5.9"),
        ),
        runtime_exec(
            Runtime::Pi,
            "pi",
            RuntimeRowState::Detected,
            RuntimeSignInStatus::Unknown,
            Some("1.5.2"),
        ),
        runtime_exec(
            Runtime::Copilot,
            "GitHub Copilot CLI",
            RuntimeRowState::NotFound,
            RuntimeSignInStatus::Unknown,
            None,
        ),
        runtime_exec(
            Runtime::OpenCode,
            "OpenCode",
            RuntimeRowState::NotFound,
            RuntimeSignInStatus::Unknown,
            None,
        ),
    ];

    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (_store, handle, mut visual) =
        open_test_root_with_size(&temp, &mut cx, size(px(1000.), px(600.)), |root, _| {
            root.setup.status = Some(runtime_status(true, runtimes));
            root.setup.proposal = Some(two_slot_proposal());
            root.agent_tasks.apply(vec![install_task(
                Runtime::Copilot,
                RuntimeUpdateState::Running,
            )]);
        });

    let assert_footer_clears_pill = |visual: &mut VisualTestContext, label: &str| {
        let surface = visual.debug_bounds("SETUP_SURFACE").unwrap();
        let content = visual.debug_bounds("SETUP_CONTENT").unwrap();
        let max_offset = handle
            .read_with(visual, |root, _| root.setup.scroll.max_offset().y)
            .unwrap();
        assert!(
                max_offset > px(0.),
                "{label}: expected scrollable overflow (max_offset={max_offset:?}, surface={surface:?}, content={content:?})"
            );
        handle
            .update(visual, |root, _, cx| {
                root.setup
                    .scroll
                    .set_offset(gpui::point(px(0.), -max_offset));
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();

        let pill = visual
            .debug_bounds("BACKGROUND_TASK_PILL")
            .expect("expected active background task pill");
        let surface = visual.debug_bounds("SETUP_SURFACE").unwrap();
        let skip = visual.debug_bounds("SETUP_SKIP").unwrap();
        let start_chat = visual.debug_bounds("SETUP_START_CHAT").unwrap();

        assert!(
                    skip.bottom() <= surface.bottom() && skip.bottom() < pill.origin.y,
                    "{label}: expected Skip setup ({skip:?}) above pill ({pill:?}) and inside surface ({surface:?})"
                );
        assert!(
                    start_chat.bottom() <= surface.bottom()
                        && start_chat.bottom() + px(8.) <= pill.origin.y,
                    "{label}: expected Start your first chat ({start_chat:?}) clear above pill ({pill:?}) inside surface ({surface:?})"
                );
    };

    assert_footer_clears_pill(&mut visual, "collapsed");

    handle
        .update(&mut visual, |root, _, cx| {
            root.setup.more_agents = true;
            root.setup.scroll.set_offset(gpui::point(px(0.), px(0.)));
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("SETUP_AGENT_ROW_copilot").is_some());

    assert_footer_clears_pill(&mut visual, "expanded");
}

#[gpui::test]
fn setup_expanded_agent_list_bounds_scroll_clips_long_commands_and_keeps_1440x900_visible(
    cx: &mut TestAppContext,
) {
    let temp = tempfile::TempDir::new().unwrap();
    let catalog = test_catalog();
    let all_runtimes: Vec<_> = catalog
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            runtime_exec(
                entry.name,
                &entry.display_name,
                if index == 0 {
                    RuntimeRowState::Detected
                } else {
                    RuntimeRowState::NotFound
                },
                if index == 0 {
                    RuntimeSignInStatus::SignedIn
                } else {
                    RuntimeSignInStatus::Unknown
                },
                (index == 0).then_some("2.1.4"),
            )
        })
        .collect();

    let (_store, handle, mut visual) =
        open_test_root_with_size(&temp, cx, size(px(1440.), px(900.)), |root, _| {
            root.setup.status = Some(runtime_status(true, all_runtimes));
            root.setup.catalog = catalog;
            root.setup.proposal = Some(StarterCrewProposal {
                name: "Pair coding".into(),
                slots: vec![
                    StarterCrewSlotProposal {
                        handle: "coder".into(),
                        display_name: "Coder".into(),
                        runtime: Runtime::Antigravity,
                    },
                    StarterCrewSlotProposal {
                        handle: "reviewer".into(),
                        display_name: "Reviewer".into(),
                        runtime: Runtime::Copilot,
                    },
                ],
            });
        });

    assert!(visual.debug_bounds("SETUP_AGENT_ROW_pi").is_some());
    assert!(visual.debug_bounds("SETUP_AGENT_ROW_copilot").is_none());
    assert!(visual.debug_bounds("SETUP_AGENTS_SCROLL").is_none());
    assert!(visual.debug_bounds("SETUP_AGENTS_SCROLL_META").is_none());

    click_selector(&mut visual, "SETUP_MORE_AGENTS");
    visual.run_until_parked();

    assert!(handle
        .read_with(&visual, |root, _| root.setup.more_agents)
        .unwrap());
    let scroll_bounds = visual
        .debug_bounds("SETUP_AGENTS_SCROLL")
        .expect("expected bounded scroll viewport when expanded");
    let scrollbar_bounds = visual
        .debug_bounds("SETUP_AGENTS_SCROLLBAR")
        .expect("expected inner scrollbar track");
    assert!(
        scroll_bounds.size.height <= px(217.),
        "expected bounded viewport <= 216px (+1px tolerance), got {scroll_bounds:?}"
    );
    assert!(
        visual.debug_bounds("SETUP_AGENTS_SCROLL_META").is_some(),
        "expected right-hand overflow hint when 9 agents exceed 216px viewport"
    );
    let more_bounds = visual.debug_bounds("SETUP_MORE_AGENTS").unwrap();
    let card_bounds = visual.debug_bounds("SETUP_AGENTS_CARD").unwrap();
    assert!(more_bounds.origin.y >= scroll_bounds.bottom());
    assert!(more_bounds.bottom() <= card_bounds.bottom());

    let (outer_max, inner_max) = handle
        .read_with(&visual, |root, _| {
            (
                root.setup.scroll.max_offset().y,
                root.setup.agents_scroll.max_offset().y,
            )
        })
        .unwrap();
    let brand = visual.debug_bounds("SETUP_BRAND").unwrap();
    let card = visual.debug_bounds("SETUP_AGENTS_CARD").unwrap();
    let surface = visual.debug_bounds("SETUP_SURFACE").unwrap();
    let content = visual.debug_bounds("SETUP_CONTENT").unwrap();
    let starter = visual.debug_bounds("SETUP_STARTER_CREW").unwrap();
    let skip = visual.debug_bounds("SETUP_SKIP").unwrap();
    let start_chat = visual.debug_bounds("SETUP_START_CHAT").unwrap();
    assert_eq!(
            outer_max,
            px(0.),
            "expanded 9-agent card at 1440x900 must not trigger outer scroll (surface={surface:?}, content={content:?}, brand={brand:?}, card={card:?}, starter={starter:?}, start_chat={start_chat:?})"
        );
    assert!(
            content.size.height <= px(852.),
            "expected expanded 9-agent content height <= 852px so Linux/Windows custom-titlebar 1440x900 keeps outer scroll at 0px (surface={surface:?}, content={content:?})"
        );
    assert!(
            starter.origin.y >= surface.origin.y
                && starter.bottom() <= surface.bottom()
                && skip.bottom() <= surface.bottom()
                && start_chat.bottom() <= surface.bottom(),
            "expected starter crew & footer visible in 1440x900 (surface={surface:?}, starter={starter:?}, start_chat={start_chat:?})"
        );
    assert!(
        inner_max > px(100.),
        "expected inner agent list scroll overflow for 9 rows, got {inner_max:?}"
    );

    let pi_row = visual.debug_bounds("SETUP_AGENT_ROW_pi").unwrap();
    let pi_install = visual.debug_bounds("SETUP_INSTALL_pi").unwrap();
    assert!(
            pi_row.right() <= scrollbar_bounds.origin.x + px(0.5)
                && pi_install.right() <= scrollbar_bounds.origin.x + px(0.5),
            "expected pi row and Install button left of scrollbar gutter (row={pi_row:?}, install={pi_install:?}, bar={scrollbar_bounds:?})"
        );

    let trae_before = visual.debug_bounds("SETUP_AGENT_ROW_trae").unwrap();
    assert!(
            trae_before.origin.y >= scroll_bounds.bottom(),
            "expected 9th row below 216px viewport before scrolling (trae={trae_before:?}, viewport={scroll_bounds:?})"
        );

    handle
        .update(&mut visual, |root, _, cx| {
            root.setup
                .agents_scroll
                .set_offset(gpui::point(px(0.), -inner_max));
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();

    let trae_after = visual.debug_bounds("SETUP_AGENT_ROW_trae").unwrap();
    assert!(
            trae_after.origin.y >= scroll_bounds.origin.y
                && trae_after.bottom() <= scroll_bounds.bottom() + px(1.),
            "expected 9th row scrolled cleanly into viewport (trae={trae_after:?}, viewport={scroll_bounds:?})"
        );
    let copilot_install = visual.debug_bounds("SETUP_INSTALL_copilot").unwrap();
    assert!(
            copilot_install.right() <= scrollbar_bounds.origin.x + px(0.5),
            "expected overflow row button clear of scrollbar gutter (install={copilot_install:?}, bar={scrollbar_bounds:?})"
        );

    click_selector(&mut visual, "SETUP_MORE_AGENTS");
    visual.run_until_parked();
    assert!(!handle
        .read_with(&visual, |root, _| root.setup.more_agents)
        .unwrap());
    assert_eq!(
        handle
            .read_with(&visual, |root, _| root.setup.agents_scroll.offset().y)
            .unwrap(),
        px(0.)
    );
    assert!(visual.debug_bounds("SETUP_AGENTS_SCROLL").is_none());
    assert!(visual.debug_bounds("SETUP_AGENT_ROW_trae").is_none());

    handle
        .update(&mut visual, |root, _, cx| {
            let mut five_catalog = test_catalog();
            five_catalog.truncate(5);
            let five_runtimes: Vec<_> = five_catalog
                .iter()
                .map(|entry| {
                    runtime_exec(
                        entry.name,
                        &entry.display_name,
                        RuntimeRowState::NotFound,
                        RuntimeSignInStatus::Unknown,
                        None,
                    )
                })
                .collect();
            root.setup.status = Some(runtime_status(true, five_runtimes));
            root.setup.catalog = five_catalog;
            root.setup.more_agents = true;
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    let five_max = handle
        .read_with(&visual, |root, _| root.setup.agents_scroll.max_offset().y)
        .unwrap();
    assert!(
        five_max <= px(16.),
        "expected 5th peeking row to spill <= 16px below 216px viewport, got {five_max:?}"
    );
    assert!(visual.debug_bounds("SETUP_AGENTS_SCROLL_META").is_none());
}

#[gpui::test]
fn setup_starter_crew_long_names_never_overlap_status_across_widths_and_states(
    cx: &mut TestAppContext,
) {
    let temp = tempfile::TempDir::new().unwrap();
    let catalog = test_catalog();
    let runtimes = vec![
        runtime_exec(
            Runtime::Antigravity,
            "Antigravity CLI",
            RuntimeRowState::Detected,
            RuntimeSignInStatus::SignedIn,
            Some("1.4.2"),
        ),
        runtime_exec(
            Runtime::Copilot,
            "GitHub Copilot CLI",
            RuntimeRowState::Detected,
            RuntimeSignInStatus::SignedIn,
            Some("0.1.9"),
        ),
    ];

    let (_store, handle, mut visual) =
        open_test_root_with_size(&temp, cx, size(px(1440.), px(900.)), |root, _| {
            root.setup.status = Some(runtime_status(true, runtimes));
            root.setup.catalog = catalog;
            root.setup.proposal = Some(StarterCrewProposal {
                name: "Pair coding".into(),
                slots: vec![
                    StarterCrewSlotProposal {
                        handle: "coder".into(),
                        display_name: "Coder".into(),
                        runtime: Runtime::Antigravity,
                    },
                    StarterCrewSlotProposal {
                        handle: "reviewer".into(),
                        display_name: "Reviewer".into(),
                        runtime: Runtime::Copilot,
                    },
                ],
            });
        });

    let assert_layout_contained = |visual: &mut VisualTestContext, label: &str| {
        let surface = visual.debug_bounds("SETUP_SURFACE").unwrap();
        let card = visual.debug_bounds("SETUP_STARTER_CREW").unwrap();
        let slots = visual
            .debug_bounds("SETUP_STARTER_SLOTS")
            .expect("expected slots row");
        let slot_0 = visual.debug_bounds("SETUP_STARTER_SLOT_0").unwrap();
        let slot_1 = visual.debug_bounds("SETUP_STARTER_SLOT_1").unwrap();
        let status = visual
            .debug_bounds("SETUP_STARTER_STATUS")
            .expect("expected right status column");
        let toggle = visual
            .debug_bounds("SETUP_STARTER_TOGGLE")
            .expect("expected toggle action");
        let skip = visual
            .debug_bounds("SETUP_SKIP")
            .expect("expected skip button");
        let start_chat = visual
            .debug_bounds("SETUP_START_CHAT")
            .expect("expected primary start chat button");

        assert!(
                slots.right() + px(12.) <= status.origin.x,
                "{label}: slots row must stay strictly left of status column (slots={slots:?}, status={status:?})"
            );
        assert!(
                slot_0.right() + px(12.) <= status.origin.x
                    && slot_1.right() + px(12.) <= status.origin.x
                    && slot_0.right() < toggle.origin.x
                    && slot_1.right() < toggle.origin.x,
                "{label}: slot pills must stay clear of status & toggle (slot_0={slot_0:?}, slot_1={slot_1:?}, status={status:?}, toggle={toggle:?})"
            );
        assert!(
                slot_0.origin.y >= card.origin.y && slot_1.bottom() <= card.bottom(),
                "{label}: slot pills must remain inside starter crew card (card={card:?}, slot_0={slot_0:?}, slot_1={slot_1:?})"
            );
        assert!(
                skip.origin.x >= surface.origin.x
                    && skip.right() <= surface.right()
                    && start_chat.origin.x >= surface.origin.x
                    && start_chat.right() <= surface.right(),
                "{label}: footer buttons must stay horizontally inside surface (surface={surface:?}, skip={skip:?}, start_chat={start_chat:?})"
            );
        let separated_horizontally = skip.right() + px(8.) <= start_chat.origin.x
            || start_chat.right() + px(8.) <= skip.origin.x;
        let separated_vertically = skip.bottom() + px(4.) <= start_chat.origin.y
            || start_chat.bottom() + px(4.) <= skip.origin.y;
        assert!(
            separated_horizontally || separated_vertically,
            "{label}: footer buttons must not overlap (skip={skip:?}, start_chat={start_chat:?})"
        );
        (slot_0, slot_1)
    };

    assert_layout_contained(&mut visual, "1440x900 will create");
    click_selector(&mut visual, "SETUP_STARTER_TOGGLE");
    visual.run_until_parked();
    assert_layout_contained(&mut visual, "1440x900 wont create");
    click_selector(&mut visual, "SETUP_STARTER_TOGGLE");
    visual.run_until_parked();

    handle
        .update(&mut visual, |_, window, cx| {
            window.set_rem_size(px(20.));
            window.refresh();
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    assert_layout_contained(&mut visual, "1440x900 zoomed 125% will create");
    click_selector(&mut visual, "SETUP_STARTER_TOGGLE");
    visual.run_until_parked();
    assert_layout_contained(&mut visual, "1440x900 zoomed 125% wont create");

    visual.simulate_resize(size(px(900.), px(800.)));
    handle
        .update(&mut visual, |_, window, cx| {
            window.set_rem_size(px(16. * 1.3));
            window.refresh();
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    assert_layout_contained(&mut visual, "900x800 zoomed 130% wont create");
    click_selector(&mut visual, "SETUP_STARTER_TOGGLE");
    visual.run_until_parked();
    assert_layout_contained(&mut visual, "900x800 zoomed 130% will create");

    let max_scroll = handle
        .read_with(&visual, |root, _| root.setup.scroll.max_offset().y)
        .unwrap();
    if max_scroll > px(0.) {
        handle
            .update(&mut visual, |root, _, cx| {
                root.setup
                    .scroll
                    .set_offset(gpui::point(px(0.), -max_scroll));
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
    }
    let surface = visual.debug_bounds("SETUP_SURFACE").unwrap();
    let skip = visual.debug_bounds("SETUP_SKIP").unwrap();
    let start_chat = visual.debug_bounds("SETUP_START_CHAT").unwrap();
    assert!(
            skip.bottom() <= surface.bottom() && start_chat.bottom() <= surface.bottom(),
            "900x800 zoomed 130%: expected footer buttons vertically reachable in surface (surface={surface:?}, skip={skip:?}, start_chat={start_chat:?})"
        );

    visual.simulate_resize(size(px(720.), px(800.)));
    handle
        .update(&mut visual, |root, window, cx| {
            root.setup.scroll.set_offset(gpui::point(px(0.), px(0.)));
            window.set_rem_size(px(16.));
            window.refresh();
            cx.notify();
        })
        .unwrap();
    visual.run_until_parked();
    let (narrow_will_0, narrow_will_1) =
        assert_layout_contained(&mut visual, "720x800 narrow will create");
    assert!(
            narrow_will_1.origin.y >= narrow_will_0.bottom(),
            "expected wrapped slots in Will be created at narrow width (slot_0={narrow_will_0:?}, slot_1={narrow_will_1:?})"
        );
    click_selector(&mut visual, "SETUP_STARTER_TOGGLE");
    visual.run_until_parked();
    let (narrow_0, narrow_1) = assert_layout_contained(&mut visual, "720x800 narrow wont create");
    assert!(
            narrow_1.origin.y >= narrow_0.bottom(),
            "expected second long slot pill to wrap cleanly at narrow width (slot_0={narrow_0:?}, slot_1={narrow_1:?})"
        );
}

#[test]
fn setup_state_handles_visibility_generation_guards_and_finish_resets() {
    let mut state = SetupState::default();
    assert!(state.is_visible(false, false));
    assert!(!state.is_visible(false, true));
    assert!(!state.is_visible(true, false));

    state.open_reset();
    assert!(state.is_visible(false, true));
    assert!(!state.is_visible(true, true));

    state.expand_more_agents();
    assert!(state.more_agents);
    state.collapse_more_agents();
    assert!(!state.more_agents);

    assert_eq!(state.create_starter_crew, None);
    state.toggle_starter_crew();
    assert_eq!(state.create_starter_crew, Some(false));
    state.toggle_starter_crew();
    assert_eq!(state.create_starter_crew, Some(true));

    state.generation = 2;
    state.loading = true;
    assert!(!state.apply_refresh_result(
        1,
        Some(runtime_status(true, Vec::new())),
        test_catalog(),
        Some(two_slot_proposal()),
    ));
    assert!(state.loading);
    assert!(state.status.is_none());

    assert!(state.apply_refresh_result(
        2,
        Some(runtime_status(true, Vec::new())),
        test_catalog(),
        Some(two_slot_proposal()),
    ));
    assert!(!state.loading);
    assert!(state.status.is_some());
    assert!(!state.catalog.is_empty());

    state.generation = 3;
    state.rechecking = true;
    assert!(!state.apply_recheck_result(
        2,
        Some(runtime_status(false, Vec::new())),
        Vec::new(),
        None
    ));
    assert!(!state.rechecking);
    assert!(state.status.as_ref().unwrap().npm_available);

    state.rechecking = true;
    assert!(state.apply_recheck_result(
        3,
        Some(runtime_status(false, Vec::new())),
        Vec::new(),
        None
    ));
    assert!(!state.rechecking);
    assert!(!state.status.as_ref().unwrap().npm_available);

    state.finishing = true;
    state.fail_finish();
    assert!(!state.finishing);
    assert!(state.open);

    state.finishing = true;
    state.more_agents = true;
    state.complete_finish();
    assert!(!state.finishing);
    assert!(!state.open);
    assert!(!state.more_agents);
    assert_eq!(state.create_starter_crew, None);
    assert_eq!(state.telemetry_desired, None);
}

#[gpui::test]
fn setup_telemetry_checkbox_defaults_checked_and_syncs_opt_out_to_privacy_pane(
    cx: &mut TestAppContext,
) {
    let temp = tempfile::tempdir().unwrap();
    let (store, handle, mut visual) = open_test_root(&temp, cx, |root, _| {
        root.setup.status = Some(runtime_status(true, Vec::new()));
        root.setup.proposal = Some(two_slot_proposal());
    });
    wait_until(&mut visual, |visual| {
        let _ = handle.update(visual, |_, window, _| window.refresh());
        store.read_with(visual, |store, _| store.telemetry.is_some())
            && visual.debug_bounds("SETUP_TELEMETRY_CHECKBOX").is_some()
    });

    assert!(visual.debug_bounds("SETUP_TELEMETRY_STRIP").is_some());
    assert!(visual.debug_bounds("SETUP_TELEMETRY_CHECKBOX").is_some());
    assert!(visual.debug_bounds("SETUP_TELEMETRY_LINK").is_some());
    let checked = handle
        .read_with(&visual, |root, cx| {
            root.setup
                .telemetry_checked(root.app_store.read(cx).telemetry.as_ref())
        })
        .unwrap();
    assert!(checked);

    // Unchecking updates daemon telemetry state and clears Install ID.
    click_selector(&mut visual, "SETUP_TELEMETRY_CHECKBOX");
    wait_until(&mut visual, |visual| {
        store.read_with(visual, |store, _| {
            store
                .telemetry
                .as_ref()
                .is_some_and(|status| !status.enabled && status.install_id.is_none())
        })
    });

    // Finishing setup keeps telemetry disabled and synced in Settings → Privacy.
    click_selector(&mut visual, "SETUP_SKIP");
    wait_until(&mut visual, |visual| {
        store.read_with(visual, |store, _| store.onboarding_completed())
    });

    let _ = handle.update(&mut visual, |root, window, cx| {
        root.enter_settings_pane(
            crate::surfaces::settings::page::SettingsPane::Privacy,
            window,
            cx,
        );
    });
    visual.run_until_parked();
    assert!(visual.debug_bounds("PRIVACY_INSTALL_ID_CLEARED").is_some());
    assert!(store.read_with(&visual, |store, _| {
        store
            .telemetry
            .as_ref()
            .is_some_and(|status| !status.enabled && status.install_id.is_none())
    }));
}

#[gpui::test]
fn setup_telemetry_opt_out_persists_before_onboarding_complete_on_immediate_finish(
    cx: &mut TestAppContext,
) {
    let temp = tempfile::tempdir().unwrap();
    let (store, handle, mut visual) = open_test_root(&temp, cx, |root, _| {
        root.setup.status = Some(runtime_status(true, Vec::new()));
        root.setup.proposal = Some(two_slot_proposal());
        root.setup.set_telemetry_desired(false);
    });

    let _ = handle.update(&mut visual, |root, window, cx| {
        root.finish_setup(false, window, cx);
    });
    wait_until(&mut visual, |visual| {
        store.read_with(visual, |store, _| store.onboarding_completed())
    });

    let status = store
        .read_with(&visual, |store, _| store.client.telemetry_status())
        .unwrap();
    assert!(!status.enabled);
    assert!(status.install_id.is_none());
}

#[gpui::test]
fn setup_telemetry_checkbox_locks_when_env_override_is_active(cx: &mut TestAppContext) {
    let _ = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut vars = std::collections::BTreeMap::new();
    vars.insert("DO_NOT_TRACK".to_owned(), "1".to_owned());
    let telemetry = std::sync::Arc::new(runner_daemon::telemetry::TelemetryService::with_config(
        runner_daemon::telemetry::TelemetryConfig {
            dev_build: true,
            env: runner_daemon::telemetry::TelemetryEnvSource::Fixed(vars),
            network_enabled: false,
            ..runner_daemon::telemetry::TelemetryConfig::default()
        },
    ));
    let store =
        crate::app_store::test_lifecycle_store_with_telemetry(cx, temp.path(), Some(telemetry));
    let handle = cx.add_window(|window, cx| {
        window.resize(size(px(1200.), px(960.)));
        let mut root = NativeRoot::new(
            "main".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.setup.loaded_once = true;
        root.setup.catalog = test_catalog();
        root.setup.status = Some(runtime_status(true, Vec::new()));
        root.setup.proposal = Some(two_slot_proposal());
        root
    });
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(1200.), px(960.)));
    let _ = handle.update(&mut visual, |_, window, _| window.refresh());
    wait_until(&mut visual, |visual| {
        let _ = handle.update(visual, |_, window, _| window.refresh());
        store.read_with(visual, |store, _| store.telemetry.is_some())
            && visual.debug_bounds("SETUP_TELEMETRY_ENV_NOTICE").is_some()
    });

    click_selector(&mut visual, "SETUP_TELEMETRY_CHECKBOX");
    let checked = handle
        .read_with(&visual, |root, cx| {
            root.setup
                .telemetry_checked(root.app_store.read(cx).telemetry.as_ref())
        })
        .unwrap();
    assert!(!checked);
    let status = store
        .read_with(&visual, |store, _| store.client.telemetry_status())
        .unwrap();
    assert!(!status.enabled);
    assert_eq!(status.env_override.as_deref(), Some("DO_NOT_TRACK=1"));
}

#[gpui::test]
fn setup_overlapping_telemetry_clicks_and_skip_persists_final_opt_out_with_zero_pings(
    cx: &mut TestAppContext,
) {
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    let _ = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();

    // Local listener counting any accidental heartbeat delivery.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let stop_server = Arc::new(AtomicBool::new(false));
    let hits_bg = Arc::clone(&hits);
    let stop_bg = Arc::clone(&stop_server);
    let server_handle = std::thread::spawn(move || {
        while let Ok((_stream, _)) = listener.accept() {
            if stop_bg.load(Ordering::Acquire) {
                break;
            }
            hits_bg.fetch_add(1, Ordering::Relaxed);
        }
    });

    let telemetry = Arc::new(runner_daemon::telemetry::TelemetryService::with_config(
        runner_daemon::telemetry::TelemetryConfig {
            dev_build: false,
            endpoint: format!("http://{addr}/api/heartbeat"),
            version: "0.20.4".into(),
            build_stamp: "20261126.0220".into(),
            channel: "production".into(),
            os: "macos".into(),
            arch: "aarch64".into(),
            timeout: Duration::from_secs(2),
            interval: runner_daemon::telemetry::HEARTBEAT_INTERVAL,
            env: runner_daemon::telemetry::TelemetryEnvSource::Fixed(Default::default()),
            network_enabled: true,
        },
    ));
    let store = crate::app_store::test_lifecycle_store_with_telemetry(
        cx,
        temp.path(),
        Some(Arc::clone(&telemetry)),
    );
    let core = store.read_with(cx, |store, _| store.test_core.clone());
    core.runtime_discovery.write().unwrap().checking = false;
    telemetry.start_scheduler(core.clone());

    let handle = cx.add_window(|window, cx| {
        window.resize(size(px(1200.), px(960.)));
        let mut root = NativeRoot::new(
            "main".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.setup.loaded_once = true;
        root.setup.catalog = test_catalog();
        root.setup.status = Some(runtime_status(true, Vec::new()));
        root.setup.proposal = Some(two_slot_proposal());
        root
    });
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(1200.), px(960.)));
    wait_until(&mut visual, |visual| {
        let _ = handle.update(visual, |_, window, _| window.refresh());
        store.read_with(visual, |store, _| store.telemetry.is_some())
            && visual.debug_bounds("SETUP_TELEMETRY_CHECKBOX").is_some()
    });

    // First opt out cleanly so the next click will attempt to re-enable.
    click_selector(&mut visual, "SETUP_TELEMETRY_CHECKBOX");
    wait_until(&mut visual, |visual| {
        store.read_with(visual, |store, _| {
            store
                .telemetry
                .as_ref()
                .is_some_and(|status| !status.enabled)
        }) && handle
            .read_with(visual, |root, _| {
                root.setup.telemetry_write_in_flight.is_none()
            })
            .unwrap_or(false)
    });

    // Overlapping sequence without draining between clicks:
    // 1. Click ON spawns an enable write (`latest_seq` = N).
    // 2. While that enable write is still queued/in-flight, click OFF coalesces the
    //    final opt-out (`latest_seq` = N + 1) and immediately click Skip, which marks
    //    the write state finished before spawning.
    // Even when both background tasks execute in any order, the stale enable is dropped
    // and the final opt-out commits before onboarding_complete() wakes the worker.
    let checkbox = visual
        .debug_bounds("SETUP_TELEMETRY_CHECKBOX")
        .expect("SETUP_TELEMETRY_CHECKBOX")
        .center();
    let skip = visual
        .debug_bounds("SETUP_SKIP")
        .expect("SETUP_SKIP")
        .center();
    visual.simulate_click(checkbox, Modifiers::default());
    visual.simulate_click(checkbox, Modifiers::default());
    visual.simulate_click(skip, Modifiers::default());

    wait_until(&mut visual, |visual| {
        store.read_with(visual, |store, _| store.onboarding_completed())
    });
    visual.run_until_parked();

    let status = core.telemetry.status(&core).unwrap();
    assert!(
        !status.enabled,
        "final opt-out must win over earlier enable"
    );
    assert!(status.install_id.is_none());
    assert_eq!(
        core.telemetry.try_send_once(&core, chrono::Utc::now()),
        runner_daemon::telemetry::SendOutcome::SkippedDisabled
    );

    telemetry.stop();
    stop_server.store(true, Ordering::Release);
    let _ = TcpStream::connect(addr);
    server_handle.join().unwrap();
    assert_eq!(
        hits.load(Ordering::Relaxed),
        0,
        "heartbeat worker must send zero requests after opt-out in Setup"
    );
}

#[gpui::test]
fn setup_three_queued_telemetry_clicks_without_finish_persists_final_opt_out(
    cx: &mut TestAppContext,
) {
    let temp = tempfile::tempdir().unwrap();
    let (store, handle, mut visual) = open_test_root(&temp, cx, |root, _| {
        root.setup.status = Some(runtime_status(true, Vec::new()));
        root.setup.proposal = Some(two_slot_proposal());
    });
    wait_until(&mut visual, |visual| {
        let _ = handle.update(visual, |_, window, _| window.refresh());
        store.read_with(visual, |store, _| store.telemetry.is_some())
            && visual.debug_bounds("SETUP_TELEMETRY_CHECKBOX").is_some()
    });

    // Starting from ON, queue OFF -> ON -> OFF without draining or clicking Finish.
    let checkbox = visual
        .debug_bounds("SETUP_TELEMETRY_CHECKBOX")
        .expect("SETUP_TELEMETRY_CHECKBOX")
        .center();
    visual.simulate_click(checkbox, Modifiers::default());
    visual.simulate_click(checkbox, Modifiers::default());
    visual.simulate_click(checkbox, Modifiers::default());

    wait_until(&mut visual, |visual| {
        handle
            .read_with(visual, |root, _| {
                root.setup.telemetry_write_in_flight.is_none()
                    && root.setup.telemetry_desired.is_none()
            })
            .unwrap_or(false)
    });

    let status = store
        .read_with(&visual, |store, _| store.client.telemetry_status())
        .unwrap();
    assert!(
        !status.enabled,
        "three queued clicks OFF->ON->OFF must persist final OFF even without Finish"
    );
    assert!(status.install_id.is_none());
}

#[gpui::test]
fn setup_acknowledged_toggle_then_external_opt_out_and_skip_preserves_external_opt_out(
    cx: &mut TestAppContext,
) {
    let temp = tempfile::tempdir().unwrap();
    let (store, handle, mut visual) = open_test_root(&temp, cx, |root, _| {
        root.setup.status = Some(runtime_status(true, Vec::new()));
        root.setup.proposal = Some(two_slot_proposal());
    });
    wait_until(&mut visual, |visual| {
        let _ = handle.update(visual, |_, window, _| window.refresh());
        store.read_with(visual, |store, _| store.telemetry.is_some())
            && visual.debug_bounds("SETUP_TELEMETRY_CHECKBOX").is_some()
    });

    // Toggle OFF then ON in Setup and wait for both to be fully acknowledged.
    click_selector(&mut visual, "SETUP_TELEMETRY_CHECKBOX");
    wait_until(&mut visual, |visual| {
        handle
            .read_with(visual, |root, _| {
                root.setup.telemetry_write_in_flight.is_none()
                    && root.setup.telemetry_desired.is_none()
            })
            .unwrap_or(false)
            && store.read_with(visual, |store, _| {
                store.telemetry.as_ref().is_some_and(|s| !s.enabled)
            })
    });
    click_selector(&mut visual, "SETUP_TELEMETRY_CHECKBOX");
    wait_until(&mut visual, |visual| {
        handle
            .read_with(visual, |root, _| {
                root.setup.telemetry_write_in_flight.is_none()
                    && root.setup.telemetry_desired.is_none()
            })
            .unwrap_or(false)
            && store.read_with(visual, |store, _| {
                store.telemetry.as_ref().is_some_and(|s| s.enabled)
            })
    });

    // Another window/client turns telemetry OFF externally.
    let core = store.read_with(&visual, |store, _| store.test_core.clone());
    core.telemetry.set_enabled(&core, false).unwrap();
    wait_until(&mut visual, |visual| {
        store.read_with(visual, |store, _| {
            store.telemetry.as_ref().is_some_and(|s| !s.enabled)
        })
    });

    // Clicking Skip in Setup must NOT restore the earlier acknowledged ON choice.
    click_selector(&mut visual, "SETUP_SKIP");
    wait_until(&mut visual, |visual| {
        store.read_with(visual, |store, _| store.onboarding_completed())
    });

    let status = core.telemetry.status(&core).unwrap();
    assert!(
        !status.enabled,
        "Finish must not overwrite an external opt-out after an earlier click was retired"
    );
    assert!(status.install_id.is_none());
}

#[gpui::test]
fn setup_telemetry_checkbox_ignores_clicks_until_initial_status_loads(cx: &mut TestAppContext) {
    let _ = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut vars = std::collections::BTreeMap::new();
    vars.insert("DO_NOT_TRACK".to_owned(), "1".to_owned());
    let telemetry = std::sync::Arc::new(runner_daemon::telemetry::TelemetryService::with_config(
        runner_daemon::telemetry::TelemetryConfig {
            dev_build: true,
            env: runner_daemon::telemetry::TelemetryEnvSource::Fixed(vars),
            network_enabled: false,
            ..runner_daemon::telemetry::TelemetryConfig::default()
        },
    ));
    let store =
        crate::app_store::test_lifecycle_store_with_telemetry(cx, temp.path(), Some(telemetry));
    assert!(store.read_with(cx, |store, _| store.telemetry.is_none()));
    let handle = cx.add_window(|window, cx| {
        window.resize(size(px(1200.), px(960.)));
        let mut root = NativeRoot::new(
            "main".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.setup.loaded_once = true;
        root.setup.catalog = test_catalog();
        root.setup.status = Some(runtime_status(true, Vec::new()));
        root.setup.proposal = Some(two_slot_proposal());
        // Attempt to toggle BEFORE initial async telemetry_status has arrived.
        root.setup.toggle_telemetry(&store, cx);
        root
    });
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    wait_until(&mut visual, |visual| {
        store.read_with(visual, |store, _| store.telemetry.is_some())
    });

    let status = store
        .read_with(&visual, |store, _| store.client.telemetry_status())
        .unwrap();
    assert!(
        status.stored_enabled,
        "pre-load click under DO_NOT_TRACK must not mutate stored preference"
    );
    assert!(!status.enabled);
    assert_eq!(status.env_override.as_deref(), Some("DO_NOT_TRACK=1"));
}

#[gpui::test]
fn setup_closing_window_during_pending_opt_out_aborts_finish_without_completing_onboarding(
    cx: &mut TestAppContext,
) {
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    let _ = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let stop_server = Arc::new(AtomicBool::new(false));
    let hits_bg = Arc::clone(&hits);
    let stop_bg = Arc::clone(&stop_server);
    let server_handle = std::thread::spawn(move || {
        while let Ok((_stream, _)) = listener.accept() {
            if stop_bg.load(Ordering::Acquire) {
                break;
            }
            hits_bg.fetch_add(1, Ordering::Relaxed);
        }
    });

    let telemetry = Arc::new(runner_daemon::telemetry::TelemetryService::with_config(
        runner_daemon::telemetry::TelemetryConfig {
            dev_build: false,
            endpoint: format!("http://{addr}/api/heartbeat"),
            version: "0.20.4".into(),
            build_stamp: "20261126.0220".into(),
            channel: "production".into(),
            os: "macos".into(),
            arch: "aarch64".into(),
            timeout: Duration::from_secs(2),
            interval: runner_daemon::telemetry::HEARTBEAT_INTERVAL,
            env: runner_daemon::telemetry::TelemetryEnvSource::Fixed(Default::default()),
            network_enabled: true,
        },
    ));
    let store = crate::app_store::test_lifecycle_store_with_telemetry(
        cx,
        temp.path(),
        Some(Arc::clone(&telemetry)),
    );
    let core = store.read_with(cx, |store, _| store.test_core.clone());
    core.runtime_discovery.write().unwrap().checking = false;
    telemetry.start_scheduler(core.clone());

    let handle = cx.add_window(|window, cx| {
        window.resize(size(px(1200.), px(960.)));
        let mut root = NativeRoot::new(
            "main".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.setup.loaded_once = true;
        root.setup.catalog = test_catalog();
        root.setup.status = Some(runtime_status(true, Vec::new()));
        root.setup.proposal = Some(two_slot_proposal());
        root
    });
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(handle.into(), cx);
    visual.simulate_resize(size(px(1200.), px(960.)));
    wait_until(&mut visual, |visual| {
        let _ = handle.update(visual, |_, window, _| window.refresh());
        store.read_with(visual, |store, _| store.telemetry.is_some())
    });

    // Hold an opt-out write in flight (SQLite still defaults to enabled=true),
    // start Skip (which registers the waiter on SetupState), then immediately close
    // the window before that write can acknowledge.
    let _ = handle.update(&mut visual, |root, window, cx| {
        root.setup.set_telemetry_desired(false);
        root.setup.telemetry_write_in_flight = root.setup.telemetry_desired;
        root.finish_setup(false, window, cx);
        window.remove_window();
    });
    visual.run_until_parked();
    drop(visual);
    cx.run_until_parked();

    let onboarding = store
        .read_with(cx, |store, _| store.client.onboarding_status())
        .unwrap();
    assert!(
        !onboarding.completed,
        "closing Setup while an opt-out write is pending must abort Finish before onboarding_complete()"
    );
    assert_eq!(
        core.telemetry.try_send_once(&core, chrono::Utc::now()),
        runner_daemon::telemetry::SendOutcome::SkippedOnboardingIncomplete
    );

    telemetry.stop();
    stop_server.store(true, Ordering::Release);
    let _ = TcpStream::connect(addr);
    server_handle.join().unwrap();
    assert_eq!(
        hits.load(Ordering::Relaxed),
        0,
        "heartbeat worker must not send while opt-out write was unacknowledged"
    );
}
