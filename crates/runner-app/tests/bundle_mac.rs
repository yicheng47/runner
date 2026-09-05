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
fn nightly_plist_uses_the_isolated_rolling_channel() {
    let version = bundle_version();
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
        "com.wycstudios.runner.nightly"
    );
    assert_eq!(plist_value(&plist, "CFBundleName"), "Runner Nightly");
    assert_eq!(plist_value(&plist, "CFBundleExecutable"), "Runner");
    assert_eq!(
        plist_value(&plist, "SUFeedURL"),
        "https://github.com/yicheng47/runner/releases/download/nightly/appcast.xml"
    );
    assert_eq!(
        plist_value(&plist, "CFBundleShortVersionString"),
        format!("{version}.20260821.1432")
    );
    assert_eq!(plist_value(&plist, "CFBundleVersion"), "20260821.1432");
    assert_eq!(
        plist_value(&plist, "SUPublicEDKey"),
        "X2r1GfMmzcCS/9//sSUyyBNxMajjcMqVwQeHHKtAHMs="
    );
    assert_update_preferences(&plist);
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
