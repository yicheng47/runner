//! OSC 52 clipboard writes from the programs in Runner's panes (#899).

use futures::StreamExt;
use gpui::{App, AppContext, ClipboardItem};
use runner_core::protocol::ClientEvent;
use runner_terminal::terminal::ClipboardTarget;

use crate::surfaces::pane_shows_session;
use crate::{global_app_store, AppRoute, NativeRoot};

/// One listener for the whole app, so a session shown in several windows
/// writes the clipboard once.
pub(crate) fn install(cx: &mut App) {
    let mut events = global_app_store(cx).read(cx).client.subscribe();
    let (tx, mut rx) = futures::channel::mpsc::unbounded::<ClientEvent>();
    cx.background_spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) if event.name == "session/clipboard" => {
                    if tx.unbounded_send(event).is_err() {
                        break;
                    }
                }
                Ok(_) | Err(runner_core::protocol::EventError::Lagged(_)) => {}
                Err(runner_core::protocol::EventError::Closed) => break,
            }
        }
    })
    .detach();
    cx.spawn(async move |cx| {
        while let Some(event) = rx.next().await {
            cx.update(|cx| {
                if let Some((target, text)) = shown_write(&event, cx) {
                    write(target, text, cx);
                }
            });
        }
    })
    .detach();
}

/// The write to make, or `None` when no window shows the session in its active tab.
fn shown_write(event: &ClientEvent, cx: &App) -> Option<(ClipboardTarget, String)> {
    let field = |name| event.payload.get(name).and_then(serde_json::Value::as_str);
    let session = field("session_id")?;
    let target = ClipboardTarget::parse(field("target")?)?;
    let text = field("text").filter(|text| !text.is_empty())?;
    cx.windows()
        .into_iter()
        .filter_map(|handle| handle.downcast::<NativeRoot>())
        .any(|handle| {
            handle
                .read(cx)
                .is_ok_and(|root| root.shows_terminal(session, cx))
        })
        .then(|| (target, text.to_owned()))
}

