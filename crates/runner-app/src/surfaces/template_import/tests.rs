use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use gpui::{px, size, ExternalPaths, FileDropEvent, TestAppContext, VisualTestContext};
use runner_core::protocol::model::Runtime;
use runner_core::template::parse_template;
use runner_daemon::{db, session, shell_path};

use super::{ImportStep, ReaderMode};
use crate::surfaces::AppRoute;
use crate::theme_snapshot::ThemeGuard;
use crate::*;

struct TemplateTestHarness {
    visual: VisualTestContext,
    host: gpui::WindowHandle<NativeRoot>,
    core: AppCore,
    _cx: TestAppContext,
    temp: tempfile::TempDir,
    _theme: ThemeGuard,
}

fn make_stub_bin(dir: &std::path::Path, name: &str) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\necho '1.0.0'\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
    #[cfg(windows)]
    {
        let path = dir.join(format!("{name}.cmd"));
        std::fs::write(&path, "@echo off\r\necho 1.0.0\r\n").unwrap();
        path
    }
}

fn harness(label: &str, installed_runtimes: &[Runtime]) -> TemplateTestHarness {
    let theme = ThemeGuard::new();
    theme::set_active_variant(theme::ThemeVariant::Carbon);
    let temp = tempfile::tempdir().unwrap();
    let bin_dir = temp.path().join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let pool = Arc::new(db::open_fixture_pool(&temp.path().join("runner.db")).unwrap());
    // Pin every built-in agent runtime via explicit overrides so test visibility
    // never depends on what CLIs happen to be installed on the host machine.
    for entry in runner_daemon::runtimes::catalogs() {
        if installed_runtimes.contains(&entry.name) {
            let bin = make_stub_bin(&bin_dir, entry.command);
            db::set_runtime_override(&pool, entry.name.key(), Some(&bin.display().to_string()))
                .unwrap();
        } else {
            let missing = temp
                .path()
                .join(format!("missing-runtime-{}", entry.name.key()));
            db::set_runtime_override(
                &pool,
                entry.name.key(),
                Some(&missing.display().to_string()),
            )
            .unwrap();
        }
    }
    let mut env = shell_path::LoginShellEnv {
        path: Some(bin_dir.display().to_string()),
        ..Default::default()
    };
    env.vars
        .insert("PATH".into(), bin_dir.display().to_string());
    let runtime_shell_env = Arc::new(RwLock::new(env));
    let runtime_discovery = Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
    let core = crate::test_support::core(
        pool,
        temp.path().to_owned(),
        session::SessionManager::new(
            runtime_shell_env.clone(),
            runtime_discovery.clone(),
            Arc::new(session::pty_runtime::PtyRuntime::new()),
        ),
        runtime_shell_env,
        runtime_discovery,
    );
    let mut cx = TestAppContext::single();
    let store = cx.new(|cx| {
        AppStore::new(
            core.clone(),
            None,
            None,
            temp.path().join("settings.json"),
            AppSettings::default(),
            None,
            cx,
        )
    });
    cx.update(|cx| {
        cx.set_global(crate::GlobalAppStore(store.clone()));
        cx.set_global(crate::WindowLayoutCheckpoint::default());
        #[cfg(target_os = "macos")]
        let updater = cx.new(|cx| crate::Updater::new(false, cx));
        #[cfg(any(windows, target_os = "linux"))]
        let updater = cx.new(|cx| crate::Updater::new(false, temp.path().join("updates"), cx));
        cx.set_global(crate::GlobalUpdater(updater));
    });
    let host = cx.add_window(|window, cx| {
        NativeRoot::new(
            label.to_owned(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        )
    });
    let visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(1440.), px(900.)));
    visual.run_until_parked();
    TemplateTestHarness {
        visual,
        host,
        core,
        _cx: cx,
        temp,
        _theme: theme,
    }
}

fn wait_until_diagnosed(
    visual: &mut VisualTestContext,
    mut ready: impl FnMut(&mut VisualTestContext) -> Result<(), String>,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        visual.run_until_parked();
        match ready(visual) {
            Ok(()) => return,
            Err(detail) => {
                assert!(Instant::now() < deadline, "timed out waiting for: {detail}");
                std::thread::yield_now();
            }
        }
    }
}

fn wait_until(
    visual: &mut VisualTestContext,
    what: &str,
    mut ready: impl FnMut(&mut VisualTestContext) -> bool,
) {
    wait_until_diagnosed(visual, |visual| {
        if ready(visual) {
            Ok(())
        } else {
            Err(what.to_owned())
        }
    });
}

impl TemplateTestHarness {
    fn wait(&mut self, what: &str, mut predicate: impl FnMut(&NativeRoot) -> bool) {
        let host = self.host;
        wait_until(&mut self.visual, what, |visual| {
            host.update(visual, |root, _, _| predicate(root)).unwrap()
        });
    }

