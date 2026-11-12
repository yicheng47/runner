use super::*;
use gpui::{px, size, EntityInputHandler, TestAppContext, VisualTestContext};
use runner_app::i18n::{testing::with_locale, Locale};
#[test]
fn every_status_renders_in_both_languages_and_themes() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            let window = cx.add_window(|window, cx| {
                let mut root = NativeRoot::new(
                    "remote-snapshots".into(),
                    temp.path().join("logs"),
                    None,
                    None,
                    store.clone(),
                    window,
                    cx,
                );
                root.route = crate::AppRoute::Settings;
                root.settings_page.pane = SettingsPane::Remote;
                root
            });
            cx.simulate_window_resize(window.into(), size(px(1200.), px(900.)));
            cx.run_until_parked();
            for variant in [
                theme::ThemeVariant::Carbon,
                theme::ThemeVariant::RunnerLight,
            ] {
                theme::set_active_variant(variant);
                let now = chrono::Utc::now();
                let mut states = vec![
                    RemoteState::Off,
                    RemoteState::Enrolling {
                        user_code: "BCDF-GHJK".into(),
                        verification_uri: "https://runner.example/hosts/add".into(),
                        verification_uri_complete:
                            "https://runner.example/hosts/add?code=BCDF-GHJK".into(),
                        expires_at: now + chrono::Duration::minutes(10),
                    },
                    RemoteState::Confirming {
                        account: Default::default(),
                        expires_at: now + chrono::Duration::minutes(5),
                    },
                    RemoteState::Connecting { attempt: 0 },
                    RemoteState::Connected { since: now },
                    RemoteState::Reconnecting {
                        attempt: 2,
                        next_retry_at: now + chrono::Duration::seconds(15),
                    },
                    RemoteState::Paused,
                ];
                states.extend(
                    [
                        StopReason::Removed,
                        StopReason::InUseElsewhere,
                        StopReason::UpdateRequired,
                        StopReason::Denied,
                        StopReason::Expired,
                    ]
                    .into_iter()
                    .map(|reason| RemoteState::Stopped { reason }),
                );
                for state in states {
                    let stop_reason = if let RemoteState::Stopped { reason } = state {
                        Some(reason)
                    } else {
                        None
                    };
                    let enrolling = matches!(state, RemoteState::Enrolling { .. });
                    let off = matches!(state, RemoteState::Off);
                    let signed_in = matches!(
                        state,
                        RemoteState::Connecting { .. }
                            | RemoteState::Connected { .. }
                            | RemoteState::Reconnecting { .. }
                            | RemoteState::Paused
                    );
                    let waiting = matches!(
                        state,
                        RemoteState::Enrolling { .. } | RemoteState::Confirming { .. }
                    );
                    store.update(&mut cx, |store, cx| {
                        store.remote_status = Some(RemoteStatus {
                            state,
                            account: Some(runner_core::remote::RemoteAccount {
                                username: "jason".into(),
                                display_name: "Jason".into(),
                            }),
                            host_id: Some("7383920468475904".into()),
                            host_name: Some("Mac Studio".into()),
                            fingerprint: Some("630d cd29 66c4 3366 9112".into()),
                            ..Default::default()
                        });
                        cx.notify();
                    });
                    window.update(&mut cx, |_, _, cx| cx.notify()).unwrap();
                    cx.run_until_parked();
                    let mut visual = VisualTestContext::from_window(window.into(), &cx);
                    assert_eq!(
                        visual.debug_bounds("REMOTE_STATUS_CARD").is_some(),
                        signed_in
                    );
                    assert_eq!(
                        visual.debug_bounds("REMOTE_RELAY_SIGNED_OUT").is_some(),
                        !signed_in && !waiting
                    );
                    for selector in ["REMOTE_INVITE_DISCORD", "REMOTE_INVITE_WECHAT_README"] {
                        assert_eq!(
                            visual.debug_bounds(selector).is_some(),
                            !signed_in && !waiting
                        );
                    }
                    assert!(visual.debug_bounds("REMOTE_TAILSCALE_DISABLED").is_none());
                    assert_eq!(visual.debug_bounds("REMOTE_DEVICES").is_some(), signed_in);
                    assert_eq!(
                        visual.debug_bounds("ACCOUNT_APPROVAL_REMINDER").is_some(),
                        enrolling
                    );
                    window
                        .update(&mut cx, |root, _, cx| {
                            root.settings_page.pane = SettingsPane::Account;
                            cx.notify();
                        })
                        .unwrap();
                    cx.run_until_parked();
                    let card = visual.debug_bounds("ACCOUNT_STATUS_CARD").unwrap();
                    assert!(card.bottom() < px(900.));
                    assert_eq!(
                        visual.debug_bounds("ACCOUNT_APPROVAL_REMINDER").is_some(),
                        enrolling
                    );
                    assert_eq!(visual.debug_bounds("ACCOUNT_SIGN_IN").is_some(), off);
                    assert_eq!(visual.debug_bounds("ACCOUNT_DETAILS").is_some(), signed_in);
                    if signed_in {
                        let sign_out = visual.debug_bounds("ACCOUNT_SIGN_OUT_SECTION").unwrap();
                        let pane = visual.debug_bounds("ACCOUNT_SETTINGS").unwrap();
                        assert_eq!(sign_out.top(), card.bottom() + px(20.));
                        assert_eq!(pane.bottom(), sign_out.bottom());
                    }
                    assert_eq!(
                        visual.debug_bounds("ACCOUNT_RETRY").is_some(),
                        matches!(
                            stop_reason,
                            Some(StopReason::Removed | StopReason::InUseElsewhere)
                        )
                    );
                    assert_eq!(
                        visual.debug_bounds("ACCOUNT_SIGN_IN_AGAIN").is_some(),
                        matches!(
                            stop_reason,
                            Some(StopReason::Removed | StopReason::Denied | StopReason::Expired)
                        )
                    );
                    assert_eq!(
                        visual.debug_bounds("ACCOUNT_STOP_SIGN_OUT").is_some(),
                        stop_reason == Some(StopReason::InUseElsewhere)
                    );
                    assert_eq!(
                        visual.debug_bounds("ACCOUNT_CHECK_UPDATES").is_some(),
                        stop_reason == Some(StopReason::UpdateRequired)
                    );
                    assert!(visual.debug_bounds("REMOTE_RELAY_SIGNED_OUT").is_none());
                    window
                        .update(&mut cx, |root, _, cx| {
                            root.settings_page.pane = SettingsPane::Remote;
                            cx.notify();
                        })
                        .unwrap();
                }
                cx.simulate_window_resize(window.into(), size(px(640.), px(700.)));
                cx.run_until_parked();
                let mut visual = VisualTestContext::from_window(window.into(), &cx);
                let card = visual.debug_bounds("REMOTE_RELAY_SIGNED_OUT").unwrap();
                for selector in [
                    "REMOTE_SIGN_IN",
                    "REMOTE_INVITE_DISCORD",
                    "REMOTE_INVITE_WECHAT_README",
                ] {
                    let link = visual.debug_bounds(selector).unwrap();
                    assert!(
                        link.left() >= card.left() && link.right() <= card.right(),
                        "{selector}"
                    );
                    assert!(
                        link.top() >= card.top() && link.bottom() <= card.bottom(),
                        "{selector}"
                    );
                }
                window
                    .update(&mut cx, |root, window, cx| {
                        root.enter_settings_pane(SettingsPane::About, window, cx);
                    })
                    .unwrap();
                cx.run_until_parked();
                let mut visual = VisualTestContext::from_window(window.into(), &cx);
                for selector in [
                    "about-discord",
                    "about-wechat-readme",
                    "ABOUT_INVITE_GUIDANCE",
                ] {
                    assert!(visual.debug_bounds(selector).is_some(), "{selector}");
                }
                window
                    .update(&mut cx, |root, _, cx| {
                        root.settings_page.pane = SettingsPane::Remote;
                        cx.notify();
                    })
                    .unwrap();
                cx.simulate_window_resize(window.into(), size(px(1200.), px(900.)));
            }
        });
    }
}
#[derive(Default)]
struct RemoteRequests {
    calls: std::sync::Mutex<Vec<&'static str>>,
    status: std::sync::Mutex<RemoteStatus>,
    trust: std::sync::Mutex<RemoteTrust>,
    fail_logout: std::sync::atomic::AtomicBool,
}
impl runner_core::protocol::Transport for RemoteRequests {
    fn call(
        &self,
        request: runner_core::protocol::Request,
    ) -> Result<runner_core::protocol::Response, runner_core::protocol::ClientError> {
        use runner_core::protocol::{Request, Response};
        let mut status = self.status.lock().unwrap();
        let mut calls = self.calls.lock().unwrap();
        Ok(match request {
            Request::remote_status {} => Response::remote_status(Ok(status.clone())),
            Request::remote_trust {} => {
                calls.push("trust");
                Response::remote_trust(Ok(self.trust.lock().unwrap().clone()))
            }
            Request::remote_enroll_start {
                origin,
                confirm_account,
            } => {
                assert!(origin.is_none());
                assert!(!confirm_account);
                calls.push("sign in");
                status.state = RemoteState::Enrolling {
                    user_code: "BCDF-GHJK".into(),
                    verification_uri: "https://runner.example/add".into(),
                    verification_uri_complete: "https://runner.example/add?code=BCDF-GHJK".into(),
                    expires_at: chrono::Utc::now() + chrono::Duration::minutes(10),
                };
                Response::remote_enroll_start(Ok(status.clone()))
            }
            Request::remote_enroll_cancel {} => {
                calls.push("cancel");
                *status = RemoteStatus::default();
                Response::remote_enroll_cancel(Ok(status.clone()))
            }
            Request::remote_retry {} => {
                calls.push("retry");
                status.state = RemoteState::Connecting { attempt: 0 };
                Response::remote_retry(Ok(status.clone()))
            }
            Request::remote_set_paused { paused } => {
                calls.push(if paused { "pause" } else { "resume" });
                status.state = if paused {
                    RemoteState::Paused
                } else {
                    RemoteState::Connecting { attempt: 0 }
                };
                Response::remote_set_paused(Ok(status.clone()))
            }
            Request::remote_set_pin { .. } => {
                calls.push("set pin");
                Response::remote_set_pin(Ok(RemoteTrust {
                    pin_set: true,
                    ..Default::default()
                }))
            }
            Request::remote_logout {} => {
                calls.push("sign out");
                if self.fail_logout.load(std::sync::atomic::Ordering::SeqCst) {
                    return Err(runner_core::protocol::ClientError::msg(
                        "test sign-out failed",
                    ));
                }
                *status = RemoteStatus::default();
                Response::remote_logout(Ok(runner_core::remote::RemoteLogout::Removed))
            }
            Request::remote_reset_identity {} => {
                calls.push("reset");
                *status = RemoteStatus::default();
                Response::remote_reset_identity(Ok(RemoteTrust::default()))
            }
            _ => {
                return Err(runner_core::protocol::ClientError::msg(
                    "unused fixture request",
                ))
            }
        })
    }
    fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
        unreachable!("the request fixture does not subscribe")
    }
}

