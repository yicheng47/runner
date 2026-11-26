use crate::*;

#[cfg(target_os = "macos")]
fn bundle_provides_app_icon(
    bundle_path: &std::path::Path,
    resource_dir: Option<&std::path::Path>,
    icon_file: Option<&str>,
    icon_name: Option<&str>,
) -> bool {
    if !bundle_path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
    {
        return false;
    }
    let Some(resources) = resource_dir else {
        return false;
    };
    if icon_name
        .map(str::trim)
        .is_some_and(|name| !name.is_empty())
        && resources.join("Assets.car").is_file()
    {
        return true;
    }
    let Some(file) = icon_file.map(str::trim).filter(|file| !file.is_empty()) else {
        return false;
    };
    if resources.join(file).is_file() {
        return true;
    }
    std::path::Path::new(file).extension().is_none()
        && resources.join(format!("{file}.icns")).is_file()
}

/// Sets the Dock and About icon for unbundled dev binaries and iconless test
/// wrappers. Bundled apps leave the icon to AppKit/IconServices: calling
/// `setApplicationIconImage` with a raw PNG overrides `Runner.icns` and
/// bypasses macOS Tahoe's Liquid Glass framing, making the running Dock tile
/// look smaller and rimless.
#[cfg(target_os = "macos")]
#[cfg(not(test))]
pub(crate) fn install_app_icon() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::{ns_string, NSBundle, NSData, NSString};

    let bundle = NSBundle::mainBundle();
    let bundle_path = PathBuf::from(bundle.bundlePath().to_string());
    let resource_dir = bundle
        .resourcePath()
        .map(|path| PathBuf::from(path.to_string()));
    let info_string = |key: &NSString| {
        bundle
            .objectForInfoDictionaryKey(key)
            .and_then(|value| value.downcast::<NSString>().ok())
            .map(|value| value.to_string())
    };
    let icon_file = info_string(ns_string!("CFBundleIconFile"));
    let icon_name = info_string(ns_string!("CFBundleIconName"));
    if bundle_provides_app_icon(
        &bundle_path,
        resource_dir.as_deref(),
        icon_file.as_deref(),
        icon_name.as_deref(),
    ) {
        return;
    }

    let main_thread = MainThreadMarker::new().expect("Runner must start on the main thread");
    let data = NSData::with_bytes(include_bytes!("../../../assets/icon.png"));
    let image = NSImage::initWithData(NSImage::alloc(), &data)
        .expect("bundled Runner app icon must be valid");
    unsafe {
        NSApplication::sharedApplication(main_thread).setApplicationIconImage(Some(&image));
    }
}

/// Sparkle replaces the bundle in place, but Finder, Launchpad and the Dock keep
/// the previous version's icon cached until LaunchServices re-reads the bundle.
#[cfg(target_os = "macos")]
#[cfg(not(test))]
pub(crate) fn refresh_cached_app_icon() {
    use core_foundation::base::TCFType;
    use core_foundation::url::{CFURLRef, CFURL};
    use objc2_app_kit::NSWorkspace;
    use objc2_foundation::{NSBundle, NSString};

    #[link(name = "CoreServices", kind = "framework")]
    unsafe extern "C" {
        fn LSRegisterURL(url: CFURLRef, update: u8) -> i32;
    }

    let path = NSBundle::mainBundle().bundlePath().to_string();
    // A development binary is not a bundle, and a translocated copy runs from
    // a temporary path that should not be registered.
    if !path.ends_with(".app") || path.contains("/AppTranslocation/") {
        return;
    }
    std::thread::spawn(move || {
        objc2::rc::autoreleasepool(|_| {
            let Some(url) = CFURL::from_path(&path, true) else {
                return;
            };
            unsafe { LSRegisterURL(url.as_concrete_TypeRef(), 1) };
            NSWorkspace::sharedWorkspace().noteFileSystemChanged_(&NSString::from_str(&path));
        });
    });
}

