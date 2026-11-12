use gpui::{App, ClipboardItem, Keystroke};
use runner_app::terminal_ime::{self, TerminalKeyRoute};
use runner_app::terminal_paste::{image_paste_shortcut, TerminalPaste};
use runner_core::protocol::ClientError;
use runner_core::Runtime;
use runner_terminal::terminal::TerminalMirror;

pub(crate) fn handle_key(
    terminal: &TerminalMirror,
    composing: bool,
    key: &Keystroke,
    option_is_meta: bool,
    cx: &mut App,
) -> Result<bool, ClientError> {
    if terminal_ime::swallows_option_copy(key, option_is_meta) {
        cx.stop_propagation();
        return Ok(false);
    }
    if terminal_ime::terminal_key_route(composing, key, option_is_meta) != TerminalKeyRoute::Raw {
        return Ok(false);
    }
    let sent = terminal.send_key(
        &key.key,
        key.modifiers.control,
        key.modifiers.alt,
        key.modifiers.shift,
        key.key_char.as_deref(),
    )?;
    if sent {
        terminal.scroll_to_bottom();
        cx.stop_propagation();
    }
    Ok(sent)
}

pub(crate) fn copy_selection(terminal: &TerminalMirror, cx: &mut App) -> bool {
    let Some(text) = terminal.selection_text() else {
        return false;
    };
    cx.write_to_clipboard(ClipboardItem::new_string(text));
    // Ctrl+C copies only while text is selected; clearing it lets the next press interrupt.
    #[cfg(any(windows, target_os = "linux"))]
    terminal.clear_selection();
    cx.stop_propagation();
    true
}

pub(crate) fn paste(
    terminal: &TerminalMirror,
    paste: &TerminalPaste,
    runtime: Option<Runtime>,
) -> Result<(), ClientError> {
    match paste {
        TerminalPaste::Image => terminal.write_user_bytes(image_paste_shortcut(runtime)),
        TerminalPaste::Text(text) => terminal.paste(text),
    }
}

