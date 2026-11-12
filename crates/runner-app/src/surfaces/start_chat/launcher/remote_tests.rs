use super::*;
use crate::surfaces::remote_client::tests::{
    action_trace, add_window, choose_host, enable_front_page, fixture, overview,
};
use gpui::{TestAppContext, VisualTestContext};

#[test]
fn direct_role_start_refreshes_recency_in_both_remote_launcher_windows() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let (store, fake, _) = fixture(&mut cx, temp.path());
    enable_front_page(&fake);
    let mut data = overview();
    data.tabs.clear();
    data.sessions.clear();
    fake.set_overview(data);
    let first = add_window(&mut cx, &store, temp.path(), "recent-first");
    let second = add_window(&mut cx, &store, temp.path(), "recent-second");
    for window in [first, second] {
        window.update(&mut cx, choose_host).unwrap();
    }
    cx.run_until_parked();
    for window in [first, second] {
        window
            .update(&mut cx, |root, window, cx| {
                root.render_launcher(None, true, window, cx);
            })
            .unwrap();
    }
    cx.run_until_parked();

    for window in [first, second] {
        window
            .update(&mut cx, |root, window, cx| {
                let launcher = root.render_launcher(None, true, window, cx);
                assert_eq!(
                    launcher.read(cx).rows(cx).0[0].choice,
                    Choice::Chat(EntryKey::Agent("codex".into()))
                );
            })
            .unwrap();
    }
    first
        .update(&mut cx, |root, window, cx| {
            root.open_remote_role_chat("host-role".into(), window, cx)
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(action_trace(&fake, "startChat").len(), 1);
    for window in [first, second] {
        window
            .update(&mut cx, |root, window, cx| {
                let launcher = root.render_launcher(None, true, window, cx);
                assert_eq!(
                    launcher.read(cx).rows(cx).0[0].choice,
                    Choice::Chat(EntryKey::Role("host-role".into()))
                );
            })
            .unwrap();
    }
}

#[test]
fn no_tab_remote_terminal_click_and_shortcut_send_a_nonempty_tab_id() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for shortcut in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = fixture(&mut cx, temp.path());
        enable_front_page(&fake);
        let mut data = overview();
        data.tabs.clear();
        data.sessions.clear();
        fake.set_overview(data);
        let window = add_window(&mut cx, &store, temp.path(), "no-tab-terminal");
        window.update(&mut cx, choose_host).unwrap();
        cx.run_until_parked();
        window
            .update(&mut cx, |root, window, cx| {
                let view = root.render_launcher(None, true, window, cx);
                if shortcut {
                    root.new_terminal_action(&NewTerminal, window, cx);
                } else {
                    view.update(cx, |launcher, cx| {
                        launcher.start(Choice::Terminal, false, window, cx)
                    });
                }
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(action_trace(&fake, "startShell").len(), 1);
        let layouts = action_trace(&fake, "layout");
        assert_eq!(layouts.len(), 1);
        assert!(!layouts[0]["target"]["id"].as_str().unwrap().is_empty());
        assert_eq!(layouts[0]["panes"]["session"], "started-session");
    }
}

#[test]
fn host_role_directory_metadata_and_explicit_change_keep_role_settings_without_overrides() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for (metadata, changed) in [(true, false), (true, true), (false, false)] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = fixture(&mut cx, temp.path());
        enable_front_page(&fake);
        let mut data = overview();
        data.tabs[0].panes = runner_core::companion::Pane::Leaf { session: None };
        fake.set_overview(data);
        let window = add_window(&mut cx, &store, temp.path(), "launcher-role-folder");
        window.update(&mut cx, choose_host).unwrap();
        cx.run_until_parked();
        if metadata && !changed {
            fake.queue_result("role", Ok(serde_json::json!({"id":"host-role","handle":"host-coder","name":"Host coder","runtime":"codex","command":"codex","args":[],"workingDir":"/role-directory","systemPrompt":null,"model":"host-model","effort":"low","speed":null,"permissionMode":null})));
        }
        let view = window
            .update(&mut cx, |root, window, cx| {
                let layout = root.front_page_layout().unwrap();
                let pane = PaneKey::new(&layout.id, &layout.focused_pane_id);
                let view = root.render_launcher(Some(pane), true, window, cx);
                let access = crate::surfaces::front_page::RemoteAccess::new(
                    root.front_page_client(cx).target,
                );
                let mut features = vec!["catalog".into(), "startChat".into(), "layout".into()];
                if metadata {
                    features.push("role".into());
                }
                access.set_features(features);
                let client = FrontPageClient::remote(root.core(cx).clone(), access);
                view.update(cx, |launcher, cx| {
                    launcher.client = client;
                    launcher
                        .cwd
                        .update(cx, |field, cx| field.reset("/shown", cx));
                    launcher.changed_directory = changed;
                    launcher.start(
                        Choice::Chat(EntryKey::Role("host-role".into())),
                        false,
                        window,
                        cx,
                    );
                });
                view
            })
            .unwrap();
        cx.run_until_parked();
        let calls = action_trace(&fake, "startChat");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["role"], "host-role");
        assert_eq!(
            calls[0]["cwd"],
            if metadata && !changed {
                "/role-directory"
            } else {
                "/shown"
            }
        );
        for field in ["runtime", "model", "effort", "speed"] {
            assert!(calls[0][field].is_null(), "{field}");
        }
        view.read_with(&cx, |launcher, _| assert!(launcher.pending.is_some()));
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        view.update(&mut visual, |launcher, cx| {
            assert!(launcher.pending.is_some());
            launcher.selected = 0;
            cx.notify();
        });
        visual.run_until_parked();
    }
}
