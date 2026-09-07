use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;

use anyhow::Context as _;
use gpui::prelude::*;
use gpui::{
    actions, div, img, relative, rems, AnyElement, App, Context, Entity, FocusHandle, FontWeight,
    KeyDownEvent, MouseButton, Render, Subscription, Window,
};
use runner_app::ui::{Button, ButtonSize, ButtonVariant};
use runner_app::updater::{
    global_updater, windows_download_url, UpdateInfo, UpdateState, UpdateStep, Updater,
};

use crate::{assets::app_icon_source, theme, NativeRoot};

actions!(windows_updates, [OpenUpdateDialog]);

const DETACHED_PROCESS: u32 = 0x00000008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;

type CloseHandler = Rc<dyn Fn(&mut Window, &mut App)>;

pub(crate) struct UpdateDialog {
    updater: Entity<Updater>,
    log_dir: PathBuf,
    focus: FocusHandle,
    previous_focus: Option<FocusHandle>,
    close: CloseHandler,
    installing: bool,
    _subscription: Subscription,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DialogAction {
    Download,
    Cancel,
    Install,
    Retry,
}

struct DialogContent {
    title: String,
    subtitle: String,
    body: Option<String>,
    progress: Option<(u64, u64)>,
    secondary: Option<&'static str>,
    primary: Option<(&'static str, DialogAction)>,
}

fn dialog_content(
    state: &UpdateState,
    info: Option<&UpdateInfo>,
    size: u64,
) -> Option<DialogContent> {
    let version = info.map_or("", UpdateInfo::version);
    let installed = format!("Installed v{}", runner_app::version::display_version());
    let mut content = DialogContent {
        title: String::new(),
        subtitle: installed.clone(),
        body: None,
        progress: None,
        secondary: None,
        primary: None,
    };
    match state {
        UpdateState::UpToDate { .. } => return None,
        UpdateState::Available { sig_url, .. } => {
            content.title = format!("Runner {version} is available");
            if size > 0 {
                content.subtitle = format!("{installed} · {:.1} MB download", megabytes(size));
            }
            content.secondary = Some("View downloads");
            if sig_url.is_some() {
                content.body = Some(
                    "Download it now, or open the release page to get the installer yourself."
                        .into(),
                );
                content.primary = Some(("Download", DialogAction::Download));
            } else {
                content.body = Some("This release cannot be installed from Runner. Open the release page to download it.".into());
            }
        }
        UpdateState::Downloading { received, total } => {
            content.title = format!("Downloading Runner {version}");
            content.subtitle = format!("{installed} · verified before install");
            content.progress = Some((*received, if *total == 0 { size } else { *total }));
            content.primary = Some(("Cancel", DialogAction::Cancel));
        }
        UpdateState::Ready { .. } => {
            content.title = format!("Runner {version} is ready to install");
            content.subtitle = if size > 0 {
                format!(
                    "{installed} · {:.1} MB downloaded and verified",
                    megabytes(size)
                )
            } else {
                format!("{installed} · downloaded and verified")
            };
            content.body = Some(
                "Runner closes, the installer runs, and Runner reopens on the new version.".into(),
            );
            content.secondary = Some("Later");
            content.primary = Some(("Install and restart", DialogAction::Install));
        }
        UpdateState::Failed { step, message, .. } => {
            content.title = if info.is_some() {
                format!("Runner {version} couldn't be installed")
            } else {
                "Runner couldn't check for updates".into()
            };
            let step_name = match step {
                UpdateStep::Check => "Update check",
                UpdateStep::Download => "Download",
                UpdateStep::Verify => "Signature verification",
                UpdateStep::Install => "Installer",
            };
            content.body = Some(format!("{step_name} failed. {message}"));
            content.secondary = Some("View downloads");
            content.primary = Some(if *step == UpdateStep::Install {
                ("Install and restart", DialogAction::Install)
            } else {
                ("Retry", DialogAction::Retry)
            });
        }
    }
    Some(content)
}

fn megabytes(bytes: u64) -> f64 {
    bytes as f64 / 1_000_000.
}

fn progress_caption(received: u64, total: u64) -> String {
    if total == 0 {
        format!("{:.1} MB downloaded", megabytes(received))
    } else {
        format!(
            "{:.1} MB of {:.1} MB · {:.0}%",
            megabytes(received),
            megabytes(total),
            (received as f64 / total as f64 * 100.).min(100.)
        )
    }
}

fn installer_command(path: &Path, log: &Path, pid: u32) -> Command {
    let mut command = Command::new(path);
    command
        .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .args([
            "/SILENT".into(),
            "/NORESTART".into(),
            format!("/WAITPID={pid}"),
            "/RELAUNCH=1".into(),
            format!("/LOG={}", log.display()),
        ]);
    command
}

impl UpdateDialog {
    fn new(
        updater: Entity<Updater>,
        log_dir: PathBuf,
        close: CloseHandler,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let previous_focus = window.focused(cx);
        let focus = cx.focus_handle();
        focus.focus(window);
        let subscription = cx.observe(&updater, |_, _, cx| cx.notify());
        Self {
            updater,
            log_dir,
            focus,
            previous_focus,
            close,
            installing: false,
            _subscription: subscription,
        }
    }