pub(crate) fn paste_text_from_clipboard(
    terminal: &TerminalMirror,
    cx: &mut App,
) -> Result<(), ClientError> {
    if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
        paste(terminal, &TerminalPaste::Text(text), None)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::index::{Column, Line, Point, Side};
    use alacritty_terminal::selection::SelectionType;
    use gpui::TestAppContext;
    use runner_core::protocol::terminal::{
        TerminalAttachment, TerminalController, TerminalFrame, TerminalMetadata, TerminalSnapshot,
        TerminalSubscription,
    };
    use runner_core::protocol::{DaemonClient, EventSubscription, Request, Response, Transport};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Host {
        inputs: Mutex<Vec<Vec<u8>>>,
        observed: Mutex<Vec<runner_core::protocol::terminal::InputEvent>>,
        fail: AtomicBool,
    }
    struct Frames;
    impl TerminalSubscription for Frames {
        fn recv(&mut self) -> Result<TerminalFrame, ClientError> {
            Err(ClientError::msg("closed"))
        }
        fn cancellation(&self) -> Arc<dyn Fn() + Send + Sync> {
            Arc::new(|| {})
        }
    }
    impl Transport for Host {
        fn call(&self, request: Request) -> Result<Response, ClientError> {
            match request {
                Request::terminal_metadata { .. } => {
                    Ok(Response::terminal_metadata(Ok(TerminalMetadata::default())))
                }
                Request::terminal_observe_input { input, .. } => {
                    self.observed.lock().unwrap().push(input);
                    Ok(Response::terminal_observe_input(Ok(())))
                }
                _ => Err(ClientError::msg("unexpected request")),
            }
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            unreachable!()
        }
        fn attach(&self, _: &str) -> Result<TerminalAttachment, ClientError> {
            Ok(TerminalAttachment {
                subscriber_id: 1,
                snapshot: TerminalSnapshot {
                    seq: 0,
                    cols: 10,
                    rows: 2,
                    controller: TerminalController::Desktop,
                    bytes: b"first\r\nsecond\r\nthird\r\nfourth\x1b[?2004h".to_vec(),
                    unfinished_len: 0,
                    preceding_char: None,
                },
                frames: Box::new(Frames),
            })
        }
        fn input(&self, _: &str, bytes: &[u8]) -> Result<(), ClientError> {
            if self.fail.load(Ordering::Relaxed) {
                return Err(ClientError::msg("injected input failure"));
            }
            self.inputs.lock().unwrap().push(bytes.to_vec());
            Ok(())
        }
    }

    fn fixture() -> (Arc<Host>, Arc<TerminalMirror>) {
        let host = Arc::new(Host::default());
        let terminal = TerminalMirror::attach(
            DaemonClient::new(host.clone()),
            "shared-input".into(),
            Arc::new(|| {}),
        )
        .unwrap();
        (host, terminal)
    }

    struct ShortcutTerminal {
        terminal: Arc<TerminalMirror>,
        root_focus: gpui::FocusHandle,
        terminal_focus: gpui::FocusHandle,
        switches: usize,
    }

    impl gpui::Render for ShortcutTerminal {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            use gpui::prelude::*;
            gpui::div()
                .key_context("Root")
                .track_focus(&self.root_focus)
                .on_action(
                    cx.listener(|this, _: &crate::SwitchSidebarView, _, _| this.switches += 1),
                )
                .child(
                    gpui::div()
                        .key_context("Terminal")
                        .track_focus(&self.terminal_focus)
                        .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                            handle_key(&this.terminal, false, &event.keystroke, false, cx).unwrap();
                        })),
                )
        }
    }

    #[gpui::test]
    fn ctrl_period_routes_to_the_action_or_emits_the_negotiated_terminal_bytes(
        cx: &mut TestAppContext,
    ) {
        use crate::keymap::{self, KeymapOverrides, TerminalCtrlKeys};
        let (host, terminal) = fixture();
        let mut ctrl_period = keymap::entry("switch-sidebar-view")
            .unwrap()
            .default
            .clone();
        ctrl_period.meta = false;
        ctrl_period.ctrl = true;
        let overrides = KeymapOverrides::from([("switch-sidebar-view".into(), Some(ctrl_period))]);
        let window = cx.add_window(|_, cx| ShortcutTerminal {
            terminal: terminal.clone(),
            root_focus: cx.focus_handle(),
            terminal_focus: cx.focus_handle(),
            switches: 0,
        });
        for (seq, output, expected) in [
            (1, b"\x1b[>1u".as_slice(), b"\x1b[46;5u".as_slice()),
            (2, b"\x1b[<u".as_slice(), b".".as_slice()),
        ] {
            terminal.test_feed(seq, output);
            for mode in [
                TerminalCtrlKeys::RunnerFirst,
                TerminalCtrlKeys::TerminalFirst,
            ] {
                cx.update(|cx| keymap::install_bindings(cx, &overrides, mode, false));
                window
                    .update(cx, |view, window, cx| view.terminal_focus.focus(window, cx))
                    .unwrap();
                cx.run_until_parked();
                let switches = window.read_with(cx, |view, _| view.switches).unwrap();
                host.inputs.lock().unwrap().clear();
                host.observed.lock().unwrap().clear();
                cx.simulate_keystrokes(window.into(), "ctrl-.");
                cx.run_until_parked();
                window
                    .read_with(cx, |view, _| {
                        assert_eq!(
                            view.switches,
                            switches + usize::from(mode == TerminalCtrlKeys::RunnerFirst)
                        );
                    })
                    .unwrap();
                if mode == TerminalCtrlKeys::TerminalFirst {
                    assert_eq!(*host.inputs.lock().unwrap(), [expected.to_vec()]);
                    use runner_core::protocol::terminal::{InputEvent, InputKind};
                    use runner_terminal::{
                        input_state::{InputState, InputTracker},
                        replay,
                    };
                    use std::time::{Duration, Instant};
                    let expected_kind = if seq == 1 {
                        InputKind::Navigate
                    } else {
                        InputKind::Content { text: ".".into() }
                    };
                    let observed = host.observed.lock().unwrap();
                    assert_eq!(
                        *observed,
                        [InputEvent::Key {
                            kind: expected_kind
                        }]
                    );
                    let start = Instant::now();
                    let mut term = replay::replay_bytes(80, 24, b"> ");
                    let mut tracker = InputTracker::new(start);
                    tracker.initial_observation(start);
                    tracker.observe_input(&observed[0], start, &term);
                    replay::feed(&mut term, b".");
                    tracker.observe_output(start + Duration::from_millis(1), &term);
                    assert_eq!(
                        tracker.latest_observation().state,
                        if seq == 1 {
                            InputState::Idle
                        } else {
                            InputState::Drafting
                        }
                    );
                } else {
                    assert!(host.inputs.lock().unwrap().is_empty());
                    assert!(host.observed.lock().unwrap().is_empty());
                }
                host.inputs.lock().unwrap().clear();
                host.observed.lock().unwrap().clear();
                let switches = window.read_with(cx, |view, _| view.switches).unwrap();
                window
                    .update(cx, |view, window, cx| view.root_focus.focus(window, cx))
                    .unwrap();
                cx.simulate_keystrokes(window.into(), "ctrl-.");
                window
                    .read_with(cx, |view, _| assert_eq!(view.switches, switches + 1))
                    .unwrap();
                assert!(host.inputs.lock().unwrap().is_empty());
                assert!(host.observed.lock().unwrap().is_empty());
            }
        }
    }

    #[gpui::test]
    fn shared_key_handler_preserves_ime_meta_scroll_and_errors(cx: &mut TestAppContext) {
        let (host, terminal) = fixture();
        terminal.scroll_local(1);
        assert!(terminal.scroll_state().display_offset > 0);
        cx.update(|cx| {
            for (composing, mut key, meta) in [
                (true, Keystroke::parse("enter").unwrap(), true),
                (false, Keystroke::parse("a").unwrap(), true),
                (false, Keystroke::parse("alt-c").unwrap(), true),
                (false, Keystroke::parse("alt-b").unwrap(), false),
            ] {
                if key.key == "b" {
                    key.key_char = Some("∫".into());
                }
                assert!(!handle_key(&terminal, composing, &key, meta, cx).unwrap());
            }
            let mut shortcut = Keystroke::parse("c").unwrap();
            shortcut.modifiers.platform = true;
            assert!(!handle_key(&terminal, false, &shortcut, true, cx).unwrap());
            assert!(host.inputs.lock().unwrap().is_empty());
            assert!(terminal.scroll_state().display_offset > 0);
            assert!(handle_key(
                &terminal,
                false,
                &Keystroke::parse("alt-b").unwrap(),
                true,
                cx,
            )
            .unwrap());
            assert!(*host.inputs.lock().unwrap() == vec![b"\x1bb".to_vec()]);
            assert_eq!(terminal.scroll_state().display_offset, 0);
            terminal.scroll_local(1);
            host.fail.store(true, Ordering::Relaxed);
            let result = handle_key(
                &terminal,
                false,
                &Keystroke::parse("enter").unwrap(),
                true,
                cx,
            );
            assert!(result.is_err());
            assert!(terminal.scroll_state().display_offset > 0);
            assert_eq!(host.inputs.lock().unwrap().len(), 1);
        });
    }

    #[gpui::test]
    fn shared_copy_and_paste_keep_selection_text_brackets_and_image_shortcuts(
        cx: &mut TestAppContext,
    ) {
        let (host, terminal) = fixture();
        cx.update(|cx| {
            cx.write_to_clipboard(ClipboardItem::new_string("unchanged".into()));
            assert!(!copy_selection(&terminal, cx));
            assert!(cx.read_from_clipboard().unwrap().text().as_deref() == Some("unchanged"));
            terminal.start_selection(
                SelectionType::Simple,
                Point::new(Line(0), Column(0)),
                Side::Left,
            );
            terminal.update_selection(Point::new(Line(0), Column(4)), Side::Right);
            let selected = terminal.selection_text().unwrap();
            assert!(copy_selection(&terminal, cx));
            assert!(cx.read_from_clipboard().unwrap().text() == Some(selected.clone()));
            paste_text_from_clipboard(&terminal, cx).unwrap();
            assert!(
                host.inputs.lock().unwrap()[0]
                    == format!("\x1b[200~{selected}\x1b[201~").as_bytes()
            );
            assert!(terminal.selection_text().is_none());
            paste(&terminal, &TerminalPaste::Image, Some(Runtime::ClaudeCode)).unwrap();
            assert!(
                host.inputs.lock().unwrap()[1] == image_paste_shortcut(Some(Runtime::ClaudeCode))
            );
            host.fail.store(true, Ordering::Relaxed);
            assert!(paste_text_from_clipboard(&terminal, cx).is_err());
            assert!(paste(&terminal, &TerminalPaste::Image, None).is_err());
        });
    }

    #[gpui::test]
    fn copy_clears_the_selection_on_windows_and_linux_so_the_next_ctrl_c_interrupts(
        cx: &mut TestAppContext,
    ) {
        let (host, terminal) = fixture();
        cx.update(|cx| {
            terminal.start_selection(
                SelectionType::Simple,
                Point::new(Line(0), Column(0)),
                Side::Left,
            );
            terminal.update_selection(Point::new(Line(0), Column(4)), Side::Right);
            let selected = terminal.selection_text().unwrap();
            assert!(copy_selection(&terminal, cx));
            assert_eq!(
                cx.read_from_clipboard().unwrap().text(),
                Some(selected.clone())
            );
            if cfg!(any(windows, target_os = "linux")) {
                assert_eq!(terminal.selection_text(), None);
                assert!(!copy_selection(&terminal, cx));
                let ctrl_c = Keystroke::parse("ctrl-c").unwrap();
                assert!(handle_key(&terminal, false, &ctrl_c, false, cx).unwrap());
                assert_eq!(host.inputs.lock().unwrap().last().unwrap(), b"\x03");
            } else {
                assert_eq!(terminal.selection_text(), Some(selected));
                assert!(host.inputs.lock().unwrap().is_empty());
            }
        });
    }
}