fn write(target: ClipboardTarget, text: String, cx: &mut App) {
    let item = ClipboardItem::new_string(text);
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    if target == ClipboardTarget::Selection {
        return cx.write_to_primary(item);
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    let _ = target;
    cx.write_to_clipboard(item);
}

impl NativeRoot {
    /// Whether a pane of this window's active tab shows the terminal, its open
    /// drawer's selected shell included. A pane hidden behind a maximized
    /// neighbour is not shown.
    pub(crate) fn shows_terminal(&self, id: &str, cx: &App) -> bool {
        if self.remote_client.is_active() {
            let hidden = runner_core::remote::terminal_target(id).is_some_and(|(_, session)| {
                self.front_page_layout().is_some_and(|layout| {
                    layout.contains_session(session)
                        && !pane_shows_session(&self.maximized_panes, layout, session)
                })
            });
            return !hidden && self.remote_client.shows_remote_terminal(id, cx);
        }
        match &self.route {
            AppRoute::Chat => self.tabs.active().is_some_and(|tab| {
                pane_shows_session(&self.maximized_panes, tab, id)
                    || tab.drawer_open() && tab.active_drawer_shell() == Some(id)
            }),
            AppRoute::Mission(mission) => self
                .mission_workspace
                .read(cx)
                .shown_terminals(mission, cx)
                .iter()
                .any(|session| session == id),
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surfaces::remote_client::tests::{add_window, choose_host, fixture};
    use runner_app::pane_layout::{PaneLayout, TabSet};
    use runner_core::protocol::node::{NodeRow, NodeType};

    fn tab(id: &str, session: &str) -> NodeRow {
        tab_with(id, &PaneLayout::single(Some(session), &[]))
    }

    fn tab_with(id: &str, layout: &PaneLayout) -> NodeRow {
        NodeRow {
            id: id.into(),
            parent_id: None,
            position: 0,
            node_type: NodeType::Tab,
            name: None,
            ref_id: None,
            layout: Some(layout.serialize().unwrap()),
            pinned_position: None,
            last_completed_at: None,
            last_viewed_at: None,
            created_at: "2026-11-05T00:00:00Z".into(),
        }
    }

    fn event(session: &str, target: &str, text: &str) -> ClientEvent {
        ClientEvent {
            name: "session/clipboard".into(),
            payload: serde_json::json!({"session_id":session,"target":target,"text":text}),
        }
    }

    #[test]
    fn writes_once_only_when_a_window_shows_the_session_in_its_active_tab() {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let (store, fake, _) = fixture(&mut cx, temp.path());
        let rows = [
            tab("01ARZ3NDEKTSV4RRFFQ69G5FAV", "shown"),
            tab("01ARZ3NDEKTSV4RRFFQ69G5FAW", "hidden"),
        ];
        let windows = ["first", "second"].map(|name| {
            let window = add_window(&mut cx, &store, temp.path(), name);
            window
                .update(&mut cx, |root, _, _| {
                    root.tabs = TabSet::from_rows(&rows);
                    root.tabs.activate("01ARZ3NDEKTSV4RRFFQ69G5FAV");
                    let active = root.tabs.active_mut().unwrap();
                    active.add_drawer_shell("drawer".into());
                    active.set_drawer_open(true);
                    root.route = AppRoute::Chat;
                })
                .unwrap();
            window
        });
        let decide = |cx: &mut gpui::TestAppContext, event: ClientEvent| {
            cx.read(|cx| shown_write(&event, cx))
        };

        assert_eq!(
            decide(&mut cx, event("shown", "clipboard", "both windows")),
            Some((ClipboardTarget::Clipboard, "both windows".into()))
        );
        assert_eq!(
            decide(&mut cx, event("drawer", "selection", "shell")),
            Some((ClipboardTarget::Selection, "shell".into()))
        );
        for unshown in [
            event("hidden", "clipboard", "inactive tab"),
            event("unknown", "clipboard", "no pane"),
            event("shown", "clipboard", ""),
            event("shown", "primary", "unknown target"),
        ] {
            assert_eq!(decide(&mut cx, unshown), None);
        }

        windows[0]
            .update(&mut cx, |root, _, _| root.route = AppRoute::Settings)
            .unwrap();
        assert!(decide(&mut cx, event("shown", "clipboard", "second window")).is_some());
        windows[1]
            .update(&mut cx, |root, _, _| {
                root.tabs.activate("01ARZ3NDEKTSV4RRFFQ69G5FAW");
            })
            .unwrap();
        assert_eq!(
            decide(&mut cx, event("shown", "clipboard", "neither")),
            None
        );

        cx.update(|cx| {
            cx.set_global(crate::GlobalAppStore(store.clone()));
            cx.write_to_clipboard(ClipboardItem::new_string("before".into()));
            install(cx);
        });
        fake.emit(
            "session/clipboard",
            event("shown", "clipboard", "dropped").payload,
        );
        fake.emit(
            "session/clipboard",
            event("hidden", "clipboard", "copied").payload,
        );
        cx.run_until_parked();
        assert_eq!(
            cx.read(|cx| cx.read_from_clipboard().and_then(|item| item.text())),
            Some("copied".into())
        );
    }

    #[test]
    fn a_pane_hidden_by_a_maximized_neighbour_does_not_write() {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let (store, _, _) = fixture(&mut cx, temp.path());
        let mut layout = PaneLayout::single(Some("front"), &[]);
        let back = layout
            .split("p1", runner_app::pane_layout::SplitOrientation::Row)
            .unwrap();
        layout.assign_session(&back, "back").unwrap();
        assert!(layout.focus_pane("p1"));
        let row = tab_with("01ARZ3NDEKTSV4RRFFQ69G5FAX", &layout);
        let windows = ["maximized", "split"].map(|name| {
            let window = add_window(&mut cx, &store, temp.path(), name);
            window
                .update(&mut cx, |root, _, _| {
                    root.tabs = TabSet::from_rows(std::slice::from_ref(&row));
                    root.tabs.activate("01ARZ3NDEKTSV4RRFFQ69G5FAX");
                    root.route = AppRoute::Chat;
                })
                .unwrap();
            window
        });
        let decide = |cx: &mut gpui::TestAppContext, session: &str| {
            cx.read(|cx| shown_write(&event(session, "clipboard", "text"), cx))
                .is_some()
        };
        windows[1]
            .update(&mut cx, |root, _, _| root.route = AppRoute::Settings)
            .unwrap();
        windows[0]
            .update(&mut cx, |root, _, _| {
                root.maximized_panes
                    .insert("01ARZ3NDEKTSV4RRFFQ69G5FAX".into(), "p1".into());
            })
            .unwrap();
        assert!(decide(&mut cx, "front"), "the maximized pane is shown");
        assert!(!decide(&mut cx, "back"), "its hidden neighbour is not");

        windows[1]
            .update(&mut cx, |root, _, _| root.route = AppRoute::Chat)
            .unwrap();
        assert!(decide(&mut cx, "back"), "another window still shows it");
        windows[1]
            .update(&mut cx, |root, _, _| root.route = AppRoute::Settings)
            .unwrap();
        windows[0]
            .update(&mut cx, |root, _, _| root.maximized_panes.clear())
            .unwrap();
        assert!(
            decide(&mut cx, "back"),
            "restoring the split shows it again"
        );
    }

    #[test]
    fn a_remote_pane_hidden_by_a_maximized_neighbour_does_not_write() {
        use crate::surfaces::remote_client::tests::overview;
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let (store, fake, _) = fixture(&mut cx, temp.path());
        let window = add_window(&mut cx, &store, temp.path(), "remote-maximized");
        window.update(&mut cx, choose_host).unwrap();
        cx.run_until_parked();
        let mut pushed = overview();
        let neighbour = pushed.sessions["s"].clone();
        pushed.sessions.insert("t".into(), neighbour);
        pushed.tabs[0].panes = serde_json::from_value(serde_json::json!({"split":"row","sizes":[0.5,0.5],"a":{"session":"s"},"b":{"session":"t"}})).unwrap();
        fake.set_overview(pushed);
        cx.run_until_parked();
        let shown = |cx: &mut gpui::TestAppContext, session: &str| {
            let address = runner_core::remote::terminal_address("host", session);
            cx.read(|cx| shown_write(&event(&address, "clipboard", "remote"), cx))
                .is_some()
        };
        assert!(shown(&mut cx, "s") && shown(&mut cx, "t"));
        let pane = window
            .update(&mut cx, |root, window, cx| {
                let pane = root
                    .front_page_layout()
                    .unwrap()
                    .root
                    .leaves()
                    .into_iter()
                    .find(|leaf| leaf.session_id.as_deref() == Some("s"))
                    .unwrap()
                    .id
                    .clone();
                root.toggle_maximize_pane(&pane, window, cx);
                pane
            })
            .unwrap();
        assert!(shown(&mut cx, "s"));
        assert!(
            !shown(&mut cx, "t"),
            "the hidden remote neighbour does not write"
        );
        window
            .update(&mut cx, |root, window, cx| {
                root.toggle_maximize_pane(&pane, window, cx)
            })
            .unwrap();
        assert!(shown(&mut cx, "t"), "restoring the split shows it again");
    }

    #[test]
    fn a_remote_window_shows_its_terminals_under_their_remote_address() {
        let temp = tempfile::tempdir().unwrap();
        let mut cx = gpui::TestAppContext::single();
        let (store, _, _) = fixture(&mut cx, temp.path());
        let window = add_window(&mut cx, &store, temp.path(), "remote");
        window.update(&mut cx, choose_host).unwrap();
        cx.run_until_parked();
        let address = runner_core::remote::terminal_address("host", "s");
        assert!(cx
            .read(|cx| shown_write(&event(&address, "clipboard", "remote"), cx))
            .is_some());
        assert!(cx
            .read(|cx| shown_write(&event("s", "clipboard", "local id"), cx))
            .is_none());
    }
}
