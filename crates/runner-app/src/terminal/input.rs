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
                Request::terminal_observe_input { .. } => {
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
}