    fn wait_visual(&mut self, what: &str, predicate: impl FnMut(&mut VisualTestContext) -> bool) {
        wait_until(&mut self.visual, what, predicate);
    }

    fn wait_fs(&mut self, what: &str, mut predicate: impl FnMut() -> bool) {
        wait_until(&mut self.visual, what, |_| predicate());
    }

    fn update<T>(
        &mut self,
        f: impl FnOnce(&mut NativeRoot, &mut gpui::Window, &mut gpui::Context<NativeRoot>) -> T,
    ) -> T {
        let out = self.host.update(&mut self.visual, f).unwrap();
        self.visual.run_until_parked();
        out
    }

    fn read<T>(&mut self, f: impl FnOnce(&NativeRoot) -> T) -> T {
        self.host
            .update(&mut self.visual, |root, _, _| f(root))
            .unwrap()
    }

    fn click(&mut self, selector: &'static str) {
        let bounds = self
            .visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not on screen"));
        self.visual
            .simulate_click(bounds.center(), gpui::Modifiers::default());
        self.visual.run_until_parked();
    }

    fn drop_file(&mut self, selector: &'static str, path: PathBuf) {
        let target = self
            .visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not on screen"));
        self.visual.simulate_event(FileDropEvent::Entered {
            position: target.center(),
            paths: ExternalPaths([path].into_iter().collect()),
        });
        self.visual.simulate_event(FileDropEvent::Submit {
            position: target.center(),
        });
        self.visual.run_until_parked();
    }

    fn write_file(&self, name: &str, content: &str) -> PathBuf {
        let path = self.temp.path().join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    fn create_role(&self, handle: &str, runtime: Runtime) -> runner_core::protocol::model::Role {
        runner_daemon::ops::role::role_create(
            &self.core,
            runner_daemon::ops::role::CreateRoleInput {
                handle: handle.into(),
                display_name: format!("Role {handle}"),
                runtime,
                command: runtime.to_string(),
                args: vec!["--secret-flag".into()],
                working_dir: Some("/private/path".into()),
                system_prompt: Some(format!("You are {handle}.")),
                env: [("SECRET_KEY".into(), "top-secret".into())]
                    .into_iter()
                    .collect(),
                model: Some("gpt-5".into()),
                effort: Some("high".into()),
                codex_speed: None,
                permission_mode: runner_daemon::router::runtime::PermissionMode::Default,
            },
        )
        .unwrap()
    }
}

const TWO_CODEX_CREW: &str = r#"version = 1
kind = "crew"

[meta]
author = "jason"
description = "Two codex roles."
tags = ["coding", "review"]

[crew]
name = "Codex Pair"
conventions = """
Hand off via Runner messages.
"""

[[roles]]
handle = "coder"
display_name = "Coder"
runtime = "codex"
model = "gpt-5"
effort = "high"
system_prompt = """
Implement the requested change.
"""

[[roles]]
handle = "reviewer"
display_name = "Reviewer"
runtime = "codex"
model = "gpt-5"
effort = "xhigh"
system_prompt = """
Review the working tree diff.
"""

[[slots]]
handle = "coder"
role = "coder"

[[slots]]
handle = "reviewer"
role = "reviewer"
"#;

#[test]
fn template_entry_points_show_locally_and_hide_on_remote_host() {
    let mut h = harness("template-entry-local", &[Runtime::ClaudeCode]);
    h.update(|root, window, cx| root.open_crews(window, cx));
    h.wait_visual("crews empty loaded", |v| {
        v.debug_bounds("EMPTY_IMPORT_CREW").is_some()
    });
    assert!(h.visual.debug_bounds("CREWS_IMPORT").is_some());
    assert!(h.visual.debug_bounds("CREWS_BROWSE_SAMPLES").is_some());
    assert!(h.visual.debug_bounds("EMPTY_IMPORT_CREW").is_some());
    assert!(h.visual.debug_bounds("EMPTY_BROWSE_SAMPLES").is_some());

    h.update(|root, window, cx| root.open_roles(window, cx));
    h.wait_visual("roles loaded", |v| v.debug_bounds("ROLES_IMPORT").is_some());
    assert!(h.visual.debug_bounds("ROLES_IMPORT").is_some());

    let role = h.create_role("alpha", Runtime::ClaudeCode);
    let crew = runner_daemon::ops::crew::crew_create(
        &h.core,
        runner_daemon::ops::crew::CreateCrewInput {
            name: "Alpha Crew".into(),
            system_prompt_addendum: None,
        },
    )
    .unwrap();
    h.update(|root, window, cx| root.open_role_detail(role.handle.clone(), window, cx));
    h.wait_visual("role detail loaded", |v| {
        v.debug_bounds("ROLE_DETAIL_MORE").is_some()
    });
    assert!(h.visual.debug_bounds("ROLE_DETAIL_MORE").is_some());

    h.update(|root, window, cx| root.open_crew_editor(crew.id.clone(), window, cx));
    h.wait_visual("crew detail loaded", |v| {
        v.debug_bounds("CREW_DETAIL_MORE").is_some()
    });
    assert!(h.visual.debug_bounds("CREW_DETAIL_MORE").is_some());

    // Remote host hides all template entry points.
    use crate::surfaces::remote_client::tests as remote;
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = remote::fixture(&mut cx, temp.path());
    remote::enable_front_page(&fake);
    let window = remote::add_window(&mut cx, &store, temp.path(), "template-entry-remote");
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.simulate_resize(size(px(1440.), px(900.)));
    window.update(&mut visual, remote::choose_host).unwrap();
    visual.run_until_parked();
    fake.queue_result("catalog", Ok(remote::library_catalog(1)));
    window
        .update(&mut visual, |root, window, cx| root.open_crews(window, cx))
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("CREWS_IMPORT").is_none());
    assert!(visual.debug_bounds("CREWS_BROWSE_SAMPLES").is_none());
    assert!(visual.debug_bounds("EMPTY_IMPORT_CREW").is_none());