/// An in-app update of a tarball install replaces `lib/runner/icon.png` behind
/// the desktop icon's symlink, which icon caches do not notice on their own.
/// Once per update, bumping the theme directory's mtime makes GTK rescan it,
/// and KDE, whose loader keeps returning cached pixmaps, reloads on its
/// session-bus `iconChanged` signal.
#[cfg(target_os = "linux")]
#[cfg(not(test))]
pub(crate) fn refresh_cached_app_icon() {
    use std::fs;
    use std::path::Path;
    use std::process::{Command, Stdio};

    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(package) = exe.parent() else {
        return;
    };
    let Some(prefix) = package.parent().and_then(Path::parent) else {
        return;
    };
    let theme = prefix.join("share/icons/hicolor");
    let modified = |path: &Path| fs::metadata(path).and_then(|meta| meta.modified()).ok();
    if fs::symlink_metadata(theme.join("256x256/apps/runner.png"))
        .is_ok_and(|meta| meta.file_type().is_symlink())
        && let (Some(icon), Some(theme_modified)) =
            (modified(&package.join("icon.png")), modified(&theme))
        && icon > theme_modified
        && fs::File::open(&theme)
            .and_then(|dir| dir.set_modified(std::time::SystemTime::now()))
            .is_ok()
    {
        std::thread::spawn(|| {
            let _ = Command::new("dbus-send")
                .args([
                    "--session",
                    "--type=signal",
                    "/KIconLoader",
                    "org.kde.KIconLoader.iconChanged",
                    "int32:0",
                ])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        });
    }
}

