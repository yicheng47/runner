use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{div, Context, Render, Window};
use runner_app::ui::{Button, ButtonSize, PaneHeader, SettingsCard, SettingsRow};

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
            Err(error) => Some(crate::i18n::tf(
                "format.couldn_t_reveal_logs",
                &[("error", (error).to_string())],
            )),
        };
        cx.notify();
    }
}

impl Render for DiagnosticsPane {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(target_os = "macos")]
        let logs_label = crate::i18n::text("ui.reveal_logs_in_finder");
        #[cfg(any(windows, target_os = "linux"))]
        let logs_label = crate::i18n::text("ui.open_log_folder");
        let logs_description = crate::i18n::text(
            "ui.open_the_folder_containing_runner_log_so_you_can_attach_it_to_a_bug_report",
        )
        .to_owned();
        #[cfg(any(windows, target_os = "linux"))]
        let logs_description = runner_app::updater::global_updater(cx)
            .read(cx)
            .install_log_path()
            .map(|path| {
                crate::i18n::tf(
                    "format.installer_log",
                    &[
                        ("logs_description", (logs_description).to_string()),
                        ("path", (path.display()).to_string()),
                    ],
                )
            })
            .unwrap_or(logs_description);
        div()
            .flex()
            .flex_col()
            .gap_5()
            .child(PaneHeader::new(
                crate::i18n::text("ui.diagnostics"),
                crate::i18n::text("ui.logs_and_troubleshooting_tools"),
            ))
            .child(SettingsCard::new([SettingsRow::new(
                crate::i18n::text("ui.application_logs"),
                Button::new("diagnostics-reveal-logs", logs_label)
                    .icon("folder-open.svg")
                    .size(ButtonSize::Sm)
                    .on_press({
                        let pane = cx.entity();
                        move |_, cx| {
                            pane.update(cx, |pane, pane_cx| pane.reveal_logs(pane_cx));
                        }
                    }),
            )
            .subtitle(logs_description)
            .into_any_element()]))
            .children(
                self.error.clone().map(|error| {
                    runner_app::ui::notice_banner(error, runner_app::ui::Tone::Danger)
                }),
            )
    }
}