    fake.queue_result("catalog", Ok(remote::library_catalog(1)));
    window
        .update(&mut visual, |root, window, cx| root.open_roles(window, cx))
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("ROLES_IMPORT").is_none());
}

#[test]
fn step1_error_recovery_and_choose_file_prompt_work() {
    let mut h = harness("template-error-retry", &[Runtime::ClaudeCode]);
    h.update(|root, window, cx| root.open_crews(window, cx));
    h.wait_visual("crews loaded", |v| v.debug_bounds("CREWS_IMPORT").is_some());
    h.click("CREWS_IMPORT");
    assert!(h.visual.debug_bounds("TEMPLATE_IMPORT_DROP_ZONE").is_some());

    h.click("TEMPLATE_IMPORT_CHOOSE_FILE");
    assert!(h.visual.did_prompt_for_paths());

    let bad = h.write_file(
        "bad.crew.toml",
        "version = 1\nkind = \"crew\"\n\n[crew]\nname = \"Bad\"\n\n[[roles]]\nhandle = \"coder\"\ndisplay_name = \"Coder\"\nruntime = \"codex\"\nworking_dir = \"/tmp\"\n\n[[slots]]\nhandle = \"coder\"\nrole = \"coder\"\n",
    );
    // Drop the invalid file directly onto the Step 1 drop zone via ExternalPaths.
    h.drop_file("TEMPLATE_IMPORT_DROP_ZONE", bad);
    h.wait("error state", |root| {
        matches!(
            root.template_import_modal.as_ref().map(|m| &m.step),
            Some(ImportStep::Error(_))
        )
    });
    assert!(h
        .visual
        .debug_bounds("TEMPLATE_IMPORT_ERROR_STATE")
        .is_some());
    assert!(h
        .visual
        .debug_bounds("TEMPLATE_IMPORT_ERROR_DROP_ZONE")
        .is_some());
    h.read(|root| {
        let Some(ImportStep::Error(err)) = root.template_import_modal.as_ref().map(|m| &m.step)
        else {
            panic!("expected error step");
        };
        assert_eq!(err.filename, "bad.crew.toml");
        assert!(err.line.is_some());
    });
    assert!(runner_daemon::ops::crew::crew_list(&h.core, 1, 20, "")
        .unwrap()
        .items
        .is_empty());

    // Recover by dropping a valid template onto the error drop zone.
    let good = h.write_file("good.crew.toml", TWO_CODEX_CREW);
    h.drop_file("TEMPLATE_IMPORT_ERROR_DROP_ZONE", good);
    h.wait("preview state after recovery", |root| {
        matches!(
            root.template_import_modal.as_ref().map(|m| &m.step),
            Some(ImportStep::Preview(_))
        )
    });
}