#[cfg(not(test))]
pub(crate) fn run() -> Result<()> {
    let paths = runner_app::bootstrap::native_paths()?;
    if let Err(error) = runner_app::logging::install(&paths.log_dir) {
        eprintln!("Runner file logging unavailable: {error:#}");
    }
    runner_app::logging::startup_banner(
        &runner_app::version::display_version(),
        &paths.app_data_dir,
    );
    #[cfg(windows)]
    let relaunches = match single_instance::claim(&runner_core::app_paths::app_identifier(cfg!(
        debug_assertions
    ))) {
        single_instance::Instance::Primary(relaunches) => relaunches,
        single_instance::Instance::Secondary => return Ok(()),
    };
    let (core, connection) = runner_app::bootstrap::connect(&paths)?;
    print_startup_paths(&paths);
    let ui_settings_path = settings_path(&paths.app_data_dir);

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    let application = gpui_platform::application();
    #[cfg(windows)]
    let application =
        gpui::Application::with_platform(Rc::new(gpui_windows::WindowsPlatform::new(false)?));
    let application = application.with_assets(Assets);
    #[cfg(target_os = "macos")]
    let application = application.with_quit_mode(QuitMode::Explicit);
    application.on_reopen(handle_reopen);
    let stall_watchdog: Arc<std::sync::Mutex<Option<stall_watchdog::StallWatchdogGuard>>> =
        Arc::new(std::sync::Mutex::new(None));
    let startup_watchdog = Arc::clone(&stall_watchdog);
    application.run(move |cx: &mut App| {
        cx.set_app_identity(
            &runner_core::app_paths::app_identifier(cfg!(debug_assertions)),
            "Runner",
        );
        runner_app::appearance::install(cx);
        cx.text_system()
            .add_fonts(
                INTER_FONTS
                    .into_iter()
                    .chain(JETBRAINS_MONO_FONTS)
                    .map(Cow::Borrowed)
                    .collect(),
            )
            .expect("bundled fonts must be valid");
        #[cfg(target_os = "macos")]
        crate::terminal::fonts::register_bundled_font_for_fallback();

        #[cfg(target_os = "macos")]
        install_app_icon();
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        refresh_cached_app_icon();

        // Installed here, rather than in `boot_core`, so AppKit
        // registration happens on GPUI's process main thread.
        #[cfg(target_os = "macos")]
        runner_app::bootstrap::install_wake(core.client.clone());

        let (settings, settings_error) = match AppSettings::load(&ui_settings_path) {
            Ok(settings) => (settings, None),
            Err(error) => (AppSettings::default(), Some(error.to_string())),
        };
        i18n::set_active_locale(
            settings
                .language
                .resolve(sys_locale::get_locale().as_deref()),
        );
        #[cfg(not(test))]
        let home_dir = paths.home_dir.clone();
        #[cfg(test)]
        let home_dir = None;
        #[cfg(all(not(test), target_os = "macos"))]
        let command_install_support = Some(CommandInstallSupport::system(PathBuf::from(
            "/usr/local/bin",
        )));
        #[cfg(all(not(test), target_os = "linux"))]
        let command_install_support =
            Some(CommandInstallSupport::system(PathBuf::from("/usr/bin")));
        #[cfg(all(not(test), windows))]
        let command_install_support = Some(CommandInstallSupport::system(PathBuf::new()));
        #[cfg(test)]
        let command_install_support = None;
        let app_store = cx.new(|cx| {
            AppStore::new(
                core.clone(),
                home_dir,
                command_install_support,
                ui_settings_path.clone(),
                settings,
                settings_error,
                cx,
            )
        });
        let keymap_overrides = app_store.read(cx).settings.keymap_overrides.clone();
        let terminal_ctrl_keys = app_store.read(cx).settings.terminal_ctrl_keys();
        #[cfg(target_os = "macos")]
        let automatically_check_for_updates =
            app_store.read(cx).settings.automatically_check_for_updates;
        #[cfg(any(windows, target_os = "linux"))]
        let automatically_download_updates =
            app_store.read(cx).settings.automatically_download_updates;
        let skipped_update_version = app_store.read(cx).settings.skipped_update_version.clone();
        keymap::install_bindings(cx, &keymap_overrides, terminal_ctrl_keys, false);
        cx.set_global(GlobalAppStore(app_store));
        notifications::install(cx);
        crate::terminal::clipboard::install(cx);
        cx.set_global(GlobalNativePaths(paths.clone()));
        #[cfg(target_os = "macos")]
        let updater = cx.new(|cx| Updater::new(automatically_check_for_updates, cx));
        #[cfg(any(windows, target_os = "linux"))]
        let updater = cx.new(|cx| {
            Updater::new(
                automatically_download_updates,
                paths.app_data_dir.join("updates"),
                cx,
            )
        });
        updater.update(cx, |updater, _| {
            updater.set_skipped_version(skipped_update_version)
        });
        cx.set_global(GlobalUpdater(updater.clone()));
        updater.read(cx).start();
        cx.set_global(WindowLayoutCheckpoint::default());
        cx.set_global(runner_app::lifecycle::QuitState::default());
        *startup_watchdog.lock().unwrap() = Some(
            stall_watchdog::StallWatchdogGuard::start_for_app(cx.foreground_executor().clone()),
        );

        // App::shutdown clears its windows before polling the returned future, so the
        // checkpoint must run in this callback body rather than inside the future.
        let stop_connection = connection.clone();
        cx.set_global(surfaces::quit_dialog::SessionStopper(Arc::new(move || {
            stop_connection.shutdown()
        })));
        let quit_connection = connection.clone();
        let quit_watchdog = Arc::clone(&startup_watchdog);
        #[cfg(windows)]
        let quit_relaunches = relaunches.clone();
        cx.on_app_quit(move |cx| {
            if let Some(mut guard) = quit_watchdog.lock().unwrap().take() {
                guard.stop();
            }
            #[cfg(windows)]
            quit_relaunches.close();
            let dismissals = notifications::dismiss_all(cx);
            save_window_settings(cx);
            checkpoint_window_layout_on_quit(cx);
            let stop = cx
                .global::<runner_app::lifecycle::QuitState>()
                .stop_sessions(runner_app::lifecycle::update_quit_pending());
            if stop {
                if let Err(error) = quit_connection.shutdown() {
                    eprintln!("Runner quit session teardown failed: {error:#}");
                }
            } else {
                quit_connection.disconnect();
            }
            dismissals
        })
        .detach();

        cx.on_action(|_: &Quit, cx| {
            cx.defer(|cx| {
                if let Some(window) = cx
                    .active_window()
                    .and_then(|window| window.downcast::<NativeRoot>())
                {
                    let _ = window.update(cx, |this, window, cx| this.request_quit(window, cx));
                } else {
                    let store = global_app_store(cx);
                    let state = store.read(cx);
                    let choice = runner_app::lifecycle::quit_choice(
                        runner_app::lifecycle::QuitRequest::User,
                        state.settings.quit_behavior,
                        state.live_session_count,
                    );
                    if let Some(choice) = choice {
                        surfaces::quit_dialog::finish_quit(choice, cx);
                    } else {
                        handle_reopen(cx);
                        if let Some(window) = cx
                            .active_window()
                            .and_then(|window| window.downcast::<NativeRoot>())
                        {
                            let _ =
                                window.update(cx, |this, window, cx| this.request_quit(window, cx));
                        }
                    }
                }
            });
        });
        #[cfg(target_os = "macos")]
        cx.on_action(|_: &QuitAndStopSessions, cx| {
            cx.defer(|cx| {
                if cx.global::<runner_app::lifecycle::QuitState>().stopping {
                    return;
                }
                let live = global_app_store(cx).read(cx).live_session_count;
                let shown = live > 0
                    && cx
                        .active_window()
                        .and_then(|window| window.downcast::<NativeRoot>())
                        .and_then(|window| {
                            window
                                .update(cx, |this, window, cx| {
                                    let summary = runner_app::lifecycle::SessionSummary {
                                        live,
                                        working: Vec::new(),
                                    };
                                    this.show_stopping_quit(summary, window, cx)
                                })
                                .ok()
                        })
                        .unwrap_or(false);
                if !shown {
                    surfaces::quit_dialog::finish_quit(runner_app::lifecycle::QuitChoice::Stop, cx);
                }
            });
        });
        #[cfg(target_os = "macos")]
        cx.on_action(|_: &Hide, cx| cx.hide());
        #[cfg(target_os = "macos")]
        cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
        #[cfg(target_os = "macos")]
        cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
        // Global action listeners run while GPUI still holds the active
        // window's update lease (both the key-binding and the menu-item
        // paths dispatch through `active_window.update`), so a nested
        // `window.update` here fails silently. Defer the work one tick.
        cx.on_action(|_: &Minimize, cx| {
            if let Some(window) = cx.active_window() {
                cx.defer(move |cx| {
                    if let Err(error) = window.update(cx, |_, window, _| window.minimize_window()) {
                        eprintln!("Runner minimize failed: {error:#}");
                    }
                });
            }
        });
        cx.on_action(|_: &CloseTab, cx| {
            if let Some(window) = cx
                .active_window()
                .and_then(|window| window.downcast::<NativeRoot>())
            {
                cx.defer(move |cx| {
                    if let Err(error) = window.update(cx, close_tab) {
                        eprintln!("Runner close tab failed: {error:#}");
                    }
                });
            }
        });
        cx.on_action(|_: &CloseWindow, cx| {
            if let Some(window) = cx
                .active_window()
                .and_then(|window| window.downcast::<NativeRoot>())
            {
                cx.defer(move |cx| {
                    if let Err(error) = window.update(cx, close_window) {
                        eprintln!("Runner close window failed: {error:#}");
                    }
                });
            }
        });
        cx.on_action(|_: &NewWindow, cx| {
            cx.defer(|cx| {
                if let Err(error) = open_new_runner_window(None, cx) {
                    eprintln!("Runner new window failed: {error:#}");
                }
                cx.activate(true);
            });
        });
        #[cfg(target_os = "macos")]
        cx.on_action(|_: &AboutRunner, _| {
            platform_ui::show_about_panel();
        });
        cx.on_action(|_: &CheckForUpdates, cx| {
            let active = cx.active_window();
            cx.defer(move |cx| {
                let native_window = |cx: &App| {
                    cx.windows()
                        .into_iter()
                        .find_map(|window| window.downcast::<NativeRoot>())
                };
                let mut window = active
                    .and_then(|window| window.downcast::<NativeRoot>())
                    .or_else(|| native_window(cx));
                if window.is_none() {
                    if let Err(error) = open_new_runner_window(None, cx) {
                        eprintln!("Runner check for updates failed: {error:#}");
                        return;
                    }
                    window = native_window(cx);
                }
                let Some(window) = window else {
                    return;
                };
                cx.activate(true);
                let result = window.update(cx, |root, window, cx| {
                    window.activate_window();
                    crate::surfaces::update_ui::check_for_updates_from_menu(root, window, cx);
                });
                if let Err(error) = result {
                    eprintln!("Runner check for updates failed: {error:#}");
                }
            });
        });
        cx.set_menus(app_menus());

        let restored_layout = window_state::read_layout(&paths.app_data_dir);
        for warning in &restored_layout.warnings {
            eprintln!("Runner window-layout: restore fallback: {warning}");
        }
        let mut restored_labels = Vec::new();
        for restored in window_state::restore_order(restored_layout.layout) {
            let result = match restored {
                window_state::RestoredWindowState::Main(state) => {
                    open_runner_window("main".into(), state.route, None, cx)
                }
                window_state::RestoredWindowState::Secondary(state) => open_runner_window(
                    runner_core::protocol::window::allocate_label(),
                    state.route,
                    Some(state.frame),
                    cx,
                ),
            };
            match result {
                Ok(label) => restored_labels.push(label),
                Err(error) => eprintln!("Runner window-layout restore failed: {error:#}"),
            }
        }
        if restored_labels.is_empty() {
            match open_runner_window("main".into(), None, None, cx) {
                Ok(label) => restored_labels.push(label),
                Err(error) => {
                    eprintln!("Runner could not open a window: {error:#}");
                    // A normal quit would save this windowless layout over the saved one.
                    std::process::exit(1);
                }
            }
        }
        if let Some(label) = restored_labels.last() {
            focus_other_window(label, cx);
        }
        let (_, restored_windows) = collect_window_layout(cx);
        eprintln!(
            "Runner window-layout: restored count={} {}",
            restored_windows.len(),
            format_window_layout_details(&restored_windows),
        );
        cx.activate(true);
        #[cfg(windows)]
        cx.spawn(async move |cx| {
            while let Some(handoff) = relaunches.next().await {
                cx.update(|cx| {
                    // Refused during a committed quit, so the launch starts once this one exits.
                    if quit_committed(cx) {
                        return;
                    }
                    handle_reopen(cx);
                    focus_other_window("main", cx);
                    handoff.accept();
                });
            }
        })
        .detach();
    });
    if let Some(mut guard) = stall_watchdog.lock().unwrap().take() {
        guard.stop();
    }

    Ok(())
}

