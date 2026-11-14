use super::*;

pub(super) fn partition_runtimes(
    runtimes: &[RuntimeExecutableStatus],
) -> (Vec<&RuntimeExecutableStatus>, Vec<&RuntimeExecutableStatus>) {
    runtimes.iter().partition(|runtime| match runtime.state {
        RuntimeRowState::Detected
        | RuntimeRowState::Override
        | RuntimeRowState::Checking
        | RuntimeRowState::ProbeTimedOut => true,
        RuntimeRowState::NotFound | RuntimeRowState::InvalidOverride => false,
    })
}

pub(super) fn runtime_default_enabled(
    catalog: &[RuntimeCatalogEntry],
    name: Runtime,
) -> Option<bool> {
    catalog
        .iter()
        .find(|runtime| runtime.name == name)
        .map(|runtime| runtime.default_enabled)
}

pub(super) fn runtime_default_eligible(
    runtime: &RuntimeExecutableStatus,
    catalog: &[RuntimeCatalogEntry],
    settings: &AppSettings,
) -> bool {
    matches!(
        runtime.effective_source,
        Some(RuntimeCommandSource::Detected | RuntimeCommandSource::Override)
    ) && runtime_default_enabled(catalog, runtime.name)
        .is_some_and(|default| settings.is_agent_enabled(runtime.name, default))
}

pub(super) fn reconcile_default_runtime(
    settings: &mut AppSettings,
    status: &RuntimeStatusResponse,
    catalog: &[RuntimeCatalogEntry],
) -> bool {
    if settings.default_runtime.is_empty() {
        return false;
    }
    let enabled = Runtime::parse(&settings.default_runtime)
        .and_then(|runtime| {
            runtime_default_enabled(catalog, runtime)
                .map(|default| settings.is_agent_enabled(runtime, default))
        })
        .unwrap_or(false);
    let available = status.runtimes.iter().any(|runtime| {
        runtime.name.key() == settings.default_runtime
            && matches!(
                runtime.effective_source,
                Some(RuntimeCommandSource::Detected | RuntimeCommandSource::Override)
            )
    });
    if !enabled || (!status.shell.checking && !available) {
        settings.default_runtime.clear();
        true
    } else {
        false
    }
}

pub(super) fn update_agent_enabled_preferences(
    settings: &mut AppSettings,
    runtime: Runtime,
    enabled: bool,
) -> bool {
    let before = (
        settings.default_runtime.clone(),
        settings.disabled_agents.clone(),
        settings.enabled_agents.clone(),
    );
    if enabled {
        settings.disabled_agents.remove(runtime.key());
        settings.enabled_agents.insert(runtime.to_string());
    } else {
        settings.disabled_agents.insert(runtime.to_string());
        settings.enabled_agents.remove(runtime.key());
        if settings.default_runtime == runtime.key() {
            settings.default_runtime.clear();
        }
    }
    before
        != (
            settings.default_runtime.clone(),
            settings.disabled_agents.clone(),
            settings.enabled_agents.clone(),
        )
}

pub(super) fn update_default_runtime(settings: &mut AppSettings, runtime: Option<Runtime>) -> bool {
    if settings.default_runtime == runtime.map(Runtime::key).unwrap_or_default() {
        return false;
    }
    settings.default_runtime = runtime.map(Runtime::key).unwrap_or_default().to_owned();
    true
}

pub(super) fn override_validation_error_message(error: &OverrideValidationError) -> String {
    error.message.clone()
}

pub(super) fn custom_flags_validation_error_message(error: &CustomFlagsValidationError) -> String {
    error.message.clone()
}

pub(super) fn custom_flags_placeholder(runtime: Runtime) -> &'static str {
    match runtime {
        Runtime::Codex => "--search -c model_verbosity=high",
        Runtime::ClaudeCode => "--verbose --max-budget-usd 5",
        Runtime::Antigravity => "--verbose",
        Runtime::Pi => "--no-tools --verbose",
        Runtime::Copilot => "--no-color",
        Runtime::Trae => "--verbose",
        Runtime::Cursor => "--verbose",
        Runtime::Grok => "--verbose",
        Runtime::OpenCode => "--print-logs",
        Runtime::Shell => "",
    }
}

pub(super) fn runtime_defaults_visible(runtime: &RuntimeExecutableStatus) -> bool {
    !matches!(
        runtime.state,
        RuntimeRowState::NotFound | RuntimeRowState::InvalidOverride
    )
}

pub(super) fn runtime_default_value(value: Option<&str>) -> String {
    value
        .unwrap_or(crate::i18n::text("ui.runtime_default_effort"))
        .to_owned()
}

pub(super) fn runtime_default_effort_value(value: Option<&str>) -> String {
    let value = runtime_default_value(value);
    runner_app::ui::select::effort_label(&value, &value)
        .resolve()
        .to_string()
}