#[test]
fn crew_preview_bulk_switch_resets_model_effort_and_imports_fresh_roles() {
    let mut h = harness("template-bulk-import", &[Runtime::ClaudeCode]);
    let existing = h.create_role("coder", Runtime::Codex);
    let path = h.write_file("codex-pair.crew.toml", TWO_CODEX_CREW);

    h.update(|root, window, cx| {
        root.open_template_import_modal(window, cx);
        root.load_template_import_file(path, window, cx);
    });
    h.wait("crew preview loaded", |root| {
        matches!(
            root.template_import_modal.as_ref().map(|m| &m.step),
            Some(ImportStep::Preview(_))
        )
    });
    assert!(h.visual.debug_bounds("TEMPLATE_IMPORT_META_TAGS").is_some());
    assert!(h
        .visual
        .debug_bounds("TEMPLATE_IMPORT_BULK_BANNER")
        .is_some());
    assert!(h
        .visual
        .debug_bounds("TEMPLATE_IMPORT_NEW_ROLE_BADGE:0")
        .is_some());
    assert!(h
        .visual
        .debug_bounds("TEMPLATE_IMPORT_SLOT_RUNTIME:0")
        .is_some());
    assert!(h
        .visual
        .debug_bounds("TEMPLATE_IMPORT_SLOT_RUNTIME:1")
        .is_some());

    // Select a replacement from the real bulk StyledSelect menu (exercises on_change without re-entrancy panic).
    h.click("STYLED_SELECT_template-bulk-rt-codex");
    assert!(h.visual.debug_bounds("STYLED_SELECT_MENU").is_some());
    h.click("STYLED_SELECT_OPTION_0");
    assert!(h
        .visual
        .debug_bounds("TEMPLATE_IMPORT_BULK_BANNER")
        .is_none());
    assert!(h
        .visual
        .debug_bounds("TEMPLATE_IMPORT_SLOT_RESET:0")
        .is_some());
    assert!(h
        .visual
        .debug_bounds("TEMPLATE_IMPORT_SLOT_RESET:1")
        .is_some());

    h.click("TEMPLATE_IMPORT_CONFIRM");
    h.wait("imported crew page opened", |root| {
        root.template_import_modal.is_none() && matches!(root.route, AppRoute::CrewEditor(_))
    });

    let AppRoute::CrewEditor(crew_id) = h.read(|root| root.route.clone()) else {
        panic!("expected CrewEditor route");
    };
    let crew = runner_daemon::ops::crew::crew_get(&h.core, &crew_id).unwrap();
    assert_eq!(crew.name, "Codex Pair");
    let slots = runner_daemon::ops::slot::slot_list(&h.core, &crew_id).unwrap();
    assert_eq!(slots.len(), 2);
    // Existing @coder was never reused or modified.
    assert_ne!(slots[0].role.id, existing.id);
    assert_eq!(slots[0].role.handle, "coder-2");
    assert_eq!(slots[1].role.handle, "reviewer");
    for slot in &slots {
        assert_eq!(slot.role.runtime, "claude-code");
        assert_eq!(slot.role.model, None);
        assert_eq!(slot.role.effort, None);
        assert_eq!(slot.slot.model_override, None);
        assert_eq!(slot.slot.effort_override, None);
    }
    let untouched = runner_daemon::ops::role::role_get(&h.core, &existing.id).unwrap();
    assert_eq!(untouched.runtime, "codex");
    assert_eq!(untouched.model.as_deref(), Some("gpt-5"));
}

#[test]
fn prompt_reader_defaults_to_source_shows_hidden_chars_and_esc_returns() {
    let mut h = harness("template-reader", &[Runtime::ClaudeCode, Runtime::Codex]);
    let toml = "version = 1\nkind = \"crew\"\n\n[crew]\nname = \"Reader Crew\"\nconventions = \"\"\"\nUse messages.\n\"\"\"\n\n[[roles]]\nhandle = \"coder\"\ndisplay_name = \"Coder\"\nruntime = \"claude-code\"\nsystem_prompt = \"\"\"\n# Heading\nSafe\u{200B}text\u{202E}bidi\n![alt](https://example.com/pixel.png)\n\"\"\"\n\n[[slots]]\nhandle = \"coder\"\nrole = \"coder\"\n";
    let path = h.write_file("reader.crew.toml", toml);
    h.update(|root, window, cx| {
        root.open_template_import_modal(window, cx);
        root.load_template_import_file(path, window, cx);
    });
    h.wait("preview ready", |root| {
        matches!(
            root.template_import_modal.as_ref().map(|m| &m.step),
            Some(ImportStep::Preview(_))
        )
    });

    h.click("TEMPLATE_IMPORT_SLOT:0");
    assert!(h
        .visual
        .debug_bounds("TEMPLATE_IMPORT_READER_SOURCE_BODY")
        .is_some());
    assert!(h
        .visual
        .debug_bounds("TEMPLATE_READER_HIDDEN_CHAR")
        .is_some());

    h.click("TEMPLATE_IMPORT_READER_RENDERED");
    assert!(h
        .visual
        .debug_bounds("TEMPLATE_IMPORT_READER_RENDERED_BODY")
        .is_some());
    h.read(|root| {
        assert!(matches!(
            root.template_import_modal.as_ref().map(|m| &m.step),
            Some(ImportStep::Reader {
                mode: ReaderMode::Rendered,
                index: 0,
                ..
            })
        ));
    });

    h.click("TEMPLATE_IMPORT_READER_NEXT");
    h.read(|root| {
        assert!(matches!(
            root.template_import_modal.as_ref().map(|m| &m.step),
            Some(ImportStep::Reader { index: 1, .. })
        ));
    });

    h.click("TEMPLATE_IMPORT_READER_PREV");
    h.read(|root| {
        assert!(matches!(
            root.template_import_modal.as_ref().map(|m| &m.step),
            Some(ImportStep::Reader { index: 0, .. })
        ));
    });

    // First Esc returns to Step 2 Preview; second Esc closes the modal.
    h.visual.simulate_keystrokes("escape");
    h.visual.run_until_parked();
    h.read(|root| {
        assert!(matches!(
            root.template_import_modal.as_ref().map(|m| &m.step),
            Some(ImportStep::Preview(_))
        ));
    });
    h.visual.simulate_keystrokes("escape");
    h.visual.run_until_parked();
    assert!(h.read(|root| root.template_import_modal.is_none()));
}

