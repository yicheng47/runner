use std::fmt;
use std::path::{Path, PathBuf};

const APP_IDENTIFIER: &str = "com.wycstudios.runner";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IpcEndpoint(pub PathBuf);

impl fmt::Display for IpcEndpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.display().fmt(f)
    }
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::home_dir()
}

fn app_identifier(debug: bool) -> String {
    if debug {
        format!("{APP_IDENTIFIER}-dev")
    } else {
        APP_IDENTIFIER.to_string()
    }
}

pub fn app_data_dir(debug: bool) -> Option<PathBuf> {
    Some(app_data_dir_for_home(&home_dir()?, debug))
}

/// On Windows and Linux, `home` is the fallback when APPDATA or XDG_DATA_HOME is unset.
pub fn app_data_dir_for_home(home: &Path, debug: bool) -> PathBuf {
    #[cfg(target_os = "macos")]
    let base = home.join("Library").join("Application Support");
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local").join("share"));
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join("AppData").join("Roaming"));
    base.join(app_identifier(debug))
}

pub fn log_dir(debug: bool) -> Option<PathBuf> {
    Some(log_dir_for_home(&home_dir()?, debug))
}

pub fn log_dir_for_home(home: &Path, debug: bool) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        home.join("Library")
            .join("Logs")
            .join(app_identifier(debug))
    }
    #[cfg(not(target_os = "macos"))]
    {
        app_data_dir_for_home(home, debug).join("logs")
    }
}

pub fn mcp_endpoint(app_data_dir: &Path, debug: bool) -> IpcEndpoint {
    #[cfg(unix)]
    {
        let _ = debug;
        IpcEndpoint(app_data_dir.join("mcp.sock"))
    }
    #[cfg(windows)]
    {
        let _ = app_data_dir;
        IpcEndpoint(PathBuf::from(format!(
            r"\\.\pipe\{}",
            app_identifier(debug)
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcp_endpoint_separates_development_and_production() {
        let home = Path::new("/home/tester");
        let release = mcp_endpoint(&app_data_dir_for_home(home, false), false);
        let debug = mcp_endpoint(&app_data_dir_for_home(home, true), true);
        assert_ne!(release, debug);
        #[cfg(unix)]
        assert!(release.0.ends_with("com.wycstudios.runner/mcp.sock"));
        #[cfg(windows)]
        {
            assert_eq!(release.to_string(), r"\\.\pipe\com.wycstudios.runner");
            assert_eq!(debug.to_string(), r"\\.\pipe\com.wycstudios.runner-dev");
        }
    }
}
