use super::*;
use gpui::{Context, Render, TestAppContext, VisualTestContext, Window};
use runner_app::terminal_ime::TerminalInput;
use runner_app::ui::Scrollbar;
use runner_core::protocol::terminal::{
    TerminalAttachment, TerminalFrame, TerminalMetadata, TerminalSnapshot, TerminalSubscription,
};
use runner_core::protocol::{
    ClientError, DaemonClient, EventSubscription, Request, Response, Transport,
};
use runner_terminal::terminal::TerminalMirror;
use std::sync::{mpsc, Mutex};
use std::time::Duration;

#[derive(Default)]
pub(crate) struct RecordingHost {
    pub(crate) calls: Mutex<Vec<Request>>,
    inputs: Mutex<Vec<Vec<u8>>>,
    pub(crate) sizes: Mutex<Vec<(u16, u16)>>,
    controller: Mutex<TerminalController>,
    stream: Mutex<Option<(mpsc::Sender<TerminalFrame>, mpsc::Receiver<TerminalFrame>)>>,
    processed: Mutex<Option<mpsc::Sender<()>>>,
    input_tracker: Mutex<Option<runner_terminal::input_state::InputTracker>>,
    input_observation: Mutex<Option<runner_terminal::input_state::InputObservation>>,
}
struct Frames(mpsc::Receiver<TerminalFrame>);
impl TerminalSubscription for Frames {
    fn recv(&mut self) -> Result<TerminalFrame, ClientError> {
        self.0.recv().map_err(|_| ClientError::msg("closed"))
    }
    fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
        Arc::new(|| {})
    }
}
impl Transport for RecordingHost {
    fn call(&self, request: Request) -> Result<Response, ClientError> {
        let response = match &request {
            Request::terminal_metadata { .. } => {
                if let Some(tx) = &*self.processed.lock().unwrap() {
                    let _ = tx.send(());
                }
                Response::terminal_metadata(Ok(TerminalMetadata::default()))
            }
            Request::terminal_take_back { .. } => Response::terminal_take_back(Ok(())),
            Request::terminal_observe_input { input, .. } => {
                if let Some(tracker) = self.input_tracker.lock().unwrap().as_mut() {
                    if let Some(observation) = tracker.observe_input(
                        input,
                        std::time::Instant::now(),
                        &runner_terminal::replay::new_term(80, 24),
                    ) {
                        *self.input_observation.lock().unwrap() = Some(observation);
                    }
                }
                Response::terminal_observe_input(Ok(()))
            }
            Request::terminal_input_reset_guard { .. } => {
                Response::terminal_input_reset_guard(Ok(0))
            }
            Request::terminal_reset_input_state { .. } => {
                Response::terminal_reset_input_state(Ok(()))
            }
            _ => return Err(ClientError::msg("unexpected request")),
        };
        if !matches!(request, Request::terminal_metadata { .. }) {
            self.calls.lock().unwrap().push(request);
        }
        Ok(response)
    }
    fn subscribe(&self) -> Box<dyn EventSubscription> {
        unreachable!()
    }
    fn attach(&self, _: &str) -> Result<TerminalAttachment, ClientError> {
        let (_, rx) = self.stream.lock().unwrap().take().unwrap();
        Ok(TerminalAttachment {
            subscriber_id: 1,
            snapshot: TerminalSnapshot {
                seq: 0,
                cols: 80,
                rows: 24,
                controller: self.controller.lock().unwrap().clone(),
                bytes: Vec::new(),
                unfinished_len: 0,
                preceding_char: None,
            },
            frames: Box::new(Frames(rx)),
        })
    }
    fn input(&self, _: &str, bytes: &[u8]) -> Result<(), ClientError> {
        self.inputs.lock().unwrap().push(bytes.to_vec());
        Ok(())
    }
    fn resize(&self, _: &str, _: u64, cols: u16, rows: u16) {
        self.sizes.lock().unwrap().push((cols, rows));
    }
}