fn switch_window(
    cx: &mut TestAppContext,
    store: &gpui::Entity<crate::app_store::AppStore>,
    path: &std::path::Path,
    name: &str,
) -> gpui::WindowHandle<NativeRoot> {
    let window = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            name.into(),
            path.join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.route = crate::AppRoute::Chat;
        root
    });
    cx.simulate_window_resize(window.into(), size(px(1440.), px(1100.)));
    window
}

#[test]
fn pin_prompt_requires_this_windows_live_sign_in_and_fresh_trust() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for case in [
        "new",
        "confirming",
        "connecting",
        "existing",
        "other-window",
        "left-settings",
        "left-during-trust",
        "startup",
        "paused",
        "reconnecting",
        "daemon-reconnect",
        "cancelled",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let requests = std::sync::Arc::new(RemoteRequests::default());
        requests.trust.lock().unwrap().pin_set = case == "existing";
        cx.run_until_parked();
        store.update(&mut cx, |store, _| {
            store.remote_trust = Some(RemoteTrust::default());
        });
        let window = switch_window(&mut cx, &store, temp.path(), "sign-in-pin");
        let peer = switch_window(&mut cx, &store, temp.path(), "peer-pin");
        store.update(&mut cx, |store, _| {
            store.client = DaemonClient::new(requests.clone())
        });
        window
            .update(&mut cx, |root, _, cx| {
                root.route = crate::AppRoute::Settings;
                root.settings_page.pane = SettingsPane::Account;
                if case != "startup" && case != "other-window" {
                    root.remote_connect(cx);
                }
            })
            .unwrap();
        cx.run_until_parked();
        peer.update(&mut cx, |root, _, _| root.route = crate::AppRoute::Settings)
            .unwrap();
        let previous = match case {
            "confirming" | "other-window" => RemoteState::Confirming {
                account: Default::default(),
                expires_at: chrono::Utc::now(),
            },
            "paused" => RemoteState::Paused,
            "reconnecting" => RemoteState::Reconnecting {
                attempt: 1,
                next_retry_at: chrono::Utc::now(),
            },
            _ => requests.status.lock().unwrap().state.clone(),
        };
        store.update(&mut cx, |store, _| {
            store.remote_status = (case != "startup").then_some(RemoteStatus {
                state: previous,
                ..Default::default()
            });
        });
        if case == "left-settings" || case == "cancelled" {
            window
                .update(&mut cx, |root, _, cx| {
                    if case == "cancelled" {
                        root.remote_cancel_start(cx);
                    } else {
                        root.route = crate::AppRoute::Chat;
                    }
                })
                .unwrap();
        }
        let events = store.read_with(&cx, |store, _| store.test_core.events.clone());
        if case == "daemon-reconnect" {
            events.emit("daemon/reconnected", &serde_json::Value::Null);
            cx.run_until_parked();
        }
        events.emit(
            "remote/status",
            &RemoteStatus {
                state: if case == "connecting" {
                    RemoteState::Connecting { attempt: 0 }
                } else {
                    RemoteState::Connected {
                        since: chrono::Utc::now(),
                    }
                },
                ..Default::default()
            },
        );
        if case == "left-during-trust" {
            window
                .update(&mut cx, |root, _, _| root.route = crate::AppRoute::Chat)
                .unwrap();
        }
        cx.run_until_parked();
        window
            .read_with(&cx, |root, _| {
                assert_eq!(
                    root.settings_page.remote.pin.is_some(),
                    matches!(case, "new" | "confirming" | "connecting"),
                    "{case}"
                );
            })
            .unwrap();
        peer.read_with(&cx, |root, _| {
            assert!(root.settings_page.remote.pin.is_none())
        })
        .unwrap();
        if case == "new" {
            window
                .update(&mut cx, |root, _, _| root.dismiss_remote_dialog())
                .unwrap();
            events.emit(
                "remote/status",
                &RemoteStatus {
                    state: RemoteState::Connected {
                        since: chrono::Utc::now(),
                    },
                    ..Default::default()
                },
            );
            cx.run_until_parked();
            window
                .read_with(&cx, |root, _| {
                    assert!(root.settings_page.remote.pin.is_none())
                })
                .unwrap();
        }
    }
}