pub(super) fn runtime_presentation(
    runtime: &RuntimeExecutableStatus,
    probe_outcome: Option<DiscoveryOutcome>,
    validation: Option<&str>,
    saving: bool,
) -> RuntimePresentation {
    let state = if validation.is_some() {
        RuntimeRowState::InvalidOverride
    } else {
        runtime.state
    };
    let (badge, tone, spinning) = if saving {
        (crate::i18n::text("ui.saving"), BadgeTone::Muted, true)
    } else {
        match state {
            RuntimeRowState::Detected => {
                (crate::i18n::text("ui.detected"), BadgeTone::Accent, false)
            }
            RuntimeRowState::Override => {
                (crate::i18n::text("ui.override"), BadgeTone::Neutral, false)
            }
            RuntimeRowState::NotFound => {
                (crate::i18n::text("ui.not_found"), BadgeTone::Danger, false)
            }
            RuntimeRowState::Checking => (crate::i18n::text("ui.checking"), BadgeTone::Muted, true),
            RuntimeRowState::ProbeTimedOut => (
                if probe_outcome == Some(DiscoveryOutcome::Timeout) {
                    crate::i18n::text("ui.probe_timed_out")
                } else {
                    crate::i18n::text("ui.detection_failed")
                },
                BadgeTone::Warning,
                false,
            ),
            RuntimeRowState::InvalidOverride => {
                (crate::i18n::text("ui.invalid"), BadgeTone::Danger, false)
            }
        }
    };
    let caption = if let Some(validation) = validation {
        Some(validation.to_owned())
    } else {
        match state {
            RuntimeRowState::Override => Some(runtime.detected_path.as_ref().map_or_else(
                || {
                    crate::i18n::tf(
                        "format.was_not_found_automatically",
                        &[("command", (runtime.command).to_string())],
                    )
                },
                |path| crate::i18n::tf("format.detected", &[("path", (path).to_string())]),
            )),
            RuntimeRowState::NotFound => Some(crate::i18n::tf(
                "format.install_or_set_an_explicit_executable_path",
                &[("runtime", (runtime.display_name).to_string())],
            )),
            RuntimeRowState::ProbeTimedOut => {
                let failure = match probe_outcome {
                    Some(DiscoveryOutcome::Timeout) => {
                        crate::i18n::text("ui.login_shell_timed_out")
                    }
                    Some(DiscoveryOutcome::WindowsRegistryError) => {
                        crate::i18n::text("ui.windows_path_refresh_failed")
                    }
                    _ => crate::i18n::text("ui.shell_detection_failed"),
                };
                Some(runtime.detected_path.as_ref().map_or_else(
                    || {
                        crate::i18n::tf(
                            "format.refresh_or_set_an_explicit_executable_path",
                            &[("failure", (failure).to_string())],
                        )
                    },
                    |path| {
                        crate::i18n::tf(
                            "format.using_the_last_resolved_path",
                            &[
                                ("failure", (failure).to_string()),
                                ("path", (path).to_string()),
                            ],
                        )
                    },
                ))
            }
            RuntimeRowState::Checking if runtime.detected_path.is_some() => {
                runtime.detected_path.as_ref().map(|path| {
                    crate::i18n::tf(
                        "format.using_last_detected_path",
                        &[("path", (path).to_string())],
                    )
                })
            }
            _ => None,
        }
    };
    let version = validation
        .is_none()
        .then(|| version_caption(runtime))
        .flatten();
    let caption = match (version, caption) {
        (Some(version), Some(caption)) => Some(format!("{version} · {caption}")),
        (Some(version), None) => Some(match &runtime.detected_path {
            Some(path) if state == RuntimeRowState::Detected => crate::i18n::tf(
                "format.detected_version_path",
                &[
                    ("version", (version).to_string()),
                    ("path", (path).to_string()),
                ],
            ),
            _ => version,
        }),
        (None, caption) => caption,
    };
    let auto_path = if runtime.state == RuntimeRowState::Checking {
        crate::i18n::message("ui.auto_detecting")
    } else if let Some(path) = &runtime.detected_path {
        crate::i18n::messagef("format.auto", &[("path", (path).to_string())])
    } else {
        crate::i18n::messagef(
            "format.auto_not_found_on_path",
            &[("command", (runtime.command).to_string())],
        )
    };
    RuntimePresentation {
        badge,
        tone,
        spinning,
        caption,
        auto_path,
        show_reset: runtime.override_path.is_some(),
    }
}

/// `2.1.266`, or `0.153.4 → 0.155.0` while an update is available.
pub(super) fn version_caption(runtime: &RuntimeExecutableStatus) -> Option<String> {
    let installed = runtime.installed_version.as_deref()?;
    Some(match runtime.available_version.as_deref() {
        Some(available) => format!("{installed} → {available}"),
        None => installed.to_owned(),
    })
}

/// Update is offered only while npm has a newer version, and never for a
/// runtime without an update command. Running sessions only earn a caption.
pub(super) fn update_action(
    runtime: &RuntimeExecutableStatus,
    running: usize,
) -> Option<UpdateAction> {
    let installed = runtime.installed_version.as_deref()?;
    runtime.available_version.as_ref()?;
    let name = &runtime.display_name;
    let caption = match running {
        0 => None,
        1 => Some(crate::i18n::tf(
            "format.1_running_session_keeps_until_it_relaunches",
            &[
                ("name", (name).to_string()),
                ("installed", (installed).to_string()),
            ],
        )),
        running => Some(crate::i18n::tf(
            "format.runtime_running_sessions_keep_version",
            &[
                ("running", (running).to_string()),
                ("name", (name).to_string()),
                ("installed", (installed).to_string()),
            ],
        )),
    };
    Some(UpdateAction { caption })
}