/// Rebuilt whenever key bindings change: the macOS menu bar reads its
/// shortcuts from the bindings when it is set, and a stale one still fires.
pub(crate) fn app_menus() -> Vec<Menu> {
    vec![
        Menu {
            name: "Runner".into(),
            disabled: false,
            items: vec![
                #[cfg(target_os = "macos")]
                MenuItem::action(crate::i18n::text("ui.about_runner"), AboutRunner),
                #[cfg(not(target_os = "linux"))]
                MenuItem::action(crate::i18n::text("ui.check_for_updates"), CheckForUpdates),
                MenuItem::separator(),
                #[cfg(not(target_os = "linux"))]
                MenuItem::os_submenu(crate::i18n::text("ui.services"), SystemMenuType::Services),
                MenuItem::separator(),
                #[cfg(not(target_os = "linux"))]
                MenuItem::action(crate::i18n::text("ui.hide_runner"), Hide),
                #[cfg(not(target_os = "linux"))]
                MenuItem::action(crate::i18n::text("ui.hide_others"), HideOthers),
                #[cfg(not(target_os = "linux"))]
                MenuItem::action(crate::i18n::text("ui.show_all"), ShowAll),
                MenuItem::separator(),
                MenuItem::action(crate::i18n::text("ui.quit_runner"), Quit),
                #[cfg(target_os = "macos")]
                MenuItem::action(
                    crate::i18n::text("ui.quit_and_stop_sessions"),
                    QuitAndStopSessions,
                ),
            ],
        },
        Menu {
            name: crate::i18n::text("ui.file").into(),
            disabled: false,
            items: vec![MenuItem::action(
                crate::i18n::text("ui.new_window"),
                NewWindow,
            )],
        },
        Menu {
            name: crate::i18n::text("ui.edit").into(),
            disabled: false,
            items: vec![
                MenuItem::os_action(crate::i18n::text("ui.undo"), Undo, OsAction::Undo),
                MenuItem::os_action(crate::i18n::text("ui.redo"), Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action(crate::i18n::text("ui.cut"), Cut, OsAction::Cut),
                MenuItem::os_action(crate::i18n::text("ui.copy"), Copy, OsAction::Copy),
                MenuItem::os_action(crate::i18n::text("ui.paste"), Paste, OsAction::Paste),
                MenuItem::os_action(
                    crate::i18n::text("ui.select_all"),
                    SelectAll,
                    OsAction::SelectAll,
                ),
            ],
        },
        Menu {
            name: crate::i18n::text("ui.view").into(),
            disabled: false,
            items: vec![MenuItem::action(
                crate::i18n::text("ui.enter_full_screen"),
                ToggleFullscreen,
            )],
        },
        Menu {
            name: crate::i18n::text("ui.window").into(),
            disabled: false,
            items: vec![
                MenuItem::action(crate::i18n::text("ui.minimize"), Minimize),
                MenuItem::separator(),
                MenuItem::action(crate::i18n::text("ui.close_tab"), CloseTab),
                MenuItem::action(crate::i18n::text("ui.close_window"), CloseWindow),
            ],
        },
    ]
}

#[cfg(all(windows, not(test)))]
pub(crate) fn quit_committed(cx: &App) -> bool {
    runner_app::lifecycle::update_quit_pending()
        || cx
            .try_global::<runner_app::lifecycle::QuitState>()
            .is_some_and(|quit| quit.committed || quit.stopping)
}

#[cfg(not(test))]
pub(crate) fn print_startup_paths(paths: &NativePaths) {
    eprintln!(
        "Runner: database={} logs={}",
        paths.app_data_dir.join("runner.db").display(),
        paths.log_dir.display()
    );
}

#[cfg(not(test))]
pub(crate) fn main() {
    #[cfg(target_os = "linux")]
    if let Some(result) = runner_app::updater::linux::run_installer() {
        if let Err(error) = result {
            eprintln!("Runner update failed: {error:#}");
            std::process::exit(1);
        }
        return;
    }
    if let Err(error) = run() {
        let message = format!("{error:#}");
        runner_app::logging::startup_failure(&message);
        eprintln!("Runner failed: {message}");
        #[cfg(windows)]
        show_startup_error(&message);
        std::process::exit(1);
    }
    #[cfg(target_os = "linux")]
    if let Err(error) = runner_app::updater::linux::relaunch() {
        eprintln!("Runner relaunch failed: {error:#}");
    }
}

/// Release builds on Windows have no console, so a failed start is invisible without a dialog.
#[cfg(all(windows, not(test)))]
pub(crate) fn show_startup_error(message: &str) {
    use windows::core::HSTRING;
    use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
    let text = HSTRING::from(crate::i18n::tf(
        "startup.failed",
        &[("message", message.to_string())],
    ));
    unsafe { MessageBoxW(None, &text, &HSTRING::from("Runner"), MB_OK | MB_ICONERROR) };
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use std::fs;

    use super::bundle_provides_app_icon;

    #[test]
    fn bundled_app_with_icns_skips_runtime_icon_override() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Runner.app");
        let resources = app.join("Contents/Resources");
        fs::create_dir_all(&resources).unwrap();
        fs::write(resources.join("Runner.icns"), b"icns").unwrap();

        assert!(bundle_provides_app_icon(
            &app,
            Some(&resources),
            Some("Runner.icns"),
            None,
        ));
        assert!(bundle_provides_app_icon(
            &app,
            Some(&resources),
            Some("Runner"),
            None,
        ));
    }

    #[test]
    fn bundled_app_with_asset_catalog_icon_skips_runtime_icon_override() {
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("Runner.app");
        let resources = app.join("Contents/Resources");
        fs::create_dir_all(&resources).unwrap();
        fs::write(resources.join("Assets.car"), b"car").unwrap();

        assert!(bundle_provides_app_icon(
            &app,
            Some(&resources),
            None,
            Some("Runner"),
        ));
    }

    #[test]
    fn unbundled_or_iconless_app_keeps_runtime_icon_override() {
        let dir = tempfile::tempdir().unwrap();
        let unbundled = dir.path().join("target/debug");
        fs::create_dir_all(&unbundled).unwrap();
        fs::write(unbundled.join("Runner.icns"), b"icns").unwrap();
        assert!(!bundle_provides_app_icon(
            &unbundled,
            Some(&unbundled),
            Some("Runner.icns"),
            None,
        ));

        let wrapper = dir.path().join("Runner Screenshot Dev.app");
        let resources = wrapper.join("Contents/Resources");
        fs::create_dir_all(&resources).unwrap();
        assert!(!bundle_provides_app_icon(
            &wrapper,
            Some(&resources),
            None,
            None,
        ));
        assert!(!bundle_provides_app_icon(
            &wrapper,
            Some(&resources),
            Some("Runner.icns"),
            None,
        ));
        assert!(!bundle_provides_app_icon(
            &wrapper,
            Some(&resources),
            None,
            Some("Runner"),
        ));
    }
}