#[test]
fn replaced_sign_in_prompts_only_the_winning_window() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    cx.run_until_parked();
    let first = switch_window(&mut cx, &store, temp.path(), "first-sign-in");
    let second = switch_window(&mut cx, &store, temp.path(), "second-sign-in");
    let requests = std::sync::Arc::new(RemoteRequests::default());
    store.update(&mut cx, |store, _| {
        store.client = DaemonClient::new(requests.clone())
    });
    first
        .update(&mut cx, |root, _, cx| {
            root.route = crate::AppRoute::Settings;
            root.remote_connect(cx);
        })
        .unwrap();
    for _ in 0..100 {
        if !requests.calls.lock().unwrap().is_empty() {
            break;
        }
        assert!(cx.dispatcher.tick(true));
    }
    assert_eq!(*requests.calls.lock().unwrap(), ["sign in"]);
    first
        .read_with(&cx, |root, _| assert!(root.settings_page.remote.busy))
        .unwrap();
    second
        .update(&mut cx, |root, _, cx| {
            root.route = crate::AppRoute::Settings;
            root.remote_connect(cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(*requests.calls.lock().unwrap(), ["sign in", "sign in"]);
    store.update(&mut cx, |store, _| {
        store.remote_status = Some(requests.status.lock().unwrap().clone())
    });
    store
        .read_with(&cx, |store, _| store.test_core.events.clone())
        .emit(
            "remote/status",
            &RemoteStatus {
                state: RemoteState::Connected {
                    since: chrono::Utc::now(),
                },
                ..Default::default()
            },
        );
    cx.run_until_parked();
    first
        .read_with(&cx, |root, _| {
            assert!(root.settings_page.remote.pin.is_none())
        })
        .unwrap();
    second
        .read_with(&cx, |root, _| {
            assert!(root.settings_page.remote.pin.is_some())
        })
        .unwrap();
    assert_eq!(
        *requests.calls.lock().unwrap(),
        ["sign in", "sign in", "trust"]
    );
}

#[test]
fn setup_row_replaces_pin_row_until_set_and_opens_existing_dialog() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            let window = switch_window(&mut cx, &store, temp.path(), "pin-setup");
            cx.run_until_parked();
            window
                .update(&mut cx, |root, _, cx| {
                    root.route = crate::AppRoute::Settings;
                    root.settings_page.pane = SettingsPane::Remote;
                    cx.notify();
                })
                .unwrap();
            for state in [
                RemoteState::Connected {
                    since: chrono::Utc::now(),
                },
                RemoteState::Paused,
                RemoteState::Reconnecting {
                    attempt: 1,
                    next_retry_at: chrono::Utc::now(),
                },
            ] {
                for (pin_set, pin_locked) in
                    [(false, false), (true, false), (false, true), (true, true)]
                {
                    store.update(&mut cx, |store, cx| {
                        store.remote_status = Some(RemoteStatus {
                            state: state.clone(),
                            ..Default::default()
                        });
                        store.remote_trust = Some(RemoteTrust {
                            pin_set,
                            pin_locked,
                            ..Default::default()
                        });
                        cx.notify();
                    });
                    window
                        .update(&mut cx, |root, _, cx| {
                            root.dismiss_remote_dialog();
                            cx.notify();
                        })
                        .unwrap();
                    cx.run_until_parked();
                    let mut visual = VisualTestContext::from_window(window.into(), &cx);
                    let setup = !pin_set && !pin_locked;
                    assert_eq!(visual.debug_bounds("REMOTE_PIN_SETUP").is_some(), setup);
                    assert_eq!(visual.debug_bounds("REMOTE_PIN_ROW").is_some(), !setup);
                    if setup {
                        let row = visual.debug_bounds("REMOTE_PIN_SETUP").unwrap();
                        visual.simulate_click(
                            gpui::point(row.right() - px(45.), row.center().y),
                            Default::default(),
                        );
                        window
                            .read_with(&cx, |root, _| {
                                assert!(root.settings_page.remote.pin.is_some())
                            })
                            .unwrap();
                    }
                }
            }
        });
    }
}

#[test]
fn cancel_start_ignores_late_daemon_results_and_unlocks_sign_in() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for fail in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        cx.run_until_parked();
        let window = switch_window(&mut cx, &store, temp.path(), "cancel-start");
        let requests = std::sync::Arc::new(RemoteRequests::default());
        requests
            .fail_logout
            .store(true, std::sync::atomic::Ordering::SeqCst);
        store.update(&mut cx, |store, _| {
            store.client = DaemonClient::new(requests.clone());
        });
        let completed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let pending = completed.clone();
        window
            .update(&mut cx, |root, _, cx| {
                root.route = crate::AppRoute::Settings;
                root.settings_page.pane = SettingsPane::Remote;
                root.remote_request(
                    true,
                    move |client| {
                        pending.store(true, std::sync::atomic::Ordering::SeqCst);
                        if fail {
                            Err(runner_core::protocol::ClientError::msg(
                                "late startup error",
                            ))
                        } else {
                            client.remote_enroll_start(None, false)
                        }
                    },
                    cx,
                );
                assert!(root.settings_page.remote.busy);
                assert!(root.settings_page.remote.opening);
            })
            .unwrap();
        for _ in 0..100 {
            if completed.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            assert!(cx.dispatcher.tick(true));
        }
        assert!(completed.load(std::sync::atomic::Ordering::SeqCst));
        window
            .update(&mut cx, |root, _, cx| {
                assert!(root.settings_page.remote.busy);
                let generation = root.settings_page.remote.request_generation;
                root.remote_cancel_start(cx);
                assert!(!root.settings_page.remote.busy);
                assert!(!root.settings_page.remote.opening);
                assert_eq!(root.settings_page.remote.request_generation, generation + 1);
            })
            .unwrap();
        cx.run_until_parked();
        assert!(completed.load(std::sync::atomic::Ordering::SeqCst));
        assert!(
            cx.opened_url().is_none(),
            "abandoned start must not open a browser"
        );
        window
            .read_with(&cx, |root, _| {
                assert!(!root.settings_page.remote.busy);
                assert!(root.settings_page.remote.error.is_none());
            })
            .unwrap();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        assert!(visual.debug_bounds("REMOTE_CANCEL_START").is_none());
        let sign_in = visual.debug_bounds("REMOTE_SIGN_IN").unwrap();
        visual.simulate_click(sign_in.center(), Default::default());
        visual.run_until_parked();
        assert!(
            cx.opened_url().is_some(),
            "Sign in must be usable after Cancel"
        );
    }
}