    pub(crate) fn restore_focus(&self, window: &mut Window) {
        if let Some(focus) = &self.previous_focus {
            focus.focus(window);
        }
    }

    fn act(&mut self, action: DialogAction, cx: &mut Context<Self>) {
        match action {
            DialogAction::Download => self
                .updater
                .update(cx, |updater, cx| updater.download_update(cx)),
            DialogAction::Cancel => self
                .updater
                .update(cx, |updater, _| updater.cancel_download()),
            DialogAction::Retry => self.updater.update(cx, |updater, cx| {
                if matches!(
                    updater.state(),
                    UpdateState::Failed {
                        step: UpdateStep::Check,
                        ..
                    }
                ) {
                    updater.check_for_updates();
                } else {
                    updater.download_update(cx);
                }
            }),
            DialogAction::Install => {
                if self.installing {
                    return;
                }
                self.installing = true;
                let result = self
                    .updater
                    .update(cx, |updater, _| updater.prepare_install(&self.log_dir))
                    .and_then(|(path, log)| {
                        installer_command(&path, &log, std::process::id())
                            .spawn()
                            .context("Could not start the Windows installer")
                    });
                match result {
                    Ok(_) => cx.quit(),
                    Err(error) => {
                        self.installing = false;
                        self.updater
                            .update(cx, |updater, cx| updater.fail_install(error, cx));
                    }
                }
            }
        }
    }
}

impl Render for UpdateDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let updater = self.updater.read(cx);
        let Some(content) = dialog_content(
            updater.state(),
            updater.update_info(),
            updater.download_size(),
        ) else {
            return div().into_any_element();
        };
        let checking = updater.is_checking();
        self.render_content(content, checking, cx)
    }
}

