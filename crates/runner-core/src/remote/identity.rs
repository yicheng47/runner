use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, rename_all = "camelCase")]
pub struct HostInfo {
    pub hostname: String,
    pub platform: String,
    pub arch: String,
    pub os_version: String,
    pub runner_version: String,
}
impl HostInfo {
    pub fn current() -> Self {
        Self::normalized(
            &hostname::get().unwrap_or_default().to_string_lossy(),
            std::env::consts::OS,
            std::env::consts::ARCH,
            &os_version(),
            crate::version::CRATE_VERSION,
        )
    }
    pub fn normalized(
        hostname: &str,
        platform: &str,
        arch: &str,
        os_version: &str,
        version: &str,
    ) -> Self {
        Self {
            hostname: bounded(hostname, 255),
            platform: if platform == "macos" {
                "macos"
            } else if platform == "windows" {
                "windows"
            } else {
                "linux"
            }
            .into(),
            arch: bounded(
                &if arch == "aarch64" {
                    "arm64".into()
                } else {
                    arch.to_lowercase()
                        .chars()
                        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
                        .collect::<String>()
                },
                16,
            ),
            os_version: bounded(os_version, 64),
            runner_version: bounded(version, 32),
        }
    }
}
fn bounded(value: &str, limit: usize) -> String {
    let value = value.trim();
    if value.is_empty() {
        return "unknown".into();
    }
    let mut end = value.len().min(limit);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].into()
}
#[cfg(target_os = "macos")]
fn os_version() -> String {
    let output = std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", "kern.osproductversion"])
        .output();
    output
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .map(|s| format!("macOS {s}"))
        .unwrap_or_else(|| "unknown".into())
}
#[cfg(windows)]
fn os_version() -> String {
    use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion");
    match key {
        Ok(key) => windows_version(
            &key.get_value::<String, _>("CurrentBuildNumber")
                .unwrap_or_default(),
            &key.get_value::<String, _>("DisplayVersion")
                .unwrap_or_default(),
        ),
        Err(_) => "unknown".into(),
    }
}
#[cfg(not(any(target_os = "macos", windows)))]
fn os_version() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|s| {
            s.lines().find_map(|l| {
                l.strip_prefix("PRETTY_NAME=")
                    .map(|v| v.trim_matches('"').to_owned())
            })
        })
        .unwrap_or_else(|| "unknown".into())
}
#[cfg(any(windows, test))]
fn windows_version(build: &str, display: &str) -> String {
    let Ok(build) = build.parse::<u32>() else {
        return "unknown".into();
    };
    if display.trim().is_empty() {
        return "unknown".into();
    }
    format!(
        "Windows {} {}",
        if build >= 22000 { "11" } else { "10" },
        display.trim()
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn normalizes_platforms_and_byte_limits() {
        for (os, arch, expected) in [
            ("macos", "aarch64", "arm64"),
            ("windows", "x86_64", "x86_64"),
            ("linux", "x86_64", "x86_64"),
            ("linux", "aarch64", "arm64"),
        ] {
            let info =
                HostInfo::normalized(&"主机".repeat(100), os, arch, &"版".repeat(30), "0.16.4");
            assert_eq!(info.platform, os);
            assert_eq!(info.arch, expected);
            assert!(info.hostname.len() <= 255);
            assert!(info.os_version.len() <= 64);
        }
        assert_eq!(windows_version("22631", "24H2"), "Windows 11 24H2");
        assert_eq!(windows_version("19045", "22H2"), "Windows 10 22H2");
        assert_eq!(windows_version("bad", ""), "unknown");
        assert_eq!(
            HostInfo::normalized("", "linux", "A!BC", "", "").arch,
            "abc"
        );
        let info = HostInfo::current();
        assert!(!info.hostname.is_empty());
        assert!(!info.os_version.is_empty());
    }
}