#[test]
fn immediate_sign_in_preserves_new_attempt_and_skips_undispatched_start() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for dispatch_first in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        cx.run_until_parked();
        let window = switch_window(&mut cx, &store, temp.path(), "cancel-restart");
        let requests = std::sync::Arc::new(RemoteRequests::default());
        store.update(&mut cx, |store, _| {
            store.client = DaemonClient::new(requests.clone())
        });
        window
            .update(&mut cx, |root, _, cx| {
                root.route = crate::AppRoute::Settings;
                root.settings_page.pane = SettingsPane::Remote;
                root.remote_connect(cx);
            })
            .unwrap();
        if dispatch_first {
            for _ in 0..100 {
                if !requests.calls.lock().unwrap().is_empty() {
                    break;
                }
                assert!(cx.dispatcher.tick(true));
            }
            assert_eq!(*requests.calls.lock().unwrap(), ["sign in"]);
        }
        window
            .update(&mut cx, |root, _, cx| {
                assert!(root.settings_page.remote.busy);
                root.remote_cancel_start(cx);
                assert!(!root.settings_page.remote.busy);
                assert!(!root.settings_page.remote.opening);
                root.remote_connect(cx);
                assert!(root.settings_page.remote.busy);
            })
            .unwrap();
        // Both cleanup and the second Start are queued before background work resumes.
        cx.run_until_parked();
        let calls = requests.calls.lock().unwrap().clone();
        if dispatch_first {
            assert!(
                calls == ["sign in", "sign out", "sign in"] || calls == ["sign in", "sign in"],
                "cleanup may precede the newer Start or skip it after replacement: {calls:?}"
            );
        } else {
            assert_eq!(calls, ["sign in"]);
        }
        assert!(matches!(
            requests.status.lock().unwrap().state,
            RemoteState::Enrolling { .. }
        ));
        assert!(cx.opened_url().is_some());
        window
            .read_with(&cx, |root, _| assert!(!root.settings_page.remote.busy))
            .unwrap();
    }
}

#[test]
fn cancelled_start_cannot_clear_a_second_windows_enrollment() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for fail in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        cx.run_until_parked();
        let first = switch_window(&mut cx, &store, temp.path(), "cancel-first");
        let second = switch_window(&mut cx, &store, temp.path(), "retry-second");
        let daemon = std::sync::Arc::new(RemoteRequests::default());
        store.update(&mut cx, |store, _| {
            store.client = DaemonClient::new(daemon.clone());
        });
        let (release, pending) = futures::channel::oneshot::channel();
        let requests = store.read_with(&cx, |store, _| store.enrollment_requests.clone());
        let abandoned = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancelled = abandoned.clone();
        let client = DaemonClient::new(daemon.clone());
        let task = cx
            .background_spawn(async move {
                requests
                    .start(cancelled, async move {
                        pending.await.unwrap();
                        if fail {
                            Err(runner_core::protocol::ClientError::msg(
                                "late startup error",
                            ))
                        } else {
                            client.remote_enroll_start(None, false)
                        }
                    })
                    .await
            })
            .shared();
        first
            .update(&mut cx, |root, _, _| {
                root.settings_page.remote.busy = true;
                root.settings_page.remote.opening = true;
                root.settings_page.remote.enrollment_start =
                    Some(EnrollmentStart { abandoned, task });
            })
            .unwrap();
        cx.run_until_parked();
        first
            .update(&mut cx, |root, _, cx| {
                root.remote_cancel_start(cx);
                assert!(!root.settings_page.remote.busy);
                assert!(!root.settings_page.remote.opening);
            })
            .unwrap();
        second
            .update(&mut cx, |root, _, cx| root.remote_connect(cx))
            .unwrap();
        cx.run_until_parked();
        assert!(
            daemon.calls.lock().unwrap().is_empty(),
            "second window must wait for the pending daemon call"
        );
        assert!(cx.opened_url().is_none());
        release.send(()).unwrap();
        cx.run_until_parked();
        let calls = daemon.calls.lock().unwrap();
        assert_eq!(
            calls.last(),
            Some(&"sign in"),
            "cleanup must not follow the newer enrollment"
        );
        assert_eq!(
            calls.iter().filter(|call| **call == "sign in").count(),
            if fail { 1 } else { 2 }
        );
        assert!(matches!(
            daemon.status.lock().unwrap().state,
            RemoteState::Enrolling { .. }
        ));
        assert!(cx.opened_url().is_some());
        second
            .read_with(&cx, |root, _| {
                assert!(!root.settings_page.remote.busy);
                assert!(root.settings_page.remote.error.is_none());
            })
            .unwrap();
    }
}

#[test]
fn sign_out_and_reset_hide_the_picker_and_keep_settings_in_place() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for reset in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        cx.run_until_parked();
        let requests = std::sync::Arc::new(RemoteRequests::default());
        *requests.status.lock().unwrap() = RemoteStatus {
            state: RemoteState::Paused,
            ..Default::default()
        };
        store.update(&mut cx, |store, _| {
            store.remote_status = Some(requests.status.lock().unwrap().clone());
        });
        let window = switch_window(&mut cx, &store, temp.path(), "sign-out-settings");
        let peer = switch_window(&mut cx, &store, temp.path(), "sign-out-settings-peer");
        store.update(&mut cx, |store, _| {
            store.client = DaemonClient::new(requests.clone())
        });
        let pane = if reset {
            SettingsPane::Remote
        } else {
            SettingsPane::Account
        };
        window
            .update(&mut cx, |root, window, cx| {
                root.enter_settings_pane(pane, window, cx);
            })
            .unwrap();
        cx.run_until_parked();
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        let mut peer_visual = VisualTestContext::from_window(peer.into(), &cx);
        visual.update(|window, _| window.activate_window());
        assert!(peer_visual.debug_bounds("SIDEBAR_SETTINGS").is_some());
        assert!(peer_visual.debug_bounds("SIDEBAR_HOST_BUTTON").is_some());
        window
            .update(&mut visual, |root, window, cx| {
                root.confirm_sign_out(cx);
                root.settings_page.remote.reset_identity = reset;
                root.settings_page.remote.focus.focus(window, cx);
            })
            .unwrap();
        visual.run_until_parked();
        visual.simulate_keystrokes("enter");
        visual.run_until_parked();
        store.update(&mut visual, |store, cx| {
            store.remote_status = Some(requests.status.lock().unwrap().clone());
            store.revisions.shell_settings += 1;
            cx.notify();
        });
        visual.run_until_parked();
        assert_eq!(
            *requests.calls.lock().unwrap(),
            [if reset { "reset" } else { "sign out" }]
        );
        assert!(peer_visual.debug_bounds("SIDEBAR_HOST_BUTTON").is_some());
        assert!(peer_visual.debug_bounds("SIDEBAR_SETTINGS").is_some());
        window
            .read_with(&visual, |root, _| assert_eq!(root.settings_page.pane, pane))
            .unwrap();
    }
}

