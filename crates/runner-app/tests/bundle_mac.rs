#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("script/bundle-mac")
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new("bash")
        .arg(script())
        .args(args)
        .output()
        .unwrap()
}

fn bundle_version() -> String {
    let output = run(&["--print-version"]);
    assert!(output.status.success());
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn plist_value<'a>(plist: &'a str, key: &str) -> &'a str {
    let marker = format!("<key>{key}</key>");
    let after_key = plist.split_once(&marker).unwrap().1;
    let after_open = after_key.split_once("<string>").unwrap().1;
    after_open.split_once("</string>").unwrap().0
}

fn plist_bool(plist: &str, key: &str) -> bool {
    let marker = format!("<key>{key}</key>");
    let value = plist.split_once(&marker).unwrap().1.trim_start();
    if value.starts_with("<true/>") {
        true
    } else if value.starts_with("<false/>") {
        false
    } else {
        panic!("{key} is not a plist boolean")
    }
}

fn assert_update_preferences(plist: &str) {
    assert!(!plist_bool(plist, "SUAllowsAutomaticUpdates"));
    assert!(!plist_bool(plist, "SUAutomaticallyUpdate"));
    assert!(plist_bool(plist, "SUEnableAutomaticChecks"));
    assert!(!plist_bool(plist, "SUEnableSystemProfiling"));
}

#[test]
fn nightly_plist_uses_the_production_identity_with_the_nightly_feed() {
    let output = run(&[
        "--channel",
        "nightly",
        "--stamp",
        "20260821.1432",
        "--sha",
        "abc1234",
        "--print-plist",
    ]);
    assert!(output.status.success());
    let plist = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        plist_value(&plist, "CFBundleIdentifier"),
        "com.wycstudios.runner"
    );
    assert_eq!(plist_value(&plist, "CFBundleName"), "Runner");
    assert_eq!(plist_value(&plist, "CFBundleDisplayName"), "Runner");
    assert_eq!(plist_value(&plist, "CFBundleExecutable"), "Runner");
    assert_eq!(
        plist_value(&plist, "SUFeedURL"),
        "https://github.com/yicheng47/runner/releases/download/nightly/appcast.xml"
    );
    assert_eq!(plist_value(&plist, "CFBundleShortVersionString"), "abc1234");
    assert_eq!(plist_value(&plist, "CFBundleVersion"), "20260821.1432");
    assert_eq!(
        plist_value(&plist, "SUPublicEDKey"),
        "X2r1GfMmzcCS/9//sSUyyBNxMajjcMqVwQeHHKtAHMs="
    );
    assert_update_preferences(&plist);
}

#[test]
fn preview_nightly_plist_reads_its_own_feed() {
    let output = run(&[
        "--channel",
        "preview-nightly",
        "--stamp",
        "20260821.1432",
        "--sha",
        "abc1234",
        "--print-plist",
    ]);
    assert!(output.status.success());
    let plist = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        plist_value(&plist, "CFBundleIdentifier"),
        "com.wycstudios.runner"
    );
    assert_eq!(plist_value(&plist, "CFBundleName"), "Runner");
    assert_eq!(
        plist_value(&plist, "SUFeedURL"),
        "https://github.com/yicheng47/runner/releases/download/preview-nightly/appcast.xml"
    );
    assert_eq!(plist_value(&plist, "CFBundleShortVersionString"), "abc1234");
    assert_eq!(plist_value(&plist, "CFBundleVersion"), "20260821.1432");
    assert_update_preferences(&plist);
    let nightly = run(&[
        "--channel",
        "nightly",
        "--stamp",
        "20260821.1432",
        "--sha",
        "abc1234",
        "--print-plist",
    ]);
    assert_eq!(
        plist.replace("/download/preview-nightly/", "/download/nightly/"),
        String::from_utf8(nightly.stdout).unwrap()
    );
}

