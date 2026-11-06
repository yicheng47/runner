use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{div, rems, svg, Context, CursorStyle, KeyDownEvent, Render, Window};
use runner_app::ui::{Button, ButtonSize, PaneHeader, SettingsCard, SettingsRow};

use crate::theme;

pub(super) const DISCORD_URL: &str = "https://discord.gg/6BrfZdHKNf";
pub(super) const WECHAT_README_URL: &str = "https://github.com/yicheng47/runner#community";

pub(crate) struct AboutPane {
    log_dir: PathBuf,
    error: Option<String>,
}

impl AboutPane {
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

impl Render for AboutPane {
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
                crate::i18n::text("ui.about"),
                crate::i18n::text("ui.version_credits_and_links"),
            ))
            .child(super::app_identity_card(
                [div()
                    .truncate()
                    .text_size(theme::text_body())
                    .text_color(theme::muted())
                    .child(crate::i18n::text("ui.local_cockpit_for_coding_agents"))
                    .into_any_element()],
                None,
                None,
            ))
            .child(SettingsCard::new([SettingsRow::new(
                crate::i18n::text("ui.application_logs"),
                Button::new("about-reveal-logs", logs_label)
                    .size(ButtonSize::Compact)
                    .debug_selector("ABOUT_REVEAL_LOGS")
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
            .child(SettingsCard::new([
                link_row(
                    "about-github",
                    "github.svg",
                    "GitHub",
                    Some("https://github.com/yicheng47/runner"),
                    None,
                ),
                link_row(
                    "about-documentation",
                    "book-text.svg",
                    crate::i18n::text("ui.documentation"),
                    Some("https://github.com/yicheng47/runner#readme"),
                    None,
                ),
                link_row(
                    "about-discord",
                    "discord.svg",
                    "Discord",
                    Some(DISCORD_URL),
                    None,
                ),
                link_row(
                    "about-wechat-readme",
                    "wechat.svg",
                    crate::i18n::text("community.wechat_readme"),
                    Some(WECHAT_README_URL),
                    None,
                ),
                link_row(
                    "about-license",
                    "scale.svg",
                    crate::i18n::text("ui.license"),
                    None,
                    Some("MIT"),
                ),
            ]))
            .child(
                div()
                    .debug_selector(|| "ABOUT_INVITE_GUIDANCE".into())
                    .text_size(theme::text_ui())
                    .text_color(theme::muted())
                    .child(crate::i18n::text("community.invite_guidance")),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_size(theme::text_meta())
                    .text_color(theme::faint())
                    .child("© 2026 Yicheng Wang (Jason Wang)"),
            )
    }
}

fn link_row(
    id: &'static str,
    icon: &'static str,
    label: &'static str,
    url: Option<&'static str>,
    trailing: Option<&'static str>,
) -> gpui::AnyElement {
    let interactive = url.is_some();
    let mut row = div()
        .id(id)
        .debug_selector(move || id.into())
        .tab_index(if interactive { 0 } else { -1 })
        .w_full()
        .flex()
        .items_center()
        .justify_between()
        .px_4()
        .py_3()
        .cursor(if interactive {
            CursorStyle::PointingHand
        } else {
            CursorStyle::Arrow
        })
        .when(interactive, |row| {
            row.hover(|row| row.bg(theme::with_alpha(theme::raised(), 0.4)))
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap(rems(10. / 16.))
                .child(
                    svg()
                        .flex_none()
                        .path(icon)
                        .size(rems(14. / 16.))
                        .text_color(theme::muted()),
                )
                .child(
                    div()
                        .text_size(theme::text_body())
                        .text_color(theme::text())
                        .child(label),
                ),
        )
        .child(if let Some(trailing) = trailing {
            div()
                .text_size(theme::text_ui())
                .text_color(theme::faint())
                .child(trailing)
        } else {
            div().child(
                svg()
                    .flex_none()
                    .path("external-link.svg")
                    .size(rems(12. / 16.))
                    .text_color(theme::faint()),
            )
        });
    if let Some(url) = url {
        row = row.on_click(move |_, _, cx| cx.open_url(url)).on_key_down(
            move |event: &KeyDownEvent, _, cx| {
                if matches!(event.keystroke.key.as_str(), "enter" | "space") {
                    cx.stop_propagation();
                    cx.open_url(url);
                }
            },
        );
    }
    row.into_any_element()
}