pub(crate) fn mirror_named(
    id: &str,
    controller: TerminalController,
) -> (
    Arc<RecordingHost>,
    Arc<TerminalMirror>,
    mpsc::Sender<TerminalFrame>,
    mpsc::Receiver<()>,
) {
    let (tx, rx) = mpsc::channel();
    let host = Arc::new(RecordingHost {
        controller: Mutex::new(controller),
        stream: Mutex::new(Some((tx.clone(), rx))),
        ..Default::default()
    });
    let mirror =
        TerminalMirror::attach(DaemonClient::new(host.clone()), id.into(), Arc::new(|| {}))
            .unwrap();
    let (ack, received) = mpsc::channel();
    *host.processed.lock().unwrap() = Some(ack);
    (host, mirror, tx, received)
}
fn mirror(
    controller: TerminalController,
) -> (
    Arc<RecordingHost>,
    Arc<TerminalMirror>,
    mpsc::Sender<TerminalFrame>,
    mpsc::Receiver<()>,
) {
    mirror_named("test", controller)
}
pub(crate) fn remote() -> TerminalController {
    TerminalController::Browser {
        channel: 1,
        name: "MacBook Air".into(),
    }
}
pub(crate) fn change(
    tx: &mpsc::Sender<TerminalFrame>,
    ack: &mpsc::Receiver<()>,
    seq: u64,
    controller: TerminalController,
) {
    tx.send(TerminalFrame::Resized {
        seq,
        cols: 91,
        rows: 31,
        controller,
    })
    .unwrap();
    ack.recv_timeout(Duration::from_secs(5)).unwrap();
}

#[test]
fn ordered_controller_gates_every_shared_size_and_input_path() {
    let (host, terminal, tx, ack) = mirror(TerminalController::Desktop);
    terminal.resize(100, 40);
    terminal.resize(110, 41);
    assert_eq!(*host.sizes.lock().unwrap(), [(100, 40), (110, 41)]);
    terminal.send_text("desktop").unwrap();
    assert_eq!(*host.inputs.lock().unwrap(), [b"desktop".to_vec()]);
    host.inputs.lock().unwrap().clear();
    host.calls.lock().unwrap().clear();
    host.sizes.lock().unwrap().clear();
    change(&tx, &ack, 1, remote());
    assert!(!terminal.control().desktop());
    terminal.refresh_metadata();
    assert!(
        !terminal.control().desktop(),
        "ahead-of-stream metadata cannot change control"
    );
    ack.recv_timeout(Duration::from_secs(5)).unwrap();
    for seq in [2, 3] {
        if seq == 3 {
            change(&tx, &ack, seq, TerminalController::None);
        }
        terminal.resize(200, 60);
        terminal.write_user_bytes(b"mouse or key").unwrap();
        terminal
            .send_key("enter", false, false, false, None)
            .unwrap();
        terminal.send_text("IME").unwrap();
        terminal.paste("drop").unwrap();
        terminal.submit_text("composer").unwrap();
        terminal.set_composing(true);
        let mut input = TerminalInput::new(terminal.clone());
        input.replace_and_mark_text(None, "marked", None);
        assert!(!input.is_composing());
        input.commit_text("commit").unwrap();
        terminal.reset_input_state(0);
        assert!(host.inputs.lock().unwrap().is_empty());
        assert!(host.calls.lock().unwrap().is_empty());
        assert!(host.sizes.lock().unwrap().is_empty());
    }
    assert_eq!(
        terminal.control().last_remote_name.as_deref(),
        Some("MacBook Air")
    );
    change(&tx, &ack, 2, TerminalController::Desktop);
    assert!(
        !terminal.control().desktop(),
        "stale frame cannot return control"
    );
    change(&tx, &ack, 4, TerminalController::Desktop);
    terminal.resize(200, 60);
    terminal.paste("here").unwrap();
    assert_eq!(*host.sizes.lock().unwrap(), [(200, 60)]);
    assert_eq!(*host.inputs.lock().unwrap(), [b"here".to_vec()]);
}

#[test]
fn card_state_tracks_panes_groups_disconnects_and_unknown_names() {
    let desktop = TerminalControl::default();
    let remote = TerminalControl {
        controller: remote(),
        last_remote_name: Some("MacBook Air".into()),
    };
    let left = TerminalControl {
        controller: TerminalController::None,
        ..remote.clone()
    };
    assert!(card_state(std::slice::from_ref(&desktop)).is_none());
    assert_eq!(card_state(std::slice::from_ref(&remote)).unwrap().count, 1);
    assert_eq!(
        card_state(&[remote.clone(), remote.clone()]).unwrap().count,
        2
    );
    assert!(card_state(&[remote.clone(), desktop.clone()]).is_none());
    assert!(card_state(&[remote, left.clone()]).is_none());
    assert!(card_state(&[left.clone(), left]).unwrap().left);
    assert_eq!(
        card_state(&[TerminalControl {
            controller: TerminalController::None,
            last_remote_name: None
        }])
        .unwrap()
        .name,
        "Another computer"
    );
    assert!(card_state(&[desktop]).is_none());
}

