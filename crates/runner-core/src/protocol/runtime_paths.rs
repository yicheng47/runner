use super::OverrideValidationError;
use std::path::{Path, PathBuf};
pub fn validate_executable_path(path: &Path) -> std::result::Result<(), OverrideValidationError> {
    if !path.is_absolute() {
        return Err(validation_error(
            "not_absolute",
            "Choose an absolute executable path.",
        ));
    }
    let metadata = std::fs::metadata(path)
        .map_err(|_| validation_error("not_found", "File does not exist."))?;
    if !metadata.is_file() {
        return Err(validation_error("not_file", "Not a regular file."));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return Err(validation_error(
                "not_executable",
                "Not an executable file.",
            ));
        }
    }
    Ok(())
}
pub fn validation_error(code: &str, message: impl Into<String>) -> OverrideValidationError {
    OverrideValidationError {
        code: code.to_string(),
        message: message.into(),
    }
}
pub fn find_executable(command: &str, path: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    let extensions = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    for entry in std::env::split_paths(path).filter(|entry| !entry.as_os_str().is_empty()) {
        #[cfg(windows)]
        for extension in extensions
            .split(';')
            .filter(|extension| !extension.is_empty())
        {
            let candidate = entry.join(format!("{command}{extension}"));
            if validate_executable_path(&candidate).is_ok() {
                return Some(candidate);
            }
        }
        #[cfg(windows)]
        if !Path::new(command)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extensions.split(';').any(|suffix| {
                    suffix
                        .strip_prefix('.')
                        .is_some_and(|suffix| extension.eq_ignore_ascii_case(suffix))
                })
            })
        {
            continue;
        }
        let candidate = entry.join(command);
        if validate_executable_path(&candidate).is_ok() {
            return Some(candidate);
        }
    }
    None
}
