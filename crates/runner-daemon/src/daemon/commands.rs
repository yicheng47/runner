use crate::cli_install::{self, *};
use crate::error::Result;
use std::path::PathBuf;
fn with_command_registry<T>(
    integration: bool,
    action: impl FnOnce(&mut dyn UserPathRegistry) -> T,
) -> T {
    if integration {
        #[cfg(windows)]
        {
            let mut registry = cli_install::SystemUserPathRegistry;
            action(&mut registry)
        }
        #[cfg(not(windows))]
        {
            let mut registry = NoUserPathRegistry;
            action(&mut registry)
        }
    } else {
        let mut registry = NoUserPathRegistry;
        action(&mut registry)
    }
}
fn with_command_escalation<T>(
    integration: bool,
    action: impl FnOnce(&mut dyn CommandEscalation) -> T,
) -> T {
    if integration {
        #[cfg(target_os = "macos")]
        {
            let mut escalation = OsascriptEscalation;
            action(&mut escalation)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let mut escalation = NoEscalation;
            action(&mut escalation)
        }
    } else {
        let mut escalation = NoEscalation;
        action(&mut escalation)
    }
}
pub(super) fn status(
    inputs: &CommandInstallInputs,
    system: bool,
) -> Result<(RunnerCommandStatus, bool, PathBuf)> {
    let force = cli_install::force_escalated_command_install();
    with_command_registry(system, |registry| {
        let status = cli_install::command_status(inputs, registry)?;
        let requires = inputs.platform == CommandPlatform::Unix
            && (force || cli_install::default_command_target(inputs, registry)?.is_none());
        let target = cli_install::explicit_command_target_path(inputs, registry, force)?;
        Ok((status, requires, target))
    })
}
pub(super) fn install_default(
    inputs: &CommandInstallInputs,
    system: bool,
) -> Result<CommandActionOutcome> {
    with_command_registry(system, |registry| {
        cli_install::install_command_default(inputs, registry, &mut NoEscalation)
    })
}
pub(super) fn action(
    inputs: &CommandInstallInputs,
    install: bool,
    system: bool,
) -> Result<CommandActionOutcome> {
    with_command_registry(system, |registry| {
        with_command_escalation(system, |escalation| {
            if install {
                cli_install::install_command_explicit(
                    inputs,
                    registry,
                    escalation,
                    cli_install::force_escalated_command_install(),
                )
            } else {
                cli_install::uninstall_command(inputs, registry, escalation)
            }
        })
    })
}
