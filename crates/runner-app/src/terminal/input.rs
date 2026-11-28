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
    let sent = if let Some(editing) = terminal_ime::macos_editing_key(key) {
        terminal.send_key_bytes(editing.bytes, editing.kind)?;
        true
    } else {
        terminal.send_key(
            &key.key,
            key.modifiers.control,
            key.modifiers.alt,
            key.modifiers.shift,
            key.key_char.as_deref(),
        )?
    };
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
    use gpui::TestAppContext;
    use runner_core::protocol::terminal::{
        TerminalAttachment, TerminalController, TerminalFrame, TerminalMetadata, TerminalSnapshot,
        TerminalSubscription,
    };
    use runner_core::protocol::{DaemonClient, EventSubscription, Request, Response, Transport};
    use runner_terminal::{Column, GridPoint as Point, Line, SelectionKind as SelectionType, Side};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Host {
        inputs: Mutex<Vec<Vec<u8>>>,
        observed: Mutex<Vec<runner_core::protocol::terminal::InputEvent>>,
        fail: AtomicBool,
        controller: Mutex<Option<TerminalController>>,
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
                    activity: None,
                    format: Default::default(),
                    unavailable: false,
                    cols: 10,
                    rows: 2,
                    controller: self
                        .controller
                        .lock()
                        .unwrap()
                        .clone()
                        .unwrap_or(TerminalController::Desktop),
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
        start_chat_focus: gpui::FocusHandle,
        switches: usize,
        maximizes: usize,
        confirmed_start_chat: usize,
    }

    impl ShortcutTerminal {
        fn new(terminal: Arc<TerminalMirror>, cx: &mut gpui::Context<Self>) -> Self {
            Self {
                terminal,
                root_focus: cx.focus_handle(),
                terminal_focus: cx.focus_handle(),
                start_chat_focus: cx.focus_handle(),
                switches: 0,
                maximizes: 0,
                confirmed_start_chat: 0,
            }
        }
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
                .on_action(
                    cx.listener(|this, _: &crate::ToggleMaximizePane, _, _| this.maximizes += 1),
                )
                .child(
                    gpui::div()
                        .key_context("Terminal")
                        .track_focus(&self.terminal_focus)
                        .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                            handle_key(&this.terminal, false, &event.keystroke, false, cx).unwrap();
                        })),
                )
                .child(
                    gpui::div()
                        .key_context("StartChat")
                        .track_focus(&self.start_chat_focus)
                        .on_action(cx.listener(|this, _: &crate::ConfirmStartChat, _, _| {
                            this.confirmed_start_chat += 1;
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
        let window = cx.add_window(|_, cx| ShortcutTerminal::new(terminal.clone(), cx));
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
                    tracker.observe_input(&observed[0], start, &term.screen());
                    replay::feed(&mut term, b".");
                    tracker.observe_output(start + Duration::from_millis(1), &term.screen());
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
            assert_eq!(
                cx.read_from_clipboard().unwrap().text().as_deref(),
                Some("unchanged")
            );
            assert!(host.inputs.lock().unwrap().is_empty());
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

            // Written interior spaces copy as `"   "`, whereas unwritten cells
            // past end of line and same-side zero-width drags return `false`
            // so Windows/Linux Ctrl+C falls through to `\x03`.
            terminal.test_feed(1, b"\x1b[2J\x1b[Habc   D");
            terminal.start_selection(
                SelectionType::Simple,
                Point::new(Line(0), Column(3)),
                Side::Left,
            );
            terminal.update_selection(Point::new(Line(0), Column(5)), Side::Right);
            cx.write_to_clipboard(ClipboardItem::new_string("before-spaces".into()));
            assert!(copy_selection(&terminal, cx));
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()),
                Some("   ".into()),
                "explicitly written interior spaces must copy as spaces"
            );

            terminal.start_selection(
                SelectionType::Simple,
                Point::new(Line(0), Column(8)),
                Side::Left,
            );
            terminal.update_selection(Point::new(Line(0), Column(12)), Side::Right);
            cx.write_to_clipboard(ClipboardItem::new_string("keep-unwritten".into()));
            assert!(
                !copy_selection(&terminal, cx),
                "unwritten cells past end of line must not consume Copy"
            );
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()),
                Some("keep-unwritten".into())
            );

            // Same-side zero-width drag during a DEC 2026 hold resolves to
            // `false` immediately without releasing the hold.
            terminal.start_selection(
                SelectionType::Simple,
                Point::new(Line(0), Column(1)),
                Side::Left,
            );
            terminal.test_feed(2, b"\x1b[?2026h");
            terminal.update_selection(Point::new(Line(0), Column(1)), Side::Left);
            cx.write_to_clipboard(ClipboardItem::new_string("keep-zw".into()));
            assert!(!copy_selection(&terminal, cx));
            assert!(terminal.is_render_held());
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()),
                Some("keep-zw".into())
            );
            terminal.test_feed(3, b"\x1b[?2026l");
        });
    }

    #[gpui::test]
    fn macos_editing_chords_send_control_bytes_or_yield_to_user_overrides(cx: &mut TestAppContext) {
        use crate::keymap::{self, KeymapOverrides, TerminalCtrlKeys};
        use runner_core::protocol::terminal::{InputEvent, InputKind};

        let (host, terminal) = fixture();
        let window = cx.add_window(|_, cx| ShortcutTerminal::new(terminal.clone(), cx));
        cx.update(|cx| {
            keymap::install_bindings(
                cx,
                &KeymapOverrides::new(),
                TerminalCtrlKeys::RunnerFirst,
                false,
            )
        });
        window
            .update(cx, |view, window, cx| view.terminal_focus.focus(window, cx))
            .unwrap();
        cx.run_until_parked();

        // Chords write the exact control bytes across primary, alt screen, and kitty modes.
        for (seq, output) in [
            (1, b"".as_slice()),
            (2, b"\x1b[?1049h\x1b[?1h\x1b[>1u".as_slice()),
        ] {
            if !output.is_empty() {
                terminal.test_feed(seq, output);
            }
            for (chord, expected_bytes, expected_kind) in [
                ("cmd-left", b"\x01".as_slice(), InputKind::Navigate),
                ("cmd-right", b"\x05".as_slice(), InputKind::Navigate),
                ("cmd-backspace", b"\x15".as_slice(), InputKind::Edit),
            ] {
                host.inputs.lock().unwrap().clear();
                host.observed.lock().unwrap().clear();
                terminal.scroll_local(1);
                terminal.start_selection(
                    SelectionType::Simple,
                    Point::new(Line(0), Column(0)),
                    Side::Left,
                );
                terminal.update_selection(Point::new(Line(0), Column(3)), Side::Right);

                cx.simulate_keystrokes(window.into(), chord);
                cx.run_until_parked();

                if cfg!(target_os = "macos") {
                    assert_eq!(
                        *host.inputs.lock().unwrap(),
                        [expected_bytes.to_vec()],
                        "{chord} seq={seq}"
                    );
                    assert_eq!(
                        *host.observed.lock().unwrap(),
                        [InputEvent::Key {
                            kind: expected_kind
                        }],
                        "{chord} seq={seq}"
                    );
                    assert_eq!(terminal.scroll_state().display_offset, 0, "{chord}");
                    assert!(terminal.selection_text().is_none(), "{chord}");
                } else {
                    assert!(host.inputs.lock().unwrap().is_empty(), "{chord}");
                    assert!(host.observed.lock().unwrap().is_empty(), "{chord}");
                    if seq == 1 {
                        assert!(terminal.scroll_state().display_offset > 0, "{chord}");
                    }
                }
            }
        }
        terminal.test_feed(3, b"\x1b[?1049l\x1b[?1l\x1b[<u");

        // Other cmd chords and modified variants still route to the app, not the terminal.
        let default_switch =
            keymap::binding_strings(&keymap::entry("switch-sidebar-view").unwrap().default)[0]
                .clone();
        for chord in [
            default_switch.as_str(),
            "cmd-.",
            "cmd-k",
            "shift-cmd-left",
            "alt-cmd-right",
            "ctrl-cmd-backspace",
        ] {
            host.inputs.lock().unwrap().clear();
            let switches = window.read_with(cx, |view, _| view.switches).unwrap();
            cx.simulate_keystrokes(window.into(), chord);
            cx.run_until_parked();
            assert!(host.inputs.lock().unwrap().is_empty(), "{chord}");
            if chord == default_switch {
                window
                    .read_with(cx, |view, _| assert_eq!(view.switches, switches + 1))
                    .unwrap();
            }
        }

        // A Runner shortcut rebound to any of these chords wins over the terminal key.
        for chord in ["cmd-left", "cmd-right", "cmd-backspace"] {
            let override_combo =
                keymap::combo_from_keystroke(&Keystroke::parse(chord).unwrap()).unwrap();
            let overrides =
                KeymapOverrides::from([("switch-sidebar-view".into(), Some(override_combo))]);
            cx.update(|cx| {
                keymap::install_bindings(cx, &overrides, TerminalCtrlKeys::RunnerFirst, false)
            });
            host.inputs.lock().unwrap().clear();
            host.observed.lock().unwrap().clear();
            let switches = window.read_with(cx, |view, _| view.switches).unwrap();
            cx.simulate_keystrokes(window.into(), chord);
            cx.run_until_parked();
            window
                .read_with(cx, |view, _| assert_eq!(view.switches, switches + 1))
                .unwrap();
            assert!(host.inputs.lock().unwrap().is_empty(), "{chord}");
            assert!(host.observed.lock().unwrap().is_empty(), "{chord}");
        }

        // Write errors propagate without scrolling, and non-desktop control suppresses writes.
        #[cfg(target_os = "macos")]
        {
            terminal.scroll_local(1);
            assert!(terminal.scroll_state().display_offset > 0);
            host.fail.store(true, Ordering::Relaxed);
            cx.update(|cx| {
                let err = handle_key(
                    &terminal,
                    false,
                    &Keystroke::parse("cmd-backspace").unwrap(),
                    false,
                    cx,
                );
                assert!(err.is_err());
                assert!(terminal.scroll_state().display_offset > 0);
            });

            let non_desktop_host = Arc::new(Host {
                controller: Mutex::new(Some(TerminalController::None)),
                ..Default::default()
            });
            let non_desktop = TerminalMirror::attach(
                DaemonClient::new(non_desktop_host.clone()),
                "shared-input-none".into(),
                Arc::new(|| {}),
            )
            .unwrap();
            cx.update(|cx| {
                for chord in ["cmd-left", "cmd-right", "cmd-backspace"] {
                    assert!(handle_key(
                        &non_desktop,
                        false,
                        &Keystroke::parse(chord).unwrap(),
                        false,
                        cx
                    )
                    .unwrap());
                }
            });
            assert!(non_desktop_host.inputs.lock().unwrap().is_empty());
            assert!(non_desktop_host.observed.lock().unwrap().is_empty());
        }
    }

    #[gpui::test]
    fn ctrl_enter_reaches_focused_terminal_without_stealing_runner_enter_bindings(
        cx: &mut TestAppContext,
    ) {
        use crate::keymap::{self, KeymapOverrides, TerminalCtrlKeys};
        use runner_core::protocol::terminal::{InputEvent, InputKind};

        let (host, terminal) = fixture();
        let window = cx.add_window(|_, cx| ShortcutTerminal::new(terminal.clone(), cx));
        cx.update(|cx| {
            keymap::install_bindings(
                cx,
                &KeymapOverrides::new(),
                TerminalCtrlKeys::RunnerFirst,
                false,
            )
        });

        // Focused terminal sends CSI 13;5u under Kitty disambiguate and LF in legacy mode,
        // without triggering StartChat's ConfirmStartChat binding.
        for (seq, output, expected) in [
            (1, b"\x1b[>5u".as_slice(), b"\x1b[13;5u".as_slice()),
            (2, b"\x1b[<u".as_slice(), b"\n".as_slice()),
        ] {
            terminal.test_feed(seq, output);
            window
                .update(cx, |view, window, cx| view.terminal_focus.focus(window, cx))
                .unwrap();
            cx.run_until_parked();
            host.inputs.lock().unwrap().clear();
            host.observed.lock().unwrap().clear();
            cx.simulate_keystrokes(window.into(), "ctrl-enter");
            cx.run_until_parked();
            assert_eq!(
                *host.inputs.lock().unwrap(),
                [expected.to_vec()],
                "seq={seq}"
            );
            assert_eq!(
                *host.observed.lock().unwrap(),
                [InputEvent::Key {
                    kind: InputKind::Navigate
                }],
                "seq={seq}"
            );
            window
                .read_with(cx, |view, _| assert_eq!(view.confirmed_start_chat, 0))
                .unwrap();
        }

        // ToggleMaximizePane's default chord still fires while a terminal has focus.
        let maximize_chord = keymap::platform_default("shift-cmd-enter");
        host.inputs.lock().unwrap().clear();
        cx.simulate_keystrokes(window.into(), &maximize_chord);
        cx.run_until_parked();
        window
            .read_with(cx, |view, _| assert_eq!(view.maximizes, 1))
            .unwrap();
        assert!(host.inputs.lock().unwrap().is_empty());

        // StartChat's ConfirmStartChat binding still fires in StartChat context.
        let confirm_chord = keymap::platform_default("cmd-enter");
        window
            .update(cx, |view, window, cx| {
                view.start_chat_focus.focus(window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        host.inputs.lock().unwrap().clear();
        cx.simulate_keystrokes(window.into(), &confirm_chord);
        cx.run_until_parked();
        window
            .read_with(cx, |view, _| assert_eq!(view.confirmed_start_chat, 1))
            .unwrap();
        assert!(host.inputs.lock().unwrap().is_empty());

        // A user shortcut explicitly rebound to ctrl-enter follows TerminalCtrlKeys.
        let ctrl_enter_combo =
            keymap::combo_from_keystroke(&Keystroke::parse("ctrl-enter").unwrap()).unwrap();
        let overrides =
            KeymapOverrides::from([("switch-sidebar-view".into(), Some(ctrl_enter_combo))]);
        terminal.test_feed(3, b"\x1b[>1u");
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
            cx.simulate_keystrokes(window.into(), "ctrl-enter");
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
                assert_eq!(*host.inputs.lock().unwrap(), [b"\x1b[13;5u".to_vec()]);
            } else {
                assert!(host.inputs.lock().unwrap().is_empty());
            }
        }
    }

    #[gpui::test]
    fn copy_during_dec_2026_reopens_returns_live_text_synchronously_and_preserves_paste_and_ctrl_c_order(
        cx: &mut TestAppContext,
    ) {
        use std::sync::mpsc;
        use std::time::{Duration, Instant};

        // 1. Copy outside a large write completes synchronously in microseconds
        //    both outside a hold and during an open DEC 2026 hold kept alive by
        //    four repeated DECSET ?2026 begins 65 ms apart, returning live grid
        //    text immediately while the hold and published frame stay frozen.
        let (_, terminal) = fixture();
        let _view = terminal.view();
        terminal.test_feed(1, b"\x1b[2J\x1b[Hhello 1234");
        terminal.start_selection(
            SelectionType::Simple,
            Point::new(Line(0), Column(0)),
            Side::Left,
        );
        terminal.update_selection(Point::new(Line(0), Column(4)), Side::Right);

        // Warm up once, then verify median synchronous Copy latency is sub-millisecond.
        cx.update(|cx| assert!(copy_selection(&terminal, cx)));
        let mut samples = Vec::with_capacity(21);
        for _ in 0..21 {
            terminal.start_selection(
                SelectionType::Simple,
                Point::new(Line(0), Column(0)),
                Side::Left,
            );
            terminal.update_selection(Point::new(Line(0), Column(4)), Side::Right);
            let elapsed = cx.update(|cx| {
                let started = Instant::now();
                assert!(copy_selection(&terminal, cx));
                started.elapsed()
            });
            samples.push(elapsed);
        }
        samples.sort();
        let median = samples[samples.len() / 2];
        assert!(
            median < Duration::from_millis(1),
            "synchronous Copy median unexpectedly slow outside a large write: {median:?} (samples={samples:?})"
        );

        // Four repeated DECSET ?2026 reopens 65 ms apart (260 ms total) while
        // live output overwrites the selected row: Copy returns immediately with
        // the live text without releasing the hold or updating the frozen frame.
        terminal.start_selection(
            SelectionType::Simple,
            Point::new(Line(0), Column(0)),
            Side::Left,
        );
        terminal.update_selection(Point::new(Line(0), Column(4)), Side::Right);
        terminal.test_feed(2, b"\x1b[?2026h\x1b[1;1HLIVE!\x1b[2;1Hmid-hold");
        assert!(terminal.is_render_held());
        assert_eq!(
            terminal.viewport_snapshot().view().row_to_string(Line(0)),
            "hello 1234",
            "published display must remain frozen on pre-hold frame"
        );

        let (reopens_done_tx, reopens_done_rx) = mpsc::channel::<()>();
        let (close_tx, close_rx) = mpsc::channel::<()>();
        let feeder_terminal = Arc::clone(&terminal);
        let feeder = std::thread::spawn(move || {
            for seq in 3..=6 {
                std::thread::sleep(Duration::from_millis(65));
                feeder_terminal.test_feed(seq, b"\x1b[?2026h");
            }
            let _ = reopens_done_tx.send(());
            let _ = close_rx.recv_timeout(Duration::from_secs(2));
            feeder_terminal.test_feed(7, b"\x1b[?2026l");
        });

        reopens_done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("all four 65 ms DEC 2026 reopens applied");
        assert!(
            terminal.is_render_held(),
            "hold must still be active after four 65 ms reopens"
        );
        let copy_elapsed = cx.update(|cx| {
            cx.write_to_clipboard(ClipboardItem::new_string("stale".into()));
            let started = Instant::now();
            assert!(copy_selection(&terminal, cx));
            let elapsed = started.elapsed();
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()),
                Some("LIVE!".into()),
                "synchronous Copy during hold must immediately return live grid text"
            );
            elapsed
        });
        assert!(
            copy_elapsed < Duration::from_millis(5),
            "Copy after 4x65 ms DEC 2026 reopens did not return immediately: {copy_elapsed:?}"
        );
        assert!(
            terminal.is_render_held(),
            "Copy must not release the active DEC 2026 hold"
        );
        assert_eq!(
            terminal.viewport_snapshot().view().row_to_string(Line(0)),
            "hello 1234",
            "published display must stay frozen after synchronous Copy"
        );
        let _ = close_tx.send(());
        feeder.join().unwrap();
        assert!(!terminal.is_render_held());

        // 2. Copy-then-Paste-then-Enter during a DEC 2026 hold (both same-pane
        //    and cross-pane) immediately puts the freshly copied text followed
        //    by Enter in the PTY in natural order, as on `main`.
        for cross_pane in [false, true] {
            let (src_host, src) = fixture();
            let (dst_host, dst) = if cross_pane {
                fixture()
            } else {
                (Arc::clone(&src_host), Arc::clone(&src))
            };
            src.test_feed(1, b"\x1b[2J\x1b[Hfresh\r\nworld");
            if cross_pane {
                dst.test_feed(1, b"\x1b[2J\x1b[Hhello\r\nworld");
            }
            src.start_selection(
                SelectionType::Simple,
                Point::new(Line(0), Column(0)),
                Side::Left,
            );
            src.update_selection(Point::new(Line(0), Column(4)), Side::Right);
            src.test_feed(2, b"\x1b[?2026h");
            assert!(src.is_render_held());

            cx.update(|cx| {
                cx.write_to_clipboard(ClipboardItem::new_string("old".into()));
                assert!(copy_selection(&src, cx));
                paste_text_from_clipboard(&dst, cx).unwrap();
                assert!(
                    handle_key(&dst, false, &Keystroke::parse("enter").unwrap(), false, cx,)
                        .unwrap()
                );
                assert_eq!(
                    cx.read_from_clipboard().and_then(|item| item.text()),
                    Some("fresh".into()),
                    "cross_pane={cross_pane}"
                );
            });
            assert_eq!(
                *dst_host.inputs.lock().unwrap(),
                vec![b"\x1b[200~fresh\x1b[201~".to_vec(), b"\r".to_vec()],
                "cross_pane={cross_pane}: Paste must read fresh clipboard ahead of Enter"
            );
            assert!(src.is_render_held());
            src.test_feed(3, b"\x1b[?2026l");
        }

        // 3. Ctrl+C ordering and newer-selection preservation during a hold
        //    match `main`: empty Ctrl+C sends `\x03` immediately ahead of Paste
        //    and Enter, and selecting `"world"` after Ctrl+C or Enter keeps
        //    `"world"` selected across hold release.
        for ctrl_empty in [true, false] {
            let (host, term) = fixture();
            term.test_feed(1, b"\x1b[2J\x1b[Hhello\r\nworld");
            if !ctrl_empty {
                term.start_selection(
                    SelectionType::Simple,
                    Point::new(Line(0), Column(0)),
                    Side::Left,
                );
                term.update_selection(Point::new(Line(0), Column(4)), Side::Right);
            }
            term.test_feed(2, b"\x1b[?2026h");
            cx.update(|cx| {
                cx.write_to_clipboard(ClipboardItem::new_string("old".into()));
                if !copy_selection(&term, cx) {
                    assert!(handle_key(
                        &term,
                        false,
                        &Keystroke::parse("ctrl-c").unwrap(),
                        false,
                        cx,
                    )
                    .unwrap());
                    paste_text_from_clipboard(&term, cx).unwrap();
                    assert!(handle_key(
                        &term,
                        false,
                        &Keystroke::parse("enter").unwrap(),
                        false,
                        cx,
                    )
                    .unwrap());
                } else {
                    assert!(handle_key(
                        &term,
                        false,
                        &Keystroke::parse("enter").unwrap(),
                        false,
                        cx,
                    )
                    .unwrap());
                }
            });
            term.start_selection(
                SelectionType::Simple,
                Point::new(Line(1), Column(0)),
                Side::Left,
            );
            term.update_selection(Point::new(Line(1), Column(4)), Side::Right);
            term.test_feed(3, b"\x1b[?2026l");
            let expected_inputs = if ctrl_empty {
                vec![
                    b"\x03".to_vec(),
                    b"\x1b[200~old\x1b[201~".to_vec(),
                    b"\r".to_vec(),
                ]
            } else {
                vec![b"\r".to_vec()]
            };
            assert_eq!(*host.inputs.lock().unwrap(), expected_inputs);
            assert_eq!(
                term.selection_text().as_deref(),
                Some("world"),
                "ctrl_empty={ctrl_empty}: selection created after Ctrl+C/Enter must survive release"
            );
        }
    }
}