impl UpdateDialog {
    fn render_content(
        &mut self,
        content: DialogContent,
        checking: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let close_backdrop = self.close.clone();
        let close_key = self.close.clone();
        let close_secondary = self.close.clone();
        let primary = cx.weak_entity();
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .p_4()
            .bg(gpui::rgba(0x00000099))
            .occlude()
            .track_focus(&self.focus)
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                close_backdrop(window, cx)
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    close_key(window, cx);
                }
            })
            .child(
                div()
                    .w_full()
                    .max_w(rems(420. / 16.))
                    .flex()
                    .flex_col()
                    .gap(rems(14. / 16.))
                    .rounded(rems(14. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .px(rems(22. / 16.))
                    .py(rems(20. / 16.))
                    .shadow_2xl()
                    .debug_selector(|| "UPDATE_DIALOG_PANEL".into())
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(rems(12. / 16.))
                            .child(
                                img(app_icon_source())
                                    .size(rems(36. / 16.))
                                    .flex_none()
                                    .rounded(rems(9. / 16.)),
                            )
                            .child(
                                div()
                                    .w(rems(328. / 16.))
                                    .flex()
                                    .flex_col()
                                    .gap(rems(2. / 16.))
                                    .child(
                                        div()
                                            .w_full()
                                            .whitespace_normal()
                                            .text_size(rems(14. / 16.))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(content.title),
                                    )
                                    .child(
                                        div()
                                            .w_full()
                                            .whitespace_normal()
                                            .text_size(rems(11. / 16.))
                                            .text_color(theme::muted())
                                            .child(content.subtitle),
                                    ),
                            ),
                    )
                    .children(content.body.map(|body| {
                        div()
                            .w(rems(376. / 16.))
                            .whitespace_normal()
                            .text_size(rems(12. / 16.))
                            .line_height(rems(18. / 16.))
                            .text_color(theme::muted())
                            .debug_selector(|| "UPDATE_DIALOG_BODY".into())
                            .child(body)
                    }))
                    .children(content.progress.map(|(received, total)| {
                        div()
                            .w(rems(376. / 16.))
                            .flex()
                            .flex_col()
                            .gap(rems(6. / 16.))
                            .child(
                                div()
                                    .w_full()
                                    .h(rems(4. / 16.))
                                    .rounded_full()
                                    .overflow_hidden()
                                    .bg(theme::raised())
                                    .child(
                                        div()
                                            .h_full()
                                            .w(relative(if total == 0 {
                                                0.
                                            } else {
                                                (received as f32 / total as f32).clamp(0., 1.)
                                            }))
                                            .bg(theme::accent()),
                                    ),
                            )
                            .child(
                                div()
                                    .font_family(theme::SYSTEM_MONOSPACE_FONT)
                                    .text_size(rems(11. / 16.))
                                    .text_color(theme::muted())
                                    .child(progress_caption(received, total)),
                            )
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .debug_selector(|| "UPDATE_DIALOG_FOOTER".into())
                            .children(content.secondary.map(|label| {
                                Button::new("update-secondary", label)
                                    .size(ButtonSize::Sm)
                                    .on_press(move |window, cx| {
                                        if label == "Later" {
                                            close_secondary(window, cx);
                                        } else {
                                            cx.open_url(windows_download_url());
                                        }
                                    })
                            }))
                            .children(content.primary.map(|(label, action)| {
                                Button::new("update-primary", label)
                                    .size(ButtonSize::Sm)
                                    .variant(if action == DialogAction::Cancel {
                                        ButtonVariant::Secondary
                                    } else {
                                        ButtonVariant::Primary
                                    })
                                    .loading(
                                        self.installing
                                            || (checking && action == DialogAction::Retry),
                                    )
                                    .on_press(move |_, cx| {
                                        let _ =
                                            primary.update(cx, |dialog, cx| dialog.act(action, cx));
                                    })
                            })),
                    ),
            )
            .into_any_element()
    }
}

impl NativeRoot {
    pub(crate) fn open_update_dialog(
        &mut self,
        _: &OpenUpdateDialog,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.update_dialog.is_some() {
            return;
        }
        let updater = global_updater(cx);
        if matches!(updater.read(cx).state(), UpdateState::UpToDate { .. }) {
            return;
        }
        let root = cx.weak_entity();
        let close = Rc::new(move |window: &mut Window, cx: &mut App| {
            let _ = root.update(cx, |root, cx| root.close_update_dialog(window, cx));
        });
        self.update_dialog =
            Some(cx.new(|cx| UpdateDialog::new(updater, self.log_dir.clone(), close, window, cx)));
        cx.notify();
    }

    pub(crate) fn close_update_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(dialog) = self.update_dialog.take() {
            dialog.read(cx).restore_focus(window);
        }
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct DialogTestView {
        dialog: Entity<UpdateDialog>,
    }

    impl Render for DialogTestView {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let info = UpdateInfo::new("0.8.2.20260908.0100");
            let content = dialog_content(
                &UpdateState::Failed {
                    step: UpdateStep::Install,
                    message: "The installer did not finish the update. Check the installer log in Settings → Diagnostics, then try again.".into(),
                    info: Some(info.clone()),
                },
                Some(&info),
                41_200_000,
            ).unwrap();
            div().size_full().child(
                self.dialog
                    .update(cx, |dialog, cx| dialog.render_content(content, false, cx)),
            )
        }
    }

    #[test]
    fn dialog_wraps_stamped_version_and_failure_body_inside_centered_card() {
        use gpui::{px, TestAppContext, VisualTestContext};
        for rem in [16., 20.8] {
            let mut cx = TestAppContext::single();
            let window = cx.add_window(|window, cx| {
                window.set_rem_size(px(rem));
                let updater = cx.new(|cx| Updater::new(false, PathBuf::new(), cx));
                DialogTestView {
                    dialog: cx.new(|cx| {
                        UpdateDialog::new(updater, PathBuf::new(), Rc::new(|_, _| {}), window, cx)
                    }),
                }
            });
            cx.run_until_parked();
            let mut window = VisualTestContext::from_window(window.into(), &cx);
            for size in [
                gpui::size(px(700.), px(500.)),
                gpui::size(px(1440.), px(900.)),
            ] {
                window.simulate_resize(size);
                cx.run_until_parked();
                let panel = window.debug_bounds("UPDATE_DIALOG_PANEL").unwrap();
                let body = window.debug_bounds("UPDATE_DIALOG_BODY").unwrap();
                let footer = window.debug_bounds("UPDATE_DIALOG_FOOTER").unwrap();
                assert!(body.size.height >= px(rem * 18. / 16. * 2.));
                assert!(footer.top() >= body.bottom() && footer.bottom() <= panel.bottom());
                assert!(panel.size.width <= px(rem * 420. / 16. + 1.));
                assert!(panel.size.height < size.height);
                assert!((panel.center().x - size.width / 2.).abs() <= px(1.));
                assert!((panel.center().y - size.height / 2.).abs() <= px(1.));
            }
        }
    }

    #[test]
    fn installer_arguments_preserve_paths_with_spaces() {
        let path =
            Path::new(r"C:\Users\Test User\updates\Runner-Setup-0.8.2.20260908.0100-x64.exe");
        let log = Path::new(r"C:\Users\Test User\logs\update-20260908.0100.log");
        let command = installer_command(path, log, 42);
        assert_eq!(command.get_program(), path.as_os_str());
        assert_eq!(
            command
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            [
                "/SILENT",
                "/NORESTART",
                "/WAITPID=42",
                "/RELAUNCH=1",
                r"/LOG=C:\Users\Test User\logs\update-20260908.0100.log"
            ]
        );
    }

    #[test]
    fn dialog_actions_follow_update_state_and_unknown_size_is_safe() {
        let info = UpdateInfo::new("0.8.2");
        assert!(dialog_content(&UpdateState::UpToDate { checking: false }, None, 0).is_none());
        let unsigned = UpdateState::Available {
            info: info.clone(),
            installer_url: "url".into(),
            sig_url: None,
        };
        let content = dialog_content(&unsigned, Some(&info), 0).unwrap();
        assert!(content.primary.is_none());
        assert_eq!(content.secondary, Some("View downloads"));
        let download = dialog_content(
            &UpdateState::Downloading {
                received: 18_400_000,
                total: 41_200_000,
            },
            Some(&info),
            0,
        )
        .unwrap();
        assert_eq!(download.primary, Some(("Cancel", DialogAction::Cancel)));
        assert_eq!(
            progress_caption(18_400_000, 41_200_000),
            "18.4 MB of 41.2 MB · 45%"
        );
        assert_eq!(progress_caption(18_400_000, 0), "18.4 MB downloaded");
        for step in [
            UpdateStep::Check,
            UpdateStep::Download,
            UpdateStep::Verify,
            UpdateStep::Install,
        ] {
            let state = UpdateState::Failed {
                step,
                message: "test".into(),
                info: Some(info.clone()),
            };
            let content = dialog_content(&state, Some(&info), 0).unwrap();
            assert_eq!(
                content.primary.unwrap().1,
                if step == UpdateStep::Install {
                    DialogAction::Install
                } else {
                    DialogAction::Retry
                }
            );
        }
    }
}