#[test]
fn export_and_standalone_role_import_round_trip_and_both_themes_fit() {
    let mut h = harness("template-export-role", &[Runtime::ClaudeCode]);
    let role = h.create_role("architect", Runtime::ClaudeCode);
    let crew = runner_daemon::ops::crew::crew_create(
        &h.core,
        runner_daemon::ops::crew::CreateCrewInput {
            name: "Exportable".into(),
            system_prompt_addendum: Some("Stay sharp.".into()),
        },
    )
    .unwrap();
    runner_daemon::ops::slot::slot_create(
        &h.core,
        runner_daemon::ops::slot::CreateSlotInput {
            crew_id: crew.id.clone(),
            role_id: role.id.clone(),
            slot_handle: "architect".into(),
            runtime_override: None,
            model_override: None,
        },
    )
    .unwrap();

    // Export crew to disk via native save dialog test seam.
    let crew_out = h.temp.path().join("exportable.crew.toml");
    let crew_out_select = crew_out.clone();
    h.update(|root, window, cx| root.export_crew_template(crew.id.clone(), window, cx));
    h.wait_visual("crew save prompt queued", |v| v.did_prompt_for_new_path());
    h.visual
        .simulate_new_path_selection(move |_| Some(crew_out_select.clone()));
    h.wait_fs("crew export file written", || crew_out.exists());
    let exported_crew = std::fs::read_to_string(&crew_out).unwrap();
    assert!(!exported_crew.contains("SECRET_KEY"));
    assert!(!exported_crew.contains("/private/path"));
    assert!(!exported_crew.contains("--secret-flag"));
    assert!(parse_template(&exported_crew).is_ok());

    // Export role to disk, then import it back through the modal.
    let role_out = h.temp.path().join("architect.role.toml");
    let role_out_select = role_out.clone();
    h.update(|root, window, cx| root.export_role_template(role.id.clone(), window, cx));
    h.wait_visual("role save prompt queued", |v| v.did_prompt_for_new_path());
    h.visual
        .simulate_new_path_selection(move |_| Some(role_out_select.clone()));
    h.wait_fs("role export file written", || role_out.exists());

    h.update(|root, window, cx| {
        root.open_template_import_modal(window, cx);
        root.load_template_import_file(role_out.clone(), window, cx);
    });
    h.wait("standalone role preview ready", |root| {
        matches!(
            root.template_import_modal.as_ref().map(|m| &m.step),
            Some(ImportStep::Preview(_))
        )
    });

    // Verify both Dark and Runner Light themes render cleanly without overflowing bounds.
    for variant in [
        theme::ThemeVariant::Carbon,
        theme::ThemeVariant::RunnerLight,
    ] {
        theme::set_active_variant(variant);
        h.update(|_, _, cx| cx.notify());
        let modal = h.visual.debug_bounds("TEMPLATE_IMPORT_MODAL").unwrap();
        assert!(modal.right() <= px(1440.));
        assert!(modal.bottom() <= px(900.));
    }
    theme::set_active_variant(theme::ThemeVariant::Carbon);

    h.click("TEMPLATE_IMPORT_CONFIRM");
    h.wait("imported role detail opened", |root| {
        root.template_import_modal.is_none()
            && matches!(&root.route, AppRoute::RoleDetail(handle) if handle == "architect-2")
    });
    let imported = runner_daemon::ops::role::role_get_by_handle(&h.core, "architect-2").unwrap();
    assert_eq!(imported.display_name, "Role architect");
    assert!(imported.env.is_empty());
    assert_eq!(imported.working_dir, None);
}

