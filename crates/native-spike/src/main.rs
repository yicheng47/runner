//! Phase 1 spike (impl 0031): a native macOS window rendering a live
//! claude-code session — `alacritty_terminal` grid painted by GPUI,
//! plus an IME-capable composer. Run with:
//!
//! ```sh
//! cargo run -p native-spike -- claude
//! cargo run -p native-spike -- $SHELL   # or any other command
//! ```

mod composer;
mod terminal_element;
mod theme;

use std::sync::Arc;

use futures::StreamExt as _;
use gpui::{
    actions, div, prelude::*, px, size, App, Application, Bounds, Context, Entity, FocusHandle,
    KeyBinding, KeyDownEvent, Menu, MenuItem, MouseButton, ScrollDelta, ScrollWheelEvent,
    TitlebarOptions, Window, WindowBounds, WindowOptions,
};

use composer::Composer;
use native_spike::terminal::TerminalSession;
use terminal_element::TerminalElement;

actions!(spike, [Quit, TermPaste]);

struct SpikeRoot {
    terminal_focus: FocusHandle,
    session: Arc<TerminalSession>,
    composer: Entity<Composer>,
    scroll_accumulator: f32,
}

impl SpikeRoot {
    fn new(
        command: String,
        args: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let (wake_tx, mut wake_rx) = futures::channel::mpsc::unbounded::<()>();
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = wake_tx.unbounded_send(());
        });
        let session = TerminalSession::spawn(&command, &args, 100, 30, waker)
            .expect("spawn terminal session");

        cx.spawn(async move |weak, cx| {
            while wake_rx.next().await.is_some() {
                // Coalesce bursts: drain whatever queued up since.
                while wake_rx.try_recv().is_ok() {}
                if weak.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        })
        .detach();

        let terminal_focus = cx.focus_handle();
        terminal_focus.focus(window);
        let composer_focus = cx.focus_handle();
        let composer = cx.new(|_| Composer::new(composer_focus, Arc::clone(&session)));

        Self {
            terminal_focus,
            session,
            composer,
            scroll_accumulator: 0.,
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        if ks.modifiers.platform {
            return;
        }
        if self.session.send_key(
            &ks.key,
            ks.modifiers.control,
            ks.modifiers.alt,
            ks.key_char.as_deref(),
        ) {
            self.session.scroll_to_bottom();
            cx.notify();
        }
    }

    fn on_scroll(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let lines = match event.delta {
            ScrollDelta::Lines(p) => p.y,
            ScrollDelta::Pixels(p) => f32::from(p.y) / f32::from(window.line_height()),
        };
        self.scroll_accumulator += lines;
        let whole = self.scroll_accumulator.trunc() as i32;
        if whole != 0 {
            self.scroll_accumulator -= whole as f32;
            self.session.scroll(whole);
            cx.notify();
        }
    }

    fn on_paste(&mut self, _: &TermPaste, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.session.paste(&text);
        }
    }
}

impl Render for SpikeRoot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.terminal_focus.is_focused(window);
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::bg())
            .child(
                div()
                    .id("terminal")
                    .key_context("Terminal")
                    .track_focus(&self.terminal_focus)
                    .flex_1()
                    .min_h(px(0.))
                    .p_2()
                    .on_key_down(cx.listener(Self::on_key_down))
                    .on_scroll_wheel(cx.listener(Self::on_scroll))
                    .on_action(cx.listener(Self::on_paste))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, window, _| {
                            this.terminal_focus.focus(window);
                        }),
                    )
                    .child(TerminalElement::new(Arc::clone(&self.session), focused)),
            )
            .child(self.composer.clone())
    }
}

fn main() {
    let mut cli_args = std::env::args().skip(1);
    let command = cli_args.next().unwrap_or_else(|| "claude".to_string());
    let args: Vec<String> = cli_args.collect();

    Application::new().run(move |cx: &mut App| {
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-v", TermPaste, Some("Terminal")),
            KeyBinding::new("backspace", composer::Backspace, Some("Composer")),
            KeyBinding::new("delete", composer::Delete, Some("Composer")),
            KeyBinding::new("left", composer::Left, Some("Composer")),
            KeyBinding::new("right", composer::Right, Some("Composer")),
            KeyBinding::new("shift-left", composer::SelectLeft, Some("Composer")),
            KeyBinding::new("shift-right", composer::SelectRight, Some("Composer")),
            KeyBinding::new("cmd-a", composer::SelectAll, Some("Composer")),
            KeyBinding::new("home", composer::Home, Some("Composer")),
            KeyBinding::new("end", composer::End, Some("Composer")),
            KeyBinding::new("cmd-v", composer::Paste, Some("Composer")),
            KeyBinding::new("enter", composer::Submit, Some("Composer")),
            KeyBinding::new(
                "ctrl-cmd-space",
                composer::ShowCharacterPalette,
                Some("Composer"),
            ),
        ]);
        cx.set_menus(vec![Menu {
            name: "Runner Native Spike".into(),
            items: vec![MenuItem::action("Quit", Quit)],
        }]);

        let bounds = Bounds::centered(None, size(px(1000.), px(700.)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Runner Native Spike".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| SpikeRoot::new(command, args, window, cx)),
        )
        .unwrap();
        cx.activate(true);
    });
}
