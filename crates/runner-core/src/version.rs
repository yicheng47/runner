pub const CRATE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn display_version() -> String {
    compose_display_version(
        CRATE_VERSION,
        option_env!("RUNNER_MARKETING_VERSION"),
        option_env!("RUNNER_BUILD_STAMP"),
        option_env!("RUNNER_BUILD_SHA"),
    )
}

pub fn display_version_label() -> String {
    version_label(&display_version())
}

pub fn version_label(version: &str) -> String {
    if version.starts_with(|c: char| c.is_ascii_digit()) {
        format!("v{version}")
    } else {
        version.to_owned()
    }
}

fn compose_display_version(
    crate_version: &str,
    marketing_version: Option<&str>,
    build_stamp: Option<&str>,
    build_sha: Option<&str>,
) -> String {
    match build_stamp {
        Some(stamp) => {
            let marketing = marketing_version.map(str::to_owned).unwrap_or_else(|| {
                if crate_version.contains("-nightly") {
                    format!("{crate_version}.{stamp}")
                } else {
                    crate_version.to_owned()
                }
            });
            let sha = build_sha
                .filter(|sha| !sha.is_empty())
                .map(|sha| &sha[..sha.len().min(7)])
                .unwrap_or("unknown");
            format!("{marketing} ({sha})")
        }
        None => format!("{crate_version} (dev)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nightly_identifies_the_commit_without_an_official_version() {
        assert_eq!(
            compose_display_version(
                "0.8.2",
                Some("Nightly"),
                Some("20260821.1432"),
                Some("abc1234fedcba")
            ),
            "Nightly (abc1234)"
        );
    }

    #[test]
    fn production_stamp_does_not_enter_marketing_version() {
        assert_eq!(
            compose_display_version(
                "0.6.0-nightly",
                Some("0.6.0"),
                Some("20260821.1432"),
                Some("abc1234")
            ),
            "0.6.0 (abc1234)"
        );
    }

    #[test]
    fn unstamped_build_is_explicitly_dev() {
        assert_eq!(
            compose_display_version("0.6.0-nightly", None, None, None),
            "0.6.0-nightly (dev)"
        );
    }

    #[test]
    fn label_prefixes_numbered_versions_only() {
        assert_eq!(version_label("0.8.3 (05926cb)"), "v0.8.3 (05926cb)");
        assert_eq!(version_label("0.8.3-nightly (dev)"), "v0.8.3-nightly (dev)");
        assert_eq!(version_label("Nightly (05926cb)"), "Nightly (05926cb)");
    }
}