#[test]
fn real_slot_and_standalone_role_runtime_dropdowns_switch_without_reentrancy_panic() {
    let mut h = harness("template-real-selects", &[Runtime::ClaudeCode]);

    // 1. Per-slot StyledSelect on crew preview.
    let crew_path = h.write_file("codex-pair.crew.toml", TWO_CODEX_CREW);
    h.update(|root, window, cx| {
        root.open_template_import_modal(window, cx);
        root.load_template_import_file(crew_path, window, cx);
    });
    h.wait("crew preview ready", |root| {
        matches!(
            root.template_import_modal.as_ref().map(|m| &m.step),
            Some(ImportStep::Preview(_))
        )
    });
    h.click("STYLED_SELECT_template-slot-rt-coder");
    assert!(h.visual.debug_bounds("STYLED_SELECT_MENU").is_some());
    h.click("STYLED_SELECT_OPTION_0");
    h.read(|root| {
        let Some(ImportStep::Preview(state)) = root.template_import_modal.as_ref().map(|m| &m.step)
        else {
            panic!("expected preview");
        };
        assert_eq!(
            state.runtime_overrides.get("coder"),
            Some(&Runtime::ClaudeCode)
        );
        // Only 1 missing codex slot remains, so the bulk banner disappears.
        assert_eq!(state.runtime_overrides.get("reviewer"), None);
    });
    assert!(h
        .visual
        .debug_bounds("TEMPLATE_IMPORT_BULK_BANNER")
        .is_none());

    // 2. Standalone role StyledSelect on role preview.
    let role_toml = r#"version = 1
kind = "role"

[meta]
tags = ["review"]

[role]
handle = "review-codex"
display_name = "Reviewer"
runtime = "codex"
model = "gpt-5"
effort = "xhigh"
system_prompt = "Review diffs."
"#;
    let role_path = h.write_file("review-codex.role.toml", role_toml);
    h.update(|root, window, cx| {
        root.return_to_template_upload(window, cx);
        root.load_template_import_file(role_path, window, cx);
    });
    h.wait("role preview ready", |root| {
        matches!(
            root.template_import_modal.as_ref().map(|m| &m.step),
            Some(ImportStep::Preview(state)) if matches!(state.plan.template, runner_core::template::Template::Role(_))
        )
    });
    assert!(h.visual.debug_bounds("TEMPLATE_IMPORT_META_TAGS").is_some());
    h.click("STYLED_SELECT_template-standalone-role-rt");
    assert!(h.visual.debug_bounds("STYLED_SELECT_MENU").is_some());
    h.click("STYLED_SELECT_OPTION_0");
    h.read(|root| {
        let Some(ImportStep::Preview(state)) = root.template_import_modal.as_ref().map(|m| &m.step)
        else {
            panic!("expected role preview");
        };
        assert_eq!(
            state.runtime_overrides.get("review-codex"),
            Some(&Runtime::ClaudeCode)
        );
    });
    h.click("TEMPLATE_IMPORT_CONFIRM");
    h.wait("role imported", |root| {
        root.template_import_modal.is_none()
            && matches!(&root.route, AppRoute::RoleDetail(handle) if handle == "review-codex")
    });
    let created = runner_daemon::ops::role::role_get_by_handle(&h.core, "review-codex").unwrap();
    assert_eq!(created.runtime, "claude-code");
    assert_eq!(created.model, None);
    assert_eq!(created.effort, None);
}

#[test]
fn drag_enter_leave_and_submit_toggle_active_drop_state() {
    let mut h = harness("template-drag-state", &[Runtime::ClaudeCode]);
    let file = h.write_file("codex-pair.crew.toml", TWO_CODEX_CREW);
    h.update(|root, window, cx| root.open_template_import_modal(window, cx));
    let drop_bounds = h
        .visual
        .debug_bounds("TEMPLATE_IMPORT_DROP_ZONE")
        .expect("drop zone on screen");
    assert!(h
        .visual
        .debug_bounds("TEMPLATE_IMPORT_CHOOSE_FILE")
        .is_some());
    assert!(h
        .visual
        .debug_bounds("TEMPLATE_IMPORT_DROP_ACTIVE")
        .is_none());

    // Enter drop zone -> active state ("Release to preview <filename>" + "Ready to drop").
    h.visual.simulate_event(FileDropEvent::Entered {
        position: drop_bounds.center(),
        paths: ExternalPaths([file.clone()].into_iter().collect()),
    });
    h.visual.run_until_parked();
    assert!(h
        .visual
        .debug_bounds("TEMPLATE_IMPORT_DROP_ACTIVE")
        .is_some());
    assert!(h
        .visual
        .debug_bounds("TEMPLATE_IMPORT_CHOOSE_FILE")
        .is_none());
    h.read(|root| {
        assert_eq!(
            root.template_import_modal
                .as_ref()
                .and_then(|m| m.drag_hover_filename.as_deref()),
            Some("codex-pair.crew.toml")
        );
    });

    // Move outside drop zone -> returns to idle state.
    h.visual.simulate_event(FileDropEvent::Pending {
        position: gpui::point(px(4.), px(4.)),
    });
    h.visual.run_until_parked();
    assert!(h
        .visual
        .debug_bounds("TEMPLATE_IMPORT_DROP_ACTIVE")
        .is_none());
    assert!(h
        .visual
        .debug_bounds("TEMPLATE_IMPORT_CHOOSE_FILE")
        .is_some());

    // Re-enter then exit window -> returns to idle state.
    h.visual.simulate_event(FileDropEvent::Pending {
        position: drop_bounds.center(),
    });
    h.visual.run_until_parked();
    assert!(h
        .visual
        .debug_bounds("TEMPLATE_IMPORT_DROP_ACTIVE")
        .is_some());
    h.visual.simulate_event(FileDropEvent::Exited);
    h.visual.run_until_parked();
    assert!(h
        .visual
        .debug_bounds("TEMPLATE_IMPORT_DROP_ACTIVE")
        .is_none());

    // Enter and submit -> transitions cleanly into Step 2 Preview.
    h.drop_file("TEMPLATE_IMPORT_DROP_ZONE", file);
    h.wait("preview opened after drop", |root| {
        matches!(
            root.template_import_modal.as_ref().map(|m| &m.step),
            Some(ImportStep::Preview(_))
        )
    });
}