#[test]
fn production_plist_keeps_the_marketing_version_separate_from_the_stamp() {
    let version = bundle_version();
    let output = run(&[
        "--channel",
        "production",
        "--stamp",
        "20260821.1432",
        "--sha",
        "abc1234",
        "--print-plist",
    ]);
    assert!(output.status.success());
    let plist = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        plist_value(&plist, "CFBundleIdentifier"),
        "com.wycstudios.runner"
    );
    assert_eq!(plist_value(&plist, "CFBundleName"), "Runner");
    assert_eq!(plist_value(&plist, "CFBundleExecutable"), "Runner");
    assert_eq!(
        plist_value(&plist, "SUFeedURL"),
        "https://github.com/yicheng47/runner/releases/latest/download/appcast.xml"
    );
    assert_eq!(
        plist_value(&plist, "CFBundleShortVersionString"),
        version.strip_suffix("-nightly").unwrap_or(&version)
    );
    assert_eq!(plist_value(&plist, "CFBundleVersion"), "20260821.1432");
    assert_eq!(
        plist_value(&plist, "SUPublicEDKey"),
        "X2r1GfMmzcCS/9//sSUyyBNxMajjcMqVwQeHHKtAHMs="
    );
    assert_update_preferences(&plist);
}

#[test]
fn channel_plists_only_differ_in_the_feed_and_short_version() {
    let [mut nightly, production] = ["nightly", "production"].map(|channel| {
        let output = run(&[
            "--channel",
            channel,
            "--stamp",
            "20260821.1432",
            "--sha",
            "abc1234",
            "--print-plist",
        ]);
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap()
    });
    for key in ["SUFeedURL", "CFBundleShortVersionString"] {
        let nightly_value = plist_value(&nightly, key);
        let production_value = plist_value(&production, key);
        assert_ne!(nightly_value, production_value);
        nightly = nightly.replace(
            &format!("<key>{key}</key>\n    <string>{nightly_value}</string>"),
            &format!("<key>{key}</key>\n    <string>{production_value}</string>"),
        );
    }
    assert_eq!(nightly, production);
}

#[test]
fn intel_plists_read_the_intel_feed_of_their_channel() {
    for (channel, feed) in [
        (
            "nightly",
            "https://github.com/yicheng47/runner/releases/download/nightly/appcast-x64.xml",
        ),
        (
            "preview-nightly",
            "https://github.com/yicheng47/runner/releases/download/preview-nightly/appcast-x64.xml",
        ),
        (
            "production",
            "https://github.com/yicheng47/runner/releases/latest/download/appcast-x64.xml",
        ),
    ] {
        let [apple_silicon, intel] = [None, Some("x64")].map(|arch| {
            let mut args = vec![
                "--channel",
                channel,
                "--stamp",
                "20260821.1432",
                "--sha",
                "abc1234",
                "--print-plist",
            ];
            if let Some(arch) = arch {
                args.extend(["--arch", arch]);
            }
            let output = run(&args);
            assert!(output.status.success());
            String::from_utf8(output.stdout).unwrap()
        });
        assert_eq!(plist_value(&intel, "SUFeedURL"), feed);
        assert_eq!(
            intel.replace("/appcast-x64.xml", "/appcast.xml"),
            apple_silicon,
            "{channel}"
        );
    }
    let explicit = run(&[
        "--channel",
        "nightly",
        "--arch",
        "arm64",
        "--stamp",
        "20260821.1432",
        "--sha",
        "abc1234",
        "--print-plist",
    ]);
    assert!(explicit.status.success());
    assert_eq!(
        plist_value(&String::from_utf8(explicit.stdout).unwrap(), "SUFeedURL"),
        "https://github.com/yicheng47/runner/releases/download/nightly/appcast.xml"
    );
    let universal = run(&[
        "--channel",
        "nightly",
        "--arch",
        "universal",
        "--print-plist",
    ]);
    assert!(!universal.status.success());
}

#[test]
fn stamp_uses_two_ordered_signed_32_bit_components() {
    let output = run(&[
        "--channel",
        "nightly",
        "--stamp",
        "20260821.1432",
        "--print-stamp",
    ]);
    assert!(output.status.success());
    let stamp = String::from_utf8(output.stdout).unwrap();
    let stamp = stamp.trim();
    let components = stamp
        .split('.')
        .map(|component| component.parse::<u32>().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(components, vec![20_260_821, 1_432]);
    assert!(components
        .iter()
        .all(|component| *component < i32::MAX as u32));

    let invalid = run(&[
        "--channel",
        "nightly",
        "--stamp",
        "202608211432",
        "--print-stamp",
    ]);
    assert!(!invalid.status.success());
}