#[test]
fn account_actions_reuse_requests_and_sign_out_and_reset_require_confirmation() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    let window = cx.add_window(|window, cx| {
        NativeRoot::new(
            "account-actions".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        )
    });
    cx.run_until_parked();
    let requests = std::sync::Arc::new(RemoteRequests {
        calls: Default::default(),
        status: Default::default(),
        trust: Default::default(),
        fail_logout: Default::default(),
    });
    store.update(&mut cx, |store, _| {
        store.client = DaemonClient::new(requests.clone())
    });
    window
        .update(&mut cx, |root, _, cx| {
            root.route = crate::AppRoute::Settings;
            root.settings_page.pane = SettingsPane::Account;
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    let sign_in = visual.debug_bounds("ACCOUNT_SIGN_IN").unwrap();
    visual.simulate_click(sign_in.center(), Default::default());
    cx.run_until_parked();
    assert_eq!(*requests.calls.lock().unwrap(), ["sign in"]);
    window
        .update(&mut cx, |root, _, cx| {
            root.remote_request(false, |c| c.remote_enroll_cancel(), cx)
        })
        .unwrap();
    cx.run_until_parked();
    window
        .update(&mut cx, |root, _, cx| {
            root.remote_request(false, |c| c.remote_retry(), cx)
        })
        .unwrap();
    cx.run_until_parked();
    for paused in [true, false] {
        window
            .update(&mut cx, |root, _, cx| {
                root.remote_request(false, move |c| c.remote_set_paused(paused), cx)
            })
            .unwrap();
        cx.run_until_parked();
    }
    assert_eq!(
        *requests.calls.lock().unwrap(),
        ["sign in", "cancel", "retry", "pause", "resume"]
    );
    window
        .update(&mut cx, |root, window, cx| {
            root.confirm_sign_out(cx);
            root.settings_page.remote.focus.focus(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    assert!(visual.debug_bounds("CONFIRM_DIALOG_PANEL").is_some());
    assert!(!requests.calls.lock().unwrap().contains(&"sign out"));
    cx.simulate_keystrokes(window.into(), "escape");
    cx.run_until_parked();
    assert!(visual.debug_bounds("CONFIRM_DIALOG_PANEL").is_none());
    assert!(!requests.calls.lock().unwrap().contains(&"sign out"));
    window
        .update(&mut cx, |root, window, cx| {
            root.confirm_sign_out(cx);
            root.settings_page.remote.focus.focus(window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    cx.simulate_keystrokes(window.into(), "enter");
    cx.run_until_parked();
    assert_eq!(requests.calls.lock().unwrap().last(), Some(&"sign out"));
    for confirm in [false, true] {
        window
            .update(&mut cx, |root, window, cx| {
                root.settings_page.pane = SettingsPane::Remote;
                root.settings_page.remote.confirm = true;
                root.settings_page.remote.reset_identity = true;
                root.settings_page.remote.focus.focus(window, cx);
                cx.notify();
            })
            .unwrap();
        cx.run_until_parked();
        cx.simulate_keystrokes(window.into(), if confirm { "enter" } else { "escape" });
        cx.run_until_parked();
        assert_eq!(requests.calls.lock().unwrap().contains(&"reset"), confirm);
    }
    store.update(&mut cx, |store, cx| {
        store.remote_status = Some(requests.status.lock().unwrap().clone());
        cx.notify();
    });
    window.update(&mut cx, |_, _, cx| cx.notify()).unwrap();
    cx.run_until_parked();
    assert!(visual.debug_bounds("REMOTE_RELAY_SIGNED_OUT").is_some());
    assert!(visual.debug_bounds("REMOTE_TAILSCALE_DISABLED").is_none());
    let sign_in = visual.debug_bounds("REMOTE_SIGN_IN").unwrap();
    visual.simulate_click(sign_in.center(), Default::default());
    cx.run_until_parked();
    assert_eq!(requests.calls.lock().unwrap().last(), Some(&"sign in"));
}

#[test]
fn pin_confirmation_and_lock_warning_keep_secrets_out_of_errors() {
    let pin = Secret("001234".into());
    assert!(confirmed_pin(&pin, &pin).unwrap().0 == pin.0);
    for weak in ["000000", "123456", "111111", "121212", "654321"] {
        let weak = Secret(weak.into());
        assert!(confirmed_pin(&weak, &weak).unwrap().0 == weak.0);
    }
    for invalid in [
        "12345",
        "1234567",
        "0123456789",
        " 123456",
        "１２３４５６",
        "12345x",
    ] {
        let invalid = Secret(invalid.into());
        assert_eq!(
            confirmed_pin(&invalid, &invalid).unwrap_err(),
            "remote.pin_invalid"
        );
    }
    assert_eq!(
        confirmed_pin(&pin, &Secret("654321".into())).unwrap_err(),
        "remote.pin_mismatch"
    );
    let unlocked = RemoteTrust {
        pin_set: true,
        ..Default::default()
    };
    let locked = RemoteTrust {
        pin_locked: true,
        ..unlocked.clone()
    };
    assert!(!warn_pin_lock(None, &locked));
    assert!(warn_pin_lock(Some(&unlocked), &locked));
    assert!(!warn_pin_lock(Some(&locked), &locked));
    assert!(!warn_pin_lock(Some(&locked), &unlocked));
}

#[test]
fn weak_pin_warning_requires_an_explicit_choice_and_keeps_the_pin_private() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            for action in ["choose", "enter", "escape", "mouse", "keyboard"] {
                let temp = tempfile::tempdir().unwrap();
                let mut cx = TestAppContext::single();
                let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
                let window = cx.add_window(|window, cx| {
                    let mut root = NativeRoot::new(
                        "weak-pin-warning".into(),
                        temp.path().join("logs"),
                        None,
                        None,
                        store.clone(),
                        window,
                        cx,
                    );
                    root.route = crate::AppRoute::Settings;
                    root.settings_page.pane = SettingsPane::Remote;
                    root.remote_open_pin(cx);
                    root.settings_page.remote.pin.as_ref().unwrap().fields[0]
                        .read(cx)
                        .focus
                        .clone()
                        .focus(window, cx);
                    root
                });
                cx.simulate_window_resize(window.into(), size(px(1200.), px(900.)));
                cx.simulate_keystrokes(window.into(), "0 0 0 0 0 0 enter 0 0 0 0 0 0 enter");
                cx.run_until_parked();
                let mut visual = VisualTestContext::from_window(window.into(), &cx);
                assert!(visual.debug_bounds("REMOTE_PIN_WARNING").is_some());
                assert!(visual.debug_bounds("REMOTE_PIN_BOX_0").is_none());
                assert!(!crate::i18n::text("remote.pin_weak").contains("000000"));
                assert!(crate::i18n::text("remote.pin_weak").contains(match locale {
                    Locale::English => "five",
                    Locale::SimplifiedChinese => "五",
                }));
                for focus_next in [false, true] {
                    if focus_next {
                        cx.simulate_keystrokes(window.into(), "tab");
                    }
                    for key in ["enter", "space"] {
                        visual.simulate_event(gpui::KeyDownEvent {
                            keystroke: gpui::Keystroke::parse(key).unwrap(),
                            is_held: true,
                            prefer_character_input: false,
                        });
                    }
                    assert!(visual.debug_bounds("REMOTE_PIN_WARNING").is_some());
                    window
                        .read_with(&cx, |root, _| {
                            assert_eq!(root.settings_page.remote.request_generation, 0);
                        })
                        .unwrap();
                }
                cx.simulate_keystrokes(window.into(), "shift-tab");
                window
                    .update(&mut cx, |root, window, _| {
                        assert_eq!(root.settings_page.remote.request_generation, 0);
                        assert!(root
                            .settings_page
                            .remote
                            .pin
                            .as_ref()
                            .unwrap()
                            .secondary_focus
                            .is_focused(window));
                        assert!(root.toasts.active().is_none());
                    })
                    .unwrap();
                match action {
                    "choose" => {
                        let button = visual.debug_bounds("REMOTE_PIN_SECONDARY").unwrap();
                        visual.simulate_click(button.center(), Default::default());
                    }
                    "enter" => cx.simulate_keystrokes(window.into(), "enter"),
                    "escape" => cx.simulate_keystrokes(window.into(), "escape"),
                    "mouse" => {
                        let button = visual.debug_bounds("REMOTE_PIN_PRIMARY").unwrap();
                        visual.simulate_click(button.center(), Default::default());
                    }
                    _ => {
                        cx.simulate_keystrokes(window.into(), "tab shift-tab tab space");
                    }
                }
                window
                    .read_with(&cx, |root, cx| {
                        let submitted = matches!(action, "mouse" | "keyboard");
                        assert_eq!(root.settings_page.remote.request_generation > 0, submitted);
                        if matches!(action, "choose" | "enter") {
                            let dialog = root.settings_page.remote.pin.as_ref().unwrap();
                            assert_eq!(dialog.step, 0);
                            assert!(dialog
                                .fields
                                .iter()
                                .all(|field| { field.read(cx).value().0.is_empty() }));
                        } else {
                            assert!(root.settings_page.remote.pin.is_none());
                        }
                    })
                    .unwrap();
            }
        });
    }
}

#[test]
fn two_step_pin_requires_six_digits_clears_on_back_and_mismatch_and_submits_on_enter() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    with_locale(Locale::English, || {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        let window = cx.add_window(|window, cx| {
            let mut root = NativeRoot::new(
                "pin-steps".into(),
                temp.path().join("logs"),
                None,
                None,
                store.clone(),
                window,
                cx,
            );
            root.route = crate::AppRoute::Settings;
            root.settings_page.pane = SettingsPane::Remote;
            root.remote_open_pin(cx);
            root.settings_page.remote.pin.as_ref().unwrap().fields[0]
                .read(cx)
                .focus
                .clone()
                .focus(window, cx);
            root
        });
        cx.simulate_window_resize(window.into(), size(px(1200.), px(900.)));
        cx.simulate_keystrokes(window.into(), "tab tab");
        window
            .update(&mut cx, |root, window, cx| {
                assert!(root.settings_page.remote.pin.as_ref().unwrap().fields[0]
                    .read(cx)
                    .focus
                    .is_focused(window));
            })
            .unwrap();
        cx.simulate_keystrokes(window.into(), "0 0 1 2 3 enter");
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        assert!(visual.debug_bounds("REMOTE_PIN_STEP_0").is_some());
        let primary = visual.debug_bounds("REMOTE_PIN_PRIMARY").unwrap();
        visual.simulate_click(primary.center(), Default::default());
        window
            .read_with(&cx, |root, cx| {
                let dialog = root.settings_page.remote.pin.as_ref().unwrap();
                assert_eq!(dialog.step, 0);
                assert_eq!(dialog.fields[0].read(cx).value().0.len(), 5);
            })
            .unwrap();
        let input = visual.debug_bounds("REMOTE_PIN_BOX_5").unwrap();
        visual.simulate_click(input.center(), Default::default());
        cx.simulate_keystrokes(window.into(), "4");
        cx.run_until_parked();
        window
            .read_with(&cx, |root, cx| {
                assert_eq!(
                    root.settings_page.remote.pin.as_ref().unwrap().fields[0]
                        .read(cx)
                        .value()
                        .0
                        .len(),
                    6
                );
            })
            .unwrap();
        cx.simulate_keystrokes(window.into(), "tab tab");
        window
            .update(&mut cx, |root, window, _| {
                assert!(root
                    .settings_page
                    .remote
                    .pin
                    .as_ref()
                    .unwrap()
                    .primary_focus
                    .is_focused(window));
            })
            .unwrap();
        cx.simulate_keystrokes(window.into(), "tab");
        let primary = visual.debug_bounds("REMOTE_PIN_PRIMARY").unwrap();
        visual.simulate_click(primary.center(), Default::default());
        window
            .update(&mut cx, |root, window, cx| {
                let dialog = root.settings_page.remote.pin.as_ref().unwrap();
                assert_eq!(dialog.step, 1);
                assert!(dialog.fields[1].read(cx).focus.is_focused(window));
            })
            .unwrap();
        cx.simulate_keystrokes(window.into(), "6 5 4 3 2 enter");
        window
            .read_with(&cx, |root, _| {
                assert_eq!(root.settings_page.remote.pin.as_ref().unwrap().step, 1)
            })
            .unwrap();
        cx.simulate_keystrokes(window.into(), "1 enter");
        window
            .read_with(&cx, |root, cx| {
                let dialog = root.settings_page.remote.pin.as_ref().unwrap();
                assert_eq!(dialog.step, 0);
                assert!(dialog
                    .fields
                    .iter()
                    .all(|field| field.read(cx).value().0.is_empty()));
                let toast = root.toasts.active().unwrap();
                assert_eq!(toast.message, "The PINs don't match.");
                assert_eq!(toast.tone, crate::toast::ToastTone::Error);
            })
            .unwrap();
        cx.run_until_parked();
        let close = visual.debug_bounds("GLOBAL_TOAST_CLOSE").unwrap();
        visual.simulate_click(close.center(), Default::default());
        window
            .read_with(&cx, |root, cx| {
                assert!(root.toasts.active().is_none());
                let dialog = root.settings_page.remote.pin.as_ref().unwrap();
                assert_eq!(dialog.step, 0);
                assert!(dialog
                    .fields
                    .iter()
                    .all(|field| field.read(cx).value().0.is_empty()));
            })
            .unwrap();
        cx.run_until_parked();
        assert!(visual.debug_bounds("GLOBAL_TOAST").is_none());
        assert!(visual.debug_bounds("REMOTE_PIN_STEP_0").is_some());
        let input = visual.debug_bounds("REMOTE_PIN_BOX_0").unwrap();
        visual.simulate_click(input.center(), Default::default());
        cx.simulate_keystrokes(window.into(), "0 0 1 2 3 4 enter 0 0");
        cx.run_until_parked();
        let back = visual.debug_bounds("REMOTE_PIN_SECONDARY").unwrap();
        visual.simulate_click(back.center(), Default::default());
        window
            .read_with(&cx, |root, cx| {
                let dialog = root.settings_page.remote.pin.as_ref().unwrap();
                assert_eq!(dialog.step, 0);
                assert!(dialog
                    .fields
                    .iter()
                    .all(|field| field.read(cx).value().0.is_empty()));
            })
            .unwrap();
        cx.simulate_keystrokes(window.into(), "0 0 1 2 3 4 enter 0 0 1 2 3 4 enter");
        window
            .read_with(&cx, |root, _| {
                assert!(root.settings_page.remote.pin.is_none());
                assert!(root.settings_page.remote.request_generation > 0);
            })
            .unwrap();
        window
            .update(&mut cx, |root, window, cx| {
                root.remote_open_pin(cx);
                root.settings_page.remote.pin.as_ref().unwrap().fields[0]
                    .read(cx)
                    .focus
                    .clone()
                    .focus(window, cx);
            })
            .unwrap();
        cx.simulate_keystrokes(window.into(), "0 0 escape");
        window
            .read_with(&cx, |root, _| {
                assert!(root.settings_page.remote.pin.is_none())
            })
            .unwrap();
    });
}

#[test]
fn trust_rows_dialogs_and_lock_toast_render_in_both_languages_and_themes() {
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
            let window = cx.add_window(|window, cx| {
                let mut root = NativeRoot::new(
                    "trust-snapshots".into(),
                    temp.path().join("logs"),
                    None,
                    None,
                    store.clone(),
                    window,
                    cx,
                );
                root.route = crate::AppRoute::Settings;
                root.settings_page.pane = SettingsPane::Remote;
                root
            });
            cx.simulate_window_resize(window.into(), size(px(1440.), px(1160.)));
            cx.run_until_parked();
            for variant in [
                theme::ThemeVariant::Carbon,
                theme::ThemeVariant::RunnerLight,
            ] {
                theme::set_active_variant(variant);
                for (pin_set, pin_locked, devices) in
                    [(false, false, 0), (true, false, 3), (true, true, 3)]
                {
                    let now = chrono::Utc::now();
                    store.update(&mut cx, |store, cx| {
                        store.remote_status = Some(RemoteStatus {
                            state: RemoteState::Connected { since: now },
                            account: Some(runner_core::remote::RemoteAccount {
                                username: "jason".into(),
                                display_name: "Jason".into(),
                            }),
                            host_name: Some("Mac Studio".into()),
                            ..Default::default()
                        });
                        store.remote_trust = Some(RemoteTrust {
                            fingerprint: Some("630d cd29 66c4 3366 9112".into()),
                            pin_set,
                            pin_locked,
                            devices: (0..devices)
                                .map(|index| runner_core::remote::PairedDevice {
                                    fingerprint: format!("2b7f 91c0 a3e4 5d12 0f6{index}"),
                                    name: ["MacBook Air", "Windows PC", "Linux server"][index]
                                        .into(),
                                    public_key: String::new(),
                                    paired_at: now,
                                    last_seen_at: now,
                                    connected: index == 0,
                                    kind: Default::default(),
                                })
                                .collect(),
                        });
                        cx.notify();
                    });
                    window
                        .update(&mut cx, |root, _, cx| {
                            root.dismiss_remote_dialog();
                            if pin_locked {
                                root.toasts.show(
                                    crate::i18n::text("remote.pin_locked_toast").to_string(),
                                    crate::toast::ToastTone::Warning,
                                );
                                root.toasts.pin_lock_warning();
                            } else {
                                root.toasts.dismiss();
                            }
                            cx.notify();
                        })
                        .unwrap();
                    cx.run_until_parked();
                    let mut visual = VisualTestContext::from_window(window.into(), &cx);
                    let pairing = visual.debug_bounds("REMOTE_PAIRING").unwrap();
                    let table = visual.debug_bounds("REMOTE_DEVICES").unwrap();
                    assert!(table.top() > pairing.top());
                    assert!(table.bottom() < px(1160.));
                    assert_eq!(
                        visual.debug_bounds("REMOTE_DEVICE_ROW").is_some(),
                        devices > 0
                    );
                    assert_eq!(
                        visual.debug_bounds("REMOTE_CONNECTED_NOW").is_some(),
                        devices > 0
                    );
                    if devices > 0 {
                        store.update(&mut cx, |store, cx| {
                            store.remote_trust.as_mut().unwrap().devices[0].connected = false;
                            cx.notify();
                        });
                        window.update(&mut cx, |_, _, cx| cx.notify()).unwrap();
                        cx.run_until_parked();
                        assert!(visual.debug_bounds("REMOTE_CONNECTED_NOW").is_none());
                    }
                    if pin_locked {
                        let toast = visual.debug_bounds("GLOBAL_TOAST").unwrap();
                        assert!(toast.top() > px(1000.));
                    }
                    window
                        .update(&mut cx, |root, window, cx| {
                            root.remote_open_pin(cx);
                            root.settings_page.remote.pin.as_ref().unwrap().fields[0]
                                .read(cx)
                                .focus
                                .clone()
                                .focus(window, cx);
                            cx.notify();
                        })
                        .unwrap();
                    cx.run_until_parked();
                    let pin_dialog = visual.debug_bounds("REMOTE_PIN_DIALOG").unwrap();
                    assert!(pin_dialog.left() >= px(0.) && pin_dialog.right() <= px(1440.));
                    assert!(pin_dialog.top() >= px(0.) && pin_dialog.bottom() <= px(1160.));
                    let boxes = [
                        "REMOTE_PIN_BOX_0",
                        "REMOTE_PIN_BOX_1",
                        "REMOTE_PIN_BOX_2",
                        "REMOTE_PIN_BOX_3",
                        "REMOTE_PIN_BOX_4",
                        "REMOTE_PIN_BOX_5",
                    ];
                    let first = visual.debug_bounds(boxes[0]).unwrap();
                    for selector in boxes {
                        let bounds = visual.debug_bounds(selector).unwrap();
                        assert_eq!(bounds.size.height, px(48.));
                        assert!((bounds.size.width - first.size.width).abs() < px(1.));
                        assert!(
                            bounds.left() > pin_dialog.left()
                                && bounds.right() < pin_dialog.right()
                        );
                    }
                    window
                        .update(&mut cx, |root, window, cx| {
                            root.remote_advance_pin(window, cx);
                            assert_eq!(root.settings_page.remote.pin.as_ref().unwrap().step, 0);
                            root.settings_page.remote.pin.as_ref().unwrap().fields[0].update(
                                cx,
                                |field, cx| {
                                    field.replace_text_in_range(None, "001234", window, cx);
                                },
                            );
                            root.remote_advance_pin(window, cx);
                            assert_eq!(root.settings_page.remote.pin.as_ref().unwrap().step, 1);
                        })
                        .unwrap();
                    cx.run_until_parked();
                    assert!(visual.debug_bounds("REMOTE_PIN_STEP_0").is_none());
                    assert!(visual.debug_bounds("REMOTE_PIN_STEP_1").is_some());
                    window
                        .update(&mut cx, |root, _, cx| {
                            root.dismiss_remote_dialog();
                            assert!(root.settings_page.remote.pin.is_none());
                            root.settings_page.remote.confirm = true;
                            root.settings_page.remote.reset_identity = true;
                            cx.notify();
                        })
                        .unwrap();
                    cx.run_until_parked();
                    let reset_dialog = visual.debug_bounds("CONFIRM_DIALOG_PANEL").unwrap();
                    assert!(reset_dialog.left() >= px(0.) && reset_dialog.right() <= px(1440.));
                    assert!(reset_dialog.top() >= px(0.) && reset_dialog.bottom() <= px(1160.));
                }
            }
        });
    }
}