#[test]
fn oversized_and_non_regular_files_fail_immediately_without_hanging() {
    let mut h = harness("template-file-guards", &[Runtime::ClaudeCode]);
    h.update(|root, window, cx| root.open_template_import_modal(window, cx));

    // Directory is rejected as non-regular immediately.
    let dir = h.temp.path().join("not-a-file.crew.toml");
    std::fs::create_dir_all(&dir).unwrap();
    h.update(|root, window, cx| root.load_template_import_file(dir, window, cx));
    h.wait("directory error", |root| {
        matches!(
            root.template_import_modal.as_ref().map(|m| &m.step),
            Some(ImportStep::Error(err)) if !root.template_import_modal.as_ref().unwrap().loading && err.message.contains("regular file")
        )
    });

    // Oversized file (> 256 KiB) is rejected without loading unbounded bytes.
    let huge = h.write_file(
        "huge.crew.toml",
        &" ".repeat(runner_core::template::MAX_TEMPLATE_BYTES + 1024),
    );
    h.update(|root, window, cx| root.load_template_import_file(huge, window, cx));
    h.wait("oversized error", |root| {
        matches!(
            root.template_import_modal.as_ref().map(|m| &m.step),
            Some(ImportStep::Error(err)) if !root.template_import_modal.as_ref().unwrap().loading && err.message.contains("256 KiB")
        )
    });

    // On Unix, a named pipe (FIFO) must fail immediately rather than blocking open().
    #[cfg(unix)]
    {
        let fifo = h.temp.path().join("pipe.crew.toml");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(status.success());
        h.update(|root, window, cx| root.load_template_import_file(fifo, window, cx));
        h.wait("fifo rejected", |root| {
            matches!(
                root.template_import_modal.as_ref().map(|m| &m.step),
                Some(ImportStep::Error(err)) if !root.template_import_modal.as_ref().unwrap().loading && err.message.contains("regular file")
            )
        });
    }
}

#[test]
fn write_export_file_persists_atomically_and_preserves_existing_file_on_error() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("crew.crew.toml");
    super::write_export_file(&target, "original").unwrap();
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");

    // Attempting to export onto an existing directory fails cleanly without clobbering.
    let as_dir = dir.path().join("dir-target");
    std::fs::create_dir_all(&as_dir).unwrap();
    assert!(super::write_export_file(&as_dir, "new content").is_err());

    // Attempting to export into a nonexistent directory fails cleanly and leaves target intact.
    let missing_parent = dir.path().join("missing-parent/out.crew.toml");
    assert!(super::write_export_file(&missing_parent, "new content").is_err());
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
}

