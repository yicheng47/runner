use serde_json::Value;
use std::path::Path;

pub(crate) fn assert_golden(name: &str, value: Value) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/session/manager/tests/expectations")
        .join(format!("{name}.json"));
    let actual = format!("{}\n", serde_json::to_string_pretty(&value).unwrap());
    if std::env::var("RUNNER_UPDATE_GOLDEN").as_deref() == Ok("1")
        || (name.starts_with("catalog-")
            && std::env::var("RUNNER_UPDATE_CATALOG_GOLDEN").as_deref() == Ok("1"))
    {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &actual).unwrap();
    }
    let expected = std::fs::read_to_string(&path).unwrap();
    if actual != expected {
        let actual_path = std::env::temp_dir().join(format!("777-{name}-actual.json"));
        std::fs::write(&actual_path, actual).unwrap();
        let diff = std::process::Command::new("diff")
            .args(["-u"])
            .arg(&path)
            .arg(&actual_path)
            .output()
            .unwrap();
        panic!(
            "golden {name} changed:\n{}",
            String::from_utf8_lossy(&diff.stdout)
        );
    }
}

#[test]
fn permission_goldens() {
    use crate::model::Runtime;
    use crate::router::runtime::PermissionMode;
    let args = [
        "--keep",
        "--ask-for-approval",
        "never",
        "--sandbox=workspace-write",
        "--permission-mode",
        "auto",
        "--dangerously-skip-permissions",
        "--allow-tool",
        "write",
        "shell",
        "--yolo",
        "--allow-all",
        "--allow-all-tools",
        "--allow-all-paths",
        "--allow-all-urls",
        "--mode",
        "accept-edits",
        "-mode=accept-edits",
        "-dangerously-skip-permissions=true",
        "--tail",
    ]
    .map(String::from)
    .to_vec();
    let mut rows = Vec::new();
    for key in Runtime::ALL
        .map(Runtime::key)
        .into_iter()
        .chain(["unknown-runtime"])
    {
        for mode in [
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Auto,
            PermissionMode::Bypass,
        ] {
            let stored = crate::runtimes::for_key(key)
                .permissions()
                .apply(&args, mode);
            rows.push(
                serde_json::json!({"runtime": key, "mode": mode, "stored": stored,
                "inferred": crate::runtimes::for_key(key).permissions().infer(&stored)}),
            );
        }
    }
    assert_golden("permissions", serde_json::json!(rows));
}

thread_local! {
    static COMMAND_CAPTURE: std::cell::RefCell<Option<Vec<Value>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(unix)]
pub(crate) fn capture_commands(run: impl FnOnce()) -> Vec<Value> {
    COMMAND_CAPTURE.with_borrow_mut(|capture| *capture = Some(Vec::new()));
    run();
    COMMAND_CAPTURE.with_borrow_mut(|capture| capture.take().unwrap())
}

pub(crate) fn record(value: Value) -> bool {
    COMMAND_CAPTURE.with_borrow_mut(|capture| {
        if let Some(capture) = capture {
            capture.push(value);
            true
        } else {
            false
        }
    })
}

pub(crate) fn record_command(
    command: &std::process::Command,
    stdin: Option<&[u8]>,
    timeout: std::time::Duration,
) -> bool {
    let mut env: std::collections::BTreeMap<_, _> = command
        .get_envs()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.map(|value| value.to_string_lossy().into_owned()),
            )
        })
        .collect();
    if env
        .get("PATH")
        .and_then(|value| value.as_deref())
        .is_some_and(|path| path.starts_with("/golden/bin:"))
    {
        env.insert("PATH".into(), Some("/golden/bin:<DIRECT_CHAT_PATH>".into()));
    }
    record(serde_json::json!({
        "command": command.get_program().to_string_lossy(),
        "args": command.get_args().map(|arg| arg.to_string_lossy().into_owned()).collect::<Vec<_>>(),
        "env": env,
        "cwd": command.get_current_dir().map(|cwd| cwd.to_string_lossy().into_owned()),
        "stdin": stdin.map(|bytes| String::from_utf8_lossy(bytes).into_owned()),
        "timeout_ms": timeout.as_millis(),
    }))
}

#[cfg(unix)]
pub(crate) fn normalize(value: Value, home: &Path) -> Value {
    let mut text = serde_json::to_string(&value).unwrap();
    for root in [Some(home.to_path_buf()), runner_core::app_paths::home_dir()]
        .into_iter()
        .flatten()
    {
        let root = serde_json::to_string(&root.to_string_lossy()).unwrap();
        text = text.replace(&root[1..root.len() - 1], "<HOME>");
    }
    serde_json::from_str(&text.replace(env!("CARGO_PKG_VERSION"), "<VERSION>")).unwrap()
}

thread_local! {
    static CONFIG_ENV: std::cell::RefCell<Option<std::collections::BTreeMap<&'static str, std::ffi::OsString>>> = const { std::cell::RefCell::new(None) };
}

#[cfg(unix)]
pub(crate) fn with_config_env<T>(
    env: std::collections::BTreeMap<&'static str, std::ffi::OsString>,
    run: impl FnOnce() -> T,
) -> T {
    let prior = CONFIG_ENV.with_borrow_mut(|value| value.replace(env));
    let result = run();
    CONFIG_ENV.with_borrow_mut(|value| *value = prior);
    result
}

pub(crate) fn config_var_os(name: &str) -> Option<std::ffi::OsString> {
    CONFIG_ENV.with_borrow(|env| match env {
        Some(env) => env.get(name).cloned(),
        None => std::env::var_os(name),
    })
}

pub(crate) fn config_home() -> Option<std::path::PathBuf> {
    crate::runtimes::test_home().or_else(runner_core::app_paths::home_dir)
}
