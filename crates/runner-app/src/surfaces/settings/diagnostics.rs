use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{div, rems, Context, Render, Window};
use runner_app::ui::{Button, ButtonSize, PaneHeader, SettingsCard, SettingsRow};

use crate::theme;

pub(crate) struct DiagnosticsPane {
    log_dir: PathBuf,
    error: Option<String>,
}

impl DiagnosticsPane {
    pub(crate) fn new(log_dir: PathBuf) -> Self {
        Self {
            log_dir,
            error: None,
        }
    }

    fn reveal_logs(&mut self, cx: &mut Context<Self>) {
        self.error = match std::fs::create_dir_all(&self.log_dir) {
            Ok(()) => {
                cx.reveal_path(&self.log_dir);
                None
            }
            Err(error) => Some(format!("Couldn't reveal logs: {error}")),
        };
        cx.notify();
    }
}

impl Render for DiagnosticsPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(PaneHeader::new(
                "Diagnostics",
                "Logs and troubleshooting tools.",
            ))
            .child(SettingsCard::new([SettingsRow::new(
                "Application logs",
                Button::new("diagnostics-reveal-logs", "Reveal logs in Finder")
                    .icon("folder-open.svg")
                    .size(ButtonSize::Sm)
                    .on_press({
                        let pane = cx.entity();
                        move |_, cx| {
                            pane.update(cx, |pane, pane_cx| pane.reveal_logs(pane_cx));
                        }
                    }),
            )
            .subtitle("Open the folder containing runner.log so you can attach it to a bug report.")
            .into_any_element()]))
            .children(self.error.clone().map(|error| {
                div()
                    .rounded(rems(8. / 16.))
                    .border_1()
                    .border_color(theme::with_alpha(theme::danger(), 0.4))
                    .bg(theme::with_alpha(theme::danger(), 0.08))
                    .px_3()
                    .py_2()
                    .text_size(rems(12. / 16.))
                    .text_color(theme::danger())
                    .child(error)
            }))
    }
}