#[test]
fn a_device_row_opens_a_menu_with_its_fingerprint_above_revoke() {
    use runner_core::remote::{DeviceKind, PairedDevice};
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for locale in [Locale::English, Locale::SimplifiedChinese] {
        with_locale(locale, || {
            for kind in [DeviceKind::Phone, DeviceKind::Unknown] {
                let temp = tempfile::tempdir().unwrap();
                let mut cx = TestAppContext::single();
                let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
                let window = cx.add_window(|window, cx| {
                    let mut root = NativeRoot::new(
                        "device-row".into(),
                        temp.path().join("logs"),
                        None,
                        None,
                        store.clone(),
                        window,
                        cx,
                    );
                    root.route = crate::AppRoute::Settings;
                    root.settings_page.pane = SettingsPane::Remote;
                    root
                });
                cx.simulate_window_resize(window.into(), size(px(1440.), px(1160.)));
                cx.run_until_parked();
                let now = chrono::Utc::now();
                store.update(&mut cx, |store, cx| {
                    store.remote_status = Some(RemoteStatus {
                        state: RemoteState::Connected { since: now },
                        account: Some(runner_core::remote::RemoteAccount {
                            username: "jason".into(),
                            display_name: "Jason".into(),
                        }),
                        host_name: Some("Mac Studio".into()),
                        ..Default::default()
                    });
                    store.remote_trust = Some(RemoteTrust {
                        pin_set: true,
                        devices: vec![PairedDevice {
                            fingerprint: "9134 2aa5 19ea 589a 15fc".into(),
                            name: "Jason's iPhone".into(),
                            public_key: String::new(),
                            paired_at: now,
                            last_seen_at: now - chrono::Duration::days(5),
                            connected: false,
                            kind,
                        }],
                        ..Default::default()
                    });
                    cx.notify();
                });
                window.update(&mut cx, |_, _, cx| cx.notify()).unwrap();
                cx.run_until_parked();
                let mut visual = VisualTestContext::from_window(window.into(), &cx);
                assert!(visual.debug_bounds("REMOTE_DEVICE_ROW").is_some());
                assert_eq!(
                    visual.debug_bounds("REMOTE_PHONE_ICON").is_some(),
                    kind == DeviceKind::Phone
                );
                let trigger = visual.debug_bounds("POPOVER_TRIGGER").unwrap();
                visual.simulate_click(trigger.center(), gpui::Modifiers::default());
                cx.run_until_parked();
                let panel = visual.debug_bounds("POPOVER_MENU_PANEL").unwrap();
                let label = visual.debug_bounds("POPOVER_LABEL_0").unwrap();
                let value = visual.debug_bounds("POPOVER_VALUE_0").unwrap();
                let revoke = visual.debug_bounds("POPOVER_LABEL_1").unwrap();
                assert!(visual.debug_bounds("POPOVER_ITEM_0").is_none());
                assert!(visual.debug_bounds("POPOVER_ICON_1").is_some());
                assert!(panel.size.width >= px(240.));
                assert!(label.top() < value.top() && value.bottom() <= revoke.top());
                assert_eq!(label.left(), value.left());
                for part in [label, value, revoke] {
                    assert!(
                        part.left() >= panel.left()
                            && part.right() <= panel.right()
                            && part.bottom() <= panel.bottom(),
                        "{locale:?} {kind:?}: {part:?} outside {panel:?}"
                    );
                }
                // The core knows no such device, so reaching it shows up as the failure toast.
                visual.simulate_click(label.center(), gpui::Modifiers::default());
                cx.run_until_parked();
                assert!(visual.debug_bounds("POPOVER_MENU_PANEL").is_some());
                assert!(visual.debug_bounds("GLOBAL_TOAST").is_none());
                visual.simulate_click(revoke.center(), gpui::Modifiers::default());
                cx.run_until_parked();
                assert!(visual.debug_bounds("POPOVER_MENU_PANEL").is_none());
                assert!(visual.debug_bounds("GLOBAL_TOAST").is_some());
            }
        });
    }
}

#[test]
fn warning_requires_a_live_transition_to_a_relevant_stop() {
    let mut s = RemoteStatus::default();
    for reason in [
        StopReason::Removed,
        StopReason::InUseElsewhere,
        StopReason::UpdateRequired,
        StopReason::Denied,
        StopReason::Expired,
    ] {
        s.state = RemoteState::Stopped { reason };
        assert!(!warn_transition(None, &s));
        assert!(!warn_transition(Some(&s), &s));
        assert_eq!(
            warn_transition(Some(&RemoteStatus::default()), &s),
            matches!(
                reason,
                StopReason::Removed | StopReason::InUseElsewhere | StopReason::UpdateRequired
            )
        );
    }
}