struct ControlView {
    chats: Vec<AttachedChat>,
}
impl Render for ControlView {
    fn render(&mut self, _: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let style = super::super::element::TerminalStyle {
            palette: runner_terminal::palette::RUNNER,
            font: crate::app_settings::AppSettings::default()
                .terminal_font_family
                .font(),
            font_size: 14.,
            app_zoom: 1.,
        };
        let panes = self
            .chats
            .iter()
            .enumerate()
            .map(|(index, chat)| {
                div()
                    .track_focus(&chat.terminal_focus)
                    .key_context("Terminal")
                    .w(gpui::px(if index == 0 { 400. } else { 200. }))
                    .h(gpui::px(if index == 0 { 300. } else { 400. }))
                    .child(super::super::TerminalElement::new(
                        chat.terminal.clone(),
                        chat.terminal_interaction.clone(),
                        chat.terminal_input.clone(),
                        chat.terminal_focus.clone(),
                        true,
                        true,
                        style.clone(),
                    ))
            })
            .collect::<Vec<_>>();
        cover(
            div().size_full().flex().children(panes),
            "test-take-back",
            &self.chats.iter().collect::<Vec<_>>(),
            GroupKind::Tab,
        )
    }
}
pub(crate) fn attached<T: 'static>(
    terminal: Arc<TerminalMirror>,
    cx: &mut Context<T>,
    cols: u16,
    rows: u16,
) -> AttachedChat {
    let interaction = cx.new(|_| super::super::TerminalInteraction::new(terminal.clone()));
    interaction.update(cx, |interaction, _| {
        interaction.pane_size = Some(runner_app::terminal_resize::TerminalGridSize { cols, rows })
    });
    let input = cx.new(|_| TerminalInput::new(terminal.clone()));
    AttachedChat {
        _terminal_view: terminal.view(),
        terminal_scrollbar: cx.new(|_| Scrollbar::terminal(terminal.clone())),
        terminal,
        terminal_interaction: interaction,
        _terminal_input_subscription: cx.observe(&input, |_, _, _| {}),
        _terminal_focus_subscription: cx.observe(&input, |_, _, _| {}),
        terminal_input: input,
        terminal_focus: cx.focus_handle(),
    }
}

#[test]
fn take_back_and_continue_send_each_panes_size_and_enter_activates_the_card() {
    for controller in [remote(), TerminalController::None] {
        let (left_host, left, _left_tx, _) = mirror(controller.clone());
        let (right_host, right, _right_tx, _) = mirror(controller);
        let mut cx = TestAppContext::single();
        let window = cx.add_window(|_, cx| ControlView {
            chats: vec![attached(left, cx, 101, 30), attached(right, cx, 72, 45)],
        });
        let mut visual = VisualTestContext::from_window(window.into(), &cx);
        let bounds = visual
            .debug_bounds("REMOTE_CONTROL_CARD")
            .expect("control card");
        assert!(bounds.size.width <= gpui::px(400.));
        window
            .update(&mut cx, |view, window, cx| {
                view.chats[0].terminal_focus.focus(window, cx)
            })
            .unwrap();
        cx.simulate_keystrokes(window.into(), "enter");
        let measured = window
            .read_with(&cx, |view, cx| {
                view.chats
                    .iter()
                    .map(|chat| chat.terminal_interaction.read(cx).pane_size.unwrap())
                    .collect::<Vec<_>>()
            })
            .unwrap();
        for (host, size) in [(&left_host, measured[0]), (&right_host, measured[1])] {
            assert!(
                host.sizes.lock().unwrap().is_empty(),
                "covered prepaint pushes no size"
            );
            assert!(
                matches!(host.calls.lock().unwrap().as_slice(), [Request::terminal_take_back { cols, rows, .. }] if (*cols, *rows) == (size.cols, size.rows))
            );
        }
        assert_ne!(measured[0], measured[1]);
        left_host.calls.lock().unwrap().clear();
        right_host.calls.lock().unwrap().clear();
        let action = visual.debug_bounds("REMOTE_CONTROL_ACTION").unwrap();
        visual.simulate_click(action.center(), Default::default());
        for (host, size) in [(&left_host, measured[0]), (&right_host, measured[1])] {
            assert!(
                matches!(host.calls.lock().unwrap().as_slice(), [Request::terminal_take_back { cols, rows, .. }] if (*cols, *rows) == (size.cols, size.rows))
            );
        }
    }
}

