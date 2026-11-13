use std::path::Path;
use std::process::Command;

const TEST: &str = "daemon_cleanup_preserves_recording_without_mission_context";
const CHILD: &str = "DAEMON_CLEAN_ENV_TEST";
const RECORDING: &str = "RUNNER_RECORD_INPUT_FIXTURE";

// No libtest worker: environment mutation must run on the only thread in this process.
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--ignored") {
        return;
    }
    if let Some(filter) = args.first().filter(|arg| !arg.starts_with('-')) {
        if !TEST.contains(filter.as_str()) {
            return;
        }
    }
    if args.iter().any(|arg| arg == "--list") {
        println!("{TEST}: test");
        return;
    }
    let data = Path::new("test-data");
    let path = std::env::join_paths([data.join("bin"), data.join("missions/slot"), "kept".into()])
        .unwrap();
    if std::env::var_os(CHILD).is_some() {
        // This executable has no test harness and has not started any threads.
        unsafe { runner_core::daemon_process::clean_environment(data) };
        assert!(std::env::var_os("RUNNER_MISSION_ID").is_none());
        assert_eq!(std::env::var_os(RECORDING).unwrap(), "fixture-prefix");
        assert_eq!(
            std::env::var_os("PATH").unwrap(),
            std::env::join_paths([Path::new("kept")]).unwrap()
        );
        return;
    }
    let output = Command::new(std::env::current_exe().unwrap())
        .env(CHILD, "1")
        .env(RECORDING, "fixture-prefix")
        .env("RUNNER_MISSION_ID", "parent-mission")
        .env("PATH", path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
}