#[test]
fn switching_to_remote_host_clears_open_local_preview_and_drops_late_completions() {
    use crate::surfaces::remote_client::tests as remote;
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = remote::fixture(&mut cx, temp.path());
    remote::enable_front_page(&fake);
    let window = remote::add_window(&mut cx, &store, temp.path(), "template-host-switch");
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.simulate_resize(size(px(1440.), px(900.)));
    visual.run_until_parked();

    let file = temp.path().join("pair.crew.toml");
    std::fs::write(&file, TWO_CODEX_CREW).unwrap();
    window
        .update(&mut visual, |root, window, cx| {
            root.open_template_import_modal(window, cx);
            root.load_template_import_file(file.clone(), window, cx);
        })
        .unwrap();
    wait_until(&mut visual, "local preview", |visual| {
        window
            .update(visual, |root, _, _| {
                matches!(
                    root.template_import_modal.as_ref().map(|m| &m.step),
                    Some(ImportStep::Preview(_))
                )
            })
            .unwrap()
    });
    assert!(visual.debug_bounds("TEMPLATE_IMPORT_MODAL").is_some());

    // Start a local submit and immediately switch to a remote host before it finishes.
    let local_core = window
        .update(&mut visual, |root, _, cx| root.core(cx).clone())
        .unwrap();
    window
        .update(&mut visual, |root, window, cx| {
            root.submit_template_import(window, cx);
            remote::choose_host(root, window, cx);
            assert!(
                root.template_import_modal.is_none(),
                "choosing a remote host in the current window clears the local import modal synchronously"
            );
        })
        .unwrap();
    // Wait deterministically for both the host switch and the in-flight submit
    // continuation to finish, then confirm the late completion neither reopens
    // the modal, shows a local toast, nor navigates on the remote host.
    wait_until_diagnosed(&mut visual, |visual| {
        let (remote_active, in_flight) = window
            .update(visual, |root, _, _| {
                (
                    root.remote_client.is_active(),
                    root.template_import_in_flight,
                )
            })
            .unwrap();
        let imported = local_core
            .crew_list(1, 20, "")
            .unwrap()
            .items
            .iter()
            .any(|item| item.crew.name == "Codex Pair");
        if remote_active && in_flight == 0 && imported {
            Ok(())
        } else {
            Err(format!(
                "host switch and local submit to finish (remote_active={remote_active}, in_flight={in_flight}, imported={imported})"
            ))
        }
    });
    visual.run_until_parked();
    window
        .update(&mut visual, |root, _, _| {
            assert!(root.remote_client.is_active());
            assert!(root.template_import_modal.is_none());
            assert!(!matches!(root.route, AppRoute::CrewEditor(_)));
            assert!(root.toasts.active().is_none());
        })
        .unwrap();
    assert!(visual.debug_bounds("TEMPLATE_IMPORT_MODAL").is_none());

    // Also verify an in-flight preview load is dropped when switching hosts before it completes.
    window
        .update(&mut visual, |root, window, cx| {
            root.return_to_this_computer(window, cx);
            root.open_template_import_modal(window, cx);
            root.load_template_import_file(file.clone(), window, cx);
            remote::choose_host(root, window, cx);
            assert!(root.template_import_modal.is_none());
        })
        .unwrap();
    wait_until_diagnosed(&mut visual, |visual| {
        let (remote_active, in_flight) = window
            .update(visual, |root, _, _| {
                (
                    root.remote_client.is_active(),
                    root.template_import_in_flight,
                )
            })
            .unwrap();
        if remote_active && in_flight == 0 {
            Ok(())
        } else {
            Err(format!(
                "host switch and preview load to settle (remote_active={remote_active}, in_flight={in_flight})"
            ))
        }
    });
    visual.run_until_parked();
    window
        .update(&mut visual, |root, _, _| {
            assert!(root.remote_client.is_active());
            assert!(root.template_import_modal.is_none());
        })
        .unwrap();
    assert!(visual.debug_bounds("TEMPLATE_IMPORT_MODAL").is_none());
}

#[test]
fn unpaired_host_cancel_preserves_edited_local_template_preview() {
    use crate::surfaces::remote_client::tests as remote;
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, clients) = remote::fixture(&mut cx, temp.path());
    remote::enable_front_page(&fake);
    let window = remote::add_window(&mut cx, &store, temp.path(), "template-unpaired-cancel");
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    visual.simulate_resize(size(px(1440.), px(900.)));
    visual.run_until_parked();

    let file = temp.path().join("pair.crew.toml");
    std::fs::write(&file, TWO_CODEX_CREW).unwrap();
    window
        .update(&mut visual, |root, window, cx| {
            root.open_template_import_modal(window, cx);
            root.load_template_import_file(file.clone(), window, cx);
        })
        .unwrap();
    wait_until(&mut visual, "local preview", |visual| {
        window
            .update(visual, |root, _, _| {
                matches!(
                    root.template_import_modal.as_ref().map(|m| &m.step),
                    Some(ImportStep::Preview(_))
                )
            })
            .unwrap()
    });

    // Edit the preview name and runtime override before triggering an unpaired host switch.
    window
        .update(&mut visual, |root, window, cx| {
            if let Some(ImportStep::Preview(state)) =
                root.template_import_modal.as_ref().map(|m| &m.step)
            {
                if let Some(field) = state.crew_name.as_ref() {
                    field.update(cx, |field, cx| field.set_text("Custom Pair Name", cx));
                }
            }
            root.set_template_slot_runtime("coder", "claude-code", cx);
            remote::choose_unpaired_host(&fake, &clients, root, window, cx);
            assert!(
                root.template_import_modal.is_some(),
                "opening the PIN dialog for an unpaired host must not discard the local preview"
            );
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("COMPUTER_PAIR_PRIMARY").is_some());

    // Cancel pairing with Escape; the window stays local and the edited preview remains intact.
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(visual.debug_bounds("COMPUTER_PAIR_PRIMARY").is_none());
    assert!(visual.debug_bounds("TEMPLATE_IMPORT_MODAL").is_some());
    window
        .update(&mut visual, |root, _, cx| {
            assert!(root.remote_client.is_local());
            let Some(ImportStep::Preview(state)) =
                root.template_import_modal.as_ref().map(|m| &m.step)
            else {
                panic!("expected local template preview to survive cancelled pairing");
            };
            let crew_name = state
                .crew_name
                .as_ref()
                .expect("crew_name field")
                .read(cx)
                .text()
                .to_owned();
            assert_eq!(crew_name, "Custom Pair Name");
            assert_eq!(
                state.runtime_overrides.get("coder"),
                Some(&Runtime::ClaudeCode)
            );
        })
        .unwrap();
}