#[cfg(debug_assertions)]
#[test]
fn cards_wrap_and_keep_action_padding_in_both_themes_and_languages() {
    use runner_app::i18n::{testing::with_locale, Locale};
    struct Preview {
        left: bool,
        count: usize,
    }
    impl Render for Preview {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().size_full().relative().child(card(
                "preview-take-back".into(),
                CardState {
                    name: "MacBook Air".into(),
                    left: self.left,
                    count: self.count,
                },
                GroupKind::Tab,
                Rc::new(|_, _| {}),
            ))
        }
    }
    let _theme = crate::theme_snapshot::ThemeGuard::new();
    for variant in [
        theme::ThemeVariant::Carbon,
        theme::ThemeVariant::RunnerLight,
    ] {
        theme::set_active_variant(variant);
        for locale in [Locale::English, Locale::SimplifiedChinese] {
            with_locale(locale, || {
                for (left, count) in [(false, 1), (false, 2), (true, 1), (true, 2)] {
                    let mut cx = TestAppContext::single();
                    let window = cx.add_window(|_, _| Preview { left, count });
                    let mut visual = VisualTestContext::from_window(window.into(), &cx);
                    let card = visual.debug_bounds("REMOTE_CONTROL_CARD").unwrap();
                    let title = visual.debug_bounds("REMOTE_CONTROL_TITLE").unwrap();
                    let detail = visual.debug_bounds("REMOTE_CONTROL_DETAIL").unwrap();
                    let action = visual.debug_bounds("REMOTE_CONTROL_ACTION").unwrap();
                    assert_eq!(card.size.width, gpui::px(400.));
                    assert_eq!(card.bottom() - action.bottom(), gpui::px(25.));
                    assert_eq!(title.size.width, detail.size.width);
                    assert!(detail.size.height >= gpui::px(20.));
                    if locale == Locale::English && count == 2 {
                        assert!(detail.size.height >= gpui::px(40.));
                    }
                    assert!(detail.bottom() < action.top());
                }
            });
        }
    }
}

#[test]
fn hidden_mission_slots_use_the_slot_pane_and_closed_drawers_keep_the_grid() {
    struct HiddenMission {
        chats: Vec<AttachedChat>,
    }
    impl Render for HiddenMission {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let style = super::super::element::TerminalStyle {
                palette: runner_terminal::palette::RUNNER,
                font: crate::app_settings::AppSettings::default()
                    .terminal_font_family
                    .font(),
                font_size: 14.,
                app_zoom: 1.,
            };
            let chats = self.chats.iter().collect::<Vec<_>>();
            let body = measure_panes(div().relative().flex_1().min_h(gpui::px(0.)), &chats, style);
            cover(
                div()
                    .relative()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(div().h(gpui::px(38.)).flex_none())
                    .child(body),
                "hidden-mission",
                &chats,
                GroupKind::Mission(38.),
            )
        }
    }
    let (host, terminal, _tx, _) = mirror(remote());
    let mut cx = TestAppContext::single();
    let window = cx.add_window(|_, cx| {
        let interaction = cx.new(|_| super::super::TerminalInteraction::new(terminal.clone()));
        let input = cx.new(|_| TerminalInput::new(terminal.clone()));
        HiddenMission {
            chats: vec![AttachedChat {
                _terminal_view: terminal.view(),
                terminal_scrollbar: cx.new(|_| Scrollbar::terminal(terminal.clone())),
                terminal,
                terminal_interaction: interaction,
                _terminal_input_subscription: cx.observe(&input, |_, _, _| {}),
                _terminal_focus_subscription: cx.observe(&input, |_, _, _| {}),
                terminal_input: input,
                terminal_focus: cx.focus_handle(),
            }],
        }
    });
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    let measured = window
        .read_with(&cx, |view, cx| {
            assert!(mission_covered(view.chats.iter().map(Some)));
            view.chats[0]
                .terminal_interaction
                .read(cx)
                .pane_size
                .unwrap()
        })
        .unwrap();
    assert!(measured.cols > 80 && measured.rows > 24);
    let action = visual.debug_bounds("REMOTE_CONTROL_ACTION").unwrap();
    visual.simulate_click(action.center(), Default::default());
    assert!(
        matches!(host.calls.lock().unwrap().as_slice(), [Request::terminal_take_back { cols, rows, .. }] if (*cols, *rows) == (measured.cols, measured.rows))
    );
    host.calls.lock().unwrap().clear();
    window
        .update(&mut cx, |view, _, cx| {
            view.chats[0]
                .terminal_interaction
                .update(cx, |interaction, _| interaction.pane_size = None);
            take_back(&[&view.chats[0]], cx);
        })
        .unwrap();
    assert!(matches!(
        host.calls.lock().unwrap().as_slice(),
        [Request::terminal_take_back {
            cols: 80,
            rows: 24,
            ..
        }]
    ));
    assert!(host.sizes.lock().unwrap().is_empty());
}