pub(super) fn runtime_badge(runtime: Runtime, presentation: &RuntimePresentation) -> AnyElement {
    let (background, foreground) = match presentation.tone {
        BadgeTone::Accent => (theme::with_alpha(theme::accent(), 0.1), theme::accent()),
        BadgeTone::Neutral => (theme::with_alpha(theme::text(), 0.05), theme::text()),
        BadgeTone::Danger => (theme::with_alpha(theme::danger(), 0.1), theme::danger()),
        BadgeTone::Warning => (theme::with_alpha(theme::warning(), 0.1), theme::warning()),
        BadgeTone::Muted => (theme::with_alpha(theme::faint(), 0.1), theme::faint()),
    };
    div()
        .debug_selector(|| format!("AGENT_BADGE_{runtime}"))
        .flex()
        .items_center()
        .gap(rems(6. / 16.))
        .rounded_full()
        .bg(background)
        .px_2()
        .py(rems(2. / 16.))
        .text_size(theme::text_caption())
        .font_weight(FontWeight::MEDIUM)
        .text_color(foreground)
        .child(if presentation.spinning {
            spinner(
                SharedString::from(format!("runtime-{runtime}-spinner")),
                10.,
                foreground,
            )
        } else {
            div()
                .size(rems(6. / 16.))
                .rounded_full()
                .bg(foreground)
                .into_any_element()
        })
        .child(presentation.badge)
        .into_any_element()
}

pub(super) fn shell_description(shell: Option<&ShellDiscoveryStatus>) -> String {
    let Some(shell) = shell else {
        return crate::i18n::text("ui.loading_shell_discovery_status").into();
    };
    let selected = shell.shell.as_deref();
    let duration = shell
        .duration_ms
        .map(|duration| {
            crate::i18n::tf(
                "format.in_s",
                &[(
                    "seconds",
                    (format!("{:.1}", duration as f64 / 1_000.)).to_string(),
                )],
            )
        })
        .unwrap_or_default();
    if shell.checking {
        #[cfg(windows)]
        if selected.is_none() {
            return if shell.using_last_known_good {
                crate::i18n::text(
                    "ui.refreshing_windows_path_agents_keep_using_the_last_saved_environment",
                )
                .into()
            } else {
                crate::i18n::text("ui.checking_the_windows_registry_for_path").into()
            };
        }
        let Some(selected) = selected else {
            return crate::i18n::text("ui.checking_login_shell_for_path_and_proxy_settings").into();
        };
        return if shell.using_last_known_good {
            crate::i18n::tf(
                "format.checking_agents_keep_using_the_last_saved_environment",
                &[("selected", (selected).to_string())],
            )
        } else {
            crate::i18n::tf(
                "format.checking_for_path_and_proxy_settings",
                &[("selected", (selected).to_string())],
            )
        };
    }
    match shell.outcome {
        Some(DiscoveryOutcome::Ok) => crate::i18n::tf(
            "format.path_captured_from_your_login_shell_spawned_agents_inherit_it",
            &[
                ("shell", (selected.unwrap_or("")).to_string()),
                ("duration", (duration).to_string()),
            ],
        ),
        Some(DiscoveryOutcome::WindowsRegistry) => crate::i18n::tf(
            "format.path_refreshed_from_the_windows_registry_spawned_agents_inherit_it",
            &[("duration", (duration).to_string())],
        ),
        Some(DiscoveryOutcome::WindowsRegistryError) => crate::i18n::text(
            "ui.could_not_read_windows_path_from_the_registry_refresh_or_set_an_override_below",
        )
        .into(),
        Some(DiscoveryOutcome::Timeout) => crate::i18n::tf(
            "format.did_not_respond_agents_keep_using_the_last_saved_environment",
            &[
                ("shell", (selected.unwrap_or("")).to_string()),
                ("duration", (duration).to_string()),
            ],
        ),
        Some(DiscoveryOutcome::SpawnError) => crate::i18n::tf(
            "format.could_not_start_refresh_after_fixing_the_login_shell_or_set_an_override_below",
            &[("shell", (selected.unwrap_or("")).to_string())],
        ),
        Some(DiscoveryOutcome::EmptyCapture) => crate::i18n::tf(
            "format.returned_no_usable_environment_refresh_or_set_an_override_below",
            &[("shell", (selected.unwrap_or("")).to_string())],
        ),
        Some(DiscoveryOutcome::NoShell) => crate::i18n::text(
            "ui.no_supported_login_shell_was_configured_set_an_executable_override_below",
        )
        .into(),
        None => selected.map_or_else(
            || crate::i18n::text("ui.waiting_to_check_the_login_shell").into(),
            |shell| crate::i18n::tf("format.waiting_to_check", &[("shell", (shell).to_string())]),
        ),
    }
}
