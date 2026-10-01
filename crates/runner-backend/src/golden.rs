use serde_json::Value;
use std::path::Path;

pub(crate) fn assert_golden(name: &str, value: Value) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/session/manager/tests/expectations")
        .join(format!("{name}.json"));
    let actual = format!("{}\n", serde_json::to_string_pretty(&value).unwrap());
    if std::env::var("RUNNER_UPDATE_GOLDEN").as_deref() == Ok("1") {
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