#[test]
fn resync_keeps_the_last_name_while_a_fresh_unowned_snapshot_has_none() {
    let (host, terminal, tx, ack) = mirror(remote());
    let (next_tx, next_rx) = mpsc::channel();
    *host.controller.lock().unwrap() = TerminalController::None;
    *host.stream.lock().unwrap() = Some((next_tx.clone(), next_rx));
    tx.send(TerminalFrame::Resync).unwrap();
    ack.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(terminal.control().controller, TerminalController::None);
    assert_eq!(
        terminal.control().last_remote_name.as_deref(),
        Some("MacBook Air")
    );
    let (_, fresh, _fresh_tx, _) = mirror(TerminalController::None);
    assert_eq!(fresh.control().last_remote_name, None);
    assert_eq!(
        card_state(&[fresh.control()]).unwrap().name,
        "Another computer"
    );
}

#[test]
fn split_tab_switches_between_group_and_pane_cards_and_resumes_desktop_pushes() {
    use runner_app::pane_layout::{PaneLayout, SplitOrientation, TabSet};
    let temp = tempfile::tempdir().unwrap();
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, _| {
        crate::app_store::seed_mixed_working_sessions(store)
    });
    let (left_host, left, left_tx, left_ack) = mirror_named("direct-agent", remote());
    let (right_host, right, right_tx, right_ack) = mirror_named("busy-shell", remote());
    let mut layout = PaneLayout::single(Some("direct-agent"), &[]);
    let second = layout.split("p1", SplitOrientation::Row).unwrap();
    layout.assign_session(&second, "busy-shell").unwrap();
    let row = store.read_with(&cx, |store, _| {
        store
            .client
            .node_tab_upsert(runner_core::protocol::NodeTabUpsertInput {
                id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
                parent_id: None,
                name: "Control test".into(),
                layout: layout.serialize().unwrap(),
            })
            .unwrap()
    });
    store.update(&mut cx, |store, _| store.nodes = vec![row.clone()]);
    let window = cx.add_window(|window, cx| {
        let mut root = NativeRoot::new(
            "control-test".into(),
            temp.path().join("logs"),
            None,
            None,
            store.clone(),
            window,
            cx,
        );
        root.route = crate::AppRoute::Chat;
        root.tabs = TabSet::from_rows(&[row]);
        root.attached
            .insert("direct-agent".into(), attached(left, cx, 101, 30));
        root.attached
            .insert("busy-shell".into(), attached(right, cx, 72, 45));
        root
    });
    let mut visual = VisualTestContext::from_window(window.into(), &cx);
    assert!(visual
        .debug_bounds("REMOTE_CONTROL_OVERLAY_take-back-tab")
        .is_some());
    assert!(left_host.sizes.lock().unwrap().is_empty());
    assert!(right_host.sizes.lock().unwrap().is_empty());
    change(&left_tx, &left_ack, 1, TerminalController::Desktop);
    window.update(&mut cx, |_, _, cx| cx.notify()).unwrap();
    cx.run_until_parked();
    assert!(visual
        .debug_bounds("REMOTE_CONTROL_OVERLAY_take-back-tab")
        .is_none());
    assert!(visual
        .debug_bounds("REMOTE_CONTROL_OVERLAY_take-back-busy-shell")
        .is_some());
    assert!(!left_host.sizes.lock().unwrap().is_empty());
    assert!(right_host.sizes.lock().unwrap().is_empty());
    change(&right_tx, &right_ack, 1, TerminalController::None);
    window.update(&mut cx, |_, _, cx| cx.notify()).unwrap();
    cx.run_until_parked();
    assert!(visual
        .debug_bounds("REMOTE_CONTROL_OVERLAY_take-back-busy-shell")
        .is_some());
    change(&right_tx, &right_ack, 2, TerminalController::Desktop);
    window.update(&mut cx, |_, _, cx| cx.notify()).unwrap();
    cx.run_until_parked();
    assert!(visual.debug_bounds("REMOTE_CONTROL_CARD").is_none());
    assert!(!right_host.sizes.lock().unwrap().is_empty());
}

