//! OSC 52 clipboard writes from the programs in Runner's panes (#899).

use futures::StreamExt;
use gpui::{App, AppContext, ClipboardItem};
use runner_core::protocol::ClientEvent;
use runner_terminal::terminal::ClipboardTarget;

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
    /// drawer's selected shell included.
    pub(crate) fn shows_terminal(&self, id: &str, cx: &App) -> bool {
        if self.remote_client.is_active() {
            return self.remote_client.shows_remote_terminal(id, cx);
        }
        match &self.route {
            AppRoute::Chat => self.tabs.active().is_some_and(|tab| {
                tab.contains_session(id)
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
        let layout = PaneLayout::single(Some(session), &[]);
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