#[test]
fn composition_started_before_takeover_clears_authoritative_drafting_on_desktop_ack() {
    use runner_terminal::input_state::{InputState, InputTracker};
    for resync in [false, true] {
        let (host, terminal, tx, ack) = mirror(TerminalController::Desktop);
        let now = std::time::Instant::now();
        let mut tracker = InputTracker::new(now);
        *host.input_observation.lock().unwrap() = Some(tracker.initial_observation(now));
        *host.input_tracker.lock().unwrap() = Some(tracker);
        let mut input = TerminalInput::new(terminal.clone());
        input.replace_and_mark_text(None, "preedit", None);
        let observation = host.input_observation.lock().unwrap().unwrap();
        assert!(observation.composing);
        assert_eq!(observation.state, InputState::Drafting);
        host.calls.lock().unwrap().clear();
        change(&tx, &ack, 1, remote());
        if resync {
            input.replace_and_mark_text(None, "blocked preedit update", None);
        } else {
            assert!(input.cancel_composition());
        }
        assert!(!input.is_composing());
        assert!(
            host.calls.lock().unwrap().is_empty(),
            "cancel stays local while covered"
        );
        change(&tx, &ack, 2, TerminalController::None);
        terminal.take_back(120, 40).unwrap();
        assert!(
            host.input_observation.lock().unwrap().unwrap().composing,
            "Take back must await Desktop acknowledgment"
        );
        change(&tx, &ack, 1, TerminalController::Desktop);
        assert!(
            host.input_observation.lock().unwrap().unwrap().composing,
            "stale acknowledgment cannot flush cancellation"
        );
        host.calls.lock().unwrap().clear();
        let (_next_tx, active_tx) = if resync {
            let (next_tx, next_rx) = mpsc::channel();
            *host.controller.lock().unwrap() = TerminalController::Desktop;
            *host.stream.lock().unwrap() = Some((next_tx.clone(), next_rx));
            tx.send(TerminalFrame::Resync).unwrap();
            ack.recv_timeout(Duration::from_secs(5)).unwrap();
            (Some(next_tx.clone()), next_tx)
        } else {
            change(&tx, &ack, 3, TerminalController::Desktop);
            (None, tx.clone())
        };
        let observation = host.input_observation.lock().unwrap().unwrap();
        assert!(
            !observation.composing,
            "authoritative preedit must clear on Desktop acknowledgment"
        );
        assert_eq!(
            observation.state,
            InputState::Idle,
            "cancelled preedit must release the Drafting delivery hold without a local commit"
        );
        assert!(matches!(
            host.calls.lock().unwrap().as_slice(),
            [Request::terminal_observe_input {
                input: runner_core::protocol::terminal::InputEvent::Composing { composing: false },
                ..
            }]
        ));
        assert!(host.inputs.lock().unwrap().is_empty());
        assert!(host.sizes.lock().unwrap().is_empty());
        input.replace_and_mark_text(None, "new preedit", None);
        change(&active_tx, &ack, 4, TerminalController::Desktop);
        assert!(
            host.input_observation.lock().unwrap().unwrap().composing,
            "the cancellation must not outlive handover and clear a new preedit"
        );
        input.cancel_composition();
        assert_eq!(
            host.input_observation.lock().unwrap().unwrap().state,
            InputState::Idle
        );
    }
}
